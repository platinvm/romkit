//! Generation V: Black, White, Black 2, and White 2.
//!
//! The games share one engine, its data formats, and their file paths; an [`Edition`] records
//! where each keeps the text banks the data tables are named from. [`Gen5`] opens any of them
//! and parses each table on first use.
//!
//! ```no_run
//! use romkit::games::pokemon::gen5::Gen5;
//! use romkit::io::FileSource;
//!
//! let game = Gen5::open(FileSource::open("black2.nds")?)?;
//! for species in game.species()? {
//!     println!("{:?}: {:?}", species.name, species.types);
//! }
//! # Ok::<(), romkit::Error>(())
//! ```

mod assets;
mod items;
mod moves;
mod sound;
mod species;
mod sprites;
pub mod text;

use std::sync::OnceLock;

pub use items::{BattlePockets, Item, NaturalGift, Pocket};
pub use moves::{
    Ailment, BattleStat, Move, MoveClass, MoveFlags, MoveKind, MoveTarget, Span, StatChange,
};
pub use species::{Ability, Evolution, EvolutionMethod, Form, HeldItems, Species};
use text::Text;

/// Shared with Gen 4, whose records use the same values. A [`SpeciesId`] here is the National
/// Pokédex number up to 649, then the records of alternate forms.
pub use crate::games::pokemon::gen4::{
    AbilityId, Color, EggGroup, GenderRatio, GrowthRate, ItemId, LevelUpMove, MoveId, SpeciesId,
    Stats, Variant,
};
use sprites::Archives;

use crate::formats::narc::Narc;
use crate::formats::sdat::Sdat;
use crate::games::GameId;
use crate::io::{ByteSource, Window};
use crate::platforms::nds::NdsRom;
use crate::{Error, Result};

/// The data tables, at the same paths in every Gen 5 game.
const PERSONAL: &str = "a/0/1/6";
const LEARNSETS: &str = "a/0/1/8";
const EVOLUTIONS: &str = "a/0/1/9";
const MOVES: &str = "a/0/2/1";
const ITEMS: &str = "a/0/2/4";
/// System text: names, descriptions, and menus.
const TEXT: &str = "a/0/0/2";
/// Story text: what people say on each map.
const STORY_TEXT: &str = "a/0/0/3";

/// Where a Gen 5 game keeps the names and descriptions of its data, as banks of its system text,
/// and its sound archive.
#[derive(Debug)]
pub struct Edition {
    species_names: usize,
    move_names: usize,
    move_descriptions: usize,
    ability_names: usize,
    ability_descriptions: usize,
    item_names: usize,
    item_descriptions: usize,
    sprites: Archives,
    sound: &'static str,
}

const BLACK_WHITE: Edition = Edition {
    species_names: 70,
    move_names: 203,
    move_descriptions: 202,
    ability_names: 182,
    ability_descriptions: 183,
    item_names: 54,
    item_descriptions: 53,
    sprites: Archives {
        trainers: "a/0/7/2",
        forms: 652,
        first_icon: 7,
    },
    sound: "wb_sound_data.sdat",
};

const BLACK2_WHITE2: Edition = Edition {
    species_names: 90,
    move_names: 403,
    move_descriptions: 402,
    ability_names: 374,
    ability_descriptions: 375,
    item_names: 64,
    item_descriptions: 63,
    sprites: Archives {
        trainers: "a/0/7/1",
        forms: 685,
        first_icon: 8,
    },
    sound: "swan_sound_data.sdat",
};

impl Edition {
    pub fn of(game: GameId) -> Option<&'static Self> {
        match game {
            GameId::PokemonBlack | GameId::PokemonWhite => Some(&BLACK_WHITE),
            GameId::PokemonBlack2 | GameId::PokemonWhite2 => Some(&BLACK2_WHITE2),
            _ => None,
        }
    }
}

/// A Gen 5 game opened over a byte source. Opening reads only the ROM's header and filesystem;
/// each table is read and parsed when first asked for, then kept.
pub struct Gen5<S> {
    game: GameId,
    edition: &'static Edition,
    rom: NdsRom<S>,
    text: OnceLock<Narc>,
    story_text: OnceLock<Narc>,
    species: OnceLock<Vec<Species>>,
    moves: OnceLock<Vec<Move>>,
    abilities: OnceLock<Vec<Ability>>,
    items: OnceLock<Vec<Item>>,
    sprites: OnceLock<sprites::Index>,
    sound: OnceLock<Sdat>,
}

impl<S: ByteSource> Gen5<S> {
    pub fn open(source: S) -> Result<Self> {
        let rom = NdsRom::open(source)?;
        let identity = rom.header().identity();
        let game = GameId::identify(&identity)
            .filter(|&game| Edition::of(game).is_some())
            .ok_or_else(|| Error::Unsupported(format!("{identity} is not a Gen 5 Pokémon game")))?;
        Ok(Self {
            game,
            edition: Edition::of(game).expect("filtered above"),
            rom,
            text: OnceLock::new(),
            story_text: OnceLock::new(),
            species: OnceLock::new(),
            moves: OnceLock::new(),
            abilities: OnceLock::new(),
            items: OnceLock::new(),
            sprites: OnceLock::new(),
            sound: OnceLock::new(),
        })
    }

    pub fn game(&self) -> GameId {
        self.game
    }

    /// The DS container, for files this module does not model.
    pub fn rom(&self) -> &NdsRom<S> {
        &self.rom
    }

    /// The system text: names, descriptions, and menus.
    pub fn text(&self) -> Result<Text<'_, Window<&S>>> {
        self.archive(TEXT, &self.text)
    }

    /// The story text: what people say, map by map.
    pub fn story_text(&self) -> Result<Text<'_, Window<&S>>> {
        self.archive(STORY_TEXT, &self.story_text)
    }

    fn archive<'a>(
        &'a self,
        path: &str,
        narc: &'a OnceLock<Narc>,
    ) -> Result<Text<'a, Window<&'a S>>> {
        let archive = self.rom.file(path)?;
        let narc = cached(narc, || Narc::parse(&archive))?;
        Ok(Text::new(archive, narc))
    }

    /// Every species and alternate form, indexed by [`SpeciesId`] from 1.
    pub fn species(&self) -> Result<&[Species]> {
        cached(&self.species, || {
            let personal = self.rom.read(PERSONAL)?;
            let evolutions = self.rom.read(EVOLUTIONS)?;
            let learnsets = self.rom.read(LEARNSETS)?;
            let names = self.text()?.strings(self.edition.species_names)?;
            species::parse(&personal, &evolutions, &learnsets, &names)
        })
        .map(Vec::as_slice)
    }

    /// Every move, indexed by [`MoveId`] from 1.
    pub fn moves(&self) -> Result<&[Move]> {
        cached(&self.moves, || {
            let text = self.text()?;
            let names = text.strings(self.edition.move_names)?;
            let descriptions = text.strings(self.edition.move_descriptions)?;
            moves::parse(&self.rom.read(MOVES)?, &names, &descriptions)
        })
        .map(Vec::as_slice)
    }

    /// Every ability, with the species that can have it.
    pub fn abilities(&self) -> Result<&[Ability]> {
        cached(&self.abilities, || {
            let text = self.text()?;
            let names = text.strings(self.edition.ability_names)?;
            let descriptions = text.strings(self.edition.ability_descriptions)?;
            Ok(species::abilities(self.species()?, &names, &descriptions))
        })
        .map(Vec::as_slice)
    }

    /// Every item, indexed by [`ItemId`] from 1.
    pub fn items(&self) -> Result<&[Item]> {
        cached(&self.items, || {
            let text = self.text()?;
            let names = text.strings(self.edition.item_names)?;
            let descriptions = text.strings(self.edition.item_descriptions)?;
            items::parse(&self.rom.read(ITEMS)?, &names, &descriptions)
        })
        .map(Vec::as_slice)
    }
}

/// The value in `cell`, computed on first use. A failure is returned, not kept.
fn cached<T>(cell: &OnceLock<T>, init: impl FnOnce() -> Result<T>) -> Result<&T> {
    if let Some(value) = cell.get() {
        return Ok(value);
    }
    let value = init()?;
    Ok(cell.get_or_init(|| value))
}

raw_enum! {
    /// Gen 5 numbers types without Gen 4's `???`, so Fire and those after it are one lower.
    pub enum Type: u8 {
        Normal = 0,
        Fighting = 1,
        Flying = 2,
        Poison = 3,
        Ground = 4,
        Rock = 5,
        Bug = 6,
        Ghost = 7,
        Steel = 8,
        Fire = 9,
        Water = 10,
        Grass = 11,
        Electric = 12,
        Psychic = 13,
        Ice = 14,
        Dragon = 15,
        Dark = 16,
    }
}
