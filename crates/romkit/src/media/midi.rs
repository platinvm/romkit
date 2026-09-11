/// A Standard MIDI File (format 1): one track per part, each event at an absolute tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Midi {
    pub ticks_per_beat: u16,
    pub tracks: Vec<Vec<(u32, Event)>>,
}

/// What a track can hold. Channels are 0 to 15; data values are 7-bit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    NoteOn {
        channel: u8,
        key: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        key: u8,
    },
    Control {
        channel: u8,
        controller: u8,
        value: u8,
    },
    Program {
        channel: u8,
        program: u8,
    },
    /// From -8192 to 8191, 0 being no bend.
    PitchBend {
        channel: u8,
        value: i16,
    },
    /// Microseconds per beat.
    Tempo(u32),
    /// A named point in time, such as `loopStart`.
    Marker(String),
}

impl Midi {
    /// Encode as a `.mid` file. Within a tick, note-offs come before anything else and note-ons
    /// after it, so a repeated note is not cut short and starts with its settings applied.
    pub fn to_mid(&self) -> Vec<u8> {
        let mut file = b"MThd\0\0\0\x06\0\x01".to_vec();
        file.extend((self.tracks.len() as u16).to_be_bytes());
        file.extend(self.ticks_per_beat.to_be_bytes());
        for track in &self.tracks {
            let mut events: Vec<_> = track.iter().collect();
            events.sort_by_key(|(tick, event)| (*tick, event.rank()));
            let mut body = Vec::new();
            let mut now = 0;
            for (tick, event) in events {
                variable_length(&mut body, tick - now);
                now = *tick;
                event.encode(&mut body);
            }
            body.extend(b"\0\xFF\x2F\0");
            file.extend(b"MTrk");
            file.extend((body.len() as u32).to_be_bytes());
            file.extend(body);
        }
        file
    }
}

impl Event {
    fn rank(&self) -> u8 {
        match self {
            Self::NoteOff { .. } => 0,
            Self::NoteOn { .. } => 2,
            _ => 1,
        }
    }

    fn encode(&self, out: &mut Vec<u8>) {
        let status = |kind: u8, channel: u8| kind | channel & 0x0F;
        match *self {
            Self::NoteOn {
                channel,
                key,
                velocity,
            } => out.extend([status(0x90, channel), key & 0x7F, velocity.clamp(1, 0x7F)]),
            Self::NoteOff { channel, key } => out.extend([status(0x80, channel), key & 0x7F, 0]),
            Self::Control {
                channel,
                controller,
                value,
            } => out.extend([status(0xB0, channel), controller & 0x7F, value & 0x7F]),
            Self::Program { channel, program } => {
                out.extend([status(0xC0, channel), program & 0x7F])
            }
            Self::PitchBend { channel, value } => {
                let value = (i32::from(value).clamp(-8192, 8191) + 8192) as u16;
                out.extend([
                    status(0xE0, channel),
                    (value & 0x7F) as u8,
                    (value >> 7) as u8,
                ]);
            }
            Self::Tempo(microseconds) => {
                out.extend(b"\xFF\x51\x03");
                out.extend(&microseconds.min(0xFF_FFFF).to_be_bytes()[1..]);
            }
            Self::Marker(ref text) => {
                out.extend(b"\xFF\x06");
                variable_length(out, text.len() as u32);
                out.extend(text.as_bytes());
            }
        }
    }
}

/// MIDI's variable-length quantity: 7 bits per byte, most significant first. Values above
/// 28 bits do not fit and are clamped.
fn variable_length(out: &mut Vec<u8>, value: u32) {
    let value = value.min(0x0FFF_FFFF);
    let mut shift = 21;
    while shift > 0 && value >> shift == 0 {
        shift -= 7;
    }
    while shift > 0 {
        out.push(0x80 | (value >> shift) as u8 & 0x7F);
        shift -= 7;
    }
    out.push(value as u8 & 0x7F);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_variable_lengths() {
        let encode = |value| {
            let mut out = Vec::new();
            variable_length(&mut out, value);
            out
        };
        assert_eq!(encode(0), [0]);
        assert_eq!(encode(0x7F), [0x7F]);
        assert_eq!(encode(0x80), [0x81, 0]);
        assert_eq!(encode(0x0FFF_FFFF), [0xFF, 0xFF, 0xFF, 0x7F]);
    }

    #[test]
    fn writes_tracks_with_ordered_deltas() {
        let note = |key| Event::NoteOn {
            channel: 1,
            key,
            velocity: 100,
        };
        let midi = Midi {
            ticks_per_beat: 48,
            tracks: vec![vec![
                (48, note(60)),
                (0, Event::Tempo(500_000)),
                (
                    48,
                    Event::NoteOff {
                        channel: 1,
                        key: 60,
                    },
                ),
                (96, Event::Marker("loopEnd".into())),
            ]],
        };
        let file = midi.to_mid();
        assert_eq!(&file[..14], b"MThd\0\0\0\x06\0\x01\0\x01\0\x30");
        assert_eq!(&file[14..18], b"MTrk");
        let body = &file[22..];
        assert_eq!(
            body.len() as u32,
            u32::from_be_bytes(file[18..22].try_into().unwrap())
        );
        assert_eq!(
            body,
            [
                &[0, 0xFF, 0x51, 3, 0x07, 0xA1, 0x20][..],
                &[0x30, 0x81, 60, 0],
                &[0, 0x91, 60, 100],
                &[0x30, 0xFF, 6, 7],
                b"loopEnd",
                &[0, 0xFF, 0x2F, 0],
            ]
            .concat()
        );
    }
}
