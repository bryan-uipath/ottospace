//! The new-task box: one line of text becomes a task. Tags: `#project`, `>repo`, `@agent`,
//! `/path`, `$skill`, `p1`..`p9`; the rest is the title. See "Tags" in `docs/reference.md`.

/// What one line says. e.g. `fix zoom flicker #canvas >app @claude p1`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub title: String,
    /// From `#canvas`: the project the task serves. A task need not have one.
    pub project: Option<String>,
    /// From `>app`: the name of a listed repository, as typed.
    pub repo: Option<String>,
    /// From `@claude`: who works the task, as typed.
    pub agent: Option<String>,
    /// From `/src/export` or `~/notes`: paths the task is about, as typed, each once.
    pub paths: Vec<String>,
    /// From `$code-review`: skills to load for the task, as typed, each once.
    pub skills: Vec<String>,
    pub priority: Option<u8>,
}

/// What the shell last read for the line in the box. The core reads no disk: a tag counts only
/// if its name is here (or, for `>repo`, in the Repos list).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Known {
    pub agents: Vec<String>,
    pub skills: Vec<String>,
    /// The line's path words that name a file or directory.
    pub paths: Vec<String>,
    /// Paths that finish the one being typed. A directory ends in `/`.
    pub entries: Vec<String>,
}

/// A tag's name: starts with a letter, so `#123` (an issue), `> 3` and `$5` stay in the title.
/// Punctuation after it is not part of it: `#canvas,` names `canvas`.
fn name(word: &str, mark: char) -> Option<&str> {
    let name = word.strip_prefix(mark)?.trim_end_matches(|c: char| c.is_ascii_punctuation() && !matches!(c, '-' | '_'));
    name.chars().next().filter(|c| c.is_alphabetic()).map(|_| name)
}

/// A word that may be a path: it starts with `/` or `~/`. Punctuation that ends a clause is not
/// part of it: `/src/export,` is `/src/export`.
pub fn path(word: &str) -> Option<&str> {
    let path = word.trim_end_matches([',', '.', ';', ':', '!', '?', ')']);
    (path.starts_with("~/") || (path.starts_with('/') && path.len() > 1)).then_some(path)
}

/// The words of a line that may be paths, for the shell to look up.
pub fn path_words(line: &str) -> Vec<&str> {
    line.split_whitespace().filter_map(path).collect()
}

/// Reads a line. `known(mark, name)`: true if a `>repo`, `@agent`, `$skill` or `/path` names
/// something that exists; any other stays in the title, where it was typed. Of two `#project`,
/// `>repo`, `@agent` or priorities the last one counts; every `/path` and `$skill` is kept.
pub fn parse(line: &str, known: impl Fn(char, &str) -> bool) -> Draft {
    let mut draft = Draft::default();
    let mut title: Vec<&str> = Vec::new();
    let named = |word, mark| name(word, mark).filter(|name| known(mark, name));
    let once = |list: &mut Vec<String>, item: &str| {
        if !list.iter().any(|kept| kept == item) {
            list.push(item.to_string());
        }
    };
    for word in line.split_whitespace() {
        let priority = word.strip_prefix(['p', 'P']).and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=9).contains(n));
        if let Some(project) = name(word, '#') {
            draft.project = Some(project.to_string());
        } else if let Some(repo) = named(word, '>') {
            draft.repo = Some(repo.to_string());
        } else if let Some(agent) = named(word, '@') {
            draft.agent = Some(agent.to_string());
        } else if let Some(skill) = named(word, '$') {
            once(&mut draft.skills, skill);
        } else if let Some(path) = path(word).filter(|path| known('/', path)) {
            once(&mut draft.paths, path);
        } else if priority.is_some() && word.len() == 2 {
            draft.priority = priority;
        } else {
            title.push(word);
        }
    }
    draft.title = title.join(" ");
    draft
}

/// The tag being typed at the end of the line, as (mark, what is typed of the name).
/// `fix >ap` → `('>', "ap")`. A path is all of its word: `see ~/no` → `('/', "~/no")`.
/// `None` after a space, or when the last word is ordinary.
pub fn completing(line: &str) -> Option<(char, &str)> {
    let word = line.rsplit(char::is_whitespace).next()?;
    if word.starts_with('/') || word.starts_with("~/") {
        return Some(('/', word));
    }
    let mark = word.chars().next().filter(|c| matches!(c, '#' | '>' | '@' | '$'))?;
    Some((mark, &word[1..]))
}

/// Names that complete what is typed: those that start with it, then those that contain it,
/// without regard to case. At most five.
pub fn suggest<'a>(typed: &str, names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let typed = typed.to_lowercase();
    let mut names: Vec<&str> = names.into_iter().filter(|n| n.to_lowercase().contains(&typed)).collect();
    names.sort_by_key(|n| (!n.to_lowercase().starts_with(&typed), n.to_lowercase()));
    names.dedup();
    names.into_iter().take(5).map(str::to_string).collect()
}

/// The line with the word being typed replaced by `name`, and a space to go on typing. No space
/// after a directory (`/src/`): the path goes on.
pub fn complete(line: &str, name: &str) -> String {
    match completing(line) {
        Some(('/', typed)) => {
            let space = if name.ends_with('/') { "" } else { " " };
            format!("{}{name}{space}", &line[..line.len() - typed.len()])
        }
        Some((mark, typed)) => format!("{}{mark}{name} ", &line[..line.len() - typed.len() - 1]),
        None => line.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One repository, two agents, two skills, two paths.
    fn known(mark: char, name: &str) -> bool {
        match mark {
            '>' => name.eq_ignore_ascii_case("app"),
            '@' => ["claude", "codex"].contains(&name),
            '$' => ["code-review", "deploy"].contains(&name),
            '/' => ["/src/export", "~/notes"].contains(&name),
            _ => false,
        }
    }

    #[test]
    fn a_line_is_a_title_with_optional_tags() {
        let draft = parse("fix zoom flicker #canvas >app @claude p1", known);
        assert_eq!(draft.title, "fix zoom flicker");
        assert_eq!(
            (draft.project.as_deref(), draft.repo.as_deref(), draft.agent.as_deref(), draft.priority),
            (Some("canvas"), Some("app"), Some("claude"), Some(1))
        );
        // Tags may come anywhere; the last of a kind counts; punctuation after a name is dropped.
        let draft = parse("#a P2 look at   #b-2, the logs in >App.", known);
        assert_eq!(
            (draft.title.as_str(), draft.project.as_deref(), draft.priority),
            ("look at the logs in", Some("b-2"), Some(2))
        );
        assert_eq!(draft.repo.as_deref(), Some("App"));
        assert_eq!(parse("  ", known).title, "");
    }

    #[test]
    fn a_task_has_one_agent_and_any_number_of_paths_and_skills() {
        let draft = parse("review the export PR >app @claude @codex $code-review /src/export $deploy ~/notes.", known);
        assert_eq!((draft.title.as_str(), draft.agent.as_deref()), ("review the export PR", Some("codex")));
        assert_eq!(
            (draft.skills, draft.paths),
            (vec!["code-review".to_string(), "deploy".into()], vec!["/src/export".to_string(), "~/notes".into()])
        );
        // Named twice, kept once.
        let draft = parse("$deploy /src/export, then $deploy /src/export", known);
        assert_eq!((draft.title.as_str(), draft.skills.len(), draft.paths.len()), ("then", 1, 1));
    }

    #[test]
    fn words_that_only_look_like_tags_stay_in_the_title() {
        let line = "see #123 and p10 or p0, mail a@b.c, # alone, @ too, use > 3 workers, >3 costs $5 or $ 5, a->b";
        let draft = parse(line, known);
        assert_eq!(draft.title, line);
        assert_eq!((draft.project, draft.repo, draft.agent, draft.priority), (None, None, None, None));
        // A person, a repository, a skill or a path nobody knows: the word stays where it was typed.
        let draft = parse("ask @ana about >app today, not >web or $HOME, see /review and / and ~", known);
        assert_eq!(draft.title, "ask @ana about today, not >web or $HOME, see /review and / and ~");
        assert_eq!(
            (draft.repo.as_deref(), draft.agent, draft.skills.len(), draft.paths.len()),
            (Some("app"), None, 0, 0)
        );
        // `@app` named a repository once. It does not now.
        assert_eq!(parse("fix @app", known), Draft { title: "fix @app".into(), ..Draft::default() });
    }

    #[test]
    fn path_words_are_those_the_shell_looks_up() {
        assert_eq!(path_words("see /src/export, ~/notes and a/b or / or ~x"), ["/src/export", "~/notes"]);
    }

    #[test]
    fn the_word_being_typed_is_completed() {
        assert_eq!(completing("fix >ap"), Some(('>', "ap")));
        assert_eq!(
            (completing("fix #"), completing("ask @cl"), completing("with $de")),
            (Some(('#', "")), Some(('@', "cl")), Some(('$', "de")))
        );
        assert_eq!((completing("see /src/ex"), completing("see ~/no")), (Some(('/', "/src/ex")), Some(('/', "~/no"))));
        assert_eq!((completing("fix >app "), completing("fix"), completing("")), (None, None, None));
        // Names that start with what is typed come first.
        let names = ["flow-app", "App", "notes", "apollo", "App"];
        assert_eq!(suggest("ap", names), ["apollo", "App", "flow-app"]);
        assert_eq!(complete("fix >ap", "apollo"), "fix >apollo ");
        assert_eq!(complete("with $de", "deploy"), "with $deploy ");
        assert_eq!(complete("fix it", "apollo"), "fix it");
        // A path replaces its whole word. A directory takes no space: the path goes on.
        assert_eq!(complete("see /src/ex", "/src/export/"), "see /src/export/");
        assert_eq!(complete("see ~/no", "~/notes.md"), "see ~/notes.md ");
    }
}
