//! Game Boy and Game Boy Color cartridge headers.

use super::{GameCode, Identity, Platform, Region};
use crate::Result;
use crate::io::{Reader, ascii};

/// The first bytes of the Nintendo logo every cartridge carries at 0x104.
const LOGO: [u8; 4] = [0xCE, 0xED, 0x66, 0x66];
const HEADER_END: usize = 0x150;
/// Set in the CGB flag (0x143) by cartridges that use Game Boy Color features.
const CGB: u8 = 0x80;

pub fn is_gb(head: &[u8]) -> bool {
    head.len() >= HEADER_END && head[0x104..0x108] == LOGO
}

pub fn identify(head: &[u8]) -> Result<Identity> {
    let header = Reader::new("Game Boy header", head);
    let color = header.u8_at(0x143)? & CGB != 0;
    // Color-aware headers shorten the title to 15 bytes; later ones also end it with a
    // four-character manufacturer code.
    let code = header.array_at::<4>(0x13F)?;
    let (title_len, code) = match color {
        false => (16, None),
        true if code.iter().all(u8::is_ascii_alphanumeric) => (11, Some(GameCode(code))),
        true => (15, None),
    };
    Ok(Identity {
        platform: if color {
            Platform::GameBoyColor
        } else {
            Platform::GameBoy
        },
        title: ascii(header.slice_at(0x134, title_len)?),
        code,
        region: match header.u8_at(0x14A)? {
            0 => Region::Japan,
            _ => Region::Overseas,
        },
        revision: header.u8_at(0x14C)?.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(title_area: &[u8; 15], cgb_flag: u8) -> Vec<u8> {
        let mut bytes = vec![0; HEADER_END];
        bytes[0x104..0x108].copy_from_slice(&LOGO);
        bytes[0x134..0x143].copy_from_slice(title_area);
        bytes[0x143] = cgb_flag;
        bytes[0x14A] = 1;
        bytes
    }

    #[test]
    fn color_titles_split_off_their_manufacturer_code() {
        let gold = identify(&header(b"POKEMON_GLDAAUE", 0x80)).unwrap();
        assert_eq!(gold.platform, Platform::GameBoyColor);
        assert_eq!(gold.title, "POKEMON_GLD");
        assert_eq!(gold.code, Some(GameCode(*b"AAUE")));

        let yellow = identify(&header(b"POKEMON YELLOW\0", 0x80)).unwrap();
        assert_eq!(yellow.title, "POKEMON YELLOW");
        assert_eq!(yellow.code, None);
    }

    #[test]
    fn original_titles_use_all_sixteen_bytes() {
        let red = identify(&header(b"POKEMON RED\0\0\0\0", 0)).unwrap();
        assert_eq!(
            (red.platform, red.title.as_str()),
            (Platform::GameBoy, "POKEMON RED")
        );
        assert_eq!(red.region, Region::Overseas);
    }
}
