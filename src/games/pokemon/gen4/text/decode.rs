//! Bank decoding. A bank starts with its message count and a seed, then one encrypted
//! `(offset, length)` entry per message; each message is a run of encrypted 16-bit codes.
//! Messages are numbered from 1 in the keys.

use std::borrow::Cow;
use std::sync::LazyLock;

use super::{Message, Token, charmap};
use crate::io::{ByteSource, Reader};
use crate::{Error, Result};

const FORMAT: &str = "Gen 4 text bank";

const ENTRY_KEY_FACTOR: u32 = 765;
const CODE_KEY_FACTOR: u32 = 596_947;
const CODE_KEY_STEP: u16 = 0x493D;

const END: u16 = 0xFFFF;
const COMMAND: u16 = 0xFFFE;
/// Starts a message stored as 9-bit codes (trainer names), which ends with `PACKED_END`.
const PACKED: u16 = 0xF100;
const PACKED_END: u16 = 0x01FF;
const LINE_BREAK: u16 = 0xE000;
const SCROLL_LINE: u16 = 0x25BC;
const CLEAR_BOX: u16 = 0x25BD;

/// Every message in a bank already in memory.
pub(super) fn bank(bytes: &[u8]) -> Result<Vec<Message>> {
    let (count, seed) = header(bytes)?;
    (0..count).map(|index| entry(bytes, seed, index)).collect()
}

/// One message, reading only the bank header, its entry, and its codes.
pub(super) fn message<S: ByteSource + ?Sized>(bank: &S, index: usize) -> Result<Option<Message>> {
    let (count, seed) = header(bank)?;
    (index < count)
        .then(|| entry(bank, seed, index))
        .transpose()
}

fn header<S: ByteSource + ?Sized>(bank: &S) -> Result<(usize, u32)> {
    let header = read(bank, 0, 4)?;
    let header = Reader::new(FORMAT, &header);
    Ok((header.u16_at(0)?.into(), header.u16_at(2)?.into()))
}

fn entry<S: ByteSource + ?Sized>(bank: &S, seed: u32, index: usize) -> Result<Message> {
    let number = index as u32 + 1;
    let key = ENTRY_KEY_FACTOR.wrapping_mul(number).wrapping_mul(seed) & 0xFFFF;
    let key = key | key << 16;
    let entry = read(bank, 4 + index as u64 * 8, 8)?;
    let entry = Reader::new(FORMAT, &entry);
    let offset = (entry.u32_at(0)? ^ key) as u64;
    let len = (entry.u32_at(4)? ^ key) as u64;

    let mut key = (CODE_KEY_FACTOR.wrapping_mul(number) & 0xFFFF) as u16;
    let codes = read(bank, offset, len * 2)?
        .as_chunks()
        .0
        .iter()
        .map(|&code| {
            let code = u16::from_le_bytes(code) ^ key;
            key = key.wrapping_add(CODE_KEY_STEP);
            code
        })
        .collect();
    tokenize(codes).map_err(|reason| Error::malformed(FORMAT, offset, reason))
}

fn read<S: ByteSource + ?Sized>(bank: &S, offset: u64, len: u64) -> Result<Cow<'_, [u8]>> {
    bank.read(offset, len)
        .map_err(|_| Error::malformed(FORMAT, offset, "entry or message exceeds the bank"))
}

fn tokenize(codes: Vec<u16>) -> Result<Message, &'static str> {
    let (codes, end) = match codes.first() {
        Some(&PACKED) => (unpack(&codes[1..]), PACKED_END),
        _ => (codes, END),
    };
    let mut tokens = Vec::new();
    let mut text = String::new();
    let mut rest = codes.as_slice();
    while let Some((&code, tail)) = rest.split_first() {
        rest = tail;
        let token = match code {
            _ if code == end => break,
            LINE_BREAK => Token::LineBreak,
            SCROLL_LINE => Token::ScrollLine,
            CLEAR_BOX => Token::ClearBox,
            COMMAND => {
                let [id, argc, tail @ ..] = rest else {
                    return Err("truncated command");
                };
                let args = tail
                    .get(..*argc as usize)
                    .ok_or("truncated command arguments")?;
                rest = &tail[args.len()..];
                Token::Command {
                    id: *id,
                    args: args.to_vec(),
                }
            }
            _ => match char_for(code) {
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

/// 9-bit codes packed into the low 15 bits of each word, least significant first.
fn unpack(words: &[u16]) -> Vec<u16> {
    let mut codes = Vec::with_capacity(words.len() * 15 / 9);
    let (mut buffer, mut bits) = (0u32, 0);
    for &word in words {
        buffer |= ((word & 0x7FFF) as u32) << bits;
        bits += 15;
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

/// Code → character, expanded once from the charmap runs; `'\0'` marks unassigned codes.
static CHARS: LazyLock<Vec<char>> = LazyLock::new(|| {
    let mut chars = Vec::new();
    for &(start, run) in charmap::RUNS {
        chars.resize(start as usize, '\0');
        chars.extend(run.chars());
    }
    chars
});

fn char_for(code: u16) -> Option<char> {
    CHARS.get(code as usize).copied().filter(|&ch| ch != '\0')
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// Encrypt messages, given as codes, into a bank laid out as the games store them.
    pub fn encode(seed: u16, messages: &[Vec<u16>]) -> Vec<u8> {
        let mut entries = Vec::new();
        let mut data = Vec::new();
        let data_start = 4 + messages.len() * 8;
        for (index, codes) in messages.iter().enumerate() {
            let number = index as u32 + 1;
            let key = ENTRY_KEY_FACTOR
                .wrapping_mul(number)
                .wrapping_mul(seed.into())
                & 0xFFFF;
            let key = key | key << 16;
            entries.extend(((data_start + data.len()) as u32 ^ key).to_le_bytes());
            entries.extend((codes.len() as u32 ^ key).to_le_bytes());
            let mut key = (CODE_KEY_FACTOR.wrapping_mul(number) & 0xFFFF) as u16;
            for &code in codes {
                data.extend((code ^ key).to_le_bytes());
                key = key.wrapping_add(CODE_KEY_STEP);
            }
        }
        [(messages.len() as u16).to_le_bytes(), seed.to_le_bytes()]
            .concat()
            .into_iter()
            .chain(entries)
            .chain(data)
            .collect()
    }

    #[test]
    fn charmap_maps_known_codes() {
        assert_eq!(char_for(0x012B), Some('A'));
        assert_eq!(char_for(0x0121), Some('0'));
        assert_eq!(char_for(0x01DE), Some(' '));
        assert_eq!(char_for(0x0000), None);
        assert_eq!(char_for(0x01F0), None);
        assert_eq!(char_for(0xFFF0), None);
    }

    #[test]
    fn unpacks_9bit_codes() {
        // 'A', 'B', then the end code, packed 15 bits per word.
        let mut bits: u64 = 0x12B | 0x12C << 9 | 0x1FF << 18;
        let mut words = Vec::new();
        while bits != 0 {
            words.push((bits & 0x7FFF) as u16);
            bits >>= 15;
        }
        assert_eq!(unpack(&words), [0x12B, 0x12C, PACKED_END]);
    }

    #[test]
    fn decodes_encrypted_banks() {
        // "Hi", a line break, {STRVAR_1 1, 0}, "!".
        let first = vec![
            0x0132, 0x014D, LINE_BREAK, COMMAND, 0x0101, 1, 0, 0x01AB, END,
        ];
        // A large seed exercises the 32-bit overflow in the key arithmetic.
        let bank = encode(0xFFF1, &[first, vec![0x012B, END]]);
        let messages = super::bank(bank.as_slice()).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].markup(), "Hi\n{STRVAR_1 1, 0}!");
        assert_eq!(messages[1].plain(), "A");
        assert_eq!(
            message(bank.as_slice(), 1).unwrap().as_ref(),
            Some(&messages[1])
        );
        assert_eq!(message(bank.as_slice(), 2).unwrap(), None);
    }

    #[test]
    fn rejects_truncated_commands() {
        let bank = encode(7, &[vec![COMMAND, 0x0200]]);
        assert!(super::bank(bank.as_slice()).is_err());
        let bank = encode(7, &[vec![COMMAND, 0x0200, 2, 0]]);
        assert!(super::bank(bank.as_slice()).is_err());
    }
}
