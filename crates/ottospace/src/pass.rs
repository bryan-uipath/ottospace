//! A task agent's pass: 16 random bytes minted when ottospace starts the agent. A socket request
//! that carries it is that task's (`docs/ai/agent-mode.md`, "Who is asking"). The agent's
//! environment has the path of a file that holds it (`VAR`), not the pass: an agent that runs
//! `env` prints a path, which grants nothing a same-user process could not read already. The file
//! is 0600 in a 0700 folder ottospace makes beside its socket, one per process, gone when the
//! pass is. A secret: never in the state, a trace or a log; no `Serialize`, and its `Debug` hides
//! it. Its text is also hidden from what clients read (`hide`), should an agent print the file.

use std::borrow::Cow;
use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::DirBuilderExt;
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
    /// Its folder, and its file's name there: removed through the folder, never by path.
    folder: Folder,
    name: CString,
}

/// The live passes' hex, for `hide`. Only `Live` adds and removes.
static LIVE: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The folder pass files go in: beside the control socket, so a test run's is in its own folder.
pub(crate) fn folder() -> PathBuf {
    crate::control::path().with_extension("passes")
}

/// The pass folder, opened once and used through its fd from then on: what is checked is what
/// is listed, written and unlinked, even if its path is swapped for a link meanwhile.
struct Folder(OwnedFd);

impl Folder {
    /// `dir` itself, never a link (`O_NOFOLLOW`), and only if it is `uid`'s, mode 0700: else None.
    fn open(dir: &Path, uid: libc::uid_t) -> Option<Self> {
        let path = CString::new(dir.as_os_str().as_bytes()).ok()?;
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        // SAFETY: `path` is a valid C string; the fd is owned from here on.
        let fd = unsafe { libc::open(path.as_ptr(), flags) };
        let folder = Self((fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })?);
        let meta = folder.stat(c".")?;
        let ours = meta.st_uid == uid && meta.st_mode & libc::S_IFMT == libc::S_IFDIR && meta.st_mode & 0o7777 == 0o700;
        ours.then_some(folder)
    }

    /// `name` in the folder, a link not followed.
    fn stat(&self, name: &CStr) -> Option<libc::stat> {
        // SAFETY: a zeroed `stat` is valid; `fstatat` fills it.
        let mut meta: libc::stat = unsafe { std::mem::zeroed() };
        let found = unsafe { libc::fstatat(self.0.as_raw_fd(), name.as_ptr(), &mut meta, libc::AT_SYMLINK_NOFOLLOW) };
        (found == 0).then_some(meta)
    }

    /// The entries' names, `.` and `..` left out. Read from a fresh open of the fd's folder, so
    /// each listing starts at its top.
    fn names(&self) -> Vec<CString> {
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC;
        // SAFETY: `fdopendir` takes the new fd, `closedir` closes it; names are copied before.
        let fd = unsafe { libc::openat(self.0.as_raw_fd(), c".".as_ptr(), flags) };
        if fd < 0 {
            return Vec::new();
        }
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            unsafe { libc::close(fd) };
            return Vec::new();
        }
        let mut names = Vec::new();
        loop {
            let entry = unsafe { libc::readdir(stream) };
            if entry.is_null() {
                break;
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            if name != c"." && name != c".." {
                names.push(name.to_owned());
            }
        }
        unsafe { libc::closedir(stream) };
        names
    }

    /// Unlinks `name` from the folder if it is a regular file. A link is neither followed nor removed.
    fn unlink_file(&self, name: &CStr) {
        if self.stat(name).is_some_and(|meta| meta.st_mode & libc::S_IFMT == libc::S_IFREG) {
            // SAFETY: a name in the folder's fd; `unlinkat` never follows its last component.
            unsafe { libc::unlinkat(self.0.as_raw_fd(), name.as_ptr(), 0) };
        }
    }

    /// A new file `name` in the folder, 0600 from its first byte; never one that is there, or a link.
    fn create(&self, name: &CStr) -> std::io::Result<std::fs::File> {
        let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        // SAFETY: a name in the folder's fd; the new fd is owned by the `File`.
        let fd = unsafe { libc::openat(self.0.as_raw_fd(), name.as_ptr(), flags, 0o600 as libc::c_uint) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) }))
    }
}

fn uid() -> libc::uid_t {
    // SAFETY: no arguments, cannot fail.
    unsafe { libc::geteuid() }
}

/// At start and at quit: a run's files (one before ended without dropping them) go, and so does
/// the folder. Only regular files, through the folder's fd, and only from the folder itself,
/// ours and 0700: a link at its place, or one in it, is left alone.
pub(crate) fn sweep(dir: &Path) {
    sweep_as(dir, uid());
}

/// `sweep`, the folder required to be `uid`'s.
fn sweep_as(dir: &Path, uid: libc::uid_t) {
    let Some(folder) = Folder::open(dir, uid) else { return };
    folder.names().iter().for_each(|name| folder.unlink_file(name));
    // `rmdir` takes only an empty folder, never a link.
    let _ = std::fs::remove_dir(dir);
}

/// The folder for pass files, 0700: made with the first pass, gone with the last (`Live::drop`).
/// A link or anything else at its place, or a folder not ours and 0700, makes no folder.
fn ready(dir: &Path) -> std::io::Result<Folder> {
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Err(err) if err.kind() != std::io::ErrorKind::AlreadyExists => return Err(err),
        _ => {}
    }
    Folder::open(dir, uid()).ok_or_else(|| std::io::Error::other("the pass folder's place holds something else"))
}

impl Live {
    /// From the OS's random source, as session ids are, with its file in `folder()`.
    pub(crate) fn mint() -> Result<Self, String> {
        Self::mint_in(&folder())
    }

    /// With its file in `dir`, made if it is not there.
    pub(crate) fn mint_in(dir: &Path) -> Result<Self, String> {
        // The folder itself, never a link to elsewhere.
        let folder = ready(dir).map_err(|err| format!("no folder for the pass's file: {err}"))?;
        let mut bytes = [0u8; 16];
        let random = std::fs::File::open("/dev/urandom").and_then(|mut source| source.read_exact(&mut bytes));
        // All zeros is `GONE`'s: never a live pass.
        if random.is_err() || bytes == [0; 16] {
            return Err("no random bytes for a pass".into());
        }
        let pass = Pass(bytes);
        static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let name = format!("pass-{}-{n}", std::process::id());
        let file = dir.join(&name);
        let name = CString::new(name).map_err(|err| err.to_string())?;
        let written = folder.create(&name).and_then(|mut out| out.write_all(pass.hex().as_bytes()));
        written.map_err(|err| format!("the pass's file: {err}"))?;
        LIVE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(pass.hex());
        Ok(Self { pass, file, folder, name })
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
        self.folder.unlink_file(&self.name);
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

/// Whether any pass is live: else nothing is hidden, not even `hide_digits_at_top`'s digits.
pub(crate) fn any_live() -> bool {
    !LIVE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_empty()
}

/// Where a whole live pass is in `text`, in either case: byte ranges, sorted, merged where they
/// overlap. Only whole passes: what is hidden then never says whether a part of one was guessed.
fn found(text: &str) -> Vec<std::ops::Range<usize>> {
    if text.len() < 32 {
        return Vec::new();
    }
    found_among(text, &live())
}

/// `found`, among the passes `live`.
fn found_among(text: &str, live: &[String]) -> Vec<std::ops::Range<usize>> {
    if live.is_empty() {
        return Vec::new();
    }
    // ASCII lower case keeps every byte where it was: a match's range is the original's.
    let lower = text.to_ascii_lowercase();
    // Every start, overlapping ones too: `match_indices` skips one that begins inside another.
    let mut found = Vec::new();
    for hex in live {
        let mut from = 0;
        while let Some(at) = lower[from..].find(hex.as_str()) {
            found.push(from + at..from + at + hex.len());
            from += at + 1;
        }
    }
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
/// the program's own line breaks or cursor moves, on the alternate screen, in a box. The rows are
/// searched as one text, each row's edges left out of it (an indent, a box's borders). A pass's
/// first row shows `HIDDEN`; its rest, on that row and later ones, goes. Whole passes only: no
/// part of one decides what is hidden. The caller gives rows past the edges of what it shows
/// (`terminal::lines_of`), so a pass the edge cuts is searched whole.
pub(crate) fn hide_rows(rows: Vec<String>) -> Vec<String> {
    if live().is_empty() {
        return rows;
    }
    // The searched text, and where each row starts in it: (its first byte there, the leading
    // bytes left out). Each row's edges (spaces, a box's borders) are left out of it.
    let (mut joined, mut starts) = (String::new(), Vec::with_capacity(rows.len()));
    for row in &rows {
        let end = row.trim_end_matches(edge).len();
        let skip = end - row[..end].trim_start_matches(edge).len();
        starts.push((joined.len(), skip));
        joined.push_str(&row[skip..end]);
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

/// Rows read from where a cut may have left the end of a pass (`terminal::cut_history`): a run
/// of 8 or more hex digits at their start, each row's indent left out and the run going on to the
/// next row while it fills a row, is `HIDDEN`, whatever the digits. It looks at no pass, so what
/// it hides says nothing of one.
pub(crate) fn hide_digits_at_top(mut rows: Vec<String>) -> Vec<String> {
    // Per row: (where its run starts, how long it is).
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for row in &rows {
        let at = row.len() - row.trim_start_matches(edge).len();
        let run = row[at..].bytes().take_while(u8::is_ascii_hexdigit).count();
        runs.push((at, run));
        // The run ends in this row: what follows it is not a digit.
        if run == 0 || at + run < row.trim_end_matches(edge).len() {
            break;
        }
    }
    if runs.iter().map(|(_, run)| run).sum::<usize>() < 8 {
        return rows;
    }
    for (n, (at, run)) in runs.into_iter().enumerate() {
        let mark = if n == 0 { HIDDEN } else { "" };
        rows[n].replace_range(at..at + run, mark);
    }
    rows
}

/// What may stand at a row's edges around a pass the program printed whole: spaces, and the
/// borders of a box drawn round it (an approval prompt's), U+2500 to U+257F.
pub(crate) fn edge(c: char) -> bool {
    c == ' ' || ('\u{2500}'..='\u{257f}').contains(&c)
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
    use std::os::unix::fs::PermissionsExt;

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

    /// The sweep works through the folder's fd: a link at its place, a link in it, a folder of
    /// another mode or owner: nothing elsewhere is removed.
    #[test]
    fn a_sweep_removes_only_files_in_our_own_folder() {
        let outside = crate::scratch::Scratch::dir(&format!("ottospace-test-sweep-outside-{}", std::process::id()));
        let target = outside.join("keep");
        std::fs::write(&target, "keep").unwrap();
        // A link in the folder: the file it points to survives, and so does the link.
        let dir = minted_dir("sweep-entry-link");
        std::os::unix::fs::symlink(&target, dir.join("pass-1-0")).unwrap();
        std::fs::write(dir.join("pass-1-1"), "old").unwrap();
        sweep(dir.as_ref());
        assert!(target.exists(), "a link's target is not removed");
        assert!(dir.join("pass-1-0").symlink_metadata().is_ok(), "nor the link");
        assert!(!dir.join("pass-1-1").exists(), "a file is");
        // A link at the folder's place: its target's files stay.
        let real = minted_dir("sweep-real");
        std::fs::write(real.join("pass-2-0"), "old").unwrap();
        let link = crate::scratch::Scratch::path(&format!("ottospace-test-sweep-link-{}", std::process::id()));
        std::os::unix::fs::symlink(real.as_ref(), link.as_ref()).unwrap();
        sweep(link.as_ref());
        assert!(real.join("pass-2-0").exists(), "a linked folder is refused");
        assert!(Live::mint_in(link.as_ref()).is_err());
        // Another mode, or another owner: left alone, and no pass is minted in it.
        let open = crate::scratch::Scratch::dir(&format!("ottospace-test-sweep-mode-{}", std::process::id()));
        std::fs::set_permissions(open.as_ref(), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(open.join("pass-3-0"), "old").unwrap();
        sweep(open.as_ref());
        assert!(open.join("pass-3-0").exists(), "0755: left alone");
        assert!(Live::mint_in(open.as_ref()).is_err());
        std::fs::write(real.join("pass-4-0"), "old").unwrap();
        sweep_as(real.as_ref(), uid() + 1);
        assert!(real.join("pass-4-0").exists(), "another owner's: left alone");
        // Ours, 0700: every file goes, then the folder.
        sweep(real.as_ref());
        assert!(!real.exists());
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
        // A run of digits a cut left at the start is hidden whatever its digits, indented, or
        // over narrow rows.
        let top = |rows: &[&str]| hide_digits_at_top(rows.iter().map(|row| row.to_string()).collect());
        assert_eq!(top(&[&format!("{} $", &hex[20..])]), ["[hidden] $"]);
        assert_eq!(top(&[&format!("{} $", &wrong[20..])]), ["[hidden] $"]);
        assert_eq!(top(&["abc1234 $", "5678"]), ["abc1234 $", "5678"], "7 digits");
        assert_eq!(top(&[&format!("    {}", &hex[20..])]), ["    [hidden]"]);
        assert_eq!(top(&["abcd", " 1234", "56 $", "ff"]), ["[hidden]", " ", " $", "ff"]);
        assert_eq!(top(&["│ abcd │", "│ 1234 │"]), ["│ [hidden] │", "│  │"]);
        // In a box (an approval prompt), its borders and padding between the rows: one pass.
        let boxed = vec![row(&format!("│ PASS={} │", &hex[..12])), row(&format!("│ {}     │", &hex[12..]))];
        assert_eq!(hide_rows(boxed), ["│ PASS=[hidden] │", "│      │"]);
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
        // One that begins inside another (this pass's first digit is its last): one range.
        let hex = "a0123456789abcdef0123456789abcda".to_string();
        assert_eq!(found_among(&format!("{hex}{}", &hex[1..]), &[hex]), [0..63]);
    }
}
