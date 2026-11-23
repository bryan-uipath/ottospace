//! Caps Lock as the return key (`return_caps_lock`), carried out: the key mapping read and set
//! through `KeyTable` (`/usr/bin/hidutil`, a fixed argv, no shell), and what ottospace added kept
//! in `<home>/caps_return.json`, so a run after a crash knows its own entry. Never in a test run.

use std::fs;
use std::path::Path;
use std::process::Command;

use ottospace_core::caps_return::{self, Mapping, Start};
use serde::{Deserialize, Serialize};

use crate::{config, running};

const RECORD: &str = "caps_return.json";

/// The system's key mapping. Tests put a table of their own in its place.
pub(crate) trait KeyTable {
    /// `hidutil property --get UserKeyMapping`'s output.
    fn get(&self) -> Result<String, String>;
    /// `hidutil property --set <json>`.
    fn set(&self, json: &str) -> Result<(), String>;
}

struct Hidutil;

impl Hidutil {
    fn run(args: &[&str]) -> Result<String, String> {
        let out = Command::new("/usr/bin/hidutil").args(args).output().map_err(|e| format!("hidutil: {e}"))?;
        if !out.status.success() {
            return Err(format!("hidutil failed ({})", out.status));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

impl KeyTable for Hidutil {
    fn get(&self) -> Result<String, String> {
        Self::run(&["property", "--get", "UserKeyMapping"])
    }
    fn set(&self, json: &str) -> Result<(), String> {
        Self::run(&["property", "--set", json]).map(drop)
    }
}

/// What ottospace added, and which run of it. While a run in `owned`'s change crashes, both
/// entries are listed: either one found is its own.
#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct Record {
    pid: u32,
    owned: Vec<Mapping>,
}

/// At start. `ours`: the entry to add (the setting on, the return key registered), else None
/// (a leftover of an earlier run is taken out). What went wrong, for the status bar.
pub(crate) fn start(ours: Option<Mapping>) -> Option<String> {
    let home = running::home()?;
    let me = std::process::id();
    start_with(&Hidutil, home, ours, me, running::alive)
        .err()
        .map(|why| format!("Caps Lock is not the return key: {why}"))
}

/// On a clean quit: takes out only what this run added.
pub(crate) fn quit() {
    if let Some(home) = running::home() {
        let _ = quit_with(&Hidutil, home, std::process::id());
    }
}

/// The record, if any. Err: one ottospace could not have written; then nothing is changed.
fn recorded(home: &Path) -> Result<Option<Record>, String> {
    let Ok(bytes) = fs::read(home.join(RECORD)) else { return Ok(None) };
    let record: Option<Record> = serde_json::from_slice(&bytes).ok();
    match record {
        Some(r) if r.owned.iter().all(caps_return::could_be_ours) => Ok(Some(r)),
        _ => Err(format!("{RECORD} is not one ottospace wrote, left as it is")),
    }
}

fn write(home: &Path, pid: u32, owned: Vec<Mapping>) -> Result<(), String> {
    let json = serde_json::to_vec(&Record { pid, owned }).map_err(|e| e.to_string())?;
    config::write_private(home, RECORD, &json).map_err(|e| e.to_string())
}

fn read(keys: &impl KeyTable) -> Result<Vec<Mapping>, String> {
    caps_return::parse(&keys.get()?).ok_or_else(|| "the key mapping could not be read".to_string())
}

fn start_with(
    keys: &impl KeyTable,
    home: &Path,
    ours: Option<Mapping>,
    me: u32,
    alive: impl Fn(u32) -> bool,
) -> Result<(), String> {
    let record = recorded(home)?;
    // Another ottospace that runs holds it: left to that one.
    if record.as_ref().is_some_and(|r| r.pid != me && alive(r.pid)) {
        return Ok(());
    }
    let left = record.map(|r| r.owned).unwrap_or_default();
    let Some(ours) = ours else { return clean(keys, home, &left) };
    match caps_return::add(&read(keys)?, ours, &left) {
        Start::Set(table) => {
            // Recorded first: a crash before the set leaves a record, never an entry nobody owns.
            let both = left.iter().copied().filter(|m| *m != ours).chain([ours]).collect();
            write(home, me, both)?;
            match keys.set(&caps_return::to_json(&table)) {
                Ok(()) => write(home, me, vec![ours]),
                Err(why) => {
                    let _ = match left.is_empty() {
                        true => fs::remove_file(home.join(RECORD)).map_err(|e| e.to_string()),
                        false => write(home, me, left),
                    };
                    Err(why)
                }
            }
        }
        Start::Kept => write(home, me, vec![ours]),
        Start::Theirs(dst) => {
            let _ = fs::remove_file(home.join(RECORD));
            match dst == ours.dst {
                true => Ok(()),
                false => Err("Caps Lock is mapped already, left as it is".into()),
            }
        }
    }
}

fn quit_with(keys: &impl KeyTable, home: &Path, me: u32) -> Result<(), String> {
    match recorded(home)? {
        Some(record) if record.pid == me => clean(keys, home, &record.owned),
        _ => Ok(()),
    }
}

/// Takes `owned` out of the table, then the record.
fn clean(keys: &impl KeyTable, home: &Path, owned: &[Mapping]) -> Result<(), String> {
    if !owned.is_empty()
        && let Some(table) = caps_return::remove(&read(keys)?, owned)
    {
        keys.set(&caps_return::to_json(&table))?;
    }
    config::removed(fs::remove_file(home.join(RECORD))).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;
    use std::cell::RefCell;

    /// A key mapping in memory, written back as hidutil prints it. `fail`: every set fails.
    #[derive(Default)]
    struct Table(RefCell<Vec<Mapping>>, bool);

    impl KeyTable for Table {
        fn get(&self) -> Result<String, String> {
            let entries: Vec<String> = (self.0.borrow().iter())
                .map(|m| {
                    format!(
                        "{{ HIDKeyboardModifierMappingDst = {}; HIDKeyboardModifierMappingSrc = {}; }}",
                        m.dst, m.src
                    )
                })
                .collect();
            Ok(if entries.is_empty() { "(null)".into() } else { format!("( {} )", entries.join(", ")) })
        }
        fn set(&self, json: &str) -> Result<(), String> {
            if self.1 {
                return Err("hidutil failed".into());
            }
            let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
            let field = |e: &serde_json::Value, k: &str| e[k].as_u64().ok_or("no number");
            let table: Result<Vec<Mapping>, _> = (v["UserKeyMapping"].as_array().ok_or("no list")?.iter())
                .map(|e| {
                    let (src, dst) =
                        (field(e, "HIDKeyboardModifierMappingSrc")?, field(e, "HIDKeyboardModifierMappingDst")?);
                    Ok::<_, &str>(Mapping { src, dst })
                })
                .collect();
            *self.0.borrow_mut() = table?;
            Ok(())
        }
    }

    fn table(entries: &[Mapping]) -> Table {
        Table(RefCell::new(entries.to_vec()), false)
    }

    fn home(name: &str) -> Scratch {
        Scratch::dir(&format!("ottospace-test-caps-{name}-{}", std::process::id()))
    }

    fn entries(keys: &Table) -> Vec<Mapping> {
        keys.0.borrow().clone()
    }

    const USERS: Mapping = Mapping { src: 0x7_0000_0064, dst: 0x7_0000_0035 };
    const DEAD: fn(u32) -> bool = |_| false;
    const ALIVE: fn(u32) -> bool = |_| true;

    fn f(n: u8) -> Mapping {
        caps_return::caps_as(n).expect("F13-F20")
    }

    #[test]
    fn start_adds_one_entry_and_quit_takes_out_only_it() {
        let home = home("quit");
        let keys = table(&[USERS]);
        assert_eq!(start_with(&keys, home.as_ref(), Some(f(18)), 1, DEAD), Ok(()));
        assert_eq!(entries(&keys), [USERS, f(18)]);
        // A crash, then a start again (another pid): nothing stacked.
        assert_eq!(start_with(&keys, home.as_ref(), Some(f(18)), 2, DEAD), Ok(()));
        assert_eq!(entries(&keys), [USERS, f(18)]);
        // The crashed run's pid quits nothing; this run's quit takes out only its entry.
        assert_eq!(quit_with(&keys, home.as_ref(), 1), Ok(()));
        assert_eq!(entries(&keys).len(), 2);
        assert_eq!(quit_with(&keys, home.as_ref(), 2), Ok(()));
        assert_eq!((entries(&keys), recorded(home.as_ref())), (vec![USERS], Ok(None)));
    }

    #[test]
    fn a_second_ottospace_leaves_the_first_ones_entry() {
        let home = home("second");
        let keys = table(&[]);
        start_with(&keys, home.as_ref(), Some(f(18)), 1, DEAD).expect("added");
        // A second run while the first lives: no key of its own, another key, or its quit.
        for ours in [None, Some(f(19)), Some(f(18))] {
            assert_eq!(start_with(&keys, home.as_ref(), ours, 2, ALIVE), Ok(()));
            assert_eq!(quit_with(&keys, home.as_ref(), 2), Ok(()));
            assert_eq!(entries(&keys), [f(18)]);
        }
    }

    #[test]
    fn a_users_caps_lock_is_kept_and_the_setting_off_takes_out_a_leftover() {
        let home = home("theirs");
        let theirs = Mapping { src: caps_return::CAPS_LOCK, dst: 0x7_0000_0029 };
        let keys = table(&[theirs]);
        assert!(start_with(&keys, home.as_ref(), Some(f(18)), 1, DEAD).is_err());
        assert_eq!((entries(&keys), recorded(home.as_ref())), (vec![theirs], Ok(None)));
        // Its own leftover from a crash, the setting since turned off: taken out at start.
        let keys = table(&[USERS]);
        start_with(&keys, home.as_ref(), Some(f(18)), 1, DEAD).expect("added");
        assert_eq!(start_with(&keys, home.as_ref(), None, 2, DEAD), Ok(()));
        assert_eq!((entries(&keys), recorded(home.as_ref())), (vec![USERS], Ok(None)));
    }

    #[test]
    fn a_failed_change_of_key_keeps_the_old_entry_its_own() {
        let home = home("change");
        let keys = table(&[USERS]);
        start_with(&keys, home.as_ref(), Some(f(13)), 1, DEAD).expect("added");
        // Now F18, and hidutil fails: F13 stays recorded, so a later start still changes it.
        let failing = Table(RefCell::new(entries(&keys)), true);
        assert!(start_with(&failing, home.as_ref(), Some(f(18)), 2, DEAD).is_err());
        assert_eq!(recorded(home.as_ref()).map(|r| r.map(|r| r.owned)), Ok(Some(vec![f(13)])));
        assert_eq!(start_with(&keys, home.as_ref(), Some(f(18)), 3, DEAD), Ok(()));
        assert_eq!(entries(&keys), [USERS, f(18)]);
    }

    #[test]
    fn a_record_ottospace_could_not_have_written_changes_nothing() {
        let home = home("damaged");
        let keys = table(&[USERS]);
        let users = Record { pid: 1, owned: vec![USERS] };
        fs::write(home.as_ref().join(RECORD), serde_json::to_vec(&users).expect("json")).expect("write");
        assert!(start_with(&keys, home.as_ref(), None, 2, DEAD).is_err());
        fs::write(home.as_ref().join(RECORD), b"not json").expect("write");
        assert!(start_with(&keys, home.as_ref(), Some(f(18)), 2, DEAD).is_err());
        assert_eq!(entries(&keys), [USERS]);
    }
}
