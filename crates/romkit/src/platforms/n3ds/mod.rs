//! Nintendo 3DS card images (NCSD): the header, the ExeFS, and the RomFS filesystem.
//!
//! An image is a table of up to eight partitions, each an NCCH container; the first holds the
//! game. Its header, which is never encrypted, identifies the game and locates two filesystems:
//! the ExeFS (code, icon, banner) and the RomFS (everything else). Retail cards encrypt both;
//! romkit reads them from decrypted dumps only, which mark themselves with a header flag.

mod romfs;

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ops::Range;

use super::{GameCode, Identity, Platform};
use crate::io::{ByteSource, Reader, Window, ascii};
use crate::{Error, Result};

/// NCSD and NCCH headers both start with a 0x100-byte signature and end at 0x200.
const HEADER_LEN: u64 = 0x200;
/// Offsets and sizes in both headers count media units of `0x200 << exponent` bytes.
const MEDIA_UNIT: u64 = 0x200;
/// `flags[7]` of an NCCH header: its contents are stored decrypted.
const NO_CRYPTO: u8 = 0x04;
/// The ExeFS header lists up to ten files, 16 bytes each, then holds their hashes.
const EXEFS_HEADER_LEN: u64 = 0x200;
const EXEFS_FILES: usize = 10;

pub fn is_n3ds(head: &[u8]) -> bool {
    head.get(0x100..0x104) == Some(b"NCSD")
}

pub fn identify<S: ByteSource + ?Sized>(source: &S) -> Result<Identity> {
    Ok(Header::read(source)?.identity())
}

/// A 3DS image opened over a byte source. Opening reads the headers and the filesystem tables,
/// about 40 KB for a Pokémon game; file contents are read on demand.
#[derive(Debug, Clone)]
pub struct N3dsRom<S> {
    source: S,
    header: Header,
    fs: FileSystem,
}

impl<S: ByteSource> N3dsRom<S> {
    pub fn open(source: S) -> Result<Self> {
        let header = Header::read(&source)?;
        if header.encrypted {
            return Err(Error::Unsupported(format!(
                "{} is encrypted; romkit reads decrypted 3DS images",
                header.identity()
            )));
        }
        let mut files = BTreeMap::new();
        if !header.exefs.is_empty() {
            for (name, range) in exefs(&source, header.exefs.clone())? {
                files.insert(format!("exefs/{name}"), range);
            }
        }
        if !header.romfs.is_empty() {
            for (path, range) in romfs::parse(&source, header.romfs.clone())? {
                if files.insert(path, range).is_some() {
                    return Err(Error::malformed("RomFS", 0, "a path appears twice"));
                }
            }
        }
        Ok(Self {
            source,
            header,
            fs: FileSystem { files },
        })
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

    /// A named file, as a source over its bytes in the image.
    pub fn file(&self, path: &str) -> Result<Window<&S>> {
        let range = self
            .fs
            .range(path)
            .ok_or_else(|| Error::NotFound(path.to_owned()))?;
        Window::new(&self.source, range.start, range.end - range.start)
    }

    /// A named file's bytes, borrowed when the image is in memory.
    pub fn read(&self, path: &str) -> Result<Cow<'_, [u8]>> {
        let file = self.file(path)?;
        self.source.read(file.offset(), file.size())
    }
}

/// The game partition's files: RomFS files by their path, and ExeFS files (`.code`, `icon`,
/// `banner`, `logo`) under `exefs/`.
#[derive(Debug, Clone, Default)]
pub struct FileSystem {
    files: BTreeMap<String, Range<u64>>,
}

impl FileSystem {
    /// Where the file at `path` (optionally with a leading `/`) lies in the image.
    pub fn range(&self, path: &str) -> Option<Range<u64>> {
        self.files.get(path.trim_start_matches('/')).cloned()
    }

    /// Every file and where it lies, in path order.
    pub fn paths(&self) -> impl ExactSizeIterator<Item = (&str, Range<u64>)> {
        self.files
            .iter()
            .map(|(path, range)| (path.as_str(), range.clone()))
    }
}

/// The fields romkit reads from the NCSD header and the game partition's NCCH header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// The product code, e.g. `CTR-P-EKJA`.
    pub product: String,
    pub code: GameCode,
    pub revision: u16,
    pub encrypted: bool,
    /// Where the ExeFS lies in the image; empty when there is none.
    pub exefs: Range<u64>,
    /// Where the RomFS lies in the image; empty when there is none.
    pub romfs: Range<u64>,
}

impl Header {
    /// Read both headers: 1 KB at the start of the image and of its first partition.
    pub fn read<S: ByteSource + ?Sized>(source: &S) -> Result<Self> {
        let ncsd = source.read(0, HEADER_LEN)?;
        let ncsd = Reader::new("NCSD header", &ncsd);
        let partition = ncsd.u32_at(0x120)? as u64 * media_unit(&ncsd)?;
        let ncch = source
            .read(partition, HEADER_LEN)
            .map_err(|_| ncsd.error_at(0x120, "first partition lies outside the image"))?;
        let mut header = Reader::new("NCCH header", &ncch);
        header.seek(0x100)?;
        header.expect(b"NCCH")?;
        // The product code reads like `CTR-P-EKJA`; its last part is the game code.
        let product = ascii(header.slice_at(0x150, 16)?);
        let code = product
            .rsplit('-')
            .next()
            .and_then(|code| code.as_bytes().try_into().ok())
            .map(GameCode)
            .ok_or_else(|| header.error_at(0x150, "product code has no game code"))?;
        let unit = media_unit(&header)?;
        let region = |at: usize| -> Result<Range<u64>> {
            let start = partition + header.u32_at(at)? as u64 * unit;
            let end = start + header.u32_at(at + 4)? as u64 * unit;
            match end <= source.size() {
                true => Ok(start..end),
                false => Err(header.error_at(at, "region lies outside the image")),
            }
        };
        Ok(Self {
            product,
            code,
            revision: header.u16_at(0x112)?,
            encrypted: header.u8_at(0x18F)? & NO_CRYPTO == 0,
            exefs: region(0x1A0)?,
            romfs: region(0x1B0)?,
        })
    }

    pub fn identity(&self) -> Identity {
        Identity {
            platform: Platform::Nintendo3ds,
            title: self.product.clone(),
            code: Some(self.code),
            region: self.code.region(),
            revision: self.revision,
        }
    }
}

/// The media unit, from the exponent in `flags[6]` (at 0x18E in both headers).
fn media_unit(header: &Reader) -> Result<u64> {
    match header.u8_at(0x18E)? {
        exponent @ 0..=15 => Ok(MEDIA_UNIT << exponent),
        _ => Err(header.error_at(0x18E, "media unit is out of range")),
    }
}

/// The ExeFS file table: `(name, offset, size)` entries, offsets counting from the end of the
/// header.
fn exefs<S: ByteSource + ?Sized>(
    source: &S,
    region: Range<u64>,
) -> Result<Vec<(String, Range<u64>)>> {
    if region.end - region.start < EXEFS_HEADER_LEN {
        return Err(Error::malformed("ExeFS header", 0, "header is truncated"));
    }
    let header = source.read(region.start, EXEFS_HEADER_LEN)?;
    let header = Reader::new("ExeFS header", &header);
    let mut files = Vec::new();
    for index in 0..EXEFS_FILES {
        let at = index * 16;
        let name = ascii(header.slice_at(at, 8)?);
        if name.is_empty() {
            continue;
        }
        let start = region.start + EXEFS_HEADER_LEN + header.u32_at(at + 8)? as u64;
        let end = start + header.u32_at(at + 12)? as u64;
        if name.contains('/') || end > region.end {
            return Err(header.error_at(at, "invalid ExeFS file"));
        }
        files.push((name, start..end));
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Counting, n3ds};

    #[test]
    fn identifies_the_first_partition() {
        let mut image = vec![0; 0x600];
        image[0x100..0x104].copy_from_slice(b"NCSD");
        image[0x120] = 2; // first partition at media unit 2 (0x400)
        image[0x500..0x504].copy_from_slice(b"NCCH");
        image[0x512] = 1;
        image[0x550..0x55A].copy_from_slice(b"CTR-P-EKJA");

        assert!(is_n3ds(&image));
        let identity = identify(image.as_slice()).unwrap();
        assert_eq!(identity.title, "CTR-P-EKJA");
        assert_eq!(identity.code, Some(GameCode(*b"EKJA")));
        assert_eq!(identity.revision, 1);
        // Without the no-crypto flag, the contents are encrypted.
        assert!(matches!(
            N3dsRom::open(image.as_slice()),
            Err(Error::Unsupported(_))
        ));

        image[0x120] = 9;
        assert!(identify(image.as_slice()).is_err());
    }

    #[test]
    fn lists_exefs_and_romfs_files() {
        let image = n3ds(
            "CTR-P-EKJA",
            &[("icon", b"SMDH")],
            &[
                ("a/0/0/1", b"first"),
                ("a/0/0/2", b""),
                ("sound/x.bin", b"x"),
            ],
        );
        let rom = N3dsRom::open(image.as_slice()).unwrap();
        assert_eq!(rom.header().code, GameCode(*b"EKJA"));
        let paths: Vec<_> = rom.fs().paths().map(|(path, _)| path).collect();
        assert_eq!(paths, ["a/0/0/1", "a/0/0/2", "exefs/icon", "sound/x.bin"]);
        assert_eq!(&*rom.read("a/0/0/1").unwrap(), b"first");
        assert_eq!(&*rom.read("/sound/x.bin").unwrap(), b"x");
        assert_eq!(&*rom.read("exefs/icon").unwrap(), b"SMDH");
        assert_eq!(rom.file("a/0/0/2").unwrap().size(), 0);
        assert!(matches!(rom.file("a/0/0/3"), Err(Error::NotFound(_))));
    }

    #[test]
    fn opening_reads_only_the_tables() {
        let big = vec![0xAB; 1 << 20];
        let image = n3ds("CTR-P-EKJA", &[], &[("big.bin", &big), ("small", b"tiny")]);
        let source = Counting::new(image.as_slice());
        let rom = N3dsRom::open(&source).unwrap();
        assert!(
            source.bytes_read() < 4096,
            "read {} bytes",
            source.bytes_read()
        );
        assert_eq!(
            &*rom.file("big.bin").unwrap().read(10, 2).unwrap(),
            [0xAB; 2]
        );
    }
}
