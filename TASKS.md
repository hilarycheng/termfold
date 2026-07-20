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
  - Implement the required commands, PID-prefix selector, defaults, session-name
    validation, strict configuration parsing, and actionable errors.
  - Requirements: Command-Line Contract; Configuration.
  - Depends on: T00, T01.
  - Done when: every documented command and configuration validation path behaves
    as specified.

- [ ] **T03 — Implement session, tab, pane, and layout state**
  - Enforce resource limits, split constraints, deterministic focus, resize, and
    close behaviour without starting PTYs yet.
  - Requirements: Tabs and Panes; Resource Limits; Session and Process Lifecycle.
  - Depends on: T01.
  - Done when: state transitions cannot violate the documented limits or hierarchy.

- [ ] **T04 — Implement secure runtime paths**
  - Validate runtime-directory ownership and permissions, reject symlinks, create
    the Unix socket securely, and handle stale sockets safely.
  - Requirements: IPC and Filesystem Security.
  - Depends on: T01.
  - Done when: runtime paths and sockets meet every ownership, mode, and type rule.

- [ ] **T05 — Implement framed IPC**
  - Add versioned messages, the 1 MiB frame limit, malformed-frame rejection, and
    single-client attachment enforcement.
  - Requirements: IPC and Filesystem Security; Command-Line Contract.
  - Depends on: T00, T04.
  - Done when: client and server exchange only bounded, valid protocol messages.

- [ ] **T06 — Implement PTY and child-process lifecycle**
  - Launch the approved shell directly with the required environment and working
    directory, propagate sizes, terminate gracefully, and reap every child.
  - Requirements: Shell Launch; Session and Process Lifecycle.
  - Depends on: T00, T01.
  - Done when: pane processes start, resize, terminate, and reap deterministically.

- [ ] **T07 — Implement server lifecycle**
  - Add one server process per session, auto-start, PID-prefix discovery, create,
    attach, detach, list with attachment state, kill, empty-pane cascading, and
    shutdown with the session.
  - Requirements: Command-Line Contract; Session and Process Lifecycle.
  - Depends on: T02, T03, T05, T06.
  - Done when: sessions persist only while required and a second client is rejected.

- [ ] **T08 — Implement the terminal parser and screen model**
  - Support the required UTF-8, cell-width, cursor, scrolling, editing, SGR, screen,
    input-mode, and escape-sequence behaviour with bounded parsing.
  - Ignore OSC 52 writes and safely discard unsupported or oversized sequences.
  - Requirements: Terminal Behaviour; Resource Limits.
  - Depends on: T01.
  - Done when: the advertised `xterm-256color` subset is represented correctly.

- [ ] **T09 — Implement client terminal safety**
  - Manage terminal modes, alternate screen, resize signals, disconnects, normal
    exit, panic, and catchable termination signals with deterministic restoration.
  - Requirements: First-Release Scope; Terminal Behaviour; Mouse and Scrollback.
  - Depends on: T05, T08.
  - Done when: every supported exit path restores the outer terminal.

- [ ] **T10 — Implement pane and status rendering**
  - Render pane content, ASCII borders, active-pane state, and the one-row status
    bar with required truncation priorities and clock-only redraws.
  - Requirements: Tabs and Panes; Status Bar.
  - Depends on: T03, T08, T09.
  - Done when: normal and narrow layouts preserve the specified visibility order.

- [ ] **T11 — Implement keyboard input**
  - Forward bytes unchanged outside prefix mode and implement every required prefix
    command, unsupported-command message, and close confirmation.
  - Requirements: Default Keys.
  - Depends on: T03, T06, T09.
  - Done when: keyboard-only operation covers all first-release actions.

- [ ] **T12 — Implement bounded scrollback**
  - Retain complete lines up to the configured limit, discard oldest lines first,
    and implement scrollback mode.
  - Requirements: Mouse and Scrollback; Configuration; Resource Limits.
  - Depends on: T00, T08, T11.
  - Done when: history remains bounded and navigable without corrupting pane output.

- [ ] **T13 — Implement optional mouse input**
  - Keep mouse disabled by default; add SGR click, drag, wheel, tab selection, pane
    selection, border resize, application forwarding, and cleanup.
  - Requirements: Mouse and Scrollback.
  - Depends on: T03, T09, T10, T12.
  - Done when: mouse behaviour is complete without reducing keyboard functionality.

- [ ] **T14 — Complete lifecycle and compatibility integration**
  - Verify attach/detach persistence, pane-exit cascading, resize propagation,
    bounded queues, SSH behaviour, WSL behaviour, and narrow-terminal handling.
  - Requirements: all first-release behavioural sections.
  - Depends on: T07 through T13.
  - Done when: all components operate together without terminal or process leaks.

- [ ] **T15 — Perform release validation**
  - Run the approved formatting, lint, test, security, musl build, static-linkage,
    checksum, compatibility, and resource-measurement checks.
  - Requirements: Implementation and Acceptance; release checklist in `AGENTS.md`.
  - Depends on: T00 through T14.
  - Done when: every approved budget and release-checklist item passes or has a
    documented blocker.
