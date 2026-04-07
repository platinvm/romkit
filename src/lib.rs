//! Identify game ROMs and extract their data and assets as structured files.

mod error;
pub mod io;
#[cfg(test)]
mod testing;

pub use error::{Error, Result};
pub use io::ByteSource;
