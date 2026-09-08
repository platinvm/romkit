//! Games: the catalog of recognized titles, the [`Game`] trait that every extractor implements,
//! and one module per game family.
//!
//! Adding a game to a family that romkit already extracts is usually one entry in that
//! family's table (see [`pokemon::gen4::Edition`]). A new family is a module with a
//! typed handle that implements [`Game`], plus a line in [`open`] and [`GameId::is_supported`].

mod catalog;
pub mod pokemon;

use serde::Serialize;

pub use catalog::GameId;

use crate::Result;
use crate::io::ByteSource;
use crate::platforms::identify;
use pokemon::{gen3, gen4, gen5};

/// A game romkit can extract, seen as a tree of asset files.
///
/// Each extractor also has a typed API for programs that want the data itself, such as
/// [`gen4::Gen4::species`]. This trait is the uniform view that hosts use to browse and export any
/// supported game the same way: list the asset paths, then extract the ones they need.
pub trait Game {
    fn id(&self) -> GameId;

    /// Every asset's path, e.g. `species.json`. Listing extracts nothing.
    fn assets(&self) -> Result<Vec<String>>;

    /// One asset's contents; the path's extension says what they are.
    fn extract(&self, path: &str) -> Result<Vec<u8>>;
}

impl GameId {
    /// Whether romkit can extract this game.
    pub fn is_supported(self) -> bool {
        gen3::Edition::covers(self)
            || gen4::Edition::of(self).is_some()
            || gen5::Edition::of(self).is_some()
    }
}

/// The extractor for `game` over `source`, or `None` when the game is not supported.
pub fn open<'a>(game: GameId, source: impl ByteSource + 'a) -> Option<Result<Box<dyn Game + 'a>>> {
    if gen3::Edition::covers(game) {
        // Only the dumps whose tables are known: other languages and revisions are recognized
        // but not extracted.
        gen3::Edition::of(&identify(&source).ok()?)?;
        Some(gen3::Gen3::open(source).map(|game| Box::new(game) as _))
    } else if gen4::Edition::of(game).is_some() {
        Some(gen4::Gen4::open(source).map(|game| Box::new(game) as _))
    } else if gen5::Edition::of(game).is_some() {
        Some(gen5::Gen5::open(source).map(|game| Box::new(game) as _))
    } else {
        None
    }
}

/// Data assets are pretty-printed JSON, readable as exported files and in a browser alike.
pub(crate) fn json(value: &(impl Serialize + ?Sized)) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec_pretty(value)?)
}
