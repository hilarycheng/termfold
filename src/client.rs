use std::{
    io::{self, Read, Write},
    os::fd::AsRawFd,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::{
    ipc::{self, Message},
    runtime::RuntimeDir,
    session::{MAX_SESSIONS_PER_USER, Size},
};

const SERVER_START_TIMEOUT: Duration = Duration::from_secs(2);
const CONTROL_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionInfo {
    pub name: String,
    pub pid: u32,
    pub attached_clients: u32,
}

impl SessionInfo {
    pub fn is_attached(&self) -> bool {
        self.attached_clients != 0
    }
}

pub fn discover(runtime: &RuntimeDir) -> Result<Vec<SessionInfo>, String> {
    let mut sessions = Vec::new();
    for name in runtime.session_names()? {
        if let Ok(info) = query_status(runtime, &name) {
            sessions.push(info);
        }
    }
    sessions.sort_by_key(|session| session.pid);
    Ok(sessions)
}

pub fn create_and_attach(runtime: &RuntimeDir, name: &str) -> Result<(), String> {
    let creation_lock = runtime.lock_creation()?;
    let sessions = discover(runtime)?;
    if sessions.iter().any(|session| session.name == name) {
        return Err(format!("session '{name}' already exists"));
    }
    if sessions.len() >= MAX_SESSIONS_PER_USER {
        return Err("current user already has 32 sessions".into());
    }

    let size = terminal_size();
    let mut child = spawn_server(name, size)?;
    let expected_pid = child.id();
    let deadline = Instant::now() + SERVER_START_TIMEOUT;
    loop {
        match query_status(runtime, name) {
            Ok(info) if info.pid == expected_pid => break,
            Ok(_) => {
                let _ = child.wait();
                return Err(format!("session '{name}' already exists"));
            }
            Err(_) if Instant::now() < deadline => {
                if let Some(status) = child
                    .try_wait()
                    .map_err(|error| format!("cannot inspect session server: {error}"))?
                {
                    return Err(format!("session server exited with {status}"));
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("session server did not start: {error}"));
            }
        }
    }
    drop(creation_lock);
    let result = attach(runtime, name);
    if child.try_wait().ok().flatten().is_some() {
        let _ = child.wait();
    }
    result
}

pub fn attach(runtime: &RuntimeDir, name: &str) -> Result<(), String> {
    let mut stream = runtime.connect(name)?;
    let size = terminal_size();
    ipc::write_message(
        &mut stream,
        &Message::Attach {
            columns: size.columns,
            rows: size.rows,
        },
    )
    .map_err(|error| error.to_string())?;
    match ipc::read_message(&mut stream).map_err(|error| error.to_string())? {
        Some(Message::Attached) => {}
        Some(Message::Error(error)) => return Err(error),
        Some(_) => return Err("session returned an unexpected attach response".into()),
        None => return Err("session disconnected during attach".into()),
    }

    let mut input_stream = stream
        .try_clone()
        .map_err(|error| format!("cannot clone session connection: {error}"))?;
    thread::spawn(move || {
        let mut input = io::stdin().lock();
        let mut buffer = [0; 8192];
        loop {
            match input.read(&mut buffer) {
                Ok(0) => {
                    let _ = ipc::write_message(&mut input_stream, &Message::Detach);
                    break;
                }
                Ok(length) => {
                    if ipc::write_message(
                        &mut input_stream,
                        &Message::Input(buffer[..length].to_vec()),
                    )
                    .is_err()
                    {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let mut output = io::stdout().lock();
    loop {
        match ipc::read_message(&mut stream).map_err(|error| error.to_string())? {
            Some(Message::Screen(bytes)) => {
                output
                    .write_all(&bytes)
                    .and_then(|()| output.flush())
                    .map_err(|error| format!("cannot write terminal output: {error}"))?;
            }
            Some(Message::Error(error)) => return Err(error),
            Some(Message::Terminating) | None => return Ok(()),
            Some(_) => return Err("session returned an unexpected message".into()),
        }
    }
}

pub fn kill(runtime: &RuntimeDir, name: &str) -> Result<(), String> {
    let mut stream = runtime.connect(name)?;
    stream
        .set_read_timeout(Some(CONTROL_TIMEOUT))
        .map_err(|error| format!("cannot configure session connection: {error}"))?;
    ipc::write_message(&mut stream, &Message::Kill).map_err(|error| error.to_string())?;
    loop {
        match ipc::read_message(&mut stream) {
            Ok(Some(Message::Terminating)) => {}
            Ok(None) => return Ok(()),
            Ok(Some(_)) => {}
            Err(ipc::ProtocolError::Io(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionReset | io::ErrorKind::UnexpectedEof
                ) =>
            {
                return Ok(());
            }
            Err(error) => return Err(format!("session did not terminate: {error}")),
        }
    }
}

pub fn query_status(runtime: &RuntimeDir, name: &str) -> Result<SessionInfo, String> {
    let mut stream = runtime.connect(name)?;
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .map_err(|error| format!("cannot configure session connection: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_millis(500)))
        .map_err(|error| format!("cannot configure session connection: {error}"))?;
    ipc::write_message(&mut stream, &Message::StatusRequest).map_err(|error| error.to_string())?;
    match ipc::read_message(&mut stream).map_err(|error| error.to_string())? {
        Some(Message::Status {
            pid,
            attached_clients,
        }) => Ok(SessionInfo {
            name: name.to_owned(),
            pid,
            attached_clients,
        }),
        _ => Err(format!("session '{name}' returned an invalid status")),
    }
}

fn spawn_server(name: &str, size: Size) -> Result<Child, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("cannot locate termfold executable: {error}"))?;
    let mut command = Command::new(executable);
    command
        .arg("--server")
        .arg(name)
        .arg(size.columns.to_string())
        .arg(size.rows.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    use std::os::unix::process::CommandExt;
    // SAFETY: setsid is async-signal-safe and does not access parent-process memory.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    command
        .spawn()
        .map_err(|error| format!("cannot start session server: {error}"))
}

fn terminal_size() -> Size {
    let mut window = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: stdin is a live descriptor and window points to writable winsize storage.
    if unsafe { libc::ioctl(io::stdin().as_raw_fd(), libc::TIOCGWINSZ, &raw mut window) } == 0
        && window.ws_col != 0
        && window.ws_row != 0
    {
        Size {
            columns: window.ws_col,
            rows: window.ws_row,
        }
    } else {
        Size {
            columns: 80,
            rows: 24,
        }
    }
}
