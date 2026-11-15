//! How much the user uses each key: a score per binding that decays, and whether it is known.
//! Only the user's own key presses count. The shell keeps it in `<home>/keys.json`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::commands::Command;

/// Why a client's count of a key, or reset of the counts, is refused.
pub const USERS_ALONE: &str = "only the user's own keys are counted";
/// Why a client's counts are refused: the shell reads them from `keys.json`.
pub const READ_ALONE: &str = "only ottospace reads keys.json";

/// A key the user can learn, by a stable id (`keys.json`'s): one row of the key reference.
/// Arrows and H J K L of one move are one binding, as is a numbered family (⌘1–9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Binding {
    SplitRight,
    SplitDown,
    PickPane,
    ClosePane,
    FocusNext,
    FocusDir,
    Resize,
    Equalize,
    FitPane,
    Swap,
    MoveToTab,
    NewTab,
    TabNumber,
    NextTab,
    CloseTab,
    SaveWorkbook,
    NewTask,
    NextTask,
    TalkFocus,
    TalkToggle,
    SpaceNumber,
    NextSpace,
    HideSpace,
    MarkPane,
    MarkJump,
    MarkNumber,
    Copy,
    Paste,
    VisualMode,
    ListMode,
    SneakMode,
    Zoom,
    Palette,
    ShowKeys,
    EditSettings,
    Hide,
    Quit,
    ReturnKey,
    CaptureKey,
}

/// Every binding, in the key reference's order.
pub const ALL: &[Binding] = &[
    Binding::SplitRight,
    Binding::SplitDown,
    Binding::PickPane,
    Binding::ClosePane,
    Binding::FocusNext,
    Binding::FocusDir,
    Binding::Resize,
    Binding::Equalize,
    Binding::FitPane,
    Binding::Swap,
    Binding::MoveToTab,
    Binding::NewTab,
    Binding::TabNumber,
    Binding::NextTab,
    Binding::CloseTab,
    Binding::SaveWorkbook,
    Binding::NewTask,
    Binding::NextTask,
    Binding::TalkFocus,
    Binding::TalkToggle,
    Binding::SpaceNumber,
    Binding::NextSpace,
    Binding::HideSpace,
    Binding::MarkPane,
    Binding::MarkJump,
    Binding::MarkNumber,
    Binding::Copy,
    Binding::Paste,
    Binding::VisualMode,
    Binding::ListMode,
    Binding::SneakMode,
    Binding::Zoom,
    Binding::Palette,
    Binding::ShowKeys,
    Binding::EditSettings,
    Binding::Hide,
    Binding::Quit,
    Binding::ReturnKey,
    Binding::CaptureKey,
];

impl Binding {
    /// The commands whose keys this is. Empty: a key with no command (⌘1–9, ⌘C, the return key).
    pub fn commands(self) -> &'static [Command] {
        use Command as C;
        match self {
            Binding::SplitRight => &[C::SplitRight],
            Binding::SplitDown => &[C::SplitDown],
            Binding::PickPane => &[C::PickPane, C::PickPaneDown],
            Binding::ClosePane => &[C::ClosePane],
            Binding::FocusNext => &[C::FocusNext, C::FocusPrev],
            Binding::FocusDir => &[C::FocusLeft, C::FocusRight, C::FocusUp, C::FocusDown],
            Binding::Resize => &[C::ResizeLeft, C::ResizeRight, C::ResizeUp, C::ResizeDown],
            Binding::Equalize => &[C::Equalize],
            Binding::FitPane => &[C::FitPane],
            Binding::Swap => &[C::SwapLeft, C::SwapRight, C::SwapUp, C::SwapDown],
            Binding::NewTab => &[C::NewTab],
            Binding::NextTab => &[C::NextTab, C::PrevTab],
            Binding::CloseTab => &[C::CloseTab],
            Binding::SaveWorkbook => &[C::SaveWorkbook],
            Binding::NewTask => &[C::NewTask],
            Binding::NextTask => &[C::NextTask],
            Binding::TalkFocus => &[C::TalkFocus],
            Binding::TalkToggle => &[C::TalkToggle],
            Binding::NextSpace => &[C::NextSpace, C::PrevSpace],
            Binding::HideSpace => &[C::HideSpace],
            Binding::MarkPane => &[C::MarkPane, C::MarkTab],
            Binding::MarkJump => &[C::MarkJump],
            Binding::VisualMode => &[C::VisualMode],
            Binding::ListMode => &[C::ListMode],
            Binding::SneakMode => &[C::SneakMode],
            Binding::Zoom => &[C::Zoom],
            Binding::ShowKeys => &[C::ShowKeys],
            Binding::EditSettings => &[C::EditSettings],
            Binding::Hide => &[C::Hide],
            Binding::Quit => &[C::Quit],
            Binding::MoveToTab
            | Binding::TabNumber
            | Binding::SpaceNumber
            | Binding::MarkNumber
            | Binding::Copy
            | Binding::Paste
            | Binding::Palette
            | Binding::ReturnKey
            | Binding::CaptureKey => &[],
        }
    }

    /// The binding a command's key counts for. `None`: its key, if any, is not one to learn
    /// (a debug key, or a command reached by the palette and menus alone).
    pub fn of(command: Command) -> Option<Binding> {
        ALL.iter().copied().find(|binding| binding.commands().contains(&command))
    }

    /// Its id in `keys.json`, e.g. `split_right`.
    pub fn id(self) -> String {
        serde_json::to_value(self).ok().and_then(|id| id.as_str().map(str::to_string)).unwrap_or_default()
    }
}

/// A score halves in a week without use: about two weeks of memory.
pub const HALF_LIFE_MS: u64 = 7 * 24 * 60 * 60 * 1000;
/// Known from this score: five uses within a day or two, or a steady habit.
pub const KNOWN_AT: f64 = 4.5;
/// A known key is known until its score decays below this: about eight idle days from `KNOWN_AT`.
pub const FORGOTTEN_BELOW: f64 = 2.0;
/// The most a score holds: a key used every day is forgotten within about three idle weeks.
pub const MOST: f64 = 20.0;
/// A press this soon after the last use is the same use: a held key repeats, a burst of ⌘] is one.
pub const REPEAT_MS: u64 = 1000;

/// One binding's use: its score at `at` (ms), and whether it was known then.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Use {
    pub score: f64,
    pub at: u64,
    #[serde(default)]
    pub known: bool,
}

impl Use {
    /// The score decayed to `now`. A clock set back decays nothing.
    pub fn score_at(&self, now: u64) -> f64 {
        let weeks = now.saturating_sub(self.at) as f64 / HALF_LIFE_MS as f64;
        self.score * 0.5f64.powf(weeks)
    }

    /// Known at `now`: it reached `KNOWN_AT`, and has not decayed below `FORGOTTEN_BELOW` since.
    pub fn known_at(&self, now: u64) -> bool {
        self.known && self.score_at(now) >= FORGOTTEN_BELOW
    }

    /// One more use at `now`, unless it is within `REPEAT_MS` of the last.
    fn pressed(self, now: u64) -> Use {
        if self.score > 0.0 && (self.at..self.at.saturating_add(REPEAT_MS)).contains(&now) {
            return self;
        }
        let known = self.known_at(now);
        let score = (self.score_at(now) + 1.0).min(MOST);
        Use { score, at: now.max(self.at), known: known || score >= KNOWN_AT }
    }
}

/// Every binding's use. Bindings never pressed are not in it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyUse(BTreeMap<Binding, Use>);

/// `keys.json`'s version: a file of a higher one is a newer build's.
pub const VERSION: u64 = 1;
/// The largest `keys.json` read.
pub const MAX_FILE_BYTES: usize = 64 * 1024;

/// Why `keys.json` was not read.
#[derive(Debug, PartialEq)]
pub enum Unread {
    /// A newer build's, of this version: left as it is.
    Newer(u64),
    /// Not a file of key counts.
    Bad(String),
}

/// The file: per binding id, its use. Each record may gain fields; an id this build does not
/// know is skipped. e.g. `{"version":1,"keys":{"split_right":{"score":3.2,"at":1731600000000,"known":false}}}`.
#[derive(Serialize, Deserialize)]
struct File {
    version: u64,
    #[serde(default)]
    keys: BTreeMap<String, serde_json::Value>,
}

impl KeyUse {
    /// One use of `binding` at `now` (ms).
    pub fn press(&mut self, binding: Binding, now: u64) {
        let used = self.0.get(&binding).copied().unwrap_or_default().pressed(now);
        self.0.insert(binding, used);
    }

    pub fn get(&self, binding: Binding) -> Option<Use> {
        self.0.get(&binding).copied()
    }

    /// Its score at `now`; 0 if never used.
    pub fn score(&self, binding: Binding, now: u64) -> f64 {
        self.0.get(&binding).map_or(0.0, |used| used.score_at(now))
    }

    pub fn known(&self, binding: Binding, now: u64) -> bool {
        self.0.get(&binding).is_some_and(|used| used.known_at(now))
    }

    /// Each binding used, with its use.
    pub fn uses(&self) -> impl Iterator<Item = (Binding, Use)> + '_ {
        self.0.iter().map(|(binding, used)| (*binding, *used))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `keys.json`'s text. A record holds no key text, nothing typed and no argument.
    pub fn write(&self) -> String {
        let keys = self.0.iter().filter_map(|(binding, used)| Some((binding.id(), serde_json::to_value(used).ok()?)));
        crate::pretty_json(&File { version: VERSION, keys: keys.collect() })
    }

    /// Reads `keys.json`. A record that does not read (a negative score, an unknown id) is skipped.
    pub fn parse(text: &str) -> Result<KeyUse, Unread> {
        if text.len() > MAX_FILE_BYTES {
            return Err(Unread::Bad(format!("larger than {MAX_FILE_BYTES} bytes")));
        }
        let file: serde_json::Value = serde_json::from_str(text).map_err(|err| Unread::Bad(err.to_string()))?;
        // The version first: a newer build's file may have another shape, and must not be overwritten.
        if let Some(version) = file["version"].as_u64().filter(|v| *v > VERSION) {
            return Err(Unread::Newer(version));
        }
        let file: File = serde_json::from_value(file).map_err(|err| Unread::Bad(err.to_string()))?;
        let record = |(id, value): (String, serde_json::Value)| {
            let binding: Binding = serde_json::from_value(serde_json::Value::String(id)).ok()?;
            let used: Use = serde_json::from_value(value).ok()?;
            let sane = used.score.is_finite() && (0.0..=MOST).contains(&used.score);
            sane.then_some((binding, used))
        };
        Ok(KeyUse(file.keys.into_iter().filter_map(record).collect()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands;

    const DAY: u64 = 24 * 60 * 60 * 1000;
    const HOUR: u64 = DAY / 24;
    /// A start well past 0, as the shell's clock is.
    const T0: u64 = 1_731_600_000_000;

    /// `n` presses two seconds apart, from `T0`.
    fn burst(n: u64) -> Vec<u64> {
        (0..n).map(|i| i * 2000).collect()
    }

    /// `presses` uses at those times after `T0`, then asked at `then` after `T0`.
    fn after(presses: &[u64], then: u64) -> (f64, bool) {
        let mut uses = KeyUse::default();
        for at in presses {
            uses.press(Binding::SplitRight, T0 + at);
        }
        (uses.score(Binding::SplitRight, T0 + then), uses.known(Binding::SplitRight, T0 + then))
    }

    #[test]
    fn known_once_used_enough_and_forgotten_when_idle() {
        let five_in_a_day: Vec<u64> = (0..5).map(|i| i * 5 * HOUR).collect();
        let cases: &[(&str, Vec<u64>, u64, bool)] = &[
            ("never used", vec![], 0, false),
            ("once", vec![0], 0, false),
            ("four in a minute", burst(4), 60_000, false),
            ("five in a minute", burst(5), 60_000, true),
            // A key held down repeats: one use a second.
            ("held for half a second", (0..20).map(|i| i * 25).collect(), 500, false),
            ("five in a day", five_in_a_day.clone(), 20 * HOUR, true),
            ("five, one a day", (0..5).map(|d| d * DAY).collect(), 4 * DAY, false),
            ("seven, one a day", (0..7).map(|d| d * DAY).collect(), 6 * DAY, true),
            // Known, it stays known while it decays to 2: about eight idle days.
            ("known, a week idle", five_in_a_day.clone(), 20 * HOUR + 7 * DAY, true),
            ("known, ten days idle", five_in_a_day.clone(), 20 * HOUR + 10 * DAY, false),
            ("known, a month idle", five_in_a_day.clone(), 30 * DAY, false),
        ];
        for (name, presses, then, known) in cases {
            assert_eq!(after(presses, *then).1, *known, "{name}: {:?}", after(presses, *then));
        }
    }

    #[test]
    fn a_score_halves_in_a_week() {
        let (score, _) = after(&[0, 0], 7 * DAY);
        assert!((score - 0.5).abs() < 1e-9, "{score}");
        let (score, _) = after(&[0], 14 * DAY);
        assert!((score - 0.25).abs() < 1e-9, "{score}");
    }

    #[test]
    fn a_forgotten_key_is_learned_again_by_use() {
        let mut presses = burst(5);
        // Forgotten after ten days; one use then is not enough, three are.
        presses.push(10 * DAY);
        assert!(!after(&presses, 10 * DAY).1);
        presses.extend([10 * DAY + 2000, 10 * DAY + 4000]);
        assert!(after(&presses, 10 * DAY + 4000).1, "{:?}", after(&presses, 10 * DAY + 4000));
    }

    #[test]
    fn a_score_has_a_ceiling_and_a_clock_set_back_decays_nothing() {
        let (score, _) = after(&burst(100), 200_000);
        assert!((score - MOST).abs() < 1e-3, "{score}");
        let mut uses = KeyUse::default();
        uses.press(Binding::Copy, T0);
        assert_eq!(uses.score(Binding::Copy, T0 - DAY), 1.0);
        uses.press(Binding::Copy, T0 - DAY);
        assert_eq!(uses.get(Binding::Copy).map(|used| (used.score, used.at)), Some((2.0, T0)));
    }

    #[test]
    fn the_file_holds_ids_scores_and_times_and_reads_back() {
        let mut uses = KeyUse::default();
        uses.press(Binding::FocusDir, T0);
        (0..5).for_each(|_| uses.press(Binding::Copy, T0 + HOUR));
        let text = uses.write();
        assert!(text.contains("\"focus_dir\"") && text.contains("\"copy\"") && text.contains("\"version\": 1"));
        assert_eq!(KeyUse::parse(&text), Ok(uses));
    }

    #[test]
    fn a_file_that_does_not_read_is_told_and_a_bad_record_is_skipped() {
        let cases: &[(&str, Result<usize, ()>)] = &[
            ("", Err(())),
            ("{", Err(())),
            ("[]", Err(())),
            (r#"{"version":1}"#, Ok(0)),
            (r#"{"version":1,"keys":{"copy":{"score":1,"at":5}}}"#, Ok(1)),
            // A binding this build does not know, a negative or huge score, a record of another shape.
            (r#"{"version":1,"keys":{"gone":{"score":1,"at":5},"copy":{"score":-1,"at":5}}}"#, Ok(0)),
            (r#"{"version":1,"keys":{"copy":{"score":1e300,"at":5},"paste":"x"}}"#, Ok(0)),
            // A field a later build adds is no harm.
            (r#"{"version":1,"keys":{"copy":{"score":1,"at":5,"hinted":2}},"seen":[]}"#, Ok(1)),
        ];
        for (text, read) in cases {
            let got = KeyUse::parse(text).map(|uses| uses.0.len()).map_err(|_| ());
            assert_eq!(got, *read, "{text}");
        }
        assert_eq!(KeyUse::parse(r#"{"version":2,"keys":{}}"#), Err(Unread::Newer(2)));
        assert_eq!(KeyUse::parse(r#"{"version":2,"keys":[]}"#), Err(Unread::Newer(2)));
        assert!(matches!(KeyUse::parse(&" ".repeat(MAX_FILE_BYTES + 1)), Err(Unread::Bad(_))));
    }

    /// The user's own keys count, and a count ends no mode; nobody else counts or resets.
    #[test]
    fn only_the_users_keys_count_and_a_count_is_no_input() {
        use crate::model::{Action, Model, Origin};
        let mut m = Model::default();
        m.update(Action::ToggleKeys);
        let used = Action::KeyUsed { binding: Binding::ShowKeys, at: T0 };
        for origin in [Origin::Socket, Origin::System] {
            assert_eq!(m.refusal(&used, origin), Some(USERS_ALONE));
            assert_eq!(m.update(used.clone().sent_by(origin)), []);
        }
        assert!(m.key_use.is_empty());
        assert_eq!(m.update(used.clone()), []);
        assert_eq!((m.key_use.score(Binding::ShowKeys, T0), m.keys), (1.0, true));
        m.update(Action::ResetKeyUse.sent_by(Origin::Socket));
        assert!(!m.key_use.is_empty());
        m.update(Action::ResetKeyUse);
        assert!(m.key_use.is_empty());
        // keys.json, as the shell read it; never a client's.
        let read = KeyUse::parse(r#"{"version":1,"keys":{"copy":{"score":5,"at":1,"known":true}}}"#).unwrap();
        let restore = Action::RestoreKeyUse { uses: read.clone() };
        assert_eq!(m.refusal(&restore, Origin::Socket), Some(READ_ALONE));
        m.update(restore.clone().sent_by(Origin::Socket));
        assert!(m.key_use.is_empty());
        m.update(restore.sent_by(Origin::System));
        assert_eq!(m.key_use, read);
    }

    /// A key of a feature that is off is not counted.
    #[test]
    fn a_key_of_a_feature_that_is_off_is_not_counted() {
        use crate::features::{Feature, Features};
        use crate::model::{Action, Model, Origin};
        let mut m = Model::default();
        let on = |m: &mut Model, on: bool| {
            let features = Features::default().with(Feature::Marks, on);
            m.update(Action::SetFeatures { features }.sent_by(Origin::System));
        };
        on(&mut m, false);
        for binding in [Binding::MarkPane, Binding::MarkJump, Binding::MarkNumber] {
            assert!(!m.binding_on(binding), "{binding:?}");
            m.update(Action::KeyUsed { binding, at: T0 });
        }
        assert!(m.key_use.is_empty() && m.binding_on(Binding::Copy));
        on(&mut m, true);
        m.update(Action::KeyUsed { binding: Binding::MarkNumber, at: T0 });
        assert_eq!(m.key_use.score(Binding::MarkNumber, T0), 1.0);
    }

    /// Ids are the command's own where a binding is one command's: one source for names.
    #[test]
    fn a_command_counts_for_one_binding_and_ids_follow_the_commands() {
        for command in commands::ALL {
            let of: Vec<Binding> = ALL.iter().copied().filter(|b| b.commands().contains(command)).collect();
            assert!(of.len() <= 1, "{command:?} in {of:?}");
        }
        for binding in ALL {
            if let [command] = binding.commands() {
                assert_eq!(serde_json::to_value(command).ok(), serde_json::to_value(binding).ok(), "{binding:?}");
            }
        }
        for (i, binding) in ALL.iter().enumerate() {
            assert!(!ALL[..i].contains(binding), "{binding:?} twice");
        }
        assert_eq!(Binding::of(Command::FocusUp), Some(Binding::FocusDir));
        assert_eq!(Binding::of(Command::AxOverlay), None);
        assert_eq!(Binding::Resize.id(), "resize");
    }
}
