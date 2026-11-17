//! A transcript's Markdown file (`ottospace_core::listen::file` says what goes in it): made with
//! its header when the transcript starts, a line appended and flushed as each final line comes,
//! and written again once at the end, for the header's `Ended`. In `transcripts_dir`; a folder
//! made here is 0700, each file is made 0600. The writes run on a thread of their own.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;

use ottospace_core::listen::Record;
use ottospace_core::listen::file::{self, Head};

/// The zone east of UTC, in seconds, at `ms`.
fn offset(ms: u64) -> i64 {
    ottospace_mac::time::utc_offset((ms / 1_000) as i64)
}

/// A clash this many times over is no clash: something else is wrong.
const TRIES: usize = 100;

/// The file of the transcript on, written on its thread.
pub(crate) struct Writer {
    sender: mpsc::Sender<Vec<Record>>,
    thread: JoinHandle<()>,
}

/// Makes `dir` (0700, with the folders above it) if it is not there, and the transcript's file
/// in it (0600) with its header. `Err`: why not, for the notice.
pub(crate) fn begin(dir: &Path, head: Head) -> Result<Writer, String> {
    if !dir.is_dir() {
        DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(|err| err.to_string())?;
    }
    let (path, mut made) = make(dir, &head)?;
    made.write_all(file::header(&head, None, &offset).as_bytes()).map_err(|err| err.to_string())?;
    let (sender, receiver) = mpsc::channel();
    let thread = std::thread::Builder::new()
        .name("transcript-file".into())
        .spawn(move || write(receiver, made, path, head))
        .map_err(|err| err.to_string())?;
    Ok(Writer { sender, thread })
}

/// The first free name of the transcript's, made new with mode 0600.
fn make(dir: &Path, head: &Head) -> Result<(PathBuf, File), String> {
    for name in file::names(head, offset(head.started)).take(TRIES) {
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path) {
            Ok(made) => return Ok((path, made)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err.to_string()),
        }
    }
    Err("no free file name".into())
}

impl Writer {
    /// The records of a batch, in order. An end or a failure is the last.
    pub(crate) fn send(&self, records: Vec<Record>) {
        if !records.is_empty() {
            let _ = self.sender.send(records);
        }
    }

    /// Waits for what was sent to be written: ottospace is quitting.
    pub(crate) fn finish(self) {
        let Writer { sender, thread } = self;
        drop(sender);
        let _ = thread.join();
    }
}

/// The thread: each line appended and flushed; at the end the whole file again, through a
/// temporary file in the same folder renamed over it; with no line and no start, the file goes.
/// Ends when the transcript does, or when ottospace quits (the header then says it is still on:
/// read as ended at its last line).
fn write(receiver: mpsc::Receiver<Vec<Record>>, mut made: File, path: PathBuf, head: Head) {
    let mut body = String::new();
    let failed = |what: &str, err: io::Error| {
        crate::trace::record(
            "transcript_file",
            serde_json::json!({ "failed": what, "kind": format!("{:?}", err.kind()) }),
        );
    };
    for record in receiver.iter().flatten() {
        match record {
            Record::End { at } => {
                if let Err(err) = rewrite(&path, &head, at, &body) {
                    failed("end", err);
                }
                return;
            }
            Record::Failed => {
                if body.is_empty()
                    && let Err(err) = fs::remove_file(&path)
                {
                    failed("remove", err);
                }
                return;
            }
            // A line that fails to write is not traced: when one is written says when someone spoke.
            record => {
                let Some(line) = file::line(&record, &offset) else { continue };
                let _ = made.write_all(line.as_bytes()).and_then(|()| made.flush());
                body += &line;
            }
        }
    }
}

/// The file again, with its end in the header.
fn rewrite(path: &Path, head: &Head, at: u64, body: &str) -> io::Result<()> {
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let temp = path.with_file_name(format!(".{name}.tmp"));
    // A temporary file a crash left keeps its mode: made new, so it is 0600.
    let _ = fs::remove_file(&temp);
    let mut out = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temp)?;
    out.write_all(file::header(head, Some(at), &offset).as_bytes())?;
    out.write_all(body.as_bytes())?;
    out.sync_all()?;
    fs::rename(&temp, path)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use ottospace_core::listen::Speaker;

    use super::*;

    /// A folder of the test's own, removed when it ends.
    struct Temp(PathBuf);

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn temp(name: &str) -> Temp {
        Temp(std::env::temp_dir().join(format!("ottospace-test-listen-file-{name}-{}", std::process::id())))
    }

    fn head(started: u64) -> Head {
        Head { app: "Canary".into(), bundle: Some("dev.ottospace.canary".into()), mic: false, started }
    }

    fn files(dir: &Path) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().path()).collect();
        files.sort();
        files
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_file_is_made_private_written_as_lines_come_and_ended_once() {
        let temp = temp("lines");
        let dir = temp.0.join("notes/transcripts");
        let started = 1_794_819_600_000;
        let writer = begin(&dir, head(started)).unwrap();
        assert_eq!((mode(&dir), mode(&temp.0.join("notes"))), (0o700, 0o700));
        let path = files(&dir).remove(0);
        assert_eq!(mode(&path), 0o600);
        let hostile = "[link](http://example.com) <b>bold</b>\n# heading";
        writer.send(vec![Record::Line { from: Speaker::Them, at: started + 12_000, text: hostile.into() }]);
        writer.send(vec![Record::Paused { at: started + 20_000, ms: 180_000 }]);
        writer.send(vec![Record::Line { from: Speaker::Me, at: started + 200_000, text: "made-up words".into() }]);
        // Written as they come: the header still says it is on.
        let written = || fs::read_to_string(&path).unwrap();
        let wait = std::time::Instant::now();
        while !written().contains("made-up words") && wait.elapsed().as_secs() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(written().contains(&format!("- Ended: {}\n", file::STILL)), "{}", written());
        writer.send(vec![Record::End { at: started + 47 * 60_000 }]);
        writer.finish();
        let text = written();
        assert!(text.contains(", 47 minutes\n") && !text.contains(file::STILL), "{text}");
        assert_eq!(mode(&path), 0o600, "the rewrite keeps it private");
        assert_eq!(files(&dir), [path.clone()], "no temporary file is left");
        // Heard words are text: no link, image, HTML or heading but the title.
        let parser = pulldown_cmark::Parser::new(&text);
        let mut headings = 0;
        for event in parser {
            use pulldown_cmark::{Event, Tag};
            match event {
                Event::Start(Tag::Heading { .. }) => headings += 1,
                Event::Start(Tag::Link { .. } | Tag::Image { .. }) | Event::Html(_) | Event::InlineHtml(_) => {
                    panic!("{event:?} in {text}")
                }
                _ => {}
            }
        }
        assert_eq!(headings, 1, "{text}");
        let lines: Vec<&str> = text.lines().filter(|line| line.starts_with("- ") && line.contains("**")).collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert!(text.contains("*paused 3 minutes*"), "{text}");
    }

    #[test]
    fn a_clash_takes_the_next_name_and_a_folder_there_keeps_its_mode() {
        let temp = temp("clash");
        let dir = temp.0.clone();
        DirBuilder::new().mode(0o755).create(&dir).unwrap();
        let first = begin(&dir, head(1_794_819_600_000)).unwrap();
        let second = begin(&dir, head(1_794_819_600_000)).unwrap();
        first.finish();
        second.finish();
        let names: Vec<String> = files(&dir).iter().map(|p| p.file_name().unwrap().to_string_lossy().into()).collect();
        assert_eq!(names.len(), 2);
        assert!(names[0].ends_with("-canary-2.md") && names[1].ends_with("-canary.md"), "{names:?}");
        assert_eq!(mode(&dir), 0o755);
    }

    #[test]
    fn one_that_never_began_keeps_no_file_and_a_folder_that_cannot_be_made_says_why() {
        let temp = temp("failed");
        let writer = begin(&temp.0, head(1_794_819_600_000)).unwrap();
        writer.send(vec![Record::Failed]);
        writer.finish();
        assert_eq!(files(&temp.0), Vec::<PathBuf>::new());
        fs::write(temp.0.join("file"), "").unwrap();
        assert!(begin(&temp.0.join("file/under"), head(0)).is_err());
    }
}
