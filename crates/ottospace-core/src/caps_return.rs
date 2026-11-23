//! Caps Lock as the return key (`return_caps_lock`): ottospace adds one entry to macOS's key
//! mapping (`hidutil`'s `UserKeyMapping`) at start and takes out only that one at quit. Tables
//! in, tables out: the shell reads and sets them.

use serde::{Deserialize, Serialize};

/// Caps Lock's HID usage (page 7, 0x39).
pub const CAPS_LOCK: u64 = 0x7_0000_0039;

/// One entry: the key `src` acts as `dst` (HID usages, page in the high bits).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Mapping {
    pub src: u64,
    pub dst: u64,
}

/// Caps Lock as F`n`, for F13 to F20 (usages 0x68 to 0x6F).
pub fn caps_as(n: u8) -> Option<Mapping> {
    (13..=20).contains(&n).then(|| Mapping { src: CAPS_LOCK, dst: 0x7_0000_0068 + u64::from(n - 13) })
}

/// What to do at start.
#[derive(Debug, PartialEq)]
pub enum Start {
    /// Set this table: the user's entries, then ottospace's.
    Set(Vec<Mapping>),
    /// ottospace's entry is there already, left by a run that did not quit cleanly.
    Kept,
    /// Caps Lock is the user's, mapped to this: left alone.
    Theirs(u64),
}

/// The table with `ours` added. `left`: the entry an earlier run recorded as its own.
/// E.g. `[A→B]` → `Set([A→B, ⇪→F18])`; `[⇪→Esc]` → `Theirs(Esc)`.
pub fn add(table: &[Mapping], ours: Mapping, left: Option<Mapping>) -> Start {
    match table.iter().position(|m| m.src == ours.src) {
        None => Start::Set(table.iter().copied().chain([ours]).collect()),
        Some(at) if Some(table[at]) == left && table[at] == ours => Start::Kept,
        // Its own, to another return key: changed in place.
        Some(at) if Some(table[at]) == left => {
            let mut table = table.to_vec();
            table[at] = ours;
            Start::Set(table)
        }
        Some(at) => Start::Theirs(table[at].dst),
    }
}

/// The table without `ours`, every other entry as it was. None: it is not there.
pub fn remove(table: &[Mapping], ours: Mapping) -> Option<Vec<Mapping>> {
    let at = table.iter().position(|m| *m == ours)?;
    let mut table = table.to_vec();
    table.remove(at);
    Some(table)
}

/// `hidutil property --get UserKeyMapping`'s output: `(null)`, or `( { …Dst = n; …Src = n; }, … )`.
/// None: anything else, or an entry with another key: then nothing is changed.
pub fn parse(text: &str) -> Option<Vec<Mapping>> {
    let text = text.trim();
    if text == "(null)" {
        return Some(Vec::new());
    }
    let mut rest = text.strip_prefix('(')?.strip_suffix(')')?.trim();
    let mut table = Vec::new();
    while !rest.is_empty() {
        let (entry, after) = rest.strip_prefix('{')?.split_once('}')?;
        let (mut src, mut dst) = (None, None);
        for field in entry.split(';').map(str::trim).filter(|f| !f.is_empty()) {
            let (key, value) = field.split_once('=')?;
            let value = value.trim();
            let value = match value.strip_prefix("0x") {
                Some(hex) => u64::from_str_radix(hex, 16).ok()?,
                None => value.parse().ok()?,
            };
            let slot = match key.trim() {
                "HIDKeyboardModifierMappingSrc" => &mut src,
                "HIDKeyboardModifierMappingDst" => &mut dst,
                _ => return None,
            };
            if slot.replace(value).is_some() {
                return None;
            }
        }
        table.push(Mapping { src: src?, dst: dst? });
        rest = after.trim_start();
        rest = rest.strip_prefix(',').unwrap_or(rest).trim_start();
    }
    Some(table)
}

/// The argument `hidutil property --set` takes for `table`.
pub fn to_json(table: &[Mapping]) -> String {
    let entries: Vec<String> = (table.iter())
        .map(|m| format!(r#"{{"HIDKeyboardModifierMappingSrc":{},"HIDKeyboardModifierMappingDst":{}}}"#, m.src, m.dst))
        .collect();
    format!(r#"{{"UserKeyMapping":[{}]}}"#, entries.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    const F18: u64 = 0x7_0000_006D;
    const ESC: u64 = 0x7_0000_0029;
    const A: Mapping = Mapping { src: 0x7_0000_0064, dst: 0x7_0000_0035 };

    fn ours() -> Mapping {
        caps_as(18).expect("F18")
    }

    #[test]
    fn caps_lock_maps_to_an_f_key_from_13_to_20() {
        assert_eq!(ours(), Mapping { src: CAPS_LOCK, dst: F18 });
        assert_eq!(caps_as(13).map(|m| m.dst), Some(0x7_0000_0068));
        assert_eq!(caps_as(20).map(|m| m.dst), Some(0x7_0000_006F));
        assert_eq!((caps_as(12), caps_as(21)), (None, None));
    }

    #[test]
    fn the_entry_is_added_after_the_users_and_only_it_is_taken_out() {
        assert_eq!(add(&[], ours(), None), Start::Set(vec![ours()]));
        let added = add(&[A], ours(), None);
        assert_eq!(added, Start::Set(vec![A, ours()]));
        // At quit, the user's entries stay, also one they added meanwhile.
        let b = Mapping { src: 1, dst: 2 };
        assert_eq!(remove(&[A, ours(), b], ours()), Some(vec![A, b]));
        // Gone already (the user set the table again): nothing to set.
        assert_eq!(remove(&[A], ours()), None);
    }

    #[test]
    fn a_caps_lock_the_user_mapped_is_left_alone() {
        let theirs = Mapping { src: CAPS_LOCK, dst: ESC };
        assert_eq!(add(&[A, theirs], ours(), None), Start::Theirs(ESC));
        // Even to F18, by hand (no record): it is theirs, never taken out.
        assert_eq!(add(&[ours()], ours(), None), Start::Theirs(F18));
        // A record of ours that no longer matches does not make theirs ours.
        assert_eq!(add(&[theirs], ours(), Some(ours())), Start::Theirs(ESC));
    }

    #[test]
    fn after_a_crash_its_own_leftover_is_kept_not_stacked() {
        // The record matches the entry: nothing to set, the user's entries untouched.
        assert_eq!(add(&[A, ours()], ours(), Some(ours())), Start::Kept);
        // Its leftover to another return key: changed in place, nothing stacked.
        let f13 = caps_as(13).expect("F13");
        assert_eq!(add(&[f13, A], ours(), Some(f13)), Start::Set(vec![ours(), A]));
        // A record but no entry (it crashed before setting it): added again.
        assert_eq!(add(&[A], ours(), Some(ours())), Start::Set(vec![A, ours()]));
    }

    #[test]
    fn hidutils_output_is_read_or_refused() {
        assert_eq!(parse("(null)\n"), Some(vec![]));
        assert_eq!(parse("(\n)"), Some(vec![]));
        let one = "(\n        {\n        HIDKeyboardModifierMappingDst = 30064771181;\n        HIDKeyboardModifierMappingSrc = 30064771129;\n    }\n)\n";
        assert_eq!(parse(one), Some(vec![ours()]));
        let two = "( { HIDKeyboardModifierMappingSrc = 0x700000064; HIDKeyboardModifierMappingDst = 0x700000035; }, { HIDKeyboardModifierMappingDst = 30064771181; HIDKeyboardModifierMappingSrc = 30064771129; } )";
        assert_eq!(parse(two), Some(vec![A, ours()]));
        // Anything else: an unknown key, a missing or doubled field, a stray word, an error.
        for bad in [
            "",
            "error",
            "( { HIDKeyboardModifierMappingSrc = 1; } )",
            "( { HIDKeyboardModifierMappingSrc = 1; HIDKeyboardModifierMappingDst = 2; Other = 3; } )",
            "( { HIDKeyboardModifierMappingSrc = 1; HIDKeyboardModifierMappingSrc = 1; HIDKeyboardModifierMappingDst = 2; } )",
            "( { HIDKeyboardModifierMappingSrc = x; HIDKeyboardModifierMappingDst = 2; } )",
            "( junk )",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_table_is_written_as_hidutil_takes_it() {
        assert_eq!(to_json(&[]), r#"{"UserKeyMapping":[]}"#);
        assert_eq!(
            to_json(&[ours()]),
            r#"{"UserKeyMapping":[{"HIDKeyboardModifierMappingSrc":30064771129,"HIDKeyboardModifierMappingDst":30064771181}]}"#
        );
        // What it writes, it reads back.
        let text = format!(
            "( {} )",
            "{ HIDKeyboardModifierMappingSrc = 30064771172; HIDKeyboardModifierMappingDst = 30064771125; }"
        );
        assert_eq!(parse(&text), Some(vec![A]));
    }
}
