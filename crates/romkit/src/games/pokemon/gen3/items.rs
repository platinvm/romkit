//! Items: one 44-byte record per item, which holds its name and points to its description and
//! to the code that uses it.

use serde::Serialize;

use super::{Edition, Gen3, ItemId, text};
use crate::Result;
use crate::io::{ByteSource, Reader};

const RECORD_LEN: usize = 44;
const NAME_LEN: usize = 14;
const DESCRIPTION: u32 = 0x14;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    pub id: ItemId,
    pub name: String,
    pub description: String,
    pub price: u16,
    pub pocket: Pocket,
    /// The effect of holding it, and that effect's parameter.
    pub hold_effect: u8,
    pub hold_parameter: u8,
    /// Nonzero for key items, which cannot be sold, tossed, or held.
    pub importance: u8,
    /// Can be registered to the Select button.
    pub registrable: bool,
    /// How it is used from the bag: by its own menu, on a party Pokémon, in the field.
    pub usage: u8,
    /// Nonzero when it can be used in battle.
    pub battle_usage: u8,
    /// A number the item's code reads, such as the Poké Ball kind or the TM's index.
    pub secondary_id: u32,
}

/// Every item but the placeholder at index 0.
pub(super) fn parse<S: ByteSource>(game: &Gen3<S>) -> Result<Vec<Item>> {
    let edition = game.edition;
    let count = edition.item_count as usize;
    let records = game.read(edition.items, count * RECORD_LEN)?;
    (1..count)
        .map(|index| {
            let mut item =
                parse_record(edition, index, &records[index * RECORD_LEN..][..RECORD_LEN])?;
            let at = edition.items + (index * RECORD_LEN) as u32 + DESCRIPTION;
            item.description = game.string_at(at)?;
            Ok(item)
        })
        .collect()
}

fn parse_record(edition: &Edition, index: usize, bytes: &[u8]) -> Result<Item> {
    let mut record = Reader::new("Gen 3 item", bytes);
    let name = text::decode(record.take(NAME_LEN)?);
    record.skip(2)?; // the item's own number, 0 for unused slots
    let price = record.u16()?;
    let hold_effect = record.u8()?;
    let hold_parameter = record.u8()?;
    record.skip(4)?; // the description pointer
    let importance = record.u8()?;
    let registrable = record.u8()? != 0;
    let pocket = match record.u8()? {
        raw @ 1..=5 => edition.pockets[raw as usize - 1],
        raw => Pocket::Unknown(raw),
    };
    let usage = record.u8()?;
    record.skip(4)?; // the field use function
    let battle_usage = record.u8()?;
    record.skip(7)?; // padding and the battle use function
    Ok(Item {
        id: ItemId(index as u16),
        name,
        description: String::new(),
        price,
        pocket,
        hold_effect,
        hold_parameter,
        importance,
        registrable,
        usage,
        battle_usage,
        secondary_id: record.u32()?,
    })
}

raw_enum! {
    /// Where the bag keeps an item, by Ruby and Sapphire's numbering.
    pub enum Pocket: u8 {
        Items = 1,
        PokeBalls = 2,
        /// TMs and HMs; FireRed and LeafGreen's TM Case.
        Machines = 3,
        /// FireRed and LeafGreen's Berry Pouch.
        Berries = 4,
        KeyItems = 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::pokemon::gen3::{FIRERED, RUBY};

    #[test]
    fn parses_records_with_each_editions_pockets() {
        // "POTION", price 300, pocket 1, then a key item in pocket 2.
        let mut potion = [0u8; RECORD_LEN];
        potion[..7].copy_from_slice(&[0xCA, 0xC9, 0xCE, 0xC3, 0xC9, 0xC8, 0xFF]);
        potion[0x10..0x12].copy_from_slice(&300u16.to_le_bytes());
        potion[0x1A] = 1;
        potion[0x20] = 1;
        let item = parse_record(&RUBY, 13, &potion).unwrap();
        assert_eq!((item.name.as_str(), item.price), ("POTION", 300));
        assert_eq!((item.pocket, item.battle_usage), (Pocket::Items, 1));

        let mut key = potion;
        key[0x1A] = 2;
        assert_eq!(
            parse_record(&RUBY, 1, &key).unwrap().pocket,
            Pocket::PokeBalls
        );
        assert_eq!(
            parse_record(&FIRERED, 1, &key).unwrap().pocket,
            Pocket::KeyItems
        );
        key[0x1A] = 9;
        assert_eq!(
            parse_record(&RUBY, 1, &key).unwrap().pocket,
            Pocket::Unknown(9)
        );
        assert!(parse_record(&RUBY, 1, &key[..40]).is_err());
    }
}
