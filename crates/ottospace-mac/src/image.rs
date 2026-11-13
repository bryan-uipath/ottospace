//! Pictures of panes, as PNG bytes: encoded from pixels, drawn from a terminal's rows, and with
//! annotations drawn on. All of it draws into a bitmap of its own, off screen.

use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSBezierPath, NSBitmapImageFileType, NSBitmapImageRep, NSColor, NSDeviceRGBColorSpace, NSFont,
    NSFontAttributeName, NSForegroundColorAttributeName, NSGraphicsContext, NSImage, NSLineCapStyle, NSLineJoinStyle,
    NSStringDrawing,
};
use objc2_foundation::{NSData, NSDictionary, NSPoint, NSRect, NSSize, NSString};
use ottospace_core::capture::Shape;

/// An empty RGBA bitmap of that many pixels (null planes: AppKit allocates it).
fn bitmap(width: usize, height: usize) -> Option<Retained<NSBitmapImageRep>> {
    // Past this a picture is no pane's: refuse before AppKit allocates it.
    if width == 0 || height == 0 || width > 16_384 || height > 16_384 {
        return None;
    }
    unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(), std::ptr::null_mut(), width as isize, height as isize, 8, 4, true, false, NSDeviceRGBColorSpace, 0, 0,
        )
    }
}

fn encode(bitmap: &NSBitmapImageRep) -> Option<Vec<u8>> {
    let data = unsafe { bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }?;
    Some(data.to_vec())
}

/// Runs `draw` with the bitmap as the thread's drawing target, one unit a pixel, origin bottom-left.
fn draw_into(bitmap: &NSBitmapImageRep, draw: impl FnOnce()) -> Option<()> {
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(bitmap)?;
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&context));
    draw();
    context.flushGraphics();
    NSGraphicsContext::restoreGraphicsState_class();
    Some(())
}

fn colour(rgb: u32) -> Retained<NSColor> {
    let part = |shift: u32| f64::from((rgb >> shift) & 0xff) / 255.0;
    NSColor::colorWithSRGBRed_green_blue_alpha(part(16), part(8), part(0), 1.0)
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

/// Text attributes: a font and a colour.
fn attributes(font: &NSFont, rgb: u32) -> Retained<NSDictionary<NSString, AnyObject>> {
    let colour = colour(rgb);
    let (font, colour): (&AnyObject, &AnyObject) = (font, &colour);
    unsafe { NSDictionary::from_slices(&[NSFontAttributeName, NSForegroundColorAttributeName], &[font, colour]) }
}

/// `width`×`height` pixels of RGBA, row after row, as a PNG. `None`: the bytes are not that many.
pub fn png_from_rgba(width: usize, height: usize, rgba: &[u8]) -> Option<Vec<u8>> {
    if rgba.len() != width.checked_mul(height)?.checked_mul(4)? {
        return None;
    }
    let bitmap = bitmap(width, height)?;
    let (data, stride) = (bitmap.bitmapData(), usize::try_from(bitmap.bytesPerRow()).ok()?);
    if data.is_null() || stride < width * 4 {
        return None;
    }
    for (row, pixels) in rgba.chunks_exact(width * 4).enumerate() {
        // Each row of the bitmap is `stride` bytes, of which the first `width * 4` are pixels.
        unsafe { std::ptr::copy_nonoverlapping(pixels.as_ptr(), data.add(row * stride), width * 4) };
    }
    encode(&bitmap)
}

/// The pixel size of a PNG, from its header.
pub fn png_size(png: &[u8]) -> Option<(usize, usize)> {
    let be = |at: usize| Some(u32::from_be_bytes(png.get(at..at + 4)?.try_into().ok()?) as usize);
    png.starts_with(b"\x89PNG\r\n\x1a\n").then(|| Some((be(16)?, be(20)?))).flatten()
}

/// `png` with `marks` drawn on it in `rgb`, lines and text sized for a picture `scale` pixels a
/// point. Positions are 0..1 of the picture, from its top left.
pub fn annotate(png: &[u8], marks: &[Shape], rgb: u32, scale: f64) -> Option<Vec<u8>> {
    let (width, height) = png_size(png)?;
    let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(png))?;
    let bitmap = bitmap(width, height)?;
    let (w, h) = (width as f64, height as f64);
    // The picture's y runs down, AppKit's up.
    let at = |p: [f64; 2]| NSPoint::new(p[0] * w, h - p[1] * h);
    let inside = |r: [f64; 4]| rect(r[0] * w, h - (r[1] + r[3]) * h, r[2] * w, r[3] * h);
    draw_into(&bitmap, || {
        image.drawInRect(rect(0.0, 0.0, w, h));
        colour(rgb).set();
        let line = |path: &NSBezierPath| {
            path.setLineWidth(2.5 * scale);
            path.setLineCapStyle(NSLineCapStyle::Round);
            path.setLineJoinStyle(NSLineJoinStyle::Round);
            path.stroke();
        };
        let font = NSFont::boldSystemFontOfSize(13.0 * scale);
        for mark in marks {
            match mark {
                Shape::Ellipse { rect } => line(&NSBezierPath::bezierPathWithOvalInRect(inside(*rect))),
                Shape::Box { rect } => line(&NSBezierPath::bezierPathWithRect(inside(*rect))),
                Shape::Stroke { points } => {
                    let path = NSBezierPath::bezierPath();
                    for (i, point) in points.iter().enumerate() {
                        if i == 0 { path.moveToPoint(at(*point)) } else { path.lineToPoint(at(*point)) }
                    }
                    line(&path);
                }
                // A pill in the mark's colour with dark text, its top left at the point.
                Shape::Note { at: point, text } => {
                    let (text, dark) = (NSString::from_str(text), attributes(&font, 0x09090b));
                    let size = unsafe { text.sizeWithAttributes(Some(&dark)) };
                    let (pad, origin) = (5.0 * scale, at(*point));
                    let pill = rect(origin.x, origin.y - size.height - pad, size.width + 2.0 * pad, size.height + pad);
                    let radius = 4.0 * scale;
                    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(pill, radius, radius).fill();
                    unsafe {
                        text.drawAtPoint_withAttributes(NSPoint::new(pill.origin.x + pad, pill.origin.y + pad / 2.0), Some(&dark))
                    };
                    colour(rgb).set();
                }
            }
        }
    })?;
    encode(&bitmap)
}

/// A run of a terminal row's cells that share a style.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub fg: u32,
    pub bg: Option<u32>,
    pub bold: bool,
}

/// A terminal's rows and how they are drawn, in points.
pub struct Grid<'a> {
    pub rows: &'a [Vec<Run>],
    /// The picture's size, and one cell's.
    pub size: [f64; 2],
    pub cell: [f64; 2],
    pub font: &'a str,
    pub font_size: f64,
    pub background: u32,
    /// Pixels a point.
    pub scale: f64,
}

/// A picture of a terminal's rows: each run at its cell, in its colours, as the pane draws them.
pub fn grid_png(grid: &Grid) -> Option<Vec<u8>> {
    let scale = grid.scale;
    let (w, h) = (grid.size[0] * scale, grid.size[1] * scale);
    let bitmap = bitmap(w.round() as usize, h.round() as usize)?;
    let (cell_w, cell_h) = (grid.cell[0] * scale, grid.cell[1] * scale);
    let size = grid.font_size * scale;
    let named = |name: &str| NSFont::fontWithName_size(&NSString::from_str(name), size);
    let plain = named(grid.font).unwrap_or_else(|| NSFont::userFixedPitchFontOfSize(size).unwrap_or_else(|| NSFont::systemFontOfSize(size)));
    let bold = named(&format!("{}-Bold", grid.font)).unwrap_or_else(|| plain.clone());
    draw_into(&bitmap, || {
        colour(grid.background).set();
        NSBezierPath::fillRect(rect(0.0, 0.0, w, h));
        // A line of text is not as high as a row: it sits in the row's middle.
        let line = unsafe { NSString::from_str("M").sizeWithAttributes(Some(&attributes(&plain, 0))) }.height;
        for (row, runs) in grid.rows.iter().enumerate() {
            let bottom = h - (row + 1) as f64 * cell_h;
            let mut column = 0;
            for run in runs {
                let cells = run.text.chars().count();
                let x = column as f64 * cell_w;
                column += cells;
                if let Some(bg) = run.bg {
                    colour(bg).set();
                    NSBezierPath::fillRect(rect(x, bottom, cells as f64 * cell_w, cell_h));
                }
                if run.text.trim().is_empty() {
                    continue;
                }
                let attributes = attributes(if run.bold { &bold } else { &plain }, run.fg);
                let at = NSPoint::new(x, bottom + (cell_h - line) / 2.0);
                unsafe { NSString::from_str(&run.text).drawAtPoint_withAttributes(at, Some(&attributes)) };
            }
        }
    })?;
    encode(&bitmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The RGBA pixels of a PNG, by way of AppKit's own decoder.
    fn pixels(png: &[u8]) -> (usize, usize, Vec<u8>) {
        let bitmap = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &NSData::with_bytes(png)).expect("a PNG");
        let (w, h) = (bitmap.pixelsWide() as usize, bitmap.pixelsHigh() as usize);
        let (stride, data) = (bitmap.bytesPerRow() as usize, bitmap.bitmapData());
        let mut out = Vec::new();
        for row in 0..h {
            out.extend_from_slice(unsafe { std::slice::from_raw_parts(data.add(row * stride), w * 4) });
        }
        (w, h, out)
    }

    fn pixel(png: &[u8], x: usize, y: usize) -> [u8; 3] {
        let (w, _, data) = pixels(png);
        let at = (y * w + x) * 4;
        [data[at], data[at + 1], data[at + 2]]
    }

    fn near(a: [u8; 3], b: [u8; 3]) -> bool {
        a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 12)
    }

    #[test]
    fn pixels_are_encoded_as_a_png_of_their_size() {
        // 3×2: a red row over a blue row.
        let rgba: Vec<u8> = [[255, 0, 0, 255]; 3].into_iter().chain([[0, 0, 255, 255]; 3]).flatten().collect();
        let png = png_from_rgba(3, 2, &rgba).expect("a PNG");
        assert_eq!(png_size(&png), Some((3, 2)));
        assert!(near(pixel(&png, 1, 0), [255, 0, 0]) && near(pixel(&png, 1, 1), [0, 0, 255]));
        assert_eq!(png_from_rgba(3, 2, &rgba[..20]), None, "not that many bytes");
        assert_eq!((png_from_rgba(0, 0, &[]), png_size(b"not a png")), (None, None));
    }

    #[test]
    fn annotations_are_drawn_on_a_copy_of_the_picture() {
        let black: Vec<u8> = [0, 0, 0, 255].repeat(200 * 100);
        let png = png_from_rgba(200, 100, &black).expect("a PNG");
        let amber = [0xff, 0xb9, 0x00];
        let marks = [
            Shape::Box { rect: [0.1, 0.2, 0.3, 0.4] },
            Shape::Ellipse { rect: [0.5, 0.1, 0.4, 0.4] },
            Shape::Stroke { points: vec![[0.0, 0.9], [1.0, 0.9]] },
            Shape::Note { at: [0.5, 0.6], text: "this one".into() },
        ];
        let drawn = annotate(&png, &marks, 0xffb900, 1.0).expect("the annotated picture");
        assert_eq!(png_size(&drawn), Some((200, 100)));
        // The box's left edge is at x 20, from y 20 to 60; inside it the picture is as it was.
        assert!(near(pixel(&drawn, 20, 40), amber), "{:?}", pixel(&drawn, 20, 40));
        assert!(near(pixel(&drawn, 50, 40), [0, 0, 0]));
        // The ellipse's top is at (140, 10), the stroke along y 90, the note's pill right of (100, 60).
        assert!(near(pixel(&drawn, 140, 10), amber), "{:?}", pixel(&drawn, 140, 10));
        assert!(near(pixel(&drawn, 100, 90), amber));
        assert!(near(pixel(&drawn, 103, 62), amber), "{:?}", pixel(&drawn, 103, 62));
        // The picture as taken is not changed.
        assert!(near(pixel(&png, 20, 40), [0, 0, 0]));
        assert_eq!(annotate(b"not a png", &marks, 0, 1.0), None);
    }

    #[test]
    fn a_terminals_rows_are_drawn_in_their_cells_and_colours() {
        let run = |text: &str, fg, bg| Run { text: text.into(), fg, bg, bold: false };
        let rows = vec![vec![run("  ", 0xffffff, None), run("##", 0x00ff00, Some(0x0000ff))], vec![], vec![run("@@@@", 0xffffff, None)]];
        let grid = Grid { rows: &rows, size: [80.0, 60.0], cell: [10.0, 20.0], font: "Menlo", font_size: 13.0, background: 0x101010, scale: 2.0 };
        let png = grid_png(&grid).expect("a picture");
        assert_eq!(png_size(&png), Some((160, 120)), "two pixels a point");
        // The first row's two blank cells are background; the next two have their own.
        assert!(near(pixel(&png, 10, 5), [0x10, 0x10, 0x10]));
        assert!(near(pixel(&png, 41, 1), [0, 0, 255]) && near(pixel(&png, 79, 39), [0, 0, 255]));
        assert!(near(pixel(&png, 81, 20), [0x10, 0x10, 0x10]), "past the run");
        // Text is drawn: the third row has pixels that are not background, the second has none.
        let (w, _, data) = pixels(&png);
        let lit = |row: usize| {
            let line = &data[(row * 40 + 20) * w * 4..][..w * 4];
            line.iter().step_by(4).filter(|red| **red > 0x80).count()
        };
        assert!(lit(2) > 0 && lit(1) == 0, "{} {}", lit(2), lit(1));
    }
}
