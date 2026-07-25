use std::time::{Duration, Instant};

use crate::session::{Direction, Split};

const MAX_FILENAME_BYTES: usize = 4096;
const MAX_MOUSE_SEQUENCE_BYTES: usize = 32;
const MOUSE_SEQUENCE_TIMEOUT: Duration = Duration::from_millis(10);

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
    ScrollView,
    Scroll(i32),
    ExitScrollView,
    ClearScrollback,
    SaveScrollback(String),
    Mouse(MouseEvent),
    Status(String),
}

#[derive(Debug)]
enum Mode {
    Normal,
    Prefix(Vec<u8>),
    ConfirmClose,
    Filename(Vec<u8>),
    Resize(Vec<u8>),
    Scroll(Vec<u8>),
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

    pub fn advance(&mut self, bytes: &[u8]) -> Vec<Action> {
        if self.pending_mouse.is_empty() && bytes == b"\x1b" {
            match self.mode {
                Mode::Resize(_) => {
                    self.mode = Mode::Normal;
                    return vec![Action::Status("resize mode ended".into())];
                }
                Mode::Scroll(_) => {
                    self.mode = Mode::Normal;
                    return vec![Action::ExitScrollView];
                }
                _ => {}
            }
        }
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
            self.advance_byte(input[offset], &mut actions, &mut forwarded);
            offset += 1;
        }
        push_forward(&mut actions, &mut forwarded);
        actions
    }

    pub fn flush_pending_mouse(&mut self) -> Vec<Action> {
        if self
            .pending_since
            .is_none_or(|since| since.elapsed() < MOUSE_SEQUENCE_TIMEOUT)
        {
            return Vec::new();
        }
        self.pending_since = None;
        let input = std::mem::take(&mut self.pending_mouse);
        let mut actions = Vec::new();
        let mut forwarded = Vec::new();
        for byte in input {
            self.advance_byte(byte, &mut actions, &mut forwarded);
        }
        push_forward(&mut actions, &mut forwarded);
        actions
    }

    fn advance_byte(&mut self, byte: u8, actions: &mut Vec<Action>, forwarded: &mut Vec<u8>) {
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
                        [b'x'] => Mode::ConfirmClose,
                        [b'S'] => Mode::Filename(Vec::new()),
                        [b'r'] => Mode::Resize(Vec::new()),
                        [b'['] => Mode::Scroll(Vec::new()),
                        _ => Mode::Normal,
                    };
                    actions.push(action);
                }
            }
            Mode::ConfirmClose => {
                actions.push(if matches!(byte, b'y' | b'Y') {
                    Action::ClosePane
                } else {
                    Action::Status("close cancelled".into())
                });
                self.mode = Mode::Normal;
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
                    [27, b'[', final_byte @ (b'A'..=b'D')] => {
                        Some(Action::Resize(direction(*final_byte)))
                    }
                    [27] | [27, b'['] => None,
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
            Mode::Scroll(sequence) => {
                sequence.push(byte);
                let action = match sequence.as_slice() {
                    [b'q' | 3] => Some(Action::ExitScrollView),
                    [b'k'] | [27, b'[', b'A'] => Some(Action::Scroll(1)),
                    [b'j'] | [27, b'[', b'B'] => Some(Action::Scroll(-1)),
                    [27, b'[', b'5', b'~'] => Some(Action::Scroll(i32::MAX)),
                    [27, b'[', b'6', b'~'] => Some(Action::Scroll(i32::MIN)),
                    [27] | [27, b'['] | [27, b'[', b'5' | b'6'] => None,
                    _ => Some(Action::Status(
                        "scroll view: arrows/Page Up/Page Down, q exits".into(),
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
        }
    }
}

enum MouseParse {
    Complete(MouseEvent, usize),
    Incomplete,
    Invalid,
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
            b'[' => Action::ScrollView,
            b'C' => Action::ClearScrollback,
            b'S' => Action::Status("save scrollback file: ".into()),
            _ => Action::Status("unsupported prefix command".into()),
        });
    }
    match sequence {
        [27]
        | [27, b'[']
        | [27, b'[', b'1']
        | [27, b'[', b'1', b';']
        | [27, b'[', b'1', b';', b'5'] => None,
        [27, b'[', final_byte @ (b'A'..=b'D')] => Some(Action::Focus(direction(*final_byte))),
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
        let mut input = Input::new(2, false);
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
        ] {
            assert_eq!(input.advance(bytes), vec![action]);
        }
        assert_eq!(input.advance(b"q"), vec![Action::ExitScrollView]);
        assert_eq!(
            input.advance(b"\x02?"),
            vec![Action::Status("unsupported prefix command".into())]
        );
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
    fn scroll_view_consumes_navigation_until_exit() {
        let mut input = Input::new(2, false);
        assert_eq!(input.advance(b"\x02["), vec![Action::ScrollView]);
        assert_eq!(
            input.advance(b"\x1b[A\x1b[6~"),
            vec![Action::Scroll(1), Action::Scroll(i32::MIN)]
        );
        assert_eq!(input.advance(b"q"), vec![Action::ExitScrollView]);
        assert_eq!(input.advance(b"x"), vec![Action::Forward(b"x".to_vec())]);
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
        assert_eq!(
            input.advance(b"\x1b"),
            vec![Action::Status("resize mode ended".into())]
        );
        assert_eq!(input.advance(b"x"), vec![Action::Forward(b"x".to_vec())]);
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
