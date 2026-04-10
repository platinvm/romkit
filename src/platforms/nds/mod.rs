//! Nintendo DS cartridges.

use std::ops::Range;

use super::{GameCode, Identity, Platform};
use crate::Result;
use crate::io::{Reader, ascii};

/// The header holds a CRC of the Nintendo logo at 0x15C, the same on every cartridge.
const LOGO_CRC: u16 = 0xCF56;
/// The header fields romkit reads all lie below this offset.
const HEADER_LEN: usize = 0x160;

pub fn is_nds(head: &[u8]) -> bool {
    head.get(0x15C..0x15E) == Some(&LOGO_CRC.to_le_bytes())
}

/// The header fields romkit uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub title: String,
    pub code: GameCode,
    pub revision: u8,
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
