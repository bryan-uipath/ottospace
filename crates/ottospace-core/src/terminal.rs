//! Terminal pane rules: scrolling back through history. The shell applies them to its
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
}
