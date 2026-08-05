use std::{
    cmp::Reverse,
    collections::VecDeque,
    io,
    ops::Range,
    path::{Path, PathBuf},
};

use crate::{session::Size, terminal::Terminal};
mod frame;
mod hex;
mod line;
mod search;
mod source;
mod text;
pub(super) mod worker;

use frame::{FrameContext, FrameKey, FrameSlots, PageFrame};
use hex::bytes_per_row;
use line::{LineBoundary, LineScanner, ScanStep};
use search::SearchQuery;
use source::FileSource;
#[cfg(test)]
use source::{BLOCK_CACHE_SIZE, BLOCK_SIZE};
use text::DecodedText;

const MAX_MATCH_OFFSETS: usize = 4096;
pub(crate) const MAX_QUERY_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum ViewerMode {
    #[default]
    Text,
    Hex,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SearchState {
    query: SearchQuery,
    forward: bool,
    offset: u64,
}

pub(super) struct SearchWork {
    query: SearchQuery,
    forward: bool,
    ranges: VecDeque<Range<u64>>,
    wrap_range: Range<u64>,
    matches: Vec<u64>,
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
    search_wrapped: bool,
    frames: FrameSlots,
    mode: ViewerMode,
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
            search_wrapped: false,
            frames: FrameSlots::default(),
            mode: ViewerMode::Text,
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
            self.mode as u8,
            size,
            self.tab_width,
            self.generation,
        )
    }

    pub(super) fn invalidate_frames(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.frames.clear();
        self.pending_page_direction = None;
    }

    pub(super) fn rollback_pending_page(&mut self) {
        self.restore_state(self.committed);
        self.pending_page_direction = None;
    }

    pub(super) fn toggle_mode(&mut self) -> io::Result<()> {
        let previous_mode = self.mode;
        let previous_state = self.state();
        self.mode = match self.mode {
            ViewerMode::Text => ViewerMode::Hex,
            ViewerMode::Hex => ViewerMode::Text,
        };
        let result = (|| {
            if self.mode == ViewerMode::Hex {
                if self.length() == 0 {
                    self.position = 0;
                    self.viewport = 0;
                } else {
                    self.position = self.position.min(self.length() - 1);
                    self.viewport = self.hex_row_start(self.viewport);
                    self.preferred_column = self.position % self.hex_width();
                    self.ensure_cursor_visible()?;
                }
                self.horizontal = 0;
                Ok(())
            } else {
                self.set_position(self.position.min(self.length()))
            }
        })();
        if result.is_err() {
            self.mode = previous_mode;
            self.restore_state(previous_state);
            return result;
        }
        self.invalidate_frames();
        Ok(())
    }

    pub fn render(&mut self, terminal: &mut Terminal, size: Size) -> io::Result<()> {
        let committed = self.committed;
        let previous_frames = self.frames.clone();
        let previous_direction = self.pending_page_direction;
        self.visible_rows = usize::from(size.rows).max(1);
        self.visible_columns = usize::from(size.columns).max(1);
        let hex = self.mode == ViewerMode::Hex;
        let columns = usize::from(size.columns);
        let result = (|| {
            if hex {
                let length = self.length();
                self.position = if length == 0 {
                    0
                } else {
                    self.position.min(length - 1)
                };
                self.viewport = self.hex_row_start(self.viewport);
                self.ensure_cursor_visible()?;
            } else {
                self.position = self.position.min(self.length());
                self.viewport = self.viewport.min(self.length());
                self.viewport = self.line_start_at(self.viewport)?;
            }

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
                self.build_page_at(self.viewport, context)?
            };

            let (cursor_row, cursor_column) = if hex {
                self.hex_cursor_position(&frame)
            } else {
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
                (cursor_row.unwrap_or(0), cursor_column)
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
            let row_count = if hex {
                frame
                    .hex
                    .as_ref()
                    .map_or(0, |page| page.rows.len().max(usize::from(page.narrow)))
            } else {
                frame.rows.len()
            };
            for row in 0..row_count {
                terminal.advance(frame.render_row(row, self.horizontal, columns).as_bytes());
                if row + 1 < row_count {
                    terminal.advance(b"\r\n");
                }
            }
            let (row, column) = (
                cursor_row.saturating_add(1),
                cursor_column.saturating_add(1),
            );
            terminal.advance(format!("\x1b[{row};{column}H").as_bytes());

            // ponytail: neighbour prefetch is disabled; add bounded low-priority work if cold-page latency matters.
            Ok(())
        })();
        if result.is_err() {
            self.restore_state(committed);
            self.frames = previous_frames;
            self.pending_page_direction = previous_direction;
        }
        result
    }

    fn hex_cursor_position(&self, frame: &PageFrame) -> (usize, usize) {
        let Some(page) = frame.hex.as_ref() else {
            return (0, 0);
        };
        if page.narrow || page.bytes_per_row == 0 {
            return (0, 0);
        }
        let row = self
            .position
            .saturating_sub(frame.source_range.start)
            .checked_div(page.bytes_per_row as u64)
            .unwrap_or(0) as usize;
        let byte =
            self.position.saturating_sub(frame.source_range.start) % page.bytes_per_row as u64;
        (
            row.min(page.rows.len().saturating_sub(1)),
            page.rows
                .get(row)
                .and_then(|row| row.hex_cells.get(byte as usize))
                .map_or(0, |cells| cells.start),
        )
    }

    fn build_page_at(&mut self, viewport: u64, context: FrameContext) -> io::Result<PageFrame> {
        frame::build(
            &mut self.source,
            &mut self.lines,
            viewport,
            self.search.as_ref().map(|search| &search.query),
            self.search.as_ref().map(|search| search.offset),
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

    pub fn move_lines(&mut self, amount: i32) -> io::Result<()> {
        if self.mode == ViewerMode::Hex {
            return self.navigation(|viewer| viewer.move_hex_rows(amount));
        }
        self.navigation(|viewer| viewer.move_lines_inner(amount))
    }

    pub fn move_horizontal(&mut self, amount: i32) -> io::Result<()> {
        self.navigation(|viewer| {
            if viewer.mode == ViewerMode::Hex {
                return viewer.move_hex_bytes(amount);
            }
            viewer.move_text_tokens(amount)
        })
    }

    fn move_text_tokens(&mut self, amount: i32) -> io::Result<()> {
        let line = self.line_start_at(self.position)?;
        let boundary = self.line_boundary(line)?;
        let decoded = self.decode_line(&boundary)?;
        let source = self
            .position
            .saturating_sub(line)
            .min(boundary.content_end.saturating_sub(line)) as usize;
        let mut token = decoded.cursor_source_at_source(source).unwrap_or(0);
        let steps = amount.unsigned_abs() as usize;
        for _ in 0..steps {
            let next = if amount < 0 {
                decoded.previous_cursor_stop(token)
            } else {
                decoded.next_cursor_stop(token)
            };
            let Some(next) = next else { break };
            token = next.source.start;
        }
        self.position = line.saturating_add(token as u64);
        self.preferred_column = decoded.cursor_cell_at_source(token).unwrap_or(0) as u64;
        self.adjust_horizontal()?;
        self.ensure_cursor_visible()
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

    fn hex_width(&self) -> u64 {
        bytes_per_row(self.visible_columns).max(1) as u64
    }

    fn hex_row_start(&self, position: u64) -> u64 {
        let width = self.hex_width();
        position.min(self.length()) / width * width
    }

    fn move_hex_rows(&mut self, amount: i32) -> io::Result<()> {
        if self.length() == 0 {
            self.position = 0;
            self.viewport = 0;
            return Ok(());
        }
        let width = self.hex_width();
        let last_row = (self.length() - 1) / width;
        let current_row = (self.position.min(self.length() - 1)) / width;
        let preferred = if self.preferred_column == 0 {
            self.position % width
        } else {
            self.preferred_column.min(width - 1)
        };
        let target_row = if amount < 0 {
            current_row.saturating_sub(amount.unsigned_abs() as u64)
        } else {
            current_row.saturating_add(amount as u64).min(last_row)
        };
        self.position = target_row
            .saturating_mul(width)
            .saturating_add(preferred)
            .min(self.length() - 1);
        self.preferred_column = self.position % width;
        self.ensure_cursor_visible()
    }

    fn move_hex_bytes(&mut self, amount: i32) -> io::Result<()> {
        if self.length() == 0 {
            self.position = 0;
            self.viewport = 0;
            self.preferred_column = 0;
            return Ok(());
        }
        let last = self.length() - 1;
        self.position = if amount < 0 {
            self.position
                .min(last)
                .saturating_sub(amount.unsigned_abs() as u64)
        } else {
            self.position
                .min(last)
                .saturating_add(amount as u64)
                .min(last)
        };
        self.preferred_column = self.position % self.hex_width();
        self.ensure_cursor_visible()
    }

    fn hex_bottom(&mut self) -> io::Result<()> {
        if self.length() == 0 {
            self.position = 0;
            self.viewport = 0;
            return Ok(());
        }
        self.position = self.length() - 1;
        self.preferred_column = self.position % self.hex_width();
        self.ensure_cursor_visible()
    }

    fn hex_line_start(&mut self) -> io::Result<()> {
        self.position = self.hex_row_start(self.position);
        self.preferred_column = 0;
        self.ensure_cursor_visible()
    }

    fn hex_line_end(&mut self) -> io::Result<()> {
        if self.length() == 0 {
            self.position = 0;
            self.viewport = 0;
            return Ok(());
        }
        let width = self.hex_width();
        self.position = self
            .hex_row_start(self.position)
            .saturating_add(width - 1)
            .min(self.length() - 1);
        self.preferred_column = self.position % width;
        self.ensure_cursor_visible()
    }

    fn scroll_hex_viewport(&mut self, amount: i32) -> io::Result<()> {
        let width = self.hex_width();
        let last_row = self.length().saturating_sub(1) / width;
        let row = self.viewport / width;
        let target = if amount < 0 {
            row.saturating_sub(amount.unsigned_abs() as u64)
        } else {
            row.saturating_add(amount as u64).min(last_row)
        };
        self.viewport = target.saturating_mul(width);
        Ok(())
    }

    pub fn page(&mut self, rows: u16, forward: bool) -> io::Result<()> {
        if self.mode == ViewerMode::Hex {
            let result = self.navigation(|viewer| {
                viewer.visible_rows = usize::from(rows).max(1);
                let lines = usize::from(rows.saturating_sub(2).max(1));
                let amount = i32::try_from(lines).unwrap_or(i32::MAX);
                viewer.move_hex_rows(if forward { amount } else { -amount })
            });
            if result.is_ok() {
                self.pending_page_direction = Some(forward);
            }
            return result;
        }
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
        if self.mode == ViewerMode::Hex {
            let result = self.navigation(|viewer| {
                viewer.visible_rows = usize::from(rows).max(1);
                let lines = usize::from(rows.saturating_sub(2).max(1)) / 2;
                let amount = i32::try_from(lines.max(1)).unwrap_or(i32::MAX);
                viewer.move_hex_rows(if forward { amount } else { -amount })
            });
            if result.is_ok() {
                self.pending_page_direction = Some(forward);
            }
            return result;
        }
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
        if self.mode == ViewerMode::Hex {
            return self.navigation(|viewer| viewer.hex_bottom());
        }
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
        if self.mode == ViewerMode::Hex {
            return self.navigation(|viewer| viewer.hex_line_start());
        }
        self.navigation(|viewer| {
            let line = viewer.line_start_at(viewer.position)?;
            viewer.position = viewer.cursor_at_cell(line, 0)?.0;
            viewer.preferred_column = 0;
            viewer.horizontal = 0;
            viewer.ensure_cursor_visible()
        })
    }

    pub fn line_end(&mut self, columns: usize) -> io::Result<()> {
        if self.mode == ViewerMode::Hex {
            return self.navigation(|viewer| viewer.hex_line_end());
        }
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
        if self.mode == ViewerMode::Hex {
            return self.navigation(|viewer| viewer.scroll_hex_viewport(amount));
        }
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
        let state = self.state();
        let matches = self.matches.clone();
        let search = self.search.clone();
        let result = (|| match self.begin_search_work(query.as_bytes().to_vec(), forward) {
            SearchStart::Complete(found) => Ok(found),
            SearchStart::Work(mut work) => loop {
                match self.step_search_work(&mut work)? {
                    SearchStep::Continue => continue,
                    SearchStep::Complete(found) => break Ok(found),
                }
            },
        })();
        if result.is_err() {
            self.restore_state(state);
            self.matches = matches;
            self.search = search;
        }
        result
    }

    #[cfg(test)]
    pub fn repeat_search(&mut self, same_direction: bool) -> io::Result<bool> {
        let state = self.state();
        let matches = self.matches.clone();
        let search = self.search.clone();
        let result = (|| match self.begin_repeat_search_work(same_direction)? {
            SearchStart::Complete(found) => Ok(found),
            SearchStart::Work(mut work) => loop {
                match self.step_search_work(&mut work)? {
                    SearchStep::Continue => continue,
                    SearchStep::Complete(found) => break Ok(found),
                }
            },
        })();
        if result.is_err() {
            self.restore_state(state);
            self.matches = matches;
            self.search = search;
        }
        result
    }

    pub(super) fn begin_search_work(&mut self, input: Vec<u8>, forward: bool) -> SearchStart {
        self.search_wrapped = false;
        let Ok(query) = SearchQuery::parse(&input) else {
            return SearchStart::Complete(false);
        };
        self.search_work_at(query, forward, self.position)
    }

    pub(super) fn begin_repeat_search_work(
        &mut self,
        same_direction: bool,
    ) -> io::Result<SearchStart> {
        self.search_wrapped = false;
        let Some(previous) = self.search.clone() else {
            return Ok(SearchStart::Complete(false));
        };
        let forward = if same_direction {
            previous.forward
        } else {
            !previous.forward
        };
        if let Some(offset) = self.cached_match(previous.offset, forward) {
            self.set_position(offset)?;
            self.search = Some(SearchState {
                query: previous.query,
                forward,
                offset,
            });
            self.invalidate_frames();
            return Ok(SearchStart::Complete(true));
        }
        Ok(self.search_work_at_excluding(
            previous.query,
            forward,
            previous.offset,
            Some(previous.offset),
        ))
    }

    fn search_work_at(&mut self, query: SearchQuery, forward: bool, start: u64) -> SearchStart {
        self.search_work_at_excluding(query, forward, start, None)
    }

    fn search_work_at_excluding(
        &mut self,
        query: SearchQuery,
        forward: bool,
        start: u64,
        excluded: Option<u64>,
    ) -> SearchStart {
        if query.as_bytes().is_empty() {
            return SearchStart::Complete(false);
        }
        let Some(maximum) = self.length().checked_sub(query.len() as u64) else {
            return SearchStart::Complete(false);
        };
        let query_len = query.len() as u64;
        let start = if forward {
            start.min(maximum.saturating_add(1))
        } else {
            start.min(maximum)
        };
        let limit = maximum.saturating_add(1);
        let primary = if forward {
            start..limit
        } else {
            0..start.saturating_add(1)
        };
        let current = self
            .current_frame()
            .and_then(|frame| frame_candidate_range(frame.source_range.clone(), query_len))
            .and_then(|range| intersect_range(range, &primary));
        let neighbour = self
            .frames
            .neighbour(forward)
            .and_then(|frame| frame_candidate_range(frame.source_range.clone(), query_len))
            .and_then(|range| intersect_range(range, &primary));

        let mut ranges = VecDeque::with_capacity(5);
        let mut selected = Vec::new();
        if let Some(offset) = excluded.filter(|offset| *offset < limit) {
            selected.push(offset..offset.saturating_add(1));
        }
        for priority in [current, neighbour].into_iter().flatten() {
            let mut pieces = subtract_range(priority, &selected);
            order_ranges(&mut pieces, forward);
            selected.extend(pieces.iter().cloned());
            ranges.extend(pieces);
        }
        let mut remaining = subtract_range(primary, &selected);
        order_ranges(&mut remaining, forward);
        ranges.extend(remaining);

        let wrap = if forward {
            0..start.min(limit)
        } else {
            start.saturating_add(1).min(limit)..limit
        };
        let mut wrapped = subtract_range(wrap.clone(), &selected);
        order_ranges(&mut wrapped, forward);
        ranges.extend(wrapped);

        SearchStart::Work(SearchWork {
            query,
            forward,
            ranges,
            wrap_range: wrap,
            matches: Vec::new(),
        })
    }

    pub(super) fn step_search_work(&mut self, work: &mut SearchWork) -> io::Result<SearchStep> {
        let Some(range) = work.ranges.front().cloned() else {
            self.search_wrapped = false;
            return Ok(SearchStep::Complete(false));
        };
        let query_len = work.query.len();
        let candidate_limit = source::BLOCK_SIZE as usize - query_len + 1;
        let (scan_start, scan_end) = if work.forward {
            let scan_end = range
                .start
                .saturating_add(candidate_limit as u64)
                .min(range.end);
            (range.start, scan_end)
        } else {
            let scan_start = range
                .end
                .saturating_sub(candidate_limit as u64)
                .max(range.start);
            (scan_start, range.end)
        };
        let read_end = scan_end
            .saturating_add(query_len.saturating_sub(1) as u64)
            .min(self.length());
        let bytes = self.source.read_range(scan_start..read_end)?;
        let candidate_count = (scan_end - scan_start) as usize;
        let mut found = None;
        if work.forward {
            for offset in 0..candidate_count {
                let end = offset.saturating_add(query_len);
                if end <= bytes.len() && work.query.matches_bytes(&bytes[offset..end]) {
                    let source = scan_start + offset as u64;
                    remember_match(&mut work.matches, source);
                    found.get_or_insert(source);
                }
            }
        } else {
            for offset in (0..candidate_count).rev() {
                let end = offset.saturating_add(query_len);
                if end <= bytes.len() && work.query.matches_bytes(&bytes[offset..end]) {
                    let source = scan_start + offset as u64;
                    remember_match(&mut work.matches, source);
                    found.get_or_insert(source);
                }
            }
        }
        if work.forward {
            if let Some(range) = work.ranges.front_mut() {
                range.start = scan_end;
            }
        } else if let Some(range) = work.ranges.front_mut() {
            range.end = scan_start;
        }
        if work.ranges.front().is_some_and(|range| range.is_empty()) {
            work.ranges.pop_front();
        }
        if let Some(offset) = found {
            self.set_position(offset)?;
            self.matches = std::mem::take(&mut work.matches);
            self.search_wrapped = work.wrap_range.contains(&offset);
            self.search = Some(SearchState {
                query: work.query.clone(),
                forward: work.forward,
                offset,
            });
            self.invalidate_frames();
            return Ok(SearchStep::Complete(true));
        }
        if work.ranges.is_empty() {
            self.search_wrapped = false;
            Ok(SearchStep::Complete(false))
        } else {
            Ok(SearchStep::Continue)
        }
    }

    pub(super) fn search_wrapped(&self) -> bool {
        self.search_wrapped
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

    fn cached_match(&self, start: u64, forward: bool) -> Option<u64> {
        if forward {
            self.matches.iter().copied().find(|offset| *offset > start)
        } else {
            self.matches
                .iter()
                .copied()
                .filter(|offset| *offset < start)
                .max()
        }
    }

    fn decode_line(&mut self, line: &LineBoundary) -> io::Result<DecodedText> {
        frame::decode_line(&mut self.source, line, self.tab_width)
    }

    fn line_boundary(&mut self, start: u64) -> io::Result<LineBoundary> {
        let length = self.length();
        frame::line_boundary(&mut self.source, &mut self.lines, length, start)
    }

    fn set_position(&mut self, position: u64) -> io::Result<()> {
        if self.mode == ViewerMode::Hex {
            if self.length() == 0 {
                self.position = 0;
                self.viewport = 0;
            } else {
                self.position = position.min(self.length() - 1);
                self.preferred_column = self.position % self.hex_width();
                self.ensure_cursor_visible()?;
            }
            return Ok(());
        }
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
        self.preferred_column = decoded.cursor_cell_at_source(source).unwrap_or(0) as u64;
        self.adjust_horizontal()?;
        self.ensure_cursor_visible()
    }

    fn cursor_at_cell(&mut self, line: u64, cell: u64) -> io::Result<(u64, u64)> {
        if self.mode == ViewerMode::Hex {
            if self.length() == 0 {
                return Ok((0, 0));
            }
            let width = self.hex_width();
            let position = self
                .hex_row_start(line)
                .saturating_add(cell.min(width - 1))
                .min(self.length() - 1);
            return Ok((position, position % width));
        }
        let boundary = self.line_boundary(line)?;
        let decoded = self.decode_line(&boundary)?;
        let cell = cell.min(usize::MAX as u64) as usize;
        let source = decoded.cursor_source_at_cell(cell).unwrap_or(0);
        let actual_cell = decoded.cursor_cell_at_source(source).unwrap_or(0);
        Ok((line.saturating_add(source as u64), actual_cell as u64))
    }

    fn cursor_cell(&mut self, line: u64, position: u64) -> io::Result<usize> {
        if self.mode == ViewerMode::Hex {
            return Ok(position.saturating_sub(self.hex_row_start(line)) as usize);
        }
        let boundary = self.line_boundary(line)?;
        let decoded = self.decode_line(&boundary)?;
        let source = position
            .saturating_sub(line)
            .min(boundary.content_end.saturating_sub(line)) as usize;
        Ok(decoded.cursor_cell_at_source(source).unwrap_or(0))
    }

    fn adjust_horizontal(&mut self) -> io::Result<()> {
        if self.mode == ViewerMode::Hex {
            self.horizontal = 0;
            return Ok(());
        }
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
        if self.mode == ViewerMode::Hex {
            if self.length() == 0 {
                self.viewport = 0;
                return Ok(());
            }
            let width = self.hex_width();
            let cursor_row = self.position.min(self.length() - 1) / width;
            let mut viewport_row = self.viewport / width;
            let rows = self.visible_rows.max(1) as u64;
            if cursor_row < viewport_row {
                viewport_row = cursor_row;
            } else if cursor_row >= viewport_row.saturating_add(rows) {
                viewport_row = cursor_row.saturating_sub(rows - 1);
            }
            self.viewport = viewport_row.saturating_mul(width);
            return Ok(());
        }
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
        if self.mode == ViewerMode::Hex {
            return Ok(self
                .hex_row_start(start)
                .saturating_add(self.hex_width())
                .min(self.length()));
        }
        Ok(self.line_boundary(start)?.next)
    }

    fn previous_line(&mut self, start: u64) -> io::Result<u64> {
        if self.mode == ViewerMode::Hex {
            return Ok(self.hex_row_start(start).saturating_sub(self.hex_width()));
        }
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
        if self.mode == ViewerMode::Hex {
            return Ok(self
                .length()
                .saturating_sub(1)
                .checked_div(self.hex_width())
                .map_or(0, |row| row.saturating_mul(self.hex_width())));
        }
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
        if self.mode == ViewerMode::Hex {
            return Ok(self.hex_row_start(position));
        }
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

fn frame_candidate_range(frame: Range<u64>, query_len: u64) -> Option<Range<u64>> {
    let end = frame.end.checked_sub(query_len.saturating_sub(1))?;
    (frame.start < end).then_some(frame.start..end)
}

fn intersect_range(first: Range<u64>, second: &Range<u64>) -> Option<Range<u64>> {
    let start = first.start.max(second.start);
    let end = first.end.min(second.end);
    (start < end).then_some(start..end)
}

fn subtract_range(base: Range<u64>, excluded: &[Range<u64>]) -> Vec<Range<u64>> {
    let mut parts = vec![base];
    for cut in excluded {
        let mut remaining = Vec::with_capacity(parts.len() + 1);
        for part in parts {
            if cut.end <= part.start || cut.start >= part.end {
                remaining.push(part);
                continue;
            }
            if part.start < cut.start {
                remaining.push(part.start..cut.start.min(part.end));
            }
            if cut.end < part.end {
                remaining.push(cut.end.max(part.start)..part.end);
            }
        }
        parts = remaining;
    }
    parts
}

fn order_ranges(ranges: &mut [Range<u64>], forward: bool) {
    if forward {
        ranges.sort_by_key(|range| range.start);
    } else {
        ranges.sort_by_key(|range| Reverse(range.end));
    }
}

fn remember_match(matches: &mut Vec<u64>, offset: u64) {
    if matches.len() < MAX_MATCH_OFFSETS {
        matches.push(offset);
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
    fn hex_frame_renders_and_navigates_by_bytes() {
        let path = temp_path("termfold-viewer-hex");
        fs::write(&path, (0..40).collect::<Vec<_>>()).unwrap();
        let size = Size {
            columns: 80,
            rows: 4,
        };
        let mut terminal = Terminal::new(size).unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();
        viewer.mode = ViewerMode::Hex;

        viewer.render(&mut terminal, size).unwrap();
        let frame = viewer.current_frame().expect("hex frame");
        let page = frame.hex.as_ref().expect("hex page");
        assert_eq!(page.bytes_per_row, 16);
        assert_eq!(frame.source_range, 0..40);
        assert_eq!(frame.source_cell_spans.len(), 40);
        assert_eq!(frame.cursor_stops.len(), 40);
        assert_eq!(
            page.render_row(0),
            "00000000  00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F  ................"
        );

        viewer.move_lines(1).unwrap();
        assert_eq!(viewer.position, 16);
        viewer.line_end(80).unwrap();
        assert_eq!(viewer.position, 31);
        viewer.page(size.rows, true).unwrap();
        assert_eq!(viewer.position, 39);
        viewer.top();
        viewer.bottom().unwrap();
        assert_eq!(viewer.position, 39);
        assert!(viewer.lines.is_empty());

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn hex_narrow_render_preserves_byte_position() {
        let path = temp_path("termfold-viewer-hex-narrow");
        fs::write(&path, b"0123456789").unwrap();
        let size = Size {
            columns: 27,
            rows: 3,
        };
        let mut terminal = Terminal::new(size).unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();
        viewer.mode = ViewerMode::Hex;
        viewer.position = 5;

        viewer.render(&mut terminal, size).unwrap();
        assert_eq!(viewer.position, 5);
        let frame = viewer.current_frame().expect("narrow frame");
        assert_eq!(
            frame.render_row(0, 0, usize::from(size.columns)),
            hex::NARROW_MESSAGE
        );
        assert!(frame.hex.as_ref().expect("hex page").narrow);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn toggling_mode_invalidates_frames_and_preserves_byte_position() {
        let path = temp_path("termfold-viewer-toggle-mode");
        fs::write(&path, b"0123456789\n").unwrap();
        let size = Size {
            columns: 80,
            rows: 4,
        };
        let mut terminal = Terminal::new(size).unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();
        viewer.position = 5;
        viewer.render(&mut terminal, size).unwrap();
        let generation = viewer.generation;

        viewer.toggle_mode().unwrap();
        assert_eq!(viewer.mode, ViewerMode::Hex);
        assert_eq!(viewer.position, 5);
        assert!(viewer.current_frame().is_none());
        assert!(viewer.generation > generation);
        viewer.render(&mut terminal, size).unwrap();
        assert!(viewer.current_frame().unwrap().hex.is_some());

        viewer.toggle_mode().unwrap();
        assert_eq!(viewer.mode, ViewerMode::Text);
        assert_eq!(viewer.position, 5);
        assert!(viewer.current_frame().is_none());

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn uses_configured_tab_width_for_text() {
        let path = temp_path("termfold-viewer-tab-width");
        fs::write(&path, b"a\t\n").unwrap();
        let size = Size {
            columns: 20,
            rows: 2,
        };
        let mut terminal = Terminal::new(size).unwrap();
        let mut viewer = Viewer::open(path.clone(), 4).unwrap();

        viewer.render(&mut terminal, size).unwrap();
        assert_eq!(viewer.current_frame().unwrap().render_row(0, 0, 20), "a   ");

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
    fn highlights_all_visible_matches_with_the_active_match_distinct() {
        let path = temp_path("termfold-viewer-highlights");
        let mut data = b"a\t\xE7\x95\x8C".to_vec();
        data.extend_from_slice(&[0xff]);
        data.extend_from_slice(b"hit hit\n");
        fs::write(&path, data).unwrap();
        let size = Size {
            columns: 32,
            rows: 3,
        };
        let mut terminal = Terminal::new(size).unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();

        viewer.render(&mut terminal, size).unwrap();
        assert!(viewer.search("hit", true).unwrap());
        viewer.render(&mut terminal, size).unwrap();

        let frame = viewer.current_frame().expect("current frame");
        assert_eq!(frame.visible_match_ranges, vec![6..9, 10..13]);
        assert_eq!(frame.active_match_range, Some(6..9));

        let row = &terminal.screen().rows()[0];
        assert!(row[14].attributes().inverse);
        assert!(row[14].attributes().underline);
        assert!(row[18].attributes().inverse);
        assert!(!row[18].attributes().underline);
        assert!(!row[10].attributes().inverse);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn highlights_are_clipped_to_the_horizontal_view() {
        let path = temp_path("termfold-viewer-highlight-clipping");
        fs::write(&path, b"0123456789hit\n").unwrap();
        let size = Size {
            columns: 4,
            rows: 2,
        };
        let mut terminal = Terminal::new(size).unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();

        viewer.render(&mut terminal, size).unwrap();
        assert!(viewer.search("hit", true).unwrap());
        viewer.horizontal = 10;
        viewer.render(&mut terminal, size).unwrap();

        let row = &terminal.screen().rows()[0];
        assert!(row[0].attributes().inverse);
        assert!(row[0].attributes().underline);
        assert!(row[2].attributes().inverse);
        assert!(row[2].attributes().underline);
        assert!(!row[3].attributes().inverse);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn wrapped_search_sets_the_wrapped_result_flag() {
        let path = temp_path("termfold-viewer-highlight-wrap");
        fs::write(&path, b"hit---middle---hit").unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();
        viewer.position = 15;

        assert!(viewer.search("hit", true).unwrap());
        assert!(!viewer.search_wrapped());
        assert!(viewer.repeat_search(true).unwrap());
        assert!(viewer.search_wrapped());

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
    fn final_viewer_row_does_not_scroll_the_terminal() {
        fn screen_line(terminal: &Terminal, row: usize) -> String {
            terminal.screen().rows()[row]
                .iter()
                .filter(|cell| !cell.is_continuation())
                .map(crate::terminal::Cell::character)
                .collect()
        }

        let hex_data: Vec<u8> = (0..48).collect();
        let cases = [
            (
                b"one\ntwo\nthree\n".as_slice(),
                ViewerMode::Text,
                8,
                3,
                "one",
                "three",
            ),
            (
                b"1234\n5678".as_slice(),
                ViewerMode::Text,
                4,
                2,
                "1234",
                "5678",
            ),
            (
                hex_data.as_slice(),
                ViewerMode::Hex,
                80,
                3,
                "00000000",
                "00000020",
            ),
            (b"short".as_slice(), ViewerMode::Text, 8, 3, "short", ""),
            (b"".as_slice(), ViewerMode::Text, 8, 3, "", ""),
        ];
        for (data, mode, columns, rows, first, last) in cases {
            let path = temp_path("termfold-viewer-final-row");
            fs::write(&path, data).unwrap();
            let size = Size { columns, rows };
            let mut terminal = Terminal::new(size).unwrap();
            let mut viewer = Viewer::open(path.clone(), 8).unwrap();
            viewer.mode = mode;

            viewer.render(&mut terminal, size).unwrap();

            assert!(screen_line(&terminal, 0).starts_with(first));
            assert!(screen_line(&terminal, rows as usize - 1).starts_with(last));
            fs::remove_file(path).unwrap();
        }

        let path = temp_path("termfold-viewer-narrow-hex");
        fs::write(&path, b"0123456789").unwrap();
        let size = Size {
            columns: 27,
            rows: 3,
        };
        let mut terminal = Terminal::new(size).unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();
        viewer.mode = ViewerMode::Hex;
        viewer.render(&mut terminal, size).unwrap();
        assert_eq!(terminal.screen().cursor().row, 0);
        fs::remove_file(path).unwrap();

        let path = temp_path("termfold-viewer-resize-final-row");
        fs::write(&path, b"one\ntwo\nthree\n").unwrap();
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
        let size = Size {
            columns: 8,
            rows: 2,
        };
        viewer.render(&mut terminal, size).unwrap();
        assert!(screen_line(&terminal, 0).starts_with("one"));
        assert!(screen_line(&terminal, 1).starts_with("two"));
        fs::remove_file(path).unwrap();
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
    fn horizontal_movement_uses_text_tokens_and_hex_bytes() {
        let path = temp_path("termfold-viewer-horizontal");
        fs::write(&path, "a\t界e\u{301}x\nlast\n01234567890123456789").unwrap();
        let mut viewer = Viewer::open(path.clone(), 4).unwrap();

        viewer.move_horizontal(1).unwrap();
        assert_eq!((viewer.position, viewer.preferred_column), (1, 1));
        viewer.move_horizontal(1).unwrap();
        assert_eq!((viewer.position, viewer.preferred_column), (2, 4));
        viewer.move_horizontal(1).unwrap();
        assert_eq!((viewer.position, viewer.preferred_column), (5, 6));
        viewer.move_horizontal(1).unwrap();
        assert_eq!((viewer.position, viewer.preferred_column), (8, 7));
        viewer.move_horizontal(1).unwrap();
        assert_eq!((viewer.position, viewer.preferred_column), (8, 7));
        viewer.move_horizontal(1).unwrap();
        assert_eq!((viewer.position, viewer.preferred_column), (8, 7));
        viewer.move_horizontal(-10).unwrap();
        assert_eq!((viewer.position, viewer.preferred_column), (0, 0));

        viewer.mode = ViewerMode::Hex;
        viewer.visible_columns = 80;
        viewer.position = 15;
        viewer.move_horizontal(1).unwrap();
        assert_eq!((viewer.position, viewer.preferred_column), (16, 0));
        viewer.move_horizontal(-100).unwrap();
        assert_eq!((viewer.position, viewer.preferred_column), (0, 0));
        viewer.move_horizontal(100).unwrap();
        assert_eq!((viewer.position, viewer.preferred_column), (34, 2));

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
    fn invalid_query_keeps_the_last_successful_query() {
        let path = temp_path("termfold-viewer-invalid-query");
        fs::write(&path, b"hit\n").unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();
        assert!(viewer.search("hit", true).unwrap());
        let previous = viewer.search.clone();

        assert!(matches!(
            viewer.begin_search_work(b"hex:GG".to_vec(), true),
            SearchStart::Complete(false)
        ));
        assert_eq!(viewer.search, previous);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn search_prioritizes_current_and_neighbour_frames() {
        let path = temp_path("termfold-viewer-search-priority");
        fs::write(&path, vec![b'x'; 400]).unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();
        let size = Size {
            columns: 80,
            rows: 24,
        };
        let context = viewer.frame_context(size);
        viewer.frames.set_context(context);
        viewer.frames.commit(
            PageFrame {
                key: FrameKey::new(context, 100),
                source_range: 100..200,
                ..PageFrame::default()
            },
            None,
        );
        viewer.frames.insert_neighbour(
            PageFrame {
                key: FrameKey::new(context, 200),
                source_range: 200..300,
                ..PageFrame::default()
            },
            true,
        );

        let SearchStart::Work(work) = viewer.begin_search_work(b"xxx".to_vec(), true) else {
            panic!("search should be incremental");
        };
        assert_eq!(
            work.ranges.iter().take(2).cloned().collect::<Vec<_>>(),
            vec![100..198, 200..298]
        );

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn search_wraps_once_and_reverses_without_repeating_the_cursor() {
        let path = temp_path("termfold-viewer-search-wrap");
        fs::write(&path, b"hit---middle---hit").unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();
        let first = 0;
        let second = 15;

        viewer.position = second;
        assert!(viewer.search("hit", true).unwrap());
        assert_eq!(viewer.position, second);
        assert!(viewer.repeat_search(true).unwrap());
        assert_eq!(viewer.position, first);
        assert!(viewer.repeat_search(true).unwrap());
        assert_eq!(viewer.position, second);

        viewer.position = first;
        assert!(viewer.search("hit", false).unwrap());
        assert_eq!(viewer.position, first);
        assert!(viewer.repeat_search(true).unwrap());
        assert_eq!(viewer.position, second);
        assert!(viewer.repeat_search(true).unwrap());
        assert_eq!(viewer.position, first);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn incremental_search_finds_a_match_across_a_raw_block() {
        let path = temp_path("termfold-viewer-search-block-boundary");
        let mut data = vec![b'x'; BLOCK_SIZE as usize - 1];
        data.extend_from_slice(b"abc");
        fs::write(&path, &data).unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();

        assert!(viewer.search("abc", true).unwrap());
        assert_eq!(viewer.position, BLOCK_SIZE - 1);
        viewer.position = viewer.length();
        assert!(viewer.search("abc", false).unwrap());
        assert_eq!(viewer.position, BLOCK_SIZE - 1);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn incremental_search_terminates_without_a_match() {
        let path = temp_path("termfold-viewer-search-no-match");
        fs::write(&path, vec![b'x'; BLOCK_SIZE as usize * 2]).unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();
        let SearchStart::Work(mut work) = viewer.begin_search_work(b"z".to_vec(), true) else {
            panic!("search should be incremental");
        };
        let mut steps = 0;
        let result = loop {
            steps += 1;
            match viewer.step_search_work(&mut work).unwrap() {
                SearchStep::Continue => continue,
                SearchStep::Complete(found) => break found,
            }
        };
        assert!(!result);
        assert!(steps >= 3);

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
    fn page_render_commits_current_without_prefetching_a_neighbour() {
        let path = temp_path("termfold-viewer-no-prefetch");
        fs::write(&path, b"one\ntwo\nthree\nfour\n").unwrap();
        let size = Size {
            columns: 16,
            rows: 1,
        };
        let mut terminal = Terminal::new(size).unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();

        viewer.render(&mut terminal, size).unwrap();
        assert_eq!(viewer.frames.count(), 1);
        viewer.page(size.rows, true).unwrap();
        viewer.render(&mut terminal, size).unwrap();
        assert_eq!(viewer.frames.count(), 2);

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
        assert!(
            cold_reads <= 2,
            "cold paging reads including prefetch: {cold_reads}"
        );
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

    #[test]
    fn acceptance_stress_keeps_frames_cache_and_search_steps_bounded() {
        let path = temp_path("termfold-viewer-acceptance-bounds");
        let mut data = Vec::new();
        for line in 0..7_000 {
            data.extend_from_slice(format!("line {line:04}\n").as_bytes());
        }
        fs::write(&path, data).unwrap();

        let size = Size {
            columns: 32,
            rows: 8,
        };
        let mut terminal = Terminal::new(size).unwrap();
        let mut viewer = Viewer::open(path.clone(), 8).unwrap();
        viewer.render(&mut terminal, size).unwrap();

        for _ in 0..100 {
            viewer.page(size.rows, true).unwrap();
            viewer.render(&mut terminal, size).unwrap();
            assert!(viewer.frames.count() <= 3);
            assert!(
                viewer
                    .current_frame()
                    .is_none_or(|frame| frame.source_bytes() <= frame::MAX_FRAME_SOURCE_BYTES)
            );
            assert!(viewer.source.cache_block_count() <= BLOCK_CACHE_SIZE);
            assert!(cache_bytes(&viewer) <= BLOCK_SIZE as usize * BLOCK_CACHE_SIZE);
        }

        viewer.source.reset_metrics();
        assert!(!viewer.search("not-found", true).unwrap());
        assert!(viewer.source.max_range_bytes() <= BLOCK_SIZE as usize);

        for _ in 0..100 {
            viewer.page(size.rows, false).unwrap();
            viewer.render(&mut terminal, size).unwrap();
            assert!(viewer.frames.count() <= 3);
            assert!(
                viewer
                    .current_frame()
                    .is_none_or(|frame| frame.source_bytes() <= frame::MAX_FRAME_SOURCE_BYTES)
            );
            assert!(viewer.source.cache_block_count() <= BLOCK_CACHE_SIZE);
            assert!(cache_bytes(&viewer) <= BLOCK_SIZE as usize * BLOCK_CACHE_SIZE);
        }

        assert!(viewer.source.peak_cache_bytes() <= BLOCK_SIZE as usize * BLOCK_CACHE_SIZE);
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
        viewer.page(size.rows, true).unwrap();

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
