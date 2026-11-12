//! Finds repositories on disk for the Repos section. Reads git's own files; runs no `git`.

use std::fs;
use std::path::{Path, PathBuf};

use ottospace_core::repos::{Repo, Worktree, branch_of, name_of, worktree_path};

/// Where to look: `$OTTOSPACE_REPOS` (directories, `:` between them), else `~/code` and `~/notes`.
/// Each directory is a repository itself, or holds repositories one level down.
pub fn roots() -> Vec<PathBuf> {
    match std::env::var_os("OTTOSPACE_REPOS") {
        Some(list) => std::env::split_paths(&list).filter(|p| !p.as_os_str().is_empty()).collect(),
        None => std::env::var_os("HOME")
            .map(|home| vec![Path::new(&home).join("code"), Path::new(&home).join("notes")])
            .unwrap_or_default(),
    }
}

/// Every repository under `roots`, unsorted. The core arranges the list.
pub fn scan(roots: &[PathBuf]) -> Vec<Repo> {
    let mut repos = Vec::new();
    for root in roots {
        if root.join(".git").exists() {
            repos.extend(read(root));
            continue;
        }
        let children = fs::read_dir(root).into_iter().flatten().flatten().map(|entry| entry.path());
        repos.extend(children.filter(|dir| dir.join(".git").exists()).filter_map(|dir| read(&dir)));
    }
    repos
}

/// The repository at `dir`. A linked worktree (its `.git` is a file) reads as a repository with
/// no worktrees of its own; the core drops it if its main repository lists it.
fn read(dir: &Path) -> Option<Repo> {
    let dot_git = dir.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        // e.g. `gitdir: /code/app/.git/worktrees/app-2`
        PathBuf::from(fs::read_to_string(&dot_git).ok()?.trim().strip_prefix("gitdir: ")?)
    };
    let branch = |git_dir: &Path| fs::read_to_string(git_dir.join("HEAD")).ok().and_then(|head| branch_of(&head));
    let linked = fs::read_dir(git_dir.join("worktrees")).into_iter().flatten().flatten().filter_map(|entry| {
        let path = worktree_path(&fs::read_to_string(entry.path().join("gitdir")).ok()?)?;
        // A worktree whose directory was deleted stays in git's list until it is pruned.
        Path::new(&path).is_dir().then(|| Worktree { name: name_of(&path), branch: branch(&entry.path()), path })
    });
    let path = dir.to_string_lossy().into_owned();
    Some(Repo { name: name_of(&path), branch: branch(&git_dir), worktrees: linked.collect(), path })
}
