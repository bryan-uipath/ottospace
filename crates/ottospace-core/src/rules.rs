//! Window rules: which windows of other apps ottospace leaves alone. A rule is a selector over a
//! few facts of a window, and what to do with a window it selects. The settings file holds the
//! user's rules; slots (`docs/ai/slots.md`) use the same selector.

use serde::{Deserialize, Serialize};

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
#[serde(deny_unknown_fields)]
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
    /// Never put it in a pane: not when its app is hosted, and not when it opens later.
    Ignore,
    /// Treat it as any other window. A user's rule can say this to undo a built-in `ignore`.
    Manage,
}

/// True if ottospace may put this window in a pane. The first rule that selects the window
/// decides, the user's rules before the built-in ones. With no rule: only a standard window,
/// which leaves out panels, pop-ups and an app's floating controls.
pub fn manages(user: &[Rule], window: &Facts) -> bool {
    let built_in = built_in();
    match user.iter().chain(&built_in).find(|rule| rule.select.selects(window)) {
        Some(rule) => rule.action == Action::Manage && window.subrole == "AXStandardWindow",
        None => window.subrole == "AXStandardWindow",
    }
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
