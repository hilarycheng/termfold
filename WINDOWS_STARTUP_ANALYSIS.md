# Windows Startup Failure Analysis

## Status

This document records the diagnosis and intended correction for T23. It does
not mark the implementation complete. Code changes and Windows verification
remain subject to separate approval.

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

## Verification criteria

Focused native Windows verification must demonstrate:

- default `cmd.exe` and configured MSYS2 shells both start inside ConPTY without
  writing their banner or prompt to the parent console;
- session creation receives the startup status response and attaches;
- control requests and server output can progress concurrently;
- partial frames remain correctly buffered and bounded;
- disconnect and cancellation cannot access freed `OVERLAPPED` state;
- idle operation does not poll or busy-loop;
- ConPTY, pipe, process, thread, event, and job handles are closed
  deterministically.

The Windows backend is not complete until these checks pass.
