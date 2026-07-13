//! SWAR, the wave archive: the samples that instruments and sound effects play.
//!
//! Layout: a 16-byte header, then a `DATA` chunk: its 8-byte header, 32 reserved bytes, a count,
//! and the offset of each wave in the file. A wave (an SWAV without its file header) is a
//! 12-byte header followed by its samples:
//!
//! ```text
//! 0x0  u8   encoding: 0 PCM8, 1 PCM16, 2 IMA-ADPCM
//! 0x1  u8   loops
//! 0x2  u16  sample rate
//! 0x4  u16  hardware timer, the same rate as the DS clock divider
//! 0x6  u16  loop start, in 4-byte words
//! 0x8  u32  length after the loop start, in 4-byte words
//! ```

use super::wave;
use crate::io::Reader;
use crate::media::Audio;
use crate::{Error, Result};

const COUNT_AT: usize = 0x38;
/// How many bytes [`len`] needs.
pub const HEADER_LEN: u64 = COUNT_AT as u64 + 4;

/// The number of waves in the archive, from its first [`HEADER_LEN`] bytes.
pub fn len(head: &[u8]) -> Result<usize> {
    let mut reader = Reader::new("SWAR", head);
    reader.expect(b"SWAR")?;
    Ok(reader.u32_at(COUNT_AT)? as usize)
}

/// Decode wave `index` of the archive.
pub fn wave(swar: &[u8], index: usize) -> Result<Audio> {
    let count = len(swar)?;
    if index >= count {
        return Err(Error::NotFound(format!("wave {index} of {count}")));
    }
    let reader = Reader::new("SWAR", swar);
    let at = reader.u32_at(index.saturating_mul(4).saturating_add(COUNT_AT + 4))? as usize;
    let encoding = reader.u8_at(at)?;
    let rate = wave::sample_rate(reader.u16_at(at + 2)?, reader.u16_at(at + 4)?);
    let loop_start = reader.u16_at(at + 6)? as usize * 4;
    let size = (reader.u32_at(at + 8)? as usize).saturating_mul(4);
    let data = reader.slice_at(at + 12, size.saturating_add(loop_start))?;
    let loops = reader.u8_at(at + 1)? != 0;
    Ok(Audio {
        sample_rate: rate,
        channels: 1,
        samples: wave::decode("SWAV", encoding, data)?,
        loop_start: loops.then(|| wave::sample_count(encoding, loop_start) as u32),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::formats::sdat::wave::{IMA_ADPCM, PCM8, tests::adpcm};

    /// A SWAR holding waves given as `(encoding, loop start in words, data)`.
    pub fn swar(waves: &[(u8, Option<u16>, &[u8])]) -> Vec<u8> {
        let mut bytes = b"SWAR\xFF\xFE\x00\x01\0\0\0\0\x10\0\x01\0DATA".to_vec();
        bytes.resize(COUNT_AT, 0);
        bytes.extend((waves.len() as u32).to_le_bytes());
        let mut at = bytes.len() + 4 * waves.len();
        for (_, _, data) in waves {
            bytes.extend((at as u32).to_le_bytes());
            at += 12 + data.len();
        }
        for &(encoding, loop_start, data) in waves {
            let start = loop_start.unwrap_or(0);
            bytes.extend([encoding, loop_start.is_some() as u8]);
            bytes.extend([8000u16, 2095, start].map(u16::to_le_bytes).concat());
            bytes.extend((data.len() as u32 / 4 - u32::from(start)).to_le_bytes());
            bytes.extend(data);
        }
        bytes
    }

    #[test]
    fn decodes_waves_with_loops() {
        let pcm = [0x40, 0xC0, 0, 0, 1, 2, 3, 4];
        let sine: Vec<i16> = (0..56).map(|i| (i * 300) as i16).collect();
        let bytes = swar(&[(PCM8, None, &pcm), (IMA_ADPCM, Some(2), &adpcm(&sine))]);
        assert_eq!(len(&bytes).unwrap(), 2);

        let first = wave(&bytes, 0).unwrap();
        assert_eq!((first.sample_rate, first.loop_start), (8000, None));
        assert_eq!(&first.samples[..2], [0x4000, -0x4000]);

        let second = wave(&bytes, 1).unwrap();
        assert_eq!(second.samples.len(), 56);
        assert_eq!(
            second.loop_start,
            Some(8),
            "two words: the header, then 8 samples"
        );
        assert!(matches!(wave(&bytes, 2), Err(Error::NotFound(_))));
    }

    #[test]
    fn rejects_waves_past_the_end() {
        let mut bytes = swar(&[(PCM8, None, &[0; 8])]);
        let len = bytes.len();
        bytes.truncate(len - 1);
        assert!(wave(&bytes, 0).is_err());
    }
}
