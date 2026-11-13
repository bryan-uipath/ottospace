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

/// How many directories the settings may name through the Repos section's +.
pub const MAX_DIRS: usize = 64;

/// A directory repositories are looked for in: `text` as the settings name it (`~/code`), and
/// `path` resolved (`/Users/me/code`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepoDir {
    pub text: String,
    pub path: String,
}

/// Where the Repos list is read from.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct RepoDirs {
    pub dirs: Vec<RepoDir>,
    /// `OTTOSPACE_REPOS` names them, not the settings: none is added or removed.
    pub fixed: bool,
}

const FIXED: &str = "OTTOSPACE_REPOS names the repositories: the settings are not read";

impl RepoDirs {
    /// Why `dir` (resolved), where `found` repositories are, cannot be added. `Ok(false)`: it is
    /// listed already, as a directory of the settings or (`listed`) through one.
    pub fn may_add(&self, dir: &str, found: usize, listed: bool) -> Result<bool, String> {
        match () {
            _ if !dir.starts_with('/') => Err(format!("Not a directory: {dir}")),
            _ if found == 0 => Err(format!("No repository in {dir}")),
            _ if listed || self.dirs.iter().any(|d| d.path == dir) => Ok(false),
            _ if self.fixed => Err(FIXED.into()),
            _ if self.dirs.len() >= MAX_DIRS => Err(format!("The settings name {MAX_DIRS} directories already")),
            _ => Ok(true),
        }
    }

    /// The directory to take off the settings so the repository at `path` is no longer listed.
    /// `Err`: why none is. One that holds other repositories too is the user's to remove.
    pub fn to_remove(&self, path: &str) -> Result<RepoDir, String> {
        match (self.dirs.iter().find(|d| d.path == path), self.holder(path)) {
            _ if self.fixed => Err(FIXED.into()),
            (Some(dir), _) => Ok(dir.clone()),
            (None, Some(holder)) => Err(format!(
                "{} is listed through {}: take that directory out of `repos` in settings.json",
                name_of(path),
                holder.text
            )),
            (None, None) => Err(format!("{} is not listed by the settings", name_of(path))),
        }
    }

    /// The directory that lists `path` as one of the repositories it holds: its parent.
    pub fn holder(&self, path: &str) -> Option<&RepoDir> {
        let parent = path.rsplit_once('/').map_or("", |(parent, _)| parent);
        self.dirs.iter().find(|d| d.path == parent)
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
    /// The line under the branch: `Default`, or a linked worktree's directory name.
    pub label: String,
}

impl Repo {
    /// The repository's rows: its own directory first, then its worktrees in their order.
    pub fn checkouts(&self) -> Vec<Checkout> {
        let row = |path: &str, branch: &Option<String>, worktree: Option<&str>| Checkout {
            path: path.to_string(),
            branch: branch.clone().unwrap_or_else(|| "(no branch)".into()),
            default: worktree.is_none(),
            label: worktree.unwrap_or("Default").to_string(),
        };
        let linked = self.worktrees.iter().map(|w| row(&w.path, &w.branch, Some(&w.name)));
        std::iter::once(row(&self.path, &self.branch, None)).chain(linked).collect()
    }
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

/// The list as shown: repositories by name, each one's worktrees by name. A directory that is a
/// worktree of a listed repository is not listed again as a repository, and a repository found
/// twice (overlapping places to look) is listed once. Paths must be in one form: the shell
/// resolves them before they get here.
pub fn arrange(found: Vec<Repo>) -> Vec<Repo> {
    let linked: Vec<String> = found.iter().flat_map(|r| r.worktrees.iter().map(|w| w.path.clone())).collect();
    let mut repos: Vec<Repo> = Vec::new();
    for mut repo in found {
        if linked.contains(&repo.path) || repos.iter().any(|r| r.path == repo.path) {
            continue;
        }
        repo.worktrees.sort_by(|a, b| a.name.cmp(&b.name));
        repos.push(repo);
    }
    repos.sort_by(|a, b| (a.name.to_lowercase(), &a.path).cmp(&(b.name.to_lowercase(), &b.path)));
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
        let arranged = arrange(vec![repo("zed", &[]), repo("app-2", &[]), repo("App", &["app-3", "app-2"])]);
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
            row("/code/app", "main", true, "Default"),
            row("/code/app-2", "fix/zoom", false, "app-2"),
            row("/code/Default", "(no branch)", false, "Default"),
        ];
        assert_eq!(app.checkouts(), expected);
    }

    #[test]
    fn a_directory_is_added_once_and_only_one_listed_as_itself_is_removed() {
        let dir = |text: &str, path: &str| RepoDir { text: text.into(), path: path.into() };
        let mut dirs =
            RepoDirs { dirs: vec![dir("~/code", "/home/code"), dir("/work/app", "/work/app")], fixed: false };
        assert_eq!(dirs.may_add("/tmp/new", 2, false), Ok(true));
        assert_eq!(dirs.may_add("/tmp/empty", 0, false), Err("No repository in /tmp/empty".into()));
        // Listed already: a directory of the settings, or a repository found through one.
        assert_eq!(
            (dirs.may_add("/home/code", 3, false), dirs.may_add("/home/code/app", 1, true)),
            (Ok(false), Ok(false))
        );
        assert_eq!(dirs.to_remove("/work/app"), Ok(dir("/work/app", "/work/app")));
        let through = "app is listed through ~/code: take that directory out of `repos` in settings.json";
        assert_eq!(dirs.to_remove("/home/code/app"), Err(through.into()));
        assert_eq!(dirs.to_remove("/else/app"), Err("app is not listed by the settings".into()));
        // No more than the settings may hold; and none while the environment names them.
        dirs.dirs = (0..MAX_DIRS).map(|n| dir(&format!("/d{n}"), &format!("/d{n}"))).collect();
        assert_eq!(dirs.may_add("/tmp/new", 1, false), Err("The settings name 64 directories already".into()));
        dirs.fixed = true;
        assert_eq!(dirs.to_remove("/d1"), Err(FIXED.into()));
        dirs.dirs.clear();
        assert_eq!(dirs.may_add("/tmp/new", 1, false), Err(FIXED.into()));
    }

    #[test]
    fn a_repository_found_twice_is_listed_once() {
        // Overlapping places to look find `/code/app` twice; `/work/app` is another repository of that name.
        let other = Repo { path: "/work/app".into(), ..repo("app", &[]) };
        let arranged = arrange(vec![repo("app", &[]), other, repo("app", &[])]);
        let paths: Vec<&str> = arranged.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(paths, ["/code/app", "/work/app"]);
    }
}
