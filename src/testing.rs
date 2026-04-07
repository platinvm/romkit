//! Helpers shared by unit tests: a metering source and builders for synthetic data.

use std::cell::Cell;

use crate::Result;
use crate::io::ByteSource;

/// Wraps a source, hides its in-memory slice (so reads take the copying path), and counts the
/// bytes read through it.
pub struct Counting<S> {
    inner: S,
    bytes_read: Cell<u64>,
}

impl<S: ByteSource> Counting<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            bytes_read: Cell::new(0),
        }
    }

    pub fn bytes_read(&self) -> u64 {
        self.bytes_read.get()
    }
}

impl<S: ByteSource> ByteSource for Counting<S> {
    fn size(&self) -> u64 {
        self.inner.size()
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        self.bytes_read
            .set(self.bytes_read.get() + buf.len() as u64);
        self.inner.read_exact_at(offset, buf)
    }
}
