use std::{
    fs,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, RawFd},
        unix::{fs::DirBuilderExt, net::UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct TestRuntime {
    root: PathBuf,
}

impl TestRuntime {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "termfold-lifecycle-{}-{unique}",
            std::process::id()
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        Self { root }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_termfold"));
        command
            .env("XDG_RUNTIME_DIR", &self.root)
            .env("SHELL", "/bin/sh");
        command
    }

    fn socket(&self, name: &str) -> PathBuf {
        self.root.join("termfold").join(format!("{name}.sock"))
    }

    fn run(&self, arguments: &[&str]) -> std::process::Output {
        self.command()
            .args(arguments)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
}

impl Drop for TestRuntime {
    fn drop(&mut self) {
        for name in ["one", "two", "default"] {
            let _ = self
                .command()
                .args(["kill", name])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn sessions_support_multiple_instances_and_concurrent_clients() {
    let runtime = TestRuntime::new();
    assert!(runtime.run(&["new", "one"]).status.success());
    assert!(runtime.run(&["new", "two"]).status.success());

    let listing = runtime.run(&["list"]);
    assert!(listing.status.success());
    let listing = String::from_utf8(listing.stdout).unwrap();
    assert!(listing.lines().any(|line| line.ends_with(" one detached")));
    assert!(listing.lines().any(|line| line.ends_with(" two detached")));
    let one_pid = listing
        .lines()
        .find(|line| line.ends_with(" one detached"))
        .unwrap()
        .split_once(' ')
        .unwrap()
        .0;
    assert!(runtime.run(&[one_pid]).status.success());

    let duplicate = runtime.run(&["new", "one"]);
    assert!(!duplicate.status.success());
    assert!(
        String::from_utf8(duplicate.stderr)
            .unwrap()
            .contains("session 'one' already exists")
    );

    let mut first = attached_client(&runtime, "one");
    let mut second = attached_client(&runtime, "one");
    wait_for_attached_count(&runtime.socket("one"), 2);

    drop(first.stdin.take());
    drop(second.stdin.take());
    wait_for_exit(&mut first);
    wait_for_exit(&mut second);

    assert!(runtime.run(&["kill", "one"]).status.success());
    assert!(runtime.run(&["kill", "two"]).status.success());
    wait_for_missing(&runtime.socket("one"));
    wait_for_missing(&runtime.socket("two"));
}

#[test]
fn initial_child_exit_removes_the_empty_session() {
    let runtime = TestRuntime::new();
    let status = runtime
        .command()
        .args(["--server", "gone", "80", "24"])
        .env("SHELL", "/bin/true")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(!runtime.socket("gone").exists());
}

#[test]
fn termination_signal_restores_the_client_terminal() {
    let runtime = TestRuntime::new();
    assert!(runtime.run(&["new", "one"]).status.success());
    let (master, slave) = open_pty();
    let original = termios(slave.as_raw_fd());
    let mut client = runtime
        .command()
        .args(["attach", "one"])
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for_attached_count(&runtime.socket("one"), 1);
    wait_for_raw(slave.as_raw_fd());

    // SAFETY: client.id() names the live child process created above.
    assert_eq!(
        unsafe { libc::kill(client.id() as libc::pid_t, libc::SIGTERM) },
        0
    );
    wait_for_exit(&mut client);
    assert_eq!(client.wait().unwrap().code(), Some(128 + libc::SIGTERM));
    assert_same_termios(termios(slave.as_raw_fd()), original);
    drop(master);
}

#[test]
fn prefix_commands_create_tabs_report_errors_and_detach() {
    let runtime = TestRuntime::new();
    assert!(runtime.run(&["new", "one"]).status.success());
    let mut stream = UnixStream::connect(runtime.socket("one")).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .write_all(&[0, 0, 0, 8, 2, 1, 0, 80, 0, 24, 5, 3])
        .unwrap();
    while read_frame(&mut stream).unwrap().0 != 5 {}

    send_input(&mut stream, b"\x02c");
    wait_for_screen(&mut stream, b"[2:shell]");
    send_input(&mut stream, b"\x02?");
    wait_for_screen(&mut stream, b"unsupported prefix command");
    send_input(&mut stream, b"\x02d");
    assert!(read_frame(&mut stream).is_none());
}

fn attached_client(runtime: &TestRuntime, name: &str) -> Child {
    runtime
        .command()
        .args(["attach", name])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn send_input(stream: &mut UnixStream, bytes: &[u8]) {
    let length = u32::try_from(bytes.len() + 2).unwrap();
    stream.write_all(&length.to_be_bytes()).unwrap();
    stream.write_all(&[2, 3]).unwrap();
    stream.write_all(bytes).unwrap();
}

fn wait_for_screen(stream: &mut UnixStream, expected: &[u8]) {
    loop {
        let (kind, payload) = read_frame(stream).expect("session disconnected");
        if kind == 6
            && payload
                .windows(expected.len())
                .any(|window| window == expected)
        {
            return;
        }
    }
}

fn read_frame(stream: &mut UnixStream) -> Option<(u8, Vec<u8>)> {
    let mut prefix = [0; 4];
    match stream.read_exact(&mut prefix) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return None,
        Err(error) => panic!("cannot read session frame: {error}"),
    }
    let mut body = vec![0; u32::from_be_bytes(prefix) as usize];
    stream.read_exact(&mut body).unwrap();
    Some((body[1], body[2..].to_vec()))
}

fn wait_for_attached_count(path: &Path, expected: u32) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if status_count(path) == Some(expected) {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("session did not reach {expected} attached clients");
}

fn status_count(path: &Path) -> Option<u32> {
    let mut stream = UnixStream::connect(path).ok()?;
    stream.write_all(&[0, 0, 0, 2, 2, 8]).ok()?;
    let mut prefix = [0; 4];
    stream.read_exact(&mut prefix).ok()?;
    if u32::from_be_bytes(prefix) != 10 {
        return None;
    }
    let mut body = [0; 10];
    stream.read_exact(&mut body).ok()?;
    (body[..2] == [2, 9]).then(|| u32::from_be_bytes(body[6..10].try_into().unwrap()))
}

fn wait_for_exit(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("attached client did not detach");
}

fn wait_for_missing(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if !path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("session socket {} was not removed", path.display());
}

fn open_pty() -> (fs::File, fs::File) {
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
    unsafe { (fs::File::from_raw_fd(master), fs::File::from_raw_fd(slave)) }
}

fn wait_for_raw(fd: RawFd) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if termios(fd).c_lflag & libc::ICANON == 0 {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("client did not enable raw terminal mode");
}

fn termios(fd: RawFd) -> libc::termios {
    // SAFETY: terminal is initialized by tcgetattr before it is read.
    let mut terminal = unsafe { std::mem::zeroed() };
    // SAFETY: fd is a live PTY descriptor and terminal is writable.
    assert_eq!(unsafe { libc::tcgetattr(fd, &raw mut terminal) }, 0);
    terminal
}

fn assert_same_termios(actual: libc::termios, expected: libc::termios) {
    assert_eq!(actual.c_iflag, expected.c_iflag);
    assert_eq!(actual.c_oflag, expected.c_oflag);
    assert_eq!(actual.c_cflag, expected.c_cflag);
    assert_eq!(actual.c_lflag, expected.c_lflag);
    assert_eq!(actual.c_cc, expected.c_cc);
}
