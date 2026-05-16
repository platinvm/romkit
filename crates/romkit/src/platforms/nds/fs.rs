//! NitroFS, the DS filesystem: a file allocation table (FAT) of byte ranges in the ROM, and a
//! file name table (FNT) that names them as a directory tree.

use std::collections::BTreeMap;
use std::ops::Range;

use crate::Result;
use crate::io::Reader;

/// Directory IDs start here; the root directory has the first one.
const ROOT: u16 = 0xF000;
const MAX_DIRECTORIES: usize = 0x1000;

#[derive(Debug, Clone)]
pub struct FileSystem {
    ranges: Vec<Range<u64>>,
    paths: BTreeMap<String, u16>,
}

impl FileSystem {
    pub fn parse(fnt: &[u8], fat: &[u8], rom_size: u64) -> Result<Self> {
        let ranges = parse_fat(fat, rom_size)?;
        let paths = parse_fnt(fnt, ranges.len())?;
        Ok(Self { ranges, paths })
    }

    /// The FAT ID of the file at `path` (`/`-separated, optionally with a leading `/`).
    pub fn id(&self, path: &str) -> Option<u16> {
        self.paths.get(path.trim_start_matches('/')).copied()
    }

    /// Where a FAT entry lies in the ROM.
    pub fn range(&self, id: u16) -> Option<Range<u64>> {
        self.ranges.get(id as usize).cloned()
    }

    /// Named files and their FAT IDs, in path order. Overlays have FAT entries but no names.
    pub fn paths(&self) -> impl ExactSizeIterator<Item = (&str, u16)> {
        self.paths.iter().map(|(path, &id)| (path.as_str(), id))
    }
}

fn parse_fat(fat: &[u8], rom_size: u64) -> Result<Vec<Range<u64>>> {
    let mut table = Reader::new("DS file allocation table", fat);
    let mut ranges = Vec::with_capacity(fat.len() / 8);
    while table.remaining() >= 8 {
        let (start, end) = (table.u32()? as u64, table.u32()? as u64);
        if start > end || end > rom_size {
            return Err(table.error_at(table.position() - 8, "file lies outside the ROM"));
        }
        ranges.push(start..end);
    }
    Ok(ranges)
}

/// Walk the directory tree from the root. The main table holds 8 bytes per directory: where its
/// entries start, and the FAT ID of its first file (the others follow in order). Each entry is
/// a length byte (high bit set for directories), a name, and for directories their ID.
fn parse_fnt(fnt: &[u8], file_count: usize) -> Result<BTreeMap<String, u16>> {
    let table = Reader::new("DS file name table", fnt);
    let directory_count = table.u16_at(6)? as usize;
    if directory_count == 0 || directory_count > MAX_DIRECTORIES {
        return Err(table.error_at(6, "directory count is out of range"));
    }
    let mut visited = vec![false; directory_count];
    let mut paths = BTreeMap::new();
    let mut pending = vec![(ROOT, String::new())];
    while let Some((directory, prefix)) = pending.pop() {
        let index = directory.wrapping_sub(ROOT) as usize;
        if index >= directory_count || std::mem::replace(&mut visited[index], true) {
            return Err(table.error_at(index * 8, "invalid or repeated directory"));
        }
        let mut entries = table.clone();
        entries.seek(table.u32_at(index * 8)? as usize)?;
        let mut file_id = table.u16_at(index * 8 + 4)?;
        loop {
            let tag = entries.u8()?;
            if tag == 0 {
                break;
            }
            let name = String::from_utf8_lossy(entries.take((tag & 0x7F) as usize)?);
            if name.is_empty() || name.contains('/') || name == "." || name == ".." {
                return Err(entries.error("invalid file name"));
            }
            let path = match prefix.is_empty() {
                true => name.into_owned(),
                false => format!("{prefix}/{name}"),
            };
            if tag & 0x80 != 0 {
                pending.push((entries.u16()?, path));
            } else if (file_id as usize) < file_count && paths.insert(path, file_id).is_none() {
                file_id += 1;
            } else {
                return Err(entries.error("file ID out of range or path repeated"));
            }
        }
    }
    Ok(paths)
}
