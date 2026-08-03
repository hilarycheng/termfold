use std::time::{Duration, Instant};

use crate::session::{Direction, Split};

const MAX_FILENAME_BYTES: usize = 4096;
const MAX_VIEW_PATH_BYTES: usize = 4096;
const MAX_SEARCH_BYTES: usize = 256;
const MAX_MOUSE_SEQUENCE_BYTES: usize = 32;
const MOUSE_SEQUENCE_TIMEOUT: Duration = Duration::from_millis(10);
const ESCAPE_SEQUENCE_TIMEOUT: Duration = Duration::from_millis(100);
const VIEWER_PREFIX_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MouseEvent {
    pub code: u16,
    pub x: u16,
    pub y: u16,
    pub release: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub enum Action {
    Forward(Vec<u8>),
    CreateTab,
    NextTab,
    PreviousTab,
    SelectTab(usize),
    Split(Split),
    Focus(Direction),
    Resize(Direction),
    ClosePane,
    Detach,
    HelpView,
    HelpScroll(i32),
    ExitHelpView,
    ScrollView,
    Scroll(i32),
    ScrollTop,
    ScrollBottom,
    Search(String),
    SearchNext(bool),
    SearchCancelled,
    ExitScrollView,
    ViewPrompt(bool),
    ViewQuery(Vec<u8>),
    ViewDirectory { query: Vec<u8>, separator: u8 },
    ViewParent,
    ViewComplete(Vec<u8>),
    ViewSelect(i32),
    OpenViewer(Vec<u8>),
    ViewCancelled,
    ViewerScroll(i32),
    ViewerViewport(i32),
    ViewerPage(bool),
    ViewerHalfPage(bool),
    ViewerLineStart,
    ViewerLineEnd,
    ViewerTop,
    ViewerBottom,
    ViewerSearchPrompt(bool),
    ViewerSearchQuery(Vec<u8>, bool),
    ViewerSearch(Vec<u8>, bool),
    ViewerSearchNext(bool),
    ViewerSearchCancelled,
    ClearScrollback,
    SaveScrollback(String),
    Mouse(MouseEvent),
    Status(String),
}

#[derive(Debug)]
enum Mode {
    Normal,
    Prefix(Vec<u8>),
    ConfirmClose(bool),
    Filename(Vec<u8>),
    Resize(Vec<u8>),
    Help(Vec<u8>),
    Scroll(Vec<u8>),
    Search(Vec<u8>),
    ViewPrompt(Vec<u8>, bool),
    Viewer(Vec<u8>),
    ViewerPrefix,
    ViewerG,
    ViewerSearch(Vec<u8>, bool),
}

#[derive(Debug)]
pub struct Input {
    prefix: u8,
    mouse: bool,
    pending_mouse: Vec<u8>,
    pending_since: Option<Instant>,
    mode: Mode,
}

impl Input {
    pub fn new(prefix: u8, mouse: bool) -> Self {
        Self {
            prefix,
            mouse,
            pending_mouse: Vec::new(),
            pending_since: None,
            mode: Mode::Normal,
        }
    }

    pub fn prefix(&self) -> u8 {
        self.prefix
    }

    pub fn is_scroll_mode(&self) -> bool {
        matches!(self.mode, Mode::Scroll(_) | Mode::Search(_))
    }

    pub fn enter_viewer(&mut self) {
        self.mode = Mode::Viewer(Vec::new());
    }

    pub fn set_view_prompt(&mut self, path: Vec<u8>) {
        if let Mode::ViewPrompt(_, return_viewer) = &self.mode {
            self.mode = Mode::ViewPrompt(path, *return_viewer);
        }
    }

    pub fn advance(&mut self, bytes: &[u8]) -> Vec<Action> {
        let mut actions = Vec::new();
        let mut forwarded = Vec::new();
        let mut input = std::mem::take(&mut self.pending_mouse);
        input.extend_from_slice(bytes);
        let mut offset = 0;
        while offset < input.len() {
            if self.mouse && input[offset] == 27 {
                match parse_mouse(&input[offset..]) {
                    MouseParse::Complete(event, length) => {
                        push_forward(&mut actions, &mut forwarded);
                        actions.push(Action::Mouse(event));
                        offset += length;
                        self.pending_since = None;
                        continue;
                    }
                    MouseParse::Incomplete => {
                        self.pending_mouse.extend_from_slice(&input[offset..]);
                        self.pending_since.get_or_insert_with(Instant::now);
                        break;
                    }
                    MouseParse::Invalid => {}
                }
            }
            if input[offset] == 27 && matches!(self.mode, Mode::ViewPrompt(_, _)) {
                match parse_view_prompt(&input[offset..]) {
                    ViewPromptParse::Complete(direction, length) => {
                        push_forward(&mut actions, &mut forwarded);
                        let action = match direction {
                            Direction::Up => Action::ViewSelect(-1),
                            Direction::Down => Action::ViewSelect(1),
                            Direction::Left => Action::ViewParent,
                            Direction::Right => match &self.mode {
                                Mode::ViewPrompt(path, _) => Action::OpenViewer(path.clone()),
                                _ => unreachable!(),
                            },
                        };
                        actions.push(action);
                        offset += length;
                        self.pending_since = None;
                        continue;
                    }
                    ViewPromptParse::Incomplete => {
                        self.pending_mouse.extend_from_slice(&input[offset..]);
                        self.pending_since.get_or_insert_with(Instant::now);
                        break;
                    }
                    ViewPromptParse::Invalid => {}
                }
            }
            if input[offset] == 27
                && offset + 1 == input.len()
                && matches!(
                    self.mode,
                    Mode::Resize(_)
                        | Mode::Scroll(_)
                        | Mode::Help(_)
                        | Mode::Search(_)
                        | Mode::ViewPrompt(_, _)
                        | Mode::Viewer(_)
                        | Mode::ViewerG
                        | Mode::ViewerSearch(_, _)
                )
            {
                self.pending_mouse.push(27);
                self.pending_since.get_or_insert_with(Instant::now);
                break;
            }
            self.advance_byte(input[offset], &mut actions, &mut forwarded);
            offset += 1;
        }
        if self.pending_mouse.is_empty() && !matches!(self.mode, Mode::ViewerPrefix) {
            self.pending_since = None;
        }
        push_forward(&mut actions, &mut forwarded);
        actions
    }

    pub fn flush_pending_mouse(&mut self) -> Vec<Action> {
        let Some(timeout) = self.pending_timeout() else {
            return Vec::new();
        };
        if !timeout.is_zero() {
            return Vec::new();
        }
        if matches!(self.mode, Mode::ViewerPrefix) {
            self.pending_since = None;
            self.mode = Mode::Viewer(Vec::new());
            return vec![Action::ViewerPage(false)];
        }
        self.pending_since = None;
        let input = std::mem::take(&mut self.pending_mouse);
        if input == b"\x1b" {
            return match self.mode {
                Mode::Resize(_) => {
                    self.mode = Mode::Normal;
                    vec![Action::Status("resize mode ended".into())]
                }
                Mode::Scroll(_) => {
                    self.mode = Mode::Normal;
                    vec![Action::ExitScrollView]
                }
                Mode::Help(_) => {
                    self.mode = Mode::Normal;
                    vec![Action::ExitHelpView]
                }
                Mode::Search(_) => {
                    self.mode = Mode::Scroll(Vec::new());
                    vec![Action::SearchCancelled]
                }
                Mode::ViewPrompt(_, return_viewer) => {
                    self.mode = if return_viewer {
                        Mode::Viewer(Vec::new())
                    } else {
                        Mode::Normal
                    };
                    vec![Action::ViewCancelled]
                }
                Mode::Viewer(_) | Mode::ViewerG => {
                    self.mode = Mode::Viewer(Vec::new());
                    Vec::new()
                }
                Mode::ViewerSearch(_, _) => {
                    self.mode = Mode::Viewer(Vec::new());
                    vec![Action::ViewerSearchCancelled]
                }
                _ => vec![Action::Forward(input)],
            };
        }
        let mut actions = Vec::new();
        let mut forwarded = Vec::new();
        for byte in input {
            self.advance_byte(byte, &mut actions, &mut forwarded);
        }
        push_forward(&mut actions, &mut forwarded);
        actions
    }

    pub fn pending_timeout(&self) -> Option<Duration> {
        let since = self.pending_since?;
        let timeout = if matches!(self.mode, Mode::ViewerPrefix) {
            VIEWER_PREFIX_TIMEOUT
        } else if self.pending_mouse.starts_with(b"\x1b")
            && matches!(
                self.mode,
                Mode::Resize(_)
                    | Mode::Scroll(_)
                    | Mode::Help(_)
                    | Mode::Search(_)
                    | Mode::ViewPrompt(_, _)
                    | Mode::Viewer(_)
                    | Mode::ViewerG
                    | Mode::ViewerSearch(_, _)
            )
        {
            ESCAPE_SEQUENCE_TIMEOUT
        } else {
            MOUSE_SEQUENCE_TIMEOUT
        };
        Some(timeout.saturating_sub(since.elapsed()))
    }

    fn advance_byte(&mut self, byte: u8, actions: &mut Vec<Action>, forwarded: &mut Vec<u8>) {
        if matches!(self.mode, Mode::ViewerPrefix) {
            self.pending_since = None;
            self.mode = Mode::Viewer(Vec::new());
            if byte == b'x' {
                self.mode = Mode::ConfirmClose(true);
                actions.push(Action::Status("close viewer tab? (y/n)".into()));
                return;
            }
            if matches!(byte, b'v' | b'V') {
                self.mode = Mode::ViewPrompt(Vec::new(), true);
                actions.push(Action::ViewPrompt(true));
                return;
            }
            actions.push(Action::ViewerPage(false));
            self.advance_byte(byte, actions, forwarded);
            return;
        }
        if matches!(self.mode, Mode::ViewerG) {
            self.mode = Mode::Viewer(Vec::new());
            if byte == b'g' {
                actions.push(Action::ViewerTop);
                return;
            }
            self.advance_byte(byte, actions, forwarded);
            return;
        }
        match &mut self.mode {
            Mode::Normal if byte == self.prefix => {
                push_forward(actions, forwarded);
                self.mode = Mode::Prefix(Vec::new());
            }
            Mode::Normal => forwarded.push(byte),
            Mode::Prefix(sequence) => {
                sequence.push(byte);
                if let Some(action) = prefix_action(self.prefix, sequence) {
                    self.mode = match sequence.as_slice() {
                        [b'x'] => Mode::ConfirmClose(false),
                        [b'S'] => Mode::Filename(Vec::new()),
                        [b'r'] => Mode::Resize(Vec::new()),
                        [b'?'] => Mode::Help(Vec::new()),
                        [b'['] => Mode::Scroll(Vec::new()),
                        [b'v' | b'V'] => Mode::ViewPrompt(Vec::new(), false),
                        _ => Mode::Normal,
                    };
                    actions.push(action);
                }
            }
            Mode::ConfirmClose(return_viewer) => {
                actions.push(if matches!(byte, b'y' | b'Y') {
                    Action::ClosePane
                } else {
                    Action::Status("close cancelled".into())
                });
                self.mode = if matches!(byte, b'y' | b'Y') {
                    Mode::Normal
                } else if *return_viewer {
                    Mode::Viewer(Vec::new())
                } else {
                    Mode::Normal
                };
            }
            Mode::Filename(_) if matches!(byte, 3 | 27) => {
                actions.push(Action::Status("scrollback export cancelled".into()));
                self.mode = Mode::Normal;
            }
            Mode::Filename(filename) if matches!(byte, 8 | 127) => {
                filename.pop();
                actions.push(filename_status(filename));
            }
            Mode::Filename(filename) if matches!(byte, b'\r' | b'\n') => {
                actions.push(match String::from_utf8(std::mem::take(filename)) {
                    Ok(filename) if !filename.is_empty() => Action::SaveScrollback(filename),
                    Ok(_) => Action::Status("scrollback export cancelled".into()),
                    Err(_) => Action::Status("filename must be UTF-8".into()),
                });
                self.mode = Mode::Normal;
            }
            Mode::Filename(filename) if filename.len() == MAX_FILENAME_BYTES => {
                actions.push(Action::Status("filename is too long".into()));
            }
            Mode::Filename(filename) => {
                filename.push(byte);
                actions.push(filename_status(filename));
            }
            Mode::Resize(sequence) => {
                sequence.push(byte);
                let action = match sequence.as_slice() {
                    [27, b'[' | b'O', final_byte @ (b'A'..=b'D')] => {
                        Some(Action::Resize(direction(*final_byte)))
                    }
                    [27] | [27, b'[' | b'O'] => None,
                    _ => Some(Action::Status(
                        "resize mode: arrows resize, Esc exits".into(),
                    )),
                };
                if let Some(action) = action {
                    sequence.clear();
                    actions.push(action);
                    if matches!(actions.last(), Some(Action::Resize(_))) {
                        actions.push(Action::Status(
                            "resize mode: arrows resize, Esc exits".into(),
                        ));
                    }
                }
            }
            Mode::Help(sequence) => {
                sequence.push(byte);
                let action = match sequence.as_slice() {
                    [b'q' | 3] => Some(Action::ExitHelpView),
                    [b'k'] | [27, b'[' | b'O', b'A'] => Some(Action::HelpScroll(-1)),
                    [b'j'] | [27, b'[' | b'O', b'B'] => Some(Action::HelpScroll(1)),
                    [27, b'[', b'5', b'~'] => Some(Action::HelpScroll(i32::MIN)),
                    [27, b'[', b'6', b'~'] => Some(Action::HelpScroll(i32::MAX)),
                    [27] | [27, b'[' | b'O'] | [27, b'[', b'5' | b'6'] => None,
                    _ => Some(Action::Status(
                        "help: arrows/j/k/Page Up/Page Down, q/Esc exits".into(),
                    )),
                };
                if let Some(action) = action {
                    if matches!(action, Action::ExitHelpView) {
                        self.mode = Mode::Normal;
                    } else {
                        sequence.clear();
                    }
                    actions.push(action);
                }
            }
            Mode::Scroll(_) if byte == b'/' => {
                self.mode = Mode::Search(Vec::new());
                actions.push(Action::Status("/".into()));
            }
            Mode::Scroll(sequence) => {
                sequence.push(byte);
                let action = match sequence.as_slice() {
                    [b'q' | 3] => Some(Action::ExitScrollView),
                    [b'k'] | [27, b'[' | b'O', b'A'] => Some(Action::Scroll(1)),
                    [b'j'] | [27, b'[' | b'O', b'B'] => Some(Action::Scroll(-1)),
                    [27, b'[', b'5', b'~'] => Some(Action::Scroll(i32::MAX)),
                    [27, b'[', b'6', b'~'] => Some(Action::Scroll(i32::MIN)),
                    [b'g'] => Some(Action::ScrollTop),
                    [b'G'] => Some(Action::ScrollBottom),
                    [b'n'] => Some(Action::SearchNext(true)),
                    [b'N'] => Some(Action::SearchNext(false)),
                    [27] | [27, b'[' | b'O'] | [27, b'[', b'5' | b'6'] => None,
                    _ => Some(Action::Status(
                        "scroll: arrows/j/k/Page Up/Page Down, / search, q/Esc exits".into(),
                    )),
                };
                if let Some(action) = action {
                    if matches!(action, Action::ExitScrollView) {
                        self.mode = Mode::Normal;
                    } else {
                        sequence.clear();
                    }
                    actions.push(action);
                }
            }
            Mode::Search(_) if matches!(byte, 3 | 27) => {
                self.mode = Mode::Scroll(Vec::new());
                actions.push(Action::SearchCancelled);
            }
            Mode::Search(query) if matches!(byte, 8 | 127) => {
                query.pop();
                actions.push(search_status(query));
            }
            Mode::Search(query) if matches!(byte, b'\r' | b'\n') => {
                actions.push(match String::from_utf8(std::mem::take(query)) {
                    Ok(query) if !query.is_empty() => Action::Search(query),
                    Ok(_) => Action::Status("search is empty".into()),
                    Err(_) => Action::Status("search must be UTF-8".into()),
                });
                self.mode = Mode::Scroll(Vec::new());
            }
            Mode::Search(query) if query.len() == MAX_SEARCH_BYTES => {
                actions.push(Action::Status("search is too long".into()));
            }
            Mode::Search(query) => {
                query.push(byte);
                actions.push(search_status(query));
            }
            Mode::ViewPrompt(_, _) if matches!(byte, 14 | 16) => {
                actions.push(Action::ViewSelect(if byte == 14 { 1 } else { -1 }));
            }
            Mode::ViewPrompt(_, return_viewer) if matches!(byte, 3 | 27) => {
                self.mode = if *return_viewer {
                    Mode::Viewer(Vec::new())
                } else {
                    Mode::Normal
                };
                actions.push(Action::ViewCancelled);
            }
            Mode::ViewPrompt(path, _) if matches!(byte, 8 | 127) => {
                if path.pop().is_some() {
                    actions.push(Action::ViewQuery(path.clone()));
                } else {
                    actions.push(Action::ViewParent);
                }
            }
            Mode::ViewPrompt(path, _) if matches!(byte, b'/' | b'\\') => {
                actions.push(Action::ViewDirectory {
                    query: path.clone(),
                    separator: byte,
                });
            }
            Mode::ViewPrompt(path, _) if byte == 9 => {
                actions.push(Action::ViewComplete(path.clone()));
            }
            Mode::ViewPrompt(path, _) if matches!(byte, b'\r' | b'\n') => {
                actions.push(Action::OpenViewer(path.clone()));
            }
            Mode::ViewPrompt(path, _) if path.len() == MAX_VIEW_PATH_BYTES => {
                actions.push(Action::Status("viewer path is too long".into()));
            }
            Mode::ViewPrompt(_, _) if byte.is_ascii_control() => {
                actions.push(Action::Status("viewer path accepts printable text".into()));
            }
            Mode::ViewPrompt(path, _) => {
                path.push(byte);
                actions.push(Action::ViewQuery(path.clone()));
            }
            Mode::Viewer(sequence) if sequence.is_empty() && byte == self.prefix => {
                self.mode = Mode::ViewerPrefix;
                self.pending_since = Some(Instant::now());
            }
            Mode::Viewer(sequence) if sequence.is_empty() && byte == 2 => {
                sequence.push(byte);
                actions.push(Action::ViewerPage(false));
                sequence.clear();
            }
            Mode::Viewer(sequence) if sequence.is_empty() && byte == b'g' => {
                self.mode = Mode::ViewerG;
            }
            Mode::Viewer(sequence) => {
                sequence.push(byte);
                let action = match sequence.as_slice() {
                    [b'k'] | [27, b'[', b'A'] | [27, b'O', b'A'] => Some(Action::ViewerScroll(-1)),
                    [b'j'] | [27, b'[', b'B'] | [27, b'O', b'B'] => Some(Action::ViewerScroll(1)),
                    [27, b'[', b'5', b'~'] => Some(Action::ViewerPage(false)),
                    [27, b'[', b'6', b'~'] => Some(Action::ViewerPage(true)),
                    [21] => Some(Action::ViewerHalfPage(false)),
                    [4] => Some(Action::ViewerHalfPage(true)),
                    [6] => Some(Action::ViewerPage(true)),
                    [5] => Some(Action::ViewerViewport(1)),
                    [25] => Some(Action::ViewerViewport(-1)),
                    [b'0'] | [27, b'[', b'H'] | [27, b'[', b'1', b'~'] | [27, b'[', b'7', b'~'] => {
                        Some(Action::ViewerLineStart)
                    }
                    [b'$'] | [27, b'[', b'F'] | [27, b'[', b'4', b'~'] | [27, b'[', b'8', b'~'] => {
                        Some(Action::ViewerLineEnd)
                    }
                    [27, b'[', b'1', b';', b'5', b'H'] | [27, b'[', b'1', b';', b'5', b'~'] => {
                        Some(Action::ViewerTop)
                    }
                    [27, b'[', b'1', b';', b'5', b'F'] | [27, b'[', b'4', b';', b'5', b'~'] => {
                        Some(Action::ViewerBottom)
                    }
                    [b'G'] => Some(Action::ViewerBottom),
                    [b'/'] => Some(Action::ViewerSearchPrompt(true)),
                    [b'?'] => Some(Action::ViewerSearchPrompt(false)),
                    [b'n'] => Some(Action::ViewerSearchNext(true)),
                    [b'N'] => Some(Action::ViewerSearchNext(false)),
                    [27]
                    | [27, b'[']
                    | [27, b'O']
                    | [27, b'[', b'1']
                    | [27, b'[', b'4']
                    | [27, b'[', b'5' | b'6']
                    | [27, b'[', b'7' | b'8']
                    | [27, b'[', b'1', b';']
                    | [27, b'[', b'1', b';', b'5']
                    | [27, b'[', b'4', b';']
                    | [27, b'[', b'4', b';', b'5'] => None,
                    _ => Some(Action::Status(
                        "viewer: j/k arrows Home/End gg/G Ctrl-u/d Ctrl-b/f / ? n/N".into(),
                    )),
                };
                if let Some(action) = action {
                    self.mode = if matches!(action, Action::ViewerSearchPrompt(_)) {
                        if matches!(action, Action::ViewerSearchPrompt(true)) {
                            Mode::ViewerSearch(Vec::new(), true)
                        } else {
                            Mode::ViewerSearch(Vec::new(), false)
                        }
                    } else {
                        Mode::Viewer(Vec::new())
                    };
                    actions.push(action);
                }
            }
            Mode::ViewerPrefix | Mode::ViewerG => unreachable!("viewer mode handled above"),
            Mode::ViewerSearch(_, _) if matches!(byte, 3 | 27) => {
                self.mode = Mode::Viewer(Vec::new());
                actions.push(Action::ViewerSearchCancelled);
            }
            Mode::ViewerSearch(query, forward) if matches!(byte, 8 | 127) => {
                query.pop();
                actions.push(Action::ViewerSearchQuery(query.clone(), *forward));
                if query.is_empty() {
                    actions.push(Action::Status(if *forward { "/" } else { "?" }.into()));
                }
            }
            Mode::ViewerSearch(query, forward) if matches!(byte, b'\r' | b'\n') => {
                if query.is_empty() {
                    actions.push(Action::Status("viewer search is empty".into()));
                } else {
                    actions.push(Action::ViewerSearch(query.clone(), *forward));
                    self.mode = Mode::Viewer(Vec::new());
                }
            }
            Mode::ViewerSearch(query, _) if query.len() == MAX_SEARCH_BYTES => {
                actions.push(Action::Status("viewer search is too long".into()));
            }
            Mode::ViewerSearch(_, _) if byte.is_ascii_control() => {
                actions.push(Action::Status(
                    "viewer search accepts printable text".into(),
                ));
            }
            Mode::ViewerSearch(query, forward) => {
                query.push(byte);
                actions.push(Action::ViewerSearchQuery(query.clone(), *forward));
                actions.push(Action::Status(viewer_search_status(*forward, query)));
            }
        }
    }
}

enum MouseParse {
    Complete(MouseEvent, usize),
    Incomplete,
    Invalid,
}

enum ViewPromptParse {
    Complete(Direction, usize),
    Incomplete,
    Invalid,
}

fn parse_view_prompt(bytes: &[u8]) -> ViewPromptParse {
    for (sequence, direction) in [
        (b"\x1b[A".as_slice(), Direction::Up),
        (b"\x1b[B".as_slice(), Direction::Down),
        (b"\x1b[C".as_slice(), Direction::Right),
        (b"\x1b[D".as_slice(), Direction::Left),
        (b"\x1bOA".as_slice(), Direction::Up),
        (b"\x1bOB".as_slice(), Direction::Down),
        (b"\x1bOC".as_slice(), Direction::Right),
        (b"\x1bOD".as_slice(), Direction::Left),
    ] {
        if bytes.starts_with(sequence) {
            return ViewPromptParse::Complete(direction, sequence.len());
        }
        if sequence.starts_with(bytes) {
            return ViewPromptParse::Incomplete;
        }
    }
    ViewPromptParse::Invalid
}

fn parse_mouse(bytes: &[u8]) -> MouseParse {
    if !b"\x1b[<".starts_with(bytes) && !bytes.starts_with(b"\x1b[<") {
        return MouseParse::Invalid;
    }
    if bytes.len() < 3 {
        return MouseParse::Incomplete;
    }
    let Some(length) = bytes
        .iter()
        .take(MAX_MOUSE_SEQUENCE_BYTES)
        .position(|byte| matches!(byte, b'M' | b'm'))
        .map(|index| index + 1)
    else {
        return if bytes.len() < MAX_MOUSE_SEQUENCE_BYTES {
            MouseParse::Incomplete
        } else {
            MouseParse::Invalid
        };
    };
    let final_byte = bytes[length - 1];
    let Ok(parameters) = std::str::from_utf8(&bytes[3..length - 1]) else {
        return MouseParse::Invalid;
    };
    let mut parameters = parameters.split(';');
    let (Some(code), Some(x), Some(y)) = (
        parameters
            .next()
            .and_then(|value| value.parse::<u16>().ok()),
        parameters
            .next()
            .and_then(|value| value.parse::<u16>().ok()),
        parameters
            .next()
            .and_then(|value| value.parse::<u16>().ok()),
    ) else {
        return MouseParse::Invalid;
    };
    if parameters.next().is_some() || x == 0 || y == 0 {
        return MouseParse::Invalid;
    }
    MouseParse::Complete(
        MouseEvent {
            code,
            x: x - 1,
            y: y - 1,
            release: final_byte == b'm',
        },
        length,
    )
}

fn prefix_action(prefix: u8, sequence: &[u8]) -> Option<Action> {
    if sequence.len() == 1 && sequence[0] != 27 {
        return Some(match sequence[0] {
            byte if byte == prefix => Action::Forward(vec![prefix]),
            b'c' => Action::CreateTab,
            b'n' => Action::NextTab,
            b'p' => Action::PreviousTab,
            b'1'..=b'9' => Action::SelectTab(usize::from(sequence[0] - b'1')),
            b'0' => Action::SelectTab(9),
            b'|' => Action::Split(Split::LeftRight),
            b'-' => Action::Split(Split::TopBottom),
            b'r' => Action::Status("resize mode: arrows resize, Esc exits".into()),
            b'x' => Action::Status("close pane? (y/n)".into()),
            b'd' => Action::Detach,
            b'?' => Action::HelpView,
            b'[' => Action::ScrollView,
            b'v' => Action::ViewPrompt(false),
            b'V' => Action::ViewPrompt(false),
            b'C' => Action::ClearScrollback,
            b'S' => Action::Status("save scrollback file: ".into()),
            _ => Action::Status("unsupported prefix command".into()),
        });
    }
    match sequence {
        [27]
        | [27, b'[' | b'O']
        | [27, b'[', b'1']
        | [27, b'[', b'1', b';']
        | [27, b'[', b'1', b';', b'5'] => None,
        [27, b'[' | b'O', final_byte @ (b'A'..=b'D')] => {
            Some(Action::Focus(direction(*final_byte)))
        }
        [27, b'[', b'1', b';', b'5', final_byte @ (b'A'..=b'D')] => {
            Some(Action::Resize(direction(*final_byte)))
        }
        sequence
            if sequence
                .last()
                .is_some_and(|byte| (0x40..=0x7e).contains(byte)) =>
        {
            Some(Action::Status("unsupported prefix command".into()))
        }
        sequence if sequence.len() >= 8 => {
            Some(Action::Status("unsupported prefix command".into()))
        }
        _ => None,
    }
}

fn direction(final_byte: u8) -> Direction {
    match final_byte {
        b'A' => Direction::Up,
        b'B' => Direction::Down,
        b'C' => Direction::Right,
        b'D' => Direction::Left,
        _ => unreachable!(),
    }
}

fn filename_status(filename: &[u8]) -> Action {
    let visible: String = String::from_utf8_lossy(filename)
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    Action::Status(format!("save scrollback file: {visible}"))
}

fn search_status(query: &[u8]) -> Action {
    let visible: String = String::from_utf8_lossy(query)
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    Action::Status(format!("/{visible}"))
}

fn viewer_search_status(forward: bool, query: &[u8]) -> String {
    let visible: String = String::from_utf8_lossy(query)
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    format!("{}{visible}", if forward { "/" } else { "?" })
}

fn push_forward(actions: &mut Vec<Action>, bytes: &mut Vec<u8>) {
    if !bytes.is_empty() {
        actions.push(Action::Forward(std::mem::take(bytes)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_forwarding_commands_and_prompts_across_reads() {
        let mut input = Input::new(2, false);
        assert_eq!(
            input.advance(b"abc"),
            vec![Action::Forward(b"abc".to_vec())]
        );
        assert!(input.advance(b"\x02\x1b[").is_empty());
        assert_eq!(
            input.advance(b"D\x02\x1b[1;5C"),
            vec![
                Action::Focus(Direction::Left),
                Action::Resize(Direction::Right)
            ]
        );
        assert_eq!(
            input.advance(b"\x02x"),
            vec![Action::Status("close pane? (y/n)".into())]
        );
        assert_eq!(input.advance(b"y"), vec![Action::ClosePane]);
        assert_eq!(
            input.advance(b"\x02Slog.txt\r"),
            vec![
                Action::Status("save scrollback file: ".into()),
                Action::Status("save scrollback file: l".into()),
                Action::Status("save scrollback file: lo".into()),
                Action::Status("save scrollback file: log".into()),
                Action::Status("save scrollback file: log.".into()),
                Action::Status("save scrollback file: log.t".into()),
                Action::Status("save scrollback file: log.tx".into()),
                Action::Status("save scrollback file: log.txt".into()),
                Action::SaveScrollback("log.txt".into()),
            ]
        );
    }

    #[test]
    fn maps_every_single_byte_prefix_command() {
        for (bytes, action) in [
            (&b"\x02\x02"[..], Action::Forward(vec![2])),
            (b"\x02c", Action::CreateTab),
            (b"\x02n", Action::NextTab),
            (b"\x02p", Action::PreviousTab),
            (b"\x021", Action::SelectTab(0)),
            (b"\x020", Action::SelectTab(9)),
            (b"\x02|", Action::Split(Split::LeftRight)),
            (b"\x02-", Action::Split(Split::TopBottom)),
            (b"\x02d", Action::Detach),
            (b"\x02C", Action::ClearScrollback),
            (b"\x02[", Action::ScrollView),
            (b"\x02v", Action::ViewPrompt(false)),
            (b"\x02V", Action::ViewPrompt(false)),
        ] {
            let mut input = Input::new(2, false);
            assert_eq!(input.advance(bytes), vec![action]);
        }
        let mut input = Input::new(2, false);
        assert_eq!(input.advance(b"\x02["), vec![Action::ScrollView]);
        assert_eq!(input.advance(b"q"), vec![Action::ExitScrollView]);
        assert_eq!(input.advance(b"\x02?"), vec![Action::HelpView]);
        assert_eq!(input.advance(b"q"), vec![Action::ExitHelpView]);
        assert_eq!(
            input.advance(b"\x02xN"),
            vec![
                Action::Status("close pane? (y/n)".into()),
                Action::Status("close cancelled".into())
            ]
        );
        assert_eq!(
            input.advance(b"\x02Signored\x1b"),
            vec![
                Action::Status("save scrollback file: ".into()),
                Action::Status("save scrollback file: i".into()),
                Action::Status("save scrollback file: ig".into()),
                Action::Status("save scrollback file: ign".into()),
                Action::Status("save scrollback file: igno".into()),
                Action::Status("save scrollback file: ignor".into()),
                Action::Status("save scrollback file: ignore".into()),
                Action::Status("save scrollback file: ignored".into()),
                Action::Status("scrollback export cancelled".into()),
            ]
        );
    }

    #[test]
    fn viewer_prompt_and_navigation_are_consumed() {
        let mut input = Input::new(2, false);
        assert_eq!(input.advance(b"\x02v"), vec![Action::ViewPrompt(false)]);
        assert!(input.advance(b"\x1b").is_empty());
        assert_eq!(
            input.advance(b"[A\x1b[B\x1b[C\x1b[D\x0e\x10"),
            vec![
                Action::ViewSelect(-1),
                Action::ViewSelect(1),
                Action::OpenViewer(Vec::new()),
                Action::ViewParent,
                Action::ViewSelect(1),
                Action::ViewSelect(-1),
            ]
        );
        assert_eq!(
            input.advance(b"log"),
            vec![
                Action::ViewQuery(b"l".to_vec()),
                Action::ViewQuery(b"lo".to_vec()),
                Action::ViewQuery(b"log".to_vec()),
            ]
        );
        assert_eq!(
            input.advance(b"\t"),
            vec![Action::ViewComplete(b"log".to_vec())]
        );
        assert_eq!(
            input.advance(b"\\"),
            vec![Action::ViewDirectory {
                query: b"log".to_vec(),
                separator: b'\\',
            }]
        );
        assert_eq!(
            input.advance(b"\r"),
            vec![Action::OpenViewer(b"log".to_vec())]
        );
        input.enter_viewer();
        assert_eq!(
            input.advance(b"jkG/q"),
            vec![
                Action::ViewerScroll(1),
                Action::ViewerScroll(-1),
                Action::ViewerBottom,
                Action::ViewerSearchPrompt(true),
                Action::ViewerSearchQuery(b"q".to_vec(), true),
                Action::Status("/q".into()),
            ]
        );
        assert_eq!(
            input.advance(b"\r"),
            vec![Action::ViewerSearch(b"q".to_vec(), true)]
        );
    }

    #[test]
    fn viewer_uses_vim_navigation_and_prefix_kill() {
        let mut input = Input::new(2, false);
        input.enter_viewer();
        assert_eq!(
            input.advance(b"0$G"),
            vec![
                Action::ViewerLineStart,
                Action::ViewerLineEnd,
                Action::ViewerBottom,
            ]
        );
        assert!(input.advance(b"g").is_empty());
        assert_eq!(input.advance(b"g"), vec![Action::ViewerTop]);
        assert_eq!(
            input.advance(b"\x15\x04\x06\x05\x19"),
            vec![
                Action::ViewerHalfPage(false),
                Action::ViewerHalfPage(true),
                Action::ViewerPage(true),
                Action::ViewerViewport(1),
                Action::ViewerViewport(-1),
            ]
        );
        assert_eq!(
            input.advance(b"\x1b[H\x1b[F\x1b[1;5H\x1b[1;5F"),
            vec![
                Action::ViewerLineStart,
                Action::ViewerLineEnd,
                Action::ViewerTop,
                Action::ViewerBottom,
            ]
        );
        assert!(input.advance(b"\x02").is_empty());
        input.pending_since = Some(Instant::now() - VIEWER_PREFIX_TIMEOUT);
        assert_eq!(input.flush_pending_mouse(), vec![Action::ViewerPage(false)]);
        assert_eq!(
            input.advance(b"\x02x"),
            vec![Action::Status("close viewer tab? (y/n)".into())]
        );
        assert_eq!(
            input.advance(b"n"),
            vec![Action::Status("close cancelled".into())]
        );
        assert!(matches!(
            input.advance(b"q").as_slice(),
            [Action::Status(_)]
        ));
        assert!(input.advance(b"\x1b").is_empty());
        input.pending_since = Some(Instant::now() - ESCAPE_SEQUENCE_TIMEOUT);
        assert!(input.flush_pending_mouse().is_empty());
        assert_eq!(input.advance(b"\x02v"), vec![Action::ViewPrompt(true)]);
        assert_eq!(input.advance(b"\x03"), vec![Action::ViewCancelled]);
        assert_eq!(input.advance(b"\x02V"), vec![Action::ViewPrompt(true)]);
        assert_eq!(input.advance(b"\x03"), vec![Action::ViewCancelled]);
        assert_eq!(input.advance(b"j"), vec![Action::ViewerScroll(1)]);
    }

    #[test]
    fn scroll_view_consumes_navigation_until_exit() {
        let mut input = Input::new(2, false);
        assert_eq!(input.advance(b"\x02["), vec![Action::ScrollView]);
        assert!(input.advance(b"\x1b").is_empty());
        input.pending_since = Some(Instant::now() - Duration::from_millis(50));
        assert!(input.flush_pending_mouse().is_empty());
        assert_eq!(
            input.advance(b"OA\x1b[Ajk"),
            vec![
                Action::Scroll(1),
                Action::Scroll(1),
                Action::Scroll(-1),
                Action::Scroll(1),
            ]
        );
        assert!(input.advance(b"\x1b").is_empty());
        assert_eq!(input.advance(b"[5~"), vec![Action::Scroll(i32::MAX)]);
        assert!(input.advance(b"\x1b").is_empty());
        assert_eq!(
            input.advance(b"[6~gG/error\rnN"),
            vec![
                Action::Scroll(i32::MIN),
                Action::ScrollTop,
                Action::ScrollBottom,
                Action::Status("/".into()),
                Action::Status("/e".into()),
                Action::Status("/er".into()),
                Action::Status("/err".into()),
                Action::Status("/erro".into()),
                Action::Status("/error".into()),
                Action::Search("error".into()),
                Action::SearchNext(true),
                Action::SearchNext(false),
            ]
        );
        assert!(input.advance(b"\x1b").is_empty());
        input.pending_since = Some(Instant::now() - ESCAPE_SEQUENCE_TIMEOUT);
        assert_eq!(input.flush_pending_mouse(), vec![Action::ExitScrollView]);
        assert_eq!(input.advance(b"x"), vec![Action::Forward(b"x".to_vec())]);
    }

    #[test]
    fn help_view_pages_and_escape_exits() {
        let mut input = Input::new(2, false);
        assert_eq!(input.advance(b"\x02?"), vec![Action::HelpView]);
        assert_eq!(
            input.advance(b"j\x1b[5~"),
            vec![Action::HelpScroll(1), Action::HelpScroll(i32::MIN)]
        );
        assert!(input.advance(b"\x1b").is_empty());
        input.pending_since = Some(Instant::now() - ESCAPE_SEQUENCE_TIMEOUT);
        assert_eq!(input.flush_pending_mouse(), vec![Action::ExitHelpView]);
    }

    #[test]
    fn resize_mode_repeats_until_escape() {
        let mut input = Input::new(2, false);
        assert_eq!(
            input.advance(b"\x02r"),
            vec![Action::Status(
                "resize mode: arrows resize, Esc exits".into()
            )]
        );
        assert_eq!(
            input.advance(b"\x1b[C\x1b[A"),
            vec![
                Action::Resize(Direction::Right),
                Action::Status("resize mode: arrows resize, Esc exits".into()),
                Action::Resize(Direction::Up),
                Action::Status("resize mode: arrows resize, Esc exits".into()),
            ]
        );
        assert!(input.advance(b"\x1b").is_empty());
        input.pending_since = Some(Instant::now() - ESCAPE_SEQUENCE_TIMEOUT);
        assert_eq!(
            input.flush_pending_mouse(),
            vec![Action::Status("resize mode ended".into())]
        );
        assert_eq!(input.advance(b"x"), vec![Action::Forward(b"x".to_vec())]);
    }

    #[test]
    fn application_cursor_arrows_drive_termfold_modes() {
        let mut input = Input::new(2, false);
        assert_eq!(
            input.advance(b"\x02\x1bOA"),
            vec![Action::Focus(Direction::Up)]
        );
        assert_eq!(
            input.advance(b"\x02r"),
            vec![Action::Status(
                "resize mode: arrows resize, Esc exits".into()
            )]
        );
        assert_eq!(
            input.advance(b"\x1bOD"),
            vec![
                Action::Resize(Direction::Left),
                Action::Status("resize mode: arrows resize, Esc exits".into())
            ]
        );
    }

    #[test]
    fn parses_bounded_sgr_mouse_across_reads() {
        let mut input = Input::new(2, true);
        assert!(input.advance(b"\x1b[<0;12").is_empty());
        assert_eq!(
            input.advance(b";7M"),
            vec![Action::Mouse(MouseEvent {
                code: 0,
                x: 11,
                y: 6,
                release: false,
            })]
        );
        assert_eq!(
            input.advance(b"\x1b[<64;2;3M"),
            vec![Action::Mouse(MouseEvent {
                code: 64,
                x: 1,
                y: 2,
                release: false,
            })]
        );
        assert_eq!(
            input.advance(b"\x1b[A"),
            vec![Action::Forward(b"\x1b[A".to_vec())]
        );
        assert_eq!(
            Input::new(2, false).advance(b"\x1b[<0;12;7M"),
            vec![Action::Forward(b"\x1b[<0;12;7M".to_vec())]
        );
    }
}
