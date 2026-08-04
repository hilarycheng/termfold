use std::{
    cmp::{max, min},
    collections::VecDeque,
    io,
    ops::Range,
};

use super::{
    line::{LineBoundary, LineScanner, ScanStep},
    source::FileSource,
    text::{decode, DecodedText},
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

#[derive(Clone, Debug, Default)]
pub(super) struct PageFrame {
    pub(super) source_range: Range<u64>,
    pub(super) rows: Vec<DecodedText>,
    pub(super) line_boundaries: Vec<LineBoundary>,
    pub(super) source_cell_spans: Vec<SourceCellSpan>,
    pub(super) cursor_stops: Vec<CursorStop>,
    pub(super) visible_match_ranges: Vec<Range<u64>>,
}

impl PageFrame {
    pub(super) fn render_row(&self, row: usize, horizontal: u64, columns: usize) -> String {
        let Some(decoded) = self.rows.get(row) else {
            return String::new();
        };
        let first = min(horizontal, decoded.width as u64) as usize;
        decoded.render_cells(first..first.saturating_add(columns))
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

pub(super) fn build(
    source: &mut FileSource,
    lines: &mut VecDeque<LineBoundary>,
    length: u64,
    tab_width: usize,
    viewport: u64,
    rows: usize,
    matches: &[u64],
    match_length: Option<usize>,
) -> io::Result<PageFrame> {
    let viewport = line_boundary(source, lines, length, viewport)?.start;
    let mut frame = PageFrame {
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

    if let Some(match_length) = match_length {
        for &offset in matches {
            let end = offset.saturating_add(match_length as u64);
            let start = max(offset, frame.source_range.start);
            let end = min(end, frame.source_range.end);
            if start < end {
                frame.visible_match_ranges.push(start..end);
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
