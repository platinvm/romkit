//! NCER, cells: sprites assembled from hardware objects (OAM entries), each a block of tiles
//! placed relative to the cell's origin. A `CEBK` chunk holds the cell count, whether each cell
//! carries a bounding box, the offset of the cell table, and the tile mapping. Each cell entry
//! has its object count, an attribute word, and the offset of its objects (6 bytes each, the
//! three OAM attribute words) after the table; a bounding box adds 8 bytes to every entry.

use super::ncgr::draw_tile;
use super::{Palette, Tiles, chunk};
use crate::Result;
use crate::io::Reader;
use crate::media::Image;

const FORMAT: &str = "NCER";
/// Object sizes in pixels by shape (square, wide, tall) and size code.
const SIZES: [[(u8, u8); 4]; 3] = [
    [(8, 8), (16, 16), (32, 32), (64, 64)],
    [(16, 8), (32, 8), (32, 16), (64, 32)],
    [(8, 16), (8, 32), (16, 32), (32, 64)],
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cells {
    pub cells: Vec<Cell>,
    /// How tile numbers count, as in [`Tiles::step`].
    pub step: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// The first object is drawn on top.
    pub objects: Vec<Object>,
}

/// One hardware object: a rectangle of tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Object {
    /// The top left, relative to the cell's origin.
    pub x: i16,
    pub y: i16,
    pub width: u8,
    pub height: u8,
    /// The first tile, counted in steps of the mapping.
    pub tile: u16,
    /// The palette bank, for 4-bit graphics.
    pub bank: u8,
    pub flip_x: bool,
    pub flip_y: bool,
}

impl Cells {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Reader::new(FORMAT, bytes).expect(b"RECN")?;
        let cebk = chunk(FORMAT, bytes, b"KBEC")?;
        let count = cebk.u16_at(0)? as usize;
        let entry_len = match cebk.u16_at(2)? {
            0 => 8,
            _ => 16,
        };
        let table = cebk.u32_at(4)? as usize;
        let mapping = cebk.u32_at(8)?;
        let objects_at = table.saturating_add(count * entry_len);
        // Cells may share objects, but never so many that a small file decodes to a huge one.
        let mut budget = cebk.len();
        let cells = (0..count)
            .map(|index| {
                let entry = table.saturating_add(index * entry_len);
                let first = objects_at.saturating_add(cebk.u32_at(entry + 4)? as usize);
                let count = cebk.u16_at(entry)? as usize;
                budget = budget
                    .checked_sub(count)
                    .ok_or_else(|| cebk.error_at(entry, "too many objects"))?;
                let objects = (0..count)
                    .map(|object| {
                        let at = first.saturating_add(object * 6);
                        let attributes =
                            [cebk.u16_at(at)?, cebk.u16_at(at + 2)?, cebk.u16_at(at + 4)?];
                        Ok(Object::from_attributes(attributes))
                    })
                    .collect::<Result<_>>()?;
                Ok(Cell { objects })
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            cells,
            // 0–3 are the 1D mappings with steps of 32 to 256 bytes, 4 the 2D mapping.
            step: (mapping < 4).then(|| 32 << mapping),
        })
    }

    /// Cell `index` drawn with `tiles` and `palette`, cropped to its objects. Cells that do not
    /// exist, or have no objects, are empty images.
    pub fn image(&self, index: usize, tiles: &Tiles, palette: &Palette) -> Image {
        let Some(cell) = self.cells.get(index) else {
            return Image::new(0, 0);
        };
        let (left, top, width, height) = cell.bounds();
        let mut image = Image::new(width, height);
        let tile_len = tiles.depth.tile_len();
        for object in cell.objects.iter().rev() {
            let colors = tiles.colors(palette, object.bank);
            let start = object.tile as usize * self.step.unwrap_or(32);
            let (columns, rows) = (object.width as usize / 8, object.height as usize / 8);
            for row in 0..rows {
                for column in 0..columns {
                    let offset = match self.step {
                        Some(_) => start + (row * columns + column) * tile_len,
                        // The 2D mapping's sheet is 32 4-bit tiles (1024 bytes) wide.
                        None => start + row * 1024 + column * tile_len,
                    };
                    let (x, y) = (
                        if object.flip_x {
                            columns - 1 - column
                        } else {
                            column
                        },
                        if object.flip_y { rows - 1 - row } else { row },
                    );
                    let x = (object.x as i32 - left) as u32 + x as u32 * 8;
                    let y = (object.y as i32 - top) as u32 + y as u32 * 8;
                    let tile = tiles.tile(offset / tile_len);
                    draw_tile(
                        &mut image,
                        &tile,
                        colors,
                        (x, y),
                        (object.flip_x, object.flip_y),
                    );
                }
            }
        }
        image
    }
}

impl Cell {
    /// The rectangle its objects cover: left and top relative to the origin, width, height.
    pub fn bounds(&self) -> (i32, i32, u32, u32) {
        let edges = |object: &Object| {
            let (x, y) = (object.x as i32, object.y as i32);
            (x, y, x + object.width as i32, y + object.height as i32)
        };
        let Some(bounds) = self
            .objects
            .iter()
            .map(edges)
            .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))
        else {
            return (0, 0, 0, 0);
        };
        let (left, top, right, bottom) = bounds;
        (left, top, (right - left) as u32, (bottom - top) as u32)
    }
}

impl Object {
    /// Decode the three OAM attribute words. Attribute 0 holds y (8 bits, signed), the affine
    /// and double-size flags (bits 8–9), and the shape (bits 14–15); attribute 1 holds x
    /// (9 bits, signed), the flips (bits 12–13, unless affine), and the size code (bits 14–15);
    /// attribute 2 the tile number (10 bits) and the palette bank (bits 12–15).
    fn from_attributes([zero, one, two]: [u16; 3]) -> Self {
        let affine = zero & 0x100 != 0;
        let shape = (zero >> 14).min(2) as usize;
        let (width, height) = SIZES[shape][(one >> 14) as usize];
        let mut x = ((one & 0x1FF) as i16) << 7 >> 7;
        let mut y = (zero as u8 as i8) as i16;
        // A double-size affine object centers its sprite in an area twice as large.
        if affine && zero & 0x200 != 0 {
            x += width as i16 / 2;
            y += height as i16 / 2;
        }
        Self {
            x,
            y,
            width,
            height,
            tile: two & 0x3FF,
            bank: (two >> 12) as u8,
            flip_x: !affine && one & 0x1000 != 0,
            flip_y: !affine && one & 0x2000 != 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::g2d::testing::{ncgr, nitro, words};

    /// One cell of two objects: a 16×8 at (-8, -4) using tiles 0–1, and an 8×8 at (8, -4),
    /// flipped horizontally, using tile 2 with palette bank 1.
    fn ncer() -> Vec<u8> {
        let mut body = words(&[1, 0]);
        body.extend(12u32.to_le_bytes()); // the cell table follows the mapping
        body.extend(0u32.to_le_bytes()); // 1D, 32-byte steps
        body.extend(words(&[2, 0, 0, 0]));
        body.extend(words(&[0x40FC, 0x01F8, 0x0000]));
        body.extend(words(&[0x00FC, 0x1008, 0x1002]));
        nitro(b"RECN", &[(b"KBEC", &body)])
    }

    #[test]
    fn decodes_objects() {
        let cells = Cells::parse(&ncer()).unwrap();
        assert_eq!(cells.step, Some(32));
        let [first, second] = cells.cells[0].objects[..] else {
            panic!("two objects");
        };
        assert_eq!(
            (first.x, first.y, first.width, first.height),
            (-8, -4, 16, 8)
        );
        assert_eq!((second.x, second.tile, second.bank), (8, 2, 1));
        assert!(second.flip_x && !first.flip_x);
        assert_eq!(cells.cells[0].bounds(), (-8, -4, 24, 8));
    }

    #[test]
    fn assembles_cells() {
        // Tiles 1 and 2 each have index 1 at their top left.
        let mut data = vec![0; 96];
        data[32] = 0x01;
        data[64] = 0x01;
        let tiles = Tiles::parse(&ncgr((0xFFFF, 0xFFFF), 3, 0, &data)).unwrap();
        let mut colors = vec![[0; 4]; 32];
        colors[1] = [255, 0, 0, 255];
        colors[17] = [0, 0, 255, 255];
        let image = Cells::parse(&ncer())
            .unwrap()
            .image(0, &tiles, &Palette { colors });
        assert_eq!((image.width, image.height), (24, 8));
        assert_eq!(image.get(8, 0), Some([255, 0, 0, 255]));
        // The flipped tile's top left pixel lands at its top right.
        assert_eq!(image.get(23, 0), Some([0, 0, 255, 255]));
        assert_eq!(image.get(16, 0), Some([0; 4]));
    }

    #[test]
    fn rejects_objects_past_the_end() {
        let mut bytes = ncer();
        bytes.truncate(bytes.len() - 2);
        assert!(Cells::parse(&bytes).is_err());
    }
}
