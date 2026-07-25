//! Checks against real ROM dumps, which are never committed. Point `ROMKIT_ROMS` at a folder of
//! dumps to run them (`ROMKIT_ROMS=~/roms cargo test -p romkit --test roms -- --nocapture`);
//! without it they pass vacuously.

use std::path::{Path, PathBuf};

use romkit::ByteSource;
use romkit::formats::sdat::Sdat;
use romkit::formats::{Format, narc::Narc};
use romkit::games::pokemon::gen4::text::{Text, Token};
use romkit::games::pokemon::gen4::{Gen4, MoveClass, SpeciesId, Type};
use romkit::games::sound::Sound;
use romkit::io::FileSource;
use romkit::media::Audio;
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
