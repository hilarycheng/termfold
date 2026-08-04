use std::{
    collections::VecDeque,
    io,
    path::{Path, PathBuf},
};

use crate::{session::Size, terminal::Terminal};
mod line;
mod source;
mod text;

use line::{LineBoundary, LineScanner, ScanStep};
use source::FileSource;
#[cfg(test)]
use source::{BLOCK_CACHE_SIZE, BLOCK_SIZE};
use text::decode;

const LINE_CACHE_SIZE: usize = 64;
const MAX_MATCH_OFFSETS: usize = 4096;
const MAX_LINE_BYTES: usize = 64 * 1024;

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

#[derive(Debug)]
pub struct Viewer {
    source: FileSource,
    tab_width: usize,
    position: u64,
    viewport: u64,
    horizontal: u64,
    preferred_column: u64,
    visible_rows: usize,
    visible_columns: usize,
    matches: Vec<u64>,
    search: Option<SearchState>,
    page: Vec<String>,
    committed: ViewState,
    lines: VecDeque<LineBoundary>,
}

impl Viewer {
    pub fn open(path: PathBuf, tab_width: usize) -> io::Result<Self> {
        let committed = ViewState {
            visible_rows: 1,
            visible_columns: 1,
            ..ViewState::default()
        };
        Ok(Self {
            source: FileSource::open(path)?,
            tab_width,
            position: 0,
            viewport: 0,
            horizontal: 0,
            preferred_column: 0,
            visible_rows: 1,
            visible_columns: 1,
            matches: Vec::new(),
            search: None,
            page: Vec::new(),
            committed,
            lines: VecDeque::new(),
        })
    }

    pub fn path(&self) -> &Path {
        self.source.path()
    }

    fn length(&self) -> u64 {
        self.source.len()
    }

    pub fn render(&mut self, terminal: &mut Terminal, size: Size) -> io::Result<()> {
        let committed = self.committed;
        let previous_page = std::mem::take(&mut self.page);
        self.visible_rows = usize::from(size.rows).max(1);
        self.visible_columns = usize::from(size.columns).max(1);
        let columns = usize::from(size.columns);
        let result = self.build_page(size);
        let (cursor_row, cursor_line) = match result {
            Ok(page) => page,
            Err(error) => {
                self.restore_state(committed);
                self.page = previous_page;
                return Err(error);
            }
        };

        if let Err(error) = terminal.resize(size).map_err(io::Error::other) {
            self.restore_state(committed);
            self.page = previous_page;
            return Err(error);
        }

        self.committed = self.state();
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
        self.position = self.position.min(self.length());
        self.viewport = self.viewport.min(self.length());
        self.viewport = self.line_start_at(self.viewport)?;
        let cursor_line = self.line_start_at(self.position)?;
        let mut position = self.viewport;
        let mut cursor_row = None;
        let rows = usize::from(size.rows);
        let columns = usize::from(size.columns);
        let mut budget = columns.saturating_mul(rows).saturating_mul(4);
        self.page = Vec::with_capacity(rows);
        for row in 0..rows {
            if position >= self.length() || budget == 0 {
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
        self.navigation(|viewer| viewer.move_lines_inner(amount))
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
                if line.next < self.length() {
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
                if next > target && next < self.length() {
                    target = next;
                }
                remaining = 0;
            }
            if remaining > 0 {
                for _ in 0..remaining {
                    let next = self.next_line(target)?;
                    if next <= target || next >= self.length() {
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
            viewer.visible_rows = usize::from(rows).max(1);
            let lines = usize::from(rows.saturating_sub(2).max(1));
            let amount = i32::try_from(lines).unwrap_or(i32::MAX);
            viewer.move_lines_inner(if forward { amount } else { -amount })
        })
    }

    pub fn half_page(&mut self, rows: u16, forward: bool) -> io::Result<()> {
        self.navigation(|viewer| {
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
            let line = viewer.last_line()?;
            viewer.position = line.saturating_add(viewer.line_length(line)?);
            viewer.preferred_column = viewer.position.saturating_sub(line);
            viewer.adjust_horizontal();
            viewer.ensure_cursor_visible()
        })
    }

    pub fn line_start(&mut self) -> io::Result<()> {
        self.navigation(|viewer| {
            viewer.position = viewer.line_start_at(viewer.position)?;
            viewer.preferred_column = 0;
            viewer.horizontal = 0;
            viewer.ensure_cursor_visible()
        })
    }

    pub fn line_end(&mut self, columns: usize) -> io::Result<()> {
        self.navigation(|viewer| {
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
            let mut viewport = viewer.line_start_at(viewer.viewport)?;
            if amount > 0 {
                for _ in 0..amount.unsigned_abs() {
                    let next = viewer.next_line(viewport)?;
                    if next >= viewer.length() {
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
        self.position = state.position.min(self.length());
        self.viewport = state.viewport.min(self.length());
        self.horizontal = state.horizontal;
        self.preferred_column = state.preferred_column;
        self.visible_rows = state.visible_rows;
        self.visible_columns = state.visible_columns;
    }

    fn cached_line(&mut self, start: u64) -> Option<LineBoundary> {
        let index = self.lines.iter().position(|line| {
            line.start == start && line.resume.is_none_or(|scanner| scanner.is_forward())
        })?;
        let line = self.lines.remove(index)?;
        self.lines.push_front(line);
        Some(line)
    }

    fn cached_line_containing(&mut self, position: u64) -> Option<LineBoundary> {
        let index = self.lines.iter().position(|line| {
            line.resume.is_none_or(|scanner| scanner.is_forward())
                && line.start <= position
                && (position < line.next
                    || (line.complete && line.content_end == line.next && position == line.next))
        })?;
        let line = self.lines.remove(index)?;
        self.lines.push_front(line);
        Some(line)
    }

    fn cached_previous_line(&mut self, start: u64) -> Option<u64> {
        let index = self
            .lines
            .iter()
            .position(|line| {
                line.next == start && line.resume.is_none_or(|scanner| scanner.is_forward())
            })
            .or_else(|| self.lines.iter().position(|line| line.next == start))?;
        let line = self.lines.remove(index)?;
        self.lines.push_front(line);
        Some(line.start)
    }

    fn cached_forward_continuation(&mut self, start: u64) -> Option<LineScanner> {
        let index = self.lines.iter().position(|line| {
            !line.complete
                && line.next == start
                && line.resume.is_some_and(|scanner| scanner.is_forward())
        })?;
        let line = self.lines.remove(index)?;
        self.lines.push_front(line);
        line.resume
    }

    fn cached_reverse_continuation(&mut self, start: u64) -> Option<LineScanner> {
        let index = self.lines.iter().position(|line| {
            !line.complete
                && line.start == start
                && line.resume.is_some_and(|scanner| !scanner.is_forward())
        })?;
        let line = self.lines.remove(index)?;
        self.lines.push_front(line);
        line.resume
    }

    fn cache_line(&mut self, line: LineBoundary) {
        let forward = line.resume.is_none_or(|scanner| scanner.is_forward());
        if let Some(index) = self.lines.iter().position(|cached| {
            cached.start == line.start
                && cached.resume.is_none_or(|scanner| scanner.is_forward()) == forward
        }) {
            self.lines.remove(index);
        }
        self.lines.push_front(line);
        self.lines.truncate(LINE_CACHE_SIZE);
    }

    fn search_from(&mut self, query: &[u8], forward: bool, start: u64) -> io::Result<Option<u64>> {
        self.matches.clear();
        let maximum = self.length().checked_sub(query.len() as u64);
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
            if self.source.read_byte(offset + index as u64)? != Some(*byte) {
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
        let line = self.line_boundary(start)?;
        let line_length = line.content_end.saturating_sub(start);
        if self.horizontal >= line_length || budget == 0 {
            return Ok((line.next, String::new(), line.complete));
        }
        let read_start = start.saturating_add(self.horizontal).min(line.content_end);
        let read_end = read_start
            .saturating_add(limit.min(budget) as u64)
            .min(line.content_end);
        let bytes = self.source.read_range(read_start..read_end)?;
        if bytes.len() != (read_end - read_start) as usize {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "viewer source range is shorter than the snapshot",
            ));
        }
        Ok((
            line.next,
            decode(&bytes, self.tab_width).render(columns),
            line.complete,
        ))
    }

    fn line_boundary(&mut self, start: u64) -> io::Result<LineBoundary> {
        let start = start.min(self.length());
        if let Some(line) = self.cached_line(start) {
            return Ok(line);
        }
        let mut scanner = self
            .cached_forward_continuation(start)
            .unwrap_or_else(|| LineScanner::forward(start, self.length()));
        let line = match scanner.step(&mut self.source)? {
            ScanStep::Boundary {
                start: content_end,
                end: next,
            } => LineBoundary {
                start,
                content_end,
                next,
                complete: true,
                resume: None,
            },
            ScanStep::Yield { position, .. } => LineBoundary {
                start,
                content_end: position,
                next: position,
                complete: false,
                resume: Some(scanner),
            },
            ScanStep::Done { position } => LineBoundary {
                start,
                content_end: position,
                next: position,
                complete: true,
                resume: None,
            },
        };
        self.cache_line(line);
        Ok(line)
    }

    fn set_position(&mut self, position: u64) -> io::Result<()> {
        self.position = position.min(self.length());
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
        Ok(self.line_boundary(start)?.next)
    }

    fn line_length(&mut self, start: u64) -> io::Result<u64> {
        let line = self.line_boundary(start)?;
        let length = line.content_end.saturating_sub(start);
        Ok(length)
    }

    fn previous_line(&mut self, start: u64) -> io::Result<u64> {
        if start == 0 {
            return Ok(0);
        }
        if let Some(previous) = self.cached_previous_line(start) {
            return Ok(previous);
        }
        let mut scanner = self
            .cached_reverse_continuation(start)
            .unwrap_or_else(|| LineScanner::reverse(start.min(self.length()), 0, true));
        Ok(match scanner.step(&mut self.source)? {
            ScanStep::Boundary { end, .. } => end,
            ScanStep::Yield {
                position,
                content_end,
            } => {
                self.cache_line(LineBoundary {
                    start: position,
                    content_end: content_end.min(start),
                    next: start,
                    complete: false,
                    resume: Some(scanner),
                });
                position
            }
            ScanStep::Done { position } => position,
        })
    }

    fn last_line(&mut self) -> io::Result<u64> {
        if self.length() == 0 {
            return Ok(0);
        }
        if let Some(index) = self
            .lines
            .iter()
            .position(|line| {
                line.next == self.length() && line.resume.is_none_or(|scanner| scanner.is_forward())
            })
            .or_else(|| {
                self.lines
                    .iter()
                    .position(|line| line.next == self.length())
            })
        {
            let line = self
                .lines
                .remove(index)
                .expect("line index came from cache");
            let start = line.start;
            self.lines.push_front(line);
            return Ok(start);
        }
        let length = self.length();
        let mut scanner = self
            .cached_reverse_continuation(length)
            .unwrap_or_else(|| LineScanner::reverse(length, 0, true));
        Ok(match scanner.step(&mut self.source)? {
            ScanStep::Boundary { end, .. } => end,
            ScanStep::Yield {
                position,
                content_end,
            } => {
                self.cache_line(LineBoundary {
                    start: position,
                    content_end: content_end.min(length),
                    next: length,
                    complete: false,
                    resume: Some(scanner),
                });
                position
            }
            ScanStep::Done { position } => position,
        })
    }

    fn line_start_at(&mut self, position: u64) -> io::Result<u64> {
        let position = position.min(self.length());
        if self.lines.iter().any(|line| {
            !line.complete
                && line.next == position
                && line.resume.is_some_and(|scanner| scanner.is_forward())
        }) {
            return Ok(position);
        }
        if let Some(line) = self.cached_line_containing(position) {
            return Ok(line.start);
        }
        let mut scanner = LineScanner::reverse(position, 0, false);
        Ok(match scanner.step(&mut self.source)? {
            ScanStep::Boundary { end, .. } => end,
            ScanStep::Yield {
                position: scan_position,
                content_end,
            } => {
                self.cache_line(LineBoundary {
                    start: scan_position,
                    content_end: content_end.min(position),
                    next: position,
                    complete: false,
                    resume: Some(scanner),
                });
                scan_position
            }
            ScanStep::Done { position } => position,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs::{self, OpenOptions},
        time::SystemTime,
    };

    #[cfg(unix)]
    use std::fs::File;

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
        viewer.source.cache_bytes()
    }

    fn page_bytes(viewer: &Viewer) -> usize {
        viewer.page.iter().map(String::capacity).sum()
    }

    #[test]
    fn uses_configured_tab_width_for_text() {
        let path = temp_path("termfold-viewer-tab-width");
        fs::write(&path, b"a\t\n").unwrap();
        let mut viewer = Viewer::open(path.clone(), 4).unwrap();

        let (_, line, complete) = viewer.read_line(0, 20, MAX_LINE_BYTES).unwrap();
        assert_eq!(line, "a   ");
        assert!(complete);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn line_boundaries_handle_all_eol_forms_and_empty_lines() {
        let path = temp_path("termfold-viewer-eol-forms");
        fs::write(&path, b"lf\ncrlf\r\ncr\rmixed\r\n\nend").unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();

        for (start, content_end, next) in [
            (0, 2, 3),
            (3, 7, 9),
            (9, 11, 12),
            (12, 17, 19),
            (19, 19, 20),
            (20, 23, 23),
        ] {
            let line = viewer.line_boundary(start).unwrap();
            assert_eq!(
                (line.start, line.content_end, line.next),
                (start, content_end, next)
            );
            assert_eq!(viewer.next_line(start).unwrap(), next);
        }
        assert_eq!(viewer.line_start_at(19).unwrap(), 19);
        assert_eq!(viewer.previous_line(20).unwrap(), 19);
        assert_eq!(viewer.previous_line(19).unwrap(), 12);

        let empty_path = temp_path("termfold-viewer-empty");
        fs::write(&empty_path, b"").unwrap();
        let mut empty = Viewer::open(empty_path.clone(), 8).unwrap();
        let line = empty.line_boundary(0).unwrap();
        assert_eq!((line.start, line.content_end, line.next), (0, 0, 0));
        assert_eq!(empty.previous_line(0).unwrap(), 0);

        fs::remove_file(path).unwrap();
        fs::remove_file(empty_path).unwrap();
    }

    #[test]
    fn reverse_discovery_handles_crlf_split_at_a_block_boundary() {
        let path = temp_path("termfold-viewer-eol-boundary");
        let mut data = vec![b'x'; BLOCK_SIZE as usize - 1];
        data.extend_from_slice(b"\r\ntail\nlast");
        fs::write(&path, data).unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();

        let first = viewer.line_boundary(0).unwrap();
        assert_eq!(
            (first.content_end, first.next),
            (BLOCK_SIZE - 1, BLOCK_SIZE + 1)
        );
        assert_eq!(
            viewer.previous_line(BLOCK_SIZE + 6).unwrap(),
            BLOCK_SIZE + 1
        );
        assert_eq!(viewer.previous_line(BLOCK_SIZE + 1).unwrap(), 0);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn long_line_scanning_resumes_one_block_at_a_time() {
        let path = temp_path("termfold-viewer-long-eol");
        let mut data = vec![b'x'; BLOCK_SIZE as usize * 9];
        data.extend_from_slice(b"\nend");
        fs::write(&path, data).unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();

        let mut start = 0;
        for _ in 0..9 {
            let line = viewer.line_boundary(start).unwrap();
            assert!(!line.complete);
            assert_eq!(line.content_end, start + BLOCK_SIZE);
            assert_eq!(line.next, start + BLOCK_SIZE);
            start = line.next;
        }
        let line = viewer.line_boundary(start).unwrap();
        assert!(line.complete);
        assert_eq!((line.content_end, line.next), (start, start + 1));

        fs::remove_file(path).unwrap();
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
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();

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
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();
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
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();
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
            assert!(viewer.source.cache_block_count() <= BLOCK_CACHE_SIZE);
            assert!(cache_bytes(&viewer) <= BLOCK_SIZE as usize * BLOCK_CACHE_SIZE);
        }
        assert_ne!(viewer.page[0], first_line);
        assert!(viewer.page.capacity() <= usize::from(size.rows));

        for _ in 0..(BLOCK_CACHE_SIZE * 3 / 2) {
            viewer.page(size.rows, false).unwrap();
            viewer.render(&mut terminal, size).unwrap();
            assert!(viewer.source.cache_block_count() <= BLOCK_CACHE_SIZE);
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
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();
        viewer.render(&mut terminal, size).unwrap();
        viewer.source.reset_metrics();

        viewer.page(size.rows, true).unwrap();
        assert!(viewer.source.block_reads() <= 1);
        assert!(
            viewer.source.block_accesses() <= 8,
            "block accesses: {}",
            viewer.source.block_accesses()
        );

        viewer.source.reset_metrics();
        viewer.page(size.rows, false).unwrap();
        assert!(viewer.source.block_reads() <= 1);
        assert!(
            viewer.source.block_accesses() <= 8,
            "block accesses: {}",
            viewer.source.block_accesses()
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
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();
        viewer.render(&mut terminal, size).unwrap();
        assert!(viewer.source.cache_block_count() <= BLOCK_CACHE_SIZE);
        assert!(cache_bytes(&viewer) <= BLOCK_SIZE as usize * BLOCK_CACHE_SIZE);
        assert!(viewer.lines.iter().any(|line| {
            line.start == 0
                && line.content_end == BLOCK_SIZE
                && line.next == BLOCK_SIZE
                && !line.complete
        }));

        viewer.source.reset_metrics();
        for _ in 0..4 {
            viewer.page(size.rows, true).unwrap();
            viewer.render(&mut terminal, size).unwrap();
            viewer.page(size.rows, false).unwrap();
            viewer.render(&mut terminal, size).unwrap();
            assert!(viewer.source.cache_block_count() <= BLOCK_CACHE_SIZE);
            assert!(cache_bytes(&viewer) <= BLOCK_SIZE as usize * BLOCK_CACHE_SIZE);
        }
        assert!(
            viewer.source.block_reads() <= 1,
            "repeated EOF reads: {}",
            viewer.source.block_reads()
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
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();
        viewer.render(&mut terminal, size).unwrap();
        assert!(
            viewer
                .lines
                .iter()
                .any(|line| line.start == 0 && line.next > line.start)
        );

        viewer.source.reset_metrics();
        for _ in 0..4 {
            viewer.page(size.rows, true).unwrap();
            viewer.render(&mut terminal, size).unwrap();
            viewer.page(size.rows, false).unwrap();
            viewer.render(&mut terminal, size).unwrap();
        }
        assert!(
            viewer.source.block_reads() <= 1,
            "repeated long-line reads: {}",
            viewer.source.block_reads()
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
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();
        viewer.render(&mut terminal, size).unwrap();

        assert!(
            viewer.source.block_reads() <= BLOCK_CACHE_SIZE * 2,
            "render read the complete unterminated line: {} blocks",
            viewer.source.block_reads()
        );

        viewer.source.reset_metrics();
        let mut position = viewer.position;
        for _ in 0..4 {
            let reads = viewer.source.block_reads();
            viewer.page(size.rows, true).unwrap();
            viewer.render(&mut terminal, size).unwrap();
            assert!(viewer.source.block_reads() - reads <= BLOCK_CACHE_SIZE * 2);
            assert!(viewer.position > position);
            position = viewer.position;
        }
        for _ in 0..4 {
            let reads = viewer.source.block_reads();
            viewer.page(size.rows, false).unwrap();
            viewer.render(&mut terminal, size).unwrap();
            assert!(viewer.source.block_reads() - reads <= BLOCK_CACHE_SIZE * 2);
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
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();

        let page_before = page_bytes(&viewer);
        let started = std::time::Instant::now();
        viewer.render(&mut terminal, size).unwrap();
        let initial_elapsed = started.elapsed();
        assert_eq!(viewer.source.block_reads(), 3);
        assert!(viewer.source.block_accesses() < 20);
        let initial_reads = viewer.source.block_reads();
        let initial_page = page_bytes(&viewer);

        let started = std::time::Instant::now();
        for _ in 0..3 {
            viewer.page(size.rows, true).unwrap();
            viewer.render(&mut terminal, size).unwrap();
        }
        let cold_elapsed = started.elapsed();
        let cold_reads = viewer.source.block_reads() - initial_reads;
        assert_eq!(cold_reads, 1);
        let cold_page = page_bytes(&viewer);

        let started = std::time::Instant::now();
        let warm_reads = viewer.source.block_reads();
        for _ in 0..3 {
            viewer.page(size.rows, false).unwrap();
            viewer.render(&mut terminal, size).unwrap();
        }
        let warm_elapsed = started.elapsed();
        assert_eq!(viewer.source.block_reads(), warm_reads);

        let started = std::time::Instant::now();
        let long_accesses_before = viewer.source.block_accesses();
        viewer.bottom().unwrap();
        viewer.render(&mut terminal, size).unwrap();
        let long_line_elapsed = started.elapsed();
        let long_line_reads = viewer.source.block_reads() - warm_reads;
        let long_line_accesses = viewer.source.block_accesses() - long_accesses_before;
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
            viewer.source.peak_cache_bytes(),
            page_before,
            initial_page,
            cold_page,
            final_page
        );

        fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn failed_page_load_restores_the_last_committed_display() {
        let path = temp_path("termfold-viewer-rollback");
        fs::write(&path, b"stable page\nnext page\n").unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();
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
        viewer.source.replace_file(directory);
        let error = viewer.render(&mut terminal, size).unwrap_err();

        assert!(!error.to_string().is_empty());
        assert_eq!(viewer.page, page);
        assert_eq!(viewer.state().position, state.position);
        assert_eq!(viewer.state().viewport, state.viewport);
        fs::remove_file(path).unwrap();
    }
}
