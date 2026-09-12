//! Species: personal records joined with the evolution and level-up learnset tables, which have
//! one entry per personal record. The personal archive's last entry repeats every record back
//! to back. Abilities are derived from the species, as in Generation IV.
//!
//! Personal records share their first 0x38 bytes across the games:
//!
//! ```text
//! 0x00  base stats (6 bytes), types (2), catch rate, evolution stage, EV yield (u16)
//! 0x0C  held items (3 × u16), gender ratio, hatch cycles, friendship, growth rate
//! 0x16  egg groups (2), abilities (3), escape rate
//! 0x1C  first alternate form's record (u16), form sprite (u16), form count, color
//! 0x22  base experience, height in cm, weight in hg (u16 each)
//! 0x28  TM and HM bits (16 bytes), type tutor bits (4 bytes)
//! ```
//!
//! Omega Ruby and Alpha Sapphire then add move tutor bits (16 bytes at 0x40), Ultra Sun and Ultra
//! Moon add theirs at 0x3C, and Generation VII ends with a species' Z-Crystal (0x4C) and whether
//! it is a regional form (0x52).

use std::collections::BTreeMap;

use serde::Serialize;

use super::super::gen4::{Ability, Color, EggGroup, GenderRatio, GrowthRate, LevelUpMove, Stats};
use super::{AbilityId, Edition, ItemId, MoveId, SpeciesId, Type};
use crate::Result;
use crate::formats::garc::Garc;
use crate::io::Reader;

const EVOLUTION_SLOTS: usize = 8;
const LEARNSET_END: u32 = 0xFFFF_FFFF;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Species {
    pub id: SpeciesId,
    /// Alternate forms have no name of their own.
    pub name: Option<String>,
    /// For an alternate form with its own record, which species and form it is.
    pub form_of: Option<Form>,
    /// Single-typed species repeat their type.
    pub types: [Type; 2],
    pub base_stats: Stats,
    pub ev_yield: Stats,
    /// The first, second, and hidden ability.
    pub abilities: [Option<AbilityId>; 3],
    pub catch_rate: u8,
    /// 1 for unevolved species, 2 or 3 for their evolutions.
    pub evolution_stage: u8,
    pub base_experience: u16,
    pub base_friendship: u8,
    pub growth_rate: GrowthRate,
    pub gender_ratio: GenderRatio,
    pub egg_groups: [EggGroup; 2],
    pub hatch_cycles: u8,
    pub held_items: HeldItems,
    pub escape_rate: u8,
    /// How many forms the species has, counting the base form.
    pub form_count: u8,
    pub color: Color,
    pub height_cm: u16,
    /// In hectograms (tenths of a kilogram).
    pub weight_hg: u16,
    /// The TMs and HMs it can learn: TM01–TM100 are 0–99, then HM01 onwards.
    pub machines: Vec<u8>,
    /// The type-specific moves of the move tutors (the Pledges, Draco Meteor, ...), by bit.
    pub type_tutors: Vec<u8>,
    /// The other move tutors' moves, by bit (Omega Ruby, Alpha Sapphire, Ultra Sun, and Ultra
    /// Moon).
    pub move_tutors: Vec<u8>,
    /// The Z-Crystal that turns one of its moves into an exclusive Z-Move (Generation VII).
    pub z_crystal: Option<ZCrystal>,
    /// A regional form, such as the Alolan ones (Generation VII).
    pub regional_form: bool,
    pub evolutions: Vec<Evolution>,
    /// Moves learned by leveling up, by ascending level; level 0 is on evolving.
    pub level_up_moves: Vec<LevelUpMove>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Form {
    pub species: SpeciesId,
    /// 1 for the first alternate form.
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
pub struct ZCrystal {
    pub item: ItemId,
    /// The move it powers up, and what it becomes.
    pub r#move: MoveId,
    pub z_move: MoveId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Evolution {
    pub method: EvolutionMethod,
    /// What the method depends on, as stored: an item, a move, a species, a type, a version,
    /// a beauty value, or (in Generation VI) the level.
    pub parameter: u16,
    pub into: SpeciesId,
    /// The form it evolves into, -1 for its own (Generation VII).
    pub form: Option<i8>,
    /// The level it needs, 0 for none (Generation VII).
    pub level: Option<u8>,
}

/// Every species but the placeholder at index 0, with its evolutions, learnset, and name.
pub(super) fn parse(
    edition: &Edition,
    personal: &[u8],
    evolutions: &[u8],
    learnsets: &[u8],
    names: &[String],
) -> Result<Vec<Species>> {
    let personal_garc = Garc::parse(personal)?;
    let evolution_garc = Garc::parse(evolutions)?;
    let learnset_garc = Garc::parse(learnsets)?;
    // The last personal entry repeats all the others.
    let count = personal_garc.len().saturating_sub(1);
    let mut form_records = Vec::new();
    let mut species = (1..count)
        .map(|index| {
            let record = personal_garc.member(personal, index)?;
            let (mut species, form_record) = parse_personal(edition, index, &record)?;
            form_records.push(form_record);
            species.name = names.get(index).filter(|name| !name.is_empty()).cloned();
            let evolutions = evolution_garc.member(evolutions, index)?;
            species.evolutions = parse_evolutions(edition.generation, &evolutions)?;
            species.level_up_moves = parse_learnset(&learnset_garc.member(learnsets, index)?)?;
            Ok(species)
        })
        .collect::<Result<Vec<_>>>()?;

    // A species with alternate forms of their own points to the first form's record. The form
    // records, which follow all the species, point back to themselves, so they are skipped.
    let records = species.len();
    for (index, first) in form_records.into_iter().enumerate() {
        if species[index].form_of.is_some() || first == 0 {
            continue;
        }
        for form in 1..species[index].form_count {
            let record = first as usize + form as usize - 1;
            if record > index + 1 && record <= records {
                let id = species[index].id;
                let form_record = &mut species[record - 1];
                form_record.form_of = Some(Form { species: id, form });
                form_record.name = None;
            }
        }
    }
    Ok(species)
}

/// A species, and the record of its first alternate form (0 for none).
fn parse_personal(edition: &Edition, index: usize, bytes: &[u8]) -> Result<(Species, u16)> {
    let mut record = Reader::new("Gen 6/7 personal record", bytes);
    if bytes.len() != edition.personal_len {
        return Err(record.error_at(0, "record has the wrong length"));
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
    // Two bits per stat, in the same order as the base stats.
    let ev_bits = record.u16_at(0x0A)?;
    let ev = |stat: u16| (ev_bits >> (stat * 2) & 0b11) as u8;
    let item = |at: usize| -> Result<Option<ItemId>> {
        Ok(Some(record.u16_at(at)?).filter(|&raw| raw != 0).map(ItemId))
    };
    let ability = |at: usize| -> Result<Option<AbilityId>> {
        Ok(Some(record.u8_at(at)?)
            .filter(|&raw| raw != 0)
            .map(AbilityId))
    };
    let bits = |at: usize, len: usize| -> Result<Vec<u8>> {
        let bytes = record.slice_at(at, len)?;
        Ok((0..len * 8)
            .filter(|&bit| bytes[bit / 8] >> (bit % 8) & 1 != 0)
            .map(|bit| bit as u8)
            .collect())
    };
    let tutors_at = match (edition.generation, edition.personal_len) {
        (6, 0x50) => Some(0x40),
        (7, _) => Some(0x3C),
        _ => None,
    };
    let generation7 = edition.generation == 7;
    let z_crystal = match generation7 {
        true => item(0x4C)?.map(|item| -> Result<ZCrystal> {
            Ok(ZCrystal {
                item,
                r#move: MoveId(record.u16_at(0x4E)?),
                z_move: MoveId(record.u16_at(0x50)?),
            })
        }),
        false => None,
    }
    .transpose()?;
    let species = Species {
        id: SpeciesId(index as u16),
        name: None,
        form_of: None,
        types: [
            Type::from_raw(record.u8_at(6)?),
            Type::from_raw(record.u8_at(7)?),
        ],
        base_stats,
        ev_yield: Stats {
            hp: ev(0),
            attack: ev(1),
            defense: ev(2),
            speed: ev(3),
            special_attack: ev(4),
            special_defense: ev(5),
        },
        abilities: [ability(0x18)?, ability(0x19)?, ability(0x1A)?],
        catch_rate: record.u8_at(0x08)?,
        evolution_stage: record.u8_at(0x09)?,
        base_experience: record.u16_at(0x22)?,
        base_friendship: record.u8_at(0x14)?,
        growth_rate: GrowthRate::from_raw(record.u8_at(0x15)?),
        gender_ratio: GenderRatio::from_raw(record.u8_at(0x12)?),
        egg_groups: [
            EggGroup::from_raw(record.u8_at(0x16)?),
            EggGroup::from_raw(record.u8_at(0x17)?),
        ],
        hatch_cycles: record.u8_at(0x13)?,
        held_items: HeldItems {
            common: item(0x0C)?,
            rare: item(0x0E)?,
            very_rare: item(0x10)?,
        },
        escape_rate: record.u8_at(0x1B)?,
        form_count: record.u8_at(0x20)?,
        color: Color::from_raw(record.u8_at(0x21)? & 0x3F),
        height_cm: record.u16_at(0x24)?,
        weight_hg: record.u16_at(0x26)?,
        machines: bits(0x28, 16)?,
        type_tutors: bits(0x38, 4)?,
        move_tutors: tutors_at.map_or(Ok(Vec::new()), |at| bits(at, 16))?,
        z_crystal,
        regional_form: generation7 && record.u8_at(0x52)? == 1,
        evolutions: Vec::new(),
        level_up_moves: Vec::new(),
    };
    Ok((species, record.u16_at(0x1C)?))
}

/// Eight `(method, parameter, species)` slots, with a form and a level in Generation VII;
/// method 0 marks an empty slot.
fn parse_evolutions(generation: u8, bytes: &[u8]) -> Result<Vec<Evolution>> {
    let slot_len = if generation == 7 { 8 } else { 6 };
    let record = Reader::new("Gen 6/7 evolution record", bytes);
    if bytes.len() != EVOLUTION_SLOTS * slot_len {
        return Err(record.error_at(0, "record has the wrong length"));
    }
    let mut evolutions = Vec::new();
    for at in (0..bytes.len()).step_by(slot_len) {
        let method = record.u16_at(at)?;
        if method == 0 {
            continue;
        }
        let extra = |offset: usize| -> Result<Option<u8>> {
            (generation == 7)
                .then(|| record.u8_at(at + offset))
                .transpose()
        };
        evolutions.push(Evolution {
            method: EvolutionMethod::from_raw(method),
            parameter: record.u16_at(at + 2)?,
            into: SpeciesId(record.u16_at(at + 4)?),
            form: extra(6)?.map(|form| form as i8),
            level: extra(7)?,
        });
    }
    Ok(evolutions)
}

/// `(move, level)` pairs of u16, up to a `0xFFFFFFFF` terminator.
fn parse_learnset(bytes: &[u8]) -> Result<Vec<LevelUpMove>> {
    let mut entries = Reader::new("Gen 6/7 learnset", bytes);
    let mut moves = Vec::new();
    loop {
        let entry = entries.u32()?;
        if entry == LEARNSET_END {
            return Ok(moves);
        }
        moves.push(LevelUpMove {
            level: (entry >> 16) as u8,
            r#move: MoveId(entry as u16),
        });
    }
}

/// One ability per name but the placeholder at index 0, with the species that can have it.
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

raw_enum! {
    /// How an evolution is triggered; [`Evolution::parameter`] holds what it depends on.
    pub enum EvolutionMethod: u16 {
        /// Level up with high friendship.
        Friendship = 1,
        FriendshipDay = 2,
        FriendshipNight = 3,
        Level = 4,
        Trade = 5,
        /// Trade holding the item in the parameter.
        TradeHolding = 6,
        /// Karrablast and Shelmet, traded for each other.
        TradeKarrablastShelmet = 7,
        UseItem = 8,
        /// Level up with Attack greater than Defense (Tyrogue).
        LevelAttackGreater = 9,
        LevelAttackEqual = 10,
        LevelAttackLess = 11,
        /// Wurmple into Silcoon, decided by its personality value.
        LevelPersonalityLow = 12,
        /// Wurmple into Cascoon.
        LevelPersonalityHigh = 13,
        /// Nincada into Ninjask.
        LevelNinjask = 14,
        /// Nincada into Shedinja, given a free party slot.
        LevelShedinja = 15,
        /// Level up with the beauty in the parameter (Feebas).
        Beauty = 16,
        UseItemMale = 17,
        UseItemFemale = 18,
        HoldItemDay = 19,
        HoldItemNight = 20,
        /// Level up knowing the move in the parameter.
        KnowsMove = 21,
        /// Level up with the species in the parameter in the party.
        SpeciesInParty = 22,
        LevelMale = 23,
        LevelFemale = 24,
        /// Level up in a special magnetic field.
        MagneticField = 25,
        /// Level up near a Moss Rock.
        MossRock = 26,
        /// Level up near an Ice Rock.
        IceRock = 27,
        /// Level up holding the 3DS upside down (Inkay).
        LevelUpsideDown = 28,
        /// Level up with high affection knowing a move of the type in the parameter (Sylveon).
        AffectionKnowsMoveType = 29,
        /// Level up with a Dark-type Pokémon in the party (Pancham).
        LevelDarkTypeInParty = 30,
        /// Level up in the rain (Sliggoo).
        LevelRain = 31,
        LevelDay = 32,
        LevelNight = 33,
        /// Level up as a female, into form 1 (Espurr into the female Meowstic).
        LevelFemaleForm1 = 34,
        /// Level up in the version in the parameter (Cosmoem).
        LevelVersion = 36,
        LevelVersionDay = 37,
        LevelVersionNight = 38,
        /// Level up at the summit of Mount Lanakila (Crabrawler).
        LevelMountLanakila = 39,
        /// Level up at dusk (Rockruff into Dusk Form Lycanroc).
        LevelDusk = 40,
        /// Level up in Ultra Space.
        LevelUltraSpace = 41,
        /// Use an item in Ultra Space.
        UseItemUltraSpace = 42,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learnsets_pair_moves_and_levels_up_to_the_terminator() {
        let bytes = [[33u16, 1], [45, 5], [0xFFFF, 0xFFFF]]
            .concat()
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        let moves = parse_learnset(&bytes).unwrap();
        assert_eq!(
            moves,
            [
                LevelUpMove {
                    level: 1,
                    r#move: MoveId(33)
                },
                LevelUpMove {
                    level: 5,
                    r#move: MoveId(45)
                }
            ]
        );
        assert!(parse_learnset(&bytes[..8]).is_err());
    }

    #[test]
    fn evolutions_have_a_form_and_level_in_generation_7() {
        let mut slots = vec![0; 64];
        slots[..8].copy_from_slice(&[8, 0, 83, 0, 26, 0, 1, 0]); // Thunder Stone into form 1
        let evolution = parse_evolutions(7, &slots).unwrap()[0];
        assert_eq!(evolution.method, EvolutionMethod::UseItem);
        assert_eq!((evolution.parameter, evolution.into), (83, SpeciesId(26)));
        assert_eq!((evolution.form, evolution.level), (Some(1), Some(0)));
        assert_eq!(
            serde_json::to_value(evolution).unwrap(),
            serde_json::json!({"method": "use_item", "parameter": 83, "into": 26, "form": 1, "level": 0})
        );
        assert!(parse_evolutions(6, &slots).is_err());
        assert_eq!(parse_evolutions(6, &slots[..48]).unwrap()[0].form, None);
    }
}
