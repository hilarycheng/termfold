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
  `Luna Medium`, `Luna High`, or `Luna Max`. This is advisory execution metadata,
  not product behaviour, permission to broaden scope, or permission to combine
  tasks. The owner may tune the level after observing results.
- Add `Platform:` only when behaviour or implementation depends on an
  operating-system-specific API or code path. Accepted values are `Linux`,
  `Windows`, or `Linux and Windows; separate implementations`. Omit the field for
  shared source and shared behaviour. Cross-platform validation targets belong
  under focused checks or evidence, not under `Platform:`.

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

- [*] **T28A — Define the replacement viewer contract and implementation plan**
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

- [*] **T28B — Mechanically establish the viewer module tree**
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

- [*] **T28C — Extract snapshot FileSource and the raw-block cache**
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

- [*] **T28D — Implement one universal, resumable line scanner**
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

- [*] **T28E — Implement safe text-token decoding**
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

- [ ] **T28F — Add and validate `viewer_tab_width`**
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
    passed. The full test run required elevated temporary Unix-socket access.

- [ ] **T28G — Build source-byte to display-cell spans**
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
    x86_64 musl release build passed.

- [ ] **T28H — Correct Text-mode cursor semantics**
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
    x86_64 musl release build passed.

- [ ] **T28I — Introduce the Current PageFrame builder**
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
    tests; the x86_64 musl release build passed.

- [ ] **T28J — Add Previous/Current/Next frame rotation**
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
    tests; the x86_64 musl release build passed.

- [ ] **T28K — Add the session-scoped Viewer Worker foundation**
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
    tests; the x86_64 musl release build passed.

- [ ] **T28L — Move page and line navigation behind the Viewer Worker**
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
    Native Windows/MSVC validation was not available in this Linux host.

- [ ] **T28M — Enforce zero repeat backlog and one changed-intent replacement**
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
    Windows/MSVC validation was not available in this Linux host.

- [ ] **T28N — Define text and hex search query types**
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
    x86_64-unknown-linux-musl release build passed.

- [ ] **T28O — Implement incremental cancellable forward/reverse search**
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
    validation was not available in this Linux host.

- [ ] **T28P — Highlight visible search matches**
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
    validation was not available in this Linux host.
  - Verification note: repository-wide `cargo fmt` was applied and
    `cargo fmt --check` passed.

- [ ] **T28Q — Implement Hex PageFrame rendering and navigation**
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
    67 viewer tests; the x86_64-unknown-linux-musl release build passed.
    Native Windows/MSVC validation was not available in this Linux host.

- [ ] **T28R — Add Hex-mode ASCII and exact-byte search**
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
    Native Windows/MSVC validation was not available in this Linux host.

- [ ] **T28S — Wire mode switching, cancellation, and input actions**
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
    Native Windows/MSVC validation was not available.

- [ ] **T28T — Remove superseded and duplicated viewer code**
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
    Native Windows/MSVC validation was not available in this Linux host.

- [ ] **T28U — Run viewer acceptance, resource, and documentation checks**
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
    for this warning-only cleanup. T28U remains incomplete until those checks
    and native Windows acceptance are rerun on the final source.
  - Done when: the complete T28 contract passes on authoritative Linux/WSL and
    native Windows environments, or each unavailable acceptance environment is
    recorded as a blocker and T28 remains incomplete.

### T28 native Windows confirmation ledger

- Confirmed: T28B, T28C, T28D, and T28E each record native Windows/MSVC focused
  tests and release-build evidence.
- Not confirmed: T28F through T28T have Linux/WSL implementation evidence but
  no recorded native Windows/MSVC focused test and build for their final code;
  their unchecked status is intentional until that validation is supplied.
- T28A is documentation-only. T28U remains the acceptance gate for the missing
  native Windows validation and the final cross-platform claim.

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

- [*] **T29A — Define the viewer-correction contract and Luna execution plan**
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

- [*] **T29B — Keep Tab-completed viewer paths editable**
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

- [*] **T29C — Prevent literal-tilde path-prompt crashes**
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

- [ ] **T29D — Implement horizontal viewer cursor primitives**
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

- [ ] **T29E — Wire `h`/`l` and Left/Right through the Viewer Worker**
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

- [ ] **T29F — Remove the phantom row above the status bar**
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

- [ ] **T29G — Make Viewer Help complete and return-safe**
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

- [ ] **T29H — Wake the Session Server on Viewer Worker results**
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

- [ ] **T29I — Remove neighbour prefetch from the visible-frame critical path**
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

- [ ] **T29J — Combine page navigation and visible rendering safely**
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

- [ ] **T29K — Run viewer-correction acceptance and update documentation**
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
