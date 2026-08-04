use std::{
    collections::VecDeque,
    io,
    path::{Path, PathBuf},
};

use crate::{session::Size, terminal::Terminal};
mod line;
mod frame;
mod source;
mod text;
pub(super) mod worker;

use frame::{FrameContext, FrameKey, FrameSlots, PageFrame};
use line::{LineBoundary, LineScanner, ScanStep};
use source::FileSource;
#[cfg(test)]
use source::{BLOCK_CACHE_SIZE, BLOCK_SIZE};
use text::DecodedText;

const MAX_MATCH_OFFSETS: usize = 4096;

#[derive(Clone, Debug)]
struct SearchState {
    query: Vec<u8>,
    forward: bool,
    offset: u64,
}

pub(super) struct SearchWork {
    query: Vec<u8>,
    forward: bool,
    ranges: VecDeque<(u64, u64)>,
}

pub(super) enum SearchStart {
    Complete(bool),
    Work(SearchWork),
}

pub(super) enum SearchStep {
    Complete(bool),
    Continue,
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
    frames: FrameSlots,
    mode: u8,
    generation: u64,
    pending_page_direction: Option<bool>,
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
            frames: FrameSlots::default(),
            mode: 0,
            generation: 0,
            pending_page_direction: None,
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

    fn current_frame(&self) -> Option<&PageFrame> {
        self.frames.current()
    }

    fn frame_context(&self, size: Size) -> FrameContext {
        FrameContext::new(
            self.length(),
            self.mode,
            size,
            self.tab_width,
            self.generation,
        )
    }

    #[cfg(test)]
    pub(super) fn invalidate_frames(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.frames.clear();
        self.pending_page_direction = None;
    }

    pub fn render(&mut self, terminal: &mut Terminal, size: Size) -> io::Result<()> {
        let committed = self.committed;
        let previous_frames = self.frames.clone();
        let previous_direction = self.pending_page_direction;
        self.visible_rows = usize::from(size.rows).max(1);
        self.visible_columns = usize::from(size.columns).max(1);
        let columns = usize::from(size.columns);
        let result = (|| {
            self.position = self.position.min(self.length());
            self.viewport = self.viewport.min(self.length());
            self.viewport = self.line_start_at(self.viewport)?;

            let context = self.frame_context(size);
            let key = FrameKey::new(context, self.viewport);
            let direction = self.pending_page_direction;
            let same_context = self.frames.context() == Some(context);
            if same_context && self.frames.current().is_some() {
                self.validate_snapshot()?;
            }
            let mut rotated = None;
            let frame = if same_context && self.frames.current_matches(key) {
                self.frames
                    .current()
                    .cloned()
                    .ok_or_else(|| io::Error::other("viewer current frame disappeared"))?
            } else if same_context
                && direction == Some(true)
                && self.frames.neighbour_matches(key, true)
            {
                rotated = Some(true);
                self.frames
                    .neighbour(true)
                    .cloned()
                    .ok_or_else(|| io::Error::other("viewer next frame disappeared"))?
            } else if same_context
                && direction == Some(false)
                && self.frames.neighbour_matches(key, false)
            {
                rotated = Some(false);
                self.frames
                    .neighbour(false)
                    .cloned()
                    .ok_or_else(|| io::Error::other("viewer previous frame disappeared"))?
            } else {
                self.build_page_at(size, self.viewport, context)?
            };

            let cursor_line = self.line_start_at(self.position)?;
            let cursor_row = frame.cursor_row(cursor_line);
            let cursor_column = match cursor_row {
                Some(_) => frame
                    .cursor_column(cursor_line, self.position, self.horizontal, columns)
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "viewer frame has no cursor stop",
                        )
                    })?,
                None => 0,
            };

            terminal.resize(size).map_err(io::Error::other)?;

            self.frames.set_context(context);
            if let Some(forward) = rotated {
                let success = if forward {
                    self.frames.rotate_forward(key)
                } else {
                    self.frames.rotate_backward(key)
                };
                if !success {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "viewer neighbour frame is no longer valid",
                    ));
                }
            } else if !self.frames.current_matches(key) {
                self.frames.commit(frame.clone(), direction);
            }
            self.committed = self.state();
            self.pending_page_direction = None;

            terminal.advance(b"\x1b[2J\x1b[H");
            for row in 0..frame.rows.len() {
                terminal.advance(frame.render_row(row, self.horizontal, columns).as_bytes());
                terminal.advance(b"\r\n");
            }
            let (row, column) = match cursor_row {
                Some(row) => (row.saturating_add(1), cursor_column.saturating_add(1)),
                None => (1, 1),
            };
            terminal.advance(format!("\x1b[{row};{column}H").as_bytes());

            if direction.is_some() {
                self.prefetch_neighbour(size, direction.unwrap_or(true));
            }
            Ok(())
        })();
        if result.is_err() {
            self.restore_state(committed);
            self.frames = previous_frames;
            self.pending_page_direction = previous_direction;
        }
        result
    }

    fn build_page_at(
        &mut self,
        size: Size,
        viewport: u64,
        context: FrameContext,
    ) -> io::Result<PageFrame> {
        let length = self.length();
        frame::build(
            &mut self.source,
            &mut self.lines,
            length,
            self.tab_width,
            viewport,
            usize::from(size.rows),
            &self.matches,
            self.search.as_ref().map(|search| search.query.len()),
            context,
        )
    }

    fn validate_snapshot(&mut self) -> io::Result<()> {
        if self.length() > 0 && self.source.read_byte(0)?.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "viewer snapshot is no longer readable",
            ));
        }
        Ok(())
    }

    fn prefetch_neighbour(&mut self, size: Size, forward: bool) {
        let Some(current_start) = self.current_frame().map(|frame| frame.key.source_start) else {
            return;
        };
        let steps = usize::from(size.rows.saturating_sub(2).max(1));
        let mut start = current_start;
        for _ in 0..steps {
            let next = if forward {
                self.next_line(start)
            } else {
                self.previous_line(start)
            };
            let Ok(next) = next else {
                return;
            };
            if (forward && next <= start) || (!forward && next >= start) {
                return;
            }
            start = next;
        }
        if (forward && start >= self.length()) || (!forward && start == current_start) {
            return;
        }

        let context = self.frame_context(size);
        if self
            .frames
            .neighbour_matches(FrameKey::new(context, start), forward)
        {
            return;
        }
        let Ok(frame) = self.build_page_at(size, start, context) else {
            return;
        };
        if frame.key.source_start == start {
            self.frames.insert_neighbour(frame, forward);
        }
    }

    pub fn move_lines(&mut self, amount: i32) -> io::Result<()> {
        self.navigation(|viewer| viewer.move_lines_inner(amount))
    }

    fn move_lines_inner(&mut self, amount: i32) -> io::Result<()> {
        let current_line = self.line_start_at(self.position)?;
        let preferred = self
            .preferred_column
            .max(self.cursor_cell(current_line, self.position)? as u64);
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
        let (position, _) = self.cursor_at_cell(target, preferred)?;
        self.position = position;
        self.preferred_column = preferred;
        self.adjust_horizontal()?;
        self.ensure_cursor_visible()?;
        Ok(())
    }

    pub fn page(&mut self, rows: u16, forward: bool) -> io::Result<()> {
        let result = self.navigation(|viewer| {
            viewer.visible_rows = usize::from(rows).max(1);
            let lines = usize::from(rows.saturating_sub(2).max(1));
            let amount = i32::try_from(lines).unwrap_or(i32::MAX);
            viewer.move_lines_inner(if forward { amount } else { -amount })
        });
        if result.is_ok() {
            self.pending_page_direction = Some(forward);
        }
        result
    }

    pub fn half_page(&mut self, rows: u16, forward: bool) -> io::Result<()> {
        let result = self.navigation(|viewer| {
            viewer.visible_rows = usize::from(rows).max(1);
            let lines = usize::from(rows.saturating_sub(2).max(1)) / 2;
            let amount = i32::try_from(lines.max(1)).unwrap_or(i32::MAX);
            viewer.move_lines_inner(if forward { amount } else { -amount })
        });
        if result.is_ok() {
            self.pending_page_direction = Some(forward);
        }
        result
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
            let (position, cell) = viewer.cursor_at_cell(line, usize::MAX as u64)?;
            viewer.position = position;
            viewer.preferred_column = cell;
            viewer.adjust_horizontal()?;
            viewer.ensure_cursor_visible()
        })
    }

    pub fn line_start(&mut self) -> io::Result<()> {
        self.navigation(|viewer| {
            let line = viewer.line_start_at(viewer.position)?;
            viewer.position = viewer.cursor_at_cell(line, 0)?.0;
            viewer.preferred_column = 0;
            viewer.horizontal = 0;
            viewer.ensure_cursor_visible()
        })
    }

    pub fn line_end(&mut self, columns: usize) -> io::Result<()> {
        self.navigation(|viewer| {
            let start = viewer.line_start_at(viewer.position)?;
            let (position, cell) = viewer.cursor_at_cell(start, usize::MAX as u64)?;
            viewer.position = position;
            viewer.preferred_column = cell;
            viewer.horizontal = cell.saturating_sub(columns.max(1) as u64 - 1);
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

    #[cfg(test)]
    pub fn search(&mut self, query: &str, forward: bool) -> io::Result<bool> {
        let query = query.as_bytes().to_vec();
        let has_query = !query.is_empty();
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
        } else if has_query {
            self.invalidate_frames();
        }
        result
    }

    #[cfg(test)]
    pub fn repeat_search(&mut self, same_direction: bool) -> io::Result<bool> {
        let had_search = self.search.is_some();
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
        } else if had_search {
            self.invalidate_frames();
        }
        result
    }

    pub(super) fn begin_search_work(&mut self, query: Vec<u8>, forward: bool) -> SearchStart {
        self.matches.clear();
        self.search_work_at(query, forward, self.position)
    }

    pub(super) fn begin_repeat_search_work(
        &mut self,
        same_direction: bool,
    ) -> io::Result<SearchStart> {
        let Some(previous) = self.search.clone() else {
            return Ok(SearchStart::Complete(false));
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
            return Ok(SearchStart::Complete(true));
        }
        self.matches.clear();
        Ok(self.search_work_at(previous.query, forward, start))
    }

    fn search_work_at(&mut self, query: Vec<u8>, forward: bool, start: u64) -> SearchStart {
        if query.is_empty() {
            return SearchStart::Complete(false);
        }
        let Some(maximum) = self.length().checked_sub(query.len() as u64) else {
            self.search = None;
            return SearchStart::Complete(false);
        };
        let start = start.min(maximum);
        let mut ranges = VecDeque::with_capacity(2);
        if forward {
            ranges.push_back((start, maximum));
            if start > 0 {
                ranges.push_back((0, start - 1));
            }
        } else {
            ranges.push_back((start, 0));
            if start < maximum {
                ranges.push_back((maximum, start + 1));
            }
        }
        SearchStart::Work(SearchWork {
            query,
            forward,
            ranges,
        })
    }

    pub(super) fn step_search_work(&mut self, work: &mut SearchWork) -> io::Result<SearchStep> {
        let Some((start, end)) = work.ranges.front_mut() else {
            self.search = None;
            return Ok(SearchStep::Complete(false));
        };
        let (scan_start, scan_end, exhausted) = if work.forward {
            let scan_end = start
                .saturating_add(source::BLOCK_SIZE.saturating_sub(1))
                .min(*end);
            let scan_start = *start;
            *start = scan_end.saturating_add(1);
            (scan_start, scan_end, scan_end == *end)
        } else {
            let scan_start = start
                .saturating_sub(source::BLOCK_SIZE.saturating_sub(1))
                .max(*end);
            let scan_end = *start;
            *start = scan_start.saturating_sub(1);
            (scan_end, scan_start, scan_start == *end)
        };
        let found = if work.forward {
            self.collect_forward(&work.query, scan_start, scan_end)?
        } else {
            self.collect_reverse(&work.query, scan_start, scan_end)?
        };
        if exhausted {
            work.ranges.pop_front();
        }
        if let Some(offset) = found {
            self.set_position(offset)?;
            self.search = Some(SearchState {
                query: work.query.clone(),
                forward: work.forward,
                offset,
            });
            return Ok(SearchStep::Complete(true));
        }
        if work.ranges.is_empty() {
            self.search = None;
            Ok(SearchStep::Complete(false))
        } else {
            Ok(SearchStep::Continue)
        }
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
        frame::cache_line(&mut self.lines, line);
    }

    #[cfg(test)]
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

    #[cfg(test)]
    fn read_line(
        &mut self,
        start: u64,
        columns: usize,
        budget: usize,
    ) -> io::Result<(u64, String, bool)> {
        let line = self.line_boundary(start)?;
        if budget == 0 {
            return Ok((line.next, String::new(), line.complete));
        }
        let decoded = self.decode_line(&line)?;
        let first_cell = self.horizontal.min(decoded.width as u64) as usize;
        let rendered = if first_cell == 0 {
            decoded.render(columns)
        } else {
            decoded.render_cells(first_cell..first_cell.saturating_add(columns))
        };
        Ok((
            line.next,
            rendered,
            line.complete,
        ))
    }

    fn decode_line(&mut self, line: &LineBoundary) -> io::Result<DecodedText> {
        frame::decode_line(&mut self.source, line, self.tab_width)
    }

    fn line_boundary(&mut self, start: u64) -> io::Result<LineBoundary> {
        let length = self.length();
        frame::line_boundary(
            &mut self.source,
            &mut self.lines,
            length,
            start,
        )
    }

    fn set_position(&mut self, position: u64) -> io::Result<()> {
        let requested = position.min(self.length());
        let line = self.line_start_at(requested)?;
        let boundary = self.line_boundary(line)?;
        let decoded = self.decode_line(&boundary)?;
        let source = requested
            .saturating_sub(line)
            .min(boundary.content_end.saturating_sub(line)) as usize;
        let source = decoded
            .cursor_source_at_source(source)
            .or_else(|| decoded.cursor_source_at_cell(0))
            .unwrap_or(0);
        self.position = line.saturating_add(source as u64);
        self.preferred_column = decoded
            .cursor_cell_at_source(source)
            .unwrap_or(0) as u64;
        self.adjust_horizontal()?;
        self.ensure_cursor_visible()
    }

    fn cursor_at_cell(&mut self, line: u64, cell: u64) -> io::Result<(u64, u64)> {
        let boundary = self.line_boundary(line)?;
        let decoded = self.decode_line(&boundary)?;
        let cell = cell.min(usize::MAX as u64) as usize;
        let source = decoded.cursor_source_at_cell(cell).unwrap_or(0);
        let actual_cell = decoded.cursor_cell_at_source(source).unwrap_or(0);
        Ok((line.saturating_add(source as u64), actual_cell as u64))
    }

    fn cursor_cell(&mut self, line: u64, position: u64) -> io::Result<usize> {
        let boundary = self.line_boundary(line)?;
        let decoded = self.decode_line(&boundary)?;
        let source = position
            .saturating_sub(line)
            .min(boundary.content_end.saturating_sub(line)) as usize;
        Ok(decoded.cursor_cell_at_source(source).unwrap_or(0))
    }

    fn adjust_horizontal(&mut self) -> io::Result<()> {
        let line = self.line_start_at(self.position)?;
        let column = self.cursor_cell(line, self.position)? as u64;
        let width = self.visible_columns.max(1) as u64;
        if column < self.horizontal {
            self.horizontal = column;
        } else if column >= self.horizontal.saturating_add(width) {
            self.horizontal = column.saturating_sub(width.saturating_sub(1));
        }
        Ok(())
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
        viewer.current_frame().map_or(0, |frame| {
            frame
                .rows
                .iter()
                .flat_map(|row| row.tokens.iter())
                .map(|token| token.rendered.capacity())
                .sum()
        })
    }

    fn page_line(viewer: &Viewer, row: usize, columns: usize) -> String {
        viewer
            .current_frame()
            .expect("rendered frame")
            .render_row(row, viewer.horizontal, columns)
    }

    #[test]
    fn uses_configured_tab_width_for_text() {
        let path = temp_path("termfold-viewer-tab-width");
        fs::write(&path, b"a\t\n").unwrap();
        let mut viewer = Viewer::open(path.clone(), 4).unwrap();

        let (_, line, complete) = viewer.read_line(0, 20, 64 * 1024).unwrap();
        assert_eq!(line, "a   ");
        assert!(complete);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn current_frame_contains_page_mapping_and_visible_matches() {
        let path = temp_path("termfold-viewer-frame");
        fs::write(&path, b"alpha\nbeta\n").unwrap();
        let size = Size {
            columns: 16,
            rows: 3,
        };
        let mut terminal = Terminal::new(size).unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();

        viewer.render(&mut terminal, size).unwrap();
        assert!(viewer.search("beta", true).unwrap());
        viewer.render(&mut terminal, size).unwrap();

        let frame = viewer.current_frame().expect("current frame");
        assert_eq!(frame.source_range, 0..11);
        assert_eq!(frame.rows.len(), 2);
        assert_eq!(
            frame
                .line_boundaries
                .iter()
                .map(|line| line.start)
                .collect::<Vec<_>>(),
            vec![0, 6]
        );
        assert_eq!(frame.source_cell_spans.len(), 9);
        assert_eq!(frame.cursor_stops.len(), 9);
        assert_eq!(frame.visible_match_ranges, vec![6..10]);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn empty_frame_renders_and_long_pages_stop_at_the_source_limit() {
        let empty_path = temp_path("termfold-viewer-empty-frame");
        fs::write(&empty_path, []).unwrap();
        let empty_size = Size {
            columns: 8,
            rows: 3,
        };
        let mut empty_terminal = Terminal::new(empty_size).unwrap();
        let mut empty = Viewer::open(empty_path.clone(), 8).unwrap();
        empty.render(&mut empty_terminal, empty_size).unwrap();
        let empty_frame = empty.current_frame().expect("empty frame");
        assert_eq!(empty_frame.source_range, 0..0);
        assert!(empty_frame.rows.is_empty());

        let long_path = temp_path("termfold-viewer-frame-limit");
        let mut data = Vec::with_capacity(64 * 1024 * 5);
        for _ in 0..5 {
            data.extend(std::iter::repeat_n(b'x', 64 * 1024 - 1));
            data.push(b'\n');
        }
        fs::write(&long_path, data).unwrap();
        let size = Size {
            columns: 16,
            rows: 8,
        };
        let mut terminal = Terminal::new(size).unwrap();
        let mut viewer = Viewer::open(long_path.clone(), 8).unwrap();
        viewer.render(&mut terminal, size).unwrap();
        let frame = viewer.current_frame().expect("bounded frame");
        assert_eq!(frame.source_bytes(), 256 * 1024);
        assert_eq!(frame.rows.len(), 4);

        fs::remove_file(empty_path).unwrap();
        fs::remove_file(long_path).unwrap();
    }

    #[test]
    fn frame_clipping_replacements_and_cursor_use_the_decoded_mapping() {
        let path = temp_path("termfold-viewer-frame-cursor");
        fs::write(&path, b"ab\0c\n").unwrap();
        let size = Size {
            columns: 3,
            rows: 2,
        };
        let mut terminal = Terminal::new(size).unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();
        viewer.horizontal = 1;
        viewer.position = 1;
        viewer.render(&mut terminal, size).unwrap();

        let frame = viewer.current_frame().expect("current frame");
        assert_eq!(frame.render_row(0, viewer.horizontal, 3), "b^@");
        assert_eq!(terminal.screen().cursor().column, 0);
        assert_eq!(frame.cursor_stops[1].source, 1);

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
    fn line_end_stops_at_the_last_token_for_every_line_ending() {
        let path = temp_path("termfold-viewer-cursor-eol");
        fs::write(&path, b"lf\ncrlf\r\ncr\rempty\nlast").unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();

        for (start, expected) in [(0, 1), (3, 6), (9, 10), (12, 16), (18, 21)] {
            viewer.position = start;
            viewer.line_end(80).unwrap();
            assert_eq!(viewer.position, expected);
            assert_eq!(
                viewer.cursor_cell(start, viewer.position).unwrap(),
                (expected - start) as usize
            );
        }

        let empty_path = temp_path("termfold-viewer-cursor-empty-line");
        fs::write(&empty_path, b"a\n\nb").unwrap();
        let mut empty = Viewer::open(empty_path.clone(), 8).unwrap();
        empty.line_end(80).unwrap();
        empty.move_lines(1).unwrap();
        assert_eq!((empty.position, empty.preferred_column), (2, 0));
        empty.move_lines(1).unwrap();
        assert_eq!(empty.position, 3);

        fs::remove_file(path).unwrap();
        fs::remove_file(empty_path).unwrap();
    }

    #[test]
    fn vertical_movement_preserves_display_cells_and_valid_cursor_stops() {
        let path = temp_path("termfold-viewer-cursor-cells");
        let mut data = b"abcX\na\tZ\n".to_vec();
        data.extend_from_slice("界e\u{301}z\n".as_bytes());
        data.extend_from_slice(&[0xff, b'x']);
        fs::write(&path, data).unwrap();
        let mut viewer = Viewer::open(path.clone(), 4).unwrap();

        viewer.line_end(80).unwrap();
        assert_eq!((viewer.position, viewer.preferred_column), (3, 3));

        viewer.move_lines(1).unwrap();
        assert_eq!(viewer.position, 6);
        assert_eq!(viewer.cursor_cell(5, viewer.position).unwrap(), 1);

        viewer.move_lines(1).unwrap();
        assert_eq!(viewer.position, 15);
        assert_eq!(viewer.cursor_cell(9, viewer.position).unwrap(), 3);

        viewer.move_lines(1).unwrap();
        assert_eq!(viewer.position, 17);
        assert_eq!(viewer.cursor_cell(17, viewer.position).unwrap(), 0);

        fs::remove_file(path).unwrap();
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
        assert_eq!(viewer.position, 24);

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
        assert_eq!(viewer.position, 15);
        viewer.move_lines(1).unwrap();
        assert_eq!(viewer.position, 26);

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
        let first_line = page_line(&viewer, 0, usize::from(size.columns));

        for _ in 0..(BLOCK_CACHE_SIZE * 3 / 2) {
            viewer.page(size.rows, true).unwrap();
            viewer.render(&mut terminal, size).unwrap();
            assert!(viewer.source.cache_block_count() <= BLOCK_CACHE_SIZE);
            assert!(cache_bytes(&viewer) <= BLOCK_SIZE as usize * BLOCK_CACHE_SIZE);
        }
        assert_ne!(page_line(&viewer, 0, usize::from(size.columns)), first_line);
        assert!(
            viewer.current_frame().expect("rendered frame").rows.len() <= usize::from(size.rows)
        );

        for _ in 0..(BLOCK_CACHE_SIZE * 3 / 2) {
            viewer.page(size.rows, false).unwrap();
            viewer.render(&mut terminal, size).unwrap();
            assert!(viewer.source.cache_block_count() <= BLOCK_CACHE_SIZE);
        }
        assert_eq!(page_line(&viewer, 0, usize::from(size.columns)), first_line);

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
        assert!(viewer.source.block_reads() <= 2);
        assert!(
            viewer.source.block_accesses() <= 8,
            "block accesses: {}",
            viewer.source.block_accesses()
        );

        viewer.source.reset_metrics();
        viewer.page(size.rows, false).unwrap();
        assert!(viewer.source.block_reads() <= 2);
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
            viewer.source.block_reads() <= 2,
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
            viewer.source.block_reads() <= 2,
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
        assert!(cold_reads <= 2, "cold paging reads including prefetch: {cold_reads}");
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
        let page_source = viewer
            .current_frame()
            .map(|frame| frame.source_range.clone());
        let page_rows = viewer.current_frame().map(|frame| {
            frame
                .rows
                .iter()
                .map(|row| row.render(usize::from(size.columns)))
                .collect::<Vec<_>>()
        });
        let state = viewer.state();

        let directory = File::open(std::env::temp_dir()).unwrap();
        assert!(directory.metadata().unwrap().len() > 0);
        viewer.source.replace_file(directory);
        let error = viewer.render(&mut terminal, size).unwrap_err();

        assert!(!error.to_string().is_empty());
        assert_eq!(
            viewer
                .current_frame()
                .map(|frame| frame.source_range.clone()),
            page_source
        );
        assert_eq!(
            viewer.current_frame().map(|frame| {
                frame
                    .rows
                    .iter()
                    .map(|row| row.render(usize::from(size.columns)))
                    .collect::<Vec<_>>()
            }),
            page_rows
        );
        assert_eq!(viewer.state().position, state.position);
        assert_eq!(viewer.state().viewport, state.viewport);
        fs::remove_file(path).unwrap();
    }
}
