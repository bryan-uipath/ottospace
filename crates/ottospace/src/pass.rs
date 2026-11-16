//! A task agent's pass: 16 random bytes minted when ottospace starts the agent, in its
//! environment only. A socket request that carries it is that task's (`docs/ai/agent-mode.md`,
//! "Who is asking"). A secret: never in the state, a trace, a log or a file; no `Serialize`, and
//! its `Debug` hides it.

use std::io::Read;

use ottospace_core::tasks::{Caller, TaskId};

/// The variable it travels in. No `KEY`, `TOKEN` or `SECRET` in the name: Codex's tool shell
/// may drop such names (agent-mode.md, "Probes").
pub(crate) const VAR: &str = "OTTOSPACE_TASK_PASS";

pub(crate) struct Pass([u8; 16]);

impl Pass {
    /// From the OS's random source, as session ids are. `None`: no random bytes.
    pub(crate) fn mint() -> Option<Self> {
        let mut bytes = [0u8; 16];
        std::fs::File::open("/dev/urandom").and_then(|mut source| source.read_exact(&mut bytes)).ok()?;
        Some(Self(bytes))
    }

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
        let (one, two) = (Pass::mint().expect("random bytes"), Pass::mint().expect("random bytes"));
        assert!(!one.matches(&two), "two mints, two passes");
        let hex = one.hex();
        assert_eq!(hex.len(), 32);
        assert!(Pass::parse(&hex).is_some_and(|back| back.matches(&one)));
        // Its Debug says nothing of it.
        assert_eq!(format!("{one:?}"), "Pass(..)");
        assert!(!format!("{:?}", Some(&one)).contains(&hex[..8]));
        // Only 32 hex digits are a pass.
        for bad in ["", "00", &hex[..31], &format!("{hex}0"), &"g".repeat(32), &"é".repeat(16), &"+f".repeat(16)] {
            assert!(Pass::parse(bad).is_none(), "{bad}");
        }
        // Upper case is the same pass.
        assert!(Pass::parse(&hex.to_uppercase()).is_some_and(|back| back.matches(&one)));
        // No pass: anyone. A live one: its task. Any other, or one revoked (no longer live): unknown.
        let live = [(&one, 7)];
        assert_eq!(caller(None, live), Caller::Anyone);
        assert_eq!(caller(Some(&one), live), Caller::Task(7));
        assert_eq!(caller(Some(&two), live), Caller::Unknown);
        assert_eq!(caller(Some(&one), []), Caller::Unknown);
        let flipped = Pass::parse(&format!("{}{}", &hex[..31], if hex.ends_with('0') { '1' } else { '0' }));
        assert_eq!(caller(flipped.as_ref(), live), Caller::Unknown);
    }
}
