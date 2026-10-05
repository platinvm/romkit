//! Species: 32-byte base stats records in Pokédex order, joined with names, evolutions and
//! learnsets, and egg moves.
//!
//! A record holds the Pokédex number, six stats, two types, the catch rate, the base
//! experience, two wild held items, the gender ratio, the egg cycles, the front pic's size, the
//! growth rate, the egg groups (a nibble each), and 8 bytes of TM, HM, and tutor bits.
//!
//! The evolution table points each species to its evolutions, ended by 0, then its level-up
//! moves as `(level, move)` pairs, also ended by 0. An evolution is a method byte, one
//! parameter, and the species: by level (1), item (2), trade holding an item or 0xFF for none
//! (3), friendship at a time of day (4), or level with Attack against Defense (5, then the
//! level and the comparison). The egg move table points to lists ended by 0xFF.

use serde::Serialize;

use super::Type;
use super::{EggGroup, GenderRatio, GrowthRate, ItemId, LevelUpMove, MoveId, SpeciesId, Stats};
use crate::Result;
use crate::games::pokemon::gen1::Bank;
use crate::games::pokemon::gen1::{bits, parse_learnset};
use crate::io::Reader;

const RECORD_LEN: usize = 32;
const EGG_MOVES_END: u8 = 0xFF;
const NO_ITEM: u8 = 0xFF;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Species {
    pub id: SpeciesId,
    pub name: String,
    /// Single-typed species repeat their type.
    pub types: [Type; 2],
    pub base_stats: Stats,
    pub catch_rate: u8,
    pub base_experience: u8,
    /// Items wild ones may hold: commonly, then rarely.
    pub held_items: [Option<ItemId>; 2],
    pub gender_ratio: GenderRatio,
    /// Each cycle is 256 steps.
    pub hatch_cycles: u8,
    pub growth_rate: GrowthRate,
    pub egg_groups: [EggGroup; 2],
    /// The TMs, HMs, and tutor moves it can learn: 0–49 are TM01–TM50, 50–56 are HM01–HM07,
    /// and in Crystal 57–59 are the move tutor's Flamethrower, Thunderbolt, and Ice Beam.
    pub machines: Vec<u8>,
    pub evolutions: Vec<Evolution>,
    /// Moves learned by leveling up, by ascending level.
    pub level_up_moves: Vec<LevelUpMove>,
    /// Moves its eggs can inherit from the father.
    pub egg_moves: Vec<MoveId>,
    /// The front pic's size in tiles, which the pic itself does not say.
    #[serde(skip)]
    pub(super) pic_size: (u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Evolution {
    #[serde(flatten)]
    pub method: EvolutionMethod,
    pub into: SpeciesId,
}

/// How an evolution is triggered. Named as in Gen 4, whose methods include these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum EvolutionMethod {
    Level {
        level: u8,
    },
    UseItem {
        item: ItemId,
    },
    Trade,
    TradeHolding {
        item: ItemId,
    },
    /// Level up with high friendship, at any time or at a time of day.
    Friendship {
        time: TimeOfDay,
    },
    /// Level up with Attack greater than Defense.
    LevelAttackGreater {
        level: u8,
    },
    LevelAttackEqual {
        level: u8,
    },
    LevelAttackLess {
        level: u8,
    },
}

raw_enum! {
    pub enum TimeOfDay: u8 {
        Any = 1,
        /// Morning or day.
        Day = 2,
        Night = 3,
    }
}

/// Every species, from base stats records, names, and the banks and addresses of the evolution
/// and egg move tables.
pub(super) fn parse(
    records: &[u8],
    names: &[String],
    (evolutions, evolution_table): (&Bank, u16),
    (egg_moves, egg_move_table): (&Bank, u16),
) -> Result<Vec<Species>> {
    let evolution_table = evolutions.reader("Gen 2 evolution table", evolution_table)?;
    let egg_move_table = egg_moves.reader("Gen 2 egg move table", egg_move_table)?;
    records
        .as_chunks::<RECORD_LEN>()
        .0
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let mut species = parse_record(record)?;
            if usize::from(species.id.0) != index + 1 {
                let records = Reader::new("Gen 2 base stats", records);
                return Err(records.error_at(index * RECORD_LEN, "out of Pokédex order"));
            }
            species.name = names.get(index).cloned().unwrap_or_default();
            let address = evolution_table.u16_at(index * 2)?;
            let mut list = evolutions.reader("Gen 2 evolutions and learnset", address)?;
            species.evolutions = parse_evolutions(&mut list)?;
            species.level_up_moves = parse_learnset(&mut list)?;
            let address = egg_move_table.u16_at(index * 2)?;
            let list = egg_moves.from(address)?;
            let end = list
                .iter()
                .position(|&id| id == EGG_MOVES_END)
                .ok_or_else(|| {
                    Reader::new("Gen 2 egg moves", list).error_at(0, "list without an end")
                })?;
            species.egg_moves = list[..end].iter().map(|&id| MoveId(id.into())).collect();
            Ok(species)
        })
        .collect()
}

fn parse_record(bytes: &[u8]) -> Result<Species> {
    let mut record = Reader::new("Gen 2 base stats", bytes);
    let id = SpeciesId(record.u8()?.into());
    let base_stats = Stats {
        hp: record.u8()?,
        attack: record.u8()?,
        defense: record.u8()?,
        speed: record.u8()?,
        special_attack: record.u8()?,
        special_defense: record.u8()?,
    };
    let types = [Type::from_raw(record.u8()?), Type::from_raw(record.u8()?)];
    let catch_rate = record.u8()?;
    let base_experience = record.u8()?;
    let item = |raw: u8| (raw != 0).then_some(ItemId(raw.into()));
    let held_items = [item(record.u8()?), item(record.u8()?)];
    let gender_ratio = GenderRatio::from_raw(record.u8()?);
    record.skip(1)?; // always 100
    let hatch_cycles = record.u8()?;
    record.skip(1)?; // always 5
    let size = record.u8()?;
    record.skip(4)?; // unused pointers
    let growth_rate = GrowthRate::from_raw(record.u8()?);
    let groups = record.u8()?;
    let machines = bits(record.take(8)?, 64);
    Ok(Species {
        id,
        name: String::new(),
        types,
        base_stats,
        catch_rate,
        base_experience,
        held_items,
        gender_ratio,
        hatch_cycles,
        growth_rate,
        egg_groups: [
            EggGroup::from_raw(groups >> 4),
            EggGroup::from_raw(groups & 0xF),
        ],
        machines,
        evolutions: Vec::new(),
        level_up_moves: Vec::new(),
        egg_moves: Vec::new(),
        pic_size: (size >> 4, size & 0xF),
    })
}

/// Evolutions up to the 0 that ends them.
fn parse_evolutions(list: &mut Reader) -> Result<Vec<Evolution>> {
    let mut evolutions = Vec::new();
    loop {
        let at = list.position();
        let method = list.u8()?;
        if method == 0 {
            return Ok(evolutions);
        }
        let parameter = list.u8()?;
        let item = ItemId(parameter.into());
        let method = match (method, parameter) {
            (1, level) => EvolutionMethod::Level { level },
            (2, _) => EvolutionMethod::UseItem { item },
            (3, NO_ITEM) => EvolutionMethod::Trade,
            (3, _) => EvolutionMethod::TradeHolding { item },
            (4, time) => EvolutionMethod::Friendship {
                time: TimeOfDay::from_raw(time),
            },
            (5, level) => match list.u8()? {
                1 => EvolutionMethod::LevelAttackLess { level },
                2 => EvolutionMethod::LevelAttackGreater { level },
                3 => EvolutionMethod::LevelAttackEqual { level },
                _ => return Err(list.error_at(at, "unknown stat comparison")),
            },
            _ => return Err(list.error_at(at, "unknown evolution method")),
        };
        evolutions.push(Evolution {
            method,
            into: SpeciesId(list.u8()?.into()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_records_evolutions_and_egg_moves() {
        let mut record = [0; RECORD_LEN];
        record[..17].copy_from_slice(&[
            1, 45, 49, 49, 45, 65, 65, 0x16, 0x03, 45, 64, 0, 0, 0x1F, 100, 20, 5,
        ]);
        record[17] = 0x55;
        record[22..25].copy_from_slice(&[3, 0x17, 0b0000_0001]);
        // Evolutions at 0x4100, egg moves at 0x4200, through tables at 0x4000.
        let mut bank = vec![0; 0x4000];
        bank[..2].copy_from_slice(&0x4100u16.to_le_bytes());
        bank[2..4].copy_from_slice(&0x4200u16.to_le_bytes());
        bank[0x100..0x10A].copy_from_slice(&[5, 20, 2, 2, 1, 16, 2, 0, 1, 33]);
        bank[0x10A] = 0;
        bank[0x200..0x203].copy_from_slice(&[0x71, 0x82, EGG_MOVES_END]);
        let bank = Bank {
            number: 1,
            bytes: bank.into(),
        };
        let species = parse(
            &record,
            &["BULBASAUR".to_owned()],
            (&bank, 0x4000),
            (&bank, 0x4002),
        );
        let bulbasaur = &species.unwrap()[0];
        assert_eq!(bulbasaur.types, [Type::Grass, Type::Poison]);
        assert_eq!(bulbasaur.base_stats.special_defense, 65);
        assert_eq!(bulbasaur.egg_groups, [EggGroup::Monster, EggGroup::Grass]);
        assert_eq!(bulbasaur.machines, [0]);
        assert_eq!(
            bulbasaur.evolutions[0],
            Evolution {
                method: EvolutionMethod::LevelAttackGreater { level: 20 },
                into: SpeciesId(2)
            }
        );
        assert_eq!(
            bulbasaur.evolutions[1].method,
            EvolutionMethod::Level { level: 16 }
        );
        assert_eq!(bulbasaur.level_up_moves.len(), 1);
        assert_eq!(bulbasaur.egg_moves, [MoveId(0x71), MoveId(0x82)]);
        assert_eq!(bulbasaur.pic_size, (5, 5));
        assert!(parse_evolutions(&mut Reader::new("test", &[9, 1, 1])).is_err());
    }
}
