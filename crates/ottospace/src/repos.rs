//! Finds repositories on disk for the Repos section. Reads git's own files; runs no `git`.

use std::fs;
use std::path::{Path, PathBuf};

use ottospace_core::repos::{Repo, Worktree, branch_of, name_of, worktree_path};

/// Where to look. Each directory is a repository itself, or holds repositories one level down.
pub fn roots() -> Vec<PathBuf> {
    crate::config::repo_roots()
}

/// Every repository under `roots()`, unsorted. The core arranges the list.
pub fn scan() -> Vec<Repo> {
    roots().iter().flat_map(|root| found_in(root)).collect()
}

/// The repositories of one directory: itself, or those it holds one level down.
pub fn found_in(root: &Path) -> Vec<Repo> {
    if root.join(".git").exists() {
        return read(root).into_iter().collect();
    }
    let children = fs::read_dir(root).into_iter().flatten().flatten().map(|entry| entry.path());
    children.filter(|dir| dir.join(".git").exists()).filter_map(|dir| read(&dir)).collect()
}

/// The repository at `dir`. A linked worktree (its `.git` is a file) reads as a repository with
/// no worktrees of its own; the core drops it if its main repository lists it.
///
/// Every path is resolved (symlinks, `..`), so a worktree found through its repository and the
/// same directory found by itself have the same path, and the core can tell they are one.
fn read(dir: &Path) -> Option<Repo> {
    let dir = dir.canonicalize().ok()?;
    let dot_git = dir.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        // e.g. `gitdir: /code/app/.git/worktrees/app-2`, or relative to `dir`.
        let pointer = text(&dot_git)?;
        dir.join(pointer.trim().strip_prefix("gitdir: ")?).canonicalize().ok()?
    };
    let branch = |git_dir: &Path| text(&git_dir.join("HEAD")).and_then(|head| branch_of(&head));
    let linked = fs::read_dir(git_dir.join("worktrees")).into_iter().flatten().flatten().filter_map(|entry| {
        // The worktree's `.git` file: absolute, or relative to this entry. Gone if the worktree
        // was deleted and git has not pruned it yet.
        let backlink = entry.path().join(text(&entry.path().join("gitdir"))?.trim()).canonicalize().ok()?;
        let path = worktree_path(&backlink.to_string_lossy())?;
        Some(Worktree { name: name_of(&path), branch: branch(&entry.path()), path })
    });
    let path = dir.to_string_lossy().into_owned();
    Some(Repo { name: name_of(&path), branch: branch(&git_dir), worktrees: linked.collect(), path })
}

/// A small regular file's text. Not a pipe or device (reading one would hang), and not a huge file.
fn text(file: &Path) -> Option<String> {
    let meta = fs::metadata(file).ok()?;
    (meta.is_file() && meta.len() < 4096).then(|| fs::read_to_string(file).ok()).flatten()
}
