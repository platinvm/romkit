//! Moves: one 16-byte record per move in the move table.

use serde::{Serialize, Serializer};

use super::{MoveId, Type};
use crate::Result;
use crate::formats::narc::Narc;
use crate::io::Reader;

const RECORD_LEN: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Move {
    pub id: MoveId,
    pub name: Option<String>,
    pub description: Option<String>,
    pub r#type: Type,
    pub class: MoveClass,
    pub power: u8,
    pub accuracy: u8,
    pub pp: u8,
    pub priority: i8,
    pub target: MoveTarget,
    pub flags: MoveFlags,
    /// The battle effect script.
    pub effect: u16,
    /// The chance of the effect's secondary part, in percent.
    pub effect_chance: u8,
    pub contest_type: ContestType,
    pub contest_effect: u8,
}

/// Every move but the placeholder at index 0.
pub(super) fn parse(table: &[u8], names: &[String], descriptions: &[String]) -> Result<Vec<Move>> {
    let narc = Narc::parse(table)?;
    (1..narc.len())
        .map(|index| {
            let bytes = narc.member(table, index)?;
            let mut record = Reader::new("Gen 4 move", &bytes);
            if bytes.len() != RECORD_LEN {
                return Err(record.error_at(0, "record is not 16 bytes"));
            }
            Ok(Move {
                id: MoveId(index as u16),
                name: names.get(index).cloned(),
                description: descriptions.get(index).cloned(),
                effect: record.u16()?,
                class: MoveClass::from_raw(record.u8()?),
                power: record.u8()?,
                r#type: Type::from_raw(record.u8()?),
                accuracy: record.u8()?,
                pp: record.u8()?,
                effect_chance: record.u8()?,
                target: MoveTarget::from_raw(record.u16()?),
                priority: record.i8()?,
                flags: MoveFlags(record.u8()?),
                contest_effect: record.u8()?,
                contest_type: ContestType::from_raw(record.u8()?),
            })
        })
        .collect()
}

raw_enum! {
    pub enum MoveClass: u8 {
        Physical = 0,
        Special = 1,
        Status = 2,
    }
}

raw_enum! {
    /// Who a move can hit.
    pub enum MoveTarget: u16 {
        Selected = 0x0000,
        /// Picked by the move itself, e.g. Counter.
        Automatic = 0x0001,
        RandomOpponent = 0x0002,
        BothOpponents = 0x0004,
        AllOthers = 0x0008,
        User = 0x0010,
        UserSide = 0x0020,
        Field = 0x0040,
        OpponentSide = 0x0080,
        Ally = 0x0100,
        UserOrAlly = 0x0200,
        /// Me First's own targeting.
        MeFirst = 0x0400,
    }
}

raw_enum! {
    pub enum ContestType: u8 {
        Cool = 0,
        Beauty = 1,
        Cute = 2,
        Smart = 3,
        Tough = 4,
    }
}

/// How a move behaves in battle. Serialized as the names of the flags that are set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MoveFlags(pub u8);

impl MoveFlags {
    /// Flag names, by bit.
    pub const NAMES: [&str; 8] = [
        "makes_contact",
        "blocked_by_protect",
        "reflected_by_magic_coat",
        "stolen_by_snatch",
        "copied_by_mirror_move",
        "triggers_kings_rock",
        "hides_hp_bars",
        "hides_shadows",
    ];

    pub fn names(self) -> impl Iterator<Item = &'static str> {
        (0..8)
            .filter(move |bit| self.0 & 1 << bit != 0)
            .map(|bit| Self::NAMES[bit])
    }
}

impl Serialize for MoveFlags {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.names())
    }
}
