//! Media romkit produces from game data, in formats every viewer understands: images become
//! PNG, audio becomes WAV. Decoders in [`formats`](crate::formats) and the games build these
//! types; hosts only ever see the encoded files.

mod audio;
mod image;

pub use audio::Audio;
pub use image::{Image, Rgba, bgr555};
