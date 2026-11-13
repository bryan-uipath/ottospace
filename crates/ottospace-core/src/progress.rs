//! What a task's agent did: time at work, tokens by model, the context it filled. The shell
//! reads them from the task's session files (`ottospace-sessions`), counts time by the rule
//! here, and the task keeps them in its file, so they outlive those files. Time comes in;
//! nothing here reads a clock. Numbers and model names only: no prices.

use serde::{Deserialize, Serialize};

use crate::sessions::{self, RUNNERS};

/// A gap between two moments of activity longer than this is the agent waiting (for the user,
/// most often): not time at work.
pub const QUIET_GAP_MS: u64 = 120_000;

/// How long after a task is done its session is still read: what it wrote after the last read.
pub const LAST_READ_MS: u64 = 300_000;

/// The most sessions a task keeps the work of, and the most token rows a session keeps.
pub const MAX_SESSIONS: usize = 16;
pub const MAX_MODELS: usize = 16;

/// Time at work, from the times of an agent's activity (a model request, a tool call's start or
/// end), oldest first. The gap between two of them counts while it is at most `QUIET_GAP_MS`.
/// The part of it after `day` (the start of the user's day, passed in) counts for that day too.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clock {
    pub ms: u64,
    /// The last activity counted.
    #[serde(default)]
    pub last: Option<u64>,
    /// The day counted for (when it started, milliseconds since 1970), and its time at work.
    #[serde(default)]
    pub day: u64,
    #[serde(default)]
    pub day_ms: u64,
}

impl Clock {
    /// Activity at `at`. One older than the last counted adds nothing.
    pub fn tick(&mut self, at: u64, day: u64) {
        if day != self.day {
            (self.day, self.day_ms) = (day, 0);
        }
        match self.last {
            Some(last) if at < last => return,
            Some(last) if at - last <= QUIET_GAP_MS => {
                self.ms = self.ms.saturating_add(at - last);
                self.day_ms = self.day_ms.saturating_add(at.saturating_sub(last.max(day)));
            }
            _ => {}
        }
        self.last = Some(at);
    }

    /// Its time at work on `day`.
    pub fn on(&self, day: u64) -> u64 {
        if self.day == day { self.day_ms } else { 0 }
    }
}

/// Tokens of one model, summed. `input` counts every prompt token, cached or not.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    /// In short (`sessions::short_model`): `opus-5.5`, `gpt-6-astra`.
    pub model: String,
    /// A sub-agent's requests.
    #[serde(default)]
    pub sub: bool,
    pub input: u64,
    pub output: u64,
    pub cached: u64,
}

/// What one session of a task's agent did, as last read.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Work {
    pub runner: String,
    pub session: String,
    #[serde(default)]
    pub clock: Clock,
    #[serde(default)]
    pub tokens: Vec<Tokens>,
    /// Tokens in the context window after the main loop's last request, and the window.
    #[serde(default)]
    pub context: Option<u64>,
    #[serde(default)]
    pub window: Option<u64>,
}

impl Work {
    /// What is kept of it: a known runner and a plain id; each model in short, or left out if it
    /// is not a name; one row a model, at most `MAX_MODELS`, the main loop's first.
    pub fn kept(self) -> Option<Work> {
        let plain = !self.session.is_empty() && self.session.len() <= 128;
        if !RUNNERS.contains(&self.runner.as_str()) || !plain || self.session.chars().any(char::is_control) {
            return None;
        }
        Some(Work { tokens: merged(self.tokens.iter()), ..self })
    }
}

/// Rows of one (model in short, sub-agent's) each, summed: the main loop's first, then by model.
pub fn merged<'a>(rows: impl Iterator<Item = &'a Tokens>) -> Vec<Tokens> {
    let mut out: Vec<Tokens> = Vec::new();
    for row in rows {
        let Some(model) = sessions::short_model(&row.model) else { continue };
        match out.iter_mut().find(|r| r.model == model && r.sub == row.sub) {
            Some(sum) => {
                sum.input = sum.input.saturating_add(row.input);
                sum.output = sum.output.saturating_add(row.output);
                sum.cached = sum.cached.saturating_add(row.cached);
            }
            None => out.push(Tokens { model, ..row.clone() }),
        }
    }
    out.sort_by(|a, b| (a.sub, &a.model).cmp(&(b.sub, &b.model)));
    out.truncate(MAX_MODELS);
    out
}

/// A session's work replaces what was kept of that session, and goes last: the newest. At most
/// `MAX_SESSIONS`, the oldest dropped.
pub fn record(list: &mut Vec<Work>, work: Work) {
    list.retain(|w| (&w.runner, &w.session) != (&work.runner, &work.session));
    list.push(work);
    if list.len() > MAX_SESSIONS {
        list.remove(0);
    }
}

/// A time at work as people say it: `40s`, `12m`, `1h 05m`.
pub fn duration(ms: u64) -> String {
    let (s, m) = (ms / 1000, ms / 60_000);
    match () {
        _ if s < 60 => format!("{s}s"),
        _ if m < 60 => format!("{m}m"),
        _ => format!("{}h {:02}m", m / 60, m % 60),
    }
}

/// A count of tokens in short: `950`, `12.4k`, `3.1M`.
pub fn count(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_at_work_is_activity_until_a_quiet_gap() {
        let (s, day) = (1_000, 0);
        let mut clock = Clock::default();
        // Steps 10 s apart: 30 s at work. Then a long wait for the user: nothing. Then 5 s more.
        for at in [0, 10, 20, 30, 30 + 600, 30 + 605] {
            clock.tick(at * s, day);
        }
        assert_eq!(clock.ms, 35 * s);
        // A gap of exactly the limit counts; one more does not. An older time adds nothing.
        let mut edge = Clock::default();
        [0, QUIET_GAP_MS, 2 * QUIET_GAP_MS + 1, 5].iter().for_each(|at| edge.tick(*at, day));
        assert_eq!((edge.ms, edge.last), (QUIET_GAP_MS, Some(2 * QUIET_GAP_MS + 1)));
    }

    #[test]
    fn the_day_counts_what_lies_after_its_start() {
        let day = 100 * 86_400_000;
        let mut clock = Clock::default();
        clock.tick(day - 30_000, day - 86_400_000);
        // The gap across midnight: 40 s in all, 10 s of it today.
        clock.tick(day + 10_000, day);
        assert_eq!((clock.ms, clock.on(day), clock.on(day - 86_400_000)), (40_000, 10_000, 0));
        // The next day starts at nothing.
        clock.tick(day + 20_000, day + 86_400_000);
        assert_eq!((clock.on(day + 86_400_000), clock.on(day)), (0, 0));
    }

    #[test]
    fn work_is_kept_with_models_in_short_and_one_row_each() {
        let row = |model: &str, sub, input| Tokens { model: model.into(), sub, input, output: 1, cached: 0 };
        let work = Work {
            runner: "claude".into(),
            session: "s1".into(),
            tokens: vec![
                row("claude-opus-5-5", false, 10),
                row("claude-opus-5-5[1m]", false, 5),
                row("claude-haiku-5", true, 3),
                row("a private note", false, 99),
            ],
            ..Work::default()
        };
        let kept = work.clone().kept().unwrap();
        let rows: Vec<_> = kept.tokens.iter().map(|t| (t.model.as_str(), t.sub, t.input, t.output)).collect();
        assert_eq!(rows, [("opus-5.5", false, 15, 2), ("haiku-5", true, 3, 1)]);
        for bad in [Work { runner: "gemini".into(), ..work.clone() }, Work { session: "a\nb".into(), ..work }] {
            assert_eq!(bad.kept(), None);
        }
        // A session's work replaces its own, goes last, and the oldest drop past the limit.
        let mut list = Vec::new();
        for n in 0..=MAX_SESSIONS {
            record(&mut list, Work { runner: "claude".into(), session: n.to_string(), ..Work::default() });
        }
        record(
            &mut list,
            Work {
                runner: "claude".into(),
                session: "5".into(),
                clock: Clock { ms: 7, ..Clock::default() },
                ..Work::default()
            },
        );
        assert_eq!(
            (list.len(), list[0].session.as_str(), list.last().map(|w| w.clock.ms)),
            (MAX_SESSIONS, "1", Some(7))
        );
    }

    #[test]
    fn durations_and_counts_read_short() {
        assert_eq!([0, 40_000, 720_000, 3_900_000].map(duration), ["0s", "40s", "12m", "1h 05m"]);
        assert_eq!([950, 12_400, 3_100_000].map(count), ["950", "12.4k", "3.1M"]);
    }
}
