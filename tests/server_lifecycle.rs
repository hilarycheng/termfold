use std::{
    fs,
    io::{Read, Write},
    os::unix::{fs::DirBuilderExt, net::UnixStream},
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
    stream.write_all(&[0, 0, 0, 2, 1, 8]).ok()?;
    let mut prefix = [0; 4];
    stream.read_exact(&mut prefix).ok()?;
    if u32::from_be_bytes(prefix) != 10 {
        return None;
    }
    let mut body = [0; 10];
    stream.read_exact(&mut body).ok()?;
    (body[..2] == [1, 9]).then(|| u32::from_be_bytes(body[6..10].try_into().unwrap()))
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
