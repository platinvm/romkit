//! Checks against real ROM dumps, which are never committed. Point `ROMKIT_ROMS` at a folder of
//! dumps to run them (`ROMKIT_ROMS=~/roms cargo test -p romkit --test roms -- --nocapture`);
//! without it they pass vacuously.

use std::borrow::Cow;
use std::cell::Cell;
use std::path::{Path, PathBuf};

use romkit::ByteSource;
use romkit::formats::g2d::{Cells, Palette, Screen, Tiles};
use romkit::formats::garc::Garc;
use romkit::formats::{Format, lz, narc::Narc};
use romkit::games::pokemon::gen4::text::{Text, Token};
use romkit::games::pokemon::gen4::{
    Gen4, ItemId, ModelSet, MoveClass, SpeciesId, TextureSet, Type, Variant,
};
use romkit::games::pokemon::gen5::{self, Gen5};
use romkit::games::pokemon::n3ds::{self, N3ds, TextSet};
use romkit::io::FileSource;
use romkit::media::Image;
use romkit::platforms::n3ds::N3dsRom;
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

/// A dump that counts the bytes read through it.
struct Counting {
    file: FileSource,
    bytes_read: Cell<u64>,
}

impl Counting {
    fn open(path: &Path) -> Self {
        Self {
            file: open(path),
            bytes_read: Cell::new(0),
        }
    }
}

impl ByteSource for Counting {
    fn size(&self) -> u64 {
        self.file.size()
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> romkit::Result<()> {
        self.bytes_read
            .set(self.bytes_read.get() + buf.len() as u64);
        self.file.read_exact_at(offset, buf)
    }
}

fn dumps_3ds() -> impl Iterator<Item = PathBuf> {
    dumps()
        .into_iter()
        .filter(|path| path.extension() == Some("3ds".as_ref()))
}

#[test]
fn reads_every_3ds_filesystem_lazily() {
    for path in dumps_3ds() {
        let name = name(&path);
        let source = Counting::open(&path);
        let rom = N3dsRom::open(&source).unwrap_or_else(|error| panic!("{name}: {error}"));
        let opened = source.bytes_read.get();
        // Headers and RomFS metadata only, out of 2 to 4 GB.
        assert!(opened < 64 << 10, "{name}: opening read {opened} bytes");
        assert!(rom.fs().paths().len() > 600, "{name}");
        assert_eq!(
            &*rom.file("exefs/icon").unwrap().read(0, 4).unwrap(),
            b"SMDH"
        );
        // Game Freak keeps its data in numbered archives; some numbers are left empty.
        let archives: Vec<_> = rom
            .fs()
            .paths()
            .filter(|(path, range)| path.starts_with("a/") && !range.is_empty())
            .collect();
        let (mut files, mut compressed) = (0, 0);
        for (path, range) in &archives {
            let archive = rom.file(path).unwrap();
            let garc =
                Garc::parse(&archive).unwrap_or_else(|error| panic!("{name}: {path}: {error}"));
            files += garc.files().len();
            // Unpack the files of the smaller archives.
            if range.end - range.start > 4 << 20 {
                continue;
            }
            for index in 0..garc.files().len() {
                let source = garc.file_source(&archive, index).unwrap();
                let stored = source.read_all().unwrap();
                compressed += usize::from(matches!(
                    lz::decompress_if_compressed(&stored),
                    Cow::Owned(_)
                ));
            }
        }
        println!(
            "{name}: {} files, {} archives holding {files} files ({compressed} compressed among \
             the smaller ones), opening read {opened} bytes",
            rom.fs().paths().len(),
            archives.len()
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
fn decodes_all_3ds_text() {
    for path in dumps_3ds() {
        let name = name(&path);
        let game = N3ds::open(open(&path)).unwrap();
        let (mut banks, mut messages) = (0, 0);
        for language in game.languages() {
            for set in [TextSet::Main, TextSet::Story] {
                let text = game.text(set, language).unwrap();
                for bank in 0..text.len() {
                    let decoded = text
                        .bank(bank)
                        .unwrap_or_else(|error| panic!("{name}: {language} {bank}: {error}"));
                    banks += 1;
                    messages += decoded.len();
                }
            }
        }
        let english = game.text(TextSet::Main, "en").unwrap();
        let french = game.text(TextSet::Main, "fr").unwrap();
        let types = (0..english.len())
            .find(|&bank| {
                english.strings(bank).unwrap().first().map(String::as_str) == Some("Normal")
            })
            .unwrap();
        assert_eq!(english.strings(types).unwrap()[17], "Fairy", "{name}");
        assert_eq!(french.strings(types).unwrap()[1], "Combat", "{name}");
        println!(
            "{name}: {} languages, {banks} banks, {messages} messages",
            game.languages().len()
        );
    }
}

#[test]
fn extracts_3ds_data() {
    for path in dumps_3ds() {
        let name = name(&path);
        let game = N3ds::open(open(&path)).unwrap();
        let generation7 = ["Sun", "Moon"].iter().any(|title| name.contains(title));
        let species = game.species().unwrap();
        let pikachu = &species[24];
        assert_eq!(pikachu.name.as_deref(), Some("Pikachu"), "{name}");
        assert_eq!(pikachu.types, [n3ds::Type::Electric; 2]);
        let stats = &pikachu.base_stats;
        assert_eq!((stats.hp, stats.defense, stats.speed), (35, 40, 90));
        assert_eq!(
            pikachu.abilities.map(|a| a.map(|a| a.0)),
            [Some(9), Some(9), Some(31)]
        );
        assert!(pikachu.machines.contains(&23), "TM24 Thunderbolt");
        // A Thunder Stone (item 83) makes a Raichu, an Alolan one (form 1) in Alola.
        let evolution = pikachu.evolutions.last().unwrap();
        assert_eq!(evolution.method, n3ds::EvolutionMethod::UseItem);
        assert_eq!((evolution.parameter, evolution.into), (83, SpeciesId(26)));
        assert_eq!(evolution.form, generation7.then_some(1));
        assert_eq!(
            pikachu.z_crystal.map(|z| (z.item.0, z.z_move.0)),
            generation7.then_some((794, 658)),
            "Pikanium Z, Catastropika"
        );

        // Mega Venusaur, and the Alolan Raichu of Generation VII, have records of their own.
        let form = |species: u16, form: u8| {
            species_form(species, form, game.species().unwrap())
                .unwrap_or_else(|| panic!("{name}: no form {form} of {species}"))
        };
        assert_eq!(form(3, 1).base_stats.attack, 100);
        if generation7 {
            let alolan = form(26, 1);
            assert!(alolan.regional_form);
            assert_eq!(alolan.types, [n3ds::Type::Electric, n3ds::Type::Psychic]);
        }
        // The last species of the National Pokédex, and the moves and abilities, by generation.
        let last = species.iter().rfind(|s| s.name.is_some()).unwrap();
        let counts = (
            last.id.0,
            game.moves().unwrap().len(),
            game.abilities().unwrap().len(),
        );
        let expected = match name.split(" (").next().unwrap() {
            "Pokemon X" | "Pokemon Y" => (721, 617, 188),
            "Pokemon Omega Ruby" | "Pokemon Alpha Sapphire" => (721, 621, 191),
            "Pokemon Sun" | "Pokemon Moon" => (802, 719, 232),
            _ => (807, 728, 233),
        };
        assert_eq!(counts, expected, "{name}");
        // Every record past the National Pokédex is an alternate form of a species.
        let orphans = species[last.id.0 as usize..]
            .iter()
            .filter(|form| form.form_of.is_none())
            .count();
        assert_eq!(orphans, 0, "{name}");

        let moves = game.moves().unwrap();
        let thunderbolt = &moves[84];
        assert_eq!(thunderbolt.name.as_deref(), Some("Thunderbolt"));
        assert_eq!((thunderbolt.power, thunderbolt.pp), (90, 15));
        assert_eq!(thunderbolt.class, n3ds::MoveClass::Special);
        assert_eq!(thunderbolt.ailment, n3ds::Ailment::Paralysis);

        let abilities = game.abilities().unwrap();
        assert_eq!(abilities[8].name, "Static");
        assert!(abilities[8].species.contains(&SpeciesId(25)));
        println!(
            "{name}: {} species and forms (last named {:?} #{}), {} moves (last {:?}), {} abilities",
            species.len(),
            last.name.as_deref().unwrap_or(""),
            last.id.0,
            moves.len(),
            moves.last().unwrap().name.as_deref().unwrap_or(""),
            abilities.len(),
        );
    }
}

#[test]
fn decodes_3ds_music() {
    for path in dumps_3ds() {
        let name = name(&path);
        let game = N3ds::open(open(&path)).unwrap();
        let streams: Vec<String> = game
            .assets()
            .unwrap()
            .into_iter()
            .filter(|asset| asset.starts_with("sound/"))
            .collect();
        // X and Y keep their music as AAC, which romkit does not decode.
        if name.starts_with("Pokemon X") || name.starts_with("Pokemon Y") {
            assert!(streams.is_empty());
            continue;
        }
        assert!(streams.len() > 150, "{name}");
        let unique: std::collections::BTreeSet<_> = streams.iter().collect();
        assert_eq!(unique.len(), streams.len(), "{name}: streams share a name");
        for asset in [streams.first().unwrap(), streams.last().unwrap()] {
            let wav = game.extract(asset).unwrap();
            let data = wav.windows(4).position(|chunk| chunk == b"data").unwrap() + 8;
            let samples: Vec<i16> = wav[data..]
                .as_chunks()
                .0
                .iter()
                .map(|&pair| i16::from_le_bytes(pair))
                .collect();
            // Decoded with the wrong coefficients, DSP-ADPCM diverges and clips.
            let clipped = samples.iter().filter(|s| s.unsigned_abs() >= 32767).count();
            let loud = samples.iter().filter(|s| s.unsigned_abs() > 1000).count();
            assert!(clipped * 1000 < samples.len(), "{name}: {asset} clips");
            assert!(loud * 10 > samples.len(), "{name}: {asset} is silent");
            println!("{name}: {asset}, {} samples", samples.len());
        }
        println!("{name}: {} streams", streams.len());
    }
}

fn species_form(species: u16, form: u8, all: &[n3ds::Species]) -> Option<&n3ds::Species> {
    all.iter().find(|candidate| {
        candidate
            .form_of
            .is_some_and(|of| of.species == SpeciesId(species) && of.form == form)
    })
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
