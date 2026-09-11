//! RomFS, the 3DS filesystem, stored as the last level of an IVFC hash tree.
//!
//! The upper IVFC levels only hash the levels below them, so romkit skips to level 3. It starts
//! with a header locating four tables and the file data: directory and file hash tables, which
//! only speed up lookups by name, and directory and file metadata. Each metadata entry links to
//! its next sibling, and each directory to its first child directory and its first file, by
//! offset into their table; the root directory is at offset 0.

use std::ops::Range;

use crate::io::{ByteSource, Reader};
use crate::{Error, Result};

const FORMAT: &str = "RomFS";
const IVFC_HEADER_LEN: u64 = 0x60;
const LEVEL3_HEADER_LEN: u64 = 0x28;
/// Directory entries are 0x18 bytes and file entries 0x20, both before their names.
const DIRECTORY_ENTRY_LEN: usize = 0x18;
const FILE_ENTRY_LEN: usize = 0x20;
/// Marks the end of a sibling list, or a directory without children or files.
const NONE: u32 = 0xFFFF_FFFF;
/// Metadata tables of real games are tens of kilobytes; this bounds allocations.
const MAX_TABLE_LEN: u64 = 16 << 20;
/// File names are UTF-16, and real ones are short.
const MAX_NAME_LEN: usize = 0x200;

/// Every file under `region`, by path, with where it lies in the source.
pub(super) fn parse<S: ByteSource + ?Sized>(
    source: &S,
    region: Range<u64>,
) -> Result<Vec<(String, Range<u64>)>> {
    let ivfc = read(source, &region, 0, IVFC_HEADER_LEN)?;
    let mut ivfc = Reader::new("IVFC header", &ivfc);
    ivfc.expect(b"IVFC")?;
    // Level 3 follows the header and the master hashes, aligned to its block size.
    let master_hash_len = ivfc.u32_at(0x08)? as u64;
    let block = match ivfc.u32_at(0x4C)? {
        log2 @ 0..=31 => 1u64 << log2,
        _ => return Err(ivfc.error_at(0x4C, "block size is out of range")),
    };
    let level3 = (IVFC_HEADER_LEN + master_hash_len).next_multiple_of(block);

    let header = read(source, &region, level3, LEVEL3_HEADER_LEN)?;
    let header = Reader::new(FORMAT, &header);
    let table = |at: usize| -> Result<Vec<u8>> {
        let (offset, len) = (header.u32_at(at)? as u64, header.u32_at(at + 4)? as u64);
        if len > MAX_TABLE_LEN {
            return Err(header.error_at(at, "metadata table is too large"));
        }
        Ok(read(source, &region, level3 + offset, len)?.into_owned())
    };
    let directories = table(0x0C)?;
    let files = table(0x1C)?;
    let data = region.start + level3 + header.u32_at(0x24)? as u64;
    Tree {
        directories: Reader::new(FORMAT, &directories),
        files: Reader::new(FORMAT, &files),
        data,
        end: region.end,
    }
    .walk()
}

/// `len` bytes at `offset` within `region`.
fn read<'s, S: ByteSource + ?Sized>(
    source: &'s S,
    region: &Range<u64>,
    offset: u64,
    len: u64,
) -> Result<std::borrow::Cow<'s, [u8]>> {
    match offset.checked_add(len) {
        Some(end) if region.start + end <= region.end => source.read(region.start + offset, len),
        _ => Err(Error::malformed(
            FORMAT,
            offset,
            "table lies outside the RomFS",
        )),
    }
}

struct Tree<'a> {
    directories: Reader<'a>,
    files: Reader<'a>,
    /// Where file data starts in the source, and where the RomFS ends.
    data: u64,
    end: u64,
}

impl Tree<'_> {
    /// Visit every directory from the root. Each entry can be visited at most once in a
    /// well-formed tree, so visiting more entries than the tables hold means a cycle.
    fn walk(&self) -> Result<Vec<(String, Range<u64>)>> {
        let max_directories = self.directories.len() / DIRECTORY_ENTRY_LEN;
        let max_files = self.files.len() / FILE_ENTRY_LEN;
        let mut found = Vec::new();
        let mut pending = vec![(0, String::new())];
        let mut visited = 0;
        while let Some((directory, prefix)) = pending.pop() {
            let at = directory as usize;
            let mut child = self.directories.u32_at(at + 8)?;
            while child != NONE {
                visited += 1;
                if visited > max_directories {
                    return Err(self.directories.error_at(at, "directory tree has a cycle"));
                }
                let name = name(&self.directories, child as usize + DIRECTORY_ENTRY_LEN - 4)?;
                pending.push((child, join(&prefix, &name)));
                child = self.directories.u32_at(child as usize + 4)?;
            }
            let mut file = self.directories.u32_at(at + 12)?;
            while file != NONE {
                if found.len() == max_files {
                    return Err(self.files.error_at(file as usize, "file list has a cycle"));
                }
                let at = file as usize;
                let (offset, len) = (u64_at(&self.files, at + 8)?, u64_at(&self.files, at + 16)?);
                let start = self.data.saturating_add(offset);
                let end = start.saturating_add(len);
                if end > self.end {
                    return Err(self.files.error_at(at, "file lies outside the RomFS"));
                }
                let name = name(&self.files, at + FILE_ENTRY_LEN - 4)?;
                found.push((join(&prefix, &name), start..end));
                file = self.files.u32_at(at + 4)?;
            }
        }
        Ok(found)
    }
}

fn u64_at(table: &Reader, at: usize) -> Result<u64> {
    Ok(table.u32_at(at)? as u64 | (table.u32_at(at + 4)? as u64) << 32)
}

/// The UTF-16 name whose byte length is stored at `at`, just before the name itself.
fn name(table: &Reader, at: usize) -> Result<String> {
    let len = table.u32_at(at)? as usize;
    if len > MAX_NAME_LEN || !len.is_multiple_of(2) {
        return Err(table.error_at(at, "name length is invalid"));
    }
    let units: Vec<u16> = table
        .slice_at(at + 4, len)?
        .as_chunks()
        .0
        .iter()
        .map(|&unit| u16::from_le_bytes(unit))
        .collect();
    match String::from_utf16(&units) {
        Ok(name) if !name.is_empty() && !name.contains('/') && name != "." && name != ".." => {
            Ok(name)
        }
        _ => Err(table.error_at(at, "invalid file name")),
    }
}

fn join(prefix: &str, name: &str) -> String {
    match prefix.is_empty() {
        true => name.to_owned(),
        false => format!("{prefix}/{name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::romfs;

    #[test]
    fn walks_nested_directories() {
        let image = romfs(&[("a/b/c", b"deep"), ("a/x", b"mid"), ("top", b"top")]);
        let mut files = parse(image.as_slice(), 0..image.len() as u64).unwrap();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        let read = |range: &Range<u64>| &image[range.start as usize..range.end as usize];
        let paths: Vec<_> = files.iter().map(|(path, _)| path.as_str()).collect();
        assert_eq!(paths, ["a/b/c", "a/x", "top"]);
        assert_eq!(read(&files[0].1), b"deep");
        assert_eq!(read(&files[2].1), b"top");
    }

    #[test]
    fn rejects_cycles_and_files_outside() {
        let image = romfs(&[("a/b", b"x")]);
        let level3 = 0x1000;
        let u32_at =
            |image: &[u8], at: usize| u32::from_le_bytes(image[at..][..4].try_into().unwrap());
        let directories = level3 + u32_at(&image, level3 + 0x0C) as usize;
        let files = level3 + u32_at(&image, level3 + 0x1C) as usize;
        let patch = |at: usize, value: u32| {
            let mut patched = image.clone();
            patched[at..][..4].copy_from_slice(&value.to_le_bytes());
            parse(patched.as_slice(), 0..patched.len() as u64)
        };
        assert!(patch(files + 16, 1).is_ok());

        // The root's first child directory is the root itself.
        assert!(patch(directories + 8, 0).is_err());
        // The root's first child is its own next sibling.
        let child = u32_at(&image, directories + 8);
        assert!(patch(directories + child as usize + 4, child).is_err());
        // The file runs past the end.
        assert!(patch(files + 16, 0x10_0000).is_err());
    }
}
