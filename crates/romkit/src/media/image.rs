use crate::{Error, Result};

/// An 8-bit RGBA color.
pub type Rgba = [u8; 4];

/// A color in the 15-bit BGR555 format of the GBA and DS (bit 15 is ignored), as opaque RGBA.
/// Each 5-bit channel is scaled to 8 bits by repeating its high bits, so white stays white.
pub fn bgr555(color: u16) -> Rgba {
    let channel = |shift: u16| {
        let value = (color >> shift & 0x1F) as u8;
        value << 3 | value >> 2
    };
    [channel(0), channel(5), channel(10), 0xFF]
}

/// An RGBA image, row by row from the top left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<Rgba>,
}

impl Image {
    /// A fully transparent image.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![[0; 4]; (width * height) as usize],
        }
    }

    /// An image of palette indices, given row by row. Index 0 is transparent when
    /// `transparent_zero` is set, as for most sprites; indices past the palette are transparent.
    pub fn from_indices(
        width: u32,
        height: u32,
        indices: &[u8],
        palette: &[Rgba],
        transparent_zero: bool,
    ) -> Self {
        let pixels = indices
            .iter()
            .take((width * height) as usize)
            .map(|&index| match palette.get(index as usize) {
                Some(_) if index == 0 && transparent_zero => [0; 4],
                Some(&color) => color,
                None => [0; 4],
            })
            .collect::<Vec<_>>();
        let mut image = Self::new(width, height);
        image.pixels[..pixels.len()].copy_from_slice(&pixels);
        image
    }

    pub fn get(&self, x: u32, y: u32) -> Option<Rgba> {
        (x < self.width && y < self.height).then(|| self.pixels[(y * self.width + x) as usize])
    }

    /// Set a pixel; out-of-bounds writes are ignored, so callers can draw clipped shapes.
    pub fn set(&mut self, x: u32, y: u32, color: Rgba) {
        if x < self.width && y < self.height {
            self.pixels[(y * self.width + x) as usize] = color;
        }
    }

    /// Copy `other` onto this image with its top left at `(x, y)`, skipping transparent pixels.
    pub fn draw(&mut self, other: &Image, x: i32, y: i32) {
        for row in 0..other.height {
            for column in 0..other.width {
                let color = other.pixels[(row * other.width + column) as usize];
                let (tx, ty) = (x + column as i32, y + row as i32);
                if color[3] != 0 && tx >= 0 && ty >= 0 {
                    self.set(tx as u32, ty as u32, color);
                }
            }
        }
    }

    /// Encode as PNG.
    pub fn to_png(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, self.width.max(1), self.height.max(1));
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let failed = |error: png::EncodingError| Error::Host(format!("PNG encoding: {error}"));
        let mut writer = encoder.write_header().map_err(failed)?;
        let data = match self.width == 0 || self.height == 0 {
            true => vec![0; 4],
            false => self.pixels.concat(),
        };
        writer.write_image_data(&data).map_err(failed)?;
        writer.finish().map_err(failed)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgr555_scales_channels_to_full_range() {
        assert_eq!(bgr555(0x7FFF), [255, 255, 255, 255]);
        assert_eq!(bgr555(0x001F), [255, 0, 0, 255]);
        assert_eq!(bgr555(0x7C00), [0, 0, 255, 255]);
        assert_eq!(bgr555(0x8000), [0, 0, 0, 255]);
    }

    #[test]
    fn indexed_images_map_through_the_palette() {
        let palette = [[1, 2, 3, 255], [4, 5, 6, 255]];
        let image = Image::from_indices(2, 1, &[0, 1], &palette, true);
        assert_eq!(image.pixels, [[0; 4], [4, 5, 6, 255]]);
        let opaque = Image::from_indices(2, 1, &[0, 7], &palette, false);
        assert_eq!(opaque.pixels, [[1, 2, 3, 255], [0; 4]]);
    }

    #[test]
    fn encodes_png() {
        let mut image = Image::new(3, 2);
        image.set(1, 1, [255, 0, 0, 255]);
        let png = image.to_png().unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert!(Image::new(0, 0).to_png().is_ok());
    }
}
