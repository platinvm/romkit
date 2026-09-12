//! Moves: one 12-byte record and one 13-byte name per move, and a pointer to each move's
//! description. Gen 3 has no physical or special class per move: the type decides.

use serde::Serialize;

use super::{Gen3, MoveFlags, MoveId, Type, text};
use crate::Result;
use crate::io::{ByteSource, Reader};

const MOVES: usize = 355;
const RECORD_LEN: usize = 12;
const NAME_LEN: usize = 13;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Move {
    pub id: MoveId,
    pub name: String,
    pub description: String,
    pub r#type: Type,
    pub power: u8,
    /// In percent; 0 for moves that never miss.
    pub accuracy: u8,
    pub pp: u8,
    pub priority: i8,
    pub target: MoveTarget,
    pub flags: MoveFlags,
    /// The battle effect script.
    pub effect: u8,
    /// The chance of the effect's secondary part, in percent.
    pub effect_chance: u8,
}

/// Every move but the placeholder at index 0.
pub(super) fn parse<S: ByteSource>(game: &Gen3<S>) -> Result<Vec<Move>> {
    let edition = game.edition;
    let records = game.read(edition.moves, MOVES * RECORD_LEN)?;
    let names = game.read(edition.move_names, MOVES * NAME_LEN)?;
    (1..MOVES)
        .map(|index| {
            let mut record = parse_record(index, &records[index * RECORD_LEN..][..RECORD_LEN])?;
            record.name = text::decode(&names[index * NAME_LEN..][..NAME_LEN]);
            record.description =
                game.string_at(edition.move_descriptions + 4 * (index as u32 - 1))?;
            Ok(record)
        })
        .collect()
}

fn parse_record(index: usize, bytes: &[u8]) -> Result<Move> {
    let mut record = Reader::new("Gen 3 move", bytes);
    Ok(Move {
        id: MoveId(index as u16),
        name: String::new(),
        description: String::new(),
        effect: record.u8()?,
        power: record.u8()?,
        r#type: Type::from_raw(record.u8()?),
        accuracy: record.u8()?,
        pp: record.u8()?,
        effect_chance: record.u8()?,
        target: MoveTarget::from_raw(record.u8()?),
        priority: record.i8()?,
        flags: MoveFlags(record.u8()?),
    })
}

raw_enum! {
    /// Who a move can hit.
    pub enum MoveTarget: u8 {
        Selected = 0x00,
        /// Picked by the move itself, e.g. Counter.
        Automatic = 0x01,
        UserOrSelected = 0x02,
        RandomOpponent = 0x04,
        BothOpponents = 0x08,
        User = 0x10,
        AllOthers = 0x20,
        OpponentSide = 0x40,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_records() {
        // Thunderbolt: paralyze effect, 95 power, Electric, 100%, 15 PP, 10% chance.
        let bytes = [6, 95, 13, 100, 15, 10, 0, 0, 0x12, 0, 0, 0];
        let thunderbolt = parse_record(85, &bytes).unwrap();
        assert_eq!(
            (thunderbolt.power, thunderbolt.r#type),
            (95, Type::Electric)
        );
        assert_eq!((thunderbolt.pp, thunderbolt.effect_chance), (15, 10));
        assert_eq!(thunderbolt.target, MoveTarget::Selected);
        assert_eq!(
            thunderbolt.flags.names().collect::<Vec<_>>(),
            ["blocked_by_protect", "copied_by_mirror_move"]
        );
        assert!(parse_record(1, &bytes[..8]).is_err());
    }
}
