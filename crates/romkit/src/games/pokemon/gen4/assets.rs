//! A Gen 4 game as assets:
//!
//! ```text
//! species.json      every species and form, with evolutions and level-up moves
//! moves.json
//! abilities.json
//! text/<bank>.json  one file per text bank, its messages in markup form
//! sprites/pokemon/<variant>/<species>.png  80×80 battle sprites: front, back, with -shiny,
//!                                          and with -female where females look different
//! sprites/icons/<species>.png              32×32 box icons
//! sprites/trainers/<class>.png             80×80 trainer class sprites
//! sprites/items/<item>.png                 32×32 item icons
//! ```

use serde::Serialize;

use super::Gen4;
use crate::games::{Game, GameId, json};
use crate::io::ByteSource;
use crate::{Error, Result};

const TABLES: [&str; 3] = ["species.json", "moves.json", "abilities.json"];

#[derive(Serialize)]
struct TextBank {
    bank: usize,
    name: Option<&'static str>,
    /// See [`Message::markup`](super::text::Message::markup).
    messages: Vec<String>,
}

/// A bank's asset name: its descriptive name where known, else its number.
fn bank_stem(bank: usize, name: Option<&str>) -> String {
    name.map_or_else(|| format!("{bank:04}"), str::to_owned)
}

impl<S: ByteSource> Game for Gen4<S> {
    fn id(&self) -> GameId {
        self.game
    }

    fn assets(&self) -> Result<Vec<String>> {
        let text = self.text()?;
        let banks =
            (0..text.len()).map(|bank| format!("text/{}.json", bank_stem(bank, text.name(bank))));
        let tables = TABLES.into_iter().map(str::to_owned);
        Ok(tables.chain(banks).chain(self.sprite_assets()?).collect())
    }

    fn extract(&self, path: &str) -> Result<Vec<u8>> {
        match path {
            "species.json" => json(self.species()?),
            "moves.json" => json(self.moves()?),
            "abilities.json" => json(self.abilities()?),
            _ if path.starts_with("sprites/") => self.extract_sprite(path),
            _ => {
                let text = self.text()?;
                let stem = path
                    .strip_prefix("text/")
                    .and_then(|file| file.strip_suffix(".json"));
                let bank = stem
                    .and_then(|stem| text.find(stem).or_else(|| stem.parse().ok()))
                    .filter(|&bank| bank < text.len())
                    .ok_or_else(|| Error::NotFound(path.to_owned()))?;
                json(&TextBank {
                    bank,
                    name: text.name(bank),
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
