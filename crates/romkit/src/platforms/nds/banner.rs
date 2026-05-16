use serde::Serialize;

use crate::Result;
use crate::io::Reader;

const TITLES: usize = 0x240;
const TITLE_LEN: usize = 0x100;

/// The banner: the localized titles shown on the DS menu, usually `name\npublisher`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Banner {
    pub japanese: String,
    pub english: String,
    pub french: String,
    pub german: String,
    pub italian: String,
    pub spanish: String,
    /// From banner version 2.
    pub chinese: Option<String>,
    /// From banner version 3.
    pub korean: Option<String>,
}

impl Banner {
    /// Large enough for a version 3 banner, the largest with titles.
    pub const MAX_LEN: u64 = (TITLES + 8 * TITLE_LEN) as u64;

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let banner = Reader::new("DS banner", bytes);
        let version = banner.u8_at(0)?;
        let title = |index: usize| -> Result<String> {
            let utf16 = banner.slice_at(TITLES + index * TITLE_LEN, TITLE_LEN)?;
            let units: Vec<u16> = utf16
                .as_chunks()
                .0
                .iter()
                .map(|&unit| u16::from_le_bytes(unit))
                .take_while(|&unit| unit != 0)
                .collect();
            Ok(String::from_utf16_lossy(&units))
        };
        let since = |min: u8, index| (version >= min).then(|| title(index)).transpose();
        Ok(Self {
            japanese: title(0)?,
            english: title(1)?,
            french: title(2)?,
            german: title(3)?,
            italian: title(4)?,
            spanish: title(5)?,
            chinese: since(2, 6)?,
            korean: since(3, 7)?,
        })
    }
}
