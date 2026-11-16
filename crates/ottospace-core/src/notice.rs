//! The notice bar: one app-level message at the window's foot, by level.

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

/// What a notice is about: each kind of app-level message, classified once (`level`, `about`).
///
/// An answer to its sender is on the bar only when the user's own step said it: a client's or a
/// timer's is in the reply's `notice`, and moves nothing. The app's state shows whoever's step
/// found it (`on_bar`).
///
/// | Kind | Level | About | Another sender's on the bar |
/// |---|---|---|---|
/// | `failed` | error | the app: a step of the user's or a timer's that failed; of a client's, a shell or agent not started, a file not written, a window of the user's it moved | yes, never over the user's (`replaces`) |
/// | `refused` | warning | the sender: why its step did nothing | no |
/// | `unsaved` | error | the app | yes |
/// | `told` | info | the sender: what its step did | no |
/// | `settings_problem`, `settings_retired`, `settings_restart` | error, warning, info | the app | yes |
/// | `start` | error | the app | yes |
/// | `keys` | warning | the app | yes |
/// | `editor` | info | the app | yes |
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A step that failed: a shell or agent that did not start, a file not written.
    Failed,
    /// Why a step did nothing: the core's reason (`Effect::Notice`), or the shell's to a client.
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

    /// Whom it is for: an answer is its sender's, the app's state is everyone's.
    pub fn about(self) -> About {
        match self {
            Kind::Refused | Kind::Told => About::Sender,
            _ => About::App,
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

/// Whom a notice is for (`Kind::about`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum About {
    /// An answer to the one whose step it was: why it did nothing, or what it did.
    Sender,
    /// The app's state: a file not read or not saved, a shell or agent that did not start.
    App,
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

/// Whether what a step of `by` said shows on the user's bar. An answer shows to the user alone:
/// a client's refusal is in its reply's `notice`, and moves nothing of the user's.
pub fn on_bar(kind: Kind, by: Origin) -> bool {
    by == Origin::User || kind.about() == About::App
}

/// The kind of a step's line. `refused`: the core's own reason (`Effect::Notice`). `broke`: the
/// app's failure (a shell or agent not started, a file not written). A client's is otherwise
/// its answer: the shell's reason why its request did nothing.
pub fn said(refused: bool, broke: bool, by: Origin) -> Kind {
    if refused || (by == Origin::Socket && !broke) { Kind::Refused } else { Kind::Failed }
}

/// The bar's slot for a step's line: what it says, and whose step said it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Held {
    pub said: Option<Notice>,
    pub by: Option<Origin>,
}

impl Held {
    /// The slot after a step of `by` left `now` as its line (`None`: it cleared the line). An
    /// answer to another sender is not taken, nor a client's line over the user's; a client
    /// clears only a client's line, so its success closes no bar it did not open.
    pub fn after(&self, now: Option<Notice>, by: Origin) -> Held {
        let held_by = self.by.filter(|_| self.said.is_some());
        let takes = match &now {
            Some(notice) => on_bar(notice.kind, by) && replaces(by, held_by),
            None => by != Origin::Socket || held_by.is_none_or(|held_by| held_by == by),
        };
        if takes { Held { said: now, by: Some(by) } } else { self.clone() }
    }
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
    fn an_answer_shows_to_the_user_alone_and_the_apps_state_to_everyone() {
        use Kind::*;
        use Origin::*;
        let all = [Failed, Refused, Unsaved, Told, SettingsProblem, SettingsRetired, SettingsRestart, Start, Keys];
        for kind in all.into_iter().chain([Editor]) {
            let answer = matches!(kind, Refused | Told);
            assert_eq!(kind.about() == About::Sender, answer, "{kind:?}");
            // The user's own always shows; anyone else's only when it is about the app.
            assert_eq!((on_bar(kind, User), on_bar(kind, Socket), on_bar(kind, System)), (true, !answer, !answer));
        }
    }

    #[test]
    fn what_a_clients_step_says_is_its_answer_unless_the_app_broke() {
        use Origin::*;
        // (the core's reason, marked as broken, the sender) -> the kind
        for (refused, broke, by, kind) in [
            (true, false, User, Kind::Refused),
            (false, false, User, Kind::Failed),
            (false, true, User, Kind::Failed),
            (false, false, System, Kind::Failed),
            (true, false, System, Kind::Refused),
            (false, false, Socket, Kind::Refused),
            (true, false, Socket, Kind::Refused),
            (false, true, Socket, Kind::Failed),
        ] {
            assert_eq!(said(refused, broke, by), kind, "{refused} {broke} {by:?}");
        }
        // So a client's refusal never reaches the bar, and a shell it could not start does.
        assert!(!on_bar(said(false, false, Socket), Socket));
        assert!(on_bar(said(false, true, Socket), Socket));
    }

    #[test]
    fn a_refusal_of_another_sender_leaves_the_bar_as_it_was() {
        use Origin::*;
        let line = |kind, text: &str| Some(Notice { kind, text: text.into() });
        let held = |said: &Option<Notice>, by| Held { said: said.clone(), by: Some(by) };
        let (refused, failed) =
            (line(Kind::Refused, "only the user renames a workspace"), line(Kind::Failed, "no shell"));
        let users = line(Kind::Refused, "No repository in /tmp");
        // An empty bar stays empty, and so closed: nothing under the top row moves.
        for by in [Socket, System] {
            assert_eq!(Held::default().after(refused.clone(), by), Held::default(), "{by:?}");
        }
        // What stands, whoever said it, is neither covered nor cleared by the refusal.
        for stands in [held(&users, User), held(&failed, System), held(&failed, Socket)] {
            assert_eq!(stands.after(refused.clone(), Socket), stands);
            assert_eq!(stands.after(refused.clone(), System), stands);
        }
        // The user's own refusal shows, also over a client's failure.
        assert_eq!(Held::default().after(users.clone(), User), held(&users, User));
        assert_eq!(held(&failed, Socket).after(users.clone(), User), held(&users, User));
        // The app's failure shows whoever's step found it, but a client's never over the user's.
        assert_eq!(Held::default().after(failed.clone(), Socket), held(&failed, Socket));
        assert_eq!(held(&failed, System).after(failed.clone(), Socket), held(&failed, Socket));
        assert_eq!(held(&users, User).after(failed.clone(), Socket), held(&users, User));
        assert_eq!(held(&users, User).after(failed.clone(), System), held(&failed, System));
        // Nor does a client's step clear the user's or the system's: only a client's.
        assert_eq!(held(&users, User).after(None, Socket), held(&users, User));
        assert_eq!(held(&failed, System).after(None, Socket), held(&failed, System));
        assert_eq!(held(&failed, Socket).after(None, Socket), held(&None, Socket));
        assert_eq!(held(&failed, Socket).after(None, System), held(&None, System));
        assert_eq!(held(&users, User).after(None, User), held(&None, User));
        // The user's line once cleared, a client's failure shows.
        assert_eq!(held(&None, User).after(failed.clone(), Socket), held(&failed, Socket));
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
