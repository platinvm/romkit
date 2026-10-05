//! The compression of Gen 2 graphics, which pret calls LZ3.
//!
//! A stream of commands up to an `0xFF` byte. A command byte holds a kind in its top 3 bits
//! and a length − 1 in the other 5; kind 7 marks a long command, whose kind is the next 3 bits
//! and whose length − 1 is the last 2 bits and the following byte. The kinds: copy literal
//! bytes (0), repeat one byte (1), alternate two bytes (2), write zeros (3), or copy earlier
//! output forward (4), with each byte's bits reversed (5), or backward (6). Copies start at a
//! byte with the high bit set and a distance back of its low 7 bits + 1, or else at the
//! big-endian 15-bit offset from the start of the output.

use crate::{Error, Result};

const FORMAT: &str = "Gen 2 LZ";
const END: u8 = 0xFF;
/// More than any picture with all its animation tiles, so corrupt data cannot grow without
/// bounds.
const LIMIT: usize = 0x8000;

pub fn decompress(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    let mut at = 0;
    let next = |at: &mut usize| -> Result<u8> {
        let byte = bytes
            .get(*at)
            .copied()
            .ok_or_else(|| Error::malformed(FORMAT, *at as u64, "data ends early"))?;
        *at += 1;
        Ok(byte)
    };
    loop {
        let start = at;
        let command = next(&mut at)?;
        if command == END {
            return Ok(out);
        }
        let (kind, len) = match command >> 5 {
            7 => (
                command >> 2 & 7,
                usize::from(command & 3) << 8 | usize::from(next(&mut at)?),
            ),
            kind => (kind, usize::from(command & 0x1F)),
        };
        let len = len + 1;
        if out.len() + len > LIMIT {
            return Err(Error::malformed(FORMAT, start as u64, "output too large"));
        }
        match kind {
            0 => {
                let literal = bytes
                    .get(at..at + len)
                    .ok_or_else(|| Error::malformed(FORMAT, at as u64, "data ends early"))?;
                out.extend_from_slice(literal);
                at += len;
            }
            1 => {
                let byte = next(&mut at)?;
                out.extend(std::iter::repeat_n(byte, len));
            }
            2 => {
                let pair = [next(&mut at)?, next(&mut at)?];
                out.extend((0..len).map(|index| pair[index % 2]));
            }
            3 => out.extend(std::iter::repeat_n(0, len)),
            4..=6 => {
                let first = next(&mut at)?;
                let from = match first & 0x80 {
                    0 => Some(usize::from(first) << 8 | usize::from(next(&mut at)?)),
                    _ => out.len().checked_sub(usize::from(first & 0x7F) + 1),
                };
                let bad = || Error::malformed(FORMAT, start as u64, "copy outside the output");
                let from = from.filter(|&from| from < out.len()).ok_or_else(bad)?;
                for index in 0..len {
                    let byte = match kind {
                        6 => out[from.checked_sub(index).ok_or_else(bad)?],
                        _ => *out.get(from + index).ok_or_else(bad)?,
                    };
                    out.push(if kind == 5 { byte.reverse_bits() } else { byte });
                }
            }
            _ => return Err(Error::malformed(FORMAT, start as u64, "unknown command")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompresses_every_command() {
        let bytes = [
            0x01, b'a', b'b', // literal "ab"
            0x22, b'c', // "ccc"
            0x43, b'x', b'y', // "xyxy"
            0x61, // two zeros
            0x81, 0x00, 0x00, // copy "ab" from offset 0
            0xA0, 0x81, // the byte 2 back ("a"), bits reversed
            0xC1, 0x00, 0x01, // "ba" backward from offset 1
            0xE0, 0x02, b'z', // a long literal of 3 bytes, here 1 for brevity: see below
        ];
        // The long literal claims 3 bytes but only 1 is there.
        assert!(decompress(&bytes).is_err());
        let mut bytes = bytes[..bytes.len() - 3].to_vec();
        bytes.extend([0xE0, 0x00, b'z', END]);
        let mut expected = b"abcccxyxy\0\0ab".to_vec();
        expected.extend([b'a'.reverse_bits(), b'b', b'a', b'z']);
        assert_eq!(decompress(&bytes).unwrap(), expected);
    }

    #[test]
    fn rejects_corrupt_data() {
        assert!(decompress(&[0x80, 0x81, END]).is_err()); // copy before the start
        assert!(decompress(&[0x00]).is_err()); // truncated
        assert!(decompress(&[0x03, 1, 2]).is_err());
        assert!(decompress(&[0xFC, 0x00, END]).is_err()); // long kind 7
        // Endless zeros stop at the limit.
        assert!(decompress(&[0xEF, 0xFF].repeat(64)).is_err());
    }
}
