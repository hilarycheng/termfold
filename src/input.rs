use crate::session::{Direction, Split};

const MAX_FILENAME_BYTES: usize = 4096;

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
    SaveScrollback(String),
    Status(String),
}

#[derive(Debug)]
enum Mode {
    Normal,
    Prefix(Vec<u8>),
    ConfirmClose,
    Filename(Vec<u8>),
}

#[derive(Debug)]
pub struct Input {
    prefix: u8,
    mode: Mode,
}

impl Input {
    pub fn new(prefix: u8) -> Self {
        Self {
            prefix,
            mode: Mode::Normal,
        }
    }

    pub fn advance(&mut self, bytes: &[u8]) -> Vec<Action> {
        let mut actions = Vec::new();
        let mut forwarded = Vec::new();
        for &byte in bytes {
            match &mut self.mode {
                Mode::Normal if byte == self.prefix => {
                    push_forward(&mut actions, &mut forwarded);
                    self.mode = Mode::Prefix(Vec::new());
                }
                Mode::Normal => forwarded.push(byte),
                Mode::Prefix(sequence) => {
                    sequence.push(byte);
                    if let Some(action) = prefix_action(self.prefix, sequence) {
                        self.mode = match sequence.as_slice() {
                            [b'x'] => Mode::ConfirmClose,
                            [b'S'] => Mode::Filename(Vec::new()),
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
            }
        }
        push_forward(&mut actions, &mut forwarded);
        actions
    }
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
            b'%' => Action::Split(Split::LeftRight),
            b'"' => Action::Split(Split::TopBottom),
            b'x' => Action::Status("close pane? (y/n)".into()),
            b'd' => Action::Detach,
            b'[' => Action::ScrollView,
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
        let mut input = Input::new(2);
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
        let mut input = Input::new(2);
        for (bytes, action) in [
            (&b"\x02\x02"[..], Action::Forward(vec![2])),
            (b"\x02c", Action::CreateTab),
            (b"\x02n", Action::NextTab),
            (b"\x02p", Action::PreviousTab),
            (b"\x021", Action::SelectTab(0)),
            (b"\x020", Action::SelectTab(9)),
            (b"\x02%", Action::Split(Split::LeftRight)),
            (b"\x02\"", Action::Split(Split::TopBottom)),
            (b"\x02d", Action::Detach),
            (b"\x02[", Action::ScrollView),
        ] {
            assert_eq!(input.advance(bytes), vec![action]);
        }
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
}
