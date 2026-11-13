//! `<home>/running`: the running app's pid and program path, one per line. `osi` reads it to say
//! whether an install left the old build running. A subcommand (`ctl`, `--version`) writes none.

use std::path::{Path, PathBuf};

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

/// The user's folder, outside a test run: a test never writes it, nor touches the user's.
fn home() -> Option<&'static Path> {
    if std::env::var_os("OTTOSPACE_E2E").is_some() {
        return None;
    }
    config::get().home.as_deref()
}

/// Says this app is running. A second instance writes over the first one's file.
pub fn write() {
    let (Some(home), Ok(program)) = (home(), std::env::current_exe()) else { return };
    let program = program.canonicalize().unwrap_or(program);
    let _ = config::write_private(home, NAME, text(std::process::id(), &program).as_bytes());
}

/// On a clean quit: removes the file if it is still this app's.
pub fn remove() {
    let Some(home) = home() else { return };
    let file = home.join(NAME);
    let ours =
        std::fs::read_to_string(&file).ok().and_then(|t| parse(&t)).is_some_and(|(pid, _)| pid == std::process::id());
    if ours {
        let _ = std::fs::remove_file(file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
