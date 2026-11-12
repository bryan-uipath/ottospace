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
/// worktree of a listed repository is not listed again as a repository.
pub fn arrange(mut repos: Vec<Repo>) -> Vec<Repo> {
    let linked: Vec<String> = repos.iter().flat_map(|r| r.worktrees.iter().map(|w| w.path.clone())).collect();
    repos.retain(|r| !linked.contains(&r.path));
    for repo in &mut repos {
        repo.worktrees.sort_by(|a, b| a.name.cmp(&b.name));
    }
    repos.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    repos.dedup_by(|a, b| a.path == b.path);
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
}
