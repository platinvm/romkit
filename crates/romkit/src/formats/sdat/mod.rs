//! SDAT, the Nitro sound archive: a DS game's music, sound effects, and their samples in one
//! file.
//!
//! Layout: a 64-byte header locating four blocks. `SYMB` (optional) names the sounds, `INFO`
//! describes them, `FAT` lists the ranges of the files they use, and `FILE` holds those files.
//! Both `SYMB` and `INFO` open with eight offsets, one per kind of sound (sequences, sequence
//! archives, banks, wave archives, players, groups, stream players, streams), each to a list:
//! a count, then one entry per sound. A sound's index is the same in both blocks.
//!
//! The files are Nitro formats of their own: sequences (SSEQ, SSAR) are the notes,
//! banks (SBNK) map notes to waves, wave archives ([`swar`]) hold the samples, and streams
//! ([`strm`]) are prerecorded audio.

pub mod strm;
pub mod swar;
mod wave;

use std::borrow::Cow;
use std::ops::Range;

use crate::io::{ByteSource, Reader, Window, ascii};
use crate::{Error, Result};

const HEADER_LEN: u64 = 0x40;

/// The kinds of sound, in the order `SYMB` and `INFO` list them.
const SEQUENCES: usize = 0;
const SEQUENCE_ARCHIVES: usize = 1;
const BANKS: usize = 2;
const WAVE_ARCHIVES: usize = 3;
const STREAMS: usize = 7;

/// A sound archive's tables. Like [`Narc`](super::narc::Narc), it holds no file contents:
/// files are read from the archive's source on demand.
///
/// Each list is indexed by sound number; a number without an `INFO` entry is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sdat {
    pub sequences: Vec<Option<Sequence>>,
    pub sequence_archives: Vec<Option<SequenceArchive>>,
    pub banks: Vec<Option<Bank>>,
    pub wave_archives: Vec<Option<WaveArchive>>,
    pub streams: Vec<Option<Stream>>,
    files: Vec<Range<u64>>,
}

/// A piece of music or a sound effect: an SSEQ file played with an instrument bank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sequence {
    pub name: Option<String>,
    pub file: u32,
    pub bank: u16,
    pub volume: u8,
    pub channel_priority: u8,
    pub player_priority: u8,
    pub player: u8,
}

/// An SSAR file: sequences sharing one block of events, usually sound effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceArchive {
    pub name: Option<String>,
    pub file: u32,
    /// The archived sequences' names, by index within the archive.
    pub sequence_names: Vec<Option<String>>,
}

/// An SBNK file: instruments, playing waves from up to four wave archives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bank {
    pub name: Option<String>,
    pub file: u32,
    pub wave_archives: [Option<u16>; 4],
}

/// A SWAR file of waves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaveArchive {
    pub name: Option<String>,
    pub file: u32,
}

/// A STRM file of prerecorded audio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stream {
    pub name: Option<String>,
    pub file: u32,
    pub volume: u8,
    pub priority: u8,
    pub player: u8,
}

impl Sdat {
    /// Read the header and the `SYMB`, `INFO`, and `FAT` blocks of the archive in `source`.
    pub fn parse<S: ByteSource + ?Sized>(source: &S) -> Result<Self> {
        let head = source.read_up_to(0, HEADER_LEN)?;
        let mut header = Reader::new("SDAT", &head);
        header.expect(b"SDAT")?;
        let symbols = block(source, &header, 0, b"SYMB")?;
        let symbols = symbols
            .as_deref()
            .map(|bytes| Reader::new("SDAT SYMB", bytes));
        let info = block(source, &header, 1, b"INFO")?
            .ok_or_else(|| header.error_at(0x18, "no INFO block"))?;
        let info = Reader::new("SDAT INFO", &info);
        let fat = block(source, &header, 2, b"FAT ")?
            .ok_or_else(|| header.error_at(0x20, "no FAT block"))?;
        let files = files(&Reader::new("SDAT FAT", &fat), source.size())?;
        let file = |at: usize| {
            let id = info.u32_at(at)? & 0xFF_FFFF;
            match (id as usize) < files.len() {
                true => Ok(id),
                false => Err(info.error_at(at, "file is not in the FAT")),
            }
        };

        let mut names = sound_names(symbols.as_ref(), SEQUENCES)?;
        let sequences = entries(&info, SEQUENCES, |at, index| {
            Ok(Sequence {
                name: take(&mut names, index),
                file: file(at)?,
                bank: info.u16_at(at + 4)?,
                volume: info.u8_at(at + 6)?,
                channel_priority: info.u8_at(at + 7)?,
                player_priority: info.u8_at(at + 8)?,
                player: info.u8_at(at + 9)?,
            })
        })?;
        let mut names = archive_names(symbols.as_ref())?;
        let sequence_archives = entries(&info, SEQUENCE_ARCHIVES, |at, index| {
            let (name, sequence_names) =
                names.get_mut(index).map(std::mem::take).unwrap_or_default();
            Ok(SequenceArchive {
                name,
                file: file(at)?,
                sequence_names,
            })
        })?;
        let mut names = sound_names(symbols.as_ref(), BANKS)?;
        let banks = entries(&info, BANKS, |at, index| {
            let mut wave_archives = [None; 4];
            for (slot, id) in wave_archives.iter_mut().enumerate() {
                *id = Some(info.u16_at(at + 4 + 2 * slot)?).filter(|&id| id != 0xFFFF);
            }
            Ok(Bank {
                name: take(&mut names, index),
                file: file(at)?,
                wave_archives,
            })
        })?;
        let mut names = sound_names(symbols.as_ref(), WAVE_ARCHIVES)?;
        let wave_archives = entries(&info, WAVE_ARCHIVES, |at, index| {
            Ok(WaveArchive {
                name: take(&mut names, index),
                file: file(at)?,
            })
        })?;
        let mut names = sound_names(symbols.as_ref(), STREAMS)?;
        let streams = entries(&info, STREAMS, |at, index| {
            Ok(Stream {
                name: take(&mut names, index),
                file: file(at)?,
                volume: info.u8_at(at + 4)?,
                priority: info.u8_at(at + 5)?,
                player: info.u8_at(at + 6)?,
            })
        })?;
        Ok(Self {
            sequences,
            sequence_archives,
            banks,
            wave_archives,
            streams,
            files,
        })
    }

    /// Where file `id` lies within the archive.
    pub fn range(&self, id: u32) -> Result<Range<u64>> {
        self.files
            .get(id as usize)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("SDAT file {id} of {}", self.files.len())))
    }

    /// A file's bytes, borrowed when the archive's source is in memory.
    pub fn file<'s, S: ByteSource + ?Sized>(
        &self,
        archive: &'s S,
        id: u32,
    ) -> Result<Cow<'s, [u8]>> {
        let range = self.range(id)?;
        archive.read(range.start, range.end - range.start)
    }

    /// A file as a source of its own, for reading parts of large files.
    pub fn file_source<'s, S: ByteSource + ?Sized>(
        &self,
        archive: &'s S,
        id: u32,
    ) -> Result<Window<&'s S>> {
        let range = self.range(id)?;
        Window::new(archive, range.start, range.end - range.start)
    }
}

/// Header block `index`, checking its signature; `None` when the archive leaves it out.
fn block<'s, S: ByteSource + ?Sized>(
    source: &'s S,
    header: &Reader,
    index: usize,
    magic: &[u8; 4],
) -> Result<Option<Cow<'s, [u8]>>> {
    let at = 0x10 + 8 * index;
    let (offset, len) = (header.u32_at(at)? as u64, header.u32_at(at + 4)? as u64);
    if offset == 0 || len == 0 {
        return Ok(None);
    }
    let bytes = source
        .read(offset, len)
        .map_err(|_| header.error_at(at, "block lies outside the file"))?;
    match bytes.starts_with(magic) {
        true => Ok(Some(bytes)),
        false => Err(Error::malformed("SDAT", offset, "unexpected block")),
    }
}

/// The FAT: a count, then 16 bytes per file, its offset in the archive and its length.
fn files(fat: &Reader, archive_len: u64) -> Result<Vec<Range<u64>>> {
    let count = list_len(fat, 8, 16)?;
    (0..count)
        .map(|index| {
            let at = 12 + 16 * index;
            let start = fat.u32_at(at)? as u64;
            let end = start + fat.u32_at(at + 4)? as u64;
            match end <= archive_len {
                true => Ok(start..end),
                false => Err(fat.error_at(at, "file lies outside the archive")),
            }
        })
        .collect()
}

/// The length of the list at `at`, checking that its entries fit in the block.
fn list_len(block: &Reader, at: usize, entry_len: usize) -> Result<usize> {
    let count = block.u32_at(at)? as usize;
    match count <= block.len().saturating_sub(at + 4) / entry_len {
        true => Ok(count),
        false => Err(block.error_at(at, "list overruns its block")),
    }
}

/// Where the list of `kind` lies in a `SYMB` or `INFO` block, and its length.
fn list(block: &Reader, kind: usize, entry_len: usize) -> Result<(usize, usize)> {
    let at = block.u32_at(8 + 4 * kind)? as usize;
    let count = list_len(block, at, entry_len)?;
    Ok((at + 4, count))
}

/// The `INFO` entries of `kind`, each built by `entry(offset, index)`.
fn entries<T>(
    info: &Reader,
    kind: usize,
    mut entry: impl FnMut(usize, usize) -> Result<T>,
) -> Result<Vec<Option<T>>> {
    let (start, count) = list(info, kind, 4)?;
    (0..count)
        .map(|index| match info.u32_at(start + 4 * index)? as usize {
            0 => Ok(None),
            at if at < info.len() => entry(at, index).map(Some),
            _ => Err(info.error_at(start + 4 * index, "entry lies outside the block")),
        })
        .collect()
}

/// Sound `index`'s name, moved out of `names`.
fn take(names: &mut [Option<String>], index: usize) -> Option<String> {
    names.get_mut(index).and_then(Option::take)
}

/// Names by sound index, `None` where a sound has none.
type Names = Vec<Option<String>>;

/// The `SYMB` names of `kind`, by index; none without a `SYMB` block.
fn sound_names(symbols: Option<&Reader>, kind: usize) -> Result<Names> {
    let Some(symbols) = symbols else {
        return Ok(Vec::new());
    };
    let (start, count) = list(symbols, kind, 4)?;
    (0..count)
        .map(|index| name(symbols, start + 4 * index))
        .collect()
}

/// Sequence archive names pair each archive's name with a list of its sequences' names.
fn archive_names(symbols: Option<&Reader>) -> Result<Vec<(Option<String>, Names)>> {
    let Some(symbols) = symbols else {
        return Ok(Vec::new());
    };
    let (start, count) = list(symbols, SEQUENCE_ARCHIVES, 8)?;
    (0..count)
        .map(|index| {
            let at = start + 8 * index;
            let sequences = match symbols.u32_at(at + 4)? as usize {
                0 => Vec::new(),
                list => {
                    let count = list_len(symbols, list, 4)?;
                    (0..count)
                        .map(|index| name(symbols, list + 4 + 4 * index))
                        .collect::<Result<_>>()?
                }
            };
            Ok((name(symbols, at)?, sequences))
        })
        .collect()
}

/// The NUL-terminated name that the offset at `at` points to, if any.
fn name(symbols: &Reader, at: usize) -> Result<Option<String>> {
    match symbols.u32_at(at)? as usize {
        0 => Ok(None),
        offset => {
            let tail = symbols.slice_at(offset, symbols.len().saturating_sub(offset))?;
            Ok(Some(ascii(tail)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::sdat;

    #[test]
    fn reads_names_records_and_files() {
        let bytes = sdat(
            &[
                (SEQUENCES, Some("SEQ_TOWN"), 1),
                (WAVE_ARCHIVES, Some("WAVE_ARC_PV025"), 0),
                (WAVE_ARCHIVES, None, u32::MAX),
                (WAVE_ARCHIVES, None, 2),
            ],
            &[b"SWAR....", b"SSEQ", b"SWAR"],
        );
        let archive = Sdat::parse(bytes.as_slice()).unwrap();
        let sequence = archive.sequences[0].as_ref().unwrap();
        assert_eq!(
            (sequence.name.as_deref(), sequence.file),
            (Some("SEQ_TOWN"), 1)
        );
        assert_eq!(&*archive.file(bytes.as_slice(), 1).unwrap(), b"SSEQ");
        let waves = &archive.wave_archives;
        assert_eq!(waves.len(), 3);
        assert_eq!(
            waves[0].as_ref().unwrap().name.as_deref(),
            Some("WAVE_ARC_PV025")
        );
        assert_eq!(waves[1], None);
        assert_eq!(waves[2].as_ref().unwrap().name, None);
        assert!(archive.streams.is_empty() && archive.banks.is_empty());
        assert!(matches!(
            archive.file(bytes.as_slice(), 3),
            Err(Error::NotFound(_))
        ));
    }

    #[test]
    fn rejects_bad_tables() {
        let bytes = sdat(&[(SEQUENCES, None, 5)], &[b"SSEQ"]);
        assert!(
            Sdat::parse(bytes.as_slice()).is_err(),
            "file 5 does not exist"
        );

        let mut bytes = sdat(&[], &[b"SSEQ"]);
        let fat = u32::from_le_bytes(bytes[0x20..0x24].try_into().unwrap()) as usize;
        bytes[fat + 8..fat + 12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Sdat::parse(bytes.as_slice()).is_err(), "huge file count");
        assert!(Sdat::parse(&bytes[..0x30]).is_err());
    }
}
