//! A 3DS Pokémon game as assets:
//!
//! ```text
//! species.json                       every species and form, with evolutions and level-up moves
//! moves.json
//! abilities.json
//! text/<language>/game/<bank>.json   names, descriptions, and menus, one file per bank
//! text/<language>/story/<bank>.json  what characters say
//! ```
//!
//! Banks are numbered, but the ones romkit reads are named, e.g.
//! `text/en/game/species_names.json`.

use serde::Serialize;

use super::{N3ds, TextSet};
use crate::games::{Game, GameId, json};
use crate::io::ByteSource;
use crate::{Error, Result};

const TABLES: [&str; 3] = ["species.json", "moves.json", "abilities.json"];
const SETS: [(TextSet, &str); 2] = [(TextSet::Game, "game"), (TextSet::Story, "story")];

#[derive(Serialize)]
struct TextBank {
    bank: usize,
    name: Option<&'static str>,
    /// See [`Message::markup`](super::text::Message::markup).
    messages: Vec<String>,
    /// Each message's flags, when any is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    flags: Option<Vec<u16>>,
}

impl<S: ByteSource> N3ds<S> {
    /// A bank's asset name: its descriptive name where known, else its number.
    fn bank_stem(&self, set: TextSet, bank: usize) -> String {
        self.edition
            .bank_name(set, bank)
            .map_or_else(|| format!("{bank:04}"), str::to_owned)
    }

    /// The text bank at `path`, e.g. `text/en/game/0012.json`.
    fn text_bank(&self, path: &str) -> Option<Result<Vec<u8>>> {
        let [language, set, file] = path
            .strip_prefix("text/")?
            .splitn(3, '/')
            .collect::<Vec<_>>()
            .try_into()
            .ok()?;
        let (set, _) = SETS.into_iter().find(|&(_, name)| name == set)?;
        let stem = file.strip_suffix(".json")?;
        let text = match self.text(set, language) {
            Ok(text) => text,
            Err(error) => return Some(Err(error)),
        };
        let bank = (0..text.len()).find(|&bank| self.bank_stem(set, bank) == stem)?;
        Some(text.bank(bank).and_then(|messages| {
            let flags: Vec<u16> = messages.iter().map(|message| message.flags).collect();
            json(&TextBank {
                bank,
                name: self.edition.bank_name(set, bank),
                messages: messages.iter().map(|message| message.markup()).collect(),
                flags: flags.iter().any(|&flags| flags != 0).then_some(flags),
            })
        }))
    }
}

impl<S: ByteSource> Game for N3ds<S> {
    fn id(&self) -> GameId {
        self.game
    }

    fn assets(&self) -> Result<Vec<String>> {
        let mut assets: Vec<String> = TABLES.into_iter().map(str::to_owned).collect();
        for language in self.languages() {
            for (set, name) in SETS {
                let banks = self.text(set, language)?.len();
                assets.extend((0..banks).map(|bank| {
                    format!("text/{language}/{name}/{}.json", self.bank_stem(set, bank))
                }));
            }
        }
        Ok(assets)
    }

    fn extract(&self, path: &str) -> Result<Vec<u8>> {
        match path {
            "species.json" => json(self.species()?),
            "moves.json" => json(self.moves()?),
            "abilities.json" => json(self.abilities()?),
            _ => self
                .text_bank(path)
                .unwrap_or_else(|| Err(Error::NotFound(path.to_owned()))),
        }
    }
}
