//! Checks against real ROM dumps, which are never committed. Point `ROMKIT_ROMS` at a folder of
//! dumps to run them (`ROMKIT_ROMS=~/roms cargo test -p romkit --test roms -- --nocapture`);
//! without it they pass vacuously.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use romkit::ByteSource;
use romkit::formats::garc::{self, Garc};
use romkit::formats::{Format, narc::Narc};
use romkit::games::pokemon::gen4::text::{Text, Token};
use romkit::games::pokemon::gen4::{Gen4, MoveClass, SpeciesId, Type};
use romkit::games::pokemon::n3ds::{self, N3ds, TextSet};
use romkit::io::FileSource;
use romkit::platforms::n3ds::N3dsRom;
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
            // Unpack the files of the smaller archives, and check that listing them, from
            // their first bytes alone, agrees.
            if range.end - range.start > 4 << 20 {
                continue;
            }
            for index in 0..garc.files().len() {
                let source = garc.file_source(&archive, index).unwrap();
                let stored = source.read_all().unwrap();
                // As `Rom::archive` lists them: small files whole, others by their head.
                let head = if stored.len() > 0x10000 {
                    &stored[..64]
                } else {
                    &stored[..]
                };
                let (size, _) = garc::peek(stored.len(), head);
                let contents = garc::unpack(stored.clone());
                assert_eq!(size, contents.len(), "{name}: {path} file {index}");
                compressed += usize::from(contents.len() != stored.len() || *contents != *stored);
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
fn decodes_all_3ds_text() {
    for path in dumps_3ds() {
        let name = name(&path);
        let game = N3ds::open(open(&path)).unwrap();
        let (mut banks, mut messages) = (0, 0);
        for language in game.languages() {
            for set in [TextSet::Game, TextSet::Story] {
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
        let english = game.text(TextSet::Game, "en").unwrap();
        let french = game.text(TextSet::Game, "fr").unwrap();
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
