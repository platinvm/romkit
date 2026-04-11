//! Nintendo 3DS card images (NCSD).
//!
//! Only identification is supported: it comes from the header of the first partition (NCCH),
//! which is never encrypted, unlike the partition contents.

use super::{GameCode, Identity, Platform};
use crate::io::{ByteSource, Reader, ascii};
use crate::{Error, Result};

/// Offsets and sizes in NCSD tables count 0x200-byte media units.
const MEDIA_UNIT: u64 = 0x200;

pub fn is_n3ds(head: &[u8]) -> bool {
    head.get(0x100..0x104) == Some(b"NCSD")
}

pub fn identify<S: ByteSource + ?Sized>(source: &S) -> Result<Identity> {
    let ncsd = source.read(0, 0x200)?;
    let partition = Reader::new("NCSD header", &ncsd).u32_at(0x120)? as u64 * MEDIA_UNIT;
    let ncch = source.read(partition, 0x200).map_err(|_| {
        Error::malformed(
            "NCSD header",
            0x120,
            "first partition lies outside the image",
        )
    })?;
    let mut header = Reader::new("NCCH header", &ncch);
    header.seek(0x100)?;
    header.expect(b"NCCH")?;
    // The product code reads like `CTR-P-EKJA`; its last part is the game code.
    let product = ascii(header.slice_at(0x150, 16)?);
    let code = product
        .rsplit('-')
        .next()
        .and_then(|code| code.as_bytes().try_into().ok())
        .map(GameCode)
        .ok_or_else(|| header.error_at(0x150, "product code has no game code"))?;
    Ok(Identity {
        platform: Platform::Nintendo3ds,
        title: product,
        code: Some(code),
        region: code.region(),
        revision: header.u16_at(0x112)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_the_first_partition() {
        let mut image = vec![0; 0x600];
        image[0x100..0x104].copy_from_slice(b"NCSD");
        image[0x120] = 2; // first partition at media unit 2 (0x400)
        image[0x500..0x504].copy_from_slice(b"NCCH");
        image[0x512] = 1;
        image[0x550..0x55A].copy_from_slice(b"CTR-P-EKJA");

        assert!(is_n3ds(&image));
        let identity = identify(image.as_slice()).unwrap();
        assert_eq!(identity.title, "CTR-P-EKJA");
        assert_eq!(identity.code, Some(GameCode(*b"EKJA")));
        assert_eq!(identity.revision, 1);

        image[0x120] = 9;
        assert!(identify(image.as_slice()).is_err());
    }
}
