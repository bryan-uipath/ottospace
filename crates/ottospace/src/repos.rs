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

/// The repository `dir` (resolved) is in: the nearest folder up with a `.git`; a linked
/// worktree's (`gitdir: /code/app/.git/worktrees/app-2`) is its main repository, `/code/app`.
/// `None`: in none.
pub fn repository_of(dir: &Path) -> Option<PathBuf> {
    let top = dir.ancestors().find(|at| at.join(".git").exists())?;
    let pointer = text(&top.join(".git")).and_then(|text| Some(top.join(text.trim().strip_prefix("gitdir: ")?)));
    let main = pointer.and_then(|git_dir| {
        let worktrees = git_dir.canonicalize().ok()?.parent()?.to_path_buf();
        let dot_git = worktrees.parent().filter(|_| worktrees.ends_with("worktrees"))?;
        dot_git.ends_with(".git").then(|| dot_git.parent().map(Path::to_path_buf)).flatten()
    });
    Some(main.unwrap_or_else(|| top.to_path_buf()))
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
    let git_dir = git_dir(&dir)?;
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

/// The checkout `dir` is in: the nearest folder up with a `.git`, and its branch. `None`: in none.
pub fn checkout_of(dir: &Path) -> Option<(PathBuf, Option<String>)> {
    let top = dir.ancestors().find(|at| at.join(".git").exists())?;
    Some((top.to_path_buf(), branch(&git_dir(top)?)))
}

/// The length of a checkout's `HEAD` log (`<gitdir>/logs/HEAD`): it grows by a line per commit.
/// One stat, no `git`. A checkout with no log yet (no commit) is 0, so its first commit grows it.
/// `None`: no checkout, or the log cannot be read.
pub fn head_log(checkout: &Path) -> Option<u64> {
    match fs::metadata(git_dir(checkout)?.join("logs").join("HEAD")) {
        Ok(meta) => meta.is_file().then(|| meta.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(0),
        Err(_) => None,
    }
}

/// A checkout's git folder: its `.git`, or where its `.git` file points (a linked worktree's).
fn git_dir(dir: &Path) -> Option<PathBuf> {
    let dot_git = dir.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    // e.g. `gitdir: /code/app/.git/worktrees/app-2`, or relative to `dir`.
    let pointer = text(&dot_git)?;
    dir.join(pointer.trim().strip_prefix("gitdir: ")?).canonicalize().ok()
}

/// The branch checked out, or a short commit id, as `HEAD` in `git_dir` says.
fn branch(git_dir: &Path) -> Option<String> {
    text(&git_dir.join("HEAD")).and_then(|head| branch_of(&head))
}

/// A small regular file's text. Not a pipe or device (reading one would hang), and not a huge file.
fn text(file: &Path) -> Option<String> {
    small_text(file, 4096)
}

/// As `text`, up to `most` bytes.
pub(crate) fn small_text(file: &Path, most: u64) -> Option<String> {
    let meta = fs::metadata(file).ok()?;
    (meta.is_file() && meta.len() < most).then(|| fs::read_to_string(file).ok()).flatten()
}
