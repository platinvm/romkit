//! `romkit`: inspect ROMs and export their assets from the command line.

use std::fmt::Write as _;
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use romkit::io::FileSource;
use romkit::{FileInfo, Game, Rom};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// The ROM to open; it is read in place, never loaded whole.
    rom: PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Show what the ROM is (the default).
    Info,
    /// List the ROM's files, or the members of an archive.
    Files { archive: Option<String> },
    /// Write a file, or one archive member, to stdout.
    Read {
        path: String,
        #[arg(long)]
        member: Option<usize>,
    },
    /// List the assets romkit can extract.
    Assets,
    /// Write one asset to stdout.
    Extract { asset: String },
    /// Extract every asset into a directory, with a manifest.
    Export { dir: PathBuf },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let output = run(&cli).map(|output| std::io::stdout().lock().write_all(&output));
    match output {
        // A closed pipe, as with `| head`, just ends the output.
        Ok(Ok(())) => ExitCode::SUCCESS,
        Ok(Err(error)) if error.kind() == ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Ok(Err(error)) => fail(error),
        Err(error) => fail(error),
    }
}

fn fail(error: impl std::fmt::Display) -> ExitCode {
    eprintln!("romkit: {error}");
    ExitCode::FAILURE
}

/// Run a command, returning what it writes to stdout.
fn run(cli: &Cli) -> romkit::Result<Vec<u8>> {
    let rom = Rom::open(FileSource::open(&cli.rom)?)?;
    match cli.command.as_ref().unwrap_or(&Command::Info) {
        Command::Info => info(&rom),
        Command::Files { archive: None } => Ok(list(rom.files()?)),
        Command::Files {
            archive: Some(path),
        } => Ok(list(rom.archive(path)?)),
        Command::Read { path, member: None } => rom.read_file(path),
        Command::Read {
            path,
            member: Some(index),
        } => rom.read_member(path, *index),
        Command::Assets => Ok(lines(game(&rom)?.assets()?)),
        Command::Extract { asset } => game(&rom)?.extract(asset),
        Command::Export { dir } => export(&rom, dir),
    }
}

fn info(rom: &Rom) -> romkit::Result<Vec<u8>> {
    let overview = rom.overview()?;
    let identity = &overview.identity;
    let mut out = format!(
        "{identity}\nregion {:?}, revision {}\n",
        identity.region, identity.revision
    );
    if let Some(banner) = &overview.banner {
        _ = writeln!(out, "banner {:?}", banner.english);
    }
    _ = match (overview.game_name, overview.supported) {
        (Some(name), true) => writeln!(out, "{name}: {} assets", game(rom)?.assets()?.len()),
        (Some(name), false) => writeln!(out, "{name}: recognized, not supported yet"),
        (None, _) => writeln!(out, "unrecognized game"),
    };
    Ok(out.into_bytes())
}

fn list(files: Vec<FileInfo>) -> Vec<u8> {
    lines(files.into_iter().map(|file| {
        let format = file.format.map_or("", |format| format.extension);
        format!("{:>10}  {format:<6} {}", file.size, file.path)
    }))
}

fn lines(lines: impl IntoIterator<Item = String>) -> Vec<u8> {
    let text: String = lines.into_iter().map(|line| line + "\n").collect();
    text.into_bytes()
}

fn game<'r>(rom: &'r Rom) -> romkit::Result<&'r dyn Game> {
    rom.game()
        .ok_or_else(|| romkit::Error::Unsupported(format!("no extractor for {}", rom.identity())))
}

fn export(rom: &Rom, dir: &Path) -> romkit::Result<Vec<u8>> {
    let manifest = rom.export(|path, contents| {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().expect("asset paths are relative"))?;
        Ok(std::fs::write(path, contents)?)
    })?;
    let (count, name) = (manifest.assets.len(), manifest.game.name());
    Ok(format!("exported {count} assets of {name} to {}\n", dir.display()).into_bytes())
}
