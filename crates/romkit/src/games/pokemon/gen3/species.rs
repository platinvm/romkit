//! Species: base stats (28 bytes, laid out as the first 28 of Gen 4's personal records), names,
//! evolutions, TM/HM bits, and level-up learnsets, one entry per internal species in each
//! table; egg moves in a list of their own. Species references are converted from internal
//! indices to National Pokédex numbers.

use std::collections::BTreeMap;

use serde::Serialize;

use super::{Ability, AbilityId, Color, EggGroup, Evolution, EvolutionMethod, GenderRatio};
use super::{FORMAT, Gen3, INTERNAL_SPECIES, NATIONAL_DEX, text};
use super::{GrowthRate, HeldItems, ItemId, LevelUpMove, MoveId, SpeciesId, Stats, Type};
use crate::io::{ByteSource, Reader};
use crate::{Error, Result};

const NAME_LEN: usize = 11;
const BASE_STATS_LEN: usize = 28;
const EVOLUTION_LEN: usize = 8;
const EVOLUTION_SLOTS: usize = 5;
const MACHINES_LEN: usize = 8;
/// 50 TMs, then 8 HMs.
const MACHINES: u8 = 58;
const LEARNSET_END: u16 = 0xFFFF;
/// Learnsets hold about twenty moves; one that runs longer has no terminator.
const MAX_LEARNSET: u64 = 64;
/// In the egg move list, a species' moves follow its internal index plus this.
const EGG_SPECIES: u16 = 20000;
const MAX_EGG_MOVES: u64 = 0x2000;
const ABILITIES: usize = 78;
const ABILITY_NAME_LEN: usize = 13;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Species {
    /// The National Pokédex number.
    pub id: SpeciesId,
    /// The games' internal index, which differs from the Pokédex number from Treecko on.
    pub index: u16,
    pub name: String,
    /// Single-typed species repeat their type.
    pub types: [Type; 2],
    pub base_stats: Stats,
    pub ev_yield: Stats,
    pub abilities: [Option<AbilityId>; 2],
    pub catch_rate: u8,
    pub base_experience: u8,
    pub base_friendship: u8,
    pub growth_rate: GrowthRate,
    pub gender_ratio: GenderRatio,
    pub egg_groups: [EggGroup; 2],
    pub hatch_cycles: u8,
    pub held_items: HeldItems,
    pub safari_flee_rate: u8,
    pub color: Color,
    /// Whether its sprite is drawn as is where others are mirrored, as on the summary screen.
    pub no_flip: bool,
    /// The TMs and HMs it can learn: 0–49 are TM01–TM50, 50–57 are HM01–HM08.
    pub machines: Vec<u8>,
    pub evolutions: Vec<Evolution>,
    /// Moves learned by leveling up, by ascending level.
    pub level_up_moves: Vec<LevelUpMove>,
    /// Moves its eggs can inherit from the father.
    pub egg_moves: Vec<MoveId>,
}

/// Every species in National Pokédex order. The 25 unused internal slots after Celebi are
/// left out.
pub(super) fn parse<S: ByteSource>(game: &Gen3<S>) -> Result<Vec<Species>> {
    let edition = game.edition;
    let names = game.read(edition.species_names, INTERNAL_SPECIES * NAME_LEN)?;
    let stats = game.read(edition.base_stats, INTERNAL_SPECIES * BASE_STATS_LEN)?;
    let evolutions = game.read(
        edition.evolutions,
        INTERNAL_SPECIES * EVOLUTION_LEN * EVOLUTION_SLOTS,
    )?;
    let machines = game.read(edition.machines, INTERNAL_SPECIES * MACHINES_LEN)?;
    let dex = game.national_dex()?;
    // None for the unused slots.
    let national = |index: u16| -> Option<SpeciesId> {
        let number = *dex.get((index as usize).checked_sub(1)?)?;
        (1..=NATIONAL_DEX)
            .contains(&number)
            .then_some(SpeciesId(number))
    };
    let mut egg_moves = egg_moves(game)?;
    let mut species = Vec::new();
    for index in 1..INTERNAL_SPECIES as u16 {
        let Some(id) = national(index) else {
            continue;
        };
        let at = index as usize;
        let mut record = parse_base_stats(&stats[at * BASE_STATS_LEN..][..BASE_STATS_LEN])?;
        record.id = id;
        record.index = index;
        record.name = text::decode(&names[at * NAME_LEN..][..NAME_LEN]);
        let bits = u64::from_le_bytes(machines[at * MACHINES_LEN..][..8].try_into().expect("8"));
        record.machines = (0..MACHINES).filter(|bit| bits >> bit & 1 != 0).collect();
        for slot in 0..EVOLUTION_SLOTS {
            let start = (at * EVOLUTION_SLOTS + slot) * EVOLUTION_LEN;
            let field = |offset: usize| {
                u16::from_le_bytes([evolutions[start + offset], evolutions[start + offset + 1]])
            };
            if field(0) != 0 {
                let offset = edition.evolutions as u64 + start as u64;
                record.evolutions.push(Evolution {
                    method: EvolutionMethod::decode(field(0), field(2)),
                    into: national(field(4)).ok_or_else(|| {
                        Error::malformed(FORMAT, offset, "evolves into no species")
                    })?,
                });
            }
        }
        record.level_up_moves = learnset(game, edition.learnsets + 4 * index as u32)?;
        record.egg_moves = egg_moves.remove(&index).unwrap_or_default();
        species.push(record);
    }
    species.sort_by_key(|species| species.id);
    if (1..=NATIONAL_DEX).ne(species.iter().map(|species| species.id.0)) {
        return Err(Error::malformed(
            FORMAT,
            edition.national_dex.into(),
            "Pokédex numbers are not each used once",
        ));
    }
    Ok(species)
}

fn parse_base_stats(bytes: &[u8]) -> Result<Species> {
    let mut record = Reader::new("Gen 3 base stats", bytes);
    let mut stats = || -> Result<Stats> {
        Ok(Stats {
            hp: record.u8()?,
            attack: record.u8()?,
            defense: record.u8()?,
            speed: record.u8()?,
            special_attack: record.u8()?,
            special_defense: record.u8()?,
        })
    };
    let base_stats = stats()?;
    let types = [Type::from_raw(record.u8()?), Type::from_raw(record.u8()?)];
    let catch_rate = record.u8()?;
    let base_experience = record.u8()?;
    // Two bits per stat, in the same order as the base stats.
    let ev_bits = record.u16()?;
    let ev = |stat: u16| (ev_bits >> (stat * 2) & 0b11) as u8;
    let item = |raw: u16| (raw != 0).then_some(ItemId(raw));
    let held_items = HeldItems {
        common: item(record.u16()?),
        rare: item(record.u16()?),
    };
    let gender_ratio = GenderRatio::from_raw(record.u8()?);
    let hatch_cycles = record.u8()?;
    let base_friendship = record.u8()?;
    let growth_rate = GrowthRate::from_raw(record.u8()?);
    let egg_groups = [
        EggGroup::from_raw(record.u8()?),
        EggGroup::from_raw(record.u8()?),
    ];
    let ability = |raw: u8| (raw != 0).then_some(AbilityId(raw));
    let abilities = [ability(record.u8()?), ability(record.u8()?)];
    let safari_flee_rate = record.u8()?;
    let color_and_flip = record.u8()?;
    Ok(Species {
        id: SpeciesId(0),
        index: 0,
        name: String::new(),
        types,
        base_stats,
        ev_yield: Stats {
            hp: ev(0),
            attack: ev(1),
            defense: ev(2),
            speed: ev(3),
            special_attack: ev(4),
            special_defense: ev(5),
        },
        abilities,
        catch_rate,
        base_experience,
        base_friendship,
        growth_rate,
        gender_ratio,
        egg_groups,
        hatch_cycles,
        held_items,
        safari_flee_rate,
        color: Color::from_raw(color_and_flip & 0x7F),
        no_flip: color_and_flip & 0x80 != 0,
        machines: Vec::new(),
        evolutions: Vec::new(),
        level_up_moves: Vec::new(),
        egg_moves: Vec::new(),
    })
}

/// The learnset a pointer at `at` points to.
fn learnset<S: ByteSource>(game: &Gen3<S>, at: u32) -> Result<Vec<LevelUpMove>> {
    let offset = game.follow(at)?;
    let bytes = game.source.read_up_to(offset.into(), MAX_LEARNSET * 2)?;
    parse_learnset(&bytes)
        .ok_or_else(|| Error::malformed(FORMAT, offset.into(), "unterminated learnset"))
}

/// 16-bit entries, `level << 9 | move`, up to a `0xFFFF` terminator.
fn parse_learnset(bytes: &[u8]) -> Option<Vec<LevelUpMove>> {
    let mut moves = Vec::new();
    for entry in bytes
        .as_chunks()
        .0
        .iter()
        .map(|&pair| u16::from_le_bytes(pair))
    {
        if entry == LEARNSET_END {
            return Some(moves);
        }
        moves.push(LevelUpMove {
            level: (entry >> 9) as u8,
            r#move: MoveId(entry & 0x1FF),
        });
    }
    None
}

fn egg_moves<S: ByteSource>(game: &Gen3<S>) -> Result<BTreeMap<u16, Vec<MoveId>>> {
    let offset = game.edition.egg_moves;
    let bytes = game.source.read_up_to(offset.into(), MAX_EGG_MOVES * 2)?;
    parse_egg_moves(&bytes)
        .ok_or_else(|| Error::malformed(FORMAT, offset.into(), "malformed egg move list"))
}

/// Egg moves by internal species: 16-bit entries, each species' index plus 20000 followed by
/// its moves, up to a `0xFFFF` terminator.
fn parse_egg_moves(bytes: &[u8]) -> Option<BTreeMap<u16, Vec<MoveId>>> {
    let mut lists = BTreeMap::new();
    let mut current = None;
    for entry in bytes
        .as_chunks()
        .0
        .iter()
        .map(|&pair| u16::from_le_bytes(pair))
    {
        match entry {
            LEARNSET_END => return Some(lists),
            _ if entry > EGG_SPECIES => current = Some(entry - EGG_SPECIES),
            _ => lists
                .entry(current?)
                .or_insert_with(Vec::new)
                .push(MoveId(entry)),
        }
    }
    None
}

/// One ability per name, but the placeholder at index 0.
pub(super) fn abilities<S: ByteSource>(game: &Gen3<S>) -> Result<Vec<Ability>> {
    let edition = game.edition;
    let names = game.read(edition.ability_names, ABILITIES * ABILITY_NAME_LEN)?;
    let mut holders: BTreeMap<AbilityId, Vec<SpeciesId>> = BTreeMap::new();
    for species in game.species()? {
        for ability in species.abilities.iter().flatten() {
            let holders = holders.entry(*ability).or_default();
            if holders.last() != Some(&species.id) {
                holders.push(species.id);
            }
        }
    }
    (1..ABILITIES)
        .map(|index| {
            let id = AbilityId(index as u8);
            Ok(Ability {
                id,
                name: text::decode(&names[index * ABILITY_NAME_LEN..][..ABILITY_NAME_LEN]),
                description: Some(game.string_at(edition.ability_descriptions + 4 * index as u32)?),
                species: holders.remove(&id).unwrap_or_default(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_stats_read_like_gen4_personal_records() {
        let mut bytes = [0u8; BASE_STATS_LEN];
        bytes[..6].copy_from_slice(&[35, 55, 30, 90, 50, 40]);
        bytes[6..8].copy_from_slice(&[13, 13]);
        bytes[0x0A] = 0x80; // 2 speed EVs
        bytes[0x0C] = 0x92; // held 50% of the time
        bytes[0x16..0x18].copy_from_slice(&[9, 0]);
        bytes[0x19] = 0x82; // yellow, never flipped
        let pikachu = parse_base_stats(&bytes).unwrap();
        assert_eq!(pikachu.types, [Type::Electric; 2]);
        assert_eq!(
            (pikachu.base_stats.special_defense, pikachu.ev_yield.speed),
            (40, 2)
        );
        assert_eq!(pikachu.held_items.common, Some(ItemId(0x92)));
        assert_eq!(pikachu.abilities, [Some(AbilityId(9)), None]);
        assert_eq!((pikachu.color, pikachu.no_flip), (Color::Yellow, true));
        assert!(parse_base_stats(&bytes[..20]).is_err());
    }

    #[test]
    fn learnsets_and_egg_moves_end_at_their_terminator() {
        let words = |values: &[u16]| values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let learnset: Vec<u8> = words(&[1 << 9 | 33, 4 << 9 | 45, LEARNSET_END, 7]);
        let moves = parse_learnset(&learnset).unwrap();
        assert_eq!(moves.len(), 2);
        assert_eq!((moves[1].level, moves[1].r#move), (4, MoveId(45)));
        assert_eq!(parse_learnset(&learnset[..4]), None);

        let eggs: Vec<u8> = words(&[20001, 113, 130, 20004, 99, LEARNSET_END]);
        let lists = parse_egg_moves(&eggs).unwrap();
        assert_eq!(lists[&1], [MoveId(113), MoveId(130)]);
        assert_eq!(lists[&4], [MoveId(99)]);
        assert_eq!(parse_egg_moves(&eggs[..8]), None);
        assert_eq!(
            parse_egg_moves(&words(&[113, LEARNSET_END])),
            None,
            "a move before a species"
        );
    }
}
