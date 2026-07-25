//! A DS game's sound archive as assets, for any game with an SDAT:
//!
//! ```text
//! sound/sequences/<sequence>.mid            music and sound effects, as MIDI
//! sound/sequences/<archive>/<sequence>.mid  the sequences of a sequence archive
//! sound/waves/<wave archive>/<index>.wav    the samples instruments and effects play
//! sound/streams/<stream>.wav                prerecorded audio
//! ```
//!
//! Sounds are named after the archive's symbols, e.g. `SEQ_TOWN01`; a sound without a name
//! that works as a file name, or sharing it with another, is numbered instead (`0042`).
//!
//! A game exposes these assets by keeping its parsed [`Sdat`] and handing [`Sound`] the
//! archive's file: `Sound::new(rom.file(path)?, &sdat).assets()`.

use std::collections::HashMap;

use crate::formats::sdat::{Sdat, sseq, strm, swar};
use crate::io::ByteSource;
use crate::media::{Audio, Midi};
use crate::{Error, Result};

/// An SDAT with its tables, decoding sounds on demand.
pub struct Sound<'a, A> {
    archive: A,
    sdat: &'a Sdat,
}

impl<'a, A: ByteSource> Sound<'a, A> {
    /// `sdat` holds the tables of `archive`.
    pub fn new(archive: A, sdat: &'a Sdat) -> Self {
        Self { archive, sdat }
    }

    pub fn sdat(&self) -> &'a Sdat {
        self.sdat
    }

    /// Sequence `index`, as MIDI.
    pub fn sequence(&self, index: usize) -> Result<Midi> {
        let sequence = sound(&self.sdat.sequences, index, "sequence")?;
        sseq::to_midi(&self.sdat.file(&self.archive, sequence.file)?)
    }

    /// Wave `wave` of wave archive `index`.
    pub fn wave(&self, index: usize, wave: usize) -> Result<Audio> {
        let archive = sound(&self.sdat.wave_archives, index, "wave archive")?;
        swar::wave(&self.sdat.file_source(&self.archive, archive.file)?, wave)
    }

    /// The number of waves in wave archive `index`.
    pub fn wave_count(&self, index: usize) -> Result<usize> {
        let archive = sound(&self.sdat.wave_archives, index, "wave archive")?;
        swar::len(&self.sdat.file_source(&self.archive, archive.file)?)
    }

    /// Stream `index`.
    pub fn stream(&self, index: usize) -> Result<Audio> {
        let stream = sound(&self.sdat.streams, index, "stream")?;
        strm::decode(&self.sdat.file(&self.archive, stream.file)?)
    }

    /// Every sound's asset path. Reads only the headers of wave and sequence archives.
    pub fn assets(&self) -> Result<Vec<String>> {
        let sdat = self.sdat;
        let mut assets = Vec::new();
        let names = stems(sdat.sequences.iter().map(|s| s.as_ref()?.name.as_deref()));
        for (sequence, stem) in sdat.sequences.iter().zip(names) {
            if sequence.is_some() {
                assets.push(format!("sound/sequences/{stem}.mid"));
            }
        }
        let names = stems(
            sdat.sequence_archives
                .iter()
                .map(|s| s.as_ref()?.name.as_deref()),
        );
        for (archive, stem) in sdat.sequence_archives.iter().zip(names) {
            let Some(archive) = archive else { continue };
            let count = sseq::archive_len(&sdat.file(&self.archive, archive.file)?)?;
            for name in archive_stems(&archive.sequence_names, count) {
                assets.push(format!("sound/sequences/{stem}/{name}.mid"));
            }
        }
        let names = stems(
            sdat.wave_archives
                .iter()
                .map(|s| s.as_ref()?.name.as_deref()),
        );
        for (index, stem) in names.iter().enumerate() {
            if sdat.wave_archives[index].is_some() {
                let count = self.wave_count(index)?;
                assets.extend((0..count).map(|wave| format!("sound/waves/{stem}/{wave:04}.wav")));
            }
        }
        let names = stems(sdat.streams.iter().map(|s| s.as_ref()?.name.as_deref()));
        for (stream, stem) in sdat.streams.iter().zip(names) {
            if stream.is_some() {
                assets.push(format!("sound/streams/{stem}.wav"));
            }
        }
        Ok(assets)
    }

    /// One asset listed by [`assets`](Self::assets).
    pub fn extract(&self, path: &str) -> Result<Vec<u8>> {
        let sdat = self.sdat;
        let not_found = || Error::NotFound(path.to_owned());
        let (kind, file) = path
            .strip_prefix("sound/")
            .and_then(|path| path.split_once('/'))
            .ok_or_else(not_found)?;
        match (kind, file.strip_suffix(".mid"), file.strip_suffix(".wav")) {
            ("sequences", Some(stem), _) => match stem.split_once('/') {
                None => {
                    let names = sdat.sequences.iter().map(|s| s.as_ref()?.name.as_deref());
                    let index = find(names, stem).ok_or_else(not_found)?;
                    Ok(self.sequence(index)?.to_mid())
                }
                Some((archive, sequence)) => {
                    let names = sdat.sequence_archives.iter();
                    let index = find(names.map(|s| s.as_ref()?.name.as_deref()), archive);
                    let archive = index
                        .and_then(|index| sdat.sequence_archives[index].as_ref())
                        .ok_or_else(not_found)?;
                    let ssar = sdat.file(&self.archive, archive.file)?;
                    let names = archive_stems(&archive.sequence_names, sseq::archive_len(&ssar)?);
                    let index = names
                        .iter()
                        .position(|name| name == sequence)
                        .ok_or_else(not_found)?;
                    Ok(sseq::archived_to_midi(&ssar, index)?.to_mid())
                }
            },
            ("waves", _, Some(stem)) => {
                let (archive, wave) = stem.split_once('/').ok_or_else(not_found)?;
                let names = sdat
                    .wave_archives
                    .iter()
                    .map(|s| s.as_ref()?.name.as_deref());
                let index = find(names, archive).ok_or_else(not_found)?;
                let wave = wave.parse().map_err(|_| not_found())?;
                Ok(self.wave(index, wave)?.to_wav())
            }
            ("streams", _, Some(stem)) => {
                let names = sdat.streams.iter().map(|s| s.as_ref()?.name.as_deref());
                let index = find(names, stem).ok_or_else(not_found)?;
                Ok(self.stream(index)?.to_wav())
            }
            _ => Err(not_found()),
        }
    }
}

/// Sound `index` of a list, if the archive has it.
fn sound<'a, T>(sounds: &'a [Option<T>], index: usize, kind: &str) -> Result<&'a T> {
    sounds
        .get(index)
        .and_then(Option::as_ref)
        .ok_or_else(|| Error::NotFound(format!("{kind} {index}")))
}

/// Each sound's file stem: its name where that is a plain file name that no other sound has,
/// else its number.
fn stems<'a>(names: impl Iterator<Item = Option<&'a str>>) -> Vec<String> {
    let plain = |name: &str| {
        let valid = |byte: u8| byte.is_ascii_alphanumeric() || b"_-".contains(&byte);
        name.len() <= 64 && name.bytes().all(valid) && !name.bytes().all(|b| b.is_ascii_digit())
    };
    let names: Vec<_> = names.map(|name| name.filter(|name| plain(name))).collect();
    let mut uses = HashMap::new();
    for &name in names.iter().flatten() {
        *uses.entry(name).or_insert(0) += 1;
    }
    let stem = |(index, name): (usize, &Option<&str>)| match name {
        Some(name) if uses[name] == 1 => name.to_string(),
        _ => format!("{index:04}"),
    };
    names.iter().enumerate().map(stem).collect()
}

/// The stems of the `count` sequences in a sequence archive.
fn archive_stems(names: &[Option<String>], count: usize) -> Vec<String> {
    let names = (0..count).map(|index| names.get(index).and_then(Option::as_deref));
    stems(names)
}

/// The index of the sound with stem `stem`.
fn find<'a>(names: impl Iterator<Item = Option<&'a str>>, stem: &str) -> Option<usize> {
    stems(names).iter().position(|name| name == stem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{sdat, swar};

    #[test]
    fn names_files_after_unique_plain_symbols() {
        let names = [
            Some("SEQ_A"),
            None,
            Some("a/b"),
            Some("TWICE"),
            Some("TWICE"),
            Some("7"),
        ];
        assert_eq!(
            stems(names.into_iter()),
            ["SEQ_A", "0001", "0002", "0003", "0004", "0005"]
        );
    }

    #[test]
    fn lists_and_extracts_every_kind() {
        let sseq =
            b"SSEQ\xFF\xFE\x00\x01\0\0\0\0\x10\0\x01\0DATA\0\0\0\0\x1C\0\0\0\x3C\x64\x30\xFF";
        let swar = swar(&[(0, None, &[0; 4]), (0, None, &[1; 8])]);
        let mut strm =
            b"STRM\xFF\xFE\x00\x01\0\0\0\0\x10\0\x02\0HEAD\x50\0\0\0\x01\0\x01\0".to_vec();
        strm.extend([8000u16, 0].map(u16::to_le_bytes).concat());
        strm.extend(
            [0u32, 2, 0x68, 1, 4, 2, 4, 2]
                .map(u32::to_le_bytes)
                .concat(),
        );
        strm.resize(0x60, 0);
        strm.extend(b"DATA\0\0\0\0\x01\0\x02\0");
        let bytes = sdat(
            &[
                (0, Some("SEQ_TITLE"), 0),
                (0, None, 0),
                (3, Some("WAVE_ARC_SE"), 1),
                (7, Some("STRM_VOICE"), 2),
            ],
            &[sseq, &swar, &strm],
        );
        let table = Sdat::parse(bytes.as_slice()).unwrap();
        let sound = Sound::new(bytes.as_slice(), &table);
        assert_eq!(
            sound.assets().unwrap(),
            [
                "sound/sequences/SEQ_TITLE.mid",
                "sound/sequences/0001.mid",
                "sound/waves/WAVE_ARC_SE/0000.wav",
                "sound/waves/WAVE_ARC_SE/0001.wav",
                "sound/streams/STRM_VOICE.wav",
            ]
        );
        for path in sound.assets().unwrap() {
            let file = sound.extract(&path).unwrap();
            let magic: &[u8] = if path.ends_with(".mid") {
                b"MThd"
            } else {
                b"RIFF"
            };
            assert_eq!(&file[..4], magic, "{path}");
        }
        assert_eq!(sound.stream(0).unwrap().samples, [1, 2]);
        for missing in [
            "sound/sequences/SEQ_NONE.mid",
            "sound/sequences/SEQ_TITLE.wav",
            "sound/waves/WAVE_ARC_SE/0002.wav",
            "sound/waves/WAVE_ARC_SE.wav",
            "sound/streams/STRM_VOICE.mid",
            "sound/other/x.wav",
            "species.json",
        ] {
            assert!(
                matches!(sound.extract(missing), Err(Error::NotFound(_))),
                "{missing}"
            );
        }
    }
}
