//! Media romkit produces from game data, in formats every viewer understands: images become
//! PNG, audio becomes WAV, 3D models become glTF. Decoders in [`formats`](crate::formats) and
//! the games build these types; hosts only ever see the encoded files.

mod audio;
mod gltf;
mod image;
mod mesh;

pub use audio::Audio;
pub use image::{Image, Rgba, bgr555};
pub use mesh::{AlphaMode, Material, Mesh, Primitive, Texture, Vertex, Wrap};
