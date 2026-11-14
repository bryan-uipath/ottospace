//! The user's folder: `~/.ottospace`, or `$OTTOSPACE_HOME`. It holds `settings.json` and the
//! tasks kept between runs. Read once, at start: a changed setting applies at the next start.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::{fs, io};

use ottospace_core::repos::{RepoDir, RepoDirs};
use ottospace_core::settings::{DEFAULT_REPOS, ListChange, Settings, expand_home, with_changes};
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

pub fn user_home() -> String {
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
/// the settings' `repos` as the Repos section last changed them, else `~/code` and `~/notes`.
/// Each as named and resolved (links, `..`) now: a link may point elsewhere since the start.
pub fn repo_dirs() -> RepoDirs {
    let resolved = |text: String, path: PathBuf| {
        let path = path.canonicalize().unwrap_or(path).to_string_lossy().into_owned();
        RepoDir { text, path }
    };
    if let Some(list) = std::env::var_os("OTTOSPACE_REPOS") {
        let named = std::env::split_paths(&list).filter(|p| !p.as_os_str().is_empty());
        return RepoDirs { dirs: named.map(|p| resolved(p.to_string_lossy().into_owned(), p)).collect(), fixed: true };
    }
    let changed = NAMED.lock().ok().and_then(|named| named.clone());
    let settings = || get().settings.repos.clone().unwrap_or_else(|| DEFAULT_REPOS.map(String::from).to_vec());
    let dirs = changed.unwrap_or_else(settings).into_iter().map(|text| {
        let path = PathBuf::from(expand_home(&text, &user_home()));
        resolved(text, path)
    });
    RepoDirs { dirs: dirs.collect(), fixed: false }
}

/// The settings' `repos` as the Repos section last changed them, by name.
static NAMED: Mutex<Option<Vec<String>>> = Mutex::new(None);

/// Where repositories are looked for now, on any thread.
pub fn repo_roots() -> Vec<PathBuf> {
    repo_dirs().dirs.into_iter().map(|dir| PathBuf::from(dir.path)).collect()
}

/// The model's directories changed: later reads of the list look there.
pub fn set_repo_roots(dirs: &RepoDirs) {
    if let Ok(mut named) = NAMED.lock() {
        *named = Some(dirs.dirs.iter().map(|dir| dir.text.clone()).collect());
    }
}

/// Where the runners' sessions are read from.
pub struct SessionDirs {
    pub claude: Vec<PathBuf>,
    pub codex: Option<PathBuf>,
}

/// Where sessions are read from: `session_dirs` in the settings, else `~/.claude` and `~/.codex`
/// (an app started from the Dock has no `CLAUDE_CONFIG_DIR` to go by). `None`: nothing is read,
/// because `"sessions": false` says so, or because this is a test run whose settings name no
/// directory: a test never reads the user's own sessions.
pub fn session_dirs() -> Option<SessionDirs> {
    let settings = &get().settings;
    let test_run = std::env::var_os("OTTOSPACE_E2E").is_some();
    if settings.sessions == Some(false) || (test_run && settings.session_dirs.is_none()) {
        return None;
    }
    let named = settings.session_dirs.clone().unwrap_or_default();
    let path = |dir: &String| PathBuf::from(expand_home(dir, &user_home()));
    // In a test run a runner the settings leave out has no directory, not the user's.
    let default = |dir: &str| (!test_run).then(|| dir.to_string());
    let claude = named.claude.unwrap_or_else(|| default("~/.claude").into_iter().collect());
    let codex = named.codex.or_else(|| default("~/.codex"));
    Some(SessionDirs { claude: claude.iter().map(path).collect(), codex: codex.as_ref().map(path) })
}

/// How long an adopted session is quiet before its task needs the user, in milliseconds:
/// `session_quiet_minutes` in the settings, else 3 minutes. 0: never.
pub fn session_quiet_ms() -> u64 {
    let minutes = get().settings.session_quiet_minutes.map(u64::from);
    minutes.map_or(ottospace_core::sessions::QUIET_MS, |minutes| minutes.saturating_mul(60_000))
}

/// How many of a terminal's last lines a workbook keeps: `workbook_scrollback_lines` in the
/// settings, else 500, at most 2000. 0: none.
pub fn workbook_lines() -> usize {
    let lines = get().settings.workbook_scrollback_lines.map(|lines| lines as usize);
    lines.unwrap_or(ottospace_core::workbook::HISTORY_LINES).min(ottospace_core::workbook::MAX_HISTORY_LINES)
}

/// Where tasks are kept: `tasks` in the settings (a relative path is inside the user's folder),
/// else `tasks` in the user's folder. `None`: nothing is kept.
pub fn tasks_dir() -> Option<PathBuf> {
    let home = get().home.as_ref()?;
    let dir = get().settings.tasks.as_deref().map_or_else(|| "tasks".to_string(), |dir| expand_home(dir, &user_home()));
    Some(home.join(dir)) // `join` keeps an absolute path as it is
}

/// Where captures are kept: `materials` in the user's folder, whatever `tasks` says. Not in the
/// tasks folder, which may be a notes repository that is shared: a picture of a window is not
/// for there. `None`: nothing is kept.
pub fn materials_dir() -> Option<PathBuf> {
    Some(get().home.as_ref()?.join("materials"))
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

/// Writes `text` to `name` in `dir`, whole or not at all: beside the file, synced, then renamed
/// over it. The folder and the file are the user's alone: a title or a directory can be private.
/// A file that is there keeps its mode.
fn write_whole(dir: &Path, name: &str, text: &str) -> io::Result<()> {
    write_private(dir, name, text.as_bytes())
}

/// As `write_whole`, for any bytes: a capture's picture.
pub fn write_private(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    write_mode(dir, name, bytes, None)
}

/// As `write_private`; `mode`: the file's, else the one there keeps its own.
fn write_mode(dir: &Path, name: &str, bytes: &[u8], mode: Option<u32>) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    if !dir.is_dir() {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let kept = || fs::metadata(dir.join(name)).map(|meta| meta.permissions().mode() & 0o777).unwrap_or(0o600);
    let mode = mode.unwrap_or_else(kept);
    // The process id keeps two running ottospaces from writing one draft.
    let draft = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let write = || {
        let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&draft)?;
        file.write_all(bytes)?;
        file.set_permissions(fs::Permissions::from_mode(mode))?;
        // On disk before it takes the old file's place: a crash leaves one or the other, whole.
        file.sync_all()?;
        fs::rename(&draft, dir.join(name))
    };
    let moved = write();
    if moved.is_err() {
        let _ = fs::remove_file(&draft);
    }
    moved
}

/// Removes drafts of `settings.json` an ottospace that crashed left in the user's folder
/// (`.settings.json.<pid>.tmp`), once they are a minute old: not one being written now.
pub fn remove_stale_drafts() {
    let Some(home) = get().home.as_ref() else { return };
    let old = |path: &PathBuf| {
        let age = fs::metadata(path).and_then(|meta| meta.modified()).ok().and_then(|at| at.elapsed().ok());
        age.is_some_and(|age| age.as_secs() >= 60)
    };
    let draft = |path: &PathBuf| {
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
        name.starts_with(".settings.json.") && name.ends_with(".tmp")
    };
    let entries = fs::read_dir(home).into_iter().flatten().flatten().map(|entry| entry.path());
    for path in entries.filter(|path| draft(path) && old(path)) {
        let _ = fs::remove_file(path);
    }
}

/// Writes one task to `<id>.json` in the tasks folder.
pub fn save_task(task: &Task) -> io::Result<()> {
    let Some(dir) = tasks_dir() else { return Ok(()) };
    // Less of a reference than the session knows: the folder may be shared.
    let text = serde_json::to_string_pretty(&task.for_file()).map_err(io::Error::other)? + "\n";
    write_whole(&dir, &format!("{}.json", task.id), &text)
}

/// Makes `changes` to `apps`, `apps_hidden` and `repos` in `settings.json`, as the file is now: read
/// again here, so an entry written since the start (by hand, by another ottospace) stays.
/// Nothing is written in a test run that named no folder, or if the file would not change.
/// `Err`: the file could not be read as settings, a list in it is not a list of names, or it
/// is a link to nowhere. It is left as it is.
pub fn save_apps(changes: &[ListChange]) -> Result<(), String> {
    save_settings(|text| with_changes(text, changes))
}

/// Sets one key of `settings.json` to `value`, as `save_apps` changes its lists.
pub fn save_value(key: &str, value: &serde_json::Value) -> Result<(), String> {
    save_settings(|text| ottospace_core::settings::with_value(text, key, value))
}

/// `settings.json` made over by `edit`, from the file as it is now.
fn save_settings(edit: impl FnOnce(&str) -> Result<String, String>) -> Result<(), String> {
    let Some(home) = get().home.as_ref() else { return Ok(()) };
    let file = home.join("settings.json");
    // Through a link to the file it names: a settings file kept elsewhere stays linked. A link
    // to nowhere is not replaced by a file: where it points is the user's to mend.
    let linked = fs::symlink_metadata(&file).is_ok_and(|meta| meta.file_type().is_symlink());
    let file = match fs::canonicalize(&file) {
        Ok(target) => target,
        Err(_) if linked => return Err("it is a link to a file that is not there".into()),
        Err(_) => file,
    };
    let text = match fs::read_to_string(&file) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err.to_string()),
    };
    let new = edit(&text)?;
    let (Some(dir), Some(name)) = (file.parent(), file.file_name().and_then(|name| name.to_str())) else {
        return Err("no place for the file".into());
    };
    if new == text { Ok(()) } else { write_whole(dir, name, &new).map_err(|err| err.to_string()) }
}

/// True if marks are kept between runs: not in a test run that named no folder.
pub fn keeps_marks() -> bool {
    get().home.is_some()
}

/// The text of `marks.json` in the user's folder. `None`: no file, or nothing is kept. The
/// error names a file that could not be read; it must then be left as it is.
pub fn read_marks() -> Result<Option<String>, String> {
    let Some(file) = get().home.as_ref().map(|home| home.join("marks.json")) else { return Ok(None) };
    match fs::metadata(&file) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.to_string()),
        // A small regular file only: not a pipe, which would hang the read.
        Ok(meta) if !meta.is_file() || meta.len() > 1_048_576 => Err("not a file of marks".into()),
        Ok(_) => fs::read_to_string(&file).map(Some).map_err(|err| err.to_string()),
    }
}

/// Writes `marks.json`, as `save_task` writes a task.
pub fn save_marks(text: &str) -> io::Result<()> {
    let Some(home) = get().home.as_ref() else { return Ok(()) };
    write_whole(home, "marks.json", text)
}

/// The most workbook files read, and the largest one read: a folder is input, not trusted text.
const MAX_WORKBOOKS: usize = 512;
const MAX_WORKBOOK_BYTES: u64 = ottospace_core::workbook::MAX_FILE_BYTES as u64;

/// Where workbooks are kept: `workbooks` in the user's folder. `None`: nothing is kept.
fn workbooks_dir() -> Option<PathBuf> {
    Some(get().home.as_ref()?.join("workbooks"))
}

/// A workbook's file name. `None` unless `uid` is 16 hex digits: no other id becomes a path.
fn workbook_file(uid: &str) -> Option<String> {
    ottospace_core::workbook::valid_uid(uid).then(|| format!("{uid}.json"))
}

/// What the workbooks folder held at start.
#[derive(Default)]
pub struct KeptWorkbooks {
    /// Each `<id>.json`: the id in its name, and its text.
    pub files: Vec<(String, String)>,
    /// Files named `*.json` that are not that: another name, a link, a pipe, too large.
    pub unread: usize,
    pub error: Option<String>,
}

/// Reads the workbooks folder. Only a small regular file named for an id: not a link or a pipe.
pub fn load_workbooks() -> KeptWorkbooks {
    let Some(dir) = workbooks_dir() else { return KeptWorkbooks::default() };
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return KeptWorkbooks::default(),
        Err(err) => return KeptWorkbooks { error: Some(format!("workbooks: {err}")), ..KeptWorkbooks::default() },
    };
    let json = |path: &PathBuf| path.extension().is_some_and(|ext| ext == "json");
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).filter(json).collect();
    paths.sort();
    let mut kept = KeptWorkbooks::default();
    for path in paths {
        let uid = path.file_stem().and_then(|stem| stem.to_str()).filter(|stem| workbook_file(stem).is_some());
        let read = || String::from_utf8(crate::collect::read_plain(&path, MAX_WORKBOOK_BYTES)?).ok();
        let text = uid.filter(|_| kept.files.len() < MAX_WORKBOOKS).and_then(|_| read());
        match (uid, text) {
            (Some(uid), Some(text)) => kept.files.push((uid.to_string(), text)),
            _ => kept.unread += 1,
        }
    }
    kept
}

/// Writes one workbook to `<uid>.json` in the workbooks folder, as `save_task` writes a task.
pub fn save_workbook(uid: &str, text: &str) -> io::Result<()> {
    let Some(dir) = workbooks_dir() else { return Ok(()) };
    let name = workbook_file(uid).ok_or_else(|| io::Error::other("not a workbook's id"))?;
    write_whole(&dir, &name, text)
}

/// Removes a workbook's file. One that is gone already is removed.
pub fn remove_workbook(uid: &str) -> io::Result<()> {
    let Some(dir) = workbooks_dir() else { return Ok(()) };
    let name = workbook_file(uid).ok_or_else(|| io::Error::other("not a workbook's id"))?;
    removed(fs::remove_file(dir.join(name)))
}

/// A removal or a move of a file that is not there succeeded: it is gone.
pub fn removed(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

/// `layout.json` in the user's folder: the tabs and layout kept between runs. `None`: nothing
/// is kept (a test run that named no folder), or `restore_layout` is off.
fn layout_file() -> Option<PathBuf> {
    get().settings.restore_layout.unwrap_or(true).then(layout_path).flatten()
}

/// Where `layout.json` is, whether it is kept or not.
fn layout_path() -> Option<PathBuf> {
    Some(get().home.as_ref()?.join("layout.json"))
}

/// True if the layout is kept between runs.
pub fn keeps_layout() -> bool {
    layout_file().is_some()
}

/// True if an app a restored pane names may be started: `restore_apps`, default off.
pub fn restores_apps() -> bool {
    get().settings.restore_apps.unwrap_or(false)
}

/// The text of `layout.json`. `None`: no file, or nothing is kept. The error: it is there and
/// is not a small plain file (a link, a pipe, too large) or not text.
pub fn read_layout() -> Result<Option<String>, String> {
    let Some(file) = layout_file() else { return Ok(None) };
    match fs::symlink_metadata(&file) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.to_string()),
        Ok(_) => {}
    }
    let most = ottospace_core::layout_file::MAX_FILE_BYTES as u64;
    let bytes = crate::collect::read_plain(&file, most).ok_or(format!("not a plain file of at most {most} bytes"))?;
    String::from_utf8(bytes).map(Some).map_err(|_| "not text".into())
}

/// Writes `layout.json`, as `save_task` writes a task: whole and renamed into place, but always
/// 0600: it may hold terminal text.
pub fn save_layout(text: &str) -> io::Result<()> {
    let Some(file) = layout_file() else { return Ok(()) };
    let dir = file.parent().ok_or_else(|| io::Error::other("no folder"))?;
    write_mode(dir, "layout.json", text.as_bytes(), Some(0o600))
}

/// Puts `layout.json` aside as `layout.json.<end>` (`bad`; an older one is replaced), or with
/// `None` removes it and the copy set aside, whatever `restore_layout` says: both may hold
/// terminal text.
pub fn layout_aside(end: Option<&str>) -> io::Result<()> {
    let Some(file) = layout_path() else { return Ok(()) };
    let bad = file.with_extension("json.bad");
    match end {
        Some(end) => removed(fs::rename(&file, file.with_extension(format!("json.{end}")))),
        None => removed(fs::remove_file(&file)).and(removed(fs::remove_file(bad))),
    }
}
