use std::{
    env,
    ffi::{OsStr, OsString, c_void},
    fs::{self, File},
    io::{self, ErrorKind, Read, Write},
    mem::size_of,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle},
        process::ExitStatusExt,
    },
    path::{Path, PathBuf},
    process::ExitStatus,
    ptr, thread,
    time::{Duration, Instant},
};

use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_BROKEN_PIPE, ERROR_NO_DATA, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    },
    Storage::FileSystem::{ReadFile, WriteFile},
    System::{
        Console::{
            COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, PSEUDOCONSOLE_INHERIT_CURSOR,
            ResizePseudoConsole,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
        Pipes::{CreatePipe, PIPE_NOWAIT, PeekNamedPipe, SetNamedPipeHandleState},
        Threading::{
            CREATE_NEW_PROCESS_GROUP, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
            DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
            InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
            PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW,
            TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
        },
    },
};

use crate::session::Size;

pub const TERMINATION_GRACE: Duration = Duration::from_secs(2);
const PSEUDOCONSOLE_RESIZE_QUIRK: u32 = 0x2;
const PSEUDOCONSOLE_WIN32_INPUT_MODE: u32 = 0x4;

pub(crate) fn is_eof_error(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(code) if code == ERROR_BROKEN_PIPE as i32 || code == ERROR_NO_DATA as i32
    )
}

#[derive(Debug)]
pub struct LaunchContext {
    shell: OsString,
    arguments: Vec<OsString>,
    working_directory: PathBuf,
    environment: Vec<(OsString, OsString)>,
    terminfo_root: PathBuf,
    inner_term: String,
}

impl LaunchContext {
    pub fn capture(
        terminfo_root: PathBuf,
        inner_term: String,
        windows_shell: &[String],
    ) -> io::Result<Self> {
        let (shell, arguments) = approved_shell(windows_shell)?;
        Ok(Self {
            shell,
            arguments,
            working_directory: env::current_dir()?,
            environment: env::vars_os().collect(),
            terminfo_root,
            inner_term,
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
pub struct PtyMaster {
    input: Option<File>,
    output: Option<File>,
}

impl PtyMaster {
    fn close(&mut self) {
        self.input.take();
        self.output.take();
    }
}

impl Read for PtyMaster {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let Some(output) = &self.output else {
            return Ok(0);
        };
        let handle = output.as_raw_handle() as HANDLE;
        let mut available = 0;
        // SAFETY: handle is the readable host end of the ConPTY output pipe.
        if unsafe {
            PeekNamedPipe(
                handle,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                &raw mut available,
                ptr::null_mut(),
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            return if is_eof_error(&error) {
                Ok(0)
            } else {
                Err(error)
            };
        }
        if available == 0 {
            return Err(io::Error::from(ErrorKind::WouldBlock));
        }
        let mut read = 0;
        let length = buffer.len().min(available as usize).min(u32::MAX as usize) as u32;
        // SAFETY: buffer is writable for length and handle is a live pipe handle.
        if unsafe {
            ReadFile(
                handle,
                buffer.as_mut_ptr(),
                length,
                &raw mut read,
                ptr::null_mut(),
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            if is_eof_error(&error) {
                Ok(0)
            } else {
                Err(error)
            }
        } else {
            Ok(read as usize)
        }
    }
}

impl Write for PtyMaster {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let Some(input) = &self.input else {
            return Err(io::Error::new(
                ErrorKind::BrokenPipe,
                "ConPTY input is closed",
            ));
        };
        let mut written = 0;
        let length = buffer.len().min(u32::MAX as usize) as u32;
        // SAFETY: buffer is readable for length and handle is the writable host end.
        if unsafe {
            WriteFile(
                input.as_raw_handle() as HANDLE,
                buffer.as_ptr(),
                length,
                &raw mut written,
                ptr::null_mut(),
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_DATA as i32) {
                Err(io::Error::from(ErrorKind::WouldBlock))
            } else {
                Err(error)
            }
        } else if written == 0 {
            Err(io::Error::from(ErrorKind::WouldBlock))
        } else {
            Ok(written as usize)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct PtyChild {
    master: PtyMaster,
    process: HANDLE,
    job: HANDLE,
    console: Option<HPCON>,
    console_close: Option<thread::JoinHandle<()>>,
    pid: u32,
}

impl PtyChild {
    pub fn spawn(context: &LaunchContext, size: Size) -> io::Result<Self> {
        validate_size(size)?;
        let (console, mut master, console_input, console_output) = create_pseudo_console(size)?;
        let mut attribute_size = 0;
        // SAFETY: first call queries the required attribute-list size.
        unsafe {
            InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &raw mut attribute_size);
        }
        if attribute_size == 0 {
            let error = io::Error::last_os_error();
            master.close();
            close_console(console);
            return Err(error);
        }
        let words = attribute_size.div_ceil(size_of::<usize>());
        let mut attributes = vec![0_usize; words];
        let attribute_list = attributes.as_mut_ptr().cast();
        // SAFETY: attributes is aligned and large enough for the queried size.
        if unsafe {
            InitializeProcThreadAttributeList(attribute_list, 1, 0, &raw mut attribute_size)
        } == 0
        {
            let error = io::Error::last_os_error();
            master.close();
            close_console(console);
            return Err(error);
        }
        // SAFETY: both attribute values remain live until the list is deleted.
        if unsafe {
            UpdateProcThreadAttribute(
                attribute_list,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                console as *const c_void,
                size_of::<HPCON>(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            // SAFETY: list was initialized above.
            unsafe {
                DeleteProcThreadAttributeList(attribute_list);
            }
            master.close();
            close_console(console);
            return Err(error);
        }

        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
        startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
        startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
        startup.lpAttributeList = attribute_list;
        let application = wide(&context.shell);
        let mut command_line = command_line(context);
        let current_directory = wide(context.working_directory.as_os_str());
        let environment = environment_block(context);
        let mut process = PROCESS_INFORMATION::default();
        let flags = EXTENDED_STARTUPINFO_PRESENT
            | CREATE_UNICODE_ENVIRONMENT
            | CREATE_NEW_PROCESS_GROUP
            | CREATE_SUSPENDED;
        // SAFETY: all pointers remain live for CreateProcessW and startup contains a
        // valid pseudoconsole attribute list.
        let created = unsafe {
            CreateProcessW(
                application.as_ptr(),
                command_line.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                0,
                flags,
                environment.as_ptr().cast(),
                current_directory.as_ptr(),
                &startup.StartupInfo,
                &raw mut process,
            )
        };
        let create_error = (created == 0).then(io::Error::last_os_error);
        // SAFETY: CreateProcessW has returned and no longer reads the attribute list.
        unsafe {
            DeleteProcThreadAttributeList(attribute_list);
        }
        drop(console_input);
        drop(console_output);
        if let Some(error) = create_error {
            master.close();
            close_console(console);
            return Err(error);
        }

        let job = match create_kill_job() {
            Ok(job) => job,
            Err(error) => {
                // SAFETY: process/thread were returned by CreateProcessW.
                unsafe {
                    TerminateProcess(process.hProcess, 1);
                    CloseHandle(process.hThread);
                    CloseHandle(process.hProcess);
                }
                master.close();
                close_console(console);
                return Err(error);
            }
        };
        // SAFETY: both handles are live and the child is still suspended.
        if unsafe { AssignProcessToJobObject(job, process.hProcess) } == 0 {
            let error = io::Error::last_os_error();
            // SAFETY: all handles are live and uniquely owned here.
            unsafe {
                TerminateProcess(process.hProcess, 1);
                CloseHandle(process.hThread);
                CloseHandle(process.hProcess);
                CloseHandle(job);
            }
            master.close();
            close_console(console);
            return Err(error);
        }
        // SAFETY: hThread is the suspended primary thread.
        if unsafe { ResumeThread(process.hThread) } == u32::MAX {
            let error = io::Error::last_os_error();
            // SAFETY: terminating the job stops the suspended child before cleanup.
            unsafe {
                TerminateJobObject(job, 1);
                CloseHandle(process.hThread);
                CloseHandle(process.hProcess);
                CloseHandle(job);
            }
            master.close();
            close_console(console);
            return Err(error);
        }
        // SAFETY: the primary thread is running and its handle is no longer needed.
        unsafe {
            CloseHandle(process.hThread);
        }
        Ok(Self {
            master,
            process: process.hProcess,
            job,
            console: Some(console),
            console_close: None,
            pid: process.dwProcessId,
        })
    }

    pub fn master(&mut self) -> &mut PtyMaster {
        &mut self.master
    }

    pub fn id(&self) -> u32 {
        self.pid
    }

    pub fn resize(&self, size: Size) -> io::Result<()> {
        validate_size(size)?;
        let Some(console) = self.console else {
            return Err(io::Error::new(ErrorKind::BrokenPipe, "ConPTY is closed"));
        };
        // SAFETY: console is live and coordinate values were validated.
        let result = unsafe { ResizePseudoConsole(console, coordinates(size)) };
        if result < 0 {
            Err(hresult_error("cannot resize ConPTY", result))
        } else {
            Ok(())
        }
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        // SAFETY: process is a live process handle.
        match unsafe { WaitForSingleObject(self.process, 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0;
                // SAFETY: process is signaled and code is writable.
                if unsafe { GetExitCodeProcess(self.process, &raw mut code) } == 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(Some(ExitStatus::from_raw(code)))
                }
            }
            _ => Err(io::Error::last_os_error()),
        }
    }

    fn request_termination(&mut self) -> io::Result<()> {
        self.master.close();
        let Some(console) = self.console.take() else {
            return Ok(());
        };
        match thread::Builder::new()
            .name("termfold-conpty-close".into())
            .spawn(move || close_console(console))
        {
            Ok(thread) => {
                self.console_close = Some(thread);
                Ok(())
            }
            Err(error) => {
                self.console = Some(console);
                Err(error)
            }
        }
    }

    fn force_termination(&self) -> io::Result<()> {
        // SAFETY: job is live and contains the pane process tree.
        if unsafe { TerminateJobObject(self.job, 1) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn console_is_closing(&self) -> bool {
        self.console.is_some()
            || self
                .console_close
                .as_ref()
                .is_some_and(|thread| !thread.is_finished())
    }

    fn finish_console_close(&mut self) {
        if let Some(console) = self.console.take() {
            close_console(console);
        }
        if let Some(thread) = self.console_close.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for PtyChild {
    fn drop(&mut self) {
        let _ = self.force_termination();
        // SAFETY: process is live and bounded because job termination was requested.
        unsafe {
            WaitForSingleObject(self.process, 2_000);
        }
        let _ = self.request_termination();
        // SAFETY: closing the kill-on-close job bounds the console-close thread.
        unsafe {
            CloseHandle(self.job);
        }
        self.finish_console_close();
        // SAFETY: process is uniquely owned by this PtyChild.
        unsafe {
            CloseHandle(self.process);
        }
    }
}

pub fn terminate_all(children: &mut [&mut PtyChild]) -> io::Result<()> {
    let mut first_error = None;
    for child in children.iter_mut() {
        remember_error(&mut first_error, child.request_termination());
    }
    let deadline = Instant::now() + TERMINATION_GRACE;
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
            running |= child.console_is_closing();
        }
        if !running || Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    for child in children.iter_mut() {
        if child.try_wait()?.is_none() || child.console_is_closing() {
            remember_error(&mut first_error, child.force_termination());
        }
        // SAFETY: force_termination bounds the remaining process lifetime.
        unsafe {
            WaitForSingleObject(child.process, 2_000);
        }
        child.finish_console_close();
    }
    first_error.map_or(Ok(()), Err)
}

fn create_pseudo_console(size: Size) -> io::Result<(HPCON, PtyMaster, File, File)> {
    let (input_read, input_write) = anonymous_pipe()?;
    let (output_read, output_write) = anonymous_pipe()?;
    let mut console: HPCON = 0;
    // SAFETY: pipe handles are live and console is writable.
    let result = unsafe {
        CreatePseudoConsole(
            coordinates(size),
            input_read.as_raw_handle() as HANDLE,
            output_write.as_raw_handle() as HANDLE,
            PSEUDOCONSOLE_INHERIT_CURSOR
                | PSEUDOCONSOLE_RESIZE_QUIRK
                | PSEUDOCONSOLE_WIN32_INPUT_MODE,
            &raw mut console,
        )
    };
    if result < 0 {
        return Err(hresult_error("cannot create ConPTY", result));
    }
    let mode = PIPE_NOWAIT;
    // SAFETY: input_write is the writable host end of an anonymous pipe.
    if unsafe {
        SetNamedPipeHandleState(
            input_write.as_raw_handle() as HANDLE,
            &mode,
            ptr::null(),
            ptr::null(),
        )
    } == 0
    {
        let error = io::Error::last_os_error();
        drop(input_write);
        drop(output_read);
        close_console(console);
        return Err(error);
    }
    Ok((
        console,
        PtyMaster {
            input: Some(input_write),
            output: Some(output_read),
        },
        input_read,
        output_write,
    ))
}

fn anonymous_pipe() -> io::Result<(File, File)> {
    let mut read = ptr::null_mut();
    let mut write = ptr::null_mut();
    // SAFETY: output pointers are writable and null attributes create private handles.
    if unsafe { CreatePipe(&raw mut read, &raw mut write, ptr::null(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreatePipe returned two unique live handles.
    Ok(unsafe {
        (
            File::from_raw_handle(read as _),
            File::from_raw_handle(write as _),
        )
    })
}

fn create_kill_job() -> io::Result<HANDLE> {
    // SAFETY: null attributes and name create an unnamed job.
    let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if job.is_null() {
        return Err(io::Error::last_os_error());
    }
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: job is live and limits points to initialized storage.
    if unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast::<c_void>(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    } == 0
    {
        let error = io::Error::last_os_error();
        // SAFETY: job is live and uniquely owned.
        unsafe {
            CloseHandle(job);
        }
        Err(error)
    } else {
        Ok(job)
    }
}

fn approved_shell(configured: &[String]) -> io::Result<(OsString, Vec<OsString>)> {
    if let Some((shell, arguments)) = configured.split_first() {
        let path = Path::new(shell);
        if !path.is_absolute() || !fs::metadata(path).is_ok_and(|metadata| metadata.is_file()) {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "configuration field 'windows_shell': first value must be an absolute executable file",
            ));
        }
        return Ok((
            OsString::from(shell.as_str()),
            arguments
                .iter()
                .map(|argument| OsString::from(argument.as_str()))
                .collect(),
        ));
    }
    let configured = env::var_os("COMSPEC").filter(|shell| {
        let path = Path::new(shell);
        path.is_absolute() && fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
    });
    if let Some(shell) = configured {
        return Ok((shell, Vec::new()));
    }
    let fallback = env::var_os("SystemRoot")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| path.join("System32").join("cmd.exe"))
        .filter(|path| fs::metadata(path).is_ok_and(|metadata| metadata.is_file()));
    fallback
        .map(PathBuf::into_os_string)
        .map(|shell| (shell, Vec::new()))
        .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "cannot locate cmd.exe"))
}

fn command_line(context: &LaunchContext) -> Vec<u16> {
    let mut command = Vec::new();
    for argument in std::iter::once(context.shell.as_os_str())
        .chain(context.arguments.iter().map(OsString::as_os_str))
    {
        if !command.is_empty() {
            command.push(b' ' as u16);
        }
        quote_argument(&mut command, argument);
    }
    command.push(0);
    command
}

fn quote_argument(command: &mut Vec<u16>, argument: &OsStr) {
    command.push(b'"' as u16);
    let mut backslashes = 0;
    for character in argument.encode_wide() {
        if character == b'\\' as u16 {
            backslashes += 1;
        } else {
            if character == b'"' as u16 {
                command.extend(std::iter::repeat_n(b'\\' as u16, backslashes * 2 + 1));
            } else {
                command.extend(std::iter::repeat_n(b'\\' as u16, backslashes));
            }
            backslashes = 0;
            command.push(character);
        }
    }
    command.extend(std::iter::repeat_n(b'\\' as u16, backslashes * 2));
    command.push(b'"' as u16);
}

fn environment_block(context: &LaunchContext) -> Vec<u16> {
    let mut environment = context
        .environment
        .iter()
        .filter(|(key, _)| {
            !matches!(
                key.to_string_lossy().to_ascii_uppercase().as_str(),
                "TERM" | "COLORTERM" | "TERMINFO"
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    environment.extend([
        ("TERM".into(), context.inner_term.clone().into()),
        ("COLORTERM".into(), "truecolor".into()),
        (
            "TERMINFO".into(),
            context.terminfo_root.as_os_str().to_owned(),
        ),
    ]);
    environment.sort_by(|(left, _), (right, _)| {
        left.to_string_lossy()
            .to_ascii_lowercase()
            .cmp(&right.to_string_lossy().to_ascii_lowercase())
    });
    let mut block = Vec::new();
    for (key, value) in environment {
        block.extend(key.encode_wide());
        block.push(b'=' as u16);
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

fn validate_size(size: Size) -> io::Result<()> {
    if size.columns == 0 || size.rows == 0 {
        Err(io::Error::new(
            ErrorKind::InvalidInput,
            "PTY size must be non-zero",
        ))
    } else if size.columns > i16::MAX as u16 || size.rows > i16::MAX as u16 {
        Err(io::Error::new(
            ErrorKind::InvalidInput,
            "ConPTY size exceeds 32767 cells",
        ))
    } else {
        Ok(())
    }
}

fn coordinates(size: Size) -> COORD {
    COORD {
        X: size.columns as i16,
        Y: size.rows as i16,
    }
}

fn remember_error(first: &mut Option<io::Error>, result: io::Result<()>) {
    if let Err(error) = result
        && first.is_none()
    {
        *first = Some(error);
    }
}

fn close_console(console: HPCON) {
    // SAFETY: console is a live HPCON returned by CreatePseudoConsole.
    unsafe {
        ClosePseudoConsole(console);
    }
}

fn hresult_error(operation: &str, result: i32) -> io::Error {
    io::Error::other(format!("{operation}: HRESULT 0x{:08X}", result as u32))
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}
