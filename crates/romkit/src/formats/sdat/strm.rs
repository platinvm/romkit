//! STRM, the stream: prerecorded audio, such as voices or songs that are not sequenced.
//!
//! Layout: a 16-byte header, then a `HEAD` chunk:
//!
//! ```text
//! 0x18  u8   encoding: 0 PCM8, 1 PCM16, 2 IMA-ADPCM
//! 0x19  u8   loops
//! 0x1A  u8   channels
//! 0x1C  u16  sample rate
//! 0x1E  u16  hardware timer
//! 0x20  u32  loop start, in samples
//! 0x24  u32  samples per channel
//! 0x28  u32  offset of the sample data in the file
//! 0x2C  u32  blocks
//! 0x30  u32  bytes per channel in a block
//! 0x34  u32  samples per channel in a block
//! 0x38  u32  bytes per channel in the last block
//! 0x3C  u32  samples per channel in the last block
//! ```
//!
//! The data interleaves channels block by block: block 0 of each channel, then block 1, and so
//! on, so that the game can stream one block at a time. Each IMA-ADPCM block starts afresh with
//! its own header.

use super::wave;
use crate::Result;
use crate::io::Reader;
use crate::media::Audio;

/// Decode a whole stream.
pub fn decode(strm: &[u8]) -> Result<Audio> {
    let mut reader = Reader::new("STRM", strm);
    reader.expect(b"STRM")?;
    reader.seek(0x10)?;
    reader.expect(b"HEAD")?;
    let encoding = reader.u8_at(0x18)?;
    let channels = reader.u8_at(0x1A)? as usize;
    if !(1..=2).contains(&channels) {
        return Err(reader.error_at(0x1A, "streams have one or two channels"));
    }
    let field = |at| reader.u32_at(at).map(|value| value as usize);
    let (length, data_at, blocks) = (field(0x24)?, field(0x28)?, field(0x2C)?);
    let (block_len, block_samples) = (field(0x30)?, field(0x34)?);
    let (last_len, last_samples) = (field(0x38)?, field(0x3C)?);
    let data = reader.slice_at(data_at, strm.len().saturating_sub(data_at))?;
    let full_blocks = blocks.saturating_sub(1);
    let full_len = block_len.saturating_mul(channels);
    if blocks > 0
        && (block_len == 0 && full_blocks > 0 || full_blocks > data.len() / full_len.max(1))
    {
        return Err(reader.error_at(0x2C, "blocks overrun the data"));
    }

    let data = Reader::new("STRM", data);
    let mut decoded = vec![Vec::new(); channels];
    for block in 0..blocks {
        let (len, samples) = match block == full_blocks {
            true => (last_len, last_samples),
            false => (block_len, block_samples),
        };
        for (channel, decoded) in decoded.iter_mut().enumerate() {
            let bytes = data.slice_at(block * full_len + channel * len.min(data.len()), len)?;
            let samples = wave::decode("STRM", encoding, bytes)?
                .into_iter()
                .take(samples);
            decoded.extend(samples);
        }
    }
    let frames = decoded.iter().map(Vec::len).min().unwrap_or(0).min(length);
    Ok(Audio {
        sample_rate: wave::sample_rate(reader.u16_at(0x1C)?, reader.u16_at(0x1E)?),
        channels: channels as u16,
        samples: (0..frames)
            .flat_map(|frame| decoded.iter().map(move |channel| channel[frame]))
            .collect(),
        loop_start: (reader.u8_at(0x19)? != 0).then_some(field(0x20)? as u32),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::sdat::wave::{IMA_ADPCM, PCM16, tests::adpcm};

    /// A stream of `channels`, each given as its blocks of encoded data.
    fn strm(
        encoding: u8,
        loop_start: Option<u32>,
        channels: &[Vec<Vec<u8>>],
        samples: &[u32],
    ) -> Vec<u8> {
        let blocks = channels[0].len();
        let block_len = channels[0][0].len() as u32;
        let last_len = channels[0][blocks - 1].len() as u32;
        let mut bytes = b"STRM\xFF\xFE\x00\x01\0\0\0\0\x10\0\x02\0HEAD\x50\0\0\0".to_vec();
        bytes.extend([
            encoding,
            loop_start.is_some() as u8,
            channels.len() as u8,
            0,
        ]);
        bytes.extend([22050u16, 760].map(u16::to_le_bytes).concat());
        let total = samples.iter().sum::<u32>();
        let fields = [
            loop_start.unwrap_or(0),
            total,
            0x68,
            blocks as u32,
            block_len,
            samples[0],
            last_len,
            samples[blocks - 1],
        ];
        bytes.extend(fields.map(u32::to_le_bytes).concat());
        bytes.resize(0x60, 0);
        bytes.extend(b"DATA\0\0\0\0");
        for block in 0..blocks {
            for channel in channels {
                bytes.extend(&channel[block]);
            }
        }
        bytes
    }

    #[test]
    fn interleaves_stereo_blocks() {
        let pcm = |samples: &[i16]| samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let left = vec![pcm(&[1, 2, 3, 4]), pcm(&[5, 6])];
        let right = vec![pcm(&[2, 4, 6, 8]), pcm(&[10, 12])];
        let audio = decode(&strm(PCM16, Some(1), &[left, right], &[4, 2])).unwrap();
        assert_eq!(
            (audio.channels, audio.sample_rate, audio.loop_start),
            (2, 22050, Some(1))
        );
        assert_eq!(audio.samples, [1, 2, 2, 4, 3, 6, 4, 8, 5, 10, 6, 12]);
    }

    #[test]
    fn restarts_adpcm_every_block() {
        let ramp: Vec<i16> = (0..64).map(|i| i * 100).collect();
        let blocks = vec![adpcm(&ramp[..32]), adpcm(&ramp[32..])];
        let audio = decode(&strm(IMA_ADPCM, None, &[blocks], &[32, 32])).unwrap();
        assert_eq!(
            (audio.channels, audio.samples.len(), audio.loop_start),
            (1, 64, None)
        );
        assert!(
            (i32::from(audio.samples[33]) - 3300).abs() < 400,
            "{}",
            audio.samples[33]
        );
    }

    #[test]
    fn rejects_blocks_past_the_data() {
        let mut bytes = strm(PCM16, None, &[vec![vec![0; 8], vec![0; 8]]], &[4, 4]);
        bytes[0x2C..0x30].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&bytes).is_err());
        bytes[0x2C..0x30].copy_from_slice(&3u32.to_le_bytes());
        assert!(decode(&bytes).is_err());
    }
}
