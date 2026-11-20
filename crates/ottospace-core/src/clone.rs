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

/// The user's repositories and their organisations', most recently pushed first: one GraphQL
/// read, a fixed text (`gh api graphql -f query=…`).
pub const LIST_QUERY: &str = "query{viewer{login organizations(first:20){nodes{login}} \
repositories(first:100,orderBy:{field:PUSHED_AT,direction:DESC},\
ownerAffiliations:[OWNER,ORGANIZATION_MEMBER,COLLABORATOR]){nodes{nameWithOwner description isPrivate isFork isArchived}}}}";
/// A search by name among those owners' repositories: the fixed text, and `q` its one variable
/// (`-f q=…`: a string, never read as a file or a number).
pub const SEARCH_QUERY: &str = "query($q:String!){search(query:$q,type:REPOSITORY,first:30)\
{nodes{...on Repository{nameWithOwner description isPrivate isFork isArchived}}}}";

/// The most rows the dialog lists, and of a description's characters shown.
pub const MAX_ROWS: usize = 50;
const MAX_DESCRIPTION: usize = 120;
/// The longest answer of gh's that is read.
pub const MAX_BYTES: usize = 1 << 20;

/// A repository gh listed: its checked name, and what is drawn of it. Its description is other
/// people's text: cleaned, one line, cut short, drawn only.
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub remote: Remote,
    pub description: String,
    pub private: bool,
    pub fork: bool,
    pub archived: bool,
}

/// gh's answer to `LIST_QUERY` (the repositories, and the owners: the user's login and their
/// organisations') or `SEARCH_QUERY` (the repositories). A name that is not a checked one is
/// left out, never shown.
pub fn parse_listed(text: &str) -> Result<(Vec<Listed>, Vec<String>), &'static str> {
    if text.len() > MAX_BYTES {
        return Err("gh's answer is too long");
    }
    let json: serde_json::Value = serde_json::from_str(text).map_err(|_| "gh's answer does not read")?;
    let data = &json["data"];
    let viewer = &data["viewer"];
    let nodes = viewer["repositories"]["nodes"].as_array().or(data["search"]["nodes"].as_array());
    let Some(nodes) = nodes else { return Err("gh's answer does not read") };
    let listed = nodes.iter().filter_map(|node| {
        let (owner, name) = node["nameWithOwner"].as_str()?.split_once('/')?;
        let flag = |key: &str| node[key].as_bool().unwrap_or(false);
        let description = one_line(node["description"].as_str().unwrap_or(""));
        let description = match description.chars().count() > MAX_DESCRIPTION {
            true => description.chars().take(MAX_DESCRIPTION).chain(['…']).collect(),
            false => description,
        };
        let (private, fork, archived) = (flag("isPrivate"), flag("isFork"), flag("isArchived"));
        Some(Listed { remote: Remote::new(owner, name)?, description, private, fork, archived })
    });
    let orgs = viewer["organizations"]["nodes"].as_array().into_iter().flatten().map(|org| &org["login"]);
    let owners = std::iter::once(&viewer["login"]).chain(orgs).filter_map(|login| login.as_str());
    let owners = owners.filter(|login| valid_owner(login)).map(str::to_string).take(21).collect();
    Ok((listed.take(100).collect(), owners))
}

/// The search for what is typed, among `owners`' repositories: `<words> in:name user:<owner>…`.
/// None for a line that is a repository already, too short, or has anything but a name's
/// characters: nothing of it but those reaches GitHub's search.
pub fn search_q(typed: &str, owners: &[String]) -> Option<String> {
    let typed = typed.trim();
    let ok = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
    let plain = (2..=MAX_NAME).contains(&typed.len()) && typed.chars().all(ok) && !typed.starts_with('-');
    if !plain || owners.is_empty() {
        return None;
    }
    let owners: Vec<String> = owners.iter().filter(|o| valid_owner(o)).map(|o| format!("user:{o}")).collect();
    Some(format!("{typed} in:name {}", owners.join(" ")))
}

/// How the list came: not yet, read, or why not.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Load {
    #[default]
    Loading,
    Read,
    Failed(String),
}

/// A row of the dialog: a repository, and the listed checkout whose remote it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub listed: Listed,
    pub here: Option<String>,
}

/// The dialog: what is typed, the row the keys are on, what it says (a refusal or a failure),
/// the folder the user said may be made (after `Failure::DirMissing`), and what gh listed and found.
#[derive(Clone, Debug, PartialEq)]
pub struct Dialog {
    pub provider: Provider,
    pub query: Field,
    pub selected: usize,
    pub said: Option<String>,
    pub make_dir: Option<String>,
    pub load: Load,
    pub listed: Vec<Listed>,
    pub owners: Vec<String>,
    /// What a search found, and for which `q`.
    pub found: Option<(String, Vec<Listed>)>,
    /// Each listed checkout's GitHub remote (`owner/name`, lower case) and its path.
    pub here: Vec<(String, String)>,
}

impl Dialog {
    pub fn new(provider: Provider) -> Dialog {
        let query = Field::new("", crate::model::MAX_FILTER);
        Dialog {
            provider,
            query,
            selected: 0,
            said: None,
            make_dir: None,
            load: Load::Loading,
            listed: Vec::new(),
            owners: Vec::new(),
            found: None,
            here: Vec::new(),
        }
    }

    /// The search to run for what is typed now, if any.
    pub fn search(&self) -> Option<String> {
        search_q(self.query.text(), &self.owners)
    }

    /// Its rows: the repository the line names (if not among them), then those listed whose
    /// `owner/name` has what is typed in it (all, most recently pushed first, with nothing
    /// typed), then what the search for it found. At most `MAX_ROWS`.
    pub fn rows(&self) -> Vec<Row> {
        let typed = self.query.text().trim().to_lowercase();
        let found = self.found.as_ref().filter(|(q, _)| Some(q) == self.search().as_ref()).map(|(_, found)| found);
        let pasted = parse(self.query.text()).map(|remote| Listed {
            remote,
            description: String::new(),
            private: false,
            fork: false,
            archived: false,
        });
        let matching = self.listed.iter().filter(|listed| listed.remote.slug().to_lowercase().contains(&typed));
        let mut rows: Vec<Listed> = Vec::new();
        for listed in matching.chain(found.into_iter().flatten()).cloned() {
            if !rows.iter().any(|row| row.remote == listed.remote) {
                rows.push(listed);
            }
        }
        if let Some(pasted) =
            pasted.filter(|p| !rows.iter().any(|row| row.remote.slug().eq_ignore_ascii_case(&p.remote.slug())))
        {
            rows.insert(0, pasted);
        }
        rows.truncate(MAX_ROWS);
        let here = |listed: &Listed| {
            let slug = listed.remote.slug().to_lowercase();
            self.here.iter().find(|(remote, _)| *remote == slug).map(|(_, path)| path.clone())
        };
        rows.into_iter().map(|listed| Row { here: here(&listed), listed }).collect()
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

    #[test]
    fn ghs_list_is_data_checked_names_and_clean_descriptions() {
        let answer = r#"{"data":{"viewer":{"login":"me","organizations":{"nodes":[{"login":"acme"},{"login":"-bad"}]},
            "repositories":{"nodes":[
              {"nameWithOwner":"me/app","description":"An app\u001b[31m\nrm -rf / \u0007","isPrivate":true,"isFork":false,"isArchived":false},
              {"nameWithOwner":"acme/../etc","description":null},
              {"nameWithOwner":"acme/.hidden"},
              {"nameWithOwner":"acme/tool","description":"x","isFork":true,"isArchived":true}]}}}}"#;
        let (listed, owners) = parse_listed(answer).unwrap();
        let slugs: Vec<String> = listed.iter().map(|l| l.remote.slug()).collect();
        assert_eq!(
            (slugs, owners),
            (vec!["me/app".to_string(), "acme/tool".into()], vec!["me".to_string(), "acme".into()])
        );
        assert_eq!(listed[0].description, "An app rm -rf /");
        assert!(listed[0].private && listed[1].fork && listed[1].archived);
        let long = format!(
            r#"{{"data":{{"search":{{"nodes":[{{"nameWithOwner":"o/x","description":"{}"}}]}}}}}}"#,
            "d".repeat(500)
        );
        assert_eq!(parse_listed(&long).unwrap().0[0].description.chars().count(), MAX_DESCRIPTION + 1);
        assert!(parse_listed("not json").is_err() && parse_listed(&" ".repeat(MAX_BYTES + 1)).is_err());
    }

    #[test]
    fn a_search_is_a_name_among_the_owners_and_nothing_else_of_the_line() {
        let owners = vec!["me".to_string(), "acme".into()];
        assert_eq!(search_q("app", &owners).as_deref(), Some("app in:name user:me user:acme"));
        for typed in ["a", "app repo:evil/x", "app user:evil", "-app", "octo/app", "a\"b", "", "é"] {
            assert_eq!(search_q(typed, &owners), None, "{typed:?}");
        }
        assert_eq!(search_q("app", &[]), None);
    }

    #[test]
    fn the_rows_are_the_paste_then_the_listed_that_match_then_what_the_search_found() {
        let listed = |slug: &str| {
            let (owner, name) = slug.split_once('/').unwrap();
            Listed {
                remote: Remote::new(owner, name).unwrap(),
                description: String::new(),
                private: false,
                fork: false,
                archived: false,
            }
        };
        let mut dialog = Dialog::new(Provider::GitHub);
        (dialog.listed, dialog.owners) =
            (vec![listed("me/app"), listed("acme/api"), listed("me/notes")], vec!["me".into()]);
        dialog.here = vec![("me/notes".into(), "/code/notes".into())];
        let slugs = |d: &Dialog| d.rows().iter().map(|r| r.listed.remote.slug()).collect::<Vec<_>>();
        assert_eq!(slugs(&dialog), ["me/app", "acme/api", "me/notes"]);
        assert_eq!(dialog.rows()[2].here.as_deref(), Some("/code/notes"));
        dialog.query = Field::new("ap", 200);
        assert_eq!(slugs(&dialog), ["me/app", "acme/api"]);
        // Found for this line: after the listed, once each; for another line: not shown.
        dialog.found = Some((dialog.search().unwrap(), vec![listed("me/apple"), listed("me/app")]));
        assert_eq!(slugs(&dialog), ["me/app", "acme/api", "me/apple"]);
        dialog.query = Field::new("api", 200);
        assert_eq!(slugs(&dialog), ["acme/api"]);
        // A paste not listed comes first; one listed is its row.
        dialog.query = Field::new("other/thing", 200);
        assert_eq!(slugs(&dialog), ["other/thing"]);
        dialog.query = Field::new("https://github.com/ME/App", 200);
        assert_eq!(slugs(&dialog), ["ME/App"]);
    }
}
