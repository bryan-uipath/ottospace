//! Which of a checkout's files the code city leaves out: those under a folder the rule names,
//! dropped before the layout, so they cost no room, move nothing and are in no count. A name
//! (`node_modules`) leaves out every folder of that name, at any depth; a path (`notes/archive`)
//! leaves out that one folder of the checkout, from its root. No patterns: a name or a path says
//! plainly what goes. Built in: folders of tools' output and of other people's code
//! (`node_modules`, `target`, `dist`, …), which a checkout's `.gitignore` most often leaves out
//! already; `!target` keeps one of them. Pure: strings in, a decision out, nothing read.

/// Folders left out of every city, unless an entry `!name` keeps one.
pub const BUILT_IN: [&str; 15] = [
    ".git",
    "node_modules",
    "bower_components",
    "jspm_packages",
    "target",
    "dist",
    ".next",
    ".nuxt",
    ".turbo",
    ".gradle",
    ".venv",
    "venv",
    "__pycache__",
    ".tox",
    "Pods",
];

/// Most entries a setting may hold, all checkouts' together.
pub const MOST: usize = 256;

/// A rule: built-in folder names (exact: `target`, not LLVM's `Target`), the user's names
/// (case ignored), each left out anywhere, and the user's paths from the checkout's root (exact).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Exclude {
    built_in: Vec<&'static str>,
    names: Vec<String>,
    paths: Vec<Vec<String>>,
}

/// One entry of `city_exclude`, or why it is none: a folder's name, `!` and a built-in name, or a
/// path inside the checkout (`a/b` or `a/b/`: no `/` first, no `.` or `..`, no empty part). No
/// pattern characters: `*`, `?` and `[` are refused, not matched literally.
fn entry(text: &str) -> Result<Entry, String> {
    let wrong = |why: &str| Err(format!("{text:?} {why}"));
    let odd = |c: char| c.is_control() || matches!(c, '\\' | '*' | '?' | '[' | ']');
    if text.is_empty() || text.trim() != text || text.chars().any(odd) {
        return wrong("is not a folder's name or a path in the checkout (no patterns)");
    }
    if let Some(name) = text.strip_prefix('!') {
        return match BUILT_IN.iter().find(|built| **built == name) {
            Some(built) => Ok(Entry::Keep(built)),
            None => wrong(&format!("keeps none of the built-in folders: {}", BUILT_IN.join(", "))),
        };
    }
    // One `/` at the end is allowed (a folder); any other empty part is not.
    let parts: Vec<&str> = text.strip_suffix('/').unwrap_or(text).split('/').collect();
    if text.starts_with('/') || text.starts_with('~') || parts.iter().any(|p| matches!(*p, "" | "." | "..")) {
        return wrong("is outside the checkout: a path is from its root, as `notes/archive`");
    }
    Ok(match parts.as_slice() {
        [name] => Entry::Name(name.to_lowercase()),
        parts => Entry::Path(parts.iter().map(|p| p.to_string()).collect()),
    })
}

enum Entry {
    Name(String),
    Path(Vec<String>),
    Keep(&'static str),
}

/// Whether each entry is one, or the first wrong one's why.
fn check(entries: &[String]) -> Result<(), String> {
    entries.iter().try_for_each(|text| entry(text).map(drop))
}

impl Exclude {
    /// The built-in folders and `entries`' (wrong ones left out: `check_setting` says why).
    pub fn new(entries: &[String]) -> Exclude {
        let read: Vec<Entry> = entries.iter().filter_map(|text| entry(text).ok()).collect();
        let kept: Vec<&str> =
            read.iter().filter_map(|e| if let Entry::Keep(n) = e { Some(*n) } else { None }).collect();
        let built_in = BUILT_IN.iter().copied().filter(|n| !kept.contains(n)).collect();
        let names = read.iter().filter_map(|e| if let Entry::Name(n) = e { Some(n.clone()) } else { None });
        let paths = read.iter().filter_map(|e| if let Entry::Path(p) = e { Some(p.clone()) } else { None });
        Exclude { built_in, names: names.collect(), paths: paths.collect() }
    }

    /// Whether the file at `file` (its path in the checkout, `a/b/c.rs`) is left out: a folder
    /// above it has a name left out, or is a path left out.
    pub fn excludes(&self, file: &str) -> bool {
        let parts: Vec<&str> = file.split('/').collect();
        let folders = &parts[..parts.len().saturating_sub(1)];
        let named = |f: &&str| self.built_in.contains(f) || self.names.contains(&f.to_lowercase());
        folders.iter().any(named)
            || self.paths.iter().any(|p| folders.len() >= p.len() && folders.iter().zip(p).all(|(f, p)| f == p))
    }

    /// The same rule as git's `--exclude` patterns, for the files git lists that it tracks not
    /// (a big untracked folder costs no read): a name anywhere (`name/`), a path from the root
    /// (`/a/b/`). Entries hold no pattern characters (`entry`).
    pub fn git_patterns(&self) -> Vec<String> {
        let names = self.built_in.iter().map(|n| format!("{n}/")).chain(self.names.iter().map(|n| format!("{n}/")));
        names.chain(self.paths.iter().map(|p| format!("/{}/", p.join("/")))).collect()
    }
}

/// `city_exclude`'s entries for one checkout: every checkout's (`*`) and those under a key
/// `names_it` says is this checkout.
pub fn entries_for(
    setting: &std::collections::BTreeMap<String, Vec<String>>,
    names_it: impl Fn(&str) -> bool,
) -> Vec<String> {
    (setting.iter()).filter(|(key, _)| *key == "*" || names_it(key)).flat_map(|(_, entries)| entries.clone()).collect()
}

/// `city_exclude` as written, or why not: keys `*` or a checkout's path, each a list of entries.
pub fn check_setting(setting: &std::collections::BTreeMap<String, Vec<String>>) -> Result<(), String> {
    if setting.values().map(Vec::len).sum::<usize>() > MOST {
        return Err(format!("at most {MOST} entries"));
    }
    if setting.keys().any(|key| key.trim().is_empty()) {
        return Err("a key is `*` (every checkout) or a checkout's path".into());
    }
    setting.values().try_for_each(|entries| check(entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::{atlas, repo};

    fn rule(entries: &[&str]) -> Exclude {
        Exclude::new(&entries.iter().map(|e| e.to_string()).collect::<Vec<_>>())
    }

    /// A name leaves out its folders at any depth, a path only its own; a file is never a folder
    /// left out by its own name; case is ignored.
    #[test]
    fn a_name_goes_anywhere_and_a_path_from_the_root() {
        let r = rule(&["archive", "notes/old"]);
        assert!(r.excludes("node_modules/a/b.js") && r.excludes("web/node_modules/x.js"));
        assert!(r.excludes("archive/2020.md") && r.excludes("notes/Archive/x.md"));
        // A path is exact; a built-in name too (LLVM's `lib/Target` is source).
        assert!(!r.excludes("Notes/old/a.md") && !r.excludes("lib/Target/x.cpp") && r.excludes("x/target/y"));
        assert!(r.excludes("notes/old/a.md") && r.excludes("notes/old/deep/b.md"));
        assert!(!r.excludes("old/a.md") && !r.excludes("notes/older/a.md") && !r.excludes("x/notes/old/a.md"));
        assert!(!r.excludes("archive") && !r.excludes("src/target.rs") && !r.excludes("README.md"));
        assert!(!rule(&[]).excludes("src/lib.rs"));
    }

    /// `!name` keeps a built-in folder; it keeps no other.
    #[test]
    fn a_built_in_folder_may_be_kept() {
        assert!(rule(&[]).excludes("target/x.rs"));
        assert!(!rule(&["!target"]).excludes("target/x.rs"));
        assert!(rule(&["!target"]).excludes("node_modules/x.js"));
        assert!(check(&["!archive".into()]).unwrap_err().contains("built-in"));
        assert_eq!(rule(&["!target", "notes/old"]).git_patterns().last().map(String::as_str), Some("/notes/old/"));
        assert!(rule(&[]).git_patterns().contains(&"node_modules/".to_string()));
    }

    /// An entry never reaches outside the checkout: no absolute path, no `~`, no `.` or `..`, no
    /// empty part, no backslash, no control character, no padding.
    #[test]
    fn an_entry_is_inside_the_checkout() {
        for bad in [
            "",
            " a",
            "a ",
            "/etc",
            "~/x",
            "../x",
            "a/../b",
            "./a",
            "a//b",
            "a\\b",
            "a\nb",
            ".",
            "archive//",
            "*.log",
            "node_modules/**",
            "a?",
            "[ab]",
        ] {
            assert!(check(&[bad.into()]).is_err(), "{bad:?}");
        }
        for good in ["archive", "notes/archive", "notes/archive/", ".cache", "a b"] {
            assert!(check(&[good.into()]).is_ok(), "{good:?}");
        }
        // A wrong entry is left out of the rule, the others kept.
        assert!(rule(&["../x", "archive"]).excludes("archive/a.md"));
    }

    /// `*`'s entries go to every checkout, a path's to its own.
    #[test]
    fn a_checkouts_entries_are_everyones_and_its_own() {
        let setting: std::collections::BTreeMap<String, Vec<String>> =
            [("*".into(), vec!["archive".into()]), ("~/notes".into(), vec!["old".into()])].into();
        assert_eq!(entries_for(&setting, |key| key == "~/notes"), ["archive", "old"]);
        assert_eq!(entries_for(&setting, |_| false), ["archive"]);
        assert!(check_setting(&setting).is_ok());
        let bad: std::collections::BTreeMap<String, Vec<String>> = [("*".into(), vec!["../x".into()])].into();
        assert!(check_setting(&bad).unwrap_err().contains("outside the checkout"));
    }

    /// What stays is laid out as if the folders left out were never there: the same plan.
    #[test]
    fn the_plan_of_what_stays_does_not_depend_on_what_goes() {
        let kept = [("src/a.rs", 900), ("src/b.rs", 4000), ("docs/c.md", 300), ("r.md", 20)];
        let gone = [("node_modules/x/y.js", 90_000), ("notes/archive/old.md", 900), ("web/node_modules/z.js", 9)];
        let r = rule(&["notes/archive"]);
        let all: Vec<(String, u64)> = kept.iter().chain(&gone).map(|(p, b)| (p.to_string(), *b)).collect();
        let stays: Vec<(String, u64)> = all.into_iter().filter(|(p, _)| !r.excludes(p)).collect();
        let only: Vec<(String, u64)> = kept.iter().map(|(p, b)| (p.to_string(), *b)).collect();
        assert_eq!(stays, only);
        assert_eq!(atlas(&repo(&stays)), atlas(&repo(&only)));
    }
}
