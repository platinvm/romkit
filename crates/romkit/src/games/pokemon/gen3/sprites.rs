//! Sprites: Pokémon battle sprites and box icons, trainer sprites, and item icons, all 4-bit
//! graphics in 8×8 tiles laid out row by row, with BGR555 palettes.
//!
//! Battle sprites are LZ77-compressed 64×64 sheets with compressed palettes, normal and shiny;
//! a few species keep their forms below the first (Castform, Deoxys), and only the first is
//! the sprite. The sprite tables run past the species to the Egg and Unown's other forms.
//! Icons are uncompressed 32×64, two 32×32 frames, drawn with one of six shared palettes.
//! Trainer sprites are like battle sprites; item icons are compressed 24×24 images, each with
//! its own palette.

use super::{FORMAT, Gen3, ItemId, SpeciesId};
use crate::formats::g2d::{Depth, Palette, Tiles};
use crate::io::ByteSource;
use crate::media::{Image, bgr555};
use crate::{Error, Result};

const SPRITE_TILES: (u16, u16) = (8, 8);
const ICON_TILES: (u16, u16) = (4, 4);
const ITEM_ICON_TILES: (u16, u16) = (3, 3);
const ICON_PALETTES: u8 = 6;
const TILE_LEN: usize = 32;

/// A battle sprite: front or back, normal or shiny colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Variant {
    pub back: bool,
    pub shiny: bool,
}

impl Variant {
    pub const ALL: [Variant; 4] = [
        Variant {
            back: false,
            shiny: false,
        },
        Variant {
            back: false,
            shiny: true,
        },
        Variant {
            back: true,
            shiny: false,
        },
        Variant {
            back: true,
            shiny: true,
        },
    ];

    /// Its asset directory, e.g. `front-shiny`.
    pub fn name(self) -> &'static str {
        match (self.back, self.shiny) {
            (false, false) => "front",
            (false, true) => "front-shiny",
            (true, false) => "back",
            (true, true) => "back-shiny",
        }
    }
}

impl<S: ByteSource> Gen3<S> {
    /// A species' battle sprite, 64×64.
    pub fn pokemon_sprite(&self, species: SpeciesId, variant: Variant) -> Result<Image> {
        let index = self.internal_index(species)? as u32;
        let edition = self.edition;
        let sheets = match variant.back {
            true => edition.back_sprites,
            false => edition.front_sprites,
        };
        let palettes = match variant.shiny {
            true => edition.shiny_palettes,
            false => edition.palettes,
        };
        let palette = self.compressed_at(palettes + 8 * index)?;
        Ok(tiled(
            &self.compressed_at(sheets + 8 * index)?,
            SPRITE_TILES,
            &palette,
        ))
    }

    /// A species' box icon, the first of its two 32×32 frames.
    pub fn pokemon_icon(&self, species: SpeciesId) -> Result<Image> {
        let index = self.internal_index(species)? as u32;
        let edition = self.edition;
        let number = self.read(edition.icon_palette_numbers + index, 1)?[0];
        if number >= ICON_PALETTES {
            return Err(Error::malformed(
                FORMAT,
                (edition.icon_palette_numbers + index).into(),
                "no such icon palette",
            ));
        }
        let palette = self.read(self.follow(edition.icon_palettes + 8 * number as u32)?, 32)?;
        let tiles = ICON_TILES.0 as usize * ICON_TILES.1 as usize * TILE_LEN;
        let pixels = self.read(self.follow(edition.icons + 4 * index)?, tiles)?;
        Ok(tiled(&pixels, ICON_TILES, &palette))
    }

    /// The number of trainer sprites.
    pub fn trainer_sprite_count(&self) -> u16 {
        self.edition.trainer_count
    }

    /// A trainer class's sprite, 64×64, by its number from 0.
    pub fn trainer_sprite(&self, number: u16) -> Result<Image> {
        let edition = self.edition;
        if number >= edition.trainer_count {
            return Err(Error::NotFound(format!("trainer sprite {number}")));
        }
        let at = 8 * number as u32;
        let palette = self.compressed_at(edition.trainer_palettes + at)?;
        Ok(tiled(
            &self.compressed_at(edition.trainer_sprites + at)?,
            SPRITE_TILES,
            &palette,
        ))
    }

    /// Whether the game has item icons; Ruby and Sapphire's bag shows none.
    pub fn has_item_icons(&self) -> bool {
        self.edition.item_icons.is_some()
    }

    /// An item's icon, 24×24. Item 0's is the question mark shown for unknown items.
    pub fn item_icon(&self, item: ItemId) -> Result<Image> {
        let edition = self.edition;
        let table = edition
            .item_icons
            .filter(|_| item.0 < edition.item_count)
            .ok_or_else(|| Error::NotFound(format!("icon of item {}", item.0)))?;
        let at = table + 8 * item.0 as u32;
        let palette = self.compressed_at(at + 4)?;
        Ok(tiled(&self.compressed_at(at)?, ITEM_ICON_TILES, &palette))
    }
}

/// The first `width` × `height` tiles of 4-bit `pixels`, colored by a stored palette, with
/// index 0 transparent.
fn tiled(pixels: &[u8], (width, height): (u16, u16), palette: &[u8]) -> Image {
    let len = width as usize * height as usize * TILE_LEN;
    let tiles = Tiles {
        size: Some((width, height)),
        depth: Depth::Four,
        step: None,
        tiled: true,
        data: pixels[..len.min(pixels.len())].to_vec(),
    };
    let colors = palette
        .as_chunks()
        .0
        .iter()
        .map(|&color| bgr555(u16::from_le_bytes(color)))
        .collect();
    tiles.image(&Palette { colors }, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_are_laid_out_row_by_row() {
        // Two 8×8 tiles side by side: the first all index 1, the second all index 2.
        let pixels = [[0x11u8; 32], [0x22; 32]].concat();
        let palette = [0u16, 0x001F, 0x03E0].map(u16::to_le_bytes).concat();
        let image = tiled(&pixels, (2, 1), &palette);
        assert_eq!((image.width, image.height), (16, 8));
        assert_eq!(image.get(7, 7), Some([255, 0, 0, 255]));
        assert_eq!(image.get(8, 0), Some([0, 255, 0, 255]));
        // A short sheet makes a smaller image, never a panic.
        assert_eq!(tiled(&pixels[..32], SPRITE_TILES, &palette).width, 8);
        assert_eq!(Variant::ALL.map(Variant::name)[3], "back-shiny");
    }
}
