//! The ARM9 binary, the game's main code. Games keep some tables in it rather than in files.
//!
//! The Nitro SDK appends a 12-byte footer after the binary in the ROM: a magic word, then the
//! offset of the module parameters in the binary. Those hold, at 0x14, the address where the
//! compressed part of the binary ends once loaded, or 0 when it is not compressed, and end at
//! 0x1C with the same magic word followed by its byte-swapped form. DSi-enhanced games leave
//! the footer out, so the parameters are found by those two words instead.

use crate::io::Reader;
use crate::{Error, Result};

const FOOTER_MAGIC: u32 = 0xDEC0_0621;
const COMPRESSED_END: usize = 0x14;
/// Where the module parameters' magic words start within them.
const PARAMS_MAGIC: usize = 0x1C;

/// Decompress the ARM9 binary `code`, loaded at `address`, given the `footer` that follows it.
pub(super) fn arm9(mut code: Vec<u8>, address: u32, footer: &[u8]) -> Result<Vec<u8>> {
    let footer = Reader::new("DS ARM9 footer", footer);
    let params = match footer.u32_at(0) {
        Ok(FOOTER_MAGIC) => footer.u32_at(4)? as usize,
        _ => {
            let magic = [FOOTER_MAGIC.to_le_bytes(), FOOTER_MAGIC.swap_bytes().to_le_bytes()];
            let found = code.windows(8).position(|window| window == magic.as_flattened());
            match found.and_then(|at| at.checked_sub(PARAMS_MAGIC)) {
                Some(params) => params,
                None => return Ok(code),
            }
        }
    };
    let end = match Reader::new("DS ARM9", &code).u32_at(params.saturating_add(COMPRESSED_END))? {
        0 => return Ok(code),
        end => end.wrapping_sub(address) as usize,
    };
    if end > code.len() {
        return Err(Error::malformed(
            "DS ARM9",
            params as u64,
            "compressed part exceeds the binary",
        ));
    }
    let rest = code.split_off(end);
    let mut out = decompress_backward(&code)?;
    out.extend(rest);
    Ok(out)
}

/// Decompress code compressed backward, the variant the Nitro SDK applies to the ARM9 binary
/// and overlays so they can expand in place in memory.
///
/// An 8-byte footer ends the data: the length of the compressed part counted back from the end
/// (24 bits) and the footer's own length with padding (8 bits), then how many bytes
/// decompression adds. Bytes before the compressed part are stored as is. The compressed part
/// is read from its end towards its start, and written from the end of the output: flag bytes
/// as in LZ77 type `0x10`, whose references are 2 bytes, high byte first, of length − 3
/// (4 bits) and distance − 3 (12 bits) forward from the byte being written.
fn decompress_backward(bytes: &[u8]) -> Result<Vec<u8>> {
    const FORMAT: &str = "backward LZ77";
    /// The DS has 4 MB of main memory; no code expands beyond it.
    const MAX_LEN: usize = 4 << 20;
    let input = Reader::new(FORMAT, bytes);
    let footer_at = bytes
        .len()
        .checked_sub(8)
        .ok_or_else(|| input.error_at(0, "too short for its footer"))?;
    let lengths = input.u32_at(footer_at)?;
    let added = input.u32_at(footer_at + 4)? as usize;
    let start = bytes
        .len()
        .checked_sub(lengths as usize & 0xFF_FFFF)
        .ok_or_else(|| input.error_at(footer_at, "compressed part exceeds the data"))?;
    let mut source = bytes
        .len()
        .checked_sub((lengths >> 24) as usize)
        .filter(|&source| source >= start)
        .ok_or_else(|| input.error_at(footer_at, "footer exceeds the compressed part"))?;
    if bytes.len().saturating_add(added) > MAX_LEN {
        return Err(input.error_at(footer_at + 4, "decompressed size is too large"));
    }
    let mut out = bytes.to_vec();
    out.resize(bytes.len() + added, 0);
    let mut target = out.len();
    let corrupt = |at: usize| Error::malformed(FORMAT, at as u64, "reference out of range");
    'blocks: while source > start {
        source -= 1;
        let flags = bytes[source];
        for bit in (0..8).rev() {
            if source <= start {
                break 'blocks;
            }
            if flags >> bit & 1 == 0 {
                source -= 1;
                target = target.checked_sub(1).ok_or_else(|| corrupt(source))?;
                out[target] = bytes[source];
                continue;
            }
            source = source.checked_sub(2).ok_or_else(|| corrupt(source))?;
            let [low, high] = input.array_at(source)?;
            let length = (high >> 4) as usize + 3;
            let distance = ((high as usize & 0xF) << 8 | low as usize) + 3;
            for _ in 0..length {
                target = target.checked_sub(1).ok_or_else(|| corrupt(source))?;
                out[target] = *out.get(target + distance).ok_or_else(|| corrupt(source))?;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "ab" stored as is, then 6 compressed bytes read from the end: the flags (literal,
    /// literal, literal, reference), "z", "y", "x", and a reference of 18 bytes from 3 ahead,
    /// high byte 0xF0 first. The footer says the last 14 bytes are compressed, the footer is 8
    /// of them, and decompressing adds 7 bytes.
    fn compressed() -> Vec<u8> {
        let mut bytes = b"ab".to_vec();
        bytes.extend([0x00, 0xF0, b'x', b'y', b'z', 0b0001_0000]);
        bytes.extend((14u32 | 8 << 24).to_le_bytes());
        bytes.extend(7u32.to_le_bytes());
        bytes
    }

    #[test]
    fn decompresses_backward() {
        let expected = [&b"ab"[..], &b"xyz".repeat(7)].concat();
        assert_eq!(decompress_backward(&compressed()).unwrap(), expected);

        assert!(decompress_backward(&[0; 4]).is_err());
        let mut corrupt = compressed();
        corrupt[3] = 0xFF; // a reference reaching past the end
        assert!(decompress_backward(&corrupt).is_err());
    }

    #[test]
    fn decompresses_the_part_the_module_parameters_name() {
        // Module parameters at 0, saying the compressed part ends at 0x2000_0000 + 0x40 + 16.
        let mut code = vec![0; 0x40];
        code[COMPRESSED_END..COMPRESSED_END + 4].copy_from_slice(&0x2000_0050u32.to_le_bytes());
        code.extend(compressed());
        code.extend(b"tail");
        let footer = [FOOTER_MAGIC.to_le_bytes(), 0u32.to_le_bytes(), [0; 4]].concat();
        let out = arm9(code.clone(), 0x2000_0000, &footer).unwrap();
        assert_eq!(&out[0x40..0x42], b"ab");
        assert_eq!(out.len(), code.len() + 7);
        assert!(out.ends_with(b"xyztail"));
        // Without the footer or the parameters' magic words, the binary is taken as stored.
        assert_eq!(arm9(code.clone(), 0x2000_0000, &[]).unwrap(), code);
        // Without the footer, the parameters are found by their magic words.
        code[PARAMS_MAGIC..PARAMS_MAGIC + 4].copy_from_slice(&FOOTER_MAGIC.to_le_bytes());
        code[PARAMS_MAGIC + 4..PARAMS_MAGIC + 8].copy_from_slice(&0x2106_C0DEu32.to_le_bytes());
        assert_eq!(arm9(code, 0x2000_0000, &[]).unwrap()[0x40..], out[0x40..]);
    }
}
