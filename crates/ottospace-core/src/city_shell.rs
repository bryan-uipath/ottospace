//! Which of a checkout's files and folders a shell line names, for the code city: best-effort
//! and bounded, read as words only (never expanded, never run). What a pipeline really reads
//! (`xargs`, a glob, a script) cannot be known. E.g. in `/code/app`:
//! `cd /code/app && cat src/main.rs | wc -l` → `["src/main.rs"]`.

/// The most of a line read, and the most paths given.
const MOST_BYTES: usize = 2048;
const MOST_PATHS: usize = 8;

/// The paths a shell `line` names inside `checkout` (absolute), relative to it, in order, each
/// once. The line runs in the checkout until a `cd` says otherwise (one followed by `&&`, `;` or
/// a new line: a `cd` in a pipeline stays in its stage); a `cd` out of it, or to a place not
/// known, stops relative words counting until a `cd` back.
pub fn paths(line: &str, checkout: &str) -> Vec<String> {
    paths_from(line, checkout, "")
}

/// `paths`, for a line run in `from` (relative to the checkout; "": its root), as a session
/// started in a folder below the checkout runs its lines.
pub fn paths_from(line: &str, checkout: &str, from: &str) -> Vec<String> {
    let checkout = checkout.trim_end_matches('/');
    // Where the line is now, relative to the checkout ("": its root); none: not known.
    let mut cwd = Some(from.to_string());
    let mut found: Vec<String> = Vec::new();
    for (words, then) in commands(line) {
        let Some((program, args)) = words.split_first() else { continue };
        if program == "cd" {
            if then {
                cwd = args.first().and_then(|dir| inside(dir, checkout, cwd.as_deref()));
            }
            continue;
        }
        for word in args {
            let Some(path) = inside(word, checkout, cwd.as_deref()).filter(|path| !path.is_empty()) else { continue };
            // A bare word (`test`, `main`) is a subcommand or a name, not a path.
            if word.contains(['/', '.']) && !found.contains(&path) {
                found.push(path);
                if found.len() == MOST_PATHS {
                    return found;
                }
            }
        }
    }
    found
}

/// A line's commands, read to `MOST_BYTES` (a word cut there is dropped): each its words, quotes
/// taken off (a quoted word may hold spaces and separators), and whether the next runs after it
/// (`&&`, `;`, a new line) rather than beside it (`|`, `&`). A `#` outside quotes, at a word's
/// start, comments out the rest of its line. A `\` keeps the next character as it is.
pub(crate) fn commands(line: &str) -> Vec<(Vec<String>, bool)> {
    let mut end = line.len().min(MOST_BYTES);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    let cut = end < line.len();
    let mut out: Vec<(Vec<String>, bool)> = Vec::new();
    let (mut words, mut word, mut quote, mut started) = (Vec::new(), String::new(), None, false);
    let mut chars = line[..end].chars().peekable();
    let mut comment = false;
    while let Some(c) = chars.next() {
        if comment {
            comment = c != '\n';
            if comment {
                continue;
            }
        }
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), '\\') if quote == Some('"') => word.extend(chars.next()),
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => (quote, started) = (Some(c), true),
            (None, '\\') => {
                word.extend(chars.next());
                started = true;
            }
            (None, '#') if !started => comment = true,
            (None, c) if c.is_whitespace() || matches!(c, ';' | '|' | '&') => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
                let then = match c {
                    ';' | '\n' => Some(true),
                    '&' if chars.peek() == Some(&'&') => {
                        chars.next();
                        Some(true)
                    }
                    '|' if chars.peek() == Some(&'|') => {
                        chars.next();
                        Some(true)
                    }
                    '|' | '&' => Some(false),
                    _ => None,
                };
                if let Some(then) = then {
                    out.push((std::mem::take(&mut words), then));
                }
            }
            (None, c) => {
                word.push(c);
                started = true;
            }
        }
    }
    // A word the cut went through (or an open quote) is no word.
    if started && !cut && quote.is_none() {
        words.push(word);
    }
    out.push((words, true));
    out
}

/// A literal path (a tool's argument, a result's line: no shell syntax in it) in `checkout`,
/// relative to it ("": the checkout itself); a relative one from `base` (relative to the
/// checkout). None outside it, with a `..`, or a URL.
pub fn path(word: &str, checkout: &str, base: &str) -> Option<String> {
    if word.is_empty() || word.len() > 1024 || word.contains("://") || word.contains(char::is_control) {
        return None;
    }
    let (word, checkout) = (plain(word), plain(checkout.trim_end_matches('/')));
    match word.strip_prefix(checkout) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => join("", rest),
        _ if word.starts_with('/') => None,
        _ => join(base, word),
    }
}

/// macOS's `/tmp` and `/var` are `/private/tmp` and `/private/var`: either names the same place.
fn plain(path: &str) -> &str {
    let bare = path.strip_prefix("/private").filter(|rest| rest.starts_with("/tmp/") || rest.starts_with("/var/"));
    bare.unwrap_or(path)
}

/// `word` as a path in the checkout, relative to it, from `cwd`; none if it is not one (a flag,
/// a variable, a glob, a `..`, a URL) or lies outside.
fn inside(word: &str, checkout: &str, cwd: Option<&str>) -> Option<String> {
    let odd = |c: char| "$`*?[]{}~=<>()\\".contains(c) || c.is_control();
    if word.is_empty() || word.len() > 256 || word.starts_with('-') || word.contains("://") || word.contains(odd) {
        return None;
    }
    let (word, checkout) = (plain(word), plain(checkout));
    let rel = match word.strip_prefix(checkout) {
        Some("") => "",
        Some(rest) => rest.strip_prefix('/')?,
        None if word.starts_with('/') => return None,
        None => return join(cwd?, word),
    };
    join("", rel)
}

/// `base`/`rel`, without `.` or empty parts; none with a `..`.
fn join(base: &str, rel: &str) -> Option<String> {
    let parts: Vec<&str> =
        base.split('/').chain(rel.split('/')).filter(|part| !part.is_empty() && *part != ".").collect();
    (!parts.contains(&"..")).then(|| parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "/code/app";

    #[test]
    fn a_line_names_the_checkouts_paths_after_its_cd() {
        let cases = [
            ("cd /code/app && cat src/main.rs | wc -l", vec!["src/main.rs"]),
            ("cd /code/app && rg loop src/model docs/a.md", vec!["src/model", "docs/a.md"]),
            ("ls /code/app/src", vec!["src"]),
            ("cd src && cat main.rs", vec!["src/main.rs"]),
            ("cd ./src/ ; cat ./model/../x.rs; cat lib.rs", vec!["src/lib.rs"]),
            ("cat ./README.md Cargo.toml", vec!["README.md", "Cargo.toml"]),
            ("cargo test -p core && git diff main", vec![]),
            ("cd /elsewhere && cat src/main.rs; cd /code/app && cat a.rs", vec!["a.rs"]),
            ("cd && cat src/main.rs", vec![]),
            ("cat /code/application/x.rs /etc/hosts ../up.rs", vec![]),
            ("cat $HOME/x.rs src/*.rs ~/a.rs `pwd`/b.rs https://x.example/a.rs", vec![]),
            ("cat \"src/main.rs\" 'docs/a b.md'", vec!["src/main.rs", "docs/a b.md"]),
            ("ls src/ ./docs; cat ./Makefile", vec!["src", "docs", "Makefile"]),
            ("cd sdk && npm test", vec![]),
            ("echo 'x; cd docs ;' && cat src/main.rs", vec!["src/main.rs"]),
            ("cd docs | cat src/main.rs", vec!["src/main.rs"]),
            ("cargo test # src/a.rs", vec![]),
            ("git commit -m \"fix\" && cat a\\ b.rs", vec!["a b.rs"]),
            ("cargo build 2>&1 > out.log", vec!["out.log"]),
            ("cat src/a.rs src/a.rs /code/app/src/a.rs", vec!["src/a.rs"]),
        ];
        for (line, want) in cases {
            assert_eq!(paths(line, APP), want, "{line}");
        }
    }

    #[test]
    fn a_long_line_is_read_only_so_far() {
        let many: Vec<String> = (0..20).map(|n| format!("f{n}.rs")).collect();
        assert_eq!(paths(&format!("cat {}", many.join(" ")), APP), many[..MOST_PATHS]);
        let late = format!("echo {} src/late.rs", "x".repeat(MOST_BYTES));
        assert_eq!(paths(&late, APP), Vec::<String>::new());
        // Cut inside a word: the word is dropped, never read as a shorter path.
        let through = format!("cat{}src/a.rs", " ".repeat(MOST_BYTES - 8));
        assert_eq!(paths(&through, APP), Vec::<String>::new());
        // A checkout with a space, quoted.
        assert_eq!(paths("cd \"/code/my app\" && cat src/a.rs", "/code/my app"), ["src/a.rs"]);
        // `/private/tmp` is `/tmp`.
        assert_eq!(paths("cat /private/tmp/r/a.rs /tmp/r/b.rs", "/tmp/r"), ["a.rs", "b.rs"]);
        assert_eq!(path("/tmp/r/c.rs", "/private/tmp/r", "").as_deref(), Some("c.rs"));
        // A literal path keeps what a shell word may not: `[id].tsx`.
        assert_eq!(path("/code/app/src/[id].tsx", APP, "").as_deref(), Some("src/[id].tsx"));
        assert_eq!(path("mod.rs", APP, "src/ui").as_deref(), Some("src/ui/mod.rs"));
        assert_eq!(path("/etc/hosts", APP, ""), None);
        // Cut inside a character: no panic.
        assert_eq!(paths(&"é".repeat(MOST_BYTES), APP), Vec::<String>::new());
    }
}
