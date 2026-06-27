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
