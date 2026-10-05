//! Items: a list of names and a table of prices, three bytes of binary-coded decimal each
//! (`00 12 00` is 1200). TMs and HMs are items too, numbered from 0xC4, but have neither.

use serde::Serialize;

use super::ItemId;
use crate::Result;
use crate::io::Reader;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    pub id: ItemId,
    pub name: String,
    /// What shops charge; 0 for items they do not sell.
    pub price: u32,
}

pub(super) fn parse(names: &[String], prices: &[u8]) -> Result<Vec<Item>> {
    let reader = Reader::new("Gen 1 item prices", prices);
    prices
        .as_chunks::<3>()
        .0
        .iter()
        .enumerate()
        .map(|(index, price)| {
            Ok(Item {
                id: ItemId(index as u16 + 1),
                name: names.get(index).cloned().unwrap_or_default(),
                price: bcd(price).ok_or_else(|| reader.error_at(index * 3, "not decimal"))?,
            })
        })
        .collect()
}

/// A number stored as binary-coded decimal, two digits per byte, most significant first.
pub(crate) fn bcd(bytes: &[u8]) -> Option<u32> {
    bytes.iter().try_fold(0, |value, &byte| {
        let (high, low) = (u32::from(byte >> 4), u32::from(byte & 0xF));
        (high < 10 && low < 10).then_some(value * 100 + high * 10 + low)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_are_decimal() {
        let items = parse(
            &["ULTRA BALL".to_owned()],
            &[0x00, 0x12, 0x00, 0x01, 0x00, 0x50],
        );
        let items = items.unwrap();
        assert_eq!(
            (items[0].name.as_str(), items[0].price),
            ("ULTRA BALL", 1200)
        );
        assert_eq!((items[1].id, items[1].price), (ItemId(2), 10050));
        assert!(parse(&[], &[0x0A, 0, 0]).is_err());
    }
}
