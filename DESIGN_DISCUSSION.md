# Design Discussion

This document records agreed direction from the 2026-07-31 discussion. It is
not an implementation-status document.

## Session termination

- `Ctrl-b x` already asks `close pane? (y/n)` before closing the active pane.
- `termfold kill [NAME]` terminates the entire session immediately.
- Proposed: require confirmation for the CLI kill command, with an explicit
  non-interactive override for scripts.

## Windows rendering

Linux rendering is reported as acceptable; optimize the native Windows path
only after measuring the failing case and outer terminal.

Current shared rendering scans and snapshots the visible terminal cells on
each PTY update. Windows-specific investigation must distinguish renderer CPU,
named-pipe transfer, and outer-terminal output latency before changing shared
rendering behaviour.

## Updating while sessions are active

- A session server owns its PTYs and cannot be live-upgraded today.
- Windows cannot replace an executable while an active client or server holds
  it open.
- A newer executable may run side-by-side and attach to an older session only
  while their IPC protocol versions remain compatible.
- A true server upgrade requires explicit PTY/session handover and is not in
  scope for the initial viewer work.

## Startup profiles

Startup profiles are declarative session definitions in `config.toml`. They
run once when Termfold creates a session and never when a client attaches to an
existing session.

- A pane launch target is either a shell or a direct program. The shell remains
  the default target when no profile target is defined.
- A direct target is an absolute executable plus literal arguments; it starts
  without an intermediate Bash, Fish, or command-shell process.
- A profile directory is the session's initial directory and overrides the
  creating client's directory after validation.
- A profile may define multiple tabs and a nested, mixed horizontal/vertical
  split tree. Every leaf supplies its launch target and may override the
  profile directory.
- All configured targets and directories must be validated before spawning.
  If a launch fails, Termfold must terminate every target it already started.

## Mouse and WezTerm

- `mouse = false` is the current default and disables Termfold mouse capture.
- In WezTerm's alternate screen, a wheel event without mouse reporting becomes
  Up/Down key input. Termfold forwards that normal input to the active pane,
  which can move an interactive prompt.
- The WezTerm workaround is a `mouse_bindings` entry for `WheelUp` and
  `WheelDown` with `alt_screen = true`, `mouse_reporting = false`, and
  `action = wezterm.action.Nop`.
- With `mouse = true`, Termfold receives mouse sequences. It forwards them to
  the active application only when that application enables mouse reporting;
  otherwise Termfold owns pane/status interactions and wheel scrollback.

## Large-file viewer

### Goal

Provide a read-only local text/log viewer without loading the entire file into
memory. It must support start/end navigation, forward and reverse literal
search, and Vim-like navigation.

### Entry points

- `Ctrl-b v` prompts for a path using the active pane's working directory and
  opens a viewer pane.
- `termfold view FILE` targets the caller's current Termfold session.
- Panes need a Termfold-provided session identifier so the CLI can identify
  that session. Outside Termfold, require an explicit session argument.

### File and search model

- Use standard-library seek/read operations over fixed-size blocks.
- Retain only the displayed page, a small block cache, and a bounded cache of
  matching byte offsets.
- `/` searches forward; `?` searches backward; `n` and `N` repeat in each
  direction; `g` and `G` go to start and end.
- Do not use `grep`, shell interpolation, or a full-file index. They conflict
  with the portability and no-external-command requirements.

### Path prompt

Use an Ido-inspired, not Emacs-sized, path selector:

- Start at the active pane's reported working directory.
- List only the current directory; do not scan the filesystem tree.
- Filter candidates as the user types.
- `Tab` completes/cycles matching names; `Enter` enters a directory or accepts
  a file; `Backspace` supports parent navigation.
- Obtain the active pane directory from OSC 7 reports. Do not infer it from a
  shell process: Linux `/proc/<pid>/cwd` is not portable and Windows has no
  reliable equivalent.

Shells that do not emit OSC 7 fall back to the server startup directory and a
user-editable path prompt.
