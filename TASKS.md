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

- [ ] **T28C — Extract snapshot FileSource and the raw-block cache**
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

- [ ] **T28D — Implement one universal, resumable line scanner**
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

- [ ] **T28E — Implement safe text-token decoding**
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
  - Done when: viewer navigation and rendering no longer derive a terminal column
    by subtracting two byte offsets.

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

- [ ] **T28P — Highlight visible search matches**
  - Map every matching source range in Current to display-cell spans through the
    PageFrame mapping.
  - Render ordinary visible matches with an attribute-based highlight and the
    active match with inverse plus underline.
  - Do not scan outside Current only to produce highlights.
  - Preserve the last successful query for `n`/`N`; report `wrapped` after a
    wrapped success.
  - Allowed production files: `src/viewer/frame.rs`, `src/viewer/search.rs`,
    `src/viewer/mod.rs`.
  - Focused tests: multiple visible matches, active distinction, horizontal
    clipping, tabs, wide text, invalid-byte replacement, and monochrome output.
  - Depends on: T28I, T28O.
  - Done when: all visible matches are marked without colour-only meaning or a
    full-file highlight scan.

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
  - Done when: the complete T28 contract passes on authoritative Linux/WSL and
    native Windows environments, or each unavailable acceptance environment is
    recorded as a blocker and T28 remains incomplete.

