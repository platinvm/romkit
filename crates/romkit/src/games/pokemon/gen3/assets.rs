//! A Gen 3 game as assets, species keyed by National Pokédex number:
//!
//! ```text
//! species.json
//! moves.json
//! abilities.json
//! items.json
//! sprites/pokemon/<variant>/<species>.png  64×64 battle sprites: front, back, with -shiny
//! sprites/icons/<species>.png              32×32 box icons
//! sprites/trainers/<number>.png            64×64 trainer sprites
//! sprites/items/<item>.png                 24×24 item icons, but in Ruby and Sapphire
//! sound/cries/<species>.wav
//! ```

use super::{Gen3, ItemId, NATIONAL_DEX, SpeciesId, Variant};
use crate::games::{Game, GameId, json};
use crate::io::ByteSource;
use crate::{Error, Result};

const TABLES: [&str; 4] = ["species.json", "moves.json", "abilities.json", "items.json"];

impl<S: ByteSource> Game for Gen3<S> {
    fn id(&self) -> GameId {
        self.game()
    }

    fn assets(&self) -> Result<Vec<String>> {
        let mut paths: Vec<String> = TABLES.into_iter().map(str::to_owned).collect();
        let species = 1..=NATIONAL_DEX;
        for variant in Variant::ALL {
            let directory = variant.name();
            paths.extend(
                species
                    .clone()
                    .map(|n| format!("sprites/pokemon/{directory}/{n:04}.png")),
            );
        }
        paths.extend(species.clone().map(|n| format!("sprites/icons/{n:04}.png")));
        let trainers = 0..self.trainer_sprite_count();
        paths.extend(trainers.map(|n| format!("sprites/trainers/{n:04}.png")));
        if self.has_item_icons() {
            let items = 1..self.edition.item_count;
            paths.extend(items.map(|n| format!("sprites/items/{n:04}.png")));
        }
        paths.extend(species.map(|n| format!("sound/cries/{n:04}.wav")));
        Ok(paths)
    }

    fn extract(&self, path: &str) -> Result<Vec<u8>> {
        match path {
            "species.json" => return json(self.species()?),
            "moves.json" => return json(self.moves()?),
            "abilities.json" => return json(self.abilities()?),
            "items.json" => return json(self.items()?),
            _ => {}
        }
        let not_found = || Error::NotFound(path.to_owned());
        let (directory, file) = path.rsplit_once('/').ok_or_else(not_found)?;
        let (stem, extension) = file.split_once('.').ok_or_else(not_found)?;
        let number: u16 = stem.parse().map_err(|_| not_found())?;
        match (directory, extension) {
            ("sound/cries", "wav") => Ok(self.cry(SpeciesId(number))?.to_wav()),
            ("sprites/icons", "png") => self.pokemon_icon(SpeciesId(number))?.to_png(),
            ("sprites/trainers", "png") => self.trainer_sprite(number)?.to_png(),
            ("sprites/items", "png") if number > 0 => self.item_icon(ItemId(number))?.to_png(),
            (_, "png") => {
                let variant = directory
                    .strip_prefix("sprites/pokemon/")
                    .and_then(|name| Variant::ALL.into_iter().find(|v| v.name() == name))
                    .ok_or_else(not_found)?;
                self.pokemon_sprite(SpeciesId(number), variant)?.to_png()
            }
            _ => Err(not_found()),
        }
    }
}
