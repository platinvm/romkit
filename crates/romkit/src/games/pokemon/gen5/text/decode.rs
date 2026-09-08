//! Bank decoding.
//!
//! A bank starts with its section count, its message count, the length of a section, and a
//! zero word, then each section's offset. A section is its length, one `(offset, length, flags)`
//! entry per message, offsets relative to the section, then the messages. Every release seen
//! has one section; only the first is read.
//!
//! A message is UTF-16, each code XORed with a key that rotates left by 3 bits per code. Since
//! every message ends with `0xFFFF`, its last code gives the key, and decoding runs backwards.

use super::{Message, Token};
use crate::io::Reader;
use crate::{Error, Result};

const FORMAT: &str = "Gen 5 text bank";

const END: u16 = 0xFFFF;
const LINE_BREAK: u16 = 0xFFFE;
/// Followed by the command's ID, its argument count, and the arguments.
const COMMAND: u16 = 0xF000;
/// Starts a message stored as 9-bit codes (trainer names), which ends with `PACKED_END`.
const PACKED: u16 = 0xF100;
const PACKED_END: u16 = 0x01FF;
/// The commands that end a text box line or page.
const SCROLL_LINE: u16 = 0xBE00;
const CLEAR_BOX: u16 = 0xBE01;
/// The font's symbols, where Unicode has enclosed alphanumerics.
const SYMBOLS: std::ops::RangeInclusive<u16> = 0x2460..=0x24FF;

/// Every message in a bank: those of its first section.
pub(super) fn bank(bytes: &[u8]) -> Result<Vec<Message>> {
    let bank = Reader::new(FORMAT, bytes);
    let (sections, count) = (bank.u16_at(0)?, bank.u16_at(2)?);
    if sections == 0 {
        return Ok(Vec::new());
    }
    let section = bank.u32_at(12)? as usize;
    (0..count as usize)
        .map(|index| {
            let entry = section.saturating_add(4 + index * 8);
            let offset = section.saturating_add(bank.u32_at(entry)? as usize);
            let len = bank.u16_at(entry + 4)? as usize;
            let codes = decrypt(bank.slice_at(offset, len * 2)?);
            tokenize(&codes).map_err(|reason| Error::malformed(FORMAT, offset as u64, reason))
        })
        .collect()
}

fn decrypt(bytes: &[u8]) -> Vec<u16> {
    let mut codes: Vec<u16> = bytes
        .as_chunks()
        .0
        .iter()
        .map(|&code| u16::from_le_bytes(code))
        .collect();
    let mut key = codes.last().map_or(0, |&last| last ^ END);
    for code in codes.iter_mut().rev() {
        *code ^= key;
        key = key.rotate_right(3);
    }
    codes
}

fn tokenize(codes: &[u16]) -> Result<Message, &'static str> {
    let unpacked;
    let (mut rest, end) = match codes.split_first() {
        Some((&PACKED, words)) => {
            unpacked = unpack(words);
            (unpacked.as_slice(), PACKED_END)
        }
        _ => (codes, END),
    };
    let mut tokens = Vec::new();
    let mut text = String::new();
    while let Some((&code, tail)) = rest.split_first() {
        rest = tail;
        let token = match code {
            _ if code == end => break,
            LINE_BREAK => Token::LineBreak,
            COMMAND => {
                let [id, argc, tail @ ..] = rest else {
                    return Err("truncated command");
                };
                let args = tail
                    .get(..*argc as usize)
                    .ok_or("truncated command arguments")?;
                rest = &tail[args.len()..];
                match (*id, args) {
                    (SCROLL_LINE, []) => Token::ScrollLine,
                    (CLEAR_BOX, []) => Token::ClearBox,
                    _ => Token::Command {
                        id: *id,
                        args: args.to_vec(),
                    },
                }
            }
            _ if SYMBOLS.contains(&code) => Token::Symbol(code),
            _ => match char::from_u32(code.into()).filter(|ch| !ch.is_control()) {
                Some(ch) => {
                    text.push(ch);
                    continue;
                }
                None => Token::Unknown(code),
            },
        };
        if !text.is_empty() {
            tokens.push(Token::Text(std::mem::take(&mut text)));
        }
        tokens.push(token);
    }
    if !text.is_empty() {
        tokens.push(Token::Text(text));
    }
    Ok(Message { tokens })
}

/// 9-bit codes packed into whole 16-bit words, least significant first.
fn unpack(words: &[u16]) -> Vec<u16> {
    let mut codes = Vec::with_capacity(words.len() * 16 / 9);
    let (mut buffer, mut bits) = (0u32, 0);
    for &word in words {
        buffer |= u32::from(word) << bits;
        bits += 16;
        while bits >= 9 {
            codes.push((buffer & 0x1FF) as u16);
            if codes.last() == Some(&PACKED_END) {
                return codes;
            }
            buffer >>= 9;
            bits -= 9;
        }
    }
    codes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encrypt messages, given as codes, into a one-section bank laid out as the games store
    /// them.
    fn encode(messages: &[Vec<u16>]) -> Vec<u8> {
        let entries_len = 4 + messages.len() * 8;
        let mut entries = Vec::new();
        let mut data = Vec::new();
        for (index, codes) in messages.iter().enumerate() {
            entries.extend(((entries_len + data.len()) as u32).to_le_bytes());
            entries.extend((codes.len() as u16).to_le_bytes());
            entries.extend(0u16.to_le_bytes());
            let mut key = 0x1234u16.wrapping_mul(index as u16 + 7);
            for &code in codes {
                data.extend((code ^ key).to_le_bytes());
                key = key.rotate_left(3);
            }
        }
        let section_len = (entries_len + data.len()) as u32;
        [
            &1u16.to_le_bytes()[..],
            &(messages.len() as u16).to_le_bytes(),
            &section_len.to_le_bytes(),
            &0u32.to_le_bytes(),
            &16u32.to_le_bytes(),
            &section_len.to_le_bytes(),
            &entries,
            &data,
        ]
        .concat()
    }

    fn codes(text: &str) -> impl Iterator<Item = u16> + '_ {
        text.encode_utf16()
    }

    #[test]
    fn decodes_encrypted_banks() {
        // "Hé", a line break, {STRVAR 1, 0}, a scroll, "♂" as the font's symbol.
        let first = codes("Hé")
            .chain([LINE_BREAK, COMMAND, 0x0101, 1, 0, COMMAND, SCROLL_LINE, 0])
            .chain([0x246D, END])
            .collect();
        let bank = encode(&[first, vec![0x0041, END, END], vec![]]);
        let messages = super::bank(&bank).unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].markup(), "Hé\n{STRVAR 1, 0}\r{MALE}");
        assert_eq!(messages[0].plain(), "Hé\n\n♂");
        assert_eq!(messages[1].plain(), "A");
        assert_eq!(messages[2], Message::default());
    }

    #[test]
    fn unpacks_9bit_codes() {
        // "Al", then the end code, packed 16 bits per word.
        let mut bits: u64 = 0x41 | 0x6C << 9 | 0x1FF << 18;
        let mut words = vec![PACKED];
        while bits != 0 {
            words.push(bits as u16);
            bits >>= 16;
        }
        words.push(END);
        let messages = super::bank(&encode(&[words])).unwrap();
        assert_eq!(messages[0].plain(), "Al");
    }

    #[test]
    fn rejects_truncated_messages() {
        assert!(super::bank(&encode(&[vec![COMMAND, 0x0100, END]])).is_err());
        assert!(super::bank(&encode(&[vec![COMMAND, 0x0100, 3, 0, END]])).is_err());
        let mut bank = encode(&[vec![0x41, END]]);
        bank[20..24].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes()); // the message's offset
        assert!(super::bank(&bank).is_err());
        assert!(super::bank(&bank[..10]).is_err());
    }
}
