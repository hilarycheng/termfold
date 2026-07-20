use std::{
    io::{self, Read, Write},
    os::fd::AsRawFd,
    panic,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
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
const ENTER_TERMINAL: &[u8] = b"\x1b[?1049h";
const RESTORE_TERMINAL: &[u8] =
    b"\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?2004l\x1b[0m\x1b[?25h\x1b[?1049l";
const SIGNALS: [libc::c_int; 5] = [
    libc::SIGWINCH,
    libc::SIGHUP,
    libc::SIGINT,
    libc::SIGQUIT,
    libc::SIGTERM,
];

struct TerminalGuard(Arc<TerminalRestorer>);

struct TerminalRestorer {
    input: libc::c_int,
    output: libc::c_int,
    original: libc::termios,
    restored: AtomicBool,
}

impl TerminalGuard {
    fn enter() -> Result<Option<Self>, String> {
        // SAFETY: isatty only inspects the supplied live file descriptors.
        if unsafe { libc::isatty(libc::STDIN_FILENO) } != 1
            || unsafe { libc::isatty(libc::STDOUT_FILENO) } != 1
        {
            return Ok(None);
        }
        Self::enter_on(libc::STDIN_FILENO, libc::STDOUT_FILENO).map(Some)
    }

    fn enter_on(input: libc::c_int, output: libc::c_int) -> Result<Self, String> {
        // SAFETY: termios is initialized by tcgetattr before it is read.
        let mut original = unsafe { std::mem::zeroed() };
        // SAFETY: input is a live terminal descriptor and original is writable.
        if unsafe { libc::tcgetattr(input, &raw mut original) } == -1 {
            return Err(format!(
                "cannot read terminal mode: {}",
                io::Error::last_os_error()
            ));
        }
        let mut raw = original;
        // SAFETY: raw is initialized termios storage.
        unsafe { libc::cfmakeraw(&raw mut raw) };
        // SAFETY: input is a live terminal descriptor and raw is initialized.
        if unsafe { libc::tcsetattr(input, libc::TCSAFLUSH, &raw) } == -1 {
            return Err(format!(
                "cannot enable raw terminal mode: {}",
                io::Error::last_os_error()
            ));
        }

        let restorer = Arc::new(TerminalRestorer {
            input,
            output,
            original,
            restored: AtomicBool::new(false),
        });
        if let Err(error) = write_fd(output, ENTER_TERMINAL) {
            restorer.restore();
            return Err(format!("cannot enter alternate screen: {error}"));
        }
        Ok(Self(restorer))
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.0.restore();
    }
}

impl TerminalRestorer {
    fn restore(&self) {
        if self.restored.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = write_fd(self.output, RESTORE_TERMINAL);
        // SAFETY: input remains owned by the process and original came from tcgetattr.
        unsafe { libc::tcsetattr(self.input, libc::TCSAFLUSH, &self.original) };
    }
}

struct BlockedSignals {
    set: libc::sigset_t,
    old: libc::sigset_t,
    active: bool,
}

struct SignalGuard {
    old: libc::sigset_t,
    running: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl BlockedSignals {
    fn block() -> Result<Self, String> {
        // SAFETY: both signal sets are initialized by libc before use.
        let (mut set, mut old) = unsafe { (std::mem::zeroed(), std::mem::zeroed()) };
        // SAFETY: set is writable and each listed signal number is valid.
        unsafe {
            libc::sigemptyset(&raw mut set);
            for signal in SIGNALS {
                libc::sigaddset(&raw mut set, signal);
            }
        }
        // SAFETY: pointers reference initialized signal-set storage.
        if unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &set, &raw mut old) } != 0 {
            return Err("cannot block terminal signals".into());
        }
        Ok(Self {
            set,
            old,
            active: true,
        })
    }

    fn listen(
        mut self,
        writer: Arc<Mutex<std::os::unix::net::UnixStream>>,
        terminal: Option<Arc<TerminalRestorer>>,
    ) -> Result<SignalGuard, String> {
        let set = self.set;
        let old = self.old;
        self.active = false;
        let running = Arc::new(AtomicBool::new(true));
        let thread_running = Arc::clone(&running);
        let signal_thread = thread::Builder::new()
            .name("termfold-signals".into())
            .spawn(move || {
                loop {
                    let mut signal = 0;
                    // SAFETY: set is initialized, blocked in this thread, and signal is writable.
                    if unsafe { libc::sigwait(&set, &raw mut signal) } != 0 {
                        continue;
                    }
                    if !thread_running.load(Ordering::Acquire) {
                        break;
                    }
                    if signal == libc::SIGWINCH {
                        let size = terminal_size();
                        let message = Message::Resize {
                            columns: size.columns,
                            rows: size.rows,
                        };
                        if let Ok(mut stream) = writer.lock() {
                            let _ = ipc::write_message(&mut *stream, &message);
                        }
                        continue;
                    }
                    if let Some(terminal) = &terminal {
                        terminal.restore();
                    }
                    // The terminal is restored explicitly because _exit skips destructors.
                    // SAFETY: _exit terminates the current process without touching Rust state.
                    unsafe { libc::_exit(128 + signal) };
                }
            })
            .map_err(|error| {
                // SAFETY: old was returned by pthread_sigmask in this thread.
                unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, &old, std::ptr::null_mut()) };
                format!("cannot start terminal signal handler: {error}")
            })?;
        Ok(SignalGuard {
            old,
            running,
            thread: Some(signal_thread),
        })
    }
}

impl Drop for BlockedSignals {
    fn drop(&mut self) {
        if self.active {
            // SAFETY: old was returned by pthread_sigmask in this thread.
            unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, &self.old, std::ptr::null_mut()) };
        }
    }
}

impl Drop for SignalGuard {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(signal_thread) = self.thread.take() {
            // SAFETY: SIGWINCH is blocked in every thread and consumed by sigwait.
            unsafe { libc::kill(libc::getpid(), libc::SIGWINCH) };
            let _ = signal_thread.join();
        }
        // SAFETY: old was returned by pthread_sigmask in this thread.
        unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, &self.old, std::ptr::null_mut()) };
    }
}

fn write_fd(fd: libc::c_int, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        // SAFETY: bytes points to readable memory for its reported length.
        let written = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
        if written == -1 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        bytes = &bytes[written as usize..];
    }
    Ok(())
}

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

    let blocked_signals = BlockedSignals::block()?;
    let terminal = TerminalGuard::enter()?;
    if let Some(terminal) = &terminal {
        let restorer = Arc::clone(&terminal.0);
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            restorer.restore();
            previous(info);
        }));
    }

    let input_stream = stream
        .try_clone()
        .map_err(|error| format!("cannot clone session connection: {error}"))?;
    let input_stream = Arc::new(Mutex::new(input_stream));
    let _signals = blocked_signals.listen(
        Arc::clone(&input_stream),
        terminal.as_ref().map(|terminal| Arc::clone(&terminal.0)),
    )?;
    thread::spawn(move || {
        let mut input = io::stdin().lock();
        let mut buffer = [0; 8192];
        loop {
            match input.read(&mut buffer) {
                Ok(0) => {
                    if let Ok(mut stream) = input_stream.lock() {
                        let _ = ipc::write_message(&mut *stream, &Message::Detach);
                    }
                    break;
                }
                Ok(length) => {
                    let Ok(mut stream) = input_stream.lock() else {
                        break;
                    };
                    if ipc::write_message(&mut *stream, &Message::Input(buffer[..length].to_vec()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs::File,
        os::fd::{FromRawFd, RawFd},
    };

    #[test]
    fn terminal_guard_restores_mode_and_outer_screen() {
        let (mut master, slave) = open_pty();
        let original = termios(slave.as_raw_fd());

        let guard = TerminalGuard::enter_on(slave.as_raw_fd(), slave.as_raw_fd()).unwrap();
        let mut entered = vec![0; ENTER_TERMINAL.len()];
        master.read_exact(&mut entered).unwrap();
        assert_eq!(entered, ENTER_TERMINAL);
        assert_eq!(termios(slave.as_raw_fd()).c_lflag & libc::ICANON, 0);

        drop(guard);
        let mut restored_output = vec![0; RESTORE_TERMINAL.len()];
        master.read_exact(&mut restored_output).unwrap();
        assert_eq!(restored_output, RESTORE_TERMINAL);
        let restored = termios(slave.as_raw_fd());
        assert_eq!(restored.c_iflag, original.c_iflag);
        assert_eq!(restored.c_oflag, original.c_oflag);
        assert_eq!(restored.c_cflag, original.c_cflag);
        assert_eq!(restored.c_lflag, original.c_lflag);
        assert_eq!(restored.c_cc, original.c_cc);
    }

    fn open_pty() -> (File, File) {
        let (mut master, mut slave): (RawFd, RawFd) = (-1, -1);
        // SAFETY: openpty initializes both descriptors; null optional arguments are permitted.
        assert_eq!(
            unsafe {
                libc::openpty(
                    &raw mut master,
                    &raw mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                )
            },
            0
        );
        // SAFETY: openpty returned two newly owned descriptors.
        unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) }
    }

    fn termios(fd: RawFd) -> libc::termios {
        // SAFETY: terminal is initialized by tcgetattr before it is read.
        let mut terminal = unsafe { std::mem::zeroed() };
        // SAFETY: fd is a live PTY descriptor and terminal is writable.
        assert_eq!(unsafe { libc::tcgetattr(fd, &raw mut terminal) }, 0);
        terminal
    }
}
