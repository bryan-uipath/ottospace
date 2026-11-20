//! A repository cloned from a provider into `clone_dir`: the user's paste read as `owner/name`,
//! checked by the provider's own rules, the folder it goes to, and what a failure means. A
//! remote's name is data: it is checked here before it is ever an argument or a path.

use serde::Serialize;

use crate::edit::Field;

/// Why a client's request opens, types in, takes from or closes no clone dialog.
pub const USERS_ALONE: &str = "only the user clones a repository";
/// Why what is behind the dialog waits.
pub const OPEN: &str = "the clone dialog is open";
/// Why a client says nothing of a clone's progress or end.
pub const SHELLS_ALONE: &str = "only ottospace says how a clone went";
/// Why ↵ starts no second clone.
pub const ONE_AT_A_TIME: &str = "A clone is running: wait for it, or Esc to cancel it";
/// Why ↵ clones nothing: no `clone_dir`, as in a test run with no folder of its own.
pub const NO_DIR: &str = "No folder to clone into in this run (`clone_dir`)";
/// Why ↵ on a line that is no repository clones nothing.
pub const NOT_A_REPO: &str = "Paste owner/name, or a GitHub link";

/// GitHub's longest login, and its longest repository name.
pub const MAX_OWNER: usize = 39;
pub const MAX_NAME: usize = 100;
/// The longest line of gh's that is shown.
const MAX_SAID: usize = 200;

/// Where a repository is cloned from. A second provider is a second variant, and its runner in
/// the shell (`clone.rs`): nothing else here asks which one it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    GitHub,
}

/// A repository of a provider: `owner/name`, each checked (`valid_owner`, `valid_name`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Remote {
    pub owner: String,
    pub name: String,
}

impl Remote {
    /// Both parts checked, else none.
    pub fn new(owner: &str, name: &str) -> Option<Remote> {
        (valid_owner(owner) && valid_name(name)).then(|| Remote { owner: owner.into(), name: name.into() })
    }

    /// `owner/name`.
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// A GitHub login or organisation: letters, digits and `-`, not first or last, at most 39.
pub fn valid_owner(owner: &str) -> bool {
    let ok = |c: char| c.is_ascii_alphanumeric() || c == '-';
    (1..=MAX_OWNER).contains(&owner.len()) && owner.chars().all(ok) && !owner.starts_with('-') && !owner.ends_with('-')
}

/// A repository name that is also a plain folder name: letters, digits, `.`, `_`, `-`, at most
/// 100; not `.` first (`..`, a hidden folder), not `-` first (an option), not ending in `.git`.
pub fn valid_name(name: &str) -> bool {
    let ok = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
    (1..=MAX_NAME).contains(&name.len())
        && name.chars().all(ok)
        && !name.starts_with(['.', '-'])
        && !name.ends_with(".git")
}

/// What the user typed or pasted, as a repository: `owner/name(.git)`, or a github.com URL as a
/// remote's (`prs::github_path`: `https://`, `git@`, `ssh://`). Anything else: none.
pub fn parse(text: &str) -> Option<Remote> {
    let text = text.trim();
    if text.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let (owner, name) = match crate::prs::github_path(text) {
        Some(path) => path,
        None if text.contains(':') => return None,
        None => {
            let path = text.strip_suffix(".git").unwrap_or(text);
            path.split_once('/')?
        }
    };
    Remote::new(owner, name)
}

/// Where `name` is cloned: `<dir>/<name>`. `dir` must be absolute; `name` a checked one, so the
/// folder is always one plain name inside `dir`.
pub fn destination(dir: &str, name: &str) -> Option<String> {
    let parts_ok = !dir.split('/').any(|part| part == "..");
    (dir.starts_with('/') && parts_ok && valid_name(name)).then(|| format!("{}/{name}", dir.trim_end_matches('/')))
}

/// Why a clone ended without a repository. Whatever gh says is `Said`: shown cleaned, one line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// No gh, or (a test run) no `OTTOSPACE_GH`.
    NoGh,
    /// `clone_dir` is not there, and the user has not said to make it.
    DirMissing,
    /// Something is at the destination already: nothing ran.
    Exists,
    /// The user's Esc: the program was stopped, and the folder it made removed.
    Cancelled,
    /// gh's exit and its standard error.
    Said(String),
}

/// What a toast says of a clone that ended, in the state every reply carries: no name, no path.
pub const CLONED: &str = "Cloned: listed in Repos, its row selected";

impl Failure {
    /// The failure for a toast: no repository's name or path, which are the user's data (the
    /// dialog, drawn only, says them).
    pub fn public(&self) -> String {
        match self {
            Failure::DirMissing => "clone_dir does not exist: From GitHub… makes it on your next ↵".into(),
            Failure::Exists => "Already there: nothing was cloned".into(),
            Failure::Cancelled => "Cancelled the clone: its folder was removed".into(),
            Failure::Said(_) => {
                let plain = self.explain(&Remote { owner: String::new(), name: String::new() }, "");
                match plain.starts_with("Not found") {
                    true => "Not found on GitHub, or not yours to read".into(),
                    false if plain.starts_with("gh: ") => "The clone failed: From GitHub… says why".into(),
                    false => plain,
                }
            }
            Failure::NoGh => self.explain(&Remote { owner: String::new(), name: String::new() }, ""),
        }
    }

    /// The failure in plain words, for the dialog.
    pub fn explain(&self, remote: &Remote, dest: &str) -> String {
        match self {
            Failure::NoGh => "gh is not installed: install it (brew install gh), then gh auth login".into(),
            Failure::DirMissing => format!("{} does not exist: ↵ makes it and clones", parent(dest)),
            Failure::Exists => format!("Already there: {dest}. Nothing was cloned"),
            Failure::Cancelled => format!("Cancelled the clone of {}: its folder was removed", remote.slug()),
            Failure::Said(said) => {
                let lower = said.to_lowercase();
                if lower.contains("gh auth login") || lower.contains("not logged") || lower.contains("authentication") {
                    "gh is not logged in: run gh auth login".into()
                } else if lower.contains("could not resolve to a repository") || lower.contains("not found") {
                    format!("Not found on GitHub, or not yours to read: {}", remote.slug())
                } else if lower.contains("could not resolve host")
                    || lower.contains("network")
                    || lower.contains("timed out")
                {
                    "Could not reach GitHub: check the network".into()
                } else {
                    let line = said.lines().map(str::trim).rfind(|line| !line.is_empty()).unwrap_or("it failed");
                    format!("gh: {}", one_line(line))
                }
            }
        }
    }
}

/// The folder a destination is in.
fn parent(dest: &str) -> &str {
    dest.rsplit_once('/').map_or(dest, |(dir, _)| if dir.is_empty() { "/" } else { dir })
}

/// Another program's text, to show: no escapes or control characters, one line, cut short.
pub fn one_line(text: &str) -> String {
    let clean = crate::transcript::clean(text).replace('\n', " ");
    let mut out: String = clean.trim().chars().take(MAX_SAID).collect();
    if clean.trim().chars().count() > MAX_SAID {
        out.push('…');
    }
    out
}

/// The dialog: what is typed, the row the keys are on, what it says (a refusal or a failure),
/// and the folder the user said may be made (after `Failure::DirMissing`).
#[derive(Clone, Debug, PartialEq)]
pub struct Dialog {
    pub provider: Provider,
    pub query: Field,
    pub selected: usize,
    pub said: Option<String>,
    pub make_dir: Option<String>,
}

impl Dialog {
    pub fn new(provider: Provider) -> Dialog {
        Dialog { provider, query: Field::new("", crate::model::MAX_FILTER), selected: 0, said: None, make_dir: None }
    }

    /// Its rows: the repository the line names, if it names one.
    pub fn rows(&self) -> Vec<Remote> {
        parse(self.query.text()).into_iter().collect()
    }
}

/// A clone on its way: which ask it is, of what, to where, and gh's last line.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub id: u64,
    pub remote: Remote,
    pub dest: String,
    pub line: String,
    /// The user's Esc stopped it: its end says so, whatever the program said.
    pub cancelled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slug(text: &str) -> Option<String> {
        parse(text).map(|remote| remote.slug())
    }

    #[test]
    fn a_paste_is_owner_and_name_or_a_github_link() {
        for text in [
            "octo/hello",
            " octo/hello ",
            "octo/hello.git",
            "https://github.com/octo/hello",
            "https://github.com/octo/hello/",
            "https://github.com/octo/hello.git",
            "git@github.com:octo/hello.git",
            "git@github.com:octo/hello",
            "ssh://git@github.com/octo/hello",
        ] {
            assert_eq!(slug(text).as_deref(), Some("octo/hello"), "{text}");
        }
        assert_eq!(slug("my-org/a.b_c-d").as_deref(), Some("my-org/a.b_c-d"));
    }

    /// A name from a paste or from gh becomes a folder and an argument: nothing that is a path,
    /// an option, a host or a line of its own gets through.
    #[test]
    fn a_hostile_paste_is_no_repository() {
        let long = format!("o/{}", "a".repeat(MAX_NAME + 1));
        let hostile = [
            "",
            "hello",
            "../x",
            "o/../x",
            "o/..",
            "o/.",
            "o/.hidden",
            "a/b/c",
            "/etc/passwd",
            "o/-rf",
            "-o/x",
            "o-/x",
            "o/x\ny",
            "o/x y",
            "o/x;rm",
            "o/$(id)",
            "o/`id`",
            "o/x|y",
            "o/x%2e%2e",
            "o/~",
            "o/x\u{202e}",
            "o/é",
            "o/.git",
            "o/x.git.git",
            "http://github.com/o/x",
            "https://evil.com/o/x",
            "https://github.com.evil.com/o/x",
            "https://github.com/o/x/tree/main",
            "https://github.com/o",
            "git@evil.com:o/x.git",
            "file:///tmp/x",
            "ext::sh -c id",
            "o:x/y",
            &long,
        ];
        for text in hostile {
            assert_eq!(slug(text), None, "{text:?}");
        }
        assert!(!valid_owner(&"a".repeat(MAX_OWNER + 1)));
        assert!(valid_name(&"a".repeat(MAX_NAME)));
    }

    #[test]
    fn the_destination_is_one_plain_name_inside_an_absolute_folder() {
        assert_eq!(destination("/Users/me/code", "hello").as_deref(), Some("/Users/me/code/hello"));
        assert_eq!(destination("/Users/me/code/", "hello").as_deref(), Some("/Users/me/code/hello"));
        for (dir, name) in [
            ("code", "hello"),
            ("~/code", "hello"),
            ("", "hello"),
            ("/a/../b", "hello"),
            ("/code", "../x"),
            ("/code", "a/b"),
            ("/code", ".."),
            ("/code", "-rf"),
            ("/code", "x\n"),
        ] {
            assert_eq!(destination(dir, name), None, "{dir:?} {name:?}");
        }
    }

    #[test]
    fn a_failure_is_said_in_plain_words_and_gh_text_is_one_clean_line() {
        let remote = Remote::new("octo", "hello").unwrap();
        let dest = "/Users/me/code/hello";
        let said = |text: &str| Failure::Said(text.into()).explain(&remote, dest);
        assert_eq!(
            said("To get started with GitHub CLI, please run:  gh auth login"),
            "gh is not logged in: run gh auth login"
        );
        assert_eq!(
            said("GraphQL: Could not resolve to a Repository with the name 'octo/hello'."),
            "Not found on GitHub, or not yours to read: octo/hello"
        );
        assert_eq!(
            said("fatal: unable to access: Could not resolve host: github.com"),
            "Could not reach GitHub: check the network"
        );
        assert_eq!(said("warming\n\x1b[31mfatal: odd\x07 thing\x1b[0m\n"), "gh: fatal: odd thing");
        assert_eq!(said(""), "gh: it failed");
        assert_eq!(Failure::Exists.explain(&remote, dest), "Already there: /Users/me/code/hello. Nothing was cloned");
        assert_eq!(Failure::DirMissing.explain(&remote, dest), "/Users/me/code does not exist: ↵ makes it and clones");
        let long = one_line(&"x".repeat(500));
        assert_eq!(long.chars().count(), MAX_SAID + 1);
    }
}
