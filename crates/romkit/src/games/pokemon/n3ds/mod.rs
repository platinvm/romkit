//! Generations VI and VII, the 3DS games: X, Y, Omega Ruby, Alpha Sapphire, Sun, Moon, Ultra
//! Sun, and Ultra Moon.
//!
//! The games share one engine lineage and its data formats; records grow a few fields from one
//! generation to the next. They keep their data in numbered GARC archives, and their text in one
//! archive per language. An [`Edition`] records which archives hold what and which text banks
//! hold the names; [`N3ds`] opens any of the games and parses each table on first use.
//!
//! ```no_run
//! use romkit::games::pokemon::n3ds::N3ds;
//! use romkit::io::FileSource;
//!
//! let game = N3ds::open(FileSource::open("sun.3ds")?)?;
//! for species in game.species()? {
//!     println!("{:?}: {:?}", species.name, species.types);
//! }
//! # Ok::<(), romkit::Error>(())
//! ```

mod assets;
mod moves;
mod species;
pub mod text;

use std::sync::OnceLock;

pub use moves::{Ailment, Move, MoveCategory, MoveClass, MoveFlags, MoveTarget, StatChange, ZMove};
pub use species::{Evolution, EvolutionMethod, Form, HeldItems, Species, ZCrystal};
use text::Text;

pub use super::gen4::{Ability, AbilityId, ItemId, MoveId, SpeciesId};
use crate::formats::garc::Garc;
use crate::games::GameId;
use crate::io::{ByteSource, Window};
use crate::platforms::n3ds::N3dsRom;
use crate::{Error, Result};

/// Text languages, in the order of each game's text archives. `ja-kana` is Japanese without
/// kanji, which the games offer to young players. Chinese text is stored in the games' own
/// private-use encoding, kept as is.
pub const LANGUAGES: [&str; 10] = [
    "ja-kana", "ja-kanji", "en", "fr", "it", "de", "es", "ko", "zh-hans", "zh-hant",
];

/// A game's two sets of text archives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextSet {
    /// Names, descriptions, and menus.
    Main,
    /// What characters say.
    Story,
}

/// Where a 3DS Pokémon game keeps its data.
#[derive(Debug)]
pub struct Edition {
    generation: u8,
    personal: &'static str,
    /// Personal records grow from 0x40 bytes in X and Y to 0x54 in Sun and Moon.
    personal_len: usize,
    evolutions: &'static str,
    learnsets: &'static str,
    moves: &'static str,
    /// The main text archive of the first language; the others follow in number order.
    main_text: &'static str,
    story_text: &'static str,
    languages: usize,
    species_names: usize,
    move_names: usize,
    move_descriptions: usize,
    ability_names: usize,
    ability_descriptions: usize,
}

const X_Y: Edition = Edition {
    generation: 6,
    personal: "a/2/1/8",
    personal_len: 0x40,
    evolutions: "a/2/1/5",
    learnsets: "a/2/1/4",
    moves: "a/2/1/2",
    main_text: "a/0/7/2",
    story_text: "a/0/8/0",
    languages: 8,
    species_names: 80,
    move_names: 13,
    move_descriptions: 15,
    ability_names: 34,
    ability_descriptions: 33,
};

const OMEGA_RUBY_ALPHA_SAPPHIRE: Edition = Edition {
    generation: 6,
    personal: "a/1/9/5",
    personal_len: 0x50,
    evolutions: "a/1/9/2",
    learnsets: "a/1/9/1",
    moves: "a/1/8/9",
    main_text: "a/0/7/1",
    story_text: "a/0/7/9",
    languages: 8,
    species_names: 98,
    move_names: 14,
    move_descriptions: 16,
    ability_names: 37,
    ability_descriptions: 36,
};

const SUN_MOON: Edition = Edition {
    generation: 7,
    personal: "a/0/1/7",
    personal_len: 0x54,
    evolutions: "a/0/1/4",
    learnsets: "a/0/1/3",
    moves: "a/0/1/1",
    main_text: "a/0/3/0",
    story_text: "a/0/4/0",
    languages: 10,
    species_names: 55,
    move_names: 113,
    move_descriptions: 112,
    ability_names: 96,
    ability_descriptions: 97,
};

/// Ultra Sun and Ultra Moon add banks before the ones Sun and Moon name.
const ULTRA_SUN_ULTRA_MOON: Edition = Edition {
    species_names: 60,
    move_names: 118,
    move_descriptions: 117,
    ability_names: 101,
    ability_descriptions: 102,
    ..SUN_MOON
};

impl Edition {
    pub fn of(game: GameId) -> Option<&'static Self> {
        match game {
            GameId::PokemonX | GameId::PokemonY => Some(&X_Y),
            GameId::PokemonOmegaRuby | GameId::PokemonAlphaSapphire => {
                Some(&OMEGA_RUBY_ALPHA_SAPPHIRE)
            }
            GameId::PokemonSun | GameId::PokemonMoon => Some(&SUN_MOON),
            GameId::PokemonUltraSun | GameId::PokemonUltraMoon => Some(&ULTRA_SUN_ULTRA_MOON),
            _ => None,
        }
    }

    /// Descriptive names of the main text banks that romkit reads.
    fn bank_name(&self, set: TextSet, bank: usize) -> Option<&'static str> {
        let named = [
            (self.species_names, "species_names"),
            (self.move_names, "move_names"),
            (self.move_descriptions, "move_descriptions"),
            (self.ability_names, "ability_names"),
            (self.ability_descriptions, "ability_descriptions"),
        ];
        let (_, name) = named.into_iter().find(|&(known, _)| known == bank)?;
        (set == TextSet::Main).then_some(name)
    }
}

/// A 3DS Pokémon game opened over a byte source. Opening reads only the image's headers and
/// filesystem; each table is read and parsed when first asked for, then kept.
pub struct N3ds<S> {
    game: GameId,
    edition: &'static Edition,
    rom: N3dsRom<S>,
    /// The tables of the text archives: main text by language, then story text.
    texts: [OnceLock<Garc>; 2 * LANGUAGES.len()],
    species: OnceLock<Vec<Species>>,
    moves: OnceLock<Vec<Move>>,
    abilities: OnceLock<Vec<Ability>>,
}

impl<S: ByteSource> N3ds<S> {
    pub fn open(source: S) -> Result<Self> {
        let rom = N3dsRom::open(source)?;
        let identity = rom.header().identity();
        let game = GameId::identify(&identity)
            .filter(|&game| Edition::of(game).is_some())
            .ok_or_else(|| Error::Unsupported(format!("{identity} is not a 3DS Pokémon game")))?;
        Ok(Self {
            game,
            edition: Edition::of(game).expect("filtered above"),
            rom,
            texts: Default::default(),
            species: OnceLock::new(),
            moves: OnceLock::new(),
            abilities: OnceLock::new(),
        })
    }

    pub fn game(&self) -> GameId {
        self.game
    }

    /// The 3DS container, for files this module does not model.
    pub fn rom(&self) -> &N3dsRom<S> {
        &self.rom
    }

    /// The game's text languages, a prefix of [`LANGUAGES`].
    pub fn languages(&self) -> &'static [&'static str] {
        &LANGUAGES[..self.edition.languages]
    }

    /// One language's text, e.g. `text(TextSet::Main, "en")`.
    pub fn text(&self, set: TextSet, language: &str) -> Result<Text<'_, Window<&S>>> {
        let index = self
            .languages()
            .iter()
            .position(|&known| known == language)
            .ok_or_else(|| Error::NotFound(format!("{language} text")))?;
        let (first, slot) = match set {
            TextSet::Main => (self.edition.main_text, index),
            TextSet::Story => (self.edition.story_text, LANGUAGES.len() + index),
        };
        let archive = self.rom.file(&archive_path(first, index))?;
        let garc = cached(&self.texts[slot], || Garc::parse(&archive))?;
        Ok(Text::new(archive, garc))
    }

    /// Every species and alternate form, indexed by [`SpeciesId`] from 1.
    pub fn species(&self) -> Result<&[Species]> {
        cached(&self.species, || {
            let edition = self.edition;
            let names = self
                .text(TextSet::Main, "en")?
                .strings(edition.species_names)?;
            species::parse(
                edition,
                &self.rom.read(edition.personal)?,
                &self.rom.read(edition.evolutions)?,
                &self.rom.read(edition.learnsets)?,
                &names,
            )
        })
        .map(Vec::as_slice)
    }

    /// Every move, indexed by [`MoveId`] from 1.
    pub fn moves(&self) -> Result<&[Move]> {
        cached(&self.moves, || {
            let text = self.text(TextSet::Main, "en")?;
            let names = text.strings(self.edition.move_names)?;
            let descriptions = text.strings(self.edition.move_descriptions)?;
            moves::parse(
                self.edition.generation,
                &self.rom.read(self.edition.moves)?,
                &names,
                &descriptions,
            )
        })
        .map(Vec::as_slice)
    }

    /// Every ability, with the species that can have it.
    pub fn abilities(&self) -> Result<&[Ability]> {
        cached(&self.abilities, || {
            let text = self.text(TextSet::Main, "en")?;
            let names = text.strings(self.edition.ability_names)?;
            let descriptions = text.strings(self.edition.ability_descriptions)?;
            Ok(species::abilities(self.species()?, &names, &descriptions))
        })
        .map(Vec::as_slice)
    }
}

/// The archive `offset` places after `first` in number order: `a/0/7/9` + 1 is `a/0/8/0`.
fn archive_path(first: &str, offset: usize) -> String {
    let number = first
        .bytes()
        .filter(u8::is_ascii_digit)
        .fold(0, |number, digit| number * 10 + (digit - b'0') as usize)
        + offset;
    format!("a/{}/{}/{}", number / 100, number / 10 % 10, number % 10)
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
    /// Types since Generation VI: Fairy is new, and the `???` type of Generation IV is gone.
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
        Fairy = 17,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_paths_count_in_decimal() {
        assert_eq!(archive_path("a/0/7/9", 1), "a/0/8/0");
        assert_eq!(archive_path("a/0/3/0", 9), "a/0/3/9");
        assert_eq!(archive_path("a/2/1/8", 0), "a/2/1/8");
    }

    #[test]
    fn every_3ds_pokemon_game_has_an_edition() {
        use GameId::*;
        let games = [
            PokemonX,
            PokemonY,
            PokemonOmegaRuby,
            PokemonAlphaSapphire,
            PokemonSun,
            PokemonMoon,
            PokemonUltraSun,
            PokemonUltraMoon,
        ];
        assert!(games.into_iter().all(|game| Edition::of(game).is_some()));
        assert!(Edition::of(PokemonPlatinum).is_none());
        assert_eq!(X_Y.bank_name(TextSet::Main, 80), Some("species_names"));
        assert_eq!(X_Y.bank_name(TextSet::Story, 80), None);
    }
}
