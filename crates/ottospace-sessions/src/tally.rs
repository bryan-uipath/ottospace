//! What a session's work added up to: tokens by model, its sub-agents' apart, and when it was
//! active. Folded from events as `Snapshot` is, and read on from where the last read stopped:
//! the session's own file and each sub-agent's, each with its cursor. Numbers and model names only.

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;

use crate::{Cursor, Event, SessionRef, Usage, What};

/// A model's name when a request did not say it.
pub const UNKNOWN: &str = "unknown";

/// Tokens of a session's model requests, summed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tally {
    /// By (model, made by a sub-agent): `input` counts every prompt token, cached or not.
    pub models: BTreeMap<(String, bool), Usage>,
    /// Tokens in the context window after the main loop's last request, and the window, when known.
    pub context: Option<u64>,
    pub window: Option<u64>,
}

impl Tally {
    /// Adds one event. `sub`: it is a sub-agent's.
    pub fn apply(&mut self, event: &Event, sub: bool) {
        // After a compaction the context holds what it says, as `Snapshot` has it.
        if let (What::Compaction { tokens_after: Some(after) }, false) = (&event.what, sub) {
            self.context = Some(*after);
        }
        let What::ModelStep { model, usage: Some(usage), window } = &event.what else { return };
        let name = model.clone().unwrap_or_else(|| UNKNOWN.to_string());
        let sum = self.models.entry((name, sub)).or_default();
        sum.input = sum.input.saturating_add(usage.input);
        sum.output = sum.output.saturating_add(usage.output);
        sum.cached = sum.cached.saturating_add(usage.cached);
        if !sub {
            self.context = Some(usage.input.saturating_add(usage.output));
            self.window = window.or(self.window);
        }
    }
}

/// When something happened that is the agent at work: a model request, a tool call's start or end.
fn activity(event: &Event) -> Option<u64> {
    let work = matches!(event.what, What::ModelStep { .. } | What::ToolStart { .. } | What::ToolEnd { .. });
    (work && event.at > 0).then_some(event.at)
}

/// What one `Reader::advance` read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Read {
    /// The times of the activity read now, from every file, oldest first.
    pub times: Vec<u64>,
    /// Every file was read to its end: the tally is the whole session's so far.
    pub caught_up: bool,
    /// The session's file was cut short or replaced: the tally began again, and so must
    /// whatever was folded from earlier times.
    pub restarted: bool,
}

/// A session's tally, read on from where the last call stopped.
#[derive(Clone, Debug, Default)]
pub struct Reader {
    main: Cursor,
    subs: BTreeMap<PathBuf, Cursor>,
    pub tally: Tally,
}

impl Reader {
    /// Reads on, at most `budget` bytes a file: the session's own, then each sub-agent's.
    pub fn advance(
        &mut self,
        runner: &dyn crate::SessionReader,
        session: &SessionRef,
        budget: u64,
    ) -> io::Result<Read> {
        let mut read = Read { caught_up: true, ..Read::default() };
        // On an error the cursor stays: the next read must not count from the start again.
        let batch = runner.read(session, self.main.clone(), budget)?;
        if batch.restarted {
            (self.tally, self.subs, read.restarted) = (Tally::default(), BTreeMap::new(), true);
        }
        self.fold(&batch.events, false, &mut read.times);
        (self.main, read.caught_up) = (batch.cursor, !batch.more);
        for sub in runner.subagents(session) {
            let cursor = self.subs.remove(&sub.path).unwrap_or_default();
            // One that cannot be read now is read from where it stopped next time.
            let Ok(batch) = runner.read(&sub, cursor.clone(), budget) else {
                self.subs.insert(sub.path, cursor);
                continue;
            };
            // A sub-agent's file is not cut short by the runner; if it is, its tokens are counted
            // again from its start: too many rather than none.
            self.fold(&batch.events, true, &mut read.times);
            read.caught_up &= !batch.more;
            self.subs.insert(sub.path, batch.cursor);
        }
        read.times.sort_unstable();
        Ok(read)
    }

    fn fold(&mut self, events: &[Event], sub: bool, times: &mut Vec<u64>) {
        for event in events {
            self.tally.apply(event, sub);
            times.extend(activity(event));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_compaction_sets_the_context_and_a_sub_agents_does_not() {
        let step =
            What::ModelStep { model: None, usage: Some(Usage { input: 90, output: 10, cached: 0 }), window: None };
        let mut tally = Tally::default();
        tally.apply(&Event { at: 1, what: step }, false);
        tally.apply(&Event { at: 2, what: What::Compaction { tokens_after: Some(30) } }, true);
        assert_eq!(tally.context, Some(100));
        tally.apply(&Event { at: 3, what: What::Compaction { tokens_after: Some(30) } }, false);
        assert_eq!((tally.context, tally.models[&(UNKNOWN.to_string(), false)].input), (Some(30), 90));
    }
}
