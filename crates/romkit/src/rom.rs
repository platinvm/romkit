//! [`Rom`]: any ROM, opened for browsing and extraction. The entry point for hosts that do not
//! know in advance what they will be given, such as the website.

use std::rc::Rc;

use serde::Serialize;

use crate::formats::Format;
use crate::formats::narc::Narc;
use crate::games::{self, Game, GameId};
use crate::io::ByteSource;
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
    nds: Option<NdsRom<Shared<'a>>>,
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
        let nds = match identity.platform {
            Platform::NintendoDs => Some(NdsRom::open(Rc::clone(&source))?),
            _ => None,
        };
        Ok(Self {
            source,
            identity,
            game_id,
            game,
            nds,
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
            banner: self.nds.as_ref().map(NdsRom::banner).transpose()?.flatten(),
        })
    }

    /// Every named file, with its size and format. Only DS ROMs have a filesystem; other ROMs
    /// list nothing.
    pub fn files(&self) -> Result<Vec<FileInfo>> {
        let Some(nds) = &self.nds else {
            return Ok(Vec::new());
        };
        nds.fs()
            .paths()
            .map(|(path, _)| describe(path.to_owned(), &nds.file(path)?))
            .collect()
    }

    pub fn read_file(&self, path: &str) -> Result<Vec<u8>> {
        Ok(self.nds()?.read(path)?.into_owned())
    }

    /// The members of the archive at `path`.
    pub fn archive(&self, path: &str) -> Result<Vec<FileInfo>> {
        let file = self.nds()?.file(path)?;
        let narc = Narc::parse(&file)?;
        (0..narc.len())
            .map(|index| describe(index.to_string(), &narc.member_source(&file, index)?))
            .collect()
    }

    pub fn read_member(&self, path: &str, index: usize) -> Result<Vec<u8>> {
        let file = self.nds()?.file(path)?;
        Ok(Narc::parse(&file)?.member(&file, index)?.into_owned())
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

    /// Extract every asset through `write(path, contents)`, then write a `manifest.json`
    /// describing the ROM and the assets.
    pub fn export(&self, mut write: impl FnMut(&str, &[u8]) -> Result<()>) -> Result<Manifest> {
        let game = self
            .game()
            .ok_or_else(|| Error::Unsupported(format!("no extractor for {}", self.identity)))?;
        let assets = game.assets()?;
        for path in &assets {
            write(path, &game.extract(path)?)?;
        }
        let manifest = Manifest {
            romkit: env!("CARGO_PKG_VERSION"),
            game: game.id(),
            identity: self.identity.clone(),
            crc32: format!("{:08x}", self.crc32()?),
            assets,
        };
        write("manifest.json", &games::json(&manifest)?)?;
        Ok(manifest)
    }

    fn nds(&self) -> Result<&NdsRom<Shared<'a>>> {
        self.nds.as_ref().ok_or_else(|| {
            Error::Unsupported(format!("{} has no filesystem", self.identity.platform))
        })
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
    use crate::testing::{narc, nds};

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
}
