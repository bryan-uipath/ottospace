//! Reading JSON-lines session files a runner may still be writing.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde_json::Value;

/// Calls `each` with every complete line after byte `offset` and returns the new offset.
/// A last line without its newline is still being written: it is left for the next call.
/// A file shorter than `offset` was replaced: reading starts over.
pub fn lines_after(path: &Path, offset: u64, mut each: impl FnMut(&str)) -> io::Result<u64> {
    let mut file = File::open(path)?;
    let mut offset = if file.metadata()?.len() < offset { 0 } else { offset };
    file.seek(SeekFrom::Start(offset))?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 || line.last() != Some(&b'\n') {
            return Ok(offset);
        }
        offset += read as u64;
        // A line that is not UTF-8 is skipped like any other record nobody knows.
        if let Ok(text) = std::str::from_utf8(&line) {
            each(text.trim_end());
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
    file.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.split('\n').collect();
    // The last piece is empty or half written; the first is cut unless the read began at the start.
    lines.pop();
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    Ok(lines.into_iter().map(str::to_string).collect())
}

/// A file's modification time in Unix milliseconds.
pub fn modified_ms(path: &Path) -> Option<u64> {
    let modified = path.metadata().ok()?.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64)
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
    if year < 1970 || !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60
    {
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

    #[test]
    fn parses_times() {
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_time("2000-03-01T00:00:00.000Z"), Some(951_868_800_000));
        assert_eq!(parse_time("2024-02-29T12:30:15.5Z"), Some(1_709_209_815_500));
        assert_eq!(parse_time("2026-11-12T10:44:00.123456Z"), Some(1_794_480_240_123));
        for bad in
            ["", "yesterday", "2026-11-12", "2026-13-01T00:00:00Z", "2026-11-12T10:44:00+01:00", "2026-11-12T1é:00:00Z"]
        {
            assert_eq!(parse_time(bad), None, "{bad}");
        }
    }
}
