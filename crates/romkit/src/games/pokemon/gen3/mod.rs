//! Generation III: Ruby, Sapphire, Emerald, FireRed, and LeafGreen.
//!
//! GBA games have no filesystem: their data are tables compiled into the ROM, found by address
//! and linked by pointers (GBA addresses, the ROM being mapped at `0x08000000`). An [`Edition`]
//! records where one dump keeps each table; the addresses were located by content in real
//! dumps and agree with the table pointers that Emerald, FireRed, and LeafGreen keep in their
//! header. [`Gen3`] opens any of them and parses each table on first use.
//!
//! The games number species in their own internal order, which leaves the National Pokédex at
//! Celebi: 25 unused slots follow, then the Hoenn species in Hoenn Pokédex order. romkit
//! identifies species by National Pokédex number and keeps the internal index beside it.
//!
//! ```no_run
//! use romkit::games::pokemon::gen3::Gen3;
//! use romkit::io::FileSource;
//!
//! let game = Gen3::open(FileSource::open("emerald.gba")?)?;
//! for species in game.species()? {
//!     println!("{} {}: {:?}", species.id.0, species.name, species.types);
//! }
//! # Ok::<(), romkit::Error>(())
//! ```

mod assets;
mod cries;
mod items;
mod moves;
mod species;
mod sprites;
pub mod text;

use std::borrow::Cow;
use std::sync::OnceLock;

pub use items::{Item, Pocket};
pub use moves::{Move, MoveTarget};
pub use species::Species;
pub use sprites::Variant;

/// Shared with Gen 4, whose records use the same values. Item numbers are Gen 3's own.
pub use crate::games::pokemon::gen4::{
    Ability, AbilityId, Color, EggGroup, Evolution, EvolutionMethod, GenderRatio, GrowthRate,
    HeldItems, ItemId, LevelUpMove, MoveFlags, MoveId, SpeciesId, Stats, Type,
};

use crate::games::GameId;
use crate::io::ByteSource;
use crate::platforms::{Identity, identify};
use crate::{Error, Result};

const FORMAT: &str = "Gen 3 ROM";
/// Where the GBA maps the cartridge.
const ROM_BASE: u32 = 0x0800_0000;
/// Strings are short; a longer one means a bad pointer.
const MAX_STRING: u64 = 1024;
/// Every compressed graphic is smaller; a larger size means a bad pointer.
const MAX_DECOMPRESSED: usize = 0x8000;

/// Where one dump of a Gen 3 game keeps its tables, as offsets in the ROM.
#[derive(Debug)]
pub struct Edition {
    game: GameId,
    /// The header's product code and revision, which tell dumps apart.
    code: [u8; 4],
    revision: u16,
    /// Bag pockets by their stored number from 1; FireRed and LeafGreen order them differently.
    pockets: [Pocket; 5],
    /// Names (11 bytes), base stats (28), evolutions (40), and TM/HM bits (8), per internal
    /// species from 0, and a pointer to each species' level-up learnset.
    species_names: u32,
    base_stats: u32,
    evolutions: u32,
    learnsets: u32,
    machines: u32,
    /// The National Pokédex number of each internal species from 1, as 16-bit values.
    national_dex: u32,
    egg_moves: u32,
    /// Records (12 bytes) and names (13) per move from 0, and a description pointer per move
    /// from 1.
    moves: u32,
    move_names: u32,
    move_descriptions: u32,
    /// Names (13 bytes) and description pointers per ability from 0.
    ability_names: u32,
    ability_descriptions: u32,
    /// Records (44 bytes) per item from 0.
    items: u32,
    item_count: u16,
    /// Pairs of pointers to each item's compressed icon and palette; Ruby and Sapphire show no
    /// icons.
    item_icons: Option<u32>,
    /// Per sprite: a pointer to the compressed graphic or palette, with its size and tag.
    front_sprites: u32,
    back_sprites: u32,
    palettes: u32,
    shiny_palettes: u32,
    /// Per sprite: a pointer to an uncompressed two-frame icon, its palette's number, and the
    /// palettes themselves.
    icons: u32,
    icon_palette_numbers: u32,
    icon_palettes: u32,
    trainer_sprites: u32,
    trainer_palettes: u32,
    trainer_count: u16,
    /// Voices (12 bytes) per cry, and the cry of each Hoenn species from Treecko on.
    cries: u32,
    hoenn_cries: u32,
}

/// Ruby, Sapphire, and Emerald: Items, Poké Balls, TMs and HMs, Berries, Key Items.
const HOENN_POCKETS: [Pocket; 5] = [
    Pocket::Items,
    Pocket::PokeBalls,
    Pocket::Machines,
    Pocket::Berries,
    Pocket::KeyItems,
];
/// FireRed and LeafGreen: Items, Key Items, Poké Balls, TM Case, Berry Pouch.
const KANTO_POCKETS: [Pocket; 5] = [
    Pocket::Items,
    Pocket::KeyItems,
    Pocket::PokeBalls,
    Pocket::Machines,
    Pocket::Berries,
];

const RUBY: Edition = Edition {
    game: GameId::PokemonRuby,
    code: *b"AXVE",
    revision: 0,
    pockets: HOENN_POCKETS,
    species_names: 0x1F716C,
    base_stats: 0x1FEC18,
    evolutions: 0x203B68,
    learnsets: 0x207BC8,
    machines: 0x1FD0F0,
    national_dex: 0x1FC516,
    egg_moves: 0x2091DC,
    moves: 0x1FB12C,
    move_names: 0x1F8320,
    move_descriptions: 0x3C09D8,
    ability_names: 0x1FA248,
    ability_descriptions: 0x1FA110,
    items: 0x3C5564,
    item_count: 349,
    item_icons: None,
    front_sprites: 0x1E8354,
    back_sprites: 0x1E97F4,
    palettes: 0x1EA5B4,
    shiny_palettes: 0x1EB374,
    icons: 0x3BBD20,
    icon_palette_numbers: 0x3BC400,
    icon_palettes: 0x3BC5B8,
    trainer_sprites: 0x1EC53C,
    trainer_palettes: 0x1EC7D4,
    trainer_count: 83,
    cries: 0x452590,
    hoenn_cries: 0x1FDE6A,
};

const SAPPHIRE: Edition = Edition {
    game: GameId::PokemonSapphire,
    code: *b"AXPE",
    revision: 0,
    pockets: HOENN_POCKETS,
    species_names: 0x1F70FC,
    base_stats: 0x1FEBA8,
    evolutions: 0x203AF8,
    learnsets: 0x207B58,
    machines: 0x1FD080,
    national_dex: 0x1FC4A6,
    egg_moves: 0x20916C,
    moves: 0x1FB0BC,
    move_names: 0x1F82B0,
    move_descriptions: 0x3C0A30,
    ability_names: 0x1FA1D8,
    ability_descriptions: 0x1FA0A0,
    items: 0x3C55BC,
    item_count: 349,
    item_icons: None,
    front_sprites: 0x1E82E4,
    back_sprites: 0x1E9784,
    palettes: 0x1EA544,
    shiny_palettes: 0x1EB304,
    icons: 0x3BBD78,
    icon_palette_numbers: 0x3BC458,
    icon_palettes: 0x3BC610,
    trainer_sprites: 0x1EC4CC,
    trainer_palettes: 0x1EC764,
    trainer_count: 83,
    cries: 0x4525EC,
    hoenn_cries: 0x1FDDFA,
};

/// Emerald's front sprites are its still ones; another table holds two-frame animations.
const EMERALD: Edition = Edition {
    game: GameId::PokemonEmerald,
    code: *b"BPEE",
    revision: 0,
    pockets: HOENN_POCKETS,
    species_names: 0x3185C8,
    base_stats: 0x3203CC,
    evolutions: 0x32531C,
    learnsets: 0x32937C,
    machines: 0x31E898,
    national_dex: 0x31DC82,
    egg_moves: 0x32ADD8,
    moves: 0x31C898,
    move_names: 0x31977C,
    move_descriptions: 0x61C524,
    ability_names: 0x31B6DB,
    ability_descriptions: 0x31BAD4,
    items: 0x5839A0,
    item_count: 377,
    item_icons: Some(0x614410),
    front_sprites: 0x301418,
    back_sprites: 0x3028B8,
    palettes: 0x303678,
    shiny_palettes: 0x304438,
    icons: 0x57BCA8,
    icon_palette_numbers: 0x57C388,
    icon_palettes: 0x57C540,
    trainer_sprites: 0x305654,
    trainer_palettes: 0x30593C,
    trainer_count: 93,
    cries: 0x69DCF4,
    hoenn_cries: 0x31F61C,
};

const FIRERED: Edition = Edition {
    game: GameId::PokemonFireRed,
    code: *b"BPRE",
    revision: 0,
    pockets: KANTO_POCKETS,
    species_names: 0x245EE0,
    base_stats: 0x254784,
    evolutions: 0x259754,
    learnsets: 0x25D7B4,
    machines: 0x252BC8,
    national_dex: 0x251FEE,
    egg_moves: 0x25EF0C,
    moves: 0x250C04,
    move_names: 0x247094,
    move_descriptions: 0x4886E8,
    ability_names: 0x24FC40,
    ability_descriptions: 0x24FB08,
    items: 0x3DB028,
    item_count: 375,
    item_icons: Some(0x3D4294),
    front_sprites: 0x2350AC,
    back_sprites: 0x23654C,
    palettes: 0x23730C,
    shiny_palettes: 0x2380CC,
    icons: 0x3D37A0,
    icon_palette_numbers: 0x3D3E80,
    icon_palettes: 0x3D4038,
    trainer_sprites: 0x23957C,
    trainer_palettes: 0x239A1C,
    trainer_count: 148,
    cries: 0x48C914,
    hoenn_cries: 0x2539D4,
};

const LEAFGREEN: Edition = Edition {
    game: GameId::PokemonLeafGreen,
    code: *b"BPGE",
    revision: 0,
    pockets: KANTO_POCKETS,
    species_names: 0x245EBC,
    base_stats: 0x254760,
    evolutions: 0x259734,
    learnsets: 0x25D794,
    machines: 0x252BA4,
    national_dex: 0x251FCA,
    egg_moves: 0x25EEEC,
    moves: 0x250BE0,
    move_names: 0x247070,
    move_descriptions: 0x487FC4,
    ability_names: 0x24FC1C,
    ability_descriptions: 0x24FAE4,
    items: 0x3DAE64,
    item_count: 375,
    item_icons: Some(0x3D40D0),
    front_sprites: 0x235088,
    back_sprites: 0x236528,
    palettes: 0x2372E8,
    shiny_palettes: 0x2380A8,
    icons: 0x3D35DC,
    icon_palette_numbers: 0x3D3CBC,
    icon_palettes: 0x3D3E74,
    trainer_sprites: 0x239558,
    trainer_palettes: 0x2399F8,
    trainer_count: 148,
    cries: 0x48C1F0,
    hoenn_cries: 0x2539B0,
};

/// The dumps whose tables are known: the first USA release of each game.
const EDITIONS: [&Edition; 5] = [&RUBY, &SAPPHIRE, &EMERALD, &FIRERED, &LEAFGREEN];

impl Edition {
    /// Whether romkit knows the tables of some dump of `game`.
    pub fn covers(game: GameId) -> bool {
        EDITIONS.iter().any(|edition| edition.game == game)
    }

    /// The edition of the dump with this header. Other languages and revisions keep their
    /// tables elsewhere, so they are recognized but not extracted.
    pub fn of(identity: &Identity) -> Option<&'static Self> {
        EDITIONS.into_iter().find(|edition| {
            identity.code.is_some_and(|code| code.0 == edition.code)
                && identity.revision == edition.revision
        })
    }
}

/// A Gen 3 game opened over a byte source. Opening reads only the header; each table is read
/// and parsed when first asked for, then kept.
pub struct Gen3<S> {
    edition: &'static Edition,
    source: S,
    national_dex: OnceLock<Vec<u16>>,
    species: OnceLock<Vec<Species>>,
    moves: OnceLock<Vec<Move>>,
    abilities: OnceLock<Vec<Ability>>,
    items: OnceLock<Vec<Item>>,
}

impl<S: ByteSource> Gen3<S> {
    pub fn open(source: S) -> Result<Self> {
        let identity = identify(&source)?;
        let edition = Edition::of(&identity).ok_or_else(|| {
            Error::Unsupported(format!("{identity} is not a known Gen 3 Pokémon dump"))
        })?;
        Ok(Self {
            edition,
            source,
            national_dex: OnceLock::new(),
            species: OnceLock::new(),
            moves: OnceLock::new(),
            abilities: OnceLock::new(),
            items: OnceLock::new(),
        })
    }

    pub fn game(&self) -> GameId {
        self.edition.game
    }

    /// Every species, in National Pokédex order: `species()[n - 1]` is number `n`.
    pub fn species(&self) -> Result<&[Species]> {
        cached(&self.species, || species::parse(self)).map(Vec::as_slice)
    }

    /// Every move, indexed by [`MoveId`] from 1.
    pub fn moves(&self) -> Result<&[Move]> {
        cached(&self.moves, || moves::parse(self)).map(Vec::as_slice)
    }

    /// Every ability, indexed by [`AbilityId`] from 1, with the species that can have it.
    pub fn abilities(&self) -> Result<&[Ability]> {
        cached(&self.abilities, || species::abilities(self)).map(Vec::as_slice)
    }

    /// Every item, indexed by [`ItemId`] from 1.
    pub fn items(&self) -> Result<&[Item]> {
        cached(&self.items, || items::parse(self)).map(Vec::as_slice)
    }

    /// The National Pokédex number of each internal species from 1.
    fn national_dex(&self) -> Result<&[u16]> {
        cached(&self.national_dex, || {
            let table = self.read(self.edition.national_dex, (INTERNAL_SPECIES - 1) * 2)?;
            Ok(table
                .as_chunks()
                .0
                .iter()
                .map(|&number| u16::from_le_bytes(number))
                .collect())
        })
        .map(Vec::as_slice)
    }

    /// The internal index of a species, by National Pokédex number.
    fn internal_index(&self, species: SpeciesId) -> Result<usize> {
        if (1..=NATIONAL_DEX).contains(&species.0)
            && let Some(index) = self.national_dex()?.iter().position(|&n| n == species.0)
        {
            return Ok(index + 1);
        }
        Err(Error::NotFound(format!("species {}", species.0)))
    }

    /// `len` bytes at a ROM offset.
    fn read(&self, offset: u32, len: usize) -> Result<Cow<'_, [u8]>> {
        self.source.read(offset.into(), len as u64)
    }

    /// The ROM offset a GBA pointer points to, checked to lie within the ROM.
    fn offset(&self, pointer: u32) -> Result<u32> {
        pointer
            .checked_sub(ROM_BASE)
            .filter(|&offset| u64::from(offset) < self.source.size())
            .ok_or_else(|| Error::malformed(FORMAT, pointer.into(), "pointer outside the ROM"))
    }

    /// The pointer at a ROM offset, as the offset it points to.
    fn follow(&self, at: u32) -> Result<u32> {
        let pointer = self.read(at, 4)?;
        self.offset(u32::from_le_bytes(*pointer.first_chunk().expect("4 bytes")))
    }

    /// The string a pointer at `at` points to.
    fn string_at(&self, at: u32) -> Result<String> {
        let offset = self.follow(at)?;
        let bytes = self.source.read_up_to(offset.into(), MAX_STRING)?;
        if !bytes.contains(&text::END) {
            return Err(Error::malformed(
                FORMAT,
                offset.into(),
                "unterminated string",
            ));
        }
        Ok(text::decode(&bytes))
    }

    /// The LZ77 data a pointer at `at` points to, decompressed.
    fn compressed_at(&self, at: u32) -> Result<Vec<u8>> {
        let offset = self.follow(at)?;
        let header = self.read(offset, 4)?;
        let len = u32::from_le_bytes(*header.first_chunk().expect("4 bytes")) as usize >> 8;
        if len > MAX_DECOMPRESSED {
            return Err(Error::malformed(
                FORMAT,
                offset.into(),
                "compressed data too large",
            ));
        }
        // At worst a flag byte per eight literal bytes.
        let bytes = self
            .source
            .read_up_to(offset.into(), (4 + len + len / 8 + 1) as u64)?;
        crate::formats::lz::decompress(&bytes)
    }
}

/// Internal species slots, from the placeholder at 0 to Chimecho at 411.
const INTERNAL_SPECIES: usize = 412;
/// Gen 3 completes the National Pokédex at Deoxys.
const NATIONAL_DEX: u16 = 386;

/// The value in `cell`, computed on first use. A failure is returned, not kept.
fn cached<T>(cell: &OnceLock<T>, init: impl FnOnce() -> Result<T>) -> Result<&T> {
    if let Some(value) = cell.get() {
        return Ok(value);
    }
    let value = init()?;
    Ok(cell.get_or_init(|| value))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GBA ROM of `len` bytes with a header for `code` and `revision`.
    fn rom(code: &[u8; 4], revision: u8, len: usize) -> Vec<u8> {
        let mut rom = vec![0; len];
        rom[0x04..0x08].copy_from_slice(&[0x24, 0xFF, 0xAE, 0x51]);
        rom[0xA0..0xAC].copy_from_slice(b"POKEMON RUBY");
        rom[0xAC..0xB0].copy_from_slice(code);
        rom[0xB2] = 0x96;
        rom[0xBC] = revision;
        rom
    }

    #[test]
    fn opens_only_known_dumps() {
        assert!(Gen3::open(rom(b"AXVE", 0, 0x200)).is_ok());
        let unknown = [rom(b"AXVE", 1, 0x200), rom(b"AXVD", 0, 0x200)];
        for rom in unknown {
            assert!(matches!(Gen3::open(&rom), Err(Error::Unsupported(_))));
            assert!(crate::games::open(GameId::PokemonRuby, rom.as_slice()).is_none());
        }
        assert!(GameId::PokemonRuby.is_supported());
    }

    #[test]
    fn bad_pointers_and_truncated_dumps_are_errors() {
        let mut bytes = rom(b"AXVE", 0, 0x400);
        bytes[0x300..0x304].copy_from_slice(&0x0800_0200u32.to_le_bytes());
        bytes[0x304..0x308].copy_from_slice(&0x0800_0400u32.to_le_bytes());
        bytes[0x308..0x30C].copy_from_slice(&0x0200_0000u32.to_le_bytes());
        bytes[0x200] = 0xBB; // "A", unterminated
        let game = Gen3::open(bytes.as_slice()).unwrap();
        assert_eq!(game.follow(0x300).unwrap(), 0x200);
        assert!(game.follow(0x304).is_err() && game.follow(0x308).is_err());
        bytes[0x201] = text::END;
        let game = Gen3::open(bytes.as_slice()).unwrap();
        assert_eq!(game.string_at(0x300).unwrap(), "A");
        // The tables lie past the end of this small ROM.
        assert!(matches!(game.species(), Err(Error::Malformed { .. })));
        assert!(game.pokemon_sprite(SpeciesId(25), Variant::ALL[0]).is_err());
        assert!(game.cry(SpeciesId(0)).is_err());
    }
}
