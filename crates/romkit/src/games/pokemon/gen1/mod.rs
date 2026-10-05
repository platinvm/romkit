//! Generation I: Red, Green, Blue, and Yellow.
//!
//! The games keep their data in tables at fixed places in ROM, which differ from release to
//! release; an [`Edition`] records them for each release romkit knows, told apart by the
//! header's checksum. Tables point to their variable-length entries with addresses inside
//! their own 16 KB bank.
//!
//! Species have two numbers: the National Pokédex number, which orders the base stats, and an
//! internal index (Rhydon is 1), which orders names, evolutions, and learnsets. A table in ROM
//! maps one to the other.
//!
//! ```no_run
//! use romkit::games::pokemon::gen1::Gen1;
//! use romkit::io::FileSource;
//!
//! let game = Gen1::open(FileSource::open("red.gb")?)?;
//! for species in game.species()? {
//!     println!("{}: {:?}", species.name, species.types);
//! }
//! # Ok::<(), romkit::Error>(())
//! ```

mod assets;
mod items;
mod moves;
pub mod pic;
mod species;
mod sprites;
pub mod text;

use std::borrow::Cow;
use std::sync::OnceLock;

pub(crate) use assets::sprite_path;
pub use items::Item;
pub use moves::Move;
pub(crate) use moves::percent;
pub use species::{Evolution, EvolutionMethod, Species, Stats};
pub(crate) use species::{bits, parse_learnset};
pub(crate) use sprites::in_front_box;
use text::Charmap;

/// Shared with Gen 4, whose records use the same values. A [`SpeciesId`] here is the National
/// Pokédex number; [`ItemId`] and [`MoveId`] are the games' own numbers.
pub use crate::games::pokemon::gen4::{GrowthRate, ItemId, LevelUpMove, MoveId, SpeciesId};

use crate::games::GameId;
use crate::io::{ByteSource, Reader};
use crate::media::Rgba;
use crate::platforms::gb::{self, BANK_SIZE, FarPointer};
use crate::{Error, Result};

/// Species have internal indices 1 to 190, many of them unused.
const INDICES: usize = 190;
const MOVES: usize = 165;
/// Items 84 and up are the floors of the elevator menus.
const ITEMS: usize = 83;
/// 50 TMs, then 5 HMs.
const MACHINES: usize = 55;
/// The palette table has an entry for the placeholder 0, then one per Pokédex number.
const PALETTE_ENTRIES: usize = 152;

/// Where one release keeps its data.
#[derive(Debug)]
pub struct Edition {
    charmap: &'static Charmap,
    /// The length of a species name: 10 bytes, 5 in Japanese.
    name_len: usize,
    /// One 28-byte record per species, by Pokédex number.
    base_stats: FarPointer,
    species: usize,
    /// Red, Green, and Blue keep Mew's record apart from the others.
    mew: Option<FarPointer>,
    /// Pokédex numbers, by internal index.
    pokedex: FarPointer,
    names: FarPointer,
    /// Per internal index, the address of its evolutions and learnset.
    evolutions: FarPointer,
    moves: FarPointer,
    move_names: FarPointer,
    /// The move each TM and HM teaches.
    machines: FarPointer,
    item_names: FarPointer,
    /// Three bytes of binary-coded decimal per item.
    item_prices: FarPointer,
    /// The Super Game Boy palette of each Pokémon...
    palette_ids: FarPointer,
    /// ...and the palettes, four colors each.
    palettes: FarPointer,
    pic_banks: PicBanks,
}

/// Which bank holds a species' pics, by internal index: banks are filled in index order, so
/// the code compares the index with the last one of each bank.
#[derive(Debug)]
struct PicBanks {
    /// `(last index, bank)`, by ascending index; later indices are in the last bank.
    ranges: &'static [(u8, u8)],
    /// Mew, added late, has its pics with its record.
    mew: Option<u8>,
}

/// Mew's internal index.
const MEW: u8 = 0x15;

const RED_BLUE: Edition = Edition {
    charmap: &text::ENGLISH,
    name_len: 10,
    base_stats: FarPointer::new(0x0E, 0x43DE),
    species: 150,
    mew: Some(FarPointer::new(0x01, 0x425B)),
    pokedex: FarPointer::new(0x10, 0x5024),
    names: FarPointer::new(0x07, 0x421E),
    evolutions: FarPointer::new(0x0E, 0x705C),
    moves: FarPointer::new(0x0E, 0x4000),
    move_names: FarPointer::new(0x2C, 0x4000),
    machines: FarPointer::new(0x04, 0x7773),
    item_names: FarPointer::new(0x01, 0x472B),
    item_prices: FarPointer::new(0x01, 0x4608),
    palette_ids: FarPointer::new(0x1C, 0x65C8),
    palettes: FarPointer::new(0x1C, 0x6660),
    pic_banks: PicBanks {
        ranges: &[
            (0x1E, 0x09),
            (0x49, 0x0A),
            (0x73, 0x0B),
            (0x98, 0x0C),
            (0xFF, 0x0D),
        ],
        mew: Some(0x01),
    },
};

/// Yellow keeps Mew with the others, and has palettes of its own for the Game Boy Color.
const YELLOW: Edition = Edition {
    species: 151,
    mew: None,
    pokedex: FarPointer::new(0x10, 0x50B1),
    names: FarPointer::new(0x3A, 0x4000),
    evolutions: FarPointer::new(0x0E, 0x71E5),
    move_names: FarPointer::new(0x2F, 0x4000),
    machines: FarPointer::new(0x04, 0x632D),
    item_names: FarPointer::new(0x01, 0x45B7),
    item_prices: FarPointer::new(0x01, 0x4494),
    palette_ids: FarPointer::new(0x1C, 0x6921),
    palettes: FarPointer::new(0x1C, 0x69B9),
    pic_banks: PicBanks {
        mew: None,
        ..RED_BLUE.pic_banks
    },
    ..RED_BLUE
};

/// The Japanese Green (Midori), with the original pics and five-character names.
const GREEN: Edition = Edition {
    charmap: &text::JAPANESE,
    name_len: 5,
    base_stats: FarPointer::new(0x0E, 0x4000),
    mew: Some(FarPointer::new(0x01, 0x4200)),
    pokedex: FarPointer::new(0x10, 0x679A),
    names: FarPointer::new(0x0E, 0x5068),
    evolutions: FarPointer::new(0x0E, 0x7427),
    moves: FarPointer::new(0x0E, 0x5658),
    move_names: FarPointer::new(0x04, 0x4000),
    machines: FarPointer::new(0x04, 0x6276),
    item_names: FarPointer::new(0x01, 0x433F),
    item_prices: FarPointer::new(0x01, 0x421C),
    palette_ids: FarPointer::new(0x1C, 0x6A1E),
    palettes: FarPointer::new(0x1C, 0x6AB6),
    pic_banks: PicBanks {
        ranges: &[
            (0x1E, 0x09),
            (0x49, 0x0A),
            (0x74, 0x0B),
            (0x99, 0x0C),
            (0xFF, 0x0D),
        ],
        mew: Some(0x01),
    },
    ..RED_BLUE
};

/// The releases romkit has verified, by the header checksum of their dumps.
const RELEASES: &[(GameId, u16, &Edition)] = &[
    (GameId::PokemonRed, 0x91E6, &RED_BLUE),
    (GameId::PokemonBlue, 0x9D0A, &RED_BLUE),
    (GameId::PokemonYellow, 0x047C, &YELLOW),
    (GameId::PokemonGreen, 0xDDD5, &GREEN),
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

/// A Gen 1 game opened over a byte source. Opening reads only the header; each table is read
/// and parsed when first asked for, then kept.
pub struct Gen1<S> {
    game: GameId,
    edition: &'static Edition,
    source: S,
    species: OnceLock<Vec<Species>>,
    moves: OnceLock<Vec<Move>>,
    items: OnceLock<Vec<Item>>,
    palettes: OnceLock<Vec<[Rgba; 4]>>,
}

impl<S: ByteSource> Gen1<S> {
    pub fn open(source: S) -> Result<Self> {
        let (game, edition) = open_release(&source, Edition::release, "Gen 1")?;
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

    /// Every species, by Pokédex number from 1.
    pub fn species(&self) -> Result<&[Species]> {
        cached(&self.species, || {
            let edition = self.edition;
            let mut records = read(&self.source, edition.base_stats, edition.species * 28)?;
            if let Some(mew) = edition.mew {
                records
                    .to_mut()
                    .extend_from_slice(&read(&self.source, mew, 28)?);
            }
            let pokedex = read(&self.source, edition.pokedex, INDICES)?;
            let names = read(&self.source, edition.names, INDICES * edition.name_len)?;
            let names = edition.charmap.fixed(&names, edition.name_len);
            let evolutions = Bank::read(&self.source, edition.evolutions.bank)?;
            species::parse(
                &records,
                &pokedex,
                &names,
                &evolutions,
                edition.evolutions.address,
            )
        })
        .map(Vec::as_slice)
    }

    /// Every move, by [`MoveId`] from 1.
    pub fn moves(&self) -> Result<&[Move]> {
        cached(&self.moves, || {
            let edition = self.edition;
            let records = read(&self.source, edition.moves, MOVES * 6)?;
            let names = Bank::read(&self.source, edition.move_names.bank)?;
            let names = edition
                .charmap
                .list(names.from(edition.move_names.address)?, MOVES);
            let machines = read(&self.source, edition.machines, MACHINES)?;
            moves::parse(&records, &names, &machines)
        })
        .map(Vec::as_slice)
    }

    /// Every item but the TMs and HMs, by [`ItemId`] from 1.
    pub fn items(&self) -> Result<&[Item]> {
        cached(&self.items, || {
            let edition = self.edition;
            let names = Bank::read(&self.source, edition.item_names.bank)?;
            let names = edition
                .charmap
                .list(names.from(edition.item_names.address)?, ITEMS);
            let prices = read(&self.source, edition.item_prices, ITEMS * 3)?;
            items::parse(&names, &prices)
        })
        .map(Vec::as_slice)
    }
}

/// The game and edition of a Game Boy ROM, from its header: its title names the game and its
/// checksum the release, which `release` looks up.
pub(crate) fn open_release<E>(
    source: &impl ByteSource,
    release: impl Fn(GameId, u16) -> Option<&'static E>,
    generation: &str,
) -> Result<(GameId, &'static E)> {
    let head = source.read_up_to(0, 0x150)?;
    let identity = crate::identify(source)?;
    let checksum = gb::global_checksum(&head)?;
    let game = GameId::identify(&identity).ok_or_else(|| {
        Error::Unsupported(format!("{identity} is not a {generation} Pokémon game"))
    })?;
    let edition = release(game, checksum).ok_or_else(|| {
        Error::Unsupported(format!(
            "{identity} (checksum {checksum:04x}) is a release of {} romkit has no tables for",
            game.name()
        ))
    })?;
    Ok((game, edition))
}

/// `len` bytes at `at`.
pub(crate) fn read(source: &impl ByteSource, at: FarPointer, len: usize) -> Result<Cow<'_, [u8]>> {
    let offset = at.offset().ok_or_else(|| {
        Error::malformed("Game Boy ROM", at.address.into(), "pointer outside ROM")
    })?;
    source.read(offset, len as u64)
}

/// One 16 KB bank of the ROM, for following the addresses that point into it.
pub(crate) struct Bank<'a> {
    pub(crate) number: u8,
    pub(crate) bytes: Cow<'a, [u8]>,
}

impl<'a> Bank<'a> {
    pub(crate) fn read(source: &'a impl ByteSource, number: u8) -> Result<Self> {
        let offset = u64::from(number) * BANK_SIZE;
        let bytes = source
            .read(offset, BANK_SIZE)
            .map_err(|_| Error::malformed("Game Boy ROM", offset, "bank past the end"))?;
        Ok(Self { number, bytes })
    }

    /// The bank's bytes from `address`, as the game sees it while the bank is switched in.
    pub(crate) fn from(&self, address: u16) -> Result<&[u8]> {
        let base = match self.number {
            0 => 0,
            _ => BANK_SIZE as u16,
        };
        address
            .checked_sub(base)
            .and_then(|start| self.bytes.get(start.into()..))
            .ok_or_else(|| {
                Error::malformed("Game Boy ROM", address.into(), "pointer outside its bank")
            })
    }

    /// A reader over the bank's bytes from `address`; error offsets count from there.
    pub(crate) fn reader(&self, format: &'static str, address: u16) -> Result<Reader<'_>> {
        Ok(Reader::new(format, self.from(address)?))
    }
}

/// The value in `cell`, computed on first use. A failure is returned, not kept.
pub(crate) fn cached<T>(cell: &OnceLock<T>, init: impl FnOnce() -> Result<T>) -> Result<&T> {
    if let Some(value) = cell.get() {
        return Ok(value);
    }
    let value = init()?;
    Ok(cell.get_or_init(|| value))
}

raw_enum! {
    /// The types of Gen 1 and Gen 2, which number them alike. Gen 2 adds Steel, Dark, and the
    /// `???` type of Curse; Bird is an unused leftover.
    pub enum Type: u8 {
        Normal = 0x00,
        Fighting = 0x01,
        Flying = 0x02,
        Poison = 0x03,
        Ground = 0x04,
        Rock = 0x05,
        Bird = 0x06,
        Bug = 0x07,
        Ghost = 0x08,
        Steel = 0x09,
        /// The `???` type of Curse.
        Mystery = 0x13,
        Fire = 0x14,
        Water = 0x15,
        Grass = 0x16,
        Electric = 0x17,
        Psychic = 0x18,
        Ice = 0x19,
        Dragon = 0x1A,
        Dark = 0x1B,
    }
}
