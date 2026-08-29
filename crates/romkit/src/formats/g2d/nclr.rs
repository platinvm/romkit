//! NCLR, a palette: BGR555 colors in a `PLTT` chunk (bit depth, two unknown words, data size,
//! data offset), optionally followed by a `PCMP` chunk when only some banks are stored, which
//! lists the bank number of each stored one.

use super::{chunk, find_chunk};
use crate::Result;
use crate::io::Reader;
use crate::media::{Rgba, bgr555};

const FORMAT: &str = "NCLR";

/// Colors indexed as the hardware does: bank `n` of a 4-bit graphic starts at color `16 * n`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub colors: Vec<Rgba>,
}

impl Palette {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Reader::new(FORMAT, bytes).expect(b"RLCN")?;
        let pltt = chunk(FORMAT, bytes, b"TTLP")?;
        let eight_bit = pltt.u16_at(0)? == 4;
        let size = pltt.u32_at(8)? as usize;
        let offset = pltt.u32_at(12)? as usize;
        // The stated size often covers a full 256 colors that the file does not hold.
        let size = size.min(pltt.len().saturating_sub(offset));
        let stored: Vec<Rgba> = pltt
            .slice_at(offset, size & !1)?
            .as_chunks()
            .0
            .iter()
            .map(|&color| bgr555(u16::from_le_bytes(color)))
            .collect();

        let Some(pcmp) = find_chunk(FORMAT, bytes, b"PMCP")? else {
            return Ok(Self { colors: stored });
        };
        let count = pcmp.u16_at(0)? as usize;
        let numbers = pcmp.u32_at(4)? as usize;
        let bank_len = if eight_bit { 256 } else { 16 };
        let mut colors = Vec::new();
        for (index, bank) in stored.chunks(bank_len).take(count).enumerate() {
            let number = pcmp.u16_at(numbers + index * 2)? as usize;
            if number >= 16 {
                return Err(pcmp.error_at(numbers + index * 2, "bank number is out of range"));
            }
            let at = number * bank_len;
            if colors.len() < at + bank.len() {
                colors.resize(at + bank.len(), [0; 4]);
            }
            colors[at..at + bank.len()].copy_from_slice(bank);
        }
        Ok(Self { colors })
    }

    /// The colors of a 4-bit graphic using `bank`; past the end, transparent.
    pub fn bank(&self, bank: u8) -> &[Rgba] {
        let start = (bank as usize * 16).min(self.colors.len());
        &self.colors[start..(start + 16).min(self.colors.len())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::g2d::testing::{nitro, words};

    fn pltt(depth: u16, colors: &[u16]) -> Vec<u8> {
        let mut body = words(&[depth, 0, 0, 0]);
        body.extend((colors.len() as u32 * 2).to_le_bytes());
        body.extend(16u32.to_le_bytes());
        body.extend(words(colors));
        body
    }

    #[test]
    fn reads_colors_and_banks() {
        let mut colors = vec![0; 32];
        colors[17] = 0x001F;
        let bytes = nitro(b"RLCN", &[(b"TTLP", &pltt(3, &colors))]);
        let palette = Palette::parse(&bytes).unwrap();
        assert_eq!(palette.colors.len(), 32);
        assert_eq!(palette.bank(1)[1], [255, 0, 0, 255]);
        assert!(palette.bank(9).is_empty());
    }

    #[test]
    fn places_stored_banks_at_their_numbers() {
        let mut colors = vec![0; 16];
        colors[2] = 0x7C00;
        let mut pcmp = words(&[1, 0xBEEF]);
        pcmp.extend(8u32.to_le_bytes());
        pcmp.extend(words(&[3]));
        let bytes = nitro(b"RLCN", &[(b"TTLP", &pltt(3, &colors)), (b"PMCP", &pcmp)]);
        let palette = Palette::parse(&bytes).unwrap();
        assert_eq!(palette.colors.len(), 64);
        assert_eq!(palette.bank(3)[2], [0, 0, 255, 255]);
        assert_eq!(palette.bank(0)[2], [0; 4]);
    }

    #[test]
    fn rejects_other_files() {
        assert!(Palette::parse(b"RGCN\xFF\xFE\x00\x01\x10\0\0\0\x10\0\0\0").is_err());
        assert!(Palette::parse(&nitro(b"RLCN", &[])).is_err());
    }
}
