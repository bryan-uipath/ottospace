//! A conversation's activity, for the dial over its transcript: where its calls go (stations),
//! how much each is used (decayed), what is lit now, what failed since the user's last message,
//! its running time, and the lines beside it. Folded from the transcript's events,
//! times passed in. Design: `docs/ai/agent-dial.md`.

use serde::Serialize;

/// Where work goes. `You`, `Model` and `Hooks` are not tools; the rest classify tool calls
/// (`ottospace_sessions::station::classify`, the one table). `Other`: any other tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Station {
    You,
    Model,
    Memory,
    Files,
    Shell,
    Network,
    /// Every MCP tool call, whatever its server.
    Mcp,
    /// An app pane the user let the task see (agent mode): fed only by ottospace's own rows.
    Apps,
    Agents,
    Skills,
    Hooks,
    Other,
}

impl Station {
    /// Clockwise from the top: the person and the model, then the tools.
    pub const DIAL: [Station; 12] = [
        Station::You,
        Station::Model,
        Station::Files,
        Station::Shell,
        Station::Network,
        // Where Browser was: the other stations keep their places.
        Station::Mcp,
        Station::Apps,
        Station::Agents,
        Station::Skills,
        Station::Memory,
        Station::Hooks,
        Station::Other,
    ];

    /// The station a session's reader names (`id`, any case: `shell`, `files`).
    pub fn named(name: &str) -> Option<Station> {
        Station::DIAL.into_iter().find(|station| station.id().eq_ignore_ascii_case(name))
    }

    /// Its id as the state and a session's reader write it (serde's: `shell`, `mcp`, `other`).
    pub fn id(self) -> &'static str {
        match self {
            Station::You => "you",
            Station::Model => "model",
            Station::Memory => "memory",
            Station::Files => "files",
            Station::Shell => "shell",
            Station::Network => "network",
            Station::Mcp => "mcp",
            Station::Apps => "apps",
            Station::Agents => "agents",
            Station::Skills => "skills",
            Station::Hooks => "hooks",
            Station::Other => "other",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Station::You => "You",
            Station::Model => "Model",
            Station::Memory => "Memory",
            Station::Files => "Files",
            Station::Shell => "Shell",
            Station::Network => "Network",
            Station::Mcp => "MCP",
            Station::Apps => "Apps",
            Station::Agents => "Agents",
            Station::Skills => "Skills",
            Station::Hooks => "Hooks",
            Station::Other => "Tools",
        }
    }

    /// Its place on the dial.
    fn at(self) -> usize {
        Station::DIAL.iter().position(|s| *s == self).unwrap_or(0)
    }

    /// What one use of it is called on its hover.
    fn unit(self) -> (&'static str, &'static str) {
        match self {
            Station::You => ("message", "messages"),
            Station::Model => ("request", "requests"),
            Station::Hooks => ("run", "runs"),
            _ => ("call", "calls"),
        }
    }
}

/// A use counts half after this long: a spoke is heavier for what the agent does now.
pub const HALF_LIFE_MS: u64 = 10 * 60_000;
/// The context ring turns amber, then red.
pub const AMBER: f32 = 0.7;
pub const RED: f32 = 0.9;
/// The most calls kept as running: a stream that starts calls and never ends them stays small.
const MAX_RUNNING: usize = 64;
/// How long a file stays lit, and its agent over it, after its call ends: a 5 ms read is seen.
pub const HOLD_MS: u64 = 1_500;
/// The most ended file calls kept for their hold.
const MAX_HELD: usize = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Use {
    calls: u32,
    failed: u32,
    /// Failed since the user's last message: the red dot.
    failing: u32,
    /// Uses, each halved every `HALF_LIFE_MS`, as of `at`.
    weight: f64,
    at: u64,
}

#[derive(Clone, Debug, PartialEq)]
struct Running {
    id: String,
    station: Station,
    label: String,
    since: u64,
}

/// What was done last: for the idle line.
#[derive(Clone, Debug, PartialEq)]
struct Last {
    station: Station,
    label: String,
    at: u64,
}

/// One conversation's activity.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Activity {
    uses: [Use; 12],
    running: Vec<Running>,
    last: Option<Last>,
    turns: u32,
    first: Option<u64>,
    latest: u64,
    /// The model's step since the last message or call's end is counted.
    stepped: bool,
    /// Since when the model has had the turn: its lit time.
    thinking: u64,
    /// Why the last turn or the process failed, until the user's next message.
    error: Option<String>,
    /// Its file calls that ended lately, each label with when, the newest last (`HOLD_MS`).
    ended: Vec<(String, u64)>,
}

impl Activity {
    fn used(&mut self, station: Station, label: &str, at: u64) {
        let used = &mut self.uses[station.at()];
        used.weight = decayed(used.weight, used.at, at) + 1.;
        (used.at, used.calls) = (used.at.max(at), used.calls.saturating_add(1));
        self.first.get_or_insert(at);
        self.latest = self.latest.max(at);
        // The idle line says the last call or message: a model step is no news after it.
        if station != Station::Model {
            self.last = Some(Last { station, label: label.to_string(), at });
        }
    }

    /// The user's message (or one sent elsewhere, watched): a turn, and the red dots go.
    pub(crate) fn message(&mut self, at: u64) {
        self.used(Station::You, "", at);
        self.turns = self.turns.saturating_add(1);
        self.uses.iter_mut().for_each(|used| used.failing = 0);
        (self.error, self.stepped, self.thinking) = (None, false, at);
        // A new turn holds none of the last one's files.
        self.ended.clear();
    }

    /// The model wrote or asked: one request, counted once until a call ends or a message comes.
    pub(crate) fn step(&mut self, at: u64) {
        if !std::mem::replace(&mut self.stepped, true) {
            self.used(Station::Model, "", at);
        }
    }

    pub(crate) fn turn(&mut self, at: u64) {
        self.thinking = at;
    }

    pub(crate) fn start(&mut self, id: &str, station: Station, label: &str, at: u64) {
        self.step(at);
        self.used(station, label, at);
        self.running.retain(|call| call.id != id);
        self.running.push(Running { id: id.to_string(), station, label: label.to_string(), since: at });
        if self.running.len() > MAX_RUNNING {
            self.running.remove(0);
        }
    }

    /// The call `id` still runs (an interrupt, an exit or a message elsewhere ends them all).
    pub(crate) fn runs(&self, id: &str) -> bool {
        self.running.iter().any(|call| call.id == id)
    }

    /// The id of the newest call on `station` that still runs.
    pub(crate) fn running_on(&self, station: Station) -> Option<&str> {
        self.running.iter().rev().find(|call| call.station == station).map(|call| call.id.as_str())
    }

    /// When a message was last sent to its agent (the user's, or one sent elsewhere).
    pub(crate) fn messaged_at(&self) -> Option<u64> {
        let you = self.uses[Station::You.at()];
        (you.calls > 0).then_some(you.at)
    }

    /// The station of the newest call that still runs.
    pub(crate) fn running_station(&self) -> Option<Station> {
        self.running.last().map(|call| call.station)
    }

    /// The label of the newest call that still runs.
    pub(crate) fn running_label(&self) -> Option<&str> {
        self.running.last().map(|call| call.label.as_str())
    }

    pub(crate) fn end(&mut self, id: &str, failed: bool, at: u64) {
        let Some(place) = self.running.iter().position(|call| call.id == id) else { return };
        let call = self.running.remove(place);
        if call.station == Station::Files {
            // Its own earlier end, and holds over by now, go; at most `MAX_HELD` live at once.
            self.ended.retain(|(label, end)| *label != call.label && at < end + HOLD_MS);
            if self.ended.len() == MAX_HELD {
                self.ended.remove(0);
            }
            self.ended.push((call.label.clone(), at));
        }
        if failed {
            let used = &mut self.uses[call.station.at()];
            (used.failed, used.failing) = (used.failed.saturating_add(1), used.failing.saturating_add(1));
        }
        (self.stepped, self.thinking) = (false, at);
        self.latest = self.latest.max(at);
    }

    /// ottospace served (or refused) a read of an app pane for the task: its own record, not the
    /// agent's account. One use of Apps; refused, a failed one.
    pub(crate) fn app(&mut self, label: &str, failed: bool, at: u64) {
        self.used(Station::Apps, label, at);
        if failed {
            let used = &mut self.uses[Station::Apps.at()];
            (used.failed, used.failing) = (used.failed.saturating_add(1), used.failing.saturating_add(1));
        }
        self.latest = self.latest.max(at);
    }

    pub(crate) fn hook(&mut self, at: u64) {
        self.used(Station::Hooks, "", at);
    }

    /// The agent said it failed (`kind`); its turn may still go on.
    pub(crate) fn failed(&mut self, kind: &str) {
        self.error = Some(kind.to_string());
    }

    /// The turn or the process ended: its calls with it. `error`: why it failed.
    pub(crate) fn settle(&mut self, error: Option<String>, at: u64) {
        self.running.clear();
        self.stepped = false;
        self.error = error.or(self.error.take());
        self.latest = self.latest.max(at);
        self.ended.clear();
    }

    /// The labels of the files the agent is at now: its `Files` calls that run, and its last
    /// call if that was one and the agent still works. None for a conversation that is over (a
    /// call its record never ended). No clock in it: a shell line names no file. With each,
    /// whether its call runs still: one that ended may have made the file.
    pub fn files(&self, is: &Is) -> Vec<(&str, bool)> {
        if !is.live && !is.watched {
            return Vec::new();
        }
        let running = self.running.iter().filter(|call| call.station == Station::Files).map(|call| call.label.as_str());
        let works = self.pulse(is) == Pulse::Running;
        let last =
            self.last.as_ref().filter(|last| works && last.station == Station::Files).map(|last| last.label.as_str());
        let mut files: Vec<(&str, bool)> =
            running.map(|label| (label, true)).chain(last.map(|label| (label, false))).collect();
        files.sort_unstable_by(|a, b| a.0.cmp(b.0).then(b.1.cmp(&a.1)));
        files.dedup_by_key(|file| file.0);
        files
    }

    /// Nothing held: a file's history played (`Event::CaughtUp`).
    pub(crate) fn clear_holds(&mut self) {
        self.ended.clear();
    }

    /// `files`, and those whose call ended under `HOLD_MS` before `now` while the agent works:
    /// a burst of quick reads leaves a trail. Not for records read with no clock (a watched
    /// file's, all at once): their ends are not when they happened.
    pub fn files_held(&self, is: &Is, now: u64) -> Vec<(&str, bool)> {
        let mut files = self.files(is);
        if self.holds(is) {
            files.extend(self.held(now).map(|label| (label, false)));
            files.sort_unstable_by(|a, b| a.0.cmp(b.0).then(b.1.cmp(&a.1)));
            files.dedup_by_key(|file| file.0);
        }
        files
    }

    /// The newest file held at `now` while the agent works: where it stays when it is at none.
    pub fn file_held(&self, is: &Is, now: u64) -> Option<&str> {
        self.holds(is).then(|| self.held(now).last()).flatten()
    }

    /// When each hold ends, while any may: for the shell to look again then (`Model::due`).
    pub fn holds_end(&self, is: &Is) -> impl Iterator<Item = u64> {
        let ends = self.holds(is).then_some(self.ended.iter()).into_iter().flatten();
        ends.map(|(_, at)| at + HOLD_MS)
    }

    fn holds(&self, is: &Is) -> bool {
        is.held && self.pulse(is) == Pulse::Running
    }

    fn held(&self, now: u64) -> impl Iterator<Item = &str> {
        self.ended.iter().filter(move |(_, at)| now < at + HOLD_MS).map(|(label, _)| label.as_str())
    }

    /// The one file its agent is at now, of `files`: its newest call's if a file's, else its
    /// newest running call on a file. None with no file (a shell line, the turn over).
    pub fn at(&self, is: &Is) -> Option<&str> {
        let files = self.files(is);
        let last = self.last.as_ref().filter(|last| last.station == Station::Files).map(|last| last.label.as_str());
        let running = self.running.iter().rev().find(|call| call.station == Station::Files);
        let at =
            last.filter(|last| files.iter().any(|(file, _)| file == last)).or(running.map(|call| call.label.as_str()));
        at.filter(|at| files.iter().any(|(file, _)| file == at))
    }

    /// The centre's state, for a conversation in the state `is`. No clock in it.
    pub fn pulse(&self, is: &Is) -> Pulse {
        if self.error.is_some() {
            Pulse::Error
        } else if !is.live && !is.watched {
            Pulse::Ended
        } else if is.asking.is_some() {
            Pulse::NeedsYou
        } else if is.turn || (is.watched && !self.running.is_empty()) {
            Pulse::Running
        } else {
            Pulse::Idle
        }
    }

    /// The dial as drawn at `now`, for a conversation in the state `is`.
    pub fn dial(&self, now: u64, is: &Is) -> Dial {
        let model_lit = is.turn && self.running.is_empty() && is.asking.is_none();
        let weights = Station::DIAL.map(|s| decayed(self.uses[s.at()].weight, self.uses[s.at()].at, now));
        let heaviest = weights.iter().cloned().fold(0., f64::max);
        let stations = Station::DIAL.map(|station| {
            let used = self.uses[station.at()];
            let calls = self.running.iter().filter(|call| call.station == station);
            let since = match station {
                Station::Model if model_lit => Some(self.thinking),
                Station::You if is.asking.is_some() => None,
                _ => calls.clone().map(|call| call.since).min(),
            };
            let waits = (station == Station::Model && model_lit) || (station == Station::You && is.asking.is_some());
            let running = (calls.count() as u32).max(u32::from(waits));
            let lit = running > 0;
            let weight = if heaviest > 0. { (weights[station.at()] / heaviest) as f32 } else { 0. };
            let since = since.filter(|_| is.timed && lit).map(|since| now.saturating_sub(since));
            let (calls, failed, failing) = (used.calls, used.failed, used.failing);
            Spot { station, calls, failed, failing, lit, running, weight, for_ms: since }
        });
        let (context, level) = context_used(is.context, is.window);
        let pulse = self.pulse(is);
        let lines = self.lines(now, is, pulse, model_lit);
        let failed = stations.iter().any(|spot| spot.failing > 0);
        let lit = stations.iter().any(|spot| spot.lit && spot.station != Station::You);
        let ring = ring_tip(context, is.context, is.window);
        Dial { pulse, stations: stations.to_vec(), context, level, failed, lit, timed: is.timed, lines, ring }
    }

    fn lines(&self, now: u64, is: &Is, pulse: Pulse, model_lit: bool) -> Lines {
        let clock = |since: u64| is.timed.then(|| clock(now.saturating_sub(since)));
        let said = |parts: Vec<Option<String>>| parts.into_iter().flatten().collect::<Vec<_>>().join(" · ");
        let label = |station: Station, label: &str| {
            let label = crate::transcript::clean(label).replace('\n', " ");
            Some(label).filter(|l| !l.trim().is_empty() && !l.eq_ignore_ascii_case(station.name()))
        };
        let ago = |at: u64| match crate::tasks::age(at, now).as_str() {
            "now" => "just now".to_string(),
            age => format!("{age} ago"),
        };
        let newest = self.running.last();
        // The running time, under the centre: the newest call's, else the model's.
        let timer = match (is.asking, newest) {
            (Some(_), _) => None,
            (None, Some(call)) => clock(call.since),
            (None, None) => model_lit.then(|| clock(self.thinking)).flatten(),
        };
        let now_line = if let Some(asked) = is.asking {
            Some(said(vec![Some("Needs you".into()), label(Station::You, asked)]))
        } else if let Some(call) = newest {
            let more = (self.running.len() > 1).then(|| format!("{} more", self.running.len() - 1));
            Some(said(vec![Some(call.station.name().into()), label(call.station, &call.label), more]))
        } else if model_lit {
            Some(Station::Model.name().into())
        } else if let (Pulse::Error, Some(kind)) = (pulse, &self.error) {
            Some(said(vec![Some("Failed".into()), label(Station::Other, kind), Some(ago(self.latest))]))
        } else {
            let last = self.last.as_ref();
            last.map(|last| {
                said(vec![Some(last.station.name().into()), label(last.station, &last.label), Some(ago(last.at))])
            })
        };
        let end = if is.live || is.watched { now } else { self.latest };
        let turns = (self.turns > 0).then(|| {
            let plural = if self.turns == 1 { "turn" } else { "turns" };
            let lasted = self.first.map(|first| span(end.saturating_sub(first)));
            said(vec![Some(format!("{} {plural}", self.turns)), lasted])
        });
        Lines { now: now_line, turns, timer }
    }
}

/// `weight` as of `then`, halved every `HALF_LIFE_MS` until `now`.
fn decayed(weight: f64, then: u64, now: u64) -> f64 {
    weight * 0.5f64.powf(now.saturating_sub(then) as f64 / HALF_LIFE_MS as f64)
}

/// A running time: `0:14`, `12:05`, from an hour `1h02` (it fits inside the ring).
fn clock(ms: u64) -> String {
    let s = ms / 1000;
    match s {
        0..3600 => format!("{}:{:02}", s / 60, s % 60),
        _ => format!("{}h{:02}", s / 3600, s / 60 % 60),
    }
}

/// How long something lasted, roughly: `40s`, `12m`, `2h 5m`.
fn span(ms: u64) -> String {
    let s = ms / 1000;
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        _ => format!("{}h {}m", s / 3600, s / 60 % 60),
    }
}

/// Tokens as said: `950`, `84k`, `1.2M`.
fn tokens(n: u64) -> String {
    match n {
        0..1000 => n.to_string(),
        1000..1_000_000 => format!("{}k", n / 1000),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

fn ring_tip(used: Option<f32>, context: Option<u64>, window: Option<u64>) -> Option<String> {
    match (context, window, used) {
        (Some(context), Some(window), Some(used)) => Some(format!(
            "{}% of context used\n{} of {} tokens",
            (used * 100.).round() as u32,
            tokens(context),
            tokens(window)
        )),
        (Some(context), _, _) => Some(format!("{} tokens", tokens(context))),
        _ => None,
    }
}

/// What the dial needs of its conversation beside its activity.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Is<'a> {
    pub live: bool,
    pub watched: bool,
    /// A turn runs.
    pub turn: bool,
    /// A request waits: its label.
    pub asking: Option<&'a str>,
    pub context: Option<u64>,
    pub window: Option<u64>,
    /// Times are known as they happen: false for a watched session, read a message at a time.
    pub timed: bool,
    /// Its calls' ends are known as they happen, so a file holds after one (`HOLD_MS`): timed,
    /// or a watched file read to its end once (`Event::CaughtUp`). Its starts may not be: a call
    /// running when the file was first read started earlier than it was seen.
    pub held: bool,
}

/// The share of its context window a conversation used (`context` of `window` tokens), and
/// its level: amber from `AMBER`, red from `RED`. None without both.
pub fn context_used(context: Option<u64>, window: Option<u64>) -> (Option<f32>, Level) {
    let used = match (context, window) {
        (Some(context), Some(window)) if window > 0 => Some((context as f64 / window as f64).min(1.) as f32),
        _ => None,
    };
    let level = match used {
        Some(f) if f >= RED => Level::Red,
        Some(f) if f >= AMBER => Level::Amber,
        _ => Level::Calm,
    };
    (used, level)
}

/// The centre's state, coloured as a task's (`theme::pulse`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Pulse {
    Running,
    NeedsYou,
    Idle,
    Ended,
    Error,
}

/// The context ring's colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Calm,
    Amber,
    Red,
}

/// A station as drawn.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Spot {
    pub station: Station,
    pub calls: u32,
    pub failed: u32,
    /// Failed since the user's last message.
    pub failing: u32,
    /// A call there is in progress (Model: the model has the turn; You: a request waits).
    pub lit: bool,
    /// How many are in progress there now: its dot is larger for more.
    pub running: u32,
    /// Its use, decayed, against the heaviest's: 0 to 1. Not in the state: it changes with time.
    #[serde(skip)]
    pub weight: f32,
    /// How long it has been lit, when known.
    #[serde(skip)]
    pub for_ms: Option<u64>,
}

impl Spot {
    /// Its hover: `Shell · 12 calls · 1 failed`.
    pub fn tip(&self) -> String {
        let (one, many) = self.station.unit();
        let mut tip = format!("{} · {} {}", self.station.name(), self.calls, if self.calls == 1 { one } else { many });
        if self.failed > 0 {
            tip += &format!(" · {} failed", self.failed);
        }
        tip
    }
}

/// The lines beside the dial; each may be missing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lines {
    /// What it does now (`Shell · cargo test`), else the last thing and when.
    pub now: Option<String>,
    /// `4 turns · 12m`.
    pub turns: Option<String>,
    /// The running call's or the model's time, under the centre: `0:14`.
    pub timer: Option<String>,
}

/// The dial at one moment. The state reply has its structure and counts, never its lines.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Dial {
    pub pulse: Pulse,
    /// In `Station::DIAL`'s order.
    pub stations: Vec<Spot>,
    /// The context used, 0 to 1.
    pub context: Option<f32>,
    pub level: Level,
    /// A call failed since the user's last message.
    pub failed: bool,
    /// A call or the model is at work.
    pub lit: bool,
    pub timed: bool,
    #[serde(skip)]
    pub lines: Lines,
    /// The ring's hover: `84k of 200k tokens`.
    #[serde(skip)]
    pub ring: Option<String>,
}

impl Dial {
    pub fn spot(&self, station: Station) -> Option<&Spot> {
        self.stations.iter().find(|spot| spot.station == station)
    }

    /// The model has the turn and no call runs, with times known: the centre's glyph turns.
    pub fn thinking(&self) -> bool {
        self.timed && self.spot(Station::Model).is_some_and(|spot| spot.lit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A station's id is its serde name, and names it back (a session's reader writes it).
    #[test]
    fn a_stations_id_is_its_serde_name_and_names_it() {
        for station in Station::DIAL {
            assert_eq!(serde_json::to_value(station).unwrap(), station.id());
            assert_eq!(Station::named(station.id()), Some(station));
        }
    }

    /// Why, what is done, as it is then, when, and the lines: now, turns, the timer.
    type Case = (&'static str, Box<dyn Fn(&mut Activity)>, Is<'static>, u64, [Option<&'static str>; 3]);

    fn live<'a>() -> Is<'a> {
        Is { live: true, timed: true, held: true, ..Is::default() }
    }

    #[test]
    fn the_files_an_agent_is_at_are_its_running_calls_and_its_last_while_it_works() {
        let mut a = Activity::default();
        a.message(0);
        a.start("1", Station::Files, "src/a.rs", 1);
        a.start("2", Station::Shell, "cargo test", 2);
        let working = Is { turn: true, ..live() };
        assert_eq!(a.files(&working), [("src/a.rs", true)]);
        // Its last call was the shell's: the file's call ended, the agent is at no file.
        a.end("1", false, 3);
        assert_eq!(a.files(&working), []);
        a.end("2", false, 4);
        a.start("3", Station::Files, "src/b.rs", 5);
        a.end("3", false, 6);
        assert_eq!(a.files(&working), [("src/b.rs", false)]);
        // Its turn over: none. A call that runs with no turn (asked before it): still there.
        assert_eq!(a.files(&live()), []);
        a.start("4", Station::Files, "src/c.rs", 7);
        assert_eq!(a.files(&live()), [("src/c.rs", true)]);
        // The same file again, its call running: said once, as running.
        a.start("5", Station::Files, "src/b.rs", 8);
        assert_eq!(a.files(&working), [("src/b.rs", true), ("src/c.rs", true)]);
        // The conversation over, with a call its record never ended: none.
        assert_eq!(a.files(&Is::default()), []);
    }

    #[test]
    fn calls_light_their_station_and_count_and_a_failure_is_red_until_the_next_message() {
        let mut a = Activity::default();
        a.message(0);
        a.start("1", Station::Shell, "cargo test", 1_000);
        let is = Is { turn: true, ..live() };
        let dial = a.dial(15_000, &is);
        let shell = dial.spot(Station::Shell).cloned().unwrap();
        assert_eq!((shell.calls, shell.lit, shell.for_ms), (1, true, Some(14_000)));
        assert_eq!(
            (dial.lines.now.as_deref(), dial.lines.timer.as_deref()),
            (Some("Shell · cargo test"), Some("0:14"))
        );
        assert_eq!((dial.pulse, dial.lit, dial.failed), (Pulse::Running, true, false));
        // One model request for the step that asked for the call.
        assert_eq!(dial.spot(Station::Model).map(|s| (s.calls, s.lit)), Some((1, false)));
        a.end("1", true, 16_000);
        let dial = a.dial(17_000, &is);
        assert_eq!(dial.spot(Station::Shell).map(|s| (s.failed, s.failing, s.lit)), Some((1, 1, false)));
        assert!(dial.failed);
        // The model has the turn again: lit, since the call ended.
        assert_eq!(dial.spot(Station::Model).map(|s| (s.lit, s.for_ms)), Some((true, Some(1_000))));
        assert_eq!((dial.lines.now.as_deref(), dial.lines.timer.as_deref()), (Some("Model"), Some("0:01")));
        a.message(20_000);
        let dial = a.dial(20_000, &is);
        assert!(!dial.failed, "the user's next message clears it");
        assert_eq!(dial.spot(Station::Shell).map(|s| (s.failed, s.failing)), Some((1, 0)));
    }

    #[test]
    fn the_lines_say_what_it_does_or_did_and_how_long_ago() {
        let mut a = Activity::default();
        let cases: Vec<Case> = vec![
            ("nothing yet", Box::new(|_| {}), live(), 0, [None, None, None]),
            (
                "a request waits",
                Box::new(|a| a.message(0)),
                Is { asking: Some("git push"), turn: true, context: Some(62), window: Some(100), ..live() },
                5_000,
                [Some("Needs you · git push"), Some("1 turn · 5s"), None],
            ),
            (
                "idle: the last call and when",
                Box::new(|a| {
                    a.start("2", Station::Files, "src/main.rs", 10_000);
                    a.end("2", false, 11_000);
                    a.settle(None, 12_000);
                }),
                live(),
                190_000,
                [Some("Files · src/main.rs · 3m ago"), Some("1 turn · 3m"), None],
            ),
            (
                "a label that only names its station is not said twice",
                Box::new(|a| a.start("3", Station::Shell, "shell", 200_000)),
                Is { turn: true, ..live() },
                200_000,
                [Some("Shell"), Some("1 turn · 3m"), Some("0:00")],
            ),
            (
                "ended: no clock runs on",
                Box::new(|a| a.settle(None, 210_000)),
                Is::default(),
                900_000,
                [Some("Shell · 11m ago"), Some("1 turn · 3m"), None],
            ),
            (
                "failed",
                Box::new(|a| a.settle(Some("rate_limit".into()), 220_000)),
                Is::default(),
                400_000,
                [Some("Failed · rate_limit · 3m ago"), Some("1 turn · 3m"), None],
            ),
        ];
        for (why, step, is, now, want) in cases {
            step(&mut a);
            let lines = a.dial(now, &is).lines;
            assert_eq!([lines.now.as_deref(), lines.turns.as_deref(), lines.timer.as_deref()], want, "{why}");
        }
    }

    #[test]
    fn the_centre_is_coloured_by_state() {
        let mut a = Activity::default();
        let cases = [
            (Is { turn: true, ..live() }, Pulse::Running),
            (Is { asking: Some("x"), turn: true, ..live() }, Pulse::NeedsYou),
            (live(), Pulse::Idle),
            (Is::default(), Pulse::Ended),
            (Is { watched: true, ..Is::default() }, Pulse::Idle),
        ];
        for (is, want) in cases {
            assert_eq!(a.dial(0, &is).pulse, want, "{is:?}");
        }
        a.start("w", Station::Files, "a.rs", 0);
        assert_eq!(a.dial(0, &Is { watched: true, ..Is::default() }).pulse, Pulse::Running, "a watched call");
        a.settle(Some("exited".into()), 1);
        assert_eq!(a.dial(2, &live()).pulse, Pulse::Error);
        a.message(3);
        assert_eq!(a.dial(4, &live()).pulse, Pulse::Idle, "the user's message clears it");
    }

    #[test]
    fn the_ring_turns_amber_then_red() {
        let a = Activity::default();
        let cases = [
            (None, Some(100), None, Level::Calm, None),
            (Some(50), None, None, Level::Calm, Some("50 tokens")),
            (Some(69), Some(100), Some(0.69), Level::Calm, Some("69% of context used\n69 of 100 tokens")),
            (Some(75_000), Some(100_000), Some(0.75), Level::Amber, Some("75% of context used\n75k of 100k tokens")),
            (Some(920_000), Some(1_000_000), Some(0.92), Level::Red, Some("92% of context used\n920k of 1.0M tokens")),
            (Some(5), Some(0), None, Level::Calm, Some("5 tokens")),
            (Some(300), Some(200), Some(1.), Level::Red, Some("100% of context used\n300 of 200 tokens")),
        ];
        for (context, window, fraction, level, tip) in cases {
            let dial = a.dial(0, &Is { context, window, ..live() });
            assert_eq!(
                (dial.context, dial.level, dial.ring.as_deref()),
                (fraction, level, tip),
                "{context:?}/{window:?}"
            );
        }
    }

    #[test]
    fn the_timer_fits_inside_the_ring() {
        let cases = [(14_000, "0:14"), (725_000, "12:05"), (3_723_000, "1h02"), (36_000_000, "10h00")];
        for (ms, want) in cases {
            assert_eq!(clock(ms), want);
        }
    }

    #[test]
    fn spokes_weigh_recent_use() {
        let mut a = Activity::default();
        for n in 0..4 {
            a.start(&format!("f{n}"), Station::Files, "a.rs", 0);
        }
        a.start("s", Station::Shell, "ls", HALF_LIFE_MS);
        let dial = a.dial(HALF_LIFE_MS, &live());
        let weight = |s| dial.spot(s).map(|spot| spot.weight);
        // Four uses a half-life ago weigh two; the newest one, one.
        assert_eq!((weight(Station::Files), weight(Station::Shell)), (Some(1.), Some(0.5)));
        assert_eq!(weight(Station::Network), Some(0.));
    }

    #[test]
    fn mcp_has_its_own_place_and_name() {
        assert_eq!((Station::Mcp.at(), Station::Mcp.name()), (5, "MCP"));
        assert_eq!(serde_json::to_string(&Station::Mcp).unwrap(), "\"mcp\"");
        let mut a = Activity::default();
        a.start("1", Station::Mcp, "github/create_issue", 1_000);
        let dial = a.dial(2_000, &live());
        assert_eq!(dial.spot(Station::Mcp).map(|s| s.lit), Some(true));
        assert_eq!(dial.spot(Station::Mcp).map(Spot::tip).as_deref(), Some("MCP · 1 call"));
    }

    #[test]
    fn a_watched_session_steps_without_clocks() {
        let mut a = Activity::default();
        a.message(0);
        a.start("1", Station::Network, "example.com", 1_000);
        let dial = a.dial(9_000, &Is { watched: true, ..Is::default() });
        assert_eq!(dial.spot(Station::Network).map(|s| (s.lit, s.for_ms)), Some((true, None)));
        assert_eq!(dial.lines.now.as_deref(), Some("Network · example.com"));
        assert!(!dial.timed);
    }

    #[test]
    fn hovers_name_the_station_and_its_counts() {
        let mut a = Activity::default();
        a.message(0);
        a.hook(1);
        a.start("1", Station::Shell, "cargo", 2);
        a.start("2", Station::Shell, "cargo", 2);
        a.end("1", true, 3);
        let dial = a.dial(4, &live());
        let tips: Vec<String> = [Station::You, Station::Model, Station::Shell, Station::Hooks, Station::Other]
            .iter()
            .filter_map(|s| dial.spot(*s).map(Spot::tip))
            .collect();
        assert_eq!(
            tips,
            ["You · 1 message", "Model · 1 request", "Shell · 2 calls · 1 failed", "Hooks · 1 run", "Tools · 0 calls"]
        );
        // The state has counts, never a label.
        let state = serde_json::to_string(&dial).unwrap();
        assert!(!state.contains("cargo") && !state.contains("weight"), "{state}");
    }
}
