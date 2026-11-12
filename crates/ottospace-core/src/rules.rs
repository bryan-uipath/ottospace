//! Window rules: which windows of other apps ottospace leaves alone. A rule is a selector over a
//! few facts of a window, and what to do with a window it selects. The settings file holds the
//! user's rules; the scripting SDK will use the same selectors.

use serde::Deserialize;

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
}

/// `{ "app": "us.zoom.xos", "title": "zoom share*", "do": "ignore" }`. Every field given must
/// match. `*` stands for any run of characters; letters match whatever their case.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// The app's name or its bundle id.
    #[serde(default)]
    pub app: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub subrole: Option<String>,
    #[serde(rename = "do")]
    pub action: Action,
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

impl Rule {
    pub fn selects(&self, window: &Facts) -> bool {
        let app = |pattern: &String| glob(pattern, window.app) || window.bundle.is_some_and(|b| glob(pattern, b));
        self.app.as_ref().is_none_or(app)
            && self.title.as_ref().is_none_or(|pattern| glob(pattern, window.title))
            && self.subrole.as_ref().is_none_or(|pattern| glob(pattern, window.subrole))
    }
}

/// True if ottospace may put this window in a pane. The first rule that selects the window
/// decides, the user's rules before the built-in ones. With no rule: only a standard window,
/// which leaves out panels, pop-ups and an app's floating controls.
pub fn manages(user: &[Rule], window: &Facts) -> bool {
    let built_in = built_in();
    match user.iter().chain(&built_in).find(|rule| rule.selects(window)) {
        Some(rule) => rule.action == Action::Manage && window.subrole == "AXStandardWindow",
        None => window.subrole == "AXStandardWindow",
    }
}

/// Rules that ship with ottospace, for windows known to break when moved. Measured: see
/// `docs/ai/1-initial-brainstorm/zoom-ax.md`.
pub fn built_in() -> Vec<Rule> {
    let zoom = |title: &str| Rule {
        app: Some("us.zoom.xos".into()),
        title: Some(title.into()),
        subrole: None,
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
        Facts { app: "zoom.us", bundle: Some("us.zoom.xos"), title, subrole }
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
        let other =
            Facts { app: "Safari", bundle: Some("com.apple.Safari"), title: "Start Page", subrole: "AXStandardWindow" };
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
}
