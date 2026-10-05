//! Generation II: Gold, Silver, and Crystal.
//!
//! The engine grew out of Gen 1's: tables at fixed places that differ by release, entries
//! found through pointers into their own bank, and the same text encoding, recorded per release
//! in an [`Edition`]. Species are numbered by the National Pokédex throughout.
//!
//! ```no_run
//! use romkit::games::pokemon::gen2::Gen2;
//! use romkit::io::FileSource;
//!
//! let game = Gen2::open(FileSource::open("crystal.gbc")?)?;
//! for species in game.species()? {
//!     println!("{}: {:?}", species.name, species.types);
//! }
//! # Ok::<(), romkit::Error>(())
//! ```

mod assets;
mod items;
pub mod lz;
mod moves;
mod species;
mod sprites;

use std::sync::OnceLock;

pub use items::{Item, Pocket};
pub use moves::Move;
pub use species::{Evolution, EvolutionMethod, Species, TimeOfDay};

/// Shared with Gen 1, whose tables these extend.
pub use crate::games::pokemon::gen1::Type;
/// Shared with Gen 4, whose records use the same values.
pub use crate::games::pokemon::gen4::{
    EggGroup, GenderRatio, GrowthRate, ItemId, LevelUpMove, MoveId, SpeciesId, Stats,
};

use crate::Result;
use crate::games::GameId;
use crate::games::pokemon::gen1::text::Charmap;
use crate::games::pokemon::gen1::{Bank, cached, open_release, read};
use crate::io::ByteSource;
use crate::media::Rgba;
use crate::platforms::gb::FarPointer;

const SPECIES: usize = 251;
const MOVES: usize = 251;
/// Through HM07; the rest are unused.
const ITEMS: usize = 249;
/// 50 TMs, 7 HMs, and in Crystal 3 move tutors, in a list ended by 0.
const MAX_MACHINES: usize = 64;

/// Where one release keeps its data.
#[derive(Debug)]
pub struct Edition {
    /// One 32-byte record per species.
    base_stats: FarPointer,
    names: FarPointer,
    /// Per species, the address of its evolutions and learnset...
    evolutions: FarPointer,
    /// ...and of its egg moves.
    egg_moves: FarPointer,
    moves: FarPointer,
    move_names: FarPointer,
    /// Per move, the address of its description.
    move_descriptions: FarPointer,
    /// The move each TM, HM, and tutor teaches.
    machines: FarPointer,
    item_names: FarPointer,
    item_attributes: FarPointer,
    item_descriptions: FarPointer,
    /// Per species from the placeholder 0, two colors for normal and two for shiny.
    palettes: FarPointer,
    /// Per species, the far pointers of its front and back pics...
    pics: FarPointer,
    /// ...and Unown's, per letter.
    unown_pics: FarPointer,
    pic_banks: PicBanks,
}

/// The pic pointers store banks that the code translates before switching to them.
#[derive(Debug)]
enum PicBanks {
    /// Gold and Silver move three of the banks: `(stored, actual)`.
    Moved(&'static [(u8, u8)]),
    /// Crystal adds the same number to all.
    Offset(u8),
}

impl PicBanks {
    fn actual(&self, stored: u8) -> u8 {
        match *self {
            Self::Moved(moved) => moved
                .iter()
                .find(|&&(from, _)| from == stored)
                .map_or(stored, |&(_, to)| to),
            Self::Offset(offset) => stored.wrapping_add(offset),
        }
    }
}

const GOLD: Edition = Edition {
    base_stats: FarPointer::new(0x14, 0x5B0B),
    names: FarPointer::new(0x6C, 0x4B74),
    evolutions: FarPointer::new(0x10, 0x67BD),
    egg_moves: FarPointer::new(0x08, 0x79FE),
    moves: FarPointer::new(0x10, 0x5AFE),
    move_names: FarPointer::new(0x6C, 0x5574),
    move_descriptions: FarPointer::new(0x6D, 0x4000),
    machines: FarPointer::new(0x04, 0x5A66),
    item_names: FarPointer::new(0x6C, 0x4000),
    item_attributes: FarPointer::new(0x01, 0x68A0),
    item_descriptions: FarPointer::new(0x6E, 0x4000),
    palettes: FarPointer::new(0x02, 0x6D3D),
    pics: FarPointer::new(0x12, 0x4000),
    unown_pics: FarPointer::new(0x1F, 0x4000),
    pic_banks: PicBanks::Moved(&[(0x13, 0x1F), (0x14, 0x20), (0x1F, 0x2E)]),
};

const CRYSTAL: Edition = Edition {
    base_stats: FarPointer::new(0x14, 0x5424),
    names: FarPointer::new(0x14, 0x7384),
    evolutions: FarPointer::new(0x10, 0x65B1),
    egg_moves: FarPointer::new(0x08, 0x7B11),
    moves: FarPointer::new(0x10, 0x5AFB),
    move_names: FarPointer::new(0x72, 0x5F29),
    move_descriptions: FarPointer::new(0x0B, 0x4B52),
    machines: FarPointer::new(0x04, 0x567A),
    item_names: FarPointer::new(0x72, 0x4000),
    item_attributes: FarPointer::new(0x01, 0x67C1),
    item_descriptions: FarPointer::new(0x72, 0x4987),
    palettes: FarPointer::new(0x02, 0x68CE),
    pics: FarPointer::new(0x48, 0x4000),
    unown_pics: FarPointer::new(0x49, 0x4000),
    pic_banks: PicBanks::Offset(0x36),
};

/// The releases romkit has verified, by the header checksum of their dumps.
const RELEASES: &[(GameId, u16, &Edition)] = &[
    (GameId::PokemonGold, 0x682D, &GOLD),
    (GameId::PokemonCrystal, 0x129F, &CRYSTAL),
];

impl Edition {
    /// Whether romkit has the tables of a release of `game`.
    pub fn covers(game: GameId) -> bool {
        RELEASES.iter().any(|&(known, _, _)| known == game)
    }

    /// The edition of the release of `game` with this header checksum. Other languages and
    /// revisions keep their tables elsewhere, so they are recognized but not extracted.
    pub fn release(game: GameId, checksum: u16) -> Option<&'static Self> {
        RELEASES
            .iter()
            .find(|&&(known, sum, _)| known == game && sum == checksum)
            .map(|&(_, _, edition)| edition)
    }
}

/// The English charmap: Gen 1's, with the umlauts of the European releases, and é moved to
/// make room for more contractions.
pub const ENGLISH: Charmap = Charmap {
    runs: &[
        (0x72, "“”·…"),
        (
            0x7F,
            " ABCDEFGHIJKLMNOPQRSTUVWXYZ():;[]abcdefghijklmnopqrstuvwxyz",
        ),
        (0xC0, "ÄÖÜäöü"),
        (0xDF, "←'"),
        (0xE3, "-"),
        (0xE6, "?!.&é→▷▶▼♂¥×./,♀0123456789"),
    ],
    words: &[
        (0x24, "POKé"),
        (0x4A, "PKMN"),
        (0x54, "POKé"),
        (0x56, "……"),
        (0x5B, "PC"),
        (0x5C, "TM"),
        (0x5D, "TRAINER"),
        (0x5E, "ROCKET"),
        (0x70, "PO"),
        (0x71, "Ké"),
        (0xD0, "'d"),
        (0xD1, "'l"),
        (0xD2, "'m"),
        (0xD3, "'r"),
        (0xD4, "'s"),
        (0xD5, "'t"),
        (0xD6, "'v"),
        (0xE1, "PK"),
        (0xE2, "MN"),
    ],
    borrows_kana: false,
};

/// A Gen 2 game opened over a byte source. Opening reads only the header; each table is read
/// and parsed when first asked for, then kept.
pub struct Gen2<S> {
    game: GameId,
    edition: &'static Edition,
    source: S,
    species: OnceLock<Vec<Species>>,
    moves: OnceLock<Vec<Move>>,
    items: OnceLock<Vec<Item>>,
    palettes: OnceLock<Vec<[[Rgba; 2]; 2]>>,
}

impl<S: ByteSource> Gen2<S> {
    pub fn open(source: S) -> Result<Self> {
        let (game, edition) = open_release(&source, Edition::release, "Gen 2")?;
        Ok(Self {
            game,
            edition,
            source,
            species: OnceLock::new(),
            moves: OnceLock::new(),
            items: OnceLock::new(),
            palettes: OnceLock::new(),
        })
    }

    pub fn game(&self) -> GameId {
        self.game
    }

    /// Every species, by [`SpeciesId`] from 1.
    pub fn species(&self) -> Result<&[Species]> {
        cached(&self.species, || {
            let edition = self.edition;
            let records = read(&self.source, edition.base_stats, SPECIES * 32)?;
            let names = read(&self.source, edition.names, SPECIES * 10)?;
            let names = ENGLISH.fixed(&names, 10);
            let evolutions = Bank::read(&self.source, edition.evolutions.bank)?;
            let egg_moves = Bank::read(&self.source, edition.egg_moves.bank)?;
            species::parse(
                &records,
                &names,
                (&evolutions, edition.evolutions.address),
                (&egg_moves, edition.egg_moves.address),
            )
        })
        .map(Vec::as_slice)
    }

    /// Every move, by [`MoveId`] from 1.
    pub fn moves(&self) -> Result<&[Move]> {
        cached(&self.moves, || {
            let edition = self.edition;
            let records = read(&self.source, edition.moves, MOVES * 7)?;
            let names = Bank::read(&self.source, edition.move_names.bank)?;
            let names = ENGLISH.list(names.from(edition.move_names.address)?, MOVES);
            let descriptions = Bank::read(&self.source, edition.move_descriptions.bank)?;
            let descriptions = described(&descriptions, edition.move_descriptions.address, MOVES)?;
            moves::parse(&records, &names, &descriptions, &self.machines()?)
        })
        .map(Vec::as_slice)
    }

    /// Every item through HM07, by [`ItemId`] from 1.
    pub fn items(&self) -> Result<&[Item]> {
        cached(&self.items, || {
            let edition = self.edition;
            let names = Bank::read(&self.source, edition.item_names.bank)?;
            let names = ENGLISH.list(names.from(edition.item_names.address)?, ITEMS);
            let descriptions = Bank::read(&self.source, edition.item_descriptions.bank)?;
            let descriptions = described(&descriptions, edition.item_descriptions.address, ITEMS)?;
            let attributes = read(&self.source, edition.item_attributes, ITEMS * 7)?;
            items::parse(&attributes, &names, &descriptions)
        })
        .map(Vec::as_slice)
    }

    /// The moves of the TMs, HMs, and (in Crystal) move tutors, in their order.
    fn machines(&self) -> Result<Vec<u8>> {
        let list = read(&self.source, self.edition.machines, MAX_MACHINES)?;
        Ok(list.iter().take_while(|&&id| id != 0).copied().collect())
    }
}

/// `count` texts through a table of their addresses at `table`, all in `bank`.
fn described(bank: &Bank, table: u16, count: usize) -> Result<Vec<String>> {
    let table = bank.reader("Gen 2 description table", table)?;
    (0..count)
        .map(|index| Ok(ENGLISH.decode(bank.from(table.u16_at(index * 2)?)?)))
        .collect()
}
