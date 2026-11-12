//! A repository's checkouts, read from `.git` without running git.

use std::fs;
use std::path::{Component, Path, PathBuf};

/// The main checkout and every linked worktree of the repository `dir` is in, main first.
/// Outside a repository: `dir` alone. e.g. from `/code/app-wt/src` → `[/code/app, /code/app-wt]`.
pub fn checkouts(dir: &Path) -> Vec<PathBuf> {
    let Some(root) = dir.ancestors().find(|d| d.join(".git").exists()) else { return vec![dir.to_path_buf()] };
    let Some(common) = common_dir(root) else { return vec![root.to_path_buf()] };
    // The main checkout holds `.git`; a bare repository has none.
    let main = common.parent().filter(|_| common.ends_with(".git")).map(PathBuf::from);
    // Each `<common>/worktrees/<name>/gitdir` holds the path of that worktree's `.git` file.
    let mut linked: Vec<PathBuf> = fs::read_dir(common.join("worktrees"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|worktree| fs::read_to_string(worktree.path().join("gitdir")).ok())
        .filter_map(|gitdir| Path::new(gitdir.trim()).parent().map(PathBuf::from))
        .collect();
    linked.sort();
    let all: Vec<PathBuf> = main.into_iter().chain(linked).collect();
    if all.is_empty() { vec![root.to_path_buf()] } else { all }
}

/// The repository's shared `.git` directory, from a main checkout or a linked worktree.
fn common_dir(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    // A worktree's `.git` is a file: `gitdir: <common>/worktrees/<name>`, where `commondir` says `../..`.
    let text = fs::read_to_string(&dot_git).ok()?;
    let admin = root.join(text.trim().strip_prefix("gitdir:")?.trim());
    let common = fs::read_to_string(admin.join("commondir")).ok()?;
    Some(normalize(&admin.join(common.trim())))
}

/// Resolves `.` and `..` by name alone, e.g. `/a/b/../c` → `/a/c`.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_by_name() {
        assert_eq!(normalize(Path::new("/a/.git/worktrees/x/../..")), Path::new("/a/.git"));
        assert_eq!(normalize(Path::new("/a/./b/../c")), Path::new("/a/c"));
    }
}
