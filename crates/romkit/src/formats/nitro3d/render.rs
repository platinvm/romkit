//! Drawing a model into a [`Mesh`], as the DS would: run its render commands, which drive the
//! GPU's matrix stack and call shapes, and run each shape's display list, which emits
//! vertices through the current matrix. The result is static: every vertex in model space,
//! posed as the model is at rest.

use std::collections::HashMap;

use super::model::{IDENTITY, Matrix, Model};
use super::texture::Textures;
use super::{fx16, fx32};
use crate::io::Reader;
use crate::media::{AlphaMode, Material, Mesh, Primitive, Texture, Vertex, Wrap, bgr555};
use crate::{Error, Result};

/// Entries of the GPU's matrix stack that render commands address.
const STACK_LEN: usize = 32;
/// Limits on what a model may produce, as render commands can draw a shape any number of
/// times and materials can pair a texture with any palette. Game models stay far below.
const MAX_VERTICES: usize = 1 << 21;
const MAX_PIXELS: usize = 1 << 24;
const WHITE: [u8; 4] = [255; 4];

impl Model {
    /// The model's polygons, with its materials and their textures. Textures are looked up
    /// by name in `textures`, in order: usually the model file's own, then any the game
    /// keeps apart. Materials whose texture is not found are drawn untextured.
    pub fn mesh(&self, textures: &[&Textures]) -> Result<Mesh> {
        let mut mesh = Mesh {
            name: self.name.clone(),
            ..Mesh::default()
        };
        let mut images = HashMap::new();
        for material in &self.materials {
            let converted = convert_material(material, textures, &mut mesh, &mut images)?;
            mesh.materials.push(converted);
        }

        let mut stack = [IDENTITY; STACK_LEN];
        let mut current = IDENTITY;
        let mut material = None;
        let mut primitives: HashMap<Option<usize>, usize> = HashMap::new();
        let mut vertices = 0;
        let mut commands = Reader::new("NSBMD render commands", &self.commands);
        while commands.remaining() > 0 {
            let opcode = commands.u8()?;
            // Bits 5 and 6 of node opcodes add a stack slot to store into and one to load from.
            let store = opcode & 0x20 != 0;
            let load = opcode & 0x40 != 0;
            match opcode & 0x1F {
                0x00 => {}
                0x01 => break,
                // Node visibility: (node, visible).
                0x02 => commands.skip(2)?,
                0x03 => current = stack[commands.u8()? as usize % STACK_LEN],
                0x04 => {
                    material = Some(commands.u8()? as usize).filter(|&m| m < self.materials.len())
                }
                0x05 => {
                    let shape = commands.u8()? as usize;
                    let shape = self
                        .shapes
                        .get(shape)
                        .ok_or_else(|| commands.error("draws a missing shape"))?;
                    let index = *primitives.entry(material).or_insert_with(|| {
                        mesh.primitives.push(Primitive {
                            material,
                            vertices: Vec::new(),
                        });
                        mesh.primitives.len() - 1
                    });
                    let before = mesh.primitives[index].vertices.len();
                    let material = material.map(|m| &self.materials[m]);
                    let color = match material {
                        Some(m) if m.diffuse_is_vertex_color => m.diffuse,
                        _ => WHITE,
                    };
                    let state = State {
                        color,
                        size: material.map_or([1.0; 2], |m| {
                            [0, 1].map(|i| m.texture_size[i].max(1) as f32)
                        }),
                        scale: material.map_or([1.0; 2], |m| m.texture_scale),
                        translation: material.map_or([0.0; 2], |m| m.texture_translation),
                        lit: material.is_some_and(|m| m.lights != 0),
                    };
                    let out = &mut mesh.primitives[index].vertices;
                    draw(&shape.commands, state, &mut current, &stack, out)?;
                    vertices += mesh.primitives[index].vertices.len() - before;
                    if vertices > MAX_VERTICES {
                        return Err(commands.error("draws too many vertices"));
                    }
                }
                // A node's matrix, multiplied into the current one: (node, parent, unused).
                0x06 => {
                    let node = commands.u8()? as usize;
                    commands.skip(2)?;
                    let (slot, from) = slots(&mut commands, store, load)?;
                    if let Some(from) = from {
                        current = stack[from];
                    }
                    let node = self
                        .nodes
                        .get(node)
                        .ok_or_else(|| commands.error("uses a missing node"))?;
                    current = multiply(&current, &node.transform);
                    if let Some(slot) = slot {
                        stack[slot] = current;
                    }
                }
                // Billboards, which face the camera; a static mesh keeps them as modeled.
                0x07 | 0x08 => {
                    commands.skip(1)?;
                    let (slot, from) = slots(&mut commands, store, load)?;
                    if let Some(from) = from {
                        current = stack[from];
                    }
                    if let Some(slot) = slot {
                        stack[slot] = current;
                    }
                }
                0x09 => current = self.blend(&mut commands, &mut stack)?,
                // A display list called directly: (offset, size). Unused by models seen so far.
                0x0A => commands.skip(8)?,
                0x0B => {
                    let scale = if store {
                        self.inverse_scale
                    } else {
                        self.scale
                    };
                    current = multiply(&current, &scaling(scale));
                }
                // Environment and projection mapping: (material, flags).
                0x0C | 0x0D => commands.skip(2)?,
                _ => return Err(commands.error("unknown render command")),
            }
        }
        mesh.primitives
            .retain(|primitive| !primitive.vertices.is_empty());
        Ok(mesh)
    }

    /// Skinning: a weighted sum of stack matrices, each times its node's inverse bind
    /// matrix, stored into a stack slot. Parameters: the slot, a count, then (stack slot,
    /// node, weight in 256ths) per term. Inverse binds are 4×3 matrices of row vectors,
    /// each followed by a 3×3 one for normals.
    fn blend(&self, commands: &mut Reader, stack: &mut [Matrix; STACK_LEN]) -> Result<Matrix> {
        let slot = commands.u8()? as usize % STACK_LEN;
        let count = commands.u8()?;
        let binds = Reader::new("NSBMD inverse binds", &self.inverse_binds);
        let mut sum = [[0.0; 4]; 4];
        for _ in 0..count {
            let from = stack[commands.u8()? as usize % STACK_LEN];
            let node = commands.u8()? as usize;
            let weight = commands.u8()? as f32 / 256.0;
            let at = node * (12 + 9) * 4;
            // Stored as rows of row vectors: (row, column) here is (column, row) there.
            let mut bind = IDENTITY;
            for (i, value) in bind[..3].iter_mut().flatten().enumerate() {
                let (row, column) = (i / 4, i % 4);
                *value = fx32(binds.u32_at(at + 4 * (column * 3 + row))?);
            }
            let term = multiply(&from, &bind);
            for (sum, term) in sum.iter_mut().flatten().zip(term.iter().flatten()) {
                *sum += weight * term;
            }
        }
        stack[slot] = sum;
        Ok(sum)
    }
}

/// The optional (store, load) stack slots that follow a node command's parameters.
fn slots(commands: &mut Reader, store: bool, load: bool) -> Result<(Option<usize>, Option<usize>)> {
    let mut slot = |present: bool| -> Result<Option<usize>> {
        Ok(match present {
            true => Some(commands.u8()? as usize % STACK_LEN),
            false => None,
        })
    };
    Ok((slot(store)?, slot(load)?))
}

/// What a display list draws with besides its own commands.
#[derive(Clone, Copy)]
struct State {
    color: [u8; 4],
    /// Texture coordinates are in texels of a texture this size.
    size: [f32; 2],
    scale: [f32; 2],
    translation: [f32; 2],
    /// Lit polygons take their color from lighting, not from color commands.
    lit: bool,
}

/// Run a display list, appending its triangles to `out`.
///
/// A display list is packed: four command bytes, then the 32-bit parameters of those four
/// commands in order. Vertices take the latest color, normal, and texture coordinates, and
/// are grouped by begin/end commands into triangles, quads, or strips of either.
fn draw(
    list: &[u8],
    state: State,
    current: &mut Matrix,
    stack: &[Matrix; STACK_LEN],
    out: &mut Vec<Vertex>,
) -> Result<()> {
    let mut list = Reader::new("GPU display list", list);
    let mut vertex = Vertex {
        position: [0.0; 3],
        normal: [0.0; 3],
        uv: [0.0; 2],
        color: if state.lit { WHITE } else { state.color },
    };
    let mut position = [0.0f32; 3];
    let mut kind = 0;
    let mut group = Vec::new();
    while list.remaining() >= 4 {
        for opcode in list.array::<4>()? {
            let words = match opcode {
                0x00 | 0x11 | 0x15 | 0x41 => 0,
                0x10 | 0x12 | 0x13 | 0x14 | 0x20..=0x22 | 0x24..=0x2B | 0x30..=0x33 | 0x40 => 1,
                0x23 => 2,
                0x1B | 0x1C => 3,
                0x1A => 9,
                0x17 | 0x19 => 12,
                0x16 | 0x18 => 16,
                0x34 => 32,
                _ => return Err(list.error("unknown GPU command")),
            };
            let mut params = [0u32; 32];
            for param in &mut params[..words] {
                *param = list.u32()?;
            }
            let p = params[0];
            let mut emit = |position: [f32; 3], vertex: &mut Vertex| {
                vertex.position = transform(current, position, 1.0);
                group.push(*vertex);
            };
            match opcode {
                0x14 => *current = stack[p as usize % STACK_LEN],
                0x15 => *current = IDENTITY,
                0x1B => {
                    *current = multiply(current, &scaling_xyz(params[..3].iter().map(|&p| fx32(p))))
                }
                0x1C => {
                    let mut translation = IDENTITY;
                    for (row, &p) in params[..3].iter().enumerate() {
                        translation[row][3] = fx32(p);
                    }
                    *current = multiply(current, &translation);
                }
                0x20 if !state.lit => vertex.color = bgr555(p as u16),
                0x21 => {
                    let normal = [0, 10, 20].map(|shift| signed(p >> shift, 10) / 512.0);
                    vertex.normal = transform(current, normal, 0.0);
                }
                0x22 => {
                    let [s, t] = [p as u16, (p >> 16) as u16].map(|c| c as i16 as f32 / 16.0);
                    vertex.uv = [0, 1]
                        .map(|i| [s, t][i] / state.size[i] * state.scale[i] + state.translation[i]);
                }
                0x23 => {
                    position = [p as u16, (p >> 16) as u16, params[1] as u16].map(fx16);
                    emit(position, &mut vertex);
                }
                0x24 => {
                    position = [0, 10, 20].map(|shift| signed(p >> shift, 10) / 64.0);
                    emit(position, &mut vertex);
                }
                0x25..=0x27 => {
                    let (a, b) = match opcode {
                        0x25 => (0, 1),
                        0x26 => (0, 2),
                        _ => (1, 2),
                    };
                    position[a] = fx16(p as u16);
                    position[b] = fx16((p >> 16) as u16);
                    emit(position, &mut vertex);
                }
                0x28 => {
                    for (i, shift) in [0, 10, 20].into_iter().enumerate() {
                        position[i] += signed(p >> shift, 10) / 4096.0;
                    }
                    emit(position, &mut vertex);
                }
                0x30 if p >> 15 & 1 != 0 && !state.lit => vertex.color = bgr555(p as u16),
                0x40 => {
                    triangles(kind, &group, out);
                    group.clear();
                    kind = p & 3;
                }
                0x41 => {
                    triangles(kind, &group, out);
                    group.clear();
                }
                _ => {}
            }
        }
    }
    triangles(kind, &group, out);
    Ok(())
}

/// Split a group of vertices into triangles, by primitive kind: 0 triangles, 1 quads,
/// 2 triangle strips, 3 quad strips.
fn triangles(kind: u32, group: &[Vertex], out: &mut Vec<Vertex>) {
    match kind {
        0 => out.extend(&group[..group.len() / 3 * 3]),
        1 => {
            for &[a, b, c, d] in group.as_chunks().0 {
                out.extend([a, b, c, a, c, d]);
            }
        }
        2 => {
            for (i, window) in group.windows(3).enumerate() {
                match i % 2 {
                    0 => out.extend([window[0], window[1], window[2]]),
                    _ => out.extend([window[1], window[0], window[2]]),
                }
            }
        }
        _ => {
            for pair in group.windows(4).step_by(2) {
                out.extend([pair[0], pair[1], pair[3], pair[0], pair[3], pair[2]]);
            }
        }
    }
}

/// A material as glTF sees it, decoding its texture once per texture, palette, and wrap.
fn convert_material(
    material: &super::Material,
    textures: &[&Textures],
    mesh: &mut Mesh,
    images: &mut HashMap<(String, Option<String>, [Wrap; 2]), usize>,
) -> Result<Material> {
    let wrap = [0, 1].map(|i| match (material.repeat[i], material.mirror[i]) {
        (false, _) => Wrap::Clamp,
        (true, false) => Wrap::Repeat,
        (true, true) => Wrap::Mirror,
    });
    let found = material.texture.as_deref().and_then(|name| {
        textures
            .iter()
            .find_map(|source| Some((*source, source.texture(name)?)))
    });
    let texture = match found {
        Some((source, texture)) => {
            let key = (texture.name.clone(), material.palette.clone(), wrap);
            match images.get(&key) {
                Some(&index) => Some(index),
                None => {
                    let palette = material
                        .palette
                        .as_deref()
                        .and_then(|name| source.palette(name))
                        .or_else(|| source.palette_for(texture));
                    let pixels: usize = mesh.textures.iter().map(|t| t.image.pixels.len()).sum();
                    if pixels + (texture.width * texture.height) as usize > MAX_PIXELS {
                        return Err(Error::malformed(
                            "NSBMD",
                            0,
                            "materials use too many textures",
                        ));
                    }
                    let image = match source.image(texture, palette) {
                        Err(Error::NotFound(_)) => None,
                        image => Some(image?),
                    };
                    image.map(|image| {
                        mesh.textures.push(Texture { image, wrap });
                        images.insert(key, mesh.textures.len() - 1);
                        mesh.textures.len() - 1
                    })
                }
            }
        }
        None => None,
    };
    let pixels = texture.map_or(&[][..], |t| &mesh.textures[t].image.pixels[..]);
    let alpha = match material.alpha {
        1..=30 => AlphaMode::Blend,
        _ if pixels.iter().any(|p| p[3] != 0 && p[3] != 255) => AlphaMode::Blend,
        _ if pixels.iter().any(|p| p[3] == 0) => AlphaMode::Mask,
        _ => AlphaMode::Opaque,
    };
    let opacity = match material.alpha {
        // Alpha 0 draws wireframe on the DS; shown solid.
        0 => 255,
        alpha => (alpha as u32 * 255 / 31) as u8,
    };
    // A viewer's lights stand in for the game's: under white lights, the DS sums the
    // diffuse and ambient reflections with the emission. The brighter reflection plus the
    // emission is close, and keeps materials lit only by ambient light from turning black.
    let lit = material.lights != 0;
    let reflected = [0, 1, 2].map(|i| {
        let reflection = material.diffuse[i].max(material.ambient[i]);
        reflection.saturating_add(material.emission[i])
    });
    let [r, g, b] = if lit { reflected } else { [255; 3] };
    Ok(Material {
        name: material.name.clone(),
        color: [r, g, b, opacity],
        texture,
        alpha,
        double_sided: material.back,
        unlit: !lit,
    })
}

fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    let mut product = [[0.0; 4]; 4];
    for (row, out) in product.iter_mut().enumerate() {
        for (column, value) in out.iter_mut().enumerate() {
            *value = (0..4).map(|k| a[row][k] * b[k][column]).sum();
        }
    }
    product
}

/// `matrix * (v, w)`: a point when `w` is 1, a direction when it is 0.
fn transform(matrix: &Matrix, v: [f32; 3], w: f32) -> [f32; 3] {
    [0, 1, 2].map(|row| (0..3).map(|k| matrix[row][k] * v[k]).sum::<f32>() + matrix[row][3] * w)
}

fn scaling(scale: f32) -> Matrix {
    scaling_xyz([scale; 3].into_iter())
}

fn scaling_xyz(scale: impl Iterator<Item = f32>) -> Matrix {
    let mut matrix = IDENTITY;
    for (i, scale) in scale.enumerate() {
        matrix[i][i] = scale;
    }
    matrix
}

/// The low `bits` bits of `value` as a signed number.
fn signed(value: u32, bits: u32) -> f32 {
    ((value << (32 - bits)) as i32 >> (32 - bits)) as f32
}

#[cfg(test)]
mod tests {
    use super::super::testing::{block, dictionary, file};
    use super::super::texture::tests::tex0;
    use super::super::{Nsbmd, Textures};
    use super::*;

    /// Display list commands packed four to a group, padded with no-ops.
    fn display_list(commands: &[(u8, &[u32])]) -> Vec<u8> {
        let mut list = Vec::new();
        for group in commands.chunks(4) {
            let mut opcodes = [0; 4];
            for (opcode, (command, _)) in opcodes.iter_mut().zip(group) {
                *opcode = *command;
            }
            list.extend(opcodes);
            for (_, params) in group {
                list.extend(params.iter().flat_map(|p| p.to_le_bytes()));
            }
        }
        list
    }

    /// A model with one node translating by +1 in x, a position scale of 2, and one
    /// material, textured `wall` and clamped vertically, drawing `list`.
    fn model(list: &[u8]) -> Vec<u8> {
        let fx = |value: f32| ((value * 4096.0) as i32 as u32).to_le_bytes();
        let mut model = vec![0; 0x40];
        let nodes = dictionary(&[("root", &[0, 0, 0, 0])]);
        let node_at = nodes.len();
        model.extend(&nodes);
        model[0x40 + node_at - 16 - 4..][..4].copy_from_slice(&(node_at as u32).to_le_bytes());
        model.extend(6u16.to_le_bytes()); // a translation only
        model.extend(0x1000u16.to_le_bytes());
        model.extend([fx(1.0), fx(0.0), fx(0.0)].concat());

        let commands = model.len();
        model.extend([0x06, 0, 0, 0, 0x0B, 0x04, 0, 0x05, 0, 0x2B, 0x01, 0]);

        let materials = model.len();
        let users = |at: usize| [(at as u16).to_le_bytes(), [1, 0]].concat();
        // Every dictionary here has one 4-byte entry, so they all have the same length.
        let entry = dictionary(&[("", &[0; 4])]).len();
        let (material_at, textures_at) = (4 + entry, 4 + entry + 0x2C);
        let (palettes_at, user_list) = (textures_at + entry, textures_at + 2 * entry);
        let section_len = user_list + 4;
        let material_dictionary = dictionary(&[("mat", &(material_at as u32).to_le_bytes())]);
        let texture_dictionary = dictionary(&[("wall", &users(user_list))]);
        let palette_dictionary = dictionary(&[("wall", &users(user_list))]);
        model.extend((textures_at as u16).to_le_bytes());
        model.extend((palettes_at as u16).to_le_bytes());
        model.extend(&material_dictionary);
        let colors = 0x7FFFu32 | 1 << 15 | 0x7FFF << 16;
        let polygon = 1u32 << 7 | 31 << 16;
        model.extend([0, 0, 0x2C, 0]);
        for word in [colors, 0, polygon, !0, 1 << 16, !0] {
            model.extend(word.to_le_bytes());
        }
        model.extend([0, 0, 0x0E, 0, 8, 0, 8, 0]);
        model.extend([fx(1.0), fx(1.0)].concat());
        model.extend(&texture_dictionary);
        model.extend(&palette_dictionary);
        model.extend([0, 0, 0, 0]); // the user list: material 0
        assert_eq!(model.len() - materials, section_len);

        let shapes = model.len();
        let shape_dictionary = dictionary(&[("shape", &[0, 0, 0, 0])]);
        model.extend(&shape_dictionary);
        let shape_at = shape_dictionary.len();
        model[shapes + shape_at - 16 - 4..][..4].copy_from_slice(&(shape_at as u32).to_le_bytes());
        model.extend([0, 0, 0x10, 0, 0, 0, 0, 0, 0x10, 0, 0, 0]);
        model.extend((list.len() as u32).to_le_bytes());
        model.extend(list);

        let len = model.len();
        for (at, value) in [
            (0, len),
            (4, commands),
            (8, materials),
            (0xC, shapes),
            (0x10, len),
        ] {
            model[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes());
        }
        model[0x17..0x1A].copy_from_slice(&[1, 1, 1]);
        model[0x1C..0x24].copy_from_slice(&[fx(2.0), fx(0.5)].concat());
        model[0x38..0x3C].copy_from_slice(&fx(1.0));

        let models = dictionary(&[("house", &[0, 0, 0, 0])]);
        let mut mdl0 = models.clone();
        let model_at = 8 + models.len();
        mdl0[models.len() - 16 - 4..][..4].copy_from_slice(&(model_at as u32).to_le_bytes());
        mdl0.extend(model);
        let texels = [0x11; 32]; // every texel is color 1
        let textures = tex0(&[("wall", 3, 0, &texels)], ("wall", &[0, 0x001F]));
        file(b"BMD0", &[block(b"MDL0", &mdl0), block(b"TEX0", &textures)])
    }

    #[test]
    fn draws_through_nodes_scale_and_materials() {
        let fx16 = |value: f32| (value * 4096.0) as i16 as u16 as u32;
        let texcoord = |s: u32, t: u32| (s * 16) | ((t * 16) << 16);
        let list = display_list(&[
            (0x40, &[1]), // quads
            (0x22, &[texcoord(0, 0)]),
            (0x23, &[0, 0]),
            (0x22, &[texcoord(8, 0)]),
            (0x24, &[64]), // (1, 0, 0) at 6 fractional bits
            (0x22, &[texcoord(8, 8)]),
            (0x25, &[fx16(1.0) | fx16(1.0) << 16]),
            (0x22, &[texcoord(0, 8)]),
            (0x26, &[0]), // x and z to 0, y stays 1
            (0x40, &[0]), // triangles
            (0x23, &[0, fx16(1.0)]),
            (0x28, &[256]),
            (0x28, &[256 << 10]),
            (0x41, &[]),
        ]);
        let nsbmd = Nsbmd::parse(&model(&list)).unwrap();
        let model = &nsbmd.models[0];
        assert_eq!((model.name.as_str(), model.scale), ("house", 2.0));
        assert_eq!(model.materials[0].texture.as_deref(), Some("wall"));
        let textures: Vec<&Textures> = nsbmd.textures.iter().collect();
        let mesh = model.mesh(&textures).unwrap();

        let vertices = &mesh.primitives[0].vertices;
        let positions: Vec<[f32; 3]> = vertices.iter().map(|v| v.position).collect();
        let (a, b, c, d) = (
            [1.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [3.0, 2.0, 0.0],
            [1.0, 2.0, 0.0],
        );
        let (e, f, g) = ([1.0, 0.0, 2.0], [1.125, 0.0, 2.0], [1.125, 0.125, 2.0]);
        assert_eq!(positions, [a, b, c, a, c, d, e, f, g]);
        assert_eq!(vertices[2].uv, [1.0, 1.0]);

        let material = &mesh.materials[0];
        assert!(material.unlit && !material.double_sided);
        assert_eq!(material.alpha, AlphaMode::Opaque);
        let texture = &mesh.textures[material.texture.unwrap()];
        assert_eq!(texture.wrap, [Wrap::Repeat, Wrap::Clamp]);
        assert_eq!(texture.image.get(7, 7), Some([255, 0, 0, 255]));
        assert!(mesh.to_glb().is_ok());

        // Without its textures, the model is drawn untextured.
        assert_eq!(model.mesh(&[]).unwrap().materials[0].texture, None);
    }

    #[test]
    fn rejects_malformed_commands() {
        let unknown = display_list(&[(0x40, &[0]), (0x99, &[])]);
        let nsbmd = Nsbmd::parse(&model(&unknown)).unwrap();
        assert!(matches!(
            nsbmd.models[0].mesh(&[]),
            Err(Error::Malformed { .. })
        ));

        let truncated = display_list(&[(0x40, &[0]), (0x23, &[0, 0])]);
        let truncated = &truncated[..truncated.len() - 4];
        let nsbmd = Nsbmd::parse(&model(truncated)).unwrap();
        assert!(matches!(
            nsbmd.models[0].mesh(&[]),
            Err(Error::Malformed { .. })
        ));
    }
}
