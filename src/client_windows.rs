use std::{
    io,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering},
    },
    thread,
    time::Duration,
};

use std::os::windows::process::CommandExt;
use windows_sys::core::BOOL;
use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::WriteFile,
    System::{
        Console::{
            CONSOLE_SCREEN_BUFFER_INFO, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT,
            CTRL_SHUTDOWN_EVENT, DISABLE_NEWLINE_AUTO_RETURN, ENABLE_ECHO_INPUT,
            ENABLE_EXTENDED_FLAGS, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT,
            ENABLE_PROCESSED_OUTPUT, ENABLE_QUICK_EDIT_MODE, ENABLE_VIRTUAL_TERMINAL_INPUT,
            ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleCP, GetConsoleMode, GetConsoleOutputCP,
            GetConsoleScreenBufferInfo, GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
            SetConsoleCP, SetConsoleCtrlHandler, SetConsoleMode, SetConsoleOutputCP,
        },
        Threading::{CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS},
    },
};

use crate::{
    ipc::{self, Message},
    outer::Capabilities,
    runtime::ClientStream,
    session::Size,
};

use super::{ENTER_TERMINAL, restore_sequence};

static RESTORE_INPUT: AtomicIsize = AtomicIsize::new(0);
static RESTORE_OUTPUT: AtomicIsize = AtomicIsize::new(0);
static RESTORE_INPUT_MODE: AtomicU32 = AtomicU32::new(0);
static RESTORE_OUTPUT_MODE: AtomicU32 = AtomicU32::new(0);
static RESTORE_INPUT_CODE_PAGE: AtomicU32 = AtomicU32::new(0);
static RESTORE_OUTPUT_CODE_PAGE: AtomicU32 = AtomicU32::new(0);

const EMERGENCY_RESTORE: &[u8] = b"\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\
\x1b[?1l\x1b[?2004l\x1b[0m\x1b[?25h\x1b[?1049l";
const UTF8_CODE_PAGE: u32 = 65001;

pub(super) struct TerminalGuard(pub(super) Arc<TerminalRestorer>);

pub(super) struct TerminalRestorer {
    input: isize,
    output: isize,
    input_mode: u32,
    output_mode: u32,
    input_code_page: u32,
    output_code_page: u32,
    restore: Vec<u8>,
    restored: AtomicBool,
}

impl TerminalGuard {
    pub(super) fn enter(capabilities: Capabilities) -> Result<Option<Self>, String> {
        // SAFETY: GetStdHandle returns borrowed process handles and GetConsoleMode only
        // reads their console state.
        let (
            input,
            output,
            mut input_mode,
            mut output_mode,
            input_code_page,
            output_code_page,
        ) = unsafe {
            let input = GetStdHandle(STD_INPUT_HANDLE);
            let output = GetStdHandle(STD_OUTPUT_HANDLE);
            if input.is_null()
                || output.is_null()
                || input == INVALID_HANDLE_VALUE
                || output == INVALID_HANDLE_VALUE
            {
                return Ok(None);
            }
            let mut input_mode = 0;
            let mut output_mode = 0;
            if GetConsoleMode(input, &raw mut input_mode) == 0
                || GetConsoleMode(output, &raw mut output_mode) == 0
            {
                return Ok(None);
            }
            let input_code_page = GetConsoleCP();
            let output_code_page = GetConsoleOutputCP();
            if input_code_page == 0 || output_code_page == 0 {
                return Ok(None);
            }
            (
                input,
                output,
                input_mode,
                output_mode,
                input_code_page,
                output_code_page,
            )
        };

        let original_input = input_mode;
        let original_output = output_mode;
        input_mode &= !(ENABLE_ECHO_INPUT
            | ENABLE_LINE_INPUT
            | ENABLE_PROCESSED_INPUT
            | ENABLE_QUICK_EDIT_MODE);
        input_mode |= ENABLE_EXTENDED_FLAGS | ENABLE_VIRTUAL_TERMINAL_INPUT;
        output_mode |= ENABLE_PROCESSED_OUTPUT
            | ENABLE_VIRTUAL_TERMINAL_PROCESSING
            | DISABLE_NEWLINE_AUTO_RETURN;

        // SAFETY: handles and modes were validated by GetConsoleMode above.
        unsafe {
            if SetConsoleMode(output, output_mode) == 0 {
                return Err(format!(
                    "cannot enable virtual-terminal output: {}",
                    io::Error::last_os_error()
                ));
            }
            if SetConsoleMode(input, input_mode) == 0 {
                let error = io::Error::last_os_error();
                SetConsoleMode(output, original_output);
                return Err(format!("cannot enable raw terminal input: {error}"));
            }
            if SetConsoleOutputCP(UTF8_CODE_PAGE) == 0 || SetConsoleCP(UTF8_CODE_PAGE) == 0 {
                let error = io::Error::last_os_error();
                restore_console(
                    input,
                    output,
                    original_input,
                    original_output,
                    input_code_page,
                    output_code_page,
                    &[],
                );
                return Err(format!("cannot enable UTF-8 console I/O: {error}"));
            }
        }

        RESTORE_INPUT_MODE.store(original_input, Ordering::Release);
        RESTORE_OUTPUT_MODE.store(original_output, Ordering::Release);
        RESTORE_INPUT_CODE_PAGE.store(input_code_page, Ordering::Release);
        RESTORE_OUTPUT_CODE_PAGE.store(output_code_page, Ordering::Release);
        RESTORE_OUTPUT.store(output as isize, Ordering::Release);
        RESTORE_INPUT.store(input as isize, Ordering::Release);
        // SAFETY: the callback uses only atomics and Win32 functions documented for
        // console-control handlers.
        if unsafe { SetConsoleCtrlHandler(Some(console_control_handler), 1) } == 0 {
            restore_console(
                input,
                output,
                original_input,
                original_output,
                input_code_page,
                output_code_page,
                &[],
            );
            clear_emergency_restore();
            return Err(format!(
                "cannot install terminal control handler: {}",
                io::Error::last_os_error()
            ));
        }

        let restorer = Arc::new(TerminalRestorer {
            input: input as isize,
            output: output as isize,
            input_mode: original_input,
            output_mode: original_output,
            input_code_page,
            output_code_page,
            restore: restore_sequence(capabilities),
            restored: AtomicBool::new(false),
        });
        if let Err(error) = write_handle(
            output,
            if capabilities.alternate_screen {
                ENTER_TERMINAL
            } else {
                &[]
            },
        ) {
            restorer.restore();
            return Err(format!("cannot enter alternate screen: {error}"));
        }
        Ok(Some(Self(restorer)))
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.0.restore();
    }
}

impl TerminalRestorer {
    pub(super) fn restore(&self) {
        if self.restored.swap(true, Ordering::AcqRel) {
            return;
        }
        restore_console(
            self.input as HANDLE,
            self.output as HANDLE,
            self.input_mode,
            self.output_mode,
            self.input_code_page,
            self.output_code_page,
            &self.restore,
        );
        clear_emergency_restore();
        // SAFETY: this removes the exact callback installed by TerminalGuard::enter.
        unsafe {
            SetConsoleCtrlHandler(Some(console_control_handler), 0);
        }
    }
}

fn restore_console(
    input: HANDLE,
    output: HANDLE,
    input_mode: u32,
    output_mode: u32,
    input_code_page: u32,
    output_code_page: u32,
    bytes: &[u8],
) {
    let _ = write_handle(output, bytes);
    // SAFETY: the handles were validated console handles and the modes came from
    // GetConsoleMode.
    unsafe {
        SetConsoleMode(input, input_mode);
        SetConsoleMode(output, output_mode);
        SetConsoleCP(input_code_page);
        SetConsoleOutputCP(output_code_page);
    }
}

fn clear_emergency_restore() {
    RESTORE_INPUT.store(0, Ordering::Release);
    RESTORE_OUTPUT.store(0, Ordering::Release);
}

unsafe extern "system" fn console_control_handler(control: u32) -> BOOL {
    if !matches!(
        control,
        CTRL_BREAK_EVENT | CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT
    ) {
        return 0;
    }
    let input = RESTORE_INPUT.swap(0, Ordering::AcqRel) as HANDLE;
    let output = RESTORE_OUTPUT.swap(0, Ordering::AcqRel) as HANDLE;
    if input.is_null() || output.is_null() {
        return 0;
    }
    let _ = write_handle(output, EMERGENCY_RESTORE);
    // SAFETY: atomics contain the live console handles and modes installed by the
    // active TerminalGuard.
    unsafe {
        SetConsoleMode(input, RESTORE_INPUT_MODE.load(Ordering::Acquire));
        SetConsoleMode(output, RESTORE_OUTPUT_MODE.load(Ordering::Acquire));
        SetConsoleCP(RESTORE_INPUT_CODE_PAGE.load(Ordering::Acquire));
        SetConsoleOutputCP(RESTORE_OUTPUT_CODE_PAGE.load(Ordering::Acquire));
    }
    0
}

fn write_handle(handle: HANDLE, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let mut written = 0;
        let length = bytes.len().min(u32::MAX as usize) as u32;
        // SAFETY: handle is a live output handle and bytes is readable for length.
        if unsafe {
            WriteFile(
                handle,
                bytes.as_ptr(),
                length,
                &raw mut written,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "terminal write returned zero",
            ));
        }
        bytes = &bytes[written as usize..];
    }
    Ok(())
}

pub(super) struct BlockedSignals;

pub(super) struct SignalGuard {
    running: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl BlockedSignals {
    pub(super) fn block() -> Result<Self, String> {
        Ok(Self)
    }

    pub(super) fn listen(
        self,
        writer: Arc<Mutex<ClientStream>>,
        _terminal: Option<Arc<TerminalRestorer>>,
    ) -> Result<SignalGuard, String> {
        let running = Arc::new(AtomicBool::new(true));
        let thread_running = Arc::clone(&running);
        let thread = thread::Builder::new()
            .name("termfold-resize".into())
            .spawn(move || {
                let mut previous = terminal_size();
                while thread_running.load(Ordering::Acquire) {
                    thread::park_timeout(Duration::from_millis(100));
                    if !thread_running.load(Ordering::Acquire) {
                        break;
                    }
                    let size = terminal_size();
                    if size == previous {
                        continue;
                    }
                    previous = size;
                    let message = Message::Resize {
                        columns: size.columns,
                        rows: size.rows,
                    };
                    let Ok(mut stream) = writer.lock() else {
                        break;
                    };
                    if ipc::write_message(&mut *stream, &message).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| format!("cannot start terminal resize monitor: {error}"))?;
        Ok(SignalGuard {
            running,
            thread: Some(thread),
        })
    }
}

impl Drop for SignalGuard {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

pub(super) fn configure_server(command: &mut Command) {
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

pub(crate) fn terminal_size() -> Size {
    // SAFETY: GetStdHandle returns a borrowed handle and the info pointer is writable.
    unsafe {
        let output = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
        if !output.is_null()
            && output != INVALID_HANDLE_VALUE
            && GetConsoleScreenBufferInfo(output, &raw mut info) != 0
        {
            let columns = info.srWindow.Right.saturating_sub(info.srWindow.Left) + 1;
            let rows = info.srWindow.Bottom.saturating_sub(info.srWindow.Top) + 1;
            if let (Ok(columns), Ok(rows)) = (u16::try_from(columns), u16::try_from(rows))
                && columns != 0
                && rows != 0
            {
                return Size { columns, rows };
            }
        }
    }
    Size {
        columns: 80,
        rows: 24,
    }
}
