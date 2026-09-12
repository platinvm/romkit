//! A Gen 5 game's sounds: its SDAT as assets (see [`games::sound`](crate::games::sound)), and
//! the Pokémon cries in it (see [`cries`]).

use super::{Gen5, SpeciesId, cached};
use crate::formats::sdat::Sdat;
use crate::games::pokemon::cries;
use crate::games::sound::Sound;
use crate::io::{ByteSource, Window};
use crate::media::Audio;
use crate::{Error, Result};

/// The species with a cry: the National Pokédex up to Genesect.
const SPECIES: usize = 649;

impl<S: ByteSource> Gen5<S> {
    /// The game's sound archive: music, sound effects, and their samples.
    pub fn sound(&self) -> Result<Sound<'_, Window<&S>>> {
        let archive = self.rom.file(self.edition.sound)?;
        let sdat = cached(&self.sound, || Sdat::parse(&archive))?;
        Ok(Sound::new(archive, sdat))
    }

    /// The cry of a species from the National Pokédex.
    pub fn cry(&self, species: SpeciesId) -> Result<Audio> {
        let dex = usize::from(species.0);
        match (1..=SPECIES).contains(&dex) {
            true => cries::cry(&self.sound()?, dex),
            false => Err(Error::NotFound(format!("cry of species {dex}"))),
        }
    }

    pub(super) fn sound_assets(&self) -> Result<Vec<String>> {
        let sound = self.sound()?;
        let mut assets = sound.assets()?;
        assets.extend(cries::assets(&sound, SPECIES));
        Ok(assets)
    }

    pub(super) fn extract_sound(&self, path: &str) -> Result<Vec<u8>> {
        let sound = self.sound()?;
        match path.starts_with("sound/cries/") {
            true => cries::extract(&sound, SPECIES, path),
            false => sound.extract(path),
        }
    }
}
