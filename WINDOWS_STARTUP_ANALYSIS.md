# Windows Startup Failure Analysis

## Status

T23 is implemented and verified on native x86-64 Windows.

## Reported failure

Creating a native Windows session originally failed with:

```text
termfold: session server exited with exit code: 0
```

After isolating the ConPTY launch problem, the server remained alive but the
client still failed its startup status exchange with a named-pipe read timeout.
These are separate defects.

## Runtime evidence

### ConPTY launch

- A directly started server printed the `cmd.exe` banner and prompt to the
  parent's console, then exited successfully.
- Supplying a pointer to the `HPCON` value caused child initialization failure
  `0xc0000142`; the process-thread attribute requires the direct `HPCON` value.
- Setting `STARTF_USESTDHANDLES` with null standard handles stopped the parent
  console leakage. The directly started server then remained alive beyond the
  1.5-second observation period.

This proves that inherited standard handles were bypassing the pseudoconsole
during the original startup failure.

### Control IPC

The status trace showed:

1. The server accepted the client promptly.
2. It read `StatusRequest` promptly.
3. It dispatched `StatusResponse` about 50 ms later.
4. The client timed out before receiving the response.
5. The server's writer later failed with Windows error 232 after the client
   closed the pipe.

The response is therefore produced in time but blocked in the transport.
Termfold currently clones one synchronous duplex named-pipe connection: one
thread blocks in `ReadFile` while another attempts `WriteFile` through the
duplicated handle. The synchronous connection does not provide the independent
read/write progress required by this design.

## Documentation and open-source comparison

The sources agree on two different I/O requirements:

- ConPTY communication channels use synchronous pipes, normally with separate
  threads for input and output. Microsoft also requires the ConPTY-side pipe
  handles to remain valid through child creation:
  [Creating a pseudoconsole session](https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session).
- Simultaneous named-pipe input and output should use overlapped I/O with a
  separate `OVERLAPPED` structure and event for each operation:
  [Synchronous and overlapped pipe I/O](https://learn.microsoft.com/en-us/windows/win32/ipc/synchronous-and-overlapped-input-and-output).
- Tokio creates both ends of its Windows named pipes with
  `FILE_FLAG_OVERLAPPED`:
  [Tokio named-pipe source](https://docs.rs/crate/tokio/latest/source/src/net/windows/named_pipe.rs).
- Go's `npipe` implementation allocates independent event-backed overlapped
  state for every `ReadFile` and `WriteFile`:
  [npipe Windows source](https://chromium.googlesource.com/external/github.com/natefinch/npipe/+/272c8150302e83f23d32a355364578c9c13ab20f/npipe_windows.go).
- Rust's standard-library Windows pipe implementation also creates overlapped
  pipe handles:
  [Rust Windows pipe source](https://fuchsia.googlesource.com/third_party/rust/+/b4cf2cdf870512373a656393f393bce84eb78d80/library/std/src/sys/windows/pipe.rs).
- The Rust `conpty` implementation passes the direct `HPCON` value and uses
  `STARTF_USESTDHANDLES` with null standard handles to prevent inherited console
  leakage:
  [conpty process source](https://docs.rs/conpty/latest/conpty/source/src/process.rs).
- Alacritty keeps ConPTY input and output as separate synchronous channels:
  [Alacritty ConPTY source](https://git.causa-arcana.com/PolytreeDE/alacritty/src/commit/5060f8eeb864e8c304fbad9588bdd882db942356/alacritty_terminal/src/tty/windows/conpty.rs).

The ConPTY pipes and Termfold's control named pipe serve different purposes.
The synchronous ConPTY requirement does not justify synchronous duplex control
IPC.

## Rejected directions

- **Pointer-to-`HPCON`:** contradicts the API and produced child initialization
  failure.
- **Longer startup timeout:** hides the blocked write without restoring
  transport progress.
- **`PeekNamedPipe`, `PIPE_NOWAIT`, or repeated raw `ReadFile`:** changes client
  timeout behaviour but does not unblock the server's concurrent writer.
- **Job-list or child-suspension changes:** unrelated to the observed status
  response blockage.
- **A new asynchronous runtime dependency:** unnecessary for the required Win32
  operation and conflicts with the project's dependency and size goals.

## Minimal correction

1. Revert the experimental timeout, polling, `PIPE_NOWAIT`, and raw `ReadFile`
   changes.
2. Keep the verified ConPTY launch behaviour:
   - pass the direct `HPCON` value;
   - use `STARTF_USESTDHANDLES` with non-console standard handles;
   - keep ConPTY-side pipe handles alive through `CreateProcessW`.
3. Create both control named-pipe endpoints with `FILE_FLAG_OVERLAPPED`.
4. Give every outstanding control-pipe read and write independent
   `OVERLAPPED` state and events.
5. Preserve the existing framing, size limits, SID checks, DACL, protocol, and
   client/server ownership model.

No new dependency, protocol change, shell-specific branch, or additional
configuration is required.

## Implementation

- ConPTY child creation passes the direct `HPCON`, prevents inherited console
  standard handles, and retains ConPTY-side pipe handles through
  `CreateProcessW`.
- Control named-pipe clients, servers, reads, writes, and accepts use overlapped
  I/O with independent event-backed `OVERLAPPED` state.
- Timed reads cancel and complete outstanding I/O before releasing its state.
- The existing framing, limits, SID checks, DACL, and threading model are
  unchanged.

## Verification

Native Windows verification completed on 2026-07-29:

- `cargo test --locked --target x86_64-pc-windows-msvc`: 42 passed.
- The named-pipe regression check blocks one cloned reader while the cloned
  writer completes a response, then completes the reader.
- `cargo clippy --locked --target x86_64-pc-windows-msvc --all-targets
  --all-features -- -D warnings`: passed.
- `cargo build --release --locked --target x86_64-pc-windows-msvc`: passed;
  438,272 bytes.
- A native `termfold new` acceptance run reported the session as attached;
  `termfold kill` terminated it and the client exited with code 0.

## Remaining platform acceptance

Interactive Windows Terminal, WezTerm, Command Prompt, and configured MSYS2-shell
acceptance remain part of T21.

## Interactive latency analysis

The former server loop slept for 50 ms after each iteration. Client input was
read by a dedicated thread, but the server observed its channel only on the
next iteration. It then wrote pending pane input at the start of a later
iteration. This introduced an intentional 50--100 ms input delay before ConPTY
and `cmd.exe` processing, excluding terminal and application latency.

This is a shared server-loop behaviour on Linux, WSL, and native Windows, not a
confirmed MSYS2-specific defect. MSYS2 pipe bridging may add overhead, but it
has not been measured separately.

### Implemented correction

Replace periodic input and PTY polling with a bounded central event queue:

1. Client-reader threads enqueue control and input events.
2. One blocking reader per PTY enqueues output events. Linux readers wait with
   `poll`; Windows readers block on the ConPTY output pipe.
3. The server processes each bounded event batch, then immediately flushes
   pending PTY input and renders output.
4. The only periodic wake-up is the existing 50 ms nonblocking listener check;
   pending escape-sequence deadlines can wake the loop sooner.

This retains bounded queues and requires no async runtime or additional
dependency. `cargo build --locked` passed on Linux on 2026-07-30. Native
Windows/MSYS2 latency measurement remains required.
