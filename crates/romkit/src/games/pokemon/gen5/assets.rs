//! A Gen 5 game as assets:
//!
//! ```text
//! species.json             every species and form, with evolutions and level-up moves
//! moves.json
//! abilities.json
//! items.json
//! text/main/<bank>.json    one file per system text bank, its messages in markup form
//! text/story/<bank>.json   one file per story text bank
//! ```

use serde::Serialize;

use super::Gen5;
use crate::games::{Game, GameId, json};
use crate::io::ByteSource;
use crate::{Error, Result};

const TABLES: [&str; 4] = ["species.json", "moves.json", "abilities.json", "items.json"];

#[derive(Serialize)]
struct TextBank {
    archive: &'static str,
    bank: usize,
    /// See [`Message::markup`](super::text::Message::markup).
    messages: Vec<String>,
}

impl<S: ByteSource> Game for Gen5<S> {
    fn id(&self) -> GameId {
        self.game
    }

    fn assets(&self) -> Result<Vec<String>> {
        let mut assets: Vec<String> = TABLES.into_iter().map(str::to_owned).collect();
        for (archive, banks) in [
            ("main", self.text()?.len()),
            ("story", self.story_text()?.len()),
        ] {
            assets.extend((0..banks).map(|bank| format!("text/{archive}/{bank:04}.json")));
        }
        Ok(assets)
    }

    fn extract(&self, path: &str) -> Result<Vec<u8>> {
        match path {
            "species.json" => json(self.species()?),
            "moves.json" => json(self.moves()?),
            "abilities.json" => json(self.abilities()?),
            "items.json" => json(self.items()?),
            _ => {
                let not_found = || Error::NotFound(path.to_owned());
                let (archive, file) = path
                    .strip_prefix("text/")
                    .and_then(|rest| rest.split_once('/'))
                    .ok_or_else(not_found)?;
                let (archive, text) = match archive {
                    "main" => ("main", self.text()?),
                    "story" => ("story", self.story_text()?),
                    _ => return Err(not_found()),
                };
                let bank = file
                    .strip_suffix(".json")
                    .and_then(|stem| stem.parse().ok())
                    .filter(|&bank| bank < text.len())
                    .ok_or_else(not_found)?;
                json(&TextBank {
                    archive,
                    bank,
                    messages: text
                        .bank(bank)?
                        .iter()
                        .map(|message| message.markup())
                        .collect(),
                })
            }
        }
    }
}
