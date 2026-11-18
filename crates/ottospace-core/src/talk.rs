//! The right drawer: one conversation at a time, beside the workspace and in no tab. Today a
//! conversation is a task's agent in a terminal the shell hosts; what the drawer shows of it
//! is the shell's to draw. See `docs/ai/task-conversation.md`.

use serde::Serialize;

use crate::activity::Pulse;
use crate::tasks::TaskId;

/// The drawer's width in points when nobody has dragged it, and the least and most it takes.
pub const WIDTH: f32 = 480.;
pub const MIN_WIDTH: f32 = 320.;
pub const MAX_WIDTH: f32 = 1600.;

/// Whose a conversation is: a task's, or a session's that no task owns, by runner and id.
/// Drawn as the task's id, or as `{"runner": "claude", "id": "<uuid>"}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(untagged)]
pub enum Subject {
    Task(TaskId),
    Session { runner: String, id: String },
}

impl Subject {
    /// The task it is the conversation of, if it is a task's.
    pub fn task(&self) -> Option<TaskId> {
        match self {
            Subject::Task(id) => Some(*id),
            Subject::Session { .. } => None,
        }
    }
}

/// The conversation the drawer is on.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Talk {
    /// Whose it is.
    pub task: Subject,
    /// False: collapsed into the rail. The agent goes on; ⌘⇧R or its rail item opens it again.
    pub open: bool,
    /// The keyboard is in it: keys go to the agent, as in an app pane they go to the app.
    pub keys: bool,
}

/// The agent chip at the tab row's end: the selected space's conversation while it is closed.
/// A click on it opens the conversation (`Action::SpaceClick` on its space).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Chip {
    pub task: Subject,
    /// Its space's place on the top row.
    pub space: usize,
    /// As its dial's centre; `needs_you` also while it waits and its task is under Needs you.
    pub state: Pulse,
    /// `Working · cargo test`, `Needs you · git push`, `Idle`, `Ended`, `Failed`. The part
    /// after the dot is the agent's: an untrusted label.
    pub line: String,
}

/// The most characters of a call's label in the chip's line.
pub const CHIP_LABEL: usize = 60;

/// The chip's line: the state's word, then what it waits on, cleaned and cut.
pub fn chip_line(state: Pulse, label: Option<&str>) -> String {
    let word = match state {
        Pulse::Running => "Working",
        Pulse::NeedsYou => "Needs you",
        Pulse::Idle => "Idle",
        Pulse::Ended => "Ended",
        Pulse::Error => "Failed",
    };
    // Cut before it is cleaned, and after: a label may be long.
    let label = label.map(|label| label.chars().take(4 * CHIP_LABEL).collect::<String>());
    let label = label.map(|label| crate::transcript::clean(&label).replace('\n', " "));
    let label: Option<String> = label.map(|label| label.trim().chars().take(CHIP_LABEL).collect());
    match label.filter(|label| !label.is_empty()) {
        Some(label) if matches!(state, Pulse::Running | Pulse::NeedsYou) => format!("{word} · {label}"),
        _ => word.to_string(),
    }
}

/// How many sessions' reads are kept, the last first.
pub const KEPT: usize = 8;

/// A task's badge: the first letters of the title's first two words,
/// or the first two letters of a one-word title, in capitals. Whole graphemes: `éte` → `ÉT`.
pub fn monogram(title: &str) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    let words: Vec<&str> = title.split_whitespace().collect();
    let letters: Vec<&str> = match words[..] {
        [] => Vec::new(),
        [one] => one.graphemes(true).take(2).collect(),
        [one, two, ..] => [one, two].iter().filter_map(|word| word.graphemes(true).next()).collect(),
    };
    letters.concat().to_uppercase()
}

/// The monograms of the tasks, in order: each `monogram`, unless an earlier cell has it;
/// then the first letter and the first letter (not a space) where this title differs from the
/// earlier one's, else the first letter and the cell's place.
pub fn monograms(titles: &[&str]) -> Vec<String> {
    use unicode_segmentation::UnicodeSegmentation;
    let letters = |title: &str| -> Vec<String> {
        title.graphemes(true).filter(|g| !g.trim().is_empty()).map(str::to_uppercase).collect()
    };
    let mut out: Vec<String> = Vec::new();
    for (at, title) in titles.iter().enumerate() {
        let mut mono = monogram(title);
        if let Some(earlier) = out.iter().position(|seen| *seen == mono) {
            let (mine, theirs) = (letters(title), letters(titles[earlier]));
            let first = mine.first().cloned().unwrap_or_default();
            let parted = (1..mine.len())
                .filter(|i| theirs.get(*i) != Some(&mine[*i]))
                .map(|i| format!("{first}{}", mine[i]))
                .find(|candidate| !out.contains(candidate));
            // Else its place, or the first number after it no cell has.
            let numbered = (at + 1..).map(|n| format!("{first}{n}")).find(|candidate| !out.contains(candidate));
            mono = parted.or(numbered).unwrap_or_default();
        }
        out.push(mono);
    }
    out
}

/// A width asked for (a drag of the drawer's edge), as one it may have.
pub fn width(asked: f32) -> f32 {
    if asked.is_finite() { asked.clamp(MIN_WIDTH, MAX_WIDTH) } else { WIDTH }
}

/// What the drawer says of its agent apart from the title: the line at its foot, the folder in
/// its header only when it is not the default, and the facts of the title's hover.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct AgentLine {
    /// At the drawer's foot: the model in short, a mode that lets it act unasked and the folder
    /// it runs in, in short (`short_dir`): `haiku-5.5 · auto · ~/code/app`.
    pub foot: Option<String>,
    /// That mode asks for nothing at all (`bypassPermissions`): the foot is drawn as a warning.
    pub bypass: bool,
    /// Where it runs, in the header: only when not `default` (the task's repository, else home).
    pub folder: Option<String>,
    /// The title's hover, as (name, value): model, mode, running for, folder.
    pub facts: Vec<(&'static str, String)>,
}

/// Claude Code's permission modes that let the agent act without asking, by the word the foot
/// shows. `default`, `plan` and `dontAsk` ask or refuse: they are in the hover only.
const UNASKED: [(&str, &str); 3] = [("acceptEdits", "accept edits"), ("auto", "auto"), ("bypassPermissions", "bypass")];

/// Where an agent runs, as paths: `dir` its process's folder (or where it would start),
/// `default` its task's own (its repository, else home), `home` the home folder, `worktree` the
/// listed worktree `dir` is in, if any.
#[derive(Clone, Copy, Debug, Default)]
pub struct Place<'a> {
    pub dir: Option<&'a str>,
    pub default: Option<&'a str>,
    pub home: Option<&'a str>,
    pub worktree: Option<&'a str>,
}

/// The longest folder at the drawer's foot, in characters.
pub const DIR_CHARS: usize = 28;

/// `dir` at or below `root`: what follows `root` (`""`, `/src`).
fn below<'a>(dir: &'a str, root: &str) -> Option<&'a str> {
    let rest = dir.strip_prefix(root.trim_end_matches('/')).filter(|_| !root.is_empty())?;
    Some(rest).filter(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// `dir` with the home folder as `~`: `~/code/app`.
fn tilde(dir: &str, home: Option<&str>) -> String {
    home.and_then(|home| below(dir, home)).map_or_else(|| dir.to_string(), |rest| format!("~{rest}"))
}

/// A folder for the drawer's foot. In a listed worktree: from the worktree's folder name on
/// (`fix-login`, `fix-login/crates`). Else with the home folder as `~`. Longer than
/// `DIR_CHARS`: its middle is `…`, e.g. `~/code/a-long…/crates/core`.
pub fn short_dir(dir: &str, home: Option<&str>, worktree: Option<&str>) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    let name = |root: &'_ str| root.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_string();
    let inside = worktree.and_then(|root| Some(format!("{}{}", name(root), below(dir, root)?)));
    let text = inside.filter(|text| !text.is_empty()).unwrap_or_else(|| tilde(dir, home));
    let letters: Vec<&str> = text.graphemes(true).collect();
    if letters.len() <= DIR_CHARS {
        return text;
    }
    let head = (DIR_CHARS - 1) / 2;
    let tail = letters.len() - (DIR_CHARS - 1 - head);
    format!("{}…{}", letters[..head].concat(), letters[tail..].concat())
}

/// The drawer's agent line. `model`: the one the CLI reported, else the one it was started
/// with (an id or a runner's name); `mode` as the CLI reported it; `secs` how long it has run,
/// while it runs; `place` where it runs (or would).
pub fn agent_line(model: Option<&str>, mode: Option<&str>, secs: Option<u64>, place: Place) -> AgentLine {
    let model = model.and_then(crate::sessions::short_model);
    let unasked = mode.and_then(|mode| UNASKED.iter().find(|(name, _)| *name == mode)).map(|(_, word)| *word);
    let short = place.dir.map(|dir| short_dir(dir, place.home, place.worktree));
    let foot = [model.as_deref(), unasked, short.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    let elapsed = secs.map(|s| if s < 3600 { format!("{}:{:02}", s / 60, s % 60) } else { format!("{}h", s / 3600) });
    let folder = place.dir.map(|dir| tilde(dir, place.home));
    let facts = [("Model", model.clone()), ("Mode", mode.map(str::to_string)), ("Running", elapsed)];
    let facts = facts.into_iter().chain([("Folder", folder.clone())]);
    AgentLine {
        foot: Some(foot).filter(|foot| !foot.is_empty()),
        bypass: mode == Some("bypassPermissions"),
        folder: folder.filter(|_| place.dir != place.default),
        facts: facts.filter_map(|(name, value)| Some((name, value?))).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_drawer_says_its_model_at_its_foot_and_the_rest_on_hover() {
        let home = Some("/Users/me");
        let app = Place { dir: Some("/Users/me/code/app"), default: Some("/Users/me/code/app"), home, worktree: None };
        // The model in short, then the folder; a mode that asks is the hover's alone; the task's
        // own folder is not in the header.
        let line = agent_line(Some("claude-haiku-5-5"), Some("default"), Some(20), app);
        let foot = (line.foot.as_deref(), line.bypass, line.folder.as_deref());
        assert_eq!(foot, (Some("haiku-5.5 · ~/code/app"), false, None));
        let facts = [("Model", "haiku-5.5"), ("Mode", "default"), ("Running", "0:20"), ("Folder", "~/code/app")];
        assert_eq!(line.facts, facts.map(|(n, v)| (n, v.to_string())));
        // A mode that acts unasked stays in sight; bypass as a warning. Another folder: the header's too.
        let notes = Place { dir: Some("/Users/me/notes"), ..app };
        let line = agent_line(Some("claude-opus-5-5"), Some("auto"), None, notes);
        assert_eq!(
            (line.foot.as_deref(), line.folder.as_deref()),
            (Some("opus-5.5 · auto · ~/notes"), Some("~/notes"))
        );
        // Not started yet, no model known: where it would run is said all the same.
        assert_eq!(agent_line(None, None, None, app).foot.as_deref(), Some("~/code/app"));
        let foot = |mode| agent_line(Some("claude"), Some(mode), None, Place::default());
        assert_eq!(foot("acceptEdits").foot.as_deref(), Some("claude · accept edits"));
        assert_eq!(
            (foot("bypassPermissions").foot.as_deref(), foot("bypassPermissions").bypass),
            (Some("claude · bypass"), true)
        );
        assert_eq!((foot("plan").foot.as_deref(), foot("dontAsk").foot.as_deref()), (Some("claude"), Some("claude")));
        // Ended, nothing known but what it was started with: still said. An hour on: in hours.
        let line = agent_line(None, None, Some(7300), Place::default());
        assert_eq!((line.foot, line.facts), (None, vec![("Running", "2h".to_string())]));
        assert_eq!(agent_line(Some("not a model"), None, None, Place::default()).foot, None);
    }

    #[test]
    fn a_folder_is_said_from_home_or_its_worktree_and_cut_in_the_middle() {
        let home = Some("/Users/me");
        // Under home: `~`. Home itself. Outside it, or with no home known: as it is.
        assert_eq!(short_dir("/Users/me/code/app", home, None), "~/code/app");
        assert_eq!(short_dir("/Users/me", home, None), "~");
        assert_eq!(short_dir("/Users/meg/app", home, None), "/Users/meg/app");
        assert_eq!(short_dir("/srv/app", None, None), "/srv/app");
        assert_eq!(short_dir("/srv/app", Some(""), Some("")), "/srv/app");
        // A worktree: its folder's name, and what is below it. A worktree it is not in: no matter.
        let worktree = Some("/Users/me/.ottospace/worktrees/app/fix-login");
        assert_eq!(short_dir("/Users/me/.ottospace/worktrees/app/fix-login", home, worktree), "fix-login");
        assert_eq!(
            short_dir("/Users/me/.ottospace/worktrees/app/fix-login/crates", home, worktree),
            "fix-login/crates"
        );
        assert_eq!(short_dir("/Users/me/code/app", home, worktree), "~/code/app");
        // Long: the middle is cut, start and end stay, `DIR_CHARS` in all. Whole letters only.
        let long = short_dir("/Users/me/code/a-very-long-repository-name/crates/core", home, None);
        assert_eq!(long, "~/code/a-very…me/crates/core");
        assert_eq!(long.chars().count(), DIR_CHARS);
        let accents = format!("/srv/{}", "e\u{301}".repeat(40));
        let cut = short_dir(&accents, None, None);
        assert!(cut.starts_with("/srv/e\u{301}") && cut.ends_with("e\u{301}") && cut.contains('…'), "{cut}");
    }

    #[test]
    fn a_monogram_takes_whole_letters_of_the_first_two_words() {
        let cases = [
            ("fix the login loop", "FT"),
            ("refactor", "RE"),
            ("  spaced   out ", "SO"),
            ("été à Paris", "ÉÀ"),
            ("👩‍💻 ship it", "👩‍💻S"),
            ("e\u{301}clair", "E\u{301}C"),
            ("x", "X"),
            ("", ""),
        ];
        for (title, want) in cases {
            assert_eq!(monogram(title), want, "{title:?}");
        }
    }

    #[test]
    fn monograms_that_collide_take_the_first_letter_that_differs() {
        let titles = ["fix the login", "fix the logout", "fix tests", "fix the login", "refactor"];
        // `FT` three times: the later ones take the first letter where their titles part.
        assert_eq!(monograms(&titles), ["FT", "FO", "FE", "F4", "RE"]);
        assert_eq!(monograms(&["été", "étage"]), ["ÉT", "ÉA"]);
        assert_eq!(monograms(&[]), Vec::<String>::new());
        // A place already taken as a badge: the next number.
        assert_eq!(monograms(&["a3", "a", "a"]), ["A3", "A", "A4"]);
    }

    #[test]
    fn a_width_is_kept_within_bounds() {
        let asked = [0., 400., 5000., f32::NAN, f32::INFINITY, -1.];
        assert_eq!(asked.map(width), [MIN_WIDTH, 400., MAX_WIDTH, WIDTH, WIDTH, MIN_WIDTH]);
    }
}
