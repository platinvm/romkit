//! Moves: one 6-byte record per move (its number, effect, power, type, accuracy, and PP),
//! names in a list of their own, and the list of moves the TMs and HMs teach.

use serde::Serialize;

use super::{MoveId, Type};
use crate::Result;
use crate::io::Reader;

const RECORD_LEN: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Move {
    pub id: MoveId,
    pub name: String,
    pub r#type: Type,
    pub power: u8,
    /// In percent. The games store it out of 255; every stored value comes from a whole
    /// percentage, which this restores.
    pub accuracy: u8,
    pub pp: u8,
    /// The battle effect script.
    pub effect: u8,
    /// The TM or HM that teaches it, numbered as in [`Species::machines`](super::Species).
    pub machine: Option<u8>,
}

/// Every move, from the records, their names, and the move of each TM and HM.
pub(super) fn parse(records: &[u8], names: &[String], machines: &[u8]) -> Result<Vec<Move>> {
    let reader = Reader::new("Gen 1 move", records);
    records
        .as_chunks::<RECORD_LEN>()
        .0
        .iter()
        .enumerate()
        .map(|(index, bytes)| {
            let mut record = Reader::new("Gen 1 move", bytes);
            let id = record.u8()?;
            if usize::from(id) != index + 1 {
                return Err(reader.error_at(index * RECORD_LEN, "out of order"));
            }
            Ok(Move {
                id: MoveId(id.into()),
                name: names.get(index).cloned().unwrap_or_default(),
                effect: record.u8()?,
                power: record.u8()?,
                r#type: Type::from_raw(record.u8()?),
                accuracy: percent(record.u8()?),
                pp: record.u8()?,
                machine: machines
                    .iter()
                    .position(|&taught| taught == id)
                    .map(|machine| machine as u8),
            })
        })
        .collect()
}

/// A chance stored out of 255, in whole percent.
pub(crate) fn percent(raw: u8) -> u8 {
    ((u16::from(raw) * 100 + 127) / 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_records_and_finds_their_machines() {
        let records = [1, 0, 40, 0, 255, 35, 2, 0, 50, 0, 242, 25];
        let names = ["POUND", "KARATE CHOP"].map(str::to_owned);
        let moves = parse(&records, &names, &[5, 2]).unwrap();
        assert_eq!(moves[0].name, "POUND");
        assert_eq!((moves[0].accuracy, moves[1].accuracy), (100, 95));
        assert_eq!((moves[0].machine, moves[1].machine), (None, Some(1)));
        assert!(parse(&records[6..], &names, &[]).is_err());
    }

    #[test]
    fn restores_whole_percentages() {
        for percentage in 0..=100u16 {
            assert_eq!(percent((percentage * 255 / 100) as u8), percentage as u8);
        }
    }
}
