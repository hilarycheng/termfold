use std::{io, ops::Range};

use super::{
    MAX_QUERY_BYTES,
    source::{BLOCK_SIZE, FileSource},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SearchQueryError {
    Empty,
    TooLong,
    InvalidHex,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SearchQuery {
    Text(Vec<u8>),
    Hex(Vec<u8>),
}

impl SearchQuery {
    pub(super) fn parse(input: &[u8]) -> Result<Self, SearchQueryError> {
        if input.is_empty() {
            return Err(SearchQueryError::Empty);
        }
        if input.len() > MAX_QUERY_BYTES {
            return Err(SearchQueryError::TooLong);
        }
        if let Some(hex) = input.strip_prefix(b"hex:") {
            return parse_hex(hex).map(Self::Hex);
        }
        Ok(Self::Text(input.to_vec()))
    }

    pub(super) fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Text(bytes) | Self::Hex(bytes) => bytes,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.as_bytes().len()
    }

    pub(super) fn matches_bytes(&self, candidate: &[u8]) -> bool {
        self.as_bytes().len() == candidate.len()
            && self
                .as_bytes()
                .iter()
                .copied()
                .zip(candidate.iter().copied())
                .all(|(query, byte)| self.matches_byte(query, byte))
    }

    #[cfg(test)]
    pub(super) fn matches(&self, candidate: &[u8]) -> bool {
        self.matches_bytes(candidate)
    }

    pub(super) fn matches_byte(&self, query: u8, byte: u8) -> bool {
        match self {
            Self::Text(_) => query.to_ascii_lowercase() == byte.to_ascii_lowercase(),
            Self::Hex(_) => query == byte,
        }
    }

    pub(super) fn visible_matches(
        &self,
        source: &mut FileSource,
        visible: Range<u64>,
    ) -> io::Result<Vec<Range<u64>>> {
        let query_len = self.len() as u64;
        let visible = visible.start.min(source.len())..visible.end.min(source.len());
        if query_len == 0 || visible.start >= visible.end {
            return Ok(Vec::new());
        }

        let scan_start = visible.start.saturating_sub(query_len - 1);
        let candidate_step = BLOCK_SIZE - query_len + 1;
        let mut candidate_start = scan_start;
        let mut matches = Vec::new();
        while candidate_start < visible.end {
            let candidate_end = candidate_start
                .saturating_add(candidate_step)
                .min(visible.end);
            let read_end = candidate_end
                .saturating_add(query_len - 1)
                .min(source.len());
            let bytes = source.read_range(candidate_start..read_end)?;
            let candidates = (candidate_end - candidate_start) as usize;
            for offset in 0..candidates {
                let end = offset.saturating_add(query_len as usize);
                if end > bytes.len() || !self.matches_bytes(&bytes[offset..end]) {
                    continue;
                }
                let start = candidate_start + offset as u64;
                let end = start.saturating_add(query_len);
                if start < visible.end && end > visible.start {
                    matches.push(start.max(visible.start)..end.min(visible.end));
                }
            }
            candidate_start = candidate_end;
        }
        Ok(matches)
    }
}

fn parse_hex(input: &[u8]) -> Result<Vec<u8>, SearchQueryError> {
    let mut bytes = Vec::new();
    for token in input.split(|byte| *byte == b' ') {
        if token.is_empty() {
            continue;
        }
        if token.len() != 2 {
            return Err(SearchQueryError::InvalidHex);
        }
        let high = hex_digit(token[0]).ok_or(SearchQueryError::InvalidHex)?;
        let low = hex_digit(token[1]).ok_or(SearchQueryError::InvalidHex)?;
        bytes.push(high << 4 | low);
    }
    if bytes.is_empty() {
        Err(SearchQueryError::InvalidHex)
    } else {
        Ok(bytes)
    }
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
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
    fn text_search_is_ascii_case_insensitive_only() {
        let query = SearchQuery::parse(b"Ab").unwrap();
        assert!(query.matches(b"aB"));
        assert!(!query.matches(b"ac"));

        let query = SearchQuery::parse("界".as_bytes()).unwrap();
        assert!(query.matches("界".as_bytes()));
        assert!(!query.matches("界面".as_bytes()));
    }

    #[test]
    fn invalid_utf8_is_compared_exactly() {
        let query = SearchQuery::parse(&[0xff, 0x80]).unwrap();
        assert!(query.matches(&[0xff, 0x80]));
        assert!(!query.matches(&[0xff, 0x81]));
    }

    #[test]
    fn text_query_has_a_256_byte_limit() {
        assert!(SearchQuery::parse(&[b'x'; MAX_QUERY_BYTES]).is_ok());
        assert_eq!(
            SearchQuery::parse(&[b'x'; MAX_QUERY_BYTES + 1]),
            Err(SearchQueryError::TooLong)
        );
    }

    #[test]
    fn parses_space_separated_hex_bytes() {
        assert_eq!(
            SearchQuery::parse(b"hex:00 FF 1B"),
            Ok(SearchQuery::Hex(vec![0x00, 0xff, 0x1b]))
        );
        assert!(SearchQuery::parse(b"hex: 00  FF ").is_ok());
    }

    #[test]
    fn rejects_invalid_hex_forms() {
        for query in [
            b"hex:".as_slice(),
            b"hex: ",
            b"hex:0",
            b"hex:000",
            b"hex:00FF",
            b"hex:GG",
            b"hex:0G",
            b"hex:100",
            b"hex:00\tFF",
        ] {
            assert_eq!(
                SearchQuery::parse(query),
                Err(SearchQueryError::InvalidHex),
                "{query:?}"
            );
        }
    }

    #[test]
    fn rejects_empty_text() {
        assert_eq!(SearchQuery::parse(b""), Err(SearchQueryError::Empty));
    }

    #[test]
    fn visible_search_matches_case_and_raw_block_boundaries() {
        let path = temp_path("termfold-search-hex-boundary");
        let mut data = vec![b'x'; BLOCK_SIZE as usize + 8];
        data[3..5].copy_from_slice(b"Ab");
        data[BLOCK_SIZE as usize - 1..BLOCK_SIZE as usize + 2].copy_from_slice(&[0x00, 0xff, 0x1b]);
        fs::write(&path, &data).unwrap();

        let mut source = FileSource::open(path.clone()).unwrap();
        let text = SearchQuery::parse(b"aB").unwrap();
        assert_eq!(text.visible_matches(&mut source, 0..8).unwrap(), vec![3..5]);
        let hex = SearchQuery::parse(b"hex:00 FF 1B").unwrap();
        assert_eq!(
            hex.visible_matches(&mut source, BLOCK_SIZE - 2..BLOCK_SIZE + 4)
                .unwrap(),
            vec![BLOCK_SIZE - 1..BLOCK_SIZE + 2]
        );

        fs::remove_file(path).unwrap();
    }
}
