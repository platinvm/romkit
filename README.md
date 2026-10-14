# romkit

Identify game ROMs and extract their data and assets as structured files.

romkit is the shared core of two consumers:

- a website where you drop in a ROM, browse what it contains, and download what you need;
- games and tools that read a user-supplied ROM for the resources they need at runtime.

## What it reads

| Platform         | Identify | Browse files          | Extract data                                   |
| ---------------- | -------- | --------------------- | ---------------------------------------------- |
| Game Boy / Color | yes      |                       | Pokémon Red, Blue, Yellow (USA/Europe), Green (Japan), Gold (USA/Europe), Crystal (USA) |
| Game Boy Advance | yes      |                       | Pokémon Ruby, Sapphire, Emerald, FireRed, LeafGreen (USA) |
| Nintendo DS      | yes      | NitroFS, NARC members | Pokémon Diamond, Pearl, Platinum, HeartGold, SoulSilver, Black, White, Black 2, White 2 |
| Nintendo 3DS     | yes      | ExeFS, RomFS, GARC files | Pokémon X, Y, Omega Ruby, Alpha Sapphire, Sun, Moon, Ultra Sun, Ultra Moon |

The catalog recognizes every mainline Pokémon game from Red to Ultra Moon. Gen 4 games extract
`species.json` (stats, types, abilities, evolutions, level-up moves, TM/HM compatibility),
`moves.json`, `abilities.json`, every text bank as `text/<bank>.json`, and sprites as PNG:

| Asset                                     | What                                   |
| ----------------------------------------- | -------------------------------------- |
| `sprites/pokemon/<variant>/<species>.png` | 80×80 battle sprites (see below)       |
| `sprites/icons/<species>.png`             | 32×32 box icons                        |
| `sprites/trainers/<class>.png`            | 80×80 trainer class sprites            |
| `sprites/items/<item>.png`                | 32×32 item icons                       |

Battle sprite variants are `front`, `front-shiny`, `back`, and `back-shiny`, plus `front-female`,
`front-shiny-female`, `back-female`, and `back-shiny-female` for species whose females look
different.

Gen 5 games extract the same tables with their Gen 5 additions (hidden abilities, forms, tutors,
move effects in detail), `items.json`, both text archives as `text/main/<bank>.json`
(names, menus) and `text/story/<bank>.json` (dialogue), and the same sprites: 96×96 battle
sprites, 32×32 icons (with `sprites/icons-female/` where females differ), 80×80 trainer sprites
(the first frame of the animated ones in Black 2 and White 2), and 32×32 item icons. Alternate
forms with sprites of their own are named after species and form number, e.g.
`sprites/pokemon/front/0386-1.png` for Deoxys's Attack Forme.

Gen 3 games (the first USA release of each) extract `species.json` (with egg moves, and both the
National Pokédex number and the games' internal index), `moves.json`, `abilities.json`,
`items.json`, 64×64 battle sprites (`front`, `front-shiny`, `back`, `back-shiny`), 32×32 box
icons, 64×64 trainer sprites, 24×24 item icons (Emerald, FireRed, LeafGreen), and cries as
`sound/cries/<species>.wav`, all keyed by National Pokédex number. GBA games have no filesystem,
so each dump's `Edition` records where its tables are.

Gen 1 and Gen 2 games extract `species.json` (stats, types, evolutions, level-up moves, TM/HM
compatibility; Gen 1 adds the internal index and level-1 moves, Gen 2 egg moves and held items),
`moves.json` (Gen 2 with descriptions), `items.json` (Gen 2 with descriptions and attributes),
and Pokémon sprites colored with the palettes in the ROM: Gen 1 `front` (56×56) and `back`
(32×32) in their Super Game Boy colors (Yellow: Game Boy Color), Gen 2 `front` and
`front-shiny` (56×56), `back` and `back-shiny` (48×48). Names keep the games' upper case;
Japanese Green decodes to kana. Releases are told apart by their header checksum, so other
languages and revisions (and Silver, without a dump to verify) are identified but not extracted.

The 3DS games (decrypted dumps) extract `species.json` (with alternate forms, hidden abilities,
tutors, Z-Crystals), `moves.json` (battle data in detail, Z-Moves), `abilities.json`, and every
text bank of every language as `text/<language>/main/<bank>.json` and
`text/<language>/story/<bank>.json`; the banks romkit reads are named, e.g.
`text/en/main/species_names.json`. Omega Ruby, Alpha Sapphire, and Generation VII also extract
their music and jingles as `sound/<name>.wav`, decoded from BCSTM streams (X and Y use AAC, which
romkit does not decode). Languages are `ja-kana`, `ja-kanji`, `en`, `fr`, `it`, `de`,
`es`, `ko`, and in Generation VII `zh-hans` and `zh-hant`, whose characters stay in the games'
private-use encoding.

For any DS game, `formats::g2d` decodes the Nitro 2D graphics formats (NCLR palettes, NCGR
tiles, NCER cells, NSCR screens) to images, and `formats::lz` decompresses LZ77 data.

Gen 4 games also extract the overworld in 3D as glTF with embedded textures: map terrain
(`models/maps/<n>.glb`), buildings (`models/buildings/<n>.glb`), and in HeartGold and
SoulSilver indoor furniture (`models/furniture/<n>.glb`); and as PNG the texture sets of maps and
buildings and the overworld character sprites (`textures/{maps,buildings,overworld}/<n>/<name>.png`).
Any DS game's 3D models (NSBMD) and textures (NSBTX) convert through `formats::nitro3d`.
Gen 4 and Gen 5 games also extract their sound archive (SDAT): music and sound effects as MIDI
(`sound/sequences/SEQ_TOWN01_D.mid`), samples as WAV (`sound/waves/<archive>/<index>.wav`),
streams (`sound/streams/<name>.wav`), and every Pokémon's cry by National Pokédex number
(`sound/cries/0025.wav`, plus forms such as `0492-sky.wav`). The SDAT reader and its asset tree
work for any DS game.

## Layout

```text
crates/
  romkit/                the library
    src/
      io/                ByteSource, Window, FileSource; Reader, the bounds-checked cursor
      platforms/         identify() and one module per handheld; nds/ has NitroFS, the banner,
                         and the ARM9 binary; n3ds/ has the ExeFS and the RomFS
      formats/           file formats inside ROMs: signature sniffing, NARC and GARC archives,
                         LZ77, BCSTM audio, g2d/ and nitro3d/ (the Nitro 2D and 3D formats),
                         sdat/ (DS sound archives)
      games/
        catalog.rs       GameId: every recognized game and its header signature
        mod.rs           the Game trait, and open() to get a game's extractor
        sound.rs         any DS sound archive as assets
        pokemon/gen1/    the Gen1 handle, the Edition table (bank:address of each table),
                         species, moves, items, text charmaps, pic decompression, sprites
        pokemon/gen2/    the same for Gold and Crystal, plus LZ decompression
        pokemon/gen3/    the Gen3 handle, the Edition table (table offsets), species, moves,
                         items, text, sprites, cries, assets
        pokemon/gen4/    the Gen4 handle, the Edition table, species, moves, text, sprites, sound,
                         3D field, assets
        pokemon/gen5/    the same for Black, White, Black 2, and White 2, plus items
        pokemon/n3ds/    the N3ds handle and Edition table for the eight 3DS games
      rom.rs             Rom: any ROM, for hosts that take whatever they are given
    tests/roms.rs        checks against real dumps
  romkit-cli/            the romkit command-line tool
  romkit-wasm/           the wasm build, romkit.mjs (a minimal JS host), and site/, the website
```

The root `Cargo.toml` holds what the crates share: versions, dependency versions, and the build
profiles.

Each layer knows nothing of the ones above it: platforms do not know games, formats do not know
platforms.

## Opinions

- **Read lazily.** Everything reads through `ByteSource`, so opening a 128 MB DS ROM reads about
  25 KB and a 4 GB 3DS image about 50 KB, a browser reads a dropped file in place, and in-memory ROMs are borrowed, never copied.
- **Typed first, files second.** Each game has a typed handle (`Gen4::species()`) for programs.
  The `Game` trait is a uniform view over the same data as asset files (JSON for data), which
  is what the website and the exporter use.
- **Lose nothing.** Values without a known meaning are kept (`Unknown(raw)` enum variants,
  unknown text codes), and text has a lossless markup form.
- **Untrusted input.** ROMs come from users: malformed data is an error naming the format and the
  offset, never a panic or an endless loop.

## Use

In a program that needs one game's data:

```rust
use romkit::games::pokemon::gen4::Gen4;
use romkit::io::FileSource;

let game = Gen4::open(FileSource::open("platinum.nds")?)?;
let pikachu = &game.species()?[24];
println!("{:?} {:?} {:?}", pikachu.name, pikachu.types, pikachu.evolutions);
```

For any ROM, use `Rom`:

```rust
let rom = romkit::Rom::open(FileSource::open("game.nds")?)?;
println!("{}", rom.identity());
for file in rom.files()? { /* path, size, format */ }
if let Some(game) = rom.game() {
    let species = game.extract("species.json")?;
}
rom.export(|path, contents| { /* store each asset, then manifest.json */ Ok(()) })?;
```

From the command line, after `cargo install romkit-cli` (or with a binary from the GitHub
releases):

```sh
romkit game.nds                   # what is it?
romkit game.nds files [archive]   # files, or archive members
romkit game.nds read path --member 3
romkit game.nds assets
romkit game.nds extract species.json
romkit game.nds export out/
```

From a checkout, `cargo run --release -p romkit-cli --` stands in for `romkit`.

In a browser, build the wasm module and load it in a Web Worker with
`crates/romkit-wasm/romkit.mjs`:

```sh
cargo build --release -p romkit-wasm --target wasm32-unknown-unknown
```

```js
import { load } from "./romkit.mjs";
const reader = new FileReaderSync();
const romkit = await load(wasmBytes, (offset, length) =>
  new Uint8Array(reader.readAsArrayBuffer(file.slice(offset, offset + length))));
const overview = romkit.open(file.size);
const species = JSON.parse(new TextDecoder().decode(romkit.extract("species.json")));
```

The website in `crates/romkit-wasm/site` is static files around that worker. To try it locally,
put the module and the host beside it and serve the folder:

```sh
cp -r crates/romkit-wasm/site /tmp/romkit-site
cp crates/romkit-wasm/romkit.mjs /tmp/romkit-site/
cp target/wasm32-unknown-unknown/release/romkit_wasm.wasm /tmp/romkit-site/romkit.wasm
python3 -m http.server -d /tmp/romkit-site
```

## Adding a game

1. **Recognize it**: add a row to the catalog in `crates/romkit/src/games/catalog.rs`.
2. **Same engine as a supported game**: add its `Edition` (file paths and text banks) in that
   family's module, e.g. `crates/romkit/src/games/pokemon/gen4/mod.rs`. Locate text banks by their
   content in a real dump.
3. **New engine**: add a module under `crates/romkit/src/games/` with a typed handle that parses
   tables on first use and implements `Game`, then add a line to `games::open` and
   `GameId::is_supported`.
4. **Check it against a real dump** in `crates/romkit/tests/roms.rs` with facts you know to be true.

## Test

```sh
cargo test --workspace                                              # unit tests, synthetic data
ROMKIT_ROMS=~/roms cargo test -p romkit --test roms -- --nocapture  # every dump in ~/roms
```

ROM dumps are never committed; without `ROMKIT_ROMS` the real-dump tests pass vacuously.

## Release

CI (`.github/workflows/ci.yml`) checks formatting, lints, tests, and builds the CLI and the wasm
module on every push to `main` and every pull request.

To release, bump `version` in the root `Cargo.toml` (both `[workspace.package]` and the `romkit`
entry of `[workspace.dependencies]`), commit, and push a matching tag:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

- `release.yml` builds the CLI for Linux (static musl), macOS, and Windows, on x86-64 and ARM64,
  plus the wasm module with `romkit.mjs`; publishes `romkit` and `romkit-cli` to crates.io
  with the `CARGO_REGISTRY_TOKEN` secret of the `crates-io` environment; then publishes the GitHub release with `SHA256SUMS`.
- `pages.yml` builds the wasm module and deploys the website to GitHub Pages on every push to
  `main`. It can also be run by hand from the Actions tab.
