use std::{
    cmp::{max, min},
    collections::VecDeque,
    io,
    ops::Range,
};

use crate::session::Size;

use super::{
    ViewerMode,
    hex::{self, HexPage},
    line::{LineBoundary, LineScanner, ScanStep},
    search::SearchQuery,
    source::FileSource,
    text::{DecodedText, decode},
};

pub(super) const MAX_FRAME_SOURCE_BYTES: u64 = 256 * 1024;
const LINE_CACHE_SIZE: usize = 64;
const MAX_LINE_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SourceCellSpan {
    pub(super) row: usize,
    pub(super) token: usize,
    pub(super) source: Range<u64>,
    pub(super) cells: Range<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CursorStop {
    pub(super) row: usize,
    pub(super) source: u64,
    pub(super) cell: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct FrameContext {
    pub(super) snapshot_length: u64,
    pub(super) mode: u8,
    pub(super) size: Size,
    pub(super) tab_width: usize,
    pub(super) generation: u64,
}

impl FrameContext {
    pub(super) fn new(
        snapshot_length: u64,
        mode: u8,
        size: Size,
        tab_width: usize,
        generation: u64,
    ) -> Self {
        Self {
            snapshot_length,
            mode,
            size,
            tab_width,
            generation,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct FrameKey {
    pub(super) context: FrameContext,
    pub(super) source_start: u64,
}

impl FrameKey {
    pub(super) fn new(context: FrameContext, source_start: u64) -> Self {
        Self {
            context,
            source_start,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct PageFrame {
    pub(super) key: FrameKey,
    pub(super) source_range: Range<u64>,
    pub(super) rows: Vec<DecodedText>,
    pub(super) line_boundaries: Vec<LineBoundary>,
    pub(super) source_cell_spans: Vec<SourceCellSpan>,
    pub(super) cursor_stops: Vec<CursorStop>,
    pub(super) visible_match_ranges: Vec<Range<u64>>,
    pub(super) active_match_range: Option<Range<u64>>,
    pub(super) hex: Option<HexPage>,
}

impl PageFrame {
    pub(super) fn render_row(&self, row: usize, horizontal: u64, columns: usize) -> String {
        if self.key.context.mode == ViewerMode::Hex as u8 {
            return self
                .hex
                .as_ref()
                .map_or_else(String::new, |page| page.render_row(row));
        }
        let Some(decoded) = self.rows.get(row) else {
            return String::new();
        };
        let first = min(horizontal, decoded.width as u64) as usize;
        let last = first.saturating_add(columns);
        if first >= last {
            return String::new();
        }

        let styles = self.row_match_styles(row, decoded.tokens.len());
        let mut segments = Vec::new();
        let mut position = first;
        for (token, style) in decoded
            .tokens
            .iter()
            .zip(styles)
            .filter(|(token, _)| token.cells.end > first && token.cells.start < last)
        {
            let start = token.cells.start.max(first);
            let end = token.cells.end.min(last);
            if position < start {
                push_segment(&mut segments, position..start, MatchStyle::None);
            }
            push_segment(&mut segments, start..end, style);
            position = end;
        }
        if position < last {
            push_segment(&mut segments, position..last, MatchStyle::None);
        }

        let mut rendered = String::new();
        for (cells, style) in segments {
            let text = decoded.render_cells(cells);
            if text.is_empty() {
                continue;
            }
            match style {
                MatchStyle::None => rendered.push_str(&text),
                MatchStyle::Ordinary => {
                    rendered.push_str("\x1b[7m");
                    rendered.push_str(&text);
                    rendered.push_str("\x1b[0m");
                }
                MatchStyle::Active => {
                    rendered.push_str("\x1b[7;4m");
                    rendered.push_str(&text);
                    rendered.push_str("\x1b[0m");
                }
            }
        }
        rendered
    }

    fn row_match_styles(&self, row: usize, token_count: usize) -> Vec<MatchStyle> {
        let mut styles = vec![MatchStyle::None; token_count];
        let mut match_index = 0;
        for span in self.source_cell_spans.iter().filter(|span| span.row == row) {
            while self
                .visible_match_ranges
                .get(match_index)
                .is_some_and(|range| range.end <= span.source.start)
            {
                match_index += 1;
            }
            let ordinary = self
                .visible_match_ranges
                .get(match_index..)
                .unwrap_or_default()
                .iter()
                .any(|range| range.start < span.source.end && span.source.start < range.end);
            let active = self.active_match_range.as_ref().is_some_and(|range| {
                range.start < span.source.end && span.source.start < range.end
            });
            if active {
                styles[span.token] = MatchStyle::Active;
            } else if ordinary {
                styles[span.token] = MatchStyle::Ordinary;
            }
        }
        styles
    }

    pub(super) fn cursor_row(&self, line: u64) -> Option<usize> {
        self.line_boundaries
            .iter()
            .position(|boundary| boundary.start == line)
    }

    pub(super) fn cursor_column(
        &self,
        line: u64,
        position: u64,
        horizontal: u64,
        columns: usize,
    ) -> Option<usize> {
        let row = self.cursor_row(line)?;
        let boundary = self.line_boundaries.get(row)?;
        let decoded = self.rows.get(row)?;
        let source = position
            .saturating_sub(boundary.start)
            .min(boundary.content_end.saturating_sub(boundary.start)) as usize;
        let cursor = decoded.cursor_cell_at_source(source).unwrap_or(0);
        Some(
            cursor
                .saturating_sub(min(horizontal, decoded.width as u64) as usize)
                .min(columns.saturating_sub(1)),
        )
    }

    pub(super) fn source_bytes(&self) -> u64 {
        self.source_range
            .end
            .saturating_sub(self.source_range.start)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MatchStyle {
    None,
    Ordinary,
    Active,
}

fn push_segment(
    segments: &mut Vec<(Range<usize>, MatchStyle)>,
    range: Range<usize>,
    style: MatchStyle,
) {
    if range.start >= range.end {
        return;
    }
    if let Some((previous, previous_style)) = segments.last_mut()
        && *previous_style == style
        && previous.end == range.start
    {
        previous.end = range.end;
        return;
    }
    segments.push((range, style));
}

#[derive(Clone, Debug, Default)]
pub(super) struct FrameSlots {
    context: Option<FrameContext>,
    previous: Option<PageFrame>,
    current: Option<PageFrame>,
    next: Option<PageFrame>,
}

impl FrameSlots {
    pub(super) fn context(&self) -> Option<FrameContext> {
        self.context
    }

    pub(super) fn current(&self) -> Option<&PageFrame> {
        self.current.as_ref()
    }

    pub(super) fn neighbour(&self, forward: bool) -> Option<&PageFrame> {
        if forward {
            self.next.as_ref()
        } else {
            self.previous.as_ref()
        }
    }

    #[cfg(test)]
    pub(super) fn count(&self) -> usize {
        usize::from(self.previous.is_some())
            + usize::from(self.current.is_some())
            + usize::from(self.next.is_some())
    }

    pub(super) fn clear(&mut self) {
        self.previous = None;
        self.current = None;
        self.next = None;
    }

    pub(super) fn set_context(&mut self, context: FrameContext) {
        if self.context != Some(context) {
            self.clear();
            self.context = Some(context);
        }
    }

    pub(super) fn current_matches(&self, key: FrameKey) -> bool {
        self.current.as_ref().is_some_and(|frame| frame.key == key)
    }

    pub(super) fn neighbour_matches(&self, key: FrameKey, forward: bool) -> bool {
        let neighbour = if forward {
            self.next.as_ref()
        } else {
            self.previous.as_ref()
        };
        neighbour.is_some_and(|frame| frame.key == key)
    }

    pub(super) fn rotate_forward(&mut self, key: FrameKey) -> bool {
        let Some(current) = self.current.take() else {
            return false;
        };
        let Some(next) = self.next.take() else {
            self.current = Some(current);
            return false;
        };
        if self.context != Some(key.context)
            || next.key != key
            || current.key.context != key.context
            || current.key.source_start >= next.key.source_start
        {
            self.current = Some(current);
            self.next = Some(next);
            return false;
        }
        self.previous = Some(current);
        self.current = Some(next);
        true
    }

    pub(super) fn rotate_backward(&mut self, key: FrameKey) -> bool {
        let Some(current) = self.current.take() else {
            return false;
        };
        let Some(previous) = self.previous.take() else {
            self.current = Some(current);
            return false;
        };
        if self.context != Some(key.context)
            || previous.key != key
            || current.key.context != key.context
            || previous.key.source_start >= current.key.source_start
        {
            self.current = Some(current);
            self.previous = Some(previous);
            return false;
        }
        self.next = Some(current);
        self.current = Some(previous);
        true
    }

    pub(super) fn commit(&mut self, frame: PageFrame, forward: Option<bool>) {
        self.set_context(frame.key.context);
        let old = self.current.take();
        self.previous = None;
        self.next = None;
        if let Some(old) = old
            && old.key.context == frame.key.context
        {
            match forward {
                Some(true) if old.key.source_start < frame.key.source_start => {
                    self.previous = Some(old);
                }
                Some(false) if frame.key.source_start < old.key.source_start => {
                    self.next = Some(old);
                }
                _ => {}
            }
        }
        self.current = Some(frame);
    }

    #[cfg(test)]
    pub(super) fn insert_neighbour(&mut self, frame: PageFrame, forward: bool) -> bool {
        if self.context != Some(frame.key.context)
            || !self
                .current
                .as_ref()
                .is_some_and(|current| current.key.context == frame.key.context)
        {
            return false;
        }
        let current_start = self
            .current
            .as_ref()
            .map_or(0, |current| current.key.source_start);
        if forward {
            if self.next.is_some() || frame.key.source_start <= current_start {
                return false;
            }
            self.next = Some(frame);
        } else {
            if self.previous.is_some() || frame.key.source_start >= current_start {
                return false;
            }
            self.previous = Some(frame);
        }
        true
    }
}

pub(super) fn build(
    source: &mut FileSource,
    lines: &mut VecDeque<LineBoundary>,
    viewport: u64,
    query: Option<&SearchQuery>,
    active_offset: Option<u64>,
    context: FrameContext,
) -> io::Result<PageFrame> {
    let length = context.snapshot_length;
    let tab_width = context.tab_width;
    let rows = usize::from(context.size.rows);
    if context.mode == ViewerMode::Hex as u8 {
        let page = hex::build(
            source,
            viewport.min(length),
            usize::from(context.size.rows),
            usize::from(context.size.columns),
            MAX_FRAME_SOURCE_BYTES,
        )?;
        let mut frame = PageFrame {
            key: FrameKey::new(context, page.source_range.start),
            source_range: page.source_range.clone(),
            hex: Some(page),
            ..PageFrame::default()
        };
        if let Some(page) = frame.hex.as_ref() {
            for (row, row_data) in page.rows.iter().enumerate() {
                for (token, cells) in row_data.hex_cells.iter().enumerate() {
                    let source = row_data.offset + token as u64..row_data.offset + token as u64 + 1;
                    frame.source_cell_spans.push(SourceCellSpan {
                        row,
                        token,
                        source: source.clone(),
                        cells: cells.clone(),
                    });
                    frame.cursor_stops.push(CursorStop {
                        row,
                        source: source.start,
                        cell: cells.start,
                    });
                }
            }
        }
        return Ok(frame);
    }
    let viewport = line_boundary(source, lines, length, viewport)?.start;
    let mut frame = PageFrame {
        key: FrameKey::new(context, viewport),
        source_range: viewport..viewport,
        ..PageFrame::default()
    };
    let mut position = viewport;

    while frame.rows.len() < rows && position < length {
        let boundary = line_boundary(source, lines, length, position)?;
        let source_end = boundary.next.max(boundary.content_end);
        let source_bytes = source_end.saturating_sub(boundary.start);
        if frame.source_bytes().saturating_add(source_bytes) > MAX_FRAME_SOURCE_BYTES {
            break;
        }

        let decoded = decode_line(source, &boundary, tab_width)?;
        let row = frame.rows.len();
        for (token, span) in decoded.tokens.iter().enumerate() {
            let source_start = boundary.start.saturating_add(span.source.start as u64);
            let source_end = boundary.start.saturating_add(span.source.end as u64);
            frame.source_cell_spans.push(SourceCellSpan {
                row,
                token,
                source: source_start..source_end,
                cells: span.cells.clone(),
            });
            if span.cursor_stop {
                frame.cursor_stops.push(CursorStop {
                    row,
                    source: source_start,
                    cell: span.cells.start,
                });
            }
        }
        frame.rows.push(decoded);
        frame.line_boundaries.push(boundary);
        frame.source_range.end = source_end;

        if !boundary.complete || boundary.next <= position {
            break;
        }
        position = boundary.next;
    }

    if let Some(query) = query {
        frame.visible_match_ranges = query.visible_matches(source, frame.source_range.clone())?;
        if let Some(offset) = active_offset {
            let end = offset.saturating_add(query.len() as u64);
            let start = max(offset, frame.source_range.start);
            let end = min(end, frame.source_range.end);
            if start < end {
                frame.active_match_range = Some(start..end);
            }
        }
    }
    Ok(frame)
}

pub(super) fn decode_line(
    source: &mut FileSource,
    line: &LineBoundary,
    tab_width: usize,
) -> io::Result<DecodedText> {
    if line.content_end.saturating_sub(line.start) > MAX_LINE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "viewer line segment is too large",
        ));
    }
    let bytes = source.read_range(line.start..line.content_end)?;
    if bytes.len() != (line.content_end - line.start) as usize {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "viewer source range is shorter than the snapshot",
        ));
    }
    Ok(decode(&bytes, tab_width))
}

pub(super) fn line_boundary(
    source: &mut FileSource,
    lines: &mut VecDeque<LineBoundary>,
    length: u64,
    start: u64,
) -> io::Result<LineBoundary> {
    let start = start.min(length);
    if let Some(line) = cached_line(lines, start) {
        return Ok(line);
    }
    let mut scanner = cached_forward_continuation(lines, start)
        .unwrap_or_else(|| LineScanner::forward(start, length));
    let line = match scanner.step(source)? {
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
    cache_line(lines, line);
    Ok(line)
}

pub(super) fn cache_line(lines: &mut VecDeque<LineBoundary>, line: LineBoundary) {
    let forward = line.resume.is_none_or(|scanner| scanner.is_forward());
    if let Some(index) = lines.iter().position(|cached| {
        cached.start == line.start
            && cached.resume.is_none_or(|scanner| scanner.is_forward()) == forward
    }) {
        lines.remove(index);
    }
    lines.push_front(line);
    lines.truncate(LINE_CACHE_SIZE);
}

fn cached_line(lines: &mut VecDeque<LineBoundary>, start: u64) -> Option<LineBoundary> {
    let index = lines.iter().position(|line| {
        line.start == start && line.resume.is_none_or(|scanner| scanner.is_forward())
    })?;
    let line = lines.remove(index)?;
    lines.push_front(line);
    Some(line)
}

fn cached_forward_continuation(
    lines: &mut VecDeque<LineBoundary>,
    start: u64,
) -> Option<LineScanner> {
    let index = lines.iter().position(|line| {
        !line.complete
            && line.next == start
            && line.resume.is_some_and(|scanner| scanner.is_forward())
    })?;
    let line = lines.remove(index)?;
    lines.push_front(line);
    line.resume
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_context(mode: u8, size: Size) -> FrameContext {
        FrameContext::new(1024, mode, size, 8, 1)
    }

    fn frame(context: FrameContext, start: u64, end: u64) -> PageFrame {
        PageFrame {
            key: FrameKey::new(context, start),
            source_range: start..end,
            ..PageFrame::default()
        }
    }

    #[test]
    fn rotates_forward_and_backward_without_growing_slots() {
        let context = make_context(
            0,
            Size {
                columns: 80,
                rows: 24,
            },
        );
        let mut slots = FrameSlots::default();
        slots.set_context(context);
        slots.commit(frame(context, 0, 20), None);
        slots.insert_neighbour(frame(context, 10, 30), true);

        assert!(slots.rotate_forward(FrameKey::new(context, 10)));
        assert_eq!(
            slots.current().map(|frame| frame.key.source_start),
            Some(10)
        );
        assert_eq!(slots.count(), 2);

        assert!(slots.rotate_backward(FrameKey::new(context, 0)));
        assert_eq!(slots.current().map(|frame| frame.key.source_start), Some(0));
        assert_eq!(slots.count(), 2);
    }

    #[test]
    fn rejects_an_invalid_neighbour_without_losing_current() {
        let context = make_context(
            0,
            Size {
                columns: 80,
                rows: 24,
            },
        );
        let other = make_context(
            1,
            Size {
                columns: 80,
                rows: 24,
            },
        );
        let mut slots = FrameSlots::default();
        slots.set_context(context);
        slots.commit(frame(context, 0, 20), None);
        slots.insert_neighbour(frame(context, 10, 30), true);

        assert!(!slots.rotate_forward(FrameKey::new(other, 10)));
        assert_eq!(slots.current().map(|frame| frame.key.source_start), Some(0));
        assert_eq!(slots.count(), 2);
    }

    #[test]
    fn changing_context_invalidates_all_slots() {
        let size = Size {
            columns: 80,
            rows: 24,
        };
        let context = make_context(0, size);
        let mut slots = FrameSlots::default();
        slots.set_context(context);
        slots.commit(frame(context, 0, 20), None);
        slots.insert_neighbour(frame(context, 10, 30), true);
        assert_eq!(slots.count(), 2);

        slots.set_context(make_context(
            0,
            Size {
                columns: 100,
                rows: 24,
            },
        ));
        assert_eq!(slots.count(), 0);
    }

    #[test]
    fn alternating_direction_keeps_three_slot_bound() {
        let context = make_context(
            0,
            Size {
                columns: 80,
                rows: 24,
            },
        );
        let mut slots = FrameSlots::default();
        slots.set_context(context);
        slots.commit(frame(context, 10, 30), None);
        slots.insert_neighbour(frame(context, 20, 40), true);
        assert!(slots.rotate_forward(FrameKey::new(context, 20)));
        slots.insert_neighbour(frame(context, 30, 50), true);
        assert_eq!(slots.count(), 3);
        assert!(!slots.insert_neighbour(frame(context, 40, 60), true));
        assert_eq!(slots.count(), 3);

        assert!(slots.rotate_backward(FrameKey::new(context, 10)));
        assert_eq!(slots.count(), 2);
        slots.insert_neighbour(frame(context, 0, 20), false);
        assert_eq!(slots.count(), 3);
    }
}
