//! Checks against real ROM dumps, which are never committed. Point `ROMKIT_ROMS` at a folder of
//! dumps to run them (`ROMKIT_ROMS=~/roms cargo test -p romkit --test roms -- --nocapture`);
//! without it they pass vacuously.

use std::path::{Path, PathBuf};

use romkit::ByteSource;
use romkit::formats::g2d::{Cells, Palette, Screen, Tiles};
use romkit::formats::sdat::Sdat;
use romkit::formats::{Format, lz, narc::Narc};
use romkit::games::pokemon::gen4::text::{Text, Token};
use romkit::games::pokemon::gen4::{Gen4, ItemId, MoveClass, SpeciesId, Type, Variant};
use romkit::games::pokemon::gen5::{self, Gen5};
use romkit::games::sound::Sound;
use romkit::io::FileSource;
use romkit::media::{Audio, Image};
use romkit::platforms::nds::NdsRom;
use romkit::{Game, GameId, Platform, Rom, identify};

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

/// Every NCLR, NCGR, NCER, and NSCR in every DS dump parses, whether stored as a file or an
/// archive member, compressed or not.
#[test]
fn parses_every_ds_2d_graphic() {
    for path in dumps()
        .into_iter()
        .filter(|path| path.extension() == Some("nds".as_ref()))
    {
        let name = name(&path);
        let rom = NdsRom::open(open(&path)).unwrap();
        let (mut parsed, mut compressed) = (std::collections::BTreeMap::new(), 0);
        for (file, _) in rom.fs().paths() {
            let bytes = rom.read(file).unwrap();
            let members = match Narc::parse(bytes.as_ref()) {
                Ok(narc) => (0..narc.len())
                    .map(|index| narc.member(bytes.as_ref(), index).unwrap().into_owned())
                    .collect(),
                Err(_) => vec![bytes.into_owned()],
            };
            for (index, member) in members.iter().enumerate() {
                let member = lz::decompress_if_compressed(member);
                compressed += matches!(member, std::borrow::Cow::Owned(_)) as usize;
                let Some(format) = Format::sniff(&member) else {
                    continue;
                };
                let result = match format.extension {
                    "nclr" => Palette::parse(&member).map(drop),
                    "ncgr" => Tiles::parse(&member).map(drop),
                    "ncer" => Cells::parse(&member).map(drop),
                    "nscr" => Screen::parse(&member).map(drop),
                    _ => continue,
                };
                result.unwrap_or_else(|error| panic!("{name}: {file} #{index}: {error}"));
                *parsed.entry(format.extension).or_insert(0) += 1;
            }
        }
        println!("{name}: {parsed:?}, {compressed} compressed members");
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

/// The share of an image's opaque pixels that are yellow.
fn yellow_share(image: &Image) -> f64 {
    let opaque = image.pixels.iter().filter(|pixel| pixel[3] != 0);
    let yellow = opaque
        .clone()
        .filter(|[r, g, b, _]| *r > 180 && *g > 150 && *b < 120)
        .count();
    yellow as f64 / opaque.count().max(1) as f64
}

#[test]
fn extracts_gen4_sprites() {
    let gen4 = ["Diamond", "Pearl", "Platinum", "HeartGold", "SoulSilver"];
    for path in dumps()
        .into_iter()
        .filter(|path| gen4.iter().any(|game| name(path).contains(game)))
    {
        let name = name(&path);
        let game = Gen4::open(open(&path)).unwrap();
        let pikachu = SpeciesId(25);
        for variant in Variant::ALL {
            let sprite = game.pokemon_sprite(pikachu, variant).unwrap();
            assert_eq!((sprite.width, sprite.height), (80, 80), "{name}");
            for (x, y) in [(0, 0), (79, 0), (0, 79), (79, 79)] {
                assert_eq!(sprite.get(x, y).unwrap()[3], 0, "{name} {variant:?}");
            }
            assert!(yellow_share(&sprite) > 0.3, "{name} {variant:?}");
        }
        let icon = game.pokemon_icon(pikachu).unwrap();
        assert_eq!((icon.width, icon.height), (32, 32));
        assert!(yellow_share(&icon) > 0.3, "{name}");
        // Single-gender species have one sprite, whichever is asked for.
        let nidoran = SpeciesId(29);
        let female = Variant::ALL[2];
        assert_eq!(
            game.pokemon_sprite(nidoran, female).unwrap(),
            game.pokemon_sprite(nidoran, Variant::ALL[0]).unwrap()
        );
        // The Master Ball's icon is mostly purple.
        let ball = game.item_icon(ItemId(1)).unwrap();
        let purple = ball
            .pixels
            .iter()
            .filter(|[r, g, b, a]| *a != 0 && r > g && b > g)
            .count();
        assert!(purple > 100, "{name}: {purple} purple pixels");

        let assets = game.assets().unwrap();
        let count = |prefix: &str| {
            assets
                .iter()
                .filter(|path| path.starts_with(prefix))
                .count()
        };
        assert_eq!(count("sprites/pokemon/front/"), 493);
        assert_eq!(count("sprites/pokemon/back-shiny/"), 493);
        assert_eq!(count("sprites/icons/"), 493);
        assert!(assets.contains(&"sprites/pokemon/front-female/0025.png".to_owned()));
        assert!(!assets.contains(&"sprites/pokemon/front-female/0029.png".to_owned()));
        let (trainers, items) = match () {
            _ if name.contains("Platinum") => (105, 467),
            _ if name.contains("Gold") || name.contains("Silver") => (129, 536),
            _ => (98, 464),
        };
        assert_eq!(count("sprites/trainers/"), trainers, "{name}");
        assert_eq!(count("sprites/items/"), items, "{name}");
        println!(
            "{name}: {} sprites, {} female",
            count("sprites/"),
            count("sprites/pokemon/front-female/") + count("sprites/pokemon/back-female/")
        );
    }
}

/// Black, White, Black 2, and White 2.
fn gen5_dumps() -> impl Iterator<Item = PathBuf> {
    dumps().into_iter().filter(|path| {
        ["Black Version", "White Version"]
            .iter()
            .any(|game| name(path).contains(game))
    })
}

#[test]
fn decodes_all_gen5_text() {
    for path in gen5_dumps() {
        let game = Gen5::open(open(&path)).unwrap();
        for (archive, text) in [("main", game.text()), ("story", game.story_text())] {
            let text = text.unwrap();
            let mut messages = 0;
            for bank in 0..text.len() {
                for message in text.bank(bank).unwrap_or_else(|error| {
                    panic!("{}: {archive} bank {bank}: {error}", name(&path))
                }) {
                    assert!(
                        !message
                            .tokens
                            .iter()
                            .any(|token| matches!(token, gen5::text::Token::Unknown(_))),
                        "{archive} bank {bank}: {}",
                        message.markup()
                    );
                    messages += 1;
                }
            }
            println!(
                "{}: {archive}, {} banks, {messages} messages",
                name(&path),
                text.len()
            );
        }
    }
}

#[test]
fn extracts_gen5_data() {
    for path in gen5_dumps() {
        let sequel = name(&path).contains("Version 2");
        let game = Gen5::open(open(&path)).unwrap();
        let species = game.species().unwrap();
        assert_eq!(species.len(), if sequel { 708 } else { 667 });
        let pikachu = &species[24];
        assert_eq!(pikachu.name.as_deref(), Some("Pikachu"));
        assert_eq!(pikachu.types, [gen5::Type::Electric; 2]);
        let stats = &pikachu.base_stats;
        assert_eq!(
            (stats.hp, stats.attack, stats.defense, stats.speed),
            (35, 55, 30, 90)
        );
        assert_eq!((pikachu.height, pikachu.weight), (4, 60));
        assert_eq!(
            pikachu.hidden_ability,
            Some(gen5::AbilityId(31)),
            "Lightning Rod"
        );
        assert_eq!(
            pikachu.evolutions[0].method,
            gen5::EvolutionMethod::UseItem {
                item: gen5::ItemId(83)
            },
            "Thunder Stone"
        );
        assert_eq!(pikachu.evolutions[0].into, SpeciesId(26));
        assert!(pikachu.machines.contains(&23), "TM24 Thunderbolt");
        assert_eq!(species[31].name.as_deref(), Some("Nidoran♂"));
        assert_eq!(pikachu.tutors.len(), if sequel { 4 } else { 0 });

        // Deoxys's Attack, Defense, and Speed Formes have records of their own.
        let deoxys = &species[385];
        assert_eq!(
            (deoxys.name.as_deref(), deoxys.form_count),
            (Some("Deoxys"), 4)
        );
        let attack = &species[deoxys.form_records[0].0 as usize - 1];
        assert_eq!(
            attack.form_of.map(|form| (form.species, form.form)),
            Some((SpeciesId(386), 1))
        );
        assert_eq!(
            (attack.name.as_deref(), attack.base_stats.attack),
            (None, 180)
        );
        // Karrablast evolves when traded for Shelmet.
        assert_eq!(
            species[587].evolutions[0].method,
            gen5::EvolutionMethod::TradeForSpecies
        );

        let moves = game.moves().unwrap();
        assert_eq!(moves.len(), 559);
        let thunderbolt = &moves[84];
        assert_eq!(thunderbolt.name.as_deref(), Some("Thunderbolt"));
        assert_eq!(
            (thunderbolt.power, thunderbolt.class, thunderbolt.ailment),
            (95, gen5::MoveClass::Special, gen5::Ailment::Paralysis)
        );
        // The type tutors' moves, from Venusaur's and Dragonite's compatibility.
        let tutor_names = |dex: usize| -> Vec<&str> {
            species[dex - 1]
                .tutor_moves
                .iter()
                .map(|id| moves[id.0 as usize - 1].name.as_deref().unwrap())
                .collect()
        };
        assert_eq!(tutor_names(3), ["Grass Pledge", "Frenzy Plant"]);
        assert_eq!(tutor_names(149), ["Draco Meteor"]);

        let abilities = game.abilities().unwrap();
        assert_eq!(abilities[8].name, "Static");
        assert!(abilities[8].species.contains(&SpeciesId(25)));
        assert!(abilities[30].hidden.contains(&SpeciesId(25)));

        let items = game.items().unwrap();
        let potion = &items[16];
        assert_eq!(
            (potion.name.as_deref(), potion.price),
            (Some("Potion"), 300)
        );
        assert_eq!(potion.pocket, gen5::Pocket::Medicine);
        println!(
            "{}: {} species, {} moves, {} abilities, {} items",
            name(&path),
            species.len(),
            moves.len(),
            abilities.len(),
            items.len()
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

/// Whether decoded audio looks like sound rather than a decoding error: not silent, centered
/// on zero, and rarely clipped.
fn sounds_plausible(audio: &Audio) -> bool {
    let len = audio.samples.len().max(1) as f64;
    let mean = audio.samples.iter().map(|&s| f64::from(s)).sum::<f64>() / len;
    let power = audio
        .samples
        .iter()
        .map(|&s| f64::from(s).powi(2))
        .sum::<f64>();
    let rms = (power / len).sqrt();
    let clipped = audio
        .samples
        .iter()
        .filter(|s| s.unsigned_abs() >= 0x7FFF)
        .count();
    rms > 300.0 && mean.abs() < rms / 4.0 && (clipped as f64) < len / 100.0
}

#[test]
fn decodes_every_ds_sound_archive() {
    for path in dumps()
        .into_iter()
        .filter(|path| path.extension() == Some("nds".as_ref()))
    {
        let rom = NdsRom::open(open(&path)).unwrap();
        for (file, _) in rom.fs().paths().filter(|(file, _)| file.ends_with(".sdat")) {
            let name = format!("{}: {file}", name(&path));
            let archive = rom.file(file).unwrap();
            let sdat = Sdat::parse(&archive).unwrap_or_else(|error| panic!("{name}: {error}"));
            let sound = Sound::new(&archive, &sdat);
            let (mut waves, mut plausible) = (0, 0);
            for (index, _) in sdat
                .wave_archives
                .iter()
                .enumerate()
                .filter(|(_, a)| a.is_some())
            {
                for wave in 0..sound.wave_count(index).unwrap() {
                    let audio = sound
                        .wave(index, wave)
                        .unwrap_or_else(|error| panic!("{name}: wave {index}/{wave}: {error}"));
                    waves += 1;
                    plausible += sounds_plausible(&audio) as usize;
                }
            }
            assert!(
                plausible * 100 >= waves * 95,
                "{name}: {plausible} of {waves} plausible"
            );
            let mut looping = 0;
            for (index, _) in sdat
                .sequences
                .iter()
                .enumerate()
                .filter(|(_, s)| s.is_some())
            {
                let midi = sound
                    .sequence(index)
                    .unwrap_or_else(|error| panic!("{name}: sequence {index}: {error}"))
                    .to_mid();
                assert_eq!(&midi[..4], b"MThd");
                looping += midi.windows(7).any(|window| window == b"loopEnd") as usize;
            }
            for (index, _) in sdat.streams.iter().enumerate().filter(|(_, s)| s.is_some()) {
                assert!(
                    sounds_plausible(&sound.stream(index).unwrap()),
                    "{name}: stream {index}"
                );
            }
            println!(
                "{name}: {waves} waves, {} sequences ({looping} looping), {} streams",
                sdat.sequences.iter().flatten().count(),
                sdat.streams.iter().flatten().count(),
            );
        }
    }
}

#[test]
fn extracts_gen4_sound() {
    let gen4 = ["Diamond", "Pearl", "Platinum", "HeartGold", "SoulSilver"];
    for path in dumps()
        .into_iter()
        .filter(|path| gen4.iter().any(|game| name(path).contains(game)))
    {
        let game = Gen4::open(open(&path)).unwrap();
        let pikachu = game.cry(SpeciesId(25)).unwrap();
        assert_eq!(
            (pikachu.sample_rate, pikachu.channels, pikachu.samples.len()),
            (10512, 1, 8236)
        );
        assert!(sounds_plausible(&pikachu));
        assert_eq!(
            game.extract("sound/cries/0025.wav").unwrap(),
            pikachu.to_wav()
        );

        let assets = game.assets().unwrap();
        let count = |prefix| {
            assets
                .iter()
                .filter(|path| path.starts_with(prefix))
                .count()
        };
        let (sequences, waves, cries, town) = match game.game() {
            GameId::PokemonDiamond | GameId::PokemonPearl => (829, 817, 493, "SEQ_TOWN01_D"),
            GameId::PokemonPlatinum => (1013, 874, 494, "SEQ_TOWN01_D"),
            _ => (1372, 1102, 494, "SEQ_GS_T_WAKABA"),
        };
        assert_eq!(
            (
                count("sound/sequences/"),
                count("sound/waves/"),
                count("sound/cries/")
            ),
            (sequences, waves, cries)
        );
        // Pokémon Platinum and HeartGold added Shaymin's Sky Forme and its own cry.
        let sky = assets.iter().any(|path| path == "sound/cries/0492-sky.wav");
        assert_eq!(sky, cries == 494);

        let midi = game
            .extract(&format!("sound/sequences/{town}.mid"))
            .unwrap();
        assert!(midi.starts_with(b"MThd") && midi.windows(7).any(|w| w == b"loopEnd"));
        let wave = game
            .extract("sound/waves/WAVE_ARC_SE_FIELD/0000.wav")
            .unwrap();
        assert!(wave.starts_with(b"RIFF"));
        println!(
            "{}: {sequences} sequences, {waves} waves, {cries} cries",
            name(&path)
        );
    }
}
