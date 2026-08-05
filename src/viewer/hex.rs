use std::{io, ops::Range};

use super::source::{BLOCK_SIZE, FileSource};

pub(super) const NARROW_MESSAGE: &str = "terminal too narrow for hex view";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HexRow {
    pub(super) offset: u64,
    pub(super) bytes: Vec<u8>,
    pub(super) hex_cells: Vec<Range<usize>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct HexGeometry {
    pub(super) offset_width: usize,
    pub(super) hex_cells: Vec<Range<usize>>,
    pub(super) separator_columns: Vec<usize>,
    pub(super) ascii_start: usize,
}

#[derive(Clone, Debug, Default)]
pub(super) struct HexPage {
    pub(super) bytes_per_row: usize,
    pub(super) geometry: HexGeometry,
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
    select_bytes(columns, 8)
}

fn offset_width(max_offset: u64) -> usize {
    format!("{max_offset:X}").len().max(8)
}

fn row_width(offset_width: usize, bytes_per_row: usize) -> Option<usize> {
    let groups = bytes_per_row / 8 + usize::from(bytes_per_row % 8 != 0);
    let hex_width = bytes_per_row
        .checked_mul(2)?
        .checked_add(bytes_per_row.checked_sub(groups)?)?
        .checked_add(groups.saturating_sub(1).checked_mul(3)?)?;
    offset_width
        .checked_add(4)?
        .checked_add(hex_width)?
        .checked_add(bytes_per_row)
}

fn fits(columns: usize, offset_width: usize, bytes_per_row: usize) -> bool {
    row_width(offset_width, bytes_per_row)
        .and_then(|width| width.checked_add(1))
        .is_some_and(|width| width <= columns)
}

fn select_bytes(columns: usize, offset_width: usize) -> usize {
    let groups = columns.saturating_sub(offset_width.saturating_add(2)) / 34;
    let eight_byte_width = groups.saturating_mul(8);
    if eight_byte_width >= 8 && fits(columns, offset_width, eight_byte_width) {
        return eight_byte_width;
    }
    if fits(columns, offset_width, 4) { 4 } else { 0 }
}

fn geometry(columns: usize, max_offset: u64) -> Option<(usize, HexGeometry)> {
    let offset_width = offset_width(max_offset);
    let bytes_per_row = select_bytes(columns, offset_width);
    if bytes_per_row == 0 {
        return None;
    }
    Some((bytes_per_row, HexGeometry::new(offset_width, bytes_per_row)))
}

pub(super) fn build(
    source: &mut FileSource,
    start: u64,
    rows: usize,
    columns: usize,
    max_source_bytes: u64,
) -> io::Result<HexPage> {
    let Some((bytes_per_row, geometry)) = geometry(columns, source.len().saturating_sub(1)) else {
        return Ok(HexPage {
            source_range: start..start,
            narrow: true,
            ..HexPage::default()
        });
    };

    let requested = u64::try_from(rows)
        .unwrap_or(u64::MAX)
        .saturating_mul(bytes_per_row as u64)
        .min(max_source_bytes);
    let end = start.saturating_add(requested).min(source.len());
    let mut page = HexPage {
        bytes_per_row,
        geometry,
        source_range: start..start,
        ..HexPage::default()
    };
    let mut position = start;
    let mut next_row_offset = start;
    let mut pending = Vec::new();
    while position < end {
        let chunk_end = position.saturating_add(BLOCK_SIZE).min(end);
        let bytes = source.read_range(position..chunk_end)?;
        let mut remainder = bytes.as_slice();
        if !pending.is_empty() {
            let needed = bytes_per_row - pending.len();
            if remainder.len() < needed {
                pending.extend_from_slice(remainder);
                position = chunk_end;
                continue;
            }
            pending.extend_from_slice(&remainder[..needed]);
            page.rows
                .push(HexRow::new(next_row_offset, &pending, &page.geometry));
            next_row_offset = next_row_offset.saturating_add(bytes_per_row as u64);
            pending.clear();
            remainder = &remainder[needed..];
        }
        let complete_len = remainder.len() / bytes_per_row * bytes_per_row;
        for row in remainder[..complete_len].chunks_exact(bytes_per_row) {
            page.rows
                .push(HexRow::new(next_row_offset, row, &page.geometry));
            next_row_offset = next_row_offset.saturating_add(bytes_per_row as u64);
        }
        if complete_len < remainder.len() {
            pending.extend_from_slice(&remainder[complete_len..]);
        }
        position = chunk_end;
    }
    if !pending.is_empty() {
        page.rows
            .push(HexRow::new(next_row_offset, &pending, &page.geometry));
    }
    page.source_range.end = end;
    Ok(page)
}

impl HexRow {
    fn new(offset: u64, bytes: &[u8], geometry: &HexGeometry) -> Self {
        let hex_cells = geometry.hex_cells[..bytes.len()].to_vec();
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
        let mut rendered = vec![' '; self.geometry.ascii_start + row.bytes.len()];
        for (index, character) in format!(
            "{:0width$X}",
            row.offset,
            width = self.geometry.offset_width
        )
        .chars()
        .enumerate()
        {
            rendered[index] = character;
        }
        for &column in &self.geometry.separator_columns {
            rendered[column] = '│';
        }
        for (index, byte) in row.bytes.iter().enumerate() {
            let cell = &row.hex_cells[index];
            for (offset, character) in format!("{byte:02X}").chars().enumerate() {
                rendered[cell.start + offset] = character;
            }
        }
        for (index, byte) in row.bytes.iter().enumerate() {
            rendered[self.geometry.ascii_start + index] = if (0x20..=0x7e).contains(byte) {
                *byte as char
            } else {
                '.'
            };
        }
        rendered.into_iter().collect()
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
                    column: self.geometry.ascii_start + byte_index,
                    width: 1,
                    active,
                });
            }
        }
    }
}

impl HexGeometry {
    fn new(offset_width: usize, bytes_per_row: usize) -> Self {
        let mut column = offset_width + 2;
        let mut hex_cells = Vec::with_capacity(bytes_per_row);
        let mut separator_columns = Vec::with_capacity(bytes_per_row / 8);
        for index in 0..bytes_per_row {
            if index > 0 {
                if index % 8 == 0 {
                    column += 1;
                    separator_columns.push(column);
                    column += 2;
                } else {
                    column += 1;
                }
            }
            hex_cells.push(column..column + 2);
            column += 2;
        }
        Self {
            offset_width,
            hex_cells,
            separator_columns,
            ascii_start: column + 2,
        }
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
    fn width_geometry_selects_greatest_fitting_layout() {
        assert_eq!(bytes_per_row(146), 32);
        assert_eq!(bytes_per_row(112), 24);
        assert_eq!(bytes_per_row(80), 16);
        assert_eq!(bytes_per_row(78), 16);
        assert_eq!(bytes_per_row(77), 8);
        assert_eq!(bytes_per_row(48), 8);
        assert_eq!(bytes_per_row(44), 8);
        assert_eq!(bytes_per_row(28), 4);
        assert_eq!(bytes_per_row(27), 0);
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
        let geometry = HexGeometry::new(9, 4);
        let row = HexRow::new(start, &[0xab], &geometry);
        let page = HexPage {
            bytes_per_row: 4,
            geometry,
            rows: vec![row],
            ..HexPage::default()
        };
        assert!(page.render_row(0).starts_with("100000000  "));
    }

    #[test]
    fn derives_offset_width_from_the_snapshot_length() {
        let path = temp_path("termfold-hex-offset-width");
        let file = fs::File::create(&path).unwrap();
        file.set_len(0x1_0000_0001).unwrap();
        let mut source = FileSource::open(path.clone()).unwrap();
        let page = build(&mut source, 0, 1, 80, 256 * 1024).unwrap();
        assert_eq!(page.geometry.offset_width, 9);
        assert!(page.render_row(0).starts_with("000000000  "));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn separates_complete_eight_byte_groups_and_keeps_ascii_continuous() {
        let path = temp_path("termfold-hex-groups");
        fs::write(&path, (0..16).collect::<Vec<_>>()).unwrap();
        let mut source = FileSource::open(path.clone()).unwrap();
        let page = build(&mut source, 0, 1, 80, 256 * 1024).unwrap();
        assert_eq!(page.geometry.separator_columns, vec![34]);
        assert_eq!(page.geometry.ascii_start, 61);
        assert_eq!(
            page.render_row(0),
            "00000000  00 01 02 03 04 05 06 07 │ 08 09 0A 0B 0C 0D 0E 0F  ................"
        );
        assert!(
            !page
                .render_row(0)
                .chars()
                .skip(page.geometry.ascii_start)
                .any(|c| c == '│')
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn short_final_row_uses_the_same_separator_and_ascii_columns() {
        let path = temp_path("termfold-hex-short-row");
        fs::write(&path, (0..17).collect::<Vec<_>>()).unwrap();
        let mut source = FileSource::open(path.clone()).unwrap();
        let page = build(&mut source, 0, 2, 80, 256 * 1024).unwrap();
        let first = page.render_row(0);
        let second = page.render_row(1);
        assert_eq!(first.find('│'), second.find('│'));
        assert_eq!(first.chars().nth(page.geometry.ascii_start), Some('.'));
        assert_eq!(second.chars().nth(page.geometry.ascii_start), Some('.'));
        assert_eq!(second.chars().count(), page.geometry.ascii_start + 1);
        fs::remove_file(path).unwrap();
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

        let page = build(
            &mut source,
            0,
            BLOCK_SIZE as usize / 24 + 2,
            112,
            256 * 1024,
        )
        .unwrap();
        let boundary_row = BLOCK_SIZE as usize / 24;
        assert_eq!(page.bytes_per_row, 24);
        assert_eq!(page.rows[boundary_row].offset, boundary_row as u64 * 24);
        assert_eq!(
            page.rows[boundary_row + 1].offset,
            (boundary_row as u64 + 1) * 24
        );

        let page = build(&mut source, 0, usize::MAX, 80, 256 * 1024).unwrap();
        assert_eq!(page.source_range.end - page.source_range.start, 256 * 1024);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn highlights_hex_and_ascii_cells_for_each_matching_byte() {
        let geometry = HexGeometry::new(8, 4);
        let row = HexRow::new(0, b"AbCD", &geometry);
        let page = HexPage {
            bytes_per_row: 4,
            geometry,
            rows: vec![row],
            ..HexPage::default()
        };
        let mut spans = Vec::new();
        let matches = std::iter::once(1..3).collect::<Vec<_>>();
        page.for_each_highlight(&matches, Some(&(2..3)), |span| spans.push(span));

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
