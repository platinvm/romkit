//! LZ77 compression as the GBA and DS BIOS define it, which games use for graphics, archive
//! members, and whole files.
//!
//! A 4-byte header holds the type (`0x10` or `0x11`) in its low byte and the decompressed size
//! in the upper 24 bits. Then come blocks of a flag byte and eight items, its bits read from the
//! most significant: a 0 is a literal byte, a 1 a back-reference copying `length` bytes from
//! `distance` bytes back. Type `0x10` packs every reference in 2 bytes (length 3–18); type
//! `0x11` adds 3- and 4-byte forms for longer runs.

use std::borrow::Cow;

use crate::io::Reader;
use crate::{Error, Result};

const FORMAT: &str = "LZ77";

/// Decompress LZ77 data of either type.
pub fn decompress(bytes: &[u8]) -> Result<Vec<u8>> {
    decompress_counting(bytes).map(|(out, _)| out)
}

/// The decompressed bytes when `bytes` is LZ77 data, else `bytes` unchanged. Games compress
/// some files and not others in the same archive, and nothing but the content tells them apart:
/// `bytes` count as compressed when they decompress cleanly to something, and nothing but up
/// to 3 bytes of alignment padding follows.
pub fn decompress_if_compressed(bytes: &[u8]) -> Cow<'_, [u8]> {
    match decompress_counting(bytes) {
        Ok((out, read)) if !out.is_empty() && bytes.len() - read < 4 => Cow::Owned(out),
        _ => Cow::Borrowed(bytes),
    }
}

/// The decompressed bytes and how many input bytes they took.
fn decompress_counting(bytes: &[u8]) -> Result<(Vec<u8>, usize)> {
    let mut input = Reader::new(FORMAT, bytes);
    let header = input.u32()?;
    let kind = header as u8;
    if kind != 0x10 && kind != 0x11 {
        return Err(input.error_at(0, "not LZ77 data"));
    }
    let len = (header >> 8) as usize;
    // Every input byte yields at most a few hundred output bytes (type 0x10 at most 9), so a
    // tiny input claiming a huge size fails before the allocation grows.
    let mut out = Vec::with_capacity(len.min(bytes.len() * 9));
    while out.len() < len {
        let flags = input.u8()?;
        for bit in (0..8).rev() {
            if out.len() >= len {
                break;
            }
            if flags >> bit & 1 == 0 {
                out.push(input.u8()?);
                continue;
            }
            let at = input.position();
            let (length, distance) = reference(kind, &mut input)?;
            let start = out
                .len()
                .checked_sub(distance)
                .ok_or_else(|| Error::malformed(FORMAT, at as u64, "reference before the start"))?;
            // Copy byte by byte: a reference may overlap the bytes it produces.
            for index in start..start + length.min(len - out.len()) {
                out.push(out[index]);
            }
        }
    }
    Ok((out, input.position()))
}

/// A back-reference's length and distance. Its bytes, read as one big-endian number, end with
/// 12 bits of distance − 1, and the bits above hold the length minus a base. Type `0x10` uses
/// 2 bytes and base 3. Type `0x11` picks its size by the first 4 bits: 0 for 3 bytes and base
/// 0x11, 1 for 4 bytes and base 0x111 (these 4 bits are not part of the length), otherwise 2
/// bytes and base 1.
fn reference(kind: u8, input: &mut Reader) -> Result<(usize, usize)> {
    let form = input.u8_at(input.position())? >> 4;
    let (size, base) = match (kind, form) {
        (0x10, _) => (2, 3),
        (_, 0) => (3, 0x11),
        (_, 1) => (4, 0x111),
        _ => (2, 1),
    };
    let value = input
        .take(size)?
        .iter()
        .fold(0, |value, &byte| value << 8 | byte as usize);
    Ok(((value >> 12 & 0xFFFF) + base, (value & 0xFFF) + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompresses_literals_and_overlapping_references() {
        // "ab", then a reference copying 6 bytes from 2 back: "abababab".
        let bytes = [0x10, 8, 0, 0, 0b0010_0000, b'a', b'b', 0x30, 0x01];
        assert_eq!(decompress(&bytes).unwrap(), b"abababab");
    }

    #[test]
    fn decompresses_every_long_reference_form() {
        // "x", then 0x20 more bytes (8-bit form), 0x200 more (16-bit form), 4 more (short form).
        let len = 1 + 0x20 + 0x200 + 4u32;
        let mut bytes = (0x11 | len << 8).to_le_bytes().to_vec();
        bytes.extend([0b0111_0000, b'x']);
        bytes.extend([0x00, (0x20 - 0x11) << 4, 0x00]);
        bytes.extend([
            0x10,
            ((0x200 - 0x111) >> 4) as u8,
            ((0x200 - 0x111) << 4) as u8,
            0x00,
        ]);
        bytes.extend([0x30, 0x00]);
        assert_eq!(decompress(&bytes).unwrap(), vec![b'x'; len as usize]);
    }

    #[test]
    fn rejects_corrupt_data() {
        assert!(decompress(&[0x10, 4, 0, 0, 0x80, 0x00, 0x05]).is_err()); // before the start
        assert!(decompress(&[0x10, 8, 0, 0, 0x00, b'a']).is_err()); // truncated
        assert!(decompress(&[0x40, 1, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn leaves_uncompressed_data_alone() {
        let compressed = [0x10, 3, 0, 0, 0, b'a', b'b', b'c', 0];
        assert_eq!(&*decompress_if_compressed(&compressed), b"abc");
        assert!(matches!(
            decompress_if_compressed(b"RGCN\xFF\xFE"),
            Cow::Borrowed(_)
        ));
        // A record that happens to start with 0x10, the number 16 in a word, is not empty data.
        assert!(matches!(
            decompress_if_compressed(&[0x10, 0, 0, 0, 0, 0, 0, 0]),
            Cow::Borrowed(_)
        ));
        // Valid LZ77 followed by more data is not one compressed file.
        let followed = [&compressed[..], &[0; 8]].concat();
        assert_eq!(&*decompress_if_compressed(&followed), followed);
    }
}
