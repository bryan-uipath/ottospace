//! A picture's size from its first bytes, and the size a thumbnail of it is drawn at. Pure.

/// The most a thumbnail takes, in points, and the least its longer side is drawn at: a 16-pixel
/// icon is scaled up by whole steps rather than drawn as a dot.
pub const MAX: (f32, f32) = (320., 200.);
pub const LEAST: f32 = 64.;

/// Width and height of a PNG, GIF, JPEG or WebP, read from its header; `None` for anything else.
pub fn size(bytes: &[u8]) -> Option<(u32, u32)> {
    let be16 = |at: usize| Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as u32);
    let le16 = |at: usize| Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as u32);
    let be32 = |at: usize| Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let le24 = |at: usize| Some(le16(at)? | (*bytes.get(at + 2)? as u32) << 16);
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some((be32(16)?, be32(20)?)),
        [b'G', b'I', b'F', b'8', ..] => Some((le16(6)?, le16(8)?)),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', b'V', b'P', b'8', kind, ..] => match kind {
            b' ' => Some((le16(26)? & 0x3fff, le16(28)? & 0x3fff)),
            b'L' => {
                let bits = u32::from_le_bytes(bytes.get(21..25)?.try_into().ok()?);
                Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
            }
            b'X' => Some((le24(24)? + 1, le24(27)? + 1)),
            _ => None,
        },
        [0xff, 0xd8, ..] => {
            // Segments to the first start-of-frame (SOF0..SOF15 but DHT, JPG, DAC).
            let mut at = 2;
            while *bytes.get(at)? == 0xff {
                let marker = *bytes.get(at + 1)?;
                if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
                    return Some((be16(at + 7)?, be16(at + 5)?));
                }
                at += 2 + be16(at + 2)? as usize;
            }
            None
        }
        _ => None,
    }
}

/// The size a `w`×`h` picture is drawn at: within `MAX`, aspect kept; one smaller than `LEAST`
/// on its longer side scaled up by a whole factor (crisp steps) until it is not.
pub fn shown(w: u32, h: u32) -> (f32, f32) {
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let fit = (MAX.0 / w).min(MAX.1 / h);
    let scale = if fit < 1. { fit } else { (LEAST / w.max(h)).ceil().clamp(1., fit.floor()) };
    (w * scale, h * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixtures_picture_is_read_and_drawn_larger() {
        let png = include_bytes!("../tests/fixtures/markdown/checker.png");
        assert_eq!(size(png), Some((16, 16)));
        assert_eq!(shown(16, 16), (64., 64.));
    }

    #[test]
    fn each_format_and_every_size_fits() {
        let gif = [b'G', b'I', b'F', b'8', b'9', b'a', 40, 1, 30, 0];
        assert_eq!(size(&gif), Some((296, 30)));
        // A JPEG: SOI, an APP0 of 4 bytes, then SOF0 with height 100 and width 200.
        let jpeg = [0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 11, 8, 0, 100, 0, 200];
        assert_eq!(size(&jpeg), Some((200, 100)));
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
        webp.extend([0; 8]);
        webp.extend([99, 0, 0, 49, 0, 0]);
        assert_eq!(size(&webp), Some((100, 50)));
        assert_eq!(size(b"not a picture"), None);
        assert_eq!(size(&[0xff, 0xd8, 0xff]), None);
        // Large ones shrink to fit, small ones grow by whole steps, none past `MAX`.
        assert_eq!(shown(1600, 1000), (320., 200.));
        assert_eq!(shown(10, 20), (40., 80.));
        assert_eq!(shown(100, 80), (100., 80.));
        for (w, h) in [(1, 1), (1, 10_000), (10_000, 1), (0, 0), (63, 2), (500, 3)] {
            let (sw, sh) = shown(w, h);
            assert!(sw <= MAX.0 + 0.01 && sh <= MAX.1 + 0.01 && sw > 0. && sh > 0., "{w}x{h} → {sw}x{sh}");
        }
    }
}
