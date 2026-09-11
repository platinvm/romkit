//! The three sample encodings of the DS sound hardware, shared by waves and streams.

use crate::io::Reader;
use crate::{Error, Result};

pub(super) const PCM8: u8 = 0;
pub(super) const PCM16: u8 = 1;
pub(super) const IMA_ADPCM: u8 = 2;

/// The DS sound clock: a sample rate is this divided by the hardware timer value.
const CLOCK: u32 = 16_756_991;

/// The sample rate, from its own field or else from the hardware timer.
pub(super) fn sample_rate(rate: u16, timer: u16) -> u32 {
    match (rate, timer) {
        (0, 0) => 0,
        (0, timer) => CLOCK / u32::from(timer),
        (rate, _) => u32::from(rate),
    }
}

/// How many samples `len` bytes hold. IMA-ADPCM data starts with a 4-byte header.
pub(super) fn sample_count(encoding: u8, len: usize) -> usize {
    match encoding {
        PCM8 => len,
        PCM16 => len / 2,
        _ => len.saturating_sub(4) * 2,
    }
}

/// Decode `data` to 16-bit PCM. 8-bit PCM is signed; IMA-ADPCM is the DS variant: a header
/// holding the first sample and step index, then 4-bit codes, low nibble first, that the
/// hardware clamps to ±0x7FFF.
pub(super) fn decode(format: &'static str, encoding: u8, data: &[u8]) -> Result<Vec<i16>> {
    match encoding {
        PCM8 => Ok(data
            .iter()
            .map(|&byte| i16::from(byte as i8) << 8)
            .collect()),
        PCM16 => Ok(data
            .as_chunks()
            .0
            .iter()
            .map(|&pair| i16::from_le_bytes(pair))
            .collect()),
        IMA_ADPCM => {
            let mut header = Reader::new(format, data);
            let mut sample = i32::from(header.u16()? as i16);
            let mut index = usize::from(header.u8()?).min(88);
            let mut samples = Vec::with_capacity(sample_count(encoding, data.len()));
            for &byte in &data[4..] {
                for code in [byte & 0xF, byte >> 4] {
                    let step = STEPS[index];
                    let mut diff = step >> 3;
                    if code & 1 != 0 {
                        diff += step >> 2;
                    }
                    if code & 2 != 0 {
                        diff += step >> 1;
                    }
                    if code & 4 != 0 {
                        diff += step;
                    }
                    sample = match code & 8 {
                        0 => (sample + diff).min(0x7FFF),
                        _ => (sample - diff).max(-0x7FFF),
                    };
                    index = index.saturating_add_signed(INDEX_CHANGES[usize::from(code & 7)]);
                    index = index.min(88);
                    samples.push(sample as i16);
                }
            }
            Ok(samples)
        }
        _ => Err(Error::malformed(format, 0, "unknown sample encoding")),
    }
}

const INDEX_CHANGES: [isize; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

const STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// IMA-ADPCM encoding, the inverse of the decoder, for test data.
    pub fn adpcm(samples: &[i16]) -> Vec<u8> {
        let (mut predicted, mut index) = (i32::from(samples[0]), 0usize);
        let mut bytes = [&samples[0].to_le_bytes()[..], &[0, 0]].concat();
        let codes: Vec<u8> = samples
            .iter()
            .map(|&sample| {
                let step = STEPS[index];
                let delta = i32::from(sample) - predicted;
                let mut code = if delta < 0 { 8 } else { 0 };
                let (mut delta, mut diff) = (delta.abs(), step >> 3);
                for (bit, part) in [(4, step), (2, step >> 1), (1, step >> 2)] {
                    if delta >= part {
                        code |= bit;
                        delta -= part;
                        diff += part;
                    }
                }
                predicted = match code & 8 {
                    0 => (predicted + diff).min(0x7FFF),
                    _ => (predicted - diff).max(-0x7FFF),
                };
                index = index.saturating_add_signed(INDEX_CHANGES[usize::from(code & 7)]);
                index = index.min(88);
                code
            })
            .collect();
        bytes.extend(
            codes
                .chunks(2)
                .map(|pair| pair[0] | pair.get(1).unwrap_or(&0) << 4),
        );
        bytes
    }

    #[test]
    fn decodes_pcm() {
        assert_eq!(
            decode("t", PCM8, &[0x7F, 0x80, 0]).unwrap(),
            [0x7F00, -0x8000, 0]
        );
        assert_eq!(decode("t", PCM16, &[0x34, 0x12, 0xFF]).unwrap(), [0x1234]);
        assert!(decode("t", 3, &[]).is_err());
        assert_eq!(sample_rate(0, 1594), 10_512);
    }

    #[test]
    fn adpcm_follows_a_sine() {
        let sine: Vec<i16> = (0..400)
            .map(|i| ((i as f64 / 20.0).sin() * 12_000.0) as i16)
            .collect();
        let decoded = decode("t", IMA_ADPCM, &adpcm(&sine)).unwrap();
        assert_eq!(decoded.len(), sine.len());
        let error = sine
            .iter()
            .zip(&decoded)
            .skip(40)
            .map(|(&a, &b)| (i32::from(a) - i32::from(b)).abs())
            .max()
            .unwrap();
        assert!(error < 1500, "max error {error}");
        assert!(decode("t", IMA_ADPCM, &[1, 2]).is_err(), "truncated header");
    }
}
