//! `<home>/log`: what happened to the app, for its owner to read after a problem. Private (0600),
//! appended, one JSON line an event: `{"at":"2026-11-16T17:51:03-08:00","ev":"displays","count":1,..}`.
//! At 1 MB it becomes `log.1`, and the one before goes: 2 MB at most.
//! Events only, as `trace`: names, counts, sizes, ids and versions. Never terminal or transcript
//! text, titles, meetings, passes, paths or file contents.
//! Each line also goes to the unified log (`ottospace_mac::oslog`), outside a test run.
//! A test run writes the file only in the folder it names (`OTTOSPACE_HOME`).

use std::fs;
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::config;

const NAME: &str = "log";
const OLD: &str = "log.1";
/// The size at which the file is turned over.
const LIMIT: u64 = 1 << 20;

/// Writes `{"at":..,"ev":event, ..fields}`. `fields`: names, numbers and ids only.
pub(crate) fn event(event: &str, fields: Value) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs() as i64);
    let line = line(now, ottospace_mac::time::utc_offset(now), event, fields);
    if std::env::var_os("OTTOSPACE_E2E").is_none() {
        ottospace_mac::oslog::write(&line);
    }
    if let Some(home) = config::get().home.as_deref() {
        let _ = append(home, &line);
    }
}

/// Every panic, where it happened (`file:line`) and on which thread; never its message, which
/// may carry any text.
pub(crate) fn arm() {
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let at = info.location().map(|at| format!("{}:{}", at.file(), at.line()));
        let thread = std::thread::current().name().map(String::from);
        event("panic", json!({ "at": at, "thread": thread }));
        before(info);
    }));
}

/// The line for `event` at `now` (seconds since 1970), in local time `offset` seconds east of UTC.
fn line(now: i64, offset: i64, event: &str, fields: Value) -> String {
    let mut line = json!({ "at": local_time(now, offset), "ev": event });
    if let (Some(line), Value::Object(fields)) = (line.as_object_mut(), fields) {
        line.extend(fields);
    }
    line.to_string()
}

/// `2026-11-16T17:51:03-08:00`.
fn local_time(now: i64, offset: i64) -> String {
    const DAY: i64 = 86_400;
    let offset = offset.clamp(-DAY + 1, DAY - 1);
    let t = now + offset;
    let (y, m, d) = ottospace_core::ics::civil(t.div_euclid(DAY));
    let (s, o) = (t.rem_euclid(DAY), offset.abs());
    let sign = if offset < 0 { '-' } else { '+' };
    let (hour, minute, second) = (s / 3600, s / 60 % 60, s % 60);
    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}{sign}{:02}:{:02}", o / 3600, o / 60 % 60)
}

/// Appends `line` to `dir/log`, turning a full file over first. Never through a link.
fn append(dir: &Path, line: &str) -> io::Result<()> {
    if !dir.is_dir() {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let path = dir.join(NAME);
    if fs::symlink_metadata(&path).is_ok_and(|meta| meta.len() >= LIMIT) {
        // A rename replaces `log.1` itself, a link there included.
        fs::rename(&path, dir.join(OLD))?;
    }
    let mut file =
        fs::OpenOptions::new().append(true).create(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&path)?;
    // Private whatever made it.
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(format!("{line}\n").as_bytes())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A folder of the test's own, removed when it ends.
    struct Temp(PathBuf);

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn temp(name: &str) -> Temp {
        Temp(std::env::temp_dir().join(format!("ottospace-test-event-log-{name}-{}", std::process::id())))
    }

    #[test]
    fn a_line_is_the_local_time_the_event_and_its_fields() {
        // 2026-11-17T01:51:03Z, in a zone 8 hours west.
        let line = line(1_794_880_263, -8 * 3600, "displays", json!({ "count": 1 }));
        assert_eq!(line, r#"{"at":"2026-11-16T17:51:03-08:00","ev":"displays","count":1}"#);
        assert_eq!(local_time(0, 5 * 3600 + 30 * 60), "1970-01-01T05:30:00+05:30");
    }

    #[test]
    fn the_file_is_private_appended_and_turned_over_when_full() {
        let dir = temp("turn");
        append(&dir.0, "one").unwrap();
        append(&dir.0, "two").unwrap();
        let path = dir.0.join(NAME);
        assert_eq!(fs::read_to_string(&path).unwrap(), "one\ntwo\n");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        // Full: it becomes `log.1`, and the next line starts a new file.
        fs::write(&path, vec![b'x'; LIMIT as usize]).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        append(&dir.0, "three").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "three\n");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(dir.0.join(OLD)).unwrap().len(), LIMIT);
        // Full again: the oldest goes.
        fs::write(&path, vec![b'y'; LIMIT as usize]).unwrap();
        append(&dir.0, "four").unwrap();
        assert_eq!(fs::read(dir.0.join(OLD)).unwrap()[0], b'y');
        assert_eq!(fs::read_to_string(&path).unwrap(), "four\n");
    }

    #[test]
    fn the_log_is_never_written_through_a_link() {
        let dir = temp("link");
        fs::create_dir_all(&dir.0).unwrap();
        let elsewhere = dir.0.join("elsewhere");
        fs::write(&elsewhere, "").unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.0.join(NAME)).unwrap();
        assert!(append(&dir.0, "one").is_err());
        assert_eq!(fs::read_to_string(&elsewhere).unwrap(), "");
    }
}
