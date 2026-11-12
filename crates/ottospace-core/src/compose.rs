//! The new-task box: one line of text becomes a task. `#tag` names a project, `@name` a
//! repository, `p1`..`p9` a priority; the rest is the title.

/// What one line says. e.g. `fix zoom flicker #canvas @app p1`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub title: String,
    /// From `#canvas`: the project the task serves. A task need not have one.
    pub project: Option<String>,
    /// From `@app`: the name of a listed repository, as typed.
    pub repo: Option<String>,
    pub priority: Option<u8>,
}

/// A project or repository name: starts with a letter, so `#123` (an issue) stays in the title.
/// Punctuation after it is not part of it: `#canvas,` names `canvas`.
fn name(word: &str, mark: char) -> Option<&str> {
    let name = word.strip_prefix(mark)?.trim_end_matches(|c: char| c.is_ascii_punctuation() && !matches!(c, '-' | '_'));
    name.chars().next().filter(|c| c.is_alphabetic()).map(|_| name)
}

/// Reads a line. Of two `#tags` (or `@names`, or priorities) the last one counts. An `@name` is
/// a repository only if `is_repo` knows it; any other stays in the title, where it was typed.
pub fn parse(line: &str, is_repo: impl Fn(&str) -> bool) -> Draft {
    let mut draft = Draft::default();
    let mut title: Vec<&str> = Vec::new();
    for word in line.split_whitespace() {
        let priority = word.strip_prefix(['p', 'P']).and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=9).contains(n));
        if let Some(project) = name(word, '#') {
            draft.project = Some(project.to_string());
        } else if let Some(repo) = name(word, '@').filter(|name| is_repo(name)) {
            draft.repo = Some(repo.to_string());
        } else if priority.is_some() && word.len() == 2 {
            draft.priority = priority;
        } else {
            title.push(word);
        }
    }
    draft.title = title.join(" ");
    draft
}

/// The `#` or `@` word being typed at the end of the line, as (mark, what is typed of the name).
/// `fix @ap` → `('@', "ap")`. `None` after a space, or when the last word is ordinary.
pub fn completing(line: &str) -> Option<(char, &str)> {
    let word = line.rsplit(char::is_whitespace).next()?;
    let mark = word.chars().next().filter(|c| matches!(c, '#' | '@'))?;
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

/// The line with the word being typed replaced by `name`, and a space to go on typing.
pub fn complete(line: &str, name: &str) -> String {
    match completing(line) {
        Some((mark, typed)) => format!("{}{mark}{name} ", &line[..line.len() - typed.len() - 1]),
        None => line.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(name: &str) -> bool {
        name.eq_ignore_ascii_case("app")
    }

    #[test]
    fn a_line_is_a_title_with_optional_marks() {
        let draft = parse("fix zoom flicker #canvas @app p1", known);
        assert_eq!(draft.title, "fix zoom flicker");
        assert_eq!(
            (draft.project.as_deref(), draft.repo.as_deref(), draft.priority),
            (Some("canvas"), Some("app"), Some(1))
        );
        // Marks may come anywhere; the last of a kind counts; punctuation after a name is dropped.
        let draft = parse("#a P2 look at   #b-2, the logs in @App.", known);
        assert_eq!(
            (draft.title.as_str(), draft.project.as_deref(), draft.priority),
            ("look at the logs in", Some("b-2"), Some(2))
        );
        assert_eq!(draft.repo.as_deref(), Some("App"));
        assert_eq!(parse("  ", known).title, "");
    }

    #[test]
    fn words_that_only_look_like_marks_stay_in_the_title() {
        let draft = parse("see #123 and p10 or p0, mail a@b.c, # alone, @ too", known);
        assert_eq!(draft.title, "see #123 and p10 or p0, mail a@b.c, # alone, @ too");
        assert_eq!((draft.project, draft.repo, draft.priority), (None, None, None));
        // A person, not a repository: the word stays where it was typed.
        let draft = parse("ask @ana about @app today", known);
        assert_eq!((draft.title.as_str(), draft.repo.as_deref()), ("ask @ana about today", Some("app")));
    }

    #[test]
    fn the_word_being_typed_is_completed() {
        assert_eq!(completing("fix @ap"), Some(('@', "ap")));
        assert_eq!(completing("fix #"), Some(('#', "")));
        assert_eq!((completing("fix @app "), completing("fix"), completing("")), (None, None, None));
        // Names that start with what is typed come first.
        let names = ["flow-app", "App", "notes", "apollo", "App"];
        assert_eq!(suggest("ap", names), ["apollo", "App", "flow-app"]);
        assert_eq!(complete("fix @ap", "apollo"), "fix @apollo ");
        assert_eq!(complete("fix it", "apollo"), "fix it");
    }
}
