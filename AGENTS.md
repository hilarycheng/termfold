# AGENTS.md

## Project

**Termfold** is a small, traditional terminal multiplexer inspired by tmux and Byobu.

Normative product behaviour is defined in `REQUIREMENTS.md`. This file governs
the AI workflow. When they conflict, stop and request clarification.

Primary goals:

- Single static Linux binary
- Small binary size
- Fast startup
- Low memory usage
- Traditional terminal UI
- Bottom status bar
- No mandatory plugins
- No runtime network access
- Minimal external dependencies

## Communication

- Keep every reply short and precise.
- Do not repeat the user's requirements.
- Do not provide long explanations unless requested.
- Ask before making architectural changes.
- Discuss the approach before generating or modifying code.
- Do not make unrelated changes.
- State assumptions clearly.
- Report blockers immediately.

## Approval Gate

- `APPROVE` is the only authorization keyword for workspace or external changes.
- The keyword is case-sensitive and must appear as a standalone word in the
  request that describes the change.
- Approval applies only to that request's stated scope. Do not infer, reuse, or
  broaden it.
- Without `APPROVE`, only read, inspect, analyse, and propose changes.
- File edits, generated documents, dependency changes, build/test/lint commands,
  Git mutations, releases, and external writes each require in-scope approval.
- An implementation request without `APPROVE` is not authorization to change files.

## Development Workflow

Before changing code:

1. Inspect the relevant files.
2. Summarize the proposed change briefly.
3. Wait for an in-scope `APPROVE`.
4. Make the smallest practical change.
5. Run focused tests.
6. Run the approved build and verification.
7. Mark the completed task `[*]` in `TASKS.md`.
8. Commit only after the preceding checks pass.
9. Report only the result, risks, and remaining issues.

All Git commit messages MUST follow the Conventional Commits specification.

Do not:

- Rewrite working code without a clear reason.
- Add frameworks for convenience.
- Add dependencies without approval.
- Generate large amounts of boilerplate.
- Hide warnings or test failures.
- Change public behaviour accidentally.

## Platform

Primary target:

```text
x86_64-unknown-linux-musl
```

Possible future targets:

```text
aarch64-unknown-linux-musl
```

The release binary must run without external shared libraries.

Required validation:

```bash
file target/x86_64-unknown-linux-musl/release/termfold
ldd target/x86_64-unknown-linux-musl/release/termfold
```

Expected result:

```text
statically linked
not a dynamic executable
```


## Development Environments

Termfold must support development in both of these environments:

### Windows Host with MinGW/MSYS2 and WSL

- The editor or Codex App may run on Windows.
- Prefer MinGW/MSYS2 shell for Windows-side command-line work.
- Source files may be opened from Windows.
- Build, test, lint, PTY testing, and execution must run inside WSL.
- WSL is the authoritative runtime environment.
- MinGW/MSYS2 is a convenience shell, not the release runtime.
- Do not use Windows-native Rust targets for release validation.
- Prefer storing the repository inside the WSL filesystem for better performance and Linux permission behaviour.
- Avoid assumptions based on Windows paths, drive letters, CRLF, or Windows file permissions.
- Shell commands in project documentation should be compatible with Bash used by MinGW/MSYS2 and WSL where practical.

Example target:

```text
Windows Codex App
        ↓
MinGW/MSYS2 shell
        ↓
WSL Linux filesystem and shell
        ↓
cargo build / test / run
```

### Pure Linux

- The editor, shell, build tools, tests, and runtime all run directly on Linux.
- The same commands and configuration used in WSL should work unchanged.
- Do not introduce WSL-only logic into the application.

## Environment Rules

- Linux behaviour is the source of truth.
- Both environments must use the same Rust toolchain and locked dependencies.
- Use LF line endings.
- Keep scripts compatible with POSIX shell or Bash.
- Prefer commands that behave consistently in MinGW/MSYS2 Bash and WSL Bash.
- Do not require PowerShell or `cmd.exe` for core development.
- Do not store absolute developer-specific paths.
- Do not depend on Windows environment variables.
- PTY, signals, sockets, permissions, and terminal restoration must be tested in Linux or WSL.
- Release binaries must be built and validated in Linux or WSL using the musl target.

## Build Requirements

Use stable Rust.

Preferred release command:

```bash
cargo build --release --locked --target x86_64-unknown-linux-musl
```

Release profile should favour size:

```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

Avoid build-time or runtime dependence on:

- OpenSSL
- systemd
- PAM
- glibc-only APIs
- dynamically linked C libraries
- external shell commands for core functions

Pure Rust dependencies are preferred.

## Dependency Policy

Every new dependency must be justified.

Prefer:

- Rust standard library
- Small, focused crates
- Crates with active maintenance
- Crates that support musl
- Crates without large dependency trees
- Rust implementations over C bindings

Avoid:

- Async runtimes unless clearly necessary
- GUI libraries
- Plugin runtimes
- Embedded web servers
- Serialization frameworks unless needed
- General-purpose frameworks
- Duplicate crates providing similar functions

Before adding a dependency, check:

```bash
cargo tree
cargo tree --duplicates
cargo bloat --release --target x86_64-unknown-linux-musl
```

## Binary Size

Binary size is a project requirement.

For every release:

```bash
ls -lh target/x86_64-unknown-linux-musl/release/termfold
```

Investigate meaningful size increases.

Do not sacrifice correctness or terminal compatibility for insignificant size reductions.

## Scope

Initial supported features:

- Create a session
- Attach and detach
- Multiple tabs
- Multiple panes
- Horizontal split
- Vertical split
- Pane focus movement
- Pane resize
- PTY resize propagation
- Bottom status bar
- Date display
- Clock display
- Visible tab list
- Active-tab indication
- Configurable prefix key
- tmux-like key bindings
- Session persistence while the server process is alive
- Clean terminal restoration after exit or crash

Initial non-goals:

- Web interface
- Remote network protocol
- Multi-user session sharing
- Plugin system
- Scripting language
- Image protocols
- Sixel
- GPU rendering
- Full tmux command compatibility
- Full Byobu feature compatibility
- Windows native support
- macOS support
- Built-in package manager

## Terminal Model

Termfold operates through Linux PTYs.

Linux only provides a byte stream. Terminal behaviour comes from escape-sequence protocols.

Preferred approach:

- Support modern xterm-compatible terminals
- Keep terminal rendering inside Termfold
- Avoid dependence on the host's terminfo database where practical
- Use explicit capability detection only when needed
- Provide conservative fallback behaviour
- Do not assume every terminal supports mouse or true colour

Inner applications should receive the default identity defined in
`REQUIREMENTS.md`:

```text
TERM=termfold-256color
COLORTERM=truecolor
TERMINFO=<validated per-user Termfold runtime terminfo root>
```

Termfold must supply and validate the required private terminfo entry before using
the custom `TERM` value.

## Input and Mouse

Mouse support is optional and disabled by default.

When enabled:

- Use standard xterm mouse reporting
- Prefer SGR mouse mode
- Support click, drag, and wheel events
- Restore terminal mouse mode on exit
- Never leave the user's terminal in mouse-reporting mode after a crash

Keyboard handling must preserve normal application input when the prefix mode is inactive.

## UI

The default UI should resemble traditional tmux or Byobu.

Requirements:

- No top tab bar
- One compact bottom status bar
- Bottom bar must show the session name
- Bottom bar must show all tabs
- Bottom bar must clearly mark the active tab
- Bottom bar must show the current date
- Bottom bar must show a live clock
- Bottom bar should remain one line where practical
- Clear active-pane indication
- Support horizontal and vertical pane splits
- No IDE-style decorations
- No animations
- No Unicode dependency for essential borders
- Work correctly over SSH
- Work on narrow terminals

Default hierarchy:

```text
Session
└── Tabs
    └── Panes
        ├── Horizontal split
        └── Vertical split
```

Use the terms:

- Session
- Tab
- Pane

## Configuration

Configuration should be optional.

Preferred format:

```text
~/.config/termfold/config.toml
```

Termfold must start with sensible defaults when no config exists.

Configuration errors must:

- Show the exact invalid field
- Avoid silently ignoring mistakes
- Never corrupt an existing config
- Not require network access

Keep the configuration schema small.

## Architecture

Prefer a small number of clear modules:

```text
src/
├── main.rs
├── server.rs
├── client.rs
├── session.rs
├── tab.rs
├── pane.rs
├── pty.rs
├── terminal.rs
├── input.rs
├── render.rs
├── status.rs
└── config.rs
```

Do not create abstractions before they are needed.

Prefer:

- Explicit state
- Bounded channels
- Deterministic cleanup
- Clear ownership
- Small public APIs
- Direct error propagation

Avoid:

- Global mutable state
- Unbounded queues
- Hidden background threads
- Excessive traits
- Deep generic abstractions
- Complex macro systems

## Runtime Requirements

Termfold must:

- Run as an unprivileged user
- Store sockets under a user-owned runtime directory
- Set restrictive socket permissions
- Reject connections from other users
- Handle stale sockets safely
- Restore terminal modes on exit
- Reap child processes
- Avoid zombie processes
- Handle terminal resize signals
- Handle client disconnects
- Avoid busy loops
- Avoid unnecessary background tasks

Preferred runtime directory order:

1. `$XDG_RUNTIME_DIR/termfold`
2. A secure user-specific directory under `/tmp`

Never use a predictable world-writable socket path without ownership and permission checks.

## Security

Security requirements:

- No network listener
- No telemetry
- No automatic update check
- No remote code loading
- No plugin execution
- No shell command interpolation for internal operations
- Validate session and socket names
- Prevent path traversal
- Use restrictive file permissions
- Treat terminal input as untrusted bytes
- Bound memory used for scrollback and queues
- Avoid unsafe Rust unless necessary

Any `unsafe` block must include a short safety comment explaining its invariant.

Run:

```bash
cargo audit
cargo deny check
```

Do not claim that Rust or static linking makes the project automatically secure.

## Error Handling

Use structured errors internally.

User-facing errors must be:

- Short
- Specific
- Actionable

Avoid:

- Panics for normal errors
- Silent fallback after data loss
- Generic messages such as `something went wrong`

`unwrap()` and `expect()` are acceptable only for proven invariants or tests.

## Logging

Logging is disabled by default.

Optional debug logging may write to a user-selected file.

Do not:

- Write logs into the active terminal display
- Log terminal contents by default
- Log environment secrets
- Create persistent logs without user action

## Testing

Minimum test areas:

- Session lifecycle
- Attach and detach
- Pane creation and deletion
- Tab switching
- PTY resize
- Input prefix handling
- Terminal cleanup
- Socket permission checks
- Stale socket recovery
- Config parsing
- Status bar rendering
- Narrow terminal behaviour

Prefer unit tests for state logic and integration tests using PTYs.

Every bug fix should include a regression test when practical.


## Windows Terminal Compatibility

Primary Windows terminal target:

- WezTerm running a WSL shell
- WezTerm connecting to Linux through SSH

Termfold must work correctly when the outer terminal is WezTerm on Windows.

Required behaviour:

- Correct keyboard input
- Correct colour output
- Correct terminal resize handling
- Correct alternate-screen handling
- Correct copy and paste behaviour
- Correct detach and reattach behaviour
- No dependency on a Linux desktop environment
- No assumption that the outer terminal is running on Linux

Recommended outer terminal values:

```text
TERM=xterm-256color
COLORTERM=truecolor
```

Support for WezTerm-specific capabilities may be added only when a standard xterm-compatible fallback remains available.

## Mouse Integration

Mouse support must be available but disabled by default.

When enabled:

- Use standard xterm mouse reporting
- Prefer SGR extended mouse mode
- Support pane selection
- Support pane-border resize
- Support tab selection from the bottom status bar
- Support wheel scrolling in Termfold scrollback
- Forward mouse events to the active application when application mouse mode is enabled
- Restore all mouse modes on detach, exit, panic, or client disconnect
- Do not require WezTerm-specific mouse APIs

Keyboard-only operation must remain fully supported.

## Windows Command Prompt Compatibility

Pure Windows Command Prompt compatibility is best-effort only.

Termfold is not initially a native Windows executable. It remains a Linux binary running under WSL or on a remote Linux host.

Supported scenario:

```text
Windows Command Prompt
        ↓
wsl.exe
        ↓
Termfold inside WSL
```

Requirements:

- Basic text rendering should work
- Basic keyboard input should work
- Session attach and detach should work
- Horizontal and vertical splits should work
- The bottom status bar should remain readable

Limitations are acceptable for:

- Mouse integration
- True colour
- Extended keyboard protocols
- Clipboard integration
- Terminal-specific escape extensions

Do not add native Windows APIs or a Windows-specific backend unless explicitly approved.

## Compatibility

Initial compatibility target:

- Recent Linux kernels
- RHEL-compatible systems
- Ubuntu
- Debian
- Alpine
- WSL
- SSH sessions
- WezTerm on Windows through WSL or SSH
- xterm
- Kitty
- Windows Terminal through WSL or SSH
- Windows Command Prompt through `wsl.exe`, with best-effort compatibility

Do not depend on a desktop environment.

## Release Checklist

Before release:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo audit
cargo deny check
cargo build --release --locked --target x86_64-unknown-linux-musl
file target/x86_64-unknown-linux-musl/release/termfold
ldd target/x86_64-unknown-linux-musl/release/termfold
```

Also confirm:

- Binary size is acceptable
- No unexpected shared libraries
- No network access is required
- Terminal state restores correctly
- Detach and reattach work over SSH
- Release source and dependency versions are locked
- SHA-256 checksum is generated

## Decision Priority

When requirements conflict, use this order:

1. Correctness
2. Terminal restoration and session safety
3. Security
4. Portability
5. Low memory usage
6. Small binary size
7. Convenience
8. Additional features


## Default Status Bar

The default bottom status bar layout should be:

```text
[session]  1:shell  2:logs  3:db  |  2026-07-19 18:42
```

Requirements:

- Active tab must be visually distinct.
- Date format should be configurable.
- Time format should support 24-hour and 12-hour modes.
- Clock should update without redrawing unchanged panes.
- Long tab lists should truncate or scroll safely.
- The status bar must not consume more than one row by default.
