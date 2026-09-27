//! Pokémon sprites: pics colored with the palette the game gives each species on the Super
//! Game Boy (and, in Yellow, on the Game Boy Color).
//!
//! A front pic is 5×5, 6×6, or 7×7 tiles; the game shows it in a 7×7-tile box, at the bottom
//! and centered (rounding to the right). Back pics are 4×4 tiles, which the game shows at twice
//! their size; romkit keeps their pixels as stored.

use super::{Bank, Gen1, SpeciesId, cached, read};
use crate::io::ByteSource;
use crate::media::{Image, Rgba, bgr555};
use crate::{Error, Result};

use super::pic::{self, Pic};

/// The box front pics are shown in, in tiles.
pub(crate) const FRONT_BOX: u8 = 7;

impl<S: ByteSource> Gen1<S> {
    /// A species' front sprite, 56×56, or its back sprite, 32×32, with a transparent
    /// background.
    pub fn pokemon_sprite(&self, species: SpeciesId, back: bool) -> Result<Image> {
        let not_found = || Error::NotFound(format!("sprite of species {}", species.0));
        let record = usize::from(species.0)
            .checked_sub(1)
            .and_then(|index| self.species().ok()?.get(index))
            .ok_or_else(not_found)?;
        let bank = self.edition.pic_banks.bank(record.index);
        let address = if back {
            record.pics.back
        } else {
            record.pics.front
        };
        let pic = pic::decompress(Bank::read(&self.source, bank)?.from(address)?)?;
        let palette = self.palette(species)?;
        if back {
            let (width, height) = (u32::from(pic.width) * 8, u32::from(pic.height) * 8);
            return Ok(Image::from_indices(
                width,
                height,
                &pic.pixels,
                &palette,
                true,
            ));
        }
        let pixels = in_front_box(&pic)?;
        let side = u32::from(FRONT_BOX) * 8;
        Ok(Image::from_indices(side, side, &pixels, &palette, true))
    }

    /// The four colors of a species' palette.
    fn palette(&self, species: SpeciesId) -> Result<[Rgba; 4]> {
        let palettes = cached(&self.palettes, || {
            let edition = self.edition;
            let ids = read(&self.source, edition.palette_ids, super::PALETTE_ENTRIES)?;
            let highest = ids.iter().copied().max().unwrap_or(0);
            let colors = read(
                &self.source,
                edition.palettes,
                (usize::from(highest) + 1) * 8,
            )?;
            Ok(ids
                .iter()
                .map(|&id| {
                    let colors = &colors[usize::from(id) * 8..][..8];
                    std::array::from_fn(|index| {
                        bgr555(u16::from_le_bytes([
                            colors[index * 2],
                            colors[index * 2 + 1],
                        ]))
                    })
                })
                .collect::<Vec<_>>())
        })?;
        palettes
            .get(usize::from(species.0))
            .copied()
            .ok_or_else(|| Error::NotFound(format!("palette of species {}", species.0)))
    }
}

impl super::PicBanks {
    /// The bank that holds the pics of the species with internal index `index`.
    pub(super) fn bank(&self, index: u8) -> u8 {
        match self.mew {
            Some(bank) if index == super::MEW => bank,
            _ => self
                .ranges
                .iter()
                .find(|&&(last, _)| index <= last)
                .or(self.ranges.last())
                .map_or(0, |&(_, bank)| bank),
        }
    }
}

/// A front pic's pixels placed in the 7×7-tile box: at the bottom, centered.
pub(crate) fn in_front_box(pic: &Pic) -> Result<Vec<u8>> {
    if pic.width > FRONT_BOX || pic.height > FRONT_BOX {
        return Err(Error::malformed(
            "Gen 1 pic",
            0,
            "front pic larger than 7×7 tiles",
        ));
    }
    let side = usize::from(FRONT_BOX) * 8;
    let width = usize::from(pic.width) * 8;
    let left = usize::from((FRONT_BOX + 1 - pic.width) / 2) * 8;
    let top = usize::from(FRONT_BOX - pic.height) * 8;
    let mut pixels = vec![0; side * side];
    for (row, line) in pic.pixels.chunks_exact(width).enumerate() {
        let start = (top + row) * side + left;
        pixels[start..start + width].copy_from_slice(line);
    }
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_pics_sit_at_the_bottom_of_the_box_centered() {
        let pic = Pic {
            width: 5,
            height: 5,
            pixels: vec![3; 40 * 40],
        };
        let pixels = in_front_box(&pic).unwrap();
        // One blank tile column on the left, two on the right, two blank tile rows on top.
        let at = |x: usize, y: usize| pixels[y * 56 + x];
        assert_eq!((at(7, 55), at(8, 55), at(47, 16), at(48, 55)), (0, 3, 3, 0));
        assert_eq!(at(8, 15), 0);
        let wide = Pic {
            width: 8,
            height: 1,
            pixels: vec![0; 64 * 8],
        };
        assert!(in_front_box(&wide).is_err());
    }

    #[test]
    fn pic_banks_follow_index_ranges() {
        let banks = super::super::RED_BLUE.pic_banks;
        assert_eq!(banks.bank(0x1E), 0x09);
        assert_eq!(banks.bank(0x1F), 0x0A);
        assert_eq!(banks.bank(super::super::MEW), 0x01);
        assert_eq!(banks.bank(0xBE), 0x0D);
    }
}
