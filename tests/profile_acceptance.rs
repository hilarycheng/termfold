#![cfg(target_os = "linux")]

use std::{
    fs,
    os::unix::{fs::PermissionsExt, net::UnixListener},
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
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!(".termfold-profile-{}-{unique}", std::process::id()));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(root.join("config")).unwrap();
        fs::create_dir(root.join("work")).unwrap();
        fs::create_dir(root.join("override")).unwrap();
        Self { root }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_termfold"));
        command
            .env("XDG_RUNTIME_DIR", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("SHELL", "/bin/sh")
            .env("TERM", "xterm-256color");
        command
    }

    fn socket(&self, name: &str) -> PathBuf {
        self.root.join("termfold").join(format!("{name}.sock"))
    }

    fn config_path(&self) -> PathBuf {
        self.root.join("config/termfold/config.toml")
    }

    fn write_config(&self, source: &str) {
        fs::create_dir_all(self.root.join("config/termfold")).unwrap();
        fs::write(self.config_path(), source).unwrap();
    }

    fn run(&self, arguments: &[&str]) -> std::process::Output {
        self.command()
            .args(arguments)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn stop(&self, name: &str) {
        let output = self.run(&["kill", "--yes", name]);
        assert!(
            output.status.success(),
            "failed to stop {name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        wait_for_missing(&self.socket(name));
    }
}

impl Drop for TestRuntime {
    fn drop(&mut self) {
        for name in [
            "no-config",
            "absent-default",
            "default-selection",
            "named-selection",
            "explicit-none",
            "reusable",
        ] {
            let _ = self
                .command()
                .args(["kill", "--yes", name])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn startup_profiles_are_accepted_atomically_and_attach_does_not_relaunch() {
    let runtime = TestRuntime::new();

    let mut no_config = start_server(&runtime, "no-config", None);
    assert!(runtime.socket("no-config").exists());
    runtime.stop("no-config");
    wait_for_exit(&mut no_config);

    let work = runtime.root.join("work");
    let override_dir = runtime.root.join("override");
    runtime.write_config(&format!(
        "[profiles.named]\n\ndirectory = \"{}\"\ntabs = [{{ shell = true }}]\n\n[profiles.broken]\ndirectory = \"{}\"\ntabs = [{{ shell = true }}]\n\n[profiles.mid]\ndirectory = \"{}\"\ntabs = [{{ split = \"left-right\", panes = [{{ command = [\"/bin/sh\", \"-c\", \"printf '%s' $$ > rollback-pid; sleep 30\"] }}, {{ shell = true }}] }}]\n",
        work.display(),
        runtime.root.join("missing").display(),
        work.display(),
    ));
    assert!(runtime.run(&["new", "absent-default"]).status.success());
    runtime.stop("absent-default");

    runtime.write_config(&format!(
        "[profiles.default]\ndirectory = \"{}\"\ntabs = [{{ split = \"left-right\", panes = [{{ command = [\"/bin/sh\", \"-c\", \"printf 'DIRECT literal value:%s' \\\"$PWD\\\" > launch-marker; sleep 30\"] }}, {{ split = \"top-bottom\", panes = [{{ shell = true }}, {{ command = [\"/bin/sh\", \"-c\", \"pwd > override-marker; sleep 30\"], directory = \"{}\" }}] }}] }}, {{ shell = true }}]\n\n[profiles.named]\ndirectory = \"{}\"\ntabs = [{{ split = \"left-right\", panes = [{{ command = [\"/bin/sh\", \"-c\", \"printf 'NAMED literal value' > named-marker; sleep 30\"] }}, {{ shell = true }}] }}]\n\n[profiles.broken]\ndirectory = \"{}\"\ntabs = [{{ shell = true }}]\n\n[profiles.mid]\ndirectory = \"{}\"\ntabs = [{{ split = \"left-right\", panes = [{{ command = [\"/bin/sh\", \"-c\", \"printf '%s' $$ > rollback-pid; sleep 30\"] }}, {{ shell = true }}] }}]\n",
        work.display(),
        override_dir.display(),
        work.display(),
        runtime.root.join("missing").display(),
        work.display(),
    ));

    let default_output = runtime.run(&["new", "default-selection"]);
    assert!(default_output.status.success());
    assert_eq!(
        fs::read_to_string(work.join("launch-marker")).unwrap(),
        format!("DIRECT literal value:{}", work.display())
    );
    let attach_output = runtime.run(&["attach", "default-selection"]);
    assert!(attach_output.status.success());
    assert_eq!(
        fs::read_to_string(override_dir.join("override-marker")).unwrap(),
        format!("{}\n", override_dir.display())
    );
    assert_eq!(
        fs::read_to_string(work.join("launch-marker")).unwrap(),
        format!("DIRECT literal value:{}", work.display())
    );
    runtime.stop("default-selection");

    let named_new = runtime.run(&["new", "named-selection", "--profile", "named"]);
    assert!(named_new.status.success());
    let named_output = runtime.run(&["attach", "named-selection"]);
    assert!(named_output.status.success());
    assert_eq!(
        fs::read_to_string(work.join("named-marker")).unwrap(),
        "NAMED literal value"
    );
    runtime.stop("named-selection");

    assert!(
        runtime
            .run(&["new", "explicit-none", "--no-profile"])
            .status
            .success()
    );
    assert!(runtime.run(&["attach", "explicit-none"]).status.success());
    runtime.stop("explicit-none");

    let invalid = runtime.run(&["new", "reusable", "--profile", "broken"]);
    assert!(!invalid.status.success());
    assert!(!runtime.socket("reusable").exists());
    assert!(
        runtime
            .run(&["new", "reusable", "--no-profile"])
            .status
            .success()
    );
    runtime.stop("reusable");

    let runtime_path = runtime.root.join("termfold");
    fs::create_dir_all(&runtime_path).unwrap();
    let occupied_path = runtime.socket("mid-rollback");
    let listener = UnixListener::bind(&occupied_path).unwrap();
    let mut rollback = spawn_server(&runtime, "mid-rollback", Some("mid"));
    wait_for_exit(&mut rollback);
    assert!(!rollback.try_wait().unwrap().is_none());
    let pid = wait_for_pid(&work.join("rollback-pid"));
    assert!(!Path::new("/proc").join(pid.to_string()).exists());
    assert!(occupied_path.exists());
    drop(listener);
    fs::remove_file(occupied_path).unwrap();
}

#[allow(clippy::zombie_processes)]
fn start_server(runtime: &TestRuntime, name: &str, profile: Option<&str>) -> Child {
    let mut child = spawn_server(runtime, name, profile);
    let socket = runtime.socket(name);
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if socket.exists() {
            return child;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("server exited before startup: {status}");
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("server socket did not appear: {}", socket.display());
}

fn spawn_server(runtime: &TestRuntime, name: &str, profile: Option<&str>) -> Child {
    let mut command = runtime.command();
    command
        .args(["--server", name, "80", "24"])
        .args(profile.map_or(&["--no-profile"][..], |_| &["--profile"][..]));
    if let Some(profile) = profile {
        command.arg(profile);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
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

fn wait_for_exit(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    panic!("server did not exit after rollback");
}

fn wait_for_pid(path: &Path) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if let Ok(value) = fs::read_to_string(path)
            && let Ok(pid) = value.parse()
        {
            return pid;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("rollback target did not write its pid");
}
