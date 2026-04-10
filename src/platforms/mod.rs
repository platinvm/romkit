//! Platforms: what a ROM image is, read from its header.
//!
//! Each handheld has one module that knows its header layout (and, for the DS, its filesystem)
//! but nothing about the games inside. [`identify`] tries each of them in turn.

pub mod gb;
pub mod gba;
pub mod nds;

use std::fmt;

use serde::{Serialize, Serializer};

use crate::io::ByteSource;
use crate::{Error, Result};

/// Every header lies within the first 0x200 bytes.
const HEAD_SIZE: u64 = 0x200;

/// What a ROM is, from its header alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Identity {
    pub platform: Platform,
    /// The title stored in the header, e.g. `POKEMON PL`.
    pub title: String,
    /// The product code, e.g. `CPUE`. Early Game Boy titles have none.
    pub code: Option<GameCode>,
    pub region: Region,
    pub revision: u16,
}

impl fmt::Display for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ROM {:?}", self.platform, self.title)?;
        match self.code {
            Some(code) => write!(f, " ({code})"),
            None => Ok(()),
        }
    }
}

/// Identify a ROM from its header, reading only the first few hundred bytes.
pub fn identify<S: ByteSource + ?Sized>(source: &S) -> Result<Identity> {
    let head = source.read_up_to(0, HEAD_SIZE)?;
    if nds::is_nds(&head) {
        Ok(nds::Header::parse(&head)?.identity())
    } else if gba::is_gba(&head) {
        gba::identify(&head)
    } else if gb::is_gb(&head) {
        gb::identify(&head)
    } else {
        Err(Error::Unsupported(
            "not a Game Boy, GBA, or DS ROM".into(),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    GameBoy,
    /// A Game Boy cartridge that uses Game Boy Color features, alone or alongside DMG support.
    GameBoyColor,
    GameBoyAdvance,
    NintendoDs,
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::GameBoy => "Game Boy",
            Self::GameBoyColor => "Game Boy Color",
            Self::GameBoyAdvance => "Game Boy Advance",
            Self::NintendoDs => "Nintendo DS",
        })
    }
}

/// A four-character product code. The first three characters name the title, the last one
/// its region: `CPUE` is Pokémon Platinum for the USA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GameCode(pub [u8; 4]);

impl GameCode {
    /// The region-independent part of the code, e.g. `CPU`.
    pub fn title(&self) -> &[u8; 3] {
        self.0.first_chunk().expect("four bytes")
    }

    pub fn region(&self) -> Region {
        Region::from_letter(self.0[3])
    }
}

impl fmt::Display for GameCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
            .iter()
            .try_for_each(|&byte| write!(f, "{}", byte as char))
    }
}

impl Serialize for GameCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Region {
    Japan,
    Usa,
    Europe,
    /// One release for both the USA and Europe.
    UsaEurope,
    Germany,
    France,
    Italy,
    Spain,
    Korea,
    China,
    Australia,
    /// Region-free (3DS).
    World,
    /// Anywhere but Japan: all an original Game Boy header says.
    Overseas,
    Unknown(char),
}

impl Region {
    /// Decode the region letter that ends a product code.
    pub fn from_letter(letter: u8) -> Self {
        match letter {
            b'J' => Self::Japan,
            b'E' => Self::Usa,
            b'P' => Self::Europe,
            b'O' => Self::UsaEurope,
            b'D' => Self::Germany,
            b'F' => Self::France,
            b'I' => Self::Italy,
            b'S' => Self::Spain,
            b'K' => Self::Korea,
            b'C' => Self::China,
            b'U' => Self::Australia,
            b'A' => Self::World,
            other => Self::Unknown(other as char),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_bytes_are_unsupported() {
        let error = identify(&[0u8; 0x200][..]).unwrap_err();
        assert!(matches!(error, Error::Unsupported(_)));
        assert!(identify(&[][..]).is_err());
    }

    #[test]
    fn game_codes_split_into_title_and_region() {
        let code = GameCode(*b"CPUE");
        assert_eq!(code.title(), b"CPU");
        assert_eq!(code.region(), Region::Usa);
        assert_eq!(code.to_string(), "CPUE");
    }
}
