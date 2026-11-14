//! `<home>/running`: the running app's pid and program path, one per line. `osi` reads it to say
//! whether an install left the old build running. A subcommand (`ctl`, `--version`) writes none.

use std::path::{Path, PathBuf};
use std::{fs, io};

use crate::config;

const NAME: &str = "running";

/// The file's text, e.g. "4242\n/Users/me/.local/bin/ottospace\n".
fn text(pid: u32, program: &Path) -> String {
    format!("{pid}\n{}\n", program.display())
}

/// The pid and program path in `text`; `None` when it is not two such lines.
fn parse(text: &str) -> Option<(u32, PathBuf)> {
    let mut lines = text.lines();
    let pid = lines.next()?.parse().ok()?;
    let program = PathBuf::from(lines.next()?);
    (program.is_absolute() && lines.next().is_none()).then_some((pid, program))
}

/// The pid the file in `dir` names, if it reads.
fn recorded(dir: &Path) -> Option<u32> {
    fs::read_to_string(dir.join(NAME)).ok().and_then(|text| parse(&text)).map(|(pid, _)| pid)
}

/// True while process `pid` exists, ours or another user's.
pub(crate) fn alive(pid: u32) -> bool {
    i32::try_from(pid).is_ok_and(ottospace_mac::process::alive)
}

/// Records process `pid` running `program` in `dir`, unless the file names another live process:
/// a second ottospace (a `cargo run`, say) must not hide the first one. A stale record is replaced.
fn claim(dir: &Path, pid: u32, program: &Path, alive: impl Fn(u32) -> bool) -> io::Result<()> {
    if recorded(dir).is_some_and(|other| other != pid && alive(other)) {
        return Ok(());
    }
    // Removed first, so the new file is 0600 whatever the old one's mode; a symlink there goes, unfollowed.
    config::removed(fs::remove_file(dir.join(NAME)))?;
    config::write_private(dir, NAME, text(pid, program).as_bytes())
}

/// Removes the file in `dir` if it names process `pid`.
fn release(dir: &Path, pid: u32) {
    if recorded(dir) == Some(pid) {
        let _ = fs::remove_file(dir.join(NAME));
    }
}

/// The user's folder, outside a test run: a test never writes it, nor touches the user's.
fn home() -> Option<&'static Path> {
    if std::env::var_os("OTTOSPACE_E2E").is_some() {
        return None;
    }
    config::get().home.as_deref()
}

/// At start: says this app is running.
pub fn write() {
    let (Some(home), Ok(program)) = (home(), std::env::current_exe()) else { return };
    let program = program.canonicalize().unwrap_or(program);
    let _ = claim(home, std::process::id(), &program, alive);
}

/// On a clean quit: removes the file if it is still this app's.
pub fn remove() {
    if let Some(home) = home() {
        release(home, std::process::id());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn the_file_reads_back() {
        let program = Path::new("/Users/me/.local/bin/ottospace");
        assert_eq!(text(4242, program), "4242\n/Users/me/.local/bin/ottospace\n");
        assert_eq!(parse(&text(4242, program)), Some((4242, program.to_path_buf())));
    }

    #[test]
    fn anything_else_is_not_read() {
        for bad in ["", "4242\n", "x\n/a\n", "4242\nrelative/path\n", "4242\n/a\nmore\n", "-1\n/a\n"] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }

    /// A fresh folder for one test.
    fn folder(name: &str) -> crate::scratch::Scratch {
        crate::scratch::Scratch::dir(&format!("ottospace-running-{name}-{}", std::process::id()))
    }

    fn read(dir: &Path) -> String {
        fs::read_to_string(dir.join(NAME)).unwrap()
    }

    #[test]
    fn a_live_other_process_keeps_its_record() {
        let dir = folder("live");
        claim(&dir, 10, Path::new("/first/ottospace"), |_| true).unwrap();
        claim(&dir, 20, Path::new("/second/ottospace"), |pid| pid == 10).unwrap();
        assert_eq!(read(&dir), "10\n/first/ottospace\n", "the first one is alive: its record stays");
        release(&dir, 20);
        assert_eq!(read(&dir), "10\n/first/ottospace\n", "the second one's quit leaves it");
        release(&dir, 10);
        assert!(!dir.join(NAME).exists(), "the first one's quit removes it");
    }

    #[test]
    fn a_stale_record_is_replaced() {
        let dir = folder("stale");
        claim(&dir, 10, Path::new("/first/ottospace"), |_| true).unwrap();
        claim(&dir, 20, Path::new("/second/ottospace"), |_| false).unwrap();
        assert_eq!(read(&dir), "20\n/second/ottospace\n");
        // A file that does not read counts as stale too.
        fs::write(dir.join(NAME), "junk").unwrap();
        claim(&dir, 30, Path::new("/third/ottospace"), |_| true).unwrap();
        assert_eq!(read(&dir), "30\n/third/ottospace\n");
    }

    #[test]
    fn the_file_is_0600_and_a_symlink_is_not_followed() {
        let dir = folder("mode");
        fs::write(dir.join(NAME), "10\n/old\n").unwrap();
        fs::set_permissions(dir.join(NAME), fs::Permissions::from_mode(0o644)).unwrap();
        claim(&dir, 20, Path::new("/new/ottospace"), |_| false).unwrap();
        assert_eq!(fs::metadata(dir.join(NAME)).unwrap().permissions().mode() & 0o777, 0o600);
        let target = dir.join("elsewhere");
        fs::write(&target, "untouched").unwrap();
        fs::remove_file(dir.join(NAME)).unwrap();
        std::os::unix::fs::symlink(&target, dir.join(NAME)).unwrap();
        claim(&dir, 30, Path::new("/new/ottospace"), |_| false).unwrap();
        assert!(!fs::symlink_metadata(dir.join(NAME)).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&target).unwrap(), "untouched");
    }

    #[test]
    fn this_process_is_alive_and_a_group_is_not_a_process() {
        assert!(alive(std::process::id()));
        assert!(!alive(0));
        assert!(!alive(u32::MAX));
    }
}
