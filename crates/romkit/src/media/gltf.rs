//! Binary glTF 2.0 (`.glb`), the format every 3D viewer, engine, and browser library reads.
//!
//! A GLB is a 12-byte header, a JSON chunk describing the scene, and a binary chunk holding
//! the vertex data and the PNG images. Each mesh is one node of one scene. Textures are sampled
//! nearest-neighbor: the pixels of handheld textures are meant to be seen.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::{AlphaMode, Mesh, Vertex, Wrap};
use crate::Result;

const FLOAT: u32 = 5126;
const UNSIGNED_BYTE: u32 = 5121;
const UNSIGNED_SHORT: u32 = 5123;
const UNSIGNED_INT: u32 = 5125;
const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
const NEAREST: u32 = 9728;
const WHITE: [u8; 4] = [255; 4];

pub(super) fn glb(mesh: &Mesh) -> Result<Vec<u8>> {
    let mut buffer = Buffer::default();
    let mut primitives = Vec::new();
    for primitive in &mesh.primitives {
        let triangles = primitive.vertices.len() / 3 * 3;
        if triangles == 0 {
            continue;
        }
        let (vertices, indices) = merge(&primitive.vertices[..triangles]);
        let material = primitive.material.filter(|&m| m < mesh.materials.len());
        let textured = material.is_some_and(|m| mesh.materials[m].texture.is_some());

        let positions: Vec<[f32; 3]> = vertices.iter().map(|v| v.position).collect();
        let (min, max) = positions.iter().fold(
            ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]),
            |(min, max), p| {
                (
                    [0, 1, 2].map(|i| min[i].min(p[i])),
                    [0, 1, 2].map(|i| max[i].max(p[i])),
                )
            },
        );
        let mut attributes = json!({
            "POSITION": buffer.accessor(&floats(&positions), FLOAT, "VEC3", vertices.len(), ARRAY_BUFFER)
        });
        let position = &mut buffer.accessors.last_mut().expect("just added");
        position["min"] = json!(min);
        position["max"] = json!(max);
        if vertices.iter().all(|v| v.normal != [0.0; 3]) {
            let normals: Vec<[f32; 3]> = vertices.iter().map(|v| normalize(v.normal)).collect();
            attributes["NORMAL"] = buffer.accessor(
                &floats(&normals),
                FLOAT,
                "VEC3",
                normals.len(),
                ARRAY_BUFFER,
            );
        }
        if textured {
            let uvs: Vec<[f32; 2]> = vertices.iter().map(|v| v.uv).collect();
            attributes["TEXCOORD_0"] =
                buffer.accessor(&floats(&uvs), FLOAT, "VEC2", uvs.len(), ARRAY_BUFFER);
        }
        if vertices.iter().any(|v| v.color != WHITE) {
            let colors: Vec<u8> = vertices.iter().flat_map(|v| v.color).collect();
            attributes["COLOR_0"] =
                buffer.accessor(&colors, UNSIGNED_BYTE, "VEC4", vertices.len(), ARRAY_BUFFER);
            buffer.accessors.last_mut().expect("just added")["normalized"] = json!(true);
        }
        let indices = match vertices.len() <= u16::MAX as usize {
            true => {
                let bytes: Vec<u8> = indices
                    .iter()
                    .flat_map(|&i| (i as u16).to_le_bytes())
                    .collect();
                buffer.accessor(
                    &bytes,
                    UNSIGNED_SHORT,
                    "SCALAR",
                    indices.len(),
                    ELEMENT_ARRAY_BUFFER,
                )
            }
            false => {
                let bytes: Vec<u8> = indices.iter().flat_map(|&i| i.to_le_bytes()).collect();
                buffer.accessor(
                    &bytes,
                    UNSIGNED_INT,
                    "SCALAR",
                    indices.len(),
                    ELEMENT_ARRAY_BUFFER,
                )
            }
        };
        let mut primitive = json!({ "attributes": attributes, "indices": indices });
        if let Some(material) = material {
            primitive["material"] = json!(material);
        }
        primitives.push(primitive);
    }

    let mut images = Vec::new();
    let mut samplers = Vec::new();
    let mut textures = Vec::new();
    for (index, texture) in mesh.textures.iter().enumerate() {
        let view = buffer.view(&texture.image.to_png()?, None);
        images.push(json!({ "bufferView": view, "mimeType": "image/png" }));
        let [s, t] = texture.wrap.map(|wrap| match wrap {
            Wrap::Clamp => 33071,
            Wrap::Repeat => 10497,
            Wrap::Mirror => 33648,
        });
        samplers
            .push(json!({ "magFilter": NEAREST, "minFilter": NEAREST, "wrapS": s, "wrapT": t }));
        textures.push(json!({ "source": index, "sampler": index }));
    }

    let materials: Vec<Value> = mesh
        .materials
        .iter()
        .map(|material| {
            let mut pbr = json!({
                "baseColorFactor": material.color.map(|c| c as f32 / 255.0),
                "metallicFactor": 0.0,
                "roughnessFactor": 1.0,
            });
            if let Some(texture) = material.texture.filter(|&t| t < mesh.textures.len()) {
                pbr["baseColorTexture"] = json!({ "index": texture });
            }
            let mut value = json!({
                "name": material.name,
                "pbrMetallicRoughness": pbr,
                "alphaMode": match material.alpha {
                    AlphaMode::Opaque => "OPAQUE",
                    AlphaMode::Mask => "MASK",
                    AlphaMode::Blend => "BLEND",
                },
                "doubleSided": material.double_sided,
            });
            if material.unlit {
                value["extensions"] = json!({ "KHR_materials_unlit": {} });
            }
            value
        })
        .collect();

    let mut node = json!({ "name": mesh.name });
    let mut document = json!({
        "asset": { "version": "2.0", "generator": concat!("romkit ", env!("CARGO_PKG_VERSION")) },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
    });
    if !primitives.is_empty() {
        node["mesh"] = json!(0);
        document["meshes"] = json!([{ "name": mesh.name, "primitives": primitives }]);
    }
    document["nodes"] = json!([node]);
    for (key, list) in [
        ("materials", materials),
        ("textures", textures),
        ("samplers", samplers),
        ("images", images),
    ] {
        if !list.is_empty() {
            document[key] = Value::Array(list);
        }
    }
    if mesh.materials.iter().any(|material| material.unlit) {
        document["extensionsUsed"] = json!(["KHR_materials_unlit"]);
    }
    if !buffer.bytes.is_empty() {
        document["buffers"] = json!([{ "byteLength": buffer.bytes.len() }]);
        document["bufferViews"] = Value::Array(buffer.views);
        document["accessors"] = Value::Array(buffer.accessors);
    }

    let mut json = serde_json::to_vec(&document)?;
    pad(&mut json, b' ');
    let mut bin = buffer.bytes;
    pad(&mut bin, 0);
    let chunks = 8 + json.len() + if bin.is_empty() { 0 } else { 8 + bin.len() };
    let mut glb = Vec::with_capacity(12 + chunks);
    glb.extend(b"glTF");
    glb.extend(2u32.to_le_bytes());
    glb.extend((12 + chunks as u32).to_le_bytes());
    for (kind, chunk) in [(b"JSON", &json), (b"BIN\0", &bin)] {
        if !chunk.is_empty() {
            glb.extend((chunk.len() as u32).to_le_bytes());
            glb.extend(kind);
            glb.extend(chunk);
        }
    }
    Ok(glb)
}

/// The binary chunk being built, with the views and accessors that describe it.
#[derive(Default)]
struct Buffer {
    bytes: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Buffer {
    /// Append `bytes` as a view, 4-byte aligned as every accessor requires.
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        pad(&mut self.bytes, 0);
        let mut view =
            json!({ "buffer": 0, "byteOffset": self.bytes.len(), "byteLength": bytes.len() });
        if let Some(target) = target {
            view["target"] = json!(target);
        }
        self.bytes.extend(bytes);
        self.views.push(view);
        self.views.len() - 1
    }

    /// Append `bytes` as a view and an accessor over it, returning the accessor's index.
    fn accessor(
        &mut self,
        bytes: &[u8],
        component: u32,
        kind: &str,
        count: usize,
        target: u32,
    ) -> Value {
        let view = self.view(bytes, Some(target));
        self.accessors.push(json!({
            "bufferView": view,
            "componentType": component,
            "type": kind,
            "count": count,
        }));
        json!(self.accessors.len() - 1)
    }
}

/// Distinct vertices, and the triangle list as indices into them.
fn merge(triangles: &[Vertex]) -> (Vec<Vertex>, Vec<u32>) {
    let mut seen = HashMap::new();
    let mut vertices = Vec::new();
    let indices = triangles
        .iter()
        .map(|vertex| {
            let bits = (
                vertex.position.map(f32::to_bits),
                vertex.normal.map(f32::to_bits),
                vertex.uv.map(f32::to_bits),
                vertex.color,
            );
            *seen.entry(bits).or_insert_with(|| {
                vertices.push(*vertex);
                vertices.len() as u32 - 1
            })
        })
        .collect();
    (vertices, indices)
}

fn floats<const N: usize>(values: &[[f32; N]]) -> Vec<u8> {
    values
        .iter()
        .flatten()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|c| c / length)
}

fn pad(bytes: &mut Vec<u8>, with: u8) {
    bytes.resize(bytes.len().next_multiple_of(4), with);
}

#[cfg(test)]
mod tests {
    use super::super::{Image, Material, Primitive, Texture};
    use super::*;

    fn vertex(position: [f32; 3], color: [u8; 4]) -> Vertex {
        Vertex {
            position,
            normal: [0.0, 0.0, 1.0],
            uv: [position[0], position[1]],
            color,
        }
    }

    fn chunk(glb: &[u8], at: usize) -> (&[u8], &[u8]) {
        let len = u32::from_le_bytes(glb[at..at + 4].try_into().unwrap()) as usize;
        (&glb[at + 4..at + 8], &glb[at + 8..at + 8 + len])
    }

    #[test]
    fn writes_a_textured_quad_with_shared_vertices() {
        let [a, b, c, d] = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ]
        .map(|p| vertex(p, WHITE));
        let mesh = Mesh {
            name: "quad".into(),
            primitives: vec![Primitive {
                material: Some(0),
                vertices: vec![a, b, c, a, c, d],
            }],
            materials: vec![Material {
                name: "wall".into(),
                color: WHITE,
                texture: Some(0),
                alpha: AlphaMode::Mask,
                double_sided: false,
                unlit: true,
            }],
            textures: vec![Texture {
                image: Image::new(2, 2),
                wrap: [Wrap::Repeat, Wrap::Mirror],
            }],
        };
        let glb = mesh.to_glb().unwrap();
        assert_eq!(&glb[..4], b"glTF");
        assert_eq!(
            u32::from_le_bytes(glb[8..12].try_into().unwrap()) as usize,
            glb.len()
        );
        let (kind, json) = chunk(&glb, 12);
        assert_eq!(kind, b"JSON");
        let document: Value = serde_json::from_slice(json).unwrap();
        let (kind, bin) = chunk(&glb, 20 + json.len());
        assert_eq!(kind, b"BIN\0");
        // The chunk is padded to 4 bytes; the buffer is not.
        assert_eq!(
            document["buffers"][0]["byteLength"].as_u64().unwrap() as usize,
            bin.len() - 1
        );

        let primitive = &document["meshes"][0]["primitives"][0];
        let accessor = |name: &str| {
            &document["accessors"][primitive["attributes"][name].as_u64().unwrap() as usize]
        };
        assert_eq!(accessor("POSITION")["count"], 4);
        assert_eq!(accessor("POSITION")["max"], json!([1.0, 1.0, 0.0]));
        assert!(primitive["attributes"].get("COLOR_0").is_none());
        assert_eq!(
            document["accessors"][primitive["indices"].as_u64().unwrap() as usize]["count"],
            6
        );
        assert_eq!(document["samplers"][0]["wrapT"], 33648);
        assert_eq!(document["materials"][0]["alphaMode"], "MASK");
        assert_eq!(document["extensionsUsed"], json!(["KHR_materials_unlit"]));
        let image = &document["bufferViews"]
            [document["images"][0]["bufferView"].as_u64().unwrap() as usize];
        let start = image["byteOffset"].as_u64().unwrap() as usize;
        assert_eq!(&bin[start..start + 4], b"\x89PNG");
    }

    #[test]
    fn writes_an_empty_mesh_as_a_bare_node() {
        let glb = Mesh::default().to_glb().unwrap();
        let (_, json) = chunk(&glb, 12);
        let document: Value = serde_json::from_slice(json).unwrap();
        assert!(document.get("meshes").is_none() && document.get("buffers").is_none());
        assert_eq!(glb.len(), 20 + json.len());
    }
}
