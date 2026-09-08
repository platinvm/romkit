# romkit

Identify game ROMs and extract their data and assets as structured files.

romkit is the shared core of two consumers:

- a website where you drop in a ROM, browse what it contains, and download what you need;
- games and tools that read a user-supplied ROM for the resources they need at runtime.

## What it reads

| Platform         | Identify | Browse files          | Extract data                                   |
| ---------------- | -------- | --------------------- | ---------------------------------------------- |
| Game Boy / Color | yes      |                       |                                                |
| Game Boy Advance | yes      |                       | Pokémon Ruby, Sapphire, Emerald, FireRed, LeafGreen (USA) |
| Nintendo DS      | yes      | NitroFS, NARC members | Pokémon Diamond, Pearl, Platinum, HeartGold, SoulSilver, Black, White, Black 2, White 2 |
| Nintendo 3DS     | yes      |                       |                                                |

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
move effects in detail), `items.json`, and both text archives as `text/main/<bank>.json`
(names, menus) and `text/story/<bank>.json` (dialogue).

Gen 3 games (the first USA release of each) extract `species.json` (with egg moves, and both the
National Pokédex number and the games' internal index), `moves.json`, `abilities.json`,
`items.json`, 64×64 battle sprites (`front`, `front-shiny`, `back`, `back-shiny`), 32×32 box
icons, 64×64 trainer sprites, 24×24 item icons (Emerald, FireRed, LeafGreen), and cries as
`sound/cries/<species>.wav`, all keyed by National Pokédex number. GBA games have no filesystem,
so each dump's `Edition` records where its tables are.

For any DS game, `formats::g2d` decodes the Nitro 2D graphics formats (NCLR palettes, NCGR
tiles, NCER cells, NSCR screens) to images, and `formats::lz` decompresses LZ77 data.

Gen 4 games also extract the overworld in 3D as glTF with embedded textures: map terrain
(`models/maps/<n>.glb`), buildings (`models/buildings/<n>.glb`), and in HeartGold and
SoulSilver indoor furniture (`models/furniture/<n>.glb`); and as PNG the texture sets of maps and
buildings and the overworld character sprites (`textures/{maps,buildings,overworld}/<n>/<name>.png`).
Any DS game's 3D models (NSBMD) and textures (NSBTX) convert through `formats::nitro3d`.

## Layout

```text
crates/
  romkit/                the library
    src/
      io/                ByteSource, Window, FileSource; Reader, the bounds-checked cursor
      platforms/         identify() and one module per handheld; nds/ has NitroFS, the banner,
                         and the ARM9 binary
      formats/           file formats inside ROMs: signature sniffing, NARC archives, LZ77,
                         g2d/ and nitro3d/ (the Nitro 2D and 3D formats)
      games/
        catalog.rs       GameId: every recognized game and its header signature
        mod.rs           the Game trait, and open() to get a game's extractor
        pokemon/gen3/    the Gen3 handle, the Edition table (table offsets), species, moves,
                         items, text, sprites, cries, assets
        pokemon/gen4/    the Gen4 handle, the Edition table, species, moves, text, sprites, 3D field,
                         assets
        pokemon/gen5/    the same for Black, White, Black 2, and White 2, plus items
      rom.rs             Rom: any ROM, for hosts that take whatever they are given
    tests/roms.rs        checks against real dumps
  romkit-cli/            the romkit command-line tool
  romkit-wasm/           the wasm build for the website, and romkit.mjs, a minimal JS host
```

The root `Cargo.toml` holds what the crates share: versions, dependency versions, and the build
profiles.

Each layer knows nothing of the ones above it: platforms do not know games, formats do not know
platforms.

## Opinions

- **Read lazily.** Everything reads through `ByteSource`, so opening a 128 MB DS ROM reads about
  25 KB, a browser reads a dropped file in place, and in-memory ROMs are borrowed, never copied.
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

From the command line:

```sh
cargo run --release -p romkit-cli -- game.nds                   # what is it?
cargo run --release -p romkit-cli -- game.nds files [archive]   # files, or archive members
cargo run --release -p romkit-cli -- game.nds read path --member 3
cargo run --release -p romkit-cli -- game.nds assets
cargo run --release -p romkit-cli -- game.nds extract species.json
cargo run --release -p romkit-cli -- game.nds export out/
```

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
