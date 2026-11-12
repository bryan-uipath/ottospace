//! Reads what the new-task box's tags may name: agents, skills and paths. Names only, never a
//! file's text. The core checks a line's tags against this (`Action::SetKnown`).

use std::path::{Path, PathBuf};

use ottospace_core::compose::{self, Known};

use crate::config;

/// The most paths offered for the one being typed.
const ENTRIES: usize = 5;

/// What the tags of `line` may name. `repo`: the repository the line names with `>name`; a
/// `/path` may be inside it, and its `.claude/skills` count.
pub fn known(line: &str, repo: Option<&str>) -> Known {
    let repo = repo.map(Path::new);
    let paths = compose::path_words(line).into_iter().filter(|word| places(word, repo).iter().any(|p| p.exists()));
    let entries = match compose::completing(line) {
        Some(('/', typed)) => entries(typed, repo),
        _ => Vec::new(),
    };
    Known { agents: agents(), skills: skills(repo), paths: paths.map(str::to_string).collect(), entries }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|home| !home.is_empty()).map(PathBuf::from)
}

/// The built-in runners, then each `agents/<name>.md` of the user's folder, if it has one.
fn agents() -> Vec<String> {
    let mut names = vec!["claude".to_string(), "codex".to_string()];
    let folder = config::get().home.as_ref().map(|home| home.join("agents"));
    let mut files: Vec<String> = dir_names(folder.as_deref(), |path| path.is_file())
        .into_iter()
        .filter_map(|name| name.strip_suffix(".md").map(str::to_string))
        .filter(|name| !names.contains(name))
        .collect();
    files.sort();
    names.append(&mut files);
    names
}

/// The names in `dir` whose path passes `keep`. No folder: none.
fn dir_names(dir: Option<&Path>, keep: impl Fn(&Path) -> bool) -> Vec<String> {
    let entries = dir.and_then(|dir| std::fs::read_dir(dir).ok());
    let names = entries.into_iter().flatten().flatten().filter(|entry| keep(&entry.path()));
    names.filter_map(|entry| entry.file_name().into_string().ok()).collect()
}

/// Skill names: each folder with a `SKILL.md` in the runners' skill folders and the repository's.
/// `$OTTOSPACE_SKILLS` (directories, `:` between them) replaces the runners' folders. A test run
/// reads none of the user's.
fn skills(repo: Option<&Path>) -> Vec<String> {
    let set = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    let mut dirs: Vec<PathBuf> = if let Some(list) = std::env::var_os("OTTOSPACE_SKILLS") {
        std::env::split_paths(&list).collect()
    } else if std::env::var_os("OTTOSPACE_E2E").is_some() {
        Vec::new()
    } else {
        let claude = set("CLAUDE_CONFIG_DIR").map(PathBuf::from).into_iter().chain(home().map(|h| h.join(".claude")));
        let codex = set("CODEX_HOME").map(PathBuf::from).or(home().map(|h| h.join(".codex")));
        claude.chain(codex).map(|dir| dir.join("skills")).collect()
    };
    dirs.extend(repo.map(|repo| repo.join(".claude/skills")));
    let mut names: Vec<String> =
        dirs.iter().flat_map(|dir| dir_names(Some(dir), |path| path.join("SKILL.md").is_file())).collect();
    names.sort();
    names.dedup();
    names
}

/// Where a path word may be: `~/x` under the home folder; `/x` at the root, or inside `repo`.
fn places(word: &str, repo: Option<&Path>) -> Vec<PathBuf> {
    match word.strip_prefix("~/") {
        Some(rest) => home().map(|home| home.join(rest)).into_iter().collect(),
        None => {
            let inside = repo.map(|repo| repo.join(word.trim_start_matches('/')));
            inside.into_iter().chain([PathBuf::from(word)]).collect()
        }
    }
}

/// Paths that finish `typed`: the entries of its folder whose names start as its last part does.
/// e.g. `/src/ex` → `/src/export/`. Hidden files only when a `.` is typed.
fn entries(typed: &str, repo: Option<&Path>) -> Vec<String> {
    let cut = typed.rfind('/').map_or(0, |at| at + 1);
    let (folder, start) = typed.split_at(cut);
    let mut found: Vec<String> = Vec::new();
    for place in places(folder, repo) {
        let fits = |name: &String| name.starts_with(start) && (start.starts_with('.') || !name.starts_with('.'));
        for name in dir_names(Some(&place), |_| true).into_iter().filter(fits) {
            let slash = if place.join(&name).is_dir() { "/" } else { "" };
            found.push(format!("{folder}{name}{slash}"));
        }
    }
    found.sort();
    found.dedup();
    found.truncate(ENTRIES);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_looked_for_at_the_root_and_inside_the_repository() {
        let repo = std::env::temp_dir().join(format!("ottospace-tags-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(repo.join("src/export")).unwrap();
        std::fs::create_dir_all(repo.join(".claude/skills/deploy")).unwrap();
        std::fs::write(repo.join(".claude/skills/deploy/SKILL.md"), "").unwrap();
        std::fs::write(repo.join("src/extra.rs"), "").unwrap();
        std::fs::write(repo.join("src/.hidden"), "").unwrap();
        let inside = repo.to_str();
        let read = known("see /src/export, /nowhere-at-all and /src/ex", inside);
        assert_eq!(read.paths, ["/src/export"]);
        assert_eq!(read.entries, ["/src/export/", "/src/extra.rs"]);
        assert_eq!(known("see /src/.", inside).entries, ["/src/.hidden"]);
        // Without the repository the same words name nothing.
        assert_eq!(known("see /src/export", None).paths, Vec::<String>::new());
        // An absolute path counts by itself.
        let absolute = format!("look in {}/src", repo.display());
        assert_eq!(known(&absolute, None).paths.len(), 1);
        assert!(skills(Some(&repo)).contains(&"deploy".to_string()));
        assert_eq!(&agents()[..2], ["claude", "codex"]);
        let _ = std::fs::remove_dir_all(&repo);
    }
}
