//! Game Boy Pokémon text: one byte per character, ended by `0x50`.
//!
//! A [`Charmap`] maps bytes to Unicode as runs of consecutive codes, plus the codes that stand
//! for words or line breaks. Codes that name a value filled in at run time (the player's name,
//! the user of a move) become markup such as `<PLAYER>`, after pret's charmaps; codes without a
//! meaning become `<$xx>`. No character the games can print is `<`, so the markup is
//! unambiguous.

/// Ends every string and pads fixed-length names.
pub const END: u8 = 0x50;

/// How one game language encodes its text.
#[derive(Debug)]
pub struct Charmap {
    /// `(first code, its character and those of the codes after it)`.
    pub(crate) runs: &'static [(u8, &'static str)],
    /// Codes that print more than one character, or a line break.
    pub(crate) words: &'static [(u8, &'static str)],
    /// Japanese text borrows hiragana へ, べ, ぺ, and り for their katakana twins, which look
    /// the same on screen.
    pub(crate) borrows_kana: bool,
}

/// Control codes common to every language, after pret's names. Markup by default.
const CONTROL: &[(u8, &str)] = &[
    (0x49, "<PAGE>"),
    (0x4B, "<_CONT>"),
    (0x4C, "<SCROLL>"),
    (0x4E, "\n"),
    (0x4F, "\n"),
    (0x51, "\n\n"),
    (0x52, "<PLAYER>"),
    (0x53, "<RIVAL>"),
    (0x55, "\n"),
    (0x57, "<DONE>"),
    (0x58, "<PROMPT>"),
    (0x59, "<TARGET>"),
    (0x5A, "<USER>"),
    (0x5F, "<DEXEND>"),
];

/// Red, Blue, and Yellow in English, from pret's `pokered` charmap. 0x60–0x6C are bold
/// letters for the battle screen.
pub const ENGLISH: Charmap = Charmap {
    runs: &[
        (0x60, "ABCDEFGHIVSLM:ぃぅ‘’“”·…ぁぇぉ┌─┐│└┘ "),
        (
            0x80,
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ():;[]abcdefghijklmnopqrstuvwxyzé",
        ),
        (0xE0, "'"),
        (0xE3, "-"),
        (0xE6, "?!.ァゥェ▷▶▼♂¥×./,♀0123456789"),
    ],
    words: &[
        (0x4A, "PKMN"),
        (0x54, "POKé"),
        (0x56, "……"),
        (0x5B, "PC"),
        (0x5C, "TM"),
        (0x5D, "TRAINER"),
        (0x5E, "ROCKET"),
        (0xBB, "'d"),
        (0xBC, "'l"),
        (0xBD, "'s"),
        (0xBE, "'t"),
        (0xBF, "'v"),
        (0xE1, "PK"),
        (0xE2, "MN"),
        (0xE4, "'r"),
        (0xE5, "'m"),
    ],
    borrows_kana: false,
};

/// Red and Green in Japanese. Katakana take 0x80–0xB0 and hiragana 0xB1–0xE2; the voiced
/// kana each have a code of their own below 0x50.
pub const JAPANESE: Charmap = Charmap {
    runs: &[
        (0x05, "ガギグゲゴザジズゼゾダヂヅデド"),
        (0x19, "バビブボ"),
        (0x26, "がぎぐげござじずぜぞだぢづでど"),
        (0x3A, "ばびぶべぼ"),
        (0x40, "パピプポぱぴぷぺぽ"),
        (
            0x60,
            "ＡＢＣＤＥＦＧＨＩＶＳＬＭ：ぃぅ「」『』・⋯ぁぇぉ┌─┐│└┘　",
        ),
        (
            0x80,
            "アイウエオカキクケコサシスセソタチツテトナニヌネノハヒフホマミムメモヤユヨラルレロワヲンッャュョィ",
        ),
        (
            0xB1,
            "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわをんっゃゅょー゜゛？！。ァゥェ▷▶▼♂円×．／ォ♀０１２３４５６７８９",
        ),
    ],
    words: &[(0x4A, "ポケモン"), (0x54, "ポケモン"), (0x56, "⋯⋯")],
    borrows_kana: true,
};

impl Charmap {
    /// The text in `bytes`, up to the first `0x50` or the end.
    pub fn decode(&self, bytes: &[u8]) -> String {
        let end = bytes.iter().position(|&byte| byte == END);
        let mut text: Vec<char> = Vec::new();
        for &byte in &bytes[..end.unwrap_or(bytes.len())] {
            match self.character(byte) {
                Some(character) => text.push(character),
                None => text.extend(self.word(byte).chars()),
            }
        }
        if self.borrows_kana {
            restore_katakana(&mut text);
        }
        text.into_iter().collect()
    }

    fn character(&self, byte: u8) -> Option<char> {
        self.runs.iter().find_map(|&(start, run)| {
            byte.checked_sub(start)
                .and_then(|index| run.chars().nth(index.into()))
        })
    }

    fn word(&self, byte: u8) -> String {
        self.words
            .iter()
            .chain(CONTROL)
            .find(|&&(code, _)| code == byte)
            .map_or_else(|| format!("<${byte:02x}>"), |(_, word)| (*word).to_owned())
    }

    /// Strings of `len` bytes each, padded with `0x50`.
    pub fn fixed(&self, bytes: &[u8], len: usize) -> Vec<String> {
        bytes
            .chunks_exact(len)
            .map(|name| self.decode(name))
            .collect()
    }

    /// Up to `count` strings, each ended by `0x50`, one after another. Fewer when the bytes run
    /// out first.
    pub fn list(&self, bytes: &[u8], count: usize) -> Vec<String> {
        bytes
            .split(|&byte| byte == END)
            .take(count)
            .map(|text| self.decode(text))
            .collect()
    }
}

/// Turn a borrowed hiragana back into katakana when it stands next to katakana.
fn restore_katakana(text: &mut [char]) {
    let katakana = |c: Option<&char>| c.is_some_and(|&c| ('ァ'..='ヺ').contains(&c) || c == 'ー');
    for index in 0..text.len() {
        let twin = match text[index] {
            'へ' => 'ヘ',
            'べ' => 'ベ',
            'ぺ' => 'ペ',
            'り' => 'リ',
            _ => continue,
        };
        let before = index.checked_sub(1).and_then(|before| text.get(before));
        if katakana(before) || katakana(text.get(index + 1)) {
            text[index] = twin;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_english_names_words_and_markup() {
        // "NIDORAN♂", padded.
        let name = [0x8D, 0x88, 0x83, 0x8E, 0x91, 0x80, 0x8D, 0xEF, 0x50, 0x50];
        assert_eq!(ENGLISH.decode(&name), "NIDORAN♂");
        // "POKé BALL", "OAK's", a line break, the player, and an unassigned code.
        assert_eq!(
            ENGLISH.decode(&[0x54, 0x7F, 0x81, 0x4E, 0x8E, 0xBD, 0x52, 0x01]),
            "POKé B\nO's<PLAYER><$01>"
        );
        assert_eq!(ENGLISH.fixed(&[0x80, 0x50, 0x81, 0x50], 2), ["A", "B"]);
        assert_eq!(ENGLISH.list(&[0x80, 0x50, 0x81, 0x50, 0x82], 2), ["A", "B"]);
    }

    #[test]
    fn decodes_japanese_with_borrowed_kana() {
        // サイドン, then リザードン with a borrowed り, and the hiragana word りんご.
        assert_eq!(JAPANESE.decode(&[0x8A, 0x81, 0x13, 0xAB, 0x50]), "サイドン");
        assert_eq!(
            JAPANESE.decode(&[0xD8, 0x0A, 0xE3, 0x13, 0xAB]),
            "リザードン"
        );
        assert_eq!(JAPANESE.decode(&[0xD8, 0xDE, 0x2A]), "りんご");
    }
}
