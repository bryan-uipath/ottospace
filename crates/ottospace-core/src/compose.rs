//! The new-task box: one line of text becomes a task. Tags: `#project`, `>repo`, `@agent`,
//! `/path`, `$skill`, `p1`..`p9`; the rest is the title. See "Tags" in `docs/reference.md`.

use crate::tasks::MAX_TAG;

/// The most names offered for the tag being typed.
pub const MAX_SUGGESTIONS: usize = 5;

/// What one line says. e.g. `fix zoom flicker #canvas >app @claude p1`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub title: String,
    /// From `#canvas`: the project the task serves. A task need not have one.
    pub project: Option<String>,
    /// From `>app`: the name of a listed repository, as typed.
    pub repo: Option<String>,
    /// From `@claude`: who works the task. From `read`: as spelled where it was read.
    pub agent: Option<String>,
    /// From `/src/export` or `~/notes`: paths the task is about, as typed, each once.
    pub paths: Vec<String>,
    /// Where each of `paths` was found, in the same order. Set by `read`; empty from `parse`.
    pub resolved: Vec<String>,
    /// From `$code-review`: skills to load for the task, each once. From `read`: as spelled
    /// where they were read.
    pub skills: Vec<String>,
    pub priority: Option<u8>,
}

/// A path word that names a file or directory, and where it was found.
#[derive(Clone, Debug, PartialEq)]
pub struct PathFound {
    /// The word, without the backslash before a space: `/src/export`, `~/My Notes`.
    pub typed: String,
    /// The absolute path: `/code/app/src/export`.
    pub resolved: String,
}

/// The agents and skills a tag may name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Names {
    pub agents: Vec<String>,
    pub skills: Vec<String>,
}

/// What the shell last read for a line. The core reads no disk: a tag counts only if its name
/// is here (or, for `>repo`, in the Repos list).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Known {
    pub names: Names,
    /// The line's path words that name a file or directory.
    pub paths: Vec<PathFound>,
    /// Paths that finish the one being typed. A directory ends in `/`.
    pub entries: Vec<String>,
}

/// The words of a line, as typed. A space after a backslash does not end a word: `~/My\ Notes`
/// is one. No other use of a backslash is special.
fn words(line: &str) -> Vec<&str> {
    let (mut words, mut start, mut escaped) = (Vec::new(), None, false);
    for (at, c) in line.char_indices() {
        let ends = c.is_whitespace() && !(escaped && c == ' ');
        match (ends, start) {
            (true, Some(from)) => {
                words.push(&line[from..at]);
                start = None;
            }
            (false, None) => start = Some(at),
            _ => {}
        }
        escaped = c == '\\' && !escaped;
    }
    words.extend(start.map(|from| &line[from..]));
    words
}

/// A word without the backslashes that kept its spaces: `My\ Notes` → `My Notes`.
pub fn unescape(word: &str) -> String {
    word.replace("\\ ", " ")
}

/// A name with a backslash before each space, to go into a line as one word.
fn escape(name: &str) -> String {
    name.replace(' ', "\\ ")
}

/// Punctuation that ends a clause, not a name: `#canvas,` names `canvas`.
fn clause_end(c: char) -> bool {
    c.is_ascii_punctuation() && !matches!(c, '-' | '_')
}

/// A tag's name: starts with a letter, so `#123` (an issue), `> 3` and `$5` stay in the title.
fn name(word: &str, mark: char) -> Option<&str> {
    let name = word.strip_prefix(mark)?.trim_end_matches(clause_end);
    name.chars().next().filter(|c| c.is_alphabetic()).map(|_| name)
}

/// What a word may be as a path, most likely first: the word itself if it starts with `/` or
/// `~/`, then the word without the punctuation that ends a clause (`/src/export,`). A last part
/// that is `.` or `..` is never cut. `word` is without its backslashes.
pub fn path_candidates(word: &str) -> Vec<&str> {
    let shaped = |path: &str| path.starts_with("~/") || (path.starts_with('/') && path.len() > 1);
    if !shaped(word) {
        return Vec::new();
    }
    let cut = word.trim_end_matches([',', '.', ';', ':', '!', '?', ')']);
    let dots = matches!(word.rsplit('/').next(), Some("." | ".."));
    if cut == word || dots || !shaped(cut) { vec![word] } else { vec![word, cut] }
}

/// The words of a line that may be paths, for the shell to look up: for each, what it may be,
/// most likely first. The first that exists is the path.
pub fn path_words(line: &str) -> Vec<Vec<String>> {
    let candidates = |word: &&str| path_candidates(&unescape(word)).into_iter().map(str::to_string).collect();
    words(line).iter().map(candidates).filter(|list: &Vec<String>| !list.is_empty()).collect()
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
    for word in words(line) {
        // `p1,` is a priority: the comma ends the clause.
        let bare = word.trim_end_matches(clause_end);
        let digit = bare.strip_prefix(['p', 'P']).filter(|_| bare.len() == 2);
        let priority = digit.and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=9).contains(n));
        let plain = unescape(word);
        if let Some(project) = name(word, '#') {
            draft.project = Some(project.to_string());
        } else if let Some(repo) = named(word, '>') {
            draft.repo = Some(repo.to_string());
        } else if let Some(agent) = named(word, '@') {
            draft.agent = Some(agent.to_string());
        } else if let Some(skill) = named(word, '$') {
            once(&mut draft.skills, skill);
        } else if let Some(path) = path_candidates(&plain).into_iter().find(|path| known('/', path)) {
            once(&mut draft.paths, path);
        } else if priority.is_some() {
            draft.priority = priority;
        } else {
            title.push(word);
        }
    }
    draft.title = title.join(" ");
    draft
}

/// The name in `names` that `typed` spells, whatever the case of the letters.
pub fn spelled<'a>(names: &'a [String], typed: &str) -> Option<&'a String> {
    names.iter().find(|name| name.eq_ignore_ascii_case(typed))
}

/// Reads a line against what is known: `repo(name)` says if a repository of that name is
/// listed. Agents and skills come back as they are spelled where they were read, each skill
/// once; each path with where it was found. A name longer than a task may keep (`MAX_TAG`) is
/// no tag: it stays in the title, so nothing typed goes missing.
pub fn read(line: &str, repo: impl Fn(&str) -> bool, known: &Known) -> Draft {
    let found = |typed: &str| known.paths.iter().find(|path| path.typed == typed);
    let fits = |name: &str| name.chars().count() <= MAX_TAG;
    let mut draft = parse(line, |mark, name| {
        fits(name)
            && match mark {
                '>' => repo(name),
                '@' => spelled(&known.names.agents, name).is_some(),
                '$' => spelled(&known.names.skills, name).is_some(),
                _ => found(name).is_some(),
            }
    });
    draft.agent = draft.agent.and_then(|typed| spelled(&known.names.agents, &typed).cloned());
    let mut skills: Vec<String> = Vec::new();
    for skill in draft.skills.iter().filter_map(|typed| spelled(&known.names.skills, typed)) {
        if !skills.contains(skill) {
            skills.push(skill.clone());
        }
    }
    draft.skills = skills;
    draft.resolved = draft.paths.iter().filter_map(|typed| found(typed)).map(|path| path.resolved.clone()).collect();
    draft
}

/// The first of `agent` and `skills` that names nothing known, for a caller that gives them
/// outright (the socket's `task_add`): such a task is refused, as `task_new` leaves the word in the title.
pub fn unknown<'a>(names: &Names, agent: Option<&'a str>, skills: &'a [String]) -> Option<&'a str> {
    let agent = agent.filter(|agent| spelled(&names.agents, agent).is_none());
    agent.or_else(|| skills.iter().map(String::as_str).find(|skill| spelled(&names.skills, skill).is_none()))
}

/// The tag being typed at the end of the line, as (mark, what is typed of the name).
/// `fix >ap` → `('>', "ap")`. A path is all of its word, without its backslashes:
/// `see ~/My\ No` → `('/', "~/My No")`. `None` after a space, when the last word is ordinary,
/// or when a name does not start with a letter: `costs $5` is no skill.
pub fn completing(line: &str) -> Option<(char, String)> {
    let word = typing(line)?;
    if word.starts_with('/') || word.starts_with("~/") {
        return Some(('/', unescape(word)));
    }
    let mark = word.chars().next().filter(|c| matches!(c, '#' | '>' | '@' | '$'))?;
    let typed = &word[1..];
    typed.chars().next().is_none_or(char::is_alphabetic).then(|| (mark, typed.to_string()))
}

/// The word being typed, as typed: the last one, if no space follows it.
fn typing(line: &str) -> Option<&str> {
    let word = words(line).pop()?;
    line.ends_with(word).then_some(word)
}

/// Names that complete what is typed: those that start with it, then those that contain it,
/// without regard to case. At most `MAX_SUGGESTIONS`. Only names a tag can have: typed after
/// its mark, `My Project`, `2048` or `app.` would not read as that name.
pub fn suggest<'a>(typed: &str, names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let typed = typed.to_lowercase();
    let taggable = |n: &str| {
        let tag = format!("#{n}");
        words(&tag) == [tag.as_str()] && name(&tag, '#') == Some(n)
    };
    let fits = |n: &&str| taggable(n) && n.to_lowercase().contains(&typed);
    let mut names: Vec<&str> = names.into_iter().filter(fits).collect();
    names.sort_by_key(|n| (!n.to_lowercase().starts_with(&typed), n.to_lowercase()));
    names.dedup();
    names.into_iter().take(MAX_SUGGESTIONS).map(str::to_string).collect()
}

/// The line with the word being typed replaced by `name`, and a space to go on typing. No space
/// after a directory (`/src/`): the path goes on. A space in a path gets a backslash, so the
/// path stays one word: `~/Library/Application\ Support/`.
pub fn complete(line: &str, name: &str) -> String {
    let (word, name) = completion(line, name);
    format!("{}{name}{}", &line[..word.start], &line[word.end..])
}

/// As `complete`, for a caller that edits in place: the bytes of the word being typed at the
/// end of `line`, and what replaces them.
pub fn completion(line: &str, name: &str) -> (std::ops::Range<usize>, String) {
    let (Some(word), Some((mark, _))) = (typing(line), completing(line)) else {
        return (line.len()..line.len(), String::new());
    };
    let name = match mark {
        '/' if name.ends_with('/') => escape(name),
        '/' => format!("{} ", escape(name)),
        _ => format!("{mark}{name} "),
    };
    (line.len() - word.len()..line.len(), name)
}

/// As `completion`, for a cursor anywhere in `line`: the whole word under the cursor is
/// replaced, so `fix >a|pp` becomes `fix >app `, not `fix >app pp`. No second space where one
/// follows the word already.
pub fn completion_at(line: &str, cursor: usize, name: &str) -> (std::ops::Range<usize>, String) {
    let (word, mut name) = completion(&line[..cursor], name);
    if word.is_empty() {
        return (word, name);
    }
    let mut escaped = line[..cursor].ends_with('\\');
    let rest = line[cursor..].char_indices().find(|(_, c)| {
        let ends = c.is_whitespace() && !(escaped && *c == ' ');
        escaped = *c == '\\' && !escaped;
        ends
    });
    let end = rest.map_or(line.len(), |(at, _)| cursor + at);
    if end < line.len() && name.ends_with(' ') {
        name.pop();
    }
    (word.start..end, name)
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
        // A comma after a priority ends the clause, as after a name.
        let draft = parse("fix p1, now", known);
        assert_eq!((draft.title.as_str(), draft.priority), ("fix now", Some(1)));
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
        // The word as typed first: a name may end in a dot. Then without the clause's punctuation.
        assert_eq!(
            path_words("see /src/export, ~/notes and a/b or / or ~x"),
            [vec!["/src/export,", "/src/export"], vec!["~/notes"]]
        );
        assert_eq!(path_candidates("/src/v1."), ["/src/v1.", "/src/v1"]);
        // A `.` or `..` at the end is a place, not punctuation.
        assert_eq!((path_candidates("/src/.."), path_candidates("~/.")), (vec!["/src/.."], vec!["~/."]));
        assert_eq!(path_candidates("/."), ["/."], "cut, it would be no path");
        // The file that is there wins over the shorter name.
        let draft = parse("read /notes/v1. and /notes/v2.", |_, name| ["/notes/v1.", "/notes/v2"].contains(&name));
        assert_eq!(
            (draft.title.as_str(), draft.paths),
            ("read and", vec!["/notes/v1.".to_string(), "/notes/v2".into()])
        );
    }

    #[test]
    fn a_backslash_keeps_a_space_in_a_word() {
        assert_eq!(words(r"see ~/My\ Notes/a\ b.md  now"), ["see", r"~/My\ Notes/a\ b.md", "now"]);
        assert_eq!(words(r"a\\ b \x\ "), [r"a\\", "b", r"\x\ "], "a backslash before a backslash keeps no space");
        assert_eq!(path_words(r"see ~/My\ Notes, then"), [vec!["~/My Notes,", "~/My Notes"]]);
        // The path is kept without the backslash; a word that is no tag stays as typed.
        let draft = parse(r"file ~/My\ Notes under Big\ Ideas", |_, name| name == "~/My Notes");
        assert_eq!((draft.title.as_str(), draft.paths), (r"file under Big\ Ideas", vec!["~/My Notes".to_string()]));
        // Tab writes the backslash, and the next word typed is the path going on.
        assert_eq!(completing(r"see ~/Library/App"), Some(('/', "~/Library/App".into())));
        let line = complete("see ~/Library/App", "~/Library/Application Support/");
        assert_eq!(line, r"see ~/Library/Application\ Support/");
        assert_eq!(completing(&(line.clone() + "Co")), Some(('/', "~/Library/Application Support/Co".into())));
        assert_eq!(
            complete(&(line + "Co"), "~/Library/Application Support/Code/"),
            r"see ~/Library/Application\ Support/Code/"
        );
        assert_eq!(completing(r"see ~/My\ Notes "), None);
    }

    #[test]
    fn a_line_is_read_against_what_is_known() {
        let names = |list: &[&str]| list.iter().map(|name| name.to_string()).collect::<Vec<_>>();
        let found = |typed: &str, resolved: &str| PathFound { typed: typed.into(), resolved: resolved.into() };
        let long = format!("/{}", "x".repeat(MAX_TAG));
        let known = Known {
            names: Names { agents: names(&["claude", "codex"]), skills: names(&["code-review", "deploy"]) },
            paths: vec![found("/src/export", "/code/app/src/export"), found(&long, &long)],
            entries: Vec::new(),
        };
        let line = format!("ship >app @Codex $Deploy $deploy $DEPLOY /src/export {long} >web @ana $nope");
        let draft = read(&line, |name| name == "app", &known);
        // Spelled as they were read, each skill once; the path with where it was found.
        assert_eq!(
            (draft.repo.as_deref(), draft.agent.as_deref(), draft.skills),
            (Some("app"), Some("codex"), names(&["deploy"]))
        );
        assert_eq!((draft.paths, draft.resolved), (names(&["/src/export"]), names(&["/code/app/src/export"])));
        // A path too long to keep is no tag: it stays in the title with the names nobody knows.
        assert_eq!(draft.title, format!("ship {long} >web @ana $nope"));
        // Names given outright: the first that is not known.
        let skills = names(&["Deploy", "nope"]);
        assert_eq!(unknown(&known.names, Some("CLAUDE"), &skills), Some("nope"));
        assert_eq!(unknown(&known.names, Some("ana"), &skills), Some("ana"));
        assert_eq!(unknown(&known.names, None, &skills[..1]), None);
    }

    #[test]
    fn the_word_being_typed_is_completed() {
        let typed = |mark: char, name: &str| Some((mark, name.to_string()));
        assert_eq!(completing("fix >ap"), typed('>', "ap"));
        assert_eq!(
            (completing("fix #"), completing("ask @cl"), completing("with $de")),
            (typed('#', ""), typed('@', "cl"), typed('$', "de"))
        );
        assert_eq!(
            (completing("see /src/ex"), completing("see ~/no")),
            (Some(('/', "/src/ex".into())), Some(('/', "~/no".into())))
        );
        // A name starts with a letter: an amount of money is no skill, an issue number no project.
        assert_eq!((completing("costs $5"), completing("see #12"), completing("use >3")), (None, None, None));
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
        // In place, the word under the cursor goes whole, with the spaces it keeps.
        assert_eq!(completion_at("fix >app", 6, "app"), (4..8, ">app ".to_string()));
        assert_eq!(completion_at("fix >app now", 6, "apollo"), (4..8, ">apollo".to_string()));
        assert_eq!(completion_at(r"see ~/My\ Notes now", 8, "~/My Notes/"), (4..15, r"~/My\ Notes/".to_string()));
        assert_eq!(completion_at("fix it", 6, "app"), (6..6, String::new()));
    }

    /// A draft as a line that reads as it: the tags, then the title. Tabs between them: a space
    /// after a word that ends in a backslash would join it to the next.
    fn rebuild(draft: &Draft) -> String {
        let tag = |mark: char, name: &Option<String>| name.as_ref().map(|name| format!("{mark}{name}"));
        let skills = draft.skills.iter().map(|skill| format!("${skill}"));
        let paths = draft.paths.iter().map(|path| escape(path));
        let priority = draft.priority.map(|n| format!("p{n}"));
        let title = Some(draft.title.clone()).filter(|title| !title.is_empty());
        let one = [tag('#', &draft.project), tag('>', &draft.repo), tag('@', &draft.agent)].into_iter().flatten();
        one.chain(skills).chain(paths).chain(priority).chain(title).collect::<Vec<_>>().join("\t")
    }

    /// What must hold of any line, with names of any kind known. `Err`: what does not.
    fn sound(line: &str, names: &[String], known: &dyn Fn(char, &str) -> bool) -> Result<(), String> {
        let check = |holds: bool, why: String| if holds { Ok(()) } else { Err(format!("{line:?}: {why}")) };
        let typed = words(line);
        let draft = parse(line, known);
        // A tag found is a word of the line, from its start: nothing is read out of the middle of one.
        let one = [('#', &draft.project), ('>', &draft.repo), ('@', &draft.agent)];
        let one = one.into_iter().filter_map(|(mark, name)| Some(format!("{mark}{}", name.as_ref()?)));
        let many =
            draft.skills.iter().map(|skill| format!("${skill}")).chain(draft.paths.iter().map(|path| escape(path)));
        for tag in one.chain(many) {
            check(typed.iter().any(|word| word.starts_with(&tag)), format!("the tag {tag:?} is no word of the line"))?;
        }
        let priority = |word: &&str| draft.priority.is_some_and(|n| word.to_lowercase().starts_with(&format!("p{n}")));
        check(draft.priority.is_none() || typed.iter().any(priority), "the priority is no word of the line".into())?;
        // The title is the other words, in their order.
        let mut left = typed.iter();
        let kept = words(&draft.title).len() == draft.title.split(' ').count() || draft.title.is_empty();
        let in_order = draft.title.split(' ').all(|word| word.is_empty() || left.any(|typed| *typed == word));
        check(!kept || in_order, format!("the title {:?} is not words of the line, in order", draft.title))?;
        // Written out again, the draft reads as itself.
        let (again, anew) = (rebuild(&draft), parse(&rebuild(&draft), known));
        check(anew == draft, format!("{draft:?} written as {again:?} reads as {anew:?}"))?;
        // `read` finds no tag `parse` would not, and never panics on what it is told is known.
        let lists = Known {
            names: Names { agents: names.to_vec(), skills: names.to_vec() },
            paths: names.iter().map(|name| PathFound { typed: name.clone(), resolved: format!("/r{name}") }).collect(),
            entries: Vec::new(),
        };
        let drafted = read(line, |name| known('>', name), &lists);
        check(drafted.resolved.len() == drafted.paths.len(), "a path was read and not found".into())?;
        check(drafted.skills.len() <= draft_all(line).skills.len(), "a skill came from nowhere".into())?;
        let _ = path_words(line);
        // Every name offered for the word being typed finishes it as that tag, wherever the cursor is.
        for cursor in (0..=line.len()).filter(|at| line.is_char_boundary(*at)) {
            let Some((mark, start)) = completing(&line[..cursor]) else { continue };
            let offered = if mark == '/' { names.to_vec() } else { suggest(&start, names.iter().map(String::as_str)) };
            // A path the shell lists is path-shaped. KNOWN (see the ignored test below): not one
            // with a backslash, a tab or a line break in a name, which `escape` does not keep.
            let typable = |name: &&String| !name.chars().any(|c| c == '\\' || (c.is_whitespace() && c != ' '));
            let listed = |name: &&String| mark != '/' || (!path_candidates(name).is_empty() && typable(name));
            for name in offered.iter().filter(listed) {
                let (word, text) = completion_at(line, cursor, name);
                let done = format!("{}{text}{}", &line[..word.start], &line[word.end..]);
                // Up to the finished word: of two `@agent` the last counts, and one may follow.
                let draft = draft_all(&done[..word.start + text.len()]);
                let found = match mark {
                    '#' => draft.project.as_ref() == Some(name),
                    '>' => draft.repo.as_ref() == Some(name),
                    '@' => draft.agent.as_ref() == Some(name),
                    '$' => draft.skills.contains(name),
                    _ => draft.paths.contains(name),
                };
                check(found, format!("at {cursor}, {name:?} completes to {done:?}, which reads as {draft:?}"))?;
            }
            if cursor == line.len() {
                check(
                    completion_at(line, cursor, "x") == completion(line, "x"),
                    "the end of the line is not the end".into(),
                )?;
            }
        }
        Ok(())
    }

    /// The line with every name known.
    fn draft_all(line: &str) -> Draft {
        parse(line, |_, _| true)
    }

    /// Found by the test below. `escape` puts a backslash before a space and nothing else, so a
    /// path whose name holds a backslash before a space, a tab or a line break is offered and
    /// then reads as another path, or as two words.
    #[test]
    #[ignore = "a decision: how a path with a backslash, a tab or a line break in a name is typed"]
    fn a_path_with_a_backslash_or_a_tab_in_a_name_completes_to_itself() {
        for name in ["~/a\\ b", "/src/a\tb", "/src/a\nb"] {
            let done = complete("see /", name);
            assert_eq!(draft_all(&done).paths, [name], "{done:?}");
        }
    }

    /// Random lines of hostile pieces, tags and words. A failure prints the least pieces that
    /// still fail, as one line.
    #[test]
    fn any_line_reads_the_same_again_and_completes_to_its_tag() {
        use crate::fuzz::{self, HOSTILE, Rng};
        const WORDS: [&str; 16] = [
            "fix",
            "zoom",
            "#canvas",
            ">app",
            "@claude",
            "$deploy",
            "/src/export",
            "~/My\\ Notes/",
            "p1",
            "P9,",
            "p0",
            "#",
            ">ap",
            "@Claude",
            "/",
            "$5",
        ];
        const NAMES: [&str; 12] = [
            "app",
            "apollo",
            "Claude",
            "code-review",
            "My Project",
            "2048",
            "app.",
            "a,b",
            "été",
            "/src/My Dir/",
            "~/notes.md",
            "",
        ];
        let one = |seed: u64| {
            let mut rng = Rng::new(seed);
            let piece = |rng: &mut Rng| match rng.below(4) {
                0 => rng.pick(HOSTILE).to_string(),
                1 => rng.pick(&[" ", " ", "  ", "\t", ""]).to_string(),
                _ => rng.pick(&WORDS).to_string(),
            };
            let pieces: Vec<String> = (0..rng.below(12)).map(|_| piece(&mut rng)).collect();
            let hostile = |rng: &mut Rng| rng.pick(HOSTILE).to_string() + rng.pick(HOSTILE);
            let names: Vec<String> =
                NAMES.iter().map(|name| name.to_string()).chain((0..3).map(|_| hostile(&mut rng))).collect();
            // Every name known, none, or the names of even length.
            let which = seed % 3;
            let known = move |_: char, name: &str| which == 0 || (which == 2 && name.chars().count().is_multiple_of(2));
            let check =
                |pieces: &[String]| fuzz::caught(|| sound(&pieces.concat(), &names, &known)).unwrap_or_else(Err);
            check(&pieces).is_err().then(|| {
                let least = fuzz::shrink(pieces, |pieces| check(pieces).is_err());
                format!("seed {seed}, names {names:?}: {}", check(&least).err().unwrap_or_default())
            })
        };
        if let Some(report) = fuzz::each_seed(0, 20_000, one) {
            panic!("{report}");
        }
    }
}
