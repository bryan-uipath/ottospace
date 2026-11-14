//! The command palette: every command that needs no argument, by name, and how a typed query
//! ranks them. The shell runs a command as its menu item does; its key is the shell's binding.

use serde::{Deserialize, Serialize};

use crate::edit::Field;

/// Why a client's request to the palette is refused.
pub const USERS_ALONE: &str = "only the user runs the command palette";
/// Why an action waits, or a client's is refused, while the palette is open.
pub const PALETTE_OPEN: &str = "the command palette is open";

/// Where a command is listed, e.g. "Panes".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    Panes,
    Tabs,
    Tasks,
    Workbook,
    Marks,
    View,
    Developer,
    App,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Group::Panes => "Panes",
            Group::Tabs => "Tabs",
            Group::Tasks => "Tasks",
            Group::Workbook => "Workbook",
            Group::Marks => "Marks",
            Group::View => "View",
            Group::Developer => "Developer",
            Group::App => "App",
        }
    }
}

/// One list makes the enum, `ALL` and the names: a command cannot be left out of the palette.
macro_rules! commands {
    ($($variant:ident: $name:literal, $group:ident;)*) => {
        /// A command, as the menu bar names it. The shell maps each to its menu item's action.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum Command { $($variant),* }

        /// Every command, in the order the palette lists them with nothing typed.
        pub const ALL: &[Command] = &[$(Command::$variant),*];

        impl Command {
            /// Its menu item's name, e.g. "Terminal Right".
            pub fn name(self) -> &'static str {
                match self { $(Command::$variant => $name),* }
            }

            pub fn group(self) -> Group {
                match self { $(Command::$variant => Group::$group),* }
            }
        }
    };
}

commands! {
    SplitRight: "Terminal Right", Panes;
    SplitDown: "Terminal Below", Panes;
    PickPane: "New Pane…", Panes;
    PickPaneDown: "New Pane Below…", Panes;
    ClosePane: "Close Pane", Panes;
    FocusLeft: "Focus Left", Panes;
    FocusRight: "Focus Right", Panes;
    FocusUp: "Focus Up", Panes;
    FocusDown: "Focus Down", Panes;
    FocusNext: "Next Pane", Panes;
    FocusPrev: "Previous Pane", Panes;
    ResizeLeft: "Resize Left", Panes;
    ResizeRight: "Resize Right", Panes;
    ResizeUp: "Resize Up", Panes;
    ResizeDown: "Resize Down", Panes;
    Equalize: "Even Out the Splits", Panes;
    FitPane: "Fit to the App's Window", Panes;
    SwapLeft: "Swap Left", Panes;
    SwapRight: "Swap Right", Panes;
    SwapUp: "Swap Up", Panes;
    SwapDown: "Swap Down", Panes;
    NewTab: "New Tab", Tabs;
    NextTab: "Next Tab", Tabs;
    PrevTab: "Previous Tab", Tabs;
    CloseTab: "Close Tab", Tabs;
    NewTask: "New Task", Tasks;
    NextTask: "Next Task", Tasks;
    CapturePane: "Capture the Pane", Tasks;
    TalkFocus: "Type in the Conversation", Tasks;
    TalkToggle: "Open or Close the Conversation", Tasks;
    TalkGo: "Go to the Task's Tabs", Tasks;
    StopAgent: "Stop the Agent", Tasks;
    SaveWorkbook: "Save Tab as Workbook", Workbook;
    AddTabToWorkbook: "Add Tab to Workbook…", Workbook;
    SaveAllWorkbook: "Save All Tabs as Workbook", Workbook;
    StartFresh: "Start Fresh", App;
    MarkPane: "Mark Pane", Marks;
    MarkTab: "Mark Tab", Marks;
    MarkJump: "Go to Mark…", Marks;
    ListMode: "Drive the Bar with the Keyboard", View;
    SneakMode: "Label the Controls", View;
    VisualMode: "Select with the Keyboard", View;
    Zoom: "Fill the Screen", View;
    ShowKeys: "Keys", View;
    AxOverlay: "Accessibility Overlay", Developer;
    AxInspector: "Accessibility Inspector", Developer;
    InspectorFilter: "Filter the Accessibility Inspector", Developer;
    InspectorRefresh: "Read the Inspected App Again", Developer;
    CopyState: "Copy State as JSON", Developer;
    OpenSettings: "Open the Settings File", Developer;
    OpenFolder: "Open the ottospace Folder", Developer;
    BringBack: "Bring All Windows Back", Developer;
    Hide: "Hide ottospace", App;
    Quit: "Quit ottospace", App;
}

/// The open palette: a filter, and the row it highlights.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Palette {
    pub query: Field,
    /// Index into `matches()`.
    pub selected: usize,
}

impl Palette {
    pub fn matches(&self) -> Vec<Command> {
        rank(self.query.text())
    }
}

/// The commands `query` matches, best first: its letters in order, ignoring case, in the
/// command's name, else in its group and name ("dev" finds every Developer command). Nothing
/// typed: all of them, in `ALL`'s order. Ties keep that order.
pub fn rank(query: &str) -> Vec<Command> {
    let query: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
    let mut scored: Vec<(i64, usize, Command)> = ALL
        .iter()
        .enumerate()
        .filter_map(|(at, &command)| {
            let named = score(&query, command.name());
            // Behind every match of the name alone.
            let grouped =
                || score(&query, &format!("{} {}", command.group().label(), command.name())).map(|s| s - 1000);
            named.or_else(grouped).map(|score| (score, at, command))
        })
        .collect();
    scored.sort_by_key(|(score, at, _)| (-score, *at));
    scored.into_iter().map(|(.., command)| command).collect()
}

/// How well `query` (lowercase, no spaces) runs through `text` in order: higher is better.
/// `None`: it does not. A letter at the start of a word, or right after the one before it,
/// counts for more, the latter most, so "tr" ranks "Terminal Right" above "Next Tab". Spaces in `text` are
/// skipped as `query` has none.
pub fn score(query: &[char], text: &str) -> Option<i64> {
    let chars: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let starts: Vec<bool> = (0..chars.len()).map(|i| i == 0 || !chars[i - 1].is_alphanumeric()).collect();
    // best[j]: the best score of the query so far with its last letter at `j`. None: no way.
    let mut best: Vec<Option<i64>> = vec![Some(0); chars.len() + 1];
    let mut first = true;
    for &q in query {
        let mut next = vec![None; chars.len() + 1];
        // The best of `best` before each place, and whether the letter just before was matched.
        let mut before: Option<i64> = if first { Some(0) } else { None };
        for j in 0..chars.len() {
            let adjacent = if first { None } else { best[j] };
            if chars[j] == q {
                let word = if starts[j] { 8 } else { 0 };
                let run = adjacent.map(|s| s + 1 + word + 10);
                let gap = before.map(|s| s + 1 + word);
                next[j + 1] = run.max(gap);
            }
            if !first {
                before = before.max(best[j]);
            }
        }
        best = next;
        first = false;
    }
    if query.is_empty() { Some(0) } else { best.into_iter().flatten().max() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuzz::Rng;

    fn names(query: &str) -> Vec<&'static str> {
        rank(query).into_iter().map(Command::name).collect()
    }

    #[test]
    fn nothing_typed_lists_every_command_in_order() {
        assert_eq!(rank(""), ALL);
        assert_eq!(rank("  "), ALL);
    }

    #[test]
    fn letters_in_order_match_ignoring_case() {
        assert_eq!(names("TERMRIGHT")[0], "Terminal Right");
        assert!(names("thgir").is_empty());
        // Spaces in the query are not letters to find.
        assert_eq!(names("close tab")[0], "Close Tab");
    }

    #[test]
    fn word_starts_and_runs_rank_first() {
        assert_eq!(names("tr")[0], "Terminal Right");
        assert_eq!(names("acov")[0], "Accessibility Overlay");
        assert_eq!(names("next")[..2], ["Next Pane", "Next Tab"]);
        assert_eq!(names("quit")[0], "Quit ottospace");
    }

    #[test]
    fn a_group_name_finds_its_commands_after_name_matches() {
        let developer: Vec<Command> = ALL.iter().copied().filter(|c| c.group() == Group::Developer).collect();
        let found = rank("developer");
        assert!(developer.iter().all(|c| found.contains(c)), "{found:?}");
        // "Dev" matches no name, so it is the group's alone.
        assert_eq!(rank("dev"), developer);
    }

    #[test]
    fn every_command_is_listed_once_with_a_name() {
        for (i, c) in ALL.iter().enumerate() {
            assert!(!ALL[..i].contains(c), "{c:?} twice");
            assert!(!c.name().is_empty());
            assert!(!ALL[..i].iter().any(|o| o.name() == c.name()), "two commands named {}", c.name());
        }
    }

    /// A naive check of a subsequence, to hold `score` to.
    fn runs_through(query: &[char], text: &str) -> bool {
        let mut letters = text.chars().flat_map(char::to_lowercase);
        query.iter().all(|q| letters.any(|c| c == *q))
    }

    /// Random queries: letters from the names, and others. Every command matched has the letters
    /// in order; none that does is left out; none is listed twice; and the full name ranks first.
    #[test]
    fn random_queries_match_exactly_the_commands_they_run_through() {
        let seed = std::env::var("OTTOSPACE_PROP_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(7);
        let mut rng = Rng::new(seed);
        let letters: Vec<char> = ALL.iter().flat_map(|c| c.name().chars()).chain("zqXÉ…ß ".chars()).collect();
        for _ in 0..2000 {
            let len = rng.below(6);
            let query: String = (0..len).map(|_| rng.pick(&letters)).collect();
            let wanted: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
            let found = rank(&query);
            for (i, c) in found.iter().enumerate() {
                assert!(!found[..i].contains(c), "{query:?}: {c:?} twice");
            }
            for c in ALL {
                let whole = format!("{} {}", c.group().label(), c.name());
                let expect = runs_through(&wanted, c.name()) || runs_through(&wanted, &whole);
                assert_eq!(found.contains(c), expect, "{query:?} and {:?}", c.name());
            }
        }
        for c in ALL {
            assert_eq!(rank(c.name())[0], *c, "its own name ranks {:?} first", c.name());
        }
    }
}
