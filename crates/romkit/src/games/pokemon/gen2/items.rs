//! Items: 7-byte attribute records (price, held effect and its parameter, permissions, pocket,
//! and the menus that can use it), names in a list, and descriptions through a table of
//! addresses.

use serde::Serialize;

use super::ItemId;
use crate::Result;
use crate::io::Reader;

const RECORD_LEN: usize = 7;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    pub id: ItemId,
    pub name: String,
    pub description: String,
    pub price: u16,
    pub pocket: Pocket,
    /// The effect of holding it, and that effect's parameter.
    pub held_effect: u8,
    pub held_parameter: u8,
    /// Whether it can be tossed, selected, or registered, as flags.
    pub permissions: u8,
    /// How the bag uses it outside battle and in battle.
    pub field_menu: u8,
    pub battle_menu: u8,
}

raw_enum! {
    pub enum Pocket: u8 {
        Items = 1,
        KeyItems = 2,
        Balls = 3,
        Machines = 4,
    }
}

pub(super) fn parse(
    attributes: &[u8],
    names: &[String],
    descriptions: &[String],
) -> Result<Vec<Item>> {
    attributes
        .as_chunks::<RECORD_LEN>()
        .0
        .iter()
        .enumerate()
        .map(|(index, bytes)| {
            let mut record = Reader::new("Gen 2 item", bytes);
            let price = record.u16()?;
            let held_effect = record.u8()?;
            let held_parameter = record.u8()?;
            let permissions = record.u8()?;
            let pocket = Pocket::from_raw(record.u8()?);
            let menus = record.u8()?;
            Ok(Item {
                id: ItemId(index as u16 + 1),
                name: names.get(index).cloned().unwrap_or_default(),
                description: descriptions.get(index).cloned().unwrap_or_default(),
                price,
                pocket,
                held_effect,
                held_parameter,
                permissions,
                field_menu: menus >> 4,
                battle_menu: menus & 0xF,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_attributes() {
        let potion = [0x2C, 0x01, 0, 20, 0x40, 1, 0x55];
        let items = parse(&potion, &["POTION".to_owned()], &[]).unwrap();
        assert_eq!((items[0].price, items[0].pocket), (300, Pocket::Items));
        assert_eq!((items[0].field_menu, items[0].battle_menu), (5, 5));
    }
}
