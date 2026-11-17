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

/// The shortest piece of a pass hidden at the start or the end of a text: a screen's edge (its
/// top row, scrolled, or output still arriving) may cut a pass. Shorter pieces leave 100 bits.
const EDGE: usize = 8;

/// The live passes' hex. A lock a panic poisoned still holds them: hiding must not fail open.
fn live() -> Vec<String> {
    LIVE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// Where a live pass is in `text`, in either case: byte ranges, sorted, not overlapping. Also its
/// end at the start of `text`, and its start at the end, of `EDGE` digits or more.
fn found(text: &str) -> Vec<std::ops::Range<usize>> {
    if text.len() < EDGE {
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
    for hex in &live {
        let head = (EDGE..hex.len()).rev().find(|n| lower.ends_with(&hex[..*n]));
        found.extend(head.map(|n| lower.len() - n..lower.len()));
        let tail = (EDGE..hex.len()).rev().find(|n| lower.starts_with(&hex[hex.len() - n..]));
        found.extend(tail.map(|n| 0..n));
    }
    found.sort_by_key(|range| range.start);
    let mut end = 0;
    found.retain(|range| {
        let apart = end <= range.start;
        end = end.max(range.end);
        apart
    });
    found
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

/// A screen's rows with every live pass hidden, also one that wraps from a row to the next: the
/// rows are searched as one text. Its first row shows `HIDDEN`, its rest on later rows goes.
pub(crate) fn hide_rows(rows: Vec<String>) -> Vec<String> {
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
        let rows = vec!["$ env".to_string(), format!("PASS={}", &hex[..10]), hex[10..].to_string(), "$".to_string()];
        assert_eq!(hide_rows(rows), ["$ env", "PASS=[hidden]", "", "$"]);
        let rows = vec![format!("é {hex} é")];
        assert_eq!(hide_rows(rows), ["é [hidden] é"]);
        // Cut by a screen's edge: its start at the end, its end at the start, of 8 digits or
        // more. Within the text, half of it is not it.
        assert_eq!(hide(&format!("PASS={}", &hex[..12])), "PASS=[hidden]");
        assert_eq!(hide(&format!("{} $", &hex[20..])), "[hidden] $");
        let rows = vec![hex[25..].to_string(), "$ ls".to_string(), format!("PASS={}", &hex[..9])];
        assert_eq!(hide_rows(rows), [&hex[25..], "$ ls", "PASS=[hidden]"], "7 digits are left");
        let half = format!("a {} b", &hex[..16]);
        assert_eq!(hide(&half), half);
        // A made-up one is not live.
        let other = "0123456789abcdef0123456789abcdef";
        assert_eq!(hide(other), other);
        // Revoked: no longer hidden (and no longer a pass).
        drop(live);
        assert_eq!(hide(&hex), hex);
    }
}
