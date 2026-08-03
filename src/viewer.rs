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
const LINE_CACHE_SIZE: usize = 64;
const MAX_MATCH_OFFSETS: usize = 4096;
const MAX_LINE_BYTES: usize = 64 * 1024;
const MAX_LINE_SCAN_BYTES: u64 = BLOCK_SIZE * (BLOCK_CACHE_SIZE as u64 * 2);

#[derive(Debug)]
struct Block {
    offset: u64,
    bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
struct LineBoundary {
    start: u64,
    content_end: u64,
    next: u64,
    complete: bool,
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
    protected_blocks: Vec<u64>,
    lines: VecDeque<LineBoundary>,
    #[cfg(test)]
    block_reads: usize,
    #[cfg(test)]
    block_accesses: usize,
    #[cfg(test)]
    peak_cache_bytes: usize,
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
            protected_blocks: Vec::new(),
            lines: VecDeque::new(),
            #[cfg(test)]
            block_reads: 0,
            #[cfg(test)]
            block_accesses: 0,
            #[cfg(test)]
            peak_cache_bytes: 0,
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
        self.protected_blocks = previous_blocks.clone();
        self.visible_rows = usize::from(size.rows).max(1);
        self.visible_columns = usize::from(size.columns).max(1);
        let columns = usize::from(size.columns);
        let result = self.build_page(size);
        let (cursor_row, cursor_line) = match result {
            Ok(page) => page,
            Err(error) => {
                self.restore_state(committed);
                self.page = previous_page;
                self.restore_cache(&previous_blocks);
                self.protected_blocks.clear();
                return Err(error);
            }
        };

        if let Err(error) = terminal.resize(size).map_err(io::Error::other) {
            self.restore_state(committed);
            self.page = previous_page;
            self.restore_cache(&previous_blocks);
            self.protected_blocks.clear();
            return Err(error);
        }

        self.committed = self.state();
        self.protected_blocks.clear();
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
            let (next, line, complete) = self.read_line(position, columns, budget)?;
            let used = line.len().min(budget);
            budget = budget.saturating_sub(used);
            self.page.push(line);
            if !complete {
                break;
            }
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
        let steps = amount.unsigned_abs() as usize;
        let continued_long_line = self
            .lines
            .iter()
            .any(|line| !line.complete && line.next == current_line);
        if amount > 0 && steps > 0 {
            let mut remaining = steps;
            if let Some(line) = self.cached_line(current_line) {
                if line.next < self.length {
                    target = line.next;
                    remaining -= 1;
                    if !line.complete {
                        remaining = 0;
                    }
                } else {
                    remaining = 0;
                }
            } else if continued_long_line {
                let next = self.next_line(target)?;
                if next > target && next < self.length {
                    target = next;
                }
                remaining = 0;
            }
            if remaining > 0 {
                for _ in 0..remaining {
                    let next = self.next_line(target)?;
                    if next <= target || next >= self.length {
                        break;
                    }
                    target = next;
                }
            }
        } else if amount < 0 && steps > 0 {
            let mut remaining = steps;
            if let Some(previous) = self.cached_previous_line(current_line) {
                target = previous;
                remaining -= 1;
            }
            if remaining > 0 {
                for _ in 0..remaining {
                    let previous = self.previous_line(target)?;
                    if previous >= target {
                        break;
                    }
                    target = previous;
                }
            }
        }
        let target_length = if preferred == 0 {
            0
        } else {
            self.line_length(target)?
        };
        self.position = target.saturating_add(preferred.min(target_length));
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
        self.lines.clear();
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

    fn cached_line(&mut self, start: u64) -> Option<LineBoundary> {
        let index = self.lines.iter().position(|line| line.start == start)?;
        let line = self.lines.remove(index)?;
        self.lines.push_front(line);
        Some(line)
    }

    fn cached_line_containing(&mut self, position: u64) -> Option<LineBoundary> {
        let index = self.lines.iter().position(|line| {
            line.start <= position
                && (position < line.next
                    || (line.complete && line.content_end == line.next && position == line.next))
        })?;
        let line = self.lines.remove(index)?;
        self.lines.push_front(line);
        Some(line)
    }

    fn cached_previous_line(&mut self, start: u64) -> Option<u64> {
        let index = self.lines.iter().position(|line| line.next == start)?;
        let line = self.lines.remove(index)?;
        self.lines.push_front(line);
        Some(line.start)
    }

    fn cache_line(&mut self, line: LineBoundary) {
        if let Some(index) = self
            .lines
            .iter()
            .position(|cached| cached.start == line.start)
        {
            self.lines.remove(index);
        }
        self.lines.push_front(line);
        self.lines.truncate(LINE_CACHE_SIZE);
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
            let Some(index) = (1..self.blocks.len())
                .rev()
                .find(|index| !self.protected_blocks.contains(&self.blocks[*index].offset))
            else {
                break;
            };
            self.blocks.remove(index);
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
    ) -> io::Result<(u64, String, bool)> {
        let limit = MAX_LINE_BYTES.min(columns.saturating_mul(4).saturating_add(4));
        if let Some(line) = self.cached_line(start) {
            let line_length = line.content_end.saturating_sub(start);
            if self.horizontal >= line_length || budget == 0 {
                return Ok((line.next, String::new(), line.complete));
            }
            let read_start = start.saturating_add(self.horizontal);
            let read_end = read_start
                .saturating_add(limit.min(budget).saturating_add(4) as u64)
                .min(line.content_end);
            let mut bytes = Vec::with_capacity(limit.min(budget));
            self.scan_forward(read_start, read_end, |_, byte| {
                if bytes.len() < limit && bytes.len() < budget {
                    bytes.push(byte);
                }
                false
            })?;
            if read_end == line.content_end && bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
            return Ok((line.next, display_line(&bytes, columns), line.complete));
        }
        let mut bytes = Vec::with_capacity(limit.min(budget));
        let mut skipped = 0;
        let horizontal = self.horizontal;
        let scan_end = self.line_scan_end(start);
        let newline = self.scan_forward(start, scan_end, |_, byte| {
            if skipped < horizontal {
                skipped += 1;
                return byte == b'\n';
            }
            if byte == b'\n' {
                return true;
            }
            if bytes.len() < limit && bytes.len() < budget {
                bytes.push(byte);
            }
            false
        })?;
        let line = newline.map_or(
            LineBoundary {
                start,
                content_end: scan_end,
                next: scan_end,
                complete: scan_end == self.length,
            },
            |offset| LineBoundary {
                start,
                content_end: offset,
                next: offset + 1,
                complete: true,
            },
        );
        self.cache_line(line);
        if skipped < horizontal {
            return Ok((line.next, String::new(), line.complete));
        }
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        Ok((line.next, display_line(&bytes, columns), line.complete))
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
        if cursor_line > self.viewport
            && self
                .cached_line(self.viewport)
                .is_some_and(|line| !line.complete)
        {
            self.viewport = cursor_line;
            return Ok(());
        }

        let rows = self.visible_rows.max(1);
        let mut line = self.viewport;
        for _ in 0..rows {
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
        for _ in 1..rows {
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
        if let Some(line) = self.cached_line(start) {
            return Ok(line.next);
        }
        let scan_end = self.line_scan_end(start);
        let newline = self.scan_forward(start, scan_end, |_, byte| byte == b'\n')?;
        let line = newline.map_or(
            LineBoundary {
                start,
                content_end: scan_end,
                next: scan_end,
                complete: scan_end == self.length,
            },
            |offset| LineBoundary {
                start,
                content_end: offset,
                next: offset + 1,
                complete: true,
            },
        );
        let next = line.next;
        self.cache_line(line);
        Ok(next)
    }

    fn line_length(&mut self, start: u64) -> io::Result<u64> {
        if let Some(line) = self.cached_line(start) {
            return Ok(line.content_end.saturating_sub(start));
        }
        let scan_end = self.line_scan_end(start);
        let newline = self.scan_forward(start, scan_end, |_, byte| byte == b'\n')?;
        let line = newline.map_or(
            LineBoundary {
                start,
                content_end: scan_end,
                next: scan_end,
                complete: scan_end == self.length,
            },
            |offset| LineBoundary {
                start,
                content_end: offset,
                next: offset + 1,
                complete: true,
            },
        );
        let length = line.content_end.saturating_sub(start);
        self.cache_line(line);
        Ok(length)
    }

    fn previous_line(&mut self, start: u64) -> io::Result<u64> {
        if start == 0 {
            return Ok(0);
        }
        if let Some(previous) = self.cached_previous_line(start) {
            return Ok(previous);
        }
        let scan_start = start.saturating_sub(MAX_LINE_SCAN_BYTES);
        let boundary = start - 1;
        let mut skip_boundary = true;
        let previous = self.scan_reverse(start, scan_start, |offset, byte| {
            if skip_boundary && offset == boundary && byte == b'\n' {
                skip_boundary = false;
                return false;
            }
            byte == b'\n'
        })?;
        Ok(previous.map_or(scan_start, |offset| offset + 1))
    }

    fn last_line(&mut self) -> io::Result<u64> {
        if self.length == 0 {
            return Ok(0);
        }
        if let Some(index) = self.lines.iter().position(|line| line.next == self.length) {
            let line = self
                .lines
                .remove(index)
                .expect("line index came from cache");
            let start = line.start;
            self.lines.push_front(line);
            return Ok(start);
        }
        let scan_start = self.length.saturating_sub(MAX_LINE_SCAN_BYTES);
        let boundary = self.length - 1;
        let mut skip_boundary = true;
        let previous = self.scan_reverse(self.length, scan_start, |offset, byte| {
            if skip_boundary && offset == boundary && byte == b'\n' {
                skip_boundary = false;
                return false;
            }
            byte == b'\n'
        })?;
        Ok(previous.map_or(scan_start, |offset| offset + 1))
    }

    fn line_start_at(&mut self, position: u64) -> io::Result<u64> {
        let position = position.min(self.length);
        if self
            .lines
            .iter()
            .any(|line| !line.complete && line.next == position)
        {
            return Ok(position);
        }
        if let Some(line) = self.cached_line_containing(position) {
            return Ok(line.start);
        }
        let scan_start = position.saturating_sub(MAX_LINE_SCAN_BYTES);
        Ok(self
            .scan_reverse(position, scan_start, |_, byte| byte == b'\n')?
            .map_or(scan_start, |offset| offset + 1))
    }

    fn line_scan_end(&self, start: u64) -> u64 {
        start.saturating_add(MAX_LINE_SCAN_BYTES).min(self.length)
    }

    fn byte(&mut self, offset: u64) -> io::Result<Option<u8>> {
        if offset >= self.length {
            return Ok(None);
        }
        let block_offset = offset / BLOCK_SIZE * BLOCK_SIZE;
        let bytes = self.block(block_offset)?;
        bytes
            .get((offset - block_offset) as usize)
            .copied()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "viewer cache block is shorter than the file",
                )
            })
            .map(Some)
    }

    fn scan_forward<F>(&mut self, start: u64, end: u64, mut visit: F) -> io::Result<Option<u64>>
    where
        F: FnMut(u64, u8) -> bool,
    {
        let end = end.min(self.length);
        let mut position = start.min(end);
        while position < end {
            let block_offset = position / BLOCK_SIZE * BLOCK_SIZE;
            let expected = (self.length - block_offset).min(BLOCK_SIZE) as usize;
            let found = {
                let bytes = self.block(block_offset)?;
                if bytes.len() < expected {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "viewer cache block is shorter than the file",
                    ));
                }
                let first = (position - block_offset) as usize;
                let last = (end - block_offset).min(expected as u64) as usize;
                if last <= first {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "viewer cache block does not cover the requested range",
                    ));
                }
                (first..last)
                    .find(|index| {
                        let offset = block_offset + *index as u64;
                        visit(offset, bytes[*index])
                    })
                    .map(|index| block_offset + index as u64)
            };
            if found.is_some() {
                return Ok(found);
            }
            position = block_offset + (end - block_offset).min(expected as u64);
        }
        Ok(None)
    }

    fn scan_reverse<F>(&mut self, start: u64, end: u64, mut visit: F) -> io::Result<Option<u64>>
    where
        F: FnMut(u64, u8) -> bool,
    {
        let mut position = start.min(self.length);
        let end = end.min(position);
        while position > end {
            let block_offset = (position - 1) / BLOCK_SIZE * BLOCK_SIZE;
            let expected = (self.length - block_offset).min(BLOCK_SIZE) as usize;
            let found = {
                let bytes = self.block(block_offset)?;
                if bytes.len() < expected {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "viewer cache block is shorter than the file",
                    ));
                }
                let first = end.max(block_offset);
                let last = position.min(block_offset + expected as u64);
                if last <= first {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "viewer cache block does not cover the requested range",
                    ));
                }
                (first..last)
                    .rev()
                    .find(|offset| visit(*offset, bytes[(*offset - block_offset) as usize]))
            };
            if found.is_some() {
                return Ok(found);
            }
            position = end.max(block_offset);
        }
        Ok(None)
    }

    fn block(&mut self, block_offset: u64) -> io::Result<&[u8]> {
        #[cfg(test)]
        {
            self.block_accesses += 1;
        }
        if let Some(index) = self
            .blocks
            .iter()
            .position(|block| block.offset == block_offset)
        {
            if index != 0 {
                let block = self
                    .blocks
                    .remove(index)
                    .expect("block index came from cache");
                self.blocks.push_front(block);
            }
            return Ok(&self.blocks.front().expect("cache block exists").bytes);
        }

        self.file.seek(SeekFrom::Start(block_offset))?;
        let length = (self.length - block_offset).min(BLOCK_SIZE) as usize;
        let mut bytes = vec![0; length];
        self.file.read_exact(&mut bytes)?;
        self.blocks.push_front(Block {
            offset: block_offset,
            bytes,
        });
        #[cfg(test)]
        {
            self.block_reads += 1;
            self.peak_cache_bytes = self
                .peak_cache_bytes
                .max(self.blocks.iter().map(|block| block.bytes.len()).sum());
        }
        self.trim_cache();
        Ok(&self.blocks.front().expect("cache block exists").bytes)
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

    fn page_bytes(viewer: &Viewer) -> usize {
        viewer.page.iter().map(String::capacity).sum()
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
    fn page_navigation_scans_many_lines_without_cache_churn() {
        let path = temp_path("termfold-viewer-line-scan");
        let mut data = Vec::new();
        for _ in 0..20_000 {
            data.extend_from_slice(b"line\n");
        }
        fs::write(&path, data).unwrap();

        let size = Size {
            columns: 16,
            rows: 64,
        };
        let mut terminal = Terminal::new(size).unwrap();
        let mut viewer = Viewer::open(path.clone()).unwrap();
        viewer.render(&mut terminal, size).unwrap();
        viewer.block_accesses = 0;
        viewer.block_reads = 0;

        viewer.page(size.rows, true).unwrap();
        assert!(viewer.block_reads <= 1);
        assert!(
            viewer.block_accesses <= 8,
            "block accesses: {}",
            viewer.block_accesses
        );

        viewer.block_accesses = 0;
        viewer.page(size.rows, false).unwrap();
        assert!(viewer.block_reads <= 1);
        assert!(
            viewer.block_accesses <= 8,
            "block accesses: {}",
            viewer.block_accesses
        );

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn repeated_paging_on_an_eof_line_keeps_work_and_cache_bounded() {
        let path = temp_path("termfold-viewer-eof-line");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.set_len(BLOCK_SIZE * (BLOCK_CACHE_SIZE as u64 + 4) + 17)
            .unwrap();
        drop(file);

        let size = Size {
            columns: 16,
            rows: 3,
        };
        let mut terminal = Terminal::new(size).unwrap();
        let mut viewer = Viewer::open(path.clone()).unwrap();
        viewer.render(&mut terminal, size).unwrap();
        assert!(viewer.blocks.len() <= BLOCK_CACHE_SIZE);
        assert!(cache_bytes(&viewer) <= BLOCK_SIZE as usize * BLOCK_CACHE_SIZE);
        assert!(viewer.lines.iter().any(|line| {
            line.start == 0 && line.content_end == viewer.length && line.next == viewer.length
        }));

        viewer.block_reads = 0;
        for _ in 0..4 {
            viewer.page(size.rows, true).unwrap();
            viewer.render(&mut terminal, size).unwrap();
            viewer.page(size.rows, false).unwrap();
            viewer.render(&mut terminal, size).unwrap();
            assert!(viewer.blocks.len() <= BLOCK_CACHE_SIZE);
            assert!(cache_bytes(&viewer) <= BLOCK_SIZE as usize * BLOCK_CACHE_SIZE);
        }
        assert!(
            viewer.block_reads <= 1,
            "repeated EOF reads: {}",
            viewer.block_reads
        );

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn repeated_paging_reuses_a_long_line_boundary() {
        let path = temp_path("termfold-viewer-long-line");
        let mut data = vec![b'x'; BLOCK_SIZE as usize * (BLOCK_CACHE_SIZE + 4)];
        data.extend_from_slice(b"\nsecond\nthird\n");
        fs::write(&path, data).unwrap();

        let size = Size {
            columns: 16,
            rows: 3,
        };
        let mut terminal = Terminal::new(size).unwrap();
        let mut viewer = Viewer::open(path.clone()).unwrap();
        viewer.render(&mut terminal, size).unwrap();
        assert!(
            viewer
                .lines
                .iter()
                .any(|line| line.start == 0 && line.next > line.start)
        );

        viewer.block_reads = 0;
        for _ in 0..4 {
            viewer.page(size.rows, true).unwrap();
            viewer.render(&mut terminal, size).unwrap();
            viewer.page(size.rows, false).unwrap();
            viewer.render(&mut terminal, size).unwrap();
        }
        assert!(
            viewer.block_reads <= 1,
            "repeated long-line reads: {}",
            viewer.block_reads
        );

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rendering_an_unterminated_line_does_not_read_the_complete_file() {
        let path = temp_path("termfold-viewer-huge-line");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.set_len(BLOCK_SIZE * 128).unwrap();
        drop(file);

        let size = Size {
            columns: 16,
            rows: 3,
        };
        let mut terminal = Terminal::new(size).unwrap();
        let mut viewer = Viewer::open(path.clone()).unwrap();
        viewer.render(&mut terminal, size).unwrap();

        assert!(
            viewer.block_reads <= BLOCK_CACHE_SIZE * 2,
            "render read the complete unterminated line: {} blocks",
            viewer.block_reads
        );

        viewer.block_reads = 0;
        let mut position = viewer.position;
        for _ in 0..4 {
            let reads = viewer.block_reads;
            viewer.page(size.rows, true).unwrap();
            viewer.render(&mut terminal, size).unwrap();
            assert!(viewer.block_reads - reads <= BLOCK_CACHE_SIZE * 2);
            assert!(viewer.position > position);
            position = viewer.position;
        }
        for _ in 0..4 {
            let reads = viewer.block_reads;
            viewer.page(size.rows, false).unwrap();
            viewer.render(&mut terminal, size).unwrap();
            assert!(viewer.block_reads - reads <= BLOCK_CACHE_SIZE * 2);
        }
        assert_eq!(viewer.position, 0);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn paging_scans_blocks_without_bytewise_cache_churn() {
        let path = temp_path("termfold-viewer-metrics");
        let block_size = BLOCK_SIZE as usize;
        let mut data = Vec::with_capacity(block_size * (BLOCK_CACHE_SIZE + 6));
        for _ in 0..(BLOCK_CACHE_SIZE + 4) {
            data.extend(std::iter::repeat_n(b'x', block_size - 1));
            data.push(b'\n');
        }
        data.extend(std::iter::repeat_n(b'z', block_size * 2 + 17));
        fs::write(&path, data).unwrap();

        let size = Size {
            columns: 16,
            rows: 3,
        };
        let mut terminal = Terminal::new(size).unwrap();
        let mut viewer = Viewer::open(path.clone()).unwrap();

        let page_before = page_bytes(&viewer);
        let started = std::time::Instant::now();
        viewer.render(&mut terminal, size).unwrap();
        let initial_elapsed = started.elapsed();
        assert_eq!(viewer.block_reads, 3);
        assert!(viewer.block_accesses < 20);
        let initial_reads = viewer.block_reads;
        let initial_page = page_bytes(&viewer);

        let started = std::time::Instant::now();
        for _ in 0..3 {
            viewer.page(size.rows, true).unwrap();
            viewer.render(&mut terminal, size).unwrap();
        }
        let cold_elapsed = started.elapsed();
        let cold_reads = viewer.block_reads - initial_reads;
        assert_eq!(cold_reads, 1);
        let cold_page = page_bytes(&viewer);

        let started = std::time::Instant::now();
        let warm_reads = viewer.block_reads;
        for _ in 0..3 {
            viewer.page(size.rows, false).unwrap();
            viewer.render(&mut terminal, size).unwrap();
        }
        let warm_elapsed = started.elapsed();
        assert_eq!(viewer.block_reads, warm_reads);

        let started = std::time::Instant::now();
        let long_accesses_before = viewer.block_accesses;
        viewer.bottom().unwrap();
        viewer.render(&mut terminal, size).unwrap();
        let long_line_elapsed = started.elapsed();
        let long_line_reads = viewer.block_reads - warm_reads;
        let long_line_accesses = viewer.block_accesses - long_accesses_before;
        assert!(
            long_line_reads <= BLOCK_CACHE_SIZE,
            "long-line block reads: {long_line_reads}"
        );
        assert!(long_line_accesses < 100);
        assert!(cache_bytes(&viewer) <= block_size * BLOCK_CACHE_SIZE);
        let final_page = page_bytes(&viewer);

        eprintln!(
            concat!(
                "viewer paging metrics: initial={:?} ({} blocks), ",
                "cold_down={:?} ({} blocks), ",
                "warm_up={:?} (0 blocks), ",
                "long_line={:?} ({} blocks, {} accesses), ",
                "peak_cache={} bytes, page_bytes={}->{}->{}->{}"
            ),
            initial_elapsed,
            initial_reads,
            cold_elapsed,
            cold_reads,
            warm_elapsed,
            long_line_elapsed,
            long_line_reads,
            long_line_accesses,
            viewer.peak_cache_bytes,
            page_before,
            initial_page,
            cold_page,
            final_page
        );

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
