//! The notice bar: one app-level message between the top row and the rest, by level.

use serde::Serialize;

use crate::model::Origin;

/// Why a client's dismissal does nothing.
pub const USERS_ALONE: &str = "only the user dismisses a notice";

/// How pressing a notice is, least first: the bar's colour, and what may replace what.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Info,
    Warning,
    Error,
}

/// What a notice is about: each kind of app-level message, classified once (`level`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A step that failed: a shell or agent that did not start, a file not written.
    Failed,
    /// Why a move of the user's did nothing (`Effect::Notice`).
    Refused,
    /// A task, mark, workbook or layout that could not be saved.
    Unsaved,
    /// What the user's own step did (`Effect::Told`).
    Told,
    /// `settings.json` has a problem: the last good settings stay.
    SettingsProblem,
    /// `settings.json` names a retired key or flag: it applies, the key does nothing.
    SettingsRetired,
    /// `settings.json` changed a key that applies at the next start.
    SettingsRestart,
    /// Found at start: a config or font problem, a file of marks, workbooks or layout not read.
    Start,
    /// A global key (return, capture) that could not be registered.
    Keys,
    /// The user's editor gave its text back.
    Editor,
}

impl Kind {
    pub fn level(self) -> Level {
        match self {
            Kind::Failed | Kind::Unsaved | Kind::SettingsProblem | Kind::Start => Level::Error,
            Kind::Refused | Kind::SettingsRetired | Kind::Keys => Level::Warning,
            Kind::Told | Kind::SettingsRestart | Kind::Editor => Level::Info,
        }
    }

    /// The one action the bar offers beside the text.
    pub fn offer(self) -> Option<Offer> {
        match self {
            Kind::SettingsProblem | Kind::SettingsRetired => Some(Offer::OpenSettings),
            _ => None,
        }
    }

    /// It is held in a slot, emptied on a dismissal: said again, it shows again. The others (the
    /// settings' line, what was found at start) stay as they are until fixed, so the core
    /// remembers them as dismissed (`Model::notice_hidden`).
    pub fn held(self) -> bool {
        !matches!(self, Kind::SettingsProblem | Kind::SettingsRetired | Kind::SettingsRestart | Kind::Start)
    }
}

/// An action a notice offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Offer {
    OpenSettings,
}

impl Offer {
    pub fn label(self) -> &'static str {
        match self {
            Offer::OpenSettings => "Open settings",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub kind: Kind,
    pub text: String,
}

impl Notice {
    pub fn level(&self) -> Level {
        self.kind.level()
    }

    /// A level glyph, so the level is not said by colour alone.
    pub fn glyph(&self) -> &'static str {
        match self.level() {
            Level::Error => "✕",
            Level::Warning => "▲",
            Level::Info => "ⓘ",
        }
    }
}

/// The one notice the bar shows of `live` (newest and most pressing first, as the shell lists
/// them): the first of the highest level, so an error is not replaced by information. One the
/// user dismissed (`hidden`) is not shown again while it says the same.
pub fn shown(live: &[Notice], hidden: &[Notice]) -> Option<Notice> {
    let live = live.iter().filter(|notice| !hidden.contains(notice));
    // `max_by_key` gives the last of equals: reversed, the first.
    live.rev().max_by_key(|notice| notice.level()).cloned()
}

/// Whether a failure of `by`'s step may take the place of the one `shown_by` caused: a
/// client's never covers the user's.
pub fn replaces(by: Origin, shown_by: Option<Origin>) -> bool {
    !(by == Origin::Socket && shown_by == Some(Origin::User))
}

/// The lowest a raised live window's title bar may reach, from the window's top: under the top
/// row (`top_bar` high) and the notice bar, as much of it as shows (`shown`). The bar is never covered.
pub fn top_limit(top_bar: f32, shown: f32) -> f32 {
    top_bar + shown
}

/// The bar's height when open, in points.
pub const HEIGHT: f32 = 28.;
/// How long it takes to open or close, in milliseconds.
pub const OPEN_MS: f32 = 180.;

/// The bar's height `elapsed` ms after it began to move from `from` towards open or closed:
/// eased out. With reduced motion it is there at once.
pub fn height(from: f32, open: bool, elapsed: f32, reduced: bool) -> f32 {
    let to = if open { HEIGHT } else { 0. };
    let t = if reduced { 1. } else { (elapsed / OPEN_MS).clamp(0., 1.) };
    let eased = 1. - (1. - t).powi(3);
    from + (to - from) * eased
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_of_message_has_one_level_and_offer() {
        use Kind::*;
        let table = [
            (Failed, Level::Error, None, true),
            (Refused, Level::Warning, None, true),
            (Unsaved, Level::Error, None, true),
            (Told, Level::Info, None, true),
            (SettingsProblem, Level::Error, Some(Offer::OpenSettings), false),
            (SettingsRetired, Level::Warning, Some(Offer::OpenSettings), false),
            (SettingsRestart, Level::Info, None, false),
            (Start, Level::Error, None, false),
            (Keys, Level::Warning, None, true),
            (Editor, Level::Info, None, true),
        ];
        for (kind, level, offer, held) in table {
            assert_eq!((kind.level(), kind.offer(), kind.held()), (level, offer, held), "{kind:?}");
        }
    }

    #[test]
    fn the_bar_shows_the_first_of_the_highest_level_and_nothing_dismissed() {
        let notice = |kind, text: &str| Notice { kind, text: text.into() };
        let (told, refused) = (notice(Kind::Told, "Made app-fix"), notice(Kind::Refused, "No repository in /tmp"));
        let (failed, settings) = (notice(Kind::Failed, "terminal: no shell"), notice(Kind::SettingsProblem, "1:2"));
        assert_eq!(shown(&[], &[]), None);
        // An error is not replaced by information or a warning, wherever they stand.
        assert_eq!(shown(&[told.clone(), refused.clone(), settings.clone()], &[]), Some(settings.clone()));
        // Of one level, the first: the newer.
        assert_eq!(shown(&[failed.clone(), settings.clone()], &[]), Some(failed.clone()));
        assert_eq!(shown(&[told.clone(), refused.clone()], &[]), Some(refused.clone()));
        // Dismissed: the next takes its place; said differently, it shows again.
        assert_eq!(
            shown(&[failed, settings.clone(), told.clone()], std::slice::from_ref(&settings)).unwrap().kind,
            Kind::Failed
        );
        assert_eq!(shown(&[settings.clone(), told.clone()], std::slice::from_ref(&settings)), Some(told));
        assert_eq!(shown(&[notice(Kind::SettingsProblem, "3:4")], &[settings]).unwrap().text, "3:4");
    }

    #[test]
    fn a_clients_failure_never_covers_the_users() {
        use Origin::*;
        for (by, over, may) in [
            (Socket, Some(User), false),
            (Socket, Some(Socket), true),
            (Socket, Some(System), true),
            (Socket, None, true),
            (User, Some(User), true),
            (User, Some(Socket), true),
            (System, Some(User), true),
        ] {
            assert_eq!(replaces(by, over), may, "{by:?} over {over:?}");
        }
    }

    #[test]
    fn a_live_windows_title_bar_stays_under_the_notice_bar() {
        use crate::titlebar::crop;
        let (top_bar, title) = (40., 28.);
        assert_eq!((top_limit(top_bar, 0.), top_limit(top_bar, 10.), top_limit(top_bar, HEIGHT)), (40., 50., 68.));
        // A pane whose header is right under the top row: its window's title bar covers the header.
        assert_eq!(crop(title, top_bar + title, top_limit(top_bar, 0.)), title);
        // The bar opens and the pane has not moved yet: the title bar would lie on the notice. No crop.
        assert_eq!(crop(title, top_bar + title, top_limit(top_bar, HEIGHT)), 0.);
        assert_eq!(crop(title, top_bar + title + 10., top_limit(top_bar, 11.)), 0.);
        // Moved down with it, half way and at rest: the crop is back, under the bar.
        assert_eq!(crop(title, top_bar + 10. + title, top_limit(top_bar, 10.)), title);
        assert_eq!(crop(title, top_bar + HEIGHT + title, top_limit(top_bar, HEIGHT)), title);
    }

    #[test]
    fn the_bar_eases_open_and_closed_and_is_there_at_once_with_reduced_motion() {
        assert_eq!(height(0., true, 0., false), 0.);
        assert_eq!(height(0., true, OPEN_MS, false), HEIGHT);
        assert_eq!(height(0., true, 9_999., false), HEIGHT);
        let half = height(0., true, OPEN_MS / 2., false);
        assert!(half > HEIGHT / 2. && half < HEIGHT, "eased out: {half}");
        assert_eq!(height(HEIGHT, false, OPEN_MS, false), 0.);
        // From where it was, if it turns back half way.
        assert_eq!(height(10., false, 0., false), 10.);
        assert_eq!((height(0., true, 0., true), height(HEIGHT, false, 0., true)), (HEIGHT, 0.));
    }
}
