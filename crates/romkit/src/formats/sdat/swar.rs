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
use crate::io::{ByteSource, Reader};
use crate::media::Audio;
use crate::{Error, Result};

/// The wave count, followed by the offset of each wave.
const TABLE_AT: u64 = 0x38;
const WAVE_HEADER_LEN: u64 = 12;

/// The number of waves in the archive. Reads only the header.
pub fn len<S: ByteSource + ?Sized>(swar: &S) -> Result<usize> {
    let head = swar.read_up_to(0, TABLE_AT + 4)?;
    let mut reader = Reader::new("SWAR", &head);
    reader.expect(b"SWAR")?;
    let count = reader.u32_at(TABLE_AT as usize)? as u64;
    match count <= swar.size().saturating_sub(TABLE_AT + 4) / 4 {
        true => Ok(count as usize),
        false => Err(reader.error_at(TABLE_AT as usize, "wave table overruns the file")),
    }
}

/// Decode wave `index` of the archive, reading only that wave.
pub fn wave<S: ByteSource + ?Sized>(swar: &S, index: usize) -> Result<Audio> {
    let count = len(swar)?;
    if index >= count {
        return Err(Error::NotFound(format!("wave {index} of {count}")));
    }
    let entry = TABLE_AT + 4 + 4 * index as u64;
    let at = Reader::new("SWAR", &swar.read(entry, 4)?).u32()? as u64;
    let outside = |_| Error::malformed("SWAR", at, "wave lies outside the file");
    let header = swar.read(at, WAVE_HEADER_LEN).map_err(outside)?;
    let header = Reader::new("SWAR", &header);
    let encoding = header.u8_at(0)?;
    let loop_start = u64::from(header.u16_at(6)?) * 4;
    let len = loop_start + u64::from(header.u32_at(8)?) * 4;
    let data = swar.read(at + WAVE_HEADER_LEN, len).map_err(outside)?;
    Ok(Audio {
        sample_rate: wave::sample_rate(header.u16_at(2)?, header.u16_at(4)?),
        channels: 1,
        samples: wave::decode("SWAV", encoding, &data)?,
        loop_start: (header.u8_at(1)? != 0)
            .then(|| wave::sample_count(encoding, loop_start as usize) as u32),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::sdat::wave::{IMA_ADPCM, PCM8, tests::adpcm};
    use crate::testing::{Counting, swar};

    #[test]
    fn decodes_waves_with_loops() {
        let pcm = [0x40, 0xC0, 0, 0, 1, 2, 3, 4];
        let sine: Vec<i16> = (0..56).map(|i| (i * 300) as i16).collect();
        let bytes = swar(&[(PCM8, None, &pcm), (IMA_ADPCM, Some(2), &adpcm(&sine))]);
        assert_eq!(len(bytes.as_slice()).unwrap(), 2);

        let first = wave(bytes.as_slice(), 0).unwrap();
        assert_eq!((first.sample_rate, first.loop_start), (8000, None));
        assert_eq!(&first.samples[..2], [0x4000, -0x4000]);

        let second = wave(bytes.as_slice(), 1).unwrap();
        assert_eq!(second.samples.len(), 56);
        assert_eq!(
            second.loop_start,
            Some(8),
            "two words: the header, then 8 samples"
        );
        assert!(matches!(wave(bytes.as_slice(), 2), Err(Error::NotFound(_))));
    }

    #[test]
    fn rejects_waves_past_the_end() {
        let mut bytes = swar(&[(PCM8, None, &[0; 8])]);
        bytes.pop();
        assert!(wave(bytes.as_slice(), 0).is_err());
        bytes[0x38..0x3C].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(
            len(bytes.as_slice()).is_err(),
            "more waves than the file holds"
        );
    }

    #[test]
    fn reads_only_the_wave_asked_for() {
        let big = vec![0; 100_000];
        let bytes = swar(&[(PCM8, None, &big), (PCM8, None, &[1, 2, 3, 4])]);
        let source = Counting::new(bytes.as_slice());
        assert_eq!(wave(&source, 1).unwrap().samples.len(), 4);
        assert!(
            source.bytes_read() < 200,
            "read {} bytes",
            source.bytes_read()
        );
    }
}
