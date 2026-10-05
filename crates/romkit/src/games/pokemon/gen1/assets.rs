//! A Gen 1 game as assets:
//!
//! ```text
//! species.json                       every species, with evolutions and level-up moves
//! moves.json
//! items.json
//! sprites/pokemon/front/<species>.png  56×56
//! sprites/pokemon/back/<species>.png   32×32
//! ```

use super::{Gen1, SpeciesId};
use crate::games::{Game, GameId, json};
use crate::io::ByteSource;
use crate::{Error, Result};

const TABLES: [&str; 3] = ["species.json", "moves.json", "items.json"];
const SIDES: [&str; 2] = ["front", "back"];

impl<S: ByteSource> Game for Gen1<S> {
    fn id(&self) -> GameId {
        self.game
    }

    fn assets(&self) -> Result<Vec<String>> {
        let species = self.edition.species + usize::from(self.edition.mew.is_some());
        let sprites = SIDES.into_iter().flat_map(|side| {
            (1..=species).map(move |id| format!("sprites/pokemon/{side}/{id:04}.png"))
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
                let (back, id) = match sprite_path(path) {
                    Some(("front", id)) => (false, id),
                    Some(("back", id)) => (true, id),
                    _ => return Err(Error::NotFound(path.to_owned())),
                };
                self.pokemon_sprite(id, back)?.to_png()
            }
        }
    }
}

/// The side and species of `sprites/pokemon/<side>/<species>.png`.
pub(crate) fn sprite_path(path: &str) -> Option<(&str, SpeciesId)> {
    let (side, file) = path.strip_prefix("sprites/pokemon/")?.split_once('/')?;
    let id = file.strip_suffix(".png")?.parse().ok()?;
    Some((side, SpeciesId(id)))
}
