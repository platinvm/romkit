//! Items: one 36-byte record per item in the item table. The fields read here are the ones
//! every item has; the rest describe what medicine restores.

use serde::Serialize;

use super::{ItemId, Type};
use crate::Result;
use crate::formats::narc::Narc;
use crate::io::Reader;

const RECORD_LEN: usize = 36;
/// The Natural Gift type of items that do not work with Natural Gift.
const NO_TYPE: u8 = 31;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    pub id: ItemId,
    pub name: Option<String>,
    pub description: Option<String>,
    pub price: u32,
    pub pocket: Pocket,
    /// The battle bag pockets it appears in.
    pub battle_pockets: BattlePockets,
    /// Can be registered to the Y button.
    pub registrable: bool,
    /// The effect of holding it, and that effect's parameter.
    pub held_effect: u8,
    pub held_parameter: u8,
    pub fling_power: u8,
    pub fling_effect: u8,
    /// For Berries: Natural Gift's type and power when holding it.
    pub natural_gift: Option<NaturalGift>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct NaturalGift {
    pub r#type: Type,
    pub power: u8,
}

/// Every item but the placeholder at index 0.
pub(super) fn parse(table: &[u8], names: &[String], descriptions: &[String]) -> Result<Vec<Item>> {
    let narc = Narc::parse(table)?;
    (1..narc.len())
        .map(|index| {
            let bytes = narc.member(table, index)?;
            let mut record = Reader::new("Gen 5 item", &bytes);
            if bytes.len() != RECORD_LEN {
                return Err(record.error_at(0, "record is not 36 bytes"));
            }
            // Prices are stored in tens.
            let price = u32::from(record.u16()?) * 10;
            let held_effect = record.u8()?;
            let held_parameter = record.u8()?;
            record.skip(1)?; // Natural Gift's effect
            let fling_effect = record.u8()?;
            let fling_power = record.u8()?;
            let gift_power = record.u8()?;
            // The Natural Gift type (5 bits), two flags, the pocket (4 bits), and the battle
            // pockets (5 bits).
            let packed = record.u16()?;
            let gift_type = (packed & 0x1F) as u8;
            Ok(Item {
                id: ItemId(index as u16),
                name: names.get(index).cloned(),
                description: descriptions.get(index).cloned(),
                price,
                pocket: Pocket::from_raw((packed >> 7 & 0xF) as u8),
                battle_pockets: BattlePockets((packed >> 11) as u8),
                registrable: packed & 1 << 6 != 0,
                held_effect,
                held_parameter,
                fling_power,
                fling_effect,
                natural_gift: (gift_type != NO_TYPE && gift_power != 0).then_some(NaturalGift {
                    r#type: Type::from_raw(gift_type),
                    power: gift_power,
                }),
            })
        })
        .collect()
}

raw_enum! {
    /// Where the bag keeps an item.
    pub enum Pocket: u8 {
        Items = 0,
        Medicine = 1,
        Machines = 2,
        Berries = 3,
        KeyItems = 4,
    }
}

/// Which battle bag pockets hold an item. Serialized as the names of the pockets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BattlePockets(pub u8);

impl BattlePockets {
    /// Pocket names, by bit.
    pub const NAMES: [&str; 4] = [
        "poke_balls",
        "battle_items",
        "hp_pp_restore",
        "status_restore",
    ];

    /// The names of the pockets that are set; unknown bits are named by number.
    pub fn names(self) -> impl Iterator<Item = String> {
        (0..8)
            .filter(move |bit| self.0 & 1 << bit != 0)
            .map(|bit| match Self::NAMES.get(bit) {
                Some(name) => (*name).to_owned(),
                None => format!("bit_{bit}"),
            })
    }
}

impl Serialize for BattlePockets {
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
        let potion = [&[0x1E, 0, 0, 20, 0, 0, 30, 0, 0x9F, 0x20][..], &[0; 26]].concat();
        let oran = [&[2, 0, 1, 10, 7, 7, 10, 60, 0x83, 0x21][..], &[0; 26]].concat();
        let table = narc(&[&[0; 36], &potion, &oran]);
        let items = parse(&table, &["", "Potion"].map(str::to_owned), &[]).unwrap();
        let potion = &items[0];
        assert_eq!(potion.name.as_deref(), Some("Potion"));
        assert_eq!((potion.price, potion.pocket), (300, Pocket::Medicine));
        assert_eq!((potion.fling_power, potion.natural_gift), (30, None));
        assert_eq!(
            serde_json::to_value(potion.battle_pockets).unwrap(),
            serde_json::json!(["hp_pp_restore"])
        );
        let oran = &items[1];
        assert_eq!(oran.pocket, Pocket::Berries);
        assert_eq!(
            oran.natural_gift,
            Some(NaturalGift {
                r#type: Type::Poison,
                power: 60
            })
        );
        assert!(parse(&narc(&[&[0; 36], &[0; 20]]), &[], &[]).is_err());
    }
}
