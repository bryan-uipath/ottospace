//! What an agent looked at, for the code city's brief effects: a read (its file) and a search
//! (where it searched; the files its result names). From a call's words only, best-effort and
//! bounded: what a shell pipeline really reads cannot be known.

use std::collections::VecDeque;
use std::fmt;

use crate::activity::Station;
use crate::city_shell;

/// The most glances kept (the newest of the turn), the most result lines read of one search, and
/// the most operands or `cd`s kept of one shell stage.
const MOST: usize = 32;
const MOST_FOUND: usize = 64;
const MOST_WORDS: usize = 8;

/// Programs whose stage is a search, or a read, by its first word.
const SEARCHES: [&str; 7] = ["rg", "grep", "egrep", "fgrep", "find", "fd", "ag"];
const READS: [&str; 8] = ["cat", "head", "tail", "less", "bat", "nl", "sed", "wc"];

#[derive(Clone, PartialEq)]
pub enum Words {
    /// A tool's path argument, literal.
    Path(String),
    /// A shell stage: the `cd`s run before it in turn (each from the last; none: where the line
    /// started, the checkout) and its operands (a search's pattern left out).
    Stage { cds: Vec<String>, operands: Vec<String> },
    /// No scope: the whole checkout.
    All,
}

/// The kind only: its words are what the agent ran.
impl fmt::Debug for Words {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self {
            Words::Path(_) => "Path",
            Words::Stage { cds, operands } => {
                return write!(f, "Stage({} cds, {} operands)", cds.len(), operands.len());
            }
            Words::All => "All",
        };
        f.write_str(kind)
    }
}

#[derive(Clone, PartialEq)]
pub struct Glance {
    pub id: String,
    pub search: bool,
    pub words: Words,
    /// A search's result, line by line: the part before a line's first `:` (`src/a.rs:12:…`).
    pub found: Vec<String>,
}

/// Its words are what the agent ran or found: never printed (as `Text` is not).
impl fmt::Debug for Glance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Glance {{ id: {:?}, search: {}, found: {} }}", self.id, self.search, self.found.len())
    }
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
            ("Read", _) => Some((false, Words::Path(detail.to_string()))),
            ("Grep" | "Glob", _) => Some((true, scope(detail))),
            (_, Station::Shell) => stage(detail),
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
        let named =
            lines.map(|l| l.split(':').next().unwrap_or("").trim()).filter(|p| !p.is_empty() && p.len() <= 1024);
        glance.found = named.map(str::to_string).collect();
    }

    /// A new turn: the last one's glances have played.
    pub(crate) fn clear(&mut self) {
        self.list.clear();
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

/// A shell line's first stage that reads or searches (`sed -i` edits: no read), with the `cd`s
/// run before it and its operands: flags left out, and a search's pattern (its first operand,
/// unless `-e` gave it; `find`'s operands are its places, up to its first predicate).
fn stage(line: &str) -> Option<(bool, Words)> {
    let mut cds: Vec<String> = Vec::new();
    for (words, then) in city_shell::commands(line) {
        let Some((program, args)) = words.split_first() else { continue };
        if program == "cd" {
            if let Some(dir) = args.first().filter(|_| then && cds.len() < MOST_WORDS) {
                cds.push(dir.clone());
            }
            continue;
        }
        let git_grep = program == "git" && args.first().is_some_and(|a| a == "grep");
        let edits = program == "sed" && args.iter().any(|a| a == "--in-place" || a.starts_with("-i"));
        let search = SEARCHES.contains(&program.as_str()) || git_grep;
        if !search && (edits || !READS.contains(&program.as_str())) {
            continue;
        }
        let args = if git_grep { &args[1..] } else { args };
        let mut operands: Vec<String> = if program == "find" {
            args.iter().take_while(|a| !a.starts_with('-')).cloned().collect()
        } else {
            args.iter().filter(|a| !a.starts_with('-')).cloned().collect()
        };
        let given = args.iter().any(|a| a == "-e" || a == "--regexp");
        // `rg loop src`: `loop` is the pattern; `sed -n 1p a.rs`: `1p` its script.
        if (search && program != "find" && !given) || program == "sed" {
            operands.drain(..operands.len().min(1));
        }
        operands.truncate(MOST_WORDS);
        return Some((search, Words::Stage { cds, operands }));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(tool: &str, station: Station, detail: &str) -> Option<(bool, Words)> {
        let mut g = Glances::default();
        g.start("a", tool, station, detail);
        g.iter().next().map(|g| (g.search, g.words.clone()))
    }

    fn stage_of(cds: &[&str], operands: &[&str]) -> Words {
        let owned = |v: &[&str]| v.iter().map(|w| w.to_string()).collect();
        Words::Stage { cds: owned(cds), operands: owned(operands) }
    }

    #[test]
    fn reads_and_searches_are_known_by_their_tool_or_stage() {
        let path = |p: &str| Words::Path(p.into());
        let cases = [
            ("Read", Station::Files, "/code/app/src/a.rs", Some((false, path("/code/app/src/a.rs")))),
            ("Grep", Station::Files, "/code/app/src", Some((true, path("/code/app/src")))),
            ("Grep", Station::Files, r#"{"pattern":"loop"}"#, Some((true, Words::All))),
            ("Glob", Station::Files, r#"{"pattern":"*.rs","path":"src"}"#, Some((true, path("src")))),
            (
                "Bash",
                Station::Shell,
                "cd /code/app && rg -n loop src",
                Some((true, stage_of(&["/code/app"], &["src"]))),
            ),
            ("Bash", Station::Shell, "rg -e loop src docs", Some((true, stage_of(&[], &["loop", "src", "docs"])))),
            ("Bash", Station::Shell, "git grep x docs", Some((true, stage_of(&[], &["docs"])))),
            ("Bash", Station::Shell, "find src docs -name '*.rs'", Some((true, stage_of(&[], &["src", "docs"])))),
            ("Bash", Station::Shell, "rg needle", Some((true, stage_of(&[], &[])))),
            ("exec", Station::Shell, "cat a.rs && echo b.rs", Some((false, stage_of(&[], &["a.rs"])))),
            ("Bash", Station::Shell, "sed -n 1,20p src/a.rs", Some((false, stage_of(&[], &["src/a.rs"])))),
            ("Bash", Station::Shell, "sed -i 's/a/b/' src/a.rs", None),
            ("Bash", Station::Shell, "echo 'git grep' && cargo test", None),
            ("Bash", Station::Shell, "cargo test", None),
            ("Edit", Station::Files, "/code/app/src/a.rs", None),
        ];
        for (tool, station, detail, want) in cases {
            assert_eq!(one(tool, station, detail), want, "{detail}");
        }
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
        // Never printed: what it ran or found.
        let shown = format!("{:?}", g.iter().next());
        assert!(!shown.contains("a.rs"), "{shown}");
    }
}
