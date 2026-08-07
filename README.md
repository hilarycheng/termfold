# Termfold

Termfold is a small terminal multiplexer for Linux, WSL, and native x86-64
Windows. It provides
persistent local sessions, tabs, panes, scrollback, and a compact bottom status
bar in one self-contained executable.

Termfold is intentionally narrower than tmux: it has no plugins, scripting
language, network listener, or built-in clipboard. Sessions persist while their
Termfold server process remains alive.

## Platform support

Termfold runs on Linux, including:

- WSL and Linux over SSH
- WezTerm, xterm, Kitty, and Windows Terminal through WSL or SSH
- Recent Debian, Ubuntu, Alpine, and RHEL-compatible systems

Native Windows 10 version 1809 or later is supported through ConPTY in Windows
Terminal, WezTerm, and Windows Command Prompt. macOS is not supported.

## Build and install

Termfold requires stable Rust. Build the static x86-64 Linux binary from the
repository:

```bash
cargo build --release --locked --target x86_64-unknown-linux-musl
```

The binary is written to:

```text
target/x86_64-unknown-linux-musl/release/termfold
```

Copy it to a directory on your `PATH`, for example:

```bash
install -Dm755 target/x86_64-unknown-linux-musl/release/termfold \
  "$HOME/.local/bin/termfold"
```

Build the native Windows executable with the MSVC toolchain:

```text
cargo build --release --locked --target x86_64-pc-windows-msvc
```

The executable is written to
`target\x86_64-pc-windows-msvc\release\termfold.exe`.

The Windows-only `windows-sys` dependency provides direct Win32 bindings for
ConPTY, named pipes, ACLs, console control, and job objects; Rust's standard
library does not expose those APIs. It is MIT/Apache-2.0 licensed and adds no
Windows runtime library beyond system DLLs. The validated Windows release
artifact is 433,664 bytes; the dependency-specific incremental size is not
separable because no native Windows baseline existed.

## Quick start

Create and attach to a named session:

```bash
termfold new work
```

Detach with `Ctrl-b`, then `d`. Reattach later:

```bash
termfold attach work
```

Running `termfold` without arguments:

- creates and attaches to `default` when no session exists;
- attaches when exactly one detached session exists; or
- lists sessions when the choice is ambiguous.

## Commands

```text
termfold                       Select, create, or list as described above
termfold PID_PREFIX            Attach to one matching detached session
termfold new [NAME]            Create and attach to a session
termfold attach [NAME]         Attach to an existing session
termfold list                  List sessions
termfold kill [--yes] [NAME]   Terminate a session after confirmation
termfold view FILE [--session NAME]
                               Open FILE in a read-only viewer pane
termfold diagnose              Show terminal compatibility decisions
termfold --help                Show command usage
termfold --version             Show the installed version
```

`NAME` defaults to `default` where applicable and may contain 1–64 ASCII
letters, digits, underscores, or hyphens.

`termfold kill [NAME]` prompts before termination. The default answer is `no`;
only the exact answer `yes` terminates the session. `no`, `Esc`, end-of-file,
and invalid input cancel without changing the session. Scripts may use the
explicit non-interactive override `termfold kill --yes [NAME]`.

`termfold list` prints each session as:

```text
PID NAME attached|detached
```

Multiple clients owned by the same user may attach to the same session. Changes
to tabs, panes, and focus are shared between attached clients.

`termfold view FILE` opens a bounded read-only viewer in the current Termfold
session. When run outside a Termfold shell, pass `--session NAME`.

## Keyboard shortcuts

Termfold uses a prefix key so normal keyboard input continues to reach the
active application. The default prefix is `Ctrl-b`.

Shortcut notation such as `Ctrl-b c` means:

1. Press `Ctrl-b`.
2. Release it.
3. Press `c`.

Do not hold `Ctrl-b` while pressing the command key.

| Shortcut | Action |
| --- | --- |
| `Ctrl-b Ctrl-b` | Send a literal `Ctrl-b` to the active application |
| `Ctrl-b c` | Create a tab |
| `Ctrl-b n` | Select the next tab |
| `Ctrl-b p` | Select the previous tab |
| `Ctrl-b 1` … `Ctrl-b 9` | Select tab 1 through 9 |
| `Ctrl-b 0` | Select tab 10 |
| `Ctrl-b \|` | Split into left and right panes |
| `Ctrl-b -` | Split into top and bottom panes |
| `Ctrl-b Arrow` | Focus the nearest pane in that direction |
| `Ctrl-b Ctrl-Arrow` | Resize the active pane by one cell |
| `Ctrl-b r` | Enter resize mode |
| `Ctrl-b x` | Ask to close the active pane; press `y` to confirm |
| `Ctrl-b d` | Detach this client and leave the session running |
| `Ctrl-b ?` | Show the key-reminder help view |
| `Ctrl-b [` | Enter the active pane's read-only scroll view |
| `Ctrl-b v` | Prompt for a file and open a bounded read-only viewer pane |
| `Ctrl-b V` | Prompt for a file and open the viewer in a new tab |
| `Ctrl-b C` | Clear all retained scrollback for the active pane |
| `Ctrl-b S` | Prompt for a file and save the active pane's scrollback |

For resize shortcuts, press and release `Ctrl-b`, then hold `Ctrl` while pressing
an arrow key. For repeated resizing, press `Ctrl-b r`, use Arrow keys, then press
`Esc` to leave resize mode.

An unknown prefix command is not sent to the application; Termfold reports it in
the status bar.

### Key reminder

After `Ctrl-b ?`, Termfold lists each key combination and its meaning using the
configured prefix. Use `Up`, `Down`, `j`, `k`, Page Up, or Page Down to navigate;
`q`, `Ctrl-c`, or `Esc` exits.

### Scroll view

After `Ctrl-b [`:

| Key | Action |
| --- | --- |
| `Up` or `k` | Scroll up one line |
| `Down` or `j` | Scroll down one line |
| `Page Up` | Scroll up one page |
| `Page Down` | Scroll down one page |
| `g` / `G` | Go to the oldest / newest position |
| `/` | Enter a case-sensitive literal search |
| `n` / `N` | Find the next older / newer match |
| `q`, `Ctrl-c`, or `Esc` | Return to the live pane |

Scroll view is read-only. Use the outer terminal's normal text selection and
clipboard features to copy visible text. Its bottom-row reminder shows more keys
when the terminal is wide and a minimal reminder when it is narrow.

### Large-file viewer

After `Ctrl-b v` or `Ctrl-b V`, the prompt lists entries from the active pane's current
directory. Type to filter, press `Tab` to complete or cycle matches, press
`Enter` to enter a directory or open a file, and press `Backspace` on an empty
filter to move to the parent directory. The first `/` or `\` remains editable
path input; a second separator enters the filesystem root (`//` on Linux, or
the current drive root on Windows), and `~/` enters the current user's Home.
`C:\` and `C:/` enter a named Windows drive root. A literal `~` remains filter
text and is never shell-expanded. Invalid selections show a short error and
keep the prompt active; `Esc` or `Ctrl-c` cancels it.

The viewer reads fixed-size blocks and keeps only the visible page, at most eight
64 KiB raw blocks (512 KiB), three page frames, and at most 256 KiB of source
bytes per frame. Search and long-line work yields at 64 KiB. Each open uses a
fixed file snapshot; later
appends, truncation, replacement, or log rotation are not followed until the
file is reopened. It has an editor-style cursor separate from
the viewport. `Up`/`Down` and `j`/`k` move the cursor by file line, preserve its
preferred column when possible, and scroll the viewport only when the cursor
approaches an edge. Home/End or `0`/`$` move within the current line; `gg`/`G`
or Ctrl-Home/Ctrl-End move to the file start/end. End and `$` stop at the start
of the last visible token; empty lines use column zero. Page Up/Page Down and
Left/Right Arrow or `h`/`l` move between valid display tokens within the current
line in Text mode; in Hex mode they move by one source byte and may cross rows.
Ctrl-f/Ctrl-b move by one page; Ctrl-u/Ctrl-d move by half a page; Ctrl-e/Ctrl-y
scroll the viewport by one line without moving the cursor. A page is the visible
viewer height minus two rows. Repeated page input keeps at most one page in flight
and one changed-direction replacement. Use `/` for forward matching search, `?` for
reverse matching search, `]` for forward non-matching-line search, and `[` for
reverse non-matching-line search (Text mode only). These keys establish the
recorded matching or non-matching direction. `n` repeats that direction; `N` uses
the opposite direction without changing the recorded direction, so repeated `N`
commands continue opposite to the recorded direction. Each search wraps at most
once at the file boundary: forward wrap reports `search hit BOTTOM, continuing
at TOP`, and reverse wrap reports `search hit TOP, continuing at BOTTOM`. A
one-result search may wrap back to the same result. Close the viewer with
`Ctrl-b x` and confirm with `y`; `q`
and Esc do not close it. The prompt uses OSC 7 when the active shell reports it
and otherwise starts from the session's startup directory.
In the viewer, plain `?` starts reverse search; the configured prefix followed
by `?` opens grouped Help for viewer, search, and mode keys. Exiting that Help
with `q`, `Ctrl-c`, or `Esc` returns to the same viewer without changing its state. The configured prefix followed by `x`
asks to close the viewer; confirm with `y`.
Each new or repeated search starts strictly after or before the current cursor
in its selected direction; viewport-only scrolling does not change that anchor.
The viewer uses every pane-content row directly above the status bar, including
when the final line fills the terminal width or ends with a newline.
Visible matches use inverse attributes; the active match is additionally
underlined, and wrapped searches report `wrapped`.
Press `H` to toggle between Text and Hex mode without reopening the file; the
current byte position is preserved where possible, and in-flight viewer work is
cancelled before the new frame is rendered.
In Hex mode, normal search queries match ASCII bytes case-insensitively; queries
starting with `hex:` use exact space-separated bytes, such as `hex:00 FF 1B`.
Hex rows use the greatest fitting multiple-of-eight-byte layout, with a four-byte
fallback when needed, aligned separators between complete eight-byte groups, and
no separators in the ASCII column.
Matches may cross displayed rows and raw file blocks.

Text mode renders valid UTF-8 using terminal display-cell widths. Combining marks
stay with the preceding token, tabs expand to the next eight-cell stop, ASCII
controls use caret notation (`NUL` as `^@`, `ESC` as `^[`, and `DEL` as `^?`),
and invalid or other non-printable bytes render as uppercase `<XX>` values. Line
boundaries recognize LF, CRLF, and lone CR from the file contents, including
mixed line endings; EOL bytes are not displayed as line content.

### Save scrollback

After `Ctrl-b S`, type a filename and press `Enter`. `Backspace` edits the name;
`Esc` or `Ctrl-c` cancels without writing a file. The saved file is UTF-8 plain
text without terminal styling or control sequences.

## Tabs, panes, and status

Each session contains tabs, and each tab contains one or more panes. New tabs
and panes start the session's shell in the directory where the session was
created. Linux uses an absolute `$SHELL` or `/bin/sh`; Windows uses an absolute
`%COMSPEC%` or `%SystemRoot%\System32\cmd.exe`.

The bottom row shows the session, tab list, active tab, date, and time:

```text
[work]  [1:shell]  2:logs  3:db  |  2026-07-25 18:42
```

Termfold uses box-drawing pane dividers with a heavier active-pane border, with
an ASCII fallback for conservative terminals. On narrow terminals, Termfold
removes less important status content and inactive tabs first.

Closing the last pane closes its tab. Closing the last pane in the session ends
the session.

## Mouse

Mouse support is available but disabled by default. Enable it in the
configuration file with:

```toml
mouse = true
```

When enabled:

- click a pane to focus it;
- click a tab in the status bar to select it;
- drag a pane border to resize;
- use the wheel to scroll Termfold history; and
- mouse input is forwarded when the active application enables mouse reporting.

Keyboard-only operation remains fully supported.


### WezTerm wheel behaviour

With `mouse = false`, WezTerm may translate wheel input in the alternate screen
into Up/Down keys, which can move an interactive prompt inside the active pane.
To disable that translation, configure WezTerm mouse bindings for `WheelUp` and
`WheelDown` with `alt_screen = true`, `mouse_reporting = false`, and
`action = wezterm.action.Nop`.

## Configuration

Configuration is optional. Termfold reads:

- Linux: `$XDG_CONFIG_HOME/termfold/config.toml`, falling back to
  `$HOME/.config/termfold/config.toml`.
- Windows: `%APPDATA%\Termfold\config.toml`.

Start from the shipped [config.example.toml](config.example.toml). On Linux:

```bash
mkdir -p "${XDG_CONFIG_HOME:-$HOME/.config}/termfold"
cp config.example.toml "${XDG_CONFIG_HOME:-$HOME/.config}/termfold/config.toml"
```

On Windows, create `%APPDATA%\Termfold` and copy the example there as
`config.toml` (for example, in Command Prompt: `mkdir "%APPDATA%\Termfold"`
then `copy config.example.toml "%APPDATA%\Termfold\config.toml"`).

The default configuration is:

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

### Configuration fields

| Field | Accepted values |
| --- | --- |
| `prefix` | One control key from `"C-a"` through `"C-z"` |
| `mouse` | `true` or `false` |
| `scrollback_lines` | `0` through `10000` |
| `viewer_tab_width` | `1` through `16` display cells |
| `date_format` | Up to 64 characters using supported time directives |
| `time_format` | Up to 64 characters using supported time directives |
| `status_format` | Up to 512 UTF-8 characters using supported placeholders |
| `status_label` | Up to 64 printable characters |
| `status_theme` | `default` or one of the ten built-in themes below |
| `status_refresh_seconds` | `1` through `3600` |
| `cpu_temperature_path` | Optional absolute sensor path below `/sys` |
| `status_foreground`, `status_background` | Base status colours: `default`, ANSI name, or `#RRGGBB` |
| `label_foreground`, `label_background` | Label colours: `default`, ANSI name, or `#RRGGBB` |
| `active_tab_foreground`, `active_tab_background` | Active-tab colours: `default`, ANSI name, or `#RRGGBB` |
| `terminal_profile` | `"auto"` or a built-in terminal profile |
| `inner_term` | `"termfold-256color"` or compatibility value `"xterm-256color"` |
| `windows_shell` | Windows-only absolute executable path followed by literal arguments |

Built-in status themes are `catppuccin-latte`, `catppuccin-mocha`,
`solarized-light`, `solarized-dark`, `gruvbox-light`, `gruvbox-dark`,
`tokyo-night-day`, `tokyo-night`, `dracula`, and `nord`. Individual status
colour fields override the selected theme. Themes affect Termfold-owned UI, not
applications inside panes, and require no download or plugin.

Supported date and time directives are `%Y`, `%m`, `%d`, `%H`, `%I`, `%M`, `%S`,
`%p`, and `%%`. For example, use `"%I:%M %p"` for a 12-hour clock.

The status placeholders are `{session}`, `{tabs}`, `{fill}`, `{label}`,
`{cpu_usage}`, `{memory_usage}`, `{cpu_temp}`, `{date}`, and `{time}`. `{fill}`
uses every remaining column, right-aligning the content after it. The session,
tabs, date, time, and exactly one fill placeholder are required.

For example:

```toml
status_format = "[{session}]  {tabs}  │  {label}{fill}CPU {cpu_usage}%  MEM {memory_usage}%  TEMP {cpu_temp}  {date} {time}"
status_label = "PROD | db-02"
status_refresh_seconds = 2
cpu_temperature_path = "/sys/class/thermal/thermal_zone0/temp"
label_foreground = "#ffffff"
label_background = "#b00020"
```

CPU and memory values come from Linux `/proc` or native Win32 system metrics.
CPU temperature uses the configured Linux sysfs file and displays `-` on
Windows or when unavailable. Special characters are literal UTF-8 and require a
font that contains the selected glyph.

Built-in terminal profiles are `dumb`, `ansi`, `vt100`, `linux`, `xterm`,
`xterm-256color`, `screen`, `screen-256color`, `tmux`, and `tmux-256color`.
Leave `terminal_profile = "auto"` unless diagnosing a compatibility problem.

Changing `prefix` changes the first key of every shortcut. For example, with:

```toml
prefix = "C-a"
```

create a tab with `Ctrl-a c` and send a literal `Ctrl-a` with `Ctrl-a Ctrl-a`.

Profiles select the initial tabs and panes for a newly created session. The
`[profiles.default]` example is used by `termfold new` when no profile is
specified; a named profile is selected with `termfold new SESSION --profile
NAME`. Use `--no-profile` to create the ordinary single-shell layout.
Profile command targets must be absolute executable paths, and profile
directories must be absolute existing directories. Shell and command
arguments are passed literally without shell interpolation. Profile changes,
like all configuration changes, affect only newly created sessions; attaching
never reruns a profile.

Invalid or unknown configuration fields stop startup with an error naming the
field. Termfold never rewrites the configuration file.

On Windows, omitting `windows_shell` uses `%COMSPEC%`, falling back to
`%SystemRoot%\System32\cmd.exe`. Shell changes affect only newly created
sessions. Arguments are passed directly without command interpolation.

MSYS2-compatible applications use a hexadecimal terminfo-directory name on
Windows. Termfold materializes its private `termfold-256color` entry in that
layout automatically; no system `tic` installation is required.

## Terminal compatibility

For common xterm-compatible outer terminals, these values are recommended:

```bash
export TERM=xterm-256color
export COLORTERM=truecolor
```

Termfold sets the inner terminal identity itself. If rendering or input behaves
incorrectly, run:

```bash
termfold diagnose
```

The report includes the selected terminal profile, colour level, mouse and
alternate-screen support, inner terminal identity, terminfo validation, terminal
size, version, and architecture.

## Limits

- 32 concurrent sessions per user
- 32 tabs per session
- 16 panes per tab
- 2,000 scrollback lines per pane by default

Termfold has no network transport. To use it remotely, run Termfold on the
remote Linux host through SSH.


A running session server cannot be live-upgraded. End the affected session
before replacing an executable that is still in use. A newer client can attach
to an older server only while their IPC protocol versions remain compatible.

## Prior art and acknowledgements

Termfold draws inspiration from:

- zmx's small, self-contained approach to wrapping terminal sessions;
- tmux's established session, window/tab, pane, prefix-key, and status-line
  interaction conventions;
- the xterm and ncurses terminfo documentation as references for terminal
  protocols and capabilities; and
- the Catppuccin, Solarized, Gruvbox, Tokyo Night, Dracula, and Nord colour
  palettes used by the built-in status themes.

These acknowledgements do not imply endorsement, affiliation, code reuse, or
compatibility certification by any named project or its maintainers.

## Project documentation

- [Product requirements](REQUIREMENTS.md)
- [AI and engineering workflow](AGENTS.md)
- [Implementation tasks and validation](TASKS.md)
- [License](LICENSE)
