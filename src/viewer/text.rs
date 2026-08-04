use std::ops::Range;

use unicode_width::UnicodeWidthChar;

#[cfg(test)]
pub(super) const DEFAULT_TAB_WIDTH: usize = 8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TextToken {
    pub(super) source: Range<usize>,
    pub(super) cells: Range<usize>,
    pub(super) rendered: String,
    pub(super) cursor_stop: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct DecodedText {
    pub(super) tokens: Vec<TextToken>,
    pub(super) width: usize,
}

impl DecodedText {
    #[cfg(test)]
    pub(super) fn render(&self, columns: usize) -> String {
        self.render_cells(0..columns)
    }

    pub(super) fn render_cells(&self, cells: Range<usize>) -> String {
        let mut output = String::new();
        for token in &self.tokens {
            if token.cells.end <= cells.start {
                continue;
            }
            if token.cells.start < cells.start {
                let end = token.cells.end.min(cells.end);
                output.extend(std::iter::repeat(' ').take(end - cells.start));
                if token.cells.end > cells.end {
                    break;
                }
                continue;
            }
            if token.cells.end > cells.end {
                break;
            }
            output.push_str(&token.rendered);
        }
        output
    }

    pub(super) fn source_to_cell(&self, source: usize) -> Option<usize> {
        self.span_for_source(source)
            .map(|span| span.cells.start)
            .or_else(|| {
                (source == self.tokens.last().map_or(0, |span| span.source.end))
                    .then_some(self.width)
            })
    }

    pub(super) fn cell_to_token(&self, cell: usize) -> Option<&TextToken> {
        self.tokens
            .iter()
            .find(|span| span.cells.contains(&cell))
    }

    pub(super) fn cell_to_source(&self, cell: usize) -> Option<usize> {
        self.cell_to_token(cell)
            .map(|span| span.source.start)
            .or_else(|| {
                (cell == self.width)
                    .then(|| self.tokens.last().map_or(0, |span| span.source.end))
            })
    }

    pub(super) fn cursor_stop_at_source(&self, source: usize) -> Option<usize> {
        self.span_for_source(source)
            .filter(|span| span.cursor_stop && span.source.start == source)
            .map(|span| span.cells.start)
    }

    pub(super) fn cursor_cell_at_source(&self, source: usize) -> Option<usize> {
        self.cursor_stop_at_source(source)
            .or_else(|| {
                let cell = self.source_to_cell(source)?;
                let span = self.cell_to_token(cell)?;
                span.cursor_stop.then_some(span.cells.start)
            })
            .or_else(|| {
                (source == self.tokens.last().map_or(0, |span| span.source.end))
                    .then(|| self.last_cursor_stop().map(|span| span.cells.start))
                    .flatten()
            })
    }

    pub(super) fn cursor_source_at_source(&self, source: usize) -> Option<usize> {
        if self.cursor_stop_at_source(source).is_some() {
            return Some(source);
        }
        self.source_to_cell(source)
            .and_then(|cell| self.cursor_stop_at_cell(cell))
            .map(|span| span.source.start)
            .or_else(|| {
                (source == self.tokens.last().map_or(0, |span| span.source.end))
                    .then(|| self.last_cursor_stop().map(|span| span.source.start))
                    .flatten()
            })
    }

    pub(super) fn cursor_source_at_cell(&self, cell: usize) -> Option<usize> {
        self.cursor_stop_at_cell(cell)
            .map(|span| span.source.start)
            .or_else(|| {
                self.cell_to_source(cell).and_then(|source| {
                    self.span_for_source(source)
                        .filter(|span| span.cursor_stop)
                        .map(|span| span.source.start)
                })
            })
            .or_else(|| {
                self.tokens
                    .iter()
                    .rev()
                    .find(|span| span.cursor_stop && span.cells.start <= cell)
                    .map(|span| span.source.start)
            })
    }

    pub(super) fn last_cursor_stop(&self) -> Option<&TextToken> {
        self.tokens.iter().rev().find(|span| span.cursor_stop)
    }

    pub(super) fn cursor_stop_at_cell(&self, cell: usize) -> Option<&TextToken> {
        self.cell_to_token(cell).filter(|span| {
            span.cursor_stop && span.cells.start == cell
        })
    }

    fn span_for_source(&self, source: usize) -> Option<&TextToken> {
        self.tokens
            .iter()
            .find(|span| span.source.contains(&source))
    }
}

#[derive(Debug)]
pub(super) struct TextDecoder {
    tab_width: usize,
    pending: Vec<u8>,
    pending_offset: usize,
    next_offset: usize,
    output: DecodedText,
}

impl TextDecoder {
    pub(super) fn new(tab_width: usize) -> Self {
        Self {
            tab_width: tab_width.max(1),
            pending: Vec::new(),
            pending_offset: 0,
            next_offset: 0,
            output: DecodedText::default(),
        }
    }

    pub(super) fn push(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
        self.next_offset += bytes.len();
        self.process(false);
    }

    pub(super) fn finish(mut self) -> DecodedText {
        self.process(true);
        self.output
    }

    fn process(&mut self, final_chunk: bool) {
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(text) => {
                    self.output
                        .decode_valid(text, self.pending_offset, self.tab_width);
                    self.pending.clear();
                    self.pending_offset = self.next_offset;
                    return;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    if valid != 0 {
                        let text = unsafe {
                            // SAFETY: `valid_up_to` ends at a UTF-8 character boundary.
                            std::str::from_utf8_unchecked(&self.pending[..valid])
                        };
                        self.output
                            .decode_valid(text, self.pending_offset, self.tab_width);
                        self.pending.drain(..valid);
                        self.pending_offset += valid;
                        continue;
                    }

                    let invalid = match error.error_len() {
                        Some(length) => length,
                        None if final_chunk => self.pending.len(),
                        None => return,
                    };
                    for index in 0..invalid {
                        self.output
                            .push_invalid(self.pending_offset + index, self.pending[index]);
                    }
                    self.pending.drain(..invalid);
                    self.pending_offset += invalid;
                }
            }
        }
    }
}

impl DecodedText {
    fn decode_valid(&mut self, text: &str, base: usize, tab_width: usize) {
        for (offset, character) in text.char_indices() {
            self.push_character(base + offset, character, tab_width);
        }
    }

    fn push_character(&mut self, offset: usize, character: char, tab_width: usize) {
        let source_end = offset + character.len_utf8();
        if character == '\t' {
            let width = tab_width - self.width % tab_width;
            self.push_token(offset..source_end, " ".repeat(width), width, true);
            return;
        }

        if character.is_control() {
            if character.is_ascii() {
                self.push_token(offset..source_end, control_text(character as u8), 2, true);
            } else {
                self.push_character_bytes_as_invalid(offset, character);
            }
            return;
        }

        let Some(width) = character.width() else {
            self.push_character_bytes_as_invalid(offset, character);
            return;
        };

        if width == 0 {
            if let Some(token) = self.tokens.last_mut() {
                token.source.end = source_end;
                token.rendered.push(character);
            } else {
                self.push_token(offset..source_end, character.to_string(), 0, false);
            }
            return;
        }

        self.push_token(offset..source_end, character.to_string(), width, true);
    }

    fn push_character_bytes_as_invalid(&mut self, offset: usize, character: char) {
        let mut buffer = [0; 4];
        for (index, byte) in character.encode_utf8(&mut buffer).bytes().enumerate() {
            self.push_invalid(offset + index, byte);
        }
    }

    fn push_invalid(&mut self, offset: usize, byte: u8) {
        self.push_token(offset..offset + 1, format!("<{byte:02X}>"), 4, true);
    }

    fn push_token(
        &mut self,
        source: Range<usize>,
        rendered: String,
        width: usize,
        cursor_stop: bool,
    ) {
        let start = self.width;
        self.width = start.saturating_add(width);
        self.tokens.push(TextToken {
            source,
            cells: start..self.width,
            rendered,
            cursor_stop,
        });
    }
}

pub(super) fn decode(bytes: &[u8], tab_width: usize) -> DecodedText {
    let mut decoder = TextDecoder::new(tab_width);
    decoder.push(bytes);
    decoder.finish()
}

fn control_text(byte: u8) -> String {
    if byte == 0x7f {
        "^?".to_owned()
    } else {
        format!("^{}", char::from(byte ^ 0x40))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_ascii_and_tracks_source_cells() {
        let decoded = decode(b"abc", DEFAULT_TAB_WIDTH);

        assert_eq!(decoded.render(3), "abc");
        assert_eq!(decoded.width, 3);
        assert_eq!(decoded.tokens[0].source, 0..1);
        assert_eq!(decoded.tokens[0].cells, 0..1);
        assert!(decoded.tokens.iter().all(|token| token.cursor_stop));
    }

    #[test]
    fn decodes_wide_characters() {
        let decoded = decode("a界".as_bytes(), DEFAULT_TAB_WIDTH);

        assert_eq!(decoded.render(3), "a界");
        assert_eq!(decoded.width, 3);
        assert_eq!(decoded.tokens[1].source, 1..4);
        assert_eq!(decoded.tokens[1].cells, 1..3);
    }

    #[test]
    fn combines_zero_width_characters_with_the_previous_token() {
        let decoded = decode("e\u{301}x".as_bytes(), DEFAULT_TAB_WIDTH);

        assert_eq!(decoded.render(2), "e\u{301}x");
        assert_eq!(decoded.tokens.len(), 2);
        assert_eq!(decoded.tokens[0].source, 0..3);
        assert_eq!(decoded.tokens[0].cells, 0..1);
    }

    #[test]
    fn preserves_utf8_split_between_chunks() {
        let bytes = "界".as_bytes();
        let mut decoder = TextDecoder::new(DEFAULT_TAB_WIDTH);
        decoder.push(&bytes[..1]);
        decoder.push(&bytes[1..]);
        let decoded = decoder.finish();

        assert_eq!(decoded.render(2), "界");
        assert_eq!(decoded.tokens[0].source, 0..3);
    }

    #[test]
    fn renders_invalid_bytes_as_uppercase_hex() {
        let decoded = decode(&[b'a', 0xff, 0xc3, b'(', 0xfe], DEFAULT_TAB_WIDTH);

        assert_eq!(decoded.render(32), "a<FF><C3>(<FE>");
        assert_eq!(decoded.tokens[1].source, 1..2);
        assert_eq!(decoded.tokens[2].source, 2..3);
    }

    #[test]
    fn renders_ascii_controls_safely() {
        let decoded = decode(&[0, 1, b'\t', 0x1b, 0x1f, 0x7f], DEFAULT_TAB_WIDTH);

        assert_eq!(decoded.render(32), "^@^A    ^[^_^?");
    }

    #[test]
    fn expands_tabs_to_each_requested_tab_width() {
        for (width, expected) in [(1, 1), (4, 3), (8, 7), (16, 15)] {
            let decoded = decode(b"a\t", width);
            assert_eq!(decoded.width, expected + 1);
            assert_eq!(decoded.render(32).chars().count(), expected + 1);
        }
    }

    #[test]
    fn maps_source_bytes_and_display_cells_to_the_same_spans() {
        let mut bytes = "a\t界e\u{301}x".as_bytes().to_vec();
        bytes.push(0xff);
        let decoded = decode(&bytes, 4);

        assert_eq!(decoded.source_to_cell(0), Some(0));
        assert_eq!(decoded.source_to_cell(1), Some(1));
        assert_eq!(decoded.source_to_cell(2), Some(4));
        assert_eq!(decoded.source_to_cell(5), Some(6));
        assert_eq!(decoded.source_to_cell(8), Some(7));
        assert_eq!(decoded.source_to_cell(9), Some(8));
        assert_eq!(decoded.source_to_cell(10), Some(12));

        assert_eq!(decoded.cell_to_source(0), Some(0));
        assert_eq!(decoded.cell_to_source(2), Some(1));
        assert_eq!(decoded.cell_to_source(4), Some(2));
        assert_eq!(decoded.cell_to_source(5), Some(2));
        assert_eq!(decoded.cell_to_source(6), Some(5));
        assert_eq!(decoded.cell_to_source(7), Some(8));
        assert_eq!(decoded.cell_to_source(11), Some(9));
        assert_eq!(decoded.cell_to_source(decoded.width), Some(10));

        assert_eq!(decoded.cell_to_token(2).unwrap().source, 1..2);
        assert_eq!(decoded.cell_to_token(5).unwrap().source, 2..5);
        assert!(decoded.cursor_stop_at_cell(2).is_none());
        assert!(decoded.cursor_stop_at_source(6).is_none());
        assert_eq!(decoded.cursor_stop_at_source(5), Some(6));
        assert_eq!(decoded.render_cells(4..8), "界e\u{301}x");
        assert_eq!(decoded.render_cells(5..8), " e\u{301}x");
    }

    #[test]
    fn maps_empty_lines_to_cell_zero_without_a_token() {
        let decoded = decode(&[], DEFAULT_TAB_WIDTH);

        assert_eq!(decoded.width, 0);
        assert_eq!(decoded.source_to_cell(0), Some(0));
        assert_eq!(decoded.cell_to_source(0), Some(0));
        assert!(decoded.cell_to_token(0).is_none());
        assert!(decoded.cursor_stop_at_source(0).is_none());
    }
}
