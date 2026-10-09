//! [`Rom`]: any ROM, opened for browsing and extraction. The entry point for hosts that do not
//! know in advance what they will be given, such as the website.

use std::borrow::Cow;
use std::rc::Rc;

use serde::Serialize;

use crate::formats::Format;
use crate::formats::garc::Garc;
use crate::formats::lz;
use crate::formats::narc::Narc;
use crate::games::{self, Game, GameId};
use crate::io::{ByteSource, Window};
use crate::platforms::n3ds::N3dsRom;
use crate::platforms::nds::{Banner, NdsRom};
use crate::platforms::{Identity, Platform, identify};
use crate::{Error, Result};

/// A source shared by everything opened over it.
type Shared<'a> = Rc<dyn ByteSource + 'a>;

/// An identified ROM, with its game's extractor when romkit has one. Opening reads only headers
/// and tables; everything else is read on first use and kept.
pub struct Rom<'a> {
    source: Shared<'a>,
    identity: Identity,
    game_id: Option<GameId>,
    game: Option<Box<dyn Game + 'a>>,
    container: Option<Container<'a>>,
}

/// The platforms whose ROMs hold a filesystem.
enum Container<'a> {
    Nds(NdsRom<Shared<'a>>),
    N3ds(N3dsRom<Shared<'a>>),
}

impl<'a> Container<'a> {
    fn paths(&self) -> Vec<&str> {
        match self {
            Self::Nds(nds) => nds.fs().paths().map(|(path, _)| path).collect(),
            Self::N3ds(n3ds) => n3ds.fs().paths().map(|(path, _)| path).collect(),
        }
    }

    fn file(&self, path: &str) -> Result<Window<&Shared<'a>>> {
        match self {
            Self::Nds(nds) => nds.file(path),
            Self::N3ds(n3ds) => n3ds.file(path),
        }
    }
}

/// What a host shows first.
#[derive(Debug, Serialize)]
pub struct Overview {
    pub identity: Identity,
    pub game: Option<GameId>,
    pub game_name: Option<&'static str>,
    /// Whether the game's data can be extracted, beyond browsing its files.
    pub supported: bool,
    pub banner: Option<Banner>,
}

/// A file in the ROM's filesystem, or a member of an archive.
#[derive(Debug, Serialize)]
pub struct FileInfo {
    /// The file's path, or the member's index.
    pub path: String,
    pub size: u64,
    pub format: Option<Format>,
}

/// Written last by [`Rom::export`], describing the ROM and the assets extracted from it.
#[derive(Debug, Serialize)]
pub struct Manifest {
    pub romkit: &'static str,
    pub game: GameId,
    pub identity: Identity,
    /// CRC-32 of the whole ROM, comparable with No-Intro checksums.
    pub crc32: String,
    pub assets: Vec<String>,
}

impl<'a> Rom<'a> {
    pub fn open(source: impl ByteSource + 'a) -> Result<Self> {
        let source: Shared<'a> = Rc::new(source);
        let identity = identify(&source)?;
        let game_id = GameId::identify(&identity);
        let game = game_id
            .and_then(|id| games::open(id, Rc::clone(&source)))
            .transpose()?;
        let container = match identity.platform {
            Platform::NintendoDs => Some(Container::Nds(NdsRom::open(Rc::clone(&source))?)),
            Platform::Nintendo3ds => Some(Container::N3ds(N3dsRom::open(Rc::clone(&source))?)),
            _ => None,
        };
        Ok(Self {
            source,
            identity,
            game_id,
            game,
            container,
        })
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// The recognized game, supported or not.
    pub fn game_id(&self) -> Option<GameId> {
        self.game_id
    }

    /// The game's extractor, when it is supported.
    pub fn game(&self) -> Option<&dyn Game> {
        self.game.as_deref()
    }

    pub fn overview(&self) -> Result<Overview> {
        Ok(Overview {
            identity: self.identity.clone(),
            game: self.game_id,
            game_name: self.game_id.map(GameId::name),
            supported: self.game.is_some(),
            banner: match &self.container {
                Some(Container::Nds(nds)) => nds.banner()?,
                _ => None,
            },
        })
    }

    /// Every named file, with its size and format. Only DS and 3DS ROMs have a filesystem;
    /// other ROMs list nothing.
    pub fn files(&self) -> Result<Vec<FileInfo>> {
        let Some(container) = &self.container else {
            return Ok(Vec::new());
        };
        container
            .paths()
            .into_iter()
            .map(|path| describe(path.to_owned(), &container.file(path)?))
            .collect()
    }

    pub fn read_file(&self, path: &str) -> Result<Vec<u8>> {
        Ok(self.container()?.file(path)?.read_all()?.into_owned())
    }

    /// The members of the archive at `path`: a NARC's members, or every file of a GARC's
    /// entries. Compressed GARC files are listed and read decompressed.
    pub fn archive(&self, path: &str) -> Result<Vec<FileInfo>> {
        let file = self.container()?.file(path)?;
        match Archive::parse(&file)? {
            Archive::Narc(narc) => (0..narc.len())
                .map(|index| describe(index.to_string(), &narc.member_source(&file, index)?))
                .collect(),
            Archive::Garc(garc) => (0..garc.files().len())
                .map(|index| {
                    // Files that may be compressed are unpacked, so that their listing is exact.
                    let stored = garc.file_source(&file, index)?;
                    let bytes = match stored.read_up_to(0, 1)?.first() {
                        Some(0x10 | 0x11) => stored.read_all()?,
                        _ => stored.read_up_to(0, 4)?,
                    };
                    let (size, format) = match lz::decompress_if_compressed(&bytes) {
                        Cow::Owned(contents) => (contents.len() as u64, Format::sniff(&contents)),
                        Cow::Borrowed(head) => (stored.size(), Format::sniff(head)),
                    };
                    Ok(FileInfo {
                        path: index.to_string(),
                        size,
                        format,
                    })
                })
                .collect(),
        }
    }

    pub fn read_member(&self, path: &str, index: usize) -> Result<Vec<u8>> {
        let file = self.container()?.file(path)?;
        Ok(match Archive::parse(&file)? {
            Archive::Narc(narc) => narc.member(&file, index)?.into_owned(),
            Archive::Garc(garc) => {
                lz::decompress_if_compressed(&garc.file_source(&file, index)?.read_all()?)
                    .into_owned()
            }
        })
    }

    /// CRC-32 of the whole ROM, read in 1 MiB chunks.
    pub fn crc32(&self) -> Result<u32> {
        let mut hasher = crc32fast::Hasher::new();
        let mut offset = 0;
        while offset < self.source.size() {
            let chunk = self.source.read_up_to(offset, 1 << 20)?;
            hasher.update(&chunk);
            offset += chunk.len() as u64;
        }
        Ok(hasher.finalize())
    }

    /// What [`export`](Self::export) writes as `manifest.json`, for hosts that extract the
    /// assets one at a time.
    pub fn manifest(&self) -> Result<Manifest> {
        let game = self.extractor()?;
        Ok(Manifest {
            romkit: env!("CARGO_PKG_VERSION"),
            game: game.id(),
            identity: self.identity.clone(),
            crc32: format!("{:08x}", self.crc32()?),
            assets: game.assets()?,
        })
    }

    /// Extract every asset through `write(path, contents)`, then write a `manifest.json`
    /// describing the ROM and the assets.
    pub fn export(&self, mut write: impl FnMut(&str, &[u8]) -> Result<()>) -> Result<Manifest> {
        let game = self.extractor()?;
        let manifest = self.manifest()?;
        for path in &manifest.assets {
            write(path, &game.extract(path)?)?;
        }
        write("manifest.json", &games::json(&manifest)?)?;
        Ok(manifest)
    }

    fn extractor(&self) -> Result<&dyn Game> {
        self.game()
            .ok_or_else(|| Error::Unsupported(format!("no extractor for {}", self.identity)))
    }

    fn container(&self) -> Result<&Container<'a>> {
        self.container.as_ref().ok_or_else(|| {
            Error::Unsupported(format!("{} has no filesystem", self.identity.platform))
        })
    }
}

/// The archives [`Rom::archive`] lists, told apart by their signature.
enum Archive {
    Narc(Narc),
    Garc(Garc),
}

impl Archive {
    fn parse(file: &impl ByteSource) -> Result<Self> {
        match &*file.read_up_to(0, 4)? {
            b"CRAG" => Ok(Self::Garc(Garc::parse(file)?)),
            _ => Ok(Self::Narc(Narc::parse(file)?)),
        }
    }
}

fn describe(path: String, file: &impl ByteSource) -> Result<FileInfo> {
    Ok(FileInfo {
        path,
        size: file.size(),
        format: Format::sniff(&file.read_up_to(0, 4)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{garc, n3ds, narc, nds};

    #[test]
    fn browses_files_and_archive_members() {
        let archive = narc(&[b"NARCfake", b"two"]);
        let rom = Rom::open(nds(
            b"TSTE",
            &[("data/a.narc", &archive), ("readme.txt", b"hi")],
        ))
        .unwrap();
        assert_eq!(rom.game_id(), None);
        assert!(rom.game().is_none() && rom.export(|_, _| Ok(())).is_err());

        let files = rom.files().unwrap();
        assert_eq!(files[0].path, "data/a.narc");
        assert_eq!(files[0].format.unwrap().extension, "narc");
        assert_eq!((files[1].path.as_str(), files[1].size), ("readme.txt", 2));
        assert_eq!(rom.read_file("readme.txt").unwrap(), b"hi");

        let members = rom.archive("data/a.narc").unwrap();
        assert_eq!(members.len(), 2);
        assert_eq!(members[0].format.unwrap().extension, "narc");
        assert_eq!(rom.read_member("data/a.narc", 1).unwrap(), b"two");
        assert!(rom.archive("readme.txt").is_err());
    }

    #[test]
    fn browses_3ds_files_and_garc_files() {
        // Two entries: a compressed "NARCNARC", and two files of one entry.
        let compressed = [
            0x11,
            8,
            0,
            0,
            0b0000_1000,
            b'N',
            b'A',
            b'R',
            b'C',
            0x30,
            0x03,
        ];
        let archive = garc(&[&[&compressed], &[b"en", b"fr"]]);
        let rom = Rom::open(n3ds(
            "CTR-P-ABCE",
            &[("icon", b"SMDH")],
            &[("a/0/0/1", b"one"), ("a/0/0/2", &archive)],
        ))
        .unwrap();
        assert_eq!(rom.identity().platform, Platform::Nintendo3ds);
        let paths: Vec<_> = rom.files().unwrap().into_iter().map(|f| f.path).collect();
        assert_eq!(paths, ["a/0/0/1", "a/0/0/2", "exefs/icon"]);
        assert_eq!(rom.read_file("a/0/0/1").unwrap(), b"one");

        let files = rom.archive("a/0/0/2").unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(
            (files[0].size, files[0].format.unwrap().extension),
            (8, "narc")
        );
        assert_eq!(rom.read_member("a/0/0/2", 0).unwrap(), b"NARCNARC");
        assert_eq!(rom.read_member("a/0/0/2", 2).unwrap(), b"fr");
    }
}
