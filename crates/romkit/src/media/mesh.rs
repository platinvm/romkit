use super::{Image, Rgba, gltf};
use crate::Result;

/// A textured triangle mesh in model space, as decoders of 3D model files produce it.
///
/// Vertices are listed per triangle, three at a time, so decoders never index anything; the
/// glTF writer merges identical vertices.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mesh {
    pub name: String,
    pub primitives: Vec<Primitive>,
    pub materials: Vec<Material>,
    /// Shared by the materials, so an image used twice is stored once.
    pub textures: Vec<Texture>,
}

/// Triangles drawn with one material.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Primitive {
    /// An index into [`Mesh::materials`].
    pub material: Option<usize>,
    /// A triangle list: every three vertices are a triangle, counter-clockwise from the front.
    pub vertices: Vec<Vertex>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    /// Zero when the vertex has no normal.
    pub normal: [f32; 3],
    /// Texture coordinates, with (0, 0) at the image's top left and (1, 1) at its bottom right.
    pub uv: [f32; 2],
    /// Multiplies the material's color and texture.
    pub color: Rgba,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    pub name: String,
    /// Multiplies the texture and the vertex colors.
    pub color: Rgba,
    /// An index into [`Mesh::textures`].
    pub texture: Option<usize>,
    pub alpha: AlphaMode,
    /// Whether back faces are drawn too.
    pub double_sided: bool,
    /// Whether colors are shown as they are, without lighting.
    pub unlit: bool,
}

/// How a material's alpha is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaMode {
    Opaque,
    /// Pixels are either drawn or not: alpha is a cutout.
    Mask,
    Blend,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Texture {
    pub image: Image,
    /// How coordinates outside 0..1 wrap, horizontally then vertically.
    pub wrap: [Wrap; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrap {
    Clamp,
    Repeat,
    /// Repeat, flipping every other copy.
    Mirror,
}

impl Mesh {
    /// Encode as binary glTF 2.0 (`.glb`), with the textures embedded as PNG.
    pub fn to_glb(&self) -> Result<Vec<u8>> {
        gltf::glb(self)
    }

    /// The corners of the box around every vertex, or `None` without vertices.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let mut vertices = self.primitives.iter().flat_map(|p| &p.vertices);
        let first = vertices.next()?.position;
        Some(vertices.fold((first, first), |(min, max), vertex| {
            let p = vertex.position;
            (
                [0, 1, 2].map(|i| min[i].min(p[i])),
                [0, 1, 2].map(|i| max[i].max(p[i])),
            )
        }))
    }
}
