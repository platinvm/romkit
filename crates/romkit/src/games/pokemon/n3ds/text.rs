//! Gen 6 and 7 text: one GARC per language and set of text, one bank per entry, each bank a
//! list of encrypted UTF-16 lines.
//!
//! A bank starts with a header: a section count (always 1), the line count, the total length,
//! an initial key (always 0), and where the section starts. The section holds its length, then
//! an `(offset, length, flags)` entry per line, offsets counting from the section and lengths in
//! 16-bit units. A line's units are XORed with a key that starts at 0x7C89 for the first line
//! and grows by 0x2983 per line, rotating left by 3 bits after each unit. Unit 0 ends a line and
//! 0x0010 starts a variable: a count, the variable's ID, then count − 1 arguments.

use std::fmt::Write as _;

use serde::Serialize;

use crate::Result;
use crate::formats::garc::Garc;
use crate::io::{ByteSource, Reader};

const FORMAT: &str = "Gen 6/7 text bank";

const FIRST_KEY: u16 = 0x7C89;
const KEY_STEP: u16 = 0x2983;
const END: u16 = 0x0000;
const VARIABLE: u16 = 0x0010;
const SCROLL_LINE: u16 = 0xBE00;
const CLEAR_BOX: u16 = 0xBE01;

/// A text archive. Banks are decoded when asked for.
pub struct Text<'a, A> {
    archive: A,
    garc: &'a Garc,
}

impl<'a, A: ByteSource> Text<'a, A> {
    /// `garc` is the table of `archive`.
    pub fn new(archive: A, garc: &'a Garc) -> Self {
        Self { archive, garc }
    }

    /// The number of banks.
    pub fn len(&self) -> usize {
        self.garc.len()
    }

    pub fn is_empty(&self) -> bool {
        self.garc.is_empty()
    }

    /// Every message in a bank.
    pub fn bank(&self, bank: usize) -> Result<Vec<Message>> {
        decode(&self.garc.member(&self.archive, bank)?)
    }

    /// The plain text of every message in a bank, e.g. a list of names.
    pub fn strings(&self, bank: usize) -> Result<Vec<String>> {
        Ok(self.bank(bank)?.iter().map(Message::plain).collect())
    }
}

/// A decoded line: text interleaved with variables.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Message {
    pub tokens: Vec<Token>,
    /// Per-line flags stored beside the line; their meaning is unknown.
    pub flags: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Token {
    /// Text, including line breaks (`\n`).
    Text(String),
    /// Wait for input, then scroll the text box up a line.
    ScrollLine,
    /// Wait for input, then clear the text box.
    ClearBox,
    /// A formatting change or a placeholder filled in at runtime, such as a Pokémon's name.
    Variable { id: u16, args: Vec<u16> },
    /// A lone UTF-16 surrogate, kept so decoding loses nothing.
    Unknown(u16),
}

fn variable_name(id: u16) -> Option<&'static str> {
    Some(match id {
        0xBE02 => "WAIT",
        0xFF00 => "COLOR",
        _ => return None,
    })
}

/// Western text stores ♂ and ♀ in the private use area, where the games' font has them.
fn display_char(ch: char) -> char {
    match ch {
        '\u{E08E}' => '♂',
        '\u{E08F}' => '♀',
        other => other,
    }
}

impl Message {
    /// The text alone: scrolls and clears become `\n`, variables are dropped, and the
    /// private-use ♂ and ♀ become the standard characters.
    pub fn plain(&self) -> String {
        let mut out = String::new();
        for token in &self.tokens {
            match token {
                Token::Text(text) => out.extend(text.chars().map(display_char)),
                Token::ScrollLine | Token::ClearBox => out.push('\n'),
                Token::Variable { .. } | Token::Unknown(_) => {}
            }
        }
        out
    }

    /// A lossless single-string form in the syntax of the Gen 4 text: `\r` scrolls, `\f`
    /// clears, variables are `{NAME args}` (`{VAR_XXXX args}` when unnamed), and lone
    /// surrogates are `{UNK_CHAR 0xNNNN}`. A literal `\`, `{`, `\r`, or `\f` is escaped with a
    /// backslash.
    pub fn markup(&self) -> String {
        let mut out = String::new();
        for token in &self.tokens {
            match token {
                Token::Text(text) => {
                    for ch in text.chars() {
                        if matches!(ch, '\\' | '{' | '\r' | '\x0C') {
                            out.push('\\');
                        }
                        out.push(ch);
                    }
                }
                Token::ScrollLine => out.push('\r'),
                Token::ClearBox => out.push('\x0C'),
                Token::Variable { id, args } => {
                    match variable_name(*id) {
                        Some(name) => _ = write!(out, "{{{name}"),
                        None => _ = write!(out, "{{VAR_{id:04X}"),
                    }
                    for (index, arg) in args.iter().enumerate() {
                        out.push_str(if index == 0 { " " } else { ", " });
                        _ = write!(out, "{arg}");
                    }
                    out.push('}');
                }
                Token::Unknown(unit) => _ = write!(out, "{{UNK_CHAR {unit:#06X}}}"),
            }
        }
        out
    }
}

/// Every line of a bank.
pub(super) fn decode(bytes: &[u8]) -> Result<Vec<Message>> {
    let bank = Reader::new(FORMAT, bytes);
    if bank.u16_at(0)? != 1 || bank.u32_at(8)? != 0 {
        return Err(bank.error_at(0, "not a single-section bank"));
    }
    let section = bank.u32_at(0x0C)? as usize;
    let mut key = FIRST_KEY;
    (0..bank.u16_at(2)? as usize)
        .map(|line| {
            let entry = section + 4 + line * 8;
            let offset = section + bank.u32_at(entry)? as usize;
            let units = bank.slice_at(offset, bank.u16_at(entry + 4)? as usize * 2)?;
            let message = tokenize(units, key).map_err(|reason| bank.error_at(offset, reason))?;
            key = key.wrapping_add(KEY_STEP);
            Ok(Message {
                flags: bank.u16_at(entry + 6)?,
                ..message
            })
        })
        .collect()
}

/// Decrypt a line's units with `key` and split them into tokens.
fn tokenize(bytes: &[u8], mut key: u16) -> Result<Message, &'static str> {
    let units: Vec<u16> = bytes
        .as_chunks()
        .0
        .iter()
        .map(|&unit| {
            let unit = u16::from_le_bytes(unit) ^ key;
            key = key.rotate_left(3);
            unit
        })
        .collect();
    let mut tokens = Vec::new();
    let mut text = String::new();
    let mut rest = units.as_slice();
    while let Some((&unit, tail)) = rest.split_first() {
        rest = tail;
        let token = match unit {
            END => break,
            VARIABLE => {
                let [count, id, tail @ ..] = rest else {
                    return Err("truncated variable");
                };
                let args = count
                    .checked_sub(1)
                    .and_then(|len| tail.get(..len as usize))
                    .ok_or("truncated variable arguments")?;
                rest = &tail[args.len()..];
                match (*id, args) {
                    (SCROLL_LINE, []) => Token::ScrollLine,
                    (CLEAR_BOX, []) => Token::ClearBox,
                    _ => Token::Variable {
                        id: *id,
                        args: args.to_vec(),
                    },
                }
            }
            _ => {
                // A surrogate pair is two units; a lone surrogate has no character.
                let pair = [unit, rest.first().copied().unwrap_or(0)];
                match char::decode_utf16(pair).next() {
                    Some(Ok(ch)) => {
                        rest = &rest[ch.len_utf16() - 1..];
                        text.push(ch);
                        continue;
                    }
                    _ => Token::Unknown(unit),
                }
            }
        };
        if !text.is_empty() {
            tokens.push(Token::Text(std::mem::take(&mut text)));
        }
        tokens.push(token);
    }
    if !text.is_empty() {
        tokens.push(Token::Text(text));
    }
    Ok(Message { tokens, flags: 0 })
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// Encrypt lines, given as units, into a bank laid out as the games store them.
    pub fn encode(lines: &[(&[u16], u16)]) -> Vec<u8> {
        let mut entries = Vec::new();
        let mut data = Vec::new();
        let data_start = 4 + lines.len() * 8;
        let mut key = FIRST_KEY;
        for (units, flags) in lines {
            entries.extend(((data_start + data.len()) as u32).to_le_bytes());
            entries.extend((units.len() as u16).to_le_bytes());
            entries.extend(flags.to_le_bytes());
            let mut unit_key = key;
            for unit in *units {
                data.extend((unit ^ unit_key).to_le_bytes());
                unit_key = unit_key.rotate_left(3);
            }
            key = key.wrapping_add(KEY_STEP);
        }
        let section_len = (data_start + data.len()) as u32;
        let mut bank = Vec::new();
        bank.extend(1u16.to_le_bytes());
        bank.extend((lines.len() as u16).to_le_bytes());
        bank.extend(section_len.to_le_bytes());
        bank.extend(0u32.to_le_bytes());
        bank.extend(0x10u32.to_le_bytes());
        bank.extend(section_len.to_le_bytes());
        bank.extend(entries);
        bank.extend(data);
        bank
    }

    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn decodes_lines_and_variables() {
        let mut first = units("Hi {x}\n");
        first.extend([VARIABLE, 2, 0x0101, 0, VARIABLE, 1, CLEAR_BOX]);
        first.extend(units("Nidoran\u{E08F}🎵"));
        first.extend([0xD800, END, 0x41]);
        let bank = encode(&[(&first, 0), (&units("Pikachu"), 216)]);
        let messages = decode(&bank).unwrap();
        assert_eq!(
            messages[0].markup(),
            "Hi \\{x}\n{VAR_0101 0}\x0CNidoran\u{E08F}🎵{UNK_CHAR 0xD800}"
        );
        assert_eq!(messages[0].plain(), "Hi {x}\n\nNidoran♀🎵");
        assert_eq!(
            (messages[1].plain().as_str(), messages[1].flags),
            ("Pikachu", 216)
        );
    }

    #[test]
    fn rejects_truncated_variables_and_lines() {
        assert!(decode(&encode(&[(&[VARIABLE, 3, 0xFF00, 1], 0)])).is_err());
        assert!(decode(&encode(&[(&[VARIABLE], 0)])).is_err());
        let mut bank = encode(&[(&units("Pikachu"), 0)]);
        bank.truncate(bank.len() - 2);
        assert!(decode(&bank).is_err());
    }
}
