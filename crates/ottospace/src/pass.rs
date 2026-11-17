//! A task agent's pass: 16 random bytes minted when ottospace starts the agent, in its
//! environment only. A socket request that carries it is that task's (`docs/ai/agent-mode.md`,
//! "Who is asking"). A secret: never in the state, a trace, a log or a file; no `Serialize`, and
//! its `Debug` hides it.

use std::borrow::Cow;
use std::io::Read;
use std::sync::Mutex;

use ottospace_core::tasks::{Caller, TaskId};

/// The variable it travels in. No `KEY`, `TOKEN` or `SECRET` in the name: Codex's tool shell
/// may drop such names (agent-mode.md, "Probes").
pub(crate) const VAR: &str = "OTTOSPACE_TASK_PASS";

pub(crate) struct Pass([u8; 16]);

/// A minted pass, live while held: its text is hidden (`hide`) from what clients read until it
/// is dropped, which revokes it.
pub(crate) struct Live(Pass);

/// The live passes' hex, for `hide`. Only `Live` adds and removes.
static LIVE: Mutex<Vec<String>> = Mutex::new(Vec::new());

impl Live {
    /// From the OS's random source, as session ids are. `None`: no random bytes.
    pub(crate) fn mint() -> Option<Self> {
        let mut bytes = [0u8; 16];
        std::fs::File::open("/dev/urandom").and_then(|mut source| source.read_exact(&mut bytes)).ok()?;
        let pass = Pass(bytes);
        LIVE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(pass.hex());
        Some(Self(pass))
    }

    pub(crate) fn pass(&self) -> &Pass {
        &self.0
    }

    pub(crate) fn hex(&self) -> String {
        self.0.hex()
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        let hex = self.0.hex();
        let mut live = LIVE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(at) = live.iter().position(|one| *one == hex) {
            live.swap_remove(at);
        }
    }
}

impl std::fmt::Debug for Live {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Live(..)")
    }
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

/// Sorted ranges, those that overlap or touch made one.
fn merged(sorted: Vec<std::ops::Range<usize>>) -> Vec<std::ops::Range<usize>> {
    let mut out: Vec<std::ops::Range<usize>> = Vec::with_capacity(sorted.len());
    for range in sorted {
        match out.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
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

/// A grid's rows, each with whether it wraps into the next, with every live pass hidden: a
/// line that wraps is searched as one. Its first row shows `HIDDEN`, its rest on later rows
/// goes. The caller gives every row of a line it shows a part of (`Terminal::lines`), so no
/// pass is cut at the edge of what was searched.
pub(crate) fn hide_rows(rows: Vec<(String, bool)>) -> Vec<String> {
    if live().is_empty() {
        return rows.into_iter().map(|(row, _)| row).collect();
    }
    let mut out = Vec::with_capacity(rows.len());
    let mut line: Vec<String> = Vec::new();
    for (row, wraps) in rows {
        line.push(row);
        if !wraps {
            out.extend(hide_line(std::mem::take(&mut line)));
        }
    }
    out.extend(hide_line(line));
    out
}

/// `text` with a run of 8 or more hex digits at its start as `HIDDEN`, whatever the digits: for
/// text a cut may have left the end of a pass at (`terminal::cut_history`). It looks at no pass,
/// so what it hides says nothing of one.
pub(crate) fn hide_digits_at_start(text: &str) -> String {
    let run = text.bytes().take_while(u8::is_ascii_hexdigit).count();
    if run < 8 { text.to_string() } else { format!("{HIDDEN}{}", &text[run..]) }
}

/// The rows of one line, as `hide_rows` says.
fn hide_line(rows: Vec<String>) -> Vec<String> {
    let joined = rows.concat();
    let found = found(&joined);
    if found.is_empty() {
        return rows;
    }
    let mut at = 0;
    rows.into_iter()
        .map(|row| {
            let mut out = String::with_capacity(row.len());
            for (i, c) in row.char_indices() {
                let i = at + i;
                match found.iter().find(|range| range.contains(&i)) {
                    Some(range) if range.start == i => out.push_str(HIDDEN),
                    Some(_) => {}
                    None => out.push(c),
                }
            }
            at += row.len();
            out
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pass_is_random_hidden_and_names_only_its_own_task() {
        let (one, two) = (Live::mint().expect("random bytes"), Live::mint().expect("random bytes"));
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
        let live = Live::mint().expect("random bytes");
        let hex = live.hex();
        let text = format!("OTTOSPACE_TASK_PASS={hex}\nagain {} done", hex.to_uppercase());
        assert_eq!(hide(&text), "OTTOSPACE_TASK_PASS=[hidden]\nagain [hidden] done");
        assert!(matches!(hide("nothing here, nothing at all, not one"), Cow::Borrowed(_)));
        // Wrapped from one row to the next, as a narrow screen shows it; the rows around kept.
        let row = |text: &str, wraps: bool| (text.to_string(), wraps);
        let pass = format!("PASS={}", &hex[..10]);
        let rows = vec![row("$ env", false), row(&pass, true), row(&hex[10..], false), row("$", false)];
        assert_eq!(hide_rows(rows), ["$ env", "PASS=[hidden]", "", "$"]);
        assert_eq!(hide_rows(vec![row(&format!("é {hex} é"), false)]), ["é [hidden] é"]);
        // Rows that do not wrap are lines apart: a pass is not made of the end of one and the
        // start of the next.
        let rows = vec![row(&hex[..10], false), row(&hex[10..], false)];
        assert_eq!(hide_rows(rows), [&hex[..10], &hex[10..]]);
        // Only whole passes: what a client typed hides the same whether a part of it is right or
        // wrong, at the end of the text, its start or within. No oracle, one digit at a time.
        let wrong = "0123456789abcdef0123456789abcdef";
        for n in [8, 12, 31] {
            for (part, other) in [(&hex[..n], &wrong[..n]), (&hex[32 - n..], &wrong[32 - n..])] {
                for text in [format!("$ echo {part}"), format!("{part} $"), format!("a {part} b")] {
                    assert_eq!(hide(&text), text);
                    assert_eq!(hide_rows(vec![row(&text, false)]), [text.as_str()]);
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
        assert_eq!(merged(vec![0..10, 5..8, 8..20, 30..40]), [0..20, 30..40]);
        assert_eq!(merged(vec![0..40, 10..20]), [0..40]);
        // Two live passes touching in the text: neither shows a part.
        let (one, two) = (Live::mint().expect("random bytes"), Live::mint().expect("random bytes"));
        let (one, two) = (one.hex(), two.hex());
        // Side by side: one range, one word.
        let text = format!("{one}{two} {two}{one}");
        assert_eq!(hide(&text), "[hidden] [hidden]");
    }
}
