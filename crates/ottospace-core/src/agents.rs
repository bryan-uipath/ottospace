//! What an `@agent` tag names: a runner (`claude`), a runner and a model (`claude-opus-5-5`,
//! `codex-gpt-6-astra`), or a saved agent. `read` is the one reading of a tag.

use crate::sessions::RUNNERS;
use crate::settings::AgentModels;

/// The runner whose model ids start with its own name: `claude-opus-5-5` is tag and id both.
const WHOLE: &str = "claude";

/// The longest model id, or what follows `claude-` in one typed outright.
pub const MAX_MODEL: usize = 64;

/// The models offered for each runner, as its CLI names them.
#[derive(Clone, Debug, PartialEq)]
pub struct Models {
    pub claude: Vec<String>,
    pub codex: Vec<String>,
}

impl Default for Models {
    fn default() -> Self {
        let list = |ids: &[&str]| ids.iter().map(|id| id.to_string()).collect();
        Self {
            claude: list(&["claude-fable-5-2", "claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"]),
            codex: list(&["gpt-6-astra"]),
        }
    }
}

impl Models {
    /// The defaults, with each runner's list the settings name in place of its own. Only ids
    /// that are names (`is_name`) count.
    pub fn with(settings: Option<&AgentModels>) -> Self {
        let named = |list: &Option<Vec<String>>, default: Vec<String>| match list {
            Some(ids) => ids.iter().filter(|id| is_name(id)).cloned().collect(),
            None => default,
        };
        let default = Self::default();
        let Some(settings) = settings else { return default };
        Self { claude: named(&settings.claude, default.claude), codex: named(&settings.codex, default.codex) }
    }

    fn of(&self, runner: &str) -> &[String] {
        if runner == WHOLE { &self.claude } else { &self.codex }
    }
}

/// True if `id` can be a model's id: letters, digits, and `.`, `-` or `_` between them one at a
/// time, at most `MAX_MODEL`. Never a word a command line would read as an option. So what
/// follows the runner in a listed model's tag is a name too, and `typed` reads every one.
pub fn is_name(id: &str) -> bool {
    let parts = || id.split(['.', '-', '_']);
    id.len() <= MAX_MODEL && parts().all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// The tag's name for a runner's model: `gpt-6-astra` → `codex-gpt-6-astra`. An id that
/// starts with the runner's name is the tag: `claude-opus-5-5`.
pub fn tag(runner: &str, id: &str) -> String {
    if id.strip_prefix(runner).is_some_and(|rest| rest.starts_with('-')) { id.into() } else { format!("{runner}-{id}") }
}

/// What a tag names. A saved agent has neither: its runner is in its file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Agent {
    pub runner: Option<&'static str>,
    /// As the runner's CLI takes it. `None`: the runner's own default.
    pub model: Option<String>,
}

/// A runner and a model typed outright, listed or not: `codex-gpt-7` → (`codex`, `gpt-7`).
/// The runner's name in small letters, then `-`, then a name (`is_name`).
pub fn typed(name: &str) -> Option<(&'static str, &str)> {
    RUNNERS.iter().find_map(|runner| {
        let rest = name.strip_prefix(runner)?.strip_prefix('-')?;
        is_name(rest).then_some((*runner, if *runner == WHOLE { name } else { rest }))
    })
}

/// Reads an agent tag's name, whatever the case of a listed one. `claude` → the runner;
/// `claude-opus-5-5` → `claude` and that id; `codex-gpt-6-astra` → `codex` and `gpt-6-astra`;
/// `ana` → a saved agent. A listed model gives its id as listed; one not listed, as typed.
pub fn read(name: &str, models: &Models) -> Agent {
    for runner in RUNNERS {
        if name.eq_ignore_ascii_case(runner) {
            return Agent { runner: Some(runner), model: None };
        }
        if let Some(id) = models.of(runner).iter().find(|id| tag(runner, id).eq_ignore_ascii_case(name)) {
            return Agent { runner: Some(runner), model: Some(id.clone()) };
        }
    }
    typed(name).map_or_else(Agent::default, |(runner, id)| Agent { runner: Some(runner), model: Some(id.into()) })
}

/// What a name is, for the dropdown's row: `runner`, `claude model`, `saved agent`.
pub fn kind(name: &str) -> &'static str {
    match (RUNNERS.iter().any(|runner| name.eq_ignore_ascii_case(runner)), typed(name)) {
        (true, _) => "runner",
        (_, Some((WHOLE, _))) => "claude model",
        (_, Some(_)) => "codex model",
        _ => "saved agent",
    }
}

/// The names `@` offers, in order: each runner and then its models, then the saved agents.
/// A saved agent named as a built-in or as a runner's model (`codex-reviewer`) is left out:
/// that is what the name means.
pub fn names(models: &Models, saved: Vec<String>) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let built_in = RUNNERS.iter().flat_map(|runner| {
        std::iter::once(runner.to_string()).chain(models.of(runner).iter().map(|id| tag(runner, id)))
    });
    for name in built_in.chain(saved.into_iter().filter(|name| typed(name).is_none())) {
        if !names.iter().any(|seen| seen.eq_ignore_ascii_case(&name)) {
            names.push(name);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    fn agent(runner: &'static str, model: Option<&str>) -> Agent {
        Agent { runner: Some(runner), model: model.map(str::to_string) }
    }

    #[test]
    fn a_tag_reads_as_a_runner_a_runner_and_model_or_a_saved_agent() {
        let models = Models::default();
        let cases = [
            ("claude", agent("claude", None)),
            ("CODEX", agent("codex", None)),
            // Claude's ids start with `claude-`: the tag is the id. Codex's do not.
            ("claude-opus-5-5", agent("claude", Some("claude-opus-5-5"))),
            ("Claude-Opus-5-5", agent("claude", Some("claude-opus-5-5"))),
            ("codex-gpt-6-astra", agent("codex", Some("gpt-6-astra"))),
            // Not listed, and still a runner and a model: a new one works by typing it.
            ("claude-opus-9", agent("claude", Some("claude-opus-9"))),
            ("codex-gpt-7.1_mini", agent("codex", Some("gpt-7.1_mini"))),
            ("ana", Agent::default()),
            ("claudia", Agent::default()),
            ("claude-", Agent::default()),
            // A model's id never starts with `-`: a command line would read it as an option.
            ("codex--yolo", Agent::default()),
            ("codex-a b", Agent::default()),
            ("Claude-opus-9", Agent::default()),
        ];
        for (name, want) in cases {
            assert_eq!(read(name, &models), want, "{name}");
        }
        let kinds = ["claude", "claude-opus-9", "codex-gpt-6-astra", "ana"].map(kind);
        assert_eq!(kinds, ["runner", "claude model", "codex model", "saved agent"]);
        assert_eq!(read(&format!("codex-{}", "x".repeat(MAX_MODEL + 1)), &models), Agent::default());
        assert!(is_name(&"x".repeat(MAX_MODEL)) && !is_name("") && !is_name("a.") && !is_name("été"));
        assert!(!is_name("codex--preview") && !is_name("-a") && is_name("gpt-7.1_mini"));
    }

    #[test]
    fn the_settings_replace_a_runners_models() {
        let settings = AgentModels { claude: Some(list(&["opus", "claude-next-1", "-bad", "a b"])), codex: None };
        let models = Models::with(Some(&settings));
        assert_eq!(
            (models.claude.clone(), models.codex.clone()),
            (list(&["opus", "claude-next-1"]), list(&["gpt-6-astra"]))
        );
        assert_eq!(Models::with(None), Models::default());
        // A listed id is given back as listed: `opus` is what the CLI takes, not `claude-opus`.
        assert_eq!(read("claude-opus", &models), agent("claude", Some("opus")));
        assert_eq!(read("claude-next-1", &models), agent("claude", Some("claude-next-1")));
        // An empty list: the runner alone.
        let none = Models::with(Some(&AgentModels { claude: Some(Vec::new()), codex: Some(Vec::new()) }));
        assert_eq!(names(&none, Vec::new()), ["claude", "codex"]);
        // Codex names a model after itself too.
        let codex = Models { claude: Vec::new(), codex: list(&["codex-mini-latest", "gpt-6-codex"]) };
        assert_eq!(names(&codex, Vec::new()), ["claude", "codex", "codex-mini-latest", "codex-gpt-6-codex"]);
        assert_eq!(read("codex-mini-latest", &codex), agent("codex", Some("codex-mini-latest")));
        // Every model that can be listed is a row that says so, however long its id.
        let long = Models { claude: vec!["x".repeat(MAX_MODEL)], codex: vec![format!("codex-{}", "x".repeat(58))] };
        assert_eq!(
            names(&long, Vec::new()).iter().map(|name| kind(name)).filter(|kind| kind.ends_with("model")).count(),
            2
        );
    }

    #[test]
    fn each_runner_then_its_models_then_the_saved_agents() {
        let saved = list(&["ana", "Claude", "codex-gpt-6-astra", "codex-reviewer", "reviewer"]);
        let want = [
            "claude",
            "claude-fable-5-2",
            "claude-opus-5-5",
            "claude-sonnet-5-5",
            "claude-haiku-5-5",
            "codex",
            "codex-gpt-6-astra",
            "ana",
            "reviewer",
        ];
        // A saved agent named as a built-in is not offered twice, nor one that reads as a model.
        assert_eq!(names(&Models::default(), saved), want);
        // Every built-in offered is a tag's name, and its row says what `read` reads.
        for name in &want[..7] {
            let read = read(name, &Models::default());
            assert!(crate::compose::taggable('@', name) && read.runner.is_some(), "{name}");
            assert_eq!(kind(name) == "runner", read.model.is_none(), "{name}");
        }
    }
}
