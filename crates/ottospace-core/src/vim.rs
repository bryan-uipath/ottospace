//! Vim's moves for anything read a line at a time (the file viewer; the explorer next): a count
//! typed before a motion repeats it (`10j`), or names a line (`3G`, `3gg`). Pure: a key in, what
//! it does out. The caller keeps one `Keys` per view and moves its view.

use serde::Serialize;

/// The largest count kept: more digits change nothing. Far past any file the viewer reads.
pub const MAX_COUNT: u32 = 99_999;

/// A move of the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    /// `j`, ↓: a line down.
    Down,
    /// `k`, ↑: a line up.
    Up,
    /// ⌃D, ⌃U: half a page.
    HalfDown,
    HalfUp,
    /// ⌃F, Space, PageDown; ⌃B, PageUp: a page.
    PageDown,
    PageUp,
    /// `gg`, Home: the first line; with a count, that line.
    Top,
    /// `G`, End: the last line; with a count, that line.
    Bottom,
}

/// What one key did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// A digit or the first `g`: it waits for more.
    Pending,
    /// Esc took back what was typed.
    Cleared,
    /// A motion, and the count typed before it.
    Move { motion: Motion, count: Option<u32> },
    /// No move of these: anything typed before it is dropped, and the key is the caller's (⌃G,
    /// Esc with nothing typed).
    Unbound,
}

/// What is typed so far: a count, and a `g` waiting for its second.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Keys {
    count: Option<u32>,
    g: bool,
}

impl Keys {
    /// `key` as the shell names it: the character typed (`j`, `G`, `5`), else the key's name with
    /// its modifier (`escape`, `down`, `ctrl-d`).
    pub fn press(&mut self, key: &str) -> Press {
        let count = self.count;
        let digit = key.parse::<u32>().ok().filter(|_| key.len() == 1);
        match (digit, key) {
            // `0` alone is no count (vim's start of line): it is the caller's.
            (Some(0), _) if count.is_none() => {}
            (Some(digit), _) if !self.g => {
                let typed = count.unwrap_or(0).saturating_mul(10).saturating_add(digit);
                self.count = Some(typed.min(MAX_COUNT));
                return Press::Pending;
            }
            (_, "escape") if *self != Keys::default() => {
                *self = Keys::default();
                return Press::Cleared;
            }
            (_, "g") if !self.g => {
                self.g = true;
                return Press::Pending;
            }
            _ => {}
        }
        let g = self.g;
        *self = Keys::default();
        let motion = match key {
            "g" if g => Motion::Top,
            _ if g => return Press::Unbound,
            "j" | "down" => Motion::Down,
            "k" | "up" => Motion::Up,
            "ctrl-d" => Motion::HalfDown,
            "ctrl-u" => Motion::HalfUp,
            "ctrl-f" | "space" | " " | "pagedown" => Motion::PageDown,
            "ctrl-b" | "pageup" => Motion::PageUp,
            "home" => Motion::Top,
            "G" | "end" => Motion::Bottom,
            _ => return Press::Unbound,
        };
        Press::Move { motion, count }
    }

    /// What the status bar shows while a move waits: `10`, `g`, `3g`. `None`: nothing typed.
    pub fn pending(&self) -> Option<String> {
        let count = self.count.map(|n| n.to_string()).unwrap_or_default();
        let shown = format!("{count}{}", if self.g { "g" } else { "" });
        (!shown.is_empty()).then_some(shown)
    }
}

impl Serialize for Keys {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.pending().serialize(s)
    }
}

/// Where `motion` puts the first line on show: `top` now, `most` the last place it may be,
/// `page` the lines on show. A count repeats a step, or names a line from 1 (`3G`: line 3).
pub fn moved(motion: Motion, count: Option<u32>, top: u32, most: u32, page: u32) -> u32 {
    let times = count.unwrap_or(1);
    let step = |size: u32| size.max(1).saturating_mul(times);
    let line = |n: u32| n.saturating_sub(1);
    let to = match motion {
        Motion::Down => top.saturating_add(step(1)),
        Motion::Up => top.saturating_sub(step(1)),
        Motion::HalfDown => top.saturating_add(step(page / 2)),
        Motion::HalfUp => top.saturating_sub(step(page / 2)),
        Motion::PageDown => top.saturating_add(step(page)),
        Motion::PageUp => top.saturating_sub(step(page)),
        Motion::Top => count.map_or(0, line),
        Motion::Bottom => count.map_or(most, line),
    };
    to.min(most)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(keys: &mut Keys, all: &[&str]) -> Vec<Press> {
        all.iter().map(|key| keys.press(key)).collect()
    }

    #[test]
    fn a_count_repeats_the_motion_after_it() {
        let mut keys = Keys::default();
        let presses = typed(&mut keys, &["1", "0", "j"]);
        assert_eq!(presses, [Press::Pending, Press::Pending, Press::Move { motion: Motion::Down, count: Some(10) }]);
        assert_eq!((keys.pending(), moved(Motion::Down, Some(10), 2, 100, 30)), (None, 12));
        assert_eq!(keys.press("k"), Press::Move { motion: Motion::Up, count: None });
        assert_eq!(moved(Motion::Up, Some(5), 3, 100, 30), 0, "never above the first line");
    }

    #[test]
    fn zero_alone_is_no_count_but_after_a_digit_it_is_one() {
        let mut keys = Keys::default();
        assert_eq!((keys.press("0"), keys.pending()), (Press::Unbound, None));
        assert_eq!(typed(&mut keys, &["2", "0"]), [Press::Pending, Press::Pending]);
        assert_eq!(keys.pending().as_deref(), Some("20"));
    }

    #[test]
    fn esc_takes_back_a_count_and_is_the_callers_with_none() {
        let mut keys = Keys::default();
        assert_eq!(typed(&mut keys, &["4", "escape"]), [Press::Pending, Press::Cleared]);
        assert_eq!((keys.pending(), keys.press("j")), (None, Press::Move { motion: Motion::Down, count: None }));
        assert_eq!(keys.press("escape"), Press::Unbound);
        assert_eq!(typed(&mut keys, &["g", "escape"]), [Press::Pending, Press::Cleared]);
    }

    #[test]
    fn a_count_then_an_unbound_key_is_dropped_with_it() {
        let mut keys = Keys::default();
        assert_eq!(typed(&mut keys, &["7", "x"]), [Press::Pending, Press::Unbound]);
        assert_eq!(keys.press("j"), Press::Move { motion: Motion::Down, count: None });
        // A `g` waiting for its second takes no other key.
        assert_eq!(
            typed(&mut keys, &["g", "j", "j"])[1..],
            [Press::Unbound, Press::Move { motion: Motion::Down, count: None }]
        );
    }

    #[test]
    fn a_huge_count_is_capped_and_moves_no_further_than_the_end() {
        let mut keys = Keys::default();
        typed(&mut keys, &["9"; 30]);
        assert_eq!(keys.pending(), Some(MAX_COUNT.to_string()));
        let Press::Move { count, .. } = keys.press("j") else { panic!() };
        assert_eq!((count, moved(Motion::Down, count, 0, 40, 10)), (Some(MAX_COUNT), 40));
        assert_eq!(moved(Motion::PageDown, Some(u32::MAX), u32::MAX, u32::MAX, u32::MAX), u32::MAX);
    }

    #[test]
    fn a_count_names_the_line_g_and_gg_go_to() {
        let mut keys = Keys::default();
        assert_eq!(typed(&mut keys, &["3", "G"])[1], Press::Move { motion: Motion::Bottom, count: Some(3) });
        assert_eq!((moved(Motion::Bottom, Some(3), 9, 50, 10), moved(Motion::Bottom, None, 9, 50, 10)), (2, 50));
        let presses = typed(&mut keys, &["1", "2", "g", "g"]);
        assert_eq!(keys.pending(), None);
        assert_eq!(presses[3], Press::Move { motion: Motion::Top, count: Some(12) });
        assert_eq!((moved(Motion::Top, Some(12), 0, 50, 10), moved(Motion::Top, None, 9, 50, 10)), (11, 0));
        assert_eq!(moved(Motion::Top, Some(900), 0, 50, 10), 50, "a line past the end is the end");
        // While the `g` waits the status bar shows it.
        keys.press("3");
        keys.press("g");
        assert_eq!(keys.pending().as_deref(), Some("3g"));
    }

    #[test]
    fn half_and_whole_pages() {
        assert_eq!(moved(Motion::HalfDown, None, 0, 100, 30), 15);
        assert_eq!(moved(Motion::HalfUp, Some(2), 40, 100, 30), 10);
        assert_eq!(moved(Motion::PageDown, None, 0, 100, 0), 1, "a page of no lines still moves");
        assert_eq!(moved(Motion::PageUp, None, 90, 100, 30), 60);
    }
}
