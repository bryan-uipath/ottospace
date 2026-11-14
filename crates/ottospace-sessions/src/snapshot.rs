//! A session's current state, folded from its events. Pure: no clock, no I/O.

use std::collections::{BTreeMap, VecDeque};

use crate::station::Station;
use crate::{Event, What};

/// How many finished calls a snapshot keeps.
const RECENT: usize = 16;

/// How many running calls, and how many sub-agents, a snapshot keeps: the newest. A file that
/// starts calls and never ends them cannot grow a snapshot without bound.
pub const MAX_RUNNING: usize = 256;

/// A tool call, running or finished.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub id: String,
    pub tool: String,
    pub station: Station,
    pub summary: String,
    pub started: u64,
    /// When it ended; `None` while it runs.
    pub ended: Option<u64>,
    pub failed: bool,
}

/// A sub-agent still at work.
#[derive(Clone, Debug, PartialEq)]
pub struct SubAgent {
    pub id: String,
    pub label: String,
    pub started: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Active,
    Idle,
}

/// What a session is doing now and what it did so far.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// Calls in flight, oldest first.
    pub running: Vec<Call>,
    /// The last finished calls, oldest first.
    pub recent: VecDeque<Call>,
    /// Uses per station: tool calls, plus user messages on `You` and model requests on `Model`.
    pub counts: BTreeMap<Station, u32>,
    pub subagents: Vec<SubAgent>,
    /// Failed calls and errors since the last user message.
    pub errors: u32,
    /// The last failure, as (time, code or tool name).
    pub last_error: Option<(u64, String)>,
    pub model: Option<String>,
    /// Tokens in the context window after the last model request.
    pub context_tokens: Option<u64>,
    pub context_window: Option<u64>,
    pub compactions: u32,
    /// Time of the last event.
    pub last_event: Option<u64>,
}

impl Snapshot {
    pub fn fold<'a>(events: impl IntoIterator<Item = &'a Event>) -> Self {
        let mut snapshot = Self::default();
        events.into_iter().for_each(|e| snapshot.apply(e));
        snapshot
    }

    pub fn apply(&mut self, event: &Event) {
        let at = event.at;
        if at > 0 {
            self.last_event = Some(self.last_event.unwrap_or(0).max(at));
        }
        match &event.what {
            What::UserMessage => {
                *self.counts.entry(Station::You).or_default() += 1;
                self.errors = 0;
                // A new turn: a call with no end by now was interrupted and will never get one.
                self.running.clear();
            }
            What::ModelStep { model, usage, window } => {
                *self.counts.entry(Station::Model).or_default() += 1;
                if model.is_some() {
                    self.model = model.clone();
                }
                if let Some(usage) = usage {
                    self.context_tokens = Some(usage.input.saturating_add(usage.output));
                }
                if window.is_some() {
                    self.context_window = *window;
                }
            }
            What::ToolStart { id, tool, station, summary, .. } => {
                *self.counts.entry(*station).or_default() += 1;
                let (id, tool, summary) = (id.clone(), tool.clone(), summary.clone());
                self.running.push(Call {
                    id,
                    tool,
                    station: *station,
                    summary,
                    started: at,
                    ended: None,
                    failed: false,
                });
                if self.running.len() > MAX_RUNNING {
                    self.running.remove(0);
                }
            }
            What::ToolEnd { id, failed, station, summary } => {
                // An end without a start (the reader began mid-session) is dropped.
                let Some(i) = self.running.iter().position(|c| c.id == *id) else { return };
                let mut call = self.running.remove(i);
                // The end knows better: move the count to the station the call turned out to use.
                if let Some(station) = station.filter(|s| *s != call.station) {
                    if let Some(count) = self.counts.get_mut(&call.station) {
                        *count = count.saturating_sub(1);
                    }
                    *self.counts.entry(station).or_default() += 1;
                    call.station = station;
                }
                if let Some(summary) = summary {
                    call.summary = summary.clone();
                }
                (call.ended, call.failed) = (Some(at), *failed);
                if *failed {
                    self.fail(at, call.summary.clone());
                }
                self.recent.push_back(call);
                if self.recent.len() > RECENT {
                    self.recent.pop_front();
                }
            }
            What::Error { kind } => self.fail(at, kind.clone()),
            What::SubAgentStart { id, label } => {
                if !self.subagents.iter().any(|a| a.id == *id) {
                    self.subagents.push(SubAgent { id: id.clone(), label: label.clone(), started: at });
                }
                if self.subagents.len() > MAX_RUNNING {
                    self.subagents.remove(0);
                }
            }
            What::SubAgentEnd { id } => self.subagents.retain(|a| a.id != *id),
            What::Compaction { tokens_after } => {
                self.compactions += 1;
                if tokens_after.is_some() {
                    self.context_tokens = *tokens_after;
                }
            }
            What::ModelChange { to, .. } => self.model = Some(to.clone()),
        }
    }

    fn fail(&mut self, at: u64, what: String) {
        self.errors += 1;
        self.last_error = Some((at, what));
    }

    /// How full the context window is, 0 to 100, when both numbers are known.
    pub fn context_percent(&self) -> Option<u8> {
        let (tokens, window) = (self.context_tokens?, self.context_window.filter(|w| *w > 0)?);
        Some((tokens.saturating_mul(100) / window).min(100) as u8)
    }

    /// Active if the last event is at most `idle_after_ms` before `now_ms`. A long quiet command reads as
    /// idle: a caller that knows the runner's process is alive should also look at `running`.
    pub fn activity(&self, now_ms: u64, idle_after_ms: u64) -> Activity {
        match self.last_event {
            Some(at) if now_ms.saturating_sub(at) <= idle_after_ms => Activity::Active,
            _ => Activity::Idle,
        }
    }

    /// The station at work now: the newest running call's, or `Agents` while only sub-agents work.
    pub fn station(&self) -> Option<Station> {
        let agents = (!self.subagents.is_empty()).then_some(Station::Agents);
        self.running.last().map(|c| c.station).or(agents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Usage;

    fn at(at: u64, what: What) -> Event {
        Event { at, what }
    }

    fn start(time: u64, id: &str, station: Station) -> Event {
        let (id, tool, summary) = (id.to_string(), "Bash".to_string(), "cargo".to_string());
        at(time, What::ToolStart { id, tool, station, summary, diff: None })
    }

    fn end(time: u64, id: &str, failed: bool) -> Event {
        at(time, What::ToolEnd { id: id.to_string(), failed, station: None, summary: None })
    }

    #[test]
    fn running_calls_and_subagents_are_bounded() {
        let mut s = Snapshot::default();
        for i in 0..1000 {
            s.apply(&start(i, &format!("call-{i}"), Station::Shell));
            s.apply(&at(i, What::SubAgentStart { id: format!("agent-{i}"), label: "agent".into() }));
        }
        assert_eq!((s.running.len(), s.subagents.len()), (MAX_RUNNING, MAX_RUNNING));
        assert_eq!((s.running[0].id.as_str(), s.running[MAX_RUNNING - 1].id.as_str()), ("call-744", "call-999"));
        assert_eq!(s.counts.get(&Station::Shell), Some(&1000), "the count is not cut");
    }

    #[test]
    fn tracks_running_calls_and_counts() {
        let events = [
            at(1000, What::UserMessage),
            at(1100, What::ModelStep { model: Some("m-1".into()), usage: None, window: None }),
            start(1200, "a", Station::Shell),
            start(1300, "b", Station::Files),
            end(1400, "a", false),
        ];
        let s = Snapshot::fold(&events);
        assert_eq!(s.running.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["b"]);
        assert_eq!(s.station(), Some(Station::Files));
        assert_eq!(s.recent.len(), 1);
        assert_eq!((s.recent[0].started, s.recent[0].ended), (1200, Some(1400)));
        let counts: Vec<_> = s.counts.iter().map(|(k, v)| (*k, *v)).collect();
        assert_eq!(counts, [(Station::You, 1), (Station::Model, 1), (Station::Files, 1), (Station::Shell, 1)]);
        assert_eq!(s.model.as_deref(), Some("m-1"));
        assert_eq!(s.last_event, Some(1400));
    }

    #[test]
    fn errors_last_until_the_next_user_message() {
        let mut s = Snapshot::fold(&[start(10, "a", Station::Shell), end(20, "a", true)]);
        s.apply(&at(30, What::Error { kind: "rate_limit".into() }));
        assert_eq!(s.errors, 2);
        assert_eq!(s.last_error, Some((30, "rate_limit".to_string())));
        assert!(s.recent[0].failed);
        s.apply(&at(40, What::UserMessage));
        assert_eq!(s.errors, 0);
        assert_eq!(s.last_error, Some((30, "rate_limit".to_string())), "kept for the feed");
    }

    #[test]
    fn an_end_can_move_the_call_to_another_station() {
        let mut s = Snapshot::fold(&[start(10, "a", Station::Shell)]);
        let (station, summary) = (Some(Station::Network), Some("git push".to_string()));
        s.apply(&at(20, What::ToolEnd { id: "a".into(), failed: false, station, summary }));
        assert_eq!(s.counts.get(&Station::Shell), Some(&0));
        assert_eq!(s.counts.get(&Station::Network), Some(&1));
        assert_eq!(s.recent[0].summary, "git push");
    }

    #[test]
    fn a_new_turn_drops_calls_that_never_ended() {
        let mut s = Snapshot::fold(&[start(10, "a", Station::Shell)]);
        s.apply(&end(15, "unknown", false));
        assert_eq!(s.running.len(), 1, "an end without a start changes nothing");
        s.apply(&at(20, What::UserMessage));
        assert!(s.running.is_empty());
    }

    #[test]
    fn context_percent_needs_both_numbers() {
        let usage = Some(Usage { input: 49_000, output: 1_000, cached: 40_000 });
        let mut s = Snapshot::fold(&[at(1, What::ModelStep { model: None, usage, window: None })]);
        assert_eq!((s.context_tokens, s.context_percent()), (Some(50_000), None));
        s.apply(&at(2, What::ModelStep { model: None, usage, window: Some(200_000) }));
        assert_eq!(s.context_percent(), Some(25));
        s.apply(&at(3, What::Compaction { tokens_after: Some(10_000) }));
        assert_eq!((s.compactions, s.context_percent()), (1, Some(5)));
        s.context_tokens = Some(900_000);
        assert_eq!(s.context_percent(), Some(100));
    }

    #[test]
    fn subagents_and_model_changes() {
        let mut s = Snapshot::default();
        s.apply(&at(1, What::SubAgentStart { id: "x".into(), label: "Explore".into() }));
        s.apply(&at(2, What::SubAgentStart { id: "x".into(), label: "Explore".into() }));
        assert_eq!(s.subagents.len(), 1);
        assert_eq!(s.station(), Some(Station::Agents));
        s.apply(&at(3, What::SubAgentEnd { id: "x".into() }));
        assert_eq!(s.station(), None);
        s.apply(&at(4, What::ModelChange { from: "m-1".into(), to: "m-2".into() }));
        assert_eq!(s.model.as_deref(), Some("m-2"));
    }

    #[test]
    fn activity_goes_by_the_age_passed_in() {
        let s = Snapshot::fold(&[at(10_000, What::UserMessage)]);
        assert_eq!(s.activity(15_000, 5_000), Activity::Active);
        assert_eq!(s.activity(15_001, 5_000), Activity::Idle);
        assert_eq!(s.activity(5_000, 5_000), Activity::Active, "a clock behind the file's is not idle");
        assert_eq!(Snapshot::default().activity(0, 5_000), Activity::Idle);
    }
}
