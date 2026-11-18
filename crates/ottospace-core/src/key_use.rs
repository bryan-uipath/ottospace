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
    /// ⌘⇧R. `talk_toggle` was ⌘⇧L's: its count is of the old key, so it is not read.
    DrawerToggle,
    SidebarToggle,
    DialToggle,
    SpaceNumber,
    NextSpace,
    NextChat,
    HideSpace,
    MarkPane,
    MarkJump,
    MarkNumber,
    Copy,
    Paste,
    VisualMode,
    ListMode,
    SneakMode,
    /// `zoom` before ⌃⌘F filled the display: its counts are kept.
    #[serde(alias = "zoom")]
    FillDisplay,
    Palette,
    ShowKeys,
    EditSettings,
    Hide,
    Quit,
    ReturnKey,
    CaptureKey,
    Transcribe,
    AgentMenu,
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
    Binding::DrawerToggle,
    Binding::SidebarToggle,
    Binding::DialToggle,
    Binding::SpaceNumber,
    Binding::NextSpace,
    Binding::NextChat,
    Binding::HideSpace,
    Binding::MarkPane,
    Binding::MarkJump,
    Binding::MarkNumber,
    Binding::Copy,
    Binding::Paste,
    Binding::VisualMode,
    Binding::ListMode,
    Binding::SneakMode,
    Binding::FillDisplay,
    Binding::Palette,
    Binding::ShowKeys,
    Binding::EditSettings,
    Binding::Hide,
    Binding::Quit,
    Binding::ReturnKey,
    Binding::CaptureKey,
    Binding::Transcribe,
    Binding::AgentMenu,
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
            Binding::DrawerToggle => &[C::DrawerToggle],
            Binding::SidebarToggle => &[C::SidebarToggle],
            Binding::DialToggle => &[C::DialToggle],
            Binding::NextSpace => &[C::NextSpace, C::PrevSpace],
            Binding::NextChat => &[C::NextChat, C::PrevChat],
            Binding::HideSpace => &[C::HideSpace],
            Binding::MarkPane => &[C::MarkPane, C::MarkTab],
            Binding::MarkJump => &[C::MarkJump],
            Binding::VisualMode => &[C::VisualMode],
            Binding::ListMode => &[C::ListMode],
            Binding::SneakMode => &[C::SneakMode],
            Binding::FillDisplay => &[C::FillDisplay],
            Binding::ShowKeys => &[C::ShowKeys],
            Binding::EditSettings => &[C::EditSettings],
            Binding::Hide => &[C::Hide],
            Binding::Quit => &[C::Quit],
            Binding::Transcribe => &[C::Transcribe],
            Binding::AgentMenu => &[C::AgentMenu],
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

    /// Where the key reference lists it.
    pub fn group(self) -> Group {
        use Binding as B;
        match self {
            B::SplitRight
            | B::SplitDown
            | B::PickPane
            | B::ClosePane
            | B::FocusNext
            | B::FocusDir
            | B::Resize
            | B::Equalize
            | B::FitPane
            | B::Swap
            | B::MoveToTab
            | B::SidebarToggle => Group::Panes,
            B::SpaceNumber
            | B::NextSpace
            | B::NextChat
            | B::HideSpace
            | B::NewTab
            | B::TabNumber
            | B::NextTab
            | B::CloseTab
            | B::SaveWorkbook
            | B::MarkPane
            | B::MarkJump
            | B::MarkNumber => Group::Tabs,
            B::NewTask
            | B::NextTask
            | B::TalkFocus
            | B::DrawerToggle
            | B::DialToggle
            | B::CaptureKey
            | B::Transcribe
            | B::AgentMenu => Group::Tasks,
            B::Copy | B::Paste => Group::Text,
            B::VisualMode | B::ListMode | B::SneakMode => Group::Modes,
            B::Palette | B::ShowKeys | B::EditSettings | B::FillDisplay | B::ReturnKey | B::Hide | B::Quit => {
                Group::App
            }
        }
    }

    /// What it does, as the key reference says it.
    pub fn label(self) -> &'static str {
        use Binding as B;
        match self {
            B::SplitRight => "Terminal right",
            B::SplitDown => "Terminal below",
            B::PickPane => "New pane, right / below",
            B::ClosePane => "Close pane",
            B::FocusNext => "Next / previous pane",
            B::FocusDir => "Focus that way",
            B::Resize => "Resize that way",
            B::Equalize => "Even out splits",
            B::FitPane => "Fit pane to its app",
            B::Swap => "Swap that way",
            B::MoveToTab => "Move pane to tab",
            B::NewTab => "New tab",
            B::TabNumber => "Go to tab",
            B::NextTab => "Next / previous tab",
            B::CloseTab => "Close tab",
            B::SaveWorkbook => "Save tab as workspace",
            B::MarkPane => "Mark pane / tab",
            B::MarkJump => "Go to mark by letter",
            B::MarkNumber => "Go to mark",
            B::NewTask => "New task",
            B::NextTask => "Next that needs you",
            B::TalkFocus => "Type to the agent",
            B::DrawerToggle => "Collapse / open drawer",
            B::SidebarToggle => "Collapse / open sidebar",
            B::DialToggle => "Open / close agent dial",
            B::SpaceNumber => "Go to tab group",
            B::NextSpace => "Next / previous group",
            B::NextChat => "Next / previous in the drawer's rail",
            B::HideSpace => "Hide tab group",
            B::CaptureKey => "Capture, from any app",
            B::Transcribe => "Transcribe the pane / stop",
            B::AgentMenu => "What the task may do in the app pane",
            B::Copy => "Copy",
            B::Paste => "Paste",
            B::VisualMode => "Select text with keys",
            B::ListMode => "Drive the sidebar",
            B::SneakMode => "Click by letter",
            B::Palette => "Any command by name",
            B::ShowKeys => "This list",
            B::EditSettings => "Settings",
            B::FillDisplay => "Fill the display, over the menu bar and Dock",
            B::ReturnKey => "Back here, from any app",
            B::Hide => "Hide",
            B::Quit => "Quit",
        }
    }

    /// Its keys, where no binding of the keymap says them: a numbered family, read in `key_down`.
    pub fn keys(self) -> Option<&'static str> {
        Some(match self {
            Binding::TabNumber => "⌘1–9",
            Binding::MoveToTab => "⌘⇧⌥1–9",
            Binding::MarkNumber => "⌘⌥1–9",
            Binding::SpaceNumber => "⌘⌃1–9",
            _ => return None,
        })
    }

    /// Its id in `keys.json`, e.g. `split_right`.
    pub fn id(self) -> String {
        serde_json::to_value(self).ok().and_then(|id| id.as_str().map(str::to_string)).unwrap_or_default()
    }
}

/// A group of the key reference: what the user is doing. `List` and `Visual` are a mode's own
/// keys, listed first while that mode is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    List,
    Visual,
    Panes,
    Tabs,
    Tasks,
    Text,
    Modes,
    App,
}

/// The groups of every mode, in order.
const GROUPS: [Group; 6] = [Group::Panes, Group::Tabs, Group::Tasks, Group::Text, Group::Modes, Group::App];

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Group::List => "List",
            Group::Visual => "Visual",
            Group::Panes => "Panes",
            Group::Tabs => "Tabs",
            Group::Tasks => "Tasks",
            Group::Text => "Text",
            Group::Modes => "Modes",
            Group::App => "App",
        }
    }

    /// Lines with no binding of their own: a gesture, a mode's keys. Never counted.
    fn notes(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Group::Panes => &[("Drag a header", "Split, swap, move")],
            Group::Text => &[
                ("⇧PgUp ⇧PgDn", "Scroll a page"),
                ("⌃G", "Compose in your editor"),
                ("⌥⌘N", "New task: this checkout or a new worktree"),
            ],
            Group::List => &[
                ("J K ↓ ↑", "Move"),
                ("↵ ⇧↵", "Open / open below"),
                ("Tab ⇧Tab", "Next / previous section"),
                ("S H X U", "Task: snooze, hold, dismiss, undo"),
                ("R M ⇧J ⇧K", "Mark: rename, letter, move"),
                ("A space", "Workspace: add tab, list tabs"),
                ("P /", "Apps: pin, filter"),
                ("← → T", "Calendar: day back, on, today"),
                ("← → A", "Repos: fold, unfold, new worktree"),
            ],
            Group::Visual => &[
                ("H J K L", "Move"),
                ("W B E 0 ^ $", "Word, line"),
                ("⇧H ⇧M ⇧L", "Top, middle, bottom"),
                ("V ⇧V", "Select / lines"),
                ("Y ↵", "Copy and leave"),
            ],
            Group::Tabs | Group::Tasks | Group::Modes | Group::App => &[],
        }
    }
}

/// One line of the key reference.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    /// A binding, and how full its bar is: its score toward `KNOWN_AT`, 0 never used (no bar).
    Key { binding: Binding, fill: f32 },
    /// Keys and what they do, with no count.
    Note { keys: &'static str, what: &'static str },
}

/// The key reference, as the core lays it out: each group with the keys not yet known first,
/// least used first; then every known key, folded under `Known · n` unless `known_open`.
#[derive(Clone, Debug, PartialEq)]
pub struct Reference {
    pub groups: Vec<(Group, Vec<Line>)>,
    /// Key lines, in `ALL`'s order.
    pub known: Vec<Line>,
    pub known_open: bool,
}

impl KeyUse {
    /// The key reference at `now`. `mode`: a mode's group to list first (`List`, `Visual`).
    /// `listed`: whether a binding is there to list (it has a key, its feature is on).
    pub fn reference(
        &self,
        now: u64,
        mode: Option<Group>,
        listed: impl Fn(Binding) -> bool,
        known_open: bool,
    ) -> Reference {
        let listed: Vec<Binding> = ALL.iter().copied().filter(|b| listed(*b)).collect();
        let known: Vec<Binding> = listed.iter().copied().filter(|b| self.known(*b, now)).collect();
        let fill = |binding: Binding| (self.score(binding, now) / KNOWN_AT).min(1.0) as f32;
        let line = |binding: Binding| Line::Key { binding, fill: fill(binding) };
        let group = |group: Group| {
            let mut keys: Vec<Binding> =
                listed.iter().copied().filter(|b| b.group() == group && !known.contains(b)).collect();
            // Least used first; a tie keeps the order of `ALL`.
            keys.sort_by(|a, b| self.score(*a, now).total_cmp(&self.score(*b, now)));
            let keys = keys.into_iter().map(line);
            let notes = group.notes().iter().map(|(keys, what)| Line::Note { keys, what });
            (group, keys.chain(notes).collect::<Vec<Line>>())
        };
        let groups = mode.into_iter().chain(GROUPS).map(group).filter(|(_, lines)| !lines.is_empty()).collect();
        Reference { groups, known: known.into_iter().map(line).collect(), known_open }
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

    /// ⌃⌘F was `zoom` before it filled the display: an older file's counts are its.
    #[test]
    fn an_older_files_zoom_reads_as_fill_display() {
        let uses = KeyUse::parse(r#"{"version":1,"keys":{"zoom":{"score":3,"at":5,"known":true}}}"#).expect("reads");
        assert_eq!(uses.0.get(&Binding::FillDisplay), Some(&Use { score: 3., at: 5, known: true }));
        assert!(uses.write().contains("\"fill_display\""), "{}", uses.write());
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

    fn line(binding: Binding, fill: f32) -> Line {
        Line::Key { binding, fill }
    }

    fn lines(reference: &Reference, group: Group) -> Vec<String> {
        let found = reference.groups.iter().find(|(of, _)| *of == group);
        let line = |line: &Line| match line {
            Line::Key { binding, fill: 0.0 } => binding.id(),
            Line::Key { binding, fill } => format!("{} {fill:.1}", binding.id()),
            Line::Note { keys, .. } => keys.to_string(),
        };
        found.map(|(_, lines)| lines.iter().map(line).collect()).unwrap_or_default()
    }

    #[test]
    fn the_reference_lists_unknown_keys_least_used_first_and_folds_known_ones() {
        let mut uses = KeyUse::default();
        // Terminal right is known; Close pane used twice, Terminal below once.
        burst(5).iter().for_each(|at| uses.press(Binding::SplitRight, T0 + at));
        burst(2).iter().for_each(|at| uses.press(Binding::ClosePane, T0 + at));
        uses.press(Binding::SplitDown, T0);
        let now = T0 + 60_000;
        let all = |_: Binding| true;
        let reference = uses.reference(now, None, all, false);
        let panes = lines(&reference, Group::Panes);
        assert_eq!(
            panes,
            [
                "pick_pane",
                "focus_next",
                "focus_dir",
                "resize",
                "equalize",
                "fit_pane",
                "swap",
                "move_to_tab",
                "sidebar_toggle",
                "split_down 0.2",
                "close_pane 0.4",
                "Drag a header",
            ]
        );
        assert_eq!((&reference.known[..], reference.known_open), (&[line(Binding::SplitRight, 1.0)][..], false));
        let groups: Vec<Group> = reference.groups.iter().map(|(group, _)| *group).collect();
        assert_eq!(groups, GROUPS);
        // Once a known key is forgotten it is back in its group, its bar nearly empty.
        let later = uses.reference(now + 10 * DAY, None, all, false);
        assert!(later.known.is_empty() && lines(&later, Group::Panes).iter().any(|l| l == "split_right 0.4"));
    }

    #[test]
    fn a_modes_keys_come_first_and_a_key_that_is_not_there_is_not_listed() {
        let uses = KeyUse::default();
        let reference = uses.reference(T0, Some(Group::List), |b| b != Binding::ReturnKey, true);
        assert_eq!(reference.groups[0].0, Group::List);
        assert!(lines(&reference, Group::List).iter().all(|line| !line.contains('_')), "notes only");
        assert!(!lines(&reference, Group::App).contains(&"return_key".to_string()));
        assert!(lines(&reference, Group::App).contains(&"quit".to_string()));
        // A group with nothing to list is not drawn.
        let none = uses.reference(T0, None, |b| b.group() != Group::Tabs, true);
        assert!(none.groups.iter().all(|(group, _)| *group != Group::Tabs));
        // Every binding once: in its group or known.
        let mut every = KeyUse::default();
        ALL.iter().step_by(2).for_each(|b| burst(5).iter().for_each(|at| every.press(*b, T0 + at)));
        let reference = every.reference(T0 + 10_000, None, |_| true, false);
        let lines = reference.groups.iter().flat_map(|(_, lines)| lines).chain(&reference.known);
        let mut seen: Vec<Binding> = lines
            .filter_map(|line| match line {
                Line::Key { binding, .. } => Some(*binding),
                Line::Note { .. } => None,
            })
            .collect();
        let mut all = ALL.to_vec();
        seen.sort();
        all.sort();
        assert_eq!(seen, all);
    }

    /// `Known` opens only over something known, and only while the reference shows.
    #[test]
    fn known_opens_only_with_something_known() {
        use crate::model::{Action, Model};
        let mut m = Model::default();
        m.update(Action::ToggleKeys);
        m.update(Action::KeysKnown);
        assert!(!m.keys_known_open, "nothing known");
        let read = KeyUse::parse(r#"{"version":1,"keys":{"copy":{"score":5,"at":0,"known":true}}}"#).unwrap();
        m.update(Action::RestoreKeyUse { uses: read }.sent_by(crate::model::Origin::System));
        m.update(Action::KeysKnown);
        assert!(m.keys_known_open);
        m.update(Action::KeysKnown);
        assert!(!m.keys_known_open);
    }

    /// List mode's keys of marks and the Apps section are listed only while those are on.
    #[test]
    fn a_modes_keys_of_a_feature_that_is_off_are_not_listed() {
        use crate::features::Features;
        use crate::model::{Action, Model, Origin};
        let notes = |m: &Model| {
            let reference = m.key_reference(|_| true);
            let list = reference.groups.into_iter().find(|(group, _)| *group == Group::List);
            let what = |line: Line| match line {
                Line::Note { what, .. } => Some(what),
                Line::Key { .. } => None,
            };
            list.map(|(_, lines)| lines.into_iter().filter_map(what).collect::<Vec<_>>()).unwrap_or_default()
        };
        let mut m = Model::default();
        m.update(Action::ToggleSection { section: crate::model::Section::Tasks });
        m.update(Action::FocusList);
        assert_eq!(m.mode(), crate::model::Mode::List);
        m.update(Action::SetFeatures { features: Features::default() }.sent_by(Origin::System));
        let off = notes(&m);
        assert!(!off.is_empty() && off.iter().all(|w| !w.starts_with("Mark:") && !w.starts_with("Apps:")), "{off:?}");
        m.update(Action::SetFeatures { features: Features::all() }.sent_by(Origin::System));
        assert!(notes(&m).iter().any(|w| w.starts_with("Mark:")) && notes(&m).iter().any(|w| w.starts_with("Apps:")));
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
