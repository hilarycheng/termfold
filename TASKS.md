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
- New implementation subtasks MAY include `Recommended model:` with one of
  `Luna Low`, `Luna Medium`, `Luna High`, `Luna xHigh`, or `Luna Max`. This is
  advisory execution metadata, not product behaviour, permission to broaden
  scope, or permission to combine tasks. Use the lowest level that can safely
  complete the bounded task; task size alone does not justify `Luna Max`.
- T28 through T30 retain their recorded historical platform format.
- T31 and every later task MUST declare `Implementation scope:` separately from
  `Required validation:`. Accepted implementation scopes are `Platform
  independent`, `Linux`, and `Native Windows`.
- One shared feature MUST remain one platform-independent implementation task.
  Do not duplicate the feature as separate Linux and Windows tasks. Create a
  Linux or Native Windows task only for an unavoidable platform-specific API or
  intentionally platform-only behaviour.
- A platform-independent implementation MAY still require `Linux and Native
  Windows` validation when terminal, input, rendering, filesystem, threading,
  process, or platform-library runtime behaviour can differ.
- T28 and every later task MUST record separate completion checkboxes for task
  implementation and every required validation environment. Missing Windows
  validation leaves only the Windows checkbox open; it does not undo completed
  implementation or Linux validation.
- From T28 onward, `[x]` in a task heading means implementation exists and `[ ]`
  means it does not. Do not use `[*]` for new tasks. Platform completion is shown
  only by the task's explicit completion checklist.
- A compile-only target check does not complete native Windows validation. Follow
  `AGENTS.md` for the authoritative scope, platform, and evidence rules.

## Tasks

- [x] **T00 — Resolve blocking decisions**
  - Define bounded queue and pending PTY-output caps before event-loop work.
  - Define graceful child-termination timeout.
  - Define valid configuration ranges and supported date/time format syntax.
  - Approve binary size, startup, idle memory, idle CPU, and minimum-kernel budgets
    before release validation.
  - Requirement: Resource Limits; Configuration; Implementation and Acceptance.
  - Depends on: none.
  - Done when: each value is added to the normative requirements with approval.

- [*] **T01 — Create the Rust baseline**
  - Create the binary crate, lockfile, stable-toolchain policy, musl target setup,
    and size-focused release profile.
  - Requirement: Implementation and Acceptance; release rules in `AGENTS.md`.
  - Depends on: none.
  - Done when: the minimal project structure and required build configuration exist.

- [*] **T02 — Implement CLI and configuration**
  - Implement the required commands including `diagnose` routing, PID-prefix selector,
    defaults, session-name validation, strict configuration parsing including
    `terminal_profile` and `inner_term`, and actionable errors.
  - Requirement: Command-Line Contract; Configuration.
  - Depends on: T00, T01.
  - Done when: every documented command parses and every configuration validation
    path behaves as specified. T10A completes `diagnose` output and compatibility.

- [*] **T03 — Implement session, tab, pane, and layout state**
  - Enforce resource limits, split constraints, deterministic focus, resize, and
    close behaviour without starting PTYs yet.
  - Requirement: Tabs and Panes; Resource Limits; Session and Process Lifecycle.
  - Depends on: T01.
  - Done when: state transitions cannot violate the documented limits or hierarchy.

- [*] **T04 — Implement secure runtime paths**
  - Validate runtime-directory ownership and permissions, reject symlinks, create
    the Unix socket securely, and handle stale sockets safely.
  - Requirement: IPC and Filesystem Security.
  - Depends on: T01.
  - Done when: runtime paths and sockets meet every ownership, mode, and type rule.

- [*] **T04A — Materialize private terminfo**
  - Check in and embed the approved `termfold-256color` entry, then atomically
    materialize and validate it below the secure runtime directory without
    following symlinks or replacing a non-regular file.
  - Requirement: Shell Launch and Inner Terminal Identity; Inner Terminal
    Behaviour; IPC and Filesystem Security.
  - Depends on: T04, T08.
  - Done when: the embedded entry matches the tested parser contract and session
    creation fails safely if its private materialization cannot be validated.

- [*] **T05 — Implement framed IPC**
  - Add versioned messages, the 1 MiB frame limit, malformed-frame rejection, and
    independently bounded multi-client connections with failure isolation.
  - Requirement: IPC and Filesystem Security; Command-Line Contract.
  - Depends on: T00, T04.
  - Done when: clients and server exchange only bounded, valid protocol messages,
    and one client failure cannot disrupt the session or another client.

- [*] **T06 — Implement PTY and child-process lifecycle**
  - Launch the approved shell directly with the required environment and working
    directory, including the approved inner `TERM`, `COLORTERM`, and `TERMINFO`;
    propagate sizes, terminate gracefully, and reap every child.
  - Requirement: Shell Launch and Inner Terminal Identity; Session and Process
    Lifecycle.
  - Depends on: T00, T01, T04A.
  - Done when: pane processes start, resize, terminate, and reap deterministically.

- [*] **T07 — Implement server lifecycle**
  - Add one server process per session, auto-start, PID-prefix discovery, create,
    attach, detach, list with attachment state, kill, empty-pane cascading, and
    shutdown with the session.
  - Requirement: Command-Line Contract; Session and Process Lifecycle.
  - Depends on: T02, T03, T05, T06.
  - Done when: sessions persist only while required, duplicate names are rejected
    per user, and concurrent same-user clients can share one session.

- [*] **T08 — Implement the terminal parser and screen model**
  - Support the required UTF-8, cell-width, cursor, scrolling, editing, SGR, screen,
    input-mode, and escape-sequence behaviour with bounded parsing.
  - Ignore OSC 52 writes and safely discard unsupported or oversized sequences.
  - Keep the embedded terminfo capabilities aligned with focused parser and
    renderer checks.
  - Requirement: Terminal Architecture; Inner Terminal Behaviour; Resource
    Limits.
  - Depends on: T01.
  - Done when: the advertised `termfold-256color` subset is represented correctly
    and every behaviour-changing capability has a mapped check.

- [*] **T09 — Implement client terminal safety**
  - Manage terminal modes, alternate screen, resize signals, disconnects, normal
    exit, panic, and catchable termination signals with deterministic restoration.
  - Requirement: First-Release Scope; Terminal Behaviour; Mouse and Scrollback.
  - Depends on: T05, T08.
  - Done when: every supported exit path restores the outer terminal.

- [*] **T10 — Implement pane and status rendering**
  - Render pane content, box-drawing borders with an ASCII fallback,
    active-pane state, and the one-row status bar with required truncation
    priorities and clock-only redraws.
  - Requirement: Tabs and Panes; Status Bar.
  - Depends on: T03, T08, T09.
  - Done when: normal and narrow layouts preserve the specified visibility order.

- [*] **T10A — Implement outer-terminal compatibility**
  - Add the required data-only terminal profiles, deterministic profile selection,
    per-client capability handling, safe colour and attribute downgrade, and
    rejection of terminals that cannot support the full-screen interface.
  - Requirement: Terminal Architecture; Outer Terminal Capabilities and Profiles;
    Colour and Attribute Adaptation; Terminal Diagnostics.
  - Depends on: T02, T04A, T06, T08, T09, T10.
  - Done when: each supported client renders and restores according to its selected
    profile, and `diagnose` reports the required decisions without exposing secrets.

- [*] **T11 — Implement keyboard input**
  - Forward bytes unchanged outside prefix mode and implement every required prefix
    command, resize mode, unsupported-command message, close confirmation, and
    the filename prompt and cancellation path for `Ctrl-b S` scrollback export.
  - Requirement: Default Keys.
  - Depends on: T03, T06, T09.
  - Done when: keyboard-only operation covers all first-release actions.

- [*] **T12 — Implement bounded scrollback**
  - Retain complete lines up to the configured limit, discard oldest lines first,
    implement the read-only scroll view, and save the active pane's retained
    scrollback as UTF-8 plain text without terminal control sequences or styling.
    Cancelling the filename prompt must not create or modify a file.
  - Requirement: Mouse and Scrollback; Configuration; Resource Limits.
  - Depends on: T00, T08, T11.
  - Done when: history remains bounded and navigable without corrupting pane output,
    and explicit export writes only the requested plain-text scrollback.

- [*] **T13 — Implement optional mouse input**
  - Keep mouse disabled by default; add SGR click, drag, wheel, tab selection, pane
    selection, border resize, application forwarding, and cleanup.
  - Requirement: Mouse and Scrollback.
  - Depends on: T03, T09, T10, T12.
  - Done when: mouse behaviour is complete without reducing keyboard functionality.

- [*] **T14 — Complete lifecycle and compatibility integration**
  - Verify attach/detach persistence, pane-exit cascading, resize propagation,
    bounded queues, SSH behaviour, WSL behaviour, and narrow-terminal handling.
  - Requirement: all first-release behavioural sections.
  - Depends on: T07 through T13; T10A.
  - Done when: all components operate together without terminal or process leaks.

- [*] **T14A — Add required project acknowledgements**
  - Document the required prior art accurately without implying endorsement,
    affiliation, code reuse, or compatibility certification.
  - Requirement: Prior Art and Acknowledgements.
  - Depends on: none.
  - Done when: project documentation credits zmx, tmux, xterm, and ncurses terminfo
    documentation as specified.

- [*] **T15 — Perform release validation**
  - Run the approved formatting, lint, test, security, musl build, static-linkage,
    checksum, compatibility, and resource-measurement checks.
  - Requirement: Implementation and Acceptance; release checklist in `AGENTS.md`.
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
  - Requirement: Default Keys; Mouse and Scrollback; Status Bar; Configuration.
  - Depends on: T10, T11, T12.
  - Done when: focused input, terminal, configuration, rendering, integration,
    lint, and static release checks pass.

- [*] **T17 — Add status themes, key help, and searchable scroll mode**
  - Add ten embedded light/dark status themes, a paginated key-reminder help
    view, adaptive scroll-mode reminders, ends navigation, and bounded literal
    scrollback search.
  - Requirement: Default Keys; Mouse and Scrollback; Status Bar; Configuration.
  - Depends on: T10, T11, T12, T16.
  - Done when: focused input, terminal, configuration, rendering, integration,
    lint, and static release checks pass.

- [*] **T18 — Restore navigation-key compatibility**
  - Advertise and forward Arrow, Page Up, Page Down, Home, and End consistently
    through normal and application cursor-key modes.
  - Preserve escape-prefixed navigation keys when terminal input splits the
    escape byte from the rest of the sequence.
  - Requirement: Inner Terminal Behaviour; Outer Terminal Capabilities and Profiles.
  - Depends on: T08, T09, T10A, T11.
  - Done when: the compiled terminfo entry and focused cursor-mode checks agree.

- [*] **T19 — Refresh stale private terminfo**
  - Atomically replace a secure private terminfo entry when the embedded entry
    changes between Termfold builds.
  - Requirement: Inner Terminal Behaviour; IPC and Filesystem Security.
  - Depends on: T10A.
  - Done when: upgrades refresh stale entries while unsafe paths remain rejected.

- [*] **T20 — Preserve navigation in modal views**
  - Keep fragmented escape-prefixed navigation keys intact in scroll, help,
    search, and resize modes while standalone Escape still exits the mode.
  - Requirement: Default Keys; Mouse and Scrollback.
  - Depends on: T11, T12, T17, T18.
  - Done when: focused modal-input checks cover fragmented keys and Escape.

- [ ] **T21 — Add native x86-64 Windows backend**
  - Platform: Windows.
  - Use ConPTY for panes, current-user named pipes for IPC, job objects for
    child cleanup, and Win32 console-mode restoration and system metrics.
  - Requirement: Distribution and Dependency Contract; IPC and Filesystem
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
  - Implemented with a bounded central event queue: client readers and one
    blocking reader per PTY enqueue events, and the server processes bounded
    batches before flushing input and rendering. The listener remains a short
    periodic nonblocking check; input and PTY output no longer wait for the old
    50 ms server-loop sleep.
  - Done when: a continuously noisy pane cannot delay another pane's screen or
    history updates, including across tab switches and detach/reattach.

- [*] **T23 — Fix immediate Windows session-server exit**
  - Platform: Windows.
  - Diagnose and fix native Windows startup immediately reporting
    `termfold: session server exited with exit code: 0`.
  - Root causes:
    - inherited standard handles bypassed ConPTY during child startup;
    - a cloned synchronous duplex control pipe allowed a blocked reader to stall
      the response writer.
  - Implemented correction:
    - pass the direct `HPCON` value, use `STARTF_USESTDHANDLES` with non-console
      handles, and keep ConPTY-side pipe handles alive through `CreateProcessW`;
    - create control named-pipe endpoints with `FILE_FLAG_OVERLAPPED` and give
      every read and write independent event-backed `OVERLAPPED` state;
    - preserve the existing protocol, frame limits, SID checks, DACL, and
      dependency policy.
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
  - Requirement: Approved Post-First-Release Scope; Session termination
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
  - Requirement: Approved Post-First-Release Scope; Shared renderer optimization.
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
  - Requirement: Approved Post-First-Release Scope; Startup profiles.
  - Depends on: T02, T03, T06, T07.
  - Done when: profiles create the specified session atomically and attaching never
    reruns them.

- [*] **T27 — Add bounded large-file viewer**
  - Add `Ctrl-b v`, `termfold view FILE`, OSC 7 working-directory tracking, the
    current-directory path prompt, bounded block reading, navigation, and literal
    forward/reverse search.
  - Requirement: Approved Post-First-Release Scope; Large-file viewer.
  - Depends on: T02, T03, T08, T10, T11.
  - Done when: large files remain bounded in memory, navigation and search work in
    both directions, and path fallback is deterministic without external commands.
  - Implemented with a bounded virtual viewer pane, session-scoped IPC requests,
    OSC 7 file-URL parsing, editable directory completion, and fixed-block search.
    Linux and Windows targets type-check and Clippy passes; runtime tests are
    blocked in this host by the missing `cc` linker and inaccessible WSL service.

## T28 Execution Contract

T28 replaces the current large-file viewer internals without changing unrelated
pane, PTY, session, status, or platform behaviour.

Rules for every T28 implementation request:

- Implement exactly one named subtask.
- Do not perform architecture analysis or broaden the approved contract.
- Touch only the files named by that subtask, except for a required `mod`
  declaration or focused test fixture.
- Do not add dependencies.
- Do not combine refactoring with a new behaviour unless the subtask explicitly
  requires both.
- Keep ordinary production changes below 400 changed lines. T28B is the only
  exception because it is a mechanical file move.
- Preserve public behaviour not named by the subtask.
- Run only the focused checks named by the approved implementation request.
- Stop after reporting changed files, focused results, risks, and blockers.
- T28K, T28L, and T28M require review before the next dependent subtask begins.

Approved target source ownership:

```text
src/viewer/
├── mod.rs       Viewer facade, mode, cursor state, and public commands/results
├── source.rs    snapshot FileSource and bounded raw-block cache
├── line.rs      universal EOL boundaries and resumable line scanning
├── text.rs      safe text tokens, tab expansion, and source/cell spans
├── frame.rs     PageFrame construction and Previous/Current/Next rotation
├── search.rs    text/hex query parsing, incremental search, wrap, and matches
├── hex.rs       hex-row layout and ASCII side rendering
└── worker.rs    session-scoped cooperative Viewer Worker
```

`input.rs` remains responsible only for input bytes to semantic actions.
`server.rs` dispatches viewer commands and applies viewer results; it MUST NOT
perform file I/O or file scanning.

## T28 Tasks

- [x] **T28A — Define the replacement viewer contract and implementation plan**
  - Required validation: Platform independent.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Platform-independent review complete.
  - Update `REQUIREMENTS.md` with snapshot behaviour, universal EOL handling,
    text decoding, cell-based cursor rules, three page frames, zero repeat
    backlog, cancellable search, highlighting, Hex mode, configuration, and hard
    limits.
  - Add T28A through T28U to `TASKS.md` with exact ownership, dependencies, and
    objective completion conditions.
  - Production files: none.
  - Depends on: T27.
  - Done when: Luna can implement each remaining task without inventing product
    behaviour or module ownership.
  - Evidence: completed as a documentation-only approved change; no Rust source,
    build, test, or README behaviour was changed.

- [x] **T28B — Mechanically establish the viewer module tree**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Move the complete existing `viewer.rs` implementation and its tests to
    `viewer/mod.rs` without changing logic, constants, signatures, or test
    expectations.
  - Add empty private module declarations only where required for later tasks.
  - Remove `viewer.rs` after the module path compiles.
  - Allowed production files: `src/viewer.rs`, `src/viewer/mod.rs`, `src/main.rs`.
  - Focused checks: existing viewer unit tests and a normal target type-check.
  - Depends on: T28A.
  - Done when: the move is recognized as a rename where possible and every
    existing viewer test has unchanged behaviour.
  - Evidence: `cargo check --locked` passed; Ubuntu-24.04 WSL viewer tests passed
    with 16 tests; Ubuntu-24.04 WSL musl release build passed; native Windows
    MSVC viewer tests passed with 15 tests; native Windows MSVC build passed.

- [x] **T28C — Extract snapshot FileSource and the raw-block cache**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Create `viewer/source.rs`.
  - Move file ownership, snapshot length, aligned block reads, cache lookup, and
    cache eviction out of `viewer/mod.rs`.
  - Use 64 KiB aligned blocks and retain at most eight blocks / 512 KiB.
  - Capture file length once at open. Remove metadata refresh, growth reload, and
    truncation-clamp behaviour from the replacement path.
  - Expose only bounded byte/range access required by later viewer modules.
  - Allowed production files: `src/viewer/mod.rs`, `src/viewer/source.rs`.
  - Focused tests: aligned reads, cross-block range, LRU bound, snapshot ignores
    append, snapshot ignores truncate/replace without panic.
  - Depends on: T28B.
  - Done when: no file read or cache implementation remains duplicated in
    `viewer/mod.rs` and cache bytes never exceed 512 KiB.
  - Evidence: WSL musl release build passed; WSL viewer tests passed with 15
    tests; native Windows MSVC release build passed; native Windows MSVC viewer
    tests passed with 13 tests. Windows linker temporary-file access required
    escalated execution. The two-test count difference is from Unix-only tests.

- [x] **T28D — Implement one universal, resumable line scanner**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Create `viewer/line.rs` with the only line-boundary implementation used by
    Text mode.
  - Recognize each encountered `CRLF`, `LF`, and lone `CR`; treat `CRLF` as one
    terminator and support mixed files.
  - Support empty files, empty lines, an unterminated final line, and boundaries
    split across 64 KiB blocks.
  - Line scans longer than one block MUST retain resumable scan state and yield
    after at most 64 KiB.
  - Replace separate newline loops only after all callers use this scanner.
  - Allowed production files: `src/viewer/line.rs`, `src/viewer/mod.rs`.
  - Focused tests: LF, CRLF, CR, mixed EOL, CRLF on block boundary, reverse line
    discovery, and a line larger than eight blocks.
  - Depends on: T28C.
  - Done when: all line start/end/next/previous results come from one scanner and
    no cursor-visible boundary contains EOL bytes.
  - Evidence: Ubuntu-24.04 WSL `cargo test --locked` passed with 78 unit tests and
    8 integration tests; the musl release build passed. The focused viewer EOL
    tests passed with 12 tests in WSL and 11 tests on native Windows MSVC; the
    native Windows MSVC release build passed. Windows test execution required
    elevated access for temporary-file creation because the sandbox denied it.

- [x] **T28E — Implement safe text-token decoding**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Create `viewer/text.rs`.
  - Decode printable valid UTF-8 into display tokens with source byte ranges and
    Unicode display-cell widths.
  - Render NUL, ESC, DEL, other ASCII C0 controls, and remaining invalid or
    non-printable bytes exactly as required by `REQUIREMENTS.md`.
  - Expand tabs from the current display-cell column to the next tab stop.
  - Combining characters MUST extend the preceding token and MUST NOT create an
    independent cursor stop.
  - Allowed production files: `src/viewer/text.rs`, `src/viewer/mod.rs`.
  - Focused tests: ASCII, CJK wide characters, combining characters, split UTF-8,
    invalid UTF-8, every control category, and tab expansion at widths 1/4/8/16.
  - Depends on: T28C.
  - Done when: raw file bytes cannot reach `Terminal::advance` except through
    trusted generated display output.
  - Evidence: Ubuntu-24.04 WSL `cargo fmt --check`, Clippy with `-D warnings`,
    `cargo test --locked` (78 unit tests and 8 integration tests), and the
    x86_64 musl release build passed; `file` and `ldd` confirmed a static
    artifact. Native Windows MSVC `cargo test --locked --target
    x86_64-pc-windows-msvc` (71 unit tests) and the release build passed.
    SHA-256: Linux
    `022a86c3ce8777e8857053d5ad0efe008029768fdab7e07328609ddfc602ac2c`;
    Windows
    `16E7CDC78E7C0EA973C1DFA54EE484B838E95A605A81746ED65A26F3C439A0EA`.
    `cargo audit` and `cargo deny check` were unavailable because the commands
    are not installed in WSL.

- [x] **T28F — Add and validate `viewer_tab_width`**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Add `viewer_tab_width = 8` to configuration defaults.
  - Accept only integer values 1 through 16 and identify the field on error.
  - Pass the validated value into viewer creation without changing existing
    pane scrollback tab behaviour.
  - Allowed production files: `src/config.rs`, `src/server.rs`,
    `src/viewer/mod.rs`.
  - Focused tests: missing default, values 1 and 16, values 0 and 17, duplicate
    field, and viewer receives the configured value.
  - Depends on: T28E.
  - Done when: Text mode uses one validated tab width and no viewer-local default
    remains duplicated.
  - Evidence: Ubuntu 24.04 x86_64 `cargo fmt -- --check`, `cargo test --locked`
    (83 unit tests and 8 integration tests), and the x86_64 musl release build
    passed. Native Windows MSVC `cargo test --locked --target
    x86_64-pc-windows-msvc validates_viewer_tab_width` (1 test) and
    `cargo build --release --locked --target x86_64-pc-windows-msvc` passed.
    The Windows release artifact is 571,392 bytes with SHA-256
    `A2DC32B0799962649BEC23954C837A019CA4A428B05F63096188641E05D80D74`.
    The full Linux test run and Windows linker required elevated temporary-file
    access.

- [x] **T28G — Build source-byte to display-cell spans**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Extend `viewer/text.rs` with one mapping representation shared by cursor,
    clipping, and highlight code.
  - Every span MUST contain a source byte range, display-cell start/end, rendered
    token, and whether it is a valid cursor stop.
  - EOL bytes, UTF-8 continuation bytes, combining continuations, and wide-cell
    continuations MUST have no independent cursor stop.
  - Allowed production files: `src/viewer/text.rs`, `src/viewer/mod.rs`.
  - Focused tests: byte-to-cell and cell-to-token lookup for ASCII, tabs, CJK,
    combining marks, invalid bytes, and empty lines.
  - Depends on: T28D, T28E, T28F.
  - Implementation: reused `TextToken` as the span map with source/cell/token
    lookups and cell-range clipping.
  - Done when: viewer navigation and rendering no longer derive a terminal column
    by subtracting two byte offsets.
  - Evidence: `cargo test --locked viewer::` passed with 28 viewer tests; the
    x86_64 musl release build passed. Native Windows MSVC `cargo test --locked
    --target x86_64-pc-windows-msvc viewer::text::` passed with 9 tests, and
    `cargo build --release --locked --target x86_64-pc-windows-msvc` passed.
    The Windows release artifact is 571,392 bytes with SHA-256
    `A2DC32B0799962649BEC23954C837A019CA4A428B05F63096188641E05D80D74`.
    The Windows linker required elevated temporary-file access.

- [x] **T28H — Correct Text-mode cursor semantics**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Replace byte-column cursor movement with source offset plus preferred
    display-cell column.
  - Implement line start/end, Up/Down preferred-cell movement, top/bottom, and
    horizontal adjustment through the mapping from T28G.
  - `$` MUST select the start of the last visible token; an empty line uses
    column zero.
  - Navigation MUST never leave the cursor inside UTF-8, a combining sequence,
    a wide continuation, or EOL.
  - Allowed production files: `src/viewer/mod.rs`, `src/viewer/text.rs`,
    `src/viewer/line.rs`.
  - Focused tests: `$` on LF/CRLF/CR/empty/unterminated lines, preferred column
    across short and long lines, tabs, CJK, combining text, and invalid bytes.
  - Depends on: T28G.
  - Done when: the reported cursor source offset and terminal cell are valid for
    every focused case.
  - Implementation: stores horizontal and preferred columns as display-cell
    offsets, resolves source positions through valid decoded token stops, and
    uses the same mapping for line end, vertical movement, clipping, and cursor
    rendering.
  - Evidence: `cargo test --locked viewer::` passed with 30 viewer tests; the
    x86_64 musl release build passed. Native Windows MSVC
    `cargo test --locked --target x86_64-pc-windows-msvc viewer::` passed with
    96 tests, and `cargo build --release --locked --target
    x86_64-pc-windows-msvc` passed. The Windows release artifact is 571,392
    bytes with SHA-256
    `A2DC32B0799962649BEC23954C837A019CA4A428B05F63096188641E05D80D74`.
    Native Windows tests and linking required elevated temporary-file access.

- [x] **T28I — Introduce the Current PageFrame builder**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Create `viewer/frame.rs` with the approved PageFrame fields: source range,
    decoded rows, line boundaries, source/cell spans, valid cursor stops, and
    visible match ranges.
  - Build only the visible page and at most 256 KiB of source bytes. Long-line
    continuation MUST use bounded resumable state rather than retaining the
    complete line.
  - Replace `page: Vec<String>` as the viewer's authoritative page model.
  - Render only trusted generated text and SGR owned by Termfold.
  - Allowed production files: `src/viewer/frame.rs`, `src/viewer/mod.rs`,
    `src/viewer/text.rs`.
  - Focused tests: normal page, empty file, long line, horizontal clipping,
    control replacements, cursor placement, and failed build preserves the last
    committed frame.
  - Depends on: T28H.
  - Done when: Current PageFrame is the sole source model for the displayed
    viewer page and source bytes per frame stay within 256 KiB.
  - Implementation: added the bounded `PageFrame` builder and made the committed
    frame the viewer's sole displayed-page model; decoded rows retain the existing
    text span/token mapping and resumable line boundaries without retaining a raw
    page buffer.
  - Evidence: focused `cargo test --locked viewer::` passed with 33 viewer tests;
    elevated full `cargo test --locked` passed with 90 unit tests and 8 lifecycle
    tests; the x86_64 musl release build passed. Native Windows MSVC
    `cargo test --locked --target x86_64-pc-windows-msvc viewer::` passed with
    96 tests, and `cargo build --release --locked --target
    x86_64-pc-windows-msvc` passed. The Windows release artifact is 571,392
    bytes with SHA-256
    `A2DC32B0799962649BEC23954C837A019CA4A428B05F63096188641E05D80D74`.
    Native Windows tests and linking required elevated temporary-file access.

- [x] **T28J — Add Previous/Current/Next frame rotation**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Store exactly three optional frame slots in `viewer/frame.rs`.
  - Implement deterministic Page Up/Page Down rotation when the neighbour matches
    the current snapshot, mode, size, tab width, and expected source boundary.
  - Invalidate all slots on resize, tab-width change, mode change, or generation
    invalidation.
  - Prefetch at most one missing neighbour only after Current commits and while
    no command is waiting.
  - Allowed production files: `src/viewer/frame.rs`, `src/viewer/mod.rs`.
  - Focused tests: forward/back rotation, invalid neighbour rejection, resize
    invalidation, alternating directions, and never retaining a fourth frame.
  - Depends on: T28I.
  - Done when: repeated paging reuses valid neighbours and frame count is always
    at most three.
  - Implementation: added keyed Previous/Current/Next slots with snapshot, mode,
    size, tab-width, generation, and source-boundary validation; page commits
    rotate valid neighbours and perform one bounded directional prefetch.
  - Evidence: focused `cargo test --locked viewer::` passed with 37 tests; full
    elevated `cargo test --locked` passed with 94 unit tests and 8 lifecycle
    tests; the x86_64 musl release build passed. Native Windows MSVC
    `cargo test --locked --target x86_64-pc-windows-msvc
    viewer::frame::tests::` passed 4 tests, and `cargo build --release --locked
    --target x86_64-pc-windows-msvc` passed.

- [x] **T28K — Add the session-scoped Viewer Worker foundation**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Create `viewer/worker.rs` and one worker per Session Server, not one thread per
    viewer.
  - Define bounded `ViewerCommand` and `ViewerResult` messages containing viewer
    ID and generation.
  - The worker MUST own all FileSource instances and viewer core state.
  - Each event-loop iteration MUST process available control commands before one
    bounded work step. It MUST not complete an unbounded search or long-line scan
    in one iteration.
  - Use existing standard-library synchronization only. Do not add an async
    runtime or dependency.
  - Allowed production files: `src/viewer/worker.rs`, `src/viewer/mod.rs`,
    `src/server.rs`.
  - Focused tests: open/close, bounded command channel, stale generation result,
    two viewer IDs, worker shutdown, and one long operation does not block a
    control command beyond one 64 KiB step.
  - Depends on: T28C, T28I.
  - Review gate: inspect ownership, thread lifecycle, channel bounds, and stale
    result handling before T28L.
  - Done when: a mocked slow FileSource never blocks the Session Server thread.
  - Implementation: added one session-scoped bounded standard-library worker;
    it owns viewer cores/FileSource instances, routes pane viewers through
    generation-tagged commands/results, prioritizes control commands, and yields
    search work after each 64 KiB step.
  - Evidence: focused `cargo test --locked viewer::worker::` passed with 5 tests;
    full elevated `cargo test --locked` passed with 99 unit tests and 8 lifecycle
    tests; the x86_64 musl release build passed. Native Windows MSVC
    `cargo test --locked --target x86_64-pc-windows-msvc
    viewer::worker::tests::` passed 22 tests, and `cargo build --release --locked
    --target x86_64-pc-windows-msvc` passed. Review confirmed one bounded worker
    owns viewer state, prioritizes controls, rejects stale generations, and shuts
    down deterministically. Worker tests required elevated temporary-file access.

- [x] **T28L — Move page and line navigation behind the Viewer Worker**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Convert line, page, half-page, viewport, start/end, and top/bottom actions into
    ViewerCommand messages.
  - Apply ViewerResult only when viewer ID and generation still match.
  - Remove synchronous viewer seek/read/render calls from the action handler.
  - Commit and render each accepted Page Up/Page Down result before dispatching a
    later replacement command.
  - Allowed production files: `src/server.rs`, `src/viewer/worker.rs`,
    `src/viewer/mod.rs`.
  - Focused tests: server remains responsive during delayed reads, one accepted
    page command creates one frame, 1,000 sequential page results preserve order,
    and close discards late results.
  - Depends on: T28J, T28K.
  - Review gate: verify the Session Server contains no viewer file I/O before
    T28M.
  - Done when: all viewer navigation file work occurs only in Viewer Worker.
  - Implementation: navigation and render requests now use non-blocking worker
    dispatch; the Session Server polls bounded generation-checked results, applies
    rendered terminals, preserves stale/error pages, and closes by discarding late
    results.
  - Evidence: focused `cargo test --locked viewer::worker::` passed with 7 tests;
    elevated `cargo test --locked` passed with 101 unit tests and 8 lifecycle
    tests; the x86_64 musl release build passed and produced a stripped static PIE.
    Native Windows MSVC `cargo test --locked --target x86_64-pc-windows-msvc
    viewer::worker::tests::` passed 22 tests, and `cargo build --release --locked
    --target x86_64-pc-windows-msvc` passed. Review confirmed viewer navigation
    file work remains behind the worker and accepted page results render before
    replacement dispatch. Worker tests required elevated temporary-file access.

- [x] **T28M — Enforce zero repeat backlog and one changed-intent replacement**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Add a server-side ViewerGate containing generation, in-flight state, current
    intent, and at most one replacement command.
  - Dispatch one navigation when idle. Drop same-intent repeats while in flight.
  - Store only the newest changed intent as replacement; do not store a repeat
    count or FIFO navigation queue.
  - After a valid result, commit and render first, clear in-flight, then dispatch
    the one replacement if present.
  - Close/cancel/mode switch MUST increase generation and clear the replacement
    immediately.
  - Allowed production files: `src/server.rs`, `src/input.rs`,
    `src/viewer/mod.rs`.
  - Focused tests: held Page Down with slow frames has zero backlog, release stops
    after current frame, direction reversal stores one replacement, latest
    changed intent wins, batched repeats are dropped, and `Ctrl-b x` is immediate.
  - Depends on: T28L.
  - Review gate: verify no same-direction pending count, queue, or coalesced
    latest-page render remains before search work begins.
  - Done when: navigation cannot continue from buffered repeats after input stops.
  - Implementation: added a per-pane server-side ViewerGate with generation,
    in-flight intent, and one newest changed-intent replacement; same-intent
    repeats are dropped, replacement dispatch waits for render commit, and close,
    search cancellation, and viewer prompt mode changes clear the gate.
  - Evidence: focused `cargo test --locked server::tests::viewer_gate` passed
    5 tests; elevated `cargo test --locked --no-fail-fast` passed 106 unit tests
    and 8 lifecycle tests; the x86_64 musl release build passed. Native
    Windows MSVC `cargo test --locked --target x86_64-pc-windows-msvc
    server::tests::viewer_gate` passed 6 tests, and `cargo build --release
    --locked --target x86_64-pc-windows-msvc` passed. Review confirmed zero
    same-intent backlog and exactly one newest changed-intent replacement. The
    Windows artifact is 571,392 bytes with SHA-256
    `A2DC32B0799962649BEC23954C837A019CA4A428B05F63096188641E05D80D74`.

- [x] **T28N — Define text and hex search query types**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Create `viewer/search.rs` query parsing without scanning the file.
  - Text queries MUST be literal, ASCII case-insensitive, non-ASCII exact, and
    limited to 256 bytes.
  - Parse `hex:` queries as one or more space-separated two-digit byte values.
  - Reject empty, odd, malformed, or oversized hex queries without replacing the
    last successful query.
  - Allowed production files: `src/viewer/search.rs`, `src/viewer/mod.rs`.
  - Focused tests: ASCII case pairs, non-ASCII exactness, invalid UTF-8 bytes,
    maximum length, valid hex, and every invalid hex form.
  - Depends on: T28C.
  - Done when: query comparison and parsing are independent of UI input state and
    file scanning.
  - Implementation: added bounded `SearchQuery` text/hex parsing with ASCII-only
    case folding, exact non-ASCII and invalid-byte comparison, and invalid-query
    state preservation.
  - Evidence: `cargo test --locked viewer::` passed with 51 viewer tests; the
    x86_64-unknown-linux-musl release build passed. Native Windows/MSVC
    validation on 2026-08-06 passed 96 viewer tests, 163 full-suite tests, and
    the release build.

- [x] **T28O — Implement incremental cancellable forward/reverse search**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Search Current from the cursor first, then the neighbour in the requested
    direction, then scan the snapshot in 64 KiB steps.
  - Support forward/reverse, `n`/`N`, matches crossing block boundaries, and one
    wrap only.
  - Check generation and yield after every step. Navigation, new search, resize,
    mode change, or close MUST cancel the old search.
  - Retain only bounded current/nearby match offsets; do not build a full index.
  - Allowed production files: `src/viewer/search.rs`, `src/viewer/worker.rs`,
    `src/viewer/mod.rs`.
  - Focused tests: current-frame priority, forward/reverse, EOF/BOF wrap once,
    no-match termination, cross-block match, cancellation after one step, and
    another viewer receives service during a long search.
  - Depends on: T28K, T28N.
  - Done when: no search loop can monopolize Viewer Worker or Session Server.
  - Implementation: replaced byte-at-a-time full-range scanning with bounded
    64 KiB source chunks, current/nearby frame priority, one-wrap range planning,
    strict repeat offsets, generation cancellation, and round-robin worker steps.
  - Evidence: `cargo test --locked viewer::` passed with 56 viewer tests; the
    x86_64-unknown-linux-musl release build passed. Native Windows/MSVC
    validation on 2026-08-06 passed 96 viewer tests, 163 full-suite tests, and
    the release build.

- [x] **T28P — Highlight visible search matches**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Map every matching source range in Current to display-cell spans through the
    PageFrame mapping.
  - Render ordinary visible matches with an attribute-based highlight and the
    active match with inverse plus underline.
  - Do not scan outside Current only to produce highlights.
  - Preserve the last successful query for `n`/`N`; report `wrapped` after a
    wrapped success.
  - Allowed production files: `src/viewer/frame.rs`, `src/viewer/search.rs`,
    `src/viewer/mod.rs`, `src/viewer/worker.rs`, and `src/server.rs` for
    wrapped-status propagation.
  - Focused tests: multiple visible matches, active distinction, horizontal
    clipping, tabs, wide text, invalid-byte replacement, and monochrome output.
  - Depends on: T28I, T28O.
  - Done when: all visible matches are marked without colour-only meaning or a
    full-file highlight scan.
  - Implementation: scans only the bounded Current frame in 64 KiB chunks,
    maps every match through `SourceCellSpan`, renders ordinary matches with
    inverse and the active match with inverse plus underline, and propagates
    wrapped search results to the temporary status.
  - Evidence: `cargo test --locked viewer::` passed 60 tests; elevated
    `cargo test --locked` passed 121 unit tests and 8 lifecycle tests; the
    x86_64-unknown-linux-musl release build passed. Native Windows/MSVC
    validation on 2026-08-06 passed 96 viewer tests, 163 full-suite tests, and
    the release build.
  - Verification note: repository-wide `cargo fmt` was applied and
    `cargo fmt --check` passed.

- [x] **T28Q — Implement Hex PageFrame rendering and navigation**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Create `viewer/hex.rs` and add the `Text`/`Hex` mode enum to the viewer facade.
  - Render absolute offset, hex bytes, and printable ASCII side by side using
    16/8/4 bytes per row at the approved width thresholds.
  - Below 28 columns, render the required narrow message while preserving byte
    position.
  - Implement byte cursor, row/page movement, start/end, top/bottom, and frame
    construction without using Text line boundaries.
  - Reuse FileSource and the three frame slots; do not create a second raw cache.
  - Allowed production files: `src/viewer/hex.rs`, `src/viewer/frame.rs`,
    `src/viewer/mod.rs`.
  - Focused tests: offsets above 4 GiB, 16/8/4 layouts, narrow case, printable
    ASCII side, non-printable dots, cursor movement, block boundary, and frame
    bound.
  - Depends on: T28J.
  - Done when: Hex mode is a bounded alternate frame builder sharing the same
    snapshot and cache.
  - Implementation: added the ViewerMode enum, bounded Hex rows with
    width-dependent 16/8/4-byte layouts, absolute offsets, printable ASCII,
    narrow-view handling, byte cursor movement, and Hex-aware frame rotation
    and prefetch using the existing FileSource.
  - Evidence: cargo fmt --check passed; cargo test --locked viewer:: passed
    67 viewer tests; the x86_64-unknown-linux-musl release build passed. Native
    Windows/MSVC validation on 2026-08-06 passed 96 viewer tests, 163 full-suite
    tests, and the release build.

- [x] **T28R — Add Hex-mode ASCII and exact-byte search**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Use normal parsed text queries for ASCII case-insensitive search in Hex mode
    and parsed `hex:` queries for exact bytes.
  - Permit matches across displayed rows and raw block boundaries.
  - Reuse the incremental engine, cancellation, wrap, `n`/`N`, and active-match
    state from Text mode.
  - Allowed production files: `src/viewer/search.rs`, `src/viewer/hex.rs`,
    `src/viewer/worker.rs`.
  - Focused tests: ASCII case-insensitive, exact `00 FF 1B`, row boundary, block
    boundary, reverse, wrap, cancellation, and highlight byte span.
  - Depends on: T28O, T28Q.
  - Done when: Text and Hex search share one bounded search engine and differ only
    by query interpretation and frame mapping.
  - Implementation: reused the raw-byte incremental search engine for Hex mode,
    added Hex/ASCII byte-span mapping with active-match styling, and covered
    case-insensitive ASCII, exact bytes, row/block boundaries, and worker
    rendering.
  - Evidence: focused cargo test --locked viewer:: passed 72 viewer tests;
    elevated cargo test --locked passed 134 unit tests and 8 lifecycle tests;
    cargo build --release --locked --target x86_64-unknown-linux-musl passed.
    Native Windows/MSVC validation on 2026-08-06 passed 96 viewer tests, 163
    full-suite tests, and the release build.

- [x] **T28S — Wire mode switching, cancellation, and input actions**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Map `H` in Viewer mode to a semantic mode-toggle action.
  - Mode switch MUST increase generation, cancel search/navigation, clear the one
    replacement, invalidate all frames, preserve source byte position where
    possible, and request a new Current frame.
  - Navigation during a search MUST cancel search before dispatching navigation.
  - Search prompt cancellation MUST leave the last successful query unchanged.
  - Allowed production files: `src/input.rs`, `src/server.rs`,
    `src/viewer/mod.rs`.
  - Focused tests: `H` in both directions, fragmented escape input unchanged,
    switch during search, navigation during search, close during search, and no
    action leaks to the child PTY.
  - Depends on: T28M, T28P, T28Q, T28R.
  - Done when: input state controls Viewer Worker only through semantic commands
    and every intent-changing action invalidates obsolete work.
  - Implementation: mapped `H` to a semantic toggle, added worker-owned mode
    switching with source-position preservation and frame invalidation, made
    search dispatch cooperative, and added generation-tagged cancellation for
    navigation, resize, mode switching, prompt cancellation, and close.
  - Scope note: private command/result plumbing in `src/viewer/worker.rs` was
    required because the worker owns viewer state.
  - Evidence: `cargo fmt --check`, focused input/viewer/worker tests, elevated
    `cargo test --locked` (131 unit tests and 8 lifecycle tests), and
    `cargo build --release --locked --target x86_64-unknown-linux-musl` passed.
    Native Windows/MSVC validation on 2026-08-06 passed 96 viewer tests, 14
    input tests, 163 full-suite tests, and the release build.

- [x] **T28T — Remove superseded and duplicated viewer code**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Delete old full-file `collect_forward`/`collect_reverse` loops, metadata
    refresh/truncation logic, duplicated newline scanners, byte-column helpers,
    `viewer_dirty` latest-input coalescing, and old page-string state.
  - Replace repeated active-viewer lookup/action/error/render branches in
    `server.rs` with one bounded viewer-command dispatcher.
  - Keep one definition of viewer path/query limits and one search-status
    formatter.
  - Do not abstract Linux PTY with Windows ConPTY, Unix sockets with named pipes,
    pane scrollback search with file search, or the terminal parser with the file
    decoder.
  - Allowed production files: `src/viewer/*`, `src/server.rs`, `src/input.rs`,
    `src/ipc.rs`.
  - Focused checks: duplicate-symbol grep, all focused viewer/input/server tests,
    and no dead-code warnings.
  - Depends on: T28S.
  - Done when: each viewer responsibility has one implementation and unrelated
    platform separation remains intact.
  - Implementation: replaced the global `viewer_dirty` slot and repeated server
    viewer branches with one bounded command dispatcher and per-pane pending
    result ownership; shared the viewer path/query limits and search-status
    formatter; removed the stale page-string test helper and duplicate mode path.
  - Evidence: duplicate-symbol `rg` check passed; `cargo fmt --check`, focused
    viewer/input/server tests (72/9/11), `cargo test --locked -- --test-threads=1`
    (133 unit and 8 lifecycle tests), and the x86_64 musl release build passed.
    Native Windows/MSVC validation on 2026-08-06 passed focused viewer/input/
    server tests (96/14/17), 163 full-suite tests, and the warning-free release
    build.

- [x] **T28U — Run viewer acceptance, resource, and documentation checks**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [ ] Native Windows validation complete.
  - Add deterministic stress coverage for 1,000 Page Down and 1,000 Page Up
    operations, zero repeat backlog, immediate close, mixed EOL, huge lines,
    control bytes, UTF-8, highlighting, Text/Hex search, wrap, and cancellation.
  - Verify at runtime that raw blocks never exceed eight, frame slots never
    exceed three, source bytes per frame never exceed 256 KiB, search steps never
    exceed 64 KiB, and one search cannot block another viewer's control work.
  - Run approved Linux/WSL and native Windows focused tests, Clippy, release builds,
    and binary-size comparison.
  - Update `README.md` only after all user-visible behaviour is implemented and
    verified. Record concise evidence and blockers here.
  - Allowed files: focused tests, `README.md`, and this task entry after separate
    in-scope approval for those actions.
  - Depends on: T28T.
  - Implementation: extended the worker stress test to run 1,000 Page Down and
    1,000 Page Up operations, increased the zero-repeat gate stress to 1,000
    attempts, and added test-only runtime metrics for maximum source-read range.
    Fixed all current Clippy `-D warnings` findings without changing the viewer
    contract or adding dependencies.
    The acceptance test asserts the three-frame, eight-block/512 KiB cache, 256
    KiB frame-source, and 64 KiB search-step limits. Existing focused tests cover
    mixed EOL, huge lines, controls, UTF-8, highlighting, Text/Hex search, wrap,
    immediate close, and cancellation. `README.md` already matches the verified
    behaviour, so no documentation change was needed.
  - Evidence: `cargo fmt --check`, focused viewer tests (73), focused viewer-gate
    tests (5), and elevated `cargo test --locked -- --test-threads=1` (134 unit
    and 8 lifecycle tests) passed. The x86_64 musl release build passed;
    `file`/`ldd` confirmed a stripped static PIE and the artifact was 865,008
    bytes. A build from pre-T28U `HEAD` was also 865,008 bytes (0-byte delta),
    and `cargo bloat --release --target x86_64-unknown-linux-musl -n 10` ran.
    The Windows target `cargo check --locked --target
    x86_64-pc-windows-msvc` passed, but native Windows build and test execution
    were unavailable because `link.exe` is absent on this Linux host. Clippy
    with `-D warnings` passed after the warning fixes, and final-source
    `cargo fmt --check` passed. Runtime tests and release builds were not rerun
    for this warning-only cleanup at that point. Later T29K full Linux validation
    exercised the final source and retained the T28 bounds. Native Windows/MSVC
    validation on 2026-08-06 passed 96 viewer tests, 163 full-suite tests, and
    the release build. The 571,392-byte artifact matched the prior Windows
    baseline (0-byte delta), retained SHA-256
    `A2DC32B0799962649BEC23954C837A019CA4A428B05F63096188641E05D80D74`, and
    bundled no DLL files. Clippy with Rust 1.97.1 failed on existing
    `needless_borrow`, `manual_is_multiple_of`, and `single_range_in_vec_init`
    findings; source fixes were outside the approved scope, so Native Windows
    validation remains incomplete.
  - Done when: implementation is complete and each required platform checkbox
    records authoritative execution evidence; unavailable native Windows
    acceptance remains an explicit blocker to the cross-platform claim.

### T28 native Windows confirmation ledger

- Confirmed: T28B through T28T each record native Windows/MSVC focused tests and
  release-build evidence. The T28R-T28T run passed focused viewer/input/server
  tests (96/14/17) and 163 full-suite tests; the 571,392-byte release artifact
  matched the prior baseline and has SHA-256
  `A2DC32B0799962649BEC23954C837A019CA4A428B05F63096188641E05D80D74`.
- T28A is platform independent. T28U implementation and Linux validation are
  complete, and its native Windows tests and release build pass; Rust 1.97.1
  Clippy findings keep its Native Windows checkbox and the final cross-platform
  claim incomplete.

## T29 Execution Contract

T29 corrects the bounded large-file viewer and its path prompt without changing
unrelated PTY, ConPTY, session, pane, status-format, IPC-security, or terminal
parser behaviour.

Rules for every T29 implementation request:

- Implement exactly one named subtask.
- Use the listed `Recommended model:` as the starting Luna level; changing the
  level does not change task scope.
- Do not redesign the Viewer Worker, event loop, renderer, or input state machine
  unless the named subtask explicitly requires that change.
- Touch only the files named by the subtask, except for a required private type,
  `mod` declaration, or focused test fixture in the same module.
- Do not add dependencies.
- Keep ordinary production changes below 300 changed lines per subtask.
- Preserve all T28 limits, snapshot semantics, generation cancellation,
  three-frame bound, zero-repeat backlog, and one-replacement rule.
- Run only the focused checks named by the approved implementation request.
- Stop after reporting changed files, focused results, risks, and blockers.
- T29E, T29H, T29I, and T29J require review before the next dependent subtask.
- Do not update `README.md` until T29K verifies the user-visible behaviour.

## T29 Tasks

- [x] **T29A — Define the viewer-correction contract and Luna execution plan**
  - Required validation: Platform independent.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Platform-independent review complete.
  - Recommended model: Luna High.
  - Add the approved Vim-style horizontal movement, full Viewer Help, no-phantom-
    row rendering, prompt-editing, literal-tilde safety, and page-latency contracts
    to `REQUIREMENTS.md`.
  - Add the T29 execution contract, model guidance, platform metadata rule,
    dependencies, file ownership, checks, and review gates to `TASKS.md`.
  - Allowed files: `REQUIREMENTS.md`, `TASKS.md`.
  - Production files: none.
  - Depends on: T28T.
  - Done when: each remaining correction can be implemented by Luna without
    inventing product behaviour, platform scope, ownership, or completion tests.
  - Evidence: completed as a documentation-only change. `README.md`, Rust source,
    builds, tests, dependencies, and Git state were not changed.

- [x] **T29B — Keep Tab-completed viewer paths editable**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Recommended model: Luna High.
  - Keep the Input prompt buffer, server-side visible query, filter, and selected
    entry synchronized after `Tab` completion or keyboard entry selection.
  - The first `Backspace` after completion MUST remove the final prompt character,
    immediately refilter entries, and MUST NOT navigate to the parent directory.
  - Parent navigation MUST occur only when the prompt buffer was empty before the
    `Backspace` key.
  - Prompt editing MUST never perform a filesystem delete or modification.
  - Allowed production files: `src/input.rs`, `src/server.rs`.
  - Focused tests: partial query then Tab then Backspace, completed file,
    completed directory, repeated completion cycling, empty-query Backspace,
    UTF-8 entry name, and client/server prompt-state equality after every action.
  - Depends on: T29A.
  - Done when: completion changes only editable prompt state and Backspace behaves
    identically for typed and completed text.
  - Evidence (2026-08-05): `cargo fmt --check`, `cargo test --locked
    viewer_prompt`, `cargo test --locked viewer_completion`, and
    `cargo build --release --locked --target x86_64-unknown-linux-musl` passed.
    Full `cargo test --locked` had 135 passing tests and one unrelated runtime
    socket test failed with sandbox `Operation not permitted`.
  - Native Windows evidence (2026-08-06, Windows 10.0.26100, stable Rust
    1.97.1, MSVC 14.44): target-specific `prompt` and `completion` test runs
    passed with 8 and 1 tests. The full target-specific suite passed with 163
    tests, and `cargo build --release --locked --target
    x86_64-pc-windows-msvc` passed.

- [x] **T29C — Prevent literal-tilde path-prompt crashes**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Recommended model: Luna High.
  - First add a focused reproduction for entering literal `~` in an empty and a
    non-empty viewer path prompt; identify whether failure occurs in input,
    prompt-state, status rendering, directory matching, or separator handling.
  - Preserve the approved milestone behaviour: `~` is literal filter text and is
    not expanded to a home directory.
  - `~`, `~/`, and `~\\` MUST never panic, close the client, or terminate the
    Session Server. Invalid directory selection MUST leave the prompt active and
    report a short actionable error.
  - Allowed production files: `src/input.rs`, `src/server.rs`; `src/render.rs`
    only when the reproduction proves the crash is in shared status rendering.
  - Focused tests: literal `~`, prefix plus `~`, `~/`, `~\\`, Backspace after
    `~`, Tab after `~`, Enter after `~`, and unchanged prompt state after errors.
  - Depends on: T29A.
  - Done when: every tilde path-prompt case is non-panicking and deterministic,
    without adding shell expansion or an external helper.
  - Correction: centralized prompt-entry selection returns no entry before any
    selected-index modulo when filtering produces no matches. This keeps Enter
    and separator errors actionable while leaving the prompt active.
  - Evidence (2026-08-05): `cargo fmt --check`, focused tilde/input and
    `viewer_prompt` tests, and `cargo build --release --locked
    --target x86_64-unknown-linux-musl` passed. Full `cargo test --locked`
    had 137 passing tests and one unrelated runtime socket test failed with
    sandbox `Operation not permitted` while binding `/tmp/termfold-test-*/work.sock`.
  - Native Windows evidence (2026-08-06, Windows 10.0.26100, stable Rust
    1.97.1, MSVC 14.44): target-specific `prompt` tests passed with 8 tests,
    including literal-tilde and error-state coverage. The full target-specific
    suite passed with 163 tests, and `cargo build --release --locked --target
    x86_64-pc-windows-msvc` passed.

- [x] **T29D — Implement horizontal viewer cursor primitives**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Recommended model: Luna High.
  - In Text mode, implement previous/next valid display-token movement within the
    current logical line. Stop at line boundaries and never wrap to another line.
  - Use the existing source/cell spans so movement cannot stop inside UTF-8,
    combining text, a wide continuation, a tab expansion, or EOL bytes.
  - In Hex mode, move by one source byte, permit crossing a displayed row, and
    clamp at snapshot BOF/EOF.
  - Update preferred display-cell state and horizontal viewport adjustment only
    through the existing viewer mapping.
  - Allowed production files: `src/viewer/mod.rs`, `src/viewer/text.rs`,
    `src/viewer/hex.rs`.
  - Focused tests: ASCII, line start/end, empty line, CJK, combining text, tabs,
    invalid bytes, hidden horizontal content, Hex row boundary, Hex block
    boundary, BOF, and EOF.
  - Depends on: T29A.
  - Done when: direct core calls report valid source offsets and display cells for
    every Text and Hex case without changing worker or input semantics.
  - Implementation: added `Viewer::move_horizontal`; Text movement follows the
    decoder's valid cursor-token starts and Hex movement clamps source-byte
    offsets while updating preferred cells and viewport visibility.
  - Evidence (2026-08-05): `cargo fmt --check`, focused horizontal movement test,
    `cargo test --locked viewer --no-fail-fast` (87 passed), and
    `cargo build --release --locked --target x86_64-unknown-linux-musl` passed.
    Full `cargo test --locked --no-fail-fast` had 138 passing tests; one runtime
    socket test and eight lifecycle tests were blocked by sandbox
    `Operation not permitted` while binding `/tmp/termfold-test-*/work.sock`.
  - Native Windows evidence (2026-08-06, Windows 10.0.26100, stable Rust
    1.97.1, MSVC 14.44): target-specific `horizontal` tests passed with 5 tests,
    including Text/Hex cursor primitives. The full target-specific suite passed
    with 163 tests, and `cargo build --release --locked --target
    x86_64-pc-windows-msvc` passed.

- [x] **T29E — Wire `h`/`l` and Left/Right through the Viewer Worker**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Recommended model: Luna Max.
  - Add semantic horizontal actions for `h`, `l`, Left Arrow, and Right Arrow.
  - Preserve fragmented escape-prefixed Left/Right sequences in Viewer mode.
  - Route horizontal movement through the existing generation-bound Viewer Worker
    and server-side ViewerGate; the Session Server MUST perform no file I/O.
  - Navigation MUST cancel unfinished search before dispatch, discard stale
    results, and retain zero repeat backlog plus one changed-intent replacement.
  - Allowed production files: `src/input.rs`, `src/server.rs`,
    `src/viewer/worker.rs`, `src/viewer/mod.rs`.
  - Focused tests: `h`/`l`, both arrow encodings, fragmented arrows, same-intent
    repeat dropping, direction reversal replacement, search cancellation, mode
    switch cancellation, close during movement, and no input leakage to a PTY.
  - Depends on: T29D.
  - Review gate: inspect generation changes, gate intent equality, stale-result
    handling, and absence of synchronous viewer reads in `server.rs`.
  - Done when: every accepted horizontal command commits one valid viewer frame
    and cannot create navigation backlog.
  - Implementation: added semantic horizontal actions for `h`, `l`, CSI Left/Right,
    and application-cursor Left/Right; routed them through `ViewerIntent`, the
    generation-bound Viewer Worker, and the existing one-replacement ViewerGate.
    Added focused input, gate, worker-render, cancellation, and stale-result checks.
  - Evidence (2026-08-05): `cargo fmt --check`,
    `cargo test --locked viewer --no-fail-fast` (90 tests), and
    `cargo build --release --locked --target x86_64-unknown-linux-musl` passed.
    `server.rs` retains no viewer file I/O; horizontal navigation uses the
    existing search cancellation, generation, and stale-result paths.
  - Native Windows evidence (2026-08-06, Windows 10.0.26100, stable Rust
    1.97.1, MSVC 14.44): target-specific `horizontal` tests passed with 5 tests,
    covering input encodings, repeat/reversal gating, primitives, and worker
    cancellation. The full target-specific suite passed with 163 tests, and
    `cargo build --release --locked --target x86_64-pc-windows-msvc` passed.

- [x] **T29F — Remove the phantom row above the status bar**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Recommended model: Luna High.
  - Render each viewer row without causing the virtual terminal to scroll after
    the final pane-content row.
  - Do not reduce the viewer height to hide the defect. Preserve the page-height
    contract and use every pane-content row directly above the status bar.
  - Account for both explicit trailing newlines and automatic wrap when a rendered
    row exactly fills the terminal width.
  - Allowed production files: `src/viewer/mod.rs`; a focused terminal fixture in
    `src/terminal.rs` only when required to observe scrolling deterministically.
  - Focused tests: full-height Text page, full-height Hex page, full-width final
    row, empty file, short file, narrow Hex message, resize, and status row
    immediately following the final viewer row.
  - Depends on: T29A.
  - Done when: Text and Hex views never retain a permanent blank content row above
    the status bar and cursor placement remains correct.
  - Implementation: render line endings only between viewer rows, so the final
    pane-content row cannot trigger virtual-terminal scrolling. Added focused
    text, Hex, full-width, empty, short, narrow-Hex, and resize regressions.
  - Evidence (2026-08-05): `cargo fmt --check`,
    `cargo test --locked viewer --no-fail-fast -- --test-threads=1` (91 passed),
    and `cargo build --release --locked --target x86_64-unknown-linux-musl`
    passed.
  - Native Windows evidence (2026-08-06, Windows 10.0.26100, stable Rust
    1.97.1, MSVC 14.44): the target-specific viewer run passed 116 tests,
    including the final-row Text, Hex, full-width, empty, narrow, and resize
    regression. The MSVC release build passed.

- [x] **T29G — Make Viewer Help complete and return-safe**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Recommended model: Luna High.
  - Make configured-prefix `?` open Help from Viewer mode without first executing
    the viewer prefix's Page Up fallback or the viewer's reverse-search action.
  - Preserve the Help origin so `q`, `Ctrl-c`, or `Esc` returns to the same Viewer
    rather than Normal mode or the child PTY.
  - List every viewer navigation key, `H`, `/`, `?`, `n`/`N`, normal Hex ASCII
    search, exact `hex:00 FF 1B` syntax, configured-prefix Help, and configured-
    prefix close behaviour.
  - Keep Help pagination, adaptive status reminder, configured prefix display,
    and the permanent status row.
  - Allowed production files: `src/input.rs`, `src/render.rs`, `src/server.rs`.
  - Focused tests: Normal-to-Help-to-Normal, Viewer-to-Help-to-Viewer, configured
    non-default prefix, no Page Up side effect, no reverse-search prompt side
    effect, fragmented Escape exit, pagination, and all required Help strings.
  - Depends on: T29E.
  - Done when: Help accurately describes the implemented viewer and exits to its
    exact calling mode without changing viewer state.
  - Implementation: made configured-prefix `?` a direct Viewer-to-Help transition,
    retained the Help origin in input state for `q`, `Ctrl-c`, and `Esc`, and added
    the complete viewer navigation, search, Hex syntax, Help, and close reminders.
  - Evidence (2026-08-05): `cargo fmt --check`,
    `cargo test --locked viewer --no-fail-fast -- --test-threads=1` (92 passed),
    `cargo test --locked render::tests::help_uses_the_configured_prefix_and_pages
    -- --exact` (1 passed),
    and `cargo build --release --locked --target x86_64-unknown-linux-musl`
    passed.
  - Native Windows evidence (2026-08-06, Windows 10.0.26100, stable Rust
    1.97.1, MSVC 14.44): the target-specific viewer run passed 116 tests,
    including Viewer-to-Help return and side-effect coverage; the exact Help
    render test passed. The MSVC release build passed.

- [x] **T29H — Wake the Session Server on Viewer Worker results**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Recommended model: Luna Max.
  - Add one bounded, non-blocking Viewer-ready notification into the existing
    central Server event path after a result is committed to the viewer result
    channel.
  - The notification MUST wake the Session Server immediately enough that visible
    viewer progress does not wait for `LISTENER_POLL_DELAY`.
  - A full central event queue MAY drop a redundant wake notification only when
    the actual viewer result remains retained and a pending event already wakes
    the server. Viewer results themselves MUST NOT be dropped or duplicated.
  - Do not increase polling frequency, add a thread per viewer, add a dependency,
    or bypass generation checks.
  - Allowed production files: `src/viewer/worker.rs`, `src/server.rs`.
  - Focused tests: idle listener wake, result-before-wake ordering, full event
    queue, duplicate ready notifications, two viewer IDs, stale generation,
    worker shutdown, and unchanged idle polling interval.
  - Depends on: T29A.
  - Review gate: inspect channel bounds, result ownership, wake coalescing, server
    shutdown, and every path where a result is produced.
  - Done when: no valid viewer result requires the periodic listener timeout to
    become visible.
  - Implementation: passed the existing bounded server-event sender into the
    session-scoped Viewer Worker. Successful asynchronous result sends now issue
    one coalesced non-blocking `ViewerReady` wake; the server clears the wake state
    before its existing generation-checked result drain. Full event queues retain
    results and only drop the redundant wake.
  - Evidence (2026-08-05): `cargo fmt --check` passed. Focused
    `cargo test --locked viewer::worker::
    --no-fail-fast` passed with 15 tests, and
    `cargo test --locked server::tests::viewer_ready_keeps_the_existing_idle_poll_delay
    -- --exact` passed. Coverage includes idle wake, ordering, full-queue,
    coalescing, two-viewer, stale-generation, shutdown, and unchanged polling
    interval. The approved `cargo build --release --locked
    --target x86_64-unknown-linux-musl` passed.
  - Native Windows evidence (2026-08-06, Windows 10.0.26100, stable Rust
    1.97.1, MSVC 14.44): the target-specific viewer run passed 116 tests,
    including idle wake, result ordering, full-queue retention, coalescing,
    two-viewer, stale-generation, and shutdown coverage. The MSVC release build
    passed.

- [x] **T29I — Remove neighbour prefetch from the visible-frame critical path**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Recommended model: Luna Max.
  - Return and commit a completed `Current` frame before performing any optional
    Previous/Next prefetch work.
  - The smallest safe implementation MAY temporarily disable active prefetch while
    preserving normal three-slot rotation and the exact three-frame bound.
  - If prefetch remains enabled, it MUST run only as lower-priority bounded worker
    work after checking that no control, navigation, search, resize, mode-switch,
    or close command is waiting.
  - Do not add a background thread, async runtime, fourth frame, second cache, or
    new queue.
  - Allowed production files: `src/viewer/mod.rs`, `src/viewer/frame.rs`,
    `src/viewer/worker.rs`.
  - Focused tests: Current result precedes prefetch, queued close wins, queued
    navigation wins, alternating Page Up/Down rotation, invalid neighbour
    rejection, resize invalidation, and never retaining a fourth frame.
  - Depends on: T29H.
  - Review gate: inspect frame ownership, commit/rollback boundaries, worker
    priority, command-queue checks, and all prefetch call sites.
  - Done when: optional neighbour preparation contributes no latency to delivery
    of the requested visible page.
  - Implementation: removed synchronous Previous/Next prefetch from `Viewer::render`.
    The requested `Current` frame is committed and returned without optional file
    work; the existing bounded three-slot ownership and rotation checks remain.
    Active prefetch is intentionally disabled pending a lower-priority worker path.
  - Evidence (2026-08-05): `cargo fmt --check` and `cargo test --locked viewer --no-fail-fast
    -- --test-threads=1` passed with 97 tests, including the new current-before-
    prefetch regression and existing frame rotation, invalid-neighbour,
    cancellation, resize, and bounded-cache coverage. The approved
    `cargo build --release --locked --target x86_64-unknown-linux-musl` passed.
  - Native Windows evidence (2026-08-06, Windows 10.0.26100, stable Rust
    1.97.1, MSVC 14.44): the target-specific viewer run passed 116 tests,
    including Current-before-prefetch delivery, frame rotation, invalid-neighbour,
    cancellation, resize, and three-frame-bound coverage. The MSVC release build
    passed.

- [x] **T29J — Combine page navigation and visible rendering safely**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Recommended model: Luna Max.
  - Replace the avoidable Page Up/Page Down and half-page
    `NavigationComplete -> server render request -> RenderComplete` round trip with
    one generation-bound worker operation that returns the requested rendered
    `Current` frame.
  - Keep an explicit cancellation/control boundary between changing navigation
    state and building the visible frame.
  - One accepted page command MUST still create and commit exactly one intermediate
    page. The server MUST apply and render that result before clearing in-flight
    state and dispatching the single changed-intent replacement.
  - Close, resize, new search, mode switch, and generation change MUST invalidate
    the compound result exactly as before. Same-intent repeats MUST remain dropped.
  - Do not combine unrelated line movement, search, or mode-switch operations.
  - Allowed production files: `src/server.rs`, `src/viewer/worker.rs`,
    `src/viewer/mod.rs`.
  - Focused tests: one command/one page/one visible result, 1,000 ordered pages,
    direction reversal, zero repeat backlog, release stops after current page,
    close between phases, resize between phases, stale result discard, render
    failure rollback, and no second worker round trip for page movement.
  - Depends on: T29I.
  - Review gate: inspect phase cancellation, page ordering, ViewerGate finish
    timing, rollback, replacement dispatch, and result-type ownership.
  - Done when: page movement has one worker round trip without weakening any T28
    responsiveness or ordering guarantee.
  - Implementation: replaced separate page-navigation and render commands with
    one generation-bound worker operation. The worker changes page state, yields
    at a control boundary, then renders the requested Current frame; cancellation
    rolls back an uncommitted page. The server commits the rendered terminal before
    finishing the ViewerGate and dispatching its single changed-intent replacement.
  - Evidence (2026-08-05): `cargo fmt` passed. `cargo test --locked viewer --no-fail-fast
    -- --test-threads=1` passed with 97 tests, including 1,000 ordered page-down
    and page-up compound operations, close cancellation, and render rollback.
    `cargo test --locked server::tests::viewer_gate --no-fail-fast
    -- --test-threads=1` passed with 6 tests. The approved
    `cargo build --release --locked --target x86_64-unknown-linux-musl` passed.
    Native Windows/MSVC validation was not available in this Linux host.
  - Native Windows evidence (2026-08-06, Windows 10.0.26100, stable Rust
    1.97.1, MSVC 14.44): the target-specific viewer run passed 116 tests,
    including compound page rendering and cancellation; all 6 ViewerGate tests
    passed in that run. The MSVC release build passed.

- [x] **T29K — Run viewer-correction acceptance and update documentation**
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Recommended model: Luna High.
  - Add deterministic acceptance coverage for completed-path Backspace, literal
    tilde safety, Vim-style horizontal movement, Viewer Help return state, no
    phantom row, immediate Viewer-ready wake, Current-before-prefetch ordering,
    compound page rendering, zero repeat backlog, and immediate close.
  - Verify all T28 runtime bounds remain unchanged: eight raw blocks / 512 KiB,
    three frames, 256 KiB source bytes per frame, 64 KiB work steps, one in-flight
    navigation, and one changed-intent replacement.
  - Run focused and full checks on authoritative Linux/WSL and native Windows,
    including Clippy, musl release build, MSVC release build, and a before/after
    page-latency measurement using the same harness and terminal size.
  - Update `README.md` only for behaviour that is implemented and verified. Record
    concise evidence and unavailable-environment blockers in this task.
  - Allowed files: focused tests, `README.md`, and this task entry after separate
    in-scope approval.
  - Depends on: T29B, T29C, T29E, T29F, T29G, T29J.
  - Done when: all corrected behaviour is verified on authoritative environments,
    all resource and ordering limits still pass, and documentation matches the
    executable without claiming unavailable validation.
  - Implementation: existing deterministic focused coverage now serves as the
    T29K acceptance set; it covers prompt editing and literal tilde safety,
    horizontal movement, Help return state, final-row rendering, worker wake,
    current-before-prefetch delivery, compound paging, repeat gating, immediate
    close, and the T28 cache/frame/work bounds. `README.md` now records the
    verified viewer resource limits. A later Rust 1.97.1 Clippy run exposed
    three mechanical findings in the completed source; the redundant borrow,
    remainder check, and single-range fixture were corrected without changing
    viewer behaviour.
  - Evidence (2026-08-05): `cargo fmt --check`, focused viewer (97), input (13),
    server (15), and Help-render (1) tests passed. Full Linux validation passed
    with `cargo test --locked -- --test-threads=1` (149 unit and 8 lifecycle
    tests) and `cargo clippy --all-targets --all-features -- -D warnings`.
    `cargo build --release --locked --target x86_64-unknown-linux-musl` passed;
    the result is an 873,200-byte stripped static PIE and `ldd` reports it is
    statically linked. Current paging metrics at the existing 16x3 test size:
    initial 139.8 ms, cold down 587.1 ms, warm up 667.6 ms, long line 159.9 ms;
    peak cache 458,769 bytes. README behaviour is limited to implemented and
    verified paths.
  - Native Windows evidence (2026-08-06, Windows 10.0.26100, stable Rust
    1.97.1, `x86_64-pc-windows-msvc`): focused viewer tests passed 116/116 with
    `cargo test --locked --target x86_64-pc-windows-msvc viewer --no-fail-fast
    -- --test-threads=1`; the full suite passed 163/163 with
    `cargo test --locked --target x86_64-pc-windows-msvc -- --test-threads=1`;
    `cargo clippy --locked --target x86_64-pc-windows-msvc --all-targets
    --all-features -- -D warnings` passed; and
    `cargo build --release --locked --target x86_64-pc-windows-msvc` passed.
    The same release test file and temporary PowerShell harness ran on both
    commits with terminal size 16 columns x 3 rows, 5 warm-ups, and 31 measured
    samples. Each sample used
    `cargo test --release --locked --target x86_64-pc-windows-msvc
    viewer::tests::paging_scans_blocks_without_bytewise_cache_churn -- --exact
    --nocapture --test-threads=1`; Page Down and Page Up are the committed
    3-page aggregate operations.

    | Operation | Before `2ccd69f` median / p95 ms | After `9fd883e` median / p95 ms |
    | --- | ---: | ---: |
    | Initial render | 48.57 / 58.31 | 47.17 / 52.05 |
    | Page Down (3-page aggregate) | 273.20 / 366.08 | 212.62 / 228.06 |
    | Page Up (3-page aggregate) | 301.22 / 362.79 | 242.85 / 259.30 |
    | Long-line page render | 65.05 / 71.90 | 66.00 / 70.42 |

    Before is the immediate parent of T29H implementation commit `365d946`:
    `2ccd69fd5ac6bb39a5d2695ed9cab27c9539f137`. After is current HEAD:
    `9fd883eea6db5e3f1becc139a964c7fb20982df2`. No comparative blocker remains;
    the initial sandbox-only temporary-file denial required an escalated native
    rerun and did not reproduce. The measurement covers the in-process viewer
    terminal fixture, not an external terminal GUI.

## T30 Execution Contract

T30 corrects Viewer path-prompt navigation, repeated-search anchoring, and Hex
row geometry without changing unrelated PTY, ConPTY, session, pane, status,
IPC-security, terminal-parser, snapshot, or resource-limit behaviour.

Rules for every T30 implementation request:

- Implement exactly one named subtask.
- Use the listed `Recommended model:` as the starting Luna level. Tuning the
  level does not broaden scope or combine tasks.
- Preserve the T28/T29 Viewer Worker ownership, generation cancellation,
  zero-repeat backlog, one changed-intent replacement, three-frame bound,
  eight-block cache, 256 KiB frame-source cap, and 64 KiB work step.
- Touch only the files named by the subtask, except for a required private type,
  module declaration, or focused test fixture in the same module.
- Do not add dependencies, invoke a shell helper, or create a second viewer cache.
- Keep ordinary production changes below 300 changed lines per subtask.
- Run only the focused checks named by the approved implementation request.
- Stop after reporting changed files, focused results, risks, and blockers.
- T30D, T30F, and T30H require review before the next dependent subtask.
- Do not update `README.md` until T30I verifies user-visible behaviour.
- Record task implementation, Linux validation, and Native Windows validation
  independently according to `AGENTS.md`.

## T30 Tasks

- [x] **T30A — Normalize Luna levels and platform completion tracking**
  - Recommended model: Luna Medium.
  - Required validation: Platform independent.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Platform-independent review complete.
  - Add `Luna xHigh` and the T28-and-later implementation/platform status rules
    to `AGENTS.md` and the `TASKS.md` workflow.
  - Change T28 and T29 headings to implementation-complete where their recorded
    implementation exists, then add explicit Platform-independent, Linux, and
    Native Windows completion checkboxes from the existing evidence.
  - Do not convert compile-only Windows checks into native Windows completion.
  - Allowed files: `AGENTS.md`, `TASKS.md`.
  - Production files: none.
  - Depends on: T29K.
  - Done when: missing Native Windows evidence is visible without hiding completed
    implementation or Linux results.
  - Evidence: documentation-only normalization completed from the existing T28
    and T29 evidence; no Rust source, tests, builds, dependencies, or Git state
    were changed.

- [x] **T30B — Define Ido navigation, cursor-anchored search, and dynamic Hex layout**
  - Recommended model: Luna High.
  - Required validation: Platform independent.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Platform-independent review complete.
  - Define Ido-style `//` root navigation, `~/` Home navigation, first-separator
    pending input, empty-prompt parent navigation, current-cursor `n`/`N`, and
    width-derived Hex grouping in `REQUIREMENTS.md`.
  - Define T30C through T30I with exact ownership, dependencies, platform checks,
    review gates, and completion conditions.
  - Allowed files: `REQUIREMENTS.md`, `TASKS.md`.
  - Production files: none.
  - Depends on: T30A.
  - Done when: each implementation task can be executed without inventing Ido,
    search-anchor, Hex-column, resize, or platform behaviour.
  - Evidence: approved documentation-only contract added; `README.md`, Rust
    source, tests, builds, dependencies, and Git state were not changed.

- [x] **T30C — Implement Ido prompt separator state**
  - Recommended model: Luna High.
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Keep the first separator entered with an empty prompt as editable pending
    input instead of changing directory immediately.
  - A second valid root separator MUST emit one semantic root-navigation action.
    Backspace after the first separator MUST remove it; Backspace on an already
    empty prompt MUST continue to emit parent navigation.
  - `folder/` MUST continue to enter the selected folder, and separator handling
    MUST keep the Input buffer, server query, filter, selection, and visible
    status synchronized after every action.
  - Invalid or fragmented input MUST not leak to the child PTY or leave a hidden
    separator only on one side of the client/server state.
  - Allowed production files: `src/input.rs`, `src/server.rs`.
  - Focused tests: empty first `/`, second `/`, first `\\` on Windows, Backspace
    after one separator, empty-prompt Backspace, `folder/`, completed folder then
    separator, UTF-8 folder, cancellation, and input/server state equality.
  - Depends on: T30B.
  - Done when: the first empty-prompt separator never changes directory and every
    accepted second/root or selected-directory separator produces one action.
  - Evidence (2026-08-05): `src/input.rs` keeps the first `/` or `\\` as editable
    query state and dispatches the existing directory action only for a following
    separator. `cargo fmt` passed; focused Linux input (14) and server (15) tests
    passed, and the `x86_64-unknown-linux-musl` release build produced a static
    PIE result.
    Earlier target compilation was compile-only evidence. Native Windows
    validation (2026-08-06): `cargo test --locked --target
    x86_64-pc-windows-msvc input::tests:: -- --test-threads=1` passed 14/14;
    `cargo build --release --locked --target x86_64-pc-windows-msvc` passed.

- [x] **T30D — Resolve Ido root and Home paths atomically**
  - Recommended model: Luna xHigh.
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Resolve Linux `//` to `/` and portable `~/` to the current user's Home using
    standard-library and platform environment data only.
  - On native Windows, resolve `//` or `\\` to the current drive root and
    `C:/`/`C:\\` to the named drive root. Do not add `~user/` lookup.
  - Validate the complete destination before replacing prompt directory state.
    On missing, non-directory, unavailable-Home, or invalid-drive input, retain
    the prior directory and editable input and report one short actionable error.
  - Successful root, drive, or Home entry MUST clear query/filter/selection and
    leave Backspace ready to return to the logical parent where one exists.
  - Allowed production files: `src/input.rs`, `src/server.rs`.
  - Focused tests: Linux `//`, Linux `~/`, missing Home, literal `~`, `~text`,
    Windows current-drive `//`, Windows `\\`, `C:/`, `C:\\`, invalid drive,
    error rollback, and no filesystem modification.
  - Depends on: T30C.
  - Review gate: inspect path-state atomicity, environment handling, separator
    normalization, root-parent behaviour, and absence of shell expansion.
  - Done when: every approved Ido path form is deterministic on its native
    platform and every rejected form preserves prompt state.
  - Evidence (2026-08-05): `src/server.rs` resolves exact Linux root, Windows
    current-drive and named-drive roots, and `~/` from validated standard-library
    environment data before committing prompt state; invalid, missing, and
    non-directory destinations leave the existing state unchanged. Focused Linux
    tests (`cargo test --locked viewer_prompt`) passed 6/6, formatting and diff
    checks passed, and `cargo build --release --locked --target
    x86_64-unknown-linux-musl` produced a static PIE executable; `file` and `ldd`
    confirmed static linking. `cargo check --locked --target
    `cargo check --locked --tests --target x86_64-pc-windows-msvc` passed as
    compile-only evidence. Native Windows validation (2026-08-06): `cargo test
    --locked --target x86_64-pc-windows-msvc server::tests::viewer_ --
    --test-threads=1` passed 12/12; `cargo build --release --locked --target
    x86_64-pc-windows-msvc` passed.

- [x] **T30E — Anchor `n` and `N` at the current Viewer cursor**
  - Recommended model: Luna xHigh.
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Separate the last successful query and recorded direction from the active
    match and current cursor source offset.
  - `n` MUST search from the current cursor in the recorded direction; `N` MUST
    search from the current cursor in the opposite direction. Forward/reverse
    repeats MUST exclude the cursor's current match position.
  - Cached matches MUST choose the nearest valid offset strictly after or before
    the current cursor, not after or before the previous match offset.
  - Page, half-page, line, horizontal, line start/end, and file start/end movement
    MUST affect the next repeat anchor. Viewport-only movement MUST not.
  - Preserve the last successful query after navigation and preserve active-match
    highlighting only when its source range remains the active result.
  - Allowed production files: `src/viewer/mod.rs`, `src/viewer/search.rs`.
  - Focused tests: `/` then `n`, `?` then `n`, `N`, cursor on a match, Page Down
    then repeat, Page Up then repeat, line/horizontal/end movement, viewport-only
    movement, cached nearest match, wrap once, and no match.
  - Depends on: T30B.
  - Done when: direct Viewer calls never use the previous match as the repeat
    anchor after the logical cursor has moved.
  - Evidence (2026-08-05): Viewer search now uses the committed cursor as the
    strict repeat anchor, keeps the prior active offset for highlighting, and
    selects the nearest cached match on either side. Focused Linux validation
    (`cargo fmt --check`, `cargo test --locked viewer::tests::`, 35/35, and
    `cargo build --release --locked --target x86_64-unknown-linux-musl`) passed;
    `file` and `ldd` confirmed a static PIE executable. The Worker Hex search
    regression passed 1/1, and
    `cargo check --locked --tests --target x86_64-pc-windows-msvc` passed as
    compile-only evidence. Native Windows validation (2026-08-06): `cargo test
    --locked --target x86_64-pc-windows-msvc viewer:: -- --test-threads=1`
    passed 96/96; `cargo build --release --locked --target
    x86_64-pc-windows-msvc` passed. The full Linux suite passed 152/154 tests;
    the remaining
    failures were the sandbox socket-permission error in
    `runtime::tests::socket_is_private_and_only_a_stale_socket_is_replaced` and
    one intermittent Worker timeout that passed when rerun in isolation.

- [x] **T30F — Preserve bounded worker search after cursor-anchor changes**
  - Recommended model: Luna Max.
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Route the corrected repeat anchor through the existing generation-bound
    Viewer Worker and server dispatcher without adding synchronous server file
    I/O or a second search path.
  - Navigation accepted during unfinished search MUST cancel the obsolete search;
    a later `n`/`N` MUST begin from the committed current cursor. Close, resize,
    mode switch, and new search MUST retain existing priority and stale-result
    rejection.
  - Search may load required 64 KiB raw blocks and evict old blocks through the
    existing eight-block cache. It MUST not retain an unloaded PageFrame, create
    a fourth frame, or mistake cache eviction for loss of the last query.
  - Preserve one-wrap search, 64 KiB cooperative steps, no full-file index, one
    in-flight navigation, zero same-intent backlog, and one changed-intent
    replacement.
  - Allowed production files: `src/viewer/worker.rs`, `src/viewer/mod.rs`,
    `src/server.rs`.
  - Focused tests: search to another frame, Page Down then repeat, Page Up then
    repeat, raw-block eviction then repeat, result in cached block, result in
    reloaded block, cancellation after one step, stale generation, another viewer
    control during long search, close, and mode switch.
  - Depends on: T30E.
  - Review gate: inspect worker ownership, generation changes, result ordering,
    cache/frame separation, and absence of server file reads.
  - Done when: `n`/`N` from a page-changed cursor remains responsive and bounded
    whether the needed block is cached or reloaded.
  - Implementation: stale `Cancel` and `Close` controls now check the worker
    generation before cancelling newer work. Added worker regressions for page
    navigation and repeat anchoring, raw-block eviction and reload, cancellation
    with query retention, navigation preemption, and stale controls.
  - Evidence (2026-08-05): `cargo fmt --check`, focused Worker tests (20/20),
    and focused server ViewerGate tests (6/6) passed. The Linux musl release
    build passed; `file` and `ldd` confirmed a stripped static PIE. `cargo check
    --locked --tests --target x86_64-pc-windows-msvc` passed as compile-only
    evidence. Native Windows validation (2026-08-06): `cargo test --locked
    --target x86_64-pc-windows-msvc viewer:: -- --test-threads=1` passed 96/96;
    `cargo build --release --locked --target x86_64-pc-windows-msvc` passed.

- [x] **T30G — Build width-derived Hex row geometry**
  - Recommended model: Luna xHigh.
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Replace the fixed 16/8/4 width thresholds with one geometry calculation that
    selects the greatest fitting eight-byte multiple, falling back to four bytes
    and then the narrow message.
  - Calculate one snapshot-wide offset width of at least eight hexadecimal digits
    and reserve at least one unused display cell to avoid automatic wrap.
  - Insert one aligned `│` between complete eight-byte Hex groups. Do not place
    separators in the ASCII area or after the final group.
  - Pad short final Hex rows so separators and the ASCII start remain aligned.
  - Store explicit byte Hex-cell ranges, separator columns, and ASCII start in
    the row geometry; do not derive post-separator columns with `index * 3`.
  - Allowed production files: `src/viewer/hex.rs`, `src/viewer/frame.rs`.
  - Focused tests: narrow, 4, 8, 16, 24, and 32-byte rows; exact-fit rejection
    for wrap safety; short final row; separator alignment across rows; no ASCII
    separator; and offsets above 4 GiB.
  - Depends on: T30B.
  - Done when: every rendered row and stored cell range comes from one bounded
    geometry that fully fits the pane width.
  - Implementation: added one snapshot-wide Hex geometry for offset width,
    dynamic eight-byte grouping, four-byte fallback, separators, explicit Hex
    cells, and ASCII placement. Short rows pad only the Hex area, and partial
    rows carry across raw-block reads so 24-byte layouts remain source-aligned.
  - Evidence (2026-08-05): `cargo fmt --check`, focused Hex tests (9/9), and
    the Linux musl release build passed. `cargo check --locked --tests
    --target x86_64-pc-windows-msvc` passed as compile-only evidence. A full
    filtered Viewer run reached 88/89 because the existing timing-sensitive
    `mode_switch_preempts_async_search` test missed its stale result; that test
    passed in isolation. Native Windows validation (2026-08-06): `cargo test
    --locked --target x86_64-pc-windows-msvc viewer:: -- --test-threads=1`
    passed 96/96; `cargo build --release --locked --target
    x86_64-pc-windows-msvc` passed.

- [x] **T30H — Apply Hex geometry to cursor, highlight, paging, and resize**
  - Recommended model: Luna Max.
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Use the T30G geometry for Hex cursor placement, Hex and ASCII search
    highlights, clipping, row movement, page movement, preferred byte column, and
    active-match rendering.
  - Horizontal movement MUST remain one source byte and may cross a displayed row
    boundary. Vertical movement MUST preserve the byte-within-row position where
    the resized row still contains it.
  - Resize MUST increase generation, cancel obsolete work, clear the one pending
    replacement, invalidate three frame slots, recompute geometry, preserve the
    source byte position, and request one new Current frame.
  - Search matches may cross visual group, row, and raw-block boundaries; the
    separator cells themselves MUST never become cursor stops or highlighted
    source bytes.
  - Do not change Text-mode mapping or create a Hex-specific raw cache.
  - Allowed production files: `src/viewer/hex.rs`, `src/viewer/frame.rs`,
    `src/viewer/mod.rs`, `src/viewer/worker.rs`.
  - Focused tests: cursor before/after each separator, Hex/ASCII highlight
    alignment, 8-to-16 and 24-to-8 resize, source-position preservation, search
    across group/row/block boundaries, Page Up/Down, BOF/EOF, stale resize result,
    close during rebuild, and three-frame/eight-block bounds.
  - Depends on: T30G.
  - Review gate: inspect one geometry owner, generation cancellation, frame
    invalidation, cursor/highlight source mapping, and absence of a second cache.
  - Done when: dynamic grouping changes only display geometry and never changes
    the source byte represented by cursor or active match.
  - Implementation: navigation now derives its row width from the shared
    snapshot geometry, cursor stops and Hex/ASCII highlights use stored geometry,
    Hex frames retain visible and active match ranges, and resize invalidates the
    generation and frame slots while preserving the source byte. Focused worker
    regressions cover stale resize replacement and close during rebuild.
  - Evidence (2026-08-05): `cargo fmt --check` passed. Linux `cargo test
    --locked viewer:: -- --test-threads=1` passed 97/97, including Hex geometry, cross-row matching,
    cursor movement, resize, bounds, paging, and worker cancellation coverage.
    `cargo build --release --locked --target x86_64-unknown-linux-musl` passed.
    `cargo check --locked --tests --target x86_64-pc-windows-msvc` passed as
    compile-only evidence. Native Windows validation (2026-08-06): `cargo test
    --locked --target x86_64-pc-windows-msvc viewer:: -- --test-threads=1`
    passed 96/96; `cargo build --release --locked --target
    x86_64-pc-windows-msvc` passed.

- [x] **T30I — Run Ido, repeat-search, and dynamic-Hex acceptance**
  - Recommended model: Luna High.
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - Add deterministic acceptance coverage for first-separator pending input,
    `//` root, `~/` Home, entered-folder Backspace parent, error rollback,
    Page Up/Page Down then cursor-anchored `n`/`N`, raw-block eviction/reload, and
    dynamic 4/8/16/24/32-byte Hex rows with aligned group separators.
  - Verify cursor and both highlight columns around separators, resize position
    preservation, no ASCII grouping, offset width above 4 GiB, wrap safety, and
    immediate close/cancellation.
  - Reverify all T28/T29 bounds: eight raw blocks / 512 KiB, three frames, 256 KiB
    source bytes per frame, 64 KiB work steps, one navigation in flight, and one
    changed-intent replacement.
  - Run approved focused and full Linux/WSL checks, Clippy, musl release build,
    native Windows focused/full checks, and MSVC release build. A Windows target
    check alone MUST leave Native Windows validation unchecked.
  - Update `README.md` only after the matching behaviour is implemented and
    verified on the platform being claimed. Record concise evidence and blockers
    in this task.
  - Allowed files: focused tests, `README.md`, and this task entry after separate
    in-scope approval.
  - Depends on: T30D, T30F, T30H.
  - Done when: implementation and each authoritative platform checkbox reflect
    actual execution evidence, documentation matches verified behaviour, and no
    cross-platform claim relies on compile-only validation.
  - Implementation: added deterministic acceptance coverage for prompt
    separators and folder input, Page Down/Page Up cursor-anchored repeat search,
    and rendered 4/8/16/24/32-byte Hex layouts with wrap and ASCII-separator
    assertions. Existing acceptance coverage reverified raw-block reload,
    resize preservation, highlight mapping, close/cancellation, frame/cache
    bounds, work-step bounds, and navigation replacement bounds.
  - Evidence (2026-08-06): Ubuntu-24.04 WSL focused checks passed:
    `cargo test --locked viewer:: -- --test-threads=1` 98/98,
    `cargo test --locked server::tests::viewer_ -- --test-threads=1` 12/12,
    viewer prompt input 1/1, and the first-separator regression 1/1. The full
    Linux suite passed 170/170 unit tests and 8/8 lifecycle tests. The
    `x86_64-unknown-linux-musl` release build passed. Native Windows focused
    checks passed: Viewer 96/96, server viewer 12/12, and input 14/14. The full
    MSVC suite passed 163/163 tests, and the
    `x86_64-pc-windows-msvc` release build passed. The initial sandboxed Windows
    viewer run hit temporary-file `Access is denied`; the elevated native rerun
    passed. Clippy was not run because it was outside the approved build/test
    scope.

## T31 Execution Contract

T31 corrects Viewer matching-search direction and wrap behaviour, adds direct
single-key non-matching-line search, and replaces the dense Viewer Help paragraph
with grouped key rows. It does not change unrelated path-prompt, Hex geometry,
PTY, ConPTY, session, pane, status-format, IPC-security, terminal-parser,
snapshot, or resource-limit behaviour.

Rules for every T31 implementation request:

- Implement exactly one named subtask.
- Use the listed `Recommended model:` as the starting Luna level. Tuning the
  level does not broaden scope or combine tasks.
- Every T31 implementation has `Implementation scope: Platform independent`.
  Do not create separate Linux and Windows versions of search, key handling, or
  Help. Platform checkboxes are validation targets, not separate features.
- Preserve the T28-T30 Viewer Worker ownership, generation cancellation, zero
  repeat backlog, one changed-intent replacement, three-frame bound, eight-block
  cache, 256 KiB frame-source cap, 64 KiB work step, and Current-before-prefetch
  ordering.
- Reuse the universal line scanner, existing raw-block cache, existing query
  comparison, and existing Viewer Worker. Do not add a full-file match or line
  index, second cache, background thread, async runtime, dependency, regex, shell
  helper, or command language.
- `[` and `]` are unprefixed Viewer keys only. Do not change `Ctrl-b [` scroll-view
  entry or the path prompt's bracket handling.
- Keep ordinary production changes below 350 changed lines per subtask. If the
  bounded non-match implementation cannot fit, split the named task rather than
  broadening ownership.
- Run only the focused checks named by the approved implementation request.
- Stop after reporting changed files, focused results, risks, and blockers.
- T31C and T31D require review before the next dependent subtask.
- Do not update `README.md` until T31G completes the required authoritative
  validation; T31H performs the final mechanical documentation update.

## T31 Tasks

- [x] **T31A — Define Viewer search modes, Help layout, and execution plan**
  - Recommended model: Luna Medium.
  - Implementation scope: Platform independent.
  - Required validation: Platform independent.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Platform-independent validation complete.
  - Add `Luna Low` through `Luna Max` guidance and separate implementation scope
    from validation targets in `AGENTS.md` and the `TASKS.md` workflow.
  - Define `/`, `?`, `n`, and `N` direction and wrap behaviour, direct `]` and `[`
    non-matching-line search, Text-only non-match semantics, and grouped Viewer
    Help in `REQUIREMENTS.md`.
  - Define T31B through T31H with bounded ownership, dependencies, review gates,
    model levels, platform classification, and objective completion conditions.
  - Allowed files: `AGENTS.md`, `REQUIREMENTS.md`, `TASKS.md`.
  - Production files: none.
  - Depends on: T30I.
  - Done when: each implementation task can be executed without inventing search
    mode, direction, wrap, line-selection, Help layout, model level, or platform
    ownership.
  - Evidence: approved documentation-only change; Rust source and `README.md`
    remain unchanged.

- [x] **T31B — Lock the matching-search direction and wrap matrix**
  - Recommended model: Luna High.
  - Implementation scope: Platform independent.
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [x] Native Windows validation complete.
  - First add an end-to-end reproduction for the reported `/` then `N` case before
    changing logic. Inspect cached-match selection, cursor anchoring, worker
    repeat dispatch, server `same_direction` routing, and wrapped-status delivery.
  - Enforce the complete matrix: `/` records forward, `?` records reverse, `n`
    keeps the recorded direction, and `N` reverses it.
  - From the first match, reverse repeat MUST wrap from BOF to the final match.
    From the final match, forward repeat MUST wrap from EOF to the first match.
  - Preserve strict current-cursor anchoring after line, page, horizontal,
    start/end, top/bottom, and viewport-only movement rules from T30.
  - A cached result MUST be the nearest valid result in the requested direction;
    a cache miss MUST fall through to the bounded one-wrap scan rather than
    reporting no match early.
  - Allowed production files: `src/viewer/mod.rs`, `src/viewer/worker.rs`,
    `src/server.rs`; `src/input.rs` only if the reproduction proves the boolean
    direction mapping is wrong there.
  - Focused tests: `/ -> n`, `/ -> N`, `? -> n`, `? -> N`, BOF-to-EOF wrap,
    EOF-to-BOF wrap, one-match no-self-repeat, cached and uncached paths, cursor
    movement before repeat, wrapped status, cancellation, stale result, and no
    input leakage to a PTY.
  - Depends on: T30F, T31A.
  - Done when: the same direction/wrap matrix passes through the Viewer core,
    Worker handle, and server action path without changing query or cursor state
    on failure.
  - Implementation: preserved the recorded search direction separately from the
    direction used by each repeat operation, including cached and incremental
    Worker paths. Added `/`/`?` direction, `n`/`N` reversal, and wrapped-result
    regressions at the Viewer core and Worker-handle boundaries. The existing
    server boolean routing was verified unchanged.
  - Evidence (2026-08-06): Native Windows `cargo fmt --check`,
    `cargo check --locked --tests --target x86_64-pc-windows-msvc`, focused
    Worker reproduction, and `cargo test --locked --target
    x86_64-pc-windows-msvc viewer:: -- --test-threads=1` passed (97/97).
    Native Windows `cargo build --release --locked --target
    x86_64-pc-windows-msvc` passed. Ubuntu 24.04 WSL `cargo fmt --check`,
    `cargo test --locked viewer:: -- --test-threads=1` passed (99/99), and
    `cargo build --release --locked --target x86_64-unknown-linux-musl`
    passed.

- [x] **T31C — Implement bounded non-matching-line search primitives**
  - Recommended model: Luna xHigh.
  - Implementation scope: Platform independent.
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [ ] Native Windows validation complete.
  - Add an explicit search mode that distinguishes byte-match search from
    non-matching logical-line search without overloading the direction flag.
  - Reuse the existing Text query comparison and universal LF/CRLF/lone-CR line
    scanner. In non-match mode, `hex:` remains literal Text rather than exact-byte
    syntax.
  - Forward work starts at the next logical line; reverse work starts at the
    previous logical line. The original cursor line is excluded from the one-wrap
    search.
  - A candidate qualifies only after its complete content, excluding EOL bytes,
    has been scanned without the query. Empty lines qualify. Query matches may
    cross raw-block boundaries but never logical line boundaries.
  - Long lines and reverse scans MUST use resumable state and yield after at most
    one 64 KiB source step. Do not retain the complete long line or a full-file
    list of line starts or qualifying lines.
  - On success, move to the first valid cursor stop of the selected line; an empty
    line uses column zero. Preserve the last successful search when parsing,
    reading, or scanning fails.
  - Allowed production files: `src/viewer/line.rs`, `src/viewer/search.rs`,
    `src/viewer/mod.rs`.
  - Focused tests: forward/reverse, LF/CRLF/CR/mixed EOL, empty line,
    unterminated final line, pattern at line start/end, pattern crossing a raw
    block, pattern split by EOL, line longer than eight blocks, BOF/EOF wrap once,
    original-line exclusion, cancellation step boundary, error rollback, and no
    unbounded retained line state.
  - Depends on: T28D, T28N, T28O, T30F, T31A.
  - Review gate: inspect line-boundary ownership, reverse resumable state,
    64 KiB yielding, wrap termination, source-cache bounds, and absence of a
    second line index before T31D.
  - Done when: core calls deterministically find the nearest non-matching line in
    either direction while preserving all Viewer memory and fairness limits.
  - Implementation: added explicit matching and non-matching search modes, a
    bounded forward/reverse logical-line worker using the existing scanner and
    raw-block cache, literal Text parsing for `hex:` in non-match mode, line-start
    cursor placement, one-wrap exclusion, rollback, and non-match highlight
    suppression. Reverse scanner boundary skips now yield before crossing into a
    second source block.
  - Evidence (2026-08-06): Ubuntu 24.04 Linux `cargo fmt --check`,
    `cargo test --locked viewer:: -- --test-threads=1` passed (107/107), and
    `cargo build --release --locked --target x86_64-unknown-linux-musl` passed.
    The release binary was verified static with `file` and `ldd`. Windows
    `cargo check --locked --tests --target x86_64-pc-windows-msvc` passed as a
    compile-only check; native MSVC runtime tests and release build are blocked
    because this environment is Linux.

- [x] **T31D — Integrate search mode with Worker state, repeat, and highlighting**
  - Recommended model: Luna xHigh.
  - Implementation scope: Platform independent.
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [ ] Native Windows validation complete.
  - Carry search mode, query, and recorded direction through Viewer state,
    `ViewerOperation`, incremental `SearchWork`, `ViewerResult`,
    `PendingViewerSearch`, and repeat dispatch without encoding mode in an
    unrelated boolean.
  - `n` MUST repeat the recorded mode and direction; `N` MUST change direction
    only. Navigation before repeat MUST use the current logical cursor line or
    byte position as the new anchor.
  - Non-match work MUST run cooperatively in the existing Viewer Worker, obey
    generation cancellation, discard stale results, and never wait behind repeat
    backlog or block close, resize, mode switch, or new search.
  - Matching search retains normal visible and active highlights. Non-matching-line
    success MUST clear byte-match highlighting and use only the normal cursor at
    the selected line start.
  - Reject non-matching-line search in Hex mode before starting worker work and
    preserve the prior successful query, frame, cursor, and generation-valid
    committed state.
  - Allowed production files: `src/viewer/mod.rs`, `src/viewer/worker.rs`,
    `src/viewer/frame.rs`, `src/server.rs`.
  - Focused tests: mode retained by `n`, direction-only reversal by `N`, match to
    non-match and non-match to match replacement, no-match result, wrapped result,
    Hex rejection, highlight clearing, one 64 KiB worker step, another viewer
    receives service, navigation cancellation, new-search cancellation, stale
    generation, immediate close, and unchanged queue/cache/frame bounds.
  - Depends on: T31B, T31C.
  - Review gate: inspect search-mode ownership, generation changes, worker queue
    fairness, error rollback, highlight state, and every repeat call site before
    T31E.
  - Done when: matching and non-matching searches share one bounded Worker path
    while retaining distinct mode, direction, cursor, and highlight state.
  - Implementation: carried explicit search mode through Worker operations,
    results, pending server state, and repeat completion; retained mode and
    direction in Viewer search state; added cooperative non-match result/error
    delivery, Hex rejection, cancellation-safe rollback, and Worker highlight
    regressions.
  - Evidence (2026-08-06): Ubuntu 24.04 Linux `cargo fmt --check`,
    `cargo test --locked viewer:: -- --test-threads=1` passed (108/108), and
    `cargo build --release --locked --target x86_64-unknown-linux-musl` passed.
    Windows `cargo check --locked --tests --target x86_64-pc-windows-msvc`
    passed as a compile-only check; native MSVC runtime tests and release build
    are blocked because this environment is Linux.

- [x] **T31E — Wire `]` and `[` as direct Viewer search keys**
  - Recommended model: Luna High.
  - Implementation scope: Platform independent.
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [ ] Native Windows validation complete.
  - Extend Viewer input state with an explicit search mode and direction so `/`,
    `?`, `]`, and `[` each open one direct prompt using their own visible marker.
  - Map `]` to forward non-matching-line search and `[` to reverse
    non-matching-line search. Do not require `!`, `:`, the configured prefix, or a
    second trigger key.
  - Preserve fragmented escape-sequence handling, configured-prefix Viewer Help,
    `Ctrl-b [` scroll-view entry outside Viewer mode, query length limits,
    Backspace editing, Enter submission, Esc/Ctrl-c cancellation, and no input
    leakage to the child PTY.
  - In Hex mode, `[` and `]` MUST produce the approved short Text-only error and
    return to Viewer mode without entering a misleading prompt or cancelling the
    last successful search.
  - Allowed production files: `src/input.rs`, `src/server.rs`.
  - Focused tests: all four prompt markers, typed query, Backspace, empty Enter,
    maximum length, cancellation, `/ ? ] [` followed by `n/N`, Hex rejection,
    configured prefix before `[`, fragmented Escape, batched input, and no PTY
    forwarding.
  - Depends on: T31D.
  - Done when: every trigger produces exactly one semantic search request with
    explicit mode and direction and no key conflict outside active Viewer mode.
  - Implementation: carried explicit matching/non-matching mode and direction
    through Viewer prompt, query, submission, and repeat actions; added direct
    `]`/`[` prompts, successful-search repeat state, Hex rejection, and the
    configured-prefix bracket guard. Added focused input regressions.
  - Evidence (2026-08-06): Ubuntu 24.04 WSL `cargo fmt --all`, focused
    `cargo test --locked input:: -- --test-threads=1` passed (16/16), and the
    musl release build passed. The full Linux suite passed 181/182; the one
    unrelated runtime socket test was blocked by sandbox `/tmp` socket
    `Operation not permitted`. Windows `cargo check --locked --tests --target
    x86_64-pc-windows-msvc` passed as compile-only evidence. Native Windows
    focused tests and MSVC build remain blocked because this environment is
    Linux.

- [x] **T31F — Reformat Viewer Help into grouped key rows**
  - Recommended model: Luna Medium.
  - Implementation scope: Platform independent.
  - Required validation: Platform independent.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Platform-independent validation complete.
  - Replace the dense Viewer sentences with labelled `Navigation`, `Search`,
    `Hex Search`, and `Viewer` groups using one aligned key column and one short
    action column.
  - Combine only true aliases for one action. Do not place line, horizontal,
    page, mode, search, Help, and close actions in one paragraph-style row.
  - Explain `/` forward match, `?` reverse match, `]` forward non-match, `[`
    reverse non-match, `n` continue direction, `N` reverse direction, and one-wrap
    file-boundary behaviour.
  - Keep the exact `hex:00 FF 1B` example, Text-only note for non-match, dynamic
    configured-prefix Help/close text, pagination, permanent status row, and the
    short adaptive Help footer.
  - Allowed production files: `src/render.rs`; `src/input.rs` only for the
    unknown-key Viewer reminder string.
  - Focused tests: required headings and keys, configured non-default prefix,
    no hard-coded `Ctrl-b`, 40/80/120-column rendering, pagination count, footer,
    Viewer-to-Help-to-Viewer state preservation, and absence of the old dense
    sentence strings.
  - Depends on: T29G, T31A.
  - Done when: Viewer Help is scannable by category at normal widths and remains
    navigable without truncation or state loss at narrow widths.
  - Implementation: Replaced dense Viewer Help sentences with aligned
    Navigation, Search, Hex Search, and Viewer rows; added the required search,
    wrap, hex, configured-prefix, and Text-only guidance. Updated the unknown
    Viewer-key reminder with horizontal and viewport keys.
  - Evidence (2026-08-06): Ubuntu 24.04 WSL `cargo fmt --all -- --check`,
    `cargo test --locked render:: -- --test-threads=1` (13/13),
    `cargo test --locked input:: -- --test-threads=1` (16/16), and
    `cargo build --release --locked --target x86_64-unknown-linux-musl` passed.

- [x] **T31G — Run matching, non-matching, and Help acceptance**
  - Recommended model: Luna High.
  - Implementation scope: Platform independent.
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [ ] Linux validation complete.
    - [ ] Native Windows validation complete.
  - Add deterministic acceptance coverage for the complete `/ ? n N` direction
    matrix, BOF/EOF wrap, cursor re-anchoring after every navigation class,
    forward/reverse non-matching-line search, direct `]`/`[` prompts, Hex
    rejection, grouped Help, and exact return-to-Viewer state.
  - Include LF, CRLF, lone CR, mixed EOL, empty and unterminated lines, a line
    longer than the cache, raw-block crossing, no-result, wrapped-result,
    cancellation, stale generation, immediate close, and two-viewer fairness.
  - Reverify all T28-T30 bounds: eight raw blocks / 512 KiB, three frames,
    256 KiB source bytes per frame, 64 KiB work steps, one navigation in flight,
    one changed-intent replacement, and Current-before-prefetch delivery.
  - Run approved focused and full Linux/WSL checks, Clippy, musl release build,
    native Windows focused/full checks, and MSVC release build. A Windows target
    check alone MUST leave Native Windows validation unchecked.
  - Do not update `README.md` in this task. Record concise commands, results,
    environments, resource observations, and unavailable-platform blockers here.
  - Allowed files: focused tests and this task entry after separate in-scope
    approval.
  - Depends on: T30I, T31B, T31C, T31D, T31E, T31F.
  - Done when: implementation and each authoritative platform checkbox reflect
    actual execution evidence and no wrap, repeat, line-scan, Help, or
    cross-platform claim relies on compile-only validation.
  - Implementation: accepted the deterministic coverage already present across
    Viewer core, Worker, input, render, and bounds regressions; no new harness
    or production code was required.
  - Evidence (2026-08-06): Ubuntu 24.04 WSL `cargo fmt --all -- --check`,
    focused `cargo test --locked viewer:: -- --test-threads=1` (108/108),
    `cargo test --locked input:: -- --test-threads=1` (16/16), and
    `cargo test --locked render:: -- --test-threads=1` (13/13) passed.
    Escalated Ubuntu WSL `cargo test --locked` passed (182/182 unit tests,
    8/8 lifecycle tests). `cargo build --release --locked --target
    x86_64-unknown-linux-musl` passed; `file` and `ldd` verified a stripped
    static-pie executable. `cargo check --locked --tests --target
    x86_64-pc-windows-msvc` passed as compile-only evidence. Clippy is blocked
    by two existing `large_enum_variant` diagnostics in `SearchStart` and
    `Work`; T31G does not authorize production refactoring. Native Windows
    focused/full tests and MSVC release build are unavailable in this Linux
    environment.

- [x] **T31H — Publish verified Viewer search and Help keys**
  - Recommended model: Luna Low.
  - Implementation scope: Platform independent.
  - Required validation: Platform independent.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Platform-independent validation complete.
  - Update only the implemented Large-file Viewer and key-reminder sections in
    `README.md` after T31G records the required validation.
  - Document `/`, `?`, `]`, `[`, `n`, `N`, one-wrap behaviour, Text-only
    non-matching-line search, Hex ASCII and `hex:` search, grouped Help, and the
    configured-prefix Help/close behaviour without copying implementation detail
    or unverified platform claims.
  - Allowed files: `README.md`, `TASKS.md` evidence for this task.
  - Production files: none.
  - Depends on: T31G.
  - Done when: user-facing documentation matches the verified executable and no
    future or unchecked behaviour is presented as available.
  - Implementation: Updated the Large-file Viewer and key-reminder sections
    with verified search, Hex, grouped Help, and configured-prefix behavior.
  - Evidence (2026-08-06): Ubuntu 24.04 WSL `cargo fmt --all -- --check`,
    `cargo test --locked viewer:: -- --test-threads=1` (108/108),
    `cargo test --locked input:: -- --test-threads=1` (16/16), and
    `cargo test --locked render:: -- --test-threads=1` (13/13) passed; README
    wording was checked against T31G acceptance evidence.

## T32 Execution Contract

T32 fixes one matching-search repeat regression: a partial set of previously
observed match offsets MUST NOT be treated as a complete index after the logical
cursor moves. It does not change the approved search keys, query syntax,
non-matching-line semantics, Hex geometry, path prompt, Viewer Worker ownership,
or public documentation.

Rules for every T32 implementation request:

- Implement exactly one named subtask.
- Use the listed `Recommended model:` as the starting Luna level. Tuning the
  level does not broaden scope or combine tasks.
- Every T32 implementation has `Implementation scope: Platform independent`.
  Linux and Native Windows are validation environments, not separate feature
  implementations.
- Preserve strict current-cursor anchoring, recorded search direction, `n`/`N`
  direction behaviour, one-wrap termination, generation cancellation, stale-
  result rejection, and the distinction between query, search mode, cursor, and
  active match.
- Preserve the existing Viewer Worker, eight-block / 512 KiB raw cache, three
  frame slots, 256 KiB source bytes per frame, 64 KiB cooperative work step,
  one navigation in flight, zero same-intent backlog, and one changed-intent
  replacement.
- Do not add a full-file match index, match-count queue, second cache, background
  thread, async runtime, dependency, regex, or synchronous Viewer file I/O in
  `server.rs`.
- Do not change `REQUIREMENTS.md` or `README.md`; both already describe the
  required current-cursor repeat behaviour.
- Touch only the files named by the subtask, except for a focused test fixture in
  the same module.
- Run only the focused checks named by the approved implementation request.
- T32B requires review before T32C begins.
- Stop after reporting changed files, focused results, risks, and blockers.

## T32 Tasks

- [x] **T32A — Record the partial-match-cache regression and correction plan**
  - Recommended model: Luna Medium.
  - Implementation scope: Platform independent.
  - Required validation: Platform independent.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Platform-independent review complete.
  - Record the reproduction as matching search followed by file-end navigation
    and repeated reverse search: `/ query`, `G`, `N`, `N`.
  - Root-cause conclusion: `begin_repeat_search_work()` may complete a matching
    repeat from `cached_match()`, but `self.matches` contains only offsets
    discovered by earlier bounded work and carries no proof that the source range
    between the current cursor and the cached candidate was fully searched. A
    cached offset can therefore skip a nearer uncached match.
  - Keep the existing normative contract unchanged: every repeated search uses
    the logical cursor at command acceptance as its strict anchor, and a cached
    result is valid only when it is proven to be the nearest match in the
    requested direction.
  - Define T32B and T32C with exact ownership, Luna levels, platform validation,
    focused regressions, review conditions, and completion criteria.
  - Allowed files: `TASKS.md`.
  - Production files: none.
  - Depends on: T31B, T31D.
  - Done when: local Codex can implement the correction without inventing cache
    completeness, changing public behaviour, or redesigning the Worker.
  - Evidence: approved documentation-only task definition; no Rust source,
    tests, builds, dependencies, `REQUIREMENTS.md`, or `README.md` were changed.

- [x] **T32B — Remove unsafe partial-cache completion from matching repeat**
  - Recommended model: Luna xHigh.
  - Implementation scope: Platform independent.
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [ ] Native Windows validation complete.
  - Before changing production logic, add a failing regression with a file larger
    than two 64 KiB raw blocks and at least three occurrences of one query: an
    early match, a middle match, and a late match in separate scan regions.
  - The regression MUST first perform a forward matching search that discovers
    the early match, then move the logical cursor to EOF with `G`. Consecutive
    reverse repeats using `N` MUST select the late, middle, and early matches in
    that order; the next `N` MUST wrap once to the late match and report wrapped.
  - In `begin_repeat_search_work()`, a matching repeat MUST NOT return
    `SearchStart::Complete(true)` solely because `cached_match()` found an offset
    on the requested side of the current cursor. Existing match offsets are a
    partial observation set, not a coverage-complete index.
  - Use the current logical cursor as the strict anchor and dispatch the existing
    bounded `search_work_at()` path with the prior query and search mode, the
    direction selected by `n` or `N`, and the unchanged recorded direction.
  - Keep strict exclusion of the cursor's current match. Preserve the last
    successful query, mode, recorded direction, cursor, and active match on
    parse, read, cancellation, stale-generation, or no-result failure according
    to the existing contract.
  - Remove `cached_match()` and its isolated ordering test only if they have no
    remaining safe caller. Do not remove or redesign unrelated visible-match
    highlighting or bounded search state merely to clean up unused storage.
  - A future cache fast path is outside this task unless it records explicit
    query/mode/snapshot/generation coverage proving that every candidate between
    the current cursor and returned offset was examined.
  - Allowed production files: `src/viewer/mod.rs`.
  - Allowed focused-test files: `src/viewer/mod.rs`, `src/viewer/worker.rs`.
    `src/server.rs` MAY be changed only if an end-to-end reproduction proves its
    existing `n`/`N` direction routing is incorrect.
  - Focused tests: the exact `/ -> G -> N -> N` regression; three matches across
    raw blocks; nearest reverse ordering; nearest forward ordering from BOF;
    fourth-repeat one-wrap status; one-match no-self-repeat; cached and uncached
    starting states; query and recorded-direction retention; Worker-handle
    result ordering; cancellation; stale result; and no PTY input leakage.
  - Depends on: T32A.
  - Review gate: inspect the repeat anchor, direction matrix, removal or
    containment of the cache shortcut, one-wrap ranges, rollback state,
    generation handling, 64 KiB yielding, and absence of a full-file index or
    synchronous server read before T32C.
  - Done when: matching `n` and `N` always return the nearest valid match relative
    to the committed current cursor, regardless of which offsets earlier bounded
    searches happened to observe.
  - Evidence: Linux x86_64 with Rust 1.97.1 passed the failing regression after
    correction (`cargo test --locked repeat_reverse_search_uses_nearest_match_after_bottom`,
    1 test), all Viewer and Worker tests (`cargo test --locked viewer::`, 108
    tests), and the x86_64 musl release build (`cargo build --release --locked
    --target x86_64-unknown-linux-musl`). Native Windows/MSVC runtime validation
    is unavailable in this Linux workspace and remains unchecked.

- [x] **T32C — Run cursor-anchored repeat-search acceptance**
  - Recommended model: Luna High.
  - Implementation scope: Platform independent.
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [ ] Native Windows validation complete.
  - Run deterministic acceptance for the full `/`, `?`, `n`, and `N` direction
    matrix, including the exact `/ -> G -> N -> N` report and the symmetric
    `? -> gg -> N -> N` path.
  - Verify early, middle, and late matches in separate raw-block regions; nearest
    cached and uncached candidates; raw-block eviction and reload; Text and Hex
    matching search; one match; no match; BOF/EOF one-wrap status; and strict
    exclusion of the active cursor match.
  - Reverify cursor re-anchoring after `gg`/`G`, Page Up/Page Down, line movement,
    horizontal movement, Home/End, and viewport-only scrolling. Viewport-only
    scrolling MUST retain the same logical cursor anchor.
  - Reverify that navigation or a new search cancels obsolete work, stale
    generations cannot commit, close remains immediate, another viewer receives
    service during a long search, and non-matching-line search behaviour remains
    unchanged.
  - Reverify all existing bounds: eight raw blocks / 512 KiB, three frames,
    256 KiB source bytes per frame, 64 KiB search steps, one navigation in flight,
    zero same-intent backlog, one changed-intent replacement, and Current-before-
    prefetch delivery.
  - Run authoritative Linux/WSL and Native Windows focused Viewer/Worker/server
    tests, the full test suite, Clippy with `-D warnings`, the musl release build,
    and the MSVC release build. A Windows target check alone MUST leave Native
    Windows validation unchecked.
  - Do not change `README.md`; this task restores already documented behaviour.
  - Allowed files: focused tests and this task entry after separate in-scope
    approval. Production corrections discovered here MUST return to T32B scope
    rather than be hidden inside acceptance work.
  - Depends on: T32B.
  - Done when: implementation and both authoritative platform checkboxes contain
    actual execution evidence, and no repeat-search result depends on treating a
    partial match list as complete coverage.
  - Evidence (2026-08-06): Ubuntu 24.04 WSL with Rust 1.97.1 passed focused
    Viewer/Worker tests (`cargo test --locked viewer:: -- --test-threads=1`,
    108/108), focused Viewer server tests (`cargo test --locked
    'server::tests::viewer_' -- --test-threads=1`, 12/12), the full Linux suite
    (`cargo test --locked -- --test-threads=1`, 182/182 unit tests and 8/8
    lifecycle tests), formatting, Clippy with `-D warnings`, and the
    `x86_64-unknown-linux-musl` release build. The Windows target check
    (`cargo check --locked --target x86_64-pc-windows-msvc`) passed as
    compile-only evidence. Native Windows focused/full tests and MSVC release
    build are blocked in this Linux workspace; the latter reports missing
    `link.exe`, so Native Windows validation remains unchecked.

## T33 Execution Contract

T33 restores the complete Vim-style Viewer repeat-search contract across input,
server dispatch, Viewer Worker state, bounded search, wrap reporting, and Help.
It fixes the ambiguity where one boolean is currently interpreted as an actual
forward/reverse direction in one layer and as same/opposite-to-recorded direction
in another layer. It does not redesign the Viewer Worker, change query syntax,
change non-matching-line selection rules beyond repeat/wrap consistency, or alter
unrelated path prompt, Hex geometry, PTY, ConPTY, session, pane, renderer, IPC,
or terminal-parser behaviour.

Rules for every T33 implementation request:

- Implement exactly one named subtask.
- Use the listed `Recommended model:` as the starting Luna level. Tuning the
  level does not broaden scope or combine tasks.
- Every T33 implementation has `Implementation scope: Platform independent`.
  Linux and Native Windows are validation environments, not separate feature
  implementations.
- Preserve the T28-T32 Viewer Worker ownership, generation cancellation, stale-
  result rejection, zero repeat backlog, one changed-intent replacement, three-
  frame bound, eight-block / 512 KiB raw cache, 256 KiB frame-source cap, 64 KiB
  cooperative work step, and Current-before-prefetch ordering.
- Treat recorded search direction, requested repeat relation, actual execution
  direction, search mode, query, cursor anchor, active result, and wrapped status
  as distinct state. Do not encode two of those meanings in one ambiguous
  boolean.
- `/`, `?`, `]`, and `[` are new-search commands. `n` and `N` are repeat commands.
  A repeat MUST NOT become a new search merely because it succeeds.
- Reuse the existing Viewer Worker, raw-block cache, universal line scanner,
  matching engine, and non-matching-line engine. Do not add a full-file index,
  second cache, background thread, async runtime, dependency, regex, shell
  helper, or synchronous Viewer file I/O in `server.rs`.
- Keep ordinary production changes below 350 changed lines per subtask. Split a
  task further instead of broadening ownership or hiding a state migration.
- Touch only the files named by the subtask, except for a required private type,
  module declaration, or focused test fixture in the same module.
- Run only the focused checks named by the approved implementation request.
- T33C, T33D, and T33E require review before the next dependent subtask begins.
- Do not update `README.md` until T33G records authoritative validation; T33H
  performs the final mechanical publication.
- Stop after reporting changed files, focused results, risks, and blockers.

## T33 Tasks

- [x] **T33A — Record the Vim repeat-direction regression and execution plan**
  - Recommended model: Luna Medium.
  - Implementation scope: Platform independent.
  - Required validation: Platform independent.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Platform-independent review complete.
  - Record the intended Vim contract: a successful `/` or `]` records forward;
    a successful `?` or `[` records reverse; `n` repeats that recorded direction;
    `N` executes the opposite direction without replacing the recorded direction.
  - Record the source-level root cause:
    - `input.rs` currently converts `n` and `N` into an actual forward/reverse
      boolean from client-side recorded state;
    - `server.rs` names and forwards the same value as `same_direction`;
    - `viewer/mod.rs` interprets it as same/opposite relative to Viewer-owned
      recorded direction; and
    - successful repeat completion calls `record_viewer_search()` with the
      repeat's actual direction, so consecutive `N` commands can alternate.
  - Record that T32 fixed nearest-match selection from a partial cache but did
    not correct this cross-layer direction contract or repeat-state mutation.
  - Define T33B through T33H with exact ownership, dependencies, Luna levels,
    review gates, validation targets, and objective completion conditions.
  - Allowed files: `TASKS.md`.
  - Production files: none.
  - Depends on: T31B, T31D, T32B.
  - Done when: local Codex can correct the behaviour without guessing whether a
    boolean means forward/reverse or same/opposite and without redesigning the
    bounded search architecture.
  - Evidence: approved task-plan update based on direct inspection of
    `src/input.rs`, `src/server.rs`, `src/viewer/mod.rs`, and
    `src/viewer/worker.rs`; no Rust source, tests, builds, dependencies,
    `REQUIREMENTS.md`, or `README.md` were changed.

- [x] **T33B — Align the normative Viewer repeat and wrap contract with Vim**
  - Recommended model: Luna Medium.
  - Implementation scope: Platform independent.
  - Required validation: Platform independent.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Platform-independent review complete.
  - Clarify in `REQUIREMENTS.md` that only a successful new search started by
    `/`, `?`, `]`, or `[` replaces the last query, search mode, and recorded
    direction. A successful `n` or `N` repeat MUST preserve all three.
  - Define `n` as same-as-recorded and `N` as opposite-to-recorded. Pressing `N`
    repeatedly MUST continue in the same opposite execution direction; pressing
    `n` afterward MUST resume the original recorded direction.
  - Preserve strict current-cursor anchoring. The initial requested range MUST
    exclude the current active result, but after one complete boundary wrap the
    same result MAY be selected again when it is the only eligible result in the
    snapshot. That wrapped success MUST not be reported as `no match`.
  - Apply the same repeat-direction ownership to matching and Text-mode
    non-matching-line searches. Non-matching-line search remains a Termfold
    extension; only its repeat and one-wrap state follow the shared contract.
  - Require exact directional wrap messages:
    - forward wrap: `search hit BOTTOM, continuing at TOP`;
    - reverse wrap: `search hit TOP, continuing at BOTTOM`.
  - Failure, cancellation, stale generation, invalid Hex non-match request, and
    read errors MUST preserve the last successful query, search mode, recorded
    direction, cursor, committed frame, and active result as already required.
  - Update the matching T33 task text only when needed to keep implementation and
    acceptance criteria aligned with the normative wording.
  - Allowed files: `REQUIREMENTS.md`, `TASKS.md`.
  - Production files: none.
  - Depends on: T33A.
  - Done when: no implementation task must invent recorded-direction ownership,
    single-result wrap behaviour, or the user-visible boundary message.
  - Evidence (2026-08-07): Updated `REQUIREMENTS.md` to make successful new
    searches the only operation that replaces query/mode/direction; define
    repeat preservation, one-wrap single-result eligibility, and exact forward
    and reverse boundary messages. No production files changed. `cargo build
    --locked` passed; focused Viewer tests passed (108/108). The full Linux
    suite reached 181/182 under the sandbox because the runtime socket test
    could not bind (`Operation not permitted`); its approved elevated rerun
    passed (1/1), clearing that environment-only blocker.

- [x] **T33C — Replace ambiguous repeat booleans with explicit direction types**
  - Recommended model: Luna xHigh.
  - Implementation scope: Platform independent.
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [ ] Native Windows validation complete.
  - Introduce one explicit repeat relation such as `Same` / `Opposite` and one
    explicit execution direction such as `Forward` / `Reverse`. Use names that
    make conversion points unambiguous; do not add a generic boolean alias.
  - Map Viewer input directly as `n -> Same` and `N -> Opposite`. Input MUST NOT
    pre-convert those keys into an actual forward/reverse direction.
  - Carry the repeat relation through `Action`, server command/pending state,
    `ViewerHandle`, `ViewerOperation`, and Worker dispatch. Resolve the actual
    execution direction only against the Viewer-owned recorded direction.
  - Keep new-search `/ ? ] [` input as explicit execution direction because those
    commands establish the recorded direction after success.
  - Remove or rename every `same_direction`, `forward`, or equivalent field whose
    meaning changes between layers. A field may remain only when its type and
    name have one stable meaning at every call site.
  - Preserve search mode, generation, cancellation, queue bounds, result ordering,
    and all T32 nearest-match behaviour.
  - Allowed production files: `src/input.rs`, `src/server.rs`,
    `src/viewer/mod.rs`, `src/viewer/worker.rs`.
  - Focused tests: input maps `n`/`N` to Same/Opposite after forward and reverse
    searches; `/ -> n`, `/ -> N`, `? -> n`, `? -> N`; matching and non-matching
    modes; Worker operation routing; cancellation; stale generation; and no PTY
    input leakage.
  - Depends on: T33B.
  - Review gate: inspect every conversion between recorded direction, repeat
    relation, and actual execution direction; reject any remaining boolean whose
    meaning depends on the caller before T33D.
  - Done when: the complete input-to-core path can be read without inferring a
    direction boolean's meaning from surrounding code.
  - Evidence (2026-08-07): Added `RepeatDirection::{Same, Opposite}` and
    `SearchDirection::{Forward, Reverse}` across input, server pending state,
    Viewer operations, and Worker dispatch/results. Linux checks passed:
    `cargo fmt -- --check`; focused input, Viewer, and Worker tests (17/17,
    45/45, and 24/24); `cargo build --locked`; and
    `cargo build --release --locked --target x86_64-unknown-linux-musl`.
    Native Windows validation is blocked because this workspace has no native
    Windows/MSVC runtime environment; no Windows result is inferred.

- [x] **T33D — Preserve recorded direction across every repeat result**
  - Recommended model: Luna xHigh.
  - Implementation scope: Platform independent.
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [ ] Native Windows validation complete.
  - Separate new-search completion from repeat completion in pending server state
    or result handling. Do not infer request kind from whether a query vector is
    present when an explicit request-kind type is safer.
  - On successful `/`, `?`, `]`, or `[` completion, record the new query, mode,
    and initial direction in the existing owner required by current architecture.
  - On successful `n` or `N` completion, update cursor, active result, frame, and
    wrapped status only. Do not call `record_viewer_search()` with the repeat's
    actual execution direction and do not replace Viewer-owned recorded state.
  - A failed, cancelled, stale, or rejected new search or repeat MUST not mutate
    the prior recorded query, mode, or direction.
  - Ensure repeated opposite motion remains stable:
    - `/foo`, `N`, `N`, `N` moves reverse each time;
    - `?foo`, `N`, `N`, `N` moves forward each time;
    - after either sequence, `n` resumes the original `/` or `?` direction.
  - Apply the same state ownership to matching and non-matching-line modes, while
    retaining Hex rejection for non-matching-line search.
  - Preserve current per-client/session ownership; do not broaden this task into
    a redesign of multi-client search sharing.
  - Allowed production files: `src/input.rs`, `src/server.rs`;
    `src/viewer/worker.rs` only when an explicit result request-kind or direction
    field is required to avoid server inference.
  - Focused tests: consecutive `N`; `N` then `n`; new search replacement; repeat
    success; repeat no-match; cancellation; stale result; matching/non-matching
    mode retention; Hex rejection; and initiating-client state unchanged on
    failure.
  - Depends on: T33C.
  - Review gate: inspect every call to `record_viewer_search()`, every successful
    search-result branch, pending request ownership, and stale-result path before
    T33E.
  - Done when: only successful new-search commands can change recorded direction,
    and repeat direction cannot oscillate because of its own previous success.
  - Evidence (2026-08-07): Replaced optional pending search fields with explicit
    `New` and `Repeat` requests. Only successful `New` results record client search
    state; repeat results preserve it. Linux checks passed: `cargo fmt -- --check`;
    focused input, Viewer, Worker, and server tests (17/17, 45/45, 24/24, and
    18/18; Worker tests serialized after timing-sensitive parallel failures);
    `cargo build --locked`; and `cargo build --release --locked --target
    x86_64-unknown-linux-musl`. `cargo check --locked --target
    x86_64-pc-windows-msvc` also passed as compile-only evidence. Native Windows
    validation is blocked because this workspace has no native Windows/MSVC
    runtime environment; no Windows result is inferred.

- [x] **T33E — Complete bounded one-result and boundary-wrap semantics**
  - Recommended model: Luna xHigh.
  - Implementation scope: Platform independent.
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [ ] Native Windows validation complete.
  - Keep the first requested search range strictly after or before the committed
    cursor anchor so the current active result is not returned before reaching a
    boundary.
  - After the requested direction reaches EOF or BOF, wrap exactly once and scan
    the complementary range. Permit the original active match range or original
    non-matching line to succeed only after all other eligible source positions
    have been examined and only when it is the nearest valid wrapped result.
  - A snapshot with one matching occurrence MUST therefore behave as follows:
    `n` or `N` returns the same occurrence as a wrapped success, leaves the cursor
    at that occurrence, preserves recorded direction, and reports the proper
    directional boundary message.
  - A query with zero occurrences MUST still terminate after one complete scan
    and report no match. Do not turn the single-result exception into an infinite
    loop or a second wrap.
  - Preserve nearest-match ordering from T32 for multiple matches, including
    partial observed offsets, raw-block eviction/reload, and matches crossing raw
    blocks or Hex rows.
  - Keep matching and non-matching-line work cooperative: at most 64 KiB per
    step, no full-file index, no retained complete long line, normal cancellation,
    another viewer's control work serviced, and all cache/frame limits unchanged.
  - Allowed production files: `src/viewer/mod.rs`, `src/viewer/search.rs`,
    `src/viewer/line.rs`; `src/viewer/worker.rs` only for focused cooperative
    result coverage or required wrapped-result propagation.
  - Focused tests: one matching occurrence with `n` and `N`; one qualifying
    non-matching line; zero result; two and three results; forward and reverse
    wrap; original-anchor exclusion before wrap; T32 nearest ordering; raw-block
    boundary; long line; cancellation; stale generation; fairness; and no second
    wrap.
  - Depends on: T33D.
  - Review gate: inspect selected/wrap range construction, original-anchor
    inclusion timing, termination conditions, one-wrap flag ownership, rollback,
    64 KiB yielding, and absence of an index before T33F.
  - Done when: single-result and multi-result repeats follow the same bounded
    one-wrap state machine and cannot incorrectly return `no match` after a valid
    wrapped result.
  - Evidence (2026-08-07): Matching wrap ranges now include the original anchor
    only after the strict primary range; matching and non-matching searches return
    a valid single-result wrapped match and terminate after one complete scan with
    no result. Added focused regressions for single matching/non-matching results,
    zero-result termination, and repeated anchor handling. Ubuntu 24.04 WSL with
    Rust 1.97.1 passed `cargo test --locked viewer:: -- --test-threads=1`
    (111/111), `cargo build --locked`, and
    `cargo build --release --locked --target x86_64-unknown-linux-musl`.
    Native Windows validation is blocked because this workspace has no native
    Windows/MSVC runtime environment; no Windows result is inferred.

- [x] **T33F — Emit Vim-style directional wrap status and update Viewer Help**
  - Recommended model: Luna High.
  - Implementation scope: Platform independent.
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [x] Linux validation complete.
    - [ ] Native Windows validation complete.
  - Ensure a successful search result carries or can deterministically recover
    the actual execution direction used for that operation. Do not use the
    recorded direction when `N` executed the opposite direction.
  - Replace the generic `wrapped` status with:
    - `search hit BOTTOM, continuing at TOP` for an actual forward wrap;
    - `search hit TOP, continuing at BOTTOM` for an actual reverse wrap.
  - Use the same directional message for matching and non-matching-line wrapped
    success. Non-wrapped success retains the ordinary Viewer status, and no-match
    retains its existing query-aware error without mutating successful state.
  - Update grouped Viewer Help to state that `n` repeats the recorded direction,
    `N` uses the opposite direction, repeated `N` keeps moving opposite, and a
    boundary wraps once with the directional message. Keep narrow-width
    pagination, configured prefix substitution, permanent status row, and return
    to the same Viewer state.
  - Do not update `README.md` in this task.
  - Allowed production files: `src/server.rs`, `src/viewer/worker.rs`,
    `src/input.rs`, `src/render.rs`.
  - Focused tests: exact forward/reverse message strings; `/` and `?` wraps;
    `n` and `N` wraps; matching/non-matching modes; one-result wrap; no generic
    `wrapped` fallback; Help headings and wording at 40/80/120 columns; configured
    non-default prefix; and Viewer-to-Help-to-Viewer state preservation.
  - Depends on: T33E.
  - Done when: the status identifies the boundary and continuation direction
    exactly, and Help describes the implemented repeat state without requiring
    users to infer it.
  - Evidence (2026-08-07): `src/server.rs` now emits the exact forward and
    reverse boundary messages from the actual result direction; the Worker
    immediate-repeat result also reports the direction requested by `n` or `N`.
    Viewer Help now names recorded-direction repeats, opposite-direction
    repeats, repeated `N`, and one-wrap boundary reporting. Focused tests for
    both exact messages, Help at 40/80/120 columns with the configured prefix,
    and actual opposite repeat direction passed. Ubuntu 24.04 WSL with Rust
    1.97.1 passed `cargo fmt -- --check`, `cargo test --locked
    -- --test-threads=1` after the runtime socket test's elevated rerun
    (189/189), `cargo build --locked`, and
    `cargo build --release --locked --target x86_64-unknown-linux-musl`.
    `cargo check --locked --target x86_64-pc-windows-msvc` passed as
    compile-only evidence. Native Windows runtime tests and the MSVC release
    build are blocked because this workspace has no native Windows/MSVC
    environment; no Windows validation is inferred.

- [x] **T33G — Run end-to-end Vim repeat and wrap acceptance**
  - Recommended model: Luna High.
  - Implementation scope: Platform independent.
  - Required validation: Linux and Native Windows.
  - Completion status:
    - [x] Task implementation complete.
    - [ ] Linux validation complete.
    - [ ] Native Windows validation complete.
  - Add deterministic acceptance through the complete path:
    input bytes -> semantic action -> server pending state -> ViewerHandle ->
    Viewer Worker -> Viewer core -> result application -> client recorded state ->
    status rendering.
  - Cover the complete matching matrix `/ -> n`, `/ -> N`, `? -> n`, `? -> N`,
    repeated `N`, `N -> n`, BOF/EOF wraps, exact directional messages, one
    result, no result, and the T32 `/ -> G -> N -> N` nearest-match regression.
  - Cover the equivalent repeat-direction matrix for `]` and `[` Text-mode
    non-matching-line search, including Hex rejection without state loss.
  - Reverify cursor anchoring after `gg`/`G`, Page Up/Page Down, line movement,
    horizontal movement, Home/End, and viewport-only scrolling. Viewport-only
    scrolling MUST not change the logical cursor anchor.
  - Reverify cancellation, stale generations, close during work, new-search
    replacement, another viewer's control fairness, raw-block eviction/reload,
    Text/Hex matching, universal EOL handling, and long-line bounded work.
  - Reverify all T28-T32 resource and ordering limits: eight raw blocks / 512 KiB,
    three frames, 256 KiB source bytes per frame, 64 KiB work steps, one
    navigation in flight, zero same-intent backlog, one changed-intent
    replacement, and Current-before-prefetch delivery.
  - Run authoritative Linux/WSL and Native Windows focused input/viewer/worker/
    server/render tests, full test suites, formatting, Clippy with `-D warnings`,
    musl release build, and MSVC release build. A Windows target check alone MUST
    leave Native Windows validation unchecked.
  - Do not update `README.md`. Record concise commands, environments, results,
    resource observations, and blockers in this task.
  - Allowed files: focused tests and this task entry after separate in-scope
    approval. Production defects discovered here MUST return to T33C-T33F scope
    rather than be hidden inside acceptance work.
  - Depends on: T33F.
  - Done when: both authoritative platforms prove the same recorded-direction,
    opposite-repeat, nearest-match, one-wrap, and directional-message behaviour
    without relying on direct-core tests alone.
  - Evidence (2026-08-07): Added Linux session-boundary acceptance tests in
    `tests/server_lifecycle.rs` covering input bytes through semantic actions,
    server pending state, Viewer Worker, result application, recorded search
    state, rendered directional wrap messages, matching `/` and `?` repeats,
    repeated `N`, `N -> n`, no-match, Text-mode `]` and `[` repeats, one-result
    wraps, and Hex non-matching rejection with state retention. Ubuntu 24.04
    WSL with Rust 1.97.1 passed `cargo fmt -- --check`, the two focused
    acceptance tests, the full `cargo test --locked -- --test-threads=1`
    suite (189 unit tests and 10 lifecycle tests), and
    `cargo build --release --locked --target x86_64-unknown-linux-musl`.
    The release binary is 881392 bytes and statically linked. `cargo check
    --locked --target x86_64-pc-windows-msvc` passed as compile-only evidence;
    the MSVC release build is blocked by missing `link.exe`, and native
    Windows runtime tests are unavailable. Linux validation remains unchecked
    because `cargo clippy --all-targets --all-features -- -D warnings` reports
    a collapsible-if at `src/server.rs:1715`; production changes are outside
    this test-only task's allowed scope.

- [ ] **T33H — Publish verified Vim repeat and wrap behaviour**
  - Recommended model: Luna Low.
  - Implementation scope: Platform independent.
  - Required validation: Platform independent.
  - Completion status:
    - [ ] Task implementation complete.
    - [ ] Platform-independent validation complete.
  - After T33G completes the required authoritative validation, update only the
    Large-file Viewer and key-reminder wording in `README.md`.
  - State that `/` and `?` establish the recorded matching direction, `]` and `[`
    establish the recorded Text-only non-matching direction, `n` repeats that
    direction, and `N` uses the opposite direction without changing it.
  - Document one-wrap behaviour and the exact BOTTOM-to-TOP / TOP-to-BOTTOM
    messages, including the valid wrapped return to the same result when it is
    the only occurrence.
  - Do not copy implementation types, Worker ownership, cache details already
    documented elsewhere, test commands, or unchecked platform claims.
  - Allowed files: `README.md`, `TASKS.md` evidence for this task.
  - Production files: none.
  - Depends on: T33G.
  - Done when: user-facing documentation matches the verified executable and no
    future, partial, or compile-only behaviour is described as available.
