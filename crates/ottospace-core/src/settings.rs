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
///   "session_quiet_minutes": 3 }
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

/// The settings file's text with `apps` and `apps_hidden` set to these lists, for a pin made in
/// the Apps section. Only those two values change: every other key, known or not, and the
/// file's own spacing stay as they are. A list that is empty and has no key gets none.
/// `Err`: the file is not a JSON object, and must be left alone.
pub fn with_apps(text: &str, apps: &[String], hidden: &[String]) -> Result<String, String> {
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
    let Some((_, last_key, last)) = members.last() else {
        let open = text.find('{')? + 1;
        return Some(format!("{}\n  \"{key}\": {value}\n{}", &text[..open], text[open..].trim_start()));
    };
    // The last key's own line start, if only spaces come before it; else all is on one line.
    let line = text[..*last_key].rfind('\n').map(|at| &text[at..*last_key]).filter(|gap| gap.trim().is_empty());
    let between = line.map_or_else(|| ", ".to_string(), |indent| format!(",{indent}"));
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
    fn the_dock_setting_and_the_hidden_apps_are_read() {
        let settings = Settings::parse(r#"{"apps_hidden": ["Music"], "suggest_dock_apps": false}"#).unwrap();
        assert_eq!((settings.apps_hidden, settings.suggest_dock_apps), (vec!["Music".to_string()], Some(false)));
        assert_eq!(Settings::default().suggest_dock_apps, None);
    }

    #[test]
    fn a_tilde_is_the_home_directory() {
        assert_eq!(expand_home("~/code", "/Users/me"), "/Users/me/code");
        assert_eq!(expand_home("~", "/Users/me/"), "/Users/me/");
        assert_eq!(expand_home("/opt/repos", "/Users/me"), "/opt/repos");
        assert_eq!(expand_home("~other/code", "/Users/me"), "~other/code");
    }
}
