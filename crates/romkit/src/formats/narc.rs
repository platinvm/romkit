//! NARC, the Nitro archive: a flat list of unnamed members. DS games keep most of their data in
//! NARCs, one record or resource per member.
//!
//! Layout: a 16-byte header, then three chunks in a fixed order. `BTAF` lists each member's
//! start and end within the image, `BTNF` would name them (games leave it empty), and `GMIF` is
//! the image holding the members' bytes.

use std::borrow::Cow;
use std::ops::Range;

use crate::io::{ByteSource, Reader, Window};
use crate::{Error, Result};

const HEADER_LEN: u64 = 16;
const CHUNK_HEADER_LEN: u64 = 8;

/// An archive's member table. It holds no bytes, so it can be kept next to whatever owns the
/// archive's source; members are read from that source on demand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Narc {
    members: Vec<Range<u64>>,
}

impl Narc {
    /// Read the header and member table of the archive in `source`.
    pub fn parse<S: ByteSource + ?Sized>(source: &S) -> Result<Self> {
        let head = source.read_up_to(0, HEADER_LEN + CHUNK_HEADER_LEN + 4)?;
        let mut header = Reader::new("NARC", &head);
        header.expect(b"NARC")?;
        header.seek(HEADER_LEN as usize)?;
        header.expect(b"BTAF")?;
        let fat_len = header.u32()? as u64;
        let count = header.u16()? as u64;
        let entries = HEADER_LEN + CHUNK_HEADER_LEN + 4;
        if entries + count * 8 > HEADER_LEN + fat_len {
            return Err(header.error_at(entries as usize, "member table overruns its chunk"));
        }
        let fnt_at = HEADER_LEN + fat_len;
        let image_at = fnt_at + chunk_len(source, fnt_at, b"BTNF")?;
        let image = image_at + CHUNK_HEADER_LEN..image_at + chunk_len(source, image_at, b"GMIF")?;

        let table = source.read(entries, count * 8)?;
        let mut table = Reader::new("NARC", &table);
        let members = (0..count)
            .map(|_| {
                let (start, end) = (table.u32()? as u64, table.u32()? as u64);
                let member = image.start + start..image.start + end;
                match start <= end && member.end <= image.end {
                    true => Ok(member),
                    false => Err(Error::malformed(
                        "NARC",
                        entries,
                        "member lies outside the image",
                    )),
                }
            })
            .collect::<Result<_>>()?;
        Ok(Self { members })
    }

    pub fn len(&self) -> usize {
        self.members.len()
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// Where a member lies within the archive.
    pub fn range(&self, index: usize) -> Result<Range<u64>> {
        self.members
            .get(index)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("NARC member {index} of {}", self.len())))
    }

    /// A member's bytes, borrowed when the archive's source is in memory.
    pub fn member<'s, S: ByteSource + ?Sized>(
        &self,
        archive: &'s S,
        index: usize,
    ) -> Result<Cow<'s, [u8]>> {
        let range = self.range(index)?;
        archive.read(range.start, range.end - range.start)
    }

    /// A member as a source of its own, for reading parts of large members.
    pub fn member_source<'s, S: ByteSource + ?Sized>(
        &self,
        archive: &'s S,
        index: usize,
    ) -> Result<Window<&'s S>> {
        let range = self.range(index)?;
        Window::new(archive, range.start, range.end - range.start)
    }
}

/// The length of the chunk at `offset`, checking its signature.
fn chunk_len<S: ByteSource + ?Sized>(source: &S, offset: u64, magic: &[u8; 4]) -> Result<u64> {
    let header = source
        .read(offset, CHUNK_HEADER_LEN)
        .map_err(|_| Error::malformed("NARC", offset, "chunk lies outside the archive"))?;
    let mut header = Reader::new("NARC", &header);
    header
        .expect(magic)
        .map_err(|_| Error::malformed("NARC", offset, "unexpected chunk"))?;
    let len = header.u32()? as u64;
    match len >= CHUNK_HEADER_LEN && offset + len <= source.size() {
        true => Ok(len),
        false => Err(Error::malformed(
            "NARC",
            offset + 4,
            "chunk length is out of range",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Counting, narc};

    #[test]
    fn members_are_borrowed_from_memory() {
        let bytes = narc(&[b"first", b"", b"third"]);
        let archive = Narc::parse(bytes.as_slice()).unwrap();
        assert_eq!(archive.len(), 3);
        assert!(matches!(
            archive.member(bytes.as_slice(), 0).unwrap(),
            Cow::Borrowed(b"first")
        ));
        assert_eq!(&*archive.member(bytes.as_slice(), 1).unwrap(), b"");
        assert_eq!(&*archive.member(bytes.as_slice(), 2).unwrap(), b"third");
        assert!(matches!(
            archive.member(bytes.as_slice(), 3),
            Err(Error::NotFound(_))
        ));
    }

    #[test]
    fn parsing_reads_only_the_tables() {
        let big = vec![7; 100_000];
        let bytes = narc(&[&big, b"small"]);
        let source = Counting::new(bytes.as_slice());
        let archive = Narc::parse(&source).unwrap();
        assert!(
            source.bytes_read() < 100,
            "read {} bytes",
            source.bytes_read()
        );
        assert_eq!(
            &*archive
                .member_source(&source, 1)
                .unwrap()
                .read_all()
                .unwrap(),
            b"small"
        );
    }

    #[test]
    fn rejects_members_outside_the_image() {
        let mut bytes = narc(&[b"abc"]);
        bytes[32..36].copy_from_slice(&100u32.to_le_bytes()); // first member's end
        assert!(Narc::parse(bytes.as_slice()).is_err());
    }
}
