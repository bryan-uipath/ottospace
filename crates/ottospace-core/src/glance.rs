//! What an agent looked at, for the code city's brief effects: a read (its file) and a search
//! (where it searched; the files its result names). From a call's words only, best-effort and
//! bounded: what a shell pipeline really reads cannot be known.

use std::collections::VecDeque;

use crate::activity::Station;

/// The most glances kept (the newest), and the most result lines read of one search.
const MOST: usize = 32;
const MOST_FOUND: usize = 64;

/// Programs whose line is a search, or a read, by its first word.
const SEARCHES: [&str; 7] = ["rg", "grep", "egrep", "fgrep", "find", "fd", "ag"];
const READS: [&str; 8] = ["cat", "head", "tail", "less", "bat", "nl", "sed", "wc"];

#[derive(Clone, Debug, PartialEq)]
pub enum Words {
    /// A tool's path argument, as given.
    Path(String),
    /// A shell line: its paths are read as `city_shell::paths` reads them.
    Line(String),
    /// No scope: the whole checkout.
    All,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Glance {
    pub id: String,
    pub search: bool,
    pub words: Words,
    /// A search's result, line by line: the part before a line's first `:` (`src/a.rs:12:…`).
    pub found: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Glances {
    list: VecDeque<Glance>,
}

impl Glances {
    pub fn iter(&self) -> impl Iterator<Item = &Glance> {
        self.list.iter()
    }

    /// A call started: a glance if it reads or searches. `detail` as its row shows it.
    pub(crate) fn start(&mut self, id: &str, tool: &str, station: Station, detail: &str) {
        let glance = match (tool, station) {
            ("Read" | "NotebookRead", _) => Some((false, Words::Path(detail.to_string()))),
            ("Grep" | "Glob", _) => Some((true, scope(detail))),
            (_, Station::Shell) => {
                let line = detail;
                let programs: Vec<&str> =
                    line.split(['\n', ';', '|', '&']).filter_map(|c| c.split_whitespace().next()).collect();
                let has = |set: &[&str]| programs.iter().any(|p| set.contains(p));
                let grep = line.contains("git grep");
                (has(&SEARCHES) || grep)
                    .then_some(true)
                    .or(has(&READS).then_some(false))
                    .map(|search| (search, Words::Line(line.to_string())))
            }
            _ => None,
        };
        let Some((search, words)) = glance else { return };
        if self.list.len() == MOST {
            self.list.pop_front();
        }
        self.list.push_back(Glance { id: id.to_string(), search, words, found: Vec::new() });
    }

    /// A call ended: a search's result, if it printed one.
    pub(crate) fn end(&mut self, id: &str, output: Option<&str>) {
        let Some(glance) = self.list.iter_mut().find(|g| g.id == id && g.search) else { return };
        let lines = output.unwrap_or("").lines().take(MOST_FOUND);
        let named = lines.map(|l| l.split(':').next().unwrap_or("").trim()).filter(|p| !p.is_empty() && p.len() <= 256);
        glance.found = named.map(str::to_string).collect();
    }
}

/// A Grep or Glob call's scope from its detail: its `path`, given alone or in its input's JSON.
fn scope(detail: &str) -> Words {
    if !detail.trim_start().starts_with('{') {
        return Words::Path(detail.to_string());
    }
    let input: Option<serde_json::Value> = serde_json::from_str(detail).ok();
    match input.as_ref().and_then(|input| input["path"].as_str()) {
        Some(path) => Words::Path(path.to_string()),
        None => Words::All,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(tool: &str, station: Station, detail: &str) -> Option<(bool, Words)> {
        let mut g = Glances::default();
        g.start("a", tool, station, detail);
        g.iter().next().map(|g| (g.search, g.words.clone()))
    }

    #[test]
    fn reads_and_searches_are_known_by_their_tool_or_first_word() {
        let path = |p: &str| Words::Path(p.into());
        let line = |l: &str| Words::Line(l.into());
        assert_eq!(one("Read", Station::Files, "/code/app/src/a.rs"), Some((false, path("/code/app/src/a.rs"))));
        assert_eq!(one("Grep", Station::Files, "/code/app/src"), Some((true, path("/code/app/src"))));
        assert_eq!(one("Grep", Station::Files, r#"{"pattern":"loop"}"#), Some((true, Words::All)));
        assert_eq!(one("Glob", Station::Files, r#"{"pattern":"*.rs","path":"src"}"#), Some((true, path("src"))));
        assert_eq!(
            one("Bash", Station::Shell, "cd /code/app && rg -n loop src"),
            Some((true, line("cd /code/app && rg -n loop src")))
        );
        assert_eq!(one("Bash", Station::Shell, "git grep x docs"), Some((true, line("git grep x docs"))));
        assert_eq!(one("exec", Station::Shell, "cat a.rs"), Some((false, line("cat a.rs"))));
        assert_eq!(one("Bash", Station::Shell, "cargo test"), None);
        assert_eq!(one("Edit", Station::Files, "/code/app/src/a.rs"), None);
    }

    #[test]
    fn a_searchs_result_names_its_files_and_the_newest_are_kept() {
        let mut g = Glances::default();
        g.start("s", "Grep", Station::Files, "src");
        g.end("s", Some("src/a.rs:12:  loop {\nsrc/b.rs\n\n"));
        assert_eq!(g.iter().next().map(|g| g.found.clone()), Some(vec!["src/a.rs".into(), "src/b.rs".into()]));
        for n in 0..40 {
            g.start(&n.to_string(), "Read", Station::Files, "a.rs");
        }
        assert_eq!(g.iter().count(), MOST);
        assert_eq!(g.iter().next().map(|g| g.id.as_str()), Some("8"));
    }
}
