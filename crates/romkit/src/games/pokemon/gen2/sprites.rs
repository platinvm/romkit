//! Pokémon sprites: pics compressed with [`lz`](super::lz), colored with each species' normal
//! or shiny Game Boy Color palette.
//!
//! The pic table has two far pointers per species, front then back; Unown has a table of its
//! own with one entry per letter. A pic is 2bpp tiles in columns, top to bottom then left to
//! right. Front pics are as large as the species' record says (5×5 to 7×7 tiles), followed by
//! the extra tiles of their animation, and are shown in a 7×7-tile box like Gen 1's; back pics
//! are 6×6 tiles. Palettes give the two middle colors; the outer two are white and black.

use super::{Gen2, SpeciesId, cached, lz, read};
use crate::games::pokemon::gen1::pic::Pic;
use crate::games::pokemon::gen1::{Bank, in_front_box};
use crate::io::{ByteSource, Reader};
use crate::media::{Image, Rgba, bgr555};
use crate::platforms::gb::FarPointer;
use crate::{Error, Result};

const UNOWN: u16 = 201;
const BACK_SIZE: u8 = 6;
const WHITE: Rgba = [0xFF, 0xFF, 0xFF, 0xFF];
const BLACK: Rgba = [0, 0, 0, 0xFF];

impl<S: ByteSource> Gen2<S> {
    /// A species' front sprite, 56×56, or back sprite, 48×48, in its normal or shiny colors,
    /// with a transparent background. Unown is shown as its letter A.
    pub fn pokemon_sprite(&self, species: SpeciesId, back: bool, shiny: bool) -> Result<Image> {
        let not_found = || Error::NotFound(format!("sprite of species {}", species.0));
        let record = usize::from(species.0)
            .checked_sub(1)
            .and_then(|index| self.species().ok()?.get(index))
            .ok_or_else(not_found)?;
        let edition = self.edition;
        let entry = match species.0 {
            UNOWN => edition.unown_pics,
            id => FarPointer::new(edition.pics.bank, edition.pics.address + (id - 1) * 6),
        };
        let entry = read(&self.source, entry, 6)?;
        let pointer = Reader::new("Gen 2 pic table", &entry[if back { 3 } else { 0 }..]);
        let bank = edition.pic_banks.actual(pointer.u8_at(0)?);
        let tiles = lz::decompress(Bank::read(&self.source, bank)?.from(pointer.u16_at(1)?)?)?;
        let (width, height) = match back {
            true => (BACK_SIZE, BACK_SIZE),
            false => record.pic_size,
        };
        let pic = untile(&tiles, width, height)?;
        let [light, dark] = self.palettes()?[usize::from(species.0)][usize::from(shiny)];
        let palette = [WHITE, light, dark, BLACK];
        if back {
            let side = u32::from(BACK_SIZE) * 8;
            return Ok(Image::from_indices(side, side, &pic.pixels, &palette, true));
        }
        let side = 7 * 8;
        Ok(Image::from_indices(
            side,
            side,
            &in_front_box(&pic)?,
            &palette,
            true,
        ))
    }

    /// Per species from the placeholder 0: the two middle colors, normal then shiny.
    fn palettes(&self) -> Result<&[[[Rgba; 2]; 2]]> {
        cached(&self.palettes, || {
            let bytes = read(
                &self.source,
                self.edition.palettes,
                (super::SPECIES + 1) * 8,
            )?;
            let color = |at: &[u8]| bgr555(u16::from_le_bytes([at[0], at[1]]));
            Ok(bytes
                .as_chunks::<8>()
                .0
                .iter()
                .map(|entry| {
                    [
                        [color(&entry[0..]), color(&entry[2..])],
                        [color(&entry[4..]), color(&entry[6..])],
                    ]
                })
                .collect())
        })
        .map(Vec::as_slice)
    }
}

/// The first `width × height` tiles of 2bpp data, in columns, as pixels row by row. Each tile
/// row is two bytes, the low bits of its 8 pixels then the high bits.
fn untile(tiles: &[u8], width: u8, height: u8) -> Result<Pic> {
    let (columns, rows) = (usize::from(width), usize::from(height));
    if tiles.len() < columns * rows * 16 {
        return Err(Error::malformed(
            "Gen 2 pic",
            0,
            "fewer tiles than its size",
        ));
    }
    let line = columns * 8;
    let mut pixels = vec![0; line * rows * 8];
    for (tile, bytes) in tiles
        .as_chunks::<16>()
        .0
        .iter()
        .take(columns * rows)
        .enumerate()
    {
        let (x, y) = (tile / rows * 8, tile % rows * 8);
        for (row, plane) in bytes.as_chunks::<2>().0.iter().enumerate() {
            for bit in 0..8 {
                let (low, high) = (plane[0] >> (7 - bit) & 1, plane[1] >> (7 - bit) & 1);
                pixels[(y + row) * line + x + bit] = low | high << 1;
            }
        }
    }
    Ok(Pic {
        width,
        height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_fill_columns_first() {
        // Two tiles in a 1×2 column: the first all color 1, the second color 2 on row 0.
        let mut tiles = vec![0; 32];
        tiles[..16]
            .iter_mut()
            .step_by(2)
            .for_each(|low| *low = 0xFF);
        tiles[17] = 0x80;
        let pic = untile(&tiles, 1, 2).unwrap();
        assert_eq!((pic.pixels[0], pic.pixels[7 * 8 + 7]), (1, 1));
        assert_eq!((pic.pixels[8 * 8], pic.pixels[8 * 8 + 1]), (2, 0));
        assert!(untile(&tiles, 2, 2).is_err());
    }
}
