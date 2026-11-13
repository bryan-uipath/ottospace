//! A process's stdout cut into lines, whatever size its reads come in. Pure.

/// One line, without its newline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    Text(String),
    /// Longer than the cap: skipped to its newline, never held whole. Its first `HEAD` bytes
    /// (lossy), so a protocol can tell whether the CLI waits for an answer to it.
    Oversized(String),
    NotUtf8,
}

/// The longest line read. Claude's tool results carry images as base64, so a line may be MBs.
pub const MAX_LINE: usize = 32 << 20;

/// How much of an oversized line is kept.
pub const HEAD: usize = 1024;

/// Bytes in, lines out. A line split across reads comes out once it ends.
#[derive(Debug)]
pub struct Lines {
    held: Vec<u8>,
    max: usize,
    /// The bytes coming are the rest of an oversized line.
    skipping: bool,
}

impl Lines {
    pub fn new(max: usize) -> Self {
        Self { held: Vec::new(), max, skipping: false }
    }

    /// The lines `bytes` ends. What is left of an unfinished line is held for the next call.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Line> {
        let mut lines = Vec::new();
        let mut rest = bytes;
        while !rest.is_empty() {
            let end = rest.iter().position(|b| *b == b'\n');
            let piece = &rest[..end.unwrap_or(rest.len())];
            if !self.skipping {
                if self.held.len() + piece.len() > self.max {
                    // Said once, when it passes the cap; the rest of it is dropped as it comes.
                    // The same head however the reads were cut: what passed the cap, at most `HEAD`.
                    let keep = HEAD.min(self.max + 1);
                    let mut head = std::mem::take(&mut self.held);
                    head.extend_from_slice(&piece[..piece.len().min(keep.saturating_sub(head.len()))]);
                    head.truncate(keep);
                    self.skipping = true;
                    lines.push(Line::Oversized(String::from_utf8_lossy(&head).into_owned()));
                } else {
                    self.held.extend_from_slice(piece);
                }
            }
            let Some(end) = end else { break };
            if !std::mem::take(&mut self.skipping) {
                lines.push(Self::line(std::mem::take(&mut self.held)));
            }
            rest = &rest[end + 1..];
        }
        lines
    }

    /// The last line, when the stream ended without a newline.
    pub fn finish(&mut self) -> Option<Line> {
        let held = std::mem::take(&mut self.held);
        (!std::mem::take(&mut self.skipping) && !held.is_empty()).then(|| Self::line(held))
    }

    fn line(mut bytes: Vec<u8>) -> Line {
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        String::from_utf8(bytes).map_or(Line::NotUtf8, Line::Text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[&str]) -> Vec<Line> {
        lines.iter().map(|line| Line::Text(line.to_string())).collect()
    }

    #[test]
    fn lines_split_across_reads_come_out_whole() {
        let mut lines = Lines::new(100);
        assert_eq!(lines.push(b"one\ntw"), text(&["one"]));
        assert_eq!(lines.push(b"o\r\n\nthr"), text(&["two", ""]));
        assert_eq!(lines.push(b"ee"), []);
        assert_eq!(lines.finish(), Some(Line::Text("three".into())));
        assert_eq!(lines.finish(), None);
        assert_eq!(lines.push(b"\xff\xfe\n"), [Line::NotUtf8]);
    }

    #[test]
    fn a_long_line_is_dropped_as_it_comes_and_said_once() {
        let mut lines = Lines::new(4);
        assert_eq!(lines.push(b"abc"), []);
        assert_eq!(lines.push(b"def"), [Line::Oversized("abcde".into())]);
        assert!(lines.held.is_empty());
        assert_eq!(lines.push(b"ghi"), []);
        assert_eq!(lines.push(b"j\nok\n"), text(&["ok"]));
        assert_eq!(lines.push(b"12345"), [Line::Oversized("12345".into())]);
        assert_eq!(lines.finish(), None);
    }

    /// However the bytes are cut into reads, the same lines come out.
    #[test]
    fn reads_of_any_size_give_the_same_lines() {
        let mut seed = 7u64;
        let mut next = |n: usize| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize % n.max(1)
        };
        for _ in 0..500 {
            let bytes: Vec<u8> = (0..next(200)).map(|_| *b"ab\n\r\xff".get(next(5)).unwrap_or(&b'x')).collect();
            let whole = {
                let mut lines = Lines::new(8);
                let mut out = lines.push(&bytes);
                out.extend(lines.finish());
                out
            };
            let mut lines = Lines::new(8);
            let (mut out, mut at) = (Vec::new(), 0);
            while at < bytes.len() {
                let end = (at + 1 + next(9)).min(bytes.len());
                out.extend(lines.push(&bytes[at..end]));
                at = end;
            }
            out.extend(lines.finish());
            assert_eq!(out, whole, "{bytes:?}");
            assert!(lines.held.capacity() <= 64, "never holds more than the cap");
        }
    }
}
