//! Go to (⌘P): one list of places (workspaces, checkouts, tasks), then a file in a place's
//! folder. Ranked by the command palette's scorer (`commands::score`). The model builds the
//! places and acts on a choice (`model/goto.rs`); this is the list and its ranking.

use crate::commands::{letters, score};
use crate::tasks::TaskId;
use crate::workbook::WorkbookId;

/// What a match by detail alone is behind: every match by name, however long the query.
const DETAIL: i64 = 1 << 40;

/// The files listed at most: the best this many of a query's matches.
pub const MOST_ROWS: usize = 200;
/// What a match in a file's name is ahead of: any match in its folders alone. `src/model.rs`
/// for "model" before `model/tests/a.rs`.
const IN_NAME: i64 = 1 << 30;
/// A name that starts with the query, ahead of one it runs through: "ma" `main.rs` before `format.rs`.
const NAME_START: i64 = 1 << 24;
/// A file opened recently, ahead of others of its kind of match, the latest most.
const RECENT: i64 = 1 << 16;
/// Recent files kept, newest first.
pub const MOST_RECENT: usize = 50;

/// Where a row goes: as its section's own row does.
#[derive(Clone, Debug, PartialEq)]
pub enum Place {
    Workspace(WorkbookId),
    /// A listed repository's own folder or one of its worktrees, by path.
    Checkout(String),
    Task(TaskId),
}

/// A place as listed: its name, a muted detail (a folder, a repository, a task's state), and the
/// folder its files come from, if it has one.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaceRow {
    pub place: Place,
    pub name: String,
    pub detail: String,
    pub folder: Option<String>,
    /// The place on show: listed last, and a file opens there without going anywhere.
    pub current: bool,
}

/// A folder's files: being read, read (`cut`: there were more than were listed), or not read and why.
#[derive(Clone, Debug, PartialEq)]
pub enum Files {
    Reading,
    Read { paths: Vec<String>, cut: bool },
    Unread(&'static str),
}

/// Drilled into a place: its files are listed. `back`: the place query to return to.
#[derive(Clone, Debug, PartialEq)]
pub struct Drill {
    pub row: PlaceRow,
    pub back: String,
    /// ⌘P opened on it (the checkout on show): Esc closes, ⌫ with nothing typed lists the places.
    pub here: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GoTo {
    /// In the order shown with nothing typed: most recently used first, the current place last.
    pub places: Vec<PlaceRow>,
    pub drill: Option<Drill>,
    /// Each folder's files, read once while the palette is open.
    pub files: Vec<(String, Files)>,
    /// The drilled folder's files the query matches, best first, at most `MOST_ROWS`.
    pub rows: Vec<String>,
    /// The files opened recently, newest first, as paths: ranked ahead in a folder that holds them.
    pub recent: Vec<String>,
}

impl GoTo {
    /// The places `query` matches, best first, each with its score: by name, then by name and
    /// detail behind every name match. Nothing typed: all, in their order. Ties keep it.
    pub fn place_matches(&self, query: &str) -> Vec<(i64, &PlaceRow)> {
        let query = letters(query);
        let scored = |row: &PlaceRow| {
            let named = score(&query, &row.name);
            named.or_else(|| score(&query, &format!("{} {}", row.name, row.detail)).map(|s| s - DETAIL))
        };
        let mut found: Vec<(i64, &PlaceRow)> = self.places.iter().filter_map(|row| Some((scored(row)?, row))).collect();
        found.sort_by_key(|(score, _)| -score);
        found
    }

    pub fn files_of(&self, folder: &str) -> Option<&Files> {
        self.files.iter().find(|(of, _)| of == folder).map(|(_, files)| files)
    }

    /// Ranks the drilled folder's files for `query` again.
    pub fn refresh(&mut self, query: &str) {
        let folder = self.drill.as_ref().and_then(|drill| drill.row.folder.as_deref());
        self.rows = match folder.and_then(|folder| Some((folder, self.files_of(folder)?))) {
            Some((folder, Files::Read { paths, .. })) => {
                let inside = format!("{}/", folder.trim_end_matches('/'));
                let recent: Vec<&str> = self.recent.iter().filter_map(|path| path.strip_prefix(&inside)).collect();
                file_matches(paths, query, &recent)
            }
            _ => Vec::new(),
        };
    }
}

/// The paths `query` runs through, best first: a match in the file's name before one in its
/// folders, a name it starts before one it runs through, then by the scorer (word starts and
/// runs), a recent file first, then shorter, then in their order; at most `MOST_ROWS`. Nothing
/// typed: the recent files, then the shortest.
pub fn file_matches(paths: &[String], query: &str, recent: &[&str]) -> Vec<String> {
    let query = letters(query);
    let ranked = |path: &String| {
        let at = recent.iter().position(|seen| *seen == path.as_str());
        let bonus = at.map_or(0, |at| RECENT * (MOST_RECENT.saturating_sub(at)) as i64);
        file_score(&query, path).map(|score| score + bonus)
    };
    let mut found: Vec<(i64, usize, usize)> =
        paths.iter().enumerate().filter_map(|(at, path)| Some((ranked(path)?, path.len(), at))).collect();
    let most = MOST_ROWS.min(found.len());
    let key = |&(score, len, at): &(i64, usize, usize)| (-score, len, at);
    // Only the best are listed: no need to sort the rest of 50,000.
    if most > 0 && most < found.len() {
        found.select_nth_unstable_by_key(most - 1, key);
        found.truncate(most);
    }
    found.sort_unstable_by_key(key);
    found.into_iter().map(|(.., at)| paths[at].clone()).collect()
}

/// How well `query` matches a file at `path`: in its name (`IN_NAME`, more if the name starts
/// with it), else in the whole path. `None`: it does not run through the path.
fn file_score(query: &[char], path: &str) -> Option<i64> {
    if !runs_through(query, path) {
        return None;
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    let named = runs_through(query, name).then(|| score(query, name)).flatten();
    let start = |query: &[char]| {
        let mut letters = name.chars().flat_map(char::to_lowercase);
        query.iter().all(|q| letters.next() == Some(*q))
    };
    match named {
        Some(named) => Some(named + IN_NAME + if start(query) { NAME_START } else { 0 }),
        None => score(query, path),
    }
}

/// Whether `query`'s letters are in `text` in order, case aside: a cheap test before the scorer.
fn runs_through(query: &[char], text: &str) -> bool {
    let mut letters = text.chars().flat_map(char::to_lowercase);
    query.iter().all(|q| letters.any(|c| c == *q))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, detail: &str) -> PlaceRow {
        let place = Place::Checkout(format!("/code/{name}"));
        PlaceRow { place, name: name.into(), detail: detail.into(), folder: None, current: false }
    }

    fn names(goto: &GoTo, query: &str) -> Vec<String> {
        goto.place_matches(query).into_iter().map(|(_, row)| row.name.clone()).collect()
    }

    #[test]
    fn a_name_match_ranks_above_a_detail_match_and_nothing_typed_keeps_the_order() {
        let places = vec![row("docs", "app · main"), row("app", "~/code/app · main"), row("fix-login", "app · fix")];
        let goto = GoTo { places, ..GoTo::default() };
        assert_eq!(names(&goto, ""), ["docs", "app", "fix-login"]);
        // "app" names one; the others hold it in their detail, behind it.
        assert_eq!(names(&goto, "app"), ["app", "docs", "fix-login"]);
        assert_eq!(names(&goto, "fxl"), ["fix-login"]);
        assert!(names(&goto, "zzz").is_empty());
    }

    fn strings(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| path.to_string()).collect()
    }

    #[test]
    fn files_rank_by_score_then_length_and_stop_at_most_rows() {
        let paths = strings(&["src/main.rs", "README.md", "src/model/main_window.rs", "docs/m.md"]);
        assert_eq!(file_matches(&paths, "main", &[]), ["src/main.rs", "src/model/main_window.rs"]);
        // Nothing typed: the shortest first, top-level files before deep ones.
        assert_eq!(file_matches(&paths, "", &[])[..2], ["README.md", "docs/m.md"]);
        let many: Vec<String> = (0..MOST_ROWS * 2).map(|n| format!("f{n}.rs")).collect();
        assert_eq!(file_matches(&many, "f", &[]).len(), MOST_ROWS);
        // The best of many, in order: the same as a full sort.
        assert_eq!(file_matches(&many, "f1", &[])[..3], ["f1.rs", "f10.rs", "f11.rs"]);
    }

    #[test]
    fn a_match_in_the_name_beats_one_in_the_folders_and_a_name_start_beats_both() {
        let paths = strings(&["model/tests/a.rs", "src/model.rs", "src/remodel.rs", "app/mod_el.rs"]);
        // "model" in the folders alone comes last; the name that starts with it first.
        let found = file_matches(&paths, "model", &[]);
        assert_eq!((found[0].as_str(), found[3].as_str()), ("src/model.rs", "model/tests/a.rs"));
        // A path query: through the folders and the name.
        assert_eq!(file_matches(&paths, "tests/a", &[]), ["model/tests/a.rs"]);
        assert_eq!(file_matches(&paths, "MODEL", &[])[0], "src/model.rs");
        assert!(file_matches(&paths, "zzz", &[]).is_empty());
    }

    #[test]
    fn a_recent_file_comes_first_among_its_kind_and_first_with_nothing_typed() {
        let paths = strings(&["src/a.rs", "src/app.rs", "docs/deep/apple.md"]);
        assert_eq!(file_matches(&paths, "", &["docs/deep/apple.md"])[0], "docs/deep/apple.md");
        assert_eq!(file_matches(&paths, "ap", &["docs/deep/apple.md"]), ["docs/deep/apple.md", "src/app.rs"]);
        // A recent file whose match is only in its folders stays behind a match in a name.
        assert_eq!(file_matches(&paths, "src", &["src/a.rs"])[..2], ["src/a.rs", "src/app.rs"]);
    }

    /// Typed one letter at a time over 50,000 paths: each keystroke's ranking, in milliseconds.
    /// `cargo test -p ottospace-core --release -- --ignored fifty_thousand --nocapture`
    #[test]
    #[ignore = "a measure, not a check"]
    fn fifty_thousand_files_rank_as_you_type() {
        let words = ["src", "model", "views", "tests", "components", "util", "api", "lib", "core", "shell"];
        let paths: Vec<String> = (0..50_000)
            .map(|n| format!("{}/{}/{}_{n}.rs", words[n % 10], words[(n / 10) % 10], words[(n / 100) % 10]))
            .collect();
        for query in ["m", "mo", "mod", "mode", "model", "model/v", "model/vie", "model/views_12"] {
            let start = std::time::Instant::now();
            let rows = file_matches(&paths, query, &[]);
            println!("{query:>16}: {:>6.1} ms, {} rows", start.elapsed().as_secs_f64() * 1000., rows.len());
        }
    }
}
