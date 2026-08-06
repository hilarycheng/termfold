use std::io;

use super::source::{BLOCK_SIZE, FileSource};

#[derive(Clone, Copy, Debug)]
pub(super) struct LineBoundary {
    pub(super) start: u64,
    pub(super) content_end: u64,
    pub(super) next: u64,
    pub(super) complete: bool,
    pub(super) resume: Option<LineScanner>,
}

#[derive(Clone, Copy, Debug)]
enum Direction {
    Forward,
    Reverse,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct LineScanner {
    direction: Direction,
    cursor: u64,
    limit: u64,
    initial: u64,
    segment_end: u64,
    skip_boundary: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum ScanStep {
    Boundary { start: u64, end: u64 },
    Yield { position: u64, content_end: u64 },
    Done { position: u64 },
}

impl LineScanner {
    pub(super) fn forward(start: u64, length: u64) -> Self {
        let start = start.min(length);
        Self {
            direction: Direction::Forward,
            cursor: start,
            limit: length,
            initial: start,
            segment_end: start,
            skip_boundary: false,
        }
    }

    pub(super) fn reverse(start: u64, end: u64, skip_boundary: bool) -> Self {
        Self {
            direction: Direction::Reverse,
            cursor: start,
            limit: end.min(start),
            initial: start,
            segment_end: start,
            skip_boundary,
        }
    }

    pub(super) fn is_forward(self) -> bool {
        matches!(self.direction, Direction::Forward)
    }

    pub(super) fn content_end(self) -> u64 {
        self.segment_end
    }

    pub(super) fn step(&mut self, source: &mut FileSource) -> io::Result<ScanStep> {
        match self.direction {
            Direction::Forward => self.step_forward(source),
            Direction::Reverse => self.step_reverse(source),
        }
    }

    fn step_forward(&mut self, source: &mut FileSource) -> io::Result<ScanStep> {
        if self.cursor >= self.limit {
            return Ok(ScanStep::Done {
                position: self.cursor.min(self.limit),
            });
        }

        let first = self.cursor;
        let last = first.saturating_add(BLOCK_SIZE).min(self.limit);
        let bytes = read_chunk(source, first, last)?;
        for index in 0..bytes.len() {
            let offset = first + index as u64;
            let Some(end) = forward_eol_end(source, &bytes, index, offset)? else {
                continue;
            };
            self.cursor = end;
            return Ok(ScanStep::Boundary { start: offset, end });
        }

        self.cursor = last;
        if self.cursor >= self.limit {
            Ok(ScanStep::Done {
                position: self.cursor,
            })
        } else {
            Ok(ScanStep::Yield {
                position: self.cursor,
                content_end: self.cursor,
            })
        }
    }

    fn step_reverse(&mut self, source: &mut FileSource) -> io::Result<ScanStep> {
        if self.cursor <= self.limit {
            return Ok(ScanStep::Done {
                position: self.cursor.max(self.limit),
            });
        }

        let block_offset = (self.cursor - 1) / BLOCK_SIZE * BLOCK_SIZE;
        let first = self.limit.max(block_offset);
        let last = self.cursor.min(block_offset.saturating_add(BLOCK_SIZE));
        let bytes = read_chunk(source, first, last)?;
        let mut offset = last;
        while offset > first {
            offset -= 1;
            let index = (offset - first) as usize;
            let Some((start, end)) = reverse_eol(source, &bytes, index, offset)? else {
                continue;
            };
            if self.skip_boundary && end == self.initial {
                self.skip_boundary = false;
                self.segment_end = start;
                if start < first {
                    self.cursor = start;
                    return Ok(ScanStep::Yield {
                        position: start,
                        content_end: self.segment_end,
                    });
                }
                offset = start;
                continue;
            }
            self.cursor = start;
            return Ok(ScanStep::Boundary { start, end });
        }

        self.cursor = first;
        if self.cursor <= self.limit {
            Ok(ScanStep::Done {
                position: self.cursor,
            })
        } else {
            Ok(ScanStep::Yield {
                position: self.cursor,
                content_end: self.segment_end,
            })
        }
    }
}

fn read_chunk(source: &mut FileSource, start: u64, end: u64) -> io::Result<Vec<u8>> {
    let bytes = source.read_range(start..end)?;
    if bytes.len() != (end - start) as usize {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "viewer source range is shorter than the snapshot",
        ));
    }
    Ok(bytes)
}

fn forward_eol_end(
    source: &mut FileSource,
    bytes: &[u8],
    index: usize,
    offset: u64,
) -> io::Result<Option<u64>> {
    match bytes[index] {
        b'\n' => Ok(Some(offset + 1)),
        b'\r' => {
            let next = match bytes.get(index + 1).copied() {
                Some(byte) => Some(byte),
                None => source.read_byte(offset.saturating_add(1))?,
            };
            Ok(Some(if next == Some(b'\n') {
                offset + 2
            } else {
                offset + 1
            }))
        }
        _ => Ok(None),
    }
}

fn reverse_eol(
    source: &mut FileSource,
    bytes: &[u8],
    index: usize,
    offset: u64,
) -> io::Result<Option<(u64, u64)>> {
    match bytes[index] {
        b'\n' => {
            let previous = if index == 0 {
                offset
                    .checked_sub(1)
                    .map_or(Ok(None), |position| source.read_byte(position))?
            } else {
                Some(bytes[index - 1])
            };
            Ok(Some(if previous == Some(b'\r') {
                (offset - 1, offset + 1)
            } else {
                (offset, offset + 1)
            }))
        }
        b'\r' => {
            let next = match bytes.get(index + 1).copied() {
                Some(byte) => Some(byte),
                None => source.read_byte(offset.saturating_add(1))?,
            };
            Ok(Some(if next == Some(b'\n') {
                (offset, offset + 2)
            } else {
                (offset, offset + 1)
            }))
        }
        _ => Ok(None),
    }
}
