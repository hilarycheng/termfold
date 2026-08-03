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

#[derive(Clone, Copy, Debug, Default)]
struct ViewState {
    position: u64,
    viewport: u64,
    horizontal: u64,
    preferred_column: u64,
    visible_rows: usize,
    visible_columns: usize,
}

impl ViewState {
    fn clamped(self, length: u64) -> Self {
        Self {
            position: self.position.min(length),
            viewport: self.viewport.min(length),
            ..self
        }
    }
}

#[derive(Debug)]
pub struct Viewer {
    path: PathBuf,
    file: File,
    length: u64,
    position: u64,
    viewport: u64,
    horizontal: u64,
    preferred_column: u64,
    visible_rows: usize,
    visible_columns: usize,
    blocks: VecDeque<Block>,
    matches: Vec<u64>,
    search: Option<SearchState>,
    page: Vec<String>,
    committed: ViewState,
    defer_eviction: bool,
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
        let committed = ViewState {
            visible_rows: 1,
            visible_columns: 1,
            ..ViewState::default()
        };
        Ok(Self {
            path,
            file,
            length: metadata.len(),
            position: 0,
            viewport: 0,
            horizontal: 0,
            preferred_column: 0,
            visible_rows: 1,
            visible_columns: 1,
            blocks: VecDeque::new(),
            matches: Vec::new(),
            search: None,
            page: Vec::new(),
            committed,
            defer_eviction: false,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn render(&mut self, terminal: &mut Terminal, size: Size) -> io::Result<()> {
        self.refresh_metadata()?;
        self.committed = self.committed.clamped(self.length);

        let committed = self.committed;
        let previous_page = std::mem::take(&mut self.page);
        let previous_blocks = self.cache_offsets();
        self.visible_rows = usize::from(size.rows).max(1);
        self.visible_columns = usize::from(size.columns).max(1);
        let columns = usize::from(size.columns);
        self.defer_eviction = true;
        let result = self.build_page(size);
        self.defer_eviction = false;
        let (cursor_row, cursor_line) = match result {
            Ok(page) => page,
            Err(error) => {
                self.restore_state(committed);
                self.page = previous_page;
                self.restore_cache(&previous_blocks);
                return Err(error);
            }
        };

        if let Err(error) = terminal.resize(size).map_err(io::Error::other) {
            self.restore_state(committed);
            self.page = previous_page;
            self.restore_cache(&previous_blocks);
            return Err(error);
        }

        self.committed = self.state();
        self.trim_cache();
        drop(previous_page);

        terminal.advance(b"\x1b[2J\x1b[H");
        for line in &self.page {
            terminal.advance(line.as_bytes());
            terminal.advance(b"\r\n");
        }
        let (row, column) = match cursor_row {
            Some(row) => (
                row.saturating_add(1),
                self.cursor_column(cursor_line, columns).saturating_add(1),
            ),
            None => (1, 1),
        };
        terminal.advance(format!("\x1b[{row};{column}H").as_bytes());
        Ok(())
    }

    fn build_page(&mut self, size: Size) -> io::Result<(Option<usize>, u64)> {
        self.position = self.position.min(self.length);
        self.viewport = self.viewport.min(self.length);
        self.viewport = self.line_start_at(self.viewport)?;
        let cursor_line = self.line_start_at(self.position)?;
        let mut position = self.viewport;
        let mut cursor_row = None;
        let rows = usize::from(size.rows);
        let columns = usize::from(size.columns);
        let mut budget = columns.saturating_mul(rows).saturating_mul(4);
        self.page = Vec::with_capacity(rows);
        for row in 0..rows {
            if position >= self.length || budget == 0 {
                break;
            }
            if position == cursor_line {
                cursor_row = Some(row);
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
        Ok((cursor_row, cursor_line))
    }

    pub fn move_lines(&mut self, amount: i32) -> io::Result<()> {
        self.navigation(|viewer| {
            viewer.refresh_metadata()?;
            viewer.move_lines_inner(amount)
        })
    }

    fn move_lines_inner(&mut self, amount: i32) -> io::Result<()> {
        let current_line = self.line_start_at(self.position)?;
        let preferred = self
            .preferred_column
            .max(self.position.saturating_sub(current_line));
        let mut target = current_line;
        if amount > 0 {
            for _ in 0..amount.unsigned_abs() {
                let next = self.next_line(target)?;
                if next >= self.length {
                    break;
                }
                target = next;
            }
        } else {
            for _ in 0..amount.unsigned_abs() {
                target = self.previous_line(target)?;
            }
        }
        self.position = target.saturating_add(preferred.min(self.line_length(target)?));
        self.preferred_column = preferred;
        self.adjust_horizontal();
        self.ensure_cursor_visible()?;
        Ok(())
    }

    pub fn page(&mut self, rows: u16, forward: bool) -> io::Result<()> {
        self.navigation(|viewer| {
            viewer.refresh_metadata()?;
            viewer.visible_rows = usize::from(rows).max(1);
            let lines = usize::from(rows.saturating_sub(2).max(1));
            let amount = i32::try_from(lines).unwrap_or(i32::MAX);
            viewer.move_lines_inner(if forward { amount } else { -amount })
        })
    }

    pub fn half_page(&mut self, rows: u16, forward: bool) -> io::Result<()> {
        self.navigation(|viewer| {
            viewer.refresh_metadata()?;
            viewer.visible_rows = usize::from(rows).max(1);
            let lines = usize::from(rows.saturating_sub(2).max(1)) / 2;
            let amount = i32::try_from(lines.max(1)).unwrap_or(i32::MAX);
            viewer.move_lines_inner(if forward { amount } else { -amount })
        })
    }

    pub fn top(&mut self) {
        self.position = 0;
        self.viewport = 0;
        self.horizontal = 0;
        self.preferred_column = 0;
    }

    pub fn bottom(&mut self) -> io::Result<()> {
        self.navigation(|viewer| {
            viewer.refresh_metadata()?;
            let line = viewer.last_line()?;
            viewer.position = line.saturating_add(viewer.line_length(line)?);
            viewer.preferred_column = viewer.position.saturating_sub(line);
            viewer.adjust_horizontal();
            viewer.ensure_cursor_visible()
        })
    }

    pub fn line_start(&mut self) -> io::Result<()> {
        self.navigation(|viewer| {
            viewer.refresh_metadata()?;
            viewer.position = viewer.line_start_at(viewer.position)?;
            viewer.preferred_column = 0;
            viewer.horizontal = 0;
            viewer.ensure_cursor_visible()
        })
    }

    pub fn line_end(&mut self, columns: usize) -> io::Result<()> {
        self.navigation(|viewer| {
            viewer.refresh_metadata()?;
            let start = viewer.line_start_at(viewer.position)?;
            let length = viewer.line_length(start)?;
            viewer.position = start.saturating_add(length);
            viewer.preferred_column = length;
            viewer.horizontal = length.saturating_sub(columns.max(1) as u64);
            viewer.ensure_cursor_visible()
        })
    }

    pub fn scroll_viewport(&mut self, amount: i32) -> io::Result<()> {
        self.navigation(|viewer| {
            viewer.refresh_metadata()?;
            let mut viewport = viewer.line_start_at(viewer.viewport)?;
            if amount > 0 {
                for _ in 0..amount.unsigned_abs() {
                    let next = viewer.next_line(viewport)?;
                    if next >= viewer.length {
                        break;
                    }
                    viewport = next;
                }
            } else {
                for _ in 0..amount.unsigned_abs() {
                    viewport = viewer.previous_line(viewport)?;
                }
            }
            viewer.viewport = viewport;
            Ok(())
        })
    }

    pub fn search(&mut self, query: &str, forward: bool) -> io::Result<bool> {
        let query = query.as_bytes().to_vec();
        let state = self.state();
        let matches = self.matches.clone();
        let search = self.search.clone();
        let result = (|| {
            self.refresh_metadata()?;
            if query.is_empty() {
                return Ok(false);
            }
            let found = self.search_from(&query, forward, self.position)?;
            self.search = found.map(|offset| SearchState {
                query,
                forward,
                offset,
            });
            Ok(found.is_some())
        })();
        if result.is_err() {
            self.restore_state(state);
            self.matches = matches;
            self.search = search;
        }
        result
    }

    pub fn repeat_search(&mut self, same_direction: bool) -> io::Result<bool> {
        let state = self.state();
        let matches = self.matches.clone();
        let search = self.search.clone();
        let result = (|| {
            self.refresh_metadata()?;
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
                self.set_position(offset)?;
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
        })();
        if result.is_err() {
            self.restore_state(state);
            self.matches = matches;
            self.search = search;
        }
        result
    }

    fn navigation<T, F>(&mut self, operation: F) -> io::Result<T>
    where
        F: FnOnce(&mut Self) -> io::Result<T>,
    {
        let state = self.state();
        let result = operation(self);
        if result.is_err() {
            self.restore_state(state);
        }
        result
    }

    fn state(&self) -> ViewState {
        ViewState {
            position: self.position,
            viewport: self.viewport,
            horizontal: self.horizontal,
            preferred_column: self.preferred_column,
            visible_rows: self.visible_rows,
            visible_columns: self.visible_columns,
        }
    }

    fn restore_state(&mut self, state: ViewState) {
        self.position = state.position.min(self.length);
        self.viewport = state.viewport.min(self.length);
        self.horizontal = state.horizontal;
        self.preferred_column = state.preferred_column;
        self.visible_rows = state.visible_rows;
        self.visible_columns = state.visible_columns;
    }

    fn refresh_metadata(&mut self) -> io::Result<()> {
        let length = self.file.metadata()?.len();
        if length == self.length {
            return Ok(());
        }

        let previous_length = self.length;
        self.length = length;
        if length > previous_length {
            if !previous_length.is_multiple_of(BLOCK_SIZE) {
                let tail = previous_length / BLOCK_SIZE * BLOCK_SIZE;
                self.blocks.retain(|block| block.offset != tail);
            }
            return Ok(());
        }

        for block in &mut self.blocks {
            if block.offset < length {
                block
                    .bytes
                    .truncate((length - block.offset).min(BLOCK_SIZE) as usize);
            }
        }
        self.blocks
            .retain(|block| block.offset < length && !block.bytes.is_empty());
        let query_length = self
            .search
            .as_ref()
            .map_or(0, |search| search.query.len() as u64);
        self.matches
            .retain(|offset| *offset < length && query_length <= length.saturating_sub(*offset));
        if self.search.as_ref().is_some_and(|search| {
            search.offset >= length || query_length > length.saturating_sub(search.offset)
        }) {
            self.search = None;
            self.matches.clear();
        }
        self.position = self.position.min(length);
        self.viewport = self.viewport.min(length);
        self.committed = self.committed.clamped(length);
        Ok(())
    }

    fn cache_offsets(&self) -> Vec<u64> {
        self.blocks.iter().map(|block| block.offset).collect()
    }

    fn restore_cache(&mut self, offsets: &[u64]) {
        let mut restored = VecDeque::with_capacity(offsets.len());
        for offset in offsets {
            if let Some(index) = self.blocks.iter().position(|block| block.offset == *offset) {
                restored.push_back(
                    self.blocks
                        .remove(index)
                        .expect("block index came from cache"),
                );
            }
        }
        self.blocks = restored;
    }

    fn trim_cache(&mut self) {
        while self.blocks.len() > BLOCK_CACHE_SIZE {
            self.blocks.pop_back();
        }
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
            self.set_position(offset)?;
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

    fn set_position(&mut self, position: u64) -> io::Result<()> {
        self.position = position.min(self.length);
        let line = self.line_start_at(self.position)?;
        self.preferred_column = self.position.saturating_sub(line);
        self.adjust_horizontal();
        self.ensure_cursor_visible()
    }

    fn cursor_column(&mut self, line: u64, columns: usize) -> usize {
        self.position
            .saturating_sub(line)
            .saturating_sub(self.horizontal)
            .try_into()
            .unwrap_or(usize::MAX)
            .min(columns.saturating_sub(1))
    }

    fn adjust_horizontal(&mut self) {
        let Ok(line) = self.line_start_at(self.position) else {
            return;
        };
        let column = self.position.saturating_sub(line);
        let width = self.visible_columns.max(1) as u64;
        if column < self.horizontal {
            self.horizontal = column;
        } else if column >= self.horizontal.saturating_add(width) {
            self.horizontal = column.saturating_sub(width.saturating_sub(1));
        }
    }

    fn ensure_cursor_visible(&mut self) -> io::Result<()> {
        let cursor_line = self.line_start_at(self.position)?;
        self.viewport = self.line_start_at(self.viewport)?;
        if cursor_line < self.viewport {
            self.viewport = cursor_line;
            return Ok(());
        }

        let mut line = self.viewport;
        for _ in 0..self.visible_rows.max(1) {
            if line == cursor_line {
                return Ok(());
            }
            let next = self.next_line(line)?;
            if next <= line || next > cursor_line {
                break;
            }
            line = next;
        }

        let mut viewport = cursor_line;
        for _ in 1..self.visible_rows.max(1) {
            let previous = self.previous_line(viewport)?;
            if previous == viewport {
                break;
            }
            viewport = previous;
        }
        self.viewport = viewport;
        Ok(())
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

    fn line_length(&mut self, start: u64) -> io::Result<u64> {
        let mut position = start;
        while let Some(byte) = self.byte(position)? {
            if byte == b'\n' {
                break;
            }
            position += 1;
        }
        Ok(position.saturating_sub(start))
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
            let byte = block
                .bytes
                .get((offset - block_offset) as usize)
                .copied()
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "viewer cache block is shorter than the file",
                    )
                });
            self.blocks.push_front(block);
            return byte.map(Some);
        }

        self.file.seek(SeekFrom::Start(block_offset))?;
        let length = (self.length - block_offset).min(BLOCK_SIZE) as usize;
        let mut bytes = vec![0; length];
        self.file.read_exact(&mut bytes)?;
        let byte = bytes
            .get((offset - block_offset) as usize)
            .copied()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "viewer block is shorter than the file",
                )
            })?;
        self.blocks.push_front(Block {
            offset: block_offset,
            bytes,
        });
        if !self.defer_eviction {
            self.trim_cache();
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
    use std::{
        fs::{self, OpenOptions},
        io::Write,
        time::SystemTime,
    };

    fn temp_path(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn large_viewer_data() -> Vec<u8> {
        let line_length = BLOCK_SIZE as usize / 2 - 1;
        let mut data = Vec::with_capacity(line_length * BLOCK_CACHE_SIZE * 3);
        for line in 0..BLOCK_CACHE_SIZE * 3 {
            let marker = format!("{line:04}:");
            data.extend_from_slice(marker.as_bytes());
            data.resize(data.len() + line_length - marker.len(), b'x');
            data.push(b'\n');
        }
        data
    }

    fn cache_bytes(viewer: &Viewer) -> usize {
        viewer.blocks.iter().map(|block| block.bytes.len()).sum()
    }

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
        assert_eq!(viewer.position, 25);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn cursor_and_viewport_move_independently() {
        let path = std::env::temp_dir().join(format!(
            "termfold-viewer-cursor-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, b"0123456789\nshort\nabcdefghij\nlast\n").unwrap();
        let mut viewer = Viewer::open(path.clone()).unwrap();
        let mut terminal = Terminal::new(Size {
            columns: 8,
            rows: 3,
        })
        .unwrap();
        viewer
            .render(
                &mut terminal,
                Size {
                    columns: 8,
                    rows: 3,
                },
            )
            .unwrap();

        viewer.line_end(8).unwrap();
        viewer.move_lines(1).unwrap();
        assert_eq!(viewer.position, 16);
        viewer.move_lines(1).unwrap();
        assert_eq!(viewer.position, 27);

        let position = viewer.position;
        let viewport = viewer.viewport;
        viewer.scroll_viewport(1).unwrap();
        assert_eq!(viewer.position, position);
        assert_ne!(viewer.viewport, viewport);

        viewer
            .render(
                &mut terminal,
                Size {
                    columns: 8,
                    rows: 3,
                },
            )
            .unwrap();
        assert_eq!(terminal.screen().cursor().row, 1);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn paging_crosses_evicted_blocks_in_both_directions() {
        let path = temp_path("termfold-viewer-pages");
        fs::write(&path, large_viewer_data()).unwrap();
        let mut viewer = Viewer::open(path.clone()).unwrap();
        let size = Size {
            columns: 16,
            rows: 3,
        };
        let mut terminal = Terminal::new(size).unwrap();
        viewer.render(&mut terminal, size).unwrap();
        let first_line = viewer.page[0].clone();

        for _ in 0..(BLOCK_CACHE_SIZE * 3 / 2) {
            viewer.page(size.rows, true).unwrap();
            viewer.render(&mut terminal, size).unwrap();
            assert!(viewer.blocks.len() <= BLOCK_CACHE_SIZE);
            assert!(cache_bytes(&viewer) <= BLOCK_SIZE as usize * BLOCK_CACHE_SIZE);
        }
        assert_ne!(viewer.page[0], first_line);
        assert!(viewer.page.capacity() <= usize::from(size.rows));

        for _ in 0..(BLOCK_CACHE_SIZE * 3 / 2) {
            viewer.page(size.rows, false).unwrap();
            viewer.render(&mut terminal, size).unwrap();
            assert!(viewer.blocks.len() <= BLOCK_CACHE_SIZE);
        }
        assert_eq!(viewer.page[0], first_line);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn growth_reloads_a_cached_partial_tail() {
        let path = temp_path("termfold-viewer-growth");
        fs::write(&path, b"tail").unwrap();
        let mut viewer = Viewer::open(path.clone()).unwrap();
        let size = Size {
            columns: 32,
            rows: 2,
        };
        let mut terminal = Terminal::new(size).unwrap();
        viewer.render(&mut terminal, size).unwrap();

        let mut append = OpenOptions::new().append(true).open(&path).unwrap();
        append.write_all(b"-grown").unwrap();
        drop(append);
        viewer.bottom().unwrap();
        viewer.render(&mut terminal, size).unwrap();

        assert_eq!(viewer.page[0], "tail-grown");
        assert_eq!(viewer.length, 10);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn truncation_clamps_state_and_discards_search_offsets() {
        let path = temp_path("termfold-viewer-truncate");
        fs::write(&path, b"first\nsecond\nthird\n").unwrap();
        let mut viewer = Viewer::open(path.clone()).unwrap();
        let size = Size {
            columns: 32,
            rows: 3,
        };
        let mut terminal = Terminal::new(size).unwrap();
        viewer.render(&mut terminal, size).unwrap();
        assert!(viewer.search("third", true).unwrap());

        let truncate = OpenOptions::new().write(true).open(&path).unwrap();
        truncate.set_len(6).unwrap();
        drop(truncate);
        viewer.bottom().unwrap();
        viewer.render(&mut terminal, size).unwrap();

        assert_eq!(viewer.length, 6);
        assert!(viewer.position <= viewer.length);
        assert!(viewer.viewport <= viewer.length);
        assert!(viewer.search.is_none());
        assert!(viewer.matches.is_empty());
        assert_eq!(viewer.page[0], "first");
        fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn failed_page_load_restores_the_last_committed_display() {
        let path = temp_path("termfold-viewer-rollback");
        fs::write(&path, b"stable page\nnext page\n").unwrap();
        let mut viewer = Viewer::open(path.clone()).unwrap();
        let size = Size {
            columns: 32,
            rows: 3,
        };
        let mut terminal = Terminal::new(size).unwrap();
        viewer.render(&mut terminal, size).unwrap();
        let page = viewer.page.clone();
        let state = viewer.state();

        let directory = File::open(std::env::temp_dir()).unwrap();
        assert!(directory.metadata().unwrap().len() > 0);
        viewer.file = directory;
        let error = viewer.render(&mut terminal, size).unwrap_err();

        assert!(!error.to_string().is_empty());
        assert_eq!(viewer.page, page);
        assert_eq!(viewer.state().position, state.position);
        assert_eq!(viewer.state().viewport, state.viewport);
        fs::remove_file(path).unwrap();
    }
}
