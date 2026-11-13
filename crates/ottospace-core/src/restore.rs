//! Giving hosted windows back: whether the user can reach a window, where one that cannot goes,
//! and the record of hosted windows that outlives a crash. Pure: the shell reads screens and windows.
//! Coordinates are global, top-left origin, in points (as Accessibility gives them).

use serde::{Deserialize, Serialize};

use crate::ax::Frame;

/// Past this no frame is a window's. The record is a file the user can edit: its numbers are untrusted.
const LIMIT: f64 = 100_000.0;

fn valid(f: &Frame) -> bool {
    let coord = |v: f64| v.is_finite() && v.abs() <= LIMIT;
    let size = |v: f64| v.is_finite() && v > 0.0 && v <= LIMIT;
    coord(f.x) && coord(f.y) && size(f.w) && size(f.h)
}

/// How much of a window's top the user must see to take hold of it by its title bar.
pub const GRIP: (f64, f64) = (120.0, 60.0);

/// True when at least `GRIP` of the window's top (all of it, for a smaller window) lies inside one
/// screen's visible frame: below the menu bar, above the Dock.
pub fn reachable(frame: Frame, screens: &[Frame]) -> bool {
    let (wide, tall) = (GRIP.0.min(frame.w), GRIP.1.min(frame.h));
    screens.iter().any(|s| {
        let shared = (frame.x + frame.w).min(s.x + s.w) - frame.x.max(s.x);
        // A point of slack: AppKit puts a window's top at the menu bar's bottom, give or take.
        shared >= wide && frame.y >= s.y - 1.0 && frame.y + tall <= s.y + s.h + 1.0
    })
}

/// True when no part of the window lies on any display.
pub fn off_every(frame: Frame, displays: &[Frame]) -> bool {
    !displays.iter().any(|d| frame.intersects(d))
}

/// How far above the parking spot macOS may lift a parked window: it keeps a title bar above the Dock.
const LIFT: f64 = 400.0;

/// True when the window sits where ottospace parks one: its left edge on the spot's column (one
/// point of it on screen), its top at most `LIFT` above the spot.
pub fn parked(frame: Frame, spot: (f64, f64)) -> bool {
    (frame.x - spot.0).abs() <= 2.0 && frame.y <= spot.1 + 2.0 && frame.y >= spot.1 - LIFT
}

/// The window at its own size, no larger than `screen`, centred on it.
pub fn centred(frame: Frame, screen: Frame) -> Frame {
    let (w, h) = (frame.w.min(screen.w), frame.h.min(screen.h));
    Frame::new(screen.x + (screen.w - w) / 2.0, screen.y + (screen.h - h) / 2.0, w, h)
}

/// Where a window goes back to: `original` if the user can reach it there, else centred on `main`.
/// E.g. an original on a display since unplugged, or one ottospace found already parked.
pub fn destination(original: Frame, screens: &[Frame], main: Frame) -> Frame {
    if reachable(original, screens) { original } else { centred(original, main) }
}

/// One hosted window, as the record keeps it. No title: the file holds numbers only.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// The ottospace process that hosts it; 0 for a record read from the old file.
    pub owner: u32,
    pub pid: i32,
    pub window: u32,
    /// Where to put it back.
    pub frame: Frame,
}

impl Entry {
    fn valid(&self) -> bool {
        self.pid > 0 && self.window > 0 && valid(&self.frame)
    }
}

/// The record's entries. An entry with a number out of bounds is dropped; a file that does not
/// read is no entries.
pub fn parse(text: &str) -> Vec<Entry> {
    let entries: Vec<Entry> = serde_json::from_str(text).unwrap_or_default();
    entries.into_iter().filter(Entry::valid).collect()
}

pub fn to_text(entries: &[Entry]) -> String {
    serde_json::to_string(entries).unwrap_or_else(|_| "[]".into())
}

/// The old record (`hosted.txt`): a `pid window-id x y w h` line per window, owner unknown.
pub fn parse_old(text: &str) -> Vec<Entry> {
    text.lines()
        .filter_map(|line| {
            let v: Vec<f64> = line.split_whitespace().filter_map(|s| s.parse().ok()).collect();
            let [pid, window, x, y, w, h] = v[..] else { return None };
            let whole = |n: f64, max: f64| (n.fract() == 0.0 && n > 0.0 && n <= max).then_some(n);
            let pid = whole(pid, i32::MAX as f64)? as i32;
            let window = whole(window, u32::MAX as f64)? as u32;
            Some(Entry { owner: 0, pid, window, frame: Frame::new(x, y, w, h) })
        })
        .filter(Entry::valid)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: Frame = Frame { x: 0.0, y: 25.0, w: 1512.0, h: 920.0 };
    /// A second screen to the left, as `x = -1920`.
    const LEFT: Frame = Frame { x: -1920.0, y: 0.0, w: 1920.0, h: 1055.0 };

    #[test]
    fn a_window_whose_title_bar_shows_is_reachable() {
        let screens = [MAIN, LEFT];
        assert!(reachable(Frame::new(100.0, 100.0, 800.0, 600.0), &screens));
        // On the left screen.
        assert!(reachable(Frame::new(-1500.0, 200.0, 1200.0, 800.0), &screens));
        // Its top just under the menu bar.
        assert!(reachable(Frame::new(0.0, 25.0, 800.0, 600.0), &screens));
        // Mostly off the right edge, 200pt still showing.
        assert!(reachable(Frame::new(1312.0, 100.0, 800.0, 600.0), &screens));
        // A small window, whole on screen.
        assert!(reachable(Frame::new(10.0, 40.0, 80.0, 30.0), &screens));
    }

    #[test]
    fn a_window_the_user_cannot_take_hold_of_is_not() {
        let screens = [MAIN, LEFT];
        // Parked: one point of it on screen.
        assert!(!reachable(Frame::new(1511.0, 700.0, 800.0, 600.0), &screens));
        // Only 100pt of its top showing.
        assert!(!reachable(Frame::new(1412.0, 100.0, 800.0, 600.0), &screens));
        // Its title bar under the menu bar, or below the bottom.
        assert!(!reachable(Frame::new(100.0, 0.0, 800.0, 600.0), &screens));
        assert!(!reachable(Frame::new(100.0, 900.0, 800.0, 600.0), &screens));
        // On a screen since unplugged.
        assert!(!reachable(Frame::new(-1500.0, 200.0, 800.0, 600.0), &[MAIN]));
        // Straddling two screens with neither holding 120pt of it.
        assert!(!reachable(Frame::new(-100.0, 100.0, 200.0, 600.0), &[MAIN, LEFT]));
    }

    #[test]
    fn off_every_display_means_no_point_on_any() {
        let displays = [Frame::new(0.0, 0.0, 1512.0, 982.0), Frame::new(-1920.0, 0.0, 1920.0, 1080.0)];
        assert!(off_every(Frame::new(1512.0, 100.0, 800.0, 600.0), &displays));
        assert!(off_every(Frame::new(-3000.0, 100.0, 800.0, 600.0), &displays));
        assert!(!off_every(Frame::new(1511.0, 100.0, 800.0, 600.0), &displays));
        assert!(!off_every(Frame::new(-1000.0, 100.0, 800.0, 600.0), &displays));
    }

    #[test]
    fn a_parked_window_is_known_by_the_spot() {
        let spot = (1511.0, 981.0);
        assert!(parked(Frame::new(1511.0, 900.0, 800.0, 600.0), spot));
        // Lifted above the Dock.
        assert!(parked(Frame::new(1511.0, 700.0, 800.0, 600.0), spot));
        assert!(!parked(Frame::new(1400.0, 900.0, 800.0, 600.0), spot));
        assert!(!parked(Frame::new(1511.0, 100.0, 800.0, 600.0), spot));
    }

    #[test]
    fn a_window_that_cannot_go_back_is_centred_on_the_main_screen_at_its_size() {
        assert_eq!(centred(Frame::new(5000.0, 5000.0, 800.0, 600.0), MAIN), Frame::new(356.0, 185.0, 800.0, 600.0));
        // Larger than the screen: clamped to it.
        assert_eq!(centred(Frame::new(0.0, 0.0, 3000.0, 2000.0), MAIN), MAIN);
        // An original on the left screen stays there while it is plugged in; else the main screen.
        let left = Frame::new(-1500.0, 200.0, 1200.0, 800.0);
        assert_eq!(destination(left, &[MAIN, LEFT], MAIN), left);
        assert_eq!(destination(left, &[MAIN], MAIN), Frame::new(156.0, 85.0, 1200.0, 800.0));
        // An original found parked.
        let found = Frame::new(1511.0, 700.0, 800.0, 600.0);
        assert!(reachable(destination(found, &[MAIN, LEFT], MAIN), &[MAIN]));
    }

    #[test]
    fn the_record_reads_back_and_drops_what_is_no_window() {
        let entry = Entry { owner: 42, pid: 7, window: 99, frame: Frame::new(-1500.0, 200.0, 1200.0, 800.0) };
        assert_eq!(parse(&to_text(&[entry])), vec![entry]);
        assert_eq!(parse("not json"), vec![]);
        let bad = [
            Entry { pid: 0, ..entry },
            Entry { pid: -1, ..entry },
            Entry { window: 0, ..entry },
            Entry { frame: Frame::new(1e9, 0.0, 10.0, 10.0), ..entry },
            Entry { frame: Frame::new(0.0, 0.0, 0.0, 10.0), ..entry },
            Entry { frame: Frame::new(0.0, 0.0, -5.0, 10.0), ..entry },
        ];
        assert_eq!(parse(&to_text(&bad)), vec![]);
        // NaN and infinity do not survive JSON: serde writes null, which reads as no entries.
        assert_eq!(parse(r#"[{"owner":1,"pid":2,"window":3,"frame":{"x":null,"y":0,"w":1,"h":1}}]"#), vec![]);
    }

    #[test]
    fn the_old_record_reads_with_no_owner() {
        let old = "123 456 -1500 200 1200 800\n0 1 0 0 10 10\n1.5 2 0 0 10 10\n9 9 0 0 nan 10\nshort line\n";
        let frame = Frame::new(-1500.0, 200.0, 1200.0, 800.0);
        assert_eq!(parse_old(old), vec![Entry { owner: 0, pid: 123, window: 456, frame }]);
    }
}
