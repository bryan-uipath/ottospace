//! A file's strata in the code city, as the owner's city stacks them: a level for each commit of
//! the past year that touched it, the newest at the base, each in the colour of its kind of change
//! (a conventional commit's type: `feat` cyan, `fix` red, …), brighter the newer; a commit of no
//! kind it knows in the gradient of age, old violet to new cyan. From the city's one `git log`
//! (`history`); a subject is only sorted into a kind, never kept.

use std::collections::HashMap;

/// A kind of change, by a conventional commit's type (`feat:`, `fix(ui):`), as the owner's city
/// colours them: aliases share a colour (`bug`, `bugfix`, `hotfix` are `Fix`; `build` is `Ci`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Feat,
    Fix,
    Refactor,
    Chore,
    Docs,
    Test,
    Perf,
    Ci,
}

impl Change {
    /// The owner's city's colour of it.
    fn hue(self) -> u32 {
        match self {
            Change::Feat => 0x22d3ee,
            Change::Fix => 0xef4444,
            Change::Refactor => 0xa78bfa,
            Change::Chore => 0x64748b,
            Change::Docs => 0x4ade80,
            Change::Test => 0xfbbf24,
            Change::Perf => 0xf472b6,
            Change::Ci => 0x52525b,
        }
    }
}

/// A commit as one of a file's levels: when (seconds since 1970) and its kind, if known.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level {
    pub at: u64,
    pub change: Option<Change>,
}

/// Each path's levels, newest first (as git gives them), and the oldest and newest commit's time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    pub files: HashMap<String, Vec<Level>>,
    pub span: (u64, u64),
}

/// From `git log -z --name-only --format=%x01%ct%x09%s`: each commit a `\x01`, its time, a tab
/// and its subject, then a new line and the paths it changed, each ended by a NUL.
pub fn history(log: &str) -> History {
    let mut history = History { files: HashMap::new(), span: (u64::MAX, 0) };
    let (mut commit, mut first): (Option<Level>, bool) = (None, false);
    for said in log.split('\0') {
        // The new line is git's, before a commit's first path only.
        let said = if std::mem::take(&mut first) { said.strip_prefix('\n').unwrap_or(said) } else { said };
        if let Some(head) = said.strip_prefix('\x01') {
            first = true;
            let (at, subject) = head.split_once('\t').unwrap_or((head, ""));
            commit = at.parse().ok().map(|at| Level { at, change: change(subject) });
        } else if let Some(level) = commit.filter(|_| !said.is_empty()) {
            // Only a commit with a path counts for the span: a log cut short ends in a header.
            history.span = (history.span.0.min(level.at), history.span.1.max(level.at));
            history.files.entry(said.to_string()).or_default().push(level);
        }
    }
    // Newest first by time, as the owner's city sorts them: a commit's clock may be behind its parent's.
    // No more than a file stands, the newest: a city draws no more.
    for levels in history.files.values_mut() {
        levels.sort_by(|a, b| b.at.cmp(&a.at));
        levels.truncate(crate::scene::LEVELS as usize);
    }
    if history.files.is_empty() {
        history.span = (0, 0);
    }
    history
}

/// A conventional commit's type (`type(scope)!: subject`, its case ignored), as the owner's city
/// reads it. A subject with none that says it fixes (`fix`, `fixes`, `fixed`, `bug`, `bugfix`,
/// `hotfix`, as a word) is a fix. A type it has no colour for (`style:`) is none.
fn change(subject: &str) -> Option<Change> {
    let word = subject.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).map_or(subject, |end| &subject[..end]);
    let rest = &subject[word.len()..];
    let rest = rest.strip_prefix('(').and_then(|scope| Some(&scope[scope.find(')')? + 1..])).unwrap_or(rest);
    let typed = !word.is_empty() && rest.strip_prefix('!').unwrap_or(rest).starts_with(':');
    if !typed {
        let fixes =
            |w: &str| ["fix", "fixes", "fixed", "bug", "bugfix", "hotfix"].iter().any(|f| f.eq_ignore_ascii_case(w));
        return subject.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).any(fixes).then_some(Change::Fix);
    }
    Some(match word.to_ascii_lowercase().as_str() {
        "feat" => Change::Feat,
        "fix" | "bug" | "bugfix" | "hotfix" => Change::Fix,
        "refactor" => Change::Refactor,
        "chore" => Change::Chore,
        "docs" => Change::Docs,
        "test" => Change::Test,
        "perf" => Change::Perf,
        "ci" | "build" => Change::Ci,
        _ => return None,
    })
}

/// A level's colour, `age` (0 the oldest commit of the history, 1 the newest) through it: its
/// kind's, `0.55 + 0.6·age` as bright; with none, old violet to new cyan, `0.5 + 0.6·age` as
/// bright. As the owner's city paints its strata.
pub fn colour(change: Option<Change>, age: f32) -> u32 {
    let age = age.clamp(0., 1.);
    let (hue, bright) = match change {
        Some(change) => (change.hue(), 0.55 + 0.6 * age),
        None => (mix(0x4c1d95, 0x22d3ee, age), 0.5 + 0.6 * age),
    };
    channels(hue, |c| c * bright)
}

fn channels(hex: u32, f: impl Fn(f32) -> f32) -> u32 {
    [16, 8, 0].iter().fold(0, |out, shift| {
        let c = ((hex >> shift) & 0xff) as f32;
        out | (f(c).round().clamp(0., 255.) as u32) << shift
    })
}

fn mix(a: u32, b: u32, t: f32) -> u32 {
    [16, 8, 0].iter().fold(0, |out, shift| {
        let (x, y) = (((a >> shift) & 0xff) as f32, ((b >> shift) & 0xff) as f32);
        out | ((x + (y - x) * t).round() as u32) << shift
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Past the most a file stands, the newest by time are kept, though git gave one last.
    #[test]
    fn a_file_keeps_its_newest_levels_by_time() {
        let mut log: String = (1..=120).rev().map(|at| format!("\x01{at}\tx\0\na.rs\0")).collect();
        log.push_str("\x01500\tfix: late clock\0\na.rs\0");
        let levels = &history(&log).files["a.rs"];
        assert_eq!((levels.len(), levels[0].at, levels[119].at), (120, 500, 2));
    }

    #[test]
    fn a_log_gives_each_files_levels_newest_first_and_the_span() {
        let log = "\x01300\tfeat(ui)!: a\0\na.rs\0b.rs\0\x01200\tWIP\0\x01100\tfix the loop\0\na.rs\0\x01x\tbad\0\nc.rs\0\x0117";
        let h = history(log);
        let at =
            |path: &str| h.files.get(path).map(|levels| levels.iter().map(|l| (l.at, l.change)).collect::<Vec<_>>());
        assert_eq!(at("a.rs"), Some(vec![(300, Some(Change::Feat)), (100, Some(Change::Fix))]));
        assert_eq!(at("b.rs"), Some(vec![(300, Some(Change::Feat))]));
        // A commit whose time is not a number counts for nothing; nor does a header a cut log
        // ends in (`17`), or a commit with no path (`WIP`).
        assert_eq!((at("c.rs"), h.span), (None, (100, 300)));
        // A commit behind its parent's clock still lies by its time; a file stands no more than 120.
        let skewed = history("\x01100\tfeat: child\0\na.rs\0\x01101\tfix: parent\0\na.rs\0");
        let order: Vec<u64> = skewed.files["a.rs"].iter().map(|l| l.at).collect();
        assert_eq!(order, [101, 100]);
        let many: String = (0..200).map(|i| format!("\x01{}\tx\0\na.rs\0", 1000 - i)).collect();
        assert_eq!(history(&many).files["a.rs"].len(), crate::scene::LEVELS as usize);
        assert_eq!(history(""), History::default());
    }

    /// As the owner's city reads a subject: `type(scope)!:`, aliases sharing a colour; a type it
    /// has no colour for is none; with no type, a subject that says it fixes is a fix.
    #[test]
    fn a_subject_gives_its_kind_of_change() {
        let cases = [
            ("feat: a", Some(Change::Feat)),
            ("Fix(core): a", Some(Change::Fix)),
            ("hotfix!: a", Some(Change::Fix)),
            ("build: a", Some(Change::Ci)),
            ("docs(readme)!: a", Some(Change::Docs)),
            ("style: a", None),
            ("Fixes the login loop", Some(Change::Fix)),
            ("a prefix: debugging", None),
            ("Code city: lines", None),
            ("", None),
        ];
        for (subject, want) in cases {
            assert_eq!(change(subject), want, "{subject}");
        }
    }

    #[test]
    fn a_levels_colour_is_its_kinds_brighter_the_newer_else_the_age_gradient() {
        assert_eq!(colour(Some(Change::Fix), 0.), channels(0xef4444, |c| c * 0.55));
        assert_eq!(colour(Some(Change::Feat), 1.), channels(0x22d3ee, |c| c * 1.15));
        assert_eq!(colour(None, 0.), channels(0x4c1d95, |c| c * 0.5));
        assert_eq!(colour(None, 1.), channels(0x22d3ee, |c| c * 1.1));
    }
}
