//! File formats found inside ROMs, independent of any one game.

pub mod narc;
pub mod sdat;

use serde::Serialize;

/// A file type recognized from its signature, its first four bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Format {
    /// The conventional extension, e.g. `narc`.
    pub extension: &'static str,
    pub description: &'static str,
}

impl Format {
    pub fn sniff(head: &[u8]) -> Option<Self> {
        let signature = head.first_chunk::<4>()?;
        SIGNATURES
            .iter()
            .find(|(known, _, _)| *known == signature)
            .map(|&(_, extension, description)| Self {
                extension,
                description,
            })
    }
}

/// Nitro SDK (DS) formats, which store their signatures byte-reversed.
const SIGNATURES: &[(&[u8; 4], &str, &str)] = &[
    (b"NARC", "narc", "archive"),
    (b"RLCN", "nclr", "palette"),
    (b"RGCN", "ncgr", "tile graphics"),
    (b"RECN", "ncer", "sprite cells"),
    (b"RNAN", "nanr", "cell animation"),
    (b"RCSN", "nscr", "tilemap"),
    (b"BMD0", "nsbmd", "3D model"),
    (b"BTX0", "nsbtx", "3D textures"),
    (b"BCA0", "nsbca", "3D skeletal animation"),
    (b"BTP0", "nsbtp", "3D texture pattern animation"),
    (b"BTA0", "nsbta", "3D texture coordinate animation"),
    (b"BMA0", "nsbma", "3D material animation"),
    (b"BVA0", "nsbva", "3D visibility animation"),
    (b"SDAT", "sdat", "sound archive"),
    (b"SSEQ", "sseq", "music sequence"),
    (b"SSAR", "ssar", "sound effect sequences"),
    (b"SBNK", "sbnk", "instrument bank"),
    (b"SWAR", "swar", "wave archive"),
    (b"STRM", "strm", "audio stream"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_signatures() {
        assert_eq!(Format::sniff(b"RGCN\xFF\xFE").unwrap().extension, "ncgr");
        assert_eq!(Format::sniff(b"NARC").unwrap().description, "archive");
        assert_eq!(Format::sniff(b"XYZW"), None);
        assert_eq!(Format::sniff(b"NAR"), None);
    }
}
