//! The overworld in 3D: map terrain, buildings, and the textures they and the overworld
//! characters use. As assets:
//!
//! ```text
//! models/maps/<n>.glb                     one map block's terrain, 32×32 tiles
//! models/buildings/<n>.glb                buildings and props that maps place
//! models/furniture/<n>.glb                indoor objects, kept apart by HeartGold and SoulSilver
//! textures/maps/<set>/<name>.png          the texture sets that terrain is drawn with
//! textures/buildings/<set>/<name>.png     the texture sets that buildings are drawn with
//! textures/overworld/<n>/<name>.png       characters and objects, one texture per frame
//! ```
//!
//! Map terrain, and a few buildings, carry no textures of their own: the game draws them with
//! the texture set of the area being shown. Which set an area uses is recorded in tables this
//! module does not read (area data, and map headers in the ARM9 binary), so a model is drawn
//! with the set that holds the most of the textures it names, and the rest from the sets
//! that hold them.

use std::cmp::Reverse;
use std::sync::OnceLock;

use super::{Gen4, cached};
use crate::formats::narc::Narc;
use crate::formats::nitro3d::{Nsbmd, Textures};
use crate::games::GameId;
use crate::io::{ByteSource, Reader};
use crate::media::Mesh;
use crate::{Error, Result};

/// Where a game keeps its overworld models and textures.
struct Archives {
    maps: &'static str,
    /// HeartGold and SoulSilver map blocks have a background-sound section before the model.
    map_sounds: bool,
    buildings: &'static str,
    furniture: Option<&'static str>,
    map_textures: &'static str,
    building_textures: &'static str,
    overworld: &'static str,
}

const DIAMOND_PEARL: Archives = Archives {
    maps: "fielddata/land_data/land_data_release.narc",
    map_sounds: false,
    buildings: "fielddata/build_model/build_model.narc",
    furniture: None,
    map_textures: "fielddata/areadata/area_map_tex/map_tex_set.narc",
    building_textures: "fielddata/areadata/area_build_model/areabm_texset.narc",
    overworld: "data/mmodel/mmodel.narc",
};

const PLATINUM: Archives = Archives {
    maps: "fielddata/land_data/land_data.narc",
    ..DIAMOND_PEARL
};

/// HeartGold and SoulSilver number most files, but keep named copies of the building models.
const HEARTGOLD_SOULSILVER: Archives = Archives {
    maps: "a/0/6/5",
    map_sounds: true,
    buildings: "fielddata/build_model/bm_field.narc",
    furniture: Some("fielddata/build_model/bm_room.narc"),
    map_textures: "a/0/4/4",
    building_textures: "a/0/7/0",
    overworld: "a/0/8/1",
};

impl Archives {
    fn of(game: GameId) -> &'static Self {
        match game {
            GameId::PokemonPlatinum => &PLATINUM,
            GameId::PokemonHeartGold | GameId::PokemonSoulSilver => &HEARTGOLD_SOULSILVER,
            _ => &DIAMOND_PEARL,
        }
    }

    fn models(&self, set: ModelSet) -> Option<&'static str> {
        match set {
            ModelSet::Maps => Some(self.maps),
            ModelSet::Buildings => Some(self.buildings),
            ModelSet::Furniture => self.furniture,
        }
    }

    fn textures(&self, set: TextureSet) -> &'static str {
        match set {
            TextureSet::Maps => self.map_textures,
            TextureSet::Buildings => self.building_textures,
            TextureSet::Overworld => self.overworld,
        }
    }
}

/// The overworld's model archives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSet {
    Maps,
    Buildings,
    Furniture,
}

/// The overworld's texture archives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureSet {
    Maps,
    Buildings,
    Overworld,
}

const MODEL_SETS: [(ModelSet, &str); 3] = [
    (ModelSet::Maps, "maps"),
    (ModelSet::Buildings, "buildings"),
    (ModelSet::Furniture, "furniture"),
];

const TEXTURE_SETS: [(TextureSet, &str); 3] = [
    (TextureSet::Maps, "maps"),
    (TextureSet::Buildings, "buildings"),
    (TextureSet::Overworld, "overworld"),
];

/// The texture names of each member of each texture archive, read on first use.
#[derive(Default)]
pub(super) struct Cache {
    names: [OnceLock<Vec<Vec<String>>>; 3],
}

impl<S: ByteSource> Gen4<S> {
    /// The number of models in a set; zero for a set the game does not have.
    pub fn model_count(&self, set: ModelSet) -> Result<usize> {
        match Archives::of(self.game).models(set) {
            Some(archive) => Ok(Narc::parse(&self.rom.file(archive)?)?.len()),
            None => Ok(0),
        }
    }

    /// One overworld model, with its own textures, and those it lacks from the texture sets
    /// that hold the most of them.
    pub fn model(&self, set: ModelSet, index: usize) -> Result<Mesh> {
        let archives = Archives::of(self.game);
        let archive = archives
            .models(set)
            .ok_or_else(|| Error::NotFound(format!("{set:?} models")))?;
        let file = self.rom.file(archive)?;
        let member = Narc::parse(&file)?.member(&file, index)?;
        let bytes = match set {
            ModelSet::Maps => terrain(&member, archives.map_sounds)?,
            _ => &member,
        };
        let nsbmd = Nsbmd::parse(bytes)?;
        let model = nsbmd
            .models
            .first()
            .ok_or_else(|| Error::malformed("NSBMD", 0, "holds no model"))?;
        let own: Vec<&Textures> = nsbmd.textures.iter().collect();
        let missing: Vec<&str> = model
            .materials
            .iter()
            .map(|material| material.texture_name())
            .filter(|name| !own.iter().any(|textures| textures.texture(name).is_some()))
            .collect();
        let texture_set = match set {
            ModelSet::Maps => TextureSet::Maps,
            _ => TextureSet::Buildings,
        };
        let shared = self.shared_textures(texture_set, missing)?;
        model.mesh(&own.into_iter().chain(&shared).collect::<Vec<_>>())
    }

    /// The texture sets of an archive, by member; members without textures are empty.
    pub fn texture_names(&self, set: TextureSet) -> Result<&[Vec<String>]> {
        let index = TEXTURE_SETS
            .iter()
            .position(|&(s, _)| s == set)
            .expect("listed");
        cached(&self.field.names[index], || {
            let file = self.rom.file(Archives::of(self.game).textures(set))?;
            let narc = Narc::parse(&file)?;
            (0..narc.len())
                .map(|member| {
                    let member = narc.member_source(&file, member)?;
                    match &*member.read_up_to(0, 4)? {
                        b"BTX0" => Textures::names(&member),
                        // Overworld archives also hold a few models and other data.
                        _ => Ok(Vec::new()),
                    }
                })
                .collect()
        })
        .map(Vec::as_slice)
    }

    /// One member of a texture archive.
    pub fn textures(&self, set: TextureSet, member: usize) -> Result<Textures> {
        let file = self.rom.file(Archives::of(self.game).textures(set))?;
        Textures::parse(&Narc::parse(&file)?.member(&file, member)?)
    }

    /// Sets holding `wanted` textures: the one holding the most (the first of equals), then
    /// the same for those still missing, until no set holds any.
    fn shared_textures(&self, set: TextureSet, mut wanted: Vec<&str>) -> Result<Vec<Textures>> {
        let names = self.texture_names(set)?;
        let mut sets = Vec::new();
        while !wanted.is_empty() {
            let held = |names: &[String]| {
                wanted
                    .iter()
                    .filter(|w| names.iter().any(|n| n == *w))
                    .count()
            };
            let best = names
                .iter()
                .enumerate()
                .map(|(member, names)| (held(names), Reverse(member)))
                .max()
                .filter(|&(held, _)| held > 0);
            let Some((_, Reverse(member))) = best else {
                break;
            };
            wanted.retain(|w| !names[member].iter().any(|n| n == w));
            sets.push(self.textures(set, member)?);
        }
        Ok(sets)
    }

    pub(super) fn field_assets(&self) -> Result<Vec<String>> {
        let mut assets = Vec::new();
        for (set, directory) in MODEL_SETS {
            let count = self.model_count(set)?;
            assets.extend((0..count).map(|index| format!("models/{directory}/{index:04}.glb")));
        }
        for (set, directory) in TEXTURE_SETS {
            for (member, names) in self.texture_names(set)?.iter().enumerate() {
                let mut files: Vec<String> = Vec::new();
                for file in names.iter().map(|name| file_name(name)) {
                    // A name twice in one set could not be told apart; the first is kept.
                    if !files.contains(&file) {
                        files.push(file);
                    }
                }
                assets.extend(
                    files
                        .into_iter()
                        .map(|file| format!("textures/{directory}/{member:04}/{file}.png")),
                );
            }
        }
        Ok(assets)
    }

    /// An asset of this module, or `None` when `path` is not one.
    pub(super) fn extract_field(&self, path: &str) -> Option<Result<Vec<u8>>> {
        if let Some(path) = path.strip_prefix("models/") {
            let (directory, file) = path.split_once('/')?;
            let (set, _) = MODEL_SETS.into_iter().find(|&(_, d)| d == directory)?;
            let index = file.strip_suffix(".glb")?.parse().ok()?;
            return Some(self.model(set, index).and_then(|mesh| mesh.to_glb()));
        }
        let (directory, path) = path.strip_prefix("textures/")?.split_once('/')?;
        let (set, _) = TEXTURE_SETS.into_iter().find(|&(_, d)| d == directory)?;
        let (member, file) = path.split_once('/')?;
        let (member, file) = (member.parse().ok()?, file.strip_suffix(".png")?);
        let png = || {
            let textures = self.textures(set, member)?;
            let texture = textures
                .textures
                .iter()
                .find(|texture| file_name(&texture.name) == file)
                .ok_or_else(|| {
                    Error::NotFound(format!("texture {file} in {directory} {member}"))
                })?;
            textures
                .image(texture, textures.palette_for(texture))?
                .to_png()
        };
        Some(png())
    }
}

/// The terrain model of a map block. A block starts with the sizes of its sections (tile
/// permissions, building placements, the terrain model, terrain heights), followed by the
/// sections in that order; HeartGold and SoulSilver add a background-sound section after
/// the sizes, starting with a tag and its own size.
fn terrain(block: &[u8], sounds: bool) -> Result<&[u8]> {
    let block = Reader::new("map block", block);
    let header = match sounds {
        true => 20 + block.u16_at(18)? as usize,
        false => 16,
    };
    let start = header
        .saturating_add(block.u32_at(0)? as usize)
        .saturating_add(block.u32_at(4)? as usize);
    block.slice_at(start, block.u32_at(8)? as usize)
}

/// A texture name as a file name: characters other than letters, digits, `.`, `-`, and `_`
/// become `_`.
fn file_name(name: &str) -> String {
    name.chars()
        .map(|c| match c.is_ascii_alphanumeric() || "._-".contains(c) {
            true => c,
            false => '_',
        })
        .collect()
}
