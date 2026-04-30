//! Identify game ROMs and extract their data and assets as structured files.

#[macro_use]
mod macros;

mod error;
pub mod formats;
pub mod games;
pub mod io;
pub mod platforms;
#[cfg(test)]
mod testing;

pub use error::{Error, Result};
pub use games::GameId;
pub use io::ByteSource;
pub use platforms::{Identity, Platform, identify};
