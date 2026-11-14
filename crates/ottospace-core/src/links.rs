//! Which link targets in a conversation may open, and as what. The one judge: a markdown link
//! the model wrote is drawn as a link only if `target` accepts it, and the shell opens nothing
//! else. Web pages (`http`, `https`) open in the default browser; a path inside the task's
//! repository is revealed in Finder. Everything else is drawn as text. Pure: the shell resolves
//! links on disk and checks again before it reveals a file.

/// What a click on a link does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Opened in the default browser: the link as written, `http://` or `https://`.
    Web(String),
    /// Revealed in Finder: an absolute path, `..` and `.` resolved, inside the repository.
    File(String),
}

/// The longest link target judged.
const MAX_LINK: usize = 2048;

/// `raw` (a link's destination as the model wrote it) as something to open, or `None`: shown as
/// text. `repo`: the task's repository, the one folder whose files may be revealed.
pub fn target(raw: &str, repo: Option<&str>) -> Option<Target> {
    if raw.is_empty() || raw.len() > MAX_LINK {
        return None;
    }
    // Judged as written and as a browser would read it: `%0a`, `%6aavascript:` are caught too.
    let decoded = decode(raw)?;
    if !plain(raw) || !plain(&decoded) {
        return None;
    }
    // Only the schemes that may open are read as one: `main.rs:12` is a file and a line, and
    // any other scheme (`javascript:`, `data:`) keeps its `:` and is no file either.
    let named =
        scheme(&decoded).filter(|(name, _)| matches!(name.to_ascii_lowercase().as_str(), "http" | "https" | "file"));
    match named {
        Some((name, rest)) => match name.to_ascii_lowercase().as_str() {
            "http" | "https" => {
                // Written out, not encoded: the scheme as typed must be the one judged.
                // So must the host, decoded or not.
                let written = scheme(raw).is_some_and(|(said, typed)| said.eq_ignore_ascii_case(name) && host(typed));
                (written && host(rest)).then(|| Target::Web(raw.to_string()))
            }
            // `file:///path` only: `file://host/…` names another machine.
            "file" => file(rest.strip_prefix("//").filter(|path| path.starts_with('/'))?, repo),
            _ => None,
        },
        // `//host/x` is another site to a browser: never a path.
        None if decoded.starts_with("//") => None,
        None => file(&decoded, repo),
    }
}

/// No control character, space, or mark that hides or reorders text (`capture::shows`).
fn plain(text: &str) -> bool {
    text.chars().all(|c| crate::capture::shows(c) && !c.is_whitespace())
}

/// Percent-decoded, once. `None`: not UTF-8 once decoded.
fn decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[at], bytes.get(at + 1).copied().and_then(hex), bytes.get(at + 2).copied().and_then(hex)) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                at += 3;
            }
            (byte, _, _) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// A URI's scheme and what follows its `:`, by RFC 3986's grammar. `None`: no scheme.
fn scheme(text: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = text.split_once(':')?;
    let mut chars = scheme.chars();
    let first = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    (first && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))).then_some((scheme, rest))
}

/// `//host[:port]` then a path, query or fragment: a plain host, no user name (`good@evil`).
fn host(rest: &str) -> bool {
    let Some(rest) = rest.strip_prefix("//") else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let name = authority.rsplit_once(':').map_or(authority, |(name, _)| name);
    !name.is_empty()
        && authority.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']' | '_'))
}

/// A path inside `repo`, resolved without the disk: relative to it, `.` and `..` taken out, a
/// line suffix (`:12`, `:12:3`) and a fragment (`#L12`) dropped.
fn file(path: &str, repo: Option<&str>) -> Option<Target> {
    let repo = normal(repo?)?;
    let path = path.split(['#', '?']).next().unwrap_or("");
    let mut path = path;
    for _ in 0..2 {
        if let Some((before, line)) = path.rsplit_once(':')
            && !line.is_empty()
            && line.chars().all(|c| c.is_ascii_digit())
        {
            path = before;
        }
    }
    if path.is_empty() || path.contains(':') || path.contains('\\') {
        return None;
    }
    let whole = if path.starts_with('/') { normal(path)? } else { normal(&format!("{repo}/{path}"))? };
    let inside = whole == repo || whole.strip_prefix(&repo).is_some_and(|rest| rest.starts_with('/')) || repo == "/";
    inside.then_some(Target::File(whole))
}

/// An absolute path with `.`, `..` and repeated `/` taken out. `None`: not absolute, or `..`
/// above the root.
fn normal(path: &str) -> Option<String> {
    if !path.starts_with('/') {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => drop(parts.pop()?),
            part => parts.push(part),
        }
    }
    Some(format!("/{}", parts.join("/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: Option<&str> = Some("/w/app");

    fn web(url: &str) -> Option<Target> {
        Some(Target::Web(url.into()))
    }

    fn file_at(path: &str) -> Option<Target> {
        Some(Target::File(path.into()))
    }

    #[test]
    fn web_pages_open_and_nothing_else_with_a_scheme_does() {
        let cases = [
            ("https://example.com/docs?a=1#b", web("https://example.com/docs?a=1#b")),
            ("HTTP://Example.com", web("HTTP://Example.com")),
            ("http://[::1]:8080/x", web("http://[::1]:8080/x")),
            ("javascript:alert(1)", None),
            ("JavaScript://%0aalert()", None),
            ("%6aavascript:alert(1)", None),
            ("data:text/html,<b>x</b>", None),
            ("mailto:a@b.c", None),
            ("vbscript:x", None),
            ("ftp://example.com", None),
            ("https://good.com@evil.com/", None),
            ("https:evil.com", None),
            ("https:///x", None),
            ("//evil.com/x", None),
            ("%2F%2Fevil.com", None),
            ("https://exa mple.com", None),
            ("https://example.com/\u{202e}fdp", None),
            ("https://example.com/%0d%0aX", None),
            ("https://example.com/a\u{061c}b", None),
            ("https://example.com/a%D8%9Cb", None),
            ("https://example.com/a\u{2060}b", None),
            ("ht%74ps://example.com", None),
            ("https://example.com\\@evil.com", None),
            ("", None),
        ];
        for (raw, want) in cases {
            assert_eq!(target(raw, REPO), want, "{raw:?}");
        }
        assert_eq!(target(&format!("https://e.com/{}", "a".repeat(MAX_LINK)), REPO), None);
    }

    #[test]
    fn paths_open_only_inside_the_repository() {
        let cases = [
            ("src/main.rs", file_at("/w/app/src/main.rs")),
            ("./src/main.rs:12", file_at("/w/app/src/main.rs")),
            ("src/main.rs:12:3", file_at("/w/app/src/main.rs")),
            ("src/lib.rs#L4", file_at("/w/app/src/lib.rs")),
            ("/w/app/README.md", file_at("/w/app/README.md")),
            ("file:///w/app/a.png", file_at("/w/app/a.png")),
            ("src/a%20b.rs", None),
            ("src/a%2e%2e/b", file_at("/w/app/src/a../b")),
            ("../other/secret", None),
            ("src/../../other", None),
            ("%2e%2e/other", None),
            ("/w/application/x", None),
            ("/etc/passwd", None),
            ("file:///etc/passwd", None),
            ("file://host/w/app/x", None),
            ("C:\\x", None),
            ("~/.ssh/id_rsa", file_at("/w/app/~/.ssh/id_rsa")),
            ("main.rs:12", file_at("/w/app/main.rs")),
            ("README.md:4:2", file_at("/w/app/README.md")),
            ("javascript:alert(1)", None),
            ("mailto:a@b.c", None),
        ];
        for (raw, want) in cases {
            assert_eq!(target(raw, REPO), want, "{raw:?}");
        }
        // No repository: no path opens.
        assert_eq!(target("src/main.rs", None), None);
        assert_eq!(target("/w/app/a", Some("relative")), None);
    }

    /// Whatever the text, a target is a plain http(s) URL or a path inside the repository.
    #[test]
    fn any_text_gives_only_safe_targets() {
        const PIECES: &[&str] = &[
            "https://",
            "http://",
            "HTTPS:",
            "javascript:",
            "file://",
            "data:",
            "//",
            "/",
            "..",
            ".",
            "%2e",
            "%2F",
            "%0a",
            "%6a",
            "@",
            "\\",
            ":",
            "#",
            "?",
            "w",
            "app",
            "/w/app",
            "evil.com",
            "\n",
            "\u{202e}",
            " ",
            "é",
            "%",
            "%zz",
            "%ff",
        ];
        let mut seed = 11u64;
        let mut next = |n: usize| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize % n
        };
        for _ in 0..20_000 {
            let raw: String = (0..next(7)).map(|_| PIECES[next(PIECES.len())]).collect();
            match target(&raw, REPO) {
                Some(Target::Web(url)) => {
                    let lower = url.to_ascii_lowercase();
                    assert!(lower.starts_with("https://") || lower.starts_with("http://"), "{raw:?}");
                    let authority = url.splitn(4, '/').nth(2).unwrap_or("");
                    assert!(plain(&url) && !authority.contains('@') && host(&url[url.find(':').unwrap_or(0) + 1..]));
                    assert_eq!(url, raw);
                }
                Some(Target::File(path)) => {
                    assert!(path == "/w/app" || path.starts_with("/w/app/"), "{raw:?} → {path:?}");
                    assert!(path.split('/').all(|part| part != ".." && part != "."), "{raw:?} → {path:?}");
                    assert!(plain(&path), "{raw:?}");
                }
                None => {}
            }
        }
    }
}
