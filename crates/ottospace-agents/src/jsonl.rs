//! Reading JSON-lines session files a runner may still be writing.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

/// A longer line is skipped without being held in memory.
pub const MAX_LINE: u64 = 64 << 20;

/// Calls `each` with every complete line after byte `offset`, until the end of the file, until
/// `budget` bytes were read, or until `each` returns true. Returns the new offset and whether
/// it stopped before the end. A last line without its newline is still being written: it is
/// left for the next call.
pub fn lines(file: File, offset: u64, budget: u64, each: impl FnMut(&str) -> bool) -> io::Result<(u64, bool)> {
    lines_capped(file, offset, budget, MAX_LINE, each)
}

fn lines_capped(
    mut file: File,
    offset: u64,
    budget: u64,
    max_line: u64,
    mut each: impl FnMut(&str) -> bool,
) -> io::Result<(u64, bool)> {
    file.seek(SeekFrom::Start(offset))?;
    let mut reader = BufReader::new(file);
    let (start, mut offset) = (offset, offset);
    let mut line = Vec::new();
    loop {
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
            // Too long: pass over the rest of it. If it has no end yet, wait like for any half line.
            let Some(rest) = skip_line(&mut reader)? else { return Ok((offset, false)) };
            offset += read + rest;
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

/// Reads past the next newline and returns how many bytes that was, or `None` if the file ends first.
fn skip_line(reader: &mut impl BufRead) -> io::Result<Option<u64>> {
    let mut skipped = 0;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(None);
        }
        let newline = buffer.iter().position(|b| *b == b'\n');
        let used = newline.map_or(buffer.len(), |at| at + 1);
        reader.consume(used);
        skipped += used as u64;
        if newline.is_some() {
            return Ok(Some(skipped));
        }
    }
}

/// The first complete lines of a file, parsed: at most `max_lines` lines and `max_bytes` bytes.
pub fn head(path: &Path, max_lines: usize, max_bytes: u64) -> io::Result<Vec<Value>> {
    let mut reader = BufReader::new(File::open(path)?.take(max_bytes));
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
pub fn tail(path: &Path, max_bytes: u64) -> io::Result<Vec<String>> {
    let mut file = File::open(path)?;
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

/// An RFC 3339 UTC time in Unix milliseconds, e.g. `2026-11-12T10:44:00.123Z` → 1794480240123.
pub fn parse_time(text: &str) -> Option<u64> {
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
pub fn time_of(record: &Value) -> u64 {
    record["timestamp"].as_str().and_then(parse_time).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

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
        let path = std::env::temp_dir().join(format!("ottospace-agents-{}-lines.jsonl", std::process::id()));
        let long = "x".repeat(100);
        std::fs::write(&path, format!("one\n{long}\ntwo\n{long}")).unwrap();
        let mut seen = Vec::new();
        let read = lines_capped(File::open(&path).unwrap(), 0, u64::MAX, 10, |line| {
            seen.push(line.to_string());
            false
        });
        // The long line in the middle is passed over; the one at the end has no newline yet.
        assert_eq!(read.unwrap(), (4 + 101 + 4, false));
        assert_eq!(seen, ["one", "two"]);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn skips_to_the_end_of_a_line() {
        let mut reader = BufReader::with_capacity(4, Cursor::new(b"abcdefgh\nrest".to_vec()));
        assert_eq!(skip_line(&mut reader).unwrap(), Some(9));
        assert_eq!(skip_line(&mut reader).unwrap(), None, "no newline yet");
    }
}
