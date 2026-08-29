//! Gen 3 text: one byte per character in the games' own encoding, ended by `0xFF`. Names are
//! fixed-width fields padded with terminators; descriptions are strings found by pointer.
//!
//! Strings decode to text with the syntax of the Gen 4 and Gen 5 markup: `\n` breaks a line,
//! `\r` scrolls, `\f` clears the box, and what is not a character is in braces: the font's own
//! symbols (`{PK}{MN}`), placeholders (`{PLAYER}`), control codes with their arguments
//! (`{COLOR 2}`), and unassigned codes (`{UNK_CHAR 0x7F}`). The encoding has no `{` or `\`, so
//! braces are never ambiguous.

use std::fmt::Write as _;
use std::sync::LazyLock;

/// The English character map, as runs of consecutive codes, from pret's `pokeemerald`
/// decompilation. Codes it leaves unassigned are absent.
const RUNS: &[(u8, &str)] = &[
    (0x00, " ÀÁÂÇÈÉÊËÌ"),
    (0x0B, "ÎÏÒÓÔŒÙÚÛÑßàá"),
    (0x19, "çèéêëì"),
    (0x20, "îïòóôœùúûñºª"),
    (0x2D, "&+"),
    (0x35, "=;"),
    (0x51, "¿¡"),
    (0x5A, "Í%()"),
    (0x68, "â"),
    (0x6F, "í"),
    (0x79, "↑↓←→"),
    (0x85, "<>"),
    (0xA1, "0123456789!?.-·…“”‘’♂♀¥,×/ABCDEFGHIJKLMNOPQRSTUVWXYZ"),
    (0xD5, "abcdefghijklmnopqrstuvwxyz▶:ÄÖÜäöü"),
];

/// Symbols of the font that Unicode lacks.
const SYMBOLS: [(u8, &str); 6] = [
    (0x2C, "SUPER_ER"),
    (0x34, "LV"),
    (0x53, "PK"),
    (0x54, "MN"),
    (0x84, "SUPER_E"),
    (0xA0, "SUPER_RE"),
];
/// `POKéBLOCK` is drawn by five codes together.
const POKEBLOCK: [u8; 5] = [0x55, 0x56, 0x57, 0x58, 0x59];

/// Followed by one byte: a character drawn at runtime, a button icon, a symbol of the
/// extra set.
const DYNAMIC: u8 = 0xF7;
const KEYPAD_ICON: u8 = 0xF8;
const EXTRA_SYMBOL: u8 = 0xF9;
const SCROLL_LINE: u8 = 0xFA;
const CLEAR_BOX: u8 = 0xFB;
/// Followed by a control code and its arguments.
const CONTROL: u8 = 0xFC;
/// Followed by the placeholder's number.
const PLACEHOLDER: u8 = 0xFD;
const LINE_BREAK: u8 = 0xFE;
pub(super) const END: u8 = 0xFF;

/// Control codes by number, with the count of argument bytes each takes.
const CONTROLS: [(&str, usize); 25] = [
    ("CTRL_00", 0),
    ("COLOR", 1),
    ("HIGHLIGHT", 1),
    ("SHADOW", 1),
    ("COLOR_HIGHLIGHT_SHADOW", 3),
    ("PALETTE", 1),
    ("FONT", 1),
    ("RESET_FONT", 0),
    ("PAUSE", 1),
    ("PAUSE_UNTIL_PRESS", 0),
    ("WAIT_SE", 0),
    ("PLAY_BGM", 2),
    ("ESCAPE", 1),
    ("SHIFT_RIGHT", 1),
    ("SHIFT_DOWN", 1),
    ("FILL_WINDOW", 0),
    ("PLAY_SE", 2),
    ("CLEAR", 1),
    ("SKIP", 1),
    ("CLEAR_TO", 1),
    ("MIN_LETTER_SPACING", 1),
    ("JPN", 0),
    ("ENG", 0),
    ("PAUSE_MUSIC", 0),
    ("RESUME_MUSIC", 0),
];

const PLACEHOLDERS: [&str; 6] = [
    "PLACEHOLDER_00",
    "PLAYER",
    "STR_VAR_1",
    "STR_VAR_2",
    "STR_VAR_3",
    "KUN",
];

/// Code → character, expanded once from the runs; `'\0'` marks unassigned codes.
static CHARS: LazyLock<[char; 256]> = LazyLock::new(|| {
    let mut chars = ['\0'; 256];
    for &(start, run) in RUNS {
        for (offset, ch) in run.chars().enumerate() {
            chars[start as usize + offset] = ch;
        }
    }
    chars
});

/// The text of `bytes` up to the first terminator, or all of them when there is none.
pub fn decode(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut rest = bytes;
    while let Some((&code, tail)) = rest.split_first() {
        rest = tail;
        match code {
            END => break,
            LINE_BREAK => out.push('\n'),
            SCROLL_LINE => out.push('\r'),
            CLEAR_BOX => out.push('\x0C'),
            _ if code == POKEBLOCK[0] && rest.starts_with(&POKEBLOCK[1..]) => {
                out.push_str("{POKEBLOCK}");
                rest = &rest[4..];
            }
            DYNAMIC | KEYPAD_ICON | EXTRA_SYMBOL | CONTROL | PLACEHOLDER => {
                let Some((&number, tail)) = rest.split_first() else {
                    _ = write!(out, "{{UNK_CHAR {code:#04X}}}");
                    break;
                };
                rest = tail;
                let (name, args) = match code {
                    DYNAMIC => ("DYNAMIC".to_owned(), vec![number]),
                    KEYPAD_ICON => ("KEYPAD_ICON".to_owned(), vec![number]),
                    EXTRA_SYMBOL => ("EXTRA_SYMBOL".to_owned(), vec![number]),
                    PLACEHOLDER => match PLACEHOLDERS.get(number as usize) {
                        Some(name) => ((*name).to_owned(), vec![]),
                        None => ("PLACEHOLDER".to_owned(), vec![number]),
                    },
                    _ => match CONTROLS.get(number as usize) {
                        Some(&(name, count)) => {
                            let count = count.min(rest.len());
                            let args = rest[..count].to_vec();
                            rest = &rest[count..];
                            (name.to_owned(), args)
                        }
                        None => (format!("CTRL_{number:02X}"), vec![]),
                    },
                };
                out.push('{');
                out.push_str(&name);
                for (index, arg) in args.iter().enumerate() {
                    out.push_str(if index == 0 { " " } else { ", " });
                    _ = write!(out, "{arg}");
                }
                out.push('}');
            }
            _ => match CHARS[code as usize] {
                '\0' => match SYMBOLS.iter().find(|(known, _)| *known == code) {
                    Some((_, name)) => _ = write!(out, "{{{name}}}"),
                    None => _ = write!(out, "{{UNK_CHAR {code:#04X}}}"),
                },
                ch => out.push(ch),
            },
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_characters_symbols_and_codes() {
        // "PIKACHU" padded with terminators, as in the species name table.
        let name = [0xCA, 0xC3, 0xC5, 0xBB, 0xBD, 0xC2, 0xCF, END, END, END, END];
        assert_eq!(decode(&name), "PIKACHU");
        assert_eq!(
            decode(&[0xC8, 0xC3, 0xBE, 0xC9, 0xCC, 0xBB, 0xC8, 0xB5]),
            "NIDORAN♂"
        );
        assert_eq!(
            decode(&[0xCA, 0xC9, 0xC5, 0x1B, 0xC7, 0xC9, 0xC8]),
            "POKéMON"
        );
        assert_eq!(
            decode(&[0x53, 0x54, LINE_BREAK, 0xA2, 0xAB, SCROLL_LINE, CLEAR_BOX]),
            "{PK}{MN}\n1!\r\x0C"
        );
        assert_eq!(
            decode(&[PLACEHOLDER, 1, CONTROL, 1, 2, CONTROL, 0x0B, 3, 4, 0x7F]),
            "{PLAYER}{COLOR 2}{PLAY_BGM 3, 4}{UNK_CHAR 0x7F}"
        );
        assert_eq!(
            decode(&[0x55, 0x56, 0x57, 0x58, 0x59, 0xE7]),
            "{POKEBLOCK}s"
        );
        // A code cut short by the end of the bytes is kept, not a panic.
        assert_eq!(decode(&[CONTROL, 1]), "{COLOR}");
        assert_eq!(decode(&[PLACEHOLDER]), "{UNK_CHAR 0xFD}");
    }
}
