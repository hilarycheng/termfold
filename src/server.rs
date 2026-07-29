use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    thread,
    time::{Duration, SystemTime},
};

use crate::{
    config::Config,
    input::{Action, Input, MouseEvent},
    ipc::{self, Message},
    outer::{self, Capabilities},
    pty::{self, LaunchContext, PtyChild},
    render::{self, Metrics, StatusLine},
    runtime::{ClientStream, RuntimeDir, SessionSocket},
    session::{CloseResult, Direction, PaneId, Rect, Session, Size},
    terminal::{MAX_SCREEN_CELLS, MouseMode, Terminal},
};

// Two buffered frames plus one being written and one pending frame remain within
// both the four-frame worst-case payload cap and the normative 4 MiB byte cap.
const CONNECTION_QUEUE_ITEMS: usize = 2;
const LOOP_DELAY: Duration = Duration::from_millis(50);
const ENTER_MOUSE: &[u8] = b"\x1b[?1003h\x1b[?1006h";
// ponytail: one event moves 256 cells; raise only if real terminals jump farther.
const MAX_MOUSE_DRAG_CELLS: u16 = 256;

enum ClientEvent {
    Message(Message),
    Closed,
}

struct Client {
    id: u64,
    control: ClientStream,
    inbound: Receiver<ClientEvent>,
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
    mouse_capture: Option<MouseCapture>,
}

#[derive(Clone, Copy)]
enum MouseCapture {
    Click,
    Border(u16, u16),
}

struct PaneProcess {
    id: PaneId,
    child: PtyChild,
    terminal: Terminal,
    pending_input: PendingInput,
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
    mouse: bool,
    status_line: StatusLine<'a>,
    full_dirty: &'a mut bool,
}

pub fn run(
    runtime: RuntimeDir,
    name: String,
    initial_size: Size,
    config: Config,
) -> Result<(), String> {
    let terminfo_root = runtime.materialize_terminfo()?;
    let context = LaunchContext::capture(
        terminfo_root,
        config.inner_term.clone(),
        &config.windows_shell,
    )
    .map_err(|error| format!("cannot capture shell environment: {error}"))?;
    let mut session = Session::new(name);
    let first_pane = session
        .active_pane()
        .expect("new session always contains one pane");
    let content_size = pane_area(initial_size);
    let first_child = PtyChild::spawn(&context, content_size)
        .map_err(|error| format!("cannot start shell: {error}"))?;
    let mut panes = vec![PaneProcess {
        id: first_pane,
        child: first_child,
        terminal: Terminal::with_scrollback(content_size, usize::from(config.scrollback_lines))
            .map_err(|error| format!("cannot create terminal screen: {error}"))?,
        pending_input: PendingInput::new(),
    }];
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
    let mut snapshot = render::Snapshot::new();
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
        );
        flush_client_controls(&mut clients);

        for pane in &mut panes {
            if pane.pending_input.flush(pane.child.master()).is_err() {
                terminate = true;
                break;
            }
        }

        if panes.iter().all(|pane| pane.pending_input.is_empty()) {
            let events = collect_client_events(&clients);
            for (client_id, event) in events {
                match event {
                    ClientEvent::Closed => remove_client(&mut clients, client_id),
                    ClientEvent::Message(message) => {
                        let mut input = InputContext {
                            session: &mut session,
                            panes: &mut panes,
                            size: &mut authoritative_size,
                            launch: &context,
                            scrollback_limit: usize::from(config.scrollback_lines),
                            mouse: config.mouse,
                            status_line: status_line(&config, &metrics),
                            full_dirty: &mut full_dirty,
                        };
                        if handle_message(&mut clients, client_id, message, &mut input) {
                            terminate = true;
                            break;
                        }
                    }
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
                    mouse: config.mouse,
                    status_line: status_line(&config, &metrics),
                    full_dirty: &mut full_dirty,
                };
                if handle_actions(&mut clients, client_id, actions, &mut input) {
                    terminate = true;
                    break;
                }
            }
        }

        flush_broadcast(&mut pending_broadcast, &clients);
        for pane in &mut panes {
            let mut buffer = [0; 8192];
            match pane.child.master().read(&mut buffer) {
                Ok(0) => {}
                Ok(length) => {
                    let previous_epoch = pane.terminal.scrollback_epoch();
                    let previous_maximum = pane.terminal.max_scroll_offset();
                    pane.terminal.advance(&buffer[..length]);
                    let maximum = pane.terminal.max_scroll_offset();
                    if maximum < previous_maximum && session.active_pane() == Some(pane.id) {
                        for client in &mut clients {
                            if client.scroll_offset.take().is_some() {
                                client.search_query = None;
                                client.status = None;
                                full_dirty = true;
                            }
                        }
                    } else if session.active_pane() == Some(pane.id) {
                        let added = pane
                            .terminal
                            .scrollback_epoch()
                            .saturating_sub(previous_epoch)
                            as usize;
                        for client in &mut clients {
                            if let Some(offset) = &mut client.scroll_offset {
                                *offset = offset.saturating_add(added).min(maximum);
                                set_scroll_status(client, maximum);
                            }
                        }
                    }
                    pane.pending_input.push(pane.terminal.take_responses());
                    content_dirty = true;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) if pty::is_eof_error(&error) => {}
                Err(_) => terminate = true,
            }
        }

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
                snapshot = render::snapshot(&pane_screens);
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
                                    &snapshot,
                                    *capabilities,
                                )
                            },
                        )
                    })
                    .collect();
                snapshot = render::snapshot(&pane_screens);
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
                pending_broadcast = Some(PendingBroadcast { frames });
                flush_broadcast(&mut pending_broadcast, &clients);
            }
        }

        let mut exited = Vec::new();
        for pane in &mut panes {
            match pane.child.try_wait() {
                Ok(Some(_)) => exited.push(pane.id),
                Ok(None) => {}
                Err(_) => terminate = true,
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

        if !terminate {
            thread::sleep(LOOP_DELAY);
        }
    }

    for client in &clients {
        let _ = client.outbound.try_send(Message::Terminating);
    }
    let mut children = panes
        .iter_mut()
        .map(|pane| &mut pane.child)
        .collect::<Vec<_>>();
    pty::terminate_all(&mut children)
        .map_err(|error| format!("cannot terminate session children: {error}"))
}

fn accept_clients(
    listener: &SessionSocket,
    clients: &mut Vec<Client>,
    next_client_id: &mut u64,
    prefix: u8,
    mouse: bool,
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
        let (event_sender, inbound) = mpsc::sync_channel(CONNECTION_QUEUE_ITEMS);
        let (outbound, message_receiver) = mpsc::sync_channel(CONNECTION_QUEUE_ITEMS);
        thread::spawn(move || read_client(reader, event_sender));
        thread::spawn(move || write_client(writer, message_receiver));
        clients.push(Client {
            id: *next_client_id,
            control: stream,
            inbound,
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
            mouse_capture: None,
        });
        *next_client_id = next_client_id.saturating_add(1);
    }
}

fn read_client(mut stream: ClientStream, sender: SyncSender<ClientEvent>) {
    loop {
        match ipc::read_message(&mut stream) {
            Ok(Some(message)) => {
                if sender.send(ClientEvent::Message(message)).is_err() {
                    break;
                }
            }
            Ok(None) | Err(_) => {
                let _ = sender.send(ClientEvent::Closed);
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

fn collect_client_events(clients: &[Client]) -> Vec<(u64, ClientEvent)> {
    let mut events = Vec::new();
    for client in clients {
        if client.pending_control.is_some() {
            continue;
        }
        match client.inbound.try_recv() {
            Ok(event) => events.push((client.id, event)),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => events.push((client.id, ClientEvent::Closed)),
        }
    }
    events
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
        Message::Attached
        | Message::Screen(_)
        | Message::Error(_)
        | Message::Status { .. }
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
    let result: Result<(), String> = match action {
        Action::Forward(bytes) => {
            set_status(clients, client_id, None, input.full_dirty);
            if let Some(active) = input.session.active_pane()
                && let Some(pane) = input.panes.iter_mut().find(|pane| pane.id == active)
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
            let close = input.session.close_active_pane(content_size);
            if let Some(index) = input.panes.iter().position(|pane| pane.id == pane_id) {
                let mut pane = input.panes.swap_remove(index);
                let _ = pty::terminate_all(&mut [&mut pane.child]);
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
    let terminal = Terminal::with_scrollback(pane_size, scrollback_limit)
        .map_err(|error| format!("cannot create terminal screen: {error}"))?;
    panes.push(PaneProcess {
        id: pane,
        child,
        terminal,
        pending_input: PendingInput::new(),
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
        if let Err(error) = panes[index].child.resize(pane_size) {
            for resized in &mut panes[..index] {
                let old = rollback_rects
                    .iter()
                    .find(|(id, _)| *id == resized.id)
                    .map(|(_, rect)| Size {
                        columns: rect.width,
                        rows: rect.height,
                    })
                    .unwrap_or_else(|| pane_area(rollback));
                let _ = resized.child.resize(old);
                let _ = resized.terminal.resize(old);
            }
            return Err(error);
        }
        panes[index]
            .terminal
            .resize(pane_size)
            .map_err(io::Error::other)?;
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
}
