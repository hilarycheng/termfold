use std::{
    collections::VecDeque,
    env, fs,
    io::{self, Read, Write},
    path::PathBuf,
    sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
    thread,
    time::{Duration, SystemTime},
};

use crate::{
    config::Config,
    input::{self, Action, Input, MouseEvent},
    ipc::{self, Message},
    outer::{self, Capabilities},
    profile,
    pty::{self, LaunchContext, PtyChild},
    render::{self, Metrics, StatusLine},
    runtime::{ClientStream, RuntimeDir, SessionSocket},
    session::{CloseResult, Direction, PaneId, Rect, Session, Size},
    terminal::{MAX_SCREEN_CELLS, MouseMode, Terminal},
    viewer::{
        RepeatDirection, SearchDirection, SearchMode,
        worker::{ViewerHandle, ViewerUpdate, ViewerWorker, ViewerWorkerHandle},
    },
};

// Two buffered frames plus one being written and one pending frame remain within
// both the four-frame worst-case payload cap and the normative 4 MiB byte cap.
const CONNECTION_QUEUE_ITEMS: usize = 2;
const EVENT_BATCH_ITEMS: usize = 32;
const LISTENER_POLL_DELAY: Duration = Duration::from_millis(50);
const ENTER_MOUSE: &[u8] = b"\x1b[?1003h\x1b[?1006h";
// ponytail: one event moves 256 cells; raise only if real terminals jump farther.
const MAX_MOUSE_DRAG_CELLS: u16 = 256;

pub(crate) enum ClientEvent {
    Message(Message),
    Closed,
}

pub(crate) enum ServerEvent {
    Client(u64, ClientEvent),
    PaneOutput(PaneId, Vec<u8>),
    ViewerReady,
}

struct Client {
    id: u64,
    control: ClientStream,
    outbound: SyncSender<Message>,
    pending_control: Option<Message>,
    attached: bool,
    size: Option<Size>,
    capabilities: Option<Capabilities>,
    input: Input,
    status: Option<String>,
    scroll_offset: Option<usize>,
    help_offset: Option<usize>,
    search_query: Option<String>,
    viewer_prompt: Option<ViewerPrompt>,
    mouse_capture: Option<MouseCapture>,
}

#[derive(Clone, Copy)]
enum MouseCapture {
    Click,
    Border(u16, u16),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ViewerIntent {
    Scroll(i32),
    Horizontal(i32),
    Viewport(i32),
    Page { rows: u16, forward: bool },
    HalfPage { rows: u16, forward: bool },
    LineStart,
    LineEnd { columns: usize },
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ViewerNavigation {
    intent: ViewerIntent,
    client_id: u64,
}

#[derive(Debug, Default)]
struct ViewerGate {
    generation: u64,
    in_flight: bool,
    current_intent: Option<ViewerIntent>,
    replacement: Option<ViewerNavigation>,
}

enum ViewerGateDecision {
    Dispatch(ViewerNavigation),
    Replaced,
    Dropped,
}

impl ViewerGate {
    fn accept(&mut self, navigation: ViewerNavigation) -> ViewerGateDecision {
        if !self.in_flight {
            return ViewerGateDecision::Dispatch(navigation);
        }
        if self.current_intent == Some(navigation.intent) {
            return ViewerGateDecision::Dropped;
        }
        self.current_intent = Some(navigation.intent);
        self.replacement = Some(navigation);
        ViewerGateDecision::Replaced
    }

    fn begin(&mut self, intent: ViewerIntent) {
        self.generation = self.generation.wrapping_add(1);
        self.in_flight = true;
        self.current_intent = Some(intent);
    }

    fn finish(&mut self) -> Option<ViewerNavigation> {
        if !self.in_flight {
            return None;
        }
        self.in_flight = false;
        self.current_intent = None;
        self.replacement.take()
    }

    fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.in_flight = false;
        self.current_intent = None;
        self.replacement = None;
    }
}

impl ViewerIntent {
    fn dispatch(self, viewer: &mut ViewerHandle, size: Size) -> io::Result<()> {
        match self {
            Self::Scroll(amount) => viewer.move_lines(amount),
            Self::Horizontal(amount) => viewer.move_horizontal(amount),
            Self::Viewport(amount) => viewer.scroll_viewport(amount),
            Self::Page { rows, forward } => viewer.page_render(rows, forward, false, size),
            Self::HalfPage { rows, forward } => viewer.page_render(rows, forward, true, size),
            Self::LineStart => viewer.line_start(),
            Self::LineEnd { columns } => viewer.line_end(columns),
            Self::Top => viewer.top(),
            Self::Bottom => viewer.bottom(),
        }
    }
}

struct PaneProcess {
    id: PaneId,
    child: Option<PtyChild>,
    viewer: Option<ViewerHandle>,
    viewer_gate: ViewerGate,
    pending_viewer_search: Option<PendingViewerSearch>,
    pending_viewer_client: Option<u64>,
    terminal: Terminal,
    pending_input: PendingInput,
    working_directory: PathBuf,
}

enum PendingViewerSearch {
    New {
        client_id: u64,
        query: Vec<u8>,
        mode: SearchMode,
        direction: SearchDirection,
    },
    Repeat {
        client_id: u64,
        relation: RepeatDirection,
    },
}

enum ViewerCommand {
    Navigate(ViewerNavigation),
    ToggleMode { client_id: u64 },
    Search { pending: PendingViewerSearch },
}

#[derive(Clone)]
struct ViewerPrompt {
    directory: PathBuf,
    query: Vec<u8>,
    filter: Vec<u8>,
    selected: usize,
}

impl PaneProcess {
    fn clear_viewer_state(&mut self) {
        self.viewer_gate.cancel();
        self.pending_viewer_search = None;
        self.pending_viewer_client = None;
    }

    fn cancel_viewer(&mut self) {
        self.clear_viewer_state();
        if let Some(viewer) = self.viewer.as_mut() {
            let _ = viewer.cancel();
        }
    }

    fn cancel_viewer_search(&mut self) {
        if self.pending_viewer_search.take().is_some() {
            self.pending_viewer_client = None;
            if let Some(viewer) = self.viewer.as_mut() {
                let _ = viewer.cancel();
            }
        }
    }

    fn resize(&mut self, size: Size) -> io::Result<()> {
        if let Some(child) = &self.child {
            child.resize(size)?;
        }
        let changed = self.terminal.screen().size() != size;
        self.terminal.resize(size).map_err(io::Error::other)?;
        if changed && self.viewer.is_some() {
            self.clear_viewer_state();
            if let Some(viewer) = self.viewer.as_mut() {
                viewer.cancel()?;
                viewer.request_render(size)?;
            }
        }
        Ok(())
    }
}

struct PendingInput {
    chunks: VecDeque<Vec<u8>>,
    offset: usize,
}

impl PendingInput {
    fn new() -> Self {
        Self {
            chunks: VecDeque::new(),
            offset: 0,
        }
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    fn push(&mut self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            self.chunks.push_back(bytes);
        }
    }

    fn flush(&mut self, writer: &mut impl Write) -> io::Result<()> {
        while let Some(chunk) = self.chunks.front() {
            match writer.write(&chunk[self.offset..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "PTY write returned zero",
                    ));
                }
                Ok(written) => {
                    self.offset += written;
                    if self.offset == chunk.len() {
                        self.chunks.pop_front();
                        self.offset = 0;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

struct PendingBroadcast {
    frames: Vec<(u64, Vec<u8>)>,
}

struct InputContext<'a> {
    session: &'a mut Session,
    panes: &'a mut Vec<PaneProcess>,
    size: &'a mut Size,
    launch: &'a LaunchContext,
    scrollback_limit: usize,
    viewer_tab_width: u8,
    mouse: bool,
    status_line: StatusLine<'a>,
    full_dirty: &'a mut bool,
    events: &'a SyncSender<ServerEvent>,
    viewer_worker: &'a ViewerWorkerHandle,
}

pub fn run(
    runtime: RuntimeDir,
    name: String,
    initial_size: Size,
    config: Config,
    profile: Option<String>,
) -> Result<(), String> {
    let terminfo_root = runtime.materialize_terminfo()?;
    let mut context = LaunchContext::capture(
        terminfo_root,
        config.inner_term.clone(),
        &config.windows_shell,
    )
    .map_err(|error| format!("cannot capture shell environment: {error}"))?;
    context.set_session_name(&name);
    let _launch_plan = profile
        .as_deref()
        .map(|profile_name| {
            let configured = config
                .profiles
                .get(profile_name)
                .ok_or_else(|| format!("profile '{profile_name}' does not exist"))?;
            profile::build_launch_plan(name.clone(), configured, &context, pane_area(initial_size))
        })
        .transpose()?;
    let mut session = Session::new(name);
    let first_pane = session
        .active_pane()
        .expect("new session always contains one pane");
    let content_size = pane_area(initial_size);
    let first_child = PtyChild::spawn(&context, content_size)
        .map_err(|error| format!("cannot start shell: {error}"))?;
    let first_reader = first_child
        .output_reader()
        .map_err(|error| format!("cannot read shell output: {error}"))?;
    let (event_sender, event_receiver) = mpsc::sync_channel(CONNECTION_QUEUE_ITEMS);
    let mut panes = vec![PaneProcess {
        id: first_pane,
        child: Some(first_child),
        viewer: None,
        viewer_gate: ViewerGate::default(),
        pending_viewer_search: None,
        pending_viewer_client: None,
        terminal: Terminal::with_scrollback(content_size, usize::from(config.scrollback_lines))
            .map_err(|error| format!("cannot create terminal screen: {error}"))?,
        pending_input: PendingInput::new(),
        working_directory: context.working_directory().to_owned(),
    }];
    spawn_pane_reader(first_pane, first_reader, event_sender.clone())?;
    let mut viewer_worker = ViewerWorker::spawn(event_sender.clone())
        .map_err(|error| format!("cannot start viewer worker: {error}"))?;
    let viewer_worker_handle = viewer_worker.handle();
    let mut clients = Vec::<Client>::new();
    let mut next_client_id = 1_u64;
    let mut authoritative_size = initial_size;
    let mut pending_broadcast: Option<PendingBroadcast> = None;
    let clock_has_seconds = config.date_format.contains("%S") || config.time_format.contains("%S");
    let uses_metrics = ["{cpu_usage}", "{memory_usage}", "{cpu_temp}"]
        .iter()
        .any(|placeholder| config.status_format.contains(placeholder));
    let mut metrics = Metrics::default();
    metrics.refresh(config.cpu_temperature_path.as_deref());
    let mut sampled_metrics = None;
    let mut rendered_status = None;
    let mut full_dirty = false;
    let mut content_dirty = false;
    let mut terminate = false;
    let socket = runtime.bind(session.name())?;
    socket
        .set_nonblocking(true)
        .map_err(|error| format!("cannot configure session listener: {error}"))?;

    while !terminate {
        accept_clients(
            &socket,
            &mut clients,
            &mut next_client_id,
            config.prefix,
            config.mouse,
            &event_sender,
        );
        flush_client_controls(&mut clients);

        let events = collect_server_events(&event_receiver, event_timeout(&clients));
        for event in events {
            match event {
                ServerEvent::Client(client_id, event) => match event {
                    ClientEvent::Closed => remove_client(&mut clients, client_id),
                    ClientEvent::Message(message) => {
                        let mut input = InputContext {
                            session: &mut session,
                            panes: &mut panes,
                            size: &mut authoritative_size,
                            launch: &context,
                            scrollback_limit: usize::from(config.scrollback_lines),
                            viewer_tab_width: config.viewer_tab_width,
                            mouse: config.mouse,
                            status_line: status_line(&config, &metrics),
                            full_dirty: &mut full_dirty,
                            events: &event_sender,
                            viewer_worker: &viewer_worker_handle,
                        };
                        if handle_message(&mut clients, client_id, message, &mut input) {
                            terminate = true;
                            break;
                        }
                    }
                },
                ServerEvent::PaneOutput(pane_id, output) => {
                    advance_pane(
                        &mut panes,
                        &mut clients,
                        &session,
                        pane_id,
                        &output,
                        &mut content_dirty,
                        &mut full_dirty,
                    );
                }
                ServerEvent::ViewerReady => viewer_worker_handle.clear_ready(),
            }
        }

        let pending = clients
            .iter_mut()
            .filter_map(|client| {
                let actions = client.input.flush_pending_mouse();
                (!actions.is_empty()).then_some((client.id, actions))
            })
            .collect::<Vec<_>>();
        for (client_id, actions) in pending {
            let mut input = InputContext {
                session: &mut session,
                panes: &mut panes,
                size: &mut authoritative_size,
                launch: &context,
                scrollback_limit: usize::from(config.scrollback_lines),
                viewer_tab_width: config.viewer_tab_width,
                mouse: config.mouse,
                status_line: status_line(&config, &metrics),
                full_dirty: &mut full_dirty,
                events: &event_sender,
                viewer_worker: &viewer_worker_handle,
            };
            if handle_actions(&mut clients, client_id, actions, &mut input) {
                terminate = true;
                break;
            }
        }

        apply_viewer_results(
            &mut clients,
            &session,
            &mut panes,
            authoritative_size,
            &mut full_dirty,
        );

        for pane in &mut panes {
            if let Some(child) = pane.child.as_mut()
                && pane.pending_input.flush(child.master()).is_err()
            {
                terminate = true;
                break;
            }
        }
        flush_client_controls(&mut clients);
        flush_broadcast(&mut pending_broadcast, &clients);

        if pending_broadcast.is_none() && clients.iter().any(|client| client.attached) {
            let now = SystemTime::now();
            let metric_key = uses_metrics
                .then(|| render::clock_key(now, true) / u64::from(config.status_refresh_seconds));
            if metric_key.is_some() && metric_key != sampled_metrics {
                metrics.refresh(config.cpu_temperature_path.as_deref());
                sampled_metrics = metric_key;
            }
            let key = (render::clock_key(now, clock_has_seconds), metric_key);
            let status_line = status_line(&config, &metrics);
            let pane_screens = panes
                .iter()
                .map(|pane| (pane.id, &pane.terminal))
                .collect::<Vec<_>>();
            let targets = clients
                .iter()
                .filter_map(|client| {
                    client.capabilities.map(|capabilities| {
                        (
                            client.id,
                            capabilities,
                            client.status.as_deref(),
                            client.scroll_offset,
                            client.help_offset,
                        )
                    })
                })
                .collect::<Vec<_>>();
            let frames = if full_dirty {
                full_dirty = false;
                content_dirty = false;
                rendered_status = Some(key);
                let frames = targets
                    .iter()
                    .map(|(id, capabilities, message, scroll_offset, help_offset)| {
                        (
                            *id,
                            render::full(
                                &session,
                                &pane_screens,
                                authoritative_size,
                                status_line,
                                *message,
                                render_view(*scroll_offset, *help_offset, config.prefix),
                                *capabilities,
                            ),
                        )
                    })
                    .collect();
                Some(frames)
            } else if content_dirty {
                content_dirty = false;
                let frames = targets
                    .iter()
                    .map(|(id, capabilities, message, scroll_offset, help_offset)| {
                        (
                            *id,
                            if scroll_offset.is_some() || help_offset.is_some() {
                                render::full(
                                    &session,
                                    &pane_screens,
                                    authoritative_size,
                                    status_line,
                                    *message,
                                    render_view(*scroll_offset, *help_offset, config.prefix),
                                    *capabilities,
                                )
                            } else {
                                render::changes(
                                    &session,
                                    &pane_screens,
                                    authoritative_size,
                                    *capabilities,
                                )
                            },
                        )
                    })
                    .collect();
                Some(frames)
            } else if rendered_status != Some(key) {
                rendered_status = Some(key);
                Some(
                    targets
                        .iter()
                        .map(|(id, capabilities, message, _, _)| {
                            (
                                *id,
                                render::status(
                                    &session,
                                    authoritative_size,
                                    status_line,
                                    *message,
                                    true,
                                    *capabilities,
                                ),
                            )
                        })
                        .collect(),
                )
            } else {
                None
            };
            if let Some(frames) = frames {
                for pane in &mut panes {
                    pane.terminal.clear_damage();
                }
                pending_broadcast = Some(PendingBroadcast { frames });
                flush_broadcast(&mut pending_broadcast, &clients);
            }
        }

        let mut exited = Vec::new();
        for pane in &mut panes {
            if let Some(child) = pane.child.as_mut() {
                match child.try_wait() {
                    Ok(Some(_)) => exited.push(pane.id),
                    Ok(None) => {}
                    Err(_) => terminate = true,
                }
            }
        }
        for pane_id in exited {
            panes.retain(|pane| pane.id != pane_id);
            match session.close_pane(pane_id, pane_area(authoritative_size)) {
                Ok(CloseResult::SessionEmpty) => terminate = true,
                Ok(CloseResult::PaneClosed | CloseResult::TabClosed) => {
                    if resize_all(&session, &mut panes, authoritative_size, authoritative_size)
                        .is_err()
                    {
                        terminate = true;
                    } else {
                        full_dirty = true;
                    }
                }
                Err(_) => terminate = true,
            }
        }
    }

    for client in &clients {
        let _ = client.outbound.try_send(Message::Terminating);
    }
    let mut children = panes
        .iter_mut()
        .filter_map(|pane| pane.child.as_mut())
        .collect::<Vec<_>>();
    let result = pty::terminate_all(&mut children)
        .map_err(|error| format!("cannot terminate session children: {error}"));
    viewer_worker.shutdown();
    result
}

fn accept_clients(
    listener: &SessionSocket,
    clients: &mut Vec<Client>,
    next_client_id: &mut u64,
    prefix: u8,
    mouse: bool,
    events: &SyncSender<ServerEvent>,
) {
    loop {
        let stream = match listener.accept() {
            Ok(stream) => stream,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(_) => break,
        };
        let Ok(reader) = stream.try_clone() else {
            continue;
        };
        let Ok(writer) = stream.try_clone() else {
            continue;
        };
        let (outbound, message_receiver) = mpsc::sync_channel(CONNECTION_QUEUE_ITEMS);
        let id = *next_client_id;
        let events = events.clone();
        thread::spawn(move || read_client(id, reader, events));
        thread::spawn(move || write_client(writer, message_receiver));
        clients.push(Client {
            id,
            control: stream,
            outbound,
            pending_control: None,
            attached: false,
            size: None,
            capabilities: None,
            input: Input::new(prefix, mouse),
            status: None,
            scroll_offset: None,
            help_offset: None,
            search_query: None,
            viewer_prompt: None,
            mouse_capture: None,
        });
        *next_client_id = next_client_id.saturating_add(1);
    }
}

fn read_client(client_id: u64, mut stream: ClientStream, sender: SyncSender<ServerEvent>) {
    loop {
        match ipc::read_message(&mut stream) {
            Ok(Some(message)) => {
                if sender
                    .send(ServerEvent::Client(
                        client_id,
                        ClientEvent::Message(message),
                    ))
                    .is_err()
                {
                    break;
                }
            }
            Ok(None) | Err(_) => {
                let _ = sender.send(ServerEvent::Client(client_id, ClientEvent::Closed));
                break;
            }
        }
    }
}

fn write_client(mut stream: ClientStream, receiver: Receiver<Message>) {
    while let Ok(message) = receiver.recv() {
        if ipc::write_message(&mut stream, &message).is_err() {
            break;
        }
    }
}

fn collect_server_events(receiver: &Receiver<ServerEvent>, timeout: Duration) -> Vec<ServerEvent> {
    let mut events = Vec::new();
    match receiver.recv_timeout(timeout) {
        Ok(event) => events.push(event),
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {}
    }
    events.extend(
        receiver
            .try_iter()
            .take(EVENT_BATCH_ITEMS.saturating_sub(1)),
    );
    events
}

fn event_timeout(clients: &[Client]) -> Duration {
    clients
        .iter()
        .filter_map(|client| client.input.pending_timeout())
        .fold(LISTENER_POLL_DELAY, Duration::min)
}

fn spawn_pane_reader(
    pane_id: PaneId,
    reader: pty::PtyReader,
    sender: SyncSender<ServerEvent>,
) -> Result<(), String> {
    thread::Builder::new()
        .name(format!("termfold-pty-{pane_id:?}"))
        .spawn(move || read_pane(pane_id, reader, sender))
        .map(|_| ())
        .map_err(|error| format!("cannot start PTY reader: {error}"))
}

fn read_pane(pane_id: PaneId, mut reader: pty::PtyReader, sender: SyncSender<ServerEvent>) {
    loop {
        let mut buffer = [0; 8192];
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(length) => {
                if sender
                    .send(ServerEvent::PaneOutput(pane_id, buffer[..length].to_vec()))
                    .is_err()
                {
                    break;
                }
            }
            Err(error) if pty::is_eof_error(&error) => break,
            Err(_) => break,
        }
    }
}

fn advance_pane(
    panes: &mut [PaneProcess],
    clients: &mut [Client],
    session: &Session,
    pane_id: PaneId,
    output: &[u8],
    content_dirty: &mut bool,
    full_dirty: &mut bool,
) {
    let Some(pane) = panes.iter_mut().find(|pane| pane.id == pane_id) else {
        return;
    };
    let previous_epoch = pane.terminal.scrollback_epoch();
    let previous_maximum = pane.terminal.max_scroll_offset();
    pane.terminal.advance(output);
    if let Some(directory) = pane.terminal.take_working_directory() {
        pane.working_directory = directory;
    }
    let maximum = pane.terminal.max_scroll_offset();
    if maximum < previous_maximum && session.active_pane() == Some(pane.id) {
        for client in clients {
            if client.scroll_offset.take().is_some() {
                client.search_query = None;
                client.status = None;
                *full_dirty = true;
            }
        }
    } else if session.active_pane() == Some(pane.id) {
        let added = pane
            .terminal
            .scrollback_epoch()
            .saturating_sub(previous_epoch) as usize;
        for client in clients {
            if let Some(offset) = &mut client.scroll_offset {
                *offset = offset.saturating_add(added).min(maximum);
                set_scroll_status(client, maximum);
            }
        }
    }
    pane.pending_input.push(pane.terminal.take_responses());
    *content_dirty = true;
}

fn handle_message(
    clients: &mut Vec<Client>,
    client_id: u64,
    message: Message,
    input: &mut InputContext<'_>,
) -> bool {
    match message {
        Message::Attach {
            columns,
            rows,
            profile,
            color,
        } => {
            let Some(capabilities) = outer::from_wire(profile, color) else {
                queue_control(
                    clients,
                    client_id,
                    Message::Error("invalid outer-terminal profile".into()),
                );
                return false;
            };
            if !capabilities.cursor_addressing {
                queue_control(
                    clients,
                    client_id,
                    Message::Error(format!(
                        "outer terminal '{}' lacks cursor addressing",
                        capabilities.profile.name()
                    )),
                );
                return false;
            }
            let size = Size { columns, rows };
            if resize_all(input.session, input.panes, size, *input.size).is_err() {
                queue_control(
                    clients,
                    client_id,
                    Message::Error("cannot resize session PTYs".into()),
                );
                return false;
            }
            *input.size = size;
            if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                client.attached = true;
                client.size = Some(size);
                client.capabilities = Some(capabilities);
            }
            queue_control(clients, client_id, Message::Attached);
            if input.mouse && capabilities.mouse {
                queue_control(clients, client_id, Message::Screen(ENTER_MOUSE.to_vec()));
            }
            *input.full_dirty = true;
        }
        Message::Resize { columns, rows } => {
            let Some(client) = clients.iter_mut().find(|client| client.id == client_id) else {
                return false;
            };
            if !client.attached {
                remove_client(clients, client_id);
                return false;
            }
            let size = Size { columns, rows };
            client.size = Some(size);
            if resize_all(input.session, input.panes, size, *input.size).is_err() {
                remove_client(clients, client_id);
            } else {
                *input.size = size;
                *input.full_dirty = true;
            }
        }
        Message::Input(bytes) => {
            let Some(client) = clients.iter_mut().find(|client| client.id == client_id) else {
                return false;
            };
            if !client.attached {
                remove_client(clients, client_id);
                return false;
            }
            if let Some(size) = client.size {
                if resize_all(input.session, input.panes, size, *input.size).is_err() {
                    remove_client(clients, client_id);
                    return false;
                }
                *input.full_dirty |= size != *input.size;
                *input.size = size;
            }
            let actions = client.input.advance(&bytes);
            return handle_actions(clients, client_id, actions, input);
        }
        Message::Detach => remove_client(clients, client_id),
        Message::StatusRequest => {
            let attached_clients = clients.iter().filter(|client| client.attached).count() as u32;
            queue_control(
                clients,
                client_id,
                Message::Status {
                    pid: std::process::id(),
                    attached_clients,
                },
            );
        }
        Message::Kill => return true,
        Message::View { path } => match open_viewer(input, &path) {
            Ok(()) => queue_control(clients, client_id, Message::ViewerOpened),
            Err(error) => queue_control(clients, client_id, Message::Error(error)),
        },
        Message::Attached
        | Message::Screen(_)
        | Message::Error(_)
        | Message::Status { .. }
        | Message::ViewerOpened
        | Message::Terminating => remove_client(clients, client_id),
    }
    false
}

fn handle_actions(
    clients: &mut Vec<Client>,
    client_id: u64,
    actions: Vec<Action>,
    input: &mut InputContext<'_>,
) -> bool {
    for action in actions {
        let detach = matches!(action, Action::Detach);
        if handle_action(clients, client_id, action, input) {
            return true;
        }
        if detach {
            break;
        }
    }
    false
}

fn handle_action(
    clients: &mut Vec<Client>,
    client_id: u64,
    action: Action,
    input: &mut InputContext<'_>,
) -> bool {
    let size = *input.size;
    let content_size = pane_area(size);
    if let Some(intent) = viewer_intent(&action, input, content_size) {
        if let Err(error) = queue_viewer_navigation(clients, client_id, input, intent) {
            set_status(
                clients,
                client_id,
                Some(format!("viewer navigation failed: {error}")),
                input.full_dirty,
            );
        }
        *input.full_dirty = true;
        return false;
    }
    let result: Result<(), String> = match action {
        Action::Forward(bytes) => {
            set_status(clients, client_id, None, input.full_dirty);
            if let Some(active) = input.session.active_pane()
                && let Some(pane) = input.panes.iter_mut().find(|pane| pane.id == active)
                && pane.child.is_some()
            {
                pane.pending_input.push(bytes);
            }
            return false;
        }
        Action::CreateTab => input
            .session
            .create_tab()
            .map_err(|error| error.to_string())
            .and_then(|pane| {
                add_pane(
                    input.launch,
                    input.session,
                    input.panes,
                    pane,
                    content_size,
                    input.scrollback_limit,
                    input.events,
                )
                .inspect_err(|_| {
                    let _ = input.session.close_pane(pane, content_size);
                })
            }),
        Action::NextTab => input.session.next_tab().map_err(|error| error.to_string()),
        Action::PreviousTab => input
            .session
            .previous_tab()
            .map_err(|error| error.to_string()),
        Action::SelectTab(index) => {
            if input.session.select_tab(index) {
                Ok(())
            } else {
                set_status(
                    clients,
                    client_id,
                    Some("tab does not exist".into()),
                    input.full_dirty,
                );
                return false;
            }
        }
        Action::Split(split) => input
            .session
            .split_active(split, content_size)
            .map_err(|error| error.to_string())
            .and_then(|pane| {
                add_pane(
                    input.launch,
                    input.session,
                    input.panes,
                    pane,
                    content_size,
                    input.scrollback_limit,
                    input.events,
                )
                .inspect_err(|_| {
                    let _ = input.session.close_pane(pane, content_size);
                })
            }),
        Action::Focus(direction) => input
            .session
            .focus(direction, content_size)
            .map(|_| ())
            .map_err(|error| error.to_string()),
        Action::Resize(direction) => input
            .session
            .resize(direction, content_size)
            .map_err(|error| error.to_string()),
        Action::ClosePane => {
            let Some(pane_id) = input.session.active_pane() else {
                return true;
            };
            let is_viewer = input
                .panes
                .iter()
                .find(|pane| pane.id == pane_id)
                .is_some_and(|pane| pane.viewer.is_some());
            let close = input.session.close_active_pane(content_size);
            if let Some(index) = input.panes.iter().position(|pane| pane.id == pane_id) {
                let mut pane = input.panes.swap_remove(index);
                if pane.viewer.is_some() {
                    pane.clear_viewer_state();
                    if let Some(viewer) = pane.viewer.as_mut() {
                        let _ = viewer.close();
                    }
                }
                if let Some(child) = pane.child.as_mut() {
                    let _ = pty::terminate_all(&mut [child]);
                }
            }
            if is_viewer
                && let Some(client) = clients.iter_mut().find(|client| client.id == client_id)
            {
                client.viewer_prompt = None;
            }
            match close {
                Ok(CloseResult::SessionEmpty) => return true,
                Ok(CloseResult::PaneClosed | CloseResult::TabClosed) => Ok(()),
                Err(error) => Err(error.to_string()),
            }
        }
        Action::Detach => {
            remove_client(clients, client_id);
            return false;
        }
        Action::HelpView => {
            if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                client.scroll_offset = None;
                client.help_offset = Some(0);
                set_help_status(
                    client,
                    0,
                    render::help_max_offset(size.rows, client.input.prefix()),
                );
            }
            *input.full_dirty = true;
            return false;
        }
        Action::HelpScroll(amount) => {
            if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                let maximum = render::help_max_offset(size.rows, client.input.prefix());
                if let Some(offset) = client.help_offset {
                    let lines = if matches!(amount, i32::MAX | i32::MIN) {
                        usize::from(size.rows.saturating_sub(1))
                    } else {
                        amount.unsigned_abs() as usize
                    };
                    let offset = if amount > 0 {
                        offset.saturating_add(lines).min(maximum)
                    } else {
                        offset.saturating_sub(lines)
                    };
                    client.help_offset = Some(offset);
                    set_help_status(client, offset, maximum);
                }
            }
            *input.full_dirty = true;
            return false;
        }
        Action::ExitHelpView => {
            if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                client.help_offset = None;
                client.status = None;
            }
            *input.full_dirty = true;
            return false;
        }
        Action::ScrollView => {
            if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                client.help_offset = None;
                client.scroll_offset = Some(0);
                set_scroll_status(client, active_scrollback_maximum(input));
            }
            *input.full_dirty = true;
            return false;
        }
        Action::Scroll(amount) => {
            let maximum = active_scrollback_maximum(input);
            if let Some(client) = clients.iter_mut().find(|client| client.id == client_id)
                && let Some(offset) = client.scroll_offset
            {
                let lines = match amount {
                    i32::MAX => usize::from(content_size.rows),
                    i32::MIN => usize::from(content_size.rows),
                    _ => amount.unsigned_abs() as usize,
                };
                client.scroll_offset = Some(if amount > 0 {
                    offset.saturating_add(lines).min(maximum)
                } else {
                    offset.saturating_sub(lines)
                });
                set_scroll_status(client, maximum);
            }
            *input.full_dirty = true;
            return false;
        }
        Action::ScrollTop => {
            let maximum = active_scrollback_maximum(input);
            if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                client.scroll_offset = Some(maximum);
                set_scroll_status(client, maximum);
            }
            *input.full_dirty = true;
            return false;
        }
        Action::ScrollBottom => {
            let maximum = active_scrollback_maximum(input);
            if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                client.scroll_offset = Some(0);
                set_scroll_status(client, maximum);
            }
            *input.full_dirty = true;
            return false;
        }
        Action::Search(query) => {
            search_scrollback(clients, client_id, input, query, true);
            *input.full_dirty = true;
            return false;
        }
        Action::SearchNext(older) => {
            let query = clients
                .iter()
                .find(|client| client.id == client_id)
                .and_then(|client| client.search_query.clone());
            if let Some(query) = query {
                search_scrollback(clients, client_id, input, query, older);
            } else {
                set_status(
                    clients,
                    client_id,
                    Some("no previous search".into()),
                    input.full_dirty,
                );
            }
            *input.full_dirty = true;
            return false;
        }
        Action::SearchCancelled => {
            let maximum = active_scrollback_maximum(input);
            if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                set_scroll_status(client, maximum);
            }
            *input.full_dirty = true;
            return false;
        }
        Action::ExitScrollView => {
            if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                client.scroll_offset = None;
                client.search_query = None;
            }
            set_status(clients, client_id, None, input.full_dirty);
            return false;
        }
        Action::ViewPrompt(return_viewer) => {
            if return_viewer {
                cancel_active_viewer(input);
            }
            let directory = input
                .session
                .active_pane()
                .and_then(|active| input.panes.iter().find(|pane| pane.id == active))
                .map(|pane| pane.working_directory.clone())
                .ok_or_else(|| "active pane does not exist".to_string());
            match directory {
                Ok(directory) => {
                    if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                        client.scroll_offset = None;
                        client.help_offset = None;
                        client.search_query = None;
                        client.viewer_prompt = Some(ViewerPrompt {
                            directory,
                            query: Vec::new(),
                            filter: Vec::new(),
                            selected: 0,
                        });
                        client.status = client.viewer_prompt.as_ref().map(viewer_prompt_status);
                    }
                }
                Err(error) => set_status(clients, client_id, Some(error), input.full_dirty),
            }
            *input.full_dirty = true;
            return false;
        }
        Action::ViewQuery(query) => {
            let status = clients
                .iter_mut()
                .find(|client| client.id == client_id)
                .and_then(|client| client.viewer_prompt.as_mut())
                .map(|prompt| {
                    prompt.query = query.clone();
                    prompt.filter = query.clone();
                    prompt.selected = 0;
                    viewer_prompt_status(prompt)
                });
            if let Some(status) = status {
                set_status(clients, client_id, Some(status), input.full_dirty);
            }
            *input.full_dirty = true;
            return false;
        }
        Action::ViewDirectory { query, separator } => {
            let result = open_prompt_directory(clients, client_id, query, separator);
            if let Err(error) = result {
                set_status(clients, client_id, Some(error), input.full_dirty);
            } else if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                client.status = client.viewer_prompt.as_ref().map(viewer_prompt_status);
            }
            *input.full_dirty = true;
            return false;
        }
        Action::ViewParent => {
            let status = clients
                .iter_mut()
                .find(|client| client.id == client_id)
                .and_then(|client| client.viewer_prompt.as_mut())
                .map(|prompt| {
                    if let Some(parent) = prompt.directory.parent().map(PathBuf::from)
                        && parent != prompt.directory
                    {
                        prompt.directory = parent;
                        prompt.query.clear();
                        prompt.filter.clear();
                        prompt.selected = 0;
                        "".to_owned()
                    } else {
                        "already at filesystem root".to_owned()
                    }
                });
            if let Some(status) = status {
                if status.is_empty() {
                    if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                        client.input.set_view_prompt(Vec::new());
                        client.status = client.viewer_prompt.as_ref().map(viewer_prompt_status);
                    }
                } else {
                    set_status(clients, client_id, Some(status), input.full_dirty);
                }
            }
            *input.full_dirty = true;
            return false;
        }
        Action::ViewComplete(_) => {
            let completion = clients
                .iter_mut()
                .find(|client| client.id == client_id)
                .and_then(|client| client.viewer_prompt.as_mut())
                .map(complete_viewer_entry);
            match completion {
                Some(Ok(query)) => {
                    if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                        client.input.set_view_prompt(query);
                        client.status = client.viewer_prompt.as_ref().map(viewer_prompt_status);
                    }
                }
                Some(Err(error)) => set_status(clients, client_id, Some(error), input.full_dirty),
                None => {}
            }
            *input.full_dirty = true;
            return false;
        }
        Action::ViewSelect(amount) => {
            let selection = clients
                .iter_mut()
                .find(|client| client.id == client_id)
                .and_then(|client| client.viewer_prompt.as_mut())
                .map(|prompt| select_viewer_entry(prompt, amount));
            match selection {
                Some(Ok(query)) => {
                    if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                        client.input.set_view_prompt(query);
                        client.status = client.viewer_prompt.as_ref().map(viewer_prompt_status);
                    }
                }
                Some(Err(error)) => set_status(clients, client_id, Some(error), input.full_dirty),
                None => {}
            }
            *input.full_dirty = true;
            return false;
        }
        Action::OpenViewer(query) => {
            let result = open_prompt_viewer(clients, client_id, input, query);
            if let Err(error) = result {
                set_status(clients, client_id, Some(error), input.full_dirty);
            } else {
                set_viewer_status(clients, client_id, input);
            }
            *input.full_dirty = true;
            return false;
        }
        Action::ViewerScroll(_)
        | Action::ViewerHorizontal(_)
        | Action::ViewerViewport(_)
        | Action::ViewerPage(_)
        | Action::ViewerHalfPage(_)
        | Action::ViewerLineStart
        | Action::ViewerLineEnd
        | Action::ViewerTop
        | Action::ViewerBottom => unreachable!("viewer navigation was handled above"),
        Action::ViewerToggleMode => {
            let result = dispatch_viewer_command(input, ViewerCommand::ToggleMode { client_id });
            match result {
                Ok(()) => set_viewer_status(clients, client_id, input),
                Err(error) => set_status(
                    clients,
                    client_id,
                    Some(format!("viewer mode switch failed: {error}")),
                    input.full_dirty,
                ),
            }
            *input.full_dirty = true;
            return false;
        }
        Action::ViewCancelled => {
            if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                client.viewer_prompt = None;
                client.status = None;
            }
            *input.full_dirty = true;
            return false;
        }
        Action::ViewerSearchPrompt(mode, direction) => {
            cancel_active_viewer(input);
            set_status(
                clients,
                client_id,
                Some(input::viewer_search_status(mode, direction, &[])),
                input.full_dirty,
            );
            *input.full_dirty = true;
            return false;
        }
        Action::ViewerSearchQuery(query, mode, direction) => {
            set_status(
                clients,
                client_id,
                Some(input::viewer_search_status(mode, direction, &query)),
                input.full_dirty,
            );
            *input.full_dirty = true;
            return false;
        }
        Action::ViewerSearch(query, mode, direction) => {
            let status = input::viewer_search_status(mode, direction, &query);
            match queue_viewer_search(
                input,
                PendingViewerSearch::New {
                    client_id,
                    query,
                    mode,
                    direction,
                },
            ) {
                Ok(()) => set_status(clients, client_id, Some(status), input.full_dirty),
                Err(error) => set_status(
                    clients,
                    client_id,
                    Some(format!("viewer search failed: {error}")),
                    input.full_dirty,
                ),
            }
            *input.full_dirty = true;
            return false;
        }
        Action::ViewerSearchNext(_, relation) => {
            if let Err(error) = queue_viewer_search(
                input,
                PendingViewerSearch::Repeat {
                    client_id,
                    relation,
                },
            ) {
                set_status(
                    clients,
                    client_id,
                    Some(format!("viewer search failed: {error}")),
                    input.full_dirty,
                );
            }
            *input.full_dirty = true;
            return false;
        }
        Action::ViewerSearchCancelled => {
            cancel_active_viewer(input);
            set_viewer_status(clients, client_id, input);
            *input.full_dirty = true;
            return false;
        }
        Action::ClearScrollback => {
            let result = input
                .session
                .active_pane()
                .and_then(|active| input.panes.iter_mut().find(|pane| pane.id == active))
                .ok_or_else(|| "active pane does not exist".to_string())
                .map(|pane| pane.terminal.clear_scrollback());
            for client in clients.iter_mut() {
                client.scroll_offset = None;
            }
            set_status(
                clients,
                client_id,
                Some(match result {
                    Ok(()) => "scrollback cleared".into(),
                    Err(error) => error,
                }),
                input.full_dirty,
            );
            return false;
        }
        Action::SaveScrollback(filename) => {
            let result = input
                .session
                .active_pane()
                .and_then(|active| input.panes.iter().find(|pane| pane.id == active))
                .ok_or_else(|| "active pane does not exist".to_string())
                .and_then(|pane| {
                    std::fs::write(filename, pane.terminal.scrollback_text())
                        .map_err(|error| format!("cannot save scrollback: {error}"))
                });
            set_status(
                clients,
                client_id,
                Some(match result {
                    Ok(()) => "scrollback saved".into(),
                    Err(error) => error,
                }),
                input.full_dirty,
            );
            return false;
        }
        Action::Mouse(event) => {
            handle_mouse(clients, client_id, event, input);
            return false;
        }
        Action::Status(message) => {
            set_status(clients, client_id, Some(message), input.full_dirty);
            return false;
        }
    };

    match result {
        Ok(()) => {
            *input.full_dirty = true;
            if resize_all(input.session, input.panes, size, size).is_err() {
                set_status(
                    clients,
                    client_id,
                    Some("cannot resize session PTYs".into()),
                    input.full_dirty,
                );
            } else {
                set_status(clients, client_id, None, input.full_dirty);
            }
        }
        Err(error) => set_status(clients, client_id, Some(error), input.full_dirty),
    }
    false
}

fn active_scrollback_maximum(input: &InputContext<'_>) -> usize {
    input
        .session
        .active_pane()
        .and_then(|active| input.panes.iter().find(|pane| pane.id == active))
        .map_or(0, |pane| pane.terminal.max_scroll_offset())
}

fn active_viewer_size(input: &InputContext<'_>) -> Option<Size> {
    let active = input.session.active_pane()?;
    input
        .panes
        .iter()
        .find(|pane| pane.id == active && pane.viewer.is_some())
        .map(|pane| pane.terminal.screen().size())
}

fn cancel_active_viewer(input: &mut InputContext<'_>) {
    if let Some(active) = input.session.active_pane()
        && let Some(pane) = input.panes.iter_mut().find(|pane| pane.id == active)
    {
        pane.cancel_viewer();
    }
}

fn viewer_intent(
    action: &Action,
    input: &InputContext<'_>,
    content_size: Size,
) -> Option<ViewerIntent> {
    let viewer_size = active_viewer_size(input);
    Some(match action {
        Action::ViewerScroll(amount) => ViewerIntent::Scroll(*amount),
        Action::ViewerHorizontal(amount) => ViewerIntent::Horizontal(*amount),
        Action::ViewerViewport(amount) => ViewerIntent::Viewport(*amount),
        Action::ViewerPage(forward) => ViewerIntent::Page {
            rows: viewer_size.unwrap_or(content_size).rows,
            forward: *forward,
        },
        Action::ViewerHalfPage(forward) => ViewerIntent::HalfPage {
            rows: viewer_size.unwrap_or(content_size).rows,
            forward: *forward,
        },
        Action::ViewerLineStart => ViewerIntent::LineStart,
        Action::ViewerLineEnd => ViewerIntent::LineEnd {
            columns: viewer_size.map_or(1, |size| usize::from(size.columns)),
        },
        Action::ViewerTop => ViewerIntent::Top,
        Action::ViewerBottom => ViewerIntent::Bottom,
        _ => return None,
    })
}

fn queue_viewer_navigation(
    clients: &mut [Client],
    client_id: u64,
    input: &mut InputContext<'_>,
    intent: ViewerIntent,
) -> Result<(), String> {
    dispatch_viewer_command(
        input,
        ViewerCommand::Navigate(ViewerNavigation { intent, client_id }),
    )?;
    set_viewer_status(clients, client_id, input);
    Ok(())
}

fn queue_viewer_search(
    input: &mut InputContext<'_>,
    pending: PendingViewerSearch,
) -> Result<(), String> {
    dispatch_viewer_command(input, ViewerCommand::Search { pending })
}

fn dispatch_navigation(pane: &mut PaneProcess, navigation: ViewerNavigation) -> Result<(), String> {
    let size = pane.terminal.screen().size();
    let result = pane
        .viewer
        .as_mut()
        .ok_or_else(|| "active pane is not a viewer".to_owned())
        .and_then(|viewer| {
            navigation
                .intent
                .dispatch(viewer, size)
                .map_err(|error| error.to_string())
        });
    match result {
        Ok(()) => {
            pane.viewer_gate.begin(navigation.intent);
            pane.pending_viewer_client = Some(navigation.client_id);
            Ok(())
        }
        Err(error) => {
            pane.viewer_gate.cancel();
            Err(error)
        }
    }
}

fn dispatch_viewer_command(
    input: &mut InputContext<'_>,
    command: ViewerCommand,
) -> Result<(), String> {
    let active = input
        .session
        .active_pane()
        .ok_or_else(|| "active pane does not exist".to_owned())?;
    let pane = input
        .panes
        .iter_mut()
        .find(|pane| pane.id == active)
        .ok_or_else(|| "active pane does not exist".to_owned())?;
    match command {
        ViewerCommand::Navigate(navigation) => {
            pane.cancel_viewer_search();
            if let ViewerGateDecision::Dispatch(navigation) = pane.viewer_gate.accept(navigation) {
                dispatch_navigation(pane, navigation)?;
            }
        }
        ViewerCommand::ToggleMode { client_id } => {
            pane.cancel_viewer();
            pane.viewer
                .as_mut()
                .ok_or_else(|| "active pane is not a viewer".to_owned())?
                .toggle_mode()
                .map_err(|error| error.to_string())?;
            pane.pending_viewer_client = Some(client_id);
        }
        ViewerCommand::Search { pending } => {
            pane.cancel_viewer();
            let client_id = match &pending {
                PendingViewerSearch::New { client_id, .. }
                | PendingViewerSearch::Repeat { client_id, .. } => *client_id,
            };
            let viewer = pane
                .viewer
                .as_mut()
                .ok_or_else(|| "active pane is not a viewer".to_owned())?;
            match &pending {
                PendingViewerSearch::New {
                    query,
                    mode,
                    direction,
                    ..
                } => viewer
                    .start_search_mode(query.clone(), *mode, *direction)
                    .map_err(|error| error.to_string())?,
                PendingViewerSearch::Repeat { relation, .. } => viewer
                    .start_repeat_search(*relation)
                    .map_err(|error| error.to_string())?,
            }
            pane.pending_viewer_search = Some(pending);
            pane.pending_viewer_client = Some(client_id);
        }
    }
    Ok(())
}

fn new_viewer_search_state(
    pending: &PendingViewerSearch,
) -> Option<(u64, SearchMode, SearchDirection)> {
    match pending {
        PendingViewerSearch::New {
            client_id,
            mode,
            direction,
            ..
        } => Some((*client_id, *mode, *direction)),
        PendingViewerSearch::Repeat { .. } => None,
    }
}

fn apply_viewer_results(
    clients: &mut [Client],
    session: &Session,
    panes: &mut [PaneProcess],
    size: Size,
    full_dirty: &mut bool,
) {
    let rects = session.pane_rects(pane_area(size));
    for pane in panes {
        let Some(rect) = rects
            .iter()
            .find(|(id, _)| *id == pane.id)
            .map(|(_, rect)| rect)
        else {
            continue;
        };
        let viewer_size = Size {
            columns: rect.width,
            rows: rect.height,
        };
        while let Some(viewer) = pane.viewer.as_mut() {
            let update = match viewer.poll(&mut pane.terminal) {
                Ok(update) => update,
                Err(error) => {
                    report_viewer_error(
                        clients,
                        pane,
                        full_dirty,
                        format!("viewer worker failed: {error}"),
                    );
                    break;
                }
            };
            let Some(update) = update else { break };
            match update {
                ViewerUpdate::SearchComplete {
                    found,
                    wrapped,
                    mode,
                    direction,
                } => {
                    let Some(pending) = pane.pending_viewer_search.take() else {
                        continue;
                    };
                    let client_id = match &pending {
                        PendingViewerSearch::New { client_id, .. }
                        | PendingViewerSearch::Repeat { client_id, .. } => *client_id,
                    };
                    if !found {
                        pane.pending_viewer_client = None;
                        let status = match &pending {
                            PendingViewerSearch::New { query, .. } => {
                                format!(
                                    "no match: {}{}",
                                    search_marker(mode, direction),
                                    String::from_utf8_lossy(query)
                                )
                            }
                            PendingViewerSearch::Repeat { .. } => {
                                "no previous viewer search".to_owned()
                            }
                        };
                        set_status(clients, client_id, Some(status), full_dirty);
                        continue;
                    }
                    if let Some((record_client_id, mode, direction)) =
                        new_viewer_search_state(&pending)
                    {
                        if let Some(client) = clients
                            .iter_mut()
                            .find(|client| client.id == record_client_id)
                        {
                            client.input.record_viewer_search(mode, direction);
                        }
                    }
                    if wrapped {
                        set_status(
                            clients,
                            client_id,
                            Some(viewer_wrap_status(direction).into()),
                            full_dirty,
                        );
                    } else {
                        let status = viewer_status_message(clients, pane, client_id);
                        set_status(clients, client_id, status, full_dirty);
                    }
                    pane.pending_viewer_client = Some(client_id);
                    let result = pane
                        .viewer
                        .as_mut()
                        .ok_or_else(|| io::Error::other("active pane is not a viewer"))
                        .and_then(|viewer| viewer.request_render(viewer_size));
                    if let Err(error) = result {
                        report_viewer_error(
                            clients,
                            pane,
                            full_dirty,
                            format!("viewer render failed: {error}"),
                        );
                    }
                }
                ViewerUpdate::SearchError(message) => {
                    pane.pending_viewer_search = None;
                    report_viewer_error(clients, pane, full_dirty, message);
                }
                ViewerUpdate::NavigationComplete => {
                    let result = pane
                        .viewer
                        .as_mut()
                        .ok_or_else(|| io::Error::other("active pane is not a viewer"))
                        .and_then(|viewer| viewer.request_render(viewer_size));
                    if let Err(error) = result {
                        pane.viewer_gate.cancel();
                        report_viewer_error(
                            clients,
                            pane,
                            full_dirty,
                            format!("viewer render failed: {error}"),
                        );
                    }
                }
                ViewerUpdate::RenderComplete => {
                    let replacement = pane.viewer_gate.finish();
                    pane.pending_viewer_client = None;
                    *full_dirty = true;
                    if let Some(navigation) = replacement {
                        pane.pending_viewer_client = Some(navigation.client_id);
                        if let Err(error) = dispatch_navigation(pane, navigation) {
                            report_viewer_error(
                                clients,
                                pane,
                                full_dirty,
                                format!("viewer navigation failed: {error}"),
                            );
                        }
                    }
                }
                ViewerUpdate::Stale => {}
                ViewerUpdate::NavigationError(error) => {
                    pane.pending_viewer_search = None;
                    pane.viewer_gate.cancel();
                    report_viewer_error(
                        clients,
                        pane,
                        full_dirty,
                        format!("viewer navigation failed: {error}"),
                    );
                }
                ViewerUpdate::RenderError(error) => {
                    pane.viewer_gate.cancel();
                    report_viewer_error(
                        clients,
                        pane,
                        full_dirty,
                        format!("viewer render failed: {error}"),
                    );
                }
            }
        }
    }
}

fn report_viewer_error(
    clients: &mut [Client],
    pane: &mut PaneProcess,
    full_dirty: &mut bool,
    message: String,
) {
    *full_dirty = true;
    if let Some(client_id) = pane.pending_viewer_client.take() {
        set_status(clients, client_id, Some(message), full_dirty);
    }
}

fn set_viewer_status(clients: &mut [Client], client_id: u64, input: &mut InputContext<'_>) {
    let status = input
        .session
        .active_pane()
        .and_then(|active| input.panes.iter().find(|pane| pane.id == active))
        .and_then(|pane| viewer_status_message(clients, pane, client_id));
    set_status(clients, client_id, status, input.full_dirty);
}

fn search_marker(mode: SearchMode, direction: SearchDirection) -> &'static str {
    match (mode, direction) {
        (SearchMode::Matching, SearchDirection::Forward) => "/",
        (SearchMode::Matching, SearchDirection::Reverse) => "?",
        (SearchMode::NonMatching, SearchDirection::Forward) => "]",
        (SearchMode::NonMatching, SearchDirection::Reverse) => "[",
    }
}

fn viewer_wrap_status(direction: SearchDirection) -> &'static str {
    match direction {
        SearchDirection::Forward => "search hit BOTTOM, continuing at TOP",
        SearchDirection::Reverse => "search hit TOP, continuing at BOTTOM",
    }
}

fn viewer_status_message(clients: &[Client], pane: &PaneProcess, client_id: u64) -> Option<String> {
    let prefix_byte = clients
        .iter()
        .find(|client| client.id == client_id)
        .map_or(2, |client| client.input.prefix());
    let prefix = format!("Ctrl-{}", char::from(prefix_byte + b'a' - 1));
    pane.viewer.as_ref().map(|viewer| {
        format!(
            "VIEW {} | H text/hex Home/End line gg/G file / ? search n/N repeat {prefix} x close",
            viewer.path().display()
        )
    })
}

#[derive(Clone)]
struct ViewerEntry {
    name: String,
    directory: bool,
}

fn viewer_entries(directory: &std::path::Path, query: &[u8]) -> Vec<ViewerEntry> {
    const MAX_PROMPT_ENTRIES: usize = 256;
    let query = String::from_utf8_lossy(query);
    let mut entries = fs::read_dir(directory)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let directory = entry.metadata().ok()?.is_dir();
            name.starts_with(query.as_ref())
                .then_some(ViewerEntry { directory, name })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    entries.truncate(MAX_PROMPT_ENTRIES);
    entries
}

fn viewer_prompt_status(prompt: &ViewerPrompt) -> String {
    let entries = viewer_entries(&prompt.directory, &prompt.filter);
    let selected = entries
        .iter()
        .position(|entry| entry.name.as_bytes() == prompt.query.as_slice())
        .unwrap_or_else(|| prompt.selected % entries.len().max(1));
    let mut status = format!(
        "VIEW {} > {} |",
        prompt.directory.display(),
        String::from_utf8_lossy(&prompt.query)
    );
    let first = selected
        .saturating_sub(7)
        .min(entries.len().saturating_sub(8));
    for (index, entry) in entries.iter().enumerate().skip(first).take(8) {
        if index == selected {
            status.push_str(" [");
        } else {
            status.push(' ');
        }
        status.push_str(&entry.name);
        if entry.directory {
            status.push('/');
        }
        if index == selected {
            status.push(']');
        }
    }
    status.push_str(
        " | arrows/C-n/p select Right/Enter open Left parent Backspace edit/parent Tab complete Esc cancel",
    );
    status
}

fn select_viewer_entry(prompt: &mut ViewerPrompt, amount: i32) -> Result<Vec<u8>, String> {
    let entries = viewer_entries(&prompt.directory, &prompt.filter);
    if entries.is_empty() {
        return Err("no matching entries".to_owned());
    }
    let current = entries
        .iter()
        .position(|entry| entry.name.as_bytes() == prompt.query.as_slice())
        .unwrap_or_else(|| prompt.selected % entries.len());
    let shift = amount.unsigned_abs() as usize % entries.len();
    let selected = if amount >= 0 {
        (current + shift) % entries.len()
    } else {
        (current + entries.len() - shift) % entries.len()
    };
    prompt.selected = selected;
    prompt.query = entries[selected].name.as_bytes().to_vec();
    Ok(prompt.query.clone())
}

fn complete_viewer_entry(prompt: &mut ViewerPrompt) -> Result<Vec<u8>, String> {
    let entries = viewer_entries(&prompt.directory, &prompt.filter);
    if entries.is_empty() {
        return Err("no matching entries".to_owned());
    }
    let index = entries
        .iter()
        .position(|entry| entry.name.as_bytes() == prompt.query.as_slice())
        .map_or(prompt.selected % entries.len(), |index| {
            (index + 1) % entries.len()
        });
    prompt.selected = index;
    prompt.query = entries[index].name.as_bytes().to_vec();
    Ok(prompt.query.clone())
}

fn selected_viewer_entry(
    prompt: &ViewerPrompt,
    entries: &[ViewerEntry],
    query: &[u8],
    directory_only: bool,
) -> Option<usize> {
    if entries.is_empty() {
        return None;
    }
    entries
        .iter()
        .position(|entry| (!directory_only || entry.directory) && entry.name.as_bytes() == query)
        .or_else(|| {
            let selected = prompt.selected % entries.len();
            (!directory_only || entries[selected].directory).then_some(selected)
        })
}

fn open_prompt_directory(
    clients: &mut [Client],
    client_id: u64,
    query: Vec<u8>,
    separator: u8,
) -> Result<(), String> {
    let prompt = clients
        .iter()
        .find(|client| client.id == client_id)
        .and_then(|client| client.viewer_prompt.clone())
        .ok_or_else(|| "viewer prompt is not active".to_owned())?;
    let directory = resolve_prompt_directory(&prompt, &query, separator, prompt_home_directory())?;
    if !directory.is_dir() {
        return Err(format!("cannot open directory {}", directory.display()));
    }
    if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
        client.input.set_view_prompt(Vec::new());
        client.viewer_prompt = Some(ViewerPrompt {
            directory,
            query: Vec::new(),
            filter: Vec::new(),
            selected: 0,
        });
        client.status = client.viewer_prompt.as_ref().map(viewer_prompt_status);
    }
    Ok(())
}

fn resolve_prompt_directory(
    prompt: &ViewerPrompt,
    query: &[u8],
    separator: u8,
    home: Option<PathBuf>,
) -> Result<PathBuf, String> {
    #[cfg(windows)]
    if query.len() == 2 && query[1] == b':' && !query[0].is_ascii_alphabetic() {
        return Err("invalid drive root".to_owned());
    }

    let absolute = if query == b"~" && separator == b'/' {
        Some(home.ok_or_else(|| "home directory is unavailable".to_owned())?)
    } else if query.len() == 1 && query[0] == separator {
        Some(
            prompt_root(&prompt.directory, separator)
                .ok_or_else(|| "absolute path must start with the native separator".to_owned())?,
        )
    } else {
        prompt_drive_root(query)
    };
    if let Some(directory) = absolute {
        return Ok(directory);
    }
    if query.is_empty() {
        return Err("absolute path must start with the native separator".to_owned());
    }
    let entries = viewer_entries(&prompt.directory, &prompt.filter);
    let selected = selected_viewer_entry(prompt, &entries, query, true)
        .ok_or_else(|| "select a matching directory before the path separator".to_owned())?;
    Ok(prompt.directory.join(&entries[selected].name))
}

fn prompt_home_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    let home = env::var_os("USERPROFILE")
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let mut home = PathBuf::from(env::var_os("HOMEDRIVE")?);
            home.push(env::var_os("HOMEPATH")?);
            Some(home.into_os_string())
        });

    #[cfg(not(windows))]
    let home = env::var_os("HOME").filter(|value| !value.is_empty());

    home.map(PathBuf::from).filter(|path| path.is_absolute())
}

#[cfg(windows)]
fn prompt_drive_root(query: &[u8]) -> Option<PathBuf> {
    (query.len() == 2 && query[0].is_ascii_alphabetic() && query[1] == b':')
        .then(|| PathBuf::from(format!("{}:\\", query[0] as char)))
}

#[cfg(not(windows))]
fn prompt_drive_root(_: &[u8]) -> Option<PathBuf> {
    None
}

#[cfg(unix)]
fn prompt_root(_: &std::path::Path, separator: u8) -> Option<PathBuf> {
    (separator == b'/').then(|| PathBuf::from("/"))
}

#[cfg(not(unix))]
fn prompt_root(directory: &std::path::Path, separator: u8) -> Option<PathBuf> {
    if !matches!(separator, b'/' | b'\\') {
        return None;
    }
    let mut components = directory.components();
    if !matches!(
        (components.next(), components.next()),
        (
            Some(std::path::Component::Prefix(_)),
            Some(std::path::Component::RootDir)
        )
    ) {
        return None;
    }
    Some(directory.components().take(2).collect())
}

fn open_viewer(input: &mut InputContext<'_>, requested: &str) -> Result<(), String> {
    let active = input
        .session
        .active_pane()
        .ok_or_else(|| "active pane does not exist".to_owned())?;
    let directory = input
        .panes
        .iter()
        .find(|pane| pane.id == active)
        .map(|pane| pane.working_directory.clone())
        .ok_or_else(|| "active pane does not exist".to_owned())?;
    let requested = PathBuf::from(requested);
    let path = if requested.is_absolute() {
        requested
    } else {
        directory.join(requested)
    };
    open_viewer_at(input, path)
}

fn open_viewer_at(input: &mut InputContext<'_>, path: PathBuf) -> Result<(), String> {
    let mut viewer = input
        .viewer_worker
        .open(path.clone(), usize::from(input.viewer_tab_width))
        .map_err(|error| format!("cannot open viewer file {}: {error}", path.display()))?;
    let content_size = pane_area(*input.size);
    let pane = input
        .session
        .create_tab()
        .map_err(|error| error.to_string())?;
    let Some(pane_size) = input
        .session
        .pane_rects(content_size)
        .into_iter()
        .find(|(id, _)| *id == pane)
        .map(|(_, rect)| Size {
            columns: rect.width,
            rows: rect.height,
        })
    else {
        let _ = viewer.close();
        let _ = input.session.close_pane(pane, content_size);
        return Err("viewer tab has no size".into());
    };
    let terminal = match Terminal::new(pane_size) {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = viewer.close();
            let _ = input.session.close_pane(pane, content_size);
            return Err(error.to_string());
        }
    };
    let working_directory = path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    input.panes.push(PaneProcess {
        id: pane,
        child: None,
        viewer: Some(viewer),
        viewer_gate: ViewerGate::default(),
        pending_viewer_search: None,
        pending_viewer_client: None,
        terminal,
        pending_input: PendingInput::new(),
        working_directory,
    });
    let render_result = input
        .panes
        .iter_mut()
        .find(|candidate| candidate.id == pane)
        .and_then(|candidate| candidate.viewer.as_mut())
        .ok_or_else(|| "viewer pane disappeared".to_owned())
        .and_then(|viewer| {
            viewer
                .request_render(pane_size)
                .map_err(|error| error.to_string())
        });
    if let Err(error) = render_result {
        if let Some(viewer) = input
            .panes
            .iter_mut()
            .find(|candidate| candidate.id == pane)
            .and_then(|candidate| candidate.viewer.as_mut())
        {
            let _ = viewer.close();
        }
        input.panes.retain(|candidate| candidate.id != pane);
        let _ = input.session.close_pane(pane, content_size);
        let _ = resize_all(input.session, input.panes, *input.size, *input.size);
        return Err(format!("cannot render viewer: {error}"));
    }
    if let Err(error) = resize_all(input.session, input.panes, *input.size, *input.size) {
        if let Some(viewer) = input
            .panes
            .iter_mut()
            .find(|candidate| candidate.id == pane)
            .and_then(|candidate| candidate.viewer.as_mut())
        {
            let _ = viewer.close();
        }
        input.panes.retain(|candidate| candidate.id != pane);
        let _ = input.session.close_pane(pane, content_size);
        let _ = resize_all(input.session, input.panes, *input.size, *input.size);
        return Err(format!("cannot resize viewer tab: {error}"));
    }
    Ok(())
}

fn open_prompt_viewer(
    clients: &mut [Client],
    client_id: u64,
    input: &mut InputContext<'_>,
    query: Vec<u8>,
) -> Result<(), String> {
    let prompt = clients
        .iter()
        .find(|client| client.id == client_id)
        .and_then(|client| client.viewer_prompt.clone())
        .ok_or_else(|| "viewer prompt is not active".to_owned())?;
    let entries = viewer_entries(&prompt.directory, &prompt.filter);
    let selected = selected_viewer_entry(&prompt, &entries, &query, false)
        .ok_or_else(|| "no matching file or directory".to_owned())?;
    let target = prompt.directory.join(&entries[selected].name);
    if entries[selected].directory {
        if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
            client.input.set_view_prompt(Vec::new());
            client.viewer_prompt = Some(ViewerPrompt {
                directory: target,
                query: Vec::new(),
                filter: Vec::new(),
                selected: 0,
            });
            client.status = client.viewer_prompt.as_ref().map(viewer_prompt_status);
        }
        return Ok(());
    }
    open_viewer_at(input, target)?;
    if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
        client.viewer_prompt = None;
        client.input.enter_viewer();
    }
    Ok(())
}

fn render_view(
    scroll_offset: Option<usize>,
    help_offset: Option<usize>,
    prefix: u8,
) -> render::View {
    if let Some(offset) = help_offset {
        render::View::Help { offset, prefix }
    } else {
        scroll_offset.map_or(render::View::Live, render::View::Scroll)
    }
}

fn search_scrollback(
    clients: &mut [Client],
    client_id: u64,
    input: &InputContext<'_>,
    query: String,
    older: bool,
) {
    let (maximum, matches) = input
        .session
        .active_pane()
        .and_then(|active| input.panes.iter().find(|pane| pane.id == active))
        .map_or((0, Vec::new()), |pane| {
            (
                pane.terminal.max_scroll_offset(),
                // ponytail: this scan is bounded by the configured 10,000-line history.
                pane.terminal.search_scrollback(&query),
            )
        });
    let Some(client) = clients.iter_mut().find(|client| client.id == client_id) else {
        return;
    };
    let current = client.scroll_offset.unwrap_or(0);
    let next = if older {
        matches
            .iter()
            .copied()
            .find(|offset| *offset > current)
            .or_else(|| matches.first().copied())
    } else {
        matches
            .iter()
            .rev()
            .copied()
            .find(|offset| *offset < current)
            .or_else(|| matches.last().copied())
    };
    client.search_query = Some(query.clone());
    if let Some(offset) = next {
        client.scroll_offset = Some(offset);
        set_scroll_status(client, maximum);
    } else {
        client.status = Some(format!("no match: /{query}"));
    }
}

fn set_scroll_status(client: &mut Client, maximum: usize) {
    let offset = client.scroll_offset.unwrap_or(0);
    let percent = offset
        .saturating_mul(100)
        .checked_div(maximum)
        .map_or(100, |position| 100_usize.saturating_sub(position));
    let width = client.size.map_or(0, |size| usize::from(size.columns));
    client.status = Some(scroll_status_message(
        width,
        percent,
        client.search_query.as_deref(),
    ));
}

fn scroll_status_message(width: usize, percent: usize, query: Option<&str>) -> String {
    let mut message = if width >= 110 {
        format!(
            "SCROLL {percent}% | Up/k up Down/j down PgUp/PgDn page g/G ends / search n/N match q/Esc exit"
        )
    } else if width >= 70 {
        format!("SCROLL {percent}% | Up/Down j/k PgUp/PgDn / n/N q/Esc")
    } else {
        format!("SCROLL {percent}% | j/k Pg / Esc")
    };
    if let Some(query) = query
        && width >= 70
    {
        message.push_str("  /");
        message.push_str(query);
    }
    message
}

fn set_help_status(client: &mut Client, offset: usize, maximum: usize) {
    let page_rows = client
        .size
        .map_or(1, |size| usize::from(size.rows.saturating_sub(1)).max(1));
    let page = offset / page_rows + 1;
    let pages = maximum.div_ceil(page_rows) + 1;
    let width = client.size.map_or(0, |size| usize::from(size.columns));
    client.status = Some(if width >= 75 {
        format!("HELP {page}/{pages} | Up/Down or j/k line PgUp/PgDn page q/Esc exit")
    } else {
        format!("HELP {page}/{pages} | j/k Pg q/Esc")
    });
}

fn handle_mouse(
    clients: &mut [Client],
    client_id: u64,
    event: MouseEvent,
    input: &mut InputContext<'_>,
) {
    if clients
        .iter()
        .find(|client| client.id == client_id)
        .is_some_and(|client| client.help_offset.is_some())
    {
        return;
    }
    let size = *input.size;
    if event.x >= size.columns || event.y >= size.rows {
        return;
    }
    let motion = event.code & 32 != 0;
    let wheel = event.code & 64 != 0;
    let left_press = !event.release && !motion && !wheel && event.code & 3 == 0;

    if event.y == size.rows.saturating_sub(1) {
        if left_press {
            let message = clients
                .iter()
                .find(|client| client.id == client_id)
                .and_then(|client| client.status.as_deref());
            if let Some(tab) =
                render::status_tab_at(input.session, size, input.status_line, message, event.x)
                && input.session.select_tab(tab)
            {
                if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                    client.scroll_offset = None;
                }
                if resize_all(input.session, input.panes, size, size).is_ok() {
                    *input.full_dirty = true;
                } else {
                    set_status(
                        clients,
                        client_id,
                        Some("cannot resize session PTYs".into()),
                        input.full_dirty,
                    );
                }
            }
        }
        clear_mouse_capture(clients, client_id);
        return;
    }

    let content_size = pane_area(size);
    let rects = input.session.pane_rects(content_size);
    let target = rects
        .iter()
        .find(|(_, rect)| rect_contains(*rect, event.x, event.y))
        .copied();
    let capture = clients
        .iter()
        .find(|client| client.id == client_id)
        .and_then(|client| client.mouse_capture);
    let in_scroll_view = clients
        .iter()
        .find(|client| client.id == client_id)
        .is_some_and(|client| client.scroll_offset.is_some());

    if let Some(capture) = capture
        && (motion || event.release)
    {
        let moved_to = match capture {
            MouseCapture::Border(x, y) if motion => {
                resize_border(input.session, content_size, (x, y), event.x, event.y)
            }
            MouseCapture::Click | MouseCapture::Border(_, _) => None,
        };
        if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
            client.mouse_capture = if event.release {
                None
            } else {
                Some(moved_to.map_or(capture, |(x, y)| MouseCapture::Border(x, y)))
            };
        }
        if moved_to.is_some() {
            if resize_all(input.session, input.panes, size, size).is_ok() {
                *input.full_dirty = true;
            } else {
                set_status(
                    clients,
                    client_id,
                    Some("cannot resize session PTYs".into()),
                    input.full_dirty,
                );
            }
        }
        return;
    }

    if let Some((pane_id, rect)) = target
        && !in_scroll_view
        && Some(pane_id) == input.session.active_pane()
        && let Some(pane) = input.panes.iter_mut().find(|pane| pane.id == pane_id)
        && forwards_mouse(pane.terminal.modes().mouse, event)
        && let Some(bytes) = encode_mouse(event, rect, pane.terminal.modes().sgr_mouse)
    {
        pane.pending_input.push(bytes);
        return;
    }

    if wheel {
        let maximum = input
            .session
            .active_pane()
            .and_then(|active| input.panes.iter().find(|pane| pane.id == active))
            .map_or(0, |pane| pane.terminal.max_scroll_offset());
        if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
            let current = client.scroll_offset.unwrap_or(0);
            let next = if event.code & 1 == 0 {
                current.saturating_add(3).min(maximum)
            } else {
                current.saturating_sub(3)
            };
            client.scroll_offset = (next != 0 || client.input.is_scroll_mode()).then_some(next);
            if client.input.is_scroll_mode() {
                set_scroll_status(client, maximum);
            }
            *input.full_dirty = true;
        }
        return;
    }

    if event.release {
        clear_mouse_capture(clients, client_id);
    } else if left_press {
        if let Some((pane, _)) = target {
            input.session.select_pane(pane);
            if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                client.scroll_offset = None;
                client.mouse_capture = Some(MouseCapture::Click);
            }
            *input.full_dirty = true;
        } else if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
            client.mouse_capture = Some(MouseCapture::Border(event.x, event.y));
        }
    }
}

fn clear_mouse_capture(clients: &mut [Client], client_id: u64) {
    if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
        client.mouse_capture = None;
    }
}

fn forwards_mouse(mode: MouseMode, event: MouseEvent) -> bool {
    let motion = event.code & 32 != 0;
    match mode {
        MouseMode::Off => false,
        MouseMode::Press => !motion,
        MouseMode::Drag => !motion || event.code & 3 != 3,
        MouseMode::Motion => true,
    }
}

fn encode_mouse(event: MouseEvent, rect: Rect, sgr: bool) -> Option<Vec<u8>> {
    let x = event.x.checked_sub(rect.x)?.saturating_add(1);
    let y = event.y.checked_sub(rect.y)?.saturating_add(1);
    if sgr {
        return Some(
            format!(
                "\x1b[<{};{x};{y}{}",
                event.code,
                if event.release { 'm' } else { 'M' }
            )
            .into_bytes(),
        );
    }
    let code = if event.release { 3 } else { event.code };
    if code > 223 || x > 223 || y > 223 {
        return None;
    }
    Some(vec![
        27,
        b'[',
        b'M',
        (code as u8).saturating_add(32),
        (x as u8).saturating_add(32),
        (y as u8).saturating_add(32),
    ])
}

fn resize_border(
    session: &mut Session,
    size: Size,
    start: (u16, u16),
    x: u16,
    y: u16,
) -> Option<(u16, u16)> {
    let horizontal = x.abs_diff(start.0) >= y.abs_diff(start.1);
    let steps = if horizontal {
        x.abs_diff(start.0)
    } else {
        y.abs_diff(start.1)
    }
    .min(MAX_MOUSE_DRAG_CELLS);
    let mut border = start;
    let mut moved = false;
    for _ in 0..steps {
        let rects = session.pane_rects(size);
        let choice = if horizontal && x > border.0 {
            neighbor(&rects, border.0.checked_sub(1), Some(border.1))
                .map(|pane| (pane, Direction::Right))
        } else if horizontal {
            neighbor(&rects, border.0.checked_add(1), Some(border.1))
                .map(|pane| (pane, Direction::Left))
        } else if y > border.1 {
            neighbor(&rects, Some(border.0), border.1.checked_sub(1))
                .map(|pane| (pane, Direction::Down))
        } else {
            neighbor(&rects, Some(border.0), border.1.checked_add(1))
                .map(|pane| (pane, Direction::Up))
        };
        let Some((pane, direction)) = choice else {
            break;
        };
        session.select_pane(pane);
        if session.resize(direction, size).is_err() {
            break;
        }
        moved = true;
        if horizontal {
            border.0 = if x > border.0 {
                border.0.saturating_add(1)
            } else {
                border.0.saturating_sub(1)
            };
        } else {
            border.1 = if y > border.1 {
                border.1.saturating_add(1)
            } else {
                border.1.saturating_sub(1)
            };
        }
    }
    moved.then_some(border)
}

fn neighbor(rects: &[(PaneId, Rect)], x: Option<u16>, y: Option<u16>) -> Option<PaneId> {
    let (x, y) = (x?, y?);
    rects
        .iter()
        .find(|(_, rect)| rect_contains(*rect, x, y))
        .map(|(pane, _)| *pane)
}

fn rect_contains(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}

fn add_pane(
    context: &LaunchContext,
    session: &Session,
    panes: &mut Vec<PaneProcess>,
    pane: PaneId,
    size: Size,
    scrollback_limit: usize,
    events: &SyncSender<ServerEvent>,
) -> Result<(), String> {
    let pane_size = session
        .pane_rects(size)
        .into_iter()
        .find(|(id, _)| *id == pane)
        .map(|(_, rect)| Size {
            columns: rect.width,
            rows: rect.height,
        })
        .unwrap_or(size);
    let child = PtyChild::spawn(context, pane_size)
        .map_err(|error| format!("cannot start shell: {error}"))?;
    let reader = child
        .output_reader()
        .map_err(|error| format!("cannot read shell output: {error}"))?;
    let terminal = Terminal::with_scrollback(pane_size, scrollback_limit)
        .map_err(|error| format!("cannot create terminal screen: {error}"))?;
    spawn_pane_reader(pane, reader, events.clone())?;
    panes.push(PaneProcess {
        id: pane,
        child: Some(child),
        viewer: None,
        viewer_gate: ViewerGate::default(),
        pending_viewer_search: None,
        pending_viewer_client: None,
        terminal,
        pending_input: PendingInput::new(),
        working_directory: context.working_directory().to_owned(),
    });
    Ok(())
}

fn set_status(
    clients: &mut [Client],
    client_id: u64,
    status: Option<String>,
    full_dirty: &mut bool,
) {
    if let Some(client) = clients.iter_mut().find(|client| client.id == client_id)
        && client.status != status
    {
        client.status = status;
        *full_dirty = true;
    }
}

fn resize_all(
    session: &Session,
    panes: &mut [PaneProcess],
    size: Size,
    rollback: Size,
) -> io::Result<()> {
    let rects = session.pane_rects(pane_area(size));
    let rollback_rects = session.pane_rects(pane_area(rollback));
    if rects.iter().any(|(_, rect)| {
        rect.width == 0
            || rect.height == 0
            || usize::from(rect.width) * usize::from(rect.height) > MAX_SCREEN_CELLS
    }) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "terminal dimensions are outside supported limits",
        ));
    }
    for index in 0..panes.len() {
        let pane_size = rects
            .iter()
            .find(|(id, _)| *id == panes[index].id)
            .map(|(_, rect)| Size {
                columns: rect.width,
                rows: rect.height,
            })
            .unwrap_or_else(|| pane_area(size));
        if let Err(error) = panes[index].resize(pane_size) {
            for resized in &mut panes[..index] {
                let old = rollback_rects
                    .iter()
                    .find(|(id, _)| *id == resized.id)
                    .map(|(_, rect)| Size {
                        columns: rect.width,
                        rows: rect.height,
                    })
                    .unwrap_or_else(|| pane_area(rollback));
                let _ = resized.resize(old);
            }
            return Err(error);
        }
    }
    Ok(())
}

fn pane_area(size: Size) -> Size {
    Size {
        columns: size.columns.max(1),
        rows: size.rows.saturating_sub(1).max(1),
    }
}

fn queue_control(clients: &mut [Client], client_id: u64, message: Message) {
    let Some(client) = clients.iter_mut().find(|client| client.id == client_id) else {
        return;
    };
    match client.outbound.try_send(message) {
        Ok(()) => {}
        Err(TrySendError::Full(message)) => client.pending_control = Some(message),
        Err(TrySendError::Disconnected(_)) => {
            let _ = client.control.shutdown();
        }
    }
}

fn flush_client_controls(clients: &mut [Client]) {
    for client in clients {
        let Some(message) = client.pending_control.take() else {
            continue;
        };
        match client.outbound.try_send(message) {
            Ok(()) => {}
            Err(TrySendError::Full(message)) => client.pending_control = Some(message),
            Err(TrySendError::Disconnected(_)) => {
                let _ = client.control.shutdown();
            }
        }
    }
}

fn flush_broadcast(pending: &mut Option<PendingBroadcast>, clients: &[Client]) {
    let Some(broadcast) = pending else {
        return;
    };
    broadcast.frames.retain(|(client_id, bytes)| {
        let Some(client) = clients
            .iter()
            .find(|client| client.id == *client_id && client.attached)
        else {
            return false;
        };
        match client.outbound.try_send(Message::Screen(bytes.clone())) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => false,
            Err(TrySendError::Full(_)) => true,
        }
    });
    if broadcast.frames.is_empty() {
        *pending = None;
    }
}

fn remove_client(clients: &mut Vec<Client>, client_id: u64) {
    if let Some(index) = clients.iter().position(|client| client.id == client_id) {
        let client = clients.swap_remove(index);
        let _ = client.control.shutdown();
    }
}

fn status_line<'a>(config: &'a Config, metrics: &'a Metrics) -> StatusLine<'a> {
    StatusLine {
        date_format: &config.date_format,
        time_format: &config.time_format,
        format: &config.status_format,
        label: &config.status_label,
        metrics,
        foreground: config.status_foreground,
        background: config.status_background,
        label_foreground: config.label_foreground,
        label_background: config.label_background,
        active_foreground: config.active_tab_foreground,
        active_background: config.active_tab_background,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_protocol_responses_use_the_pane_input_queue() {
        let mut terminal = Terminal::new(Size {
            columns: 80,
            rows: 24,
        })
        .unwrap();
        terminal.advance(b"\x1b[6n\x1b[c");

        let mut pending = PendingInput::new();
        pending.push(terminal.take_responses());
        let mut output = Vec::new();
        pending.flush(&mut output).unwrap();

        assert_eq!(output, b"\x1b[1;1R\x1b[?1;2c");
        assert!(pending.is_empty());
    }

    #[test]
    fn viewer_ready_keeps_the_existing_idle_poll_delay() {
        assert_eq!(LISTENER_POLL_DELAY, Duration::from_millis(50));
    }

    #[test]
    fn repeat_search_completion_has_no_new_recorded_state() {
        let new_search = PendingViewerSearch::New {
            client_id: 7,
            query: b"foo".to_vec(),
            mode: SearchMode::Matching,
            direction: SearchDirection::Forward,
        };
        assert_eq!(
            new_viewer_search_state(&new_search),
            Some((7, SearchMode::Matching, SearchDirection::Forward))
        );

        let repeat = PendingViewerSearch::Repeat {
            client_id: 7,
            relation: RepeatDirection::Opposite,
        };
        assert_eq!(new_viewer_search_state(&repeat), None);
    }

    #[test]
    fn viewer_wrap_status_names_the_boundary_and_direction() {
        assert_eq!(
            viewer_wrap_status(SearchDirection::Forward),
            "search hit BOTTOM, continuing at TOP"
        );
        assert_eq!(
            viewer_wrap_status(SearchDirection::Reverse),
            "search hit TOP, continuing at BOTTOM"
        );
    }

    #[test]
    fn viewer_gate_drops_held_page_repeats() {
        let intent = ViewerIntent::Page {
            rows: 8,
            forward: true,
        };
        let mut gate = ViewerGate::default();
        assert!(matches!(
            gate.accept(ViewerNavigation {
                intent,
                client_id: 1
            }),
            ViewerGateDecision::Dispatch(_)
        ));
        gate.begin(intent);

        for _ in 0..1_000 {
            assert!(matches!(
                gate.accept(ViewerNavigation {
                    intent,
                    client_id: 1
                }),
                ViewerGateDecision::Dropped
            ));
        }
        assert!(gate.replacement.is_none());
    }

    #[test]
    fn viewer_gate_handles_horizontal_repeats_and_reversals() {
        let left = ViewerIntent::Horizontal(-1);
        let right = ViewerIntent::Horizontal(1);
        let mut gate = ViewerGate::default();
        gate.begin(left);

        for _ in 0..1_000 {
            assert!(matches!(
                gate.accept(ViewerNavigation {
                    intent: left,
                    client_id: 1,
                }),
                ViewerGateDecision::Dropped
            ));
        }
        assert!(matches!(
            gate.accept(ViewerNavigation {
                intent: right,
                client_id: 1,
            }),
            ViewerGateDecision::Replaced
        ));
        assert_eq!(
            gate.finish(),
            Some(ViewerNavigation {
                intent: right,
                client_id: 1,
            })
        );
    }

    #[test]
    fn viewer_gate_stops_after_the_current_frame() {
        let intent = ViewerIntent::Page {
            rows: 8,
            forward: true,
        };
        let mut gate = ViewerGate::default();
        gate.begin(intent);

        assert!(gate.finish().is_none());
        assert!(!gate.in_flight);
        assert!(gate.replacement.is_none());
    }

    #[test]
    fn viewer_gate_keeps_one_direction_reversal() {
        let down = ViewerIntent::Page {
            rows: 8,
            forward: true,
        };
        let up = ViewerIntent::Page {
            rows: 8,
            forward: false,
        };
        let mut gate = ViewerGate::default();
        gate.begin(down);

        assert!(matches!(
            gate.accept(ViewerNavigation {
                intent: up,
                client_id: 2
            }),
            ViewerGateDecision::Replaced
        ));
        assert_eq!(
            gate.replacement,
            Some(ViewerNavigation {
                intent: up,
                client_id: 2
            })
        );
        assert!(matches!(
            gate.accept(ViewerNavigation {
                intent: up,
                client_id: 3
            }),
            ViewerGateDecision::Dropped
        ));
    }

    #[test]
    fn viewer_gate_latest_changed_intent_wins() {
        let down = ViewerIntent::Page {
            rows: 8,
            forward: true,
        };
        let up = ViewerIntent::Page {
            rows: 8,
            forward: false,
        };
        let mut gate = ViewerGate::default();
        gate.begin(down);
        gate.accept(ViewerNavigation {
            intent: up,
            client_id: 1,
        });
        gate.accept(ViewerNavigation {
            intent: down,
            client_id: 2,
        });

        assert_eq!(
            gate.replacement,
            Some(ViewerNavigation {
                intent: down,
                client_id: 2
            })
        );
    }

    #[test]
    fn viewer_gate_close_clears_pending_navigation() {
        let down = ViewerIntent::Page {
            rows: 8,
            forward: true,
        };
        let up = ViewerIntent::Page {
            rows: 8,
            forward: false,
        };
        let mut gate = ViewerGate::default();
        gate.begin(down);
        gate.accept(ViewerNavigation {
            intent: up,
            client_id: 1,
        });
        let generation = gate.generation;

        gate.cancel();

        assert!(gate.generation > generation);
        assert!(!gate.in_flight);
        assert!(gate.current_intent.is_none());
        assert!(gate.replacement.is_none());
    }

    #[test]
    fn mouse_forwarding_translates_to_pane_coordinates() {
        let event = MouseEvent {
            code: 0,
            x: 12,
            y: 7,
            release: false,
        };
        assert_eq!(
            encode_mouse(
                event,
                Rect {
                    x: 10,
                    y: 5,
                    width: 20,
                    height: 10,
                },
                true,
            ),
            Some(b"\x1b[<0;3;3M".to_vec())
        );
        assert!(forwards_mouse(MouseMode::Press, event));
    }

    #[test]
    fn mouse_drag_resizes_the_selected_border() {
        let mut session = Session::new("s".into());
        let size = Size {
            columns: 9,
            rows: 3,
        };
        session
            .split_active(crate::session::Split::LeftRight, size)
            .unwrap();

        assert_eq!(
            resize_border(&mut session, size, (4, 1), 6, 1),
            Some((6, 1))
        );
        let rects = session.pane_rects(size);
        assert_eq!(rects[0].1.width, 6);
        assert_eq!(rects[1].1.width, 2);
    }

    #[test]
    fn scroll_status_reminder_adapts_to_width() {
        assert_eq!(
            scroll_status_message(40, 37, None),
            "SCROLL 37% | j/k Pg / Esc"
        );
        let detailed = scroll_status_message(120, 37, Some("error"));
        assert!(detailed.contains("g/G ends"));
        assert!(detailed.contains("n/N match"));
        assert!(detailed.ends_with("/error"));
    }

    #[cfg(unix)]
    #[test]
    fn viewer_prompt_root_uses_unix_separator() {
        let directory = PathBuf::from("/tmp");
        assert_eq!(prompt_root(&directory, b'/'), Some(PathBuf::from("/")));
        assert_eq!(prompt_root(&directory, b'\\'), None);
        assert_eq!(prompt_drive_root(b"C:"), None);
    }

    #[test]
    fn viewer_selection_wraps_directory_entries() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "termfold-viewer-selection-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("alpha"), b"").unwrap();
        fs::write(directory.join("beta"), b"").unwrap();
        let mut prompt = ViewerPrompt {
            directory: directory.clone(),
            query: Vec::new(),
            filter: Vec::new(),
            selected: 0,
        };

        assert_eq!(
            select_viewer_entry(&mut prompt, 1).unwrap(),
            b"beta".to_vec()
        );
        assert_eq!(
            select_viewer_entry(&mut prompt, 1).unwrap(),
            b"alpha".to_vec()
        );
        assert_eq!(
            select_viewer_entry(&mut prompt, -1).unwrap(),
            b"beta".to_vec()
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn viewer_completion_keeps_current_selection_synchronized() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "termfold-viewer-completion-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("alpha"), b"").unwrap();
        fs::create_dir(directory.join("alpine")).unwrap();
        let mut prompt = ViewerPrompt {
            directory: directory.clone(),
            query: b"al".to_vec(),
            filter: b"al".to_vec(),
            selected: 0,
        };

        assert_eq!(complete_viewer_entry(&mut prompt).unwrap(), b"alpha");
        assert_eq!(prompt.selected, 0);
        assert_eq!(complete_viewer_entry(&mut prompt).unwrap(), b"alpine");
        assert_eq!(prompt.selected, 1);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn empty_tilde_prompt_has_no_selected_entry() {
        for query in [
            b"~".as_slice(),
            b"prefix~".as_slice(),
            b"~/".as_slice(),
            b"~\\".as_slice(),
        ] {
            let prompt = ViewerPrompt {
                directory: PathBuf::from("/definitely/missing"),
                query: query.to_vec(),
                filter: query.to_vec(),
                selected: 0,
            };
            assert_eq!(selected_viewer_entry(&prompt, &[], query, false), None);
            assert_eq!(selected_viewer_entry(&prompt, &[], query, true), None);
        }
    }

    #[cfg(unix)]
    #[test]
    fn viewer_prompt_ido_resolves_linux_root_and_home() {
        let prompt = ViewerPrompt {
            directory: std::env::current_dir().unwrap(),
            query: Vec::new(),
            filter: Vec::new(),
            selected: 0,
        };
        let home = prompt.directory.clone();

        assert_eq!(
            resolve_prompt_directory(&prompt, b"/", b'/', Some(home.clone())).unwrap(),
            PathBuf::from("/")
        );
        assert_eq!(
            resolve_prompt_directory(&prompt, b"~", b'/', Some(home.clone())).unwrap(),
            home
        );
    }

    #[test]
    fn viewer_prompt_ido_errors_preserve_editable_state_and_files() {
        let directory = std::env::temp_dir().join(format!(
            "termfold-viewer-ido-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let marker = directory.join("marker");
        fs::write(&marker, b"unchanged").unwrap();
        let prompt = ViewerPrompt {
            directory: directory.clone(),
            query: b"keep".to_vec(),
            filter: b"keep".to_vec(),
            selected: 3,
        };
        let before = prompt.clone();

        assert_eq!(
            resolve_prompt_directory(&prompt, b"~", b'/', None),
            Err("home directory is unavailable".to_owned())
        );
        assert!(
            resolve_prompt_directory(&prompt, b"~text", b'/', Some(directory.clone())).is_err()
        );
        assert_eq!(prompt.directory, before.directory);
        assert_eq!(prompt.query, before.query);
        assert_eq!(prompt.filter, before.filter);
        assert_eq!(prompt.selected, before.selected);
        assert_eq!(fs::read(&marker).unwrap(), b"unchanged");

        fs::remove_file(marker).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn viewer_prompt_root_uses_windows_drive_and_separators() {
        let directory = PathBuf::from("C:\\Users");
        assert_eq!(prompt_root(&directory, b'\\'), Some(PathBuf::from("C:\\")));
        assert_eq!(prompt_root(&directory, b'/'), Some(PathBuf::from("C:\\")));
        assert_eq!(prompt_drive_root(b"C:"), Some(PathBuf::from("C:\\")));
    }

    #[cfg(windows)]
    #[test]
    fn viewer_prompt_ido_resolves_windows_roots_and_rejects_invalid_drives() {
        let directory = std::env::current_dir().unwrap();
        let root = prompt_root(&directory, b'/').unwrap();
        let prompt = ViewerPrompt {
            directory,
            query: Vec::new(),
            filter: Vec::new(),
            selected: 0,
        };

        assert_eq!(
            resolve_prompt_directory(&prompt, b"/", b'/', Some(root.clone())).unwrap(),
            root
        );
        assert_eq!(
            resolve_prompt_directory(&prompt, b"\\", b'\\', Some(root.clone())).unwrap(),
            root
        );
        assert_eq!(
            resolve_prompt_directory(&prompt, b"C:", b'/', Some(root.clone())).unwrap(),
            PathBuf::from("C:\\")
        );
        assert_eq!(
            resolve_prompt_directory(&prompt, b"C:", b'\\', Some(root.clone())).unwrap(),
            PathBuf::from("C:\\")
        );
        assert!(resolve_prompt_directory(&prompt, b"1:", b'/', Some(root)).is_err());
    }
}
