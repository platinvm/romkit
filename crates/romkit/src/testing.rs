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
