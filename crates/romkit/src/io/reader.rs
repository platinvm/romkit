use crate::{Error, Result};

/// A bounds-checked little-endian cursor over bytes already in memory.
///
/// Sequential reads advance the cursor; `*_at` reads take an absolute offset and leave it in
/// place. Every failure is a [`Error::Malformed`] naming the format and the offset.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    format: &'static str,
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    /// `format` names the data in error messages, e.g. `"NARC"`.
    pub fn new(format: &'static str, bytes: &'a [u8]) -> Self {
        Self {
            format,
            bytes,
            position: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    /// A [`Error::Malformed`] at `offset`.
    pub fn error_at(&self, offset: usize, reason: &'static str) -> Error {
        Error::malformed(self.format, offset as u64, reason)
    }

    /// A [`Error::Malformed`] at the cursor.
    pub fn error(&self, reason: &'static str) -> Error {
        self.error_at(self.position, reason)
    }

    pub fn seek(&mut self, position: usize) -> Result<()> {
        if position > self.bytes.len() {
            return Err(self.error_at(position, "seek past the end"));
        }
        self.position = position;
        Ok(())
    }

    pub fn skip(&mut self, count: usize) -> Result<()> {
        self.seek(self.position.saturating_add(count))
    }

    /// `len` bytes at an absolute offset.
    pub fn slice_at(&self, offset: usize, len: usize) -> Result<&'a [u8]> {
        offset
            .checked_add(len)
            .and_then(|end| self.bytes.get(offset..end))
            .ok_or_else(|| self.error_at(offset, "range exceeds the data"))
    }

    pub fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let bytes = self.slice_at(self.position, len)?;
        self.position += len;
        Ok(bytes)
    }

    pub fn array_at<const N: usize>(&self, offset: usize) -> Result<[u8; N]> {
        Ok(self
            .slice_at(offset, N)?
            .try_into()
            .expect("slice of length N"))
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let array = self.array_at(self.position)?;
        self.position += N;
        Ok(array)
    }

    /// Consume `magic`, or fail with a signature error.
    pub fn expect(&mut self, magic: &[u8]) -> Result<()> {
        if self.slice_at(self.position, magic.len()).ok() != Some(magic) {
            return Err(self.error("signature mismatch"));
        }
        self.position += magic.len();
        Ok(())
    }

    pub fn u8(&mut self) -> Result<u8> {
        self.array().map(u8::from_le_bytes)
    }

    pub fn i8(&mut self) -> Result<i8> {
        self.array().map(i8::from_le_bytes)
    }

    pub fn u16(&mut self) -> Result<u16> {
        self.array().map(u16::from_le_bytes)
    }

    pub fn u32(&mut self) -> Result<u32> {
        self.array().map(u32::from_le_bytes)
    }

    pub fn u8_at(&self, offset: usize) -> Result<u8> {
        self.array_at(offset).map(u8::from_le_bytes)
    }

    pub fn u16_at(&self, offset: usize) -> Result<u16> {
        self.array_at(offset).map(u16::from_le_bytes)
    }

    pub fn u32_at(&self, offset: usize) -> Result<u32> {
        self.array_at(offset).map(u32::from_le_bytes)
    }
}

/// Fixed-width ASCII text, up to the first NUL, without surrounding padding.
pub fn ascii(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len());
    bytes[..end]
        .iter()
        .map(|&byte| byte as char)
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_endian_and_advances() {
        let mut reader = Reader::new("test", &[0x34, 0x12, 0x78, 0x56, 0x34, 0x12, 0xFF]);
        assert_eq!(reader.u16().unwrap(), 0x1234);
        assert_eq!(reader.u32().unwrap(), 0x1234_5678);
        assert_eq!(reader.i8().unwrap(), -1);
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn errors_name_the_format_and_offset_without_advancing() {
        let mut reader = Reader::new("thing", &[1, 2, 3]);
        reader.skip(2).unwrap();
        let error = reader.u16().unwrap_err();
        assert_eq!(
            error.to_string(),
            "malformed thing at 0x2: range exceeds the data"
        );
        assert_eq!(reader.position(), 2);
    }

    #[test]
    fn signatures_and_absolute_reads() {
        let mut reader = Reader::new("test", b"NARC\x01\x00");
        assert_eq!(reader.u16_at(4).unwrap(), 1);
        reader.expect(b"NARC").unwrap();
        assert!(Reader::new("test", b"NARX").expect(b"NARC").is_err());
        assert!(reader.slice_at(usize::MAX, 2).is_err());
    }

    #[test]
    fn ascii_stops_at_nul_and_trims() {
        assert_eq!(ascii(b"POKEMON PL\0\0"), "POKEMON PL");
        assert_eq!(ascii(b"RED  "), "RED");
    }
}
