//! Caps Lock as the return key (`return_caps_lock`), carried out: the key mapping read and set
//! through `KeyTable` (`/usr/bin/hidutil`, a fixed argv, no shell), and what ottospace added kept
//! in `<home>/caps_return.json`, so a run after a crash knows its own entry. Never in a test run.

use std::fs;
use std::path::Path;
use std::process::Command;

use ottospace_core::caps_return::{self, Mapping, Start};

use crate::config;

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

/// The folder of the record: the user's, outside a test run only.
fn home() -> Option<&'static Path> {
    if std::env::var_os("OTTOSPACE_E2E").is_some() {
        return None;
    }
    config::get().home.as_deref()
}

/// At start. `ours`: the entry to add (the setting on, the return key registered), else None
/// (a leftover of an earlier run is taken out). What went wrong, for the status bar.
pub(crate) fn start(ours: Option<Mapping>) -> Option<String> {
    let home = home()?;
    start_with(&Hidutil, home, ours).err().map(|why| format!("Caps Lock is not the return key: {why}"))
}

/// On a clean quit: takes out only ottospace's entry.
pub(crate) fn quit() {
    if let Some(home) = home() {
        let _ = quit_with(&Hidutil, home);
    }
}

fn recorded(home: &Path) -> Option<Mapping> {
    serde_json::from_slice(&fs::read(home.join(RECORD)).ok()?).ok()
}

fn read(keys: &impl KeyTable) -> Result<Vec<Mapping>, String> {
    caps_return::parse(&keys.get()?).ok_or_else(|| "the key mapping could not be read".to_string())
}

fn start_with(keys: &impl KeyTable, home: &Path, ours: Option<Mapping>) -> Result<(), String> {
    let left = recorded(home);
    if ours.is_none() && left.is_none() {
        return Ok(());
    }
    let table = read(keys)?;
    let Some(ours) = ours else { return quit_with(keys, home) };
    match caps_return::add(&table, ours, left) {
        Start::Set(table) => {
            // Recorded first: a crash before the set leaves a record, never an entry nobody owns.
            let json = serde_json::to_vec(&ours).map_err(|e| e.to_string())?;
            config::write_private(home, RECORD, &json).map_err(|e| e.to_string())?;
            keys.set(&caps_return::to_json(&table)).inspect_err(|_| drop(fs::remove_file(home.join(RECORD))))
        }
        Start::Kept => Ok(()),
        Start::Theirs(dst) => {
            let _ = fs::remove_file(home.join(RECORD));
            match dst == ours.dst {
                true => Ok(()),
                false => Err("Caps Lock is mapped already, left as it is".into()),
            }
        }
    }
}

fn quit_with(keys: &impl KeyTable, home: &Path) -> Result<(), String> {
    let Some(ours) = recorded(home) else { return Ok(()) };
    if let Some(table) = caps_return::remove(&read(keys)?, ours) {
        keys.set(&caps_return::to_json(&table))?;
    }
    config::removed(fs::remove_file(home.join(RECORD))).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A key mapping in memory, written back as hidutil prints it.
    struct Table(RefCell<Vec<Mapping>>);

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
            let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
            let entry = |e: &serde_json::Value, k: &str| e[k].as_u64().ok_or("no number");
            let table: Result<Vec<Mapping>, _> = (v["UserKeyMapping"].as_array().ok_or("no list")?.iter())
                .map(|e| {
                    Ok::<_, &str>(Mapping {
                        src: entry(e, "HIDKeyboardModifierMappingSrc")?,
                        dst: entry(e, "HIDKeyboardModifierMappingDst")?,
                    })
                })
                .collect();
            *self.0.borrow_mut() = table?;
            Ok(())
        }
    }

    struct Home(std::path::PathBuf);
    impl Drop for Home {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn home(name: &str) -> Home {
        let dir = std::env::temp_dir().join(format!("ottospace-caps-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        Home(dir)
    }

    const USERS: Mapping = Mapping { src: 0x7_0000_0064, dst: 0x7_0000_0035 };

    #[test]
    fn start_adds_one_entry_and_quit_takes_out_only_it() {
        let home = home("quit");
        let ours = caps_return::caps_as(18);
        let keys = Table(RefCell::new(vec![USERS]));
        assert_eq!(start_with(&keys, &home.0, ours), Ok(()));
        assert_eq!(*keys.0.borrow(), [USERS, ours.expect("F18")]);
        // A crash, then a start again: nothing stacked.
        assert_eq!(start_with(&keys, &home.0, ours), Ok(()));
        assert_eq!(keys.0.borrow().len(), 2);
        assert_eq!(quit_with(&keys, &home.0), Ok(()));
        assert_eq!((keys.0.borrow().clone(), recorded(&home.0)), (vec![USERS], None));
    }

    #[test]
    fn a_users_caps_lock_is_kept_and_the_setting_off_takes_out_a_leftover() {
        let home = home("theirs");
        let theirs = Mapping { src: caps_return::CAPS_LOCK, dst: 0x7_0000_0029 };
        let keys = Table(RefCell::new(vec![theirs]));
        assert!(start_with(&keys, &home.0, caps_return::caps_as(18)).is_err());
        assert_eq!((keys.0.borrow().clone(), recorded(&home.0)), (vec![theirs], None));
        // Its own leftover from a crash, the setting since turned off: taken out at start.
        let ours = caps_return::caps_as(18);
        let keys = Table(RefCell::new(vec![USERS]));
        start_with(&keys, &home.0, ours).expect("added");
        assert_eq!(start_with(&keys, &home.0, None), Ok(()));
        assert_eq!((keys.0.borrow().clone(), recorded(&home.0)), (vec![USERS], None));
    }
}
