//! The Nitro SDK's 2D graphics formats (G2D), which DS games use for sprites, icons, and
//! backgrounds. They come in sets that only make an image together:
//!
//! - [`Palette`] (NCLR): colors, in banks of 16 for 4-bit graphics.
//! - [`Tiles`] (NCGR): pixels as palette indices, in 8×8 tiles or as a plain bitmap.
//! - [`Cells`] (NCER): sprites assembled from hardware objects, each a block of tiles.
//! - [`Screen`] (NSCR): a background, as a grid of tile references.
//!
//! Every file is a Nitro container: a 16-byte header (byte-reversed signature, byte order mark,
//! version, file size, header size, chunk count), then chunks that each start with a
//! byte-reversed signature and their length, header included.

mod ncer;
mod ncgr;
mod nclr;
mod nscr;

pub use ncer::{Cell, Cells, Object};
pub use ncgr::{Depth, Tiles};
pub use nclr::Palette;
pub use nscr::Screen;

use crate::io::Reader;
use crate::{Error, Result};

/// The chunk with signature `magic` in a Nitro file, without its 8-byte chunk header. Errors
/// reading it give offsets within the chunk.
fn chunk<'a>(format: &'static str, bytes: &'a [u8], magic: &[u8; 4]) -> Result<Reader<'a>> {
    find_chunk(format, bytes, magic)?
        .ok_or_else(|| Error::malformed(format, 0, "a required chunk is missing"))
}

/// Like [`chunk`], for chunks a file may leave out.
fn find_chunk<'a>(
    format: &'static str,
    bytes: &'a [u8],
    magic: &[u8; 4],
) -> Result<Option<Reader<'a>>> {
    let file = Reader::new(format, bytes);
    let mut at = file.u16_at(12)? as usize;
    for _ in 0..file.u16_at(14)? {
        let len = file.u32_at(at + 4)? as usize;
        if len < 8 {
            return Err(file.error_at(at + 4, "chunk is shorter than its header"));
        }
        if &file.array_at::<4>(at)? == magic {
            // Some tools write a chunk length past the end of the file; read what is there.
            let end = at.saturating_add(len).min(bytes.len());
            return Ok(Some(Reader::new(
                format,
                file.slice_at(at + 8, end - at - 8)?,
            )));
        }
        at = at.saturating_add(len);
    }
    Ok(None)
}

#[cfg(test)]
pub(crate) mod testing {
    /// A Nitro file with signature `magic` holding `chunks`, given as `(signature, body)`.
    pub fn nitro(magic: &[u8; 4], chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let body: Vec<u8> = chunks
            .iter()
            .flat_map(|(magic, body)| {
                let len = (body.len() as u32 + 8).to_le_bytes();
                [&magic[..], &len, body].concat()
            })
            .collect();
        let mut bytes = magic.to_vec();
        bytes.extend(b"\xFF\xFE\x00\x01");
        bytes.extend((16 + body.len() as u32).to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend((chunks.len() as u16).to_le_bytes());
        bytes.extend(body);
        bytes
    }

    /// An NCGR of `size` tiles (`0xFFFF` for unknown), `depth` 3 or 4, tiled when `format` is 0.
    pub fn ncgr(size: (u16, u16), depth: u32, format: u32, data: &[u8]) -> Vec<u8> {
        let mut body = words(&[size.1, size.0]);
        for value in [depth, 0x10, format, data.len() as u32, 24] {
            body.extend(value.to_le_bytes());
        }
        body.extend(data);
        nitro(b"RGCN", &[(b"RAHC", &body)])
    }

    /// Little-endian 16-bit values as bytes.
    pub fn words(values: &[u16]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }
}
