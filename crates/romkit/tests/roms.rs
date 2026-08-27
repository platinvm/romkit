//! Checks against real ROM dumps, which are never committed. Point `ROMKIT_ROMS` at a folder of
//! dumps to run them (`ROMKIT_ROMS=~/roms cargo test -p romkit --test roms -- --nocapture`);
//! without it they pass vacuously.

use std::path::{Path, PathBuf};

use romkit::ByteSource;
use romkit::formats::{Format, narc::Narc};
use romkit::games::pokemon::gen4::text::{Text, Token};
use romkit::games::pokemon::gen4::{Gen4, MoveClass, SpeciesId, Type};
use romkit::games::pokemon::gen5::{self, Gen5};
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
