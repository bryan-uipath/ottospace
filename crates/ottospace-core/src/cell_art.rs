//! Box-drawing and block characters as shapes that fill their terminal cell, so lines and blocks
//! meet across cells and rows. A font's glyph stops short of a row taller than the font: `│` under
//! `│` leaves a gap, a block-art logo shows stripes.

/// One shape, in points from the cell's top-left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Piece {
    /// A filled rectangle at `alpha` of the text's colour (shades: ░ is 0.25).
    Rect { x: f32, y: f32, w: f32, h: f32, alpha: f32 },
    /// A quarter circle of `width` from `from` to `to`, bending toward `corner` (a rounded box corner).
    Arc { from: [f32; 2], to: [f32; 2], corner: [f32; 2], width: f32 },
    /// A straight stroke of `width` (a diagonal).
    Line { from: [f32; 2], to: [f32; 2], width: f32 },
}

/// The shapes of `c` in a cell `w` by `h` points, or `None` for a character the font draws
/// (double lines among them).
pub fn pieces(c: char, w: f32, h: f32) -> Option<Vec<Piece>> {
    let code = c as u32;
    let light = (w / 8.).round().max(1.);
    match code {
        0x2500..=0x254f | 0x2574..=0x257f => Some(lines(arms(code)?, dashes(code), w, h, light)),
        0x256d..=0x2570 => Some(vec![arc(code, w, h, light)]),
        0x2571..=0x2573 => {
            let rising = Piece::Line { from: [0., h], to: [w, 0.], width: light };
            let falling = Piece::Line { from: [0., 0.], to: [w, h], width: light };
            Some(match code {
                0x2571 => vec![rising],
                0x2572 => vec![falling],
                _ => vec![rising, falling],
            })
        }
        0x2580..=0x259f => Some(blocks(code, w, h)),
        _ => None,
    }
}

/// A line's weight on one arm: none, light or heavy.
type Arms = [u8; 4];

/// Up, right, down, left of U+2500..=U+254F and U+2574..=U+257F: 0 none, 1 light, 2 heavy.
/// e.g. `┌` (U+250C) is `0110`: right and down, light.
fn arms(code: u32) -> Option<Arms> {
    const BOX: [&str; 80] = [
        "0101", "0202", "1010", "2020", "0101", "0202", "1010", "2020", // ─ ━ │ ┃ ┄ ┅ ┆ ┇
        "0101", "0202", "1010", "2020", "0110", "0210", "0120", "0220", // ┈ ┉ ┊ ┋ ┌ ┍ ┎ ┏
        "0011", "0012", "0021", "0022", "1100", "1200", "2100", "2200", // ┐ ┑ ┒ ┓ └ ┕ ┖ ┗
        "1001", "1002", "2001", "2002", "1110", "1210", "2110", "1120", // ┘ ┙ ┚ ┛ ├ ┝ ┞ ┟
        "2120", "2210", "1220", "2220", "1011", "1012", "2011", "1021", // ┠ ┡ ┢ ┣ ┤ ┥ ┦ ┧
        "2021", "2012", "1022", "2022", "0111", "0112", "0211", "0212", // ┨ ┩ ┪ ┫ ┬ ┭ ┮ ┯
        "0121", "0122", "0221", "0222", "1101", "1102", "1201", "1202", // ┰ ┱ ┲ ┳ ┴ ┵ ┶ ┷
        "2101", "2102", "2201", "2202", "1111", "1112", "1211", "1212", // ┸ ┹ ┺ ┻ ┼ ┽ ┾ ┿
        "2111", "1121", "2121", "2112", "2211", "1122", "1221", "2212", // ╀ ╁ ╂ ╃ ╄ ╅ ╆ ╇
        "1222", "2122", "2221", "2222", "0101", "0202", "1010", "2020", // ╈ ╉ ╊ ╋ ╌ ╍ ╎ ╏
    ];
    const HALF: [&str; 12] = [
        "0001", "1000", "0100", "0010", "0002", "2000", "0200", "0020", // ╴ ╵ ╶ ╷ ╸ ╹ ╺ ╻
        "0201", "1020", "0102", "2010", // ╼ ╽ ╾ ╿
    ];
    let digits = match code {
        0x2500..=0x254f => BOX[(code - 0x2500) as usize],
        _ => HALF.get(code.checked_sub(0x2574)? as usize)?,
    };
    let mut arms = [0; 4];
    for (arm, digit) in arms.iter_mut().zip(digits.bytes()) {
        *arm = digit - b'0';
    }
    Some(arms)
}

/// Dashes in a dashed line: 3 for `┄`, 4 for `┈`, 2 for `╌`; 0 for a solid one.
fn dashes(code: u32) -> u8 {
    match code {
        0x2504..=0x2507 => 3,
        0x2508..=0x250b => 4,
        0x254c..=0x254f => 2,
        _ => 0,
    }
}

/// Each arm from the cell's centre to its edge, heavy ones twice as thick. Arms overlap at the
/// centre by half the thickest crossing arm, so a corner has no notch.
fn lines(arms: Arms, dashes: u8, w: f32, h: f32, light: f32) -> Vec<Piece> {
    let (cx, cy) = ((w / 2.).floor(), (h / 2.).floor());
    let thick = |weight: u8| light * weight as f32;
    let [up, right, down, left] = arms;
    // How far a horizontal arm reaches past the centre: half the vertical line it meets.
    let (over_h, over_v) = (thick(up.max(down)) / 2., thick(left.max(right)) / 2.);
    let rect = |x: f32, y: f32, w: f32, h: f32| Piece::Rect { x, y, w, h, alpha: 1. };
    let mut pieces = Vec::new();
    if dashes > 0 {
        // Dashed: `dashes` segments across the cell, each with a gap after it.
        let (horizontal, weight) = if left > 0 { (true, left) } else { (false, up) };
        let (length, t) = (if horizontal { w } else { h }, thick(weight));
        let step = length / dashes as f32;
        for i in 0..dashes {
            let at = step * i as f32;
            pieces.push(if horizontal {
                rect(at, cy - t / 2., step * 0.6, t)
            } else {
                rect(cx - t / 2., at, t, step * 0.6)
            });
        }
        return pieces;
    }
    if up > 0 {
        let t = thick(up);
        pieces.push(rect(cx - t / 2., 0., t, cy + over_v));
    }
    if down > 0 {
        let t = thick(down);
        pieces.push(rect(cx - t / 2., cy - over_v, t, h - cy + over_v));
    }
    if left > 0 {
        let t = thick(left);
        pieces.push(rect(0., cy - t / 2., cx + over_h, t));
    }
    if right > 0 {
        let t = thick(right);
        pieces.push(rect(cx - over_h, cy - t / 2., w - cx + over_h, t));
    }
    pieces
}

/// `╭ ╮ ╯ ╰`: a quarter circle from the middle of one edge to the middle of another, as the
/// straight lines beside it would meet.
fn arc(code: u32, w: f32, h: f32, light: f32) -> Piece {
    let (cx, cy) = ((w / 2.).floor(), (h / 2.).floor());
    let (vertical, horizontal) = match code {
        0x256d => (h, w),   // ╭: down and right
        0x256e => (h, 0.),  // ╮: down and left
        0x256f => (0., 0.), // ╯: up and left
        _ => (0., w),       // ╰: up and right
    };
    Piece::Arc { from: [cx, vertical], to: [horizontal, cy], corner: [cx, cy], width: light }
}

/// U+2580..=U+259F: eighths, halves, quadrants and shades, as rectangles.
fn blocks(code: u32, w: f32, h: f32) -> Vec<Piece> {
    // [x0, y0, x1, y1] in eighths of the cell.
    let part = |[x0, y0, x1, y1]: [u8; 4]| Piece::Rect {
        x: w * x0 as f32 / 8.,
        y: h * y0 as f32 / 8.,
        w: w * (x1 - x0) as f32 / 8.,
        h: h * (y1 - y0) as f32 / 8.,
        alpha: 1.,
    };
    let shade = |alpha: f32| vec![Piece::Rect { x: 0., y: 0., w, h, alpha }];
    let (ul, ur, ll, lr) = ([0, 0, 4, 4], [4, 0, 8, 4], [0, 4, 4, 8], [4, 4, 8, 8]);
    let parts: Vec<[u8; 4]> = match code {
        0x2580 => vec![[0, 0, 8, 4]],
        0x2581..=0x2588 => vec![[0, (0x2588 - code) as u8, 8, 8]],
        0x2589..=0x258f => vec![[0, 0, (0x2590 - code) as u8, 8]],
        0x2590 => vec![[4, 0, 8, 8]],
        0x2591 => return shade(0.25),
        0x2592 => return shade(0.5),
        0x2593 => return shade(0.75),
        0x2594 => vec![[0, 0, 8, 1]],
        0x2595 => vec![[7, 0, 8, 8]],
        0x2596 => vec![ll],
        0x2597 => vec![lr],
        0x2598 => vec![ul],
        0x2599 => vec![ul, ll, lr],
        0x259a => vec![ul, lr],
        0x259b => vec![ul, ur, ll],
        0x259c => vec![ul, ur, lr],
        0x259d => vec![ur],
        0x259e => vec![ur, ll],
        _ => vec![ur, ll, lr],
    };
    parts.into_iter().map(part).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 8.;
    const H: f32 = 18.;

    /// The rectangles of `c`, as (x, y, w, h).
    fn rects(c: char) -> Vec<(f32, f32, f32, f32)> {
        let pieces = pieces(c, W, H).unwrap();
        pieces
            .iter()
            .map(|p| if let Piece::Rect { x, y, w, h, .. } = *p { (x, y, w, h) } else { panic!("{p:?}") })
            .collect()
    }

    /// A vertical line runs the cell's full height and a horizontal one its full width: rows and
    /// cells meet with no gap.
    #[test]
    fn lines_reach_every_edge_they_name() {
        assert_eq!(rects('│'), [(3.5, 0., 1., 9.), (3.5, 9., 1., 9.)]);
        assert_eq!(rects('─'), [(0., 8.5, 4., 1.), (4., 8.5, 4., 1.)]);
        assert_eq!(rects('┃'), [(3., 0., 2., 9.), (3., 9., 2., 9.)]);
        // ┌: right and down from the centre, each reaching half the other's width past it.
        assert_eq!(rects('┌'), [(3.5, 8.5, 1., 9.5), (3.5, 8.5, 4.5, 1.)]);
        // ╴ and ╿: half lines, and a heavy half above a light one.
        assert_eq!(rects('╴'), [(0., 8.5, 4., 1.)]);
        assert_eq!(rects('╿'), [(3., 0., 2., 9.), (3.5, 9., 1., 9.)]);
    }

    #[test]
    fn dashed_lines_are_segments_with_gaps() {
        let dashes = rects('┄');
        assert_eq!(dashes.len(), 3);
        assert!(dashes.iter().all(|(_, y, w, h)| (*y, *h) == (8.5, 1.) && *w < W / 3.), "{dashes:?}");
        assert_eq!(rects('╎').len(), 2);
    }

    #[test]
    fn blocks_fill_their_part_of_the_cell() {
        assert_eq!(rects('█'), [(0., 0., W, H)]);
        assert_eq!(rects('▀'), [(0., 0., W, 9.)]);
        assert_eq!(rects('▄'), [(0., 9., W, 9.)]);
        assert_eq!(rects('▁'), [(0., 15.75, W, 2.25)]);
        assert_eq!(rects('▌'), [(0., 0., 4., H)]);
        assert_eq!(rects('▏'), [(0., 0., 1., H)]);
        assert_eq!(rects('▐'), [(4., 0., 4., H)]);
        assert_eq!(rects('▛'), [(0., 0., 4., 9.), (4., 0., 4., 9.), (0., 9., 4., 9.)]);
        assert_eq!(rects('▟'), [(4., 0., 4., 9.), (0., 9., 4., 9.), (4., 9., 4., 9.)]);
        assert_eq!(pieces('▒', W, H), Some(vec![Piece::Rect { x: 0., y: 0., w: W, h: H, alpha: 0.5 }]));
    }

    #[test]
    fn rounded_corners_join_the_middles_of_two_edges() {
        let corner = |c| match pieces(c, W, H).unwrap()[..] {
            [Piece::Arc { from, to, .. }] => (from, to),
            ref other => panic!("{other:?}"),
        };
        assert_eq!(corner('╭'), ([4., 18.], [8., 9.]));
        assert_eq!(corner('╯'), ([4., 0.], [0., 9.]));
    }

    /// Characters the font draws: letters, double lines, Braille, symbols.
    #[test]
    fn other_characters_are_left_to_the_font() {
        for c in ['a', '═', '║', '╔', '⠿', '⎿', '⏺', '日'] {
            assert_eq!(pieces(c, W, H), None, "{c}");
        }
    }

    /// Every character in the two ranges it draws has shapes, none empty.
    #[test]
    fn every_drawn_character_has_a_shape() {
        let drawn = (0x2500..=0x254f).chain(0x256d..=0x259f).filter_map(char::from_u32);
        for c in drawn {
            assert!(pieces(c, W, H).is_some_and(|p| !p.is_empty()), "{c} U+{:04X}", c as u32);
        }
    }
}
