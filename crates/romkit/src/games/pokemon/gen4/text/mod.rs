//! Gen 4 text: banks of encrypted messages, one bank per member of the game's text archive.

mod charmap;
mod decode;

use std::fmt::Write as _;

use serde::Serialize;

use crate::Result;
use crate::formats::narc::Narc;
use crate::io::ByteSource;

/// Descriptive bank names for Platinum, one per line in bank order, from pret's `pokeplatinum`
/// decompilation (`unk_*` banks are not identified yet).
pub(super) const PLATINUM_BANKS: &str = include_str!("platinum_banks.txt");

/// A game's text archive. Banks and messages are decoded when asked for.
pub struct Text<'a, A> {
    archive: A,
    narc: &'a Narc,
    names: &'static str,
}

impl<'a, A: ByteSource> Text<'a, A> {
    /// `narc` is the member table of `archive`; `names` holds one bank name per line.
    pub fn new(archive: A, narc: &'a Narc, names: &'static str) -> Self {
        Self {
            archive,
            narc,
            names,
        }
    }

    /// The number of banks.
    pub fn len(&self) -> usize {
        self.narc.len()
    }

    pub fn is_empty(&self) -> bool {
        self.narc.is_empty()
    }

    /// The bank's descriptive name, e.g. `species_name`, where one is known.
    pub fn name(&self, bank: usize) -> Option<&'static str> {
        self.names.lines().nth(bank)
    }

    /// The bank with a descriptive name.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.names.lines().position(|known| known == name)
    }

    /// Every message in a bank.
    pub fn bank(&self, bank: usize) -> Result<Vec<Message>> {
        decode::bank(&self.narc.member(&self.archive, bank)?)
    }

    /// One message, reading only its own bytes; `None` past the end of the bank.
    pub fn message(&self, bank: usize, index: usize) -> Result<Option<Message>> {
        decode::message(&self.narc.member_source(&self.archive, bank)?, index)
    }

    /// The plain text of every message in a bank, e.g. a list of names.
    pub fn strings(&self, bank: usize) -> Result<Vec<String>> {
        Ok(self.bank(bank)?.iter().map(Message::plain).collect())
    }
}

/// A decoded message: text interleaved with layout and control tokens.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Message {
    pub tokens: Vec<Token>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Token {
    Text(String),
    LineBreak,
    /// Wait for input, then scroll the text box up a line.
    ScrollLine,
    /// Wait for input, then clear the text box.
    ClearBox,
    /// A control sequence: a formatting change, or a placeholder filled in at runtime.
    Command {
        id: u16,
        args: Vec<u16>,
    },
    /// A character code with no known glyph, kept so decoding loses nothing.
    Unknown(u16),
}

/// Command IDs whose high byte selects a runtime string (a name, a number, ...) to insert.
const STRING_VAR_GROUPS: [u8; 5] = [0x01, 0x03, 0x04, 0x06, 0x34];

fn command_name(id: u16) -> Option<&'static str> {
    Some(match id {
        0x0200 => "YESNO",
        0x0201 => "PAUSE",
        0x0202 => "WAIT",
        0x0203 => "CURSOR_X",
        0x0204 => "CURSOR_Y",
        0x0205 => "ALN_CENTER",
        0x0206 => "ALN_RIGHT",
        0xFF00 => "COLOR",
        0xFF01 => "SIZE",
        _ => return None,
    })
}

impl Message {
    /// The text alone: breaks become `\n` and commands are dropped.
    pub fn plain(&self) -> String {
        let mut out = String::new();
        for token in &self.tokens {
            match token {
                Token::Text(text) => out.push_str(text),
                Token::LineBreak | Token::ScrollLine | Token::ClearBox => out.push('\n'),
                Token::Command { .. } | Token::Unknown(_) => {}
            }
        }
        out
    }

    /// A lossless single-string form, following the text syntax of pret's decompilations: `\n`
    /// breaks a line, `\r` scrolls, `\f` clears, commands are `{NAME args}`, and unknown codes
    /// are `{UNK_CHAR 0xNNNN}`. A literal `\` or `{` is escaped with a backslash.
    pub fn markup(&self) -> String {
        let mut out = String::new();
        for token in &self.tokens {
            match token {
                Token::Text(text) => {
                    for ch in text.chars() {
                        if matches!(ch, '\\' | '{') {
                            out.push('\\');
                        }
                        out.push(ch);
                    }
                }
                Token::LineBreak => out.push('\n'),
                Token::ScrollLine => out.push('\r'),
                Token::ClearBox => out.push('\x0C'),
                Token::Command { id, args } => write_command(&mut out, *id, args),
                Token::Unknown(code) => _ = write!(out, "{{UNK_CHAR {code:#06X}}}"),
            }
        }
        out
    }
}

/// `{NAME arg, arg}`. String variables are `{STRVAR_<group> <kind>, <args>}`, the kind being
/// what the inserted string holds.
fn write_command(out: &mut String, id: u16, args: &[u16]) {
    let [group, kind] = id.to_be_bytes();
    let (name, kind) = match command_name(id) {
        Some(name) => (name.to_owned(), None),
        None if STRING_VAR_GROUPS.contains(&group) => {
            (format!("STRVAR_{group:X}"), Some(kind.into()))
        }
        None => (format!("CMD_{id:04X}"), None),
    };
    out.push('{');
    out.push_str(&name);
    for (index, value) in kind.into_iter().chain(args.iter().copied()).enumerate() {
        out.push_str(if index == 0 { " " } else { ", " });
        _ = write!(out, "{value}");
    }
    out.push('}');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_is_lossless_and_plain_is_text_only() {
        let message = Message {
            tokens: vec![
                Token::Text("Hi {there}".into()),
                Token::LineBreak,
                Token::Command {
                    id: 0x0101,
                    args: vec![0],
                },
                Token::Command {
                    id: 0x0200,
                    args: vec![1, 2],
                },
                Token::Command {
                    id: 0x7777,
                    args: vec![],
                },
                Token::ScrollLine,
                Token::Unknown(0x0DEF),
            ],
        };
        assert_eq!(
            message.markup(),
            "Hi \\{there}\n{STRVAR_1 1, 0}{YESNO 1, 2}{CMD_7777}\r{UNK_CHAR 0x0DEF}"
        );
        assert_eq!(message.plain(), "Hi {there}\n\n");
    }
}
