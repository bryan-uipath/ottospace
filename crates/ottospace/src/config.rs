//! The user's folder: `~/.ottospace`, or `$OTTOSPACE_HOME`. It holds `settings.json` and the
//! tasks kept between runs. Read once, at start: a changed setting applies at the next start.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::{fs, io};

use ottospace_core::settings::{Settings, expand_home};
use ottospace_core::tasks::Task;

pub struct Config {
    pub settings: Settings,
    /// `None`: a test run that named no folder. It reads no settings and keeps nothing on disk,
    /// so a test cannot read or change the user's own.
    pub home: Option<PathBuf>,
    /// What was wrong with `settings.json`, to show the user. A wrong key takes its default;
    /// the others apply.
    pub error: Option<String>,
}

pub fn get() -> &'static Config {
    static CONFIG: OnceLock<Config> = OnceLock::new();
    CONFIG.get_or_init(load)
}

fn user_home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

fn load() -> Config {
    let named = std::env::var_os("OTTOSPACE_HOME").filter(|dir| !dir.is_empty()).map(PathBuf::from);
    let test_run = std::env::var_os("OTTOSPACE_E2E").is_some();
    let home = named.or_else(|| (!test_run).then(|| Path::new(&user_home()).join(".ottospace")));
    let file = home.as_ref().map(|home| home.join("settings.json"));
    let (settings, error) = match file.as_ref().map(fs::read_to_string) {
        // Every key that is right is kept; what is wrong is named.
        Some(Ok(text)) => {
            let (settings, wrong) = Settings::read(&text);
            (settings, (!wrong.is_empty()).then(|| format!("settings.json: {}", wrong.join("; "))))
        }
        // No file is the usual case: the defaults.
        Some(Err(err)) if err.kind() != io::ErrorKind::NotFound => {
            (Settings::default(), Some(format!("settings.json: {err}")))
        }
        _ => (Settings::default(), None),
    };
    Config { settings, home, error }
}

/// Where repositories are looked for: `$OTTOSPACE_REPOS` (directories, `:` between them), else
/// `repos` in the settings, else `~/code` and `~/notes`.
pub fn repo_roots() -> Vec<PathBuf> {
    if let Some(list) = std::env::var_os("OTTOSPACE_REPOS") {
        return std::env::split_paths(&list).filter(|p| !p.as_os_str().is_empty()).collect();
    }
    let listed = get().settings.repos.clone().unwrap_or_else(|| vec!["~/code".into(), "~/notes".into()]);
    listed.iter().map(|dir| PathBuf::from(expand_home(dir, &user_home()))).collect()
}

/// Where tasks are kept: `tasks` in the settings (a relative path is inside the user's folder),
/// else `tasks` in the user's folder. `None`: nothing is kept.
pub fn tasks_dir() -> Option<PathBuf> {
    let home = get().home.as_ref()?;
    let dir = get().settings.tasks.as_deref().map_or_else(|| "tasks".to_string(), |dir| expand_home(dir, &user_home()));
    Some(home.join(dir)) // `join` keeps an absolute path as it is
}

/// What the tasks folder held at start.
#[derive(Default)]
pub struct Kept {
    pub tasks: Vec<Task>,
    /// Files named `*.json` that are not a task of that number.
    pub unread: usize,
    /// The highest number among `<number>.json` files, read or not: new tasks start above it,
    /// so a file that could not be read is never overwritten.
    pub highest: u32,
    /// The folder could not be listed. Nothing must be written into it: what it holds is unknown.
    pub error: Option<String>,
}

/// Reads the tasks folder. A file counts as a task only if it is `<id>.json` for the id inside:
/// a copy (`5 copy.json`) must not stand in for the task it was copied from.
pub fn load_tasks() -> Kept {
    let Some(dir) = tasks_dir() else { return Kept::default() };
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Kept::default(), // nothing kept yet
        Err(err) => return Kept { error: Some(format!("tasks: {}: {err}", dir.display())), ..Kept::default() },
    };
    let files = entries.flatten().map(|entry| entry.path());
    let files = files.filter(|path| path.extension().is_some_and(|ext| ext == "json"));
    // A small regular file only: not a pipe, which would hang the read.
    let read = |path: &PathBuf| {
        let meta = fs::metadata(path).ok()?;
        (meta.is_file() && meta.len() < 65_536).then(|| fs::read_to_string(path).ok()).flatten()
    };
    let mut kept = Kept::default();
    for path in files {
        let number = path.file_stem().and_then(|stem| stem.to_str()).and_then(|stem| stem.parse::<u32>().ok());
        kept.highest = kept.highest.max(number.unwrap_or(0));
        let task = read(&path).and_then(|text| serde_json::from_str::<Task>(&text).ok());
        match task.filter(|task| Some(task.id) == number) {
            Some(task) => kept.tasks.push(task),
            None => kept.unread += 1,
        }
    }
    kept
}

/// Writes one task to `<id>.json` in the tasks folder. Whole or not at all: it writes beside the
/// file, then renames over it. The file is the user's alone: a title can be private.
pub fn save_task(task: &Task) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let Some(dir) = tasks_dir() else { return Ok(()) };
    if !dir.is_dir() {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
    }
    // The process id keeps two running ottospaces from writing one draft.
    let draft = dir.join(format!(".{}.json.{}.tmp", task.id, std::process::id()));
    let text = serde_json::to_string_pretty(task).map_err(io::Error::other)? + "\n";
    let written = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&draft);
    let written = written.and_then(|mut file| file.write_all(text.as_bytes()));
    let moved = written.and_then(|()| fs::rename(&draft, dir.join(format!("{}.json", task.id))));
    if moved.is_err() {
        let _ = fs::remove_file(&draft);
    }
    moved
}
