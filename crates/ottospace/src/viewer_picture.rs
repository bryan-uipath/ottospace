//! A picture in the file viewer: decoded off the main thread with the `image` crate under limits
//! (`viewer::MAX_PIXELS`, every frame together; a hostile file is said, never a crash), each
//! frame as GPUI's `RenderImage`, then drawn fitted or zoomed over a checker, with its size in the
//! pane's foot. An animation's frames are timed here and shown only while its pane is on show.

use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use gpui::RenderImage;
use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder, Frame, ImageDecoder, ImageFormat, ImageReader, Limits};
use ottospace_core::viewer::{MAX_PIXELS, Read};

/// The longest side decoded, in pixels.
const MAX_SIDE: u32 = 32_768;
/// A frame shown for less than this plays at `SLOW` instead, as browsers do (a GIF's 0 ms).
const FASTEST: Duration = Duration::from_millis(20);
const SLOW: Duration = Duration::from_millis(100);

/// A decoded picture: its size in pixels and its frames, each with how long it shows.
pub(crate) struct Picture {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) frames: Vec<(Arc<RenderImage>, Duration)>,
}

impl Picture {
    /// The frame on show `elapsed` after it began to play, looping.
    pub(crate) fn frame_at(&self, elapsed: Duration) -> usize {
        let total: Duration = self.frames.iter().map(|(_, shows)| *shows).sum();
        if self.frames.len() < 2 || total.is_zero() {
            return 0;
        }
        let mut left = Duration::from_nanos((elapsed.as_nanos() % total.as_nanos()) as u64);
        for (at, (_, shows)) in self.frames.iter().enumerate() {
            if left < *shows {
                return at;
            }
            left -= *shows;
        }
        0
    }

    /// How long after it began to play the frame after `elapsed` shows.
    pub(crate) fn next_change(&self, elapsed: Duration) -> Option<Duration> {
        let total: Duration = self.frames.iter().map(|(_, shows)| *shows).sum();
        if self.frames.len() < 2 || total.is_zero() {
            return None;
        }
        let loops = elapsed.as_nanos() / total.as_nanos();
        let mut at = Duration::from_nanos((loops * total.as_nanos()) as u64);
        for (_, shows) in &self.frames {
            at += *shows;
            if at > elapsed {
                return Some(at);
            }
        }
        Some(at)
    }
}

/// `bytes` decoded, or what to say instead: `Binary` for no picture format the viewer takes,
/// `TooManyPixels` past the cap, `Broken` for anything its decoder refused (or panicked on).
pub(crate) fn decode(bytes: &[u8]) -> Result<Picture, Read> {
    let format = match image::guess_format(bytes) {
        Ok(
            format @ (ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::Gif
            | ImageFormat::WebP
            | ImageFormat::Bmp
            | ImageFormat::Tiff),
        ) => format,
        _ => return Err(Read::Binary),
    };
    std::panic::catch_unwind(|| decode_as(bytes, format)).unwrap_or(Err(Read::Broken))
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    (limits.max_image_width, limits.max_image_height) = (Some(MAX_SIDE), Some(MAX_SIDE));
    // Its colour, and room for what a decoder keeps besides.
    limits.max_alloc = Some(MAX_PIXELS * 4 + (64 << 20));
    limits
}

fn decode_as(bytes: &[u8], format: ImageFormat) -> Result<Picture, Read> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits());
    let (width, height) = reader.into_dimensions().map_err(|_| Read::Broken)?;
    if width == 0 || height == 0 {
        return Err(Read::Broken);
    }
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_PIXELS {
        return Err(Read::TooManyPixels { width, height, frames: 1 });
    }
    let frames = match format {
        ImageFormat::Gif => {
            let mut decoder = GifDecoder::new(Cursor::new(bytes)).map_err(|_| Read::Broken)?;
            decoder.set_limits(limits()).map_err(|_| Read::Broken)?;
            animated(decoder.into_frames(), width, height, MAX_PIXELS)?
        }
        ImageFormat::WebP => {
            let mut decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(|_| Read::Broken)?;
            decoder.set_limits(limits()).map_err(|_| Read::Broken)?;
            if decoder.has_animation() {
                animated(decoder.into_frames(), width, height, MAX_PIXELS)?
            } else {
                vec![still(image::DynamicImage::from_decoder(decoder).map_err(|_| Read::Broken)?)]
            }
        }
        _ => {
            let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
            reader.limits(limits());
            vec![still(reader.decode().map_err(|_| Read::Broken)?)]
        }
    };
    if frames.is_empty() {
        return Err(Read::Broken);
    }
    Ok(Picture { width, height, frames })
}

fn still(image: image::DynamicImage) -> (Arc<RenderImage>, Duration) {
    (render(Frame::new(image.into_rgba8())), Duration::ZERO)
}

/// Each frame of an animation, until its pixels together pass `most`.
fn animated(
    frames: image::Frames<'_>,
    width: u32,
    height: u32,
    most: u64,
) -> Result<Vec<(Arc<RenderImage>, Duration)>, Read> {
    let mut kept = Vec::new();
    let mut pixels = 0u64;
    for frame in frames {
        let frame = frame.map_err(|_| Read::Broken)?;
        let buffer = frame.buffer();
        pixels += u64::from(buffer.width()) * u64::from(buffer.height());
        if pixels > most {
            return Err(Read::TooManyPixels { width, height, frames: kept.len() as u32 + 1 });
        }
        let (numer, denom) = frame.delay().numer_denom_ms();
        let shows = Duration::from_micros(u64::from(numer) * 1000 / u64::from(denom.max(1)));
        kept.push((render(frame), if shows < FASTEST { SLOW } else { shows }));
    }
    Ok(kept)
}

/// A frame as GPUI draws it: its colour from RGBA to BGRA.
fn render(mut frame: Frame) -> Arc<RenderImage> {
    for pixel in frame.buffer_mut().chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Arc::new(RenderImage::new([frame]))
}

/// Points a pixel that fit `width` × `height` in `room` (never larger than its own size).
pub(crate) fn fit(room: (f32, f32), width: u32, height: u32) -> f32 {
    let scale = (room.0 / width.max(1) as f32).min(room.1 / height.max(1) as f32);
    if scale.is_finite() && scale > 0. { scale.min(1.) } else { 1. }
}

/// A file's bytes as the foot says them: `820 B`, `14.2 KB`, `3.1 MB`.
pub(crate) fn size(bytes: u64) -> String {
    match bytes {
        0..1000 => format!("{bytes} B"),
        1000..1_000_000 => format!("{:.1} KB", bytes as f64 / 1e3),
        _ => format!("{:.1} MB", bytes as f64 / 1e6),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageEncoder, RgbaImage};

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let pixels = vec![200u8; (width * height * 4) as usize];
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(&pixels, width, height, image::ExtendedColorType::Rgba8)
            .expect("a png");
        out
    }

    fn gif(frames: u32, side: u32, delay_ms: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut out);
            for at in 0..frames {
                let buffer = RgbaImage::from_pixel(side, side, image::Rgba([at as u8 * 40, 0, 0, 255]));
                let delay = image::Delay::from_numer_denom_ms(delay_ms, 1);
                encoder.encode_frame(Frame::from_parts(buffer, 0, 0, delay)).expect("a frame");
            }
        }
        out
    }

    #[test]
    fn a_picture_decodes_to_bgra_frames_and_a_gif_plays_its_delays() {
        let still = decode(&png(3, 2)).expect("decoded");
        assert_eq!((still.width, still.height, still.frames.len()), (3, 2, 1));
        let spin = decode(&gif(3, 4, 50)).expect("decoded");
        assert_eq!((spin.width, spin.frames.len()), (4, 3));
        assert_eq!(spin.frames[1].1, Duration::from_millis(50));
        assert_eq!(spin.frame_at(Duration::from_millis(60)), 1);
        assert_eq!(spin.frame_at(Duration::from_millis(160)), 0, "it loops");
        assert_eq!(spin.next_change(Duration::from_millis(60)), Some(Duration::from_millis(100)));
        // A 0 ms frame plays at 100 ms, as browsers do.
        assert_eq!(decode(&gif(2, 2, 0)).expect("decoded").frames[0].1, SLOW);
    }

    #[test]
    fn a_broken_huge_or_foreign_file_is_said_never_decoded() {
        assert!(matches!(decode(b"plain text"), Err(Read::Binary)));
        let mut cut = png(40, 40);
        cut.truncate(cut.len() / 2);
        assert!(matches!(decode(&cut), Err(Read::Broken)));
        // A header that claims 20,000 × 20,000 (a BMP's, which has no checksum): refused before a
        // byte of it is decoded.
        let mut huge = Vec::new();
        image::codecs::bmp::BmpEncoder::new(&mut huge)
            .write_image(&[1, 2, 3, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .expect("a bmp");
        huge[18..22].copy_from_slice(&20_000i32.to_le_bytes());
        huge[22..26].copy_from_slice(&20_000i32.to_le_bytes());
        assert!(matches!(decode(&huge), Err(Read::TooManyPixels { width: 20_000, height: 20_000, .. })));
        // Frames past the cap together: the third of 4 × 4 frames over a cap of 40 pixels.
        let frames = GifDecoder::new(Cursor::new(gif(5, 4, 50))).expect("a gif").into_frames();
        assert!(matches!(animated(frames, 4, 4, 40), Err(Read::TooManyPixels { frames: 3, .. })));
    }

    #[test]
    fn a_picture_fits_its_room_and_is_never_enlarged() {
        assert_eq!(fit((400., 300.), 800, 600), 0.5);
        assert_eq!(fit((400., 300.), 40, 30), 1.);
        assert_eq!(fit((0., 0.), 40, 30), 1.);
        assert_eq!((size(820), size(14_200), size(3_100_000)), ("820 B".into(), "14.2 KB".into(), "3.1 MB".into()));
    }
}
