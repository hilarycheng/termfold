use std::{
    env,
    ffi::{OsStr, OsString},
    fs::{self, File},
    io,
    os::{
        fd::{AsRawFd, FromRawFd, RawFd},
        unix::{fs::PermissionsExt, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    ptr, thread,
    time::{Duration, Instant},
};

use crate::session::Size;

pub const TERMINATION_GRACE: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub struct LaunchContext {
    shell: OsString,
    working_directory: PathBuf,
    environment: Vec<(OsString, OsString)>,
    terminfo_root: PathBuf,
}

impl LaunchContext {
    pub fn capture(terminfo_root: PathBuf) -> io::Result<Self> {
        Ok(Self {
            shell: approved_shell(),
            working_directory: env::current_dir()?,
            environment: env::vars_os().collect(),
            terminfo_root,
        })
    }

    pub fn shell(&self) -> &OsStr {
        &self.shell
    }

    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }
}

#[derive(Debug)]
pub struct PtyChild {
    master: File,
    child: Child,
    process_group: libc::pid_t,
}

impl PtyChild {
    pub fn spawn(context: &LaunchContext, size: Size) -> io::Result<Self> {
        validate_size(size)?;
        let (master, slave) = open_pty(size)?;
        set_nonblocking(master.as_raw_fd())?;
        let stdin = slave.try_clone()?;
        let stdout = slave.try_clone()?;
        let mut command = Command::new(&context.shell);
        command
            .current_dir(&context.working_directory)
            .env_clear()
            .envs(context.environment.iter().cloned())
            .env("TERM", "termfold-256color")
            .env("COLORTERM", "truecolor")
            .env("TERMINFO", &context.terminfo_root)
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(slave));

        // SAFETY: after fork this closure calls only async-signal-safe libc operations,
        // creates a new session, and makes the already-open slave on stdin controlling.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY, 0) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let child = command.spawn()?;
        let process_group = libc::pid_t::try_from(child.id())
            .map_err(|_| io::Error::other("child process ID exceeds Linux pid_t"))?;
        Ok(Self {
            master,
            child,
            process_group,
        })
    }

    pub fn master(&mut self) -> &mut File {
        &mut self.master
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn resize(&self, size: Size) -> io::Result<()> {
        validate_size(size)?;
        set_size(self.master.as_raw_fd(), size)
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    fn signal(&self, signal: libc::c_int) -> io::Result<()> {
        // SAFETY: process_group is the positive PID returned for our child; negating it
        // targets only the session's process group.
        if unsafe { libc::kill(-self.process_group, signal) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(error)
        }
    }
}

impl Drop for PtyChild {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.signal(libc::SIGKILL);
            let _ = self.child.wait();
        }
    }
}

pub fn terminate_all(children: &mut [&mut PtyChild]) -> io::Result<()> {
    terminate_all_with_grace(children, TERMINATION_GRACE)
}

fn terminate_all_with_grace(children: &mut [&mut PtyChild], grace: Duration) -> io::Result<()> {
    let mut first_error = None;
    for child in children.iter_mut() {
        match child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => remember_error(&mut first_error, child.signal(libc::SIGTERM)),
            Err(error) => {
                remember_error(&mut first_error, Err(error));
                remember_error(&mut first_error, child.signal(libc::SIGTERM));
            }
        }
    }

    let deadline = Instant::now() + grace;
    loop {
        let mut running = false;
        for child in children.iter_mut() {
            match child.try_wait() {
                Ok(Some(_)) => {}
                Ok(None) => running = true,
                Err(error) => {
                    running = true;
                    remember_error(&mut first_error, Err(error));
                }
            }
        }
        if !running || Instant::now() >= deadline {
            break;
        }
        thread::sleep(
            Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())),
        );
    }

    for child in children.iter_mut() {
        match child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => {
                remember_error(&mut first_error, child.signal(libc::SIGKILL));
                remember_error(&mut first_error, child.child.wait().map(|_| ()));
            }
            Err(error) => {
                remember_error(&mut first_error, Err(error));
                remember_error(&mut first_error, child.signal(libc::SIGKILL));
                remember_error(&mut first_error, child.child.wait().map(|_| ()));
            }
        }
    }

    first_error.map_or(Ok(()), Err)
}

fn remember_error(first: &mut Option<io::Error>, result: io::Result<()>) {
    if let Err(error) = result
        && first.is_none()
    {
        *first = Some(error);
    }
}

fn approved_shell() -> OsString {
    env::var_os("SHELL")
        .filter(|shell| {
            let path = Path::new(shell);
            path.is_absolute()
                && fs::metadata(path).is_ok_and(|metadata| {
                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                })
        })
        .unwrap_or_else(|| OsString::from("/bin/sh"))
}

fn validate_size(size: Size) -> io::Result<()> {
    if size.columns == 0 || size.rows == 0 {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "PTY size must be non-zero",
        ))
    } else {
        Ok(())
    }
}

fn open_pty(size: Size) -> io::Result<(File, File)> {
    let mut master: RawFd = -1;
    let mut slave: RawFd = -1;
    let window = window_size(size);
    // SAFETY: pointers reference initialized storage, optional termios is null, and
    // successful descriptors are immediately owned by File.
    if unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            ptr::null_mut(),
            ptr::null(),
            &window,
        )
    } == -1
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openpty succeeded and returned two owned descriptors.
    let master = unsafe { File::from_raw_fd(master) };
    // SAFETY: openpty succeeded and returned two owned descriptors.
    let slave = unsafe { File::from_raw_fd(slave) };
    set_close_on_exec(master.as_raw_fd())?;
    Ok((master, slave))
}

fn set_close_on_exec(fd: RawFd) -> io::Result<()> {
    // SAFETY: fd is a live descriptor owned by the caller.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd remains live and flags came from F_GETFD.
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: fd is a live descriptor owned by the caller.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd remains live and flags came from F_GETFL.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn set_size(fd: RawFd, size: Size) -> io::Result<()> {
    let window = window_size(size);
    // SAFETY: fd is a PTY master and window points to a valid winsize value.
    if unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &window) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn window_size(size: Size) -> libc::winsize {
    libc::winsize {
        ws_row: size.rows,
        ws_col: size.columns,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn context() -> LaunchContext {
        LaunchContext {
            shell: "/bin/sh".into(),
            working_directory: "/tmp".into(),
            environment: vec![
                ("TERM".into(), "wrong".into()),
                ("COLORTERM".into(), "wrong".into()),
                ("TERMINFO".into(), "wrong".into()),
                ("TERMFOLD_TEST".into(), "inherited".into()),
            ],
            terminfo_root: "/tmp/terminfo".into(),
        }
    }

    fn read_until(child: &mut PtyChild, expected: &[u8]) -> io::Result<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut output = Vec::new();
        while Instant::now() < deadline {
            let mut buffer = [0; 256];
            match child.master().read(&mut buffer) {
                Ok(0) => break,
                Ok(length) => output.extend_from_slice(&buffer[..length]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                Err(error) => return Err(error),
            }
            if output
                .windows(expected.len())
                .any(|window| window == expected)
            {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        Ok(output)
    }

    #[test]
    fn shell_inherits_context_and_pty_resizes() {
        let mut child = PtyChild::spawn(
            &context(),
            Size {
                columns: 80,
                rows: 24,
            },
        )
        .unwrap();
        child
            .resize(Size {
                columns: 100,
                rows: 40,
            })
            .unwrap();
        child
            .master()
            .write_all(
                b"printf '%s|%s|%s|%s|%s|' \"$PWD\" \"$TERM\" \"$COLORTERM\" \"$TERMINFO\" \"$TERMFOLD_TEST\"; /bin/stty size; exit\n",
            )
            .unwrap();
        let expected = b"/tmp|termfold-256color|truecolor|/tmp/terminfo|inherited|40 100";
        let output = read_until(&mut child, expected).unwrap();
        assert!(
            output
                .windows(expected.len())
                .any(|window| window == expected)
        );
        while child.try_wait().unwrap().is_none() {
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn termination_escalates_and_reaps() {
        let mut child = PtyChild::spawn(
            &context(),
            Size {
                columns: 80,
                rows: 24,
            },
        )
        .unwrap();
        child
            .master()
            .write_all(
                b"trap '' TERM; printf '\\162\\145\\141\\144\\171'; while :; do sleep 1; done\n",
            )
            .unwrap();
        assert!(
            read_until(&mut child, b"ready")
                .unwrap()
                .ends_with(b"ready")
        );

        terminate_all_with_grace(&mut [&mut child], Duration::from_millis(20)).unwrap();
        assert!(child.try_wait().unwrap().is_some());
    }
}
