//! Gen 5 text: banks of encrypted UTF-16 messages, one bank per member of a text archive. The
//! games have two archives: system text (names, descriptions, menus) and story text (dialogue).

mod decode;

use std::fmt::Write as _;

use serde::Serialize;

use crate::Result;
use crate::formats::narc::Narc;
use crate::io::ByteSource;

/// A text archive. Banks are decoded when asked for.
pub struct Text<'a, A> {
    archive: A,
    narc: &'a Narc,
}

impl<'a, A: ByteSource> Text<'a, A> {
    /// `narc` is the member table of `archive`.
    pub fn new(archive: A, narc: &'a Narc) -> Self {
        Self { archive, narc }
    }

    /// The number of banks.
    pub fn len(&self) -> usize {
        self.narc.len()
    }

    pub fn is_empty(&self) -> bool {
        self.narc.is_empty()
    }

    /// Every message in a bank.
    pub fn bank(&self, bank: usize) -> Result<Vec<Message>> {
        decode::bank(&self.narc.member(&self.archive, bank)?)
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
    /// One of the font's own symbols, which the games store where Unicode has enclosed
    /// alphanumerics (`①`...). Names use these for ♂ and ♀, not the Unicode signs.
    Symbol(u16),
    /// A code that is not a character, kept so decoding loses nothing.
    Unknown(u16),
}

/// The symbols whose meaning is known, with their text and markup names.
const SYMBOLS: [(u16, &str, &str); 4] = [
    (0x246D, "♂", "MALE"),
    (0x246E, "♀", "FEMALE"),
    (0x2486, "PK", "PK"),
    (0x2487, "MN", "MN"),
];

fn symbol(code: u16) -> Option<(&'static str, &'static str)> {
    SYMBOLS
        .iter()
        .find(|(known, _, _)| *known == code)
        .map(|&(_, text, name)| (text, name))
}

impl Message {
    /// The text alone: breaks become `\n`, known symbols become text, and commands are dropped.
    pub fn plain(&self) -> String {
        let mut out = String::new();
        for token in &self.tokens {
            match token {
                Token::Text(text) => out.push_str(text),
                Token::LineBreak | Token::ScrollLine | Token::ClearBox => out.push('\n'),
                Token::Symbol(code) => out.push_str(symbol(*code).map_or("", |(text, _)| text)),
                Token::Command { .. } | Token::Unknown(_) => {}
            }
        }
        out
    }

    /// A lossless single-string form, in the same syntax as Gen 4 text: `\n` breaks a line,
    /// `\r` scrolls, `\f` clears, commands are `{NAME args}`, symbols are `{MALE}` or
    /// `{SYMBOL 0xNNNN}`, and other codes are `{UNK_CHAR 0xNNNN}`. A literal `\` or `{` is escaped
    /// with a backslash.
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
                Token::Symbol(code) => {
                    _ = match symbol(*code) {
                        Some((_, name)) => write!(out, "{{{name}}}"),
                        None => write!(out, "{{SYMBOL {code:#06X}}}"),
                    }
                }
                Token::Unknown(code) => _ = write!(out, "{{UNK_CHAR {code:#06X}}}"),
            }
        }
        out
    }
}

/// `{NAME arg, arg}`. Commands whose high byte is 1 insert a string (a name, an item...) and
/// those whose high byte is 2 a number; they are `{STRVAR kind, args}` and `{NUMVAR kind, args}`.
fn write_command(out: &mut String, id: u16, args: &[u16]) {
    let [group, kind] = id.to_be_bytes();
    let (name, kind) = match (id, group) {
        (0xFF00, _) => ("COLOR".to_owned(), None),
        (_, 0x01) => ("STRVAR".to_owned(), Some(kind.into())),
        (_, 0x02) => ("NUMVAR".to_owned(), Some(kind.into())),
        _ => (format!("CMD_{id:04X}"), None),
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
                    id: 0xFF00,
                    args: vec![2],
                },
                Token::Command {
                    id: 0xBD01,
                    args: vec![],
                },
                Token::Symbol(0x246D),
                Token::Symbol(0x2462),
                Token::ScrollLine,
                Token::ClearBox,
                Token::Unknown(0xD800),
            ],
        };
        assert_eq!(
            message.markup(),
            "Hi \\{there}\n{STRVAR 1, 0}{COLOR 2}{CMD_BD01}{MALE}{SYMBOL 0x2462}\r\x0C{UNK_CHAR 0xD800}"
        );
        assert_eq!(message.plain(), "Hi {there}\n♂\n\n");
    }
}
