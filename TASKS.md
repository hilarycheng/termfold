# Termfold Task List

This file tracks implementation work. Product behaviour remains authoritative in
`REQUIREMENTS.md`; workflow and approval rules remain authoritative in `AGENTS.md`.

## Workflow

- Complete tasks in dependency order.
- Each implementation requires an in-scope `APPROVE`.
- Tests, builds, documentation changes, dependency changes, and Git operations
  require explicit approval covering those actions.
- Run Linux-specific validation in Linux or WSL.
- Keep modules and dependencies to the minimum required by implemented behaviour.

## Tasks

- [x] **T00 — Resolve blocking decisions**
  - Define bounded queue and pending PTY-output caps before event-loop work.
  - Define graceful child-termination timeout.
  - Define valid configuration ranges and supported date/time format syntax.
  - Approve binary size, startup, idle memory, idle CPU, and minimum-kernel budgets
    before release validation.
  - Requirements: Resource Limits; Configuration; Implementation and Acceptance.
  - Depends on: none.
  - Done when: each value is added to the normative requirements with approval.

- [*] **T01 — Create the Rust baseline**
  - Create the binary crate, lockfile, stable-toolchain policy, musl target setup,
    and size-focused release profile.
  - Requirements: Implementation and Acceptance; release rules in `AGENTS.md`.
  - Depends on: none.
  - Done when: the minimal project structure and required build configuration exist.

- [*] **T02 — Implement CLI and configuration**
  - Implement the required commands including `diagnose` routing, PID-prefix selector,
    defaults, session-name validation, strict configuration parsing including
    `terminal_profile` and `inner_term`, and actionable errors.
  - Requirements: Command-Line Contract; Configuration.
  - Depends on: T00, T01.
  - Done when: every documented command parses and every configuration validation
    path behaves as specified. T10A completes `diagnose` output and compatibility.

- [*] **T03 — Implement session, tab, pane, and layout state**
  - Enforce resource limits, split constraints, deterministic focus, resize, and
    close behaviour without starting PTYs yet.
  - Requirements: Tabs and Panes; Resource Limits; Session and Process Lifecycle.
  - Depends on: T01.
  - Done when: state transitions cannot violate the documented limits or hierarchy.

- [*] **T04 — Implement secure runtime paths**
  - Validate runtime-directory ownership and permissions, reject symlinks, create
    the Unix socket securely, and handle stale sockets safely.
  - Requirements: IPC and Filesystem Security.
  - Depends on: T01.
  - Done when: runtime paths and sockets meet every ownership, mode, and type rule.

- [*] **T04A — Materialize private terminfo**
  - Check in and embed the approved `termfold-256color` entry, then atomically
    materialize and validate it below the secure runtime directory without
    following symlinks or replacing a non-regular file.
  - Requirements: Shell Launch and Inner Terminal Identity; Inner Terminal
    Behaviour; IPC and Filesystem Security.
  - Depends on: T04, T08.
  - Done when: the embedded entry matches the tested parser contract and session
    creation fails safely if its private materialization cannot be validated.

- [*] **T05 — Implement framed IPC**
  - Add versioned messages, the 1 MiB frame limit, malformed-frame rejection, and
    independently bounded multi-client connections with failure isolation.
  - Requirements: IPC and Filesystem Security; Command-Line Contract.
  - Depends on: T00, T04.
  - Done when: clients and server exchange only bounded, valid protocol messages,
    and one client failure cannot disrupt the session or another client.

- [*] **T06 — Implement PTY and child-process lifecycle**
  - Launch the approved shell directly with the required environment and working
    directory, including the approved inner `TERM`, `COLORTERM`, and `TERMINFO`;
    propagate sizes, terminate gracefully, and reap every child.
  - Requirements: Shell Launch and Inner Terminal Identity; Session and Process
    Lifecycle.
  - Depends on: T00, T01, T04A.
  - Done when: pane processes start, resize, terminate, and reap deterministically.

- [*] **T07 — Implement server lifecycle**
  - Add one server process per session, auto-start, PID-prefix discovery, create,
    attach, detach, list with attachment state, kill, empty-pane cascading, and
    shutdown with the session.
  - Requirements: Command-Line Contract; Session and Process Lifecycle.
  - Depends on: T02, T03, T05, T06.
  - Done when: sessions persist only while required, duplicate names are rejected
    per user, and concurrent same-user clients can share one session.

- [*] **T08 — Implement the terminal parser and screen model**
  - Support the required UTF-8, cell-width, cursor, scrolling, editing, SGR, screen,
    input-mode, and escape-sequence behaviour with bounded parsing.
  - Ignore OSC 52 writes and safely discard unsupported or oversized sequences.
  - Keep the embedded terminfo capabilities aligned with focused parser and
    renderer checks.
  - Requirements: Terminal Architecture; Inner Terminal Behaviour; Resource
    Limits.
  - Depends on: T01.
  - Done when: the advertised `termfold-256color` subset is represented correctly
    and every behaviour-changing capability has a mapped check.

- [*] **T09 — Implement client terminal safety**
  - Manage terminal modes, alternate screen, resize signals, disconnects, normal
    exit, panic, and catchable termination signals with deterministic restoration.
  - Requirements: First-Release Scope; Terminal Behaviour; Mouse and Scrollback.
  - Depends on: T05, T08.
  - Done when: every supported exit path restores the outer terminal.

- [*] **T10 — Implement pane and status rendering**
  - Render pane content, box-drawing borders with an ASCII fallback,
    active-pane state, and the one-row status bar with required truncation
    priorities and clock-only redraws.
  - Requirements: Tabs and Panes; Status Bar.
  - Depends on: T03, T08, T09.
  - Done when: normal and narrow layouts preserve the specified visibility order.

- [*] **T10A — Implement outer-terminal compatibility**
  - Add the required data-only terminal profiles, deterministic profile selection,
    per-client capability handling, safe colour and attribute downgrade, and
    rejection of terminals that cannot support the full-screen interface.
  - Requirements: Terminal Architecture; Outer Terminal Capabilities and Profiles;
    Colour and Attribute Adaptation; Terminal Diagnostics.
  - Depends on: T02, T04A, T06, T08, T09, T10.
  - Done when: each supported client renders and restores according to its selected
    profile, and `diagnose` reports the required decisions without exposing secrets.

- [*] **T11 — Implement keyboard input**
  - Forward bytes unchanged outside prefix mode and implement every required prefix
    command, resize mode, unsupported-command message, close confirmation, and
    the filename prompt and cancellation path for `Ctrl-b S` scrollback export.
  - Requirements: Default Keys.
  - Depends on: T03, T06, T09.
  - Done when: keyboard-only operation covers all first-release actions.

- [*] **T12 — Implement bounded scrollback**
  - Retain complete lines up to the configured limit, discard oldest lines first,
    implement the read-only scroll view, and save the active pane's retained
    scrollback as UTF-8 plain text without terminal control sequences or styling.
    Cancelling the filename prompt must not create or modify a file.
  - Requirements: Mouse and Scrollback; Configuration; Resource Limits.
  - Depends on: T00, T08, T11.
  - Done when: history remains bounded and navigable without corrupting pane output,
    and explicit export writes only the requested plain-text scrollback.

- [*] **T13 — Implement optional mouse input**
  - Keep mouse disabled by default; add SGR click, drag, wheel, tab selection, pane
    selection, border resize, application forwarding, and cleanup.
  - Requirements: Mouse and Scrollback.
  - Depends on: T03, T09, T10, T12.
  - Done when: mouse behaviour is complete without reducing keyboard functionality.

- [*] **T14 — Complete lifecycle and compatibility integration**
  - Verify attach/detach persistence, pane-exit cascading, resize propagation,
    bounded queues, SSH behaviour, WSL behaviour, and narrow-terminal handling.
  - Requirements: all first-release behavioural sections.
  - Depends on: T07 through T13; T10A.
  - Done when: all components operate together without terminal or process leaks.

- [*] **T14A — Add required project acknowledgements**
  - Document the required prior art accurately without implying endorsement,
    affiliation, code reuse, or compatibility certification.
  - Requirements: Prior Art and Acknowledgements.
  - Depends on: none.
  - Done when: project documentation credits zmx, tmux, xterm, and ncurses terminfo
    documentation as specified.

- [*] **T15 — Perform release validation**
  - Run the approved formatting, lint, test, security, musl build, static-linkage,
    checksum, compatibility, and resource-measurement checks.
  - Requirements: Implementation and Acceptance; release checklist in `AGENTS.md`.
  - Depends on: T00 through T14; T10A; T14A.
  - Done when: every approved budget and release-checklist item passes or has a
    documented blocker.
  - Validated on Ubuntu x86_64, Linux 7.0: 673 KiB static PIE, 0.00 s startup,
    2.7 MiB idle RSS, 0.05% idle CPU, SHA-256
    `6175cad43e0259e1bede3982f17c425e3be36a4620f9f7c8e1b7abdb8653636f`.
  - Blocked by unavailable environments: live Linux 4.18, WSL, SSH, WezTerm,
    Windows Terminal, and Windows Command Prompt runs. Automated PTY, profile,
    restoration, detach, and reattach checks passed.

- [*] **T16 — Add configurable status indicators and scrollback clearing**
  - Add active-pane scrollback clearing, a validated status template with right
    alignment, configurable labels and colours, and dependency-free Linux CPU,
    memory, and temperature indicators.
  - Requirements: Default Keys; Mouse and Scrollback; Status Bar; Configuration.
  - Depends on: T10, T11, T12.
  - Done when: focused input, terminal, configuration, rendering, integration,
    lint, and static release checks pass.

- [*] **T17 — Add status themes, key help, and searchable scroll mode**
  - Add ten embedded light/dark status themes, a paginated key-reminder help
    view, adaptive scroll-mode reminders, ends navigation, and bounded literal
    scrollback search.
  - Requirements: Default Keys; Mouse and Scrollback; Status Bar; Configuration.
  - Depends on: T10, T11, T12, T16.
  - Done when: focused input, terminal, configuration, rendering, integration,
    lint, and static release checks pass.

- [*] **T18 — Restore navigation-key compatibility**
  - Advertise and forward Arrow, Page Up, Page Down, Home, and End consistently
    through normal and application cursor-key modes.
  - Preserve escape-prefixed navigation keys when terminal input splits the
    escape byte from the rest of the sequence.
  - Requirements: Inner Terminal Behaviour; Outer Terminal Capabilities and Profiles.
  - Depends on: T08, T09, T10A, T11.
  - Done when: the compiled terminfo entry and focused cursor-mode checks agree.

- [*] **T19 — Refresh stale private terminfo**
  - Atomically replace a secure private terminfo entry when the embedded entry
    changes between Termfold builds.
  - Requirements: Inner Terminal Behaviour; IPC and Filesystem Security.
  - Depends on: T10A.
  - Done when: upgrades refresh stale entries while unsafe paths remain rejected.

- [*] **T20 — Preserve navigation in modal views**
  - Keep fragmented escape-prefixed navigation keys intact in scroll, help,
    search, and resize modes while standalone Escape still exits the mode.
  - Requirements: Default Keys; Mouse and Scrollback.
  - Depends on: T11, T12, T17, T18.
  - Done when: focused modal-input checks cover fragmented keys and Escape.

- [ ] **T21 — Add native x86-64 Windows backend**
  - Use ConPTY for panes, current-user named pipes for IPC, job objects for
    child cleanup, and Win32 console-mode restoration and system metrics.
  - Requirements: Distribution and Dependency Contract; IPC and Filesystem
    Security; Implementation and Acceptance.
  - Depends on: T04 through T14.
  - Native Windows release build passes at 433,664 bytes.
  - Native Windows test suite passes: 42 tests, including current-user named-pipe
    round-trip coverage.
  - Linux musl code path passes
    `cargo check --locked --target x86_64-unknown-linux-musl`.
  - Done when: the Windows build, focused lifecycle checks, and native
    Windows Terminal, WezTerm, and Command Prompt acceptance runs pass.

- [*] **T22 — Prevent PTY output starvation**
  - Read a bounded chunk from every ready pane per server-loop iteration instead
    of stopping after the first pane that produces output.
  - Keep PTY ingestion independent of attachment and client-render backpressure so
    inactive and detached panes continue updating bounded terminal history.
  - Depends on: T06, T07, T08.
  - Done when: a continuously noisy pane cannot delay another pane's screen or
    history updates, including across tab switches and detach/reattach.

- [*] **T23 — Fix immediate Windows session-server exit**
  - Diagnose and fix native Windows startup immediately reporting
    `termfold: session server exited with exit code: 0`.
  - Root-cause evidence and the constrained correction are recorded in
    [`WINDOWS_STARTUP_ANALYSIS.md`](WINDOWS_STARTUP_ANALYSIS.md).
  - Depends on: T21.
  - Done when: creating a Windows session leaves its server running and attaches
    the client successfully without the early-exit message.
  - Native Windows tests, lint, release build, and create/attach/kill acceptance
    passed.

- [*] **T24 — Confirm CLI session termination**
  - Implemented interactive confirmation for `termfold kill [NAME]`: `no` is the
    default, only `yes` confirms, and `no`, `Esc`, EOF, or invalid input cancel.
  - Added the documented explicit non-interactive override
    `termfold kill --yes [NAME]` for scripts.
  - Requirements: Approved Post-First-Release Scope; Session termination
    confirmation.
  - Depends on: T02, T07.
  - Done when: confirmation, cancellation, invalid input, and the approved
    non-interactive path preserve or terminate the session as specified.
  - Validation passed: Linux unit tests (48), Linux lifecycle tests (8), native
    Windows tests (43), Linux musl release build, and Windows MSVC release build.

- [*] **T25 — Optimize shared VT rendering**
  - Track changed row ranges, emit sequential range updates with minimal cursor and
    SGR changes, and retain full redraws for layout, resize, and buffer changes.
  - Measure renderer CPU, IPC transfer, and outer-terminal output before adding
    any platform-specific renderer path.
  - Requirements: Approved Post-First-Release Scope; Shared renderer optimization.
  - Depends on: T10, T21, T23.
  - Done when: approved focused checks pass and native Windows latency measurements
    show whether the shared renderer meets the target.
  - Implementation and focused checks are complete. Native Windows measurement was
    4.51–4.61 us / 48 VT bytes per incremental frame versus 1.087–1.088 ms /
    5,180 VT bytes for a full redraw; framed IPC adds 6 bytes. Full native
    validation has one unrelated runtime-ACL failure. WSL musl validation passed:
    50 unit tests, 8 lifecycle tests, Clippy, and a 713 KiB static-PIE release
    binary. WSL musl release PTY measurement: 100 samples, 598 us median and
    863 us p95 from outer PTY input through Termfold IPC/rendering to output.
    Native Windows ConPTY release-path measurement: three 100-sample runs,
    15.34–15.99 ms median and 18.05–20.76 ms p95, including Windows named-pipe
    IPC, ConPTY I/O, and shared VT rendering. The harness supplied native host
    query responses; this measures ConPTY rather than the Windows Terminal GUI.

- [ ] **T26 — Add declarative startup profiles**
  - Add creation-only `config.toml` profiles with validated directories, direct
    launch targets, tabs, and nested horizontal and vertical split trees.
  - Roll back every started target if validation or launch fails.
  - Requirements: Approved Post-First-Release Scope; Startup profiles.
  - Depends on: T02, T03, T06, T07.
  - Done when: profiles create the specified session atomically and attaching never
    reruns them.

- [ ] **T27 — Add bounded large-file viewer**
  - Add `Ctrl-b v`, `termfold view FILE`, OSC 7 working-directory tracking, the
    current-directory path prompt, bounded block reading, navigation, and literal
    forward/reverse search.
  - Requirements: Approved Post-First-Release Scope; Large-file viewer.
  - Depends on: T02, T03, T08, T10, T11.
  - Done when: large files remain bounded in memory, navigation and search work in
    both directions, and path fallback is deterministic without external commands.
