//! BCSTM, the 3DS audio stream: background music, decoded to PCM.
//!
//! Layout: a header (`CSTM`) referencing three blocks. `INFO` holds the stream info (encoding,
//! loop, channel count, sample rate, loop start, length, and how the data is cut into blocks)
//! and one info record per channel, which for DSP-ADPCM holds the decoder's coefficients.
//! `SEEK` holds decoder states for seeking, unused here. `DATA` holds the samples in blocks of
//! a fixed size, each block holding every channel in turn; the last block is shorter.
//!
//! DSP-ADPCM, the GameCube's codec, packs 14 samples in 8-byte frames: a header byte selects a
//! pair of coefficients (high nibble) and a scale (low nibble), then 14 signed 4-bit deltas,
//! each predicted from the two previous samples.

use crate::io::Reader;
use crate::media::Audio;
use crate::{Error, Result};

const FORMAT: &str = "BCSTM";
const INFO: u16 = 0x4000;
const DATA: u16 = 0x4002;
const MAX_CHANNELS: usize = 16;

/// Decode a whole stream.
pub fn decode(bytes: &[u8]) -> Result<Audio> {
    let file = Reader::new(FORMAT, bytes);
    let mut header = file.clone();
    header.expect(b"CSTM")?;
    let block = |kind: u16| -> Result<usize> {
        (0..file.u16_at(0x10)? as usize)
            .map(|index| 0x14 + index * 12)
            .find(|&at| file.u16_at(at).is_ok_and(|found| found == kind))
            .map(|at| file.u32_at(at + 4).map(|offset| offset as usize))
            .ok_or_else(|| file.error_at(0x10, "missing block"))?
    };
    let (info, data) = (block(INFO)?, block(DATA)?);
    // References inside INFO count from just after its 8-byte header.
    let base = info + 8;
    let stream = base + file.u32_at(base + 4)? as usize;
    let encoding = file.u8_at(stream)?;
    let looping = file.u8_at(stream + 1)? != 0;
    let channels = file.u8_at(stream + 2)? as usize;
    let sample_rate = file.u32_at(stream + 4)?;
    let loop_start = file.u32_at(stream + 8)?;
    let frames = file.u32_at(stream + 0x0C)? as usize;
    let blocks = file.u32_at(stream + 0x10)? as usize;
    let block_len = file.u32_at(stream + 0x14)? as usize;
    let last_block_len = file.u32_at(stream + 0x24)? as usize;
    let samples_at = data + 8 + file.u32_at(stream + 0x34)? as usize;
    if channels == 0 || channels > MAX_CHANNELS || blocks == 0 {
        return Err(file.error_at(stream, "channel or block count is out of range"));
    }

    // Gather each channel's bytes from the interleaved blocks.
    let stride = block_len.checked_mul(channels);
    let channel_data = (0..channels)
        .map(|channel| -> Result<Vec<u8>> {
            let mut out = Vec::new();
            for index in 0..blocks {
                let len = if index + 1 == blocks {
                    last_block_len
                } else {
                    block_len
                };
                let at = stride
                    .and_then(|stride| stride.checked_mul(index))
                    .and_then(|at| at.checked_add(samples_at + channel * len))
                    .ok_or_else(|| file.error_at(stream, "blocks are out of range"))?;
                out.extend(file.slice_at(at, len)?);
            }
            Ok(out)
        })
        .collect::<Result<Vec<_>>>()?;

    let decoded = match encoding {
        0 => channel_data
            .iter()
            .map(|bytes| bytes.iter().map(|&byte| (byte as i8 as i16) << 8).collect())
            .collect::<Vec<Vec<i16>>>(),
        1 => channel_data
            .iter()
            .map(|bytes| {
                bytes
                    .as_chunks()
                    .0
                    .iter()
                    .map(|&pair| i16::from_le_bytes(pair))
                    .collect()
            })
            .collect(),
        2 => {
            // The channel info table: a count, then references from the table's start, each to
            // a channel record whose own reference leads to its DSP-ADPCM info.
            let table = base + file.u32_at(base + 0x14)? as usize;
            if file.u32_at(table)? as usize != channels {
                return Err(file.error_at(table, "channel count disagrees"));
            }
            channel_data
                .iter()
                .enumerate()
                .map(|(channel, bytes)| {
                    let record = table + file.u32_at(table + 4 + channel * 8 + 4)? as usize;
                    let adpcm = record + file.u32_at(record + 4)? as usize;
                    let coefficients: Vec<i16> = file
                        .slice_at(adpcm, 32)?
                        .as_chunks()
                        .0
                        .iter()
                        .map(|&pair| i16::from_le_bytes(pair))
                        .collect();
                    let history = [
                        file.u16_at(adpcm + 0x22)? as i16,
                        file.u16_at(adpcm + 0x24)? as i16,
                    ];
                    Ok(dsp_adpcm(bytes, &coefficients, history, frames))
                })
                .collect::<Result<_>>()?
        }
        _ => return Err(Error::Unsupported(format!("BCSTM encoding {encoding}"))),
    };

    // Interleave, up to the stated length.
    let frames = decoded.iter().map(Vec::len).min().unwrap_or(0).min(frames);
    let mut samples = Vec::with_capacity(frames * channels);
    for frame in 0..frames {
        samples.extend(decoded.iter().map(|channel| channel[frame]));
    }
    Ok(Audio {
        sample_rate,
        channels: channels as u16,
        samples,
        loop_start: (looping && (loop_start as usize) < frames).then_some(loop_start),
    })
}

/// Decode up to `limit` samples of DSP-ADPCM frames.
fn dsp_adpcm(bytes: &[u8], coefficients: &[i16], mut history: [i16; 2], limit: usize) -> Vec<i16> {
    let mut out = Vec::with_capacity(limit.min(bytes.len() / 8 * 14));
    for frame in bytes.chunks(8) {
        let [header, deltas @ ..] = frame else { break };
        let pair = (header >> 4) as usize * 2;
        let (first, second) = (
            *coefficients.get(pair).unwrap_or(&0) as i32,
            *coefficients.get(pair + 1).unwrap_or(&0) as i32,
        );
        let scale = 1i32 << (header & 0xF);
        for &byte in deltas {
            for nibble in [byte >> 4, byte & 0xF] {
                if out.len() == limit {
                    return out;
                }
                let delta = ((nibble as i8) << 4 >> 4) as i32;
                let predicted = first * history[0] as i32 + second * history[1] as i32;
                let sample = (((delta * scale) << 11) + 1024 + predicted) >> 11;
                let sample = sample.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                history = [sample, history[0]];
                out.push(sample);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsp_adpcm_predicts_from_the_previous_samples() {
        // Coefficient pair 1 is (2048, 0): each sample repeats the previous one, plus its delta
        // times the scale (2 here).
        let mut coefficients = [0i16; 16];
        coefficients[2] = 2048;
        let frame = [0x11, 0x1F, 0, 0, 0, 0, 0, 0];
        let samples = dsp_adpcm(&frame, &coefficients, [100, 0], 4);
        assert_eq!(samples, [102, 100, 100, 100]);
        assert_eq!(dsp_adpcm(&frame, &coefficients, [0, 0], 100).len(), 14);
    }

    /// A two-channel PCM16 stream of three frames in blocks of two.
    fn stream() -> Vec<u8> {
        let mut bytes = vec![0; 0x100];
        bytes[..4].copy_from_slice(b"CSTM");
        bytes[0x10] = 2;
        let mut put32 =
            |at: usize, value: u32| bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        // INFO at 0x40, DATA at 0xC0.
        put32(0x14, INFO as u32);
        put32(0x18, 0x40);
        put32(0x20, DATA as u32);
        put32(0x24, 0xC0);
        put32(0x48 + 4, 0x18); // stream info at INFO + 8 + 0x18
        let stream = 0x60;
        put32(stream, 0x0002_0101); // PCM16, looping, two channels
        put32(stream + 4, 32000);
        put32(stream + 8, 1); // loop start
        put32(stream + 0x0C, 3);
        put32(stream + 0x10, 2); // blocks
        put32(stream + 0x14, 4); // block size: two samples
        put32(stream + 0x24, 4); // last block, padded
        put32(stream + 0x34, 0x18); // samples at DATA + 8 + 0x18
        // Block 0: left 1 2, right -1 -2; block 1: left 3 (pad), right -3 (pad).
        let samples: [i16; 8] = [1, 2, -1, -2, 3, 0, -3, 0];
        for (index, sample) in samples.iter().enumerate() {
            let at = 0xC0 + 0x20 + index * 2;
            bytes[at..at + 2].copy_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn interleaves_channels_from_blocks() {
        let audio = decode(&stream()).unwrap();
        assert_eq!((audio.channels, audio.sample_rate), (2, 32000));
        assert_eq!(audio.samples, [1, -1, 2, -2, 3, -3]);
        assert_eq!(audio.loop_start, Some(1));
    }

    #[test]
    fn rejects_streams_past_the_end() {
        let mut bytes = stream();
        bytes[0x60 + 0x10] = 200; // blocks
        assert!(decode(&bytes).is_err());
        assert!(decode(&bytes[..0x80]).is_err());
    }
}
