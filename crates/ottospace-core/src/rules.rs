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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
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
/// A user's `split` or `float` never moves a window a built-in rule protects: only `manage` does.
fn first(user: &[Rule], window: &Facts) -> Option<Action> {
    first_of(user, window).map(|(action, _)| action)
}

/// `first`, and which rule said it.
fn first_of(user: &[Rule], window: &Facts) -> Option<(Action, Source)> {
    let of =
        |rules: &[Rule]| rules.iter().position(|rule| rule.select.selects(window)).map(|at| (rules[at].action, at));
    match (of(user), of(&built_in())) {
        (Some((Action::Split | Action::Float, _)), Some((Action::Ignore, at))) => {
            Some((Action::Ignore, Source::BuiltIn(at)))
        }
        (Some((action, at)), _) => Some((action, Source::User(at))),
        (None, built) => built.map(|(action, at)| (action, Source::BuiltIn(at))),
    }
}

/// Which rule decided: the user's `windows` rule at this index (from 0), or a built-in one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    User(usize),
    BuiltIn(usize),
}

impl Source {
    /// `windows[2]`, `built-in rule 0`: for the log and the chip.
    pub fn label(self) -> String {
        match self {
            Source::User(at) => format!("windows[{at}]"),
            Source::BuiltIn(at) => format!("built-in rule {at}"),
        }
    }
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
    /// The least width that pane's window kept when asked for less: it refuses narrower. For the
    /// log: a split would squeeze it.
    pub min_width: Option<f64>,
    /// No pane of its app is left (it was given back): a pop-up is not touched, nor offered.
    pub alone: bool,
}

/// Where a new window of a hosted app goes. None goes in a pane by itself: only a `split` rule,
/// or the user taking the offer (`Float { offer: true }`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// A pane of its own, beside its app's (`Action::Adopt`): a `split` rule said so.
    Split,
    /// A free window at its own size, over its app's pane (`float_to`). `offer`: its app's pane
    /// offers to put it in a pane (`Action::WindowOffered`).
    Float { offer: bool },
    /// Left as it is: ignored, a sheet, or not a kind of window ottospace hosts.
    Leave,
}

/// Why a window went where it did: said in the log and, for an offer, in its chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// A sheet, or a window whose parent is another: macOS keeps it there.
    Sheet,
    /// A toast, a popover: not a kind of window ottospace places.
    Kind,
    /// A rule said so (`Decision::rule`).
    Rule,
    /// Under 100×100: an app's helper.
    Tiny,
    /// A dialog or a panel: floats, and is offered (Zoom's meeting window can be an
    /// `AXFloatingWindow`).
    Dialog,
    /// It blocks its app until answered: floats, and is offered.
    Modal,
    /// A standard window no slot selects: offered.
    NoSlot,
    /// Its app has no pane left: nothing to float it over or offer it in.
    NoPane,
}

impl Why {
    /// Plain words: `no slot for this window`.
    pub fn words(self) -> &'static str {
        match self {
            Why::Sheet => "a sheet or child window: it stays on its parent",
            Why::Kind => "not a window ottospace places",
            Why::Rule => "a windows rule",
            Why::Tiny => "under 100×100: an app's helper",
            Why::Dialog => "a dialog or panel, so it floats",
            Why::Modal => "modal, so it floats",
            Why::NoSlot => "no slot for this window",
            Why::NoPane => "its app has no pane",
        }
    }

    /// A word for the log: `no_slot`.
    pub fn name(self) -> &'static str {
        match self {
            Why::Sheet => "sheet",
            Why::Kind => "kind",
            Why::Rule => "rule",
            Why::Tiny => "tiny",
            Why::Dialog => "dialog",
            Why::Modal => "modal",
            Why::NoSlot => "no_slot",
            Why::NoPane => "no_pane",
        }
    }
}

/// Where a new window goes, why, and the rule that decided or let it through (`manage`), if one did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    pub place: Place,
    pub why: Why,
    pub rule: Option<(Source, Action)>,
}

impl Decision {
    /// Why, in plain words, with the rule that decided or let it through: `no slot for this
    /// window`, `a windows rule (windows[1]: float)`.
    pub fn reason(&self) -> String {
        match self.rule {
            Some((source, action)) => format!("{} ({}: {})", self.why.words(), source.label(), action.name()),
            None => self.why.words().to_string(),
        }
    }
}

impl Action {
    /// As written after `do`: `ignore`.
    pub fn name(self) -> &'static str {
        match self {
            Action::Ignore => "ignore",
            Action::Manage => "manage",
            Action::Split => "split",
            Action::Float => "float",
        }
    }
}

/// Subroles of dialogs and panels: never split, unless a rule says so.
const POP_UP: [&str; 4] = ["AXDialog", "AXSystemDialog", "AXFloatingWindow", "AXSystemFloatingWindow"];

/// Under this, both ways, a window is an app's helper, not a pop-up: left where the app keeps it.
/// A GPUI app has a 66×20 one off screen.
const TINY: [f64; 2] = [100.0, 100.0];

/// Where a new window of a hosted app goes (`decide`, without the why).
#[cfg(test)]
fn place(user: &[Rule], window: &Facts, shape: &Shape) -> Place {
    decide(user, window, shape).place
}

/// Where a new window of a hosted app goes, and why. A sheet or child window stays on its parent.
/// Then the first rule that selects it decides; else a window under 100×100 is left alone (an
/// app's helper), and any other floats with an offer, a dialog, a panel or a modal one too: no
/// size or subrole decides a split, only the user (or a `split` rule).
pub fn decide(user: &[Rule], window: &Facts, shape: &Shape) -> Decision {
    let decided = judge(user, window, shape);
    // A pop-up of an app with no pane left is not touched: a `split` rule still opens one.
    match decided.place {
        Place::Float { .. } if shape.alone => Decision { place: Place::Leave, why: Why::NoPane, ..decided },
        _ => decided,
    }
}

fn judge(user: &[Rule], window: &Facts, shape: &Shape) -> Decision {
    let decided = |place, why, rule| Decision { place, why, rule };
    if shape.sheet {
        return decided(Place::Leave, Why::Sheet, None);
    }
    if !judged(window.subrole) {
        return decided(Place::Leave, Why::Kind, None);
    }
    let rule = first_of(user, window).map(|(action, source)| (source, action));
    match rule.map(|(_, action)| action) {
        Some(Action::Ignore) => return decided(Place::Leave, Why::Rule, rule),
        Some(Action::Split) => return decided(Place::Split, Why::Rule, rule),
        Some(Action::Float) => return decided(Place::Float { offer: false }, Why::Rule, rule),
        Some(Action::Manage) | None => {}
    }
    let under = |size: [f64; 2], limit: [f64; 2]| size[0] < limit[0] && size[1] < limit[1];
    if shape.size.is_some_and(|size| under(size, TINY)) {
        return decided(Place::Leave, Why::Tiny, rule);
    }
    if window.subrole != STANDARD {
        return decided(Place::Float { offer: true }, Why::Dialog, rule);
    }
    if shape.modal {
        return decided(Place::Float { offer: true }, Why::Modal, rule);
    }
    decided(Place::Float { offer: true }, Why::NoSlot, rule)
}

/// True if the user's own pick (⌘E's list of an app's windows) may put this window in a pane:
/// any but a sheet or child window (macOS keeps it on its parent), an app's tiny helper, or one a
/// rule ignores. A pop-up's subrole or a `float` rule does not stop it: the user asked for it.
pub fn pickable(user: &[Rule], window: &Facts, shape: &Shape) -> bool {
    let tiny = shape.size.is_some_and(|size| size[0] < TINY[0] && size[1] < TINY[1]);
    !shape.sheet && !tiny && first(user, window) != Some(Action::Ignore)
}

/// Where a floating pop-up could be seen, as the shell read it. Global frames, points.
#[derive(Clone, Copy, Debug)]
pub struct Sight<'a> {
    /// Each screen's visible frame: below the menu bar, beside the Dock.
    pub screens: &'a [Frame],
    /// The visible frame of the screen with the menu bar.
    pub main: Frame,
    /// ottospace's own window.
    pub ottospace: Option<Frame>,
    /// None of the app's panes is live: ottospace is in front of every window of the app.
    pub parked: bool,
    /// Where parked windows sit (`hosted::parking_corners`), each a top left.
    pub spots: &'a [(f64, f64)],
}

/// True when the user can get at the pop-up where it is: `restore::GRIP` of its top on one screen
/// and, while its app is parked, beside ottospace's window, unless it is over its app's `pane`
/// (entering the pane brings it up). One over a parking spot is not: macOS put it by its parked
/// parent, mostly off the screen's corner.
pub fn reachable(window: Frame, pane: Option<Frame>, sight: &Sight) -> bool {
    let at = |&(x, y): &(f64, f64)| {
        (window.x..window.x + window.w).contains(&x) && (window.y..window.y + window.h).contains(&y)
    };
    if sight.spots.iter().any(at) {
        return false;
    }
    let (x, y) = (window.x + window.w / 2.0, window.y + window.h / 2.0);
    let over = pane.is_some_and(|p| (p.x..=p.x + p.w).contains(&x) && (p.y..=p.y + p.h).contains(&y));
    let cover = sight.ottospace.filter(|_| sight.parked && !over);
    crate::restore::reachable_beside(window, sight.screens, cover)
}

/// Where a floating pop-up's top left goes (global frames), or `None` to leave it: one the user
/// can reach (`reachable`) is left where it is. Any other goes at its own size to the middle of
/// its app's `pane` (no pane drawn: of the main screen), moved fully onto the visible frame of
/// the pane's screen; a pop-up bigger than that screen at its top left.
pub fn float_to(window: Frame, pane: Option<Frame>, sight: &Sight) -> Option<(f64, f64)> {
    if reachable(window, pane, sight) {
        return None;
    }
    let holds = |s: &&Frame, p: &Frame| {
        let (x, y) = (p.x + p.w / 2.0, p.y + p.h / 2.0);
        (s.x..s.x + s.w).contains(&x) && (s.y..s.y + s.h).contains(&y)
    };
    // A pane whose middle is on no screen: the main screen's middle instead.
    let on = pane.and_then(|p| sight.screens.iter().find(|s| holds(s, &p)).map(|s| (*s, p)));
    let (screen, around) = on.unwrap_or((sight.main, sight.main));
    let middle = crate::restore::centred(window, around);
    // At its own size: `centred` shrinks it to the pane, so one bigger starts at the pane's top left.
    let clamp = |at: f64, size: f64, low: f64, span: f64| at.min(low + span - size).max(low);
    let to = Frame {
        x: clamp(middle.x, window.w, screen.x, screen.w),
        y: clamp(middle.y, window.h, screen.y, screen.h),
        ..window
    };
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
    fn the_users_pick_may_host_a_floated_pop_up_but_never_a_sheet_a_helper_or_an_ignored_window() {
        // Zoom in a meeting: its main window, the share viewer, and the meeting window, which
        // `place` floated (a floating window's subrole).
        let main = zoom("Zoom Workplace", STANDARD);
        let share = zoom("Zoom Meeting", STANDARD);
        let meeting = Facts { identifier: Some("zm.meeting.window.main"), ..zoom("Zoom Meeting", "AXFloatingWindow") };
        let shape = Shape { size: Some([1200.0, 800.0]), ..Shape::default() };
        assert!(matches!(place(&[], &meeting, &shape), Place::Float { .. }));
        for window in [main, share, meeting] {
            assert!(pickable(&[], &window, &shape), "{window:?}");
        }
        // A user's `float` rule is overridden by their own pick; `ignore`, built in or theirs, is not.
        let float = Rule {
            select: Selector { title: Some("Zoom Meeting".into()), ..Selector::default() },
            action: Action::Float,
        };
        assert!(pickable(&[float], &meeting, &shape));
        let ignore = Rule {
            select: Selector { title: Some("Zoom Meeting".into()), ..Selector::default() },
            action: Action::Ignore,
        };
        assert!(!pickable(&[ignore], &meeting, &shape));
        assert!(!pickable(&[], &zoom("Zoom Share Toolbar Window", "AXFloatingWindow"), &shape));
        // A sheet stays on its window; a helper under 100×100 is no window to show.
        assert!(!pickable(&[], &main, &Shape { sheet: true, ..shape }));
        assert!(!pickable(&[], &main, &Shape { size: Some([66.0, 20.0]), ..shape }));
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

    const OFFER: Place = Place::Float { offer: true };
    const FLOAT: Place = Place::Float { offer: false };

    #[test]
    fn a_dialog_a_panel_a_modal_window_and_a_sheet_never_split() {
        // A dialog, a panel, a modal window float, and are offered: the user may put one in a pane.
        let big = beside([1200.0, 900.0]);
        for subrole in ["AXDialog", "AXSystemDialog", "AXFloatingWindow", "AXSystemFloatingWindow"] {
            let decided = decide(&[], &zoom("Settings", subrole), &big);
            assert_eq!((decided.place, decided.why), (OFFER, Why::Dialog), "{subrole}");
        }
        let standard = zoom("Zoom Workplace", "AXStandardWindow");
        assert_eq!(decide(&[], &standard, &Shape { modal: true, ..big }).why, Why::Modal);
        assert_eq!(place(&[], &standard, &Shape { modal: true, ..big }), OFFER);
        // A sheet stays on its window, whatever a rule says.
        let split = serde_json::from_str::<Rule>(r#"{"app": "zoom.us", "do": "split"}"#).unwrap();
        assert_eq!(place(&[split], &standard, &Shape { sheet: true, ..big }), Place::Leave);
        // Other kinds are left alone, as before: Zoom's reactions, a built-in ignore.
        assert_eq!(place(&[], &zoom("Toast", "AXUnknown"), &big), Place::Leave);
        assert_eq!(place(&[], &zoom("zoom share toolbar window", "AXStandardWindow"), &big), Place::Leave);
    }

    #[test]
    fn no_size_splits_a_standard_window_it_floats_with_an_offer() {
        let standard = zoom("Zoom Meeting", "AXStandardWindow");
        // Big or small, squeezing its app's window or not: offered, never split by itself.
        for shape in [
            beside([1200.0, 900.0]),
            beside([420.0, 300.0]),
            Shape { min_width: Some(900.0), ..beside([1000.0, 800.0]) },
            Shape::default(),
        ] {
            let decided = decide(&[], &standard, &shape);
            assert_eq!((decided.place, decided.why, decided.rule), (OFFER, Why::NoSlot, None), "{shape:?}");
        }
        assert_eq!(Why::NoSlot.words(), "no slot for this window");
        // A helper under 100×100 is left alone, a dialog too; a rule still decides.
        assert_eq!(place(&[], &standard, &beside([66.0, 20.0])), Place::Leave);
        assert_eq!(place(&[], &zoom("x", "AXFloatingWindow"), &beside([66.0, 20.0])), Place::Leave);
        let float = serde_json::from_str::<Rule>(r#"{"app": "zoom.us", "do": "float"}"#).unwrap();
        assert_eq!(place(&[float], &standard, &beside([66.0, 20.0])), FLOAT);
    }

    /// The owner's call: Zoom's main window, the share viewer (same title, no id) and the meeting
    /// window as a panel. None splits; each is offered, the panel too, and its reason says why it floats.
    #[test]
    fn zooms_three_meeting_windows_are_each_offered_none_split() {
        let main = Facts { identifier: Some("zm.meeting.window.main"), ..zoom("Zoom Meeting", "AXStandardWindow") };
        let viewer = zoom("Zoom Meeting", "AXStandardWindow");
        let meeting = zoom("Zoom Meeting", "AXFloatingWindow");
        let shape = beside([1083.0, 935.0]);
        let reasons: Vec<(Place, String)> = [main, viewer, meeting]
            .iter()
            .map(|window| decide(&[], window, &shape))
            .map(|decided| (decided.place, decided.reason()))
            .collect();
        let offer = |why: &str| (OFFER, why.to_string());
        let said = [
            offer("no slot for this window"),
            offer("no slot for this window"),
            offer("a dialog or panel, so it floats"),
        ];
        assert_eq!(reasons, said);
        // An app given back (no pane left) is not touched, said so.
        let alone = decide(&[], &main, &Shape { alone: true, ..shape });
        assert_eq!((alone.place, alone.why), (Place::Leave, Why::NoPane));
        // Only sheets, child windows and helpers get no offer.
        assert_eq!(place(&[], &meeting, &Shape { sheet: true, ..shape }), Place::Leave);
        assert_eq!(place(&[], &meeting, &beside([56.0, 56.0])), Place::Leave);
    }

    #[test]
    fn the_rule_that_decided_is_named() {
        let rule = |text: &str| serde_json::from_str::<Rule>(text).unwrap();
        let rules = [rule(r#"{"title": "Lunch", "do": "ignore"}"#), rule(r#"{"app": "zoom.us", "do": "manage"}"#)];
        let decided = decide(&rules, &zoom("Zoom Meeting", "AXStandardWindow"), &Shape::default());
        assert_eq!((decided.place, decided.rule), (OFFER, Some((Source::User(1), Action::Manage))));
        assert_eq!(Source::User(1).label(), "windows[1]");
        assert_eq!(decided.reason(), "no slot for this window (windows[1]: manage)");
        let decided = decide(&[], &zoom("Reactions", "AXStandardWindow"), &Shape::default());
        assert_eq!((decided.why, decided.rule), (Why::Rule, Some((Source::BuiltIn(3), Action::Ignore))));
        // A user's split under a built-in ignore: the built-in one is named.
        let split = [rule(r#"{"app": "zoom.us", "do": "split"}"#)];
        let decided = decide(&split, &zoom("Meeting chat", "AXStandardWindow"), &Shape::default());
        assert_eq!((decided.place, decided.rule), (Place::Leave, Some((Source::BuiltIn(4), Action::Ignore))));
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
        // A big standard window kept free, with no offer, and never hosted with its app either.
        let workplace = zoom("Zoom Workplace", "AXStandardWindow");
        assert_eq!(place(&rules, &workplace, &beside([1400.0, 900.0])), FLOAT);
        assert!(!manages(&rules, &workplace) && manages(&rules, &zoom("Settings", "AXStandardWindow")));
        // `ignore` leaves it alone; `manage` is the rules above.
        let ignore = rule(r#"{"app": "zoom.us", "do": "ignore"}"#);
        assert_eq!(place(&[ignore], &zoom("Settings", "AXDialog"), &tiny), Place::Leave);
        let manage = rule(r#"{"app": "zoom.us", "do": "manage"}"#);
        assert_eq!(place(std::slice::from_ref(&manage), &zoom("Settings", "AXDialog"), &tiny), OFFER);
        assert_eq!(place(&[manage], &zoom("Meeting chat", "AXStandardWindow"), &beside([900.0, 700.0])), OFFER);
    }

    #[test]
    fn a_built_in_rule_beats_a_users_split_or_float() {
        let rule = |text: &str| serde_json::from_str::<Rule>(text).unwrap();
        let (split, float) =
            (rule(r#"{"app": "zoom.us", "do": "split"}"#), rule(r#"{"app": "zoom.us", "do": "float"}"#));
        let big = beside([1200.0, 900.0]);
        for title in ["zoom share toolbar window", "Meeting chat", "Reactions"] {
            let window = zoom(title, "AXStandardWindow");
            for user in [&split, &float] {
                assert_eq!(place(std::slice::from_ref(user), &window, &big), Place::Leave, "{title}");
                assert!(!manages(std::slice::from_ref(user), &window), "{title}");
            }
        }
        // Other windows of the app still follow the user's rule.
        assert_eq!(place(&[split], &zoom("Settings", "AXDialog"), &big), Place::Split);
        assert_eq!(place(&[float], &zoom("Zoom Workplace", "AXStandardWindow"), &big), FLOAT);
        // Only a `manage` rule undoes the built-in one: then offered, as any standard window.
        let manage = rule(r#"{"app": "zoom.us", "title": "Meeting chat", "do": "manage"}"#);
        assert_eq!(place(&[manage], &zoom("Meeting chat", "AXStandardWindow"), &big), OFFER);
    }

    /// One screen; ottospace's window over most of it, its app's pane in it.
    const MAIN: Frame = Frame { x: 0.0, y: 25.0, w: 1920.0, h: 1055.0 };
    const OTTO: Frame = Frame { x: 0.0, y: 25.0, w: 1400.0, h: 1000.0 };
    const PANE: Frame = Frame { x: 100.0, y: 100.0, w: 1000.0, h: 800.0 };

    fn sight(screens: &[Frame], parked: bool) -> Sight<'_> {
        Sight { screens, main: screens[0], ottospace: Some(OTTO), parked, spots: &[] }
    }

    #[test]
    fn a_pop_up_the_user_can_reach_is_left_where_it_is() {
        let (parked, live) = (sight(&[MAIN], true), sight(&[MAIN], false));
        // Beside ottospace's window, on screen: left there, parked or live.
        let beside = Frame::new(1450.0, 200.0, 400.0, 300.0);
        assert_eq!(float_to(beside, Some(PANE), &parked), None);
        // Its title bar on screen, the rest off the bottom: still in reach.
        let low = Frame::new(1450.0, 900.0, 400.0, 600.0);
        assert_eq!(float_to(low, Some(PANE), &parked), None);
        // Over its pane: entering the pane brings it up.
        assert_eq!(float_to(Frame::new(150.0, 120.0, 400.0, 200.0), Some(PANE), &parked), None);
        // Over ottospace while its app is live: the app is in front of ottospace.
        let under = Frame::new(1200.0, 300.0, 400.0, 300.0);
        assert_eq!(float_to(under, Some(PANE), &live), None);
        // Part of its top under ottospace: 200 of it beside, enough to take hold of.
        assert!(reachable(under, Some(PANE), &parked));
    }

    #[test]
    fn a_pop_up_the_user_cannot_reach_lands_over_its_pane_on_screen() {
        let screens = [MAIN];
        let parked = sight(&screens, true);
        // By its parked window, off screen: centred over the pane.
        let off = Frame::new(5000.0, 3000.0, 400.0, 200.0);
        assert_eq!(float_to(off, Some(PANE), &parked), Some((400.0, 400.0)));
        // Under ottospace, not over its pane, while its app is parked: behind ottospace.
        let hidden = Frame::new(1250.0, 300.0, 140.0, 100.0);
        assert!(!reachable(hidden, Some(PANE), &parked) && reachable(hidden, Some(PANE), &sight(&screens, false)));
        assert_eq!(float_to(hidden, Some(PANE), &parked), Some((530.0, 450.0)));
        // Its title bar's middle under ottospace's bottom edge: hidden; below it: in sight.
        assert!(!reachable(Frame { y: 1010.0, ..hidden }, Some(PANE), &parked));
        assert!(reachable(Frame { y: 1020.0, ..hidden }, Some(PANE), &parked));
        // Its title bar above the screen's top: out of reach though most of it shows.
        let high = Frame::new(1450.0, -100.0, 400.0, 600.0);
        assert_eq!(float_to(high, Some(PANE), &parked), Some((400.0, 200.0)));
        // Bigger than the pane: at its top left, its own size, never off the screen's bottom.
        assert_eq!(float_to(Frame::new(5000.0, 0.0, 1200.0, 1000.0), Some(PANE), &parked), Some((100.0, 80.0)));
        // Bigger than the screen: at the screen's top left.
        assert_eq!(float_to(Frame::new(5000.0, 0.0, 2500.0, 1200.0), Some(PANE), &parked), Some((0.0, 25.0)));
        // No pane drawn: the middle of the main screen.
        assert_eq!(float_to(off, None, &parked), Some((760.0, 452.5)));
        // Centred on its parent at a parking spot, 150×200 of it in the screen's corner: over the pane.
        let spot = (1919.0, 1079.0);
        let corner = Frame::new(spot.0 - 150.0, spot.1 - 200.0, 700.0, 600.0);
        let spots = [spot];
        let sight = Sight { ottospace: None, spots: &spots, ..parked };
        assert!(reachable(corner, Some(PANE), &Sight { spots: &[], ..sight }));
        assert_eq!(float_to(corner, Some(PANE), &sight), Some((250.0, 200.0)));
    }

    #[test]
    fn an_unreachable_pop_up_goes_whole_onto_its_panes_screen() {
        // Two screens side by side; the pane at the bottom right of the second.
        let right = Frame::new(1920.0, 0.0, 1440.0, 900.0);
        let screens = [MAIN, right];
        let sight = Sight { ottospace: None, ..sight(&screens, true) };
        let pane = Frame::new(2900.0, 500.0, 400.0, 380.0);
        // Centred over the pane it would hang off the screen: moved up and left onto it, whole.
        let far = Frame::new(9000.0, 9000.0, 700.0, 450.0);
        assert_eq!(float_to(far, Some(pane), &sight), Some((2660.0, 450.0)));
        // A pane whose middle is on no screen: the main screen's middle.
        let away = Frame::new(3200.0, 800.0, 1000.0, 800.0);
        assert_eq!(float_to(Frame::new(9000.0, 9000.0, 400.0, 200.0), Some(away), &sight), Some((760.0, 452.5)));
        // Spanning both screens, its top on both: in reach, left there.
        assert_eq!(float_to(Frame::new(1800.0, 100.0, 400.0, 300.0), Some(pane), &sight), None);
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
