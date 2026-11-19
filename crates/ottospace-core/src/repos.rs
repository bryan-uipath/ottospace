//! Repositories and their worktrees, as the Repos section lists them. The shell reads the files;
//! the rules for what they mean are here.

use serde::{Deserialize, Serialize};

/// A git repository, and the extra worktrees made from it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Repo {
    pub name: String,
    pub path: String,
    /// The branch checked out, or a short commit id. `None` if `HEAD` could not be read.
    pub branch: Option<String>,
    pub worktrees: Vec<Worktree>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Worktree {
    pub name: String,
    pub path: String,
    pub branch: Option<String>,
}

/// How many repositories the settings' `repos` may name.
pub const MAX_DIRS: usize = 64;

/// An entry of the settings' `repos`: `text` as the settings name it (`~/code/app`), `path`
/// resolved (`/Users/me/code/app`). A repository, or (kept from the old settings, which named
/// folders to look in) a folder of repositories: a group, drawn folded. Pinned ones come first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepoDir {
    pub text: String,
    pub path: String,
    #[serde(default)]
    pub pinned: bool,
}

impl RepoDir {
    pub fn new(text: &str, path: &str) -> RepoDir {
        RepoDir { text: text.into(), path: path.into(), pinned: false }
    }
}

/// The Repos list's entries, in the user's order: pinned ones first.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct RepoDirs {
    pub dirs: Vec<RepoDir>,
    /// `OTTOSPACE_REPOS` names them, not the settings: none is added, removed or moved, and a
    /// folder of repositories lists them flat, with no group.
    pub fixed: bool,
    /// Where a new worktree goes (`worktrees_dir`, absolute). `None`: none is made, e.g. a test
    /// run with no folder of its own. Not in the state.
    #[serde(skip)]
    pub worktrees: Option<String>,
}

const FIXED: &str = "OTTOSPACE_REPOS names the repositories: the settings are not read";

impl RepoDirs {
    /// Pinned entries first, each part in its order; an entry named twice, once (the first).
    /// Every change to the list keeps it so.
    pub fn settle(&mut self) {
        let mut seen: Vec<String> = Vec::new();
        self.dirs.retain(|dir| {
            let first = !seen.contains(&dir.path);
            seen.push(dir.path.clone());
            first
        });
        self.dirs.sort_by_key(|dir| !dir.pinned);
    }

    /// Why `dir` (resolved) cannot be added. `repo`: it is a repository's own folder (a linked
    /// worktree's is not). `Ok(false)`: listed already, as an entry of its own or (`listed`) as a
    /// worktree of one, or of an `OTTOSPACE_REPOS` folder. A repository listed only through a group
    /// is added: it leaves the group.
    pub fn may_add(&self, dir: &str, repo: bool, listed: bool) -> Result<bool, String> {
        match () {
            _ if !dir.starts_with('/') => Err(format!("Not a directory: {dir}")),
            _ if listed || self.dirs.iter().any(|d| d.path == dir) => Ok(false),
            _ if !repo => Err(format!("Not a repository: {dir}")),
            _ if self.fixed => Err(FIXED.into()),
            _ if self.dirs.len() >= MAX_DIRS => Err(format!("Repos lists {MAX_DIRS} repositories already")),
            _ => Ok(true),
        }
    }

    /// The entry to take off the settings so the repository or group at `path` is no longer
    /// listed. `Err`: why none is. A repository's path is absolute: an entry the settings could
    /// not resolve (`""`) is no repository's, and is not removed in its name.
    pub fn to_remove(&self, path: &str) -> Result<RepoDir, String> {
        // An entry with no text in the settings cannot be named in a change to them.
        match (self.dirs.iter().find(|d| d.path == path && !d.text.is_empty()), self.holder(path)) {
            _ if self.fixed => Err(FIXED.into()),
            _ if !path.starts_with('/') => Err(format!("Not a directory: {path}")),
            (Some(dir), _) => Ok(dir.clone()),
            (None, Some(holder)) => {
                Err(format!("{} is listed in the folder {}: remove that from Repos", name_of(path), holder.text))
            }
            (None, None) => Err(format!("{} is not in Repos", name_of(path))),
        }
    }

    /// The entry that holds `path` as one of the repositories found in it: its parent.
    pub fn holder(&self, path: &str) -> Option<&RepoDir> {
        let parent = path.rsplit_once('/').map_or("", |(parent, _)| parent);
        self.dirs.iter().find(|d| d.path == parent)
    }

    /// The place in the list of the repository at `path`: its own entry, else the group that holds
    /// it. `None`: no entry lists it.
    pub fn place(&self, path: &str) -> Option<usize> {
        let own = self.dirs.iter().position(|d| d.path == path);
        own.or_else(|| self.dirs.iter().position(|d| Some(d) == self.holder(path)))
    }

    /// The group the repository at `path` is drawn in: the folder entry that holds it, when it is
    /// not an entry of its own. None while `OTTOSPACE_REPOS` names the list: it is flat.
    pub fn group_of(&self, path: &str) -> Option<&RepoDir> {
        let own = self.dirs.iter().any(|d| d.path == path);
        self.holder(path).filter(|_| !own && !self.fixed)
    }
}

/// One checkout as the Repos section lists it: a row under its repository's name.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Checkout {
    pub path: String,
    /// The branch, a short commit id, or `(no branch)` if `HEAD` could not be read.
    pub branch: String,
    /// True for the repository's own directory. A linked worktree is not the default.
    pub default: bool,
    /// Its directory's name.
    pub label: String,
}

impl Repo {
    /// The repository's rows: its own directory first, then its worktrees in their order.
    pub fn checkouts(&self) -> Vec<Checkout> {
        let row = |path: &str, branch: &Option<String>, name: &str, default: bool| Checkout {
            path: path.to_string(),
            branch: branch.clone().unwrap_or_else(|| "(no branch)".into()),
            default,
            label: name.to_string(),
        };
        let linked = self.worktrees.iter().map(|w| row(&w.path, &w.branch, &w.name, false));
        std::iter::once(row(&self.path, &self.branch, &self.name, true)).chain(linked).collect()
    }
}

/// Why a client's request adds no worktree: it runs git in the user's repository.
pub const USERS_ALONE: &str = "only the user adds a worktree";
/// Why the + makes none: no `worktrees_dir`, and no ottospace folder to put one in.
pub const WORKTREES_NONE: &str = "No folder for worktrees in this run";

/// Why the + makes none there: a new task's worktree is being made in that folder.
pub const BEING_MADE: &str = "A new task's worktree is being made there";

/// The longest branch name a new worktree takes, in bytes.
pub const MAX_BRANCH: usize = 200;

/// Why `branch` cannot name a new worktree's branch, by `git check-ref-format --branch`'s rules,
/// and stricter: no space, nothing git would read as an option (`-x`) or a revision (`@{-1}`).
pub fn check_branch(branch: &str) -> Result<(), String> {
    const BAD: &[char] = &[' ', '~', '^', ':', '?', '*', '[', '\\'];
    const NOT_GITS: &str = "Not a branch name git takes";
    let bad_part = |p: &str| p.is_empty() || p.starts_with('.') || p.ends_with(".lock");
    let why = match () {
        _ if branch.is_empty() => "Name a branch",
        _ if branch.len() > MAX_BRANCH => "That branch name is too long",
        _ if branch.starts_with('-') => "A branch name cannot start with -",
        _ if branch.chars().any(|c| c.is_control() || BAD.contains(&c)) => {
            "A branch name cannot hold a space, a control character or ~ ^ : ? * [ \\"
        }
        _ if branch.contains("..") || branch.contains("@{") || branch == "@" || branch == "HEAD" => NOT_GITS,
        _ if branch.ends_with('.') || branch.split('/').any(bad_part) => NOT_GITS,
        _ => return Ok(()),
    };
    Err(why.into())
}

/// Where a new worktree of the repository at `repo` on `branch` goes: `<worktrees>/<repo>/<branch>`,
/// slashes in the branch as `-` (`fix/zoom` → `fix-zoom`). `<repo>` is the repository's folder
/// name; when another of `listed` has that name too, with a short hash of its path (`app-1f2e3d`).
pub fn worktree_dir(worktrees: &str, repo: &str, listed: &[Repo], branch: &str) -> String {
    let name = Some(name_of(repo)).filter(|name| !matches!(name.as_str(), "" | "." | "..")).unwrap_or("repo".into());
    let shared = listed.iter().any(|other| other.path != repo && name_of(&other.path) == name);
    let folder = if shared { format!("{name}-{}", short_hash(repo)) } else { name };
    format!("{}/{folder}/{}", worktrees.trim_end_matches('/'), branch.replace('/', "-"))
}

/// `path` is a worktree's place in `root`, as `worktree_dir` makes one: `<root>/<repo>/<branch>`.
pub fn in_worktrees(path: &str, root: &str) -> bool {
    let rest = path.strip_prefix(root.trim_end_matches('/')).and_then(|rest| rest.strip_prefix('/'));
    let parts: Vec<&str> = rest.map(|rest| rest.split('/').collect()).unwrap_or_default();
    parts.len() == 2 && parts.iter().all(|part| !matches!(*part, "" | "." | ".."))
}

/// The most of a task's title a new worktree's branch takes: words, and bytes.
const BRANCH_WORDS: usize = 4;
const BRANCH_BYTES: usize = 32;

/// A new branch named for a task's title: its first plain words, lowercase, `-` between
/// (`Fix the zoom flicker!` → `fix-the-zoom-flicker`); none (`修正`): `task`. Not among `taken`:
/// `-2`, `-3` after. Always one `check_branch` takes.
pub fn branch_for(title: &str, taken: impl Fn(&str) -> bool) -> String {
    let words = title.split(|c: char| !c.is_ascii_alphanumeric()).filter(|word| !word.is_empty());
    let mut name = String::new();
    for word in words.take(BRANCH_WORDS) {
        if !name.is_empty() && name.len() + 1 + word.len() > BRANCH_BYTES {
            break;
        }
        if !name.is_empty() {
            name.push('-');
        }
        name.push_str(&word.to_ascii_lowercase());
    }
    name.truncate(BRANCH_BYTES);
    if name.is_empty() {
        name = "task".into();
    }
    let free = (1..).map(|n| if n == 1 { name.clone() } else { format!("{name}-{n}") }).find(|name| !taken(name));
    free.unwrap_or(name)
}

/// The most of git's message the status bar shows, in characters.
const MAX_SAID: usize = 200;

/// What git's standard error says, for the status bar: cleaned as any text is, its `fatal:` and
/// `error:` lines (else every line) on one line, cut short. e.g. `git: 'x' is already used by worktree`.
pub fn git_said(stderr: &str) -> String {
    let text = crate::transcript::clean(stderr);
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    let said = |line: &&str| line.starts_with("fatal:") || line.starts_with("error:");
    let picked: Vec<&str> = if lines.iter().any(said) { lines.into_iter().filter(said).collect() } else { lines };
    let bare = |line: &str| line.trim_start_matches("fatal:").trim_start_matches("error:").trim().to_string();
    let joined = picked.into_iter().map(bare).collect::<Vec<_>>().join("; ");
    let out = crate::ax::truncate(&joined, MAX_SAID);
    if out.is_empty() { "git failed".into() } else { format!("git: {out}") }
}

/// 24 bits of FNV-1a of `text`, as 6 hex digits: the same on every run and build.
fn short_hash(text: &str) -> String {
    let hash = text.bytes().fold(0x811c_9dc5_u32, |hash, byte| (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193));
    format!("{:06x}", hash & 0xff_ffff)
}

/// What a `HEAD` file says is checked out: `ref: refs/heads/fix/zoom` → `fix/zoom`; a bare
/// commit id (detached) → its first 7 characters.
pub fn branch_of(head: &str) -> Option<String> {
    let head = head.trim();
    match head.strip_prefix("ref: ") {
        Some(reference) => Some(reference.strip_prefix("refs/heads/").unwrap_or(reference).to_string()),
        None if head.len() >= 7 && head.chars().all(|c| c.is_ascii_hexdigit()) => Some(head[..7].to_string()),
        None => None,
    }
}

/// The worktree directory named by a `.git/worktrees/<name>/gitdir` file:
/// `/code/app-2/.git` → `/code/app-2`.
pub fn worktree_path(gitdir: &str) -> Option<String> {
    let path = gitdir.trim().strip_suffix("/.git")?;
    (!path.is_empty()).then(|| path.to_string())
}

/// The last part of a path, for display: `/code/app-2` → `app-2`.
pub fn name_of(path: &str) -> String {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path).to_string()
}

/// The list as shown: in the order of the entries (`dirs`), the repositories a group holds by
/// name. One no entry names goes last, by name. A directory that is a worktree of a listed
/// repository is not listed again as a repository, and a repository found twice is listed once.
/// Paths must be in one form: the shell resolves them before they get here.
pub fn arrange(found: Vec<Repo>, dirs: &RepoDirs) -> Vec<Repo> {
    let linked: Vec<String> = found.iter().flat_map(|r| r.worktrees.iter().map(|w| w.path.clone())).collect();
    let mut repos: Vec<Repo> = Vec::new();
    for mut repo in found {
        if linked.contains(&repo.path) || repos.iter().any(|r| r.path == repo.path) {
            continue;
        }
        repo.worktrees.sort_by(|a, b| a.name.cmp(&b.name));
        repos.push(repo);
    }
    let key = |r: &Repo| (dirs.place(&r.path).unwrap_or(usize::MAX), r.name.to_lowercase(), r.path.clone());
    repos.sort_by_cached_key(key);
    repos
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(name: &str, worktrees: &[&str]) -> Repo {
        let worktree = |w: &&str| Worktree { name: w.to_string(), path: format!("/code/{w}"), branch: None };
        let worktrees = worktrees.iter().map(worktree).collect();
        Repo { name: name.into(), path: format!("/code/{name}"), branch: None, worktrees }
    }

    #[test]
    fn head_names_a_branch_or_a_short_commit() {
        assert_eq!(branch_of("ref: refs/heads/fix/zoom\n").as_deref(), Some("fix/zoom"));
        assert_eq!(branch_of("ref: refs/remotes/origin/main").as_deref(), Some("refs/remotes/origin/main"));
        assert_eq!(branch_of("9c1344b0a15cb01d35cf2c801f28875906a5acfe\n").as_deref(), Some("9c1344b"));
        assert_eq!(branch_of("not a head"), None);
        assert_eq!(branch_of(""), None);
    }

    #[test]
    fn gitdir_file_names_the_worktree_directory() {
        assert_eq!(worktree_path("/code/app-2/.git\n").as_deref(), Some("/code/app-2"));
        assert_eq!(worktree_path("/code/app-2"), None);
        assert_eq!(worktree_path("/.git"), None);
        assert_eq!(name_of("/code/app-2/"), "app-2");
    }

    #[test]
    fn a_worktree_is_listed_under_its_repository_only() {
        // `app-2` was found twice: as a directory with a `.git` file, and as a worktree of `app`.
        let none = RepoDirs::default();
        let arranged = arrange(vec![repo("zed", &[]), repo("app-2", &[]), repo("App", &["app-3", "app-2"])], &none);
        let names: Vec<&str> = arranged.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["App", "zed"]);
        let worktrees: Vec<&str> = arranged[0].worktrees.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(worktrees, ["app-2", "app-3"]);
    }

    #[test]
    fn a_repository_lists_its_own_directory_as_default_then_its_worktrees() {
        let mut app = repo("app", &["app-2", "Default"]);
        app.branch = Some("main".into());
        app.worktrees[0].branch = Some("fix/zoom".into());
        let row = |path: &str, branch: &str, default: bool, label: &str| Checkout {
            path: path.into(),
            branch: branch.into(),
            default,
            label: label.into(),
        };
        // A worktree whose directory is named `Default` is still not the default: the flag says which.
        let expected = [
            row("/code/app", "main", true, "app"),
            row("/code/app-2", "fix/zoom", false, "app-2"),
            row("/code/Default", "(no branch)", false, "Default"),
        ];
        assert_eq!(app.checkouts(), expected);
    }

    #[test]
    fn a_branch_name_is_refused_as_git_would_and_for_anything_that_reads_as_an_option() {
        for good in ["main", "fix/zoom", "feat/map-2", "v1.2", "ü-umlaut", "a@b"] {
            assert_eq!(check_branch(good), Ok(()), "{good}");
        }
        let long = "x".repeat(MAX_BRANCH + 1);
        let bad = [
            "", "-f", "--force", "a b", "a\tb", "a\nb", "a..b", "../up", "a/../b", "a~1", "a^", "a:b", "a?", "a*",
            "a[b", "a\\b", "@", "HEAD", "a@{1}", "a.", "a/", "/a", "a//b", ".hidden", "a/.b", "a.lock", "a.lock/b",
            &long,
        ];
        for name in bad {
            assert!(check_branch(name).is_err(), "{name:?} passed");
        }
        assert_eq!(check_branch("-x"), Err("A branch name cannot start with -".into()));
    }

    #[test]
    fn what_git_said_is_one_clean_short_line() {
        let said = "Preparing worktree (new branch 'x')\nfatal: 'x' is already used by worktree at '/w/x'\n";
        assert_eq!(git_said(said), "git: 'x' is already used by worktree at '/w/x'");
        assert_eq!(git_said("warning: one\nhint: two\n"), "git: warning: one; hint: two");
        assert_eq!(git_said("fatal: \u{1b}[31mred\u{1b}[0m\u{7}"), "git: red");
        assert_eq!(git_said(""), "git failed");
        let long = git_said(&format!("fatal: {}", "x".repeat(500)));
        assert_eq!(long.chars().count(), "git: ".len() + MAX_SAID + 1);
    }

    #[test]
    fn a_worktree_goes_in_its_repositorys_folder_with_slashes_as_dashes() {
        let listed = [repo("app", &[]), repo("web", &[])];
        assert_eq!(worktree_dir("/w/", "/code/app", &listed, "fix/zoom"), "/w/app/fix-zoom");
        // Its place, with or without the root's `/`; nothing deeper, shallower or outside.
        assert!(in_worktrees("/w/app/fix-zoom", "/w/") && in_worktrees("/w/app/fix-zoom", "/w"));
        for path in ["/w/app", "/w/app/x/y", "/w/../x", "/wx/app/y", "/code/app-2/src"] {
            assert!(!in_worktrees(path, "/w"), "{path}");
        }
        // Two repositories named `app`: each folder takes a short hash of its path, the same each run.
        let other = Repo { path: "/work/app".into(), ..repo("app", &[]) };
        let listed = [repo("app", &[]), other];
        let (one, two) = (worktree_dir("/w", "/code/app", &listed, "x"), worktree_dir("/w", "/work/app", &listed, "x"));
        assert_ne!(one, two);
        assert!(one.starts_with("/w/app-") && one.ends_with("/x") && one.len() == "/w/app-123456/x".len(), "{one}");
        assert_eq!(one, worktree_dir("/w", "/code/app", &listed, "x"));
        assert_eq!(short_hash("/code/app"), short_hash("/code/app"));
        // A checked name never leaves its folder: no `..`, no part that starts with a dot, no `/` kept.
        for branch in ["a/b/c", "feat/x.y"] {
            assert!(check_branch(branch).is_ok());
            let dir = worktree_dir("/w", "/code/app", &[], branch);
            assert_eq!(dir.matches('/').count(), 3, "{dir}");
        }
    }

    #[test]
    fn a_tasks_branch_is_its_first_plain_words_and_never_one_taken() {
        let none = |_: &str| false;
        assert_eq!(branch_for("Fix the zoom flicker, again!", none), "fix-the-zoom-flicker");
        assert_eq!(branch_for("../HEAD @{-1} -x ~^:?*[\\ a.lock", none), "head-1-x-a");
        assert_eq!(branch_for("修正 🎉", none), "task");
        assert_eq!(branch_for(&"a".repeat(300), none).len(), BRANCH_BYTES);
        assert_eq!(branch_for("deploy averylongwordthatcannotfitatall", none), "deploy");
        // Taken (a listed branch, a folder there already): the next number.
        assert_eq!(branch_for("fix zoom", |b| ["fix-zoom", "fix-zoom-2"].contains(&b)), "fix-zoom-3");
        for title in ["", "-", "Fix", "a..b", "x.lock", "@", "HEAD", "日本 x"] {
            assert!(check_branch(&branch_for(title, none)).is_ok(), "{title:?}");
        }
    }

    #[test]
    fn a_repository_is_added_once_and_only_one_listed_as_itself_is_removed() {
        let dir = RepoDir::new;
        let mut dirs = RepoDirs {
            dirs: vec![dir("~/code", "/home/code"), dir("/work/app", "/work/app")],
            fixed: false,
            worktrees: None,
        };
        assert_eq!(dirs.may_add("/tmp/new", true, false), Ok(true));
        // Opt-in: a folder that is not a repository's own is refused, even one that holds some.
        assert_eq!(dirs.may_add("/tmp/folder", false, false), Err("Not a repository: /tmp/folder".into()));
        // Listed already: an entry, or a worktree of one. One found only through a group is added.
        assert_eq!(
            (dirs.may_add("/home/code", false, false), dirs.may_add("/w/app-2", true, true)),
            (Ok(false), Ok(false))
        );
        assert_eq!(dirs.may_add("/home/code/app", true, false), Ok(true));
        assert_eq!(dirs.to_remove("/work/app"), Ok(dir("/work/app", "/work/app")));
        let through = "app is listed in the folder ~/code: remove that from Repos";
        assert_eq!(dirs.to_remove("/home/code/app"), Err(through.into()));
        assert_eq!(dirs.to_remove("/else/app"), Err("app is not in Repos".into()));
        // An entry that did not resolve (`"repos": [""]`) is no repository's: not removed for one.
        let unresolved = RepoDirs { dirs: vec![dir("", "")], fixed: false, worktrees: None };
        assert_eq!(unresolved.to_remove(""), Err("Not a directory: ".into()));
        // No more than the settings may hold; and none while the environment names them.
        dirs.dirs = (0..MAX_DIRS).map(|n| dir(&format!("/d{n}"), &format!("/d{n}"))).collect();
        assert_eq!(dirs.may_add("/tmp/new", true, false), Err("Repos lists 64 repositories already".into()));
        dirs.fixed = true;
        assert_eq!(dirs.to_remove("/d1"), Err(FIXED.into()));
        dirs.dirs.clear();
        assert_eq!(dirs.may_add("/tmp/new", true, false), Err(FIXED.into()));
    }

    #[test]
    fn the_list_keeps_the_entries_order_pinned_first_and_a_groups_repositories_by_name() {
        let at = |path: &str| Repo { path: path.into(), ..repo(&name_of(path), &[]) };
        let pinned = RepoDir { pinned: true, ..RepoDir::new("/w/zed", "/w/zed") };
        let mut dirs = RepoDirs {
            dirs: vec![RepoDir::new("/w/web", "/w/web"), RepoDir::new("~/old", "/old"), pinned],
            ..RepoDirs::default()
        };
        dirs.settle();
        let found = vec![at("/old/b"), at("/w/web"), at("/stray"), at("/old/A"), at("/w/zed")];
        let paths = |repos: Vec<Repo>| repos.into_iter().map(|r| r.path).collect::<Vec<_>>();
        assert_eq!(paths(arrange(found.clone(), &dirs)), ["/w/zed", "/w/web", "/old/A", "/old/b", "/stray"]);
        // `/old` is a group of two; a repository of its own is in none. Flat while the env names them.
        let groups = |dirs: &RepoDirs| found.iter().map(|r| dirs.group_of(&r.path).is_some()).collect::<Vec<_>>();
        assert_eq!(groups(&dirs), [true, false, false, true, false]);
        dirs.fixed = true;
        assert_eq!(groups(&dirs), [false; 5]);
    }

    #[test]
    fn a_repository_found_twice_is_listed_once() {
        // Overlapping places to look find `/code/app` twice; `/work/app` is another repository of that name.
        let other = Repo { path: "/work/app".into(), ..repo("app", &[]) };
        let arranged = arrange(vec![repo("app", &[]), other, repo("app", &[])], &RepoDirs::default());
        let paths: Vec<&str> = arranged.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(paths, ["/code/app", "/work/app"]);
    }
}
