//! Moves: one 7-byte record per move (its number, effect, power, type, accuracy, PP, and the
//! chance of the effect), names in a list, and descriptions through a table of addresses.

use serde::Serialize;

use super::{MoveId, Type};
use crate::Result;
use crate::games::pokemon::gen1::percent;
use crate::io::Reader;

const RECORD_LEN: usize = 7;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Move {
    pub id: MoveId,
    pub name: String,
    pub description: String,
    pub r#type: Type,
    pub power: u8,
    /// In percent, restored from the stored fraction of 255.
    pub accuracy: u8,
    pub pp: u8,
    /// The battle effect script...
    pub effect: u8,
    /// ...and the chance of its secondary part, in percent.
    pub effect_chance: u8,
    /// The TM, HM, or tutor that teaches it, numbered as in
    /// [`Species::machines`](super::Species).
    pub machine: Option<u8>,
}

/// Every move, from the records, names, descriptions, and the move of each TM, HM, and tutor.
pub(super) fn parse(
    records: &[u8],
    names: &[String],
    descriptions: &[String],
    machines: &[u8],
) -> Result<Vec<Move>> {
    let reader = Reader::new("Gen 2 move", records);
    records
        .as_chunks::<RECORD_LEN>()
        .0
        .iter()
        .enumerate()
        .map(|(index, bytes)| {
            let mut record = Reader::new("Gen 2 move", bytes);
            let id = record.u8()?;
            if usize::from(id) != index + 1 {
                return Err(reader.error_at(index * RECORD_LEN, "out of order"));
            }
            Ok(Move {
                id: MoveId(id.into()),
                name: names.get(index).cloned().unwrap_or_default(),
                description: descriptions.get(index).cloned().unwrap_or_default(),
                effect: record.u8()?,
                power: record.u8()?,
                r#type: Type::from_raw(record.u8()?),
                accuracy: percent(record.u8()?),
                pp: record.u8()?,
                effect_chance: percent(record.u8()?),
                machine: machines
                    .iter()
                    .position(|&taught| taught == id)
                    .map(|machine| machine as u8),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_records() {
        let records = [1, 6, 95, 0x17, 255, 15, 25];
        let moves = parse(&records, &["THUNDERBOLT".to_owned()], &[], &[2, 1]).unwrap();
        let thunderbolt = &moves[0];
        assert_eq!(
            (thunderbolt.power, thunderbolt.r#type),
            (95, Type::Electric)
        );
        assert_eq!(
            (thunderbolt.effect_chance, thunderbolt.machine),
            (10, Some(1))
        );
        assert!(parse(&[2, 0, 0, 0, 0, 0, 0], &[], &[], &[]).is_err());
    }
}
