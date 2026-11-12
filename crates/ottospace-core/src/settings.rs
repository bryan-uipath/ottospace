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
    /// Apps to offer when they are not running, by name: `["Microsoft Outlook", "Slack"]`.
    /// Choosing one starts it.
    #[serde(default)]
    pub apps: Vec<String>,
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
}

/// The apps to list as not running: the ones the settings name, less those that run now
/// (whatever the case of the letters), each once, in the order given.
pub fn closed_apps(wanted: &[String], running: &[String]) -> Vec<String> {
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    let mut closed: Vec<String> = Vec::new();
    for name in wanted.iter().map(|name| name.trim()).filter(|name| !name.is_empty()) {
        if !running.iter().any(|r| same(r, name)) && !closed.iter().any(|c| same(c, name)) {
            closed.push(name.to_string());
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
        let settings = Settings::parse(r#"{"apps": ["Microsoft Outlook", "slack", " ", "Slack", "Notes"]}"#).unwrap();
        let running = vec!["Finder".to_string(), "Slack".to_string()];
        assert_eq!(closed_apps(&settings.apps, &running), ["Microsoft Outlook", "Notes"]);
        assert!(closed_apps(&Settings::default().apps, &running).is_empty());
    }

    #[test]
    fn a_tilde_is_the_home_directory() {
        assert_eq!(expand_home("~/code", "/Users/me"), "/Users/me/code");
        assert_eq!(expand_home("~", "/Users/me/"), "/Users/me/");
        assert_eq!(expand_home("/opt/repos", "/Users/me"), "/opt/repos");
        assert_eq!(expand_home("~other/code", "/Users/me"), "~other/code");
    }
}
