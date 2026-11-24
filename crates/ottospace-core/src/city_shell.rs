//! Which of a checkout's files and folders a shell line names, for the code city: best-effort
//! and bounded, read as words only (never expanded, never run). What a pipeline really reads
//! (`xargs`, a glob, a script) cannot be known. E.g. in `/code/app`:
//! `cd /code/app && cat src/main.rs | wc -l` → `["src/main.rs"]`.

/// The most of a line read, and the most paths given.
const MOST_BYTES: usize = 2048;
const MOST_PATHS: usize = 8;

/// The paths a shell `line` names inside `checkout` (absolute), relative to it, in order, each
/// once. The line runs in the checkout until a `cd` says otherwise; a `cd` out of it, or to a
/// place not known, stops relative words counting until a `cd` back.
pub fn paths(line: &str, checkout: &str) -> Vec<String> {
    let checkout = checkout.trim_end_matches('/');
    let mut end = line.len().min(MOST_BYTES);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    // Where the line is now, relative to the checkout ("": its root); none: not known.
    let mut cwd = Some(String::new());
    let mut found: Vec<String> = Vec::new();
    for command in line[..end].split(['\n', ';', '|', '&']) {
        // A quoted word whole (`"src/main.rs"`); a piece of one with a space in it (`'docs/a`) is
        // no path.
        let mut words = command.split_whitespace().map(|word| {
            let quoted = |q: char| word.len() > 1 && word.starts_with(q) && word.ends_with(q);
            match word.trim_matches(['"', '\'']) {
                bare if bare.len() == word.len() || quoted('"') || quoted('\'') => bare,
                _ => "",
            }
        });
        let Some(program) = words.next() else { continue };
        if program == "cd" {
            cwd = words.next().and_then(|dir| inside(dir, checkout, cwd.as_deref()));
            continue;
        }
        for word in words {
            let Some(path) = inside(word, checkout, cwd.as_deref()).filter(|path| !path.is_empty()) else { continue };
            // A bare word (`test`, `main`) is a subcommand or a name, not a path.
            let shaped = word.starts_with('/') || path.contains(['/', '.']);
            if shaped && !found.contains(&path) {
                found.push(path);
                if found.len() == MOST_PATHS {
                    return found;
                }
            }
        }
    }
    found
}

/// `word` as a path in the checkout, relative to it, from `cwd`; none if it is not one (a flag,
/// a variable, a glob, a `..`, a URL) or lies outside.
fn inside(word: &str, checkout: &str, cwd: Option<&str>) -> Option<String> {
    let odd = |c: char| "$`*?[]{}~=<>()\\".contains(c) || c.is_control();
    if word.is_empty() || word.len() > 256 || word.starts_with('-') || word.contains("://") || word.contains(odd) {
        return None;
    }
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
            ("cat \"src/main.rs\" 'docs/a b.md'", vec!["src/main.rs"]),
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
        // Cut inside a character: no panic.
        assert_eq!(paths(&"é".repeat(MOST_BYTES), APP), Vec::<String>::new());
    }
}
