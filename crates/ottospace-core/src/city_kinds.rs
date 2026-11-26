//! A code city building's roof cap by its file's kind (flag `city_kinds`): code, test, docs,
//! config or data, from its path by `RULES`, the first that matches; a file none match has no
//! cap. Its walls keep its folder's colour (`city_colours`). Design: `docs/ai/code-city/kinds.md`.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::atlas::Atlas;
use crate::scene::{Block, Kind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FileKind {
    Code,
    Test,
    Docs,
    Config,
    Data,
}

/// Every kind, in the order the legend and the state list them.
pub const ALL: [FileKind; 5] = [FileKind::Code, FileKind::Test, FileKind::Docs, FileKind::Config, FileKind::Data];

impl FileKind {
    pub fn name(self) -> &'static str {
        match self {
            FileKind::Code => "code",
            FileKind::Test => "test",
            FileKind::Docs => "docs",
            FileKind::Config => "config",
            FileKind::Data => "data",
        }
    }

    /// Its cap's colour: lighter and stronger than any folder's walls (lightness 0.34), duller
    /// than a lit building, never amber ("needs you"); told apart by lightness too (code dim,
    /// data pale).
    pub fn colour(self) -> u32 {
        match self {
            FileKind::Code => 0x7d8ca1,
            FileKind::Test => 0xca7dc3,
            FileKind::Docs => 0x60be7f,
            FileKind::Config => 0x47b1c2,
            FileKind::Data => 0xa99ad6,
        }
    }
}

use FileKind::{Code, Config, Data, Docs, Test};

/// The rules, first match wins, case ignored. A pattern ending in `/` is a folder on the file's
/// way (`tests/`: `a/tests/b.rs`); any other, its name; `*` stands for any run of characters.
const RULES: &[(&str, FileKind)] = &[
    // Tests: in a test folder, or named as one.
    ("tests/", Test),
    ("test/", Test),
    ("__tests__/", Test),
    ("spec/", Test),
    ("e2e/", Test),
    ("*_test.*", Test),
    ("*.test.*", Test),
    ("*_spec.*", Test),
    ("*.spec.*", Test),
    ("test_*.py", Test),
    // Docs, by extension.
    ("*.md", Docs),
    ("*.mdx", Docs),
    ("*.markdown", Docs),
    ("*.txt", Docs),
    ("*.rst", Docs),
    ("*.adoc", Docs),
    ("*.org", Docs),
    // Config: settings, manifests, locks, dotfiles, dot folders, and JSON named as settings.
    ("*.toml", Config),
    ("*.yaml", Config),
    ("*.yml", Config),
    ("*.ini", Config),
    ("*.cfg", Config),
    ("*.conf", Config),
    ("*.lock", Config),
    ("*.plist", Config),
    ("*.properties", Config),
    ("*.env", Config),
    ("makefile", Config),
    ("dockerfile*", Config),
    ("justfile", Config),
    ("gemfile", Config),
    ("package.json", Config),
    ("package-lock.json", Config),
    ("*config*.json", Config),
    ("*settings*.json", Config),
    (".*", Config),
    (".*/", Config),
    // Data: what code reads, not runs: data files, pictures, sound, fonts.
    ("*.json", Data),
    ("*.jsonl", Data),
    ("*.ndjson", Data),
    ("*.csv", Data),
    ("*.tsv", Data),
    ("*.xml", Data),
    ("*.sql", Data),
    ("*.db", Data),
    ("*.sqlite", Data),
    ("*.parquet", Data),
    ("*.png", Data),
    ("*.jpg", Data),
    ("*.jpeg", Data),
    ("*.gif", Data),
    ("*.webp", Data),
    ("*.svg", Data),
    ("*.ico", Data),
    ("*.icns", Data),
    ("*.wav", Data),
    ("*.mp3", Data),
    ("*.mp4", Data),
    ("*.ttf", Data),
    ("*.otf", Data),
    ("*.woff*", Data),
    // Code, by extension.
    ("*.rs", Code),
    ("*.ts", Code),
    ("*.tsx", Code),
    ("*.js", Code),
    ("*.jsx", Code),
    ("*.mjs", Code),
    ("*.cjs", Code),
    ("*.py", Code),
    ("*.go", Code),
    ("*.swift", Code),
    ("*.c", Code),
    ("*.h", Code),
    ("*.cc", Code),
    ("*.cpp", Code),
    ("*.hpp", Code),
    ("*.m", Code),
    ("*.mm", Code),
    ("*.java", Code),
    ("*.kt", Code),
    ("*.rb", Code),
    ("*.php", Code),
    ("*.cs", Code),
    ("*.sh", Code),
    ("*.bash", Code),
    ("*.zsh", Code),
    ("*.lua", Code),
    ("*.zig", Code),
    ("*.wgsl", Code),
    ("*.metal", Code),
    ("*.glsl", Code),
    ("*.css", Code),
    ("*.scss", Code),
    ("*.html", Code),
    ("*.vue", Code),
    ("*.svelte", Code),
    // Docs by name, after code: `notice.rs` is code, `NOTICE` docs.
    ("readme*", Docs),
    ("license*", Docs),
    ("licence*", Docs),
    ("changelog*", Docs),
    ("contributing*", Docs),
    ("authors*", Docs),
    ("notice*", Docs),
    ("copying*", Docs),
];

/// The kind of the file at `id` (its path in the checkout, `a/b/c.rs`); none: no rule matches.
pub fn kind(id: &str) -> Option<FileKind> {
    let id = id.to_lowercase();
    let (folders, name) = id.rsplit_once('/').map_or(("", id.as_str()), |(folders, name)| (folders, name));
    // The first rule of its extension (`*.rs`) at once; only the rules before it are tried.
    let (endings, others) = index();
    let ending = name.rsplit_once('.').and_then(|(_, ext)| endings.get(ext)).copied();
    let first = others.iter().take_while(|i| ending.is_none_or(|e| **i < e)).find(|i| {
        let pattern = RULES[**i].0;
        match pattern.strip_suffix('/') {
            Some(folder) => !folders.is_empty() && folders.split('/').any(|f| glob(folder, f)),
            None => glob(pattern, name),
        }
    });
    first.copied().or(ending).map(|i| RULES[i].1)
}

/// `RULES`' extensions (`*.rs`: `rs`) to their first rule's place, and the places of the others,
/// in order: a file tries a few rules, not all (9,500 files: 17 ms to 5).
fn index() -> &'static (HashMap<&'static str, usize>, Vec<usize>) {
    static INDEX: OnceLock<(HashMap<&'static str, usize>, Vec<usize>)> = OnceLock::new();
    INDEX.get_or_init(|| {
        let (mut endings, mut others) = (HashMap::new(), Vec::new());
        for (i, (pattern, _)) in RULES.iter().enumerate() {
            match pattern.strip_prefix("*.").filter(|ext| !ext.contains(['*', '.', '/'])) {
                Some(ext) => {
                    endings.entry(ext).or_insert(i);
                }
                None => others.push(i),
            }
        }
        (endings, others)
    })
}

/// Each block's cap: a plain building's file's kind; none for the ground, a district (a closed
/// one too: it stands for many kinds) or a lit building (its task's colour wins).
pub fn caps(atlas: &Atlas, blocks: &[Block]) -> Vec<Option<FileKind>> {
    (blocks.iter())
        .map(|b| match (b.kind, b.place) {
            (Kind::Building, Some(i)) => kind(&atlas.places.get(i)?.id),
            _ => None,
        })
        .collect()
}

/// `text` matches `pattern`, `*` standing for any run of characters. No allocation: it runs for
/// every rule of every file.
fn glob(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let Some(mut rest) = parts.next().and_then(|first| text.strip_prefix(first)) else { return false };
    let mut parts = parts.peekable();
    if parts.peek().is_none() {
        return rest.is_empty();
    }
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return rest.ends_with(part);
        }
        let Some(at) = rest.find(part) else { return false };
        rest = &rest[at + part.len()..];
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::{atlas, repo};
    use crate::scene::{View, scene};

    /// Each kind, by folder, name and extension; tests by folder win over extension; a file no
    /// rule knows has none. Case is ignored.
    #[test]
    fn a_file_is_sorted_by_its_path() {
        let cases = [
            ("crates/core/src/lib.rs", Some(Code)),
            ("web/app.tsx", Some(Code)),
            ("crates/core/tests/prop.rs", Some(Test)),
            ("src/__tests__/a.js", Some(Test)),
            ("e2e/flow.ts", Some(Test)),
            ("pkg/store_test.go", Some(Test)),
            ("web/list.test.ts", Some(Test)),
            ("web/list.spec.tsx", Some(Test)),
            ("py/test_parse.py", Some(Test)),
            ("tests/fixtures/data.json", Some(Test)),
            ("docs/guide.md", Some(Docs)),
            ("README.md", Some(Docs)),
            ("LICENSE", Some(Docs)),
            ("notes.txt", Some(Docs)),
            ("Cargo.toml", Some(Config)),
            ("Cargo.lock", Some(Config)),
            (".github/workflows/ci.yml", Some(Config)),
            (".gitignore", Some(Config)),
            (".vscode/launch.json", Some(Config)),
            ("Makefile", Some(Config)),
            ("Dockerfile.dev", Some(Config)),
            ("sdk/package.json", Some(Config)),
            ("sdk/tsconfig.json", Some(Config)),
            ("fixtures/users.json", Some(Data)),
            ("data/rows.csv", Some(Data)),
            ("assets/icon.svg", Some(Data)),
            ("SRC/Main.RS", Some(Code)),
            ("src/notice.rs", Some(Code)),
            ("scripts/osi", None),
            ("bin/tool.exe", None),
        ];
        for (id, want) in cases {
            assert_eq!(kind(id), want, "{id}");
        }
    }

    #[test]
    fn a_glob_matches_runs_of_characters() {
        assert!(glob("*.rs", "a.rs") && glob("*.rs", ".rs") && !glob("*.rs", "a.rsx"));
        assert!(glob("*.test.*", "a.test.ts") && !glob("*.test.*", "a.test"));
        assert!(glob("readme*", "readme") && glob("*config*.json", "tsconfig.base.json"));
        assert!(glob("makefile", "makefile") && !glob("makefile", "makefile.am"));
        assert!(glob(".*", ".env") && !glob(".*", "a.env"));
    }

    /// Only plain buildings are capped: never the ground, a district or a lit building.
    #[test]
    fn only_plain_buildings_have_a_cap() {
        let plan = atlas(&repo(&[("src/a.rs".into(), 900), ("src/b.rs".into(), 900), ("docs/c.md".into(), 900)]));
        let s = scene(&plan, &[("src/a.rs", 0xff0000)], 1.6, &View::default());
        for (b, cap) in s.blocks.iter().zip(caps(&plan, &s.blocks)) {
            assert_eq!(cap.is_some(), b.kind == Kind::Building, "{b:?}");
        }
        let id = |b: &Block| plan.places[b.place.unwrap()].id.as_str();
        let docs = s.blocks.iter().position(|b| b.kind == Kind::Building && id(b) == "docs/c.md").unwrap();
        assert_eq!(caps(&plan, &s.blocks)[docs], Some(Docs));
    }

    /// The caps' colours are apart from each other, lighter than any folder's walls, and none
    /// is amber's hue (25° to 65°).
    #[test]
    fn caps_are_apart_and_never_amber() {
        let channels = |c: u32| [c >> 16 & 0xff, c >> 8 & 0xff, c & 0xff].map(|v| v as f32 / 255.);
        for (i, a) in ALL.iter().enumerate() {
            let [r, g, b] = channels(a.colour());
            let (max, min) = (r.max(g).max(b), r.min(g).min(b));
            assert!((max + min) / 2. > 0.45, "{a:?} lighter than walls");
            let hue = if max == r {
                60. * ((g - b) / (max - min)).rem_euclid(6.)
            } else if max == g {
                60. * ((b - r) / (max - min) + 2.)
            } else {
                60. * ((r - g) / (max - min) + 4.)
            };
            assert!(!(25. ..65.).contains(&hue), "{a:?} is amber: {hue}");
            for b in &ALL[i + 1..] {
                let d: f32 = (channels(a.colour()).iter().zip(channels(b.colour()))).map(|(x, y)| (x - y).abs()).sum();
                assert!(d > 0.25, "{a:?} and {b:?} too close");
            }
        }
    }
}
