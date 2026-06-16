//! Sprites: Pokémon battle sprites and box icons, trainer sprites, and item icons.
//!
//! Battle and trainer sprites are 160×80 bitmaps holding two animation frames side by side, of
//! which the first is the sprite. Their pixels are encrypted: each 16-bit word is XORed with
//! the low half of a state that starts as the seed word and advances as the games' random
//! number generator (`state * 0x41C64E6D + 0x6073`). Diamond and Pearl seed with the last word
//! and go backward, as do HeartGold and SoulSilver for trainers; Platinum, and HeartGold and
//! SoulSilver for Pokémon, seed with the first and go forward. Either way the seed word
//! decrypts to 0, a transparent corner.
//!
//! The battle sprite archive has six members per species: the female and male back sprites,
//! the female and male front sprites, then the normal and shiny palettes. A species that looks
//! the same either way has the same bytes in both slots; a single-gender one leaves the other
//! slot empty.
//!
//! Icons and item icons are 32×32 tiles in 1D order. The palette bank of each Pokémon icon and
//! the icon of each item are tables in the ARM9 binary, found by their first entries.

use super::{Gen4, ItemId, SpeciesId, cached};
use crate::formats::g2d::{Palette, Tiles};
use crate::formats::narc::Narc;
use crate::io::ByteSource;
use crate::media::Image;
use crate::{Error, Result};

const SPRITE_SIZE: u32 = 80;
const ICON_TILES: (u16, u16) = (4, 4);
/// Icons follow a palette and the cells and animations that draw them.
const FIRST_ICON: usize = 7;
/// The icon palette bank table starts with the placeholder, then Bulbasaur to Pikachu.
const ICON_BANKS_START: [u8; 26] = [
    0, 1, 1, 1, 0, 0, 0, 0, 2, 2, 1, 1, 0, 1, 2, 2, 0, 0, 0, 2, 1, 0, 0, 2, 2, 2,
];
/// The item table has four 16-bit fields per item: data member, icon tiles member, icon palette
/// member, and Gen 3 item number. Items 1 and 2, the Master and Ultra Balls, start it after the
/// placeholder.
const ITEMS_START: [u8; 16] = [1, 0, 2, 0, 3, 0, 1, 0, 2, 0, 4, 0, 5, 0, 2, 0];
const ITEM_LEN: usize = 8;

/// Where an edition keeps its sprites.
#[derive(Debug)]
pub(super) struct Archives {
    /// Battle sprites, six members per species.
    pub pokemon: &'static str,
    pub pokemon_encryption: Encryption,
    pub trainers: Layout,
    /// Box icons: their palette, cells and animations, then one member per icon.
    pub icons: &'static str,
    /// Item icons, as tiles and palette pairs.
    pub items: &'static str,
}

/// Where an archive keeps each record's sprite: record `n` has its tiles at member
/// `n * stride + tiles` and its palette at `n * stride + palette`.
#[derive(Debug)]
pub(super) struct Layout {
    pub path: &'static str,
    pub stride: usize,
    pub tiles: usize,
    pub palette: usize,
    pub encryption: Encryption,
}

/// Which end of the pixel data seeds the decryption of battle and trainer sprites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Encryption {
    FromEnd,
    FromStart,
}

/// A battle sprite: front or back, normal or shiny colors, male or female.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Variant {
    pub back: bool,
    pub shiny: bool,
    pub female: bool,
}

impl Variant {
    pub const ALL: [Variant; 8] = [
        Self::new(false, false, false),
        Self::new(false, true, false),
        Self::new(false, false, true),
        Self::new(false, true, true),
        Self::new(true, false, false),
        Self::new(true, true, false),
        Self::new(true, false, true),
        Self::new(true, true, true),
    ];

    const fn new(back: bool, shiny: bool, female: bool) -> Self {
        Self {
            back,
            shiny,
            female,
        }
    }

    /// Its asset directory, e.g. `front-shiny-female`.
    pub fn name(self) -> String {
        let side = if self.back { "back" } else { "front" };
        let shiny = if self.shiny { "-shiny" } else { "" };
        let female = if self.female { "-female" } else { "" };
        format!("{side}{shiny}{female}")
    }
}

/// The sprite archives' member tables and the code tables that go with them, read once.
pub(super) struct Index {
    pokemon: Narc,
    /// Per species from 0: whether its female front and back sprites differ from the male ones.
    female: Vec<[bool; 2]>,
    trainers: Narc,
    icons: Narc,
    /// Per icon: its palette bank.
    icon_banks: Vec<u8>,
    item_icons: Narc,
    /// Per item from 0: its icon's tiles and palette members.
    items: Vec<(usize, usize)>,
}

impl<S: ByteSource> Gen4<S> {
    /// A species' battle sprite, 80×80. Species without a distinct female sprite give the
    /// male one for either sex, and single-gender species give theirs.
    pub fn pokemon_sprite(&self, species: SpeciesId, variant: Variant) -> Result<Image> {
        let index = self.sprite_index()?;
        let species = species.0 as usize;
        let base = species * 6;
        if species == 0 || base + 6 > index.pokemon.len() {
            return Err(Error::NotFound(format!("sprite of species {species}")));
        }
        let (female, male) = match variant.back {
            true => (base, base + 1),
            false => (base + 2, base + 3),
        };
        let filled = |member: usize| index.pokemon.range(member).is_ok_and(|r| !r.is_empty());
        let tiles = match variant.female && filled(female) || !filled(male) {
            true => female,
            false => male,
        };
        let palette = base + 4 + variant.shiny as usize;
        let archives = &self.edition.sprites;
        let archive = self.rom.file(archives.pokemon)?;
        sprite(
            &index.pokemon,
            &archive,
            (tiles, palette),
            archives.pokemon_encryption,
        )
    }

    /// A trainer class's battle sprite, 80×80.
    pub fn trainer_sprite(&self, class: usize) -> Result<Image> {
        let (index, layout) = (self.sprite_index()?, &self.edition.sprites.trainers);
        if class >= index.trainers.len() / layout.stride {
            return Err(Error::NotFound(format!("sprite of trainer class {class}")));
        }
        let base = class * layout.stride;
        let archive = self.rom.file(layout.path)?;
        let members = (base + layout.tiles, base + layout.palette);
        sprite(&index.trainers, &archive, members, layout.encryption)
    }

    /// A species' box icon, 32×32.
    pub fn pokemon_icon(&self, species: SpeciesId) -> Result<Image> {
        let index = self.sprite_index()?;
        let species = species.0 as usize;
        let bank = match index.icon_banks.get(species) {
            Some(&bank) if species > 0 => bank,
            _ => return Err(Error::NotFound(format!("icon of species {species}"))),
        };
        let archive = self.rom.file(self.edition.sprites.icons)?;
        icon(&index.icons, &archive, FIRST_ICON + species, 0, bank)
    }

    /// An item's icon, 32×32.
    pub fn item_icon(&self, item: ItemId) -> Result<Image> {
        let index = self.sprite_index()?;
        let &(tiles, palette) = index
            .items
            .get(item.0 as usize)
            .filter(|_| item.0 > 0)
            .ok_or_else(|| Error::NotFound(format!("icon of item {}", item.0)))?;
        let archive = self.rom.file(self.edition.sprites.items)?;
        icon(&index.item_icons, &archive, tiles, palette, 0)
    }

    /// Every sprite asset's path: listing reads the archives' tables, the code tables, and the
    /// female battle sprites to compare them, but decodes nothing.
    pub(super) fn sprite_assets(&self) -> Result<Vec<String>> {
        let index = self.sprite_index()?;
        let species = 1..index.pokemon.len() / 6;
        let mut paths = Vec::new();
        for variant in Variant::ALL {
            let side = variant.back as usize;
            paths.extend(
                species
                    .clone()
                    .filter(|&species| !variant.female || index.female[species][side])
                    .map(|species| format!("sprites/pokemon/{}/{species:04}.png", variant.name())),
            );
        }
        let icons = 1..index.icon_banks.len().min(species.end);
        paths.extend(icons.map(|species| format!("sprites/icons/{species:04}.png")));
        let classes = 0..index.trainers.len() / self.edition.sprites.trainers.stride;
        paths.extend(classes.map(|class| format!("sprites/trainers/{class:04}.png")));
        let items = 1..index.items.len();
        paths.extend(items.map(|item| format!("sprites/items/{item:04}.png")));
        Ok(paths)
    }

    /// The sprite asset at `path`, encoded as PNG.
    pub(super) fn extract_sprite(&self, path: &str) -> Result<Vec<u8>> {
        let not_found = || Error::NotFound(path.to_owned());
        let (directory, file) = path.rsplit_once('/').ok_or_else(not_found)?;
        let number: u16 = file
            .strip_suffix(".png")
            .and_then(|stem| stem.parse().ok())
            .ok_or_else(not_found)?;
        let image = match directory {
            "sprites/icons" => self.pokemon_icon(SpeciesId(number))?,
            "sprites/trainers" => self.trainer_sprite(number as usize)?,
            "sprites/items" => self.item_icon(ItemId(number))?,
            _ => {
                let variant = directory
                    .strip_prefix("sprites/pokemon/")
                    .and_then(|name| Variant::ALL.into_iter().find(|v| v.name() == name))
                    .ok_or_else(not_found)?;
                let female = &self.sprite_index()?.female;
                let distinct = female
                    .get(number as usize)
                    .map(|sides| sides[variant.back as usize]);
                if variant.female && distinct != Some(true) {
                    return Err(not_found());
                }
                self.pokemon_sprite(SpeciesId(number), variant)?
            }
        };
        image.to_png()
    }

    fn sprite_index(&self) -> Result<&Index> {
        cached(&self.sprites, || {
            let archives = &self.edition.sprites;
            let pokemon_archive = self.rom.file(archives.pokemon)?;
            let pokemon = Narc::parse(&pokemon_archive)?;
            let narc = |path| Narc::parse(&self.rom.file(path)?);
            let icons = narc(archives.icons)?;
            let item_icons = narc(archives.items)?;
            let code = self.rom.arm9()?;
            Ok(Index {
                female: female_sprites(&pokemon, &pokemon_archive)?,
                pokemon,
                trainers: narc(archives.trainers.path)?,
                icon_banks: icon_banks(&code, icons.len().saturating_sub(FIRST_ICON)),
                icons,
                items: item_icons_table(&code, item_icons.len()),
                item_icons,
            })
        })
    }
}

/// Per species: whether its female front and back sprites differ from the male ones.
fn female_sprites<A: ByteSource>(pokemon: &Narc, archive: &A) -> Result<Vec<[bool; 2]>> {
    let differ = |female: usize, male: usize| -> Result<bool> {
        let (female, male) = (pokemon.range(female)?, pokemon.range(male)?);
        if female.is_empty() || male.is_empty() {
            return Ok(false);
        }
        let read = |range: std::ops::Range<u64>| archive.read(range.start, range.end - range.start);
        Ok(female.end - female.start != male.end - male.start || read(female)? != read(male)?)
    };
    (0..pokemon.len() / 6)
        .map(|species| {
            let base = species * 6;
            Ok([differ(base + 2, base + 3)?, differ(base, base + 1)?])
        })
        .collect()
}

/// The palette bank of each of `count` icons, from the table in the game's code. Empty when the
/// table is not there, as in modified games.
fn icon_banks(code: &[u8], count: usize) -> Vec<u8> {
    find(code, &ICON_BANKS_START)
        .and_then(|at| code.get(at..at + count))
        .map_or_else(Vec::new, <[u8]>::to_vec)
}

/// Each item's icon tiles and palette members, from the table in the game's code, which ends
/// where the members no longer fit an archive of `members`. Empty when the table is not there.
fn item_icons_table(code: &[u8], members: usize) -> Vec<(usize, usize)> {
    let Some(start) = find(code, &ITEMS_START).and_then(|at| at.checked_sub(ITEM_LEN)) else {
        return Vec::new();
    };
    code[start..]
        .as_chunks::<ITEM_LEN>()
        .0
        .iter()
        .map(|item| {
            let field = |at: usize| u16::from_le_bytes([item[at], item[at + 1]]) as usize;
            (field(2), field(4))
        })
        .take_while(|&(tiles, palette)| tiles < members && palette < members)
        .collect()
}

/// A battle or trainer sprite from its tiles and palette members: its first frame, decrypted.
fn sprite<A: ByteSource>(
    narc: &Narc,
    archive: &A,
    (tiles, palette): (usize, usize),
    encryption: Encryption,
) -> Result<Image> {
    let mut tiles = Tiles::parse(&narc.member(archive, tiles)?)?;
    decrypt(&mut tiles.data, encryption);
    let palette = Palette::parse(&narc.member(archive, palette)?)?;
    Ok(crop(tiles.image(&palette, 0), SPRITE_SIZE, SPRITE_SIZE))
}

/// A 32×32 icon: the first frame of its tiles, in 1D order.
fn icon<A: ByteSource>(
    narc: &Narc,
    archive: &A,
    tiles: usize,
    palette: usize,
    bank: u8,
) -> Result<Image> {
    let mut tiles = Tiles::parse(&narc.member(archive, tiles)?)?;
    tiles.size = Some(ICON_TILES);
    let palette = Palette::parse(&narc.member(archive, palette)?)?;
    Ok(tiles.image(&palette, bank))
}

/// Undo the sprite encryption described in the module documentation.
fn decrypt(data: &mut [u8], encryption: Encryption) {
    let words = data.as_chunks_mut::<2>().0;
    if encryption == Encryption::FromEnd {
        words.reverse();
    }
    let mut state = words
        .first()
        .map_or(0, |&word| u16::from_le_bytes(word) as u32);
    for word in words.iter_mut() {
        *word = (u16::from_le_bytes(*word) ^ state as u16).to_le_bytes();
        state = state.wrapping_mul(0x41C6_4E6D).wrapping_add(0x6073);
    }
    if encryption == Encryption::FromEnd {
        words.reverse();
    }
}

/// The top left `width` × `height` of `image`.
fn crop(image: Image, width: u32, height: u32) -> Image {
    let mut cropped = Image::new(width.min(image.width), height.min(image.height));
    cropped.draw(&image, 0, 0);
    cropped
}

/// Where `needle` first occurs in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encrypt as the games do: the inverse of [`decrypt`], seeded with `seed`.
    fn encrypt(words: &[u16], seed: u16, encryption: Encryption) -> Vec<u8> {
        let mut words = words.to_vec();
        if encryption == Encryption::FromEnd {
            words.reverse();
        }
        let mut state = seed as u32;
        for word in &mut words {
            *word ^= state as u16;
            state = state.wrapping_mul(0x41C6_4E6D).wrapping_add(0x6073);
        }
        if encryption == Encryption::FromEnd {
            words.reverse();
        }
        words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }

    #[test]
    fn decrypts_from_either_end() {
        // Sprites start and end with transparent pixels, which encrypt to the seed.
        let plain = [0, 0x1234, 0xBEEF, 0];
        for encryption in [Encryption::FromStart, Encryption::FromEnd] {
            let mut data = encrypt(&plain, 0x5A5A, encryption);
            decrypt(&mut data, encryption);
            let expected: Vec<u8> = plain.iter().flat_map(|word| word.to_le_bytes()).collect();
            assert_eq!(data, expected);
        }
    }

    #[test]
    fn finds_code_tables_by_their_first_entries() {
        let code = [&[0xAA; 3][..], &ICON_BANKS_START, &[1, 2]].concat();
        assert_eq!(icon_banks(&code, 28)[25..], [2, 1, 2]);
        assert!(icon_banks(&code, 29).is_empty());
        assert!(icon_banks(&[0; 64], 1).is_empty());

        // Item 0, then items 1 and 2, then something that is not an item.
        let placeholder = [0, 0, 9, 0, 10, 0, 0, 0];
        let code = [
            &[0xAA; 3][..],
            &placeholder,
            &ITEMS_START,
            &[0, 0, 0xFF, 0x7F, 0, 0, 0, 0],
        ];
        let items = item_icons_table(&code.concat(), 20);
        assert_eq!(items, [(9, 10), (2, 3), (4, 5)]);
        assert!(item_icons_table(&ITEMS_START, 20).is_empty());
    }

    #[test]
    fn variants_name_their_directories() {
        let names: Vec<String> = Variant::ALL.iter().map(|v| v.name()).collect();
        assert_eq!(names[0], "front");
        assert_eq!(names[3], "front-shiny-female");
        assert_eq!(names[4], "back");
    }

    #[test]
    fn crops_the_first_frame() {
        let mut image = Image::new(160, 80);
        image.set(79, 0, [1, 2, 3, 255]);
        image.set(80, 0, [4, 5, 6, 255]);
        let frame = crop(image, 80, 80);
        assert_eq!((frame.width, frame.height), (80, 80));
        assert_eq!(frame.get(79, 0), Some([1, 2, 3, 255]));
    }
}
