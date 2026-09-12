//! Cries: 8-bit samples played by the sound engine's DirectSound channels. The cry table holds
//! one 12-byte voice per cry, which points to the sample's 16-byte header: its kind (0 for
//! plain samples, 1 for compressed), loop flag, rate (Hz × 1024), loop start, and length in
//! samples.
//!
//! Cries of species up to Celebi follow the Pokédex; Hoenn species have a table of their own,
//! as their internal order is not the Pokédex's.
//!
//! Compressed samples are delta-coded in blocks of 64: a starting sample, then 4-bit codes
//! that each add a step from a fixed table, the first in the low half of its own byte, the
//! others two to a byte, high half first. A block is 33 bytes.

use super::{FORMAT, Gen3, SpeciesId};
use crate::io::ByteSource;
use crate::media::Audio;
use crate::{Error, Result};

const CRIES: usize = 388;
const VOICE_LEN: u32 = 12;
const HEADER_LEN: u32 = 16;
/// The last species whose cry is at its Pokédex position, and the first of Hoenn's table.
const CELEBI: usize = 251;
const TREECKO: usize = 277;
const COMPRESSED: u16 = 1;
const LOOPS: u16 = 0x4000;
/// Cries last about a second; this is ten.
const MAX_SAMPLES: usize = 0x20000;
const BLOCK_SAMPLES: usize = 64;
const BLOCK_LEN: usize = 33;
const STEPS: [i8; 16] = [
    0, 1, 4, 9, 16, 25, 36, 49, -64, -49, -36, -25, -16, -9, -4, -1,
];

impl<S: ByteSource> Gen3<S> {
    /// A species' cry.
    pub fn cry(&self, species: SpeciesId) -> Result<Audio> {
        let index = self.internal_index(species)?;
        let cry = match index {
            ..=CELEBI => index - 1,
            _ => {
                let at = self.edition.hoenn_cries + 2 * index.saturating_sub(TREECKO) as u32;
                let cry = self.read(at, 2)?;
                u16::from_le_bytes([cry[0], cry[1]]) as usize
            }
        };
        if cry >= CRIES {
            return Err(Error::malformed(FORMAT, cry as u64, "no such cry"));
        }
        let wave = self.follow(self.edition.cries + VOICE_LEN * cry as u32 + 4)?;
        let header = self.read(wave, HEADER_LEN as usize)?;
        let word = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().expect("4"));
        let (kind, flags) = (word(0) as u16, (word(0) >> 16) as u16);
        let len = word(12) as usize;
        if len > MAX_SAMPLES {
            return Err(Error::malformed(FORMAT, wave.into(), "sample too long"));
        }
        let data = wave + HEADER_LEN;
        let samples = match kind {
            COMPRESSED => {
                let blocks = len.div_ceil(BLOCK_SAMPLES) * BLOCK_LEN;
                decompress(&self.source.read_up_to(data.into(), blocks as u64)?, len)
                    .ok_or_else(|| Error::malformed(FORMAT, data.into(), "truncated sample"))?
            }
            _ => self.read(data, len)?.iter().map(|&s| s as i8).collect(),
        };
        Ok(Audio {
            sample_rate: word(4) / 1024,
            channels: 1,
            samples: samples.into_iter().map(|s| i16::from(s) << 8).collect(),
            loop_start: (flags & LOOPS != 0).then(|| word(8)),
        })
    }
}

/// `len` delta-coded samples, or `None` when the bytes run out first.
fn decompress(bytes: &[u8], len: usize) -> Option<Vec<i8>> {
    let mut samples = Vec::with_capacity(len);
    for block in bytes.chunks(BLOCK_LEN) {
        let (&start, codes) = block.split_first()?;
        let mut sample = start as i8;
        samples.push(sample);
        let first = codes.first().map(|&byte| byte & 0xF);
        let rest = codes
            .iter()
            .skip(1)
            .flat_map(|&byte| [byte >> 4, byte & 0xF]);
        for code in first.into_iter().chain(rest) {
            if samples.len() >= len {
                break;
            }
            sample = sample.wrapping_add(STEPS[code as usize]);
            samples.push(sample);
        }
        if samples.len() >= len {
            samples.truncate(len);
            return Some(samples);
        }
    }
    (samples.len() >= len).then_some(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompresses_delta_blocks() {
        // A block starting at 10: +1 (low half of the first byte), then +4 and -1 (high half
        // first), then zeros; then a second block starting at -5.
        let mut bytes = vec![10, 0x01, 0x2F];
        bytes.resize(BLOCK_LEN, 0);
        bytes.extend([0xFB, 0x08]);
        let samples = decompress(&bytes, 66).unwrap();
        assert_eq!(samples[..4], [10, 11, 15, 14]);
        assert_eq!(samples[63], 14);
        assert_eq!(samples[64..], [-5, -69]);
        assert_eq!(decompress(&bytes, 70), None);
        assert_eq!(decompress(&bytes[..2], 2).unwrap(), [10, 11]);
    }
}
