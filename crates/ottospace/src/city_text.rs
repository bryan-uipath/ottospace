//! Text the code city's renderer draws in the world (`city_gpu`): lines shaped from a system
//! font (rustybuzz, over ttf-parser: both in the lock already, under GPUI's SVG stack), their
//! glyphs a signed distance field in one atlas, sampled by a quad each. Crisp at any size and
//! angle; a glyph is rasterized once, when a line first needs it. Design:
//! `docs/ai/code-city/wall-names.md`.

use std::collections::HashMap;
use std::sync::OnceLock;

use rustybuzz::ttf_parser::{GlyphId, OutlineBuilder, Tag};

/// The atlas's side, in texels; a glyph's em in it; how far (texels) its field reaches each way.
pub(crate) const ATLAS: u32 = 1024;
const EM_PX: f32 = 48.;
const SPREAD: f32 = 6.;
/// Letters are spaced this many ems apart besides their advance: a sign's capitals, tracked.
const TRACK: f32 = 0.05;
/// The system font's weight (its `wght` axis, where it has one): semibold, to read small.
const WEIGHT: f32 = 600.;
/// Lines remembered, by text: past this many all are forgotten (their glyphs stay).
const LINES: usize = 4096;
/// Per glyph: its quad's corner, its across and up sides (world), its atlas rect, its colour.
pub(crate) const QUAD: usize = 17;

/// A line to draw: its em box's bottom left (world), the unit ways it reads and up, its em
/// (plan units) and colour (RGBA). The capitals sit in the middle of its em box.
pub(crate) struct Label {
    pub text: String,
    pub origin: [f32; 3],
    pub across: [f32; 3],
    pub up: [f32; 3],
    pub em: f32,
    pub colour: [f32; 4],
}

/// A glyph in the atlas: its rect there (shares: left, top, right, bottom) and its quad from
/// the pen on the baseline (ems: left, bottom, right, top).
#[derive(Clone, Copy)]
struct Cell {
    uv: [f32; 4],
    plane: [f32; 4],
}

/// A line shaped: each glyph and its pen's place (ems), and the line's width (ems).
struct Line {
    glyphs: Vec<(u16, f32)>,
    width: f32,
}

/// A system font, read once: SF Compact (ttf-parser reads no outline of SF Pro's `SFNS.ttf`),
/// else Helvetica Neue.
fn font_data() -> Option<&'static [u8]> {
    static DATA: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    let paths = ["/System/Library/Fonts/SFCompact.ttf", "/System/Library/Fonts/HelveticaNeue.ttc"];
    DATA.get_or_init(|| paths.iter().find_map(|p| std::fs::read(p).ok())).as_deref()
}

/// The font, its glyphs' field and the lines shaped.
pub(crate) struct Text {
    face: rustybuzz::Face<'static>,
    cap: f32,
    pub(crate) pixels: Vec<u8>,
    /// Glyphs met (none: no outline, as a space; or no room), the atlas's shelf (x, y, height).
    cells: HashMap<u16, Option<Cell>>,
    shelf: (u32, u32, u32),
    lines: HashMap<String, Option<Line>>,
    /// Glyphs were added since the atlas was last uploaded.
    pub(crate) dirty: bool,
}

impl Text {
    /// None: no system font to read.
    pub(crate) fn new() -> Option<Self> {
        let mut face = rustybuzz::Face::from_slice(font_data()?, 0)?;
        face.set_variation(Tag::from_bytes(b"wght"), WEIGHT);
        let cap = face.capital_height().map_or(0.7, |c| c as f32 / face.units_per_em() as f32);
        let pixels = vec![0; (ATLAS * ATLAS) as usize];
        Some(Self { face, cap, pixels, cells: HashMap::new(), shelf: (0, 0, 0), lines: HashMap::new(), dirty: false })
    }

    /// `text`'s width in ems, its glyphs in the atlas; none if the font lacks one or the atlas is full.
    pub(crate) fn width(&mut self, text: &str) -> Option<f32> {
        if !self.lines.contains_key(text) {
            if self.lines.len() >= LINES {
                self.lines.clear();
            }
            let line = self.shape(text);
            self.lines.insert(text.to_string(), line);
        }
        self.lines.get(text)?.as_ref().map(|line| line.width)
    }

    fn shape(&mut self, text: &str) -> Option<Line> {
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        let shaped = rustybuzz::shape(&self.face, &[], buffer);
        let per = 1. / self.face.units_per_em() as f32;
        let (mut pen, mut glyphs) = (0., Vec::new());
        for (info, at) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
            let id = u16::try_from(info.glyph_id).ok().filter(|id| *id != 0)?;
            // A glyph with no outline is a space, or not drawn: then neither is its line.
            let blank =
                text.get(info.cluster as usize..).and_then(|t| t.chars().next()).is_some_and(char::is_whitespace);
            if self.cell(id).is_none() && !blank {
                return None;
            }
            glyphs.push((id, pen + at.x_offset as f32 * per));
            pen += at.x_advance as f32 * per + TRACK;
        }
        Some(Line { glyphs, width: (pen - TRACK).max(0.) })
    }

    /// Glyph `id`'s cell, rasterized into the atlas the first time.
    fn cell(&mut self, id: u16) -> Option<Cell> {
        if let Some(cell) = self.cells.get(&id) {
            return *cell;
        }
        let cell = self.rasterize(id);
        self.cells.insert(id, cell);
        cell
    }

    fn rasterize(&mut self, id: u16) -> Option<Cell> {
        let scale = EM_PX / self.face.units_per_em() as f32;
        let mut outline = Outline { scale, segments: Vec::new(), start: [0.; 2], last: [0.; 2] };
        let bounds = self.face.outline_glyph(GlyphId(id), &mut outline)?;
        let x0 = (bounds.x_min as f32 * scale - SPREAD).floor();
        let y0 = (bounds.y_min as f32 * scale - SPREAD).floor();
        let (w, h) = (
            ((bounds.x_max as f32 * scale + SPREAD).ceil() - x0) as u32,
            ((bounds.y_max as f32 * scale + SPREAD).ceil() - y0) as u32,
        );
        // The next place on the shelf, or a new shelf; none past the atlas.
        let (mut x, mut y, mut tall) = self.shelf;
        if x + w + 1 > ATLAS {
            (x, y, tall) = (0, y + tall + 1, 0);
        }
        if w + 1 > ATLAS || y + h + 1 > ATLAS {
            return None;
        }
        self.shelf = (x + w + 1, y, tall.max(h));
        let field = field(&outline.segments, [x0, y0], w, h);
        for row in 0..h {
            let at = ((y + row) * ATLAS + x) as usize;
            self.pixels[at..at + w as usize].copy_from_slice(&field[(row * w) as usize..((row + 1) * w) as usize]);
        }
        self.dirty = true;
        let n = ATLAS as f32;
        let uv = [x as f32 / n, y as f32 / n, (x + w) as f32 / n, (y + h) as f32 / n];
        let plane = [x0 / EM_PX, y0 / EM_PX, (x0 + w as f32) / EM_PX, (y0 + h as f32) / EM_PX];
        Some(Cell { uv, plane })
    }

    /// `label`'s glyphs as quads (`QUAD` floats each), its capitals centred in its em box; none if
    /// its text was not measured (`width`) or cannot be drawn.
    pub(crate) fn quads(&self, label: &Label, out: &mut Vec<f32>) {
        let Some(Some(line)) = self.lines.get(&label.text) else { return };
        let baseline = (1. - self.cap) / 2.;
        let at = |o: [f32; 3], d: [f32; 3], by: f32| -> [f32; 3] { std::array::from_fn(|i| o[i] + d[i] * by) };
        for (id, x) in &line.glyphs {
            let Some(Some(Cell { uv, plane: [l, b, r, t] })) = self.cells.get(id) else { continue };
            let corner = at(at(label.origin, label.across, (x + l) * label.em), label.up, (baseline + b) * label.em);
            let across = label.across.map(|v| v * (r - l) * label.em);
            let up = label.up.map(|v| v * (t - b) * label.em);
            out.extend(corner.into_iter().chain(across).chain(up).chain(*uv).chain(label.colour));
        }
    }
}

/// A glyph's outline as line segments, in atlas texels (y up).
struct Outline {
    scale: f32,
    segments: Vec<[[f32; 2]; 2]>,
    start: [f32; 2],
    last: [f32; 2],
}

impl Outline {
    fn to(&mut self, p: [f32; 2]) {
        self.segments.push([self.last, p]);
        self.last = p;
    }
}

/// Curves cut into this many lines: at 48 texels an em, a fraction of a texel off.
const CUTS: usize = 8;

impl OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        (self.start, self.last) = ([x * self.scale, y * self.scale], [x * self.scale, y * self.scale]);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.to([x * self.scale, y * self.scale]);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (a, b, c) = (self.last, [x1 * self.scale, y1 * self.scale], [x * self.scale, y * self.scale]);
        for k in 1..=CUTS {
            let t = k as f32 / CUTS as f32;
            let q = |i: usize| (1. - t).powi(2) * a[i] + 2. * (1. - t) * t * b[i] + t * t * c[i];
            self.to([q(0), q(1)]);
        }
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let s = self.scale;
        let (a, b, c, d) = (self.last, [x1 * s, y1 * s], [x2 * s, y2 * s], [x * s, y * s]);
        for k in 1..=CUTS {
            let t = k as f32 / CUTS as f32;
            let u = 1. - t;
            let q = |i: usize| u * u * u * a[i] + 3. * u * u * t * b[i] + 3. * u * t * t * c[i] + t * t * t * d[i];
            self.to([q(0), q(1)]);
        }
    }

    fn close(&mut self) {
        if self.last != self.start {
            self.to(self.start);
        }
    }
}

/// The signed distance field of `segments` over `w` × `h` texels from `from` (bottom left; rows
/// top first): 128 on the outline, brighter inside, `SPREAD` texels each way to 0 and 255.
fn field(segments: &[[[f32; 2]; 2]], from: [f32; 2], w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![0u8; (w * h) as usize];
    for row in 0..h {
        let y = from[1] + (h - row) as f32 - 0.5;
        // Inside by the non-zero rule: where each edge crosses this row, and which way.
        let crossings: Vec<(f32, i32)> = (segments.iter())
            .filter(|[a, b]| (a[1] <= y) != (b[1] <= y))
            .map(|[a, b]| (a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]), if b[1] > a[1] { 1 } else { -1 }))
            .collect();
        let near: Vec<&[[f32; 2]; 2]> =
            segments.iter().filter(|[a, b]| a[1].min(b[1]) - SPREAD <= y && y <= a[1].max(b[1]) + SPREAD).collect();
        for col in 0..w {
            let x = from[0] + col as f32 + 0.5;
            let winding: i32 = crossings.iter().filter(|(cx, _)| *cx > x).map(|(_, d)| d).sum();
            let d = near.iter().map(|s| apart([x, y], s)).fold(SPREAD, f32::min);
            let signed = if winding != 0 { d } else { -d };
            out[(row * w + col) as usize] = ((0.5 + 0.5 * signed / SPREAD) * 255.).round().clamp(0., 255.) as u8;
        }
    }
    out
}

/// How far `p` is from the segment `[a, b]`.
fn apart(p: [f32; 2], [a, b]: &[[f32; 2]; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx * dx + dy * dy;
    let t = if length > 0. { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length).clamp(0., 1.) } else { 0. };
    ((p[0] - a[0] - t * dx).powi(2) + (p[1] - a[1] - t * dy).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square's field: 128 on its edge, bright inside, dark out, reaching `SPREAD`.
    #[test]
    fn a_squares_field_is_signed_by_its_edge() {
        let square = [[0., 0.], [10., 0.], [10., 10.], [0., 10.]];
        let segments: Vec<_> = (0..4).map(|i| [square[i], square[(i + 1) % 4]]).collect();
        let f = field(&segments, [-6., -6.], 22, 22);
        let at = |x: u32, y_up: u32| f[((21 - y_up) * 22 + x) as usize];
        assert!(at(11, 11) > 200 && at(0, 0) == 0, "deep inside, far out");
        // A texel's middle half a texel inside the left edge, and half outside it: as far either way.
        let (inside, outside) = (at(6, 11), at(5, 11));
        assert!(inside > 128 && outside < 128 && (i32::from(inside) + i32::from(outside) - 255).abs() <= 1);
    }

    /// The system font shapes a folder's name, its glyphs into the atlas once; a glyph the font
    /// lacks draws no line. No system font (not a Mac): nothing to check.
    #[test]
    fn a_name_is_shaped_and_its_glyphs_kept_once() {
        let Some(mut text) = Text::new() else { return eprintln!("no system font: skipped") };
        let width = text.width("SRC").unwrap();
        assert!((1.5..3.).contains(&width), "{width}");
        assert!(text.dirty && text.width("SRC") == Some(width));
        let glyphs = text.cells.len();
        assert!(text.width("CRS").is_some() && text.cells.len() == glyphs, "no new glyph");
        assert!(text.width("A B").unwrap() > text.width("AB").unwrap(), "a space advances");
        assert_eq!(text.width("\u{10FFFD}"), None);
        let label = Label {
            text: "SRC".into(),
            origin: [0.; 3],
            across: [1., 0., 0.],
            up: [0., 1., 0.],
            em: 2.,
            colour: [1.; 4],
        };
        let mut quads = Vec::new();
        text.quads(&label, &mut quads);
        assert_eq!(quads.len(), 3 * QUAD);
        // Each glyph's quad left of the next, all inside the em box but for the field's reach.
        let lefts: Vec<f32> = quads.chunks(QUAD).map(|q| q[0]).collect();
        assert!(lefts.windows(2).all(|w| w[0] < w[1]) && lefts[2] < width * 2., "{lefts:?}");
    }
}
