use std::{
    collections::VecDeque,
    error::Error,
    fmt,
    io::{self, Read, Write},
};

pub const PROTOCOL_VERSION: u8 = 2;
pub const MAX_FRAME_SIZE: usize = 1024 * 1024;
pub const MAX_QUEUE_ITEMS: usize = 256;
pub const MAX_QUEUE_BYTES: usize = 4 * 1024 * 1024;

const PREFIX_SIZE: usize = 4;
const HEADER_SIZE: usize = 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Message {
    Attach {
        columns: u16,
        rows: u16,
        profile: u8,
        color: u8,
    },
    Detach,
    Input(Vec<u8>),
    Resize {
        columns: u16,
        rows: u16,
    },
    Attached,
    Screen(Vec<u8>),
    Error(String),
    StatusRequest,
    Status {
        pid: u32,
        attached_clients: u32,
    },
    Kill,
    Terminating,
}

#[derive(Debug)]
pub enum ProtocolError {
    Io(io::Error),
    FrameTooLarge,
    Malformed(&'static str),
    UnsupportedVersion(u8),
    UnsupportedMessage(u8),
    QueueFull,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "IPC error: {error}"),
            Self::FrameTooLarge => formatter.write_str("IPC frame exceeds 1 MiB"),
            Self::Malformed(reason) => write!(formatter, "malformed IPC frame: {reason}"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported IPC protocol version {version}")
            }
            Self::UnsupportedMessage(kind) => write!(formatter, "unsupported IPC message {kind}"),
            Self::QueueFull => formatter.write_str("IPC client queue limit reached"),
        }
    }
}

impl Error for ProtocolError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ProtocolError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn read_message(reader: &mut impl Read) -> Result<Option<Message>, ProtocolError> {
    let mut prefix = [0; PREFIX_SIZE];
    match reader.read(&mut prefix[..1]) {
        Ok(0) => return Ok(None),
        Ok(_) => reader.read_exact(&mut prefix[1..])?,
        Err(error) => return Err(error.into()),
    }

    let body_len = u32::from_be_bytes(prefix) as usize;
    if body_len < HEADER_SIZE {
        return Err(ProtocolError::Malformed("frame is too short"));
    }
    if body_len > MAX_FRAME_SIZE - PREFIX_SIZE {
        return Err(ProtocolError::FrameTooLarge);
    }

    let mut body = vec![0; body_len];
    reader.read_exact(&mut body)?;
    decode(&body).map(Some)
}

pub fn write_message(writer: &mut impl Write, message: &Message) -> Result<(), ProtocolError> {
    let frame = encode(message)?;
    writer.write_all(&frame)?;
    Ok(())
}

#[derive(Debug, Default)]
pub struct OutboundQueue {
    frames: VecDeque<Vec<u8>>,
    bytes: usize,
}

impl OutboundQueue {
    pub fn push(&mut self, message: &Message) -> Result<(), ProtocolError> {
        let frame = encode(message)?;
        if self.frames.len() == MAX_QUEUE_ITEMS || self.bytes + frame.len() > MAX_QUEUE_BYTES {
            return Err(ProtocolError::QueueFull);
        }
        self.bytes += frame.len();
        self.frames.push_back(frame);
        Ok(())
    }

    pub fn write_next(&mut self, writer: &mut impl Write) -> Result<bool, ProtocolError> {
        let Some(frame) = self.frames.front() else {
            return Ok(false);
        };
        writer.write_all(frame)?;
        self.bytes -= frame.len();
        self.frames.pop_front();
        Ok(true)
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

fn encode(message: &Message) -> Result<Vec<u8>, ProtocolError> {
    let (kind, payload): (u8, &[u8]) = match message {
        Message::Attach { .. } => (1, &[]),
        Message::Detach => (2, &[]),
        Message::Input(payload) => (3, payload),
        Message::Resize { .. } => (4, &[]),
        Message::Attached => (5, &[]),
        Message::Screen(payload) => (6, payload),
        Message::Error(message) => (7, message.as_bytes()),
        Message::StatusRequest => (8, &[]),
        Message::Status { .. } => (9, &[]),
        Message::Kill => (10, &[]),
        Message::Terminating => (11, &[]),
    };
    let fixed_len = match message {
        Message::Attach { .. } => 6,
        Message::Resize { .. } => 4,
        Message::Status { .. } => 8,
        _ => 0,
    };
    let body_len = HEADER_SIZE + payload.len() + fixed_len;
    if body_len > MAX_FRAME_SIZE - PREFIX_SIZE {
        return Err(ProtocolError::FrameTooLarge);
    }

    let mut frame = Vec::with_capacity(PREFIX_SIZE + body_len);
    frame.extend_from_slice(&(body_len as u32).to_be_bytes());
    frame.extend_from_slice(&[PROTOCOL_VERSION, kind]);
    match message {
        Message::Attach {
            columns,
            rows,
            profile,
            color,
        } => {
            valid_size(*columns, *rows)?;
            frame.extend_from_slice(&columns.to_be_bytes());
            frame.extend_from_slice(&rows.to_be_bytes());
            frame.extend_from_slice(&[*profile, *color]);
        }
        Message::Resize { columns, rows } => {
            valid_size(*columns, *rows)?;
            frame.extend_from_slice(&columns.to_be_bytes());
            frame.extend_from_slice(&rows.to_be_bytes());
        }
        Message::Status {
            pid,
            attached_clients,
        } => {
            frame.extend_from_slice(&pid.to_be_bytes());
            frame.extend_from_slice(&attached_clients.to_be_bytes());
        }
        _ => frame.extend_from_slice(payload),
    }
    Ok(frame)
}

fn decode(body: &[u8]) -> Result<Message, ProtocolError> {
    if body[0] != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion(body[0]));
    }
    let payload = &body[HEADER_SIZE..];
    match body[1] {
        1 => {
            if payload.len() != 6 {
                return Err(ProtocolError::Malformed(
                    "attach must contain terminal size and profile",
                ));
            }
            let (columns, rows) = decode_size(&payload[..4])?;
            Ok(Message::Attach {
                columns,
                rows,
                profile: payload[4],
                color: payload[5],
            })
        }
        2 if payload.is_empty() => Ok(Message::Detach),
        3 => Ok(Message::Input(payload.to_vec())),
        4 => {
            let (columns, rows) = decode_size(payload)?;
            Ok(Message::Resize { columns, rows })
        }
        5 if payload.is_empty() => Ok(Message::Attached),
        6 => Ok(Message::Screen(payload.to_vec())),
        7 => String::from_utf8(payload.to_vec())
            .map(Message::Error)
            .map_err(|_| ProtocolError::Malformed("error message is not UTF-8")),
        8 if payload.is_empty() => Ok(Message::StatusRequest),
        9 if payload.len() == 8 => Ok(Message::Status {
            pid: u32::from_be_bytes(payload[..4].try_into().expect("checked length")),
            attached_clients: u32::from_be_bytes(payload[4..].try_into().expect("checked length")),
        }),
        10 if payload.is_empty() => Ok(Message::Kill),
        11 if payload.is_empty() => Ok(Message::Terminating),
        2 | 5 | 8 | 9 | 10 | 11 => Err(ProtocolError::Malformed(
            "message has an unexpected payload",
        )),
        kind => Err(ProtocolError::UnsupportedMessage(kind)),
    }
}

fn decode_size(payload: &[u8]) -> Result<(u16, u16), ProtocolError> {
    if payload.len() != 4 {
        return Err(ProtocolError::Malformed(
            "terminal size must contain four bytes",
        ));
    }
    let columns = u16::from_be_bytes([payload[0], payload[1]]);
    let rows = u16::from_be_bytes([payload[2], payload[3]]);
    valid_size(columns, rows)?;
    Ok((columns, rows))
}

fn valid_size(columns: u16, rows: u16) -> Result<(), ProtocolError> {
    if columns == 0 || rows == 0 {
        Err(ProtocolError::Malformed("terminal size must be non-zero"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn messages_round_trip_and_invalid_frames_are_rejected() {
        let messages = [
            Message::Attach {
                columns: 80,
                rows: 24,
                profile: 5,
                color: 2,
            },
            Message::Input(vec![0, 1, 255]),
            Message::Resize {
                columns: 120,
                rows: 40,
            },
            Message::Screen(b"screen".to_vec()),
            Message::Error("failed".into()),
            Message::StatusRequest,
            Message::Status {
                pid: 1234,
                attached_clients: 2,
            },
            Message::Kill,
            Message::Terminating,
            Message::Detach,
        ];

        for message in messages {
            let frame = encode(&message).unwrap();
            assert!(frame.len() <= MAX_FRAME_SIZE);
            assert_eq!(
                read_message(&mut Cursor::new(frame)).unwrap(),
                Some(message)
            );
        }

        let mut wrong_version = encode(&Message::Detach).unwrap();
        wrong_version[PREFIX_SIZE] += 1;
        assert!(matches!(
            read_message(&mut Cursor::new(wrong_version)),
            Err(ProtocolError::UnsupportedVersion(_))
        ));
        assert!(matches!(
            encode(&Message::Screen(vec![0; MAX_FRAME_SIZE])),
            Err(ProtocolError::FrameTooLarge)
        ));
        assert!(matches!(
            read_message(&mut Cursor::new([0, 0, 0, 1, PROTOCOL_VERSION])),
            Err(ProtocolError::Malformed(_))
        ));
    }

    #[test]
    fn client_queue_limits_are_independent() {
        let message = Message::Screen(vec![0; MAX_FRAME_SIZE - PREFIX_SIZE - HEADER_SIZE]);
        let mut slow_client = OutboundQueue::default();
        for _ in 0..4 {
            slow_client.push(&message).unwrap();
        }
        assert_eq!(slow_client.bytes(), MAX_QUEUE_BYTES);
        assert!(matches!(
            slow_client.push(&Message::Detach),
            Err(ProtocolError::QueueFull)
        ));

        let mut other_client = OutboundQueue::default();
        other_client.push(&Message::Detach).unwrap();
        assert_eq!(other_client.len(), 1);
    }
}
