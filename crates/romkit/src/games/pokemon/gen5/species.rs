//! Species: personal records joined with the evolution and level-up learnset tables, which
//! have one member per personal record. Abilities are derived from them: the games have no
//! ability records of their own.
//!
//! Personal records are 60 bytes in Black and White; Black 2 and White 2 append 16 bytes of
//! tutor compatibility. Records past the National Pokédex hold alternate forms with stats of
//! their own, and the archive ends with a table that is not a personal record.

use std::collections::BTreeMap;

use serde::Serialize;

use super::{AbilityId, ItemId, MoveId, SpeciesId, Type};
use super::{Color, EggGroup, GenderRatio, GrowthRate, LevelUpMove, Stats};
use crate::Result;
use crate::formats::narc::Narc;
use crate::io::Reader;

const PERSONAL_LEN: usize = 60;
const PERSONAL_B2W2_LEN: usize = 76;
const EVOLUTION_LEN: usize = 42;
const EVOLUTION_SLOTS: usize = 7;
const LEARNSET_END: u16 = 0xFFFF;
/// 95 TMs, then 6 HMs.
const MACHINES: u8 = 101;
/// What the type tutors teach, by bit: the starters' pledges, the final evolutions' ultimate
/// moves, and Draco Meteor.
const TYPE_TUTOR_MOVES: [MoveId; 7] = [
    MoveId(520), // Grass Pledge
    MoveId(519), // Fire Pledge
    MoveId(518), // Water Pledge
    MoveId(338), // Frenzy Plant
    MoveId(307), // Blast Burn
    MoveId(308), // Hydro Cannon
    MoveId(434), // Draco Meteor
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Species {
    pub id: SpeciesId,
    /// Alternate forms have no name of their own.
    pub name: Option<String>,
    /// For an alternate form's record: the species and form it holds.
    pub form_of: Option<Form>,
    /// Single-typed species repeat their type.
    pub types: [Type; 2],
    pub base_stats: Stats,
    pub ev_yield: Stats,
    pub abilities: [Option<AbilityId>; 2],
    /// The ability from the Dream World and Hidden Grottoes.
    pub hidden_ability: Option<AbilityId>,
    pub catch_rate: u8,
    /// 1 for species that do not evolve from another, 2 and 3 for their evolutions.
    pub evolution_stage: u8,
    pub base_experience: u16,
    pub base_friendship: u8,
    pub growth_rate: GrowthRate,
    pub gender_ratio: GenderRatio,
    pub egg_groups: [EggGroup; 2],
    pub hatch_cycles: u8,
    pub held_items: HeldItems,
    pub flee_rate: u8,
    /// How many forms the species has, counting the base form; 1 for most.
    pub form_count: u8,
    /// The records of forms 1 and up, when they have stats of their own.
    pub form_records: Vec<SpeciesId>,
    pub color: Color,
    pub flip_sprite: bool,
    /// In tenths of a meter.
    pub height: u16,
    /// In tenths of a kilogram.
    pub weight: u16,
    /// The TMs and HMs it can learn: 0–94 are TM01–TM95, 95–100 are HM01–HM06.
    pub machines: Vec<u8>,
    /// Moves from the tutors that teach by type (pledges, ultimate moves, Draco Meteor).
    pub tutor_moves: Vec<MoveId>,
    /// Black 2 and White 2's four move tutors, in the games' order; empty in Black and White.
    /// Each lists the positions, in that tutor's own move list, of the moves it can learn.
    pub tutors: Vec<Vec<u8>>,
    pub evolutions: Vec<Evolution>,
    /// Moves learned by leveling up, by ascending level.
    pub level_up_moves: Vec<LevelUpMove>,
}

/// An alternate form, by its species and its number among the species' forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Form {
    pub species: SpeciesId,
    pub form: u8,
}

/// Items wild Pokémon may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct HeldItems {
    /// Held 50% of the time.
    pub common: Option<ItemId>,
    /// Held 5% of the time.
    pub rare: Option<ItemId>,
    /// Held 1% of the time.
    pub very_rare: Option<ItemId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Evolution {
    #[serde(flatten)]
    pub method: EvolutionMethod,
    pub into: SpeciesId,
}

/// Every species and form but the placeholder at index 0, with evolutions, learnset, and name.
pub(super) fn parse(
    personal: &[u8],
    evolutions: &[u8],
    learnsets: &[u8],
    names: &[String],
) -> Result<Vec<Species>> {
    let personal_narc = Narc::parse(personal)?;
    let evolution_narc = Narc::parse(evolutions)?;
    let learnset_narc = Narc::parse(learnsets)?;
    let is_record = |index| {
        personal_narc.range(index).is_ok_and(|range| {
            [PERSONAL_LEN, PERSONAL_B2W2_LEN].contains(&((range.end - range.start) as usize))
        })
    };
    let count = match personal_narc.len() {
        len if len > 1 && !is_record(len - 1) => len - 1,
        len => len,
    };
    let mut species = (1..count)
        .map(|index| {
            let mut species = parse_personal(index, &personal_narc.member(personal, index)?)?;
            species.name = names.get(index).cloned();
            species.evolutions = parse_evolutions(&evolution_narc.member(evolutions, index)?)?;
            species.level_up_moves = parse_learnset(&learnset_narc.member(learnsets, index)?)?;
            Ok(species)
        })
        .collect::<Result<Vec<_>>>()?;
    link_forms(&mut species);
    Ok(species)
}

/// Point alternate forms' records back at their species. The forms of one species are
/// consecutive records, starting from the one its own record names. The name bank has no
/// names for forms: past the Pokédex it repeats "Egg".
fn link_forms(species: &mut [Species]) {
    let forms: Vec<(SpeciesId, Vec<SpeciesId>)> = species
        .iter()
        .filter(|species| !species.form_records.is_empty())
        .map(|species| (species.id, species.form_records.clone()))
        .collect();
    for (base, records) in forms {
        for (form, record) in (1..).zip(records) {
            if let Some(species) = species.get_mut(record.0 as usize - 1) {
                species.name = None;
                species.form_of = Some(Form {
                    species: base,
                    form,
                });
            }
        }
    }
}

fn parse_personal(index: usize, bytes: &[u8]) -> Result<Species> {
    let mut record = Reader::new("Gen 5 personal record", bytes);
    if !matches!(bytes.len(), PERSONAL_LEN | PERSONAL_B2W2_LEN) {
        return Err(record.error_at(0, "record is not 60 or 76 bytes"));
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
    let evolution_stage = record.u8()?;
    // Two bits per stat, in the same order as the base stats.
    let ev_bits = record.u16()?;
    let ev = |stat: u16| (ev_bits >> (stat * 2) & 0b11) as u8;
    let item = |raw: u16| (raw != 0).then_some(ItemId(raw));
    let held_items = HeldItems {
        common: item(record.u16()?),
        rare: item(record.u16()?),
        very_rare: item(record.u16()?),
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
    let hidden_ability = ability(record.u8()?);
    let flee_rate = record.u8()?;
    let first_form = record.u16()?;
    record.skip(2)?; // where the form's sprites start
    let form_count = record.u8()?;
    let color_and_flip = record.u8()?;
    let base_experience = record.u16()?;
    let height = record.u16()?;
    let weight = record.u16()?;
    let machine_bits = u128::from_le_bytes(record.array()?);
    let type_tutor_bits = record.u32()?;
    let tutors = match record.remaining() {
        0 => Vec::new(),
        _ => (0..4)
            .map(|_| Ok(bits(record.u32()?.into(), 32)))
            .collect::<Result<_>>()?,
    };
    Ok(Species {
        id: SpeciesId(index as u16),
        name: None,
        form_of: None,
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
        hidden_ability,
        catch_rate,
        evolution_stage,
        base_experience,
        base_friendship,
        growth_rate,
        gender_ratio,
        egg_groups,
        hatch_cycles,
        held_items,
        flee_rate,
        form_count,
        form_records: match first_form {
            0 => Vec::new(),
            first => (first..first.saturating_add(form_count.saturating_sub(1).into()))
                .map(SpeciesId)
                .collect(),
        },
        color: Color::from_raw(color_and_flip & 0x3F),
        flip_sprite: color_and_flip & 0x40 != 0,
        height,
        weight,
        machines: bits(machine_bits, MACHINES),
        tutor_moves: bits(type_tutor_bits.into(), 32)
            .into_iter()
            .filter_map(|bit| TYPE_TUTOR_MOVES.get(bit as usize).copied())
            .collect(),
        tutors,
        evolutions: Vec::new(),
        level_up_moves: Vec::new(),
    })
}

/// The positions of the bits set among the first `count`.
fn bits(value: u128, count: u8) -> Vec<u8> {
    (0..count).filter(|bit| value >> bit & 1 != 0).collect()
}

/// Seven `(method, parameter, species)` entries; method 0 marks an empty slot.
fn parse_evolutions(bytes: &[u8]) -> Result<Vec<Evolution>> {
    let mut record = Reader::new("Gen 5 evolution record", bytes);
    if bytes.len() != EVOLUTION_LEN {
        return Err(record.error_at(0, "record is not 42 bytes"));
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

/// `(move, level)` pairs of 16-bit values, up to a pair of `0xFFFF`.
fn parse_learnset(bytes: &[u8]) -> Result<Vec<LevelUpMove>> {
    let mut entries = Reader::new("Gen 5 learnset", bytes);
    let mut moves = Vec::new();
    loop {
        let (r#move, level) = (entries.u16()?, entries.u16()?);
        if r#move == LEARNSET_END {
            return Ok(moves);
        }
        moves.push(LevelUpMove {
            level: u8::try_from(level)
                .map_err(|_| entries.error_at(entries.position() - 2, "level exceeds 255"))?,
            r#move: MoveId(r#move),
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Ability {
    pub id: AbilityId,
    pub name: String,
    pub description: Option<String>,
    /// The species and forms that can have it as one of their two regular abilities.
    pub species: Vec<SpeciesId>,
    /// Those that can have it as their hidden ability.
    pub hidden: Vec<SpeciesId>,
}

/// One ability per name, but the placeholder at index 0.
pub(super) fn abilities(
    species: &[Species],
    names: &[String],
    descriptions: &[String],
) -> Vec<Ability> {
    type Holders = BTreeMap<AbilityId, Vec<SpeciesId>>;
    let (mut regular, mut hidden) = (Holders::new(), Holders::new());
    let hold = |holders: &mut Holders, ability, species| {
        let holders = holders.entry(ability).or_default();
        if holders.last() != Some(&species) {
            holders.push(species);
        }
    };
    for species in species {
        for ability in species.abilities.iter().flatten() {
            hold(&mut regular, *ability, species.id);
        }
        if let Some(ability) = species.hidden_ability {
            hold(&mut hidden, ability, species.id);
        }
    }
    (1..names.len().min(u8::MAX as usize + 1))
        .map(|index| {
            let id = AbilityId(index as u8);
            Ability {
                id,
                name: names[index].clone(),
                description: descriptions.get(index).cloned(),
                species: regular.remove(&id).unwrap_or_default(),
                hidden: hidden.remove(&id).unwrap_or_default(),
            }
        })
        .collect()
}

/// How an evolution is triggered, with the parameter the method uses. Gen 5 numbers its
/// methods differently from Gen 4: trading for a particular species took slot 7.
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
    /// Karrablast and Shelmet, traded for each other. The partner is not stored.
    TradeForSpecies,
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
    /// Level up in a magnetic field: Chargestone Cave, or Mt. Coronet.
    MagneticField,
    /// Level up near a Moss Rock.
    MossRock,
    /// Level up near an Ice Rock.
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
            7 => Self::TradeForSpecies,
            8 => Self::UseItem { item },
            9 => Self::LevelAttackGreater { level },
            10 => Self::LevelAttackEqual { level },
            11 => Self::LevelAttackLess { level },
            12 => Self::LevelPersonalityLow { level },
            13 => Self::LevelPersonalityHigh { level },
            14 => Self::LevelNinjask { level },
            15 => Self::LevelShedinja { level },
            16 => Self::Beauty { beauty: parameter },
            17 => Self::UseItemMale { item },
            18 => Self::UseItemFemale { item },
            19 => Self::HoldItemDay { item },
            20 => Self::HoldItemNight { item },
            21 => Self::KnowsMove {
                r#move: MoveId(parameter),
            },
            22 => Self::SpeciesInParty {
                species: SpeciesId(parameter),
            },
            23 => Self::LevelMale { level },
            24 => Self::LevelFemale { level },
            25 => Self::MagneticField,
            26 => Self::MossRock,
            27 => Self::IceRock,
            raw_method => Self::Unknown {
                raw_method,
                parameter,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::narc;

    /// A 60-byte personal record with the given stats, types, abilities, forms, and bits.
    fn personal(first_form: u16, form_count: u8, extra: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; PERSONAL_LEN];
        bytes[..6].copy_from_slice(&[35, 55, 30, 90, 50, 40]);
        bytes[6..8].copy_from_slice(&[12, 12]);
        bytes[0x0A..0x0C].copy_from_slice(&0x0080u16.to_le_bytes()); // 2 speed EVs
        bytes[0x10..0x12].copy_from_slice(&236u16.to_le_bytes());
        bytes[0x18..0x1B].copy_from_slice(&[9, 0, 31]);
        bytes[0x1C..0x1E].copy_from_slice(&first_form.to_le_bytes());
        bytes[0x20] = form_count;
        bytes[0x21] = 0x42; // yellow, flipped
        bytes[0x24..0x28].copy_from_slice(&[4, 0, 60, 0]);
        bytes[0x28 + 2] = 0x80; // TM24
        bytes[0x28 + 12] = 0x10; // HM01
        bytes[0x38] = 0b100_0001; // Grass Pledge, Draco Meteor
        [bytes, extra.to_vec()].concat()
    }

    #[test]
    fn parses_records_and_links_forms() {
        let evolution = [
            &8u16.to_le_bytes()[..],
            &83u16.to_le_bytes(),
            &2u16.to_le_bytes(),
        ]
        .concat();
        let evolution = [evolution, vec![0; 36]].concat();
        let learnset = [84u16, 1, 0xFFFF, 0xFFFF].map(u16::to_le_bytes).concat();
        let tutors = [0x1012u32, 0, 0, 1 << 31].map(u32::to_le_bytes).concat();
        let records = [
            vec![0; 56],
            personal(2, 2, &tutors),
            personal(0, 2, &tutors),
            b"table".to_vec(),
        ];
        let personal = narc(&records.iter().map(Vec::as_slice).collect::<Vec<_>>());
        let evolutions = narc(&[&[0; 42], &evolution, &[0; 42]]);
        let learnsets = narc(&[&learnset, &learnset, &learnset]);
        let names = ["", "Pikachu", "Egg"].map(str::to_owned);
        let species = parse(&personal, &evolutions, &learnsets, &names).unwrap();

        assert_eq!(species.len(), 2, "the trailing table is not a species");
        let [pikachu, form] = &species[..] else {
            unreachable!()
        };
        assert_eq!(pikachu.name.as_deref(), Some("Pikachu"));
        assert_eq!(pikachu.types, [Type::Electric; 2]);
        assert_eq!((pikachu.ev_yield.speed, pikachu.base_stats.speed), (2, 90));
        assert_eq!(pikachu.held_items.very_rare, Some(ItemId(236)));
        assert_eq!(pikachu.abilities, [Some(AbilityId(9)), None]);
        assert_eq!(pikachu.hidden_ability, Some(AbilityId(31)));
        assert_eq!((pikachu.color, pikachu.flip_sprite), (Color::Yellow, true));
        assert_eq!((pikachu.height, pikachu.weight), (4, 60));
        assert_eq!(pikachu.machines, [23, 100]);
        assert_eq!(pikachu.tutor_moves, [MoveId(520), MoveId(434)]);
        assert_eq!(pikachu.tutors, [vec![1, 4, 12], vec![], vec![], vec![31]]);
        assert_eq!(
            pikachu.evolutions,
            [Evolution {
                method: EvolutionMethod::UseItem { item: ItemId(83) },
                into: SpeciesId(2)
            }]
        );
        assert_eq!(pikachu.level_up_moves.len(), 1);
        assert_eq!(pikachu.form_records, [SpeciesId(2)]);
        assert_eq!(
            form.form_of,
            Some(Form {
                species: SpeciesId(1),
                form: 1
            })
        );
        assert_eq!(form.name, None);

        let abilities = abilities(&species, &vec!["Static".to_owned(); 40], &[]);
        assert_eq!(abilities[8].species, [SpeciesId(1), SpeciesId(2)]);
        assert_eq!(abilities[30].hidden, [SpeciesId(1), SpeciesId(2)]);
    }

    #[test]
    fn rejects_malformed_records() {
        assert!(parse_personal(1, &[0; 59]).is_err());
        assert!(parse_evolutions(&[0; 40]).is_err());
        let unterminated = [1u16, 1].map(u16::to_le_bytes).concat();
        assert!(parse_learnset(&unterminated).is_err());
        let high_level = [1u16, 300, 0xFFFF, 0xFFFF].map(u16::to_le_bytes).concat();
        assert!(parse_learnset(&high_level).is_err());
    }

    #[test]
    fn trading_for_a_species_takes_method_7() {
        let evolution = Evolution {
            method: EvolutionMethod::decode(7, 0),
            into: SpeciesId(589),
        };
        assert_eq!(
            serde_json::to_value(evolution).unwrap(),
            serde_json::json!({"method": "trade_for_species", "into": 589})
        );
        assert_eq!(
            EvolutionMethod::decode(8, 82),
            EvolutionMethod::UseItem { item: ItemId(82) }
        );
    }
}
