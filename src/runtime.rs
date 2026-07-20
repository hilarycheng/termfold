use std::{
    env, fs,
    fs::{File, OpenOptions},
    io::ErrorKind,
    os::fd::AsRawFd,
    os::linux::fs::MetadataExt,
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct RuntimeDir {
    path: PathBuf,
    uid: u32,
}

impl RuntimeDir {
    pub fn discover() -> Result<Self, String> {
        let uid = current_uid()?;
        let root = env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute() && valid_parent(path, uid))
            .map(|path| path.join("termfold"))
            .unwrap_or_else(|| PathBuf::from(format!("/tmp/termfold-{uid}")));

        ensure_private_dir(&root, uid)?;
        Ok(Self { path: root, uid })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn bind(&self, session: &str) -> Result<SessionSocket, String> {
        if !valid_session_name(session) {
            return Err("session name must match [A-Za-z0-9_-]{1,64}".into());
        }

        let path = self.session_path(session);
        match UnixListener::bind(&path) {
            Ok(listener) => secure_socket(listener, path, self.uid),
            Err(error) if error.kind() == ErrorKind::AddrInUse => {
                remove_stale_socket(&path, self.uid)?;
                UnixListener::bind(&path)
                    .map_err(|error| format!("cannot bind socket {}: {error}", path.display()))
                    .and_then(|listener| secure_socket(listener, path, self.uid))
            }
            Err(error) => Err(format!("cannot bind socket {}: {error}", path.display())),
        }
    }

    pub fn connect(&self, session: &str) -> Result<UnixStream, String> {
        if !valid_session_name(session) {
            return Err("session name must match [A-Za-z0-9_-]{1,64}".into());
        }
        let path = self.session_path(session);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot inspect session '{session}': {error}"))?;
        if !metadata.file_type().is_socket() || metadata.st_uid() != self.uid {
            return Err(format!(
                "session '{session}' does not use a socket owned by the current user"
            ));
        }
        let stream = UnixStream::connect(&path)
            .map_err(|error| format!("cannot connect to session '{session}': {error}"))?;
        if peer_uid(&stream)? != self.uid {
            return Err(format!("session '{session}' belongs to another user"));
        }
        Ok(stream)
    }

    pub fn session_names(&self) -> Result<Vec<String>, String> {
        let entries = fs::read_dir(&self.path).map_err(|error| {
            format!(
                "cannot read runtime directory {}: {error}",
                self.path.display()
            )
        })?;
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!(
                    "cannot read runtime directory {}: {error}",
                    self.path.display()
                )
            })?;
            let Some(file_name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(name) = file_name.strip_suffix(".sock") else {
                continue;
            };
            if !valid_session_name(name) {
                continue;
            }
            let metadata = match fs::symlink_metadata(entry.path()) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => return Err(format!("cannot inspect session '{name}': {error}")),
            };
            if metadata.file_type().is_socket() && metadata.st_uid() == self.uid {
                names.push(name.to_owned());
            }
        }
        names.sort();
        Ok(names)
    }

    pub fn lock_creation(&self) -> Result<CreationLock, String> {
        let path = self.path.join("create.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&path)
            .map_err(|error| format!("cannot open session creation lock: {error}"))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("cannot secure session creation lock: {error}"))?;
        let metadata = file
            .metadata()
            .map_err(|error| format!("cannot inspect session creation lock: {error}"))?;
        if !metadata.file_type().is_file()
            || metadata.st_uid() != self.uid
            || metadata.st_mode() & 0o777 != 0o600
        {
            return Err("session creation lock must be a regular file owned by the current user with mode 0600".into());
        }
        // SAFETY: the descriptor is live and LOCK_EX is a valid flock operation.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == -1 {
            return Err(format!(
                "cannot lock session creation: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(CreationLock(file))
    }

    pub fn uid(&self) -> u32 {
        self.uid
    }

    fn session_path(&self, session: &str) -> PathBuf {
        self.path.join(format!("{session}.sock"))
    }
}

#[derive(Debug)]
pub struct CreationLock(File);

impl Drop for CreationLock {
    fn drop(&mut self) {
        // SAFETY: the descriptor is live and was locked by lock_creation.
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[derive(Debug)]
pub struct SessionSocket {
    listener: UnixListener,
    path: PathBuf,
    uid: u32,
    device: u64,
    inode: u64,
}

impl SessionSocket {
    pub fn listener(&self) -> &UnixListener {
        &self.listener
    }
}

impl Drop for SessionSocket {
    fn drop(&mut self) {
        let Ok(metadata) = fs::symlink_metadata(&self.path) else {
            return;
        };
        if metadata.file_type().is_socket()
            && metadata.st_uid() == self.uid
            && metadata.st_dev() == self.device
            && metadata.st_ino() == self.inode
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn current_uid() -> Result<u32, String> {
    fs::metadata("/proc/self")
        .map(|metadata| metadata.st_uid())
        .map_err(|error| format!("cannot determine current user: {error}"))
}

fn valid_parent(path: &Path, uid: u32) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.file_type().is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.st_uid() == uid
            && metadata.st_mode() & 0o022 == 0
    })
}

fn ensure_private_dir(path: &Path, uid: u32) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.file_type().is_dir()
                && !metadata.file_type().is_symlink()
                && metadata.st_uid() == uid
                && metadata.st_mode() & 0o777 == 0o700 =>
        {
            Ok(())
        }
        Ok(_) => Err(format!(
            "runtime path {} must be a real directory owned by the current user with mode 0700",
            path.display()
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(path)
                .map_err(|error| {
                    format!(
                        "cannot create runtime directory {}: {error}",
                        path.display()
                    )
                })?;
            ensure_private_dir(path, uid)
        }
        Err(error) => Err(format!(
            "cannot inspect runtime path {}: {error}",
            path.display()
        )),
    }
}

fn secure_socket(listener: UnixListener, path: PathBuf, uid: u32) -> Result<SessionSocket, String> {
    if let Err(error) = fs::set_permissions(&path, fs::Permissions::from_mode(0o600)) {
        let _ = fs::remove_file(&path);
        return Err(format!("cannot secure socket {}: {error}", path.display()));
    }
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) => {
            let _ = fs::remove_file(&path);
            return Err(format!("cannot inspect socket {}: {error}", path.display()));
        }
    };
    Ok(SessionSocket {
        listener,
        path,
        uid,
        device: metadata.st_dev(),
        inode: metadata.st_ino(),
    })
}

pub fn peer_uid(stream: &UnixStream) -> Result<u32, String> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: credentials and length point to writable storage of the declared size,
    // and stream owns a live Unix-domain socket descriptor.
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut credentials).cast::<libc::c_void>(),
            &raw mut length,
        )
    } == -1
    {
        return Err(format!(
            "cannot inspect peer credentials: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(credentials.uid)
}

fn remove_stale_socket(path: &Path, uid: u32) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect socket {}: {error}", path.display()))?;
    if !metadata.file_type().is_socket() || metadata.st_uid() != uid {
        return Err(format!(
            "refusing to remove {}: not a socket owned by the current user",
            path.display()
        ));
    }

    match UnixStream::connect(path) {
        Ok(_) => {
            return Err(format!(
                "session socket {} is already active",
                path.display()
            ));
        }
        Err(error) if error.kind() == ErrorKind::ConnectionRefused => {}
        Err(error) => {
            return Err(format!(
                "cannot prove socket {} is stale: {error}",
                path.display()
            ));
        }
    }

    let current = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot recheck socket {}: {error}", path.display()))?;
    if !current.file_type().is_socket()
        || current.st_uid() != uid
        || current.st_dev() != metadata.st_dev()
        || current.st_ino() != metadata.st_ino()
    {
        return Err(format!(
            "socket {} changed during stale check",
            path.display()
        ));
    }
    fs::remove_file(path)
        .map_err(|error| format!("cannot remove stale socket {}: {error}", path.display()))
}

fn valid_session_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn runtime_directory_and_socket_are_private_and_stale_socket_is_replaced() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!("termfold-test-{}-{unique}", std::process::id()));
        let uid = current_uid().unwrap();
        ensure_private_dir(&path, uid).unwrap();
        assert_eq!(
            fs::symlink_metadata(&path).unwrap().st_mode() & 0o777,
            0o700
        );

        let runtime = RuntimeDir {
            path: path.clone(),
            uid,
        };
        let listener = runtime.bind("work").unwrap();
        let socket = path.join("work.sock");
        assert_eq!(
            fs::symlink_metadata(&socket).unwrap().st_mode() & 0o777,
            0o600
        );
        assert!(runtime.bind("work").is_err());

        drop(listener);
        let stale = UnixListener::bind(&socket).unwrap();
        drop(stale);
        let replacement = runtime.bind("work").unwrap();
        drop(replacement);
        fs::remove_dir(path).unwrap();
    }
}
