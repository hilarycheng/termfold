# Termfold Requirements

## Authority

This document defines product behaviour. `AGENTS.md` defines the AI workflow.

- **MUST** is required for the first release.
- **SHOULD** is required unless a documented technical reason prevents it.
- **MAY** is optional.
- An unresolved requirement blocks only the affected feature, not unrelated work.
- Do not invent behaviour where this document is silent; propose the smallest
  decision and wait for `APPROVE`.

## First-Release Scope

The first release MUST provide:

- persistent same-user sessions while the server is alive;
- attach and detach;
- tabs and panes;
- horizontal and vertical splits, focus movement, and resize;
- PTY resize propagation;
- a one-row bottom status bar;
- configurable prefix, date, and time formats;
- bounded scrollback;
- optional mouse input, disabled by default; and
- deterministic terminal restoration for normal exit, panic, client disconnect,
  and catchable termination signals.

Restoration after `SIGKILL`, kernel failure, power loss, or terminal failure cannot
be guaranteed and MUST NOT be claimed.

The non-goals in `AGENTS.md` remain out of scope.

## Command-Line Contract

The initial CLI MUST support:

```text
termfold                       Attach to "default", creating it if absent
termfold new [NAME]            Create and attach to a session
termfold attach [NAME]         Attach to an existing session
termfold list                  List sessions
termfold kill [NAME]           Terminate a session
termfold --help
termfold --version
```

`NAME` defaults to `default` where applicable. Session names MUST match
`[A-Za-z0-9_-]{1,64}`. Invalid commands or names MUST return a non-zero status and
a short actionable error. A second attached client to the same session MUST fail;
multi-client display sharing is not part of the first release.

## Session and Process Lifecycle

- A client MUST start the per-user server when creation requires one.
- Detaching MUST leave the session and child processes running.
- The server MUST exit after its last session is terminated.
- The attached client's current size is authoritative and MUST be propagated to
  every affected PTY on attach and `SIGWINCH`.
- A pane child exit MUST close that pane. An empty tab MUST close; an empty session
  MUST terminate.
- Closing a live pane or session MUST request graceful child termination before
  forced termination and MUST reap every child.
- The server MUST never listen on a network socket.

## Shell Launch

- Use `$SHELL` only when it is an absolute executable path; otherwise use `/bin/sh`.
- Execute the shell directly without command interpolation.
- The first pane MUST inherit the creating client's working directory and
  environment, except for Termfold-controlled terminal variables.
- Inner applications MUST receive `TERM=xterm-256color` and
  `COLORTERM=truecolor`.
- New panes and tabs MUST inherit the session's initial working directory.

## Default Keys

The default prefix is `Ctrl-b`. Outside prefix mode, bytes MUST be forwarded
unchanged to the active application. After a prefix:

| Key | Action |
| --- | --- |
| `Ctrl-b` | Send a literal `Ctrl-b` |
| `c` | Create tab |
| `n` / `p` | Next / previous tab |
| `0`-`9` | Select tab |
| `%` | Split left/right |
| `"` | Split top/bottom |
| Arrow | Focus adjacent pane |
| `Ctrl`+Arrow | Resize by one cell |
| `x` | Close active pane after confirmation |
| `d` | Detach |
| `[` | Enter scrollback mode |

An unsupported prefix command MUST show a short status message and MUST NOT be
forwarded. Keyboard-only operation MUST remain complete when mouse support is
enabled.

## Tabs and Panes

- A tab MUST contain at least one pane while it exists.
- A split MUST preserve the active pane's process and create one new pane.
- A split that cannot give both panes at least one content row and column MUST
  fail without changing the layout.
- Focus and resize MUST operate on the nearest pane in the requested direction.
- Closing the active pane MUST focus the nearest surviving pane deterministically.
- Pane borders MUST use an ASCII fallback. Unicode MUST remain supported in
  application content.

## Terminal Behaviour

Termfold advertises `xterm-256color`; therefore it MUST implement the subset used
by ordinary interactive Linux applications:

- incremental UTF-8 decoding, combining characters, and wide-cell accounting;
- cursor movement, save/restore, scrolling regions, insertion, deletion, erase,
  wrapping, tabs, and SGR attributes;
- 16-colour, 256-colour, and true-colour SGR;
- normal and alternate screen buffers;
- application cursor keys, bracketed paste, cursor visibility, and PTY resize;
- standard xterm mouse modes required by the mouse contract; and
- safe skipping of unsupported CSI, OSC, and DCS sequences without parser loss.

OSC 52 clipboard writes MUST be ignored by default. A control sequence longer
than 4096 bytes MUST be discarded safely. Pasted input MUST use bracketed-paste
markers only when the active application enabled that mode.

## Mouse and Scrollback

Mouse support is required but MUST default to disabled. When enabled, it MUST:

- use SGR reporting;
- select panes and tabs on click;
- resize at pane borders by drag;
- scroll Termfold history by wheel; and
- forward events when the active application has enabled mouse reporting.

All enabled outer-terminal mouse modes MUST be disabled during detach and cleanup.
Each pane MUST retain at most 2,000 scrollback lines by default. The limit MUST be
configurable and MUST discard the oldest complete lines first.

## Status Bar

The default layout is:

```text
[session]  [1:shell]  2:logs  3:db  |  2026-07-19 18:42
```

- Brackets and terminal attributes MUST distinguish the active tab; colour alone
  is insufficient.
- The active tab and clock MUST remain visible when width permits.
- Inactive tabs furthest from the active tab MUST be removed first when space is
  insufficient. `<` and `>` MUST indicate omitted tabs.
- At extremely narrow widths, show active tab, then time, then session in that
  priority order.
- Only the status row MUST be redrawn for a clock-only update.
- Minute-resolution formats update once per minute; formats containing seconds
  update once per second.

## Configuration

Read configuration from `$XDG_CONFIG_HOME/termfold/config.toml`, falling back to
`$HOME/.config/termfold/config.toml`. A missing file MUST use these defaults:

```toml
prefix = "C-b"
mouse = false
scrollback_lines = 2000
date_format = "%Y-%m-%d"
time_format = "%H:%M"
```

Unknown fields, invalid key syntax, invalid time formats, and out-of-range values
MUST identify the exact field and fail startup. Termfold MUST never rewrite the
configuration file automatically.

## IPC and Filesystem Security

- Prefer `$XDG_RUNTIME_DIR/termfold` only when the runtime directory is absolute,
  owned by the current user, and not writable by other users.
- Otherwise use `/tmp/termfold-UID`, created with mode `0700` and verified as a
  real directory owned by the current user.
- The Unix socket MUST use mode `0600`.
- Symlinks MUST NOT be followed while creating, validating, or removing runtime
  paths.
- A stale socket MAY be removed only after type and ownership validation and a
  failed connection proving no server accepts it.
- IPC MUST be framed, versioned, reject malformed messages, and cap each frame at
  1 MiB.
- Session names MUST never be used as unchecked filesystem paths.

## Resource Limits

The first release MUST enforce these hard limits:

| Resource | Limit |
| --- | ---: |
| Sessions per server | 32 |
| Tabs per session | 32 |
| Panes per tab | 16 |
| IPC frame | 1 MiB |
| Control sequence | 4 KiB |
| Default scrollback per pane | 2,000 lines |

Queues and pending pane output MUST also be bounded. Their exact caps MAY follow
the chosen event-loop design but MUST be documented before that implementation is
approved.

## Implementation and Acceptance

- Start with the fewest modules that provide clear ownership; the module list in
  `AGENTS.md` is guidance, not required scaffolding.
- Reuse the standard library and existing dependencies before adding code or a
  dependency.
- Each approved change MUST identify the requirements it affects.
- Each non-trivial behaviour MUST have one focused runnable check after separate
  in-scope `APPROVE` for tests.
- Linux or WSL is authoritative for builds, PTYs, signals, permissions, and
  terminal restoration.
- A release is not acceptable until the release checklist in `AGENTS.md` passes.

Binary size, startup latency, idle memory, idle CPU, and minimum Linux kernel need
owner-approved numeric budgets before the first release. Until then, report actual
measurements without claiming those goals are satisfied.
