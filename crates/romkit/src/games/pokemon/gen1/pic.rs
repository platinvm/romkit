//! The compression of Gen 1 Pokémon and trainer pictures ("pics").
//!
//! A pic is a bitstream, read from the most significant bit. It starts with the size in tiles
//! (4 bits of width, 4 of height) and holds the picture's two bitplanes one after the other.
//! A plane is a sequence of 2-bit pairs filling the picture in columns two pixels wide, top to
//! bottom then left to right; packets alternate between runs of zero pairs and literal pairs
//! (ended by a `00` pair), the first bit telling which comes first. A zero run of `n` 1 bits
//! then a 0 bit, followed by an `n + 1`-bit value `v`, is `2^(n+1) - 1 + v` pairs long.
//!
//! Planes are stored as deltas: along each row, a 1 bit flips the pixel's color from the
//! previous one's. A bit before the second plane says how to undo this: delta-decode both
//! planes (mode 0); decode the first and XOR it into the second (mode 1); or decode both and
//! XOR the first into the second (mode 2). Another bit before the first plane says which
//! plane, the low or the high one, comes first.

use crate::{Error, Result};

const FORMAT: &str = "Gen 1 pic";
/// Longer zero runs would overflow the largest picture many times over.
const MAX_RUN_BITS: u32 = 16;

/// A decompressed picture: palette indices, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pic {
    /// In tiles of 8×8 pixels.
    pub width: u8,
    pub height: u8,
    pub pixels: Vec<u8>,
}

pub fn decompress(bytes: &[u8]) -> Result<Pic> {
    let mut bits = Bits { bytes, position: 0 };
    let (width, height) = (bits.read(4)? as u8, bits.read(4)? as u8);
    if width == 0 || height == 0 {
        return Err(Error::malformed(FORMAT, 0, "empty picture"));
    }
    let size = Size {
        width: usize::from(width) * 8,
        height: usize::from(height) * 8,
    };
    let mut planes = [
        vec![0; size.width * size.height],
        vec![0; size.width * size.height],
    ];
    let first = bits.read(1)? as usize;
    let second = first ^ 1;
    fill(&mut bits, &mut planes[first], size)?;
    let mode = match bits.read(1)? {
        0 => 0,
        _ => 1 + bits.read(1)?,
    };
    fill(&mut bits, &mut planes[second], size)?;
    match mode {
        0 => planes.iter_mut().for_each(|plane| undo_delta(plane, size)),
        1 => undo_delta(&mut planes[first], size),
        _ => {
            undo_delta(&mut planes[second], size);
            undo_delta(&mut planes[first], size);
        }
    }
    if mode != 0 {
        let [low, high] = &mut planes;
        let (from, into) = if first == 0 { (low, high) } else { (high, low) };
        into.iter_mut()
            .zip(from.iter())
            .for_each(|(bit, from)| *bit ^= from);
    }
    let [low, high] = planes;
    Ok(Pic {
        width,
        height,
        pixels: low
            .iter()
            .zip(&high)
            .map(|(low, high)| low | high << 1)
            .collect(),
    })
}

#[derive(Debug, Clone, Copy)]
struct Size {
    width: usize,
    height: usize,
}

/// One bitplane, one bit per pixel, from alternating zero-run and literal packets.
fn fill(bits: &mut Bits, plane: &mut [u8], size: Size) -> Result<()> {
    let pairs = plane.len() / 2;
    let mut pair = 0;
    let mut zeros = bits.read(1)? == 0;
    while pair < pairs {
        if zeros {
            let start = bits.position;
            let mut ones = 0;
            while bits.read(1)? == 1 {
                ones += 1;
                if ones >= MAX_RUN_BITS {
                    return Err(bits.error_at(start, "zero run too long"));
                }
            }
            pair += (2usize << ones) - 1 + bits.read(ones + 1)? as usize;
            if pair > pairs {
                return Err(bits.error_at(start, "zero run past the end of the plane"));
            }
        } else {
            while pair < pairs {
                let value = bits.read(2)? as u8;
                if value == 0 {
                    break;
                }
                let (x, y) = (pair / size.height * 2, pair % size.height);
                plane[y * size.width + x] = value >> 1;
                plane[y * size.width + x + 1] = value & 1;
                pair += 1;
            }
        }
        zeros = !zeros;
    }
    Ok(())
}

/// Along each row, a 1 bit flips the color from the previous pixel's.
fn undo_delta(plane: &mut [u8], size: Size) {
    for row in plane.chunks_exact_mut(size.width) {
        let mut color = 0;
        for bit in row {
            color ^= *bit;
            *bit = color;
        }
    }
}

/// A bitstream read from the most significant bit of each byte.
struct Bits<'a> {
    bytes: &'a [u8],
    /// In bits.
    position: usize,
}

impl Bits<'_> {
    /// The next `count` bits (at most 32) as a number.
    fn read(&mut self, count: u32) -> Result<u32> {
        let mut value = 0;
        for _ in 0..count {
            let byte = self
                .bytes
                .get(self.position / 8)
                .ok_or_else(|| self.error_at(self.position, "data ends early"))?;
            value = value << 1 | u32::from(byte >> (7 - self.position % 8) & 1);
            self.position += 1;
        }
        Ok(value)
    }

    fn error_at(&self, bit: usize, reason: &'static str) -> Error {
        Error::malformed(FORMAT, (bit / 8) as u64, reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pack a string of `0` and `1` into bytes, padding with zeros.
    fn pack(bits: &str) -> Vec<u8> {
        let bits: Vec<u8> = bits.bytes().filter(|b| b"01".contains(b)).collect();
        bits.chunks(8)
            .map(|chunk| {
                (0..8).fold(0, |byte, index| {
                    byte << 1 | chunk.get(index).map_or(0, |bit| bit - b'0')
                })
            })
            .collect()
    }

    #[test]
    fn decompresses_runs_literals_and_deltas() {
        // An 8×8 pic: the low plane is one run of 32 zero pairs; the high plane is the pair
        // 01, then a run of 31 zero pairs. Mode 0 delta-decodes the high plane's first row
        // into 0 then seven 1s.
        let bytes = pack("0001 0001 0 0 11110 00001 0 1 01 00 11110 00000");
        let pic = decompress(&bytes).unwrap();
        assert_eq!((pic.width, pic.height), (1, 1));
        assert_eq!(pic.pixels[..8], [0, 2, 2, 2, 2, 2, 2, 2]);
        assert!(pic.pixels[8..].iter().all(|&pixel| pixel == 0));
    }

    #[test]
    fn rejects_truncated_and_overlong_data() {
        assert!(decompress(&pack("0001 0001 0 0 11110")).is_err());
        assert!(decompress(&pack("0001 0001 0 0 111110 000000")).is_err());
        assert!(decompress(&pack("0000 0001")).is_err());
        assert!(decompress(&[0xFF; 64]).is_err());
    }
}
