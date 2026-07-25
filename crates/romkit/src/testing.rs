//! Helpers shared by unit tests: a metering source and builders for synthetic data.

use std::cell::Cell;
use std::collections::BTreeSet;

use crate::Result;
use crate::io::ByteSource;

/// Wraps a source, hides its in-memory slice (so reads take the copying path), and counts the
/// bytes read through it.
pub struct Counting<S> {
    inner: S,
    bytes_read: Cell<u64>,
}

impl<S: ByteSource> Counting<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            bytes_read: Cell::new(0),
        }
    }

    pub fn bytes_read(&self) -> u64 {
        self.bytes_read.get()
    }
}

impl<S: ByteSource> ByteSource for Counting<S> {
    fn size(&self) -> u64 {
        self.inner.size()
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        self.bytes_read
            .set(self.bytes_read.get() + buf.len() as u64);
        self.inner.read_exact_at(offset, buf)
    }
}

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}

/// A NARC holding `members`.
pub fn narc(members: &[&[u8]]) -> Vec<u8> {
    let image: Vec<u8> = members.concat();
    let fat_len = 12 + members.len() * 8;
    let mut bytes = Vec::new();
    bytes.extend(b"NARC\xFE\xFF\x00\x01");
    bytes.extend(((16 + fat_len + 8 + 8 + image.len()) as u32).to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(3u16.to_le_bytes());
    bytes.extend(b"BTAF");
    bytes.extend((fat_len as u32).to_le_bytes());
    bytes.extend((members.len() as u32).to_le_bytes());
    let mut end = 0;
    for member in members {
        bytes.extend((end as u32).to_le_bytes());
        end += member.len();
        bytes.extend((end as u32).to_le_bytes());
    }
    bytes.extend(b"BTNF\x08\x00\x00\x00GMIF");
    bytes.extend(((8 + image.len()) as u32).to_le_bytes());
    bytes.extend(image);
    bytes
}

/// A DS ROM with a valid header and a NitroFS holding `files`, given as `(path, contents)`.
pub fn nds(code: &[u8; 4], files: &[(&str, &[u8])]) -> Vec<u8> {
    let parent = |path: &str| {
        path.rsplit_once('/')
            .map_or("", |(parent, _)| parent)
            .to_owned()
    };
    let name = |path: &str| path.rsplit('/').next().unwrap().to_owned();
    // Every directory ("" is the root), sorted, so that a directory's ID follows its position.
    let mut directories = BTreeSet::from([String::new()]);
    for (path, _) in files {
        let mut directory = parent(path);
        while directories.insert(directory.clone()) {
            directory = parent(&directory);
        }
    }
    let directories: Vec<String> = directories.into_iter().collect();
    let id =
        |directory: &str| 0xF000 + directories.iter().position(|d| d == directory).unwrap() as u16;

    let mut fnt = vec![0; directories.len() * 8];
    let mut ordered: Vec<&[u8]> = Vec::new();
    for (index, directory) in directories.iter().enumerate() {
        let parent_id = match directory.is_empty() {
            true => directories.len() as u16,
            false => id(&parent(directory)),
        };
        let entries = fnt.len() as u32;
        put(&mut fnt, index * 8, &entries.to_le_bytes());
        put(
            &mut fnt,
            index * 8 + 4,
            &(ordered.len() as u16).to_le_bytes(),
        );
        put(&mut fnt, index * 8 + 6, &parent_id.to_le_bytes());
        for (path, contents) in files.iter().filter(|(path, _)| parent(path) == *directory) {
            fnt.push(name(path).len() as u8);
            fnt.extend(name(path).as_bytes());
            ordered.push(contents);
        }
        for child in directories
            .iter()
            .filter(|d| !d.is_empty() && parent(d) == *directory)
        {
            fnt.push(0x80 | name(child).len() as u8);
            fnt.extend(name(child).as_bytes());
            fnt.extend(id(child).to_le_bytes());
        }
        fnt.push(0);
    }

    // The header, the FNT at 0x200, the FAT, then the files.
    let fat_at = 0x200 + fnt.len();
    let mut rom = vec![0; fat_at + ordered.len() * 8];
    put(&mut rom, 0, b"TEST");
    put(&mut rom, 0x0C, code);
    put(&mut rom, 0x40, &0x200u32.to_le_bytes());
    put(&mut rom, 0x44, &(fnt.len() as u32).to_le_bytes());
    put(&mut rom, 0x48, &(fat_at as u32).to_le_bytes());
    put(&mut rom, 0x4C, &(ordered.len() as u32 * 8).to_le_bytes());
    put(&mut rom, 0x15C, &0xCF56u16.to_le_bytes());
    put(&mut rom, 0x200, &fnt);
    for (index, contents) in ordered.into_iter().enumerate() {
        let start = rom.len() as u32;
        rom.extend(contents);
        let end = rom.len() as u32;
        put(&mut rom, fat_at + index * 8, &start.to_le_bytes());
        put(&mut rom, fat_at + index * 8 + 4, &end.to_le_bytes());
    }
    rom
}

/// An SDAT holding `files`, and `sounds` given as `(kind, name, file)`, where `kind` is the index
/// of the INFO list (0 sequences, 1 sequence archives, 3 wave archives, 7 streams) and a file of
/// `u32::MAX` leaves the sound without an INFO entry.
pub fn sdat(sounds: &[(usize, Option<&str>, u32)], files: &[&[u8]]) -> Vec<u8> {
    let mut symbols = b"SYMB".to_vec();
    symbols.resize(0x40, 0);
    let mut info = b"INFO".to_vec();
    info.resize(0x40, 0);
    for kind in 0..8 {
        let sounds: Vec<_> = sounds.iter().filter(|sound| sound.0 == kind).collect();
        let count = (sounds.len() as u32).to_le_bytes();
        // Sequence archive names come in pairs: the archive's, then its sequences' (none here).
        let stride = if kind == 1 { 8 } else { 4 };
        let (names, entries) = (symbols.len(), info.len());
        put(&mut symbols, 8 + 4 * kind, &(names as u32).to_le_bytes());
        symbols.extend(count);
        symbols.resize(names + 4 + stride * sounds.len(), 0);
        put(&mut info, 8 + 4 * kind, &(entries as u32).to_le_bytes());
        info.extend(count);
        info.resize(entries + 4 + 4 * sounds.len(), 0);
        for (index, &&(_, name, file)) in sounds.iter().enumerate() {
            if let Some(name) = name {
                let at = (symbols.len() as u32).to_le_bytes();
                put(&mut symbols, names + 4 + stride * index, &at);
                symbols.extend(name.as_bytes());
                symbols.push(0);
            }
            if file != u32::MAX {
                let at = (info.len() as u32).to_le_bytes();
                put(&mut info, entries + 4 + 4 * index, &at);
                info.extend(file.to_le_bytes());
                info.extend([0; 8]);
            }
        }
    }
    let len = (symbols.len() as u32).to_le_bytes();
    put(&mut symbols, 4, &len);
    let len = (info.len() as u32).to_le_bytes();
    put(&mut info, 4, &len);

    let blocks_at = 0x40 + symbols.len() + info.len();
    let fat_len = 12 + 16 * files.len();
    let mut fat = b"FAT ".to_vec();
    fat.extend((fat_len as u32).to_le_bytes());
    fat.extend((files.len() as u32).to_le_bytes());
    let mut file_block = b"FILE".to_vec();
    file_block.extend((16 + files.concat().len() as u32).to_le_bytes());
    file_block.extend((files.len() as u32).to_le_bytes());
    file_block.extend([0; 4]);
    for file in files {
        let at = blocks_at + fat_len + file_block.len();
        fat.extend((at as u32).to_le_bytes());
        fat.extend((file.len() as u32).to_le_bytes());
        fat.extend([0; 8]);
        file_block.extend(*file);
    }

    let mut bytes = b"SDAT\xFF\xFE\x00\x01".to_vec();
    bytes.resize(0x40, 0);
    let blocks = [&*symbols, &info, &fat, &file_block];
    let mut at = 0x40;
    for (index, block) in blocks.iter().enumerate() {
        put(&mut bytes, 0x10 + 8 * index, &(at as u32).to_le_bytes());
        put(
            &mut bytes,
            0x14 + 8 * index,
            &(block.len() as u32).to_le_bytes(),
        );
        at += block.len();
    }
    put(&mut bytes, 8, &(at as u32).to_le_bytes());
    bytes.extend(blocks.concat());
    bytes
}

/// A SWAR holding waves given as `(encoding, loop start in words, data)`.
pub fn swar(waves: &[(u8, Option<u16>, &[u8])]) -> Vec<u8> {
    let mut bytes = b"SWAR\xFF\xFE\x00\x01\0\0\0\0\x10\0\x01\0DATA".to_vec();
    bytes.resize(0x38, 0);
    bytes.extend((waves.len() as u32).to_le_bytes());
    let mut at = bytes.len() + 4 * waves.len();
    for (_, _, data) in waves {
        bytes.extend((at as u32).to_le_bytes());
        at += 12 + data.len();
    }
    for &(encoding, loop_start, data) in waves {
        let start = loop_start.unwrap_or(0);
        bytes.extend([encoding, loop_start.is_some() as u8]);
        bytes.extend([8000u16, 2095, start].map(u16::to_le_bytes).concat());
        bytes.extend((data.len() as u32 / 4 - u32::from(start)).to_le_bytes());
        bytes.extend(data);
    }
    bytes
}
