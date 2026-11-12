//! What `settings.json` may say, and what applies when it says nothing. The shell reads the file.

use serde::Deserialize;

/// `~/.ottospace/settings.json`. Every key is optional. An unknown key is an error, so a typing
/// mistake does not pass for a setting that silently does nothing.
///
/// ```json
/// { "terminal": { "font": "JetBrains Mono", "font_size": 14 },
///   "repos": ["~/code", "~/work/clients"],
///   "tasks": "~/notes/tasks" }
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
    /// Apps to offer when they are not running, by name: `["Microsoft Outlook", "Slack"]`.
    /// Choosing one starts it.
    #[serde(default)]
    pub apps: Vec<String>,
    #[serde(default)]
    pub routines: Routines,
}

/// `{ "routines": { "socket": "~/.ottobot/otto.sock" } }`
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Routines {
    /// The ottobot daemon's socket. Default: `~/.ottobot/otto.sock`.
    #[serde(default)]
    pub socket: Option<String>,
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
        assert_eq!(settings.routines.socket, None);
        let named = Settings::parse(r#"{"routines": {"socket": "~/otto.sock"}}"#).unwrap();
        assert_eq!(named.routines.socket.as_deref(), Some("~/otto.sock"));
        assert!(Settings::parse(r#"{"routines": {"sock": "x"}}"#).is_err());
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
    fn a_tilde_is_the_home_directory() {
        assert_eq!(expand_home("~/code", "/Users/me"), "/Users/me/code");
        assert_eq!(expand_home("~", "/Users/me/"), "/Users/me/");
        assert_eq!(expand_home("/opt/repos", "/Users/me"), "/opt/repos");
        assert_eq!(expand_home("~other/code", "/Users/me"), "~other/code");
    }
}
