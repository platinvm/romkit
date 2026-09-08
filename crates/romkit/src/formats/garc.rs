//! GARC, Game Freak's archive on the 3DS: a list of entries, each holding one or more files
//! (one per language, say). Like NARCs on the DS, they hold most of a game's data, one record or
//! resource per entry, and they are named by number: `a/0/1/7`.
//!
//! Layout: a header (`CRAG`) giving where the data starts, then three chunks. `OTAF` holds each
//! entry's offset into the `BTAF` table, whose entries are a bit mask of the files present
//! followed by each file's start, end (padded), and length within the data. `BMIF` heads the
//! data. Many files are LZ11-compressed, which only their contents tell;
//! [`lz::decompress_if_compressed`](super::lz::decompress_if_compressed) restores them.

use std::borrow::Cow;
use std::ops::Range;

use crate::io::{ByteSource, Reader, Window};
use crate::{Error, Result};

const FORMAT: &str = "GARC";
/// Version 4 headers are 0x1C bytes, version 6 ones 0x24.
const MAX_HEADER_LEN: u64 = 0x24;
const CHUNK_HEADER_LEN: u64 = 12;

/// An archive's tables. It holds no bytes, so it can be kept next to whatever owns the archive's
/// source; files are read from that source on demand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Garc {
    /// Every file, in entry order, and where it lies in the archive.
    files: Vec<Range<u64>>,
    /// Each entry's slice of `files`.
    entries: Vec<Range<usize>>,
}

impl Garc {
    /// Read the header and tables of the archive in `source`.
    pub fn parse<S: ByteSource + ?Sized>(source: &S) -> Result<Self> {
        let head = source.read_up_to(0, MAX_HEADER_LEN)?;
        let mut header = Reader::new(FORMAT, &head);
        header.expect(b"CRAG")?;
        let header_len = header.u32_at(0x04)? as u64;
        let data = header.u32_at(0x10)? as u64;

        let (offsets_len, offsets) = chunk(source, header_len, b"OTAF")?;
        let count = Reader::new(FORMAT, &offsets).u16_at(8)? as usize;
        let (_, table) = chunk(source, header_len + offsets_len, b"BTAF")?;
        let (offsets, table) = (Reader::new(FORMAT, &offsets), Reader::new(FORMAT, &table));

        let mut garc = Self {
            files: Vec::new(),
            entries: Vec::with_capacity(count),
        };
        for entry in 0..count {
            // Offsets count from the end of the `BTAF` chunk header.
            let at = offsets.u32_at(CHUNK_HEADER_LEN as usize + entry * 4)? as usize
                + CHUNK_HEADER_LEN as usize;
            let bits = table.u32_at(at)?;
            let first = garc.files.len();
            for index in 0..bits.count_ones() as usize {
                let at = at + 4 + index * 12;
                let (start, end, len) = (
                    table.u32_at(at)?,
                    table.u32_at(at + 4)?,
                    table.u32_at(at + 8)?,
                );
                let file = data + start as u64..data + start as u64 + len as u64;
                if start > end || len > end - start || data + end as u64 > source.size() {
                    return Err(table.error_at(at, "file lies outside the archive"));
                }
                garc.files.push(file);
            }
            garc.entries.push(first..garc.files.len());
        }
        Ok(garc)
    }

    /// The number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every file of every entry, in order, and where it lies in the archive. Most entries have
    /// exactly one file, so this usually matches the entries one to one.
    pub fn files(&self) -> &[Range<u64>] {
        &self.files
    }

    /// Where an entry's files lie in the archive.
    pub fn entry(&self, index: usize) -> Result<&[Range<u64>]> {
        self.entries
            .get(index)
            .map(|files| &self.files[files.clone()])
            .ok_or_else(|| Error::NotFound(format!("GARC entry {index} of {}", self.len())))
    }

    /// An entry's first file, as stored, borrowed when the archive's source is in memory.
    pub fn member<'s, S: ByteSource + ?Sized>(
        &self,
        archive: &'s S,
        index: usize,
    ) -> Result<Cow<'s, [u8]>> {
        let range = self
            .entry(index)?
            .first()
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("GARC entry {index} has no files")))?;
        archive.read(range.start, range.end - range.start)
    }

    /// One of the archive's [`files`](Self::files) as a source of its own.
    pub fn file_source<'s, S: ByteSource + ?Sized>(
        &self,
        archive: &'s S,
        index: usize,
    ) -> Result<Window<&'s S>> {
        let range =
            self.files.get(index).cloned().ok_or_else(|| {
                Error::NotFound(format!("GARC file {index} of {}", self.files.len()))
            })?;
        Window::new(archive, range.start, range.end - range.start)
    }
}

/// A chunk's length and bytes, checking its signature.
fn chunk<S: ByteSource + ?Sized>(
    source: &S,
    offset: u64,
    magic: &[u8; 4],
) -> Result<(u64, Vec<u8>)> {
    let header = source
        .read(offset, CHUNK_HEADER_LEN)
        .map_err(|_| Error::malformed(FORMAT, offset, "chunk lies outside the archive"))?;
    let mut header = Reader::new(FORMAT, &header);
    header
        .expect(magic)
        .map_err(|_| Error::malformed(FORMAT, offset, "unexpected chunk"))?;
    let len = header.u32()? as u64;
    if len < CHUNK_HEADER_LEN || offset + len > source.size() {
        return Err(Error::malformed(
            FORMAT,
            offset + 4,
            "chunk length is out of range",
        ));
    }
    Ok((len, source.read(offset, len)?.into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Counting, garc};

    #[test]
    fn entries_hold_one_or_more_files() {
        let bytes = garc(&[&[b"first"], &[], &[b"en", b"fr"]]);
        let archive = Garc::parse(bytes.as_slice()).unwrap();
        assert_eq!(archive.len(), 3);
        assert_eq!(archive.files().len(), 3);
        assert_eq!(&*archive.member(bytes.as_slice(), 0).unwrap(), b"first");
        assert!(matches!(
            archive.member(bytes.as_slice(), 1),
            Err(Error::NotFound(_))
        ));
        assert_eq!(archive.entry(2).unwrap().len(), 2);
        let french = archive.file_source(bytes.as_slice(), 2).unwrap();
        assert_eq!(&*french.read_all().unwrap(), b"fr");
        assert!(archive.entry(3).is_err());
    }

    #[test]
    fn parsing_reads_only_the_tables() {
        let big = vec![7; 100_000];
        let bytes = garc(&[&[&big], &[b"small"]]);
        let source = Counting::new(bytes.as_slice());
        let archive = Garc::parse(&source).unwrap();
        assert!(
            source.bytes_read() < 200,
            "read {} bytes",
            source.bytes_read()
        );
        assert_eq!(&*archive.member(&source, 1).unwrap(), b"small");
    }

    #[test]
    fn rejects_files_outside_the_data() {
        let mut bytes = garc(&[&[b"abc"]]);
        // The header, the offsets chunk, the table's chunk header, its bit mask, and the start.
        let end = 0x1C + 16 + 12 + 4 + 4;
        bytes[end..end + 4].copy_from_slice(&1000u32.to_le_bytes());
        assert!(Garc::parse(bytes.as_slice()).is_err());
    }
}
