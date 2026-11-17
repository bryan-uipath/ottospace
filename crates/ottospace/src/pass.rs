//! A task agent's pass: 16 random bytes minted when ottospace starts the agent. A socket request
//! that carries it is that task's (`docs/ai/agent-mode.md`, "Who is asking"). The agent's
//! environment has the path of a file that holds it (`VAR`), not the pass: an agent that runs
//! `env` prints a path, which grants nothing a same-user process could not read already. The file
//! is 0600 in a 0700 folder ottospace makes beside its socket, one per process, gone when the
//! pass is. A secret: never in the state, a trace or a log; no `Serialize`, and its `Debug` hides
//! it. Its text is also hidden from what clients read (`hide`), should an agent print the file.

use std::borrow::Cow;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ottospace_core::tasks::{Caller, TaskId};

/// The variable the file's path travels in. No `KEY`, `TOKEN` or `SECRET` in the name: Codex's
/// tool shell may drop such names (agent-mode.md, "Probes").
pub(crate) const VAR: &str = "OTTOSPACE_TASK_PASS_FILE";

/// What a client sends when its file is gone (its agent ended): a pass never minted, so the
/// request is refused as a dead pass's, never answered as one with no pass.
const GONE: &str = "00000000000000000000000000000000";

pub(crate) struct Pass([u8; 16]);

/// A minted pass, live while held: its file is there, and its text is hidden (`hide`) from what
/// clients read, until it is dropped, which revokes it.
pub(crate) struct Live {
    pass: Pass,
    file: PathBuf,
}

/// The live passes' hex, for `hide`. Only `Live` adds and removes.
static LIVE: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The folder pass files go in: beside the control socket, so a test run's is in its own folder.
pub(crate) fn folder() -> PathBuf {
    crate::control::path().with_extension("passes")
}

/// At start: a run before's files (it ended without dropping them) go, and so does the folder.
/// Only files are removed, and only from a folder: a link at its place is left alone.
pub(crate) fn sweep(dir: &Path) {
    if !std::fs::symlink_metadata(dir).is_ok_and(|meta| meta.is_dir()) {
        return;
    }
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_file()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let _ = std::fs::remove_dir(dir);
}

/// The folder for pass files, 0700: made with the first pass, gone with the last (`Live::drop`).
/// A link or anything else at its place makes no folder.
fn ready(dir: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(dir) {
        Ok(meta) if meta.is_dir() => std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)),
        Ok(_) => Err(std::io::Error::other("the pass folder's place holds something else")),
        Err(_) => std::fs::DirBuilder::new().mode(0o700).create(dir),
    }
}

impl Live {
    /// From the OS's random source, as session ids are, with its file in `folder()`.
    pub(crate) fn mint() -> Result<Self, String> {
        Self::mint_in(&folder())
    }

    /// With its file in `dir`, made if it is not there.
    pub(crate) fn mint_in(dir: &Path) -> Result<Self, String> {
        // The folder itself, never a link to elsewhere.
        ready(dir).map_err(|err| format!("no folder for the pass's file: {err}"))?;
        let mut bytes = [0u8; 16];
        let random = std::fs::File::open("/dev/urandom").and_then(|mut source| source.read_exact(&mut bytes));
        // All zeros is `GONE`'s: never a live pass.
        if random.is_err() || bytes == [0; 16] {
            return Err("no random bytes for a pass".into());
        }
        let pass = Pass(bytes);
        static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let file = dir.join(format!("pass-{}-{n}", std::process::id()));
        // A new file, never one that is there (or a link): 0600 from its first byte.
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&file)
            .and_then(|mut out| out.write_all(pass.hex().as_bytes()));
        written.map_err(|err| format!("the pass's file: {err}"))?;
        LIVE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(pass.hex());
        Ok(Self { pass, file })
    }

    pub(crate) fn pass(&self) -> &Pass {
        &self.pass
    }

    /// Its file's path: what the agent's environment holds.
    pub(crate) fn file(&self) -> &Path {
        &self.file
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.file);
        let hex = self.pass.hex();
        let mut live = LIVE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(at) = live.iter().position(|one| *one == hex) {
            live.swap_remove(at);
        }
        // The last one: its folder goes too, if nothing else is in it.
        if let Some(dir) = self.file.parent().filter(|_| live.is_empty()) {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

impl std::fmt::Debug for Live {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Live(..)")
    }
}

/// Client side (`ottospace ctl`): the pass of the file `$OTTOSPACE_TASK_PASS_FILE` names. None:
/// no variable. A file gone or unreadable: `GONE`, refused as a dead pass.
pub(crate) fn from_env() -> Option<String> {
    let path = std::env::var_os(VAR).filter(|path| !path.is_empty())?;
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let text = text.trim();
    Some(if Pass::parse(text).is_some() { text.to_string() } else { GONE.to_string() })
}

/// What a live pass reads as wherever a client could read it.
pub(crate) const HIDDEN: &str = "[hidden]";

/// The live passes' hex. A lock a panic poisoned still holds them: hiding must not fail open.
fn live() -> Vec<String> {
    LIVE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// Where a whole live pass is in `text`, in either case: byte ranges, sorted, merged where they
/// overlap. Only whole passes: what is hidden then never says whether a part of one was guessed.
fn found(text: &str) -> Vec<std::ops::Range<usize>> {
    if text.len() < 32 {
        return Vec::new();
    }
    let live = live();
    if live.is_empty() {
        return Vec::new();
    }
    // ASCII lower case keeps every byte where it was: a match's range is the original's.
    let lower = text.to_ascii_lowercase();
    let mut found: Vec<_> =
        live.iter().flat_map(|hex| lower.match_indices(hex.as_str())).map(|(at, hex)| at..at + hex.len()).collect();
    found.sort_by_key(|range| range.start);
    merged(found)
}

/// Sorted ranges, those that overlap made one. Two that only touch (two passes side by side) stay two.
fn merged(sorted: Vec<std::ops::Range<usize>>) -> Vec<std::ops::Range<usize>> {
    let mut out: Vec<std::ops::Range<usize>> = Vec::with_capacity(sorted.len());
    for range in sorted {
        match out.last_mut() {
            Some(last) if range.start < last.end => last.end = last.end.max(range.end),
            _ => out.push(range),
        }
    }
    out
}

/// `text` with every live pass as `HIDDEN`: an agent that prints its pass (`env`) puts it on a
/// screen any client reads, in the state, a capture, a wait.
pub(crate) fn hide(text: &str) -> Cow<'_, str> {
    let found = found(text);
    if found.is_empty() {
        return Cow::Borrowed(text);
    }
    let (mut out, mut from) = (String::with_capacity(text.len()), 0);
    for range in found {
        out.push_str(&text[from..range.start]);
        out.push_str(HIDDEN);
        from = range.end;
    }
    out.push_str(&text[from..]);
    Cow::Owned(out)
}

/// A grid's rows with every live pass hidden, also one printed across rows: wrapped, broken by
/// the program's own line breaks or cursor moves, on the alternate screen. The rows are searched
/// as one text, each row's leading spaces left out of it (a continuation indented). A pass's
/// first row shows `HIDDEN`; its rest, on that row and later ones, goes. Whole passes only: no
/// part of one decides what is hidden. The caller gives rows past the edges of what it shows
/// (`terminal::lines_of`), so a pass the edge cuts is searched whole.
pub(crate) fn hide_rows(rows: Vec<String>) -> Vec<String> {
    if live().is_empty() {
        return rows;
    }
    // The searched text, and where each row starts in it: (its first byte there, the leading
    // bytes left out).
    let (mut joined, mut starts) = (String::new(), Vec::with_capacity(rows.len()));
    for (n, row) in rows.iter().enumerate() {
        let skip = if n == 0 { 0 } else { row.len() - row.trim_start().len() };
        starts.push((joined.len(), skip));
        joined.push_str(&row[skip..]);
    }
    let found = found(&joined);
    if found.is_empty() {
        return rows;
    }
    // A byte of `joined`, as (row, byte in the row).
    let place = |at: usize| {
        let n = starts.partition_point(|(start, _)| *start <= at) - 1;
        (n, starts[n].1 + at - starts[n].0)
    };
    // Per row: the bytes hidden, and where a pass's `HIDDEN` goes.
    let mut gone: Vec<Vec<bool>> = rows.iter().map(|row| vec![false; row.len()]).collect();
    let mut marks: Vec<(usize, usize)> = Vec::new();
    for range in found {
        marks.push(place(range.start));
        for at in range {
            let (n, at) = place(at);
            gone[n][at] = true;
        }
    }
    rows.iter()
        .enumerate()
        .map(|(n, row)| {
            let mut out = String::with_capacity(row.len());
            for (at, c) in row.char_indices() {
                if marks.contains(&(n, at)) {
                    out.push_str(HIDDEN);
                }
                if !gone[n][at] {
                    out.push(c);
                }
            }
            out
        })
        .collect()
}

/// `text` with a run of 8 or more hex digits at its start as `HIDDEN`, whatever the digits: for
/// text a cut may have left the end of a pass at (`terminal::cut_history`). It looks at no pass,
/// so what it hides says nothing of one.
pub(crate) fn hide_digits_at_start(text: &str) -> String {
    let run = text.bytes().take_while(u8::is_ascii_hexdigit).count();
    if run < 8 { text.to_string() } else { format!("{HIDDEN}{}", &text[run..]) }
}

impl Pass {
    /// 32 lowercase hex digits, for the agent's environment.
    pub(crate) fn hex(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Exactly 32 hex digits, else `None`.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let text = text.as_bytes();
        // Digits only: `from_str_radix` alone would take "+f".
        if text.len() != 32 || !text.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        let mut bytes = [0u8; 16];
        for (byte, pair) in bytes.iter_mut().zip(text.chunks(2)) {
            *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
        }
        Some(Self(bytes))
    }

    /// The same pass, in time that does not depend on where they differ.
    pub(crate) fn matches(&self, other: &Pass) -> bool {
        let diff = self.0.iter().zip(other.0).fold(0u8, |diff, (a, b)| diff | (a ^ b));
        std::hint::black_box(diff) == 0
    }
}

impl std::fmt::Debug for Pass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Pass(..)")
    }
}

/// Who `given` says the caller is, among the live agents' passes and their tasks. Every pass is
/// compared, a match or not.
pub(crate) fn caller<'a>(given: Option<&Pass>, live: impl IntoIterator<Item = (&'a Pass, TaskId)>) -> Caller {
    let Some(given) = given else { return Caller::Anyone };
    let found = live.into_iter().fold(None, |found, (pass, task)| match given.matches(pass) {
        true => Some(task),
        false => found,
    });
    found.map_or(Caller::Unknown, Caller::Task)
}

/// Tests only: a folder for pass files, as `sweep` makes it, gone when the test ends.
#[cfg(test)]
pub(crate) fn minted_dir(name: &str) -> crate::scratch::Scratch {
    let dir = crate::scratch::Scratch::path(&format!("ottospace-test-{name}-{}", std::process::id()));
    ready(dir.as_ref()).unwrap();
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file holds the pass, 0600 in a 0700 folder, and goes with it; `ottospace ctl` reads it,
    /// and a file gone reads as a pass never minted: refused as dead, not taken for no pass.
    #[test]
    fn a_pass_lives_in_its_own_file_until_it_is_dropped() {
        let dir = minted_dir("pass-file");
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let live = Live::mint_in(dir.as_ref()).unwrap();
        let file = live.file().to_path_buf();
        assert_eq!((mode(dir.as_ref()), mode(&file)), (0o700, 0o600));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), live.pass().hex());
        assert!(!file.to_string_lossy().contains(&live.pass().hex()), "the path says nothing of the pass");
        let read = |path: &Path| {
            // SAFETY: this test alone sets the variable, and reads it back at once.
            unsafe { std::env::set_var(VAR, path) };
            let read = from_env();
            unsafe { std::env::remove_var(VAR) };
            read
        };
        assert_eq!(read(&file), Some(live.pass().hex()));
        drop(live);
        assert!(!file.exists());
        assert_eq!(read(&file).as_deref(), Some(GONE));
        assert!(Pass::parse(GONE).is_some(), "GONE parses, so it is refused as a dead pass");
        assert_eq!(from_env(), None, "no variable: no pass");
        // A run before's files are swept at start, the folder with them; a link at the folder's
        // place makes no folder, and is left alone.
        ready(dir.as_ref()).unwrap();
        std::fs::write(dir.as_ref().join("pass-1-0"), "old").unwrap();
        sweep(dir.as_ref());
        assert!(!dir.as_ref().exists());
        let link = minted_dir("pass-link");
        std::fs::remove_dir(link.as_ref()).unwrap();
        ready(dir.as_ref()).unwrap();
        std::os::unix::fs::symlink(dir.as_ref(), link.as_ref()).unwrap();
        assert!(Live::mint_in(link.as_ref()).is_err());
        sweep(link.as_ref());
        assert!(dir.as_ref().exists(), "a link's target is not swept");
    }

    #[test]
    fn a_pass_is_random_hidden_and_names_only_its_own_task() {
        let dir = minted_dir("one-pass");
        let (one, two) = (Live::mint_in(dir.as_ref()).unwrap(), Live::mint_in(dir.as_ref()).unwrap());
        assert_eq!(format!("{one:?}"), "Live(..)");
        let (one, two) = (one.pass(), two.pass());
        assert!(!one.matches(two), "two mints, two passes");
        let hex = one.hex();
        assert_eq!(hex.len(), 32);
        assert!(Pass::parse(&hex).is_some_and(|back| back.matches(one)));
        // Its Debug says nothing of it.
        assert_eq!(format!("{one:?}"), "Pass(..)");
        assert!(!format!("{:?}", Some(one)).contains(&hex[..8]));
        // Only 32 hex digits are a pass.
        for bad in ["", "00", &hex[..31], &format!("{hex}0"), &"g".repeat(32), &"é".repeat(16), &"+f".repeat(16)] {
            assert!(Pass::parse(bad).is_none(), "{bad}");
        }
        // Upper case is the same pass.
        assert!(Pass::parse(&hex.to_uppercase()).is_some_and(|back| back.matches(one)));
        // No pass: anyone. A live one: its task. Any other, or one revoked (no longer live): unknown.
        let live = [(one, 7)];
        assert_eq!(caller(None, live), Caller::Anyone);
        assert_eq!(caller(Some(one), live), Caller::Task(7));
        assert_eq!(caller(Some(two), live), Caller::Unknown);
        assert_eq!(caller(Some(one), []), Caller::Unknown);
        let flipped = Pass::parse(&format!("{}{}", &hex[..31], if hex.ends_with('0') { '1' } else { '0' }));
        assert_eq!(caller(flipped.as_ref(), live), Caller::Unknown);
    }

    #[test]
    fn a_live_pass_is_hidden_in_any_case_and_across_rows_until_dropped() {
        let dir = minted_dir("hidden-pass");
        let live = Live::mint_in(dir.as_ref()).unwrap();
        let hex = live.pass().hex();
        let text = format!("OTTOSPACE_TASK_PASS={hex}\nagain {} done", hex.to_uppercase());
        assert_eq!(hide(&text), "OTTOSPACE_TASK_PASS=[hidden]\nagain [hidden] done");
        assert!(matches!(hide("nothing here, nothing at all, not one"), Cow::Borrowed(_)));
        // Across rows, as a narrow screen wraps it; the rows around kept.
        let row = |text: &str| text.to_string();
        let pass = format!("PASS={}", &hex[..10]);
        let rows = vec![row("$ env"), row(&pass), row(&hex[10..]), row("$")];
        assert_eq!(hide_rows(rows), ["$ env", "PASS=[hidden]", "", "$"]);
        assert_eq!(hide_rows(vec![row(&format!("é {hex} é"))]), ["é [hidden] é"]);
        // Broken by the program itself, indented, over three rows (a TUI, a narrow pane): one pass.
        let rows = vec![row(&format!("key: {}", &hex[..10])), row(&format!("     {}", &hex[10..20])), row(&hex[20..])];
        assert_eq!(hide_rows(rows), ["key: [hidden]", "     ", ""]);
        // Only whole passes: what a client typed hides the same whether a part of it is right or
        // wrong, at the end of the text, its start or within. No oracle, one digit at a time.
        let wrong = "0123456789abcdef0123456789abcdef";
        for n in [8, 12, 31] {
            for (part, other) in [(&hex[..n], &wrong[..n]), (&hex[32 - n..], &wrong[32 - n..])] {
                for text in [format!("$ echo {part}"), format!("{part} $"), format!("a {part} b")] {
                    assert_eq!(hide(&text), text);
                    assert_eq!(hide_rows(vec![row(&text)]), [text.as_str()]);
                }
                assert_eq!(hide(other), other);
            }
        }
        assert_eq!(hide(wrong), wrong, "a made-up one is not live");
        // A run of digits a cut left at the start is hidden whatever its digits.
        assert_eq!(hide_digits_at_start(&format!("{} $", &hex[20..])), "[hidden] $");
        assert_eq!(hide_digits_at_start(&format!("{} $", &wrong[20..])), "[hidden] $");
        assert_eq!(hide_digits_at_start("abc1234 $"), "abc1234 $", "7 digits");
        // Revoked: no longer hidden (and no longer a pass).
        drop(live);
        assert_eq!(hide(&hex), hex);
    }

    #[test]
    fn ranges_that_overlap_are_one_in_either_order() {
        assert_eq!(merged(vec![0..32, 20..40]), [0..40]);
        assert_eq!(merged(vec![0..10, 5..8, 9..20, 30..40]), [0..20, 30..40]);
        assert_eq!(merged(vec![0..10, 10..20]), [0..10, 10..20], "touching: two");
        assert_eq!(merged(vec![0..40, 10..20]), [0..40]);
        // Two live passes side by side in the text: both hidden, each its own word.
        let dir = minted_dir("two-passes");
        let (one, two) = (Live::mint_in(dir.as_ref()).unwrap(), Live::mint_in(dir.as_ref()).unwrap());
        let (one, two) = (one.pass().hex(), two.pass().hex());
        let text = format!("{one}{two} {two}{one}");
        assert_eq!(hide(&text), "[hidden][hidden] [hidden][hidden]");
    }
}
