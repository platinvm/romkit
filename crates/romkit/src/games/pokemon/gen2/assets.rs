//! A Gen 2 game as assets:
//!
//! ```text
//! species.json                         every species, with evolutions, learnsets, egg moves
//! moves.json
//! items.json
//! sprites/pokemon/<variant>/<species>.png  front and front-shiny 56×56, back and back-shiny
//!                                          48×48
//! ```

use super::{Gen2, SPECIES};
use crate::games::pokemon::gen1::sprite_path;
use crate::games::{Game, GameId, json};
use crate::io::ByteSource;
use crate::{Error, Result};

const TABLES: [&str; 3] = ["species.json", "moves.json", "items.json"];
/// Each variant's directory, and whether it is the back and shiny.
const VARIANTS: [(&str, bool, bool); 4] = [
    ("front", false, false),
    ("front-shiny", false, true),
    ("back", true, false),
    ("back-shiny", true, true),
];

impl<S: ByteSource> Game for Gen2<S> {
    fn id(&self) -> GameId {
        self.game
    }

    fn assets(&self) -> Result<Vec<String>> {
        let sprites = VARIANTS.into_iter().flat_map(|(variant, _, _)| {
            (1..=SPECIES).map(move |id| format!("sprites/pokemon/{variant}/{id:04}.png"))
        });
        Ok(TABLES
            .into_iter()
            .map(str::to_owned)
            .chain(sprites)
            .collect())
    }

    fn extract(&self, path: &str) -> Result<Vec<u8>> {
        match path {
            "species.json" => json(self.species()?),
            "moves.json" => json(self.moves()?),
            "items.json" => json(self.items()?),
            _ => {
                let (variant, id) =
                    sprite_path(path).ok_or_else(|| Error::NotFound(path.to_owned()))?;
                let &(_, back, shiny) = VARIANTS
                    .iter()
                    .find(|(name, _, _)| *name == variant)
                    .ok_or_else(|| Error::NotFound(path.to_owned()))?;
                self.pokemon_sprite(id, back, shiny)?.to_png()
            }
        }
    }
}
