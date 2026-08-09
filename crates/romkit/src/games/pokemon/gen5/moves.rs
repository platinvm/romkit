//! Moves: one 36-byte record per move in the move table. Gen 5 records spell out what Gen 4
//! left to effect scripts: status chances, stat changes, multi-hit counts, recoil, and healing.

use serde::Serialize;

use super::{MoveId, Type};
use crate::Result;
use crate::formats::narc::Narc;
use crate::io::Reader;

const RECORD_LEN: usize = 36;
/// The accuracy of moves that never miss.
const NEVER_MISSES: u8 = 101;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Move {
    pub id: MoveId,
    pub name: Option<String>,
    pub description: Option<String>,
    pub r#type: Type,
    pub class: MoveClass,
    /// What the move does, broadly.
    pub kind: MoveKind,
    pub power: u8,
    /// In percent; `None` for moves that never miss.
    pub accuracy: Option<u8>,
    pub pp: u8,
    pub priority: i8,
    pub target: MoveTarget,
    pub flags: MoveFlags,
    /// For moves that hit several times in one turn.
    pub hits: Option<Span>,
    pub ailment: Ailment,
    /// The chance of inflicting the ailment, in percent; 0 when it always does.
    pub ailment_chance: u8,
    /// How many turns the ailment lasts, where the move sets it.
    pub ailment_turns: Option<Span>,
    /// The critical hit stage it adds; 6 always lands one.
    pub critical_stage: u8,
    /// The chance of making the target flinch, in percent.
    pub flinch_chance: u8,
    /// The share of the damage dealt that the user regains, in percent; negative for recoil.
    pub drain: i8,
    /// The share of the user's maximum HP that it regains, in percent; negative for a loss.
    pub healing: i8,
    pub stat_changes: Vec<StatChange>,
    /// The battle effect script.
    pub effect: u16,
}

/// An inclusive range of counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Span {
    pub min: u8,
    pub max: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StatChange {
    pub stat: BattleStat,
    pub stages: i8,
    /// In percent; 0 when it always happens.
    pub chance: u8,
}

/// Every move but the placeholder at index 0.
pub(super) fn parse(table: &[u8], names: &[String], descriptions: &[String]) -> Result<Vec<Move>> {
    let narc = Narc::parse(table)?;
    (1..narc.len())
        .map(|index| {
            let bytes = narc.member(table, index)?;
            let mut record = Reader::new("Gen 5 move", &bytes);
            if bytes.len() != RECORD_LEN {
                return Err(record.error_at(0, "record is not 36 bytes"));
            }
            let span = |min: u8, max: u8| (min | max != 0).then_some(Span { min, max });
            let r#type = Type::from_raw(record.u8()?);
            let kind = MoveKind::from_raw(record.u8()?);
            let class = MoveClass::from_raw(record.u8()?);
            let power = record.u8()?;
            let accuracy = Some(record.u8()?).filter(|&accuracy| accuracy != NEVER_MISSES);
            let pp = record.u8()?;
            let priority = record.i8()?;
            let hits = record.u8()?;
            let ailment = Ailment::from_raw(record.u16()?);
            let ailment_chance = record.u8()?;
            record.skip(1)?;
            let ailment_turns = span(record.u8()?, record.u8()?);
            let critical_stage = record.u8()?;
            let flinch_chance = record.u8()?;
            let effect = record.u16()?;
            let drain = record.i8()?;
            let healing = record.i8()?;
            let target = MoveTarget::from_raw(record.u8()?);
            let stats = record.array::<3>()?;
            let stages = record.array::<3>()?;
            let chances = record.array::<3>()?;
            record.skip(2)?;
            Ok(Move {
                id: MoveId(index as u16),
                name: names.get(index).cloned(),
                description: descriptions.get(index).cloned(),
                r#type,
                class,
                kind,
                power,
                accuracy,
                pp,
                priority,
                target,
                flags: MoveFlags(record.u32()?),
                hits: span(hits & 0xF, hits >> 4),
                ailment,
                ailment_chance,
                ailment_turns,
                critical_stage,
                flinch_chance,
                drain,
                healing,
                stat_changes: (0..3)
                    .filter(|&slot| stats[slot] != 0)
                    .map(|slot| StatChange {
                        stat: BattleStat::from_raw(stats[slot]),
                        stages: stages[slot] as i8,
                        chance: chances[slot],
                    })
                    .collect(),
                effect,
            })
        })
        .collect()
}

raw_enum! {
    /// Gen 5 orders the classes differently from Gen 4.
    pub enum MoveClass: u8 {
        Status = 0,
        Physical = 1,
        Special = 2,
    }
}

raw_enum! {
    /// What a move does, broadly, as the battle engine groups moves.
    pub enum MoveKind: u8 {
        Damage = 0,
        /// Inflicts an ailment, without damage.
        Ailment = 1,
        /// Changes stats in the user's favor, without damage: raises its own or lowers a foe's.
        StatChange = 2,
        Heal = 3,
        DamageAilment = 4,
        /// Confuses the target and raises one of its stats: Swagger and Flatter.
        AilmentRaiseStat = 5,
        DamageLowerStat = 6,
        DamageRaiseStat = 7,
        /// Damages and restores the user's HP from the damage dealt.
        DamageDrain = 8,
        OneHitKo = 9,
        /// Affects the whole field, such as the weather.
        FieldEffect = 10,
        /// Affects one side of the field, such as Reflect or Spikes.
        SideEffect = 11,
        /// Forces the target out: Whirlwind and Roar.
        ForceSwitch = 12,
        /// Its own behavior, from its effect script.
        Unique = 13,
    }
}

raw_enum! {
    /// Who a move can hit.
    pub enum MoveTarget: u8 {
        /// Any adjacent Pokémon, chosen.
        Selected = 0,
        /// Acupressure.
        UserOrAlly = 1,
        Ally = 2,
        /// Me First.
        SelectedOpponent = 3,
        AllAdjacent = 4,
        AllAdjacentOpponents = 5,
        /// The user's whole party: Heal Bell and Aromatherapy.
        UserParty = 6,
        User = 7,
        /// Every Pokémon in battle: Perish Song.
        All = 8,
        RandomOpponent = 9,
        Field = 10,
        OpponentSide = 11,
        UserSide = 12,
        /// Decided by the move itself, e.g. Counter or Curse.
        Automatic = 13,
    }
}

raw_enum! {
    /// A condition a move can inflict.
    pub enum Ailment: u16 {
        None = 0,
        Paralysis = 1,
        Sleep = 2,
        Freeze = 3,
        Burn = 4,
        /// Badly poisoned when the ailment lasts 15 turns (Toxic, Poison Fang).
        Poison = 5,
        Confusion = 6,
        Infatuation = 7,
        /// Bound in place, as by Wrap.
        Trap = 8,
        Nightmare = 9,
        Torment = 12,
        Disable = 13,
        Yawn = 14,
        HealBlock = 15,
        /// Loses its Ghost or Dark immunities, or its evasion: Foresight, Miracle Eye.
        Identified = 17,
        LeechSeed = 18,
        Embargo = 19,
        PerishSong = 20,
        Ingrain = 21,
        /// Decided by the move itself: Tri Attack, Telekinesis, Smack Down.
        Special = 0xFFFF,
    }
}

raw_enum! {
    /// A stat that moves raise or lower in battle.
    pub enum BattleStat: u8 {
        Attack = 1,
        Defense = 2,
        SpecialAttack = 3,
        SpecialDefense = 4,
        Speed = 5,
        Accuracy = 6,
        Evasion = 7,
        /// Attack, Defense, Speed, Special Attack, and Special Defense at once.
        All = 8,
    }
}

/// How a move behaves in battle. Serialized as the names of the flags that are set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MoveFlags(pub u32);

impl MoveFlags {
    /// Flag names, by bit.
    pub const NAMES: [&str; 14] = [
        "makes_contact",
        "charges_first",
        "recharges_after",
        "blocked_by_protect",
        "reflected_by_magic_coat",
        "stolen_by_snatch",
        "copied_by_mirror_move",
        "punches",
        "makes_sound",
        "fails_in_gravity",
        "thaws_user",
        // Reaches any Pokémon in a Triple Battle, not only adjacent ones.
        "reaches_far",
        "heals",
        "bypasses_substitute",
    ];

    /// The names of the flags that are set; unknown bits are named by number.
    pub fn names(self) -> impl Iterator<Item = String> {
        (0..32)
            .filter(move |bit| self.0 & 1 << bit != 0)
            .map(|bit| match Self::NAMES.get(bit) {
                Some(name) => (*name).to_owned(),
                None => format!("bit_{bit}"),
            })
    }
}

impl Serialize for MoveFlags {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.names())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::narc;

    #[test]
    fn parses_records() {
        // Thunderbolt, and Psychic's stat change.
        let thunderbolt = [
            0x0C, 4, 2, 95, 100, 15, 0, 0, 1, 0, 10, 1, 0, 0, 0, 0, 6, 0, 0, 0, 0, 4, 0, 0, 0xFF,
            0, 0, 10, 0, 0, 0x53, 0x53, 0x48, 0, 0, 0,
        ];
        let comet_punch = [
            &[0, 0, 1, 18, 85, 15, 0, 0x52][..],
            &[0; 24],
            &[0x89, 0, 0, 0x80],
        ]
        .concat();
        let table = narc(&[&[0; 36], &thunderbolt, &comet_punch]);
        let names = ["", "Thunderbolt"].map(str::to_owned);
        let moves = parse(&table, &names, &[]).unwrap();
        let bolt = &moves[0];
        assert_eq!(bolt.name.as_deref(), Some("Thunderbolt"));
        assert_eq!(
            (bolt.r#type, bolt.class, bolt.power),
            (Type::Electric, MoveClass::Special, 95)
        );
        assert_eq!(
            (bolt.kind, bolt.accuracy),
            (MoveKind::DamageAilment, Some(100))
        );
        assert_eq!(
            (bolt.ailment, bolt.ailment_chance),
            (Ailment::Paralysis, 10)
        );
        assert_eq!(
            bolt.stat_changes,
            [StatChange {
                stat: BattleStat::SpecialDefense,
                stages: -1,
                chance: 10
            }]
        );
        assert_eq!(
            serde_json::to_value(bolt.flags).unwrap(),
            serde_json::json!(["blocked_by_protect", "copied_by_mirror_move"])
        );
        let punch = &moves[1];
        assert_eq!(punch.hits, Some(Span { min: 2, max: 5 }));
        assert_eq!(punch.ailment_turns, None);
        assert_eq!(
            punch.flags.names().collect::<Vec<_>>(),
            ["makes_contact", "blocked_by_protect", "punches", "bit_31"]
        );
        assert!(parse(&narc(&[&[0; 36], &[0; 35]]), &[], &[]).is_err());
    }
}
