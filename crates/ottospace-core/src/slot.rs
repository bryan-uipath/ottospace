//! Slots (`docs/ai/slots.md`): a pane that keeps its place and says which window belongs in it.
//! What a slot asks for (`Slot`, kept on its pane's record), what a look found (`Look`), and
//! which windows a slot may take, in the order it offers them (`candidates`). The model runs the
//! states (`model/slots.rs`).

use serde::{Deserialize, Serialize};

use crate::ax::Frame;
use crate::rules::{self, Facts, Rule, Selector};

/// Only the user makes, changes, fills or clears a slot: a client's key, click or label is the
/// client's.
pub const USERS_ALONE: &str = "a slot is the user's: only they make, change, fill or clear one";
/// What a look found, and that a window went, only ottospace reports.
pub const SHELLS_ALONE: &str = "only ottospace reports windows";
/// A client does not take a slot out of the user's layout, nor move it to a hidden tab.
pub const SLOT_PANE: &str = "a slot is the user's: a client does not close, move or swap its pane";

/// How long a slot stays Away once its window went, while its app runs: then it waits again.
pub const AWAY_MS: u64 = 10_000;

/// What a slot asks for, as its pane's record keeps it under `slot`. `select` holds no `app`:
/// the record's `bundle` and `name` are its app.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    #[serde(default)]
    pub select: Selector,
    #[serde(default)]
    pub pick: Pick,
    #[serde(default)]
    pub matching: Matching,
    #[serde(default)]
    pub absent: Absent,
    #[serde(default)]
    pub closed: Closed,
}

impl Slot {
    /// What it waits for, in the dialog's words: `Zoom · window id zm.meeting.window.main`,
    /// `Zoom · title “Weekly sync” · the largest`. Only `app`: any window of it.
    pub fn words(&self, app: &str) -> String {
        let select = &self.select;
        let mut words = vec![app.to_string()];
        words.extend(select.identifier.as_ref().map(|id| format!("window id {id}")));
        // A pattern written by hand in layout.json says so.
        words.extend(select.title.as_ref().map(|t| match t.contains('*') {
            true => format!("title matching “{t}”"),
            false => format!("title “{t}”"),
        }));
        words.extend(select.subrole.as_ref().map(|subrole| format!("subrole {subrole}")));
        if words.len() == 1 {
            words.push("any window".into());
        }
        words.extend(match self.pick {
            Pick::Largest => Some("the largest".to_string()),
            Pick::Smallest => Some("the smallest".to_string()),
            Pick::Display(n) => Some(format!("on display {n}")),
            Pick::Any => None,
        });
        words.join(" · ")
    }
}

/// Which window, when several match: `"largest"`, `"smallest"`, `{ "display": 2 }`, `"any"`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pick {
    Largest,
    Smallest,
    /// Only a window on this display: 1 has the menu bar, then left to right, top to bottom.
    Display(u32),
    /// Any window not already in a pane: the frontmost first.
    #[default]
    Any,
}

/// Manual offers a window and waits for the user; Automatic takes one that opened while the
/// slot waited, on the tab on show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Matching {
    #[default]
    Manual,
    Automatic,
}

/// When the app is not running: wait for it, or start it (slice 5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Absent {
    #[default]
    Wait,
    Launch,
}

/// When its window closes: keep the slot, or close the pane (slice 5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Closed {
    #[default]
    Keep,
    Close,
}

/// Where a slot is in its life (`docs/ai/slots.md` §3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Its app is not running.
    Absent,
    /// Running, and no window matches (or none was looked at yet).
    #[default]
    Empty,
    /// A window matches: the pane offers it.
    Offered,
    /// It holds its window.
    Filled,
    /// Its window went while its app runs: one of the app's that it selects comes straight back.
    Away,
}

/// A window a look found, as the shell read it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Seen {
    pub pid: i32,
    /// Its window-server id.
    pub window: u32,
    pub title: String,
    pub identifier: Option<String>,
    pub subrole: String,
    pub frame: Frame,
}

/// What a look at one app found: its windows, front to back, and the screen around them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Look {
    pub bundle: Option<String>,
    pub name: String,
    pub running: bool,
    pub windows: Vec<Seen>,
    /// Every display's frame, global top-left coordinates: the one at the origin has the menu bar.
    pub displays: Vec<Frame>,
    /// The focused window of the front app, when that app is not ottospace: the user is in it.
    pub frontmost: Option<u32>,
}

impl Look {
    pub fn facts<'a>(&'a self, seen: &'a Seen) -> Facts<'a> {
        Facts {
            app: &self.name,
            bundle: self.bundle.as_deref(),
            title: &seen.title,
            subrole: &seen.subrole,
            identifier: seen.identifier.as_deref(),
        }
    }

    /// The windows `slot` selects that may go in a pane (the window rules first: a slot cannot
    /// host what hosting refuses) and are not `passed` over, in the order it offers them.
    pub fn candidates<'a>(&'a self, slot: &Slot, rules: &[Rule], passed: impl Fn(u32) -> bool) -> Vec<&'a Seen> {
        let fits = |seen: &&Seen| {
            let facts = self.facts(seen);
            rules::manages(rules, &facts) && !passed(seen.window) && slot.select.selects(&facts)
        };
        let mut found: Vec<&Seen> = self.windows.iter().filter(fits).collect();
        let area = |seen: &&Seen| (seen.frame.w * seen.frame.h) as i64;
        match slot.pick {
            // Stable: of two the same size, the one in front.
            Pick::Largest => found.sort_by_key(|seen| std::cmp::Reverse(area(seen))),
            Pick::Smallest => found.sort_by_key(area),
            Pick::Display(n) => found.retain(|seen| display_of(&seen.frame, &self.displays) == Some(n)),
            Pick::Any => {}
        }
        found
    }
}

/// The display a frame's middle is on, numbered as `Pick::Display` says. `None`: on none.
pub fn display_of(frame: &Frame, displays: &[Frame]) -> Option<u32> {
    let mut order: Vec<&Frame> = displays.iter().collect();
    // The menu bar's display first, then left to right, then top to bottom.
    let rank = |d: &&Frame| (!(d.x == 0. && d.y == 0.), d.x as i64, d.y as i64);
    order.sort_by_key(rank);
    let (x, y) = (frame.x + frame.w / 2., frame.y + frame.h / 2.);
    let on = |d: &&&Frame| x >= d.x && x < d.x + d.w && y >= d.y && y < d.y + d.h;
    order.iter().position(|d| on(&d)).map(|at| at as u32 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(window: u32, title: &str, identifier: Option<&str>, frame: Frame) -> Seen {
        let identifier = identifier.map(String::from);
        Seen { pid: 7, window, title: title.into(), identifier, subrole: "AXStandardWindow".into(), frame }
    }

    /// Zoom with its two `Zoom Meeting` windows (zoom-ax.md) and its home window, front to back.
    fn zoom() -> Look {
        let main = Frame::new(100., 100., 1000., 700.);
        Look {
            bundle: Some("us.zoom.xos".into()),
            name: "zoom.us".into(),
            running: true,
            windows: vec![
                seen(1, "Zoom Meeting", None, Frame::new(2000., 100., 1200., 800.)),
                seen(2, "Zoom Meeting", Some("zm.meeting.window.main"), main),
                seen(3, "Zoom Workplace", None, Frame::new(0., 0., 400., 300.)),
                seen(4, "zoom share toolbar window", None, main),
            ],
            displays: vec![Frame::new(1920., 0., 1920., 1080.), Frame::new(0., 0., 1920., 1080.)],
            frontmost: None,
        }
    }

    fn slot(text: &str) -> Slot {
        serde_json::from_str(text).unwrap()
    }

    fn ids(found: Vec<&Seen>) -> Vec<u32> {
        found.iter().map(|seen| seen.window).collect()
    }

    #[test]
    fn the_record_reads_as_the_note_writes_it() {
        let text = r#"{ "select": { "identifier": "zm.meeting.window.main" },
            "pick": "largest", "matching": "manual", "absent": "wait", "closed": "keep" }"#;
        let read = slot(text);
        assert_eq!(read.select.identifier.as_deref(), Some("zm.meeting.window.main"));
        assert_eq!((read.pick, read.matching), (Pick::Largest, Matching::Manual));
        assert_eq!(slot(r#"{"pick": {"display": 2}}"#).pick, Pick::Display(2));
        assert_eq!(slot("{}"), Slot::default(), "every field has a default");
        assert!(serde_json::from_str::<Slot>(r#"{"matchng": "manual"}"#).is_err(), "a misspelt key is refused");
        let written = serde_json::to_string(&read).unwrap();
        assert_eq!(slot(&written), read);
    }

    #[test]
    fn rules_come_first_then_the_selector_then_the_pick() {
        let look = zoom();
        // Zoom's share toolbar is ignored by a built-in rule, whatever the slot says.
        assert_eq!(ids(look.candidates(&slot(r#"{"pick": "any"}"#), &[], |_| false)), [1, 2, 3]);
        let by_id = slot(r#"{"select": {"identifier": "zm.meeting.window.main"}}"#);
        assert_eq!(ids(look.candidates(&by_id, &[], |_| false)), [2]);
        let titled = |pick: &str| slot(&format!(r#"{{"select": {{"title": "zoom meeting"}}, "pick": {pick}}}"#));
        assert_eq!(ids(look.candidates(&titled(r#""largest""#), &[], |_| false)), [1, 2]);
        assert_eq!(ids(look.candidates(&titled(r#""smallest""#), &[], |_| false)), [2, 1]);
        // Display 1 has the menu bar (at the origin), though it is listed second.
        assert_eq!(ids(look.candidates(&titled(r#"{"display": 1}"#), &[], |_| false)), [2]);
        assert_eq!(ids(look.candidates(&titled(r#"{"display": 2}"#), &[], |_| false)), [1]);
        // A window passed over (in a pane, declined) is no candidate.
        assert_eq!(ids(look.candidates(&titled(r#""any""#), &[], |w| w == 1)), [2]);
        // A user's rule that ignores the main window wins over the slot.
        let ignore: Rule = serde_json::from_str(r#"{"identifier": "zm.*", "do": "ignore"}"#).unwrap();
        assert!(look.candidates(&by_id, &[ignore], |_| false).is_empty());
    }

    #[test]
    fn a_slot_says_what_it_waits_for_in_plain_words() {
        let by_id = slot(r#"{"select": {"identifier": "zm.meeting.window.main"}}"#);
        assert_eq!(by_id.words("Zoom"), "Zoom · window id zm.meeting.window.main");
        let titled = slot(r#"{"select": {"title": "Weekly sync"}, "pick": "largest"}"#);
        assert_eq!(titled.words("Zoom"), "Zoom · title “Weekly sync” · the largest");
        let pattern = slot(r#"{"select": {"title": "*Incident*"}, "pick": {"display": 2}}"#);
        assert_eq!(pattern.words("Meetings"), "Meetings · title matching “*Incident*” · on display 2");
        assert_eq!(Slot::default().words("Zoom"), "Zoom · any window");
    }

    #[test]
    fn displays_are_numbered_from_the_menu_bar_then_left_to_right() {
        let displays = [
            Frame::new(1920., 0., 1920., 1080.),
            Frame::new(-1280., 0., 1280., 1024.),
            Frame::new(0., 0., 1920., 1080.),
        ];
        let at = |x: f64, y: f64| display_of(&Frame::new(x, y, 10., 10.), &displays);
        assert_eq!((at(10., 10.), at(-500., 10.), at(2000., 10.)), (Some(1), Some(2), Some(3)));
        assert_eq!(at(10., 5000.), None);
    }
}
