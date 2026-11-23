//! A code city building's colour, from its folder by a fixed rule, so nothing changes colour as
//! files come and go: the nearest folder above it with a named colour (built in, e.g. `docs`
//! green, `tests` violet, or the user's `city_folder_colors`), else its top-level folder's own,
//! from a hash of that folder's name (never its place or order). A file in the root has none.
//! Muted, so a lit building (its task's colour) and amber ("needs you") stand out over them.

use std::collections::BTreeMap;

use crate::atlas::Atlas;
use crate::scene::{Block, Kind};

/// The colours a folder may be given by name, as hues (degrees) and saturations.
const PALETTE: [(&str, f32, f32); 12] = [
    ("red", 4., 0.32),
    ("orange", 24., 0.32),
    ("yellow", 54., 0.3),
    ("green", 140., 0.3),
    ("teal", 172., 0.3),
    ("cyan", 192., 0.32),
    ("blue", 218., 0.34),
    ("indigo", 240., 0.32),
    ("violet", 268., 0.3),
    ("pink", 322., 0.3),
    ("brown", 30., 0.18),
    ("slate", 215., 0.1),
];

/// Built-in colours by folder name (case ignored): what a folder is for.
const DEFAULTS: [(&str, &str); 17] = [
    ("docs", "green"),
    ("doc", "green"),
    ("tests", "violet"),
    ("test", "violet"),
    ("__tests__", "violet"),
    ("spec", "violet"),
    ("e2e", "pink"),
    ("src", "blue"),
    ("lib", "blue"),
    ("crates", "teal"),
    ("packages", "teal"),
    ("pkg", "teal"),
    ("apps", "cyan"),
    ("scripts", "slate"),
    ("tools", "slate"),
    ("assets", "brown"),
    ("examples", "yellow"),
];

/// Most names the user may colour.
pub const MOST: usize = 64;

/// A colour as a hue (degrees) and saturation, drawn at a fixed lightness.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tone {
    pub hue: f32,
    pub saturation: f32,
}

impl Tone {
    /// A building's top (muted, dark like the plain building's) and its edges (lighter).
    pub fn top_and_edge(self) -> (u32, u32) {
        (hsl(self.hue, self.saturation, 0.34), hsl(self.hue, (self.saturation + 0.1).min(1.), 0.64))
    }
}

/// The colour by name, or why there is none.
fn named(name: &str) -> Result<Tone, String> {
    let at = PALETTE.iter().find(|(n, ..)| n.eq_ignore_ascii_case(name.trim()));
    let names: Vec<&str> = PALETTE.iter().map(|(n, ..)| *n).collect();
    at.map(|(_, hue, saturation)| Tone { hue: *hue, saturation: *saturation })
        .ok_or_else(|| format!("a city folder color is one of {}", names.join(", ")))
}

/// The rule: folder names (lower case) with a colour, the built-in ones and the user's over them.
#[derive(Clone, Debug, PartialEq)]
pub struct FolderColours {
    names: BTreeMap<String, Tone>,
}

impl FolderColours {
    /// The built-in names only.
    pub fn built_in() -> FolderColours {
        let names = DEFAULTS.iter().filter_map(|(folder, colour)| Some((folder.to_string(), named(colour).ok()?)));
        FolderColours { names: names.collect() }
    }

    /// The built-in names with `user`'s (folder name → colour name) over them, or why not.
    pub fn new(user: &BTreeMap<String, String>) -> Result<FolderColours, String> {
        if user.len() > MOST {
            return Err(format!("at most {MOST} city folder colors"));
        }
        let mut names = FolderColours::built_in().names;
        for (folder, colour) in user {
            // As a path names it: a name padded with spaces is refused, never trimmed to another's.
            let folder = folder.to_lowercase();
            if folder.trim().is_empty()
                || folder != folder.trim()
                || folder.contains('/')
                || folder.chars().any(char::is_control)
            {
                return Err(format!("a city folder color's name is one folder's name, not {folder:?}"));
            }
            names.insert(folder, named(colour)?);
        }
        Ok(FolderColours { names })
    }

    /// The colour of a file at `id` (its path in the checkout, `a/b/c.rs`): its nearest folder
    /// with a named colour, else its top-level folder's own; none in the root.
    pub fn of(&self, id: &str) -> Option<Tone> {
        let folders: Vec<&str> = id.split('/').collect();
        let folders = &folders[..folders.len().saturating_sub(1)];
        let top = folders.first()?;
        let named = folders.iter().rev().find_map(|folder| self.names.get(&folder.to_lowercase()));
        Some(named.copied().unwrap_or_else(|| hashed(top)))
    }
}

/// A folder's own colour, from its name alone: FNV-1a, its hue round the wheel less the band of
/// amber ("needs you").
fn hashed(name: &str) -> Tone {
    let hash = name.to_lowercase().bytes().fold(0x811c9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x01000193));
    let hue = (hash % 320) as f32;
    Tone { hue: if hue < 25. { hue } else { hue + 40. }, saturation: 0.28 }
}

/// Each block's top and edge colours: a building's by its folder (`FolderColours::of`); none for
/// the ground, a district or a lit building (its task's colour).
pub fn tints(atlas: &Atlas, blocks: &[Block], rule: &FolderColours) -> Vec<Option<(u32, u32)>> {
    (blocks.iter())
        .map(|b| match (b.kind, b.place) {
            (Kind::Building, Some(i)) => rule.of(&atlas.places.get(i)?.id).map(Tone::top_and_edge),
            _ => None,
        })
        .collect()
}

fn hsl(hue: f32, s: f32, l: f32) -> u32 {
    let c = (1. - (2. * l - 1.).abs()) * s;
    let h = hue.rem_euclid(360.) / 60.;
    let x = c * (1. - (h % 2. - 1.).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    };
    let m = l - c / 2.;
    let byte = |v: f32| ((v + m).clamp(0., 1.) * 255.).round() as u32;
    byte(r) << 16 | byte(g) << 8 | byte(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::{atlas, repo};
    use crate::scene::{View, scene};

    fn rule() -> FolderColours {
        FolderColours::built_in()
    }

    fn tone(name: &str) -> Tone {
        named(name).unwrap()
    }

    /// The nearest named folder wins (`crates/a/tests` is tests'), else the top-level folder's own;
    /// a file in the root has none. Case is ignored.
    #[test]
    fn a_building_takes_its_nearest_named_folder_else_its_top_folders_own() {
        let r = rule();
        assert_eq!(r.of("docs/guide.md"), Some(tone("green")));
        assert_eq!(r.of("crates/core/tests/a.rs"), Some(tone("violet")));
        assert_eq!(r.of("crates/core/src/lib.rs"), Some(tone("blue")));
        assert_eq!(r.of("crates/Cargo.toml"), Some(tone("teal")));
        assert_eq!(r.of("Docs/a.md"), Some(tone("green")));
        assert_eq!(r.of("sdk/client/x.ts"), Some(hashed("sdk")));
        assert_eq!(r.of("README.md"), None);
    }

    /// A name's colour is its own: the same in any tree, whatever else is in it or how it is
    /// ordered; two names apart mostly differ; never amber's band.
    #[test]
    fn a_folders_own_colour_is_from_its_name_alone() {
        assert_eq!(hashed("sdk"), hashed("SDK"));
        let hues: Vec<f32> = ["sdk", "site", "notes", "archive", "infra", "web"].map(|n| hashed(n).hue).to_vec();
        let distinct: std::collections::BTreeSet<u32> = hues.iter().map(|h| *h as u32).collect();
        assert!(distinct.len() >= 5, "{hues:?}");
        for n in 0..2000 {
            let hue = hashed(&format!("folder{n}")).hue;
            assert!(!(25. ..65.).contains(&hue) && (0. ..360.).contains(&hue), "{hue}");
        }
    }

    /// Adding a file or a folder changes no other building's colour.
    #[test]
    fn adding_a_file_or_folder_changes_no_other_colour() {
        let files = |extra: &[(&str, u64)]| {
            let mut all: Vec<(String, u64)> =
                [("docs/a.md", 900), ("src/x.rs", 4000), ("sdk/y.ts", 300), ("notes/z.md", 900), ("r.md", 9)]
                    .iter()
                    .chain(extra)
                    .map(|(p, b)| (p.to_string(), *b))
                    .collect();
            all.sort();
            all
        };
        let colours = |files: &[(String, u64)]| {
            let plan = atlas(&repo(files));
            let s = scene(&plan, &[], 1.6, &View::default());
            let tints = tints(&plan, &s.blocks, &rule());
            let by_id: BTreeMap<String, Option<(u32, u32)>> = (s.blocks.iter().zip(tints))
                .filter(|(b, _)| b.kind == Kind::Building)
                .filter_map(|(b, t)| Some((plan.places[b.place?].id.clone(), t)))
                .collect();
            by_id
        };
        let before = colours(&files(&[]));
        for extra in [&[("aaa/new.rs", 900)][..], &[("sdk/more.ts", 900), ("zzz/q.rs", 9)], &[("src/deep/w.rs", 50)]] {
            let after = colours(&files(extra));
            for (id, colour) in &before {
                assert_eq!(after.get(id), Some(colour), "{id} after {extra:?}");
            }
        }
        assert_eq!(before["r.md"], None);
        assert_eq!(before["docs/a.md"], Some(tone("green").top_and_edge()));
    }

    /// The user's names go over the built-in ones; a wrong colour or name is refused. Every
    /// built-in name's colour is in the palette.
    #[test]
    fn the_users_colours_go_over_the_built_in_ones() {
        assert_eq!(FolderColours::built_in().names.len(), DEFAULTS.len());
        let user = |pairs: &[(&str, &str)]| {
            FolderColours::new(&pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect())
        };
        let r = user(&[("docs", "red"), ("Notes", "teal")]).unwrap();
        assert_eq!((r.of("docs/a.md"), r.of("notes/b/c.md")), (Some(tone("red")), Some(tone("teal"))));
        assert!(user(&[("docs", "chartreuse")]).unwrap_err().contains("one of red"));
        assert!(user(&[("a/b", "red")]).is_err());
        assert!(user(&[(" ", "red")]).is_err());
        assert!(user(&[(" docs", "red")]).is_err(), "never trimmed to another folder's name");
        let many: Vec<(String, String)> = (0..=MOST).map(|n| (format!("f{n}"), "red".into())).collect();
        assert!(FolderColours::new(&many.into_iter().collect()).is_err());
    }

    /// A building of a lit file keeps its task's colour; the ground and districts take none.
    #[test]
    fn only_plain_buildings_are_tinted() {
        let plan = atlas(&repo(&[("src/a.rs".into(), 900), ("src/b.rs".into(), 900), ("docs/c.md".into(), 900)]));
        let s = scene(&plan, &[("src/a.rs", 0xff0000)], 1.6, &View::default());
        for (b, t) in s.blocks.iter().zip(tints(&plan, &s.blocks, &rule())) {
            assert_eq!(t.is_some(), b.kind == Kind::Building, "{b:?}");
        }
    }

    #[test]
    fn hsl_is_rgb() {
        assert_eq!((hsl(0., 1., 0.5), hsl(120., 1., 0.5), hsl(240., 1., 0.5)), (0xff0000, 0x00ff00, 0x0000ff));
        assert_eq!(hsl(0., 0., 0.5), 0x808080);
    }
}
