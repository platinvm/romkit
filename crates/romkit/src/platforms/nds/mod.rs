//! Nintendo DS cartridges: the header, the NitroFS filesystem, and the banner.

mod banner;
mod code;
mod fs;

use std::borrow::Cow;
use std::ops::Range;

pub use banner::Banner;
pub use fs::FileSystem;

use super::{GameCode, Identity, Platform};
use crate::io::{ByteSource, Reader, Window, ascii};
use crate::{Error, Result};

/// The header holds a CRC of the Nintendo logo at 0x15C, the same on every cartridge.
const LOGO_CRC: u16 = 0xCF56;
/// The header fields romkit reads all lie below this offset.
const HEADER_LEN: usize = 0x160;

pub fn is_nds(head: &[u8]) -> bool {
    head.get(0x15C..0x15E) == Some(&LOGO_CRC.to_le_bytes())
}

/// A DS ROM opened over a byte source. Opening reads the header and the filesystem tables;
/// file contents are read on demand.
#[derive(Debug, Clone)]
pub struct NdsRom<S> {
    source: S,
    header: Header,
    fs: FileSystem,
}

impl<S: ByteSource> NdsRom<S> {
    pub fn open(source: S) -> Result<Self> {
        let header = Header::parse(&source.read_up_to(0, HEADER_LEN as u64)?)?;
        let fnt = read_table(&source, &header.fnt, "DS file name table")?;
        let fat = read_table(&source, &header.fat, "DS file allocation table")?;
        let fs = FileSystem::parse(&fnt, &fat, source.size())?;
        Ok(Self { source, header, fs })
    }

    pub fn source(&self) -> &S {
        &self.source
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn fs(&self) -> &FileSystem {
        &self.fs
    }

    /// A named file, as a source over its bytes in the ROM.
    pub fn file(&self, path: &str) -> Result<Window<&S>> {
        let id = self
            .fs
            .id(path)
            .ok_or_else(|| Error::NotFound(path.to_owned()))?;
        self.file_by_id(id)
    }

    /// A named file's bytes, borrowed when the ROM is in memory.
    pub fn read(&self, path: &str) -> Result<Cow<'_, [u8]>> {
        let file = self.file(path)?;
        self.source.read(file.offset(), file.size())
    }

    /// Any file allocation table entry, named or not (overlays have no name).
    pub fn file_by_id(&self, id: u16) -> Result<Window<&S>> {
        let range = self
            .fs
            .range(id)
            .ok_or_else(|| Error::NotFound(format!("DS file #{id}")))?;
        Window::new(&self.source, range.start, range.end - range.start)
    }

    /// The ARM9 binary, the game's main code, decompressed. Games keep some tables in it.
    pub fn arm9(&self) -> Result<Vec<u8>> {
        let range = &self.header.arm9;
        let binary = read_table(&self.source, range, "DS ARM9")?.into_owned();
        let footer = self.source.read_up_to(range.end, 12)?;
        code::arm9(binary, self.header.arm9_address, &footer)
    }

    pub fn banner(&self) -> Result<Option<Banner>> {
        match self.header.banner_offset {
            0 => Ok(None),
            offset => Banner::parse(&self.source.read_up_to(offset, Banner::MAX_LEN)?).map(Some),
        }
    }
}

fn read_table<'s, S: ByteSource>(
    source: &'s S,
    table: &Range<u64>,
    format: &'static str,
) -> Result<Cow<'s, [u8]>> {
    source
        .read(table.start, table.end - table.start)
        .map_err(|_| Error::malformed(format, table.start, "table lies outside the ROM"))
}

/// The header fields romkit uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub title: String,
    pub code: GameCode,
    pub revision: u8,
    /// Where the ARM9 binary lies in the ROM, and where it is loaded in memory.
    pub arm9: Range<u64>,
    pub arm9_address: u32,
    /// Where the file name table lies in the ROM.
    pub fnt: Range<u64>,
    /// Where the file allocation table lies in the ROM.
    pub fat: Range<u64>,
    /// `0` when the ROM has no banner.
    pub banner_offset: u64,
}

impl Header {
    pub fn parse(head: &[u8]) -> Result<Self> {
        let header = Reader::new("DS header", head);
        if head.len() < HEADER_LEN {
            return Err(header.error_at(0, "header is truncated"));
        }
        let table = |at| -> Result<Range<u64>> {
            let start = header.u32_at(at)? as u64;
            Ok(start..start + header.u32_at(at + 4)? as u64)
        };
        Ok(Self {
            title: ascii(header.slice_at(0, 12)?),
            code: GameCode(header.array_at(0x0C)?),
            revision: header.u8_at(0x1E)?,
            arm9: {
                let start = header.u32_at(0x20)? as u64;
                start..start + header.u32_at(0x2C)? as u64
            },
            arm9_address: header.u32_at(0x28)?,
            fnt: table(0x40)?,
            fat: table(0x48)?,
            banner_offset: header.u32_at(0x68)?.into(),
        })
    }

    pub fn identity(&self) -> Identity {
        Identity {
            platform: Platform::NintendoDs,
            title: self.title.clone(),
            code: Some(self.code),
            region: self.code.region(),
            revision: self.revision.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Counting, nds};

    #[test]
    fn files_resolve_by_path_through_nested_directories() {
        let rom = nds(
            b"TSTE",
            &[
                ("a/b/c.bin", b"deep"),
                ("a/x.bin", b"mid"),
                ("top.bin", b"top"),
                ("a/b/d.bin", b"deep2"),
            ],
        );
        let nds = NdsRom::open(rom.as_slice()).unwrap();
        assert_eq!(nds.header().code, GameCode(*b"TSTE"));
        assert_eq!(
            &*nds.file("a/b/c.bin").unwrap().read_all().unwrap(),
            b"deep"
        );
        assert_eq!(
            &*nds.file("/a/b/d.bin").unwrap().read_all().unwrap(),
            b"deep2"
        );
        assert_eq!(&*nds.file("a/x.bin").unwrap().read_all().unwrap(), b"mid");
        assert_eq!(&*nds.file("top.bin").unwrap().read_all().unwrap(), b"top");
        assert!(matches!(nds.file("a/missing.bin"), Err(Error::NotFound(_))));
        let paths: Vec<_> = nds.fs().paths().map(|(path, _)| path).collect();
        assert_eq!(paths, ["a/b/c.bin", "a/b/d.bin", "a/x.bin", "top.bin"]);
    }

    #[test]
    fn opening_reads_only_the_tables() {
        let big = vec![0xAB; 1 << 20];
        let rom = nds(b"TSTE", &[("big.bin", &big), ("small.bin", b"tiny")]);
        let source = Counting::new(rom.as_slice());
        let nds = NdsRom::open(&source).unwrap();
        assert!(
            source.bytes_read() < 1024,
            "read {} bytes",
            source.bytes_read()
        );
        assert_eq!(
            &*nds.file("big.bin").unwrap().read(10, 2).unwrap(),
            [0xAB, 0xAB]
        );
        assert!(source.bytes_read() < 1024);
    }
}
