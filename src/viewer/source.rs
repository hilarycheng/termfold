use std::{
    collections::VecDeque,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    ops::Range,
    path::{Path, PathBuf},
};

pub(super) const BLOCK_SIZE: u64 = 64 * 1024;
pub(super) const BLOCK_CACHE_SIZE: usize = 8;
const MAX_RANGE_BYTES: u64 = BLOCK_SIZE;

#[derive(Debug)]
struct Block {
    offset: u64,
    bytes: Vec<u8>,
}

#[derive(Debug)]
pub(super) struct FileSource {
    path: PathBuf,
    file: File,
    length: u64,
    blocks: VecDeque<Block>,
    #[cfg(test)]
    block_reads: usize,
    #[cfg(test)]
    block_accesses: usize,
    #[cfg(test)]
    peak_cache_bytes: usize,
}

impl FileSource {
    pub(super) fn open(path: PathBuf) -> io::Result<Self> {
        let file = File::open(&path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "viewer target is not a regular file",
            ));
        }
        Ok(Self {
            path,
            file,
            length: metadata.len(),
            blocks: VecDeque::new(),
            #[cfg(test)]
            block_reads: 0,
            #[cfg(test)]
            block_accesses: 0,
            #[cfg(test)]
            peak_cache_bytes: 0,
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn len(&self) -> u64 {
        self.length
    }

    pub(super) fn read_byte(&mut self, offset: u64) -> io::Result<Option<u8>> {
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

    pub(super) fn read_range(&mut self, range: Range<u64>) -> io::Result<Vec<u8>> {
        let requested = range.end.checked_sub(range.start).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "viewer range is reversed")
        })?;
        if requested > MAX_RANGE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "viewer range is too large",
            ));
        }

        let start = range.start.min(self.length);
        let end = range.end.min(self.length);
        let mut bytes = Vec::with_capacity((end - start) as usize);
        let mut position = start;
        while position < end {
            let block_offset = position / BLOCK_SIZE * BLOCK_SIZE;
            let block_end = block_offset.saturating_add(BLOCK_SIZE).min(end);
            let first = (position - block_offset) as usize;
            let last = (block_end - block_offset) as usize;
            let block = self.block(block_offset)?;
            if last > block.len() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "viewer cache block is shorter than the file",
                ));
            }
            bytes.extend_from_slice(&block[first..last]);
            position = block_end;
        }
        Ok(bytes)
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
        if self.blocks.len() == BLOCK_CACHE_SIZE {
            self.blocks.pop_back();
        }
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
        Ok(&self.blocks.front().expect("cache block exists").bytes)
    }

    #[cfg(test)]
    pub(super) fn cache_block_count(&self) -> usize {
        self.blocks.len()
    }

    #[cfg(test)]
    pub(super) fn cache_bytes(&self) -> usize {
        self.blocks.iter().map(|block| block.bytes.len()).sum()
    }

    #[cfg(test)]
    pub(super) fn cache_offsets(&self) -> Vec<u64> {
        self.blocks.iter().map(|block| block.offset).collect()
    }

    #[cfg(test)]
    pub(super) fn block_reads(&self) -> usize {
        self.block_reads
    }

    #[cfg(test)]
    pub(super) fn block_accesses(&self) -> usize {
        self.block_accesses
    }

    #[cfg(test)]
    pub(super) fn peak_cache_bytes(&self) -> usize {
        self.peak_cache_bytes
    }

    #[cfg(test)]
    pub(super) fn reset_metrics(&mut self) {
        self.block_reads = 0;
        self.block_accesses = 0;
    }

    #[cfg(all(test, unix))]
    pub(super) fn replace_file(&mut self, file: File) {
        self.file = file;
        self.blocks.clear();
    }
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

    #[test]
    fn reads_aligned_blocks() {
        let path = temp_path("termfold-source-aligned");
        let data = vec![b'x'; BLOCK_SIZE as usize * 2];
        fs::write(&path, &data).unwrap();

        let mut source = FileSource::open(path.clone()).unwrap();
        assert_eq!(source.read_range(1..3).unwrap(), b"xx");
        assert_eq!(source.cache_offsets(), vec![0]);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn reads_ranges_across_block_boundaries() {
        let path = temp_path("termfold-source-cross-block");
        let mut data = vec![b'x'; BLOCK_SIZE as usize * 2];
        data[BLOCK_SIZE as usize - 1] = b'a';
        data[BLOCK_SIZE as usize] = b'b';
        fs::write(&path, &data).unwrap();

        let mut source = FileSource::open(path.clone()).unwrap();
        assert_eq!(
            source
                .read_range(BLOCK_SIZE - 1..BLOCK_SIZE + 1)
                .unwrap(),
            b"ab"
        );
        assert!(source.cache_offsets().iter().all(|offset| {
            offset.is_multiple_of(BLOCK_SIZE)
        }));

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn evicts_least_recently_used_blocks_at_the_bound() {
        let path = temp_path("termfold-source-lru");
        let data = vec![b'x'; BLOCK_SIZE as usize * (BLOCK_CACHE_SIZE + 2)];
        fs::write(&path, &data).unwrap();

        let mut source = FileSource::open(path.clone()).unwrap();
        for block in 0..BLOCK_CACHE_SIZE + 2 {
            let start = block as u64 * BLOCK_SIZE;
            source.read_range(start..start + BLOCK_SIZE).unwrap();
        }
        assert_eq!(source.cache_block_count(), BLOCK_CACHE_SIZE);
        assert!(source.cache_bytes() <= BLOCK_SIZE as usize * BLOCK_CACHE_SIZE);
        assert_eq!(
            source.cache_offsets(),
            (2..BLOCK_CACHE_SIZE + 2)
                .rev()
                .map(|block| block as u64 * BLOCK_SIZE)
                .collect::<Vec<_>>()
        );

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn snapshot_ignores_append() {
        let path = temp_path("termfold-source-append");
        fs::write(&path, b"original").unwrap();
        let mut source = FileSource::open(path.clone()).unwrap();
        let length = source.len();

        let mut append = OpenOptions::new().append(true).open(&path).unwrap();
        append.write_all(b"-append").unwrap();
        drop(append);

        assert_eq!(source.len(), length);
        assert_eq!(source.read_range(0..length).unwrap(), b"original");

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn snapshot_truncate_does_not_change_length_or_panic() {
        let path = temp_path("termfold-source-truncate");
        fs::write(&path, vec![b'x'; BLOCK_SIZE as usize + 1]).unwrap();
        let mut source = FileSource::open(path.clone()).unwrap();
        let length = source.len();

        let truncate = OpenOptions::new().write(true).open(&path).unwrap();
        truncate.set_len(0).unwrap();
        drop(truncate);

        assert_eq!(source.len(), length);
        assert!(source.read_range(0..length).is_err());

        fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_replacement_keeps_the_open_file() {
        let path = temp_path("termfold-source-replace");
        let replacement = temp_path("termfold-source-replacement");
        fs::write(&path, b"original").unwrap();
        fs::write(&replacement, b"replacement").unwrap();
        let mut source = FileSource::open(path.clone()).unwrap();

        fs::rename(&replacement, &path).unwrap();

        let length = source.len();
        assert_eq!(source.read_range(0..length).unwrap(), b"original");

        fs::remove_file(path).unwrap();
    }
}
