//! Generation IV: Diamond, Pearl, Platinum, HeartGold, and SoulSilver.
//!
//! The games share one engine and its data formats; an [`Edition`] records where each keeps
//! its files and text. [`Gen4`] opens any of them and parses each table on first use.
//!
//! ```no_run
//! use romkit::games::pokemon::gen4::Gen4;
//! use romkit::io::FileSource;
//!
//! let game = Gen4::open(FileSource::open("platinum.nds")?)?;
//! for species in game.species()? {
//!     println!("{:?}: {:?}", species.name, species.types);
//! }
//! # Ok::<(), romkit::Error>(())
//! ```

mod assets;
mod moves;
mod species;
mod sprites;
pub mod text;

use std::sync::OnceLock;

use serde::Serialize;

pub use moves::{ContestType, Move, MoveClass, MoveFlags, MoveTarget};
pub use species::{
    Ability, Color, EggGroup, Evolution, EvolutionMethod, GenderRatio, GrowthRate, HeldItems,
    LevelUpMove, Species, Stats,
};
pub use sprites::Variant;
use sprites::{Archives, Encryption, Layout};
use text::Text;

use crate::formats::narc::Narc;
use crate::games::GameId;
use crate::io::{ByteSource, Window};
use crate::platforms::nds::NdsRom;
use crate::{Error, Result};

/// Where a Gen 4 game keeps its data.
#[derive(Debug)]
pub struct Edition {
    personal: &'static str,
    evolutions: &'static str,
    learnsets: &'static str,
    moves: &'static str,
    text: &'static str,
    species_names: usize,
    move_names: usize,
    move_descriptions: usize,
    ability_names: usize,
    ability_descriptions: usize,
    /// Descriptive text bank names, one per line; empty when unknown.
    bank_names: &'static str,
    sprites: Archives,
}

const DIAMOND: Edition = Edition {
    personal: "poketool/personal/personal.narc",
    evolutions: "poketool/personal/evo.narc",
    learnsets: "poketool/personal/wotbl.narc",
    moves: "poketool/waza/waza_tbl.narc",
    text: "msgdata/msg.narc",
    species_names: 362,
    move_names: 588,
    move_descriptions: 587,
    ability_names: 552,
    ability_descriptions: 554,
    bank_names: "",
    sprites: Archives {
        pokemon: "poketool/pokegra/pokegra.narc",
        pokemon_encryption: Encryption::FromEnd,
        trainers: Layout {
            path: "poketool/trgra/trfgra.narc",
            stride: 2,
            tiles: 0,
            palette: 1,
            encryption: Encryption::FromEnd,
        },
        icons: "poketool/icongra/poke_icon.narc",
        items: "itemtool/itemdata/item_icon.narc",
    },
};

/// Pearl's species differ from Diamond's only in their version-exclusive data.
const PEARL: Edition = Edition {
    personal: "poketool/personal_pearl/personal.narc",
    ..DIAMOND
};

/// Platinum keeps a `pl_` copy of the tables it changed beside the Diamond and Pearl ones.
const PLATINUM: Edition = Edition {
    personal: "poketool/personal/pl_personal.narc",
    moves: "poketool/waza/pl_waza_tbl.narc",
    text: "msgdata/pl_msg.narc",
    species_names: 412,
    move_names: 647,
    move_descriptions: 646,
    ability_names: 610,
    ability_descriptions: 612,
    bank_names: text::PLATINUM_BANKS,
    sprites: Archives {
        pokemon: "poketool/pokegra/pl_pokegra.narc",
        pokemon_encryption: Encryption::FromStart,
        trainers: PLATINUM_TRAINERS,
        icons: "poketool/icongra/pl_poke_icon.narc",
        items: "itemtool/itemdata/item_icon.narc",
    },
    ..DIAMOND
};

/// Platinum and later keep five members per trainer class: a sheet of parts, the palette, the
/// cells and animations that assemble the parts, then the whole sprite.
const PLATINUM_TRAINERS: Layout = Layout {
    path: "poketool/trgra/trfgra.narc",
    stride: 5,
    tiles: 4,
    palette: 1,
    encryption: Encryption::FromStart,
};

/// HeartGold and SoulSilver number their files instead of naming them.
const HEARTGOLD_SOULSILVER: Edition = Edition {
    personal: "a/0/0/2",
    evolutions: "a/0/3/4",
    learnsets: "a/0/3/3",
    moves: "a/0/1/1",
    text: "a/0/2/7",
    species_names: 237,
    move_names: 750,
    move_descriptions: 749,
    ability_names: 720,
    ability_descriptions: 722,
    bank_names: "",
    sprites: Archives {
        pokemon: "a/0/0/4",
        pokemon_encryption: Encryption::FromStart,
        trainers: Layout {
            path: "a/0/5/8",
            encryption: Encryption::FromEnd,
            ..PLATINUM_TRAINERS
        },
        icons: "a/0/2/0",
        items: "a/0/1/8",
    },
};

impl Edition {
    pub fn of(game: GameId) -> Option<&'static Self> {
        match game {
            GameId::PokemonDiamond => Some(&DIAMOND),
            GameId::PokemonPearl => Some(&PEARL),
            GameId::PokemonPlatinum => Some(&PLATINUM),
            GameId::PokemonHeartGold | GameId::PokemonSoulSilver => Some(&HEARTGOLD_SOULSILVER),
            _ => None,
        }
    }
}

/// A Gen 4 game opened over a byte source. Opening reads only the ROM's header and filesystem;
/// each table is read and parsed when first asked for, then kept.
pub struct Gen4<S> {
    game: GameId,
    edition: &'static Edition,
    rom: NdsRom<S>,
    text: OnceLock<Narc>,
    species: OnceLock<Vec<Species>>,
    moves: OnceLock<Vec<Move>>,
    abilities: OnceLock<Vec<Ability>>,
    sprites: OnceLock<sprites::Index>,
}

impl<S: ByteSource> Gen4<S> {
    pub fn open(source: S) -> Result<Self> {
        let rom = NdsRom::open(source)?;
        let identity = rom.header().identity();
        let game = GameId::identify(&identity)
            .filter(|&game| Edition::of(game).is_some())
            .ok_or_else(|| Error::Unsupported(format!("{identity} is not a Gen 4 Pokémon game")))?;
        Ok(Self {
            game,
            edition: Edition::of(game).expect("filtered above"),
            rom,
            text: OnceLock::new(),
            species: OnceLock::new(),
            moves: OnceLock::new(),
            abilities: OnceLock::new(),
            sprites: OnceLock::new(),
        })
    }

    pub fn game(&self) -> GameId {
        self.game
    }

    /// The DS container, for files this module does not model.
    pub fn rom(&self) -> &NdsRom<S> {
        &self.rom
    }

    /// All of the game's text.
    pub fn text(&self) -> Result<Text<'_, Window<&S>>> {
        let archive = self.rom.file(self.edition.text)?;
        let narc = cached(&self.text, || Narc::parse(&archive))?;
        Ok(Text::new(archive, narc, self.edition.bank_names))
    }

    /// Every species and alternate form, indexed by [`SpeciesId`] from 1.
    pub fn species(&self) -> Result<&[Species]> {
        cached(&self.species, || {
            let edition = self.edition;
            let personal = self.rom.read(edition.personal)?;
            let evolutions = self.rom.read(edition.evolutions)?;
            let learnsets = self.rom.read(edition.learnsets)?;
            let names = self.text()?.strings(edition.species_names)?;
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
            moves::parse(&self.rom.read(self.edition.moves)?, &names, &descriptions)
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
}

/// The value in `cell`, computed on first use. A failure is returned, not kept.
fn cached<T>(cell: &OnceLock<T>, init: impl FnOnce() -> Result<T>) -> Result<&T> {
    if let Some(value) = cell.get() {
        return Ok(value);
    }
    let value = init()?;
    Ok(cell.get_or_init(|| value))
}

/// Typed indices, serialized as bare numbers.
macro_rules! ids {
    ($( $(#[$doc:meta])* $name:ident($repr:ty); )+) => {$(
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(pub $repr);
    )+};
}

ids! {
    /// A species or form: the National Pokédex number up to 493, then the Egg, the Bad Egg,
    /// and alternate forms.
    SpeciesId(u16);
    MoveId(u16);
    AbilityId(u8);
    ItemId(u16);
}

raw_enum! {
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
        /// The `???` type of Curse.
        Mystery = 9,
        Fire = 10,
        Water = 11,
        Grass = 12,
        Electric = 13,
        Psychic = 14,
        Ice = 15,
        Dragon = 16,
        Dark = 17,
    }
}
