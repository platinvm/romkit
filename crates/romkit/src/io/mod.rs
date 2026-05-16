//! Random-access byte sources: how every parser reads a ROM.
//!
//! A ROM is only ever read where a parser looks. Everything goes through [`ByteSource`], which
//! is implemented for in-memory bytes (borrowed, never copied), for files ([`FileSource`]), and
//! by hosts over whatever they have, such as a browser `Blob`. [`Window`] narrows any source to
//! a sub-range, so files inside a ROM and members inside an archive are sources too.

mod reader;

use std::borrow::Cow;
use std::rc::Rc;
use std::sync::Arc;

pub use reader::{Reader, ascii};

use crate::{Error, Result};

/// Positional, read-only access to a sequence of bytes.
///
/// Implementors provide [`size`](Self::size) and [`read_exact_at`](Self::read_exact_at).
/// In-memory sources also return their bytes from [`as_slice`](Self::as_slice), which turns
/// every read into a borrow.
pub trait ByteSource {
    /// Total length in bytes.
    fn size(&self) -> u64;

    /// Fill `buf` with the bytes at `offset`. A range past the end is an error, never a short
    /// read or a panic.
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> Result<()>;

    /// The whole source, when it is already in memory.
    fn as_slice(&self) -> Option<&[u8]> {
        None
    }

    /// `len` bytes at `offset`: borrowed from in-memory sources, read otherwise.
    fn read(&self, offset: u64, len: u64) -> Result<Cow<'_, [u8]>> {
        let end = checked_end(offset, len, self.size())?;
        if let Some(bytes) = self.as_slice() {
            return Ok(Cow::Borrowed(&bytes[offset as usize..end as usize]));
        }
        let len = usize::try_from(len)
            .map_err(|_| Error::malformed("byte source", offset, "read does not fit in memory"))?;
        let mut buf = vec![0; len];
        self.read_exact_at(offset, &mut buf)?;
        Ok(Cow::Owned(buf))
    }

    /// Up to `len` bytes at `offset`, cut short at the end of the source.
    fn read_up_to(&self, offset: u64, len: u64) -> Result<Cow<'_, [u8]>> {
        let offset = offset.min(self.size());
        self.read(offset, len.min(self.size() - offset))
    }

    fn read_all(&self) -> Result<Cow<'_, [u8]>> {
        self.read(0, self.size())
    }
}

/// The end of `offset..offset + len`, if it lies within `size`.
fn checked_end(offset: u64, len: u64, size: u64) -> Result<u64> {
    offset
        .checked_add(len)
        .filter(|&end| end <= size)
        .ok_or_else(|| Error::malformed("byte source", offset, "read past the end"))
}

impl ByteSource for [u8] {
    fn size(&self) -> u64 {
        self.len() as u64
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        let end = checked_end(offset, buf.len() as u64, self.size())?;
        buf.copy_from_slice(&self[offset as usize..end as usize]);
        Ok(())
    }

    fn as_slice(&self) -> Option<&[u8]> {
        Some(self)
    }
}

/// Forward through owners and pointers, so a source can be borrowed or shared.
macro_rules! forward {
    ($($wrapper:ty),+) => {$(
        impl<T: ByteSource + ?Sized> ByteSource for $wrapper {
            fn size(&self) -> u64 {
                (**self).size()
            }

            fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
                (**self).read_exact_at(offset, buf)
            }

            fn as_slice(&self) -> Option<&[u8]> {
                (**self).as_slice()
            }
        }
    )+};
}

forward!(&T, Box<T>, Rc<T>, Arc<T>);

impl ByteSource for Vec<u8> {
    fn size(&self) -> u64 {
        self.as_slice().size()
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        self.as_slice().read_exact_at(offset, buf)
    }

    fn as_slice(&self) -> Option<&[u8]> {
        Some(self)
    }
}

/// The range `offset..offset + len` of another source, itself a source.
///
/// `S` is usually a reference (`Window<&S>`), or a shared pointer when the window must outlive
/// the borrow.
#[derive(Debug, Clone, Copy)]
pub struct Window<S> {
    source: S,
    offset: u64,
    len: u64,
}

impl<S: ByteSource> Window<S> {
    pub fn new(source: S, offset: u64, len: u64) -> Result<Self> {
        checked_end(offset, len, source.size())
            .map_err(|_| Error::malformed("byte source", offset, "window exceeds its source"))?;
        Ok(Self {
            source,
            offset,
            len,
        })
    }

    /// Where the window starts within its source.
    pub fn offset(&self) -> u64 {
        self.offset
    }
}

impl<S: ByteSource> ByteSource for Window<S> {
    fn size(&self) -> u64 {
        self.len
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        checked_end(offset, buf.len() as u64, self.len)?;
        self.source.read_exact_at(self.offset + offset, buf)
    }

    fn as_slice(&self) -> Option<&[u8]> {
        let bytes = self.source.as_slice()?;
        Some(&bytes[self.offset as usize..(self.offset + self.len) as usize])
    }
}

/// A file read in place, so even multi-gigabyte images are never loaded whole.
#[cfg(any(unix, windows))]
#[derive(Debug)]
pub struct FileSource {
    file: std::fs::File,
    size: u64,
}

#[cfg(any(unix, windows))]
impl FileSource {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let size = file.metadata()?.len();
        Ok(Self { file, size })
    }
}

#[cfg(any(unix, windows))]
impl ByteSource for FileSource {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        checked_end(offset, buf.len() as u64, self.size)?;
        #[cfg(unix)]
        std::os::unix::fs::FileExt::read_exact_at(&self.file, buf, offset)?;
        #[cfg(windows)]
        {
            let (mut done, mut offset) = (0, offset);
            while done < buf.len() {
                let read =
                    std::os::windows::fs::FileExt::seek_read(&self.file, &mut buf[done..], offset)?;
                if read == 0 {
                    return Err(Error::Host("file ended early".into()));
                }
                done += read;
                offset += read as u64;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Counting;

    #[test]
    fn slices_borrow_and_other_sources_read() {
        let bytes = b"0123456789".as_slice();
        assert!(matches!(bytes.read(2, 3).unwrap(), Cow::Borrowed(b"234")));

        let counting = Counting::new(bytes);
        let window = Window::new(&counting, 4, 4).unwrap();
        assert!(matches!(window.read(1, 2).unwrap(), Cow::Owned(ref v) if v == b"56"));
        assert_eq!(counting.bytes_read(), 2);
        assert_eq!(&*window.read_up_to(2, 100).unwrap(), b"67");
    }

    #[test]
    fn out_of_range_reads_are_errors() {
        let bytes = b"0123456789".as_slice();
        let mut buf = [0; 4];
        assert!(bytes.read_exact_at(8, &mut buf).is_err());
        assert!(bytes.read_exact_at(u64::MAX, &mut buf).is_err());
        assert!(Window::new(bytes, 8, 4).is_err());

        // A window never reads past its own end, even where its source has more bytes.
        let counting = Counting::new(bytes);
        let window = Window::new(&counting, 0, 4).unwrap();
        assert!(window.read_exact_at(2, &mut buf).is_err());
        assert_eq!(counting.bytes_read(), 0);
    }

    #[test]
    fn files_are_read_in_place() {
        let path = std::env::temp_dir().join(format!("romkit-io-{}", std::process::id()));
        std::fs::write(&path, b"0123456789").unwrap();
        let file = FileSource::open(&path).unwrap();
        assert_eq!(file.size(), 10);
        assert_eq!(&*file.read(3, 4).unwrap(), b"3456");
        assert!(file.read(8, 4).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
