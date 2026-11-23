//! The Projects section (`projects` flag, `docs/ai/projects.md`): each project in the folder the
//! settings' `projects_dir` names, as the shell last read it. ottospace only reads that folder.

use serde::Serialize;

/// The most projects listed, and the most entries looked at in a project's folder: a folder is
/// input, not trusted to be small.
pub const MAX_PROJECTS: usize = 500;
pub const MAX_ENTRIES: usize = 200;
/// The most of a note read for its title; a larger note is titled by its name.
pub const MAX_NOTE_BYTES: u64 = 256 * 1024;
/// The longest title kept, in characters.
const MAX_TITLE: usize = 120;
/// While the section is open the folder is read again this often; on its opening at once.
pub const READ_EVERY_MS: u64 = 10_000;

/// One project: a folder of the projects folder, or a lone `.md` file in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    /// Its folder, or the `.md` file that is the project: what names it.
    pub path: String,
    /// The note's first heading, else the name (`title_of`). The user's: never in the state.
    pub title: String,
    /// The note (`note_of`), if it has one.
    pub note: Option<String>,
    /// The last change to it or its note, ms since 1970.
    pub modified: u64,
}

/// What the Projects section lists, and in which order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Projects {
    /// The settings name a projects folder: the section shows (with the flag on).
    pub on: bool,
    /// As last read, in no order.
    pub listed: Vec<Project>,
    /// Paths the user dragged into place, in their order.
    pub placed: Vec<String>,
}

/// A row of the section as the state gives it: never a title or a path.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Row {
    /// The user placed it (dragged): it comes before the rest.
    pub placed: bool,
    /// A lone `.md` file, not a folder.
    pub file: bool,
    /// It has a note.
    pub note: bool,
    pub modified: u64,
}

impl Projects {
    /// The listed projects as the section shows them: those the user placed, in their order;
    /// then the rest, the most recently changed first (title, then path, on a tie).
    pub fn ordered(&self) -> Vec<&Project> {
        let placed = self.placed.iter().filter_map(|path| self.listed.iter().find(|p| &p.path == path));
        let mut rest: Vec<&Project> = self.listed.iter().filter(|p| !self.placed.contains(&p.path)).collect();
        rest.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.title.cmp(&b.title)).then(a.path.cmp(&b.path)));
        placed.chain(rest).collect()
    }

    /// The rows the state gives, in the section's order.
    pub fn rows(&self) -> Vec<Row> {
        let row = |p: &Project| Row {
            placed: self.placed.contains(&p.path),
            file: p.note.as_deref() == Some(p.path.as_str()),
            note: p.note.is_some(),
            modified: p.modified,
        };
        self.ordered().into_iter().map(row).collect()
    }

    /// A read of the folder: at most `MAX_PROJECTS`, each path once.
    pub fn set(&mut self, mut listed: Vec<Project>) {
        listed.truncate(MAX_PROJECTS);
        let mut seen = std::collections::BTreeSet::new();
        listed.retain(|p| seen.insert(p.path.clone()));
        self.listed = listed;
    }
}

/// Whether the folder is read again now: while the section is open, on its opening and then
/// every `READ_EVERY_MS`. Never while it is closed. `last`, `now`: ms.
pub fn due(open: bool, opened: bool, last: Option<u64>, now: u64) -> bool {
    open && (opened || last.is_none_or(|at| now.saturating_sub(at) >= READ_EVERY_MS))
}

/// Which of a project folder's files is its note, by name: `README.md`, else `index.md` (any
/// case), else its only `.md`, else the most recently changed `.md`. `files`: name and ms.
pub fn note_of(files: &[(String, u64)]) -> Option<usize> {
    let md = |name: &str| md_stem(name).is_some();
    let named = |want: &str| files.iter().position(|(name, _)| name.eq_ignore_ascii_case(want));
    let newest = files.iter().enumerate().filter(|(_, (name, _))| md(name)).max_by_key(|(_, (name, at))| (*at, name));
    named("README.md").or_else(|| named("index.md")).or(newest.map(|(at, _)| at))
}

/// A project's title: its note's first heading (`# Title`, any level; not in front matter or a
/// code block), else its name without `.md`. e.g. `checkout-redesign.md` → `checkout-redesign`.
pub fn title_of(note: Option<&str>, name: &str) -> String {
    let heading = note.and_then(first_heading).filter(|title| !title.is_empty());
    let name = md_stem(name).unwrap_or(name);
    let title = heading.unwrap_or_else(|| name.to_string());
    let title: String = title.chars().filter(|c| !c.is_control()).collect();
    crate::ax::truncate(title.trim(), MAX_TITLE)
}

/// A Markdown file's name without `.md`, in any case: `Plan.Md` → `Plan`. None: not one.
pub fn md_stem(name: &str) -> Option<&str> {
    let at = name.len().checked_sub(3)?;
    (name.is_char_boundary(at) && name[at..].eq_ignore_ascii_case(".md")).then(|| &name[..at])
}

/// The first ATX heading's words, past a front matter block (`---` … `---` at the top) and
/// outside fenced code.
fn first_heading(text: &str) -> Option<String> {
    let mut lines = text.lines().peekable();
    if lines.peek().is_some_and(|line| line.trim_end() == "---") {
        lines.next();
        for line in lines.by_ref() {
            if line.trim_end() == "---" {
                break;
            }
        }
    }
    // The open fence's character and length: only a run as long of it closes it.
    let mut fence: Option<(char, usize)> = None;
    for line in lines {
        let line = line.trim_start();
        let mark = line.chars().next().filter(|c| *c == '`' || *c == '~');
        let run = mark.map_or(0, |c| line.chars().take_while(|x| *x == c).count());
        match (fence, mark) {
            (None, Some(c)) if run >= 3 => fence = Some((c, run)),
            (Some((open, len)), Some(c)) if c == open && run >= len && line[run..].trim().is_empty() => fence = None,
            _ => {}
        }
        if fence.is_some() || run >= 3 {
            continue;
        }
        let hashes = line.chars().take_while(|c| *c == '#').count();
        let rest = &line[hashes..];
        if (1..=6).contains(&hashes) && (rest.is_empty() || rest.starts_with([' ', '\t'])) {
            return Some(closing_hashes_off(rest.trim()).to_string());
        }
    }
    None
}

/// A heading's text without its closing `#`s, which count only after a space: `C# ##` → `C#`.
fn closing_hashes_off(text: &str) -> &str {
    let kept = text.trim_end_matches('#');
    match kept.is_empty() || kept.ends_with([' ', '\t']) {
        true => kept.trim_end(),
        false => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(path: &str, title: &str, modified: u64) -> Project {
        Project { path: path.into(), title: title.into(), note: None, modified }
    }

    fn order(projects: &Projects) -> Vec<&str> {
        projects.ordered().into_iter().map(|p| p.path.as_str()).collect()
    }

    #[test]
    fn placed_projects_come_first_in_the_users_order_then_the_most_recent() {
        let listed =
            vec![project("/p/a", "A", 10), project("/p/b", "B", 30), project("/p/c", "C", 20), project("/p/d", "D", 5)];
        let mut projects = Projects { on: true, listed, placed: Vec::new() };
        assert_eq!(order(&projects), ["/p/b", "/p/c", "/p/a", "/p/d"]);
        // Placed, an old one leads; one placed that is gone is skipped.
        projects.placed = vec!["/p/d".into(), "/p/gone".into(), "/p/a".into()];
        assert_eq!(order(&projects), ["/p/d", "/p/a", "/p/b", "/p/c"]);
        let rows = projects.rows();
        assert_eq!(rows.iter().map(|r| r.placed).collect::<Vec<_>>(), [true, true, false, false]);
    }

    #[test]
    fn a_tie_goes_by_title_then_path() {
        let listed = vec![project("/p/z", "Same", 1), project("/p/y", "Same", 1), project("/p/x", "Alpha", 1)];
        assert_eq!(order(&Projects { on: true, listed, placed: Vec::new() }), ["/p/x", "/p/y", "/p/z"]);
    }

    #[test]
    fn a_read_keeps_each_path_once_and_at_most_the_limit() {
        let mut projects = Projects::default();
        let mut listed: Vec<Project> = (0..MAX_PROJECTS + 5).map(|i| project(&format!("/p/{i}"), "t", 0)).collect();
        listed.insert(1, project("/p/0", "again", 9));
        projects.set(listed);
        assert_eq!(projects.listed.len(), MAX_PROJECTS - 1);
        assert_eq!(projects.listed[0].title, "t");
    }

    #[test]
    fn the_note_is_the_readme_else_index_else_the_newest_markdown() {
        let files = |names: &[(&str, u64)]| names.iter().map(|(n, at)| (n.to_string(), *at)).collect::<Vec<_>>();
        assert_eq!(note_of(&files(&[("a.md", 9), ("readme.MD", 1), ("index.md", 5)])), Some(1));
        assert_eq!(note_of(&files(&[("a.md", 9), ("Index.md", 1)])), Some(1));
        assert_eq!(note_of(&files(&[("mock.png", 9), ("plan.md", 1)])), Some(1));
        assert_eq!(note_of(&files(&[("old.md", 1), ("new.md", 7), ("pic.png", 9)])), Some(1));
        assert_eq!(note_of(&files(&[("pic.png", 9)])), None);
    }

    #[test]
    fn the_title_is_the_first_heading_else_the_name() {
        assert_eq!(title_of(Some("intro\n## Offline sync ##\n# Later"), "x"), "Offline sync");
        let front = "---\ntitle: no\n# not this\n---\n# Launch docs\n";
        assert_eq!(title_of(Some(front), "x"), "Launch docs");
        assert_eq!(title_of(Some("```\n# a comment\n```\n#hashtag\n# Real"), "x"), "Real");
        assert_eq!(title_of(Some("no heading"), "hiring-loop.md"), "hiring-loop");
        assert_eq!(title_of(Some("#   \n"), "spike"), "spike");
        assert_eq!(title_of(None, "rate-limiter"), "rate-limiter");
        assert_eq!(title_of(Some("# a\u{7}b"), "x"), "ab");
        assert_eq!(title_of(Some("# Learn C#"), "x"), "Learn C#");
        assert_eq!(title_of(Some("# Learn C# ##"), "x"), "Learn C#");
        assert_eq!(title_of(Some("````\n```\n# Example\n````\n# Release"), "x"), "Release");
        assert_eq!(title_of(None, "Plan.Md"), "Plan");
        assert_eq!(md_stem("é.md"), Some("é"));
        assert_eq!(md_stem("md"), None);
        assert!(title_of(Some(&format!("# {}", "w".repeat(500))), "x").chars().count() <= MAX_TITLE + 1);
    }

    #[test]
    fn the_section_shows_only_with_a_folder_set_and_the_flag_on() {
        use crate::features::{Feature, Features};
        use crate::list::ListRow;
        use crate::model::{Action, Model, Origin, Section};
        let mut m = Model::default();
        let open = Action::ToggleSection { section: Section::Projects };
        m.update(open.clone());
        assert!(!m.sections().contains(&Section::Projects) && m.sidebar.is_none());
        let listed = vec![project("/p/old", "Old", 1), project("/p/new", "New", 2)];
        m.update(Action::SetProjects { on: true, listed }.sent_by(Origin::System));
        assert_eq!(m.sections()[..3], [Section::Repos, Section::Projects, Section::Tasks]);
        m.update(open.clone());
        assert_eq!(m.sidebar, Some(Section::Projects));
        let rows = [ListRow::ProjectEntry("/p/new".into()), ListRow::ProjectEntry("/p/old".into())];
        assert_eq!(m.list_rows(), rows);
        // The setting gone: the section closes.
        m.update(Action::SetProjects { on: false, listed: Vec::new() }.sent_by(Origin::System));
        assert!(m.sidebar.is_none() && !m.sections().contains(&Section::Projects));
        // The flag off: no cell, and opening does nothing.
        m.update(Action::SetProjects { on: true, listed: Vec::new() }.sent_by(Origin::System));
        m.update(
            Action::SetFeatures { features: Features::all().with(Feature::Projects, false) }.sent_by(Origin::System),
        );
        m.update(open);
        assert!(!m.sections().contains(&Section::Projects) && m.sidebar.is_none());
    }

    #[test]
    fn the_folder_is_read_on_opening_and_every_ten_seconds_while_open() {
        assert!(due(true, true, Some(5), 6));
        assert!(due(true, false, None, 0));
        assert!(!due(true, false, Some(1_000), 1_000 + READ_EVERY_MS - 1));
        assert!(due(true, false, Some(1_000), 1_000 + READ_EVERY_MS));
        assert!(!due(false, false, None, 0));
    }
}
