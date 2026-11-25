//! The file explorer pane (`Kind::Explorer { root }`): a checkout's tree, folders opened and
//! closed, a row selected, typed letters going to a name. Pure: the shell lists one folder at a
//! time (`Effect::ListDir`), off the main thread, never outside the root nor through a link, and
//! says git's marks; the state has the shape only, never a name. Design: `docs/ai/file-viewer.md` (D).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// The most entries of one folder listed: past it, the first ones, and the folder says so.
pub const MAX_ENTRIES: usize = 5000;
/// Letters typed within this long of each other go to one name.
pub const TYPED_MS: u64 = 800;

/// A name in a folder, as the shell read it: never followed if a link.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    /// A symbolic link: listed, never opened as a folder nor read through.
    #[serde(default)]
    pub link: bool,
    /// Git ignores it (shown only while ignored files are).
    #[serde(default)]
    pub ignored: bool,
}

/// What git says of a file: changed, new to it, or added.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    Modified,
    Added,
    Untracked,
    Conflicted,
}

/// A folder's listing: being read, read (`cut`: more than `MAX_ENTRIES`), not read, or (the
/// root's) waiting for Repos to list the checkout.
#[derive(Clone, Debug, PartialEq)]
pub enum Listing {
    Reading,
    Read { entries: Vec<Entry>, cut: bool },
    Unread,
    Unlisted,
}

/// One row as drawn: a path from the root (`src/main.rs`), its depth, what it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub path: String,
    pub depth: usize,
    pub dir: bool,
    pub open: bool,
    pub link: bool,
    pub ignored: bool,
    pub mark: Option<Mark>,
    /// A folder open whose listing stops at `MAX_ENTRIES`: it says so.
    pub cut: bool,
}

impl Row {
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

/// An explorer's state, by pane.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Explorer {
    /// Each folder's listing, by its path from the root (`""`: the root).
    pub listings: BTreeMap<String, Listing>,
    /// The folders opened.
    pub open: BTreeSet<String>,
    /// The row selected, by path: it stays on its file as rows come and go.
    pub selected: Option<String>,
    pub show_ignored: bool,
    /// Git's marks, by file path.
    pub marks: BTreeMap<String, Mark>,
    /// Letters typed, and when the last came.
    pub typed: String,
    pub typed_at: u64,
}

impl Explorer {
    /// The rows on show, folders first then files, each by name as a person sorts it.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        self.add_rows("", 0, &mut rows);
        rows
    }

    fn add_rows(&self, dir: &str, depth: usize, rows: &mut Vec<Row>) {
        let Some(Listing::Read { entries, .. }) = self.listings.get(dir) else { return };
        let mut shown: Vec<&Entry> = entries.iter().filter(|e| self.show_ignored || !e.ignored).collect();
        shown.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| natural(&a.name, &b.name)));
        for entry in shown {
            let path = if dir.is_empty() { entry.name.clone() } else { format!("{dir}/{}", entry.name) };
            let folder = entry.dir && !entry.link;
            let open = folder && self.open.contains(&path);
            let mark = if folder { self.folder_mark(&path) } else { self.marks.get(&path).copied() };
            rows.push(Row {
                path: path.clone(),
                depth,
                dir: folder,
                open,
                link: entry.link,
                ignored: entry.ignored,
                mark,
                cut: open && matches!(self.listings.get(&path), Some(Listing::Read { cut: true, .. })),
            });
            if open {
                self.add_rows(&path, depth + 1, rows);
            }
        }
    }

    /// A folder's mark: the strongest of the files under it.
    fn folder_mark(&self, dir: &str) -> Option<Mark> {
        let under = format!("{dir}/");
        self.marks.range(under.clone()..).take_while(|(path, _)| path.starts_with(&under)).map(|(_, m)| *m).max()
    }

    /// The selected row's place among `rows`.
    pub fn selected_at(&self, rows: &[Row]) -> Option<usize> {
        let selected = self.selected.as_deref()?;
        rows.iter().position(|row| row.path == selected)
    }
}

/// Names compared as a person reads them: case aside, numbers by value (`2` before `10`), of
/// any length (by their digits' count, leading zeros aside, then the digits).
pub fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let key = |s: &str| {
        let mut parts: Vec<(String, (usize, String))> = Vec::new();
        let mut text = String::new();
        let mut digits = String::new();
        let number = |digits: &str| {
            let value = digits.trim_start_matches('0');
            (value.len(), value.to_string())
        };
        for c in s.chars().flat_map(char::to_lowercase) {
            if c.is_ascii_digit() {
                digits.push(c);
            } else {
                if !digits.is_empty() {
                    parts.push((std::mem::take(&mut text), number(&digits)));
                    digits.clear();
                }
                text.push(c);
            }
        }
        parts.push((text, number(&digits)));
        parts
    };
    key(a).cmp(&key(b)).then_with(|| a.cmp(b))
}

/// A name the shell may list: one path part, no control character, not `.git` (git's own).
pub fn listable(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name != ".git"
        && !name.contains('/')
        && !name.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, dir: bool) -> Entry {
        Entry { name: name.into(), dir, link: false, ignored: false }
    }

    fn read(entries: Vec<Entry>) -> Listing {
        Listing::Read { entries, cut: false }
    }

    #[test]
    fn rows_are_folders_first_by_natural_name_and_open_folders_show_theirs() {
        let mut e = Explorer::default();
        e.listings.insert("".into(), read(vec![entry("b.rs", false), entry("src", true), entry("A.md", false)]));
        e.listings.insert("src".into(), read(vec![entry("f10.rs", false), entry("f2.rs", false)]));
        let names = |e: &Explorer| e.rows().into_iter().map(|row| row.path).collect::<Vec<_>>();
        assert_eq!(names(&e), ["src", "A.md", "b.rs"]);
        e.open.insert("src".into());
        assert_eq!(names(&e), ["src", "src/f2.rs", "src/f10.rs", "A.md", "b.rs"]);
        // A number past any integer still sorts after a small one (review).
        assert_eq!(natural("f18446744073709551616", "f2"), std::cmp::Ordering::Greater);
        assert_eq!(e.rows()[1].depth, 1);
    }

    #[test]
    fn ignored_files_show_only_when_asked_and_a_link_never_opens() {
        let mut e = Explorer::default();
        let mut target = entry("target", true);
        target.ignored = true;
        let mut out = entry("elsewhere", true);
        out.link = true;
        e.listings.insert("".into(), read(vec![target, out, entry("a.rs", false)]));
        e.open.insert("elsewhere".into());
        let rows = e.rows();
        assert_eq!(rows.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(), ["elsewhere", "a.rs"]);
        assert!(!rows[0].dir && !rows[0].open && rows[0].link);
        e.show_ignored = true;
        assert_eq!(e.rows().len(), 3);
    }

    #[test]
    fn a_folder_carries_the_strongest_mark_under_it() {
        let mut e = Explorer::default();
        e.listings.insert("".into(), read(vec![entry("src", true), entry("srcs.rs", false)]));
        e.marks.insert("src/a.rs".into(), Mark::Untracked);
        e.marks.insert("src/deep/b.rs".into(), Mark::Conflicted);
        e.marks.insert("srcs.rs".into(), Mark::Modified);
        let rows = e.rows();
        assert_eq!((rows[0].mark, rows[1].mark), (Some(Mark::Conflicted), Some(Mark::Modified)));
    }

    #[test]
    fn only_one_plain_name_is_listed() {
        assert!(listable("main.rs") && listable(".env"));
        for bad in ["", ".", "..", ".git", "a/b", "a\nb"] {
            assert!(!listable(bad), "{bad:?}");
        }
    }
}
