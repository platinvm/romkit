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

/// UTF-16LE, as 3DS file names are stored.
fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// A RomFS holding `files`, given as `(path, contents)`: an IVFC header whose level 3, at
/// 0x1000, holds the filesystem.
pub fn romfs(files: &[(&str, &[u8])]) -> Vec<u8> {
    const NONE: u32 = u32::MAX;
    let parent = |path: &str| {
        path.rsplit_once('/')
            .map_or("", |(parent, _)| parent)
            .to_owned()
    };
    let name = |path: &str| path.rsplit('/').next().unwrap().to_owned();
    let mut directories = BTreeSet::from([String::new()]);
    for (path, _) in files {
        let mut directory = parent(path);
        while directories.insert(directory.clone()) {
            directory = parent(&directory);
        }
    }
    let directories: Vec<String> = directories.into_iter().collect();
    let entry_len = |fixed: usize, name: &str| fixed + utf16(name).len().next_multiple_of(4);
    let offsets = |names: Vec<String>, fixed: usize| {
        let mut at = 0;
        names
            .iter()
            .map(|name| {
                let offset = at;
                at += entry_len(fixed, name);
                offset as u32
            })
            .collect::<Vec<u32>>()
    };
    let directory_at = offsets(directories.iter().map(|d| name(d)).collect(), 0x18);
    let file_at = offsets(files.iter().map(|(path, _)| name(path)).collect(), 0x20);
    let directory = |path: &str| directory_at[directories.iter().position(|d| d == path).unwrap()];
    // The entry after `index` among those that share its parent.
    let next = |paths: Vec<String>, index: usize, at: &[u32]| {
        (index + 1..paths.len())
            .find(|&other| parent(&paths[other]) == parent(&paths[index]))
            .map_or(NONE, |other| at[other])
    };
    let file_paths: Vec<String> = files.iter().map(|(path, _)| path.to_string()).collect();

    let mut directory_meta = Vec::new();
    for (index, path) in directories.iter().enumerate() {
        let children: Vec<usize> = (1..directories.len())
            .filter(|&child| parent(&directories[child]) == *path)
            .collect();
        let first_file = file_paths.iter().position(|file| parent(file) == *path);
        let sibling = match index {
            0 => NONE,
            _ => next(directories.clone(), index, &directory_at),
        };
        for value in [
            directory(&parent(path)),
            sibling,
            children.first().map_or(NONE, |&child| directory_at[child]),
            first_file.map_or(NONE, |file| file_at[file]),
            NONE,
            utf16(&name(path)).len() as u32,
        ] {
            directory_meta.extend(value.to_le_bytes());
        }
        directory_meta.extend(utf16(&name(path)));
        directory_meta.resize(directory_meta.len().next_multiple_of(4), 0);
    }
    let mut file_meta = Vec::new();
    let mut data = Vec::new();
    for (index, (path, contents)) in files.iter().enumerate() {
        file_meta.extend(directory(&parent(path)).to_le_bytes());
        file_meta.extend(next(file_paths.clone(), index, &file_at).to_le_bytes());
        file_meta.extend((data.len() as u64).to_le_bytes());
        file_meta.extend((contents.len() as u64).to_le_bytes());
        file_meta.extend(NONE.to_le_bytes());
        file_meta.extend((utf16(&name(path)).len() as u32).to_le_bytes());
        file_meta.extend(utf16(&name(path)));
        file_meta.resize(file_meta.len().next_multiple_of(4), 0);
        data.extend(*contents);
        data.resize(data.len().next_multiple_of(16), 0);
    }

    // Level 3: its header, the four tables (one-bucket hash tables), then the data.
    let mut tables = vec![0; 0x28];
    let mut table = |bytes: &[u8], at: usize| {
        let offset = tables.len() as u32;
        put(&mut tables, at, &offset.to_le_bytes());
        put(&mut tables, at + 4, &(bytes.len() as u32).to_le_bytes());
        tables.extend(bytes);
    };
    table(&NONE.to_le_bytes(), 0x04);
    table(&directory_meta, 0x0C);
    table(&NONE.to_le_bytes(), 0x14);
    table(&file_meta, 0x1C);
    tables.resize(tables.len().next_multiple_of(16), 0);
    put(&mut tables, 0, &0x28u32.to_le_bytes());
    let data_at = tables.len() as u32;
    put(&mut tables, 0x24, &data_at.to_le_bytes());
    tables.extend(data);

    let mut image = vec![0; 0x1000];
    put(&mut image, 0, b"IVFC");
    put(&mut image, 0x08, &0x20u32.to_le_bytes()); // master hash size
    put(&mut image, 0x4C, &12u32.to_le_bytes()); // level 3 blocks of 4 KiB
    image.extend(tables);
    image
}

/// A decrypted 3DS image with product code `product`, whose game partition holds an ExeFS and
/// a RomFS with these files.
pub fn n3ds(product: &str, exefs: &[(&str, &[u8])], files: &[(&str, &[u8])]) -> Vec<u8> {
    let pad = |bytes: &mut Vec<u8>| bytes.resize(bytes.len().next_multiple_of(0x200), 0);
    let mut exefs_image = vec![0; 0x200];
    for (index, (name, contents)) in exefs.iter().enumerate() {
        let offset = exefs_image.len() as u32 - 0x200;
        put(&mut exefs_image, index * 16, name.as_bytes());
        put(&mut exefs_image, index * 16 + 8, &offset.to_le_bytes());
        put(
            &mut exefs_image,
            index * 16 + 12,
            &(contents.len() as u32).to_le_bytes(),
        );
        exefs_image.extend(*contents);
        pad(&mut exefs_image);
    }
    let mut romfs_image = romfs(files);
    pad(&mut romfs_image);

    // The NCSD header, then the partition: its NCCH header, the ExeFS, and the RomFS.
    let mut image = vec![0; 0x400];
    put(&mut image, 0x100, b"NCSD");
    put(&mut image, 0x120, &1u32.to_le_bytes());
    put(&mut image, 0x300, b"NCCH");
    put(&mut image, 0x350, product.as_bytes());
    image[0x38F] = 0x04; // no crypto
    let units = |len: usize| (len / 0x200) as u32;
    put(&mut image, 0x3A0, &1u32.to_le_bytes());
    put(&mut image, 0x3A4, &units(exefs_image.len()).to_le_bytes());
    put(
        &mut image,
        0x3B0,
        &(1 + units(exefs_image.len())).to_le_bytes(),
    );
    put(&mut image, 0x3B4, &units(romfs_image.len()).to_le_bytes());
    image.extend(exefs_image);
    image.extend(romfs_image);
    image
}

/// A version 4 GARC holding `entries`, each a list of files.
pub fn garc(entries: &[&[&[u8]]]) -> Vec<u8> {
    let mut offsets = Vec::new();
    let mut table = Vec::new();
    let mut data = Vec::new();
    for files in entries {
        offsets.extend((table.len() as u32).to_le_bytes());
        table.extend(((1u64 << files.len()) - 1).to_le_bytes()[..4].to_vec());
        for file in files.iter() {
            let start = data.len() as u32;
            data.extend(*file);
            data.resize(data.len().next_multiple_of(4), 0);
            for value in [start, data.len() as u32, file.len() as u32] {
                table.extend(value.to_le_bytes());
            }
        }
    }
    let count = entries.iter().map(|files| files.len()).sum::<usize>() as u32;
    let fato_len = 12 + offsets.len();
    let fatb_len = 12 + table.len();
    let data_at = 0x1C + fato_len + fatb_len + 12;
    let mut bytes = Vec::new();
    bytes.extend(b"CRAG\x1C\0\0\0\xFF\xFE\x00\x04\x04\0\0\0");
    bytes.extend((data_at as u32).to_le_bytes());
    bytes.extend(((data_at + data.len()) as u32).to_le_bytes());
    bytes.extend(0u32.to_le_bytes());
    bytes.extend(b"OTAF");
    bytes.extend((fato_len as u32).to_le_bytes());
    bytes.extend((entries.len() as u16).to_le_bytes());
    bytes.extend(0xFFFFu16.to_le_bytes());
    bytes.extend(offsets);
    bytes.extend(b"BTAF");
    bytes.extend((fatb_len as u32).to_le_bytes());
    bytes.extend(count.to_le_bytes());
    bytes.extend(table);
    bytes.extend(b"BMIF\x0C\0\0\0");
    bytes.extend((data.len() as u32).to_le_bytes());
    bytes.extend(data);
    bytes
}
