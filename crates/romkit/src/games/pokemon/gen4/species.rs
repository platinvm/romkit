//! Species: personal records (44 bytes each) joined with the evolution and level-up learnset
//! tables, which have one member per personal record. Abilities are derived from them: the
//! games have no ability records of their own.

use std::collections::BTreeMap;

use serde::Serialize;

use super::{AbilityId, ItemId, MoveId, SpeciesId, Type};
use crate::Result;
use crate::formats::narc::Narc;
use crate::io::Reader;

const PERSONAL_LEN: usize = 44;
const EVOLUTION_LEN: usize = 44;
const EVOLUTION_SLOTS: usize = 7;
const LEARNSET_END: u16 = 0xFFFF;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Species {
    pub id: SpeciesId,
    /// Alternate forms have no name of their own.
    pub name: Option<String>,
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
    pub flip_sprite: bool,
    /// The TMs and HMs it can learn: 0–91 are TM01–TM92, 92–99 are HM01–HM08.
    pub machines: Vec<u8>,
    pub evolutions: Vec<Evolution>,
    /// Moves learned by leveling up, by ascending level.
    pub level_up_moves: Vec<LevelUpMove>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Stats {
    pub hp: u8,
    pub attack: u8,
    pub defense: u8,
    pub speed: u8,
    pub special_attack: u8,
    pub special_defense: u8,
}

/// Items wild Pokémon may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct HeldItems {
    /// Held 50% of the time.
    pub common: Option<ItemId>,
    /// Held 5% of the time.
    pub rare: Option<ItemId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LevelUpMove {
    pub level: u8,
    pub r#move: MoveId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Evolution {
    #[serde(flatten)]
    pub method: EvolutionMethod,
    pub into: SpeciesId,
}

/// Every species but the placeholder at index 0, with its evolutions, learnset, and name.
pub(super) fn parse(
    personal: &[u8],
    evolutions: &[u8],
    learnsets: &[u8],
    names: &[String],
) -> Result<Vec<Species>> {
    let personal_narc = Narc::parse(personal)?;
    let evolution_narc = Narc::parse(evolutions)?;
    let learnset_narc = Narc::parse(learnsets)?;
    (1..personal_narc.len())
        .map(|index| {
            let mut species = parse_personal(index, &personal_narc.member(personal, index)?)?;
            species.name = names.get(index).cloned();
            species.evolutions = parse_evolutions(&evolution_narc.member(evolutions, index)?)?;
            species.level_up_moves = parse_learnset(&learnset_narc.member(learnsets, index)?)?;
            Ok(species)
        })
        .collect()
}

fn parse_personal(index: usize, bytes: &[u8]) -> Result<Species> {
    let mut record = Reader::new("Gen 4 personal record", bytes);
    if bytes.len() != PERSONAL_LEN {
        return Err(record.error_at(0, "record is not 44 bytes"));
    }
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
    record.skip(2)?;
    let machine_bits = u128::from_le_bytes(record.array()?);
    Ok(Species {
        id: SpeciesId(index as u16),
        name: None,
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
        flip_sprite: color_and_flip & 0x80 != 0,
        machines: (0..128)
            .filter(|bit| machine_bits >> bit & 1 != 0)
            .collect(),
        evolutions: Vec::new(),
        level_up_moves: Vec::new(),
    })
}

/// Up to seven `(method, parameter, species)` entries; method 0 marks an empty slot.
fn parse_evolutions(bytes: &[u8]) -> Result<Vec<Evolution>> {
    let mut record = Reader::new("Gen 4 evolution record", bytes);
    if bytes.len() != EVOLUTION_LEN {
        return Err(record.error_at(0, "record is not 44 bytes"));
    }
    let mut evolutions = Vec::new();
    for _ in 0..EVOLUTION_SLOTS {
        let (method, parameter, into) = (record.u16()?, record.u16()?, record.u16()?);
        if method != 0 {
            evolutions.push(Evolution {
                method: EvolutionMethod::decode(method, parameter),
                into: SpeciesId(into),
            });
        }
    }
    Ok(evolutions)
}

/// 16-bit entries, `level << 9 | move`, up to a `0xFFFF` terminator.
fn parse_learnset(bytes: &[u8]) -> Result<Vec<LevelUpMove>> {
    let mut entries = Reader::new("Gen 4 learnset", bytes);
    let mut moves = Vec::new();
    loop {
        match entries.u16()? {
            LEARNSET_END => return Ok(moves),
            entry => moves.push(LevelUpMove {
                level: (entry >> 9) as u8,
                r#move: MoveId(entry & 0x1FF),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Ability {
    pub id: AbilityId,
    pub name: String,
    pub description: Option<String>,
    /// The species and forms that can have it.
    pub species: Vec<SpeciesId>,
}

/// One ability per name, but the placeholder at index 0.
pub(super) fn abilities(
    species: &[Species],
    names: &[String],
    descriptions: &[String],
) -> Vec<Ability> {
    let mut holders: BTreeMap<AbilityId, Vec<SpeciesId>> = BTreeMap::new();
    for species in species {
        for ability in species.abilities.iter().flatten() {
            let holders = holders.entry(*ability).or_default();
            if holders.last() != Some(&species.id) {
                holders.push(species.id);
            }
        }
    }
    (1..names.len().min(u8::MAX as usize + 1))
        .map(|index| {
            let id = AbilityId(index as u8);
            Ability {
                id,
                name: names[index].clone(),
                description: descriptions.get(index).cloned(),
                species: holders.remove(&id).unwrap_or_default(),
            }
        })
        .collect()
}

/// How an evolution is triggered, with the parameter the method uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum EvolutionMethod {
    /// Level up with high friendship.
    Friendship,
    FriendshipDay,
    FriendshipNight,
    Level {
        level: u16,
    },
    Trade,
    TradeHolding {
        item: ItemId,
    },
    UseItem {
        item: ItemId,
    },
    /// Level up with Attack greater than Defense.
    LevelAttackGreater {
        level: u16,
    },
    LevelAttackEqual {
        level: u16,
    },
    LevelAttackLess {
        level: u16,
    },
    /// Wurmple into Silcoon, decided by its personality value.
    LevelPersonalityLow {
        level: u16,
    },
    /// Wurmple into Cascoon.
    LevelPersonalityHigh {
        level: u16,
    },
    /// Nincada into Ninjask.
    LevelNinjask {
        level: u16,
    },
    /// Nincada into Shedinja, given a free party slot.
    LevelShedinja {
        level: u16,
    },
    Beauty {
        beauty: u16,
    },
    UseItemMale {
        item: ItemId,
    },
    UseItemFemale {
        item: ItemId,
    },
    HoldItemDay {
        item: ItemId,
    },
    HoldItemNight {
        item: ItemId,
    },
    KnowsMove {
        r#move: MoveId,
    },
    SpeciesInParty {
        species: SpeciesId,
    },
    LevelMale {
        level: u16,
    },
    LevelFemale {
        level: u16,
    },
    /// Level up at Mt. Coronet.
    MagneticField,
    /// Level up near the Moss Rock.
    MossRock,
    /// Level up near the Ice Rock.
    IceRock,
    /// A method without a known meaning, kept as stored.
    Unknown {
        raw_method: u16,
        parameter: u16,
    },
}

impl EvolutionMethod {
    pub fn decode(method: u16, parameter: u16) -> Self {
        let (level, item) = (parameter, ItemId(parameter));
        match method {
            1 => Self::Friendship,
            2 => Self::FriendshipDay,
            3 => Self::FriendshipNight,
            4 => Self::Level { level },
            5 => Self::Trade,
            6 => Self::TradeHolding { item },
            7 => Self::UseItem { item },
            8 => Self::LevelAttackGreater { level },
            9 => Self::LevelAttackEqual { level },
            10 => Self::LevelAttackLess { level },
            11 => Self::LevelPersonalityLow { level },
            12 => Self::LevelPersonalityHigh { level },
            13 => Self::LevelNinjask { level },
            14 => Self::LevelShedinja { level },
            15 => Self::Beauty { beauty: parameter },
            16 => Self::UseItemMale { item },
            17 => Self::UseItemFemale { item },
            18 => Self::HoldItemDay { item },
            19 => Self::HoldItemNight { item },
            20 => Self::KnowsMove {
                r#move: MoveId(parameter),
            },
            21 => Self::SpeciesInParty {
                species: SpeciesId(parameter),
            },
            22 => Self::LevelMale { level },
            23 => Self::LevelFemale { level },
            24 => Self::MagneticField,
            25 => Self::MossRock,
            26 => Self::IceRock,
            raw_method => Self::Unknown {
                raw_method,
                parameter,
            },
        }
    }
}

/// Which genders a species comes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GenderRatio {
    MaleOnly,
    FemaleOnly,
    Genderless,
    /// Female when the low byte of the personality value is below the threshold: 31 is 12.5%
    /// female, 127 is 50%, 254 would be all.
    Mixed {
        female_threshold: u8,
    },
}

impl GenderRatio {
    pub const fn from_raw(raw: u8) -> Self {
        match raw {
            0 => Self::MaleOnly,
            254 => Self::FemaleOnly,
            255 => Self::Genderless,
            female_threshold => Self::Mixed { female_threshold },
        }
    }
}

raw_enum! {
    pub enum GrowthRate: u8 {
        MediumFast = 0,
        Erratic = 1,
        Fluctuating = 2,
        MediumSlow = 3,
        Fast = 4,
        Slow = 5,
    }
}

raw_enum! {
    pub enum EggGroup: u8 {
        Monster = 1,
        Water1 = 2,
        Bug = 3,
        Flying = 4,
        Field = 5,
        Fairy = 6,
        Grass = 7,
        HumanLike = 8,
        Water3 = 9,
        Mineral = 10,
        Amorphous = 11,
        Water2 = 12,
        Ditto = 13,
        Dragon = 14,
        Undiscovered = 15,
    }
}

raw_enum! {
    /// The Pokédex search color.
    pub enum Color: u8 {
        Red = 0,
        Blue = 1,
        Yellow = 2,
        Green = 3,
        Black = 4,
        Brown = 5,
        Purple = 6,
        Gray = 7,
        White = 8,
        Pink = 9,
        /// The Egg and the Bad Egg.
        Egg = 10,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learnsets_unpack_level_and_move_up_to_the_terminator() {
        let bytes = [(1u16 << 9 | 33), (3 << 9 | 45), LEARNSET_END, 0]
            .map(u16::to_le_bytes)
            .concat();
        let moves = parse_learnset(&bytes).unwrap();
        assert_eq!(
            moves[1],
            LevelUpMove {
                level: 3,
                r#move: MoveId(45)
            }
        );
        assert_eq!(moves.len(), 2);
        assert!(parse_learnset(&bytes[..4]).is_err());
    }

    #[test]
    fn evolutions_serialize_flat_with_their_parameter() {
        let evolution = Evolution {
            method: EvolutionMethod::decode(7, 84),
            into: SpeciesId(134),
        };
        assert_eq!(
            serde_json::to_value(evolution).unwrap(),
            serde_json::json!({"method": "use_item", "item": 84, "into": 134})
        );
        assert_eq!(
            EvolutionMethod::decode(99, 1),
            EvolutionMethod::Unknown {
                raw_method: 99,
                parameter: 1
            }
        );
    }
}
