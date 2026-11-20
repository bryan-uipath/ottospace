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
        self.rows = match folder.and_then(|folder| self.files_of(folder)) {
            Some(Files::Read { paths, .. }) => file_matches(paths, query),
            _ => Vec::new(),
        };
    }
}

/// The paths `query` runs through, best first, then shorter, then in their order; at most
/// `MOST_ROWS`. Nothing typed: the shortest.
pub fn file_matches(paths: &[String], query: &str) -> Vec<String> {
    let query = letters(query);
    let mut found: Vec<(i64, usize, usize)> =
        paths.iter().enumerate().filter_map(|(at, path)| Some((score(&query, path)?, path.len(), at))).collect();
    found.sort_by_key(|(score, len, at)| (-score, *len, *at));
    found.into_iter().take(MOST_ROWS).map(|(.., at)| paths[at].clone()).collect()
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

    #[test]
    fn files_rank_by_score_then_length_and_stop_at_most_rows() {
        let paths: Vec<String> = ["src/main.rs", "README.md", "src/model/main_window.rs", "docs/m.md"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(file_matches(&paths, "main"), ["src/main.rs", "src/model/main_window.rs"]);
        // Nothing typed: the shortest first, top-level files before deep ones.
        assert_eq!(file_matches(&paths, "")[..2], ["README.md", "docs/m.md"]);
        let many: Vec<String> = (0..MOST_ROWS * 2).map(|n| format!("f{n}.rs")).collect();
        assert_eq!(file_matches(&many, "f").len(), MOST_ROWS);
    }
}
