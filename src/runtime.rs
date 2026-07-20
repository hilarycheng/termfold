use std::{
    env, fs,
    io::ErrorKind,
    os::linux::fs::MetadataExt,
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
};

#[derive(Debug)]
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

    pub fn bind(&self, session: &str) -> Result<UnixListener, String> {
        if !valid_session_name(session) {
            return Err("session name must match [A-Za-z0-9_-]{1,64}".into());
        }

        let path = self.path.join(format!("{session}.sock"));
        match UnixListener::bind(&path) {
            Ok(listener) => secure_socket(listener, &path),
            Err(error) if error.kind() == ErrorKind::AddrInUse => {
                remove_stale_socket(&path, self.uid)?;
                UnixListener::bind(&path)
                    .map_err(|error| format!("cannot bind socket {}: {error}", path.display()))
                    .and_then(|listener| secure_socket(listener, &path))
            }
            Err(error) => Err(format!("cannot bind socket {}: {error}", path.display())),
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

fn secure_socket(listener: UnixListener, path: &Path) -> Result<UnixListener, String> {
    if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(0o600)) {
        let _ = fs::remove_file(path);
        return Err(format!("cannot secure socket {}: {error}", path.display()));
    }
    Ok(listener)
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
        let replacement = runtime.bind("work").unwrap();
        drop(replacement);
        fs::remove_file(socket).unwrap();
        fs::remove_dir(path).unwrap();
    }
}
