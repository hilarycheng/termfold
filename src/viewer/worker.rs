use std::{
    collections::{HashMap, VecDeque},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
};

use crate::{session::Size, terminal::Terminal};

use super::{SearchStart, SearchStep, SearchWork, Viewer, ViewerMode};

pub(super) const VIEWER_COMMAND_CAPACITY: usize = 8;
const VIEWER_RESULT_CAPACITY: usize = VIEWER_COMMAND_CAPACITY * 2;
type ViewerId = u64;

pub(super) enum ViewerCommand {
    Open {
        id: ViewerId,
        generation: u64,
        path: PathBuf,
        tab_width: usize,
        reply: SyncSender<ViewerResult>,
    },
    Close {
        id: ViewerId,
        generation: u64,
        reply: SyncSender<ViewerResult>,
    },
    Cancel {
        id: ViewerId,
        generation: u64,
    },
    Run {
        id: ViewerId,
        generation: u64,
        operation: ViewerOperation,
        reply: SyncSender<ViewerResult>,
    },
    Shutdown,
}

pub(super) enum ViewerOperation {
    MoveLines(i32),
    ScrollViewport(i32),
    Page { rows: u16, forward: bool },
    HalfPage { rows: u16, forward: bool },
    LineStart,
    LineEnd { columns: usize },
    Top,
    Bottom,
    ToggleMode,
    Search { query: Vec<u8>, forward: bool },
    RepeatSearch { same_direction: bool },
    Render { terminal: Box<Terminal>, size: Size },
}

pub(super) enum ViewerResult {
    Opened {
        id: ViewerId,
        generation: u64,
        path: PathBuf,
    },
    Closed {
        id: ViewerId,
        generation: u64,
    },
    Done {
        id: ViewerId,
        generation: u64,
        value: Option<bool>,
        wrapped: bool,
        terminal: Option<Terminal>,
    },
    Stale {
        id: ViewerId,
        generation: u64,
        terminal: Option<Terminal>,
    },
    Error {
        id: ViewerId,
        generation: u64,
        message: String,
        terminal: Option<Terminal>,
    },
}

#[derive(Debug)]
pub(crate) enum ViewerUpdate {
    NavigationComplete,
    SearchComplete { found: bool, wrapped: bool },
    RenderComplete,
    Stale,
    NavigationError(String),
    RenderError(String),
}

#[derive(Clone)]
pub(crate) struct ViewerWorkerHandle {
    sender: SyncSender<ViewerCommand>,
    next_id: Arc<AtomicU64>,
}

pub(crate) struct ViewerHandle {
    worker: ViewerWorkerHandle,
    id: ViewerId,
    generation: u64,
    path: PathBuf,
    render_size: Option<Size>,
    closed: bool,
    result_sender: SyncSender<ViewerResult>,
    results: Receiver<ViewerResult>,
}

pub(crate) struct ViewerWorker {
    handle: ViewerWorkerHandle,
    join: Option<JoinHandle<()>>,
}

struct WorkerViewer {
    viewer: Viewer,
    generation: u64,
}

enum Work {
    Run {
        id: ViewerId,
        generation: u64,
        operation: ViewerOperation,
        reply: SyncSender<ViewerResult>,
    },
    Search {
        id: ViewerId,
        generation: u64,
        work: SearchWork,
        reply: SyncSender<ViewerResult>,
    },
}

impl ViewerWorker {
    pub(crate) fn spawn() -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(VIEWER_COMMAND_CAPACITY);
        let join = thread::Builder::new()
            .name("termfold-viewer".into())
            .spawn(move || run(receiver))
            .map_err(io::Error::other)?;
        Ok(Self {
            handle: ViewerWorkerHandle {
                sender,
                next_id: Arc::new(AtomicU64::new(1)),
            },
            join: Some(join),
        })
    }

    pub(crate) fn handle(&self) -> ViewerWorkerHandle {
        self.handle.clone()
    }

    pub(crate) fn shutdown(&mut self) {
        if let Some(join) = self.join.take() {
            let _ = self.handle.sender.send(ViewerCommand::Shutdown);
            let _ = join.join();
        }
    }
}

impl Drop for ViewerWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl ViewerWorkerHandle {
    pub(crate) fn open(&self, path: PathBuf, tab_width: usize) -> io::Result<ViewerHandle> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let generation = 0;
        match self.ask(|reply| ViewerCommand::Open {
            id,
            generation,
            path,
            tab_width,
            reply,
        })? {
            ViewerResult::Opened {
                id: result_id,
                generation: result_generation,
                path,
            } if id == result_id && generation == result_generation => {
                let (result_sender, results) = mpsc::sync_channel(VIEWER_RESULT_CAPACITY);
                Ok(ViewerHandle {
                    worker: self.clone(),
                    id,
                    generation,
                    path,
                    closed: false,
                    render_size: None,
                    result_sender,
                    results,
                })
            }
            ViewerResult::Error { message, .. } => Err(io::Error::other(message)),
            _ => Err(io::Error::other("invalid viewer open result")),
        }
    }

    fn ask<F>(&self, build: F) -> io::Result<ViewerResult>
    where
        F: FnOnce(SyncSender<ViewerResult>) -> ViewerCommand,
    {
        let (reply, receiver) = mpsc::sync_channel(1);
        match self.sender.try_send(build(reply)) {
            Ok(()) => receiver
                .recv()
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "viewer worker stopped")),
            Err(TrySendError::Full(_)) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "viewer worker command queue is full",
            )),
            Err(TrySendError::Disconnected(_)) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "viewer worker stopped",
            )),
        }
    }

    fn ask_blocking<F>(&self, build: F) -> io::Result<ViewerResult>
    where
        F: FnOnce(SyncSender<ViewerResult>) -> ViewerCommand,
    {
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .send(build(reply))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "viewer worker stopped"))?;
        receiver
            .recv()
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "viewer worker stopped"))
    }
}

impl ViewerHandle {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn close(&mut self) -> io::Result<()> {
        if self.closed {
            return Ok(());
        }
        while self.results.try_recv().is_ok() {}
        let generation = self.generation.wrapping_add(1);
        match self.worker.ask_blocking(|reply| ViewerCommand::Close {
            id: self.id,
            generation,
            reply,
        })? {
            ViewerResult::Closed {
                id,
                generation: result_generation,
            } if id == self.id && result_generation == generation => {
                self.generation = generation;
                self.closed = true;
                Ok(())
            }
            ViewerResult::Error { message, .. } => Err(io::Error::other(message)),
            _ => Err(io::Error::other("invalid viewer close result")),
        }
    }

    pub(crate) fn cancel(&mut self) -> io::Result<()> {
        if self.closed {
            return Ok(());
        }
        let generation = self.generation.wrapping_add(1);
        self.worker
            .sender
            .try_send(ViewerCommand::Cancel {
                id: self.id,
                generation,
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "viewer worker command queue is full",
                ),
                TrySendError::Disconnected(_) => {
                    io::Error::new(io::ErrorKind::BrokenPipe, "viewer worker stopped")
                }
            })?;
        self.generation = generation;
        Ok(())
    }

    #[cfg(test)]
    fn run(&mut self, operation: ViewerOperation, invalidate: bool) -> io::Result<ViewerResult> {
        let generation = if invalidate {
            self.generation.wrapping_add(1)
        } else {
            self.generation
        };
        let result = self
            .worker
            .ask(|reply| ViewerCommand::Run {
                id: self.id,
                generation,
                operation,
                reply,
            })
            .and_then(|result| self.check(result, generation));
        if result.is_ok() && invalidate {
            self.generation = generation;
        }
        result
    }

    #[cfg(test)]
    fn check(&self, result: ViewerResult, generation: u64) -> io::Result<ViewerResult> {
        match &result {
            ViewerResult::Done {
                id,
                generation: result_generation,
                ..
            }
            | ViewerResult::Stale {
                id,
                generation: result_generation,
                ..
            }
            | ViewerResult::Error {
                id,
                generation: result_generation,
                ..
            } if *id == self.id && *result_generation == generation => Ok(result),
            _ => Err(io::Error::other("invalid viewer operation result")),
        }
    }

    #[cfg(test)]
    fn boolean(&mut self, operation: ViewerOperation) -> io::Result<(bool, bool)> {
        match self.run(operation, true)? {
            ViewerResult::Done {
                value: Some(value),
                wrapped,
                ..
            } => Ok((value, wrapped)),
            ViewerResult::Error { message, .. } => Err(io::Error::other(message)),
            _ => Err(io::Error::other("invalid viewer operation result")),
        }
    }

    fn dispatch(&mut self, operation: ViewerOperation, invalidate: bool) -> io::Result<()> {
        let generation = if invalidate {
            self.generation.wrapping_add(1)
        } else {
            self.generation
        };
        let command = ViewerCommand::Run {
            id: self.id,
            generation,
            operation,
            reply: self.result_sender.clone(),
        };
        match self.worker.sender.try_send(command) {
            Ok(()) => {
                if invalidate {
                    self.generation = generation;
                }
                Ok(())
            }
            Err(TrySendError::Full(_)) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "viewer worker command queue is full",
            )),
            Err(TrySendError::Disconnected(_)) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "viewer worker stopped",
            )),
        }
    }

    pub(crate) fn request_render(&mut self, size: Size) -> io::Result<()> {
        let terminal = Terminal::new(size).map_err(io::Error::other)?;
        let invalidate = self.render_size.is_some_and(|previous| previous != size);
        let result = self.dispatch(
            ViewerOperation::Render {
                terminal: Box::new(terminal),
                size,
            },
            invalidate,
        );
        if result.is_ok() {
            self.render_size = Some(size);
        }
        result
    }

    pub(crate) fn toggle_mode(&mut self) -> io::Result<()> {
        self.dispatch(ViewerOperation::ToggleMode, true)
    }

    pub(crate) fn poll(&mut self, terminal: &mut Terminal) -> io::Result<Option<ViewerUpdate>> {
        if self.closed {
            while self.results.try_recv().is_ok() {}
            return Ok(None);
        }
        let result = match self.results.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return Ok(None),
            Err(TryRecvError::Disconnected) => {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "viewer worker stopped",
                ));
            }
        };
        match result {
            ViewerResult::Done {
                id,
                generation,
                value: Some(found),
                wrapped,
                terminal: None,
                ..
            } if id == self.id && generation == self.generation => {
                Ok(Some(ViewerUpdate::SearchComplete { found, wrapped }))
            }
            ViewerResult::Done {
                id,
                generation,
                terminal: Some(rendered),
                ..
            } if id == self.id && generation == self.generation => {
                *terminal = rendered;
                Ok(Some(ViewerUpdate::RenderComplete))
            }
            ViewerResult::Done {
                id,
                generation,
                terminal: None,
                ..
            } if id == self.id && generation == self.generation => {
                Ok(Some(ViewerUpdate::NavigationComplete))
            }
            ViewerResult::Error {
                id,
                generation,
                message,
                terminal,
                ..
            } if id == self.id && generation == self.generation => {
                Ok(Some(if terminal.is_some() {
                    ViewerUpdate::RenderError(message)
                } else {
                    ViewerUpdate::NavigationError(message)
                }))
            }
            ViewerResult::Stale {
                id,
                generation,
                terminal,
            } if id == self.id => {
                let _ = generation;
                drop(terminal);
                Ok(Some(ViewerUpdate::Stale))
            }
            ViewerResult::Done { id, .. } | ViewerResult::Error { id, .. } if id == self.id => {
                Ok(Some(ViewerUpdate::Stale))
            }
            _ => Err(io::Error::other("invalid viewer operation result")),
        }
    }

    pub(crate) fn move_lines(&mut self, amount: i32) -> io::Result<()> {
        self.dispatch(ViewerOperation::MoveLines(amount), true)
    }
    pub(crate) fn scroll_viewport(&mut self, amount: i32) -> io::Result<()> {
        self.dispatch(ViewerOperation::ScrollViewport(amount), true)
    }
    pub(crate) fn page(&mut self, rows: u16, forward: bool) -> io::Result<()> {
        self.dispatch(ViewerOperation::Page { rows, forward }, true)
    }
    pub(crate) fn half_page(&mut self, rows: u16, forward: bool) -> io::Result<()> {
        self.dispatch(ViewerOperation::HalfPage { rows, forward }, true)
    }
    pub(crate) fn line_start(&mut self) -> io::Result<()> {
        self.dispatch(ViewerOperation::LineStart, true)
    }
    pub(crate) fn line_end(&mut self, columns: usize) -> io::Result<()> {
        self.dispatch(ViewerOperation::LineEnd { columns }, true)
    }
    pub(crate) fn top(&mut self) -> io::Result<()> {
        self.dispatch(ViewerOperation::Top, true)
    }
    pub(crate) fn bottom(&mut self) -> io::Result<()> {
        self.dispatch(ViewerOperation::Bottom, true)
    }

    pub(crate) fn start_search(&mut self, query: Vec<u8>, forward: bool) -> io::Result<()> {
        self.dispatch(ViewerOperation::Search { query, forward }, true)
    }

    pub(crate) fn start_repeat_search(&mut self, same_direction: bool) -> io::Result<()> {
        self.dispatch(ViewerOperation::RepeatSearch { same_direction }, true)
    }
    #[cfg(test)]
    pub(crate) fn search(&mut self, query: &str, forward: bool) -> io::Result<(bool, bool)> {
        self.boolean(ViewerOperation::Search {
            query: query.as_bytes().to_vec(),
            forward,
        })
    }
    #[cfg(test)]
    pub(crate) fn repeat_search(&mut self, same_direction: bool) -> io::Result<(bool, bool)> {
        self.boolean(ViewerOperation::RepeatSearch { same_direction })
    }
}

fn run(receiver: Receiver<ViewerCommand>) {
    let mut viewers = HashMap::<ViewerId, WorkerViewer>::new();
    let mut queue = VecDeque::new();
    let mut active = None;
    loop {
        let command = if active.is_none() && queue.is_empty() {
            match receiver.recv() {
                Ok(command) => Some(command),
                Err(_) => return,
            }
        } else {
            match receiver.try_recv() {
                Ok(command) => Some(command),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        };
        if let Some(command) = command {
            let mut controls = Vec::new();
            let mut operations = Vec::new();
            split(command, &mut controls, &mut operations);
            for _ in 1..VIEWER_COMMAND_CAPACITY {
                let Ok(command) = receiver.try_recv() else {
                    break;
                };
                split(command, &mut controls, &mut operations);
            }
            for command in controls {
                if control(command, &mut viewers, &mut active, &mut queue) {
                    return;
                }
            }
            for command in operations {
                enqueue(command, &mut viewers, &mut active, &mut queue);
            }
        }
        if active.is_none() {
            active = queue.pop_front();
        }
        if let Some(work) = active.take() {
            if let Some(work) = step(work, &mut viewers) {
                queue.push_back(work);
            }
            active = queue.pop_front();
        }
    }
}

fn split(
    command: ViewerCommand,
    controls: &mut Vec<ViewerCommand>,
    operations: &mut Vec<ViewerCommand>,
) {
    if matches!(command, ViewerCommand::Run { .. }) {
        operations.push(command)
    } else {
        controls.push(command)
    }
}

fn control(
    command: ViewerCommand,
    viewers: &mut HashMap<ViewerId, WorkerViewer>,
    active: &mut Option<Work>,
    queue: &mut VecDeque<Work>,
) -> bool {
    match command {
        ViewerCommand::Open {
            id,
            generation,
            path,
            tab_width,
            reply,
        } => match Viewer::open(path, tab_width) {
            Ok(viewer) => {
                let path = viewer.path().to_owned();
                viewers.insert(id, WorkerViewer { viewer, generation });
                send(
                    &reply,
                    ViewerResult::Opened {
                        id,
                        generation,
                        path,
                    },
                );
            }
            Err(error) => send(
                &reply,
                ViewerResult::Error {
                    id,
                    generation,
                    message: error.to_string(),
                    terminal: None,
                },
            ),
        },
        ViewerCommand::Close {
            id,
            generation,
            reply,
        } => {
            cancel(id, active, queue);
            let result = match viewers.get(&id) {
                Some(viewer) if generation >= viewer.generation => {
                    viewers.remove(&id);
                    ViewerResult::Closed { id, generation }
                }
                _ => ViewerResult::Stale {
                    id,
                    generation,
                    terminal: None,
                },
            };
            send(&reply, result);
        }
        ViewerCommand::Cancel { id, generation } => {
            cancel(id, active, queue);
            if let Some(viewer) = viewers.get_mut(&id)
                && generation >= viewer.generation
            {
                viewer.generation = generation;
            }
        }
        ViewerCommand::Shutdown => {
            cancel_all(active, queue);
            return true;
        }
        ViewerCommand::Run { .. } => {}
    }
    false
}

fn enqueue(
    command: ViewerCommand,
    viewers: &mut HashMap<ViewerId, WorkerViewer>,
    active: &mut Option<Work>,
    queue: &mut VecDeque<Work>,
) {
    let ViewerCommand::Run {
        id,
        generation,
        operation,
        reply,
    } = command
    else {
        return;
    };
    let Some(viewer) = viewers.get_mut(&id) else {
        send(
            &reply,
            ViewerResult::Stale {
                id,
                generation,
                terminal: take_terminal_from(operation),
            },
        );
        return;
    };
    if generation < viewer.generation {
        send(
            &reply,
            ViewerResult::Stale {
                id,
                generation,
                terminal: take_terminal_from(operation),
            },
        );
        return;
    }
    if generation > viewer.generation {
        cancel(id, active, queue);
    }
    if queue.len() + usize::from(active.is_some()) >= VIEWER_COMMAND_CAPACITY {
        send(
            &reply,
            ViewerResult::Error {
                id,
                generation,
                message: "viewer worker work queue is full".into(),
                terminal: take_terminal_from(operation),
            },
        );
        return;
    }
    viewer.generation = generation;
    match operation {
        ViewerOperation::Search { query, forward } => {
            match viewer.viewer.begin_search_work(query, forward) {
                SearchStart::Complete(value) => send(
                    &reply,
                    ViewerResult::Done {
                        id,
                        generation,
                        value: Some(value),
                        wrapped: false,
                        terminal: None,
                    },
                ),
                SearchStart::Work(work) => queue.push_back(Work::Search {
                    id,
                    generation,
                    work,
                    reply,
                }),
            }
        }
        ViewerOperation::RepeatSearch { same_direction } => {
            match viewer.viewer.begin_repeat_search_work(same_direction) {
                Ok(SearchStart::Complete(value)) => send(
                    &reply,
                    ViewerResult::Done {
                        id,
                        generation,
                        value: Some(value),
                        wrapped: false,
                        terminal: None,
                    },
                ),
                Ok(SearchStart::Work(work)) => queue.push_back(Work::Search {
                    id,
                    generation,
                    work,
                    reply,
                }),
                Err(error) => send(
                    &reply,
                    ViewerResult::Error {
                        id,
                        generation,
                        message: error.to_string(),
                        terminal: None,
                    },
                ),
            }
        }
        operation => queue.push_back(Work::Run {
            id,
            generation,
            operation,
            reply,
        }),
    }
}

fn step(work: Work, viewers: &mut HashMap<ViewerId, WorkerViewer>) -> Option<Work> {
    match work {
        Work::Search {
            id,
            generation,
            mut work,
            reply,
        } => {
            let Some(viewer) = viewers.get_mut(&id) else {
                send(
                    &reply,
                    ViewerResult::Stale {
                        id,
                        generation,
                        terminal: None,
                    },
                );
                return None;
            };
            if viewer.generation != generation {
                send(
                    &reply,
                    ViewerResult::Stale {
                        id,
                        generation,
                        terminal: None,
                    },
                );
                return None;
            }
            match viewer.viewer.step_search_work(&mut work) {
                Ok(SearchStep::Continue) => Some(Work::Search {
                    id,
                    generation,
                    work,
                    reply,
                }),
                Ok(SearchStep::Complete(value)) => {
                    send(
                        &reply,
                        ViewerResult::Done {
                            id,
                            generation,
                            value: Some(value),
                            wrapped: viewer.viewer.search_wrapped(),
                            terminal: None,
                        },
                    );
                    None
                }
                Err(error) => {
                    send(
                        &reply,
                        ViewerResult::Error {
                            id,
                            generation,
                            message: error.to_string(),
                            terminal: None,
                        },
                    );
                    None
                }
            }
        }
        Work::Run {
            id,
            generation,
            operation,
            reply,
        } => {
            let Some(viewer) = viewers.get_mut(&id) else {
                send(
                    &reply,
                    ViewerResult::Error {
                        id,
                        generation,
                        message: "viewer is closed".into(),
                        terminal: take_terminal_from(operation),
                    },
                );
                return None;
            };
            if viewer.generation != generation {
                eprintln!(
                    "step stale id={id} command={generation} worker={}",
                    viewer.generation
                );
                send(
                    &reply,
                    ViewerResult::Stale {
                        id,
                        generation,
                        terminal: take_terminal_from(operation),
                    },
                );
                return None;
            }
            match execute(&mut viewer.viewer, operation) {
                Ok((value, terminal)) => send(
                    &reply,
                    ViewerResult::Done {
                        id,
                        generation,
                        value,
                        wrapped: false,
                        terminal,
                    },
                ),
                Err(error) => {
                    let (message, terminal) = *error;
                    send(
                        &reply,
                        ViewerResult::Error {
                            id,
                            generation,
                            message,
                            terminal,
                        },
                    )
                }
            }
            None
        }
    }
}

type ExecResult = Result<(Option<bool>, Option<Terminal>), Box<(String, Option<Terminal>)>>;

fn no_value(result: io::Result<()>) -> ExecResult {
    result
        .map(|_| (None, None))
        .map_err(|error| Box::new((error.to_string(), None)))
}

fn execute(viewer: &mut Viewer, operation: ViewerOperation) -> ExecResult {
    match operation {
        ViewerOperation::MoveLines(amount) => no_value(viewer.move_lines(amount)),
        ViewerOperation::ScrollViewport(amount) => no_value(viewer.scroll_viewport(amount)),
        ViewerOperation::Page { rows, forward } => no_value(viewer.page(rows, forward)),
        ViewerOperation::HalfPage { rows, forward } => no_value(viewer.half_page(rows, forward)),
        ViewerOperation::LineStart => no_value(viewer.line_start()),
        ViewerOperation::LineEnd { columns } => no_value(viewer.line_end(columns)),
        ViewerOperation::Top => {
            viewer.top();
            Ok((None, None))
        }
        ViewerOperation::Bottom => no_value(viewer.bottom()),
        ViewerOperation::ToggleMode => no_value(viewer.toggle_mode()),
        ViewerOperation::Render { mut terminal, size } => {
            match viewer.render(&mut terminal, size) {
                Ok(()) => match apply_hex_highlights(viewer, &mut terminal) {
                    Ok(()) => Ok((None, Some(*terminal))),
                    Err(error) => Err(Box::new((error.to_string(), Some(*terminal)))),
                },
                Err(error) => Err(Box::new((error.to_string(), Some(*terminal)))),
            }
        }
        ViewerOperation::Search { .. } | ViewerOperation::RepeatSearch { .. } => {
            Err(Box::new(("search was not scheduled".into(), None)))
        }
    }
}

fn apply_hex_highlights(viewer: &mut Viewer, terminal: &mut Terminal) -> io::Result<()> {
    if viewer.mode != ViewerMode::Hex {
        return Ok(());
    }
    let Some(frame) = viewer.current_frame() else {
        return Ok(());
    };
    let Some(search) = viewer.search.as_ref() else {
        return Ok(());
    };
    let source_range = frame.source_range.clone();
    let query = search.query.clone();
    let active_offset = search.offset;
    let matches = query.visible_matches(&mut viewer.source, source_range)?;
    if matches.is_empty() {
        return Ok(());
    }
    let active = active_offset..active_offset.saturating_add(query.len() as u64);
    let Some(page) = viewer.current_frame().and_then(|frame| frame.hex.as_ref()) else {
        return Ok(());
    };
    let cursor = terminal.screen().cursor();
    page.for_each_highlight(&matches, Some(&active), |span| {
        let Some(row) = page.rows.get(span.row) else {
            return;
        };
        let index = span.source.saturating_sub(row.offset) as usize;
        let Some(&byte) = row.bytes.get(index) else {
            return;
        };
        let text = if span.width == 2 {
            format!("{byte:02X}")
        } else {
            char::from(if (0x20..=0x7e).contains(&byte) {
                byte
            } else {
                b'.'
            })
            .to_string()
        };
        let style = if span.active { "7;4" } else { "7" };
        terminal.advance(
            format!(
                "\x1b[{};{}H\x1b[{}m{}\x1b[0m",
                span.row + 1,
                span.column + 1,
                style,
                text
            )
            .as_bytes(),
        );
    });
    terminal.advance(format!("\x1b[{};{}H", cursor.row + 1, cursor.column + 1).as_bytes());
    Ok(())
}

fn cancel(id: ViewerId, active: &mut Option<Work>, queue: &mut VecDeque<Work>) {
    if active.as_ref().is_some_and(|work| work_id(work) == id) {
        send_stale(active.take().unwrap());
    }
    let mut keep = VecDeque::new();
    while let Some(work) = queue.pop_front() {
        if work_id(&work) == id {
            send_stale(work)
        } else {
            keep.push_back(work)
        }
    }
    *queue = keep;
}

fn cancel_all(active: &mut Option<Work>, queue: &mut VecDeque<Work>) {
    if let Some(work) = active.take() {
        send_stale(work);
    }
    while let Some(work) = queue.pop_front() {
        send_stale(work);
    }
}

fn work_id(work: &Work) -> ViewerId {
    match work {
        Work::Run { id, .. } | Work::Search { id, .. } => *id,
    }
}

fn send_stale(work: Work) {
    match work {
        Work::Run {
            id,
            generation,
            operation,
            reply,
        } => send(
            &reply,
            ViewerResult::Stale {
                id,
                generation,
                terminal: take_terminal_from(operation),
            },
        ),
        Work::Search {
            id,
            generation,
            reply,
            ..
        } => send(
            &reply,
            ViewerResult::Stale {
                id,
                generation,
                terminal: None,
            },
        ),
    }
}

fn take_terminal_from(operation: ViewerOperation) -> Option<Terminal> {
    match operation {
        ViewerOperation::Render { terminal, .. } => Some(*terminal),
        _ => None,
    }
}

fn send(reply: &SyncSender<ViewerResult>, result: ViewerResult) {
    let _ = reply.send(result);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{session::Size, terminal::Terminal};
    use std::{
        fs, thread,
        time::{Duration, SystemTime},
    };

    fn path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "termfold-worker-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn open(label: &str, bytes: &[u8]) -> (PathBuf, ViewerWorker, ViewerHandle) {
        let path = path(label);
        fs::write(&path, bytes).unwrap();
        let worker = ViewerWorker::spawn().unwrap();
        let viewer = worker.handle().open(path.clone(), 8).unwrap();
        (path, worker, viewer)
    }

    fn wait_update(viewer: &mut ViewerHandle, terminal: &mut Terminal) -> ViewerUpdate {
        for _ in 0..10_000 {
            if let Some(update) = viewer.poll(terminal).unwrap() {
                return update;
            }
            thread::yield_now();
        }
        panic!("viewer worker did not return a result");
    }

    #[test]
    fn open_close_and_shutdown() {
        let (path, mut worker, mut viewer) = open("lifecycle", b"text");
        assert_eq!(viewer.path(), path);
        viewer.close().unwrap();
        worker.shutdown();
        assert!(worker.join.is_none());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn command_channel_is_bounded() {
        let (path, mut worker, viewer) = open("bounded", &vec![b'a'; 2 * 1024 * 1024]);
        let sender = worker.handle().sender;
        let (reply, _) = mpsc::sync_channel(1);
        sender
            .send(ViewerCommand::Run {
                id: viewer.id,
                generation: viewer.generation,
                operation: ViewerOperation::Search {
                    query: vec![b'z'; 256],
                    forward: true,
                },
                reply,
            })
            .unwrap();
        let mut full = false;
        for _ in 0..VIEWER_COMMAND_CAPACITY * 4 {
            let (reply, _) = mpsc::sync_channel(1);
            if matches!(
                sender.try_send(ViewerCommand::Run {
                    id: viewer.id,
                    generation: viewer.generation,
                    operation: ViewerOperation::Top,
                    reply,
                }),
                Err(TrySendError::Full(_))
            ) {
                full = true;
                break;
            }
        }
        assert!(full);
        worker.shutdown();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn stale_generation_is_rejected() {
        let (path, mut worker, viewer) = open("stale", b"text");
        let (advance_reply, advance_result) = mpsc::sync_channel(1);
        worker
            .handle()
            .sender
            .send(ViewerCommand::Run {
                id: viewer.id,
                generation: viewer.generation + 1,
                operation: ViewerOperation::Top,
                reply: advance_reply,
            })
            .unwrap();
        assert!(matches!(
            advance_result.recv().unwrap(),
            ViewerResult::Done { .. }
        ));
        let (reply, result) = mpsc::sync_channel(1);
        worker
            .handle()
            .sender
            .send(ViewerCommand::Run {
                id: viewer.id,
                generation: viewer.generation,
                operation: ViewerOperation::Top,
                reply,
            })
            .unwrap();
        assert!(matches!(result.recv().unwrap(), ViewerResult::Stale { .. }));
        worker.shutdown();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn two_viewers_have_independent_ids() {
        let first_path = path("first");
        let second_path = path("second");
        fs::write(&first_path, b"first").unwrap();
        fs::write(&second_path, b"second").unwrap();
        let mut worker = ViewerWorker::spawn().unwrap();
        let handle = worker.handle();
        let first = handle.open(first_path.clone(), 8).unwrap();
        let second = handle.open(second_path.clone(), 8).unwrap();
        assert_ne!(first.id, second.id);
        assert_ne!(first.path(), second.path());
        worker.shutdown();
        fs::remove_file(first_path).unwrap();
        fs::remove_file(second_path).unwrap();
    }

    #[test]
    fn close_precedes_the_next_search_step() {
        let (path, mut worker, viewer) = open("cooperative", &vec![b'a'; 4 * 1024 * 1024]);
        let sender = worker.handle().sender;
        let (search_reply, search_result) = mpsc::sync_channel(1);
        sender
            .send(ViewerCommand::Run {
                id: viewer.id,
                generation: viewer.generation,
                operation: ViewerOperation::Search {
                    query: vec![b'z'; 256],
                    forward: true,
                },
                reply: search_reply,
            })
            .unwrap();
        let _ = search_result.recv_timeout(Duration::from_millis(1));
        let (close_reply, close_result) = mpsc::sync_channel(1);
        sender
            .send(ViewerCommand::Close {
                id: viewer.id,
                generation: viewer.generation + 1,
                reply: close_reply,
            })
            .unwrap();
        assert!(matches!(
            close_result
                .recv_timeout(Duration::from_millis(500))
                .unwrap(),
            ViewerResult::Closed { .. }
        ));
        worker.shutdown();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn mode_switch_preempts_async_search() {
        let (path, mut worker, mut viewer) = open("mode-switch", &vec![b'a'; 128 * 1024]);
        let mut terminal = Terminal::new(Size {
            columns: 80,
            rows: 8,
        })
        .unwrap();
        viewer.start_search(vec![b'z'; 256], true).unwrap();
        viewer.toggle_mode().unwrap();

        let mut saw_stale = false;
        let mut switched = false;
        for _ in 0..10_000 {
            if let Some(update) = viewer.poll(&mut terminal).unwrap() {
                match update {
                    ViewerUpdate::Stale => saw_stale = true,
                    ViewerUpdate::NavigationComplete => {
                        switched = true;
                        break;
                    }
                    other => panic!("unexpected update: {other:?}"),
                }
            } else {
                thread::yield_now();
            }
        }
        assert!(saw_stale);
        assert!(switched);

        viewer.close().unwrap();
        worker.shutdown();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn hex_search_highlights_ascii_and_exact_bytes_across_rows() {
        let (path, mut worker, mut viewer) =
            open("hex-search", &[b'a', b'b', b'c', 0x00, 0xff, 0x1b, b'X']);
        let size = Size {
            columns: 28,
            rows: 4,
        };
        let mut terminal = Terminal::new(size).unwrap();

        viewer.toggle_mode().unwrap();
        assert!(matches!(
            wait_update(&mut viewer, &mut terminal),
            ViewerUpdate::NavigationComplete
        ));

        assert_eq!(
            viewer
                .boolean(ViewerOperation::Search {
                    query: b"aB".to_vec(),
                    forward: true,
                })
                .unwrap(),
            (true, false)
        );
        viewer.request_render(size).unwrap();
        assert!(matches!(
            wait_update(&mut viewer, &mut terminal),
            ViewerUpdate::RenderComplete
        ));
        assert!(terminal.screen().rows()[0][10].attributes().inverse);
        assert!(terminal.screen().rows()[0][10].attributes().underline);
        assert!(terminal.screen().rows()[0][23].attributes().inverse);

        assert_eq!(
            viewer
                .boolean(ViewerOperation::Search {
                    query: b"hex:00 FF 1B".to_vec(),
                    forward: true,
                })
                .unwrap(),
            (true, false)
        );
        viewer.request_render(size).unwrap();
        assert!(matches!(
            wait_update(&mut viewer, &mut terminal),
            ViewerUpdate::RenderComplete
        ));
        for (row, columns) in [(0, [19].as_slice()), (1, [10, 13].as_slice())] {
            for &column in columns {
                let attributes = terminal.screen().rows()[row][column].attributes();
                assert!(attributes.inverse && attributes.underline);
            }
        }
        for (row, columns) in [(0, [26].as_slice()), (1, [23, 24].as_slice())] {
            for &column in columns {
                let attributes = terminal.screen().rows()[row][column].attributes();
                assert!(attributes.inverse && attributes.underline);
            }
        }

        viewer.close().unwrap();
        worker.shutdown();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn long_search_yields_to_another_viewer() {
        let first_path = path("search-first");
        let second_path = path("search-second");
        fs::write(&first_path, vec![b'a'; 8 * 1024 * 1024]).unwrap();
        fs::write(&second_path, b"second").unwrap();
        let mut worker = ViewerWorker::spawn().unwrap();
        let handle = worker.handle();
        let first = handle.open(first_path.clone(), 8).unwrap();
        let second = handle.open(second_path.clone(), 8).unwrap();
        let sender = handle.sender;

        let (search_reply, _search_result) = mpsc::sync_channel(1);
        sender
            .send(ViewerCommand::Run {
                id: first.id,
                generation: first.generation,
                operation: ViewerOperation::Search {
                    query: vec![b'z'; 256],
                    forward: true,
                },
                reply: search_reply,
            })
            .unwrap();
        let (second_reply, second_result) = mpsc::sync_channel(1);
        sender
            .send(ViewerCommand::Run {
                id: second.id,
                generation: second.generation,
                operation: ViewerOperation::Top,
                reply: second_reply,
            })
            .unwrap();
        assert!(matches!(
            second_result.recv_timeout(Duration::from_secs(2)).unwrap(),
            ViewerResult::Done { id, .. } if id == second.id
        ));

        let (first_close_reply, first_close_result) = mpsc::sync_channel(1);
        sender
            .send(ViewerCommand::Close {
                id: first.id,
                generation: first.generation + 1,
                reply: first_close_reply,
            })
            .unwrap();
        assert!(matches!(
            first_close_result.recv_timeout(Duration::from_secs(2)).unwrap(),
            ViewerResult::Closed { id, .. } if id == first.id
        ));
        let (second_close_reply, second_close_result) = mpsc::sync_channel(1);
        sender
            .send(ViewerCommand::Close {
                id: second.id,
                generation: second.generation + 1,
                reply: second_close_reply,
            })
            .unwrap();
        assert!(matches!(
            second_close_result.recv_timeout(Duration::from_secs(2)).unwrap(),
            ViewerResult::Closed { id, .. } if id == second.id
        ));

        worker.shutdown();
        fs::remove_file(first_path).unwrap();
        fs::remove_file(second_path).unwrap();
    }

    #[test]
    fn sequential_async_pages_return_navigation_then_render() {
        let mut bytes = Vec::new();
        for index in 0..7_000 {
            bytes.extend_from_slice(format!("line {index}\n").as_bytes());
        }
        let (path, mut worker, mut viewer) = open("async-pages", &bytes);
        let size = Size {
            columns: 40,
            rows: 8,
        };
        let mut terminal = Terminal::new(size).unwrap();
        viewer.request_render(size).unwrap();
        assert!(matches!(
            wait_update(&mut viewer, &mut terminal),
            ViewerUpdate::RenderComplete
        ));
        for _ in 0..1_000 {
            viewer.page(size.rows, true).unwrap();
            let update = wait_update(&mut viewer, &mut terminal);
            assert!(
                matches!(update, ViewerUpdate::NavigationComplete),
                "unexpected update: {update:?}"
            );
            viewer.request_render(size).unwrap();
            assert!(matches!(
                wait_update(&mut viewer, &mut terminal),
                ViewerUpdate::RenderComplete
            ));
        }
        for _ in 0..1_000 {
            viewer.page(size.rows, false).unwrap();
            let update = wait_update(&mut viewer, &mut terminal);
            assert!(
                matches!(update, ViewerUpdate::NavigationComplete),
                "unexpected update: {update:?}"
            );
            viewer.request_render(size).unwrap();
            assert!(matches!(
                wait_update(&mut viewer, &mut terminal),
                ViewerUpdate::RenderComplete
            ));
        }
        viewer.close().unwrap();
        worker.shutdown();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn search_reports_a_wrapped_success_to_the_handle() {
        let (path, mut worker, mut viewer) = open("wrapped-search", b"hit---middle---hit");
        let size = Size {
            columns: 40,
            rows: 3,
        };
        let mut terminal = Terminal::new(size).unwrap();

        viewer.bottom().unwrap();
        assert!(matches!(
            wait_update(&mut viewer, &mut terminal),
            ViewerUpdate::NavigationComplete
        ));
        assert_eq!(viewer.search("hit", true).unwrap(), (true, true));
        assert_eq!(viewer.repeat_search(true).unwrap(), (true, false));

        viewer.close().unwrap();
        worker.shutdown();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn close_discards_late_async_results() {
        let (path, mut worker, mut viewer) = open("async-close", &vec![b'a'; 2 * 1024 * 1024]);
        let size = Size {
            columns: 40,
            rows: 8,
        };
        let mut terminal = Terminal::new(size).unwrap();
        viewer.page(size.rows, true).unwrap();
        viewer.close().unwrap();
        assert!(viewer.poll(&mut terminal).unwrap().is_none());
        worker.shutdown();
        fs::remove_file(path).unwrap();
    }
}
