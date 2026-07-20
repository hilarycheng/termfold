use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    thread,
    time::Duration,
};

use crate::{
    ipc::{self, Message},
    pty::{self, LaunchContext, PtyChild},
    runtime::{self, RuntimeDir},
    session::{CloseResult, PaneId, Session, Size},
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
}

struct PaneProcess {
    id: PaneId,
    child: PtyChild,
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
    bytes: Vec<u8>,
    remaining: Vec<u64>,
}

pub fn run(runtime: RuntimeDir, name: String, initial_size: Size) -> Result<(), String> {
    let socket = runtime.bind(&name)?;
    socket
        .listener()
        .set_nonblocking(true)
        .map_err(|error| format!("cannot configure session listener: {error}"))?;

    let context = LaunchContext::capture()
        .map_err(|error| format!("cannot capture shell environment: {error}"))?;
    let mut session = Session::new(name);
    let first_pane = session
        .active_pane()
        .expect("new session always contains one pane");
    let first_child = PtyChild::spawn(&context, initial_size)
        .map_err(|error| format!("cannot start shell: {error}"))?;
    let mut panes = vec![PaneProcess {
        id: first_pane,
        child: first_child,
    }];
    let mut clients = Vec::<Client>::new();
    let mut next_client_id = 1_u64;
    let mut authoritative_size = initial_size;
    let mut pending_input = PendingInput::new();
    let mut pending_broadcast: Option<PendingBroadcast> = None;
    let mut terminate = false;

    while !terminate {
        accept_clients(
            socket.listener(),
            runtime.uid(),
            &mut clients,
            &mut next_client_id,
        );
        flush_client_controls(&mut clients);

        if let Some(pane) = panes.first_mut()
            && pending_input.flush(pane.child.master()).is_err()
        {
            terminate = true;
        }

        if pending_input.is_empty() {
            let events = collect_client_events(&clients);
            for (client_id, event) in events {
                match event {
                    ClientEvent::Closed => remove_client(&mut clients, client_id),
                    ClientEvent::Message(message) => {
                        if handle_message(
                            &mut clients,
                            client_id,
                            message,
                            &mut panes,
                            &mut authoritative_size,
                            &mut pending_input,
                        ) {
                            terminate = true;
                            break;
                        }
                    }
                }
            }
        }

        flush_broadcast(&mut pending_broadcast, &clients);
        if pending_broadcast.is_none()
            && clients.iter().any(|client| client.attached)
            && let Some(pane) = panes.first_mut()
        {
            let mut buffer = vec![0; 8192];
            match pane.child.master().read(&mut buffer) {
                Ok(0) => {}
                Ok(length) => {
                    buffer.truncate(length);
                    pending_broadcast = Some(PendingBroadcast {
                        bytes: buffer,
                        remaining: clients
                            .iter()
                            .filter(|client| client.attached)
                            .map(|client| client.id)
                            .collect(),
                    });
                    flush_broadcast(&mut pending_broadcast, &clients);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) if error.raw_os_error() == Some(libc::EIO) => {}
                Err(_) => terminate = true,
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
            match session.close_pane(pane_id, authoritative_size) {
                Ok(CloseResult::SessionEmpty) => terminate = true,
                Ok(CloseResult::PaneClosed | CloseResult::TabClosed) => {}
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
    panes: &mut [PaneProcess],
    authoritative_size: &mut Size,
    pending_input: &mut PendingInput,
) -> bool {
    match message {
        Message::Attach { columns, rows } => {
            let size = Size { columns, rows };
            if resize_all(panes, size, *authoritative_size).is_err() {
                queue_control(
                    clients,
                    client_id,
                    Message::Error("cannot resize session PTYs".into()),
                );
                return false;
            }
            *authoritative_size = size;
            if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                client.attached = true;
                client.size = Some(size);
            }
            queue_control(clients, client_id, Message::Attached);
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
            if resize_all(panes, size, *authoritative_size).is_err() {
                remove_client(clients, client_id);
            } else {
                *authoritative_size = size;
            }
        }
        Message::Input(bytes) => {
            let Some(client) = clients.iter().find(|client| client.id == client_id) else {
                return false;
            };
            if !client.attached {
                remove_client(clients, client_id);
                return false;
            }
            if let Some(size) = client.size {
                if resize_all(panes, size, *authoritative_size).is_err() {
                    remove_client(clients, client_id);
                    return false;
                }
                *authoritative_size = size;
            }
            pending_input.push(bytes);
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

fn resize_all(panes: &[PaneProcess], size: Size, rollback: Size) -> io::Result<()> {
    for (index, pane) in panes.iter().enumerate() {
        if let Err(error) = pane.child.resize(size) {
            for resized in &panes[..index] {
                let _ = resized.child.resize(rollback);
            }
            return Err(error);
        }
    }
    Ok(())
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
    broadcast.remaining.retain(|client_id| {
        let Some(client) = clients
            .iter()
            .find(|client| client.id == *client_id && client.attached)
        else {
            return false;
        };
        match client
            .outbound
            .try_send(Message::Screen(broadcast.bytes.clone()))
        {
            Ok(()) | Err(TrySendError::Disconnected(_)) => false,
            Err(TrySendError::Full(_)) => true,
        }
    });
    if broadcast.remaining.is_empty() {
        *pending = None;
    }
}

fn remove_client(clients: &mut Vec<Client>, client_id: u64) {
    if let Some(index) = clients.iter().position(|client| client.id == client_id) {
        let client = clients.swap_remove(index);
        let _ = client.control.shutdown(Shutdown::Both);
    }
}
