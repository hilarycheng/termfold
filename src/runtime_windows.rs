use std::{
    env, fs,
    fs::{File, OpenOptions},
    io::{self, ErrorKind, Read, Write},
    os::windows::{
        ffi::OsStrExt,
        fs::MetadataExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Path, PathBuf},
    ptr,
    sync::Mutex,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, ERROR_NO_DATA,
        ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE,
        INVALID_HANDLE_VALUE, LocalFree, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        },
        DACL_SECURITY_INFORMATION, GetTokenInformation, PROTECTED_DACL_SECURITY_INFORMATION,
        SECURITY_ATTRIBUTES, SetFileSecurityW, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, MOVEFILE_REPLACE_EXISTING,
        MOVEFILE_WRITE_THROUGH, MoveFileExW, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile,
        WriteFile,
    },
    System::{
        IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
            GetNamedPipeServerProcessId, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, SetNamedPipeHandleState,
            WaitNamedPipeW,
        },
        Threading::{
            CreateEventW, CreateMutexW, GetCurrentProcess, INFINITE, OpenProcess, OpenProcessToken,
            PROCESS_QUERY_LIMITED_INFORMATION, ReleaseMutex, WaitForSingleObject,
        },
    },
};

const TERMINFO_ENTRY: &[u8] = include_bytes!("../terminfo/compiled/t/termfold-256color");
const PIPE_BUFFER_SIZE: u32 = 64 * 1024;

#[derive(Debug)]
pub struct ClientStream {
    file: File,
    read_timeout: Option<Duration>,
}

impl ClientStream {
    fn new(file: File) -> Self {
        Self {
            file,
            read_timeout: None,
        }
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            file: self.file.try_clone()?,
            read_timeout: self.read_timeout,
        })
    }

    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.read_timeout = timeout;
        Ok(())
    }

    pub fn shutdown(&self) -> io::Result<()> {
        // SAFETY: file owns a live named-pipe handle. DisconnectNamedPipe is valid
        // only for the server end; client-side failure is harmless during shutdown.
        if unsafe { DisconnectNamedPipe(self.file.as_raw_handle() as HANDLE) } == 0 {
            let error = io::Error::last_os_error();
            if !matches!(
                error.raw_os_error(),
                Some(code) if code == ERROR_BROKEN_PIPE as i32 || code == ERROR_NO_DATA as i32
            ) {
                return Err(error);
            }
        }
        Ok(())
    }
}

impl Read for ClientStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let handle = self.file.as_raw_handle() as HANDLE;
        let length = buffer.len().min(u32::MAX as usize) as u32;
        let result = overlapped_io(handle, self.read_timeout, |overlapped| {
            // SAFETY: buffer and overlapped remain live until the operation completes.
            unsafe {
                ReadFile(
                    handle,
                    buffer.as_mut_ptr(),
                    length,
                    ptr::null_mut(),
                    overlapped,
                )
            }
        });
        match result {
            Ok(read) => Ok(read as usize),
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(code)
                        if code == ERROR_BROKEN_PIPE as i32 || code == ERROR_NO_DATA as i32
                ) =>
            {
                Ok(0)
            }
            Err(error) => Err(error),
        }
    }
}

impl Write for ClientStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let handle = self.file.as_raw_handle() as HANDLE;
        let length = buffer.len().min(u32::MAX as usize) as u32;
        overlapped_io(handle, None, |overlapped| {
            // SAFETY: buffer and overlapped remain live until the operation completes.
            unsafe { WriteFile(handle, buffer.as_ptr(), length, ptr::null_mut(), overlapped) }
        })
        .map(|written| written as usize)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct PendingIo {
    handle: HANDLE,
    state: Box<OVERLAPPED>,
    event: HANDLE,
    pending: bool,
}

impl PendingIo {
    fn new(handle: HANDLE) -> io::Result<Self> {
        // SAFETY: default security, unnamed event, and valid Boolean flags.
        let event = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut state = Box::new(OVERLAPPED::default());
        state.hEvent = event;
        Ok(Self {
            handle,
            state,
            event,
            pending: false,
        })
    }

    fn start(&mut self, result: i32) -> io::Result<()> {
        self.pending = true;
        if result != 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_IO_PENDING as i32) {
            Ok(())
        } else {
            self.pending = false;
            Err(error)
        }
    }

    fn wait(&mut self, timeout: u32) -> io::Result<Option<u32>> {
        // SAFETY: event remains live while the operation references it.
        match unsafe { WaitForSingleObject(self.event, timeout) } {
            WAIT_OBJECT_0 => {
                let mut transferred = 0;
                // SAFETY: state belongs to this completed operation.
                let result = unsafe {
                    GetOverlappedResult(self.handle, &*self.state, &raw mut transferred, 0)
                };
                self.pending = false;
                if result == 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(Some(transferred))
                }
            }
            WAIT_TIMEOUT => Ok(None),
            _ => Err(io::Error::last_os_error()),
        }
    }

    fn as_mut_ptr(&mut self) -> *mut OVERLAPPED {
        &raw mut *self.state
    }
}

impl Drop for PendingIo {
    fn drop(&mut self) {
        if self.pending {
            // SAFETY: handle and state remain live through cancellation completion.
            unsafe {
                CancelIoEx(self.handle, &*self.state);
                WaitForSingleObject(self.event, INFINITE);
            }
        }
        // SAFETY: event is no longer referenced by an outstanding operation.
        unsafe {
            CloseHandle(self.event);
        }
    }
}

fn overlapped_io(
    handle: HANDLE,
    timeout: Option<Duration>,
    operation: impl FnOnce(*mut OVERLAPPED) -> i32,
) -> io::Result<u32> {
    let mut pending = PendingIo::new(handle)?;
    let state = pending.as_mut_ptr();
    pending.start(operation(state))?;
    let timeout = timeout
        .map(|duration| duration.as_millis().min(u128::from(u32::MAX)) as u32)
        .unwrap_or(INFINITE);
    pending
        .wait(timeout)?
        .ok_or_else(|| io::Error::new(ErrorKind::TimedOut, "session read timed out"))
}

#[derive(Clone, Debug)]
pub struct RuntimeDir {
    path: PathBuf,
    sid: String,
}

impl RuntimeDir {
    pub fn discover() -> Result<Self, String> {
        let sid = current_sid()?;
        let path = env::var_os("LOCALAPPDATA")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .map(|path| path.join("Termfold").join("runtime"))
            .unwrap_or_else(|| env::temp_dir().join(format!("Termfold-{sid}-runtime")));
        ensure_private_dir(&path, &sid)?;
        Ok(Self { path, sid })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn bind(&self, session: &str) -> Result<SessionSocket, String> {
        validate_session_name(session)?;
        let pipe_name = self.pipe_name(session);
        let listener = SessionListener::new(pipe_name, self.sid.clone())?;
        let marker = self.path.join(format!("{session}.session"));
        let marker_value = claim_marker(&marker)?;
        Ok(SessionSocket {
            listener,
            marker,
            marker_value,
        })
    }

    pub fn connect(&self, session: &str) -> Result<ClientStream, String> {
        validate_session_name(session)?;
        let name = wide(self.pipe_name(session));
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            // SAFETY: name is a terminated pipe name and all optional pointers are null.
            let handle = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    ptr::null(),
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
                    ptr::null_mut(),
                )
            };
            if handle != INVALID_HANDLE_VALUE {
                if let Err(error) = verify_pipe_peer(handle, &self.sid, true) {
                    // SAFETY: handle was returned by CreateFileW and is not yet owned.
                    unsafe {
                        CloseHandle(handle);
                    }
                    return Err(format!(
                        "session '{session}' is owned by another user: {error}"
                    ));
                }
                let mode = PIPE_READMODE_BYTE | PIPE_WAIT;
                // SAFETY: handle is a connected named-pipe client handle.
                if unsafe { SetNamedPipeHandleState(handle, &mode, ptr::null(), ptr::null()) } == 0
                {
                    let error = io::Error::last_os_error();
                    // SAFETY: handle was returned by CreateFileW and is not yet owned.
                    unsafe {
                        CloseHandle(handle);
                    }
                    return Err(format!(
                        "cannot configure session '{session}' connection: {error}"
                    ));
                }
                // SAFETY: handle is uniquely owned and transferred to File.
                let file = unsafe { File::from_raw_handle(handle as _) };
                return Ok(ClientStream::new(file));
            }

            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32) {
                return Err(format!("cannot connect to session '{session}': {error}"));
            }
            if !matches!(
                error.raw_os_error(),
                Some(code) if code == ERROR_PIPE_BUSY as i32
            ) || Instant::now() >= deadline
            {
                return Err(format!("cannot connect to session '{session}': {error}"));
            }
            // SAFETY: name is valid and the short wait bounds connection startup.
            if unsafe { WaitNamedPipeW(name.as_ptr(), 50) } == 0 {
                thread::sleep(Duration::from_millis(10));
            }
        }
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
            let Some(name) = file_name.strip_suffix(".session") else {
                continue;
            };
            if valid_session_name(name)
                && entry.metadata().is_ok_and(|metadata| {
                    metadata.is_file()
                        && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
                })
            {
                names.push(name.to_owned());
            }
        }
        names.sort();
        Ok(names)
    }

    pub fn lock_creation(&self) -> Result<CreationLock, String> {
        let descriptor = SecurityDescriptor::for_user(&self.sid, false)?;
        let attributes = descriptor.attributes();
        let name = wide(format!("Local\\termfold-{}-create", self.sid));
        // SAFETY: attributes and name remain live for the call.
        let handle = unsafe { CreateMutexW(&attributes, 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(format!(
                "cannot open session creation lock: {}",
                io::Error::last_os_error()
            ));
        }
        // SAFETY: handle is a live mutex handle.
        let wait = unsafe { WaitForSingleObject(handle, INFINITE) };
        if wait != WAIT_OBJECT_0 && wait != WAIT_ABANDONED {
            // SAFETY: handle is live and not otherwise owned.
            unsafe {
                CloseHandle(handle);
            }
            return Err(format!(
                "cannot lock session creation: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(CreationLock(handle))
    }

    pub fn materialize_terminfo(&self) -> Result<PathBuf, String> {
        let root = self.path.join("terminfo");
        let entries = root.join("t");
        ensure_private_dir(&entries, &self.sid)?;
        let target = entries.join("termfold-256color");
        if fs::read(&target).ok().as_deref() == Some(TERMINFO_ENTRY) {
            return Ok(root);
        }

        let temporary = entries.join(format!(
            ".termfold-256color.{}.{}.tmp",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| format!("cannot create private terminfo entry: {error}"))?;
        let result = file
            .write_all(TERMINFO_ENTRY)
            .and_then(|()| file.sync_all());
        drop(file);
        if result.is_ok() {
            match fs::symlink_metadata(&target) {
                Ok(metadata)
                    if metadata.is_file()
                        && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 => {}
                Ok(_) => {
                    let _ = fs::remove_file(&temporary);
                    return Err("private terminfo entry is not a regular file".into());
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => {
                    let _ = fs::remove_file(&temporary);
                    return Err(format!("cannot inspect private terminfo entry: {error}"));
                }
            }
        }
        let result = result.and_then(|()| replace_file(&temporary, &target));
        let _ = fs::remove_file(&temporary);
        result.map_err(|error| format!("cannot materialize private terminfo entry: {error}"))?;
        if fs::read(&target).ok().as_deref() != Some(TERMINFO_ENTRY) {
            return Err(format!(
                "private terminfo entry {} does not match this Termfold binary",
                target.display()
            ));
        }
        Ok(root)
    }

    fn pipe_name(&self, session: &str) -> String {
        format!(r"\\.\pipe\termfold-{}-{session}", self.sid)
    }
}

#[derive(Debug)]
pub struct CreationLock(HANDLE);

impl Drop for CreationLock {
    fn drop(&mut self) {
        // SAFETY: handle is the mutex acquired by lock_creation.
        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}

pub struct SessionSocket {
    listener: SessionListener,
    marker: PathBuf,
    marker_value: String,
}

impl SessionSocket {
    pub fn set_nonblocking(&self, _nonblocking: bool) -> io::Result<()> {
        Ok(())
    }

    pub fn accept(&self) -> io::Result<ClientStream> {
        self.listener.accept()
    }
}

impl Drop for SessionSocket {
    fn drop(&mut self) {
        if fs::read_to_string(&self.marker).ok().as_deref() == Some(self.marker_value.as_str()) {
            let _ = fs::remove_file(&self.marker);
        }
    }
}

struct SessionListener {
    pipe_name: String,
    sid: String,
    pending: Mutex<Option<PendingPipe>>,
}

impl SessionListener {
    fn new(pipe_name: String, sid: String) -> Result<Self, String> {
        let pending = create_pipe(&pipe_name, &sid, true)
            .map_err(|error| format!("cannot bind session pipe: {error}"))?;
        Ok(Self {
            pipe_name,
            sid,
            pending: Mutex::new(Some(pending)),
        })
    }

    fn accept(&self) -> io::Result<ClientStream> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| io::Error::other("session listener lock is poisoned"))?;
        if pending.is_none() {
            *pending = Some(create_pipe(&self.pipe_name, &self.sid, false)?);
        }
        let handle = pending
            .as_ref()
            .expect("pending pipe is initialized")
            .handle();
        let pipe = pending.as_mut().expect("pending pipe is initialized");
        if pipe.connection.is_none() {
            let mut connection = PendingIo::new(handle)?;
            let state = connection.as_mut_ptr();
            // SAFETY: handle is a listening pipe and state remains live while pending.
            let result = unsafe { ConnectNamedPipe(handle, state) };
            if result == 0
                && io::Error::last_os_error().raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32)
            {
                connection.pending = false;
            } else {
                connection.start(result)?;
                pipe.connection = Some(connection);
            }
        }
        if let Some(connection) = &mut pipe.connection
            && connection.wait(0)?.is_none()
        {
            return Err(io::Error::new(ErrorKind::WouldBlock, "no pending client"));
        }

        let mode = PIPE_READMODE_BYTE | PIPE_WAIT;
        // SAFETY: handle is now a connected named-pipe server handle.
        if unsafe { SetNamedPipeHandleState(handle, &mode, ptr::null(), ptr::null()) } == 0 {
            let error = io::Error::last_os_error();
            // SAFETY: handle is the connected server end of the pending pipe.
            unsafe {
                DisconnectNamedPipe(handle);
            }
            return Err(error);
        }
        if let Err(error) = verify_pipe_peer(handle, &self.sid, false) {
            // SAFETY: handle is the connected server end of the pending pipe.
            unsafe {
                DisconnectNamedPipe(handle);
            }
            return Err(error);
        }
        let connected = pending.take().expect("connected pipe is present");
        *pending = create_pipe(&self.pipe_name, &self.sid, false).ok();
        Ok(ClientStream::new(connected.into_file()))
    }
}

struct PendingPipe {
    connection: Option<PendingIo>,
    file: File,
}

impl PendingPipe {
    fn handle(&self) -> HANDLE {
        self.file.as_raw_handle() as HANDLE
    }

    fn into_file(self) -> File {
        let Self { connection, file } = self;
        drop(connection);
        file
    }
}

fn create_pipe(name: &str, sid: &str, first: bool) -> io::Result<PendingPipe> {
    let descriptor = SecurityDescriptor::for_user(sid, false).map_err(io::Error::other)?;
    let attributes = descriptor.attributes();
    let name = wide(name);
    let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    // SAFETY: name and security attributes remain live for the call.
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER_SIZE,
            PIPE_BUFFER_SIZE,
            0,
            &attributes,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: handle is uniquely owned and transferred to File.
        Ok(PendingPipe {
            connection: None,
            file: unsafe { File::from_raw_handle(handle as _) },
        })
    }
}

fn claim_marker(path: &Path) -> Result<String, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.is_file()
                && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 =>
        {
            fs::remove_file(path)
                .map_err(|error| format!("cannot remove stale session marker: {error}"))?;
        }
        Ok(_) => return Err("session marker is not a regular file".into()),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(format!("cannot inspect session marker: {error}")),
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("cannot create session marker: {error}"))?;
    let marker = marker_value();
    file.write_all(marker.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("cannot write session marker: {error}"))?;
    Ok(marker)
}

fn replace_file(source: &Path, target: &Path) -> io::Result<()> {
    let source = wide(source.as_os_str());
    let target = wide(target.as_os_str());
    // SAFETY: both paths are terminated and refer to files in the same directory.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn marker_value() -> String {
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{started}", std::process::id())
}

fn ensure_private_dir(path: &Path, sid: &str) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.is_dir()
                && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 => {}
        Ok(_) => {
            return Err(format!(
                "runtime path {} must be a real directory",
                path.display()
            ));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|error| {
                format!(
                    "cannot create runtime directory {}: {error}",
                    path.display()
                )
            })?;
        }
        Err(error) => {
            return Err(format!(
                "cannot inspect runtime path {}: {error}",
                path.display()
            ));
        }
    }
    let descriptor = SecurityDescriptor::for_user(sid, true)?;
    let path = wide(path.as_os_str());
    // SAFETY: path is terminated and descriptor is a valid self-relative descriptor.
    if unsafe {
        SetFileSecurityW(
            path.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor.0,
        )
    } == 0
    {
        return Err(format!(
            "cannot secure runtime directory: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}

struct SecurityDescriptor(*mut std::ffi::c_void);

impl SecurityDescriptor {
    fn for_user(sid: &str, inheritable: bool) -> Result<Self, String> {
        let inherit = if inheritable { "OICI" } else { "" };
        let sddl = wide(format!("D:P(A;{inherit};GA;;;{sid})"));
        let mut descriptor = ptr::null_mut();
        // SAFETY: sddl is terminated and descriptor is writable. LocalFree owns the
        // returned allocation on success.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &raw mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(format!(
                "cannot create user-only security descriptor: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(Self(descriptor))
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        }
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: pointer was allocated by ConvertStringSecurityDescriptor... .
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn current_sid() -> Result<String, String> {
    // SAFETY: GetCurrentProcess returns a process pseudo-handle valid in this process.
    sid_for_process(unsafe { GetCurrentProcess() })
}

fn process_sid(pid: u32) -> Result<String, String> {
    // SAFETY: requested access is read-only and pid came from the connected pipe.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(format!(
            "cannot open pipe peer process: {}",
            io::Error::last_os_error()
        ));
    }
    let result = sid_for_process(process);
    // SAFETY: process was returned by OpenProcess.
    unsafe {
        CloseHandle(process);
    }
    result
}

fn sid_for_process(process: HANDLE) -> Result<String, String> {
    let mut token = ptr::null_mut();
    // SAFETY: process is a live process handle and token is writable.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) } == 0 {
        return Err(format!(
            "cannot open process token: {}",
            io::Error::last_os_error()
        ));
    }
    let result = (|| {
        let mut length = 0;
        // SAFETY: the first call intentionally queries the required buffer length.
        unsafe {
            GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &raw mut length);
        }
        if length == 0 {
            return Err(format!(
                "cannot size current user token: {}",
                io::Error::last_os_error()
            ));
        }
        let mut buffer = vec![0_u8; length as usize];
        // SAFETY: buffer has the size returned by GetTokenInformation.
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                length,
                &raw mut length,
            )
        } == 0
        {
            return Err(format!(
                "cannot read current user token: {}",
                io::Error::last_os_error()
            ));
        }
        // SAFETY: successful TokenUser query initialized TOKEN_USER at buffer start.
        let user = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };
        let mut text = ptr::null_mut();
        // SAFETY: SID comes from the validated token buffer and text is writable.
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &raw mut text) } == 0 {
            return Err(format!(
                "cannot format current user SID: {}",
                io::Error::last_os_error()
            ));
        }
        let mut length = 0;
        // SAFETY: text points to a terminated string allocated by LocalAlloc.
        unsafe {
            while *text.add(length) != 0 {
                length += 1;
            }
        }
        // SAFETY: the measured slice lies within the terminated allocation.
        let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) })
            .map_err(|_| "current user SID is not valid UTF-16".to_owned());
        // SAFETY: text was allocated by ConvertSidToStringSidW.
        unsafe {
            LocalFree(text.cast());
        }
        sid
    })();
    // SAFETY: token was returned by OpenProcessToken.
    unsafe {
        CloseHandle(token);
    }
    result
}

fn verify_pipe_peer(handle: HANDLE, sid: &str, server: bool) -> io::Result<()> {
    let mut pid = 0;
    // SAFETY: handle is a connected named-pipe endpoint and pid is writable.
    let found = unsafe {
        if server {
            GetNamedPipeServerProcessId(handle, &raw mut pid)
        } else {
            GetNamedPipeClientProcessId(handle, &raw mut pid)
        }
    };
    if found == 0 {
        return Err(io::Error::last_os_error());
    }
    let peer_sid = process_sid(pid).map_err(io::Error::other)?;
    if peer_sid == sid {
        Ok(())
    } else {
        Err(io::Error::new(
            ErrorKind::PermissionDenied,
            "named-pipe peer belongs to another user",
        ))
    }
}

fn validate_session_name(name: &str) -> Result<(), String> {
    if valid_session_name(name) {
        Ok(())
    } else {
        Err("session name must match [A-Za-z0-9_-]{1,64}".into())
    }
}

fn valid_session_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn wide(value: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_user_pipe_round_trips() {
        let sid = current_sid().unwrap();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!("termfold-test-{nonce}"));
        ensure_private_dir(&path, &sid).unwrap();
        let runtime = RuntimeDir { path, sid };
        let socket = runtime.bind("round-trip").unwrap();
        let mut client = runtime.connect("round-trip").unwrap();
        let mut server = socket.accept().unwrap();

        let mut reader = server.try_clone().unwrap();
        let blocked_reader = thread::spawn(move || {
            let mut request = [0; 4];
            reader.read_exact(&mut request).unwrap();
            request
        });

        server.write_all(b"pong").unwrap();
        let mut response = [0; 4];
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        client.read_exact(&mut response).unwrap();
        assert_eq!(&response, b"pong");
        client.write_all(b"ping").unwrap();
        assert_eq!(&blocked_reader.join().unwrap(), b"ping");

        drop(server);
        drop(client);
        drop(socket);
        fs::remove_dir_all(runtime.path).unwrap();
    }
}
