//! Terminal pane rules: scrolling back, mouse selection, paste. The shell applies them to its
//! `alacritty_terminal` grid.

use crate::keys::Mods;

/// A move of the view through the scrollback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scroll {
    /// Positive: back into history.
    Lines(i32),
    PageUp,
    PageDown,
    Top,
    Bottom,
}

/// ⇧PageUp / ⇧PageDown / ⇧Home / ⇧End scroll the history. On the alternate screen (vim, less)
/// there is no history, so the key goes to the program.
pub fn scroll_key(key: &str, mods: Mods, alt_screen: bool) -> Option<Scroll> {
    if alt_screen || !mods.shift || mods.ctrl || mods.alt || mods.cmd {
        return None;
    }
    match key {
        "pageup" => Some(Scroll::PageUp),
        "pagedown" => Some(Scroll::PageDown),
        "home" => Some(Scroll::Top),
        "end" => Some(Scroll::Bottom),
        _ => None,
    }
}

/// Whether the wheel scrolls our history. Not while the program reads the mouse or uses the
/// alternate screen: the wheel is the program's there.
pub fn wheel_scrolls(mouse_mode: bool, alt_screen: bool) -> bool {
    !mouse_mode && !alt_screen
}

/// Turns trackpad pixels into whole lines, keeping the remainder for the next event.
#[derive(Clone, Copy, Debug, Default)]
pub struct Wheel {
    rest: f32,
}

impl Wheel {
    /// `dy` is the event's vertical delta in pixels; positive scrolls back into history.
    pub fn lines(&mut self, dy: f32, line_height: f32) -> i32 {
        if line_height <= 0. {
            return 0;
        }
        let total = self.rest + dy;
        let lines = (total / line_height).trunc();
        self.rest = total - lines * line_height;
        lines as i32
    }
}

/// A point on the visible grid: row from the top, column, and whether it is the cell's right half.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellPoint {
    pub row: usize,
    pub col: usize,
    pub right: bool,
}

/// The cell under `(x, y)`, measured from the grid's top-left in points. Outside the grid it
/// clamps to the nearest cell, so a drag past an edge selects to that edge.
pub fn cell_at(x: f32, y: f32, cell: (f32, f32), grid: (usize, usize)) -> CellPoint {
    let ((w, h), (cols, rows)) = (cell, grid);
    if w <= 0. || h <= 0. || cols == 0 || rows == 0 {
        return CellPoint { row: 0, col: 0, right: false };
    }
    let col = (x / w).floor();
    let row = (y / h).floor().clamp(0., (rows - 1) as f32) as usize;
    // Past the right edge is the last cell's right half; before the left edge, the first's left.
    if col >= cols as f32 {
        return CellPoint { row, col: cols - 1, right: true };
    }
    if col < 0. {
        return CellPoint { row, col: 0, right: false };
    }
    CellPoint { row, col: col as usize, right: x - col * w >= w / 2. }
}

/// What a click selects, by click count: 1 starts a plain drag selection, 2 a word, 3 a line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SelectKind {
    Simple,
    Word,
    Line,
}

pub fn select_kind(clicks: usize) -> SelectKind {
    match clicks {
        0 | 1 => SelectKind::Simple,
        2 => SelectKind::Word,
        _ => SelectKind::Line,
    }
}

/// Bytes for pasting `text`. With bracketed paste the program sees it as one paste; ESC is
/// removed so the text cannot end the bracket early. Without it, newlines become ↵ (CR).
pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let body: String = text.chars().filter(|c| *c != '\x1b').collect();
        return format!("\x1b[200~{body}\x1b[201~").into_bytes();
    }
    text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIFT: Mods = Mods { ctrl: false, alt: false, shift: true, cmd: false };

    #[test]
    fn shift_page_keys_scroll_except_on_the_alternate_screen() {
        assert_eq!(scroll_key("pageup", SHIFT, false), Some(Scroll::PageUp));
        assert_eq!(scroll_key("pagedown", SHIFT, false), Some(Scroll::PageDown));
        assert_eq!(scroll_key("home", SHIFT, false), Some(Scroll::Top));
        assert_eq!(scroll_key("end", SHIFT, false), Some(Scroll::Bottom));
        assert_eq!(scroll_key("pageup", Mods::default(), false), None, "plain PageUp is the program's");
        assert_eq!(scroll_key("pageup", Mods { ctrl: true, ..SHIFT }, false), None);
        assert_eq!(scroll_key("pageup", SHIFT, true), None, "less and vim get it");
        assert_eq!(scroll_key("a", SHIFT, false), None);
    }

    #[test]
    fn the_wheel_belongs_to_programs_that_read_the_mouse_or_use_the_alternate_screen() {
        assert!(wheel_scrolls(false, false));
        assert!(!wheel_scrolls(true, false));
        assert!(!wheel_scrolls(false, true));
    }

    #[test]
    fn wheel_keeps_partial_lines_for_the_next_event() {
        let mut wheel = Wheel::default();
        assert_eq!(wheel.lines(10., 18.), 0);
        assert_eq!(wheel.lines(10., 18.), 1); // 20px: one line, 2px left
        assert_eq!(wheel.lines(34., 18.), 2); // 36px
        assert_eq!(wheel.lines(-18., 18.), -1);
        assert_eq!(wheel.lines(5., 0.), 0);
    }

    #[test]
    fn positions_map_to_cells_and_clamp_at_the_edges() {
        let (cell, grid) = ((8., 18.), (80, 24));
        assert_eq!(cell_at(0., 0., cell, grid), CellPoint { row: 0, col: 0, right: false });
        assert_eq!(cell_at(13., 20., cell, grid), CellPoint { row: 1, col: 1, right: true });
        assert_eq!(cell_at(11.9, 35.9, cell, grid), CellPoint { row: 1, col: 1, right: false });
        assert_eq!(cell_at(-5., -5., cell, grid), CellPoint { row: 0, col: 0, right: false });
        assert_eq!(cell_at(9999., 9999., cell, grid), CellPoint { row: 23, col: 79, right: true });
        assert_eq!(cell_at(5., 5., cell, (0, 0)), CellPoint { row: 0, col: 0, right: false });
    }

    #[test]
    fn click_count_picks_the_selection_kind() {
        assert_eq!(select_kind(1), SelectKind::Simple);
        assert_eq!(select_kind(2), SelectKind::Word);
        assert_eq!(select_kind(3), SelectKind::Line);
        assert_eq!(select_kind(4), SelectKind::Line);
    }

    #[test]
    fn paste_brackets_when_asked_and_otherwise_sends_returns() {
        assert_eq!(paste("a\nb", true), b"\x1b[200~a\nb\x1b[201~".to_vec());
        assert_eq!(paste("x\x1b[201~rm", true), b"\x1b[200~x[201~rm\x1b[201~".to_vec(), "no early end");
        assert_eq!(paste("a\nb\r\nc", false), b"a\rb\rc".to_vec());
    }
}
