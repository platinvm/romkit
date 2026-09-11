//! A 3DS Pokémon game as assets:
//!
//! ```text
//! species.json                       every species and form, with evolutions and level-up moves
//! moves.json
//! abilities.json
//! text/<language>/main/<bank>.json   names, descriptions, and menus, one file per bank
//! text/<language>/story/<bank>.json  what characters say
//! sound/<name>.wav                   music and jingles, from the BCSTM streams of Omega Ruby,
//!                                    Alpha Sapphire, and Generation VII (X and Y use AAC)
//! ```
//!
//! Banks are numbered, but the ones romkit reads are named, e.g.
//! `text/en/main/species_names.json`.

use serde::Serialize;

use super::{N3ds, TextSet};
use crate::formats::bcstm;
use crate::games::{Game, GameId, json};
use crate::io::ByteSource;
use crate::{Error, Result};

const TABLES: [&str; 3] = ["species.json", "moves.json", "abilities.json"];
const SETS: [(TextSet, &str); 2] = [(TextSet::Main, "main"), (TextSet::Story, "story")];

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
    /// Every audio stream in the RomFS, as `(asset, file)`: `sound/bgm_x.dspadpcm.bcstm` is
    /// `sound/bgm_x.wav`.
    fn streams(&self) -> impl Iterator<Item = (String, &str)> {
        self.rom.fs().paths().filter_map(|(path, _)| {
            let name = path.strip_suffix(".bcstm")?.rsplit('/').next()?;
            let stem = name.split('.').next().unwrap_or(name);
            Some((format!("sound/{stem}.wav"), path))
        })
    }

    /// A bank's asset name: its descriptive name where known, else its number.
    fn bank_stem(&self, set: TextSet, bank: usize) -> String {
        self.edition
            .bank_name(set, bank)
            .map_or_else(|| format!("{bank:04}"), str::to_owned)
    }

    /// The text bank at `path`, e.g. `text/en/main/0012.json`.
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
        assets.extend(self.streams().map(|(asset, _)| asset));
        Ok(assets)
    }

    fn extract(&self, path: &str) -> Result<Vec<u8>> {
        match path {
            "species.json" => json(self.species()?),
            "moves.json" => json(self.moves()?),
            "abilities.json" => json(self.abilities()?),
            _ if path.starts_with("sound/") => {
                let (_, file) = self
                    .streams()
                    .find(|(asset, _)| asset == path)
                    .ok_or_else(|| Error::NotFound(path.to_owned()))?;
                Ok(bcstm::decode(&self.rom.read(file)?)?.to_wav())
            }
            _ => self
                .text_bank(path)
                .unwrap_or_else(|| Err(Error::NotFound(path.to_owned()))),
        }
    }
}
