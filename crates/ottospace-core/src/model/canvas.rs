//! Zoom-out's canvas (`zoom_out`): the tiles in a world where one tab is 1 × 1 (its panes area's
//! shape, whatever the window), and the camera over it. Pure numbers: the shell maps a world rect
//! to points by the panes area's width and height. Design: `docs/ai/zoom-out.md`, slice 2.

use serde::{Deserialize, Serialize};

use crate::layout::Rect;

/// Room between grid cells, in tabs.
pub const GRID_GAP: f32 = 0.12;
/// The nearest the camera comes: one tab fills the area. Nearer is the tab itself (↵).
pub const NEAREST: f32 = 1.0;
/// The farthest: a 4 × 4 workspace is a third of the area wide. Farther shows nothing more.
pub const FARTHEST: f32 = 0.08;
/// A fit is never nearer than this: one tab alone is a tile, not the tab.
pub const FIT_NEAREST: f32 = 0.5;
/// ⌘= and ⌘-: one step nearer or farther.
pub const KEY_STEP: f32 = 1.25;
/// A placed tile's corner snaps to this grid, in tabs: light enough to place by eye, firm enough
/// that tiles line up without effort (about 28 points at a fit of four tabs).
pub const SNAP: f32 = 0.25;
/// How far from the grid's corner a tile may be placed, in tabs.
const REACH: f32 = 1000.;

/// The share of the view, on each axis, that some tile always reaches into.
const KEEP: f32 = 0.1;

/// Where the camera is: `scale` (1: one tab fills the area) and `center`, the world point at the
/// area's middle.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    pub scale: f32,
    pub center: [f32; 2],
}

/// A step of the camera, the user's (the shell translates a gesture or a key).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CanvasStep {
    /// Nearer by `factor` (over 1: in), keeping the world point under `about` (the panes area's
    /// fractions, 0 to 1) where it is.
    Zoom { factor: f32, about: [f32; 2] },
    /// The view moved by `dx`, `dy` of the area (the canvas the other way).
    Pan { dx: f32, dy: f32 },
    /// Every tile in view, centred (⌘0).
    Fit,
}

/// Tile `at` of `n` on the automatic grid, in tab order (⌘1…⌘9's): the least square that holds
/// them, `GRID_GAP` apart. e.g. 7: three columns of 3, 3, 1.
pub fn grid_spot(at: usize, n: usize) -> Rect {
    let (cols, _) = super::zoom::grid(n);
    let cols = cols.max(1);
    let step = 1. + GRID_GAP;
    Rect { x: (at % cols) as f32 * step, y: (at / cols) as f32 * step, w: 1., h: 1. }
}

/// A placed tile's corner: snapped, or none when not a number or out of reach.
pub fn snapped(at: [f32; 2]) -> Option<[f32; 2]> {
    let snap = |v: f32| (v / SNAP).round() * SNAP;
    at.iter().all(|v| v.is_finite() && v.abs() <= REACH).then(|| [snap(at[0]), snap(at[1])])
}

/// Each of `n` tiles' place: `placed[at]` where the user put it, else the next grid cell that no
/// placed tile covers (a new tab takes the next free cell; nothing placed moves).
pub fn spots(placed: &[Option<[f32; 2]>]) -> Vec<Rect> {
    let n = placed.len();
    let fixed: Vec<Rect> = placed.iter().flatten().map(|p| Rect { x: p[0], y: p[1], w: 1., h: 1. }).collect();
    let overlaps = |a: &Rect, b: &Rect| a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
    let mut cell = 0;
    placed
        .iter()
        .map(|p| match p {
            Some(p) => Rect { x: p[0], y: p[1], w: 1., h: 1. },
            None => loop {
                let spot = grid_spot(cell, n);
                cell += 1;
                if !fixed.iter().any(|f| overlaps(f, &spot)) {
                    break spot;
                }
            },
        })
        .collect()
}

/// The box round `tiles`; a unit box at the origin when there are none.
pub fn extent(tiles: &[Rect]) -> Rect {
    let Some(first) = tiles.first() else { return Rect { x: 0., y: 0., w: 1., h: 1. } };
    let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x + first.w, first.y + first.h);
    for t in tiles {
        (x0, y0, x1, y1) = (x0.min(t.x), y0.min(t.y), x1.max(t.x + t.w), y1.max(t.y + t.h));
    }
    Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
}

/// The farthest the camera goes over `extent`: `FARTHEST`, or farther for a workspace too big to
/// fit there (over about a hundred tabs), so a fit always shows every tile.
pub fn farthest(extent: Rect) -> f32 {
    FARTHEST.min(fitting(extent))
}

/// The scale at which `extent` fits with a grid gap round it.
fn fitting(extent: Rect) -> f32 {
    (1. / (extent.w + 2. * GRID_GAP)).min(1. / (extent.h + 2. * GRID_GAP))
}

impl Camera {
    /// Every tile of `extent` in view with a grid gap round it, centred; never nearer than
    /// `FIT_NEAREST`.
    pub fn fit(extent: Rect) -> Camera {
        let center = [extent.x + extent.w / 2., extent.y + extent.h / 2.];
        Camera { scale: fitting(extent).clamp(farthest(extent), FIT_NEAREST), center }
    }

    /// Inside its bounds: a scale from `FARTHEST` to `NEAREST`, and a centre that leaves a tile of
    /// `tiles` reaching `KEEP` of the view into it on each axis (the nearest, if none did). Not a
    /// number: the fit.
    pub fn clamped(self, tiles: &[Rect]) -> Camera {
        let extent = extent(tiles);
        if !(self.scale.is_finite() && self.center.iter().all(|c| c.is_finite())) {
            return Camera::fit(extent);
        }
        let scale = self.scale.clamp(farthest(extent), NEAREST);
        // Half the view, in world units; the view spans center ± half.
        let half = 0.5 / scale;
        let keep = 2. * half * KEEP;
        let axis = |c: f32, lo: f32, len: f32| c.clamp(lo + keep - half, lo + len - keep + half);
        let onto = |r: &Rect| [axis(self.center[0], r.x, r.w), axis(self.center[1], r.y, r.h)];
        if tiles.is_empty() || tiles.iter().any(|t| onto(t) == self.center) {
            return Camera { scale, center: onto(&extent) };
        }
        // Over no tile (an empty cell of a short last row): onto the nearest.
        let gap = |t: &&Rect| {
            let (dx, dy) = (t.x + t.w / 2. - self.center[0], t.y + t.h / 2. - self.center[1]);
            dx * dx + dy * dy
        };
        let near = tiles.iter().min_by(|a, b| gap(a).total_cmp(&gap(b))).copied().unwrap_or(extent);
        Camera { scale, center: onto(&near) }
    }

    /// `step` taken, then kept inside the bounds of `tiles`.
    pub fn stepped(self, step: CanvasStep, tiles: &[Rect]) -> Camera {
        let moved = match step {
            CanvasStep::Fit => return Camera::fit(extent(tiles)),
            CanvasStep::Zoom { factor, about } => {
                // The world point under `about` stays under it.
                let scale = (self.scale * factor).clamp(farthest(extent(tiles)), NEAREST);
                let at = |i: usize| self.center[i] + (about[i] - 0.5) / self.scale - (about[i] - 0.5) / scale;
                Camera { scale, center: [at(0), at(1)] }
            }
            CanvasStep::Pan { dx, dy } => {
                Camera { center: [self.center[0] + dx / self.scale, self.center[1] + dy / self.scale], ..self }
            }
        };
        moved.clamped(tiles)
    }

    /// World rect `r` on screen, in the panes area's fractions (0 to 1 is the area).
    pub fn on_screen(&self, r: Rect) -> Rect {
        let s = self.scale;
        Rect { x: (r.x - self.center[0]) * s + 0.5, y: (r.y - self.center[1]) * s + 0.5, w: r.w * s, h: r.h * s }
    }

    /// The camera with `r` wholly in view, moved as little as it can (none if it is already).
    pub fn showing(self, r: Rect, tiles: &[Rect]) -> Camera {
        let half = 0.5 / self.scale;
        let axis = |c: f32, lo: f32, len: f32| {
            if len >= 2. * half { lo + len / 2. } else { c.clamp(lo + len - half, lo + half) }
        };
        Camera { center: [axis(self.center[0], r.x, r.w), axis(self.center[1], r.y, r.h)], ..self }.clamped(tiles)
    }
}

/// Whether some of `r` (area fractions) is on screen.
pub fn in_view(r: Rect) -> bool {
    r.x < 1. && r.y < 1. && r.x + r.w > 0. && r.y + r.h > 0.
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spots(n: usize) -> Vec<Rect> {
        (0..n).map(|at| grid_spot(at, n)).collect()
    }

    fn grid(n: usize) -> Rect {
        extent(&spots(n))
    }

    /// Some tile reaches a tenth of the view in, on both axes.
    fn sees(cam: &Camera, tiles: &[Rect]) -> bool {
        let into = |r: Rect| r.x < 0.9 + 1e-4 && r.x + r.w > 0.1 - 1e-4 && r.y < 0.9 + 1e-4 && r.y + r.h > 0.1 - 1e-4;
        tiles.iter().any(|t| into(cam.on_screen(*t)))
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn the_grid_is_tab_order_a_gap_apart() {
        assert_eq!(grid_spot(0, 1), Rect { x: 0., y: 0., w: 1., h: 1. });
        let s = 1. + GRID_GAP;
        assert_eq!((grid_spot(2, 7).x, grid_spot(3, 7).y, grid_spot(6, 7).y), (2. * s, s, 2. * s));
        let box7 = grid(7);
        assert!(near(box7.w, 3. + 2. * GRID_GAP) && near(box7.h, 3. + 2. * GRID_GAP), "{box7:?}");
        assert_eq!(extent(&[]), Rect { x: 0., y: 0., w: 1., h: 1. });
    }

    #[test]
    fn a_fit_shows_every_tile_never_nearer_than_half() {
        for n in [1, 2, 4, 7, 16, 40, 150] {
            let e = grid(n);
            let cam = Camera::fit(e);
            assert!(cam.scale <= FIT_NEAREST && cam.scale >= farthest(e), "{n}: {cam:?}");
            let r = cam.on_screen(e);
            if cam.scale > 0. {
                assert!(r.x >= -1e-4 && r.y >= -1e-4 && r.x + r.w <= 1. + 1e-4 && r.y + r.h <= 1. + 1e-4, "{n}: {r:?}");
            }
            assert!(near(r.x + r.w / 2., 0.5) && near(r.y + r.h / 2., 0.5), "{n}: centred {r:?}");
        }
        // Sixteen tabs at the farthest: about a third of the area.
        let far = Camera { scale: FARTHEST, ..Camera::fit(grid(16)) }.on_screen(grid(16));
        assert!(far.w > 0.3 && far.w < 0.4, "{far:?}");
    }

    #[test]
    fn a_zoom_keeps_the_point_under_the_pointer_and_stops_at_its_ends() {
        let (t, e) = (spots(4), grid(4));
        let cam = Camera::fit(e);
        let about = [0.3, 0.7];
        let world = |c: &Camera| [c.center[0] + (about[0] - 0.5) / c.scale, c.center[1] + (about[1] - 0.5) / c.scale];
        let nearer = cam.stepped(CanvasStep::Zoom { factor: 1.5, about }, &t);
        assert!(near(nearer.scale, cam.scale * 1.5));
        let (a, b) = (world(&cam), world(&nearer));
        assert!(near(a[0], b[0]) && near(a[1], b[1]), "{a:?} {b:?}");
        let mut far = cam;
        for _ in 0..50 {
            far = far.stepped(CanvasStep::Zoom { factor: 0.5, about }, &t);
        }
        let mut close = cam;
        for _ in 0..50 {
            close = close.stepped(CanvasStep::Zoom { factor: 2., about }, &t);
        }
        assert_eq!((far.scale, close.scale), (FARTHEST, NEAREST));
    }

    #[test]
    fn a_pan_never_loses_every_tile_and_fit_brings_them_back() {
        let (t, e) = (spots(7), grid(7));
        for scale in [FARTHEST, 0.3, NEAREST] {
            for (dx, dy) in [(100., 0.), (-100., 0.), (0., 100.), (0., -100.), (1e9, -1e9), (0.6, 0.8)] {
                let cam = Camera { scale, ..Camera::fit(e) }.stepped(CanvasStep::Pan { dx, dy }, &t);
                assert!(sees(&cam, &t), "{scale} {dx} {dy}: {cam:?}");
            }
        }
        // Over the empty cells of the short last row, near: onto the nearest tile.
        let empty = Camera { scale: NEAREST, center: [2.7, 2.7] }.clamped(&t);
        assert!(sees(&empty, &t), "{empty:?}");
        let lost = Camera { scale: f32::NAN, center: [f32::INFINITY, 0.] }.clamped(&t);
        assert_eq!(lost, Camera::fit(e));
        let back = Camera { scale: 0.9, center: [-50., 50.] }.stepped(CanvasStep::Fit, &t);
        assert_eq!(back, Camera::fit(e));
    }

    #[test]
    fn the_selected_tile_is_brought_into_view() {
        let (t, e) = (spots(16), grid(16));
        let cam = Camera { scale: NEAREST, center: [0.5, 0.5] };
        let last = grid_spot(15, 16);
        let r = cam.showing(last, &t).on_screen(last);
        assert!(r.x >= -1e-4 && r.y >= -1e-4 && r.x + r.w <= 1. + 1e-4 && r.y + r.h <= 1. + 1e-4, "{r:?}");
        // Already in view: it does not move.
        let fit = Camera::fit(e);
        assert_eq!(fit.showing(last, &t), fit);
    }

    #[test]
    fn a_placed_tile_snaps_and_the_others_take_the_free_cells() {
        assert_eq!(snapped([0.3, -1.12]), Some([0.25, -1.]));
        assert_eq!((snapped([f32::NAN, 0.]), snapped([1e9, 0.])), (None, None));
        // Four tabs, the second placed over the third's cell: the third and fourth go on.
        let placed = [None, Some([0., 1.12]), None, None];
        let s = super::spots(&placed);
        assert_eq!(s[1], Rect { x: 0., y: 1.12, w: 1., h: 1. });
        assert_eq!((s[0], s[2]), (grid_spot(0, 4), grid_spot(1, 4)));
        assert_eq!(s[3], grid_spot(3, 4), "the cell under the placed tile is skipped");
        let overlap = |a: Rect, b: Rect| a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
        assert!([0, 2, 3].iter().all(|at| !overlap(s[*at], s[1])), "{s:?}");
        // None placed: the grid.
        assert_eq!(super::spots(&[None, None]), vec![grid_spot(0, 2), grid_spot(1, 2)]);
    }
}
