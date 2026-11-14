//! Reads what the new-task box's tags may name: agents, skills and paths. Names only, never a
//! file's text. The core checks a line's tags against this (`Action::SetKnown`).
//!
//! Reading is disk I/O, and a path word may name a place that answers slowly (a share that is
//! offline), so a thread of its own does it (`Reader`): the window asks, and takes the answers
//! when they are there. Nothing here is called on the window's thread but `Reader`'s methods.

use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use ottospace_core::agents::{self, Models};
use ottospace_core::compose::{self, Names, PathFound};

use crate::config;

/// The most entries of one folder listed: the core narrows them to what is typed.
const MAX_LISTED: usize = 2000;

/// What to read.
pub struct Job {
    /// 0: for the line in the new-task box, where only the newest line matters. Else a socket
    /// call's own number: every one of those is answered, in order.
    pub call: u64,
    pub line: String,
    /// The folder of the path being typed at the cursor (`~/`, `./src/`), to list.
    pub typing: Option<String>,
    /// The repository the line names with `>name`: a `./path` is inside it, and its
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
    /// The folder listed, as typed and where it is.
    pub typing: Option<(String, String)>,
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
    let roots = Roots::now();
    let paths = paths(&compose::path_words(&job.line), repo, &roots);
    // The folder, with where it is: a folder taken from it is there, with no second read.
    let place = |folder: String| Some((roots.place(&folder, repo)?.to_string_lossy().into_owned(), folder));
    let typing = job.typing.and_then(place).map(|(place, folder)| (folder, place));
    let entries = typing.as_ref().map(|(folder, _)| entries(folder, repo, &roots)).unwrap_or_default();
    Read { call: job.call, line: job.line, typing, names, paths, entries }
}

/// What a path word may reach. `home`: the folder `~` stands for. `inside`: a path in none of
/// these is never looked at; `None`: any is.
struct Roots {
    home: Option<PathBuf>,
    inside: Option<Vec<PathBuf>>,
}

impl Roots {
    /// `$HOME`, anywhere. A test run (`OTTOSPACE_E2E`) reads no folder of the user's: `~` is
    /// `OTTOSPACE_PATH_HOME` (none named: no folder), and nothing outside the temp folder is
    /// looked at, where a test makes its own.
    fn now() -> Self {
        let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty()).map(PathBuf::from);
        let test = std::env::var_os("OTTOSPACE_E2E").is_some();
        let home = var("OTTOSPACE_PATH_HOME").or_else(|| var("HOME").filter(|_| !test));
        // As named and with its links resolved: `/var/folders/..` is `/private/var/folders/..`.
        let temp = std::env::temp_dir();
        let inside = test.then(|| temp.canonicalize().into_iter().chain([temp.clone()]).collect());
        Roots { home, inside }
    }

    /// Where a path word is, with `.` and `..` taken out by its text: `~/x` in the home folder,
    /// `./x` and `../x` beside the repository's own, `/x` from the root of the disk. `None`: it
    /// has none, or one out of reach.
    fn place(&self, word: &str, repo: Option<&Path>) -> Option<PathBuf> {
        let place = match word.strip_prefix("~/") {
            Some(rest) => self.home.as_ref()?.join(rest),
            None if compose::relative(word) => repo?.join(word),
            None if word.starts_with('/') => PathBuf::from(word),
            None => return None,
        };
        let mut plain = PathBuf::new();
        for part in place.components() {
            match part {
                Component::CurDir => {}
                Component::ParentDir => drop(plain.pop()),
                part => plain.push(part),
            }
        }
        let reached = self.inside.as_ref().is_none_or(|inside| inside.iter().any(|dir| plain.starts_with(dir)));
        reached.then_some(plain)
    }
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

/// The path words that name a file or directory, with where each was found. For each word, the
/// first of what it may be (`compose::path_words`) that is there.
fn paths(words: &[Vec<String>], repo: Option<&Path>, roots: &Roots) -> Vec<PathFound> {
    let found = |typed: &String| {
        let place = roots.place(typed, repo).filter(|place| place.exists())?;
        Some(PathFound { typed: typed.clone(), resolved: place.to_string_lossy().into_owned() })
    };
    words.iter().filter_map(|candidates| candidates.iter().find_map(found)).collect()
}

/// The entries of `folder` (as typed, ending in `/`), each written after it, a directory's with
/// a `/`: `~/` → `~/notes/`, `~/notes.md`. At most `MAX_LISTED`. A link is listed as what it
/// points at, and never followed further. A folder that does not read lists nothing.
fn entries(folder: &str, repo: Option<&Path>, roots: &Roots) -> Vec<String> {
    let listed = roots.place(folder, repo).and_then(|place| std::fs::read_dir(place).ok());
    let mut found = Vec::new();
    for entry in listed.into_iter().flatten().flatten().take(MAX_LISTED) {
        let Ok(name) = entry.file_name().into_string() else { continue };
        let linked = |entry: &std::fs::DirEntry| std::fs::metadata(entry.path()).is_ok_and(|meta| meta.is_dir());
        let dir = entry.file_type().is_ok_and(|kind| kind.is_dir() || (kind.is_symlink() && linked(&entry)));
        found.push(format!("{folder}{name}{}", if dir { "/" } else { "" }));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of this test's own, empty.
    fn temp(name: &str) -> crate::scratch::Scratch {
        crate::scratch::Scratch::dir(&format!("ottospace-tags-{name}-{}", std::process::id()))
    }

    fn words(line: &str) -> Vec<Vec<String>> {
        compose::path_words(line)
    }

    #[test]
    fn a_path_is_in_the_home_folder_the_repository_or_from_the_root() {
        let dir = temp("paths");
        let (repo, home) = (dir.join("app"), dir.join("home"));
        std::fs::create_dir_all(repo.join("src/export")).unwrap();
        std::fs::create_dir_all(repo.join("My Notes")).unwrap();
        std::fs::create_dir_all(home.join("notes")).unwrap();
        std::fs::write(repo.join("src/extra.rs"), "").unwrap();
        std::fs::write(repo.join("src/.hidden"), "").unwrap();
        std::fs::write(repo.join("src/v1."), "").unwrap();
        std::os::unix::fs::symlink(repo.join("src"), home.join("linked")).unwrap();
        let roots = Roots { home: Some(home.clone()), inside: None };
        let inside = Some(&*repo);
        let at = |part: &str| repo.join(part).to_string_lossy().into_owned();
        let found = |typed: &str, resolved: String| PathFound { typed: typed.into(), resolved };

        // `./` is the repository's, `~/` the home folder's, `/` the root's.
        let read = paths(&words("see ./src/export, ~/notes, ./nowhere and /tmp"), inside, &roots);
        let home_notes = home.join("notes").to_string_lossy().into_owned();
        assert_eq!(
            read,
            [found("./src/export", at("src/export")), found("~/notes", home_notes), found("/tmp", "/tmp".into())]
        );
        // `/src/export` is no longer looked for inside the repository; `./` without one is nothing.
        assert_eq!(paths(&words("see /src/export and ./src"), None, &roots), []);
        assert_eq!(paths(&words("see ../app/src"), inside, &roots), [found("../app/src", at("src"))]);
        // An absolute path counts by itself.
        let absolute = format!("look in {}", at("src"));
        assert_eq!(paths(&words(&absolute), None, &roots), [found(&at("src"), at("src"))]);
        // The word as typed first: a name may end in a dot. A space is written with a backslash.
        let read = paths(&words(r"read ./src/v1. and ./src/extra.rs. in ./My\ Notes"), inside, &roots);
        let typed: Vec<&str> = read.iter().map(|path| path.typed.as_str()).collect();
        assert_eq!(typed, ["./src/v1.", "./src/extra.rs", "./My Notes"]);

        // A folder is listed whole, a directory (or a link to one) with a `/`; the core narrows it.
        let mut listed = entries("./src/", inside, &roots);
        listed.sort();
        assert_eq!(listed, ["./src/.hidden", "./src/export/", "./src/extra.rs", "./src/v1."]);
        let mut listed = entries("~/", inside, &roots);
        listed.sort();
        assert_eq!(listed, ["~/linked/", "~/notes/"]);
        assert_eq!(
            entries("./nowhere/", inside, &roots),
            Vec::<String>::new(),
            "a folder that is not there lists nothing"
        );
        // No home folder: `~` lists nothing. Kept inside a folder: nothing outside it is looked at.
        let homeless = Roots { home: None, inside: None };
        assert_eq!((entries("~/", None, &homeless), paths(&words("~/notes"), None, &homeless)), (vec![], vec![]));
        let kept = Roots { home: Some(home.clone()), inside: Some(vec![dir.to_path_buf()]) };
        assert_eq!((entries("/", None, &kept), paths(&words("see /tmp"), None, &kept)), (vec![], vec![]));
        assert_eq!(entries("../../", inside, &kept), Vec::<String>::new(), "`..` leads out of it by its text");
        assert_eq!(entries("~/", None, &kept).len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
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
