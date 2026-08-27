//! NCGR, character graphics: palette indices in a `CHAR` chunk (height and width in tiles,
//! `0xFFFF` when the graphics are only parts for cells; bit depth; VRAM mapping; whether the
//! pixels are tiled or a plain bitmap; data size and offset). 4-bit indices pack two pixels in
//! a byte, the left one in the low half.

use super::{Palette, chunk};
use crate::Result;
use crate::io::Reader;
use crate::media::{Image, Rgba};

const FORMAT: &str = "NCGR";

raw_enum! {
    /// Bits per pixel, as the hardware's color modes name them.
    pub enum Depth: u32 {
        /// 16 colors: a pixel picks one color in a palette bank.
        Four = 3,
        /// 256 colors.
        Eight = 4,
    }
}

impl Depth {
    /// Bytes per 8×8 tile.
    pub fn tile_len(self) -> usize {
        match self {
            Depth::Eight => 64,
            _ => 32,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tiles {
    /// In tiles; `None` when the file does not say, as for parts of cells.
    pub size: Option<(u16, u16)>,
    pub depth: Depth,
    /// How cells number these tiles: the size of a tile number's step in bytes, or `None` for
    /// the 2D mapping, where tiles form a sheet 32 4-bit tiles wide.
    pub step: Option<usize>,
    /// Pixels in 8×8 tiles, row by row, rather than a plain bitmap.
    pub tiled: bool,
    /// The pixel data as stored. Some games encrypt it, so it is left for them to decrypt.
    pub data: Vec<u8>,
}

impl Tiles {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Reader::new(FORMAT, bytes).expect(b"RGCN")?;
        let char = chunk(FORMAT, bytes, b"RAHC")?;
        let (height, width) = (char.u16_at(0)?, char.u16_at(2)?);
        let mapping = char.u32_at(8)?;
        let size = char.u32_at(16)? as usize;
        let offset = char.u32_at(20)? as usize;
        Ok(Self {
            size: (width != 0xFFFF && height != 0xFFFF).then_some((width, height)),
            depth: Depth::from_raw(char.u32_at(4)?),
            // Mapping values are GX_OBJVRAMMODE_CHAR_*: bit 4 set for 1D, bits 20–21 the
            // step as a power of two times 32 bytes.
            step: (mapping & 0x10 != 0).then(|| 32 << (mapping >> 20 & 3)),
            tiled: char.u32_at(12)? & 0xFF == 0,
            data: char
                .slice_at(offset, size.min(char.len().saturating_sub(offset)))?
                .to_vec(),
        })
    }

    /// The palette index of every pixel, in storage order.
    pub fn indices(&self) -> Vec<u8> {
        match self.depth {
            Depth::Eight => self.data.clone(),
            _ => self
                .data
                .iter()
                .flat_map(|&byte| [byte & 0xF, byte >> 4])
                .collect(),
        }
    }

    /// The palette indices of tile `number` (counted in tiles of this depth), row by row;
    /// zeros past the end of the data.
    pub fn tile(&self, number: usize) -> [u8; 64] {
        let len = self.depth.tile_len();
        let mut tile = [0; 64];
        let start = number.saturating_mul(len);
        if let Some(bytes) = self.data.get(start..start.saturating_add(len)) {
            match self.depth {
                Depth::Eight => tile.copy_from_slice(bytes),
                _ => {
                    for (pair, &byte) in tile.as_chunks_mut().0.iter_mut().zip(bytes) {
                        *pair = [byte & 0xF, byte >> 4];
                    }
                }
            }
        }
        tile
    }

    /// The whole graphic as an image: `size` wide and high in tiles, or when the file does not
    /// say, 32 tiles wide (a DS screen) and as high as the data. Index 0 is transparent; 4-bit
    /// graphics use palette bank `bank`.
    pub fn image(&self, palette: &Palette, bank: u8) -> Image {
        let tiles = self.data.len() / self.depth.tile_len();
        let width = match self.size {
            Some((width, _)) => width as usize,
            None => 32,
        };
        // Never larger than the data, whatever the header claims.
        let width = width.clamp(1, tiles.max(1));
        let height = match self.size {
            Some((_, height)) => (height as usize).min(tiles.div_ceil(width)),
            None => tiles.div_ceil(width),
        };
        let (width, height) = (width as u32 * 8, height as u32 * 8);
        let colors = self.colors(palette, bank);
        if !self.tiled {
            return Image::from_indices(width, height, &self.indices(), colors, true);
        }
        let mut image = Image::new(width, height);
        for number in 0..tiles {
            let (x, y) = (number as u32 % (width / 8), number as u32 / (width / 8));
            draw_tile(
                &mut image,
                &self.tile(number),
                colors,
                (x * 8, y * 8),
                (false, false),
            );
        }
        image
    }

    /// The colors a pixel index selects from.
    pub(super) fn colors<'p>(&self, palette: &'p Palette, bank: u8) -> &'p [Rgba] {
        match self.depth {
            Depth::Eight => &palette.colors,
            _ => palette.bank(bank),
        }
    }
}

/// Draw one tile of indices at `(x, y)`, flipped as asked; index 0 is transparent.
pub(super) fn draw_tile(
    image: &mut Image,
    tile: &[u8; 64],
    colors: &[Rgba],
    (x, y): (u32, u32),
    (flip_x, flip_y): (bool, bool),
) {
    for row in 0..8 {
        for column in 0..8 {
            let index = tile[row * 8 + column];
            let color = match colors.get(index as usize) {
                Some(&color) if index != 0 => color,
                _ => continue,
            };
            let tx = if flip_x { 7 - column } else { column } as u32;
            let ty = if flip_y { 7 - row } else { row } as u32;
            image.set(x + tx, y + ty, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::g2d::testing::ncgr;

    const COLORS: [Rgba; 3] = [[9, 9, 9, 255], [255, 0, 0, 255], [0, 255, 0, 255]];

    fn palette() -> Palette {
        Palette {
            colors: COLORS.to_vec(),
        }
    }

    #[test]
    fn decodes_tiled_4_bit_graphics() {
        // Two tiles side by side: the first has a red pixel at (1, 0), the second green at (0, 1).
        let mut data = vec![0; 64];
        data[0] = 0x10;
        data[32 + 4] = 0x02;
        let tiles = Tiles::parse(&ncgr((2, 1), 3, 0, &data)).unwrap();
        assert_eq!(
            (tiles.size, tiles.depth, tiles.tiled),
            (Some((2, 1)), Depth::Four, true)
        );
        assert_eq!(tiles.step, Some(32));
        let image = tiles.image(&palette(), 0);
        assert_eq!((image.width, image.height), (16, 8));
        assert_eq!(image.get(0, 0), Some([0; 4]));
        assert_eq!(image.get(1, 0), Some(COLORS[1]));
        assert_eq!(image.get(8, 1), Some(COLORS[2]));
    }

    #[test]
    fn decodes_bitmaps_and_8_bit_graphics() {
        let mut data = vec![0; 128];
        data[9] = 2; // (9, 0) in a 16-pixel-wide bitmap
        let tiles = Tiles::parse(&ncgr((2, 1), 4, 1, &data)).unwrap();
        assert!(!tiles.tiled);
        assert_eq!(tiles.image(&palette(), 0).get(9, 0), Some(COLORS[2]));
        assert_eq!(tiles.tile(0)[9], 2);
    }

    #[test]
    fn graphics_of_unknown_size_lay_out_as_a_screen() {
        let tiles = Tiles::parse(&ncgr((0xFFFF, 0xFFFF), 3, 0, &[0; 32 * 40])).unwrap();
        assert_eq!(tiles.size, None);
        let image = tiles.image(&palette(), 0);
        assert_eq!((image.width, image.height), (256, 16));
        assert_eq!(tiles.tile(99), [0; 64]);
    }
}
