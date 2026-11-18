//! Window rules: which windows of other apps ottospace leaves alone. A rule is a selector over a
//! few facts of a window, and what to do with a window it selects. The settings file holds the
//! user's rules; slots (`docs/ai/slots.md`) use the same selector.

use serde::{Deserialize, Serialize};

use crate::ax::Frame;

/// What is known of a window when it is first seen.
#[derive(Clone, Copy, Debug)]
pub struct Facts<'a> {
    /// The app's name as macOS reports it, which is nearly always what the Apps list shows:
    /// `zoom.us`. The bundle id is the sure way to name an app.
    pub app: &'a str,
    /// The app's bundle id, if it has one: `us.zoom.xos`.
    pub bundle: Option<&'a str>,
    pub title: &'a str,
    /// The accessibility subrole: `AXStandardWindow`, `AXSystemDialog`, ...
    pub subrole: &'a str,
    /// The app's own name for the window (`AXIdentifier`), if it gives one: Zoom's main meeting
    /// window is `zm.meeting.window.main`; its share viewer, of the same title, has none.
    pub identifier: Option<&'a str>,
}

/// Which windows: `{ "app": "us.zoom.xos", "identifier": "zm.meeting.window.main" }`. Every
/// field given must match; none given selects every window. Each is a `glob`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selector {
    /// The app's name or its bundle id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// A window with no identifier has empty text here: only a pattern such as `*` matches it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subrole: Option<String>,
}

impl Selector {
    pub fn selects(&self, window: &Facts) -> bool {
        let app = |pattern: &String| glob(pattern, window.app) || window.bundle.is_some_and(|b| glob(pattern, b));
        let field = |pattern: &Option<String>, text: &str| pattern.as_ref().is_none_or(|p| glob(p, text));
        self.app.as_ref().is_none_or(app)
            && field(&self.title, window.title)
            && field(&self.identifier, window.identifier.unwrap_or_default())
            && field(&self.subrole, window.subrole)
    }
}

/// `{ "app": "us.zoom.xos", "title": "zoom share*", "do": "ignore" }`: a selector's fields beside
/// what to do. `*` stands for any run of characters; letters match whatever their case.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(from = "RuleFields")]
pub struct Rule {
    pub select: Selector,
    pub action: Action,
}

/// A rule as written. Its own struct, not `#[serde(flatten)]`, which serde refuses beside
/// `deny_unknown_fields`: a misspelt key must stay an error.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, expecting = "struct Rule")]
struct RuleFields {
    #[serde(default)]
    app: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    identifier: Option<String>,
    #[serde(default)]
    subrole: Option<String>,
    #[serde(rename = "do")]
    action: Action,
}

impl From<RuleFields> for Rule {
    fn from(RuleFields { app, title, identifier, subrole, action }: RuleFields) -> Self {
        Rule { select: Selector { app, title, identifier, subrole }, action }
    }
}

/// What a rule does with a window it selects.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Never put it in a pane, nor move it: not when its app is hosted, and not when it opens later.
    Ignore,
    /// Treat it as any other window. A user's rule can say this to undo a built-in `ignore`.
    Manage,
    /// A new window of a hosted app goes in a split, even a dialog or a small one.
    Split,
    /// A new window of a hosted app stays a free window over its app's pane, as a pop-up does.
    /// Never put in a pane, as `ignore`.
    Float,
}

/// True if ottospace may put this window in a pane. The first rule that selects the window
/// decides, the user's rules before the built-in ones. With no rule: only a standard window,
/// which leaves out panels, pop-ups and an app's floating controls.
pub fn manages(user: &[Rule], window: &Facts) -> bool {
    match first(user, window) {
        Some(Action::Manage) | None => window.subrole == STANDARD,
        // A dialog a `split` rule put in a pane is one, for the capture key and marks too.
        Some(Action::Split) => judged(window.subrole),
        Some(Action::Ignore | Action::Float) => false,
    }
}

const STANDARD: &str = "AXStandardWindow";

/// True for the kinds of window `place` decides on: standard windows, dialogs and panels. Any
/// other (a toast, a popover) is left alone, and looked at again in case it becomes one.
pub fn judged(subrole: &str) -> bool {
    subrole == STANDARD || POP_UP.contains(&subrole)
}

/// What the first rule that selects the window says, the user's rules before the built-in ones.
fn first(user: &[Rule], window: &Facts) -> Option<Action> {
    user.iter().chain(&built_in()).find(|rule| rule.select.selects(window)).map(|rule| rule.action)
}

/// What the shell read of a new window of a hosted app, and of the pane it would split. Points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shape {
    /// `AXModal`: it blocks its app until answered.
    pub modal: bool,
    /// A sheet, or a window whose `AXParent` is another window: macOS keeps it on that window.
    pub sheet: bool,
    pub size: Option<[f64; 2]>,
    /// The content size of the app pane it would split (`Adopt`'s `beside`), if drawn.
    pub pane: Option<[f64; 2]>,
    /// The least width that pane's window kept when asked for less: it refuses narrower.
    pub min_width: Option<f64>,
}

/// Where a new window of a hosted app goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// A pane of its own, beside its app's (`Action::Adopt`).
    Split,
    /// A free window at its own size, over its app's pane (`float_to`).
    Float,
    /// Left as it is: ignored, a sheet, or not a kind of window ottospace hosts.
    Leave,
}

/// Subroles of dialogs and panels: never split, unless a rule says so.
const POP_UP: [&str; 4] = ["AXDialog", "AXSystemDialog", "AXFloatingWindow", "AXSystemFloatingWindow"];

/// A standard window under this, both ways, is a pop-up: a join or login dialog is about 400×300.
const SMALL: [f64; 2] = [600.0, 400.0];

/// Where a new window of a hosted app goes. A sheet stays on its window. Then the first rule that
/// selects it decides, both ways; else a dialog, a panel or a modal window floats, and so does a
/// standard window under 600×400, under half its app's pane both ways, or whose split would
/// leave its app's pane narrower than that pane's window kept. Other standard windows split.
pub fn place(user: &[Rule], window: &Facts, shape: &Shape) -> Place {
    if shape.sheet || !judged(window.subrole) {
        return Place::Leave;
    }
    match first(user, window) {
        Some(Action::Ignore) => return Place::Leave,
        Some(Action::Split) => return Place::Split,
        Some(Action::Float) => return Place::Float,
        Some(Action::Manage) | None => {}
    }
    if window.subrole != STANDARD || shape.modal {
        return Place::Float;
    }
    let under = |size: [f64; 2], limit: [f64; 2]| size[0] < limit[0] && size[1] < limit[1];
    let half = shape.pane.map(|pane| [pane[0] / 2.0, pane[1] / 2.0]);
    let small = shape.size.is_some_and(|size| under(size, SMALL) || half.is_some_and(|half| under(size, half)));
    // A split to the right halves the pane's width; its height stays.
    let squeezed = shape.pane.zip(shape.min_width).is_some_and(|(pane, min)| min > pane[0] / 2.0);
    if small || squeezed { Place::Float } else { Place::Split }
}

/// Where a floating pop-up's top left goes (global frames), or `None` to leave it: centred over
/// its app's `pane` unless its middle already is over it, never above or left of the pane. With
/// no pane drawn, it moves only if the user could not reach it. Wherever it goes, out of reach
/// (a pane partly off screen) becomes the middle of `main`.
pub fn float_to(window: Frame, pane: Option<Frame>, screens: &[Frame], main: Frame) -> Option<(f64, f64)> {
    let over = pane.filter(|pane| {
        let (x, y) = (window.x + window.w / 2.0, window.y + window.h / 2.0);
        !((pane.x..=pane.x + pane.w).contains(&x) && (pane.y..=pane.y + pane.h).contains(&y))
    });
    let to = over.map_or(window, |pane| {
        let (x, y) = (pane.x + ((pane.w - window.w) / 2.0).max(0.0), pane.y + ((pane.h - window.h) / 2.0).max(0.0));
        Frame { x, y, ..window }
    });
    let to = crate::restore::destination(to, screens, main);
    (to != window).then_some((to.x, to.y))
}

/// The width a hosted window refuses to go below, after a check that asked it for `asked` and
/// found it `kept`: wider than asked, that; as wide, any refusal wider than `asked` is past.
pub fn refused_width(was: Option<f64>, kept: f64, asked: f64) -> Option<f64> {
    if kept > asked + 1.0 { Some(kept) } else { was.filter(|width| *width <= asked) }
}

/// Rules that ship with ottospace, for windows known to break when moved. Measured: see
/// `docs/ai/1-initial-brainstorm/zoom-ax.md`.
pub fn built_in() -> Vec<Rule> {
    let zoom = |title: &str| Rule {
        select: Selector { app: Some("us.zoom.xos".into()), title: Some(title.into()), ..Selector::default() },
        action: Action::Ignore,
    };
    // Zoom's floating controls while sharing, its annotation layer, and its pop-ups.
    vec![
        zoom("zoom share*"),
        zoom("zoom annotation*"),
        zoom("Annotation - Zoom"),
        zoom("Reactions"),
        zoom("Meeting chat"),
    ]
}

/// `pattern` against the whole of `text`: `*` is any run of characters (none too), and letters
/// match in either case. `zoom share*` matches `Zoom Share Toolbar Window`.
pub fn glob(pattern: &str, text: &str) -> bool {
    let (pattern, text): (Vec<char>, Vec<char>) =
        (pattern.to_lowercase().chars().collect(), text.to_lowercase().chars().collect());
    // `star`: where the last `*` was, and how much of the text it has taken so far.
    let (mut p, mut t, mut star): (usize, usize, Option<(usize, usize)>) = (0, 0, None);
    while t < text.len() {
        if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, t));
            p += 1;
        } else if p < pattern.len() && pattern[p] == text[t] {
            (p, t) = (p + 1, t + 1);
        } else if let Some((at, taken)) = star {
            // Let the last `*` take one more character, and try again from there.
            (p, t, star) = (at + 1, taken + 1, Some((at, taken + 1)));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zoom<'a>(title: &'a str, subrole: &'a str) -> Facts<'a> {
        Facts { app: "zoom.us", bundle: Some("us.zoom.xos"), title, subrole, identifier: None }
    }

    #[test]
    fn a_star_is_any_run_of_characters() {
        assert!(glob("zoom share*", "Zoom Share Toolbar Window"));
        assert!(glob("*chat*", "Meeting chat") && glob("*", "") && glob("Zoom Meeting", "zoom meeting"));
        assert!(glob("a*b*c", "a-b-b-c") && glob("*.app", "Zoom.app"));
        assert!(!glob("zoom share", "zoom share toolbar") && !glob("a*b", "a-b-c") && !glob("", "x"));
    }

    #[test]
    fn with_no_rule_only_standard_windows_are_managed() {
        let other = Facts {
            app: "Safari",
            bundle: Some("com.apple.Safari"),
            title: "Start Page",
            subrole: "AXStandardWindow",
            identifier: None,
        };
        assert!(manages(&[], &other));
        assert!(!manages(&[], &Facts { subrole: "AXDialog", ..other }));
    }

    #[test]
    fn zooms_floating_windows_are_left_alone() {
        // The windows recorded in a meeting. Only the meeting windows are standard.
        assert!(manages(&[], &zoom("Zoom Meeting", "AXStandardWindow")));
        for title in ["zoom share statusbar window", "zoom share toolbar window", "zoom annotation entrypoint"] {
            assert!(!manages(&[], &zoom(title, "AXSystemDialog")), "{title}");
            // Named, so they stay ignored even if Zoom made them standard windows.
            assert!(!manages(&[], &zoom(title, "AXStandardWindow")), "{title}");
        }
        assert!(!manages(&[], &zoom("Reactions", "AXUnknown")));
    }

    #[test]
    fn a_users_rule_comes_first_and_every_field_must_match() {
        let rule = |text: &str| serde_json::from_str::<Rule>(text).unwrap();
        // By name or by bundle id; title and subrole narrow it.
        let ignore_meeting = rule(r#"{"app": "zoom.us", "title": "Zoom Meeting", "do": "ignore"}"#);
        assert!(!manages(&[ignore_meeting.clone()], &zoom("Zoom Meeting", "AXStandardWindow")));
        assert!(manages(&[ignore_meeting], &zoom("Zoom Workplace", "AXStandardWindow")));
        let by_bundle = rule(r#"{"app": "us.zoom.*", "do": "ignore"}"#);
        assert!(!manages(&[by_bundle], &zoom("Zoom Workplace", "AXStandardWindow")));
        // `manage` undoes a built-in rule, for a standard window. It cannot make a dialog hostable.
        let keep_chat = rule(r#"{"app": "us.zoom.xos", "title": "Meeting chat", "do": "manage"}"#);
        assert!(manages(&[keep_chat.clone()], &zoom("Meeting chat", "AXStandardWindow")));
        assert!(!manages(&[keep_chat], &zoom("Meeting chat", "AXSystemDialog")));
        // A rule with no selector selects every window. A misspelt key is refused.
        assert!(!manages(&[rule(r#"{"do": "ignore"}"#)], &zoom("Zoom Meeting", "AXStandardWindow")));
        assert!(serde_json::from_str::<Rule>(r#"{"ap": "zoom.us", "do": "ignore"}"#).is_err());
        // The settings' notice names a rule, not the struct it is read through.
        assert!(serde_json::from_str::<Rule>("5").unwrap_err().to_string().contains("expected struct Rule at"));
        assert!(serde_json::from_str::<Rule>(r#"{"app": "zoom.us"}"#).is_err(), "a rule must say what to do");
    }

    #[test]
    fn an_identifier_tells_windows_of_one_title_apart() {
        // zoom-ax.md: the main meeting window and the share viewer share a title.
        let main = Facts { identifier: Some("zm.meeting.window.main"), ..zoom("Zoom Meeting", "AXStandardWindow") };
        let viewer = zoom("Zoom Meeting", "AXStandardWindow");
        let select = |text: &str| serde_json::from_str::<Selector>(text).unwrap();
        let by_id = select(r#"{"app": "us.zoom.xos", "identifier": "zm.meeting.window.main"}"#);
        assert!(by_id.selects(&main) && !by_id.selects(&viewer));
        // The same glob as every field: `*x*` contains, letters in either case.
        assert!(select(r#"{"identifier": "*MEETING*"}"#).selects(&main));
        // No identifier is empty text: `""` and `*` match it, a name does not.
        assert!(select(r#"{"identifier": ""}"#).selects(&viewer) && !select(r#"{"identifier": ""}"#).selects(&main));
        assert!(
            select(r#"{"identifier": "*"}"#).selects(&viewer) && !select(r#"{"identifier": "x"}"#).selects(&viewer)
        );
        // Title and identifier together: both must match. No field: every window.
        assert!(!select(r#"{"title": "Zoom Workplace", "identifier": "zm.*"}"#).selects(&main));
        assert!(select("{}").selects(&main) && select("{}").selects(&viewer));
        assert!(serde_json::from_str::<Selector>(r#"{"identifer": "x"}"#).is_err(), "a misspelt key is refused");
        // Written back without the fields not given.
        let written = serde_json::to_string(&by_id).unwrap();
        assert_eq!(written, r#"{"app":"us.zoom.xos","identifier":"zm.meeting.window.main"}"#);
    }

    /// A big pane beside it, its window with no known minimum.
    fn beside(size: [f64; 2]) -> Shape {
        Shape { size: Some(size), pane: Some([1600.0, 1000.0]), ..Shape::default() }
    }

    #[test]
    fn a_dialog_a_panel_a_modal_window_and_a_sheet_never_split() {
        let big = beside([1200.0, 900.0]);
        for subrole in ["AXDialog", "AXSystemDialog", "AXFloatingWindow", "AXSystemFloatingWindow"] {
            assert_eq!(place(&[], &zoom("Settings", subrole), &big), Place::Float, "{subrole}");
        }
        let standard = zoom("Zoom Workplace", "AXStandardWindow");
        assert_eq!(place(&[], &standard, &big), Place::Split);
        assert_eq!(place(&[], &standard, &Shape { modal: true, ..big }), Place::Float);
        // A sheet stays on its window, whatever a rule says.
        let split = serde_json::from_str::<Rule>(r#"{"app": "zoom.us", "do": "split"}"#).unwrap();
        assert_eq!(place(&[split], &standard, &Shape { sheet: true, ..big }), Place::Leave);
        // Other kinds are left alone, as before: Zoom's reactions, a built-in ignore.
        assert_eq!(place(&[], &zoom("Toast", "AXUnknown"), &big), Place::Leave);
        assert_eq!(place(&[], &zoom("zoom share toolbar window", "AXStandardWindow"), &big), Place::Leave);
    }

    #[test]
    fn a_standard_window_much_smaller_than_its_apps_pane_floats() {
        let standard = zoom("Zoom", "AXStandardWindow");
        // Under 600×400 both ways, with or without a pane.
        assert_eq!(place(&[], &standard, &Shape { size: Some([420.0, 300.0]), ..Shape::default() }), Place::Float);
        assert_eq!(place(&[], &standard, &beside([599.0, 399.0])), Place::Float);
        // Under half the pane (800×500) both ways.
        assert_eq!(place(&[], &standard, &beside([790.0, 490.0])), Place::Float);
        // One side as big as that is enough to split: a tall chat window, a wide bar.
        assert_eq!(place(&[], &standard, &beside([500.0, 800.0])), Place::Split);
        assert_eq!(place(&[], &standard, &beside([900.0, 300.0])), Place::Split);
        // Unknown size: as before.
        assert_eq!(place(&[], &standard, &Shape::default()), Place::Split);
    }

    #[test]
    fn a_split_never_squeezes_the_apps_window_below_what_it_kept() {
        let standard = zoom("Zoom", "AXStandardWindow");
        let shape = |min_width| Shape { min_width, ..beside([1000.0, 800.0]) };
        // The pane is 1600 wide: a split leaves 800.
        assert_eq!(place(&[], &standard, &shape(Some(900.0))), Place::Float);
        assert_eq!(place(&[], &standard, &shape(Some(700.0))), Place::Split);
        assert_eq!(place(&[], &standard, &shape(None)), Place::Split);
    }

    #[test]
    fn a_rule_wins_both_ways() {
        let rule = |text: &str| serde_json::from_str::<Rule>(text).unwrap();
        let split = rule(r#"{"app": "us.zoom.xos", "title": "Settings", "do": "split"}"#);
        let float = rule(r#"{"app": "us.zoom.xos", "title": "Zoom Workplace", "do": "float"}"#);
        let rules = [split, float];
        let tiny = beside([300.0, 200.0]);
        // A dialog, or a small window, forced into a split.
        assert_eq!(place(&rules, &zoom("Settings", "AXDialog"), &tiny), Place::Split);
        assert_eq!(place(&rules, &zoom("Settings", "AXStandardWindow"), &tiny), Place::Split);
        // Hosted, it counts as managed (the capture key, marks). A toast is never split.
        assert!(manages(&rules, &zoom("Settings", "AXDialog")));
        assert_eq!(place(&rules, &zoom("Settings", "AXUnknown"), &tiny), Place::Leave);
        assert!(!manages(&rules, &zoom("Settings", "AXUnknown")));
        // A big standard window kept free, and never hosted with its app either.
        let workplace = zoom("Zoom Workplace", "AXStandardWindow");
        assert_eq!(place(&rules, &workplace, &beside([1400.0, 900.0])), Place::Float);
        assert!(!manages(&rules, &workplace) && manages(&rules, &zoom("Settings", "AXStandardWindow")));
        // `ignore` leaves it alone; `manage` is the rules above.
        let ignore = rule(r#"{"app": "zoom.us", "do": "ignore"}"#);
        assert_eq!(place(&[ignore], &zoom("Settings", "AXDialog"), &tiny), Place::Leave);
        let manage = rule(r#"{"app": "zoom.us", "do": "manage"}"#);
        assert_eq!(place(std::slice::from_ref(&manage), &zoom("Settings", "AXDialog"), &tiny), Place::Float);
        assert_eq!(place(&[manage], &zoom("Meeting chat", "AXStandardWindow"), &beside([900.0, 700.0])), Place::Split);
    }

    #[test]
    fn a_floating_pop_up_is_centred_over_its_pane_unless_it_is_over_it() {
        let main = Frame::new(0.0, 0.0, 1920.0, 1080.0);
        let pane = Frame::new(100.0, 100.0, 1000.0, 800.0);
        // Put by macOS over its parked window, off screen: centred over the pane.
        let off = Frame::new(5000.0, 3000.0, 400.0, 200.0);
        assert_eq!(float_to(off, Some(pane), &[main], main), Some((400.0, 400.0)));
        // Already over the pane: left there.
        assert_eq!(float_to(Frame::new(150.0, 120.0, 400.0, 200.0), Some(pane), &[main], main), None);
        // Bigger than the pane: at its top left, not above or left of it.
        assert_eq!(float_to(Frame::new(1500.0, 0.0, 1200.0, 900.0), Some(pane), &[main], main), Some((100.0, 100.0)));
        // No pane drawn: moved only if out of reach.
        assert_eq!(float_to(Frame::new(10.0, 40.0, 400.0, 200.0), None, &[main], main), None);
        assert_eq!(float_to(off, None, &[main], main), Some((760.0, 440.0)));
        // A pane mostly off screen: the middle of the main screen instead.
        let away = Frame::new(1800.0, 1000.0, 1000.0, 800.0);
        assert_eq!(float_to(off, Some(away), &[main], main), Some((760.0, 440.0)));
    }

    #[test]
    fn the_width_a_window_refuses_is_what_it_kept_when_asked_for_less() {
        assert_eq!(refused_width(None, 900.0, 600.0), Some(900.0));
        // As wide as asked: it went to 700, so a refusal of 900 is past; one of 650 still holds.
        assert_eq!(refused_width(Some(900.0), 700.0, 700.0), None);
        assert_eq!(refused_width(Some(650.0), 700.0, 700.0), Some(650.0));
        assert_eq!(refused_width(None, 700.5, 700.0), None, "a point of rounding is no refusal");
    }

    #[test]
    fn a_rule_can_ignore_a_window_by_identifier() {
        let text = r#"{"app": "zoom.us", "identifier": "zm.meeting.window.main", "do": "ignore"}"#;
        let rule = serde_json::from_str::<Rule>(text).unwrap();
        let main = Facts { identifier: Some("zm.meeting.window.main"), ..zoom("Zoom Meeting", "AXStandardWindow") };
        assert!(!manages(std::slice::from_ref(&rule), &main));
        assert!(manages(&[rule], &zoom("Zoom Meeting", "AXStandardWindow")));
        // The fields sit beside `do`, not under a key of their own.
        assert!(serde_json::from_str::<Rule>(r#"{"select": {}, "do": "ignore"}"#).is_err());
    }
}
