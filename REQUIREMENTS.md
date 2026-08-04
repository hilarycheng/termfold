# Termfold Requirements

## Authority

This document defines product behaviour. `AGENTS.md` defines the AI workflow.

- **MUST** is required for the milestone named by its section; without a
  milestone qualifier, it is required for the first release.
- **SHOULD** is required unless a documented technical reason prevents it.
- **MAY** is optional.
- An unresolved requirement blocks only the affected feature, not unrelated work.
- Do not invent behaviour where this document is silent; propose the smallest
  decision and wait for `APPROVE`.

## Product Positioning

Termfold is an intentionally small, self-contained terminal multiplexer for
Linux, WSL, and native x86-64 Windows. It provides persistent named sessions,
tabs, panes, and an always-visible one-row status bar without becoming a
terminal workspace platform.

The primary product contract is:

```text
one downloadable executable
+ no required runtime libraries, plugins, helper programs, or system terminfo data
+ persistent local sessions, tabs, panes, colour, and a permanent status row
- no web service, network transport, AI integration, plugin ecosystem, workspace
  model, or sidebar
```

Termfold is not intended to replace every tmux, screen, or Zellij feature. It is
intended for users who need a dependable multiplexer on machines where they
cannot assume that a multiplexer or its runtime dependencies are installed.

The status bar is a safety and context boundary, not decoration. An attached
client MUST always have an unambiguous visible indication that it is inside a
Termfold session, including the session identity and active tab whenever the
terminal is wide enough.

The first release MUST NOT provide:

- runtime executable plugins or a stable plugin ABI;
- a web client or network listener;
- AI-agent awareness or integration;
- Termfold-owned OS clipboard integration, copy mode, or copy/paste buffers;
- arbitrary user-defined key tables beyond the configurable prefix;
- floating panes, sidebars, file browsers, or workspace/project management;
- a layout or scripting language;
- remote access, authentication, encryption, or network transport; or
- restoration after the Termfold server process or host has ceased to exist.

## Distribution and Dependency Contract

The official Linux release artifact MUST be one statically linked executable for
each supported architecture. The official Windows artifact MUST be one
`x86_64-pc-windows-msvc` executable with no bundled runtime DLLs; Windows system
DLLs are platform APIs, not Termfold runtime dependencies.

Termfold itself MUST NOT require:

- dynamically linked runtime libraries;
- an init-system service;
- external helper executables such as `tic`, `tput`, or `infocmp`;
- clipboard helpers such as `xclip` or `wl-copy`;
- runtime-loaded code plugins;
- system-wide configuration files; or
- a system terminfo database in order to start and operate.

The user shell is an external program invoked by Termfold and is not a Termfold
runtime library. Configuration MUST remain optional; a missing configuration
file MUST produce a complete usable default experience.

Build-time source dependencies MAY be used only when they are statically linked,
version-pinned, auditable, and materially reduce correctness risk or maintenance
cost. Adding a dependency MUST document its purpose, licence, binary-size impact,
and why a smaller existing dependency or standard-library implementation is not
sufficient.

## First-Release Scope

The first release MUST provide:

- a self-contained static Linux executable;
- a standalone native x86-64 Windows executable using ConPTY;
- persistent same-user sessions while the server is alive;
- local attach and detach;
- tabs and panes;
- horizontal and vertical splits, focus movement, and resize;
- PTY resize propagation;
- a one-row bottom status bar;
- configurable prefix, date, and time formats;
- configurable status layout, label, and colours with optional built-in Linux
  CPU, memory, and temperature indicators;
- bounded per-pane scrollback, a read-only scroll view, and explicit plain-text
  scrollback export;
- a self-contained inner terminal description;
- safe adaptation to common outer terminal capabilities;
- 16-colour, 256-colour, and true-colour rendering with safe downgrade;
- optional mouse input, disabled by default; and
- deterministic terminal restoration for normal exit, panic, client disconnect,
  and catchable termination signals.

Restoration after `SIGKILL`, kernel failure, power loss, or terminal failure cannot
be guaranteed and MUST NOT be claimed.

The exclusions in Product Positioning remain out of scope.

## Approved Post-First-Release Scope

The following work is approved for later milestones. It does not change or
block first-release acceptance.

### Session termination confirmation

- `termfold kill [NAME]` MUST ask for confirmation before terminating a session.
- Scripts MUST have a documented explicit non-interactive override.
- Cancellation or invalid confirmation input MUST leave the session unchanged.

### Shared renderer optimization

- Linux and Windows MUST continue to use one shared VT renderer.
- Each terminal screen MUST track changed row ranges and render each range with
  one cursor move, sequential text, and only required SGR changes.
- Layout, resize, and screen-buffer changes MAY trigger a full redraw.
- Further renderer branches MUST be justified by separate measurements of
  renderer CPU time, IPC transfer, and outer-terminal output latency.

### Startup profiles

- Startup profiles MUST be declarative session definitions in `config.toml`.
- A profile MUST run only when creating a session, never when attaching.
- A launch target MUST be either the default shell or an absolute executable
  with literal arguments, executed without command interpolation.
- A profile MAY set the session's validated initial directory and define
  multiple tabs with nested horizontal and vertical splits. Each leaf MUST
  define its launch target and MAY override the profile directory.
- Every target and directory MUST be validated before spawning. Any launch
  failure MUST terminate all targets already started for that profile.

### Large-file viewer

#### Scope and entry points

- `Ctrl-b v` MUST prompt for a path relative to the active pane's reported
  working directory and open a read-only viewer pane.
- `Ctrl-b V` MUST use the same prompt and open the viewer in a new tab.
- `termfold view FILE` MUST target the caller's current Termfold session.
  Outside Termfold, the command MUST require an explicit session.
- Linux absolute paths and Windows drive-root paths such as `C:\` and `C:/`
  MUST be accepted.
- The viewer is a bounded local file reader. It MUST NOT invoke `grep`, `less`,
  shell interpolation, or another external helper.
- The T28 viewer is snapshot-oriented. It MUST retain the file identity and
  length captured when the viewer opens. Appends, truncation, replacement, and
  log rotation MUST NOT be followed automatically; reopening the file creates a
  new snapshot.

#### Responsiveness and ownership

- The Session Server MUST NOT perform viewer `seek`, `read`, line scanning,
  page construction, or file search operations.
- Each session MUST own one bounded Viewer Worker responsible for all viewer
  file I/O. It MUST service viewer operations cooperatively so one long search
  cannot make Termfold input unresponsive.
- Search and long line scans MUST advance in steps of at most one 64 KiB source
  block before checking cancellation and newly available control work.
- Every viewer operation MUST carry a generation number. Navigation, a new
  search, mode switching, resize, and viewer close MUST invalidate obsolete
  operations by increasing that generation.
- Results with an obsolete generation MUST be discarded without changing the
  committed page, cursor, status, or active search.
- `Ctrl-b x` MUST remain responsive while viewer I/O is in progress. Closing a
  viewer MUST invalidate its work, clear its pending replacement command, and
  MUST NOT wait for an obsolete result before closing the pane.
- A read or decode failure MUST preserve the last committed page and report a
  short actionable error.

#### Raw block cache and page frames

- File access MUST use standard-library seek/read operations over 64 KiB aligned
  blocks.
- A viewer MUST retain at most eight raw blocks, for a maximum raw block cache of
  512 KiB. Eviction MUST be deterministic and MUST NOT invalidate a committed
  page.
- A viewer MUST retain at most three logical page frames:
  `Previous`, `Current`, and `Next`.
- `Current` is the only displayed frame. `Previous` and `Next` are optional,
  lazily populated neighbours. No fourth page frame or unbounded page history is
  permitted.
- Each page frame MUST contain only the source range needed for the visible page,
  decoded display rows, line boundaries, source-byte-to-display-cell spans,
  valid cursor stops, and visible search ranges. It MUST NOT duplicate a full
  terminal screen grid.
- Source bytes retained by one page frame MUST be capped at 256 KiB. The three
  frames together MUST retain at most 768 KiB of source bytes. A long line MAY
  use resumable scan state rather than retaining the complete line.
- Resize, tab-width change, text/hex mode change, or snapshot change MUST
  invalidate all three frames. Ordinary Page Up/Page Down MUST rotate the three
  frame roles where a valid neighbour exists.
- Prefetch MAY populate one missing neighbour only after `Current` has been
  committed and only while no input command is waiting.

#### Keyboard navigation and repeat control

- Arrow Up/Down and `j`/`k` MUST move the logical cursor by one file line while
  preserving its preferred display-cell column where possible.
- Home/End and `0`/`$` MUST move to the start/end of the current file line.
  `gg`/`G` and Ctrl-Home/Ctrl-End MUST move to the file start/end.
- Page Up/Page Down and Ctrl-b/Ctrl-f MUST move by one page, where a page is the
  visible viewer height minus two rows. Ctrl-u/Ctrl-d MUST move by half a page.
  Ctrl-y/Ctrl-e MUST scroll the viewport by one line without moving the logical
  cursor.
- Every accepted Page Up/Page Down command MUST produce and commit exactly one
  visible intermediate page. Page commands MUST NOT be coalesced into the latest
  position and MUST NOT skip intermediate pages.
- Terminal input does not provide a portable key-release event. The viewer MUST
  therefore use a zero-repeat-backlog gate:
  - at most one navigation command may be in flight;
  - a repeated command with the same navigation intent received while that
    command is in flight MUST be dropped rather than queued;
  - releasing the key produces no new repeats, so navigation stops after the
    current command commits;
  - a changed intent MAY replace one pending command, but at most one replacement
    command may exist;
  - the newest changed intent replaces the older replacement;
  - close, cancel, and mode-switch commands MUST not wait behind repeat input.
- The server MUST commit and render the completed page before dispatching the one
  permitted replacement command.

#### Universal line endings

- Line boundaries MUST be detected from the file contents, not from the host
  operating system or filename.
- Each encountered `CRLF`, `LF`, or lone `CR` sequence MUST terminate one line.
  Mixed line-ending files MUST be handled line by line.
- `CRLF` MUST be treated as one two-byte terminator, never as two empty lines.
- EOL bytes MUST never be cursor positions, searchable text display cells, or
  part of the `$` destination.
- An unterminated final line MUST remain a valid line.
- Line scanning MUST work when `CRLF`, UTF-8, or a search match crosses a raw
  block boundary.

#### Text decoding and cursor model

- Raw file bytes MUST NOT be passed directly to the terminal parser.
- Printable valid UTF-8 MUST be rendered as Unicode using terminal display-cell
  width. A cursor position MUST reference both a source byte offset and a display
  cell column.
- The cursor MUST stop only at the start of a decoded display token. It MUST NOT
  stop inside a UTF-8 sequence, on a combining continuation, on a wide-cell
  continuation, or on an EOL byte.
- Up/Down preferred-column movement and horizontal scrolling MUST use display
  cells, not byte counts.
- `$` MUST stop at the first byte of the final visible token on a non-empty line.
  On an empty line it MUST stop at column zero.
- Text mode MUST render non-printable data safely:
  - NUL as `^@`;
  - ESC as `^[`;
  - DEL as `^?`;
  - other ASCII C0 controls as their caret notation;
  - every remaining invalid or non-printable source byte as uppercase `<XX>`.
- Tab characters MUST expand to the next configured tab stop. The default tab
  width is eight display cells.
- Search highlighting, horizontal clipping, and cursor placement MUST use the
  same source-byte-to-display-cell mapping produced by the text decoder.

#### Text search

- `/` MUST start forward search and `?` reverse search. `n` MUST repeat in the
  same direction and `N` in the opposite direction.
- Text search MUST be literal and simple case-insensitive: ASCII letters MUST
  compare without case; non-ASCII UTF-8 and invalid bytes MUST compare exactly.
  Regex, smart-case, locale-sensitive folding, and full Unicode case folding are
  out of scope.
- A search query MUST contain at most 256 bytes.
- Search MUST examine the visible `Current` frame from the cursor first, then the
  nearby frame in the requested direction, then continue through the snapshot.
- Forward search reaching EOF and reverse search reaching BOF MUST wrap exactly
  once. A search MUST stop after returning to its original start position.
- Full-file indexing is prohibited. A search step MUST inspect at most 64 KiB
  before checking its generation and yielding to control work.
- Any navigation command, new search, text/hex mode change, resize, or viewer
  close MUST cancel an unfinished search.
- The last successful query MUST remain available to `n` and `N` until replaced.
- All visible matches on `Current` MUST be highlighted. The active match MUST be
  distinct from other matches using inverse plus underline. Highlighting MUST
  not rely on colour alone and MUST not trigger a full-file scan.
- A wrapped successful search SHOULD report `wrapped` in the temporary status.

#### Hex mode

- `H` MUST toggle the active viewer between Text and Hex mode without reopening
  the file. Both modes MUST share the same FileSource and raw block cache.
- Hex mode MUST display an absolute hexadecimal byte offset, hexadecimal byte
  values, and printable ASCII side by side.
- ASCII bytes `0x20` through `0x7e` MUST appear literally in the ASCII column;
  all other bytes MUST appear as `.`.
- The bytes per row MUST adapt to width:
  - 16 bytes at 80 columns or wider;
  - 8 bytes at 48 through 79 columns;
  - 4 bytes at 28 through 47 columns;
  - below 28 columns, display `terminal too narrow for hex view` without losing
    the current byte position.
- Hex-mode cursor movement MUST operate on source bytes. Page, line, start/end,
  top/bottom, and horizontal movement MUST remain bounded and deterministic.
- Normal search input in Hex mode MUST perform ASCII case-insensitive byte
  search. A query beginning with `hex:` MUST parse space-separated two-digit
  hexadecimal bytes, for example `hex:00 FF 1B`, and search those bytes exactly.
- Empty hex input, odd nibbles, non-hex characters, or values wider than one byte
  MUST be rejected without changing the previous successful query.
- Hex matches MAY cross display rows and raw block boundaries.

#### Path prompt and closing

- The path prompt MUST list only the current directory, filter as the user types,
  cycle or complete matches with `Tab`, enter directories or accept a file with
  `Enter`, and support parent navigation with `Backspace`.
- The active directory MUST come from OSC 7 when available. Otherwise the prompt
  MUST fall back to the server startup directory and remain user-editable.
- `Ctrl-b x` MUST close the viewer pane after confirmation. `q` and Esc MUST NOT
  close it.
## Command-Line Contract

The initial CLI MUST support:

```text
termfold                       Attach the only detached session, or list sessions
termfold PID_PREFIX            Attach the uniquely matching detached session
termfold new [NAME]            Create and attach to a session
termfold attach [NAME]         Attach to an existing session
termfold list                  List sessions
termfold kill [NAME]           Terminate a session
termfold view FILE [--session NAME]
                               Open FILE in the current or explicit session
termfold diagnose              Report terminal and compatibility decisions
termfold --help
termfold --version
```

`NAME` defaults to `default` where applicable. Session names MUST match
`[A-Za-z0-9_-]{1,64}`. Each session MUST run in its own Termfold server process.
With no arguments, Termfold MUST create `default` when no session exists, attach
when exactly one detached session exists, and otherwise list every session with
its process ID and attached or detached state. A decimal `PID_PREFIX` MUST attach
only when it uniquely matches a detached Termfold session process. An unknown or
ambiguous prefix MUST NOT attach and MUST instead list the Termfold process IDs.
Invalid commands or names MUST return a non-zero status and a short actionable
error.

Each user MUST have an independent session namespace and MAY run up to the
configured concurrent-session limit. Different users MAY use the same session
name, but MUST NOT discover or attach to each other's sessions. Multiple clients
owned by the same user MUST be able to attach to one session concurrently.
Attaching a client MUST NOT detach or interrupt existing clients. Every attached
client MUST receive display updates and MAY send input and commands; resulting
session, tab, pane, and focus changes are shared by all attached clients.

## Session and Process Lifecycle

- A client MUST start one dedicated server process for each created session.
- Detaching MUST leave the session and child processes running.
- A session server MUST exit when its session is terminated.
- The most recently active attached client's current size is authoritative and
  MUST be propagated to every affected PTY on attach and input, plus
  `SIGWINCH` on Linux or console-size changes on Windows.
- A pane child exit MUST close that pane. An empty tab MUST close; an empty session
  MUST terminate.
- Closing a live pane or session MUST request graceful child termination before
  forced termination and MUST reap every child. On Linux, send `SIGTERM`, wait
  up to 2 seconds, then send `SIGKILL`. On Windows, close the ConPTY, wait up to
  2 seconds, then terminate the pane job object.
- The server MUST never listen on a network socket.


### Executable updates and active sessions

- A running session server owns its PTYs or ConPTY instances and MUST NOT be
  described as live-upgradable.
- Replacing or restarting the client executable MUST NOT imply that an existing
  session server has been upgraded.
- A newer client MAY attach to an older running server only when their IPC
  protocol versions are compatible.
- On Windows, an executable held open by an active process cannot be replaced in
  place; users MUST end the affected Termfold processes before replacing it.
- PTY/session handover between server processes is outside the approved scope.

## Shell Launch and Inner Terminal Identity

- On Linux, use `$SHELL` only when it is an absolute executable path; otherwise
  use `/bin/sh`.
- On Windows, use the configured `windows_shell` executable and arguments when
  present. Otherwise use absolute `%COMSPEC%`, falling back to
  `%SystemRoot%\System32\cmd.exe`.
- Execute the shell directly without command interpolation.
- The first pane MUST inherit the creating client's working directory and
  environment, except for Termfold-controlled terminal variables.
- New panes and tabs MUST inherit the session's initial working directory.
- Inner applications MUST receive a stable terminal identity that does not
  change when clients attach from different outer terminals.

The default inner environment MUST be:

```text
TERM=termfold-256color
COLORTERM=truecolor
TERMINFO=<validated per-user Termfold runtime terminfo root>
```

Termfold MUST ship a checked-in source description for `termfold-256color` and a
compiled form embedded in the release binary. At runtime Termfold MUST materialize
that entry inside its validated user-owned runtime directory before launching the
first pane. It MUST NOT invoke `tic` or require a system installation step.

The embedded entry MUST describe only capabilities that the Termfold parser and
renderer actually implement. A release MUST NOT advertise a capability merely
because common xterm-compatible terminals support it.

Failure to create or validate the private terminfo entry MUST fail session
creation with an actionable error; Termfold MUST NOT silently advertise an
unknown or unsupported terminal identity.

A documented compatibility override MAY allow `TERM=xterm-256color` for a known
application that rejects custom terminal names, but this mode MUST be explicit,
MUST warn that it broadens the advertised contract, and MUST remain covered by
compatibility tests.

## Default Keys

The default prefix is `Ctrl-b`. Outside prefix mode, bytes MUST be forwarded
unchanged to the active application. After a prefix:

| Key | Action |
| --- | --- |
| `Ctrl-b` | Send a literal `Ctrl-b` |
| `c` | Create tab |
| `n` / `p` | Next / previous tab |
| `0`-`9` | Select tab |
| `|` | Split left/right |
| `-` | Split top/bottom |
| Arrow | Focus adjacent pane |
| `Ctrl`+Arrow | Resize by one cell |
| `r` | Enter resize mode |
| `x` | Close active pane after confirmation |
| `d` | Detach |
| `?` | Show the key-reminder help view |
| `[` | Enter read-only scroll view |
| `v` | Prompt for and open a bounded read-only file viewer pane |
| `V` | Prompt for and open a bounded read-only file viewer in a new tab |
| `C` | Clear the active pane's entire retained scrollback |
| `S` | Save the active pane's retained scrollback as plain text after prompting for a filename |

An unsupported prefix command MUST show a short status message and MUST NOT be
forwarded. Keyboard-only operation MUST remain complete when mouse support is
enabled.

In resize mode, each Arrow key MUST resize by one cell without another prefix.
`Esc` MUST leave resize mode.

The help view MUST show each key combination with its meaning, render the
configured prefix rather than assuming `Ctrl-b`, retain the bottom status row,
and support line and page navigation. Its status reminder MUST adapt to the
available width. `q`, `Ctrl-c`, and `Esc` MUST leave the help view.

## Tabs and Panes

- A tab MUST contain at least one pane while it exists.
- A split MUST preserve the active pane's process and create one new pane.
- A split that cannot give both panes at least one content row and column MUST
  fail without changing the layout.
- Focus and resize MUST operate on the nearest pane in the requested direction.
- Closing the active pane MUST focus the nearest surviving pane deterministically.
- Pane borders SHOULD use box-drawing characters when the outer-terminal profile
  supports them and MUST use an ASCII fallback otherwise. Unicode MUST remain
  supported in application content.

## Terminal Architecture

Termfold MUST treat terminal compatibility as two separate contracts:

```text
inner application -> Termfold virtual terminal -> attached outer terminal
```

The inner contract is the fixed `termfold-256color` behaviour implemented by the
terminal parser and cell model. The outer contract is a capability-adapted
renderer selected independently for each attached client.

Terminal profiles MUST NOT be allowed to alter parser semantics, the cell model,
UTF-8 decoding, control-sequence framing, scroll-region behaviour, or alternate
screen semantics. Those behaviours belong to one testable core implementation.

## Inner Terminal Behaviour

Because Termfold advertises `termfold-256color`, its checked-in terminfo entry and
implementation MUST agree on the subset used by ordinary interactive Linux
applications and by Windows console applications translated through ConPTY:

- incremental UTF-8 decoding, combining characters, and wide-cell accounting;
- cursor movement, save/restore, scrolling regions, insertion, deletion, erase,
  wrapping, tabs, and SGR attributes;
- 16-colour, 256-colour, and true-colour SGR;
- normal and alternate screen buffers;
- application cursor keys, bracketed paste, cursor visibility, and PTY resize;
- standard xterm-compatible mouse modes required by the mouse contract; and
- safe skipping of unsupported CSI, OSC, and DCS sequences without parser loss.

OSC 52 clipboard writes MUST be ignored by default. A control sequence longer
than 4096 bytes MUST be discarded safely. Pasted input MUST use bracketed-paste
markers only when the active application enabled that mode.

Termfold MUST maintain a capability-to-test mapping for the embedded terminfo
entry. Every advertised string, numeric, or boolean capability that changes
runtime behaviour MUST have a focused automated or interactive acceptance check.

## Outer Terminal Capabilities and Profiles

Terminfo is a capability description, not only a list of escape sequences. The
outer compatibility layer MUST account for string capabilities, numeric limits,
boolean behaviour, and known terminal quirks.

Termfold MUST be able to start and render without a system terminfo database. It
MAY read system terminfo data when safely available, but MUST NOT require ncurses,
link to a terminfo library, or execute `tput`, `infocmp`, or `tic` at runtime.

The release binary MUST contain data-only profiles for at least these families:

```text
dumb
ansi
vt100
linux
xterm
xterm-256color
screen
screen-256color
tmux
tmux-256color
```

Common aliases such as Kitty, Foot, Alacritty, and WezTerm SHOULD resolve through
an audited xterm-compatible family profile unless a documented quirk requires a
specific override.

Profile selection precedence MUST be:

1. an explicit validated configuration override;
2. an exact built-in terminal name or alias;
3. an audited family match;
4. a conservative ANSI fallback.

`TERM=dumb` or a terminal without the cursor-addressing capabilities required for
a full-screen interface MUST reject attach with an actionable error.

`COLORTERM=truecolor` or `COLORTERM=24bit` MAY be used as a positive colour hint,
but MUST NOT override a known incompatible terminal profile. Unknown environment
variables or terminal brand names MUST NOT automatically enable advanced modes.

Terminal-specific compatibility extensions MUST be data-only profiles compiled
into the binary. The first release MUST NOT load executable terminal plugins,
dynamic libraries, scripts, or profile files from untrusted paths. Contributors
MAY add or correct built-in profiles through normal source changes and focused
compatibility tests.

## Colour and Attribute Adaptation

Termfold MUST preserve the application's logical colour and attribute state in
its virtual terminal. Rendering to each attached client MUST safely degrade:

```text
true colour -> 256 colours -> 16 colours -> monochrome attributes
```

The downgrade MUST be deterministic. Unsupported attributes such as italic or
dim MUST be omitted or mapped conservatively without corrupting later terminal
state. Default foreground and background colours MUST remain distinguishable from
explicit palette colours.

Termfold-owned UI, including the status bar, MAY use true colour internally but
MUST follow the same downgrade rules. Information MUST NOT be communicated by
colour alone.

## Terminal Diagnostics

`termfold diagnose` MUST report enough information to reproduce a compatibility
problem without exposing secrets. At minimum it MUST show:

```text
outer TERM
outer COLORTERM
selected outer profile and match reason
selected colour level
mouse and alternate-screen support
inner TERM value
private TERMINFO path and validation result
terminal rows and columns
Termfold version and target architecture
```

The command MUST NOT print arbitrary environment values, socket contents, or
user input. A machine-readable output mode MAY be added later but is
not required for the first release.

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

The scroll view MUST allow the user to navigate the active pane's retained
history without providing text selection, copy commands, paste commands, or
tmux-style copy/paste buffers. Termfold MUST render that history as ordinary
terminal cells so the outer terminal can provide its native text selection and
OS clipboard behaviour.

The scroll view MUST support Arrow and `j`/`k` line movement, Page Up and Page
Down, `g`/`G` movement to the oldest/newest position, and bounded case-sensitive
literal search with `/`, `n`, and `N`. `q`, `Ctrl-c`, and `Esc` MUST leave the
scroll view. The status row MUST show the scroll position and an adaptive key
reminder: detailed when width permits and minimal on narrow terminals.

Termfold MUST NOT invoke OS clipboard helpers or implement its own OS clipboard
integration. Paste input received from the outer terminal MUST be forwarded to
the active application, using bracketed-paste markers only when that application
has enabled bracketed paste.

`Ctrl-b S` MUST prompt for a filename and save the active pane's retained
scrollback as UTF-8 plain text. Cancelling the prompt MUST NOT create or modify a
file. Export MUST be user-triggered rather than continuous logging and MUST omit
terminal control sequences and styling.

`Ctrl-b C` MUST remove the active pane's entire retained scrollback without
changing its visible screen. Every attached client's scroll view MUST return to
the live pane after the history is cleared.

## Status Bar

The default layout is:

```text
[session]  [1:shell]  2:logs  3:db  |  2026-07-19 18:42
```

- The status row MUST remain visible during ordinary attached operation and MUST
  make it unambiguous that the client is inside Termfold.
- Brackets and terminal attributes MUST distinguish the active tab; colour alone
  is insufficient.
- The active tab and clock MUST remain visible when width permits.
- Inactive tabs furthest from the active tab MUST be removed first when space is
  insufficient. `<` and `>` MUST indicate omitted tabs.
- At extremely narrow widths, show active tab, then time, then session in that
  priority order.
- Temporary errors and unsupported-prefix messages MAY replace non-essential
  status content but MUST NOT hide the session and active-tab identity when width
  permits.
- Only the status row MUST be redrawn for a clock-only update.
- Minute-resolution formats update once per minute; formats containing seconds
  update once per second.
- `status_format` MUST support literal UTF-8 text and the placeholders
  `{session}`, `{tabs}`, `{fill}`, `{label}`, `{cpu_usage}`, `{memory_usage}`,
  `{cpu_temp}`, `{date}`, and `{time}`. `{fill}` MUST consume the remaining
  columns so following content is right aligned.
- A configured format MUST contain `{session}`, `{tabs}`, `{date}`, `{time}`, and
  exactly one `{fill}`. Control characters and unknown placeholders MUST fail
  startup.
- Status labels and the base, label, and active-tab foreground/background
  colours MUST be configurable. Active tabs MUST retain brackets and terminal
  attributes even when custom colours are used.
- Status colours MUST accept `default`, the 16 ANSI colour names, and `#RRGGBB`;
  they MUST use the existing safe colour downgrade path.
- On Linux, `{cpu_usage}` MUST be sampled from `/proc/stat`, `{memory_usage}`
  from `MemTotal` and `MemAvailable` in `/proc/meminfo`, and `{cpu_temp}` from
  the explicitly configured sysfs file.
- On Windows, `{cpu_usage}` and `{memory_usage}` MUST use Win32 system metrics.
  `{cpu_temp}` MAY remain unavailable and render as `-`.
- Missing metric values MUST render as `-`.
- System metrics MUST refresh only when their placeholders are configured.
  Metric-only updates MUST redraw only the status row.

## Configuration

On Linux, read configuration from `$XDG_CONFIG_HOME/termfold/config.toml`,
falling back to `$HOME/.config/termfold/config.toml`. On Windows, read
`%APPDATA%\Termfold\config.toml`. A missing file MUST use these defaults:

```toml
prefix = "C-b"
mouse = false
scrollback_lines = 2000
viewer_tab_width = 8
date_format = "%Y-%m-%d"
time_format = "%H:%M"
status_format = "[{session}]  {tabs}{fill}|  {date} {time}"
status_label = ""
status_theme = "default"
status_refresh_seconds = 2
status_foreground = "black"
status_background = "cyan"
label_foreground = "bright-white"
label_background = "red"
active_tab_foreground = "black"
active_tab_background = "bright-yellow"
terminal_profile = "auto"
inner_term = "termfold-256color"
# Windows only; omitted by default:
# windows_shell = ["C:\\msys64\\usr\\bin\\bash.exe", "--login"]
```

Unknown fields, invalid key syntax, invalid time formats, unknown terminal
profiles, unsupported inner terminal values, and out-of-range values MUST identify
the exact field and fail startup. Termfold MUST never rewrite the configuration
file automatically.

- `prefix` MUST be one ASCII control key from `C-a` through `C-z`.
- `scrollback_lines` MUST be between 0 and 10,000 inclusive.
- `viewer_tab_width` MUST be between 1 and 16 inclusive and defaults to 8.
- `date_format` and `time_format` MUST each contain at most 64 characters and
  support only `%Y`, `%m`, `%d`, `%H`, `%I`, `%M`, `%S`, `%p`, and `%%` directives.
- `status_format` MUST contain at most 512 characters and follow the Status Bar
  placeholder rules.
- `status_label` MUST contain at most 64 characters and no control characters.
- `status_theme` MUST be `default`, `catppuccin-latte`, `catppuccin-mocha`,
  `solarized-light`, `solarized-dark`, `gruvbox-light`, `gruvbox-dark`,
  `tokyo-night-day`, `tokyo-night`, `dracula`, or `nord`. Individual status
  colour fields MUST override the selected theme regardless of field order.
- `status_refresh_seconds` MUST be between 1 and 3,600 inclusive.
- `cpu_temperature_path`, when set, MUST be an absolute path below `/sys` without
  parent-directory components.
- `terminal_profile` MUST be `auto` or the exact name of a built-in profile.
- `inner_term` MUST be `termfold-256color` or the explicitly supported
  compatibility value `xterm-256color`.
- `windows_shell`, when set, MUST be a non-empty array containing an absolute
  executable path followed by zero or more literal arguments. Termfold MUST
  execute it directly without command interpolation. Changes apply only to new
  sessions.

Built-in themes MUST be embedded and MUST NOT require runtime downloads. They
apply only to Termfold-owned UI and MUST use the normal outer-terminal colour
downgrade path; Termfold MUST NOT claim to recolour pane applications.

## IPC and Filesystem Security

On Linux:

- Prefer `$XDG_RUNTIME_DIR/termfold` only when the runtime directory is absolute,
  owned by the current user, and not writable by other users.
- Otherwise use `/tmp/termfold-UID`, created with mode `0700` and verified as a
  real directory owned by the current user.
- The Unix socket MUST use mode `0600`.
- The private terminfo root MUST live below the validated Termfold runtime
  directory, MUST be owned by the current user, and MUST NOT be writable by other
  users.
- Embedded terminfo extraction MUST use atomic creation and MUST NOT follow
  symlinks or replace a non-regular file.
- Symlinks MUST NOT be followed while creating, validating, or removing runtime
  paths.
- A stale socket MAY be removed only after type and ownership validation and a
  failed connection proving no server accepts it.

On Windows:

- IPC MUST use local named pipes with remote clients rejected.
- Each pipe and named creation lock MUST use a protected DACL granting access
  only to the current user SID.
- Both named-pipe endpoints MUST verify that the peer process has the current
  user SID.
- Session pipe names MUST include the current user SID.
- Runtime markers and private terminfo data MUST live below
  `%LOCALAPPDATA%\Termfold\runtime`, or a user-SID-specific temporary fallback,
  with a protected current-user DACL.
- ConPTY child trees MUST be assigned to a kill-on-close job object.

On every platform:

- IPC MUST be framed, versioned, reject malformed messages, and cap each frame at
  1 MiB.
- Each client connection MUST have independent parsing, queues, and cleanup. A
  malformed message, queue overflow, resize failure, or disconnect from one
  client MUST NOT detach, block, corrupt, or terminate another client or the
  session. Unsent data MUST NOT be silently discarded.
- Session names MUST never be used as unchecked filesystem paths.

## Resource Limits

The first release MUST enforce these hard limits:

| Resource | Limit |
| --- | ---: |
| Concurrent sessions per user | 32 |
| Tabs per session | 32 |
| Panes per tab | 16 |
| IPC frame | 1 MiB |
| Control sequence | 4 KiB |
| Default scrollback per pane | 2,000 lines |
| Viewer raw block | 64 KiB |
| Viewer raw block cache | 8 blocks / 512 KiB |
| Viewer page frames | 3 |
| Viewer source bytes per frame | 256 KiB |
| Viewer search query | 256 bytes |
| Viewer search/line-scan step | 64 KiB |
| Viewer navigation in flight | 1 |
| Viewer changed-intent replacement | 1 |

Each queue MUST hold at most 256 items and 4 MiB of payload. Pending PTY output
MUST be limited to 1 MiB per pane. Reaching a cap MUST pause reads to apply
backpressure; terminal data MUST NOT be silently discarded.

## Implementation Warnings

The following distinctions MUST remain explicit during implementation and review:

- raw PTY passthrough cannot provide a permanent status row, independent panes,
  or deterministic redraw after attach; those features require a virtual terminal
  state model;
- adding a new outer profile cannot repair a missing or incorrect inner parser
  behaviour;
- setting `TERM` is a behavioural promise, not a branding string;
- a terminal family name alone is insufficient evidence for true colour, mouse,
  alternate-screen, or keyboard-mode support;
- generated code that compiles and renders a demo is not accepted without tests
  for partial escape sequences, partial UTF-8, alternate screen, resize, signal
  cleanup, slow clients, and unsupported capability downgrade; and
- AI-generated implementation MUST NOT invent terminal behaviour or broaden the
  embedded terminfo entry beyond approved and tested requirements.
- file-byte decoding and terminal escape-sequence parsing are separate contracts;
  raw viewer bytes MUST never be interpreted as terminal control sequences;
- viewer byte offsets, decoded tokens, and terminal display cells MUST remain
  distinct types or explicitly mapped values; and
- key auto-repeat MUST be controlled by zero backlog rather than an assumed
  portable key-release event.

## Prior Art and Acknowledgements

Project documentation MUST include a prior-art section crediting at least:

- `zmx` for the small self-contained session-wrapper approach;
- `tmux` for established session, tab/window, pane, prefix, and status-line
  interaction conventions; and
- xterm and ncurses terminfo documentation for terminal protocol and capability
  references.

Credits MUST describe inspiration accurately and MUST NOT imply endorsement,
affiliation, code reuse, or compatibility certification where none exists.

## Implementation and Acceptance

- Start with the fewest modules that provide clear ownership; the module list in
  `AGENTS.md` is guidance, not required scaffolding.
- Reuse the standard library and approved existing dependencies before adding code
  or a dependency.
- Each approved change MUST identify the requirements it affects.
- Each non-trivial behaviour MUST have one focused runnable check after separate
  in-scope `APPROVE` for tests.
- Terminal profile changes MUST include a reproduction case and expected output.
- Changes to the embedded `termfold-256color` description MUST include a matching
  implementation or test change and MUST be reviewed as a public compatibility
  contract.
- Linux or WSL is authoritative for Linux builds, PTYs, signals, permissions,
  static linking, and terminal restoration.
- Native Windows is authoritative for ConPTY, named-pipe ACLs, console modes,
  job-object cleanup, and the `x86_64-pc-windows-msvc` artifact.
- A release is not acceptable until the release checklist in `AGENTS.md` passes.

The first release MUST meet these owner-approved budgets:

| Measure | Budget |
| --- | ---: |
| Stripped release binary | 5 MiB maximum |
| Startup latency | 100 ms maximum |
| Idle resident memory | 16 MiB maximum |
| Idle CPU usage | 0.1% maximum |
| Minimum Linux kernel | 4.18 |
| Minimum Windows version | Windows 10 version 1809 |
