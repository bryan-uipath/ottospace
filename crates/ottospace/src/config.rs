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
    /// What was wrong with `settings.json`, to show the user. The defaults apply.
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
        Some(Ok(text)) => match Settings::parse(&text) {
            Ok(settings) => (settings, None),
            Err(err) => (Settings::default(), Some(format!("settings.json: {err}"))),
        },
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

/// The tasks kept on disk, and how many files could not be read as a task.
pub fn load_tasks() -> (Vec<Task>, usize) {
    let Some(dir) = tasks_dir() else { return (Vec::new(), 0) };
    let files = fs::read_dir(dir).into_iter().flatten().flatten().map(|entry| entry.path());
    let files = files.filter(|path| path.extension().is_some_and(|ext| ext == "json"));
    // A small regular file only: not a pipe, which would hang the read.
    let read = |path: &PathBuf| {
        let meta = fs::metadata(path).ok()?;
        (meta.is_file() && meta.len() < 65_536).then(|| fs::read_to_string(path).ok()).flatten()
    };
    let (mut tasks, mut unread) = (Vec::new(), 0);
    for path in files {
        match read(&path).and_then(|text| serde_json::from_str::<Task>(&text).ok()) {
            Some(task) => tasks.push(task),
            None => unread += 1,
        }
    }
    (tasks, unread)
}

/// Writes one task to `<id>.json` in the tasks folder. Whole or not at all: it writes beside the
/// file, then renames over it.
pub fn save_task(task: &Task) -> io::Result<()> {
    let Some(dir) = tasks_dir() else { return Ok(()) };
    if !dir.is_dir() {
        use std::os::unix::fs::DirBuilderExt;
        // Task titles can be private: the folder is the user's alone.
        fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
    }
    let (file, draft) = (dir.join(format!("{}.json", task.id)), dir.join(format!(".{}.json.tmp", task.id)));
    fs::write(&draft, serde_json::to_string_pretty(task).map_err(io::Error::other)? + "\n")?;
    fs::rename(draft, file)
}
