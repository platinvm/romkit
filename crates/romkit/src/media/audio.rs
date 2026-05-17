/// Decoded 16-bit PCM audio, channels interleaved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Audio {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
    /// The sample frame where a looping sound starts over, if it loops.
    pub loop_start: Option<u32>,
}

impl Audio {
    /// Encode as a WAV file. A loop point is written as a `smpl` chunk, which audio editors and
    /// game engines read.
    pub fn to_wav(&self) -> Vec<u8> {
        let channels = self.channels.max(1);
        let data_len = self.samples.len() as u32 * 2;
        let block_align = channels * 2;
        let mut wav = Vec::with_capacity(data_len as usize + 104);
        let chunk = |id: &[u8; 4], body: &[u8], wav: &mut Vec<u8>| {
            wav.extend(id);
            wav.extend((body.len() as u32).to_le_bytes());
            wav.extend(body);
        };
        wav.extend(b"RIFF\0\0\0\0WAVE");
        let format = [
            &1u16.to_le_bytes()[..],
            &channels.to_le_bytes(),
            &self.sample_rate.to_le_bytes(),
            &(self.sample_rate * block_align as u32).to_le_bytes(),
            &block_align.to_le_bytes(),
            &16u16.to_le_bytes(),
        ]
        .concat();
        chunk(b"fmt ", &format, &mut wav);
        if let Some(start) = self.loop_start {
            let end = (self.samples.len() as u32 / channels as u32).saturating_sub(1);
            let period = 1_000_000_000 / self.sample_rate.max(1);
            let mut sampler = [0u32; 9].map(u32::to_le_bytes).concat();
            sampler[8..12].copy_from_slice(&period.to_le_bytes());
            sampler[12..16].copy_from_slice(&60u32.to_le_bytes()); // MIDI unity note: middle C
            sampler[28..32].copy_from_slice(&1u32.to_le_bytes()); // one loop
            let looping = [0u32, 0, start, end, 0, 0].map(u32::to_le_bytes).concat();
            chunk(b"smpl", &[sampler, looping].concat(), &mut wav);
        }
        let data: Vec<u8> = self
            .samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        chunk(b"data", &data, &mut wav);
        let riff_len = wav.len() as u32 - 8;
        wav[4..8].copy_from_slice(&riff_len.to_le_bytes());
        wav
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_wav_with_an_optional_loop() {
        let audio = Audio {
            sample_rate: 8000,
            channels: 1,
            samples: vec![0, 1000, -1000],
            loop_start: None,
        };
        let wav = audio.to_wav();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize,
            wav.len() - 8
        );
        assert_eq!(&wav[wav.len() - 14..wav.len() - 6], b"data\x06\0\0\0");

        let looped = Audio {
            loop_start: Some(1),
            ..audio
        }
        .to_wav();
        assert!(looped.windows(4).any(|window| window == b"smpl"));
    }
}
