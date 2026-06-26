//! Identify game ROMs and extract their data and assets as structured files.
//!
//! romkit reads ROMs in layers, each knowing nothing about the ones above it:
//!
//! - [`io`]: [`ByteSource`], random access to bytes that may live in memory, in a file, or
//!   behind a browser `Blob`. Only the bytes a parser needs are ever read.
//! - [`platforms`]: what a ROM is, from its header, and the DS and 3DS filesystems.
//! - [`formats`]: file formats found inside ROMs, such as NARC archives.
//! - [`media`]: what romkit produces besides data: images (PNG) and audio (WAV).
//! - [`games`]: the catalog of recognized games and their extractors, each with a typed API
//!   and the uniform [`Game`] asset view.
//!
//! [`Rom`] ties the layers together for hosts that take any ROM:
//!
//! ```no_run
//! use romkit::Rom;
//! use romkit::io::FileSource;
//!
//! let rom = Rom::open(FileSource::open("game.nds")?)?;
//! println!("{}", rom.identity());
//! if let Some(game) = rom.game() {
//!     for path in game.assets()? {
//!         let contents = game.extract(&path)?;
//!         println!("{path}: {} bytes", contents.len());
//!     }
//! }
//! # Ok::<(), romkit::Error>(())
//! ```
//!
//! Programs that need one game's data use its typed handle instead, such as
//! [`Gen4`](games::pokemon::gen4::Gen4).

#[macro_use]
mod macros;

mod error;
pub mod formats;
pub mod games;
pub mod io;
pub mod media;
pub mod platforms;
mod rom;
#[cfg(test)]
mod testing;

pub use error::{Error, Result};
pub use games::{Game, GameId};
pub use io::ByteSource;
pub use platforms::{Identity, Platform, identify};
pub use rom::{FileInfo, Manifest, Overview, Rom};
