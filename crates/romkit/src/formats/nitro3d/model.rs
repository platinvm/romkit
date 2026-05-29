//! Models: the `MDL0` block of NSBMD files, a dictionary of models.
//!
//! A model starts with a header (offsets from the model's start):
//!
//! ```text
//! 0x04 u32 render commands   0x08 u32 materials   0x0C u32 shapes   0x10 u32 inverse binds
//! 0x17 u8 nodes   0x18 u8 materials   0x19 u8 shapes
//! 0x1C fx32 position scale   0x20 fx32 its inverse
//! 0x2C fx16 × 6 bounding box (corner, size), times 0x38 fx32 box scale
//! 0x40 the node dictionary
//! ```
//!
//! Nodes are the transforms of a skeleton, materials say how to draw, and shapes are GPU
//! display lists. The render commands tie them together: which node matrices to multiply and
//! keep on the GPU's matrix stack, which material to bind, which shape to draw (see
//! [`Model::mesh`]).

use super::texture::Textures;
use super::{blocks, dictionary, fx16, fx32};
use crate::Result;
use crate::io::Reader;
use crate::media::{Rgba, bgr555};

/// An NSBMD file: models, and usually the textures they use.
#[derive(Debug, Clone, PartialEq)]
pub struct Nsbmd {
    pub models: Vec<Model>,
    /// Absent when the game keeps the textures elsewhere, as in an NSBTX file.
    pub textures: Option<Textures>,
}

impl Nsbmd {
    pub fn parse(file: &[u8]) -> Result<Self> {
        let file = Reader::new("NSBMD", file);
        let mut nsbmd = Self {
            models: Vec::new(),
            textures: None,
        };
        for (magic, offset) in blocks(&file, b"BMD0")? {
            let block = super::block(&file, "NSBMD", offset)?;
            match &magic {
                b"MDL0" => {
                    for (name, entry) in dictionary(&block, 8)? {
                        let at = Reader::new("NSBMD", entry).u32()? as usize;
                        let len = block.u32_at(at)? as usize;
                        let model = Reader::new("NSBMD model", block.slice_at(at, len)?);
                        nsbmd.models.push(Model::parse(name, &model)?);
                    }
                }
                b"TEX0" => nsbmd.textures = Some(Textures::from_block(&block)?),
                _ => {}
            }
        }
        Ok(nsbmd)
    }
}

/// A 4×4 matrix of column vectors, `m[row][column]`: transforms are `m * v`.
pub(super) type Matrix = [[f32; 4]; 4];

pub(super) const IDENTITY: Matrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub name: String,
    pub nodes: Vec<Node>,
    pub materials: Vec<Material>,
    pub shapes: Vec<Shape>,
    /// Vertices are stored divided by this, to fit the GPU's fixed-point range; a render
    /// command scales them back.
    pub scale: f32,
    pub inverse_scale: f32,
    /// The box around the model, as its minimum corner and its size.
    pub bounding_box: ([f32; 3], [f32; 3]),
    pub(super) commands: Vec<u8>,
    /// From the inverse bind table to the end of the model, read only by skinned models.
    pub(super) inverse_binds: Vec<u8>,
}

/// A transform of the model's skeleton, relative to its parent.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub name: String,
    pub transform: Matrix,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    pub name: String,
    /// The texture and palette names, looked up in the model's textures or elsewhere.
    pub texture: Option<String>,
    pub palette: Option<String>,
    /// How much of each light's color is reflected.
    pub diffuse: Rgba,
    /// How much of the ambient light is reflected.
    pub ambient: Rgba,
    /// Light given off regardless of lights.
    pub emission: Rgba,
    /// Whether the diffuse color also sets the vertex color, for unlit polygons.
    pub diffuse_is_vertex_color: bool,
    /// Which of the four lights shine on it, one bit each; none means unlit.
    pub lights: u8,
    /// Opacity, 0–31.
    pub alpha: u8,
    pub front: bool,
    pub back: bool,
    /// Whether texture coordinates repeat (else clamp), horizontally then vertically.
    pub repeat: [bool; 2],
    /// Whether repeats are mirrored.
    pub mirror: [bool; 2],
    /// The texture size the texture coordinates are in, as texels.
    pub texture_size: [u16; 2],
    /// Scale and then translation of texture coordinates, in texture units.
    pub texture_scale: [f32; 2],
    pub texture_translation: [f32; 2],
}

/// A GPU display list: commands that draw polygons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shape {
    pub name: String,
    pub(super) commands: Vec<u8>,
}

impl Model {
    fn parse(name: String, model: &Reader) -> Result<Self> {
        let offset = |at| -> Result<usize> { Ok(model.u32_at(at)? as usize) };
        let (commands, materials, shapes, inverse_binds) =
            (offset(0x04)?, offset(0x08)?, offset(0x0C)?, offset(0x10)?);
        let box_scale = fx32(model.u32_at(0x38)?);
        let corner = |at| -> Result<[f32; 3]> {
            let value =
                |i: usize| Ok::<_, crate::Error>(fx16(model.u16_at(at + 2 * i)?) * box_scale);
            Ok([value(0)?, value(1)?, value(2)?])
        };
        let nodes = dictionary(model, 0x40)?
            .into_iter()
            .map(|(name, entry)| {
                let at = 0x40 + Reader::new("NSBMD", entry).u32()? as usize;
                Ok(Node {
                    name,
                    transform: node_transform(model, at)?,
                })
            })
            .collect::<Result<_>>()?;
        let shapes = dictionary(model, shapes)?
            .into_iter()
            .map(|(name, entry)| {
                let at = shapes + Reader::new("NSBMD", entry).u32()? as usize;
                let list = at + model.u32_at(at + 8)? as usize;
                let len = model.u32_at(at + 12)? as usize;
                Ok(Shape {
                    name,
                    commands: model.slice_at(list, len)?.to_vec(),
                })
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            name,
            nodes,
            materials: parse_materials(model, materials)?,
            shapes,
            scale: fx32(model.u32_at(0x1C)?),
            inverse_scale: fx32(model.u32_at(0x20)?),
            bounding_box: (corner(0x2C)?, corner(0x32)?),
            commands: model
                .slice_at(commands, model.len().saturating_sub(commands))?
                .to_vec(),
            inverse_binds: model
                .slice_at(inverse_binds, model.len().saturating_sub(inverse_binds))?
                .to_vec(),
        })
    }
}

/// A node's transform: a flags word, then only the parts that are not identity. Bits 0–2
/// say translation, rotation, and scale are absent; bit 3 says the rotation is compressed
/// as a pivot: one ±1 entry (its index in bits 4–7, its sign in bit 8) whose row and column
/// are otherwise zero, and a 2×2 rest `[A B; ±B ±A]` (signs in bits 9 and 10).
fn node_transform(model: &Reader, at: usize) -> Result<Matrix> {
    let mut node = model.clone();
    node.seek(at)?;
    let flags = node.u16()?;
    let first = fx16(node.u16()?);
    let translation = match flags & 1 {
        0 => fx32s(&mut node, 3)?,
        _ => vec![0.0; 3],
    };
    // Row-major, as the DS multiplies row vectors; transposed below.
    let mut rotation = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    if flags & 2 == 0 {
        if flags & 8 != 0 {
            const REST: [[usize; 4]; 9] = [
                [4, 5, 7, 8],
                [3, 5, 6, 8],
                [3, 4, 6, 7],
                [1, 2, 7, 8],
                [0, 2, 6, 8],
                [0, 1, 6, 7],
                [1, 2, 4, 5],
                [0, 2, 3, 5],
                [0, 1, 3, 4],
            ];
            let pivot = (flags >> 4 & 15) as usize;
            let rest = REST
                .get(pivot)
                .ok_or_else(|| model.error_at(at, "pivot index out of range"))?;
            let (a, b) = (fx16(node.u16()?), fx16(node.u16()?));
            let sign = |bit: u16| if flags >> bit & 1 != 0 { -1.0 } else { 1.0 };
            rotation = [0.0; 9];
            rotation[pivot] = sign(8);
            rotation[rest[0]] = a;
            rotation[rest[1]] = b;
            rotation[rest[2]] = sign(9) * b;
            rotation[rest[3]] = sign(10) * a;
        } else {
            rotation[0] = first;
            for value in &mut rotation[1..] {
                *value = fx16(node.u16()?);
            }
        }
    }
    let scale = match flags & 4 {
        0 => fx32s(&mut node, 3)?,
        _ => vec![1.0; 3],
    };
    let mut matrix = IDENTITY;
    for row in 0..3 {
        for column in 0..3 {
            matrix[row][column] = rotation[column * 3 + row] * scale[column];
        }
        matrix[row][3] = translation[row];
    }
    Ok(matrix)
}

/// The material section: offsets of the texture and palette dictionaries, whose entries
/// list the materials using each name, then the material dictionary.
fn parse_materials(model: &Reader, at: usize) -> Result<Vec<Material>> {
    let mut materials = dictionary(model, at + 4)?
        .into_iter()
        .map(|(name, entry)| {
            parse_material(
                model,
                at + Reader::new("NSBMD", entry).u32()? as usize,
                name,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    for (dictionary_at, palette) in [(model.u16_at(at)?, false), (model.u16_at(at + 2)?, true)] {
        for (name, entry) in dictionary(model, at + dictionary_at as usize)? {
            let entry = Reader::new("NSBMD", entry);
            let users = model.slice_at(at + entry.u16_at(0)? as usize, entry.u8_at(2)? as usize)?;
            for &user in users {
                let material = materials
                    .get_mut(user as usize)
                    .ok_or_else(|| model.error_at(at, "texture bound to a missing material"))?;
                match palette {
                    false => material.texture = Some(name.clone()),
                    true => material.palette = Some(name.clone()),
                }
            }
        }
    }
    Ok(materials)
}

/// A material: GPU register values (colors, polygon attributes, texture parameters), then
/// the texture size and an optional texture matrix, present when flag bit 0 is set and
/// stored only in the parts that are not identity (flag bits 1–3).
fn parse_material(model: &Reader, at: usize, name: String) -> Result<Material> {
    let colors = model.u32_at(at + 0x04)?;
    let shine = model.u32_at(at + 0x08)?;
    let polygon = model.u32_at(at + 0x0C)?;
    let texture = model.u32_at(at + 0x14)?;
    let flags = model.u16_at(at + 0x1E)?;
    let mut matrix = model.clone();
    matrix.seek(at + 0x2C)?;
    let scale = match flags & 2 {
        0 => fx32s(&mut matrix, 2)?,
        _ => vec![1.0; 2],
    };
    if flags & 4 == 0 {
        matrix.skip(4)?; // rotation, as a sine and a cosine
    }
    let translation = match flags & 8 {
        0 => fx32s(&mut matrix, 2)?,
        _ => vec![0.0; 2],
    };
    let used = flags & 1 != 0;
    Ok(Material {
        name,
        texture: None,
        palette: None,
        diffuse: bgr555(colors as u16),
        ambient: bgr555((colors >> 16) as u16),
        emission: bgr555((shine >> 16) as u16),
        diffuse_is_vertex_color: colors >> 15 & 1 != 0,
        lights: (polygon & 15) as u8,
        alpha: (polygon >> 16 & 31) as u8,
        front: polygon >> 7 & 1 != 0,
        back: polygon >> 6 & 1 != 0,
        repeat: [texture >> 16 & 1 != 0, texture >> 17 & 1 != 0],
        mirror: [texture >> 18 & 1 != 0, texture >> 19 & 1 != 0],
        texture_size: [model.u16_at(at + 0x20)?, model.u16_at(at + 0x22)?],
        texture_scale: if used { [scale[0], scale[1]] } else { [1.0; 2] },
        texture_translation: if used {
            [translation[0], translation[1]]
        } else {
            [0.0; 2]
        },
    })
}

fn fx32s(reader: &mut Reader, count: usize) -> Result<Vec<f32>> {
    (0..count).map(|_| reader.u32().map(fx32)).collect()
}
