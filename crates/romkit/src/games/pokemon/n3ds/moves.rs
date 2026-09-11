//! Moves: one record per move, 0x24 bytes in Generation VI and 0x28 in VII. X and Y store one
//! record per archive entry; the later games pack them all into one entry (a `WD` pack).
//!
//! ```text
//! 0x00  type, category, class, power, accuracy, PP, priority, hit counts (min | max << 4)
//! 0x08  ailment (u16), its chance, its duration kind, its min and max turns
//! 0x0E  critical stage, flinch chance, effect (u16), drain, healing, target
//! 0x15  stat changes: three stats, three stages, three chances
//! 0x1E  flags (u32 at 0x20 in VI); VII has its Z-Move (u16), Z-Power, Z-effect, two unknown
//!       bytes, then the flags at 0x24
//! ```

use serde::{Serialize, Serializer};

use super::{MoveId, Type};
use crate::Result;
use crate::formats::garc::Garc;
use crate::io::Reader;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Move {
    pub id: MoveId,
    pub name: Option<String>,
    pub description: Option<String>,
    pub r#type: Type,
    pub class: MoveClass,
    pub power: u8,
    /// 101 for moves that never miss.
    pub accuracy: u8,
    pub pp: u8,
    pub priority: i8,
    pub target: MoveTarget,
    pub flags: MoveFlags,
    pub category: MoveCategory,
    /// The battle effect script.
    pub effect: u16,
    /// How many times it hits, `[min, max]`; `[0, 0]` for once.
    pub hits: [u8; 2],
    pub ailment: Ailment,
    /// The chance of the ailment in percent, 0 when it always happens.
    pub ailment_chance: u8,
    /// How many turns the ailment lasts, `[min, max]`; `[0, 0]` when the ailment decides.
    pub ailment_turns: [u8; 2],
    /// Raises the critical hit ratio by this many stages.
    pub critical_stage: u8,
    pub flinch_chance: u8,
    /// The percentage of the damage dealt that the user regains, or takes as recoil when
    /// negative.
    pub drain: i8,
    /// The percentage of its maximum HP that the user regains, or loses when negative.
    pub healing: i8,
    pub stat_changes: Vec<StatChange>,
    /// What Z-Power makes of the move (Generation VII).
    pub z_move: Option<ZMove>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StatChange {
    pub stat: Stat,
    pub stages: i8,
    /// The chance in percent, 0 when it always happens.
    pub chance: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ZMove {
    /// The Z-Move it becomes; status moves keep their own and gain an effect instead.
    pub r#move: Option<MoveId>,
    pub power: u8,
    /// The extra effect of a Z-powered status move.
    pub effect: u8,
}

/// Every move but the placeholder at index 0.
pub(super) fn parse(
    generation: u8,
    table: &[u8],
    names: &[String],
    descriptions: &[String],
) -> Result<Vec<Move>> {
    let garc = Garc::parse(table)?;
    let records = match garc.len() {
        1 => unpack_wd(&garc.member(table, 0)?)?,
        len => (0..len)
            .map(|index| Ok(garc.member(table, index)?.into_owned()))
            .collect::<Result<_>>()?,
    };
    records
        .iter()
        .enumerate()
        .skip(1)
        .map(|(index, record)| {
            let mut parsed = parse_move(generation, record)?;
            parsed.id = MoveId(index as u16);
            parsed.name = names.get(index).cloned();
            parsed.description = descriptions.get(index).cloned();
            Ok(parsed)
        })
        .collect()
}

/// A `WD` pack: the signature, a u16 count, then count + 1 offsets bounding the records.
fn unpack_wd(bytes: &[u8]) -> Result<Vec<Vec<u8>>> {
    let mut pack = Reader::new("WD pack", bytes);
    pack.expect(b"WD")?;
    let count = pack.u16()? as usize;
    let offsets = (0..=count)
        .map(|_| Ok(pack.u32()? as usize))
        .collect::<Result<Vec<_>>>()?;
    offsets
        .windows(2)
        .map(|bounds| {
            let len = bounds[1]
                .checked_sub(bounds[0])
                .ok_or_else(|| pack.error_at(bounds[0], "records overlap"))?;
            Ok(pack.slice_at(bounds[0], len)?.to_vec())
        })
        .collect()
}

fn parse_move(generation: u8, bytes: &[u8]) -> Result<Move> {
    let record = Reader::new("Gen 6/7 move", bytes);
    let len = if generation == 7 { 0x28 } else { 0x24 };
    if bytes.len() != len {
        return Err(record.error_at(0, "record has the wrong length"));
    }
    let byte = |at| record.u8_at(at);
    let signed = |at| -> Result<i8> { Ok(record.u8_at(at)? as i8) };
    let stat_changes = (0..3)
        .map(|index| -> Result<_> {
            Ok(StatChange {
                stat: Stat::from_raw(byte(0x15 + index)?),
                stages: signed(0x18 + index)?,
                chance: byte(0x1B + index)?,
            })
        })
        .filter(|change| !matches!(change, Ok(change) if change.stat.raw() == 0))
        .collect::<Result<_>>()?;
    let z_move = match generation {
        7 => Some(ZMove {
            r#move: Some(record.u16_at(0x1E)?).filter(|&id| id != 0).map(MoveId),
            power: byte(0x20)?,
            effect: byte(0x21)?,
        }),
        _ => None,
    };
    let hits = byte(0x07)?;
    Ok(Move {
        id: MoveId(0),
        name: None,
        description: None,
        r#type: Type::from_raw(byte(0x00)?),
        category: MoveCategory::from_raw(byte(0x01)?),
        class: MoveClass::from_raw(byte(0x02)?),
        power: byte(0x03)?,
        accuracy: byte(0x04)?,
        pp: byte(0x05)?,
        priority: signed(0x06)?,
        hits: [hits & 0xF, hits >> 4],
        ailment: Ailment::from_raw(record.u16_at(0x08)?),
        ailment_chance: byte(0x0A)?,
        ailment_turns: [byte(0x0C)?, byte(0x0D)?],
        critical_stage: byte(0x0E)?,
        flinch_chance: byte(0x0F)?,
        effect: record.u16_at(0x10)?,
        drain: signed(0x12)?,
        healing: signed(0x13)?,
        target: MoveTarget::from_raw(byte(0x14)?),
        stat_changes,
        flags: MoveFlags(record.u32_at(len - 4)?),
        z_move,
    })
}

raw_enum! {
    pub enum MoveClass: u8 {
        Status = 0,
        Physical = 1,
        Special = 2,
    }
}

raw_enum! {
    /// What a move mainly does, for the battle AI and move effects.
    pub enum MoveCategory: u8 {
        Damage = 0,
        Ailment = 1,
        /// Raises the user's stats or lowers the target's.
        NetGoodStats = 2,
        Heal = 3,
        DamageAilment = 4,
        /// Raises the target's stats and inflicts an ailment (Swagger, Flatter).
        Swagger = 5,
        DamageLowerStats = 6,
        DamageRaiseStats = 7,
        DamageHeal = 8,
        OneHitKnockOut = 9,
        /// Affects the whole field (weather, Haze).
        WholeFieldEffect = 10,
        /// Affects one side of the field (screens, hazards).
        SideFieldEffect = 11,
        ForceSwitch = 12,
        Unique = 13,
    }
}

raw_enum! {
    /// Who a move can hit.
    pub enum MoveTarget: u8 {
        /// One adjacent Pokémon of the user's choice.
        Selected = 0,
        AllyOrUser = 1,
        Ally = 2,
        /// One adjacent opponent of the user's choice.
        Opponent = 3,
        AllAdjacent = 4,
        AllAdjacentOpponents = 5,
        /// The user's whole party (Heal Bell).
        UserParty = 6,
        User = 7,
        /// Every Pokémon on the field (Perish Song).
        All = 8,
        RandomOpponent = 9,
        Field = 10,
        OpponentSide = 11,
        UserSide = 12,
        /// Decided by the move itself, e.g. Counter.
        Automatic = 13,
    }
}

raw_enum! {
    /// A status condition a move may inflict.
    pub enum Ailment: u16 {
        None = 0,
        Paralysis = 1,
        Sleep = 2,
        Freeze = 3,
        Burn = 4,
        Poison = 5,
        Confusion = 6,
        Infatuation = 7,
        /// Bound by Wrap and the like.
        Trap = 8,
        Nightmare = 9,
    }
}

raw_enum! {
    pub enum Stat: u8 {
        Attack = 1,
        Defense = 2,
        SpecialAttack = 3,
        SpecialDefense = 4,
        Speed = 5,
        Accuracy = 6,
        Evasion = 7,
        /// Attack, Defense, Special Attack, Special Defense, and Speed at once.
        All = 8,
    }
}

/// How a move behaves in battle. Serialized as the names of the flags that are set, `bit_N`
/// for those without a known meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MoveFlags(pub u32);

impl MoveFlags {
    /// Flag names, by bit; empty where unknown.
    pub const NAMES: [&str; 17] = [
        "makes_contact",
        "charges_first",
        "recharges_after",
        "blocked_by_protect",
        "reflected_by_magic_coat",
        "stolen_by_snatch",
        "copied_by_mirror_move",
        "punch",
        "sound",
        "fails_in_gravity",
        "thaws_user",
        "reaches_far_in_triples",
        "heals",
        "bypasses_substitute",
        "fails_in_sky_battles",
        "",
        "dance",
    ];

    pub fn names(self) -> impl Iterator<Item = String> {
        (0..32)
            .filter(move |bit| self.0 >> bit & 1 != 0)
            .map(
                |bit| match Self::NAMES.get(bit).filter(|name| !name.is_empty()) {
                    Some(name) => (*name).to_owned(),
                    None => format!("bit_{bit}"),
                },
            )
    }
}

impl Serialize for MoveFlags {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.names())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Thunderbolt as Sun and Moon store it.
    const THUNDERBOLT: [u8; 0x28] = [
        0x0C, 0x04, 0x02, 0x5A, 0x64, 0x0F, 0, 0, 0x01, 0, 0x0A, 0x01, 0, 0, 0, 0, //
        0x06, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x86, 0x02, //
        0xAF, 0, 0, 0, 0x48, 0, 0, 0,
    ];

    #[test]
    fn parses_generation_7_records() {
        let thunderbolt = parse_move(7, &THUNDERBOLT).unwrap();
        assert_eq!(thunderbolt.r#type, Type::Electric);
        assert_eq!(thunderbolt.class, MoveClass::Special);
        assert_eq!(
            (thunderbolt.power, thunderbolt.accuracy, thunderbolt.pp),
            (90, 100, 15)
        );
        assert_eq!(thunderbolt.ailment, Ailment::Paralysis);
        assert_eq!(thunderbolt.ailment_chance, 10);
        assert_eq!(
            thunderbolt.flags.names().collect::<Vec<_>>(),
            ["blocked_by_protect", "copied_by_mirror_move"]
        );
        let z_move = thunderbolt.z_move.unwrap();
        assert_eq!((z_move.r#move, z_move.power), (Some(MoveId(646)), 175));
        assert!(thunderbolt.stat_changes.is_empty());
        assert!(parse_move(6, &THUNDERBOLT).is_err());
    }

    #[test]
    fn unpacks_wd_packs() {
        let pack = [
            b"WD\x02\x00".as_slice(),
            &[16, 0, 0, 0, 18, 0, 0, 0, 19, 0, 0, 0],
            b"abc",
        ]
        .concat();
        assert_eq!(unpack_wd(&pack).unwrap(), [b"ab".to_vec(), b"c".to_vec()]);
        assert!(unpack_wd(&pack[..15]).is_err());
        assert_eq!(
            MoveFlags(1 << 15 | 1).names().collect::<Vec<_>>(),
            ["makes_contact", "bit_15"]
        );
    }
}
