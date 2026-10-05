//! Species: 28-byte base stats records in Pokédex order, joined with names, evolutions, and
//! learnsets in internal index order.
//!
//! A base stats record holds the Pokédex number, the five stats, two types, the catch rate,
//! the base experience, the size and addresses of the front and back pics, the four moves
//! known at level 1, the growth rate, and 7 bytes of TM and HM compatibility bits.
//!
//! The evolution table points each internal index to its evolutions, ended by 0, then its
//! level-up moves as `(level, move)` pairs, also ended by 0. An evolution is a method byte and
//! its parameters: `1, level, species`; `2, item, 1, species` (the 1 is a minimum level); or
//! `3, 1, species` for trades.

use serde::Serialize;

use super::{Bank, GrowthRate, ItemId, LevelUpMove, MoveId, SpeciesId, Type};
use crate::Result;
use crate::io::Reader;

const RECORD_LEN: usize = 28;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Species {
    pub id: SpeciesId,
    /// The internal index that orders names, evolutions, and learnsets.
    pub index: u8,
    pub name: String,
    /// Single-typed species repeat their type.
    pub types: [Type; 2],
    pub base_stats: Stats,
    pub catch_rate: u8,
    pub base_experience: u8,
    pub growth_rate: GrowthRate,
    /// The moves it knows at level 1, besides those it learns by leveling up.
    pub starting_moves: Vec<MoveId>,
    /// The TMs and HMs it can learn: 0–49 are TM01–TM50, 50–54 are HM01–HM05.
    pub machines: Vec<u8>,
    pub evolutions: Vec<Evolution>,
    /// Moves learned by leveling up, by ascending level.
    pub level_up_moves: Vec<LevelUpMove>,
    #[serde(skip)]
    pub(super) pics: Pics,
}

/// Where a species' pics are, within the bank its index puts them in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Pics {
    pub front: u16,
    pub back: u16,
}

/// Gen 1 has a single Special stat for attack and defense.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Stats {
    pub hp: u8,
    pub attack: u8,
    pub defense: u8,
    pub speed: u8,
    pub special: u8,
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
    Level { level: u8 },
    UseItem { item: ItemId },
    Trade,
}

/// Every species, from base stats records in Pokédex order, the Pokédex number of each
/// internal index, names by internal index, and the bank of the evolution table at `table`.
pub(super) fn parse(
    records: &[u8],
    pokedex: &[u8],
    names: &[String],
    evolutions: &Bank,
    table: u16,
) -> Result<Vec<Species>> {
    let table = evolutions.reader("Gen 1 evolution table", table)?;
    records
        .as_chunks::<RECORD_LEN>()
        .0
        .iter()
        .enumerate()
        .map(|(position, record)| {
            let dex = position as u8 + 1;
            let mut species = parse_record(record)?;
            if species.id.0 != u16::from(dex) {
                let reader = Reader::new("Gen 1 base stats", records);
                return Err(reader.error_at(position * RECORD_LEN, "out of Pokédex order"));
            }
            let index = pokedex
                .iter()
                .position(|&number| number == dex)
                .ok_or_else(|| {
                    Reader::new("Gen 1 Pokédex order", pokedex)
                        .error_at(0, "species without an index")
                })?;
            species.index = index as u8 + 1;
            species.name = names.get(index).cloned().unwrap_or_default();
            let address = table.u16_at(index * 2)?;
            let mut list = evolutions.reader("Gen 1 evolutions and learnset", address)?;
            species.evolutions = parse_evolutions(&mut list, pokedex)?;
            species.level_up_moves = parse_learnset(&mut list)?;
            Ok(species)
        })
        .collect()
}

fn parse_record(bytes: &[u8]) -> Result<Species> {
    let mut record = Reader::new("Gen 1 base stats", bytes);
    let id = SpeciesId(record.u8()?.into());
    let base_stats = Stats {
        hp: record.u8()?,
        attack: record.u8()?,
        defense: record.u8()?,
        speed: record.u8()?,
        special: record.u8()?,
    };
    let types = [Type::from_raw(record.u8()?), Type::from_raw(record.u8()?)];
    let catch_rate = record.u8()?;
    let base_experience = record.u8()?;
    record.skip(1)?; // the front pic's size, which the pic repeats
    let pics = Pics {
        front: record.u16()?,
        back: record.u16()?,
    };
    let starting_moves = record
        .take(4)?
        .iter()
        .filter(|&&id| id != 0)
        .map(|&id| MoveId(id.into()))
        .collect();
    let growth_rate = GrowthRate::from_raw(record.u8()?);
    let machines = bits(record.take(7)?, super::MACHINES);
    Ok(Species {
        id,
        index: 0,
        name: String::new(),
        types,
        base_stats,
        catch_rate,
        base_experience,
        growth_rate,
        starting_moves,
        machines,
        evolutions: Vec::new(),
        level_up_moves: Vec::new(),
        pics,
    })
}

/// The positions of the bits set among the first `count`, from the low bit of the first byte.
pub(crate) fn bits(bytes: &[u8], count: usize) -> Vec<u8> {
    (0..count)
        .filter(|&bit| {
            bytes
                .get(bit / 8)
                .is_some_and(|byte| byte >> (bit % 8) & 1 != 0)
        })
        .map(|bit| bit as u8)
        .collect()
}

/// Evolutions up to the 0 that ends them; their targets are internal indices.
fn parse_evolutions(list: &mut Reader, pokedex: &[u8]) -> Result<Vec<Evolution>> {
    let mut evolutions = Vec::new();
    loop {
        let at = list.position();
        let method = match list.u8()? {
            0 => return Ok(evolutions),
            1 => EvolutionMethod::Level { level: list.u8()? },
            2 => {
                let item = ItemId(list.u8()?.into());
                list.skip(1)?;
                EvolutionMethod::UseItem { item }
            }
            3 => {
                list.skip(1)?;
                EvolutionMethod::Trade
            }
            _ => return Err(list.error_at(at, "unknown evolution method")),
        };
        let index = list.u8()?;
        let into = usize::from(index)
            .checked_sub(1)
            .and_then(|index| pokedex.get(index))
            .filter(|&&dex| dex != 0)
            .ok_or_else(|| list.error_at(at, "evolves into no species"))?;
        evolutions.push(Evolution {
            method,
            into: SpeciesId((*into).into()),
        });
    }
}

/// `(level, move)` pairs up to a 0 level.
pub(crate) fn parse_learnset(list: &mut Reader) -> Result<Vec<LevelUpMove>> {
    let mut moves = Vec::new();
    loop {
        match list.u8()? {
            0 => return Ok(moves),
            level => moves.push(LevelUpMove {
                level,
                r#move: MoveId(list.u8()?.into()),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_records_evolutions_and_learnsets() {
        // Pikachu (index 0x54), evolving into Raichu (index 0x55) with a Thunder Stone.
        let record = [
            25, 35, 55, 30, 90, 50, 0x17, 0x17, 190, 82, 0x55, 0x7D, 0x4D, 0x8B, 0x4E, 84, 45, 0,
            0, 0, 0xC2, 0x43, 0, 0, 0, 0, 0, 0,
        ];
        let mut records: Vec<u8> = (1..=24).flat_map(|dex| [dex, 0, 0, 0].repeat(7)).collect();
        records.extend(record);
        let mut pokedex = vec![0; 190];
        pokedex[..24].copy_from_slice(&(1..=24).collect::<Vec<u8>>());
        pokedex[0x53] = 25;
        pokedex[0x54] = 26;

        // The table at 0x4000 points every index to 0x4200, where the lists are.
        let mut bank = 0x4200u16.to_le_bytes().repeat(190);
        bank.resize(0x4000, 0);
        bank[0x200..0x20A].copy_from_slice(&[2, 0x21, 1, 0x55, 0, 9, 86, 16, 98, 0]);
        let bank = Bank {
            number: 1,
            bytes: bank.into(),
        };
        let species = parse(&records, &pokedex, &[], &bank, 0x4000).unwrap();
        let pikachu = &species[24];
        assert_eq!((pikachu.id, pikachu.index), (SpeciesId(25), 0x54));
        assert_eq!(pikachu.types, [Type::Electric; 2]);
        assert_eq!(pikachu.base_stats.special, 50);
        assert_eq!(pikachu.starting_moves, [MoveId(84), MoveId(45)]);
        // TM01 is bit 0: 0xC2 sets TM02, TM07, TM08; 0x43 sets TM09, TM10, TM15.
        assert_eq!(pikachu.machines, [1, 6, 7, 8, 9, 14]);
        assert_eq!(
            pikachu.evolutions,
            [Evolution {
                method: EvolutionMethod::UseItem { item: ItemId(0x21) },
                into: SpeciesId(26)
            }]
        );
        assert_eq!(pikachu.level_up_moves[1].level, 16);

        // Out of order records, unknown methods, and unterminated lists are errors.
        records[24 * RECORD_LEN] = 7;
        assert!(parse(&records, &pokedex, &[], &bank, 0x4000).is_err());
        let mut list = Reader::new("test", &[9, 1, 2, 0]);
        assert!(parse_evolutions(&mut list, &pokedex).is_err());
        assert!(parse_learnset(&mut Reader::new("test", &[5, 1])).is_err());
    }
}
