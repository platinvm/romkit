//! Pokémon cries, as the DS games from Diamond to White 2 keep them: the cry of National
//! Pokédex number N is the only wave of wave archive N in the game's SDAT. Forms with a cry of
//! their own come after the species, named after their species' archive with a suffix, such as
//! `WAVE_ARC_PV516_SKY` beside `WAVE_ARC_PV516` for Shaymin's Sky Forme.
//!
//! ```text
//! sound/cries/<dex>.wav         e.g. 0025.wav
//! sound/cries/<dex>-<form>.wav  e.g. 0492-sky.wav
//! ```

use crate::games::sound::Sound;
use crate::io::ByteSource;
use crate::media::Audio;
use crate::{Error, Result};

/// The cry of National Pokédex number `dex`.
pub fn cry<A: ByteSource>(sound: &Sound<'_, A>, dex: usize) -> Result<Audio> {
    sound.wave(dex, 0)
}

/// Every cry's asset path, for a game with `species` species.
pub(crate) fn assets<A: ByteSource>(sound: &Sound<'_, A>, species: usize) -> Vec<String> {
    cries(sound, species)
        .into_iter()
        .map(|(stem, _)| format!("sound/cries/{stem}.wav"))
        .collect()
}

/// One cry listed by [`assets`].
pub(crate) fn extract<A: ByteSource>(
    sound: &Sound<'_, A>,
    species: usize,
    path: &str,
) -> Result<Vec<u8>> {
    let stem = path
        .strip_prefix("sound/cries/")
        .and_then(|file| file.strip_suffix(".wav"));
    let (_, archive) = cries(sound, species)
        .into_iter()
        .find(|(cry, _)| Some(cry.as_str()) == stem)
        .ok_or_else(|| Error::NotFound(path.to_owned()))?;
    Ok(sound.wave(archive, 0)?.to_wav())
}

/// Each cry's file stem and wave archive.
fn cries<A: ByteSource>(sound: &Sound<'_, A>, species: usize) -> Vec<(String, usize)> {
    let archives = &sound.sdat().wave_archives;
    let name = |index: usize| archives.get(index)?.as_ref()?.name.as_deref();
    let dex = (1..=species)
        .filter(|&dex| archives.get(dex).is_some_and(Option::is_some))
        .map(|dex| (format!("{dex:04}"), dex));
    let forms = (species + 1..archives.len()).filter_map(|index| {
        let (base, form) = name(index)?.rsplit_once('_')?;
        let dex = (1..=species).find(|&dex| name(dex) == Some(base))?;
        let form = form.to_ascii_lowercase();
        form.bytes()
            .all(|byte| byte.is_ascii_alphanumeric())
            .then(|| (format!("{dex:04}-{form}"), index))
    });
    dex.chain(forms).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::sdat::Sdat;
    use crate::testing::{sdat, swar};

    #[test]
    fn finds_species_and_form_cries() {
        let wave = swar(&[(0, None, &[0x10; 8])]);
        let bytes = sdat(
            &[
                (3, None, u32::MAX),
                (3, Some("WAVE_ARC_PV001"), 0),
                (3, Some("WAVE_ARC_PV777"), 0),
                (3, Some("WAVE_ARC_SE_FIELD"), 0),
                (3, Some("WAVE_ARC_PV777_SKY"), 0),
                (3, Some("WAVE_ARC_PV999_SKY"), 0),
            ],
            &[&wave],
        );
        let table = Sdat::parse(bytes.as_slice()).unwrap();
        let sound = Sound::new(bytes.as_slice(), &table);
        assert_eq!(
            assets(&sound, 2),
            [
                "sound/cries/0001.wav",
                "sound/cries/0002.wav",
                "sound/cries/0002-sky.wav"
            ]
        );
        assert_eq!(cry(&sound, 2).unwrap().samples.len(), 8);
        assert_eq!(
            &extract(&sound, 2, "sound/cries/0002-sky.wav").unwrap()[..4],
            b"RIFF"
        );
        assert!(extract(&sound, 2, "sound/cries/0003.wav").is_err());
    }
}
