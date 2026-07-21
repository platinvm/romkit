//! SSEQ, the sequence, and SSAR, the sequence archive: music and sound effects as commands to
//! the DS sequencer, converted here to MIDI.
//!
//! Layout: a 16-byte header, then a `DATA` chunk holding the offset of the commands in the
//! file. SSAR follows that offset with a count and a 12-byte record per sequence, whose first
//! field is where that sequence starts within the shared commands.
//!
//! The commands are close to MIDI's: notes with a velocity and a length, rests, and controller
//! changes, at 48 ticks per beat. Track 0 starts first and opens the others (up to 16), and
//! tracks jump, call, and loop within the commands. Each track becomes a MIDI track on its own
//! channel, in order but skipping channel 10, which General MIDI reserves for drums.
//!
//! The conversion approximates what the DS plays:
//!
//! - Programs are SBNK instrument numbers, not General MIDI ones; numbers above 127 set the
//!   bank select controller to `program >> 7`.
//! - A track that jumps back to commands it already played, or reaches the end of an endless
//!   loop, loops forever on the DS: here it ends, with `loopStart` and `loopEnd` markers.
//!   Counted loops and calls are played out.
//! - Notes without a length sound until the same key is struck again or the track ends, and
//!   for at least a beat (the DS plays them until their sample ends).
//! - Random values take the middle of their range, and each track has its own variables, all
//!   starting at -1.
//! - Tied notes are separate notes. Master volume, envelopes, sweeps, and modulation other than
//!   its depth are left out.

use std::collections::HashMap;

use crate::io::Reader;
use crate::media::{Event, Midi};
use crate::{Error, Result};

pub const TICKS_PER_BEAT: u16 = 48;
/// Commands played by all tracks together before the conversion stops, so that sequences that
/// loop in ways the converter does not recognize still end.
const MAX_COMMANDS: usize = 200_000;
/// Ticks after which a track stops: days of music.
const MAX_TICKS: u32 = 1 << 27;
/// Calls and loops nest at most this deep on the DS.
const STACK_DEPTH: usize = 3;
const TRACKS: usize = 16;

/// Convert an SSEQ file.
pub fn to_midi(sseq: &[u8]) -> Result<Midi> {
    convert(commands("SSEQ", sseq)?, 0)
}

/// The number of sequences in an SSAR file.
pub fn archive_len(ssar: &[u8]) -> Result<usize> {
    let mut reader = Reader::new("SSAR", ssar);
    reader.expect(b"SSAR")?;
    Ok(reader.u32_at(0x1C)? as usize)
}

/// Convert sequence `index` of an SSAR file.
pub fn archived_to_midi(ssar: &[u8], index: usize) -> Result<Midi> {
    let count = archive_len(ssar)?;
    if index >= count {
        return Err(Error::NotFound(format!("sequence {index} of {count}")));
    }
    let record = index.saturating_mul(12).saturating_add(0x20);
    let start = Reader::new("SSAR", ssar).u32_at(record)?;
    convert(commands("SSAR", ssar)?, start as usize)
}

/// The commands of an SSEQ or SSAR file.
fn commands<'a>(format: &'static str, file: &'a [u8]) -> Result<&'a [u8]> {
    let mut reader = Reader::new(format, file);
    reader.expect(format.as_bytes())?;
    let at = reader.u32_at(0x18)? as usize;
    reader.slice_at(at, file.len().saturating_sub(at))
}

fn convert(commands: &[u8], start: usize) -> Result<Midi> {
    let mut budget = MAX_COMMANDS;
    let mut opened = vec![(0, start, 0)];
    let mut tracks = vec![Vec::new(); TRACKS];
    let mut played = [false; TRACKS];
    while let Some((number, start, tick)) = opened.pop() {
        if std::mem::replace(&mut played[number], true) {
            continue;
        }
        let mut track = Track::new(commands, number, start, tick)?;
        track.play(&mut budget)?;
        opened.extend(track.opened);
        // Tempo is global: MIDI keeps it in the first track.
        tracks[0].extend(track.tempo);
        tracks[number].extend(track.events);
    }
    let tracks = tracks
        .into_iter()
        .zip(played)
        .filter_map(|(events, played)| played.then_some(events))
        .collect();
    Ok(Midi {
        ticks_per_beat: TICKS_PER_BEAT,
        tracks,
    })
}

/// How a command's argument is given: in the commands, at random within a range that follows,
/// or by the variable whose number follows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Argument {
    Plain,
    Random,
    Variable,
}

#[derive(Clone, Copy)]
enum Size {
    U8,
    I8,
    U16,
    I16,
    VariableLength,
}

struct Track<'a> {
    reader: Reader<'a>,
    channel: u8,
    tick: u32,
    /// Return positions of calls, and loop starts with the plays left (0 for endless loops).
    stack: Vec<(usize, Option<u8>)>,
    /// When each command was first played, to recognize jumps back as loops.
    played: HashMap<usize, u32>,
    variables: [i16; 32],
    condition: bool,
    note_wait: bool,
    transpose: i32,
    /// Notes without a length that are still sounding: their key and when they started.
    held: Vec<(u8, u32)>,
    events: Vec<(u32, Event)>,
    tempo: Vec<(u32, Event)>,
    /// Tracks this one opened: their number, start, and tick.
    opened: Vec<(usize, usize, u32)>,
}

impl<'a> Track<'a> {
    fn new(commands: &'a [u8], number: usize, start: usize, tick: u32) -> Result<Self> {
        let mut reader = Reader::new("SSEQ", commands);
        reader.seek(start)?;
        Ok(Self {
            reader,
            channel: match number {
                0..9 => number as u8,
                9..15 => number as u8 + 1,
                _ => 9,
            },
            tick,
            stack: Vec::new(),
            played: HashMap::new(),
            variables: [-1; 32],
            condition: true,
            note_wait: true,
            transpose: 0,
            held: Vec::new(),
            events: Vec::new(),
            tempo: Vec::new(),
            opened: Vec::new(),
        })
    }

    fn play(&mut self, budget: &mut usize) -> Result<()> {
        while *budget > 0 && self.tick < MAX_TICKS {
            *budget -= 1;
            let at = self.reader.position();
            self.played.entry(at).or_insert(self.tick);
            let mut command = self.reader.u8()?;
            let mut enabled = true;
            if command == 0xA2 {
                enabled = self.condition;
                command = self.reader.u8()?;
            }
            let argument = match command {
                0xA0 => Argument::Random,
                0xA1 => Argument::Variable,
                _ => Argument::Plain,
            };
            if argument != Argument::Plain {
                command = self.reader.u8()?;
            }
            if !self.command(at, command, argument, enabled)? {
                break;
            }
        }
        for (key, start) in std::mem::take(&mut self.held) {
            let (channel, end) = (self.channel, start + u32::from(TICKS_PER_BEAT));
            self.event(self.tick.max(end), Event::NoteOff { channel, key });
        }
        Ok(())
    }

    /// Play one command, or only read it when a condition disabled it. False ends the track.
    fn command(
        &mut self,
        at: usize,
        command: u8,
        argument: Argument,
        enabled: bool,
    ) -> Result<bool> {
        let channel = self.channel;
        let control = |controller, value: i32| Event::Control {
            channel,
            controller,
            value: value.clamp(0, 127) as u8,
        };
        match command {
            0x00..=0x7F => {
                let velocity = self.reader.u8()?;
                let length = self.argument(argument, Size::VariableLength)?;
                if enabled {
                    self.note(command, velocity, length.max(0) as u32);
                }
            }
            0x80 => {
                let length = self.argument(argument, Size::VariableLength)?;
                if enabled {
                    self.tick = self.tick.saturating_add(length.max(0) as u32);
                }
            }
            0x81 => {
                let program = self.argument(argument, Size::VariableLength)?;
                if enabled {
                    self.event(self.tick, control(0, program >> 7));
                    let program = (program & 0x7F) as u8;
                    self.event(self.tick, Event::Program { channel, program });
                }
            }
            0x93 => {
                let (number, start) = (self.reader.u8()? as usize, self.address()?);
                if enabled && number < TRACKS {
                    self.opened.push((number, start, self.tick));
                }
            }
            0x94 => {
                let target = self.address()?;
                if enabled {
                    if let Some(&start) = self.played.get(&target) {
                        self.mark_loop(start);
                        return Ok(false);
                    }
                    self.reader.seek(target)?;
                }
            }
            0x95 => {
                let target = self.address()?;
                if enabled {
                    self.push(self.reader.position(), None)?;
                    self.reader.seek(target)?;
                }
            }
            0xB0..=0xBD => {
                let variable = self.reader.u8()? as usize;
                let value = self.argument(argument, Size::I16)?;
                if enabled {
                    self.variable(command, variable, value);
                }
            }
            0xC0..=0xD6 => {
                let size = match command {
                    0xC3 | 0xC4 => Size::I8,
                    _ => Size::U8,
                };
                let value = self.argument(argument, size)?;
                if !enabled {
                    return Ok(true);
                }
                match command {
                    0xC0 => self.event(self.tick, control(10, value)),
                    0xC1 => self.event(self.tick, control(7, value)),
                    0xC3 => self.transpose = value,
                    0xC4 => {
                        let value = (value * 64) as i16;
                        self.event(self.tick, Event::PitchBend { channel, value });
                    }
                    0xC5 => {
                        // Registered parameter 0: the pitch bend range, in semitones.
                        for (controller, value) in [(101, 0), (100, 0), (6, value), (38, 0)] {
                            self.event(self.tick, control(controller, value));
                        }
                    }
                    0xC7 => self.note_wait = value != 0,
                    0xC9 => self.event(self.tick, control(84, value)),
                    0xCA => self.event(self.tick, control(1, value)),
                    0xCE => self.event(self.tick, control(65, (value != 0) as i32 * 127)),
                    0xCF => self.event(self.tick, control(5, value)),
                    0xD4 => self.push(self.reader.position(), Some(value as u8))?,
                    0xD5 => self.event(self.tick, control(11, value)),
                    _ => {}
                }
            }
            0xE0 | 0xE1 => {
                let value = self.argument(argument, Size::U16)?;
                if enabled && command == 0xE1 {
                    let beat = 60_000_000 / value.max(1) as u32;
                    self.tempo.push((self.tick, Event::Tempo(beat)));
                }
            }
            0xE3 => _ = self.argument(argument, Size::I16)?,
            0xFC if enabled => {
                if let Some(&(start, Some(plays))) = self.stack.last() {
                    match plays {
                        0 => {
                            let first = self.played.get(&start).copied().unwrap_or(self.tick);
                            self.mark_loop(first);
                            return Ok(false);
                        }
                        1 => _ = self.stack.pop(),
                        _ => {
                            self.stack.last_mut().expect("checked above").1 = Some(plays - 1);
                            self.reader.seek(start)?;
                        }
                    }
                }
            }
            0xFD if enabled => {
                if let Some((back, _)) = self.stack.pop() {
                    self.reader.seek(back)?;
                }
            }
            0xFC | 0xFD => {}
            0xFE => _ = self.reader.u16()?,
            0xFF => return Ok(!enabled),
            _ => return Err(self.reader.error_at(at, "unknown command")),
        }
        Ok(true)
    }

    fn note(&mut self, key: u8, velocity: u8, length: u32) {
        let (channel, now) = (self.channel, self.tick);
        let key = (i32::from(key) + self.transpose).clamp(0, 127) as u8;
        if let Some(index) = self.held.iter().position(|&(held, _)| held == key) {
            self.held.swap_remove(index);
            self.event(now, Event::NoteOff { channel, key });
        }
        let velocity = velocity & 0x7F;
        self.event(
            now,
            Event::NoteOn {
                channel,
                key,
                velocity,
            },
        );
        match length {
            0 => self.held.push((key, now)),
            _ => self.event(now.saturating_add(length), Event::NoteOff { channel, key }),
        }
        if self.note_wait {
            self.tick = now.saturating_add(length);
        }
    }

    fn variable(&mut self, command: u8, variable: usize, value: i32) {
        let Some(slot) = self.variables.get_mut(variable) else {
            return;
        };
        let current = i32::from(*slot);
        *slot = match command {
            0xB0 => value,
            0xB1 => current + value,
            0xB2 => current - value,
            0xB3 => current * value,
            0xB4 if value != 0 => current / value,
            0xB5 if value >= 0 => current << value.min(16),
            0xB5 => current >> (-value).min(16),
            // A random number up to `value`.
            0xB6 => value / 2,
            0xB8..=0xBD => {
                self.condition = match command {
                    0xB8 => current == value,
                    0xB9 => current >= value,
                    0xBA => current > value,
                    0xBB => current <= value,
                    0xBC => current < value,
                    _ => current != value,
                };
                return;
            }
            _ => return,
        } as i16;
    }

    fn argument(&mut self, argument: Argument, size: Size) -> Result<i32> {
        let reader = &mut self.reader;
        let signed = |value: u16| i32::from(value as i16);
        Ok(match (argument, size) {
            (Argument::Random, _) => (signed(reader.u16()?) + signed(reader.u16()?)) / 2,
            (Argument::Variable, _) => {
                let variable = reader.u8()? as usize;
                self.variables.get(variable).copied().map_or(0, i32::from)
            }
            (_, Size::U8) => i32::from(reader.u8()?),
            (_, Size::I8) => i32::from(reader.i8()?),
            (_, Size::U16) => i32::from(reader.u16()?),
            (_, Size::I16) => signed(reader.u16()?),
            (_, Size::VariableLength) => {
                let mut value = 0;
                for _ in 0..4 {
                    let byte = reader.u8()?;
                    value = value << 7 | i32::from(byte & 0x7F);
                    if byte & 0x80 == 0 {
                        return Ok(value);
                    }
                }
                return Err(reader.error("variable-length value is too long"));
            }
        })
    }

    /// A 24-bit offset within the commands.
    fn address(&mut self) -> Result<usize> {
        let [low, middle, high] = self.reader.array()?;
        Ok(u32::from_le_bytes([low, middle, high, 0]) as usize)
    }

    fn push(&mut self, position: usize, plays: Option<u8>) -> Result<()> {
        if self.stack.len() == STACK_DEPTH {
            return Err(self.reader.error("calls and loops nest too deeply"));
        }
        self.stack.push((position, plays));
        Ok(())
    }

    fn mark_loop(&mut self, start: u32) {
        self.event(start, Event::Marker("loopStart".into()));
        self.event(self.tick, Event::Marker("loopEnd".into()));
    }

    fn event(&mut self, tick: u32, event: Event) {
        self.events.push((tick, event));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sseq(commands: &[u8]) -> Vec<u8> {
        let mut bytes = b"SSEQ\xFF\xFE\x00\x01\0\0\0\0\x10\0\x01\0DATA\0\0\0\0\x1C\0\0\0".to_vec();
        bytes.extend(commands);
        bytes
    }

    fn notes(track: &[(u32, Event)]) -> Vec<(u32, bool, u8)> {
        let mut notes: Vec<_> = track
            .iter()
            .filter_map(|(tick, event)| match *event {
                Event::NoteOn { key, .. } => Some((*tick, true, key)),
                Event::NoteOff { key, .. } => Some((*tick, false, key)),
                _ => None,
            })
            .collect();
        notes.sort();
        notes
    }

    fn markers(track: &[(u32, Event)]) -> Vec<(u32, &str)> {
        track
            .iter()
            .filter_map(|(tick, event)| match event {
                Event::Marker(text) => Some((*tick, text.as_str())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn converts_tracks_notes_and_loops() {
        #[rustfmt::skip]
        let commands = [
            0xFE, 0x03, 0x00,       // tracks 0 and 1
            0x93, 0x01, 0x1B, 0, 0, // open track 1 at 0x1B
            0xE1, 0x78, 0x00,       // 120 beats per minute
            0xC1, 0x64,             // volume 100
            0x81, 0x05,             // program 5
            0x3C, 0x64, 0x30,       // C4 for a beat
            0x80, 0x30,             // rest a beat
            0xD4, 0x02,             // twice:
            0x40, 0x50, 0x18,       //   E4 for half a beat
            0xFC,
            0xFF,
            0x30, 0x7F, 0x00,       // 0x1B: C3 without a length
            0x80, 0x81, 0x00,       // rest 128 ticks
            0x94, 0x1B, 0, 0,       // loop back
        ];
        let midi = to_midi(&sseq(&commands)).unwrap();
        assert_eq!(midi.ticks_per_beat, 48);
        let [first, second] = &midi.tracks[..] else {
            panic!("{} tracks", midi.tracks.len());
        };
        assert!(first.contains(&(0, Event::Tempo(500_000))));
        let volume = Event::Control {
            channel: 0,
            controller: 7,
            value: 100,
        };
        assert!(first.contains(&(0, volume)));
        assert!(first.contains(&(
            0,
            Event::Program {
                channel: 0,
                program: 5
            }
        )));
        assert_eq!(
            notes(first),
            [
                (0, true, 60),
                (48, false, 60),
                (96, true, 64),
                (120, false, 64),
                (120, true, 64),
                (144, false, 64)
            ]
        );
        assert_eq!(notes(second), [(0, true, 48), (128, false, 48)]);
        assert_eq!(markers(second), [(0, "loopStart"), (128, "loopEnd")]);
        assert!(
            second
                .iter()
                .all(|(_, event)| !matches!(event, Event::NoteOn { channel: c, .. } if *c != 1))
        );
        assert_eq!(&midi.to_mid()[..4], b"MThd");
    }

    #[test]
    fn evaluates_conditions_variables_and_random_values() {
        #[rustfmt::skip]
        let commands = [
            0xB0, 0x00, 0x05, 0x00,             // var0 = 5
            0xB8, 0x00, 0x05, 0x00,             // var0 == 5
            0xA2, 0x80, 0x10,                   // if so, rest 16
            0xB8, 0x00, 0x06, 0x00,             // var0 == 6
            0xA2, 0x80, 0x10,                   // if so, rest 16 (skipped)
            0xA1, 0x80, 0x00,                   // rest var0
            0xA0, 0x80, 0x0A, 0x00, 0x14, 0x00, // rest 10 to 20
            0xA0, 0x3C, 0x64, 0x02, 0x00, 0x04, 0x00, // C4 for 2 to 4
            0xD4, 0x00, 0x80, 0x01, 0xFC,       // rest 1 forever
        ];
        let midi = to_midi(&sseq(&commands)).unwrap();
        assert_eq!(notes(&midi.tracks[0]), [(36, true, 60), (39, false, 60)]);
        assert_eq!(
            markers(&midi.tracks[0]),
            [(39, "loopStart"), (40, "loopEnd")]
        );
    }

    #[test]
    fn rejects_bad_commands_and_stops_long_sequences() {
        assert!(to_midi(&sseq(&[0xF0])).is_err(), "unknown command");
        assert!(to_midi(&sseq(&[0x80])).is_err(), "truncated");
        assert!(to_midi(&sseq(&[0x94, 0xFF, 0, 0])).is_err(), "jump outside");
        assert!(
            to_midi(&sseq(&[0x95, 0, 0, 0])).is_err(),
            "endless recursion"
        );
        assert!(to_midi(&sseq(&[0x80, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F])).is_err());
        assert!(to_midi(b"SSEQ").is_err());
        // 255 × 255 × 255 plays: cut short, not endless.
        let nested = [
            0xD4, 0xFF, 0xD4, 0xFF, 0xD4, 0xFF, 0x80, 0x01, 0xFC, 0xFC, 0xFC, 0xFF,
        ];
        assert!(to_midi(&sseq(&nested)).is_ok());
    }

    #[test]
    fn converts_archived_sequences() {
        let mut ssar = b"SSAR\xFF\xFE\x00\x01\0\0\0\0\x10\0\x01\0DATA\0\0\0\0\x38\0\0\0".to_vec();
        ssar.extend(2u32.to_le_bytes());
        for start in [0u32, 4] {
            ssar.extend(start.to_le_bytes());
            ssar.extend([0; 8]);
        }
        ssar.extend([0x3C, 0x64, 0x01, 0xFF, 0x3E, 0x64, 0x01, 0xFF]);
        assert_eq!(archive_len(&ssar).unwrap(), 2);
        let second = archived_to_midi(&ssar, 1).unwrap();
        assert_eq!(notes(&second.tracks[0]), [(0, true, 62), (1, false, 62)]);
        assert!(matches!(
            archived_to_midi(&ssar, 2),
            Err(Error::NotFound(_))
        ));
    }
}
