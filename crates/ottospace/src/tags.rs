//! Reads what the new-task box's tags may name: agents, skills and paths. Names only, never a
//! file's text. The core checks a line's tags against this (`Action::SetKnown`).
//!
//! Reading is disk I/O, and a path word may name a place that answers slowly (a share that is
//! offline), so a thread of its own does it (`Reader`): the window asks, and takes the answers
//! when they are there. Nothing here is called on the window's thread but `Reader`'s methods.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use ottospace_core::agents::{self, Models};
use ottospace_core::complete::MAX_ROWS;
use ottospace_core::compose::{self, Names, PathFound};

use crate::config;

/// What to read.
pub struct Job {
    /// 0: for the line in the new-task box, where only the newest line matters. Else a socket
    /// call's own number: every one of those is answered, in order.
    pub call: u64,
    pub line: String,
    /// The path being typed at the cursor, to list what finishes it.
    pub typing: Option<String>,
    /// Path words that were put into the line whole (dropped, picked, pasted): each is that
    /// very path, and is not looked for inside the repository.
    pub whole: Vec<String>,
    /// The repository the line names with `>name`: a `/path` may be inside it, and its
    /// `.claude/skills` count.
    pub repo: Option<String>,
    /// Read the agents and skills too. They change seldom: once when the box opens, and when
    /// the repository changes.
    pub names: bool,
}

/// What was read for a job.
pub struct Read {
    pub call: u64,
    pub line: String,
    pub typing: Option<String>,
    pub names: Option<Names>,
    pub paths: Vec<PathFound>,
    pub entries: Vec<String>,
}

pub struct Reader {
    jobs: Sender<Job>,
    read: Receiver<Read>,
}

impl Reader {
    /// `None` if the thread could not start: then no `@agent`, `$skill` or `/path` is ever known.
    pub fn start() -> Option<Self> {
        let (jobs, queue) = channel::<Job>();
        let (send, answers) = channel();
        let spawned = std::thread::Builder::new().name("tags".into()).spawn(move || {
            let (agents_dir, skill_dirs) = (agents_dir(), skill_dirs());
            let models = Models::with(config::get().settings.agent_models.as_ref());
            // Ends when the window is gone.
            while let Ok(first) = queue.recv() {
                let mut waiting: Vec<Job> = std::iter::once(first).chain(queue.try_iter()).collect();
                // Of the lines typed since, only the last is still in the box. If one of those
                // skipped asked for the names, the last one reads them.
                let names = waiting.iter().any(|job| job.call == 0 && job.names);
                let last = waiting.iter().rposition(|job| job.call == 0);
                let mut at = 0;
                waiting.retain(|job| {
                    at += 1;
                    job.call != 0 || Some(at - 1) == last
                });
                for mut job in waiting {
                    job.names |= job.call == 0 && names;
                    if send.send(read(job, &models, agents_dir.as_deref(), &skill_dirs)).is_err() {
                        return;
                    }
                }
            }
        });
        spawned.ok().map(|_| Self { jobs, read: answers })
    }

    pub fn ask(&self, job: Job) {
        let _ = self.jobs.send(job);
    }

    /// The answers that are in, oldest first.
    pub fn take(&self) -> Vec<Read> {
        self.read.try_iter().collect()
    }
}

fn read(job: Job, models: &Models, agents_dir: Option<&Path>, skill_dirs: &[PathBuf]) -> Read {
    let repo = job.repo.as_deref().map(Path::new);
    let names = job.names.then(|| Names { agents: agents(models, agents_dir), skills: skills(skill_dirs, repo) });
    let paths = paths(&compose::path_words(&job.line), repo, &job.whole);
    let entries = job.typing.as_deref().map(|typed| entries(typed, repo)).unwrap_or_default();
    Read { call: job.call, line: job.line, typing: job.typing, names, paths, entries }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|home| !home.is_empty()).map(PathBuf::from)
}

/// The user's folder of saved agents, if there is a user's folder.
fn agents_dir() -> Option<PathBuf> {
    config::get().home.as_ref().map(|home| home.join("agents"))
}

/// The runners' skill folders: `$OTTOSPACE_SKILLS` (directories, `:` between them) if set, else
/// `skills` in each runner's config folder. A test run reads none of the user's.
fn skill_dirs() -> Vec<PathBuf> {
    if let Some(list) = std::env::var_os("OTTOSPACE_SKILLS") {
        return std::env::split_paths(&list).collect();
    }
    if std::env::var_os("OTTOSPACE_E2E").is_some() {
        return Vec::new();
    }
    let runners = ottospace_sessions::config_dirs();
    runners.claude.into_iter().chain(runners.codex).map(|dir| dir.join("skills")).collect()
}

/// The built-in runners and their models, then each `<name>.md` in `dir`, if there is one.
fn agents(models: &Models, dir: Option<&Path>) -> Vec<String> {
    let files = dir_names(dir, |path| path.is_file());
    let mut saved: Vec<String> = files.iter().filter_map(|name| name.strip_suffix(".md").map(str::to_string)).collect();
    saved.sort();
    agents::names(models, saved)
}

/// The names in `dir` whose path passes `keep`. No folder: none.
fn dir_names(dir: Option<&Path>, keep: impl Fn(&Path) -> bool) -> Vec<String> {
    let entries = dir.and_then(|dir| std::fs::read_dir(dir).ok());
    let names = entries.into_iter().flatten().flatten().filter(|entry| keep(&entry.path()));
    names.filter_map(|entry| entry.file_name().into_string().ok()).collect()
}

/// Skill names: each folder with a `SKILL.md` in `dirs` and in the repository's `.claude/skills`.
fn skills(dirs: &[PathBuf], repo: Option<&Path>) -> Vec<String> {
    let repo = repo.map(|repo| repo.join(".claude/skills"));
    let mut names: Vec<String> =
        dirs.iter().chain(&repo).flat_map(|dir| dir_names(Some(dir), |path| path.join("SKILL.md").is_file())).collect();
    names.sort();
    names.dedup();
    names
}

/// Where a path word may be, most likely first: `~/x` under the home folder; `/x` inside `repo`,
/// then at the root.
fn places(word: &str, repo: Option<&Path>) -> Vec<PathBuf> {
    match word.strip_prefix("~/") {
        Some(rest) => home().map(|home| home.join(rest)).into_iter().collect(),
        None => {
            let inside = repo.map(|repo| repo.join(word.trim_start_matches('/')));
            inside.into_iter().chain([PathBuf::from(word)]).collect()
        }
    }
}

/// The path words that name a file or directory, with where each was found. For each word, the
/// first of what it may be (`compose::path_words`) that is somewhere, in the first place it is.
fn paths(words: &[Vec<String>], repo: Option<&Path>, whole: &[String]) -> Vec<PathFound> {
    let found = |typed: &String| {
        // A path dropped on the box is that file: `/tmp/a.txt`, not the repository's `tmp/a.txt`.
        let repo = repo.filter(|_| !whole.contains(typed));
        let place = places(typed, repo).into_iter().find(|place| place.exists())?;
        Some(PathFound { typed: typed.clone(), resolved: place.to_string_lossy().into_owned() })
    };
    words.iter().filter_map(|candidates| candidates.iter().find_map(found)).collect()
}

/// Paths that finish `typed`: the entries of its folder whose names start as its last part does.
/// e.g. `/src/ex` → `/src/export/`. Those inside the repository come before those at the root.
/// Hidden files only when a `.` is typed. At most `MAX_ROWS`: only those are looked at
/// to see which is a directory.
fn entries(typed: &str, repo: Option<&Path>) -> Vec<String> {
    let cut = typed.rfind('/').map_or(0, |at| at + 1);
    let (folder, start) = typed.split_at(cut);
    let fits = |name: &String| name.starts_with(start) && (start.starts_with('.') || !name.starts_with('.'));
    let mut found: Vec<(PathBuf, String)> = Vec::new();
    for place in places(folder, repo) {
        let mut names: Vec<String> = dir_names(Some(&place), |_| true).into_iter().filter(fits).collect();
        names.sort();
        for name in names {
            if !found.iter().any(|(_, seen)| *seen == name) {
                found.push((place.join(&name), name));
            }
        }
    }
    found.truncate(MAX_ROWS);
    let slash = |path: &Path| if path.is_dir() { "/" } else { "" };
    found.iter().map(|(path, name)| format!("{folder}{name}{}", slash(path))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of this test's own, empty.
    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ottospace-tags-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn words(line: &str) -> Vec<Vec<String>> {
        compose::path_words(line)
    }

    #[test]
    fn a_path_is_looked_for_inside_the_repository_then_at_the_root() {
        let repo = temp("paths");
        std::fs::create_dir_all(repo.join("src/export")).unwrap();
        std::fs::create_dir_all(repo.join("My Notes")).unwrap();
        std::fs::write(repo.join("src/extra.rs"), "").unwrap();
        std::fs::write(repo.join("src/.hidden"), "").unwrap();
        std::fs::write(repo.join("src/v1."), "").unwrap();
        // `/tmp` is at the root of every Mac; inside the repository it is the repository's.
        std::fs::create_dir_all(repo.join("tmp")).unwrap();
        let inside = Some(repo.as_path());
        let at = |part: &str| repo.join(part).to_string_lossy().into_owned();
        let found = |typed: &str, resolved: String| PathFound { typed: typed.into(), resolved };

        let read = paths(&words("see /src/export, /nowhere-at-all and /tmp"), inside, &[]);
        assert_eq!(read, [found("/src/export", at("src/export")), found("/tmp", at("tmp"))]);
        // Put in whole, `/tmp` is the root's, whatever the repository has of that name.
        let whole = paths(&words("see /src/export and /tmp"), inside, &["/tmp".to_string()]);
        assert_eq!(whole, [found("/src/export", at("src/export")), found("/tmp", "/tmp".into())]);
        // Without the repository the same word is the root's, or nothing.
        assert_eq!(paths(&words("see /src/export and /tmp"), None, &[]), [found("/tmp", "/tmp".into())]);
        // An absolute path counts by itself.
        let absolute = format!("look in {}", at("src"));
        assert_eq!(paths(&words(&absolute), None, &[]), [found(&at("src"), at("src"))]);
        // The word as typed first: a name may end in a dot. A space is written with a backslash.
        let read = paths(&words(r"read /src/v1. and /src/extra.rs. in /My\ Notes"), inside, &[]);
        let typed: Vec<&str> = read.iter().map(|path| path.typed.as_str()).collect();
        assert_eq!(typed, ["/src/v1.", "/src/extra.rs", "/My Notes"]);

        // Completion: the repository's entries first, then the root's; a directory ends in `/`.
        assert_eq!(entries("/src/ex", inside), ["/src/export/", "/src/extra.rs"]);
        assert_eq!(entries("/src/.", inside), ["/src/.hidden"]);
        assert_eq!(entries("/My", inside), ["/My Notes/"]);
        let both = entries("/tm", inside);
        assert_eq!(both, ["/tmp/"], "the repository's and the root's are one name: once");
        assert!(entries("/", inside).len() <= MAX_ROWS);
        assert_eq!(entries("/", inside)[0], "/My Notes/", "the repository's come first");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn agents_and_skills_are_read_from_the_folders_given() {
        let dir = temp("names");
        // No folder: the built-in runners, each with its models.
        let models = Models { claude: vec!["claude-opus-5-5".into()], codex: Vec::new() };
        assert_eq!(agents(&models, None), ["claude", "claude-opus-5-5", "codex"]);
        assert_eq!(agents(&models, Some(&dir.join("none"))), ["claude", "claude-opus-5-5", "codex"]);
        std::fs::create_dir_all(dir.join("agents/sub.md")).unwrap();
        for file in ["reviewer.md", "claude.md", "claude-opus-5-5.md", "codex-reviewer.md", "notes.txt", "ana.md"] {
            std::fs::write(dir.join("agents").join(file), "").unwrap();
        }
        let read = agents(&models, Some(&dir.join("agents")));
        assert_eq!(read, ["claude", "claude-opus-5-5", "codex", "ana", "reviewer"]);

        // A skill is a folder with a `SKILL.md`: in the runners' folders and the repository's.
        let skill = |root: &str, name: &str| {
            std::fs::create_dir_all(dir.join(root).join(name)).unwrap();
            std::fs::write(dir.join(root).join(name).join("SKILL.md"), "").unwrap();
        };
        skill("claude/skills", "deploy");
        skill("codex/skills", "code-review");
        skill("codex/skills", "deploy");
        skill("repo/.claude/skills", "release");
        std::fs::create_dir_all(dir.join("claude/skills/empty")).unwrap();
        let dirs = [dir.join("claude/skills"), dir.join("codex/skills"), dir.join("gone")];
        assert_eq!(skills(&dirs, None), ["code-review", "deploy"]);
        assert_eq!(skills(&dirs, Some(&dir.join("repo"))), ["code-review", "deploy", "release"]);
        assert_eq!(skills(&[], None), Vec::<String>::new());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
