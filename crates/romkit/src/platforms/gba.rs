//! Game Boy Advance cartridge headers.

use super::{GameCode, Identity, Platform};
use crate::Result;
use crate::io::{Reader, ascii};

/// The first bytes of the Nintendo logo every cartridge carries at 0x04.
const LOGO: [u8; 4] = [0x24, 0xFF, 0xAE, 0x51];
/// The value every header holds at 0xB2.
const FIXED: u8 = 0x96;
const HEADER_END: usize = 0xC0;

pub fn is_gba(head: &[u8]) -> bool {
    head.len() >= HEADER_END && head[0x04..0x08] == LOGO && head[0xB2] == FIXED
}

pub fn identify(head: &[u8]) -> Result<Identity> {
    let header = Reader::new("GBA header", head);
    let code = GameCode(header.array_at(0xAC)?);
    Ok(Identity {
        platform: Platform::GameBoyAdvance,
        title: ascii(header.slice_at(0xA0, 12)?),
        code: Some(code),
        region: code.region(),
        revision: header.u8_at(0xBC)?.into(),
    })
}
