//! Windows of one app that share a title (Zoom's several “Zoom Meeting”): a short detail tells
//! each apart where they are listed. Drawn beside the title, never in the state.

use crate::ax::Frame;
use crate::slot::display_of;

/// A listed window of one app, as the shell read it.
pub struct Listed<'a> {
    /// Its window-server id: the last resort's order.
    pub window: u32,
    pub title: &'a str,
    pub frame: Frame,
    pub subrole: &'a str,
}

/// Each window's detail, in `windows`' order: `None` for a title no other shares. Else, of what
/// differs within its title, in this order: its size (`large · 1280×800`), its display
/// (`display 2`), its kind (`panel`); still the same, `window 2` by window id.
pub fn details(windows: &[Listed], displays: &[Frame]) -> Vec<Option<String>> {
    let mut out = vec![None; windows.len()];
    for (at, window) in windows.iter().enumerate() {
        let group: Vec<&Listed> = windows.iter().filter(|w| w.title == window.title).collect();
        if group.len() > 1 {
            let place = windows[..at].iter().filter(|w| w.title == window.title).count();
            out[at] = group_words(&group, displays).into_iter().nth(place);
        }
    }
    out
}

/// One title's windows in words, in their order.
fn group_words(group: &[&Listed], displays: &[Frame]) -> Vec<String> {
    let differ = |of: &dyn Fn(&Listed) -> Option<String>| -> Vec<Option<String>> {
        let each: Vec<Option<String>> = group.iter().map(|w| of(w)).collect();
        match each.iter().all(|part| *part == each[0]) {
            true => vec![None; group.len()],
            false => each,
        }
    };
    let area = |w: &Listed| (w.frame.w * w.frame.h).round() as i64;
    let (most, least) = (group.iter().map(|w| area(w)).max(), group.iter().map(|w| area(w)).min());
    let size = differ(&|w| {
        let points = format!("{}×{}", w.frame.w.round(), w.frame.h.round());
        Some(match Some(area(w)) {
            a if a == most => format!("large · {points}"),
            a if a == least => format!("small · {points}"),
            _ => points,
        })
    });
    let display = differ(&|w| {
        Some(match display_of(&w.frame, displays) {
            Some(1) => "main display".into(),
            Some(n) => format!("display {n}"),
            None => "off screen".into(),
        })
    });
    // Only when each reads as a kind: else an unnamed one would be told apart by saying nothing.
    let named = group.iter().all(|w| role(w.subrole).is_some());
    let kind = differ(&|w| role(w.subrole).filter(|_| named).map(String::from));
    let mut words: Vec<String> = (0..group.len())
        .map(|i| [&size[i], &display[i], &kind[i]].into_iter().flatten().cloned().collect::<Vec<_>>().join(" · "))
        .collect();
    // Still alike: numbered by window id, never by list position.
    let said = words.clone();
    for (i, word) in words.iter_mut().enumerate() {
        let mut alike: Vec<u32> = (0..group.len()).filter(|j| said[*j] == said[i]).map(|j| group[j].window).collect();
        if alike.len() > 1 {
            alike.sort_unstable();
            let n = alike.iter().position(|id| *id == group[i].window).unwrap_or(0) + 1;
            *word = match said[i].is_empty() {
                true => format!("window {n}"),
                false => format!("{} · window {n}", said[i]),
            };
        }
    }
    words
}

/// A subrole that reads well: a standard window, a floating panel, a dialog. Others say nothing.
pub fn role(subrole: &str) -> Option<&'static str> {
    match subrole {
        "AXStandardWindow" => Some("window"),
        "AXFloatingWindow" | "AXSystemFloatingWindow" => Some("panel"),
        "AXDialog" | "AXSystemDialog" => Some("dialog"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(window: u32, title: &str, frame: Frame) -> Listed<'_> {
        Listed { window, title, frame, subrole: "AXStandardWindow" }
    }

    fn sized(w: f64, h: f64) -> Frame {
        Frame::new(100., 100., w, h)
    }

    /// The menu bar's display, and one to its right.
    fn two_displays() -> Vec<Frame> {
        vec![Frame::new(0., 0., 1512., 982.), Frame::new(1512., 0., 2560., 1440.)]
    }

    fn words(windows: &[Listed], displays: &[Frame]) -> Vec<Option<String>> {
        details(windows, displays)
    }

    #[test]
    fn a_title_no_other_window_shares_needs_no_detail() {
        let windows = [listed(1, "Zoom Meeting", sized(1280., 800.)), listed(2, "Chat", sized(320., 240.))];
        assert_eq!(words(&windows, &two_displays()), [None, None]);
        assert_eq!(words(&[], &[]), Vec::<Option<String>>::new());
    }

    #[test]
    fn two_of_one_title_are_told_apart_by_size_first() {
        let windows = [listed(1, "Zoom Meeting", sized(1280., 800.)), listed(2, "Zoom Meeting", sized(320., 240.))];
        let said = words(&windows, &two_displays());
        assert_eq!(said, [Some("large · 1280×800".into()), Some("small · 320×240".into())]);
    }

    #[test]
    fn two_of_one_size_on_two_displays_are_told_apart_by_display() {
        let left = Frame::new(100., 100., 800., 600.);
        let right = Frame::new(1600., 100., 800., 600.);
        let windows = [listed(1, "Zoom Meeting", right), listed(2, "Zoom Meeting", left)];
        let said = words(&windows, &two_displays());
        assert_eq!(said, [Some("display 2".into()), Some("main display".into())]);
    }

    #[test]
    fn two_alike_but_for_their_kind_are_told_apart_by_it() {
        let panel = Listed { subrole: "AXFloatingWindow", ..listed(2, "Zoom Meeting", sized(800., 600.)) };
        let windows = [listed(1, "Zoom Meeting", sized(800., 600.)), panel];
        assert_eq!(words(&windows, &[]), [Some("window".into()), Some("panel".into())]);
        // A subrole that reads as nothing: numbered instead.
        let other = Listed { subrole: "AXUnknown", ..listed(2, "Zoom Meeting", sized(800., 600.)) };
        let windows = [listed(1, "Zoom Meeting", sized(800., 600.)), other];
        assert_eq!(words(&windows, &[]), ["window 1", "window 2"].map(|w| Some(w.to_string())));
    }

    #[test]
    fn three_of_one_title_each_get_a_detail_and_others_stay_plain() {
        let windows = [
            listed(7, "Zoom Meeting", sized(640., 480.)),
            listed(5, "Zoom Meeting", sized(1280., 800.)),
            listed(9, "Chat", sized(320., 240.)),
            listed(6, "Zoom Meeting", sized(320., 240.)),
        ];
        let said = words(&windows, &two_displays());
        let want = [Some("640×480"), Some("large · 1280×800"), None, Some("small · 320×240")];
        assert_eq!(said, want.map(|w| w.map(String::from)));
    }

    /// The last resort numbers by window id: the list's order changes nothing, and a newer
    /// window closing renumbers none.
    #[test]
    fn alike_windows_are_numbered_by_window_id_stably() {
        let frame = sized(800., 600.);
        let three =
            [listed(12, "Zoom Meeting", frame), listed(10, "Zoom Meeting", frame), listed(11, "Zoom Meeting", frame)];
        let said = words(&three, &[]);
        assert_eq!(said, ["window 3", "window 1", "window 2"].map(|w| Some(w.to_string())));
        let two = [listed(11, "Zoom Meeting", frame), listed(10, "Zoom Meeting", frame)];
        assert_eq!(words(&two, &[]), ["window 2", "window 1"].map(|w| Some(w.to_string())));
        // Two alike beside one larger: only the two alike are numbered.
        let mixed = [listed(3, "M", sized(1280., 800.)), listed(2, "M", frame), listed(1, "M", frame)];
        let want = ["large · 1280×800", "small · 800×600 · window 2", "small · 800×600 · window 1"];
        assert_eq!(words(&mixed, &[]), want.map(|w| Some(w.to_string())));
    }
}
