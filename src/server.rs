use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    thread,
    time::{Duration, SystemTime},
};

use crate::{
    config::Config,
    input::{Action, Input},
    ipc::{self, Message},
    outer::{self, Capabilities},
    pty::{self, LaunchContext, PtyChild},
    render::{self, Clock},
    runtime::{self, RuntimeDir},
    session::{CloseResult, PaneId, Session, Size},
    terminal::{MAX_SCREEN_CELLS, Terminal},
};

// Two buffered frames plus one being written and one pending frame remain within
// both the four-frame worst-case payload cap and the normative 4 MiB byte cap.
const CONNECTION_QUEUE_ITEMS: usize = 2;
const LOOP_DELAY: Duration = Duration::from_millis(10);

enum ClientEvent {
    Message(Message),
    Closed,
}

struct Client {
    id: u64,
    control: UnixStream,
    inbound: Receiver<ClientEvent>,
    outbound: SyncSender<Message>,
    pending_control: Option<Message>,
    attached: bool,
    size: Option<Size>,
    capabilities: Option<Capabilities>,
    input: Input,
    status: Option<String>,
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
    full_dirty: &'a mut bool,
}

pub fn run(
    runtime: RuntimeDir,
    name: String,
    initial_size: Size,
    config: Config,
) -> Result<(), String> {
    let socket = runtime.bind(&name)?;
    socket
        .listener()
        .set_nonblocking(true)
        .map_err(|error| format!("cannot configure session listener: {error}"))?;

    let terminfo_root = runtime.materialize_terminfo()?;
    let context = LaunchContext::capture(terminfo_root, config.inner_term.clone())
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
        terminal: Terminal::new(content_size)
            .map_err(|error| format!("cannot create terminal screen: {error}"))?,
        pending_input: PendingInput::new(),
    }];
    let mut clients = Vec::<Client>::new();
    let mut next_client_id = 1_u64;
    let mut authoritative_size = initial_size;
    let mut pending_broadcast: Option<PendingBroadcast> = None;
    let clock_has_seconds = config.date_format.contains("%S") || config.time_format.contains("%S");
    let mut rendered_clock = None;
    let mut snapshot = render::Snapshot::new();
    let mut full_dirty = false;
    let mut content_dirty = false;
    let mut terminate = false;

    while !terminate {
        accept_clients(
            socket.listener(),
            runtime.uid(),
            &mut clients,
            &mut next_client_id,
            config.prefix,
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
                            full_dirty: &mut full_dirty,
                        };
                        if handle_message(&mut clients, client_id, message, &mut input) {
                            terminate = true;
                            break;
                        }
                    }
                }
            }
        }

        flush_broadcast(&mut pending_broadcast, &clients);
        if pending_broadcast.is_none() && clients.iter().any(|client| client.attached) {
            for pane in &mut panes {
                let mut buffer = [0; 8192];
                match pane.child.master().read(&mut buffer) {
                    Ok(0) => {}
                    Ok(length) => {
                        pane.terminal.advance(&buffer[..length]);
                        pane.pending_input.push(pane.terminal.take_responses());
                        content_dirty = true;
                        break;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) if error.raw_os_error() == Some(libc::EIO) => {}
                    Err(_) => terminate = true,
                }
            }
        }

        if pending_broadcast.is_none() && clients.iter().any(|client| client.attached) {
            let key = render::clock_key(SystemTime::now(), clock_has_seconds);
            let pane_screens = panes
                .iter()
                .map(|pane| (pane.id, &pane.terminal))
                .collect::<Vec<_>>();
            let targets = clients
                .iter()
                .filter_map(|client| {
                    client
                        .capabilities
                        .map(|capabilities| (client.id, capabilities, client.status.as_deref()))
                })
                .collect::<Vec<_>>();
            let frames = if full_dirty {
                full_dirty = false;
                content_dirty = false;
                rendered_clock = Some(key);
                let frames = targets
                    .iter()
                    .map(|(id, capabilities, message)| {
                        (
                            *id,
                            render::full(
                                &session,
                                &pane_screens,
                                authoritative_size,
                                Clock {
                                    date_format: &config.date_format,
                                    time_format: &config.time_format,
                                },
                                *message,
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
                    .map(|(id, capabilities, _)| {
                        (
                            *id,
                            render::changes(
                                &session,
                                &pane_screens,
                                authoritative_size,
                                &snapshot,
                                *capabilities,
                            ),
                        )
                    })
                    .collect();
                snapshot = render::snapshot(&pane_screens);
                Some(frames)
            } else if rendered_clock != Some(key) {
                rendered_clock = Some(key);
                Some(
                    targets
                        .iter()
                        .map(|(id, capabilities, message)| {
                            (
                                *id,
                                render::status(
                                    &session,
                                    authoritative_size,
                                    &config.date_format,
                                    &config.time_format,
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
                Ok(CloseResult::PaneClosed | CloseResult::TabClosed) => full_dirty = true,
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
    listener: &std::os::unix::net::UnixListener,
    uid: u32,
    clients: &mut Vec<Client>,
    next_client_id: &mut u64,
    prefix: u8,
) {
    loop {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(_) => break,
        };
        if runtime::peer_uid(&stream) != Ok(uid) {
            let _ = stream.shutdown(Shutdown::Both);
            continue;
        }
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
            input: Input::new(prefix),
            status: None,
        });
        *next_client_id = next_client_id.saturating_add(1);
    }
}

fn read_client(mut stream: UnixStream, sender: SyncSender<ClientEvent>) {
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

fn write_client(mut stream: UnixStream, receiver: Receiver<Message>) {
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
            for action in actions {
                let detach = matches!(action, Action::Detach);
                if handle_action(clients, client_id, action, input) {
                    return true;
                }
                if detach {
                    break;
                }
            }
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

fn handle_action(
    clients: &mut Vec<Client>,
    client_id: u64,
    action: Action,
    input: &mut InputContext<'_>,
) -> bool {
    let size = *input.size;
    let content_size = pane_area(size);
    let result: Result<(), String> =
        match action {
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
                    add_pane(input.launch, input.session, input.panes, pane, content_size)
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
                    add_pane(input.launch, input.session, input.panes, pane, content_size)
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
            Action::ScrollView => {
                set_status(
                    clients,
                    client_id,
                    Some("scroll view is not available".into()),
                    input.full_dirty,
                );
                return false;
            }
            Action::SaveScrollback(_) => {
                set_status(
                    clients,
                    client_id,
                    Some("scrollback export is not available".into()),
                    input.full_dirty,
                );
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

fn add_pane(
    context: &LaunchContext,
    session: &Session,
    panes: &mut Vec<PaneProcess>,
    pane: PaneId,
    size: Size,
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
    let terminal = Terminal::new(pane_size)
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
            let _ = client.control.shutdown(Shutdown::Both);
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
                let _ = client.control.shutdown(Shutdown::Both);
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
        let _ = client.control.shutdown(Shutdown::Both);
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
}
