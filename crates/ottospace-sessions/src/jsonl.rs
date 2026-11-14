//! Reading JSON-lines session files a runner may still be writing.

use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

/// A longer line is skipped without being held in memory.
pub(crate) const MAX_LINE: u64 = 64 << 20;

/// Opens a regular file. A FIFO or device named like a session file is an error, not a read that never returns.
pub(crate) fn open(path: &Path) -> io::Result<File> {
    let regular = |meta: fs::Metadata| meta.is_file().then_some(()).ok_or(io::ErrorKind::InvalidInput);
    regular(fs::metadata(path)?)?;
    let file = File::open(path)?;
    // And again on what was opened, in case the path changed in between.
    regular(file.metadata()?)?;
    Ok(file)
}

/// Calls `each` with every complete line after byte `offset`, until the end of the file, until
/// `budget` bytes were read, or until `each` returns true. Returns the new offset and whether
/// it stopped before the end. A last line without its newline is still being written: it is
/// left for the next call.
///
/// `skipping` says the bytes at `offset` are the rest of an oversized line; it is set when such
/// a line has no end yet, so the next call goes on from there and does not scan the line again.
pub(crate) fn lines(
    file: File,
    offset: u64,
    budget: u64,
    skipping: &mut bool,
    each: impl FnMut(&str) -> bool,
) -> io::Result<(u64, bool)> {
    lines_capped(file, offset, budget, MAX_LINE, skipping, each)
}

fn lines_capped(
    mut file: File,
    offset: u64,
    budget: u64,
    max_line: u64,
    skipping: &mut bool,
    mut each: impl FnMut(&str) -> bool,
) -> io::Result<(u64, bool)> {
    file.seek(SeekFrom::Start(offset))?;
    let mut reader = BufReader::new(file);
    let (start, mut offset) = (offset, offset);
    let mut line = Vec::new();
    loop {
        if *skipping {
            // Pass over the rest of an oversized line. Without its end yet, stop at what is there.
            let (skipped, ended) = skip_line(&mut reader)?;
            offset += skipped;
            *skipping = !ended;
            if !ended {
                return Ok((offset, false));
            }
        }
        if offset - start >= budget {
            return Ok((offset, true));
        }
        line.clear();
        let read = (&mut reader).take(max_line + 1).read_until(b'\n', &mut line)? as u64;
        if read == 0 {
            return Ok((offset, false));
        }
        if line.last() != Some(&b'\n') {
            if read <= max_line {
                return Ok((offset, false));
            }
            (offset, *skipping) = (offset + read, true);
            continue;
        }
        offset += read;
        // A line that is not UTF-8 is skipped like any other record nobody knows.
        if let Ok(text) = std::str::from_utf8(&line)
            && each(text.trim_end())
        {
            return Ok((offset, true));
        }
    }
}

/// Reads up to and past the next newline. Returns how many bytes that was, and whether a newline was there.
fn skip_line(reader: &mut impl BufRead) -> io::Result<(u64, bool)> {
    let mut skipped = 0;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok((skipped, false));
        }
        let newline = buffer.iter().position(|b| *b == b'\n');
        let used = newline.map_or(buffer.len(), |at| at + 1);
        reader.consume(used);
        skipped += used as u64;
        if newline.is_some() {
            return Ok((skipped, true));
        }
    }
}

/// The first complete lines of a file, parsed: at most `max_lines` lines and `max_bytes` bytes.
pub(crate) fn head(path: &Path, max_lines: usize, max_bytes: u64) -> io::Result<Vec<Value>> {
    let mut reader = BufReader::new(open(path)?.take(max_bytes));
    let mut records = Vec::new();
    let mut line = String::new();
    for _ in 0..max_lines {
        line.clear();
        // Invalid UTF-8 or the byte limit ends the head; what was read so far stands.
        if !matches!(reader.read_line(&mut line), Ok(n) if n > 0) || !line.ends_with('\n') {
            break;
        }
        records.extend(serde_json::from_str::<Value>(&line).ok());
    }
    Ok(records)
}

/// The complete lines in the last `max_bytes` of a file, oldest first.
pub(crate) fn tail(path: &Path, max_bytes: u64) -> io::Result<Vec<String>> {
    let mut file = open(path)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(max_bytes).read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.split('\n').collect();
    // The last piece is empty or half written; the first is cut unless the read began at the start.
    lines.pop();
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    Ok(lines.into_iter().map(str::to_string).collect())
}

/// A small regular file's text, e.g. a pid file. Not a pipe or device, and not a huge file.
pub(crate) fn small(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    (meta.is_file() && meta.len() < 65_536).then(|| fs::read_to_string(path).ok()).flatten()
}

/// An RFC 3339 UTC time in Unix milliseconds, e.g. `2026-11-12T10:44:00.123Z` → 1794480240123.
pub(crate) fn parse_time(text: &str) -> Option<u64> {
    let text = text.strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut date = date.split('-').map(|p| p.parse::<u64>().ok());
    let (year, month, day) = (date.next()??, date.next()??, date.next()??);
    let (clock, fraction) = time.split_once('.').unwrap_or((time, "0"));
    let mut clock = clock.split(':').map(|p| p.parse::<u64>().ok());
    let (hour, minute, second) = (clock.next()??, clock.next()??, clock.next()??);
    let in_range = (1970..=9999).contains(&year) && (1..=12).contains(&month) && (1..=31).contains(&day);
    if !in_range || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let millis = format!("{fraction:0<3}").get(..3)?.parse::<u64>().ok()?;
    // Days since 1970-01-01, by the usual civil-date arithmetic with years starting in March.
    let y = if month <= 2 { year - 1 } else { year };
    let (era, year_of_era) = (y / 400, y % 400);
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some((((days * 24 + hour) * 60 + minute) * 60 + second) * 1000 + millis)
}

/// A record's `timestamp` field in Unix milliseconds, or 0.
pub(crate) fn time_of(record: &Value) -> u64 {
    record["timestamp"].as_str().and_then(parse_time).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    /// A file path in the temp dir, removed when the test ends, pass or fail.
    fn temp(name: &str) -> Scratch {
        Scratch(std::env::temp_dir().join(format!("ottospace-sessions-{}-{name}", std::process::id())))
    }

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    impl std::ops::Deref for Scratch {
        type Target = Path;

        fn deref(&self) -> &Path {
            &self.0
        }
    }

    impl AsRef<Path> for Scratch {
        fn as_ref(&self) -> &Path {
            &self.0
        }
    }

    #[test]
    fn parses_times() {
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_time("2000-03-01T00:00:00.000Z"), Some(951_868_800_000));
        assert_eq!(parse_time("2024-02-29T12:30:15.5Z"), Some(1_709_209_815_500));
        assert_eq!(parse_time("2026-11-12T10:44:00.123456Z"), Some(1_794_480_240_123));
        assert_eq!(parse_time("9999-12-31T23:59:59.999Z"), Some(253_402_300_799_999));
        let bad = [
            "",
            "yesterday",
            "2026-11-12",
            "2026-13-01T00:00:00Z",
            "2026-11-12T10:44:00+01:00",
            "2026-11-12T1é:00:00Z",
            "10000-01-01T00:00:00Z",
            "18446744073709551615-01-01T00:00:00Z",
            "1969-12-31T23:59:59Z",
        ];
        for bad in bad {
            assert_eq!(parse_time(bad), None, "{bad}");
        }
    }

    #[test]
    fn an_oversized_line_is_skipped_not_held() {
        let path = temp("lines.jsonl");
        let long = "x".repeat(100);
        std::fs::write(&path, format!("one\n{long}\ntwo\n{long}")).unwrap();
        let mut seen = Vec::new();
        let mut skipping = false;
        let read = |offset: u64, skipping: &mut bool, seen: &mut Vec<String>| {
            let file = File::open(&path).unwrap();
            let each = |line: &str| {
                seen.push(line.to_string());
                false
            };
            lines_capped(file, offset, u64::MAX, 10, skipping, each).unwrap().0
        };
        // The long line in the middle is passed over. The one at the end has no newline yet:
        // the offset moves past what is there, and the flag says a line is being skipped.
        let offset = read(0, &mut skipping, &mut seen);
        assert_eq!((offset, skipping), (4 + 101 + 4 + 100, true));
        assert_eq!(seen, ["one", "two"]);
        // More of it arrives: only the new bytes are read.
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"yyyy").unwrap();
        let offset = read(offset, &mut skipping, &mut seen);
        assert_eq!((offset, skipping), (4 + 101 + 4 + 104, true));
        // Its end arrives, and a line after it.
        file.write_all(b"zz\nthree\n").unwrap();
        let offset = read(offset, &mut skipping, &mut seen);
        assert_eq!((offset, skipping), (4 + 101 + 4 + 107 + 6, false));
        assert_eq!(seen, ["one", "two", "three"]);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn skips_to_the_end_of_a_line() {
        let mut reader = BufReader::with_capacity(4, Cursor::new(b"abcdefgh\nrest".to_vec()));
        assert_eq!(skip_line(&mut reader).unwrap(), (9, true));
        assert_eq!(skip_line(&mut reader).unwrap(), (4, false), "no newline yet");
    }

    #[test]
    fn only_a_regular_file_is_opened() {
        // A FIFO with no writer would block `File::open` for good.
        let path = temp("pipe.jsonl");
        let _ = std::fs::remove_file(&path);
        let made = std::process::Command::new("mkfifo").arg(&*path).status().is_ok_and(|s| s.success());
        assert!(made, "mkfifo");
        assert_eq!(open(&path).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert!(head(&path, 1, 100).is_err() && tail(&path, 100).is_err() && small(&path).is_none());
        assert_eq!(open(&std::env::temp_dir()).unwrap_err().kind(), io::ErrorKind::InvalidInput, "a directory");
        std::fs::remove_file(&path).unwrap();
    }
}
