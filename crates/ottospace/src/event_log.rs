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
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::config;

const NAME: &str = "log";
const OLD: &str = "log.1";
/// The size at which the file is turned over.
const LIMIT: u64 = 1 << 20;

/// Writes `{"at":..,"ev":event, ..fields}`. `fields`: names, numbers and ids only.
pub(crate) fn event(event: &str, fields: Value) {
    write(event, fields, Some(config::get()));
}

fn write(event: &str, fields: Value, config: Option<&config::Config>) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs() as i64);
    let line = line(now, ottospace_mac::time::utc_offset(now), event, fields);
    if std::env::var_os("OTTOSPACE_E2E").is_none() {
        ottospace_mac::oslog::write(&line);
    }
    if let Some(home) = config.and_then(|config| config.home.as_deref()) {
        let _ = append(home, &line);
    }
}

/// Every panic, where it happened (`crate/src/file.rs:line`) and on which thread; never its
/// message, which may carry any text. And GPUI's own errors and warnings from its macOS layer
/// (`log_err`), which went nowhere: where, and for its window and display link (fixed words and
/// a code, e.g. "could not create display link, code: -6661") the first line said.
pub(crate) fn arm() {
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let at = info.location().map(|at| format!("{}:{}", short(at.file()), at.line()));
        let thread = std::thread::current().name().map(String::from);
        // Not `config::get`: the panic may be in reading it.
        write("panic", json!({ "at": at, "thread": thread }), config::loaded());
        before(info);
    }));
    if log::set_logger(&Gpui).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
}

/// The part of a source path from its crate's folder: `/Users/me/.cargo/registry/src/../gpui-0.2.2/src/window.rs`
/// gives `gpui-0.2.2/src/window.rs`. No user's name, no folder of theirs.
fn short(file: &str) -> &str {
    let Some(src) = file.rfind("/src/") else { return file.rsplit('/').next().unwrap_or(file) };
    file[..src].rfind('/').map_or(file, |start| &file[start + 1..])
}

/// GPUI's records from its macOS layer, as `gpui` events.
struct Gpui;

impl log::Log for Gpui {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        let file = record.file().unwrap_or_default();
        if !self.enabled(record.metadata()) || !(file.contains("/gpui-") && file.contains("/src/platform/mac/")) {
            return;
        }
        let at = format!("{}:{}", short(file), record.line().unwrap_or(0));
        // Words only from these two: their errors are GPUI's own fixed text and a code.
        let said = (file.ends_with("/window.rs") || file.ends_with("/display_link.rs")).then(|| {
            let text = record.args().to_string();
            text.lines().next().unwrap_or_default().chars().take(120).collect::<String>()
        });
        event("gpui", json!({ "level": record.level().as_str(), "at": at, "said": said }));
    }

    fn flush(&self) {}
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

/// Appends `line` to `dir/log`, turning a full file over first. Never through a link, nor into
/// anything but a plain file (a pipe would hang). One at a time: a panic's thread and the main
/// one could both turn a full file over, and the second would lose the first's `log.1`.
fn append(dir: &Path, line: &str) -> io::Result<()> {
    static ONE: Mutex<()> = Mutex::new(());
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if !dir.is_dir() {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let path = dir.join(NAME);
    if fs::symlink_metadata(&path).is_ok_and(|meta| meta.len() >= LIMIT) {
        // A rename replaces `log.1` itself, a link there included.
        fs::rename(&path, dir.join(OLD))?;
    }
    let flags = libc::O_NOFOLLOW | libc::O_NONBLOCK;
    let mut file = fs::OpenOptions::new().append(true).create(true).mode(0o600).custom_flags(flags).open(&path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("not a plain file"));
    }
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
    fn a_path_keeps_its_crate_and_never_the_folders_above() {
        let gpui = "/Users/me/.cargo/registry/src/index.crates.io-1949/gpui-0.2.2/src/platform/mac/window.rs";
        assert_eq!(short(gpui), "gpui-0.2.2/src/platform/mac/window.rs");
        assert_eq!(short("crates/ottospace/src/main.rs"), "ottospace/src/main.rs");
        assert_eq!(short("/rustc/abc/library/core/src/option.rs"), "core/src/option.rs");
        assert_eq!(short("/Users/me/build.rs"), "build.rs");
    }

    #[test]
    fn the_log_is_never_written_into_a_pipe() {
        let dir = temp("pipe");
        fs::create_dir_all(&dir.0).unwrap();
        let path = std::ffi::CString::new(dir.0.join(NAME).to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        // Returns at once, with no reader at the other end.
        assert!(append(&dir.0, "one").is_err());
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
