//! Sprites: Pokémon battle sprites and box icons, trainer sprites, and item icons.
//!
//! The battle sprite archive has 20 members per entry: the front sprite and its female
//! version, the parts and cells that animate it, the same for the back, then the normal and
//! shiny palettes. Entries are the species by National Pokédex number, a few that are not
//! species (eggs, and in Black 2 and White 2 the props of Pokéstar Studios), then alternate
//! forms. A species' personal record says where its forms' entries start and how many forms
//! it has. A female member is empty unless females look different.
//!
//! The front and back members are the whole sprite, unanimated: 96×96 pixels as 8×8 tiles in
//! the order of the hardware objects that draw them, a 64×64 block, the 32×64 to its right,
//! the 64×32 below, and the 32×32 corner. Trainer sprites are the same at 80×80.
//!
//! Icons are 32×32 tiles in 1D order, two members per entry (male, then female when it
//! differs). Their palette banks, and the icon of each item, are tables in the ARM9 binary,
//! found by their first entries.

use super::{Gen5, ItemId, PERSONAL, SpeciesId, Variant, cached};
use crate::formats::g2d::{Cell, Cells, Object, Palette, Tiles};
use crate::formats::lz;
use crate::formats::narc::Narc;
use crate::io::ByteSource;
use crate::media::Image;
use crate::{Error, Result};

/// The archives every Gen 5 game keeps at the same path.
const POKEMON: &str = "a/0/0/4";
const ICONS: &str = "a/0/0/7";
const ITEM_ICONS: &str = "a/0/2/5";

/// Species with sprites of their own, by National Pokédex number.
const SPECIES: usize = 649;
const ENTRY_LEN: usize = 20;
/// Arceus's forms differ only in palette; the archive keeps those palettes apart, so its
/// forms are not exported.
const ARCEUS: usize = 493;
/// Members of a trainer class: the sprite, its parts, cells and animations, then the palette.
const TRAINER_LEN: usize = 8;
const TRAINER_PALETTE: usize = 7;

/// The blocks a static sprite's tiles fill, in order: left, top, width, height.
const POKEMON_BLOCKS: [(i16, i16, u8, u8); 4] = [
    (0, 0, 64, 64),
    (64, 0, 32, 64),
    (0, 64, 64, 32),
    (64, 64, 32, 32),
];
const TRAINER_BLOCKS: [(i16, i16, u8, u8); 6] = [
    (0, 0, 64, 64),
    (64, 0, 16, 32),
    (64, 32, 16, 32),
    (0, 64, 32, 16),
    (32, 64, 32, 16),
    (64, 64, 16, 16),
];
const ICON_TILES: (u16, u16) = (4, 4);

/// The icon palette bank table holds a byte per entry, the male icon's bank in the low half
/// and the female's in the high half. It starts with the placeholder, then Bulbasaur to
/// Pikachu.
const ICON_BANKS_START: [u8; 26] = [
    0x00, 0x11, 0x11, 0x11, 0x00, 0x00, 0x00, 0x00, 0x22, 0x22, 0x11, 0x11, 0x00, 0x11, 0x22, 0x22,
    0x00, 0x00, 0x00, 0x22, 0x11, 0x00, 0x00, 0x22, 0x22, 0x22,
];
/// The item icon table holds each item's tiles and palette members, 16 bits each. After the
/// placeholder, items 1 to 9 have consecutive members, and the Timer Ball, item 10, shares the
/// Repeat Ball's palette.
const ITEMS_START: [u16; 20] = [
    2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 19,
];

/// Where an edition keeps the sprites whose place differs between games.
#[derive(Debug)]
pub(super) struct Archives {
    /// Trainer class sprites, [`TRAINER_LEN`] members each.
    pub trainers: &'static str,
    /// The first entry of the battle sprite archive that is an alternate form.
    pub forms: usize,
    /// Icons follow palettes and the cells and animations that draw them.
    pub first_icon: usize,
}

/// The sprite archives' member tables and the code tables that go with them, read once.
pub(super) struct Index {
    pokemon: Narc,
    /// Per species from 0: where its alternate forms' entries start, and how many it has.
    forms: Vec<(usize, usize)>,
    trainers: Narc,
    icons: Narc,
    first_icon: usize,
    /// Per icon entry: its palette banks.
    icon_banks: Vec<u8>,
    item_icons: Narc,
    /// Per item from 0: its icon's tiles and palette members.
    items: Vec<(usize, usize)>,
}

impl Index {
    /// The battle sprite archive entry of a species' form, if it has one.
    fn entry(&self, species: SpeciesId, form: u8) -> Option<usize> {
        let species = species.0 as usize;
        let &(first, forms) = self.forms.get(species).filter(|_| species > 0)?;
        let entry = match form as usize {
            0 => species,
            form if form <= forms => first + form - 1,
            _ => return None,
        };
        ((entry + 1) * ENTRY_LEN <= self.pokemon.len()).then_some(entry)
    }

    /// The tiles member of an entry's front or back sprite; the female one only where it
    /// differs.
    fn sprite(&self, entry: usize, back: bool, female: bool) -> Option<usize> {
        let member = entry * ENTRY_LEN + if back { 9 } else { 0 } + female as usize;
        filled(&self.pokemon, member).then_some(member)
    }

    /// The member of an entry's icon; the female one only where it differs.
    fn icon(&self, entry: usize, female: bool) -> Option<usize> {
        let member = self.first_icon + entry * 2 + female as usize;
        (entry < self.icon_banks.len() && filled(&self.icons, member)).then_some(member)
    }
}

impl<S: ByteSource> Gen5<S> {
    /// A species' battle sprite, 96×96: form 0 for the base form. Species without a distinct
    /// female sprite give the male one for either sex.
    pub fn pokemon_sprite(&self, species: SpeciesId, form: u8, variant: Variant) -> Result<Image> {
        let index = self.sprite_index()?;
        let entry = index.entry(species, form).ok_or_else(|| {
            Error::NotFound(format!("sprite of species {} form {form}", species.0))
        })?;
        let tiles = index
            .sprite(entry, variant.back, variant.female)
            .or_else(|| index.sprite(entry, variant.back, false))
            .ok_or_else(|| Error::NotFound(format!("sprite of species {}", species.0)))?;
        let palette = entry * ENTRY_LEN + 18 + variant.shiny as usize;
        let archive = self.rom.file(POKEMON)?;
        let tiles = Tiles::parse(&member(&index.pokemon, &archive, tiles)?)?;
        let palette = Palette::parse(&member(&index.pokemon, &archive, palette)?)?;
        Ok(assemble(&tiles, &palette, &POKEMON_BLOCKS))
    }

    /// A species' box icon, 32×32: form 0 for the base form.
    pub fn pokemon_icon(&self, species: SpeciesId, form: u8, female: bool) -> Result<Image> {
        let index = self.sprite_index()?;
        let not_found = || Error::NotFound(format!("icon of species {} form {form}", species.0));
        let entry = index.entry(species, form).ok_or_else(not_found)?;
        let female = female && index.icon(entry, true).is_some();
        let tiles = index.icon(entry, female).ok_or_else(not_found)?;
        let bank = index.icon_banks[entry] >> (4 * female as u8) & 0xF;
        let archive = self.rom.file(ICONS)?;
        icon(&index.icons, &archive, tiles, 0, bank)
    }

    /// A trainer class's battle sprite, 80×80.
    pub fn trainer_sprite(&self, class: usize) -> Result<Image> {
        let index = self.sprite_index()?;
        if class >= index.trainers.len() / TRAINER_LEN {
            return Err(Error::NotFound(format!("sprite of trainer class {class}")));
        }
        let archive = self.rom.file(self.edition.sprites.trainers)?;
        let base = class * TRAINER_LEN;
        let tiles = Tiles::parse(&member(&index.trainers, &archive, base)?)?;
        let palette = member(&index.trainers, &archive, base + TRAINER_PALETTE)?;
        Ok(assemble(
            &tiles,
            &Palette::parse(&palette)?,
            &TRAINER_BLOCKS,
        ))
    }

    /// An item's icon, 32×32.
    pub fn item_icon(&self, item: ItemId) -> Result<Image> {
        let index = self.sprite_index()?;
        let &(tiles, palette) = index
            .items
            .get(item.0 as usize)
            .filter(|_| item.0 > 0)
            .ok_or_else(|| Error::NotFound(format!("icon of item {}", item.0)))?;
        let archive = self.rom.file(ITEM_ICONS)?;
        icon(&index.item_icons, &archive, tiles, palette, 0)
    }

    /// Every sprite asset's path. Listing reads the archives' member tables, the personal
    /// records, and the code tables, but decodes nothing.
    pub(super) fn sprite_assets(&self) -> Result<Vec<String>> {
        let index = self.sprite_index()?;
        let mut sprites = Vec::new();
        for species in 1..=SPECIES {
            let forms = index.forms.get(species).map_or(0, |&(_, count)| count);
            for form in 0..=forms as u8 {
                if let Some(entry) = index.entry(SpeciesId(species as u16), form) {
                    sprites.push((stem(species, form), entry));
                }
            }
        }
        let mut paths = Vec::new();
        for variant in Variant::ALL {
            paths.extend(
                sprites
                    .iter()
                    .filter(|&&(_, entry)| {
                        index.sprite(entry, variant.back, variant.female).is_some()
                    })
                    .map(|(stem, _)| format!("sprites/pokemon/{}/{stem}.png", variant.name())),
            );
        }
        for (directory, female) in [("icons", false), ("icons-female", true)] {
            paths.extend(
                sprites
                    .iter()
                    .filter(|&&(_, entry)| index.icon(entry, female).is_some())
                    .map(|(stem, _)| format!("sprites/{directory}/{stem}.png")),
            );
        }
        let classes = 0..index.trainers.len() / TRAINER_LEN;
        paths.extend(classes.map(|class| format!("sprites/trainers/{class:04}.png")));
        let items = 1..index.items.len();
        paths.extend(items.map(|item| format!("sprites/items/{item:04}.png")));
        Ok(paths)
    }

    /// The sprite asset at `path`, encoded as PNG.
    pub(super) fn extract_sprite(&self, path: &str) -> Result<Vec<u8>> {
        let not_found = || Error::NotFound(path.to_owned());
        let (directory, file) = path.rsplit_once('/').ok_or_else(not_found)?;
        let stem = file.strip_suffix(".png").ok_or_else(not_found)?;
        let (number, form) = match stem.split_once('-') {
            Some((number, form)) => (number, form.parse().ok().filter(|&form| form > 0)),
            None => (stem, Some(0)),
        };
        let (number, form): (u16, u8) = number.parse().ok().zip(form).ok_or_else(not_found)?;
        // Only the paths that listing gives, so each image has one name.
        if stem != self::stem(number as usize, form) {
            return Err(not_found());
        }
        let index = self.sprite_index()?;
        let (species, entry) = (SpeciesId(number), index.entry(SpeciesId(number), form));
        // Female variants exist only where females look different.
        let image = match directory {
            "sprites/trainers" if form == 0 => self.trainer_sprite(number as usize)?,
            "sprites/items" if form == 0 => self.item_icon(ItemId(number))?,
            "sprites/icons" => self.pokemon_icon(species, form, false)?,
            "sprites/icons-female" if entry.and_then(|e| index.icon(e, true)).is_some() => {
                self.pokemon_icon(species, form, true)?
            }
            _ => {
                let variant = directory
                    .strip_prefix("sprites/pokemon/")
                    .and_then(|name| Variant::ALL.into_iter().find(|v| v.name() == name))
                    .ok_or_else(not_found)?;
                if variant.female
                    && entry
                        .and_then(|e| index.sprite(e, variant.back, true))
                        .is_none()
                {
                    return Err(not_found());
                }
                self.pokemon_sprite(species, form, variant)?
            }
        };
        image.to_png()
    }

    fn sprite_index(&self) -> Result<&Index> {
        cached(&self.sprites, || {
            let narc = |path| Narc::parse(&self.rom.file(path)?);
            let personal = self.rom.read(PERSONAL)?;
            let icons = narc(ICONS)?;
            let item_icons = narc(ITEM_ICONS)?;
            let code = self.rom.arm9()?;
            let icon_count = icons.len().saturating_sub(self.edition.sprites.first_icon) / 2;
            Ok(Index {
                pokemon: narc(POKEMON)?,
                forms: forms(&personal, self.edition.sprites.forms)?,
                trainers: narc(self.edition.sprites.trainers)?,
                icon_banks: icon_banks(&code, icon_count),
                icons,
                first_icon: self.edition.sprites.first_icon,
                items: item_icons_table(&code, item_icons.len()),
                item_icons,
            })
        })
    }
}

/// An asset's file name without extension: the species, and the form when not the base one.
fn stem(species: usize, form: u8) -> String {
    match form {
        0 => format!("{species:04}"),
        _ => format!("{species:04}-{form}"),
    }
}

/// Per species from 0: the battle sprite entry of its form 1, and how many alternate forms
/// it has. A personal record holds, at 0x1E, where its forms start among the form entries
/// (from `first_form`) and at 0x20 how many forms it has, counting the base one.
fn forms(personal: &[u8], first_form: usize) -> Result<Vec<(usize, usize)>> {
    let narc = Narc::parse(personal)?;
    (0..=SPECIES.min(narc.len().saturating_sub(1)))
        .map(|species| {
            let record = narc.member(personal, species)?;
            let field = |at: usize| record.get(at).copied().unwrap_or(0) as usize;
            let start = field(0x1E) | field(0x1F) << 8;
            let count = field(0x20).saturating_sub(1);
            Ok(match species {
                ARCEUS => (0, 0),
                _ => (first_form + start, count),
            })
        })
        .collect()
}

/// The palette banks of each of `count` icon entries, from the table in the game's code.
/// Empty when the table is not there, as in modified games.
fn icon_banks(code: &[u8], count: usize) -> Vec<u8> {
    find(code, &ICON_BANKS_START)
        .and_then(|at| code.get(at..at + count))
        .map_or_else(Vec::new, <[u8]>::to_vec)
}

/// Each item's icon tiles and palette members, from the table in the game's code, which ends
/// where the members no longer fit an archive of `members`. Empty when the table is not there.
fn item_icons_table(code: &[u8], members: usize) -> Vec<(usize, usize)> {
    let needle: Vec<u8> = ITEMS_START.iter().flat_map(|w| w.to_le_bytes()).collect();
    let Some(start) = find(code, &needle).and_then(|at| at.checked_sub(4)) else {
        return Vec::new();
    };
    code[start..]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&[a, b, c, d]| {
            let field = |low, high| u16::from_le_bytes([low, high]) as usize;
            (field(a, b), field(c, d))
        })
        .take_while(|&(tiles, palette)| tiles < members && palette < members)
        .collect()
}

/// Whether `member` of an archive holds something.
fn filled(narc: &Narc, member: usize) -> bool {
    narc.range(member).is_ok_and(|range| !range.is_empty())
}

/// A member's bytes, decompressed when the archive stores it compressed.
fn member<A: ByteSource>(narc: &Narc, archive: &A, index: usize) -> Result<Vec<u8>> {
    Ok(lz::decompress_if_compressed(&narc.member(archive, index)?).into_owned())
}

/// A static sprite: its tiles drawn as hardware objects filling `blocks` in order.
fn assemble(tiles: &Tiles, palette: &Palette, blocks: &[(i16, i16, u8, u8)]) -> Image {
    let mut tile = 0;
    let objects = blocks
        .iter()
        .map(|&(x, y, width, height)| {
            let object = Object {
                x,
                y,
                width,
                height,
                tile,
                bank: 0,
                flip_x: false,
                flip_y: false,
            };
            tile += (width / 8) as u16 * (height / 8) as u16;
            object
        })
        .collect();
    let cells = Cells {
        cells: vec![Cell { objects }],
        step: Some(tiles.depth.tile_len()),
    };
    cells.image(0, tiles, palette)
}

/// A 32×32 icon: the first frame of its tiles, in 1D order.
fn icon<A: ByteSource>(
    narc: &Narc,
    archive: &A,
    tiles: usize,
    palette: usize,
    bank: u8,
) -> Result<Image> {
    let mut tiles = Tiles::parse(&member(narc, archive, tiles)?)?;
    tiles.size = Some(ICON_TILES);
    let palette = Palette::parse(&member(narc, archive, palette)?)?;
    Ok(tiles.image(&palette, bank))
}

/// Where `needle` first occurs in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
