//! Textures and palettes: the `TEX0` block of NSBTX files, also found inside NSBMD models.
//!
//! The block header gives three data sections and two dictionaries (offsets from the block
//! start; sizes are stored divided by 8):
//!
//! ```text
//! 0x0C u16 texel data size      0x0E u16 texture dictionary  0x14 u32 texel data
//! 0x1C u16 4×4 texel data size  0x24 u32 4×4 texel data      0x28 u32 4×4 palette indices
//! 0x30 u16 color data size      0x34 u32 palette dictionary  0x38 u32 color data
//! ```
//!
//! A texture's entry is the GPU's `TEXIMAGE_PARAM` word: its texels' offset (in 8-byte
//! units), size, format, and whether color 0 is transparent. A palette's entry is the offset
//! of its first color. Which palette goes with which texture is up to the models that use
//! them: their materials name both.

use super::{blocks, dictionary};
use crate::io::{ByteSource, Reader};
use crate::media::{Image, Rgba, bgr555};
use crate::{Error, Result};

raw_enum! {
    /// How a texture stores its texels.
    pub enum TextureFormat: u8 {
        /// A 5-bit palette index and a 3-bit alpha per byte.
        A3i5 = 1,
        /// 2-bit palette indices.
        Palette4 = 2,
        /// 4-bit palette indices.
        Palette16 = 3,
        /// 8-bit palette indices.
        Palette256 = 4,
        /// Blocks of 4×4 texels, each with 2-bit indices into a palette slice of its own.
        Compressed = 5,
        /// A 3-bit palette index and a 5-bit alpha per byte.
        A5i3 = 6,
        /// A BGR555 color per texel, with a 1-bit alpha.
        Direct = 7,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Texture {
    pub name: String,
    pub format: TextureFormat,
    pub width: u32,
    pub height: u32,
    /// Whether palette index 0 is transparent rather than a color.
    pub transparent_zero: bool,
    /// Where its texels start in their data section.
    offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub name: String,
    /// Where its colors start in the color data.
    offset: usize,
}

/// A `TEX0` block's textures and palettes, with their data, decoded on demand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Textures {
    pub textures: Vec<Texture>,
    pub palettes: Vec<Palette>,
    block: Vec<u8>,
    texels: usize,
    compressed: usize,
    compressed_indices: usize,
    colors: std::ops::Range<usize>,
}

impl Textures {
    /// The textures of an NSBTX file.
    pub fn parse(file: &[u8]) -> Result<Self> {
        let file = Reader::new("NSBTX", file);
        let (_, offset) = blocks(&file, b"BTX0")?
            .into_iter()
            .find(|(magic, _)| magic == b"TEX0")
            .ok_or_else(|| file.error_at(0, "no TEX0 block"))?;
        Self::from_block(&super::block(&file, "NSBTX", offset)?)
    }

    /// The texture names of an NSBTX file, reading only its tables: enough to list textures
    /// without decoding them.
    pub fn names<S: ByteSource + ?Sized>(file: &S) -> Result<Vec<String>> {
        let head = file.read_up_to(0, 20)?;
        let head = Reader::new("NSBTX", &head);
        head.clone().expect(b"BTX0")?;
        // TEX0 is the first and only block.
        let block = head.u32_at(16)? as u64;
        let header = file.read(block, 0x3C)?;
        let header = Reader::new("NSBTX", &header);
        header.clone().expect(b"TEX0")?;
        let at = block + header.u16_at(0x0E)? as u64;
        let len = Reader::new("NSBTX", &file.read(at, 4)?).u16_at(2)?;
        let entries = file.read(at, len as u64)?;
        let entries = dictionary(&Reader::new("NSBTX", &entries), 0)?;
        Ok(entries.into_iter().map(|(name, _)| name).collect())
    }

    /// A `TEX0` block, from its signature to its end.
    pub(super) fn from_block(block: &Reader) -> Result<Self> {
        let colors = block.u32_at(0x38)? as usize;
        let colors_len = (block.u16_at(0x30)? as usize) << 3;
        let textures = dictionary(block, block.u16_at(0x0E)? as usize)?
            .into_iter()
            .map(|(name, entry)| {
                let param = Reader::new("NSBTX", entry).u32()?;
                Ok(Texture {
                    name,
                    format: TextureFormat::from_raw((param >> 26 & 7) as u8),
                    width: 8 << (param >> 20 & 7),
                    height: 8 << (param >> 23 & 7),
                    transparent_zero: param >> 29 & 1 != 0,
                    offset: ((param & 0xFFFF) as usize) << 3,
                })
            })
            .collect::<Result<_>>()?;
        let palettes = dictionary(block, block.u32_at(0x34)? as usize)?
            .into_iter()
            .map(|(name, entry)| {
                let offset = (Reader::new("NSBTX", entry).u16()? as usize) << 3;
                Ok(Palette { name, offset })
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            textures,
            palettes,
            block: block.slice_at(0, block.len())?.to_vec(),
            texels: block.u32_at(0x14)? as usize,
            compressed: block.u32_at(0x24)? as usize,
            compressed_indices: block.u32_at(0x28)? as usize,
            colors: colors..(colors + colors_len).min(block.len()),
        })
    }

    pub fn texture(&self, name: &str) -> Option<&Texture> {
        self.textures.iter().find(|texture| texture.name == name)
    }

    pub fn palette(&self, name: &str) -> Option<&Palette> {
        self.palettes.iter().find(|palette| palette.name == name)
    }

    /// The palette that most likely goes with `texture`, for showing textures without the
    /// models that name both. Palettes are named after their texture, often with a `_pl`
    /// suffix; animation frames (`boy1.1`, `boy1.2`) share one named without the frame
    /// number; and sets of textures abbreviate (`ngrass` uses `grass`, `allpeak` uses
    /// `apeak`). So: one of those exact names, else the palette whose name shares the longest
    /// run of characters with the texture's, then the longest start, else the first.
    pub fn palette_for(&self, texture: &Texture) -> Option<&Palette> {
        let name = texture.name.as_str();
        let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
        [name, stem]
            .into_iter()
            .flat_map(|name| [name.to_owned(), format!("{name}_pl")])
            .find_map(|name| self.palette(&name))
            .or_else(|| {
                let similarity = |palette: &&Palette| {
                    let (a, b) = (stem.as_bytes(), palette.name.as_bytes());
                    let run = (0..a.len())
                        .flat_map(|i| (0..b.len()).map(move |j| (i, j)))
                        .map(|(i, j)| {
                            a[i..]
                                .iter()
                                .zip(&b[j..])
                                .take_while(|(x, y)| x == y)
                                .count()
                        })
                        .max();
                    let start = a.iter().zip(b).take_while(|(x, y)| x == y).count();
                    (run.unwrap_or(0), start)
                };
                // The first of equals: `max_by_key` keeps the last.
                self.palettes.iter().rev().max_by_key(similarity)
            })
    }

    /// Decode `texture`; every format but direct color needs a palette.
    pub fn image(&self, texture: &Texture, palette: Option<&Palette>) -> Result<Image> {
        let data = Reader::new("NSBTX texture", &self.block);
        let (width, height) = (texture.width, texture.height);
        let count = (width * height) as usize;
        let colors = |len: usize| -> Result<Vec<u16>> {
            let palette = palette
                .ok_or_else(|| Error::NotFound(format!("palette for texture {}", texture.name)))?;
            let start = self.colors.start + palette.offset;
            if start >= self.colors.end {
                return Err(data.error_at(start, "palette lies outside the color data"));
            }
            let len = len.min((self.colors.end - start) / 2);
            let bytes = data.slice_at(start, len * 2)?;
            Ok(bytes
                .as_chunks()
                .0
                .iter()
                .map(|&c| u16::from_le_bytes(c))
                .collect())
        };
        let texels = |bits: usize| data.slice_at(self.texels + texture.offset, count * bits / 8);
        let pixels: Vec<Rgba> = match texture.format {
            TextureFormat::Palette4 | TextureFormat::Palette16 | TextureFormat::Palette256 => {
                let bits = match texture.format {
                    TextureFormat::Palette4 => 2,
                    TextureFormat::Palette16 => 4,
                    _ => 8,
                };
                let bytes = texels(bits)?;
                let mask = (1u16 << bits) - 1;
                let indices: Vec<u8> = (0..count)
                    .map(|i| (bytes[i * bits / 8] as u16 >> (i * bits % 8) & mask) as u8)
                    .collect();
                let palette: Vec<Rgba> = colors(1 << bits)?.into_iter().map(bgr555).collect();
                let image = Image::from_indices(
                    width,
                    height,
                    &indices,
                    &palette,
                    texture.transparent_zero,
                );
                return Ok(image);
            }
            TextureFormat::A3i5 | TextureFormat::A5i3 => {
                let index_bits = if texture.format == TextureFormat::A3i5 {
                    5
                } else {
                    3
                };
                let palette = colors(1 << index_bits)?;
                let max_alpha = (1u32 << (8 - index_bits)) - 1;
                texels(8)?
                    .iter()
                    .map(|&texel| {
                        let index = (texel & ((1 << index_bits) - 1)) as usize;
                        let alpha = (texel >> index_bits) as u32 * 255 / max_alpha;
                        palette.get(index).map_or([0; 4], |&color| {
                            let [r, g, b, _] = bgr555(color);
                            [r, g, b, alpha as u8]
                        })
                    })
                    .collect()
            }
            TextureFormat::Direct => texels(16)?
                .as_chunks()
                .0
                .iter()
                .map(|&texel| {
                    let color = u16::from_le_bytes(texel);
                    match color >> 15 {
                        0 => [0; 4],
                        _ => bgr555(color),
                    }
                })
                .collect(),
            TextureFormat::Compressed => self.compressed(&data, texture, &colors(1 << 15)?)?,
            TextureFormat::Unknown(format) => {
                return Err(Error::Unsupported(format!(
                    "texture {} has format {format}",
                    texture.name
                )));
            }
        };
        Ok(Image {
            width,
            height,
            pixels,
        })
    }

    /// 4×4 compressed texels. Each block has 32 bits of 2-bit indices, a row per byte, and a
    /// 16-bit word elsewhere: bits 0–13 locate its colors in the palette (in pairs), and bits
    /// 14–15 say what the four indices mean. Colors are mixed in BGR555, as the GPU does.
    fn compressed(&self, data: &Reader, texture: &Texture, palette: &[u16]) -> Result<Vec<Rgba>> {
        let (width, height) = (texture.width as usize, texture.height as usize);
        let blocks = width * height / 16;
        let texels = data.slice_at(self.compressed + texture.offset, blocks * 4)?;
        let modes = data.slice_at(self.compressed_indices + texture.offset / 2, blocks * 2)?;
        let mut pixels = vec![[0; 4]; width * height];
        for block in 0..blocks {
            let mode = u16::from_le_bytes([modes[2 * block], modes[2 * block + 1]]);
            let base = ((mode & 0x3FFF) * 2) as usize;
            let color = |i: usize| palette.get(base + i).copied();
            let mix = |a: u16, b: u16, wa: u16, wb: u16| {
                let channel =
                    |shift: u16| ((a >> shift & 31) * wa + (b >> shift & 31) * wb) / (wa + wb);
                bgr555(channel(0) | channel(5) << 5 | channel(10) << 10)
            };
            let solid = |c: Option<u16>| c.map_or([0; 4], bgr555);
            let four = match (mode >> 14, color(0), color(1)) {
                (0, ..) => [solid(color(0)), solid(color(1)), solid(color(2)), [0; 4]],
                (1, Some(a), Some(b)) => [bgr555(a), bgr555(b), mix(a, b, 1, 1), [0; 4]],
                (2, ..) => [0, 1, 2, 3].map(|i| solid(color(i))),
                (3, Some(a), Some(b)) => [bgr555(a), bgr555(b), mix(a, b, 5, 3), mix(a, b, 3, 5)],
                _ => [[0; 4]; 4],
            };
            let (x, y) = (block % (width / 4) * 4, block / (width / 4) * 4);
            for row in 0..4 {
                let bits = texels[4 * block + row];
                for column in 0..4 {
                    pixels[(y + row) * width + x + column] =
                        four[(bits >> (2 * column) & 3) as usize];
                }
            }
        }
        Ok(pixels)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::testing::{block, dictionary, file};
    use super::*;

    /// A `TEX0` block body (after its signature and size) holding `textures`, given as
    /// (name, format, width and height as GPU size codes, texels), with one palette.
    pub fn tex0(textures: &[(&str, u32, u32, &[u8])], palette: (&str, &[u16])) -> Vec<u8> {
        let mut texels = Vec::new();
        let mut entries = Vec::new();
        for &(_, format, size, data) in textures {
            let param =
                (texels.len() as u32 >> 3) | size << 20 | size << 23 | format << 26 | 1 << 29;
            entries.push([param.to_le_bytes(), [0; 4]].concat());
            texels.extend(data);
            texels.resize(texels.len().next_multiple_of(8), 0);
        }
        let named: Vec<(&str, &[u8])> = textures
            .iter()
            .zip(&entries)
            .map(|((name, ..), entry)| (*name, entry.as_slice()))
            .collect();
        let texture_dictionary = dictionary(&named);
        let palette_dictionary = dictionary(&[(palette.0, &[0, 0, 0, 0])]);
        let mut colors: Vec<u8> = palette.1.iter().flat_map(|c| c.to_le_bytes()).collect();
        colors.resize(colors.len().next_multiple_of(8), 0);

        let header = 0x3C;
        let texels_at = header + texture_dictionary.len() + palette_dictionary.len();
        let colors_at = texels_at + texels.len();
        let mut body = vec![0; header - 8];
        let mut put =
            |at: usize, value: &[u8]| body[at - 8..at - 8 + value.len()].copy_from_slice(value);
        put(0x0C, &((texels.len() >> 3) as u16).to_le_bytes());
        put(0x0E, &(header as u16).to_le_bytes());
        put(0x14, &(texels_at as u32).to_le_bytes());
        put(0x30, &((colors.len() >> 3) as u16).to_le_bytes());
        put(
            0x34,
            &((header + texture_dictionary.len()) as u32).to_le_bytes(),
        );
        put(0x38, &(colors_at as u32).to_le_bytes());
        [body, texture_dictionary, palette_dictionary, texels, colors].concat()
    }

    const RED: u16 = 0x001F;
    const BLUE: u16 = 0x7C00;

    #[test]
    fn decodes_paletted_textures_with_transparent_zero() {
        // 8×8 at 2 bits per texel: every texel is index 1, but the first is index 0.
        let mut texels = [0x55u8; 16];
        texels[0] = 0x54;
        let bytes = file(
            b"BTX0",
            &[block(
                b"TEX0",
                &tex0(&[("wall.1", 2, 0, &texels)], ("wall", &[0, RED])),
            )],
        );
        let textures = Textures::parse(&bytes).unwrap();
        let wall = &textures.textures[0];
        assert_eq!((wall.width, wall.format), (8, TextureFormat::Palette4));
        let palette = textures.palette_for(wall);
        assert_eq!(palette.unwrap().name, "wall");
        let image = textures.image(wall, palette).unwrap();
        assert_eq!(image.get(0, 0), Some([0; 4]));
        assert_eq!(image.get(1, 0), Some([255, 0, 0, 255]));
        assert!(matches!(
            textures.image(wall, None),
            Err(Error::NotFound(_))
        ));
        assert_eq!(Textures::names(bytes.as_slice()).unwrap(), ["wall.1"]);
    }

    #[test]
    fn pairs_palettes_by_name() {
        let named = |textures: &[&str], palettes: &[&str]| Textures {
            textures: textures
                .iter()
                .map(|&name| Texture {
                    name: name.into(),
                    format: TextureFormat::Palette16,
                    width: 8,
                    height: 8,
                    transparent_zero: false,
                    offset: 0,
                })
                .collect(),
            palettes: palettes
                .iter()
                .map(|&name| Palette {
                    name: name.into(),
                    offset: 0,
                })
                .collect(),
            block: Vec::new(),
            texels: 0,
            compressed: 0,
            compressed_indices: 0,
            colors: 0..0,
        };
        let set = named(
            &["allpeak", "asphalt", "ngrass", "tree01", "seed.2", "x"],
            &["apeak", "asphalt1", "fpeak", "grass", "seed_pl", "tree01"],
        );
        let pairs: Vec<&str> = set
            .textures
            .iter()
            .map(|texture| set.palette_for(texture).unwrap().name.as_str())
            .collect();
        assert_eq!(
            pairs,
            ["apeak", "asphalt1", "grass", "tree01", "seed_pl", "apeak"]
        );
    }

    #[test]
    fn decodes_alpha_direct_and_compressed_textures() {
        let a3i5 = [0xE1u8; 64]; // index 1, full alpha
        let direct: Vec<u8> = (0..64)
            .flat_map(|i| if i == 0 { [0, 0] } else { [0x1F, 0x80] })
            .collect();
        let bytes = tex0(
            &[("glass", 1, 0, &a3i5), ("photo", 7, 0, &direct)],
            ("glass", &[0, BLUE]),
        );
        let textures = Textures::from_block(&Reader::new("test", &block(b"TEX0", &bytes))).unwrap();
        let glass = textures
            .image(&textures.textures[0], textures.palettes.first())
            .unwrap();
        assert_eq!(glass.get(3, 3), Some([0, 0, 255, 255]));
        let photo = textures.image(&textures.textures[1], None).unwrap();
        assert_eq!(
            (photo.get(0, 0), photo.get(1, 0)),
            (Some([0; 4]), Some([255, 0, 0, 255]))
        );

        // One compressed 8×8 texture in mode 1: the third color mixes the first two.
        let mut compressed = textures.clone();
        compressed.textures[0].format = TextureFormat::Compressed;
        let mut block_bytes = compressed.block.clone();
        let (texels, indices) = (block_bytes.len(), block_bytes.len() + 16);
        block_bytes.extend([0b1010_1010; 16]); // four blocks of index 2
        block_bytes.extend([0x00, 0x40].repeat(4)); // mode 1, colors 0 and 1
        compressed.block = block_bytes;
        compressed.compressed = texels;
        compressed.compressed_indices = indices;
        let image = compressed
            .image(&compressed.textures[0], compressed.palettes.first())
            .unwrap();
        assert_eq!(image.get(7, 7), Some(bgr555(0x3C00)));
    }

    #[test]
    fn rejects_texels_outside_the_block() {
        let bytes = tex0(&[("short", 3, 3, &[0; 8])], ("short", &[RED]));
        let textures = Textures::from_block(&Reader::new("test", &block(b"TEX0", &bytes))).unwrap();
        assert_eq!(textures.textures[0].width, 64);
        assert!(matches!(
            textures.image(&textures.textures[0], textures.palettes.first()),
            Err(Error::Malformed { .. })
        ));
    }
}
