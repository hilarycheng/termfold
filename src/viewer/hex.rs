use std::{io, ops::Range};

use super::source::{BLOCK_SIZE, FileSource};

pub(super) const NARROW_MESSAGE: &str = "terminal too narrow for hex view";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HexRow {
    pub(super) offset: u64,
    pub(super) bytes: Vec<u8>,
    pub(super) hex_cells: Vec<Range<usize>>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct HexPage {
    pub(super) bytes_per_row: usize,
    pub(super) rows: Vec<HexRow>,
    pub(super) source_range: Range<u64>,
    pub(super) narrow: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct HighlightSpan {
    pub(super) source: u64,
    pub(super) row: usize,
    pub(super) column: usize,
    pub(super) width: usize,
    pub(super) active: bool,
}

pub(super) fn bytes_per_row(columns: usize) -> usize {
    match columns {
        80.. => 16,
        48..=79 => 8,
        28..=47 => 4,
        _ => 0,
    }
}

pub(super) fn build(
    source: &mut FileSource,
    start: u64,
    rows: usize,
    columns: usize,
    max_source_bytes: u64,
) -> io::Result<HexPage> {
    let bytes_per_row = bytes_per_row(columns);
    if bytes_per_row == 0 {
        return Ok(HexPage {
            source_range: start..start,
            narrow: true,
            ..HexPage::default()
        });
    }

    let requested = u64::try_from(rows)
        .unwrap_or(u64::MAX)
        .saturating_mul(bytes_per_row as u64)
        .min(max_source_bytes);
    let end = start.saturating_add(requested).min(source.len());
    let mut page = HexPage {
        bytes_per_row,
        source_range: start..start,
        ..HexPage::default()
    };
    let mut position = start;
    while position < end {
        let chunk_end = position.saturating_add(BLOCK_SIZE).min(end);
        let bytes = source.read_range(position..chunk_end)?;
        for (index, row) in bytes.chunks(bytes_per_row).enumerate() {
            let offset = position.saturating_add(index as u64 * bytes_per_row as u64);
            page.rows.push(HexRow::new(offset, row));
        }
        position = chunk_end;
    }
    page.source_range.end = end;
    Ok(page)
}

impl HexRow {
    fn new(offset: u64, bytes: &[u8]) -> Self {
        let hex_start = format!("{offset:08X}").len() + 2;
        let hex_cells = (0..bytes.len())
            .map(|index| {
                let start = hex_start + index * 3;
                start..start + 2
            })
            .collect();
        Self {
            offset,
            bytes: bytes.to_vec(),
            hex_cells,
        }
    }
}

impl HexPage {
    pub(super) fn render_row(&self, row: usize) -> String {
        if self.narrow {
            return NARROW_MESSAGE.into();
        }
        let Some(row) = self.rows.get(row) else {
            return String::new();
        };
        let mut rendered = format!("{:08X}  ", row.offset);
        for index in 0..self.bytes_per_row {
            if index > 0 {
                rendered.push(' ');
            }
            match row.bytes.get(index) {
                Some(byte) => rendered.push_str(&format!("{byte:02X}")),
                None => rendered.push_str("  "),
            }
        }
        rendered.push_str("  ");
        rendered.extend(row.bytes.iter().map(|byte| {
            if (0x20..=0x7e).contains(byte) {
                *byte as char
            } else {
                '.'
            }
        }));
        rendered
    }

    pub(super) fn for_each_highlight(
        &self,
        matches: &[Range<u64>],
        active: Option<&Range<u64>>,
        mut visit: impl FnMut(HighlightSpan),
    ) {
        if self.narrow || matches.is_empty() {
            return;
        }
        let mut match_index = 0;
        for (row_index, row) in self.rows.iter().enumerate() {
            for (byte_index, _) in row.bytes.iter().enumerate() {
                let source = row.offset + byte_index as u64;
                while matches
                    .get(match_index)
                    .is_some_and(|range| range.end <= source)
                {
                    match_index += 1;
                }
                let Some(range) = matches.get(match_index) else {
                    return;
                };
                if range.start > source {
                    continue;
                }
                let active = active.is_some_and(|range| range.contains(&source));
                visit(HighlightSpan {
                    source,
                    row: row_index,
                    column: row.hex_cells[byte_index].start,
                    width: 2,
                    active,
                });
                visit(HighlightSpan {
                    source,
                    row: row_index,
                    column: self.ascii_column(row) + byte_index,
                    width: 1,
                    active,
                });
            }
        }
    }

    fn ascii_column(&self, row: &HexRow) -> usize {
        row.hex_cells
            .first()
            .map_or_else(
                || format!("{:08X}", row.offset).len() + 2,
                |cell| cell.start,
            )
            .saturating_add(self.bytes_per_row * 3 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf, time::SystemTime};

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

    #[test]
    fn width_thresholds_select_16_8_4_or_nothing() {
        assert_eq!(bytes_per_row(80), 16);
        assert_eq!(bytes_per_row(48), 8);
        assert_eq!(bytes_per_row(28), 4);
        assert_eq!(bytes_per_row(27), 0);
        assert_eq!(bytes_per_row(79), 8);
        assert_eq!(bytes_per_row(47), 4);
    }

    #[test]
    fn renders_offset_hex_and_printable_ascii() {
        let path = temp_path("termfold-hex-render");
        fs::write(&path, [b'A', b' ', 0, 0x7e]).unwrap();
        let mut source = FileSource::open(path.clone()).unwrap();
        let page = build(&mut source, 0, 1, 28, 256 * 1024).unwrap();
        assert_eq!(page.render_row(0), "00000000  41 20 00 7E  A .~");
        assert_eq!(page.rows[0].hex_cells, vec![10..12, 13..15, 16..18, 19..21]);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn keeps_absolute_offsets_above_four_gib() {
        let start = 0x1_0000_0000;
        let row = HexRow::new(start, &[0xab]);
        let page = HexPage {
            bytes_per_row: 4,
            rows: vec![row],
            ..HexPage::default()
        };
        assert!(page.render_row(0).starts_with("100000000  "));
    }

    #[test]
    fn narrow_view_keeps_position_range_without_reading() {
        let path = temp_path("termfold-hex-narrow");
        fs::write(&path, b"data").unwrap();
        let mut source = FileSource::open(path.clone()).unwrap();
        let page = build(&mut source, 2, 4, 27, 256 * 1024).unwrap();
        assert!(page.narrow);
        assert_eq!(page.source_range, 2..2);
        assert_eq!(page.render_row(0), NARROW_MESSAGE);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn frame_reads_cross_block_and_stays_bounded() {
        let path = temp_path("termfold-hex-bound");
        let mut data = vec![0; BLOCK_SIZE as usize * 5];
        data[BLOCK_SIZE as usize - 1] = 0xaa;
        data[BLOCK_SIZE as usize] = 0xbb;
        fs::write(&path, &data).unwrap();
        let mut source = FileSource::open(path.clone()).unwrap();
        let page = build(&mut source, BLOCK_SIZE - 8, 3, 28, 256 * 1024).unwrap();
        assert_eq!(page.rows[0].offset, BLOCK_SIZE - 8);
        assert_eq!(page.rows[1].bytes[3], 0xaa);
        assert_eq!(page.rows[2].bytes[0], 0xbb);

        let page = build(&mut source, 0, usize::MAX, 80, 256 * 1024).unwrap();
        assert_eq!(page.source_range.end - page.source_range.start, 256 * 1024);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn highlights_hex_and_ascii_cells_for_each_matching_byte() {
        let row = HexRow::new(0, b"AbCD");
        let page = HexPage {
            bytes_per_row: 4,
            rows: vec![row],
            ..HexPage::default()
        };
        let mut spans = Vec::new();
        page.for_each_highlight(&[1..3], Some(&(2..3)), |span| spans.push(span));

        assert_eq!(
            spans,
            vec![
                HighlightSpan {
                    source: 1,
                    row: 0,
                    column: 13,
                    width: 2,
                    active: false,
                },
                HighlightSpan {
                    source: 1,
                    row: 0,
                    column: 24,
                    width: 1,
                    active: false,
                },
                HighlightSpan {
                    source: 2,
                    row: 0,
                    column: 16,
                    width: 2,
                    active: true,
                },
                HighlightSpan {
                    source: 2,
                    row: 0,
                    column: 25,
                    width: 1,
                    active: true,
                },
            ]
        );
    }
}
