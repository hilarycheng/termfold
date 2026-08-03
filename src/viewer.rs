use std::{
    collections::VecDeque,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use unicode_width::UnicodeWidthChar;

use crate::{session::Size, terminal::Terminal};

const BLOCK_SIZE: u64 = 64 * 1024;
const BLOCK_CACHE_SIZE: usize = 8;
const MAX_MATCH_OFFSETS: usize = 4096;
const MAX_LINE_BYTES: usize = 64 * 1024;

#[derive(Debug)]
struct Block {
    offset: u64,
    bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
struct SearchState {
    query: Vec<u8>,
    forward: bool,
    offset: u64,
}

#[derive(Debug)]
pub struct Viewer {
    path: PathBuf,
    file: File,
    length: u64,
    position: u64,
    horizontal: u64,
    blocks: VecDeque<Block>,
    matches: Vec<u64>,
    search: Option<SearchState>,
    page: Vec<String>,
}

impl Viewer {
    pub fn open(path: PathBuf) -> io::Result<Self> {
        let file = File::open(&path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "viewer target is not a regular file",
            ));
        }
        Ok(Self {
            path,
            file,
            length: metadata.len(),
            position: 0,
            horizontal: 0,
            blocks: VecDeque::new(),
            matches: Vec::new(),
            search: None,
            page: Vec::new(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn render(&mut self, terminal: &mut Terminal, size: Size) -> io::Result<()> {
        terminal.resize(size).map_err(io::Error::other)?;
        self.page.clear();
        let mut position = self.position;
        let rows = usize::from(size.rows);
        let columns = usize::from(size.columns);
        let mut budget = columns.saturating_mul(rows).saturating_mul(4);
        for _ in 0..rows {
            if position >= self.length || budget == 0 {
                break;
            }
            let (next, line) = self.read_line(position, columns, budget)?;
            let used = line.len().min(budget);
            budget = budget.saturating_sub(used);
            self.page.push(line);
            if next <= position {
                break;
            }
            position = next;
        }

        terminal.advance(b"\x1b[2J\x1b[H");
        for line in &self.page {
            terminal.advance(line.as_bytes());
            terminal.advance(b"\r\n");
        }
        Ok(())
    }

    pub fn move_lines(&mut self, amount: i32) -> io::Result<()> {
        if amount > 0 {
            for _ in 0..amount.unsigned_abs() {
                self.position = self.next_line(self.position)?;
            }
        } else {
            for _ in 0..amount.unsigned_abs() {
                self.position = self.previous_line(self.position)?;
            }
        }
        Ok(())
    }

    pub fn page(&mut self, rows: u16, forward: bool) -> io::Result<()> {
        let lines = usize::from(rows.saturating_sub(1).max(1));
        let amount = i32::try_from(lines).unwrap_or(i32::MAX);
        self.move_lines(if forward { amount } else { -amount })
    }

    pub fn half_page(&mut self, rows: u16, forward: bool) -> io::Result<()> {
        let lines = usize::from(rows.saturating_sub(1).max(1)) / 2;
        let amount = i32::try_from(lines.max(1)).unwrap_or(i32::MAX);
        self.move_lines(if forward { amount } else { -amount })
    }

    pub fn top(&mut self) {
        self.position = 0;
        self.horizontal = 0;
    }

    pub fn bottom(&mut self) -> io::Result<()> {
        self.position = self.last_line()?;
        self.horizontal = 0;
        Ok(())
    }

    pub fn line_start(&mut self) -> io::Result<()> {
        self.position = self.line_start_at(self.position)?;
        self.horizontal = 0;
        Ok(())
    }

    pub fn line_end(&mut self, columns: usize) -> io::Result<()> {
        let start = self.line_start_at(self.position)?;
        let mut end = start;
        let mut length: u64 = 0;
        while let Some(byte) = self.byte(end)? {
            end += 1;
            if byte == b'\n' {
                break;
            }
            length += 1;
        }
        self.horizontal = length.saturating_sub(columns.max(1) as u64);
        Ok(())
    }

    pub fn search(&mut self, query: &str, forward: bool) -> io::Result<bool> {
        if query.is_empty() {
            return Ok(false);
        }
        let query = query.as_bytes().to_vec();
        let found = self.search_from(&query, forward, self.position)?;
        self.search = found.map(|offset| SearchState {
            query,
            forward,
            offset,
        });
        Ok(found.is_some())
    }

    pub fn repeat_search(&mut self, same_direction: bool) -> io::Result<bool> {
        let Some(previous) = self.search.clone() else {
            return Ok(false);
        };
        let forward = if same_direction {
            previous.forward
        } else {
            !previous.forward
        };
        let start = if forward {
            previous.offset.saturating_add(1)
        } else {
            previous.offset.saturating_sub(1)
        };
        if let Some(offset) = self.cached_match(start, forward) {
            self.position = self.line_start_at(offset)?;
            self.horizontal = 0;
            self.search = Some(SearchState {
                query: previous.query,
                forward,
                offset,
            });
            return Ok(true);
        }
        let query = previous.query;
        let found = self.search_from(&query, forward, start)?;
        self.search = found.map(|offset| SearchState {
            query,
            forward,
            offset,
        });
        Ok(found.is_some())
    }

    fn search_from(&mut self, query: &[u8], forward: bool, start: u64) -> io::Result<Option<u64>> {
        self.matches.clear();
        let maximum = self.length.checked_sub(query.len() as u64);
        let Some(maximum) = maximum else {
            return Ok(None);
        };
        let start = start.min(maximum);
        let found = if forward {
            let found = self.collect_forward(query, start, maximum)?;
            if found.is_some() || start == 0 {
                found
            } else {
                self.collect_forward(query, 0, start - 1)?
            }
        } else {
            let found = self.collect_reverse(query, start, 0)?;
            if found.is_some() || start == maximum {
                found
            } else {
                self.collect_reverse(query, maximum, start + 1)?
            }
        };
        if let Some(offset) = found {
            self.position = self.line_start_at(offset)?;
            self.horizontal = 0;
        }
        Ok(found)
    }

    fn collect_forward(&mut self, query: &[u8], start: u64, end: u64) -> io::Result<Option<u64>> {
        let mut found = None;
        let mut offset = start;
        loop {
            if self.matches_at(offset, query)? {
                found.get_or_insert(offset);
                if self.matches.len() < MAX_MATCH_OFFSETS {
                    self.matches.push(offset);
                }
            }
            if offset == end {
                break;
            }
            offset += 1;
        }
        Ok(found)
    }

    fn collect_reverse(&mut self, query: &[u8], start: u64, end: u64) -> io::Result<Option<u64>> {
        let mut found = None;
        let mut offset = start;
        loop {
            if self.matches_at(offset, query)? {
                found.get_or_insert(offset);
                if self.matches.len() < MAX_MATCH_OFFSETS {
                    self.matches.push(offset);
                }
            }
            if offset == end {
                break;
            }
            offset = offset.saturating_sub(1);
        }
        Ok(found)
    }

    fn cached_match(&self, start: u64, forward: bool) -> Option<u64> {
        if forward {
            self.matches.iter().copied().find(|offset| *offset >= start)
        } else {
            self.matches
                .iter()
                .copied()
                .filter(|offset| *offset <= start)
                .max()
        }
    }

    fn matches_at(&mut self, offset: u64, query: &[u8]) -> io::Result<bool> {
        for (index, byte) in query.iter().enumerate() {
            if self.byte(offset + index as u64)? != Some(*byte) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn read_line(
        &mut self,
        start: u64,
        columns: usize,
        budget: usize,
    ) -> io::Result<(u64, String)> {
        let limit = MAX_LINE_BYTES.min(columns.saturating_mul(4).saturating_add(4));
        let mut bytes = Vec::with_capacity(limit.min(budget));
        let mut position = start;
        let mut skipped = 0;
        let mut line_ended = false;
        while skipped < self.horizontal {
            let Some(byte) = self.byte(position)? else {
                break;
            };
            position += 1;
            if byte == b'\n' {
                line_ended = true;
                break;
            }
            skipped += 1;
        }
        if line_ended {
            return Ok((position, String::new()));
        }
        while let Some(byte) = self.byte(position)? {
            position += 1;
            if byte == b'\n' {
                break;
            }
            if bytes.len() < limit && bytes.len() < budget {
                bytes.push(byte);
            }
        }
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        Ok((position, display_line(&bytes, columns)))
    }

    fn next_line(&mut self, start: u64) -> io::Result<u64> {
        let mut position = start;
        while let Some(byte) = self.byte(position)? {
            position += 1;
            if byte == b'\n' {
                break;
            }
        }
        Ok(position)
    }

    fn previous_line(&mut self, start: u64) -> io::Result<u64> {
        if start == 0 {
            return Ok(0);
        }
        let mut position = start - 1;
        if self.byte(position)? == Some(b'\n') {
            position = position.saturating_sub(1);
        }
        while position > 0 && self.byte(position - 1)? != Some(b'\n') {
            position -= 1;
        }
        Ok(position)
    }

    fn last_line(&mut self) -> io::Result<u64> {
        if self.length == 0 {
            return Ok(0);
        }
        let mut position = self.length;
        if self.byte(position - 1)? == Some(b'\n') {
            position -= 1;
        }
        while position > 0 && self.byte(position - 1)? != Some(b'\n') {
            position -= 1;
        }
        Ok(position)
    }

    fn line_start_at(&mut self, position: u64) -> io::Result<u64> {
        let mut position = position.min(self.length);
        while position > 0 && self.byte(position - 1)? != Some(b'\n') {
            position -= 1;
        }
        Ok(position)
    }

    fn byte(&mut self, offset: u64) -> io::Result<Option<u8>> {
        if offset >= self.length {
            return Ok(None);
        }
        let block_offset = offset / BLOCK_SIZE * BLOCK_SIZE;
        if let Some(index) = self
            .blocks
            .iter()
            .position(|block| block.offset == block_offset)
        {
            let block = self
                .blocks
                .remove(index)
                .expect("block index came from cache");
            let byte = block.bytes[(offset - block_offset) as usize];
            self.blocks.push_front(block);
            return Ok(Some(byte));
        }

        self.file.seek(SeekFrom::Start(block_offset))?;
        let length = (self.length - block_offset).min(BLOCK_SIZE) as usize;
        let mut bytes = vec![0; length];
        let mut read = 0;
        while read < length {
            let count = self.file.read(&mut bytes[read..])?;
            if count == 0 {
                break;
            }
            read += count;
        }
        bytes.truncate(read);
        if bytes.is_empty() {
            return Ok(None);
        }
        let Some(byte) = bytes.get((offset - block_offset) as usize).copied() else {
            return Ok(None);
        };
        self.blocks.push_front(Block {
            offset: block_offset,
            bytes,
        });
        while self.blocks.len() > BLOCK_CACHE_SIZE {
            self.blocks.pop_back();
        }
        Ok(Some(byte))
    }
}

fn display_line(bytes: &[u8], columns: usize) -> String {
    let mut output = String::new();
    let mut width = 0;
    for character in String::from_utf8_lossy(bytes).chars() {
        let (character, character_width) = if character == '\t' {
            (' ', 8 - width % 8)
        } else if character.is_control() {
            ('�', 1)
        } else {
            (character, character.width().unwrap_or(0))
        };
        if width.saturating_add(character_width) > columns {
            break;
        }
        output.push(character);
        width += character_width;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::SystemTime};

    #[test]
    fn paging_and_literal_search_keep_line_offsets_bounded() {
        let path = std::env::temp_dir().join(format!(
            "termfold-viewer-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, b"zero\nhit one\nhit two\nlast\n").unwrap();
        let mut viewer = Viewer::open(path.clone()).unwrap();

        viewer.line_end(3).unwrap();
        assert_eq!(viewer.horizontal, 1);
        viewer.line_start().unwrap();
        assert_eq!(viewer.horizontal, 0);
        assert!(viewer.search("hit", true).unwrap());
        assert_eq!(viewer.position, 5);
        assert!(viewer.repeat_search(true).unwrap());
        assert_eq!(viewer.position, 13);
        assert!(viewer.repeat_search(false).unwrap());
        assert_eq!(viewer.position, 5);
        viewer.bottom().unwrap();
        assert_eq!(viewer.position, 21);

        fs::remove_file(path).unwrap();
    }
}
