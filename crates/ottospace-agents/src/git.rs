//! A repository's checkouts, read from `.git` without running git.
//! The app's `repos.rs` reads the same files for its Repos list (with branches); the rules
//! here follow it: resolved paths, relative pointers, small regular files only.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The main checkout and every linked worktree of the repository `dir` is in, main first, as
/// resolved paths. Outside a repository: `dir` alone. A relative or empty `dir`, or one that is
/// not there, is an error.
/// e.g. from `/code/app-wt/src` → `[/code/app, /code/app-wt]`.
pub fn checkouts(dir: &Path) -> io::Result<Vec<PathBuf>> {
    if !dir.is_absolute() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("not an absolute directory: {dir:?}")));
    }
    // Runners record the real path, e.g. `/private/tmp/x` for `/tmp/x`. This resolves `..` too.
    let dir = dir.canonicalize()?;
    let alone = vec![dir.clone()];
    let Some(root) = dir.ancestors().find(|d| d.join(".git").exists()) else { return Ok(alone) };
    let Some(common) = common_dir(root) else { return Ok(alone) };
    // The main checkout holds `.git`; a bare repository has none.
    let main = common.parent().filter(|_| common.ends_with(".git")).map(PathBuf::from);
    // Each `<common>/worktrees/<name>/gitdir` holds the path of that worktree's `.git` file:
    // absolute, or relative to the entry. Gone if the worktree was deleted and not pruned yet.
    let mut linked: Vec<PathBuf> = fs::read_dir(common.join("worktrees"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.path().join(text(&entry.path().join("gitdir"))?.trim()).canonicalize().ok())
        .filter_map(|dot_git| dot_git.parent().map(PathBuf::from))
        .collect();
    linked.sort();
    let all: Vec<PathBuf> = main.into_iter().chain(linked).collect();
    Ok(if all.is_empty() { alone } else { all })
}

/// The repository's shared `.git` directory, from a main checkout or a linked worktree.
fn common_dir(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    if dot_git.is_dir() {
        return dot_git.canonicalize().ok();
    }
    // A worktree's `.git` is a file: `gitdir: <common>/worktrees/<name>`, absolute or relative to
    // the worktree. In that folder `commondir` says `../..`.
    let pointer = text(&dot_git)?;
    let admin = root.join(pointer.trim().strip_prefix("gitdir:")?.trim()).canonicalize().ok()?;
    admin.join(text(&admin.join("commondir"))?.trim()).canonicalize().ok()
}

/// A small regular file's text. Not a pipe or device (reading one would hang), and not a huge file.
fn text(file: &Path) -> Option<String> {
    let meta = fs::metadata(file).ok()?;
    (meta.is_file() && meta.len() < 4096).then(|| fs::read_to_string(file).ok()).flatten()
}
