//! NSCR, a screen: a background as a grid of tile references, row by row. The `SCRN` chunk
//! holds the width and height in pixels, the color mode, the screen format (text or affine),
//! the data size, then the entries. Text and extended affine entries are 16 bits: tile number
//! (10 bits), horizontal and vertical flips, palette bank (4 bits). Affine entries are a bare
//! 8-bit tile number.

use super::ncgr::draw_tile;
use super::{Palette, Tiles, chunk};
use crate::Result;
use crate::io::Reader;
use crate::media::Image;

const FORMAT: &str = "NSCR";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    /// In tiles.
    pub width: u16,
    pub height: u16,
    /// One per tile, row by row, in the 16-bit form.
    pub entries: Vec<u16>,
}

impl Screen {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Reader::new(FORMAT, bytes).expect(b"RCSN")?;
        let scrn = chunk(FORMAT, bytes, b"NRCS")?;
        let (width, height) = (scrn.u16_at(0)? / 8, scrn.u16_at(2)? / 8);
        let affine = scrn.u16_at(6)? == 1;
        let size = (scrn.u32_at(8)? as usize).min(scrn.len().saturating_sub(12));
        let data = scrn.slice_at(12, size)?;
        let entries = match affine {
            true => data.iter().map(|&tile| tile as u16).collect(),
            false => data
                .as_chunks()
                .0
                .iter()
                .map(|&entry| u16::from_le_bytes(entry))
                .collect(),
        };
        Ok(Self {
            width,
            height,
            entries,
        })
    }

    /// The background drawn with `tiles` and `palette`. Index 0 is transparent, as the
    /// backdrop shows through it; entries the screen lacks stay transparent too.
    pub fn image(&self, tiles: &Tiles, palette: &Palette) -> Image {
        // Never larger than the entries, whatever the header claims.
        let width = (self.width as usize).clamp(1, self.entries.len().max(1));
        let height = (self.height as usize).min(self.entries.len().div_ceil(width));
        let mut image = Image::new(width as u32 * 8, height as u32 * 8);
        for (index, &entry) in self.entries.iter().enumerate().take(width * height) {
            let colors = tiles.colors(palette, (entry >> 12) as u8);
            let (x, y) = ((index % width) as u32 * 8, (index / width) as u32 * 8);
            let flips = (entry & 0x400 != 0, entry & 0x800 != 0);
            draw_tile(
                &mut image,
                &tiles.tile((entry & 0x3FF) as usize),
                colors,
                (x, y),
                flips,
            );
        }
        image
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::g2d::testing::{ncgr, nitro, words};

    fn nscr(width: u16, height: u16, affine: bool, data: &[u8]) -> Vec<u8> {
        let mut body = words(&[width, height, 0, affine as u16]);
        body.extend((data.len() as u32).to_le_bytes());
        body.extend(data);
        nitro(b"RCSN", &[(b"NRCS", &body)])
    }

    #[test]
    fn draws_entries_with_flips_and_banks() {
        // Tile 1 has index 1 at its top left.
        let mut data = vec![0; 64];
        data[32] = 0x01;
        let tiles = Tiles::parse(&ncgr((2, 1), 3, 0, &data)).unwrap();
        let mut colors = vec![[0; 4]; 32];
        colors[1] = [255, 0, 0, 255];
        colors[17] = [0, 0, 255, 255];
        let palette = Palette { colors };
        // Tile 1 as is, then tile 1 flipped both ways with bank 1.
        let screen = Screen::parse(&nscr(16, 8, false, &words(&[0x0001, 0x1C01]))).unwrap();
        assert_eq!((screen.width, screen.height), (2, 1));
        let image = screen.image(&tiles, &palette);
        assert_eq!((image.width, image.height), (16, 8));
        assert_eq!(image.get(0, 0), Some([255, 0, 0, 255]));
        assert_eq!(image.get(15, 7), Some([0, 0, 255, 255]));
        assert_eq!(image.get(8, 0), Some([0; 4]));
    }

    #[test]
    fn affine_entries_are_bytes() {
        let screen = Screen::parse(&nscr(16, 8, true, &[1, 0])).unwrap();
        assert_eq!(screen.entries, [1, 0]);
    }

    #[test]
    fn oversized_headers_draw_only_what_is_there() {
        let screen = Screen::parse(&nscr(0xFFF8, 0xFFF8, false, &words(&[0; 4]))).unwrap();
        let tiles = Tiles::parse(&ncgr((1, 1), 3, 0, &[0; 32])).unwrap();
        let image = screen.image(&tiles, &Palette { colors: vec![] });
        assert_eq!((image.width, image.height), (32, 8));
    }
}
