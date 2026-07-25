# Termfold

Termfold is a small terminal multiplexer for Linux and WSL. It provides
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

Native Windows and macOS executables are not supported.

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
termfold kill [NAME]           Terminate a session
termfold diagnose              Show terminal compatibility decisions
termfold --help                Show command usage
termfold --version             Show the installed version
```

`NAME` defaults to `default` where applicable and may contain 1–64 ASCII
letters, digits, underscores, or hyphens.

`termfold list` prints each session as:

```text
PID NAME attached|detached
```

Multiple clients owned by the same user may attach to the same session. Changes
to tabs, panes, and focus are shared between attached clients.

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
| `Ctrl-b [` | Enter the active pane's read-only scroll view |
| `Ctrl-b S` | Prompt for a file and save the active pane's scrollback |

For resize shortcuts, press and release `Ctrl-b`, then hold `Ctrl` while pressing
an arrow key. For repeated resizing, press `Ctrl-b r`, use Arrow keys, then press
`Esc` to leave resize mode.

An unknown prefix command is not sent to the application; Termfold reports it in
the status bar.

### Scroll view

After `Ctrl-b [`:

| Key | Action |
| --- | --- |
| `Up` or `k` | Scroll up one line |
| `Down` or `j` | Scroll down one line |
| `Page Up` | Scroll up one page |
| `Page Down` | Scroll down one page |
| `q`, `Ctrl-c`, or `Esc` | Return to the live pane |

Scroll view is read-only. Use the outer terminal's normal text selection and
clipboard features to copy visible text.

### Save scrollback

After `Ctrl-b S`, type a filename and press `Enter`. `Backspace` edits the name;
`Esc` or `Ctrl-c` cancels without writing a file. The saved file is UTF-8 plain
text without terminal styling or control sequences.

## Tabs, panes, and status

Each session contains tabs, and each tab contains one or more panes. New tabs
and panes start the session's shell in the directory where the session was
created.

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

## Configuration

Configuration is optional. Termfold reads:

1. `$XDG_CONFIG_HOME/termfold/config.toml`, when `XDG_CONFIG_HOME` is set; or
2. `$HOME/.config/termfold/config.toml`.

The default configuration is:

```toml
prefix = "C-b"
mouse = false
scrollback_lines = 2000
date_format = "%Y-%m-%d"
time_format = "%H:%M"
terminal_profile = "auto"
inner_term = "termfold-256color"
```

### Configuration fields

| Field | Accepted values |
| --- | --- |
| `prefix` | One control key from `"C-a"` through `"C-z"` |
| `mouse` | `true` or `false` |
| `scrollback_lines` | `0` through `10000` |
| `date_format` | Up to 64 characters using supported time directives |
| `time_format` | Up to 64 characters using supported time directives |
| `terminal_profile` | `"auto"` or a built-in terminal profile |
| `inner_term` | `"termfold-256color"` or compatibility value `"xterm-256color"` |

Supported date and time directives are `%Y`, `%m`, `%d`, `%H`, `%I`, `%M`, `%S`,
`%p`, and `%%`. For example, use `"%I:%M %p"` for a 12-hour clock.

Built-in terminal profiles are `dumb`, `ansi`, `vt100`, `linux`, `xterm`,
`xterm-256color`, `screen`, `screen-256color`, `tmux`, and `tmux-256color`.
Leave `terminal_profile = "auto"` unless diagnosing a compatibility problem.

Changing `prefix` changes the first key of every shortcut. For example, with:

```toml
prefix = "C-a"
```

create a tab with `Ctrl-a c` and send a literal `Ctrl-a` with `Ctrl-a Ctrl-a`.

Invalid or unknown configuration fields stop startup with an error naming the
field. Termfold never rewrites the configuration file.

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

## Project documentation

- [Requirements](REQUIREMENTS.md)
- [Acknowledgements](ACKNOWLEDGEMENTS.md)
- [License](LICENSE)
