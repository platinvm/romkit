//! Nitro 3D: the model (NSBMD) and texture (NSBTX) files of the DS's 3D engine, decoded to
//! [`Mesh`](crate::media::Mesh) and [`Image`](crate::media::Image).
//!
//! Every file is a container: a 16-byte header (signature, byte order mark, version, file
//! size, header size, block count), then one offset per block, each block starting with its
//! own signature (`MDL0`, `TEX0`) and size. Inside blocks, named collections are
//! dictionaries: a count, a search tree the DS uses to look names up (ignored here), the
//! fixed-size entries, then a 16-byte name per entry.

mod model;
mod render;
mod texture;

pub use model::{Material, Model, Node, Nsbmd};
pub use texture::{Palette, Texture, TextureFormat, Textures};

use crate::Result;
use crate::io::{Reader, ascii};

/// The blocks of a Nitro 3D file whose signature is `magic`, as `(signature, offset)`.
fn blocks(file: &Reader, magic: &[u8; 4]) -> Result<Vec<([u8; 4], usize)>> {
    file.clone().expect(magic)?;
    let count = file.u16_at(14)? as usize;
    (0..count)
        .map(|index| {
            let offset = file.u32_at(16 + 4 * index)? as usize;
            Ok((file.array_at(offset)?, offset))
        })
        .collect()
}

/// A block's bytes, from its signature to its end, in a reader of its own; offsets within
/// blocks are relative to their start.
fn block<'a>(file: &Reader<'a>, format: &'static str, offset: usize) -> Result<Reader<'a>> {
    let len = file.u32_at(offset + 4)? as usize;
    Ok(Reader::new(format, file.slice_at(offset, len)?))
}

/// The entries of the dictionary at `at`: each name with its fixed-size data.
fn dictionary<'a>(data: &Reader<'a>, at: usize) -> Result<Vec<(String, &'a [u8])>> {
    let count = data.u8_at(at + 1)? as usize;
    let entries = at + data.u16_at(at + 6)? as usize;
    let entry_len = data.u16_at(entries)? as usize;
    let names = entries + data.u16_at(entries + 2)? as usize;
    (0..count)
        .map(|index| {
            let name = ascii(data.slice_at(names + 16 * index, 16)?);
            Ok((
                name,
                data.slice_at(entries + 4 + entry_len * index, entry_len)?,
            ))
        })
        .collect()
}

/// A 20.12 fixed-point number.
fn fx32(raw: u32) -> f32 {
    raw as i32 as f32 / 4096.0
}

/// A 4.12 fixed-point number.
fn fx16(raw: u16) -> f32 {
    raw as i16 as f32 / 4096.0
}

#[cfg(test)]
pub(crate) mod testing {
    //! Builders for synthetic Nitro 3D files.

    /// A Nitro 3D file holding `blocks`, each already starting with its signature and size.
    pub fn file(magic: &[u8; 4], blocks: &[Vec<u8>]) -> Vec<u8> {
        let header = 16 + 4 * blocks.len();
        let mut bytes = magic.to_vec();
        bytes.extend(b"\xFF\xFE\x02\x00");
        let size = header + blocks.iter().map(Vec::len).sum::<usize>();
        bytes.extend((size as u32).to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend((blocks.len() as u16).to_le_bytes());
        let mut offset = header;
        for block in blocks {
            bytes.extend((offset as u32).to_le_bytes());
            offset += block.len();
        }
        bytes.extend(blocks.concat());
        bytes
    }

    /// A block: `magic`, its size, then `body`.
    pub fn block(magic: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut bytes = magic.to_vec();
        bytes.extend((8 + body.len() as u32).to_le_bytes());
        bytes.extend(body);
        bytes
    }

    /// A dictionary of `entries`, all of the same length.
    pub fn dictionary(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let count = entries.len();
        let entry_len = entries.first().map_or(4, |(_, data)| data.len());
        let tree = 12 + 4 * count;
        let mut bytes = vec![0, count as u8];
        let size = tree + 4 + entry_len * count + 16 * count;
        bytes.extend((size as u16).to_le_bytes());
        bytes.extend(8u16.to_le_bytes());
        bytes.extend((tree as u16).to_le_bytes());
        bytes.extend(0x17Fu32.to_le_bytes());
        bytes.resize(tree, 0);
        bytes.extend((entry_len as u16).to_le_bytes());
        bytes.extend(((4 + entry_len * count) as u16).to_le_bytes());
        for (_, data) in entries {
            bytes.extend(*data);
        }
        for (name, _) in entries {
            let mut field = [0; 16];
            field[..name.len()].copy_from_slice(name.as_bytes());
            bytes.extend(field);
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_dictionaries() {
        let bytes = testing::dictionary(&[("first", &[1, 0, 0, 0]), ("second", &[2, 0, 0, 0])]);
        let entries = dictionary(&Reader::new("test", &bytes), 0).unwrap();
        assert_eq!(entries[1], ("second".to_owned(), [2, 0, 0, 0].as_slice()));
        assert!(dictionary(&Reader::new("test", &bytes[..20]), 0).is_err());
    }

    #[test]
    fn finds_blocks() {
        let file = testing::file(b"BTX0", &[testing::block(b"TEX0", &[0; 8])]);
        let reader = Reader::new("test", &file);
        assert_eq!(blocks(&reader, b"BTX0").unwrap(), [(*b"TEX0", 20)]);
        assert_eq!(block(&reader, "test", 20).unwrap().len(), 16);
        assert!(blocks(&reader, b"BMD0").is_err());
    }
}
