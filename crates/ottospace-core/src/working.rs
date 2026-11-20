//! The words of an agent at work: what it waits on when it says, else a verb that turns every
//! few seconds. Pure: the shell passes the time and Reduce Motion in.

/// While it works with nothing more to say. Short, none an error or a result.
pub const VERBS: [&str; 20] = [
    "Thinking…",
    "Pondering…",
    "Reticulating…",
    "Mulling…",
    "Musing…",
    "Considering…",
    "Deliberating…",
    "Ruminating…",
    "Noodling…",
    "Percolating…",
    "Brewing…",
    "Tinkering…",
    "Cogitating…",
    "Puzzling…",
    "Weighing…",
    "Untangling…",
    "Sifting…",
    "Contemplating…",
    "Wondering…",
    "Simmering…",
];

/// How long a verb stays before the next.
pub const TURN_MS: u64 = 4_000;

/// The verb at `now` for a turn begun at `since` (ms since 1970): the turn's own first, then the
/// next every `TURN_MS`. `calm` (Reduce Motion): the first, steady.
pub fn verb(since: u64, now: u64, calm: bool) -> &'static str {
    let turns = if calm { 0 } else { now.saturating_sub(since) / TURN_MS };
    VERBS[((since / 1000 + turns) % VERBS.len() as u64) as usize]
}

/// The call a turn waits on, as said: the agent's label, cleaned, on one line; None if blank.
pub fn waits_on(tool: Option<&str>) -> Option<String> {
    let tool = crate::transcript::clean(tool?).replace('\n', " ");
    Some(tool.trim().to_string()).filter(|tool| !tool.is_empty())
}

/// The working line for a turn begun at `since`: what it waits on (`waits_on`), else the verb.
/// Information beats whimsy.
pub fn line(tool: Option<&str>, since: u64, now: u64, calm: bool) -> String {
    match waits_on(tool) {
        Some(tool) => format!("Working · {tool}"),
        None => verb(since, now, calm).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verb_is_the_turns_own_then_turns_every_few_seconds() {
        let since = 1_700_000_000_000;
        let first = verb(since, since, false);
        assert_eq!(verb(since, since + TURN_MS - 1, false), first, "steady within its time");
        let next = verb(since, since + TURN_MS, false);
        assert_ne!(next, first);
        assert_eq!(verb(since, since + TURN_MS, false), next, "the same times, the same verb");
        // Every verb comes round, and back to the first.
        let seen: std::collections::BTreeSet<_> =
            (0..VERBS.len() as u64).map(|n| verb(since, since + n * TURN_MS, false)).collect();
        assert_eq!(seen.len(), VERBS.len());
        assert_eq!(verb(since, since + VERBS.len() as u64 * TURN_MS, false), first);
        // Another turn starts elsewhere in the list.
        assert_ne!(verb(since + 1_000, since + 1_000, false), first);
    }

    #[test]
    fn under_reduce_motion_the_verb_holds() {
        let since = 42_000;
        let held = verb(since, since, true);
        assert_eq!(held, verb(since, since, false));
        assert!((0..50).all(|n| verb(since, since + n * TURN_MS, true) == held));
    }

    #[test]
    fn a_known_call_wins_over_the_verb() {
        assert_eq!(line(Some("cargo test"), 0, 9_000, false), "Working · cargo test");
        assert_eq!(line(Some("a\nb"), 0, 0, false), "Working · a b");
        assert!(VERBS.contains(&line(None, 0, 9_000, false).as_str()));
        assert!(VERBS.contains(&line(Some("  "), 0, 0, false).as_str()), "a blank label says nothing");
        assert_eq!((waits_on(Some(" \n ")), waits_on(None)), (None, None));
    }

    #[test]
    fn every_verb_is_short_and_ends_in_an_ellipsis() {
        for verb in VERBS {
            assert!(verb.ends_with('…') && verb.chars().count() <= 15, "{verb}");
        }
        let unique: std::collections::BTreeSet<_> = VERBS.iter().collect();
        assert_eq!(unique.len(), VERBS.len());
    }
}
