//! What `settings.json` may say, and what applies when it says nothing. The shell reads the file.

use serde::Deserialize;

/// `~/.ottospace/settings.json`. Every key is optional. An unknown key is an error, so a typing
/// mistake does not pass for a setting that silently does nothing.
///
/// ```json
/// { "terminal": { "font": "JetBrains Mono", "font_size": 14 },
///   "repos": ["~/code", "~/work/clients"],
///   "tasks": "~/notes/tasks",
///   "sessions": true,
///   "session_dirs": { "claude": ["~/.claude"], "codex": "~/.codex" },
///   "session_quiet_minutes": 3,
///   "agent_models": { "claude": ["claude-opus-5-5"], "codex": ["gpt-6-astra"] },
///   "capture_key": "ctrl-alt-cmd-space",
///   "capture": { "keep_images": true, "ax": true },
///   "workbook_scrollback_lines": 500,
///   "restore_layout": true,
///   "restore_apps": false }
/// ```
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default)]
    pub terminal: Terminal,
    /// Directories that are repositories or hold them. Default: `~/code` and `~/notes`.
    #[serde(default)]
    pub repos: Option<Vec<String>>,
    /// The directory tasks are kept in. Default: `tasks` beside the settings file.
    #[serde(default)]
    pub tasks: Option<String>,
    /// Windows to leave alone: `[{ "app": "us.zoom.xos", "title": "zoom share*", "do": "ignore" }]`.
    /// See `rules`. They come before the rules that ship with ottospace.
    #[serde(default)]
    pub windows: Vec<crate::rules::Rule>,
    /// The pinned apps: listed first when they are not running, by name (`"Slack"`) or by the
    /// path of the bundle. Choosing one starts it. A pin in the Apps section writes this list.
    #[serde(default)]
    pub apps: Vec<String>,
    /// Apps of the Dock not to suggest, written as `apps` is.
    #[serde(default)]
    pub apps_hidden: Vec<String>,
    /// Suggest the Dock's apps that are not running. Default: true.
    #[serde(default)]
    pub suggest_dock_apps: Option<bool>,
    /// The wheel or trackpad over an app pane you are not in scrolls the app, which stays where it
    /// is (`app_wheel`). Default: true. False: the wheel there does nothing.
    #[serde(default)]
    pub wheel_to_apps: Option<bool>,
    /// List the agent sessions (Claude Code, Codex) of the listed repositories in the Tasks
    /// section. Default: true. False: no session file is ever read.
    #[serde(default)]
    pub sessions: Option<bool>,
    /// Where the runners keep their sessions. Default: `~/.claude` and `~/.codex`.
    #[serde(default)]
    pub session_dirs: Option<SessionDirs>,
    /// A task adopted from a session needs you once the session has been quiet this many
    /// minutes. Default: 3. 0: a quiet session never moves its task.
    #[serde(default)]
    pub session_quiet_minutes: Option<u32>,
    /// The models `@` offers after each runner, in place of the built-in list: ids as the
    /// runner's CLI takes them. `@claude-opus-5-5`, `@codex-gpt-6-astra`.
    #[serde(default)]
    pub agent_models: Option<AgentModels>,
    /// The agent a task with no `@agent` starts with: a name `@` takes. Default: `claude`.
    #[serde(default)]
    pub default_agent: Option<String>,
    /// The key that captures the pane in use, from any app: modifiers and a key with `-`
    /// between them. Default: `ctrl-alt-cmd-space`. `"off"`: no key.
    #[serde(default)]
    pub capture_key: Option<String>,
    #[serde(default)]
    pub capture: Capture,
    /// How many of a terminal's last lines a workbook keeps when it is saved, shown again above
    /// its prompt when it opens. Default: 500; at most 2000. 0: none is kept.
    #[serde(default)]
    pub workbook_scrollback_lines: Option<u32>,
    /// Tabs and layout come back after quitting (`layout.json`). Default: true. False: each start
    /// is empty, and the file is neither read nor written.
    #[serde(default)]
    pub restore_layout: Option<bool>,
    /// At a restore, start an app a pane names that is not running. Default: false: the pane
    /// waits and offers "Open <App>".
    #[serde(default)]
    pub restore_apps: Option<bool>,
    /// How the Tasks section groups its tasks: `state` (default) or `project`. Written when the
    /// user picks one in the section's header.
    #[serde(default)]
    pub tasks_view: Option<crate::model::TasksView>,
    /// Which sessions Other sessions lists: `today` (default), `week`, `month` or `all`. Written
    /// when the user picks one on its heading.
    #[serde(default)]
    pub sessions_range: Option<crate::sessions::Range>,
    /// The calendar feed of the meeting timeline: an `https://` link (a private feed's link holds
    /// a secret: it is never shown) or a file's path. None: no timeline.
    #[serde(default)]
    pub calendar_ics: Option<String>,
    /// How many hours the hairline shows from now. Default: 8; 1 to 24.
    #[serde(default)]
    pub calendar_window_hours: Option<u32>,
    /// How often the feed is read again. Default: 5; at least 1.
    #[serde(default)]
    pub calendar_refresh_minutes: Option<u32>,
    /// A meeting that starts with no Zoom meeting open alerts. Default: true.
    #[serde(default)]
    pub calendar_notify: Option<bool>,
    /// Which meetings alert: `all` (default) or only those with a Zoom, Meet or Teams `links`.
    #[serde(default)]
    pub calendar_notify_for: Option<NotifyFor>,
}

/// Which meetings alert when they start (`calendar_notify_for`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotifyFor {
    #[default]
    All,
    Links,
}

/// The default capture key. Not ⌃⌥Space: macOS gives that to "Select next source in Input menu".
pub const CAPTURE_KEY: &str = "ctrl-alt-cmd-space";

/// What a capture keeps besides what the user drew and typed.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    /// Keep the pictures on disk. False: none is ever written.
    #[serde(default = "yes")]
    pub keep_images: bool,
    /// Read an app pane's accessibility tree with its picture. False: it is never read for a capture.
    #[serde(default = "yes")]
    pub ax: bool,
}

impl Default for Capture {
    fn default() -> Self {
        Self { keep_images: true, ax: true }
    }
}

fn yes() -> bool {
    true
}

/// A runner left out keeps its default.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionDirs {
    /// Claude Code's config directories: one each for several accounts.
    #[serde(default)]
    pub claude: Option<Vec<String>>,
    /// Codex's home.
    #[serde(default)]
    pub codex: Option<String>,
}

/// A runner left out keeps its built-in models. An empty list: the runner alone.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentModels {
    #[serde(default)]
    pub claude: Option<Vec<String>>,
    #[serde(default)]
    pub codex: Option<Vec<String>>,
}

impl AgentModels {
    /// The first id that is not a model's name.
    fn wrong(&self) -> Option<&String> {
        self.claude.iter().chain(&self.codex).flatten().find(|id| !crate::agents::is_name(id))
    }
}

/// What is said of an `agent_models` entry that is not a name.
fn not_a_model(id: &str) -> String {
    format!("{id} is not a model name: letters, digits, `.`, `-` and `_`, at most {}", crate::agents::MAX_MODEL)
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Terminal {
    #[serde(default = "default_font")]
    pub font: String,
    /// In points. Kept between 6 and 72.
    #[serde(default = "default_font_size")]
    pub font_size: f32,
}

impl Default for Terminal {
    fn default() -> Self {
        Self { font: default_font(), font_size: default_font_size() }
    }
}

fn default_font() -> String {
    "Menlo".into()
}

fn default_font_size() -> f32 {
    13.
}

impl Terminal {
    /// The height of a terminal row for this font size: 13 → 18, as before there were settings.
    pub fn line_height(&self) -> f32 {
        (self.font_size * 18. / 13.).round()
    }
}

impl Settings {
    /// Reads the file's text. An empty file is the defaults. The error names the place, e.g.
    /// `unknown field "font_family" at line 1 column 28`.
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        let mut settings: Self = serde_json::from_str(text).map_err(|err| err.to_string())?;
        // NaN compares false both ways, so `clamp` would keep it.
        let size = settings.terminal.font_size;
        settings.terminal.font_size = if size.is_finite() { size.clamp(6., 72.) } else { default_font_size() };
        if settings.terminal.font.trim().is_empty() {
            settings.terminal.font = default_font();
        }
        // Task files must have a folder of their own: beside other files they are read as tasks.
        if settings.tasks.as_deref().is_some_and(|dir| matches!(dir.trim().trim_end_matches('/'), "" | "~" | ".")) {
            return Err("\"tasks\" must name a folder of its own, e.g. \"~/notes/tasks\"".into());
        }
        // Only a runner, or a runner and a model, in small letters: a saved agent starts nothing
        // yet. Whatever `agent_models` lists: each key is read by itself (`read`).
        if let Some(name) = settings.default_agent.as_deref() {
            let built_in = crate::runner::RunnerId::named(name).is_some() || crate::agents::typed(name).is_some();
            if !built_in {
                return Err(format!("{name:?} is not `claude`, `codex` or one with a model, as `codex-gpt-6-astra`"));
            }
        }
        if let Some(id) = settings.agent_models.as_ref().and_then(AgentModels::wrong) {
            return Err(not_a_model(&format!("{id:?}")));
        }
        // Never names the link: it may hold a secret.
        if let Some(feed) = settings.calendar_ics.as_deref() {
            let path = feed.starts_with('/') || feed.starts_with("~/");
            let refused = if path { None } else { crate::calendar::feed_refusal(feed) };
            if let Some(why) = refused {
                return Err(why.into());
            }
        }
        if settings.calendar_window_hours.is_some_and(|h| !(1..=24).contains(&h)) {
            return Err("calendar_window_hours must be 1 to 24".into());
        }
        if settings.calendar_refresh_minutes == Some(0) {
            return Err("calendar_refresh_minutes must be at least 1".into());
        }
        Ok(settings)
    }

    /// Reads the file's text, keeping every key that is right. One wrong key must not undo the
    /// others: with a tasks folder of one's own, falling back to every default would show an
    /// empty inbox. Returns what is wrong, one line a key, each starting with the key's name.
    /// A file that is not a JSON object gives the defaults and one line. Of a key written
    /// twice, the last one counts.
    pub fn read(text: &str) -> (Self, Vec<String>) {
        let text = text.trim_start_matches('\u{feff}'); // some editors start a file with this mark
        if text.trim().is_empty() {
            return (Self::default(), Vec::new());
        }
        let mut keys = match serde_json::from_str::<serde_json::Value>(text) {
            Ok(serde_json::Value::Object(keys)) => keys,
            Ok(_) => return (Self::default(), vec!["the file must be a JSON object: { ... }".into()]),
            Err(err) => return (Self::default(), vec![format!("not valid JSON: {err}")]),
        };
        // Each key by itself, through the same checks as a whole file. The position in the
        // message is in the one-key copy, not the user's file: leave it out.
        let mut wrong = Vec::new();
        // A model that is not a name goes by itself: the runner's other models stay.
        let runners = keys.get_mut("agent_models").and_then(serde_json::Value::as_object_mut);
        for list in runners.into_iter().flat_map(|runners| runners.values_mut()).filter_map(|list| list.as_array_mut())
        {
            list.retain(|id| {
                let named = id.as_str().is_some_and(crate::agents::is_name);
                let said = id.as_str().map_or_else(|| id.to_string(), |id| format!("{id:?}"));
                wrong.extend((!named).then(|| format!("agent_models: {}", not_a_model(&said))));
                named
            });
        }
        keys.retain(|key, value| {
            let alone = serde_json::json!({ key.as_str(): value }).to_string();
            let Err(err) = Self::parse(&alone) else { return true };
            let message = err.rsplit_once(" at line ").map_or(err.as_str(), |(message, _)| message);
            wrong.push(format!("{key}: {message}"));
            false
        });
        // What is left is right key by key, so this reads it whole, with no list of keys to keep
        // in step with the struct.
        match Self::parse(&serde_json::Value::Object(keys).to_string()) {
            Ok(settings) => (settings, wrong),
            Err(err) => (Self::default(), vec![err]),
        }
    }
}

/// An app the settings name, found on disk.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    /// As written in the settings: `Visual Studio Code`.
    pub name: String,
    /// Where it is: `/Applications/Visual Studio Code.app`.
    pub path: String,
}

/// The apps to list as not running: those the settings name, less those whose bundle runs now,
/// each once, in the order given. An app is told by where it is installed, not by its name: the
/// name of a running process often differs (`Visual Studio Code` runs as `Code`).
pub fn closed_apps(wanted: &[Installed], running_bundles: &[String]) -> Vec<Installed> {
    let mut closed: Vec<Installed> = Vec::new();
    for app in wanted {
        if !running_bundles.contains(&app.path) && !closed.iter().any(|c| c.path == app.path) {
            closed.push(app.clone());
        }
    }
    closed
}

/// The directories of `repos` when the settings name none.
pub const DEFAULT_REPOS: [&str; 2] = ["~/code", "~/notes"];

/// One entry added to, or taken off, a list of the settings file: `apps`, `apps_hidden` or `repos`.
#[derive(Clone, Debug, PartialEq)]
pub struct ListChange {
    pub key: &'static str,
    /// False: take it off.
    pub add: bool,
    pub text: String,
}

/// The settings file's text after `changes`, for a pin made in the Apps section or a directory
/// added in Repos. A file with no `repos` has the default ones: they are written with the change. Each is made
/// to the list as the file has it NOW, which may not be the one read at start: the user may
/// have edited it since, or another ottospace. Only the lists changed are touched: every
/// other key, known or not, and the file's own spacing stay as they are. An entry is added
/// once; an empty list with no key gets none.
/// `Err`: the file is not a JSON object, or a list to change is not a list of names. It must
/// then be left alone: nothing in it is dropped for not being understood.
pub fn with_changes(text: &str, changes: &[ListChange]) -> Result<String, String> {
    let body = text.trim_start_matches('\u{feff}');
    let keys = match serde_json::from_str::<serde_json::Value>(if body.trim().is_empty() { "{}" } else { body }) {
        Ok(serde_json::Value::Object(keys)) => keys,
        Ok(_) => return Err("the file must be a JSON object: { ... }".into()),
        Err(err) => return Err(format!("not valid JSON: {err}")),
    };
    let mut new = text.to_string();
    for key in ["apps", "apps_hidden", "repos"] {
        let mine: Vec<&ListChange> = changes.iter().filter(|change| change.key == key).collect();
        if mine.is_empty() {
            continue;
        }
        let names = |list: &Vec<serde_json::Value>| list.iter().map(|item| item.as_str().map(str::to_string)).collect();
        let before: Vec<String> = match keys.get(key) {
            None if key == "repos" => DEFAULT_REPOS.map(String::from).to_vec(),
            None => Vec::new(),
            Some(value) => value
                .as_array()
                .and_then(names)
                .ok_or_else(|| format!("`{key}` is not a list of names: [\"Slack\", \"Notes\"]"))?,
        };
        let mut list = before.clone();
        for change in mine {
            list.retain(|text| *text != change.text);
            if change.add {
                list.push(change.text.clone());
            }
        }
        // An entry that was there stays where it was.
        let kept = before.iter().filter(|text| list.contains(text));
        let added = list.iter().filter(|text| !before.contains(text));
        let list: Vec<String> = kept.chain(added).cloned().collect();
        // Counted in the file as it is now: another ottospace may have added since the start.
        if key == "repos" && list.len() > before.len() && list.len() > crate::repos::MAX_DIRS {
            return Err(format!("`repos` names {} directories already", before.len()));
        }
        if list != before {
            new = with_list(&new, key, &list)?;
        }
    }
    Ok(new)
}

/// `text` with its top-level `key` set to `value`: every other key, and the file's spacing, as
/// they were. e.g. `"tasks_view": "project"`.
pub fn with_value(text: &str, key: &str, value: &serde_json::Value) -> Result<String, String> {
    let body = text.trim_start_matches('\u{feff}');
    let mut keys = match serde_json::from_str::<serde_json::Value>(if body.trim().is_empty() { "{}" } else { body }) {
        Ok(serde_json::Value::Object(keys)) => keys,
        Ok(_) => return Err("the file must be a JSON object: { ... }".into()),
        Err(err) => return Err(format!("not valid JSON: {err}")),
    };
    if keys.get(key) == Some(value) {
        return Ok(text.to_string());
    }
    keys.insert(key.to_string(), value.clone());
    let wanted = serde_json::Value::Object(keys);
    let spliced = Some(body)
        .filter(|body| !body.trim().is_empty())
        .and_then(|body| splice(body, key, &value.to_string()))
        .filter(|new| serde_json::from_str(new).ok().as_ref() == Some(&wanted));
    let bom = &text[..text.len() - body.len()];
    let fresh = || serde_json::to_string_pretty(&wanted).unwrap_or_default() + "\n";
    Ok(format!("{bom}{}", spliced.unwrap_or_else(fresh)))
}

/// As `with_changes`, with both lists given whole.
#[cfg(test)]
fn with_apps(text: &str, apps: &[String], hidden: &[String]) -> Result<String, String> {
    let with_apps = with_list(text, "apps", apps)?;
    with_list(&with_apps, "apps_hidden", hidden)
}

fn with_list(text: &str, key: &str, list: &[String]) -> Result<String, String> {
    let quoted: Vec<String> = list.iter().map(|item| serde_json::Value::from(item.as_str()).to_string()).collect();
    let value = format!("[{}]", quoted.join(", "));
    let body = text.trim_start_matches('\u{feff}');
    if body.trim().is_empty() {
        return Ok(if list.is_empty() { text.to_string() } else { format!("{{\n  \"{key}\": {value}\n}}\n") });
    }
    let mut keys = match serde_json::from_str::<serde_json::Value>(body) {
        Ok(serde_json::Value::Object(keys)) => keys,
        Ok(_) => return Err("the file must be a JSON object: { ... }".into()),
        Err(err) => return Err(format!("not valid JSON: {err}")),
    };
    if list.is_empty() && !keys.contains_key(key) {
        return Ok(text.to_string());
    }
    keys.insert(key.to_string(), serde_json::json!(list));
    let wanted = serde_json::Value::Object(keys);
    // The edit is made in the text. If that does not read back as wanted, the file is written
    // out afresh: right before pretty.
    let spliced = splice(body, key, &value).filter(|new| serde_json::from_str(new).ok().as_ref() == Some(&wanted));
    let bom = &text[..text.len() - body.len()];
    let fresh = || serde_json::to_string_pretty(&wanted).unwrap_or_default() + "\n";
    Ok(format!("{bom}{}", spliced.unwrap_or_else(fresh)))
}

/// `text` (a JSON object) with the value of its top-level `key` replaced, or the key added
/// after the last one, indented as that one is. `None`: the text is not as expected.
fn splice(text: &str, key: &str, value: &str) -> Option<String> {
    let members = members(text)?;
    // Of a key written twice, the last one counts.
    if let Some((_, _, at)) = members.iter().rev().find(|(name, ..)| name == key) {
        return Some(format!("{}{value}{}", &text[..at.start], &text[at.end..]));
    }
    // A file with Windows line ends gets them in what is added.
    let end = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let Some((_, last_key, last)) = members.last() else {
        let open = text.find('{')? + 1;
        return Some(format!("{}{end}  \"{key}\": {value}{end}{}", &text[..open], text[open..].trim_start()));
    };
    // The last key's own line start, if only spaces come before it; else all is on one line.
    let line = text[..*last_key].rfind('\n').map(|at| &text[at + 1..*last_key]).filter(|gap| gap.trim().is_empty());
    let between = line.map_or_else(|| ", ".to_string(), |indent| format!(",{end}{indent}"));
    Some(format!("{}{between}\"{key}\": {value}{}", &text[..last.end], &text[last.end..]))
}

/// The top-level members of a JSON object's text: each key, where it starts, and where its
/// value is. e.g. `{"a": [1]}` → `[("a", 1, 6..9)]`.
fn members(text: &str) -> Option<Vec<(String, usize, std::ops::Range<usize>)>> {
    let bytes = text.as_bytes();
    let space = |mut at: usize| {
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        at
    };
    // The index after the string that starts at `at`.
    let string = |at: usize| {
        let mut end = at + 1;
        while *bytes.get(end)? != b'"' {
            end += if bytes[end] == b'\\' { 2 } else { 1 };
        }
        Some(end + 1)
    };
    let mut at = space(0);
    (*bytes.get(at)? == b'{').then_some(())?;
    at = space(at + 1);
    let mut found = Vec::new();
    while *bytes.get(at)? != b'}' {
        let (key_at, key_end) = (at, string(at)?);
        let key: String = serde_json::from_str(text.get(key_at..key_end)?).ok()?;
        at = space(key_end);
        (*bytes.get(at)? == b':').then_some(())?;
        let start = space(at + 1);
        // The value ends at the comma or brace that closes it, outside any string or nesting.
        let (mut end, mut depth) = (start, 0usize);
        loop {
            match *bytes.get(end)? {
                b'"' => end = string(end)? - 1,
                b'{' | b'[' => depth += 1,
                b'}' | b',' if depth == 0 => break,
                b'}' | b']' => depth -= 1,
                _ => {}
            }
            end += 1;
        }
        let value = start..start + text[start..end].trim_end().len();
        found.push((key, key_at, value));
        at = space(end + usize::from(bytes[end] == b','));
    }
    Some(found)
}

/// A path as the user wrote it → a real one: `~/code` → `/Users/me/code`. Other paths pass.
pub fn expand_home(path: &str, home: &str) -> String {
    match path.strip_prefix('~') {
        Some("") => home.to_string(),
        Some(rest) if rest.starts_with('/') => format!("{}{rest}", home.trim_end_matches('/')),
        _ => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_set_in_the_file_as_it_is() {
        let view = serde_json::json!("project");
        let text = "{\n  // no comments in JSON\n}";
        assert!(with_value(text, "tasks_view", &view).is_err());
        let text = "{\n    \"apps\": [\"Slack\"],\n    \"x\": 1\n}\n";
        let set = with_value(text, "tasks_view", &view).unwrap();
        assert_eq!(set, "{\n    \"apps\": [\"Slack\"],\n    \"x\": 1,\n    \"tasks_view\": \"project\"\n}\n");
        assert_eq!(with_value(&set, "tasks_view", &view).unwrap(), set, "the same value: no change");
        assert_eq!(with_value("", "tasks_view", &view).unwrap(), "{\n  \"tasks_view\": \"project\"\n}\n");
        // Read back: the key is known (`x` is the one that is not).
        let (settings, wrong) = Settings::read(&set);
        assert_eq!((settings.tasks_view, wrong.len()), (Some(crate::model::TasksView::Project), 1));
        let (settings, wrong) = Settings::read("{\"sessions_range\": \"week\"}");
        assert_eq!((settings.sessions_range, wrong.len()), (Some(crate::sessions::Range::Week), 0));
    }

    #[test]
    fn nothing_written_means_the_defaults() {
        let settings = Settings::parse("").unwrap();
        assert_eq!(settings, Settings::parse("{}").unwrap());
        assert_eq!((settings.terminal.font.as_str(), settings.terminal.font_size), ("Menlo", 13.));
        assert_eq!((settings.terminal.line_height(), settings.repos, settings.tasks), (18., None, None));
    }

    #[test]
    fn a_file_sets_what_it_names_and_leaves_the_rest() {
        let text = r#"{ "terminal": { "font": "JetBrains Mono" }, "repos": ["~/code"], "tasks": "~/notes/tasks" }"#;
        let settings = Settings::parse(text).unwrap();
        assert_eq!((settings.terminal.font.as_str(), settings.terminal.font_size), ("JetBrains Mono", 13.));
        assert_eq!(
            (settings.repos, settings.tasks.as_deref()),
            (Some(vec!["~/code".to_string()]), Some("~/notes/tasks"))
        );
        assert_eq!((settings.sessions, settings.session_dirs), (None, None));
    }

    #[test]
    fn sessions_can_be_turned_off_and_their_directories_named() {
        let text = r#"{ "sessions": false, "session_dirs": { "claude": ["~/.claude", "~/.claude-work"] } }"#;
        let settings = Settings::parse(text).unwrap();
        let dirs = settings.session_dirs.unwrap();
        assert_eq!((settings.sessions, dirs.claude.map(|dirs| dirs.len()), dirs.codex), (Some(false), Some(2), None));
        assert_eq!(settings.session_quiet_minutes, None);
        let quiet = |text: &str| Settings::read(text).0.session_quiet_minutes;
        assert_eq!(
            (quiet(r#"{ "session_quiet_minutes": 10 }"#), quiet(r#"{ "session_quiet_minutes": 0 }"#)),
            (Some(10), Some(0))
        );
        let (kept, wrong) = Settings::read(r#"{ "session_quiet_minutes": -1 }"#);
        assert!(kept.session_quiet_minutes.is_none() && wrong[0].starts_with("session_quiet_minutes:"), "{wrong:?}");
        let (_, wrong) = Settings::read(r#"{ "session_dirs": { "gemini": "~/.gemini" } }"#);
        assert!(wrong[0].starts_with("session_dirs: unknown field"), "{wrong:?}");
        // A model that is not a name is left out and named; the others and the other keys stay.
        let text =
            r#"{ "agent_models": { "claude": ["claude-next-1", "--yolo", "a b"], "codex": [] }, "sessions": false }"#;
        let (settings, wrong) = Settings::read(text);
        let models = settings.agent_models.unwrap();
        assert_eq!((models.claude.unwrap(), models.codex.unwrap()), (vec!["claude-next-1".to_string()], Vec::new()));
        assert_eq!((wrong.len(), settings.sessions), (2, Some(false)));
        assert!(wrong[0].starts_with(r#"agent_models: "--yolo" is not a model name"#), "{wrong:?}");
        assert!(Settings::parse(text).is_err());
        let (settings, wrong) = Settings::read(r#"{ "default_agent": "ana", "sessions": false }"#);
        assert_eq!((settings.default_agent, settings.sessions, wrong.len()), (None, Some(false), 1));
        assert!(wrong[0].starts_with(r#"default_agent: "ana" is not `claude`"#), "{wrong:?}");
        let (settings, wrong) = Settings::read(r#"{ "default_agent": "codex-gpt-6-astra" }"#);
        assert_eq!((settings.default_agent.as_deref(), wrong.len()), (Some("codex-gpt-6-astra"), 0));
        // The same answer with a list of models beside it, and from the file read whole.
        let text = r#"{ "agent_models": { "codex": ["gpt-7"] }, "default_agent": "CODEX-GPT-7" }"#;
        assert_eq!((Settings::read(text).0.default_agent, Settings::parse(text).is_err()), (None, true));
        // So does an entry that is no text. A runner nobody knows: the key takes its default.
        let (settings, wrong) = Settings::read(r#"{ "agent_models": { "claude": ["claude-next-1", 7] } }"#);
        assert_eq!(settings.agent_models.unwrap().claude.unwrap(), ["claude-next-1"]);
        assert_eq!(wrong, ["agent_models: 7 is not a model name: letters, digits, `.`, `-` and `_`, at most 64"]);
        let (settings, wrong) = Settings::read(r#"{ "agent_models": { "gemini": [] } }"#);
        assert_eq!((settings.agent_models, wrong.len()), (None, 1));
    }

    #[test]
    fn a_mistake_is_reported_and_a_wild_size_is_brought_in() {
        let err = Settings::parse(r#"{ "terminal": { "font_family": "Menlo" } }"#).unwrap_err();
        assert!(err.contains("font_family"), "{err}");
        assert!(Settings::parse("{ not json").is_err());
        for own_folder in [r#"{"tasks": ""}"#, r#"{"tasks": "~/"}"#, r#"{"tasks": "."}"#] {
            assert!(Settings::parse(own_folder).unwrap_err().contains("folder of its own"));
        }
        let size = |text: &str| Settings::parse(text).unwrap().terminal;
        assert_eq!(size(r#"{"terminal":{"font_size":400}}"#).font_size, 72.);
        assert_eq!(
            size(r#"{"terminal":{"font_size":1,"font":" "}}"#),
            Terminal { font: "Menlo".into(), font_size: 6. }
        );
        assert_eq!(size(r#"{"terminal":{"font_size":16}}"#).line_height(), 22.);
    }

    #[test]
    fn apps_named_in_the_settings_are_listed_while_they_do_not_run() {
        let settings = Settings::parse(r#"{"apps": ["Visual Studio Code", "Slack"]}"#).unwrap();
        assert_eq!(settings.apps, ["Visual Studio Code", "Slack"]);
        let app = |name: &str| Installed { name: name.into(), path: format!("/Applications/{name}.app") };
        // Code runs (under the process name "Code"); Slack was named twice.
        let wanted = [app("Visual Studio Code"), app("Slack"), app("Notes"), app("Slack")];
        let running = vec!["/Applications/Visual Studio Code.app".to_string(), "/System/Finder.app".to_string()];
        assert_eq!(closed_apps(&wanted, &running), [app("Slack"), app("Notes")]);
        assert!(closed_apps(&[], &running).is_empty());
    }

    #[test]
    fn one_wrong_key_does_not_undo_the_others() {
        let text =
            r#"{ "tasks": "~/notes/tasks", "terminl": {"font": "Monaco"}, "repos": "~/code", "apps": ["Slack"] }"#;
        let (settings, wrong) = Settings::read(text);
        // The tasks folder and the apps stand; each wrong key is named at the start of its line.
        assert_eq!(
            (settings.tasks.as_deref(), settings.apps.as_slice()),
            (Some("~/notes/tasks"), &["Slack".to_string()][..])
        );
        assert_eq!((settings.repos, settings.terminal), (None, Terminal::default()));
        assert_eq!(wrong.len(), 2, "{wrong:?}");
        assert!(
            wrong.iter().any(|w| w.starts_with("terminl: ")) && wrong.iter().any(|w| w.starts_with("repos: ")),
            "{wrong:?}"
        );
        assert!(wrong.iter().all(|w| !w.contains(" at line ")), "no position in a copy of the file: {wrong:?}");
        // A wrong key inside `terminal` costs that group only; the folder rule still applies.
        let (settings, wrong) = Settings::read(r#"{"terminal": {"font_family": "Monaco"}, "tasks": "t"}"#);
        assert_eq!((settings.terminal, settings.tasks.as_deref(), wrong.len()), (Terminal::default(), Some("t"), 1));
        assert!(Settings::read(r#"{"tasks": "~"}"#).1[0].starts_with("tasks: "));
        // Every key the struct has survives a read: none is dropped for want of a mention here.
        let whole =
            r#"{"terminal": {"font": "Monaco", "font_size": 15}, "repos": ["~/a"], "tasks": "t", "apps": ["Slack"]}"#;
        assert_eq!(Settings::read(whole), (Settings::parse(whole).unwrap(), Vec::new()));
        // Nothing written, a mark some editors add, a file that is no object, a file that is not JSON.
        assert_eq!(Settings::read("\u{feff}{}"), (Settings::default(), Vec::new()));
        assert_eq!(Settings::read("").0, Settings::default());
        assert_eq!(
            Settings::read("[1]"),
            (Settings::default(), vec!["the file must be a JSON object: { ... }".to_string()])
        );
        let (settings, wrong) = Settings::read("{ not json");
        assert!(
            settings == Settings::default() && wrong.len() == 1 && wrong[0].starts_with("not valid JSON"),
            "{wrong:?}"
        );
        // Of a key written twice, the last one counts.
        assert_eq!(Settings::read(r#"{"tasks": "a", "tasks": "b"}"#).0.tasks.as_deref(), Some("b"));
    }

    #[test]
    fn a_calendar_feed_is_a_link_or_a_path_and_a_wrong_one_is_never_named() {
        let read = |text: &str| Settings::read(text);
        let ok = read(
            r#"{"calendar_ics": "https://calendar.example.com/private-s3cret/basic.ics", "calendar_window_hours": 6}"#,
        );
        assert_eq!((ok.0.calendar_ics.is_some(), ok.0.calendar_window_hours, ok.1.len()), (true, Some(6), 0));
        assert_eq!(read(r#"{"calendar_ics": "~/cal/today.ics"}"#).1, Vec::<String>::new());
        for bad in [
            "http://calendar.example.com/s3cret.ics",
            "https://127.0.0.1/s3cret.ics",
            "s3cret.ics",
            "file:///s3cret.ics",
        ] {
            let (settings, wrong) = read(&format!(r#"{{"calendar_ics": "{bad}"}}"#));
            assert_eq!(settings.calendar_ics, None);
            assert!(
                wrong.len() == 1 && wrong[0].starts_with("calendar_ics: ") && !wrong[0].contains("s3cret"),
                "{wrong:?}"
            );
        }
        assert_eq!(read(r#"{"calendar_window_hours": 0}"#).1.len(), 1);
        assert_eq!(read(r#"{"calendar_window_hours": 25}"#).1.len(), 1);
        assert_eq!(read(r#"{"calendar_refresh_minutes": 0}"#).1.len(), 1);
        assert_eq!(read(r#"{"calendar_notify": false}"#).0.calendar_notify, Some(false));
        assert_eq!(read(r#"{"calendar_notify_for": "links"}"#).0.calendar_notify_for, Some(NotifyFor::Links));
        assert_eq!(read(r#"{"calendar_notify_for": "some"}"#).1.len(), 1);
    }

    #[test]
    fn a_pin_rewrites_two_keys_and_leaves_the_rest_of_the_file_as_it_is() {
        let list = |items: &[&str]| items.iter().map(|item| item.to_string()).collect::<Vec<_>>();
        let cases: [(&str, &str, &[&str], &[&str], &str); 9] = [
            ("no file yet", "", &["Slack"], &[], "{\n  \"apps\": [\"Slack\"]\n}\n"),
            ("nothing to say, nothing written", "", &[], &[], ""),
            (
                "an empty object",
                "{}",
                &["Slack"],
                &["Notes"],
                "{\n  \"apps\": [\"Slack\"],\n  \"apps_hidden\": [\"Notes\"]\n}",
            ),
            (
                "a value is replaced where it is; other keys, an unknown one and the spacing stay",
                "{\n    \"terminal\": { \"font\": \"Monaco\" },\n    \"apps\":   [\"Old\",\n      \"Older\"],\n    \"colour\": \"teal\"\n}\n",
                &["Slack", "Lumen \"2\""],
                &[],
                "{\n    \"terminal\": { \"font\": \"Monaco\" },\n    \"apps\":   [\"Slack\", \"Lumen \\\"2\\\"\"],\n    \"colour\": \"teal\"\n}\n",
            ),
            (
                "a new key goes after the last, indented as it is",
                "{\n\t\"repos\": [\"~/code\"],\n\t\"sessions\": false\n}\n",
                &["Slack"],
                &["Notes"],
                "{\n\t\"repos\": [\"~/code\"],\n\t\"sessions\": false,\n\t\"apps\": [\"Slack\"],\n\t\"apps_hidden\": [\"Notes\"]\n}\n",
            ),
            (
                "a file on one line stays on one line",
                "{ \"tasks\": \"t\", \"x\": {\"a}\": \"}\\\"\"} }",
                &["Slack"],
                &[],
                "{ \"tasks\": \"t\", \"x\": {\"a}\": \"}\\\"\"}, \"apps\": [\"Slack\"] }",
            ),
            (
                "a list emptied keeps its key",
                "{\"apps\":[\"Slack\"],\"apps_hidden\":[\"Notes\"]}",
                &[],
                &[],
                "{\"apps\":[],\"apps_hidden\":[]}",
            ),
            (
                "of a key written twice, the one that counts is changed",
                "{\"apps\": [\"A\"], \"apps\": [\"B\"]}",
                &["C"],
                &[],
                "{\"apps\": [\"A\"], \"apps\": [\"C\"]}",
            ),
            (
                "the mark some editors start a file with stays",
                "\u{feff}{}",
                &["Slack"],
                &[],
                "\u{feff}{\n  \"apps\": [\"Slack\"]\n}",
            ),
        ];
        for (what, text, apps, hidden, wanted) in cases {
            let new = with_apps(text, &list(apps), &list(hidden)).unwrap();
            assert_eq!(new, wanted, "{what}");
            // What was written reads back as the lists, and every other key as before.
            let (before, after) = (Settings::read(text).0, Settings::read(&new).0);
            assert_eq!((&after.apps, &after.apps_hidden), (&list(apps), &list(hidden)), "{what}");
            assert_eq!(
                Settings { apps: Vec::new(), apps_hidden: Vec::new(), ..after },
                Settings { apps: Vec::new(), apps_hidden: Vec::new(), ..before },
                "{what}"
            );
        }
        // A file that cannot be read as an object is not rewritten.
        assert!(with_apps("{ not json", &list(&["Slack"]), &[]).unwrap_err().starts_with("not valid JSON"));
        assert!(with_apps("[1]", &list(&["Slack"]), &[]).unwrap_err().contains("JSON object"));
        assert!(with_apps("{\"apps\": [\"A\"],}", &[], &[]).is_err());
    }

    #[test]
    fn a_pin_changes_the_list_the_file_has_now_and_drops_nothing_of_it() {
        let change = |spec: &str| {
            let (key, text) = spec[1..].split_once(':').unwrap();
            let key = if key == "apps" { "apps" } else { "apps_hidden" };
            ListChange { key, add: spec.starts_with('+'), text: text.into() }
        };
        let after =
            |text: &str, specs: &[&str]| with_changes(text, &specs.iter().map(|s| change(s)).collect::<Vec<_>>());
        let ok = |text: &str, specs: &[&str]| after(text, specs).unwrap();
        // One entry is added to what the file holds, whatever was read at start: a line added
        // by hand since, or by another ottospace, stays. An entry is there once.
        assert_eq!(ok(r#"{"apps": ["Slack", "Notes"]}"#, &["+apps:Zed"]), r#"{"apps": ["Slack", "Notes", "Zed"]}"#);
        assert_eq!(ok(r#"{"apps": ["Zed", "Notes"]}"#, &["+apps:Zed"]), r#"{"apps": ["Zed", "Notes"]}"#);
        assert_eq!(ok(r#"{"apps": ["Foo"]}"#, &["+apps:Foo", "+apps:Foo"]), r#"{"apps": ["Foo"]}"#);
        // One is taken off; taking off what is not there changes nothing, byte for byte.
        assert_eq!(ok(r#"{"apps": ["Slack", "Zed", "Notes"]}"#, &["-apps:Zed"]), r#"{"apps": ["Slack", "Notes"]}"#);
        assert_eq!(ok("{ \"apps\" : [ \"Slack\" ] }", &["-apps:Zed"]), "{ \"apps\" : [ \"Slack\" ] }");
        // A pin of a hidden app: on one list, off the other. The list not named is not touched,
        // even one this could not read.
        let both = ok(r#"{"apps_hidden": ["Notes", "Music"], "apps": []}"#, &["-hidden:Notes", "+apps:Notes"]);
        assert_eq!(both, r#"{"apps_hidden": ["Music"], "apps": ["Notes"]}"#);
        assert_eq!(
            ok(r#"{"apps": ["A"], "apps_hidden": 7}"#, &["+apps:B"]),
            r#"{"apps": ["A", "B"], "apps_hidden": 7}"#
        );
        // A list that is not a list of names is not written over: the entries it has that
        // were not understood would be lost.
        for text in [r#"{"apps": ["Slack", "Notes", 3]}"#, r#"{"apps": "Slack"}"#, r#"{"apps": null}"#] {
            assert!(after(text, &["+apps:Zed"]).unwrap_err().contains("`apps` is not a list of names"), "{text}");
        }
        assert!(after("{ not json", &["+apps:Zed"]).is_err() && after("[1]", &["+apps:Zed"]).is_err());
        // No file yet, or an empty one: the key is added; taking off from nothing adds none.
        assert_eq!(ok("", &["+apps:Zed"]), "{\n  \"apps\": [\"Zed\"]\n}\n");
        assert_eq!(ok("", &["-apps:Zed"]), "");
        // Windows line ends are kept in the line that is added.
        assert_eq!(
            ok("{\r\n  \"tasks\": \"t\"\r\n}\r\n", &["+apps:Zed"]),
            "{\r\n  \"tasks\": \"t\",\r\n  \"apps\": [\"Zed\"]\r\n}\r\n"
        );
    }

    #[test]
    fn a_directory_added_to_repos_keeps_the_default_ones_of_a_file_that_names_none() {
        let change = |add: bool, text: &str| [ListChange { key: "repos", add, text: text.into() }];
        let after = |text: &str, add: bool, dir: &str| with_changes(text, &change(add, dir)).unwrap();
        assert_eq!(after(r#"{"repos": ["~/work"]}"#, true, "/src/app"), r#"{"repos": ["~/work", "/src/app"]}"#);
        assert_eq!(after(r#"{"repos": ["~/work", "/src/app"]}"#, false, "/src/app"), r#"{"repos": ["~/work"]}"#);
        // No `repos`: `~/code` and `~/notes` are listed, and stay so once the key is there.
        let defaults = "{\n  \"repos\": [\"~/code\", \"~/notes\", \"/src/app\"]\n}\n";
        assert_eq!(after("", true, "/src/app"), defaults);
        assert_eq!(after(r#"{"apps": []}"#, false, "~/notes"), r#"{"apps": [], "repos": ["~/code"]}"#);
        // Taking off what is not there writes nothing.
        assert_eq!(after(r#"{"apps": []}"#, false, "/src/app"), r#"{"apps": []}"#);
        // A full list takes no more, whoever filled it; one can still be taken off.
        let full =
            serde_json::json!({"repos": (0..crate::repos::MAX_DIRS).map(|n| format!("/d{n}")).collect::<Vec<_>>()});
        let refused = with_changes(&full.to_string(), &change(true, "/src/app")).unwrap_err();
        assert_eq!(refused, "`repos` names 64 directories already");
        assert!(!after(&full.to_string(), false, "/d1").contains("\"/d1\""));
    }

    #[test]
    fn the_dock_setting_and_the_hidden_apps_are_read() {
        let settings = Settings::parse(r#"{"apps_hidden": ["Music"], "suggest_dock_apps": false}"#).unwrap();
        assert_eq!((settings.apps_hidden, settings.suggest_dock_apps), (vec!["Music".to_string()], Some(false)));
        assert_eq!(Settings::default().suggest_dock_apps, None);
        assert_eq!(Settings::parse(r#"{"wheel_to_apps": false}"#).unwrap().wheel_to_apps, Some(false));
    }

    #[test]
    fn a_tilde_is_the_home_directory() {
        assert_eq!(expand_home("~/code", "/Users/me"), "/Users/me/code");
        assert_eq!(expand_home("~", "/Users/me/"), "/Users/me/");
        assert_eq!(expand_home("/opt/repos", "/Users/me"), "/opt/repos");
        assert_eq!(expand_home("~other/code", "/Users/me"), "~other/code");
    }
}
