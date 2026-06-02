//! Checks against real ROM dumps, which are never committed. Point `ROMKIT_ROMS` at a folder of
//! dumps to run them (`ROMKIT_ROMS=~/roms cargo test -p romkit --test roms -- --nocapture`);
//! without it they pass vacuously.

use std::path::{Path, PathBuf};

use romkit::ByteSource;
use romkit::formats::{Format, narc::Narc};
use romkit::games::pokemon::gen4::text::{Text, Token};
use romkit::games::pokemon::gen4::{Gen4, ModelSet, MoveClass, SpeciesId, TextureSet, Type};
use romkit::io::FileSource;
use romkit::platforms::nds::NdsRom;
use romkit::{GameId, Platform, Rom, identify};

/// Every file in `ROMKIT_ROMS`, sorted by name.
fn dumps() -> Vec<PathBuf> {
    let Some(dir) = std::env::var_os("ROMKIT_ROMS") else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("ROMKIT_ROMS is a readable directory")
        .map(|entry| entry.expect("directory entry").path())
        .collect();
    paths.sort();
    paths
}

fn name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

fn open(path: &Path) -> FileSource {
    FileSource::open(path).expect("readable dump")
}

#[test]
fn identifies_every_platform() {
    for path in dumps() {
        let name = name(&path);
        let expected = match path.extension().and_then(|ext| ext.to_str()) {
            // Yellow is a Color-enhanced cartridge with a .gb extension.
            Some("gb") if name.contains("Yellow") => Platform::GameBoyColor,
            Some("gb") => Platform::GameBoy,
            Some("gbc") => Platform::GameBoyColor,
            Some("gba") => Platform::GameBoyAdvance,
            Some("nds") => Platform::NintendoDs,
            Some("3ds") => Platform::Nintendo3ds,
            _ => continue,
        };
        let identity = identify(&open(&path)).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(identity.platform, expected, "{name}");
        let game = GameId::identify(&identity).unwrap_or_else(|| panic!("{name}: unknown game"));
        println!("{name}: {identity} = {}", game.name());
    }
}

#[test]
fn reads_every_ds_filesystem_banner_and_archive() {
    for path in dumps()
        .into_iter()
        .filter(|path| path.extension() == Some("nds".as_ref()))
    {
        let name = name(&path);
        let rom = NdsRom::open(open(&path)).unwrap_or_else(|error| panic!("{name}: {error}"));
        let banner = rom.banner().unwrap().expect("retail games have a banner");
        assert!(
            banner.english.to_lowercase().starts_with("pokémon"),
            "{name}: {:?}",
            banner.english
        );

        let mut archives = 0;
        for (file, _) in rom.fs().paths() {
            let source = rom.file(file).unwrap();
            if Format::sniff(&source.read_up_to(0, 4).unwrap())
                .is_some_and(|f| f.extension == "narc")
            {
                Narc::parse(&source).unwrap_or_else(|error| panic!("{name}: {file}: {error}"));
                archives += 1;
            }
        }
        println!(
            "{name}: {} files, {archives} archives",
            rom.fs().paths().len()
        );
    }
}

#[test]
fn decodes_all_platinum_text() {
    for path in dumps()
        .into_iter()
        .filter(|path| name(path).contains("Platinum"))
    {
        let rom = NdsRom::open(open(&path)).unwrap();
        let archive = rom.file("msgdata/pl_msg.narc").unwrap();
        let narc = Narc::parse(&archive).unwrap();
        let text = Text::new(&archive, &narc, "");
        let mut messages = 0;
        for bank in 0..text.len() {
            for message in text.bank(bank).unwrap() {
                assert!(
                    !message
                        .tokens
                        .iter()
                        .any(|token| matches!(token, Token::Unknown(_)))
                );
                messages += 1;
            }
        }
        assert_eq!(text.strings(412).unwrap()[25], "PIKACHU");
        println!("{}: {} banks, {messages} messages", name(&path), text.len());
    }
}

#[test]
fn extracts_gen4_data() {
    let gen4 = ["Diamond", "Pearl", "Platinum", "HeartGold", "SoulSilver"];
    for path in dumps()
        .into_iter()
        .filter(|path| gen4.iter().any(|game| name(path).contains(game)))
    {
        let game = Gen4::open(open(&path)).unwrap();
        let species = game.species().unwrap();
        let pikachu = &species[24];
        assert_eq!(pikachu.name.as_deref(), Some("PIKACHU"));
        assert_eq!(pikachu.types, [Type::Electric, Type::Electric]);
        assert_eq!((pikachu.base_stats.hp, pikachu.base_stats.speed), (35, 90));
        assert_eq!(pikachu.evolutions[0].into, SpeciesId(26));
        assert!(pikachu.machines.contains(&23), "TM24 Thunderbolt");

        let thunderbolt = &game.moves().unwrap()[84];
        assert_eq!(thunderbolt.name.as_deref(), Some("Thunderbolt"));
        assert_eq!(
            (thunderbolt.power, thunderbolt.class),
            (95, MoveClass::Special)
        );

        let abilities = game.abilities().unwrap();
        let static_ = &abilities[8];
        assert_eq!(static_.name, "Static");
        assert!(static_.species.contains(&SpeciesId(25)));
        println!(
            "{}: {} species, {} moves, {} abilities; {}",
            name(&path),
            species.len(),
            game.moves().unwrap().len(),
            abilities.len(),
            serde_json::to_string(&pikachu.evolutions).unwrap()
        );
    }
}

#[test]
fn opens_every_dump_and_exports_supported_games() {
    for path in dumps() {
        let name = name(&path);
        let rom = Rom::open(open(&path)).unwrap_or_else(|error| panic!("{name}: {error}"));
        let overview = rom.overview().unwrap();
        let files = rom.files().unwrap();
        if !overview.supported {
            println!("{name}: {} files, not supported", files.len());
            continue;
        }
        let mut written = std::collections::BTreeMap::new();
        let manifest = rom
            .export(|path, contents| {
                written.insert(path.to_owned(), contents.len());
                Ok(())
            })
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(written.len(), manifest.assets.len() + 1, "{name}");
        let bytes: usize = written.values().sum();
        println!(
            "{name}: {} assets, {bytes} bytes, crc32 {}",
            manifest.assets.len(),
            manifest.crc32
        );
    }
}

#[test]
fn converts_every_gen4_overworld_model_to_valid_gltf() {
    let gen4 = ["Diamond", "Pearl", "Platinum", "HeartGold", "SoulSilver"];
    for path in dumps()
        .into_iter()
        .filter(|path| gen4.iter().any(|game| name(path).contains(game)))
    {
        let name = name(&path);
        let game = Gen4::open(open(&path)).unwrap();
        let hgss = name.contains("HeartGold") || name.contains("SoulSilver");
        let expected = match () {
            _ if hgss => [676, 340, 222],
            _ if name.contains("Platinum") => [666, 590, 0],
            _ => [578, 535, 0],
        };
        let sets = [ModelSet::Maps, ModelSet::Buildings, ModelSet::Furniture];
        let mut names = Vec::new();
        for (set, expected) in sets.into_iter().zip(expected) {
            let count = game.model_count(set).unwrap();
            assert_eq!(count, expected, "{name}: {set:?}");
            let (mut materials, mut textured, mut narrowest) = (0, 0, f32::MAX);
            for index in 0..count {
                let mesh = game
                    .model(set, index)
                    .unwrap_or_else(|error| panic!("{name}: {set:?} {index}: {error}"));
                let (min, max) = mesh.bounds().expect("models have vertices");
                assert!(
                    min.iter().chain(&max).all(|c| c.abs() <= 1024.0),
                    "{name}: {set:?} {index}: {min:?} {max:?}"
                );
                narrowest = narrowest.min((max[0] - min[0]).max(max[2] - min[2]));
                materials += mesh.materials.len();
                textured += mesh
                    .materials
                    .iter()
                    .filter(|m| m.texture.is_some())
                    .count();
                names.push(mesh.name.clone());
                check_glb(&mesh, &mesh.to_glb().unwrap());
            }
            if count > 0 {
                println!(
                    "{name}: {count} {set:?}, {textured} of {materials} materials textured, narrowest {narrowest}"
                );
                // The rest are untextured by design: shadows, lights, placeholders.
                assert!(textured * 100 >= materials * 85, "{name}: {set:?}");
            }
        }
        // The Pokémon Center.
        assert!(names.iter().any(|name| name == "pc"), "{name}");
    }
}

/// The GLB parses and validates, and holds the mesh's vertices and textures.
fn check_glb(mesh: &romkit::media::Mesh, glb: &[u8]) {
    let gltf = gltf::Gltf::from_slice(glb).expect("valid glTF");
    let blob = gltf.blob.as_deref().unwrap_or_default();
    let mut vertices = 0;
    for primitive in gltf.meshes().flat_map(|mesh| mesh.primitives()) {
        let reader = primitive.reader(|_| Some(blob));
        let positions = reader.read_positions().expect("positions").count();
        let indices = reader.read_indices().expect("indices").into_u32();
        assert!(indices.clone().all(|index| (index as usize) < positions));
        vertices += indices.count();
    }
    let expected: usize = mesh.primitives.iter().map(|p| p.vertices.len()).sum();
    assert_eq!(vertices, expected);
    assert_eq!(gltf.images().count(), mesh.textures.len());
}

#[test]
fn decodes_gen4_overworld_textures() {
    let gen4 = ["Diamond", "Pearl", "Platinum", "HeartGold", "SoulSilver"];
    for path in dumps()
        .into_iter()
        .filter(|path| gen4.iter().any(|game| name(path).contains(game)))
    {
        let name = name(&path);
        let game = Gen4::open(open(&path)).unwrap();
        let mut count = 0;
        for set in [
            TextureSet::Maps,
            TextureSet::Buildings,
            TextureSet::Overworld,
        ] {
            for (member, names) in game.texture_names(set).unwrap().iter().enumerate() {
                if names.is_empty() {
                    continue;
                }
                let textures = game.textures(set, member).unwrap();
                for texture in &textures.textures {
                    let image = textures
                        .image(texture, textures.palette_for(texture))
                        .unwrap_or_else(|error| panic!("{name}: {set:?} {member}: {error}"));
                    assert_eq!(
                        image.pixels.len(),
                        (texture.width * texture.height) as usize
                    );
                    count += 1;
                }
            }
        }
        // A child walking: 16 frames of 32×32, transparent around the sprite.
        let child = game.textures(TextureSet::Overworld, 0).unwrap();
        assert_eq!(child.textures.len(), 16, "{name}");
        let first = &child.textures[0];
        assert_eq!(
            (first.name.as_str(), first.width, first.height),
            ("babyboy1.1", 32, 32)
        );
        let image = child.image(first, child.palette_for(first)).unwrap();
        assert_eq!(image.get(0, 0).unwrap()[3], 0, "{name}");
        assert!(
            image.pixels.iter().filter(|p| p[3] == 255).count() > 100,
            "{name}"
        );
        println!("{name}: {count} textures");
    }
}
