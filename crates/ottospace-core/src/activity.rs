//! A conversation's activity, for the dial over its transcript: where its calls go (stations),
//! how much each is used (decayed), what is lit now, what failed since the user's last message,
//! the route of the last calls, and the lines beside it. Folded from the transcript's events,
//! times passed in. Design: `docs/ai/agent-dial.md`.

use std::collections::VecDeque;

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
    Browser,
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
        Station::Browser,
        Station::Apps,
        Station::Agents,
        Station::Skills,
        Station::Memory,
        Station::Hooks,
        Station::Other,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Station::You => "You",
            Station::Model => "Model",
            Station::Memory => "Memory",
            Station::Files => "Files",
            Station::Shell => "Shell",
            Station::Network => "Network",
            Station::Browser => "Browser",
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
/// The route: the last stations gone to, each fading out over `ROUTE_MS`.
pub const ROUTE: usize = 6;
pub const ROUTE_MS: u64 = 60_000;
/// The context ring turns amber, then red.
pub const AMBER: f32 = 0.7;
pub const RED: f32 = 0.9;
/// A lit call's arcs fill in a minute (inner) and ten (outer).
pub const INNER_MS: u64 = 60_000;
pub const OUTER_MS: u64 = 600_000;
/// The most calls kept as running: a stream that starts calls and never ends them stays small.
const MAX_RUNNING: usize = 64;

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
    route: VecDeque<(Station, u64)>,
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
}

impl Activity {
    fn used(&mut self, station: Station, label: &str, at: u64) {
        let used = &mut self.uses[station.at()];
        used.weight = decayed(used.weight, used.at, at) + 1.;
        (used.at, used.calls) = (used.at.max(at), used.calls.saturating_add(1));
        self.first.get_or_insert(at);
        self.latest = self.latest.max(at);
        match self.route.back_mut() {
            Some((last, when)) if *last == station => *when = at,
            _ => self.route.push_back((station, at)),
        }
        while self.route.len() > ROUTE {
            self.route.pop_front();
        }
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

    pub(crate) fn end(&mut self, id: &str, failed: bool, at: u64) {
        let Some(place) = self.running.iter().position(|call| call.id == id) else { return };
        let call = self.running.remove(place);
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
        let route = self.route.iter().filter(|(_, at)| now.saturating_sub(*at) < ROUTE_MS);
        let route = route.map(|(station, at)| Leg { station: *station, age_ms: now.saturating_sub(*at) }).collect();
        let context = match (is.context, is.window) {
            (Some(context), Some(window)) if window > 0 => Some((context as f64 / window as f64).min(1.) as f32),
            _ => None,
        };
        let level = match context {
            Some(f) if f >= RED => Level::Red,
            Some(f) if f >= AMBER => Level::Amber,
            _ => Level::Calm,
        };
        let pulse = self.pulse(is);
        let lines = self.lines(now, is, pulse, context, model_lit);
        let failed = stations.iter().any(|spot| spot.failing > 0);
        let lit = stations.iter().any(|spot| spot.lit && spot.station != Station::You);
        let ring = ring_tip(is.context, is.window);
        Dial { pulse, stations: stations.to_vec(), context, level, failed, lit, route, timed: is.timed, lines, ring }
    }

    fn lines(&self, now: u64, is: &Is, pulse: Pulse, context: Option<f32>, model_lit: bool) -> Lines {
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
        let now_line = if let Some(asked) = is.asking {
            Some(said(vec![Some("Needs you".into()), label(Station::You, asked)]))
        } else if let Some(call) = newest {
            let more = (self.running.len() > 1).then(|| format!("{} more", self.running.len() - 1));
            Some(said(vec![
                Some(call.station.name().into()),
                label(call.station, &call.label),
                clock(call.since),
                more,
            ]))
        } else if model_lit {
            Some(said(vec![Some(Station::Model.name().into()), clock(self.thinking)]))
        } else if let (Pulse::Error, Some(kind)) = (pulse, &self.error) {
            Some(said(vec![Some("Failed".into()), label(Station::Other, kind), Some(ago(self.latest))]))
        } else {
            let last = self.last.as_ref();
            last.map(|last| {
                said(vec![Some(last.station.name().into()), label(last.station, &last.label), Some(ago(last.at))])
            })
        };
        let context = context.map(|f| format!("{}% of context", (f * 100.).round() as u32));
        let end = if is.live || is.watched { now } else { self.latest };
        let turns = (self.turns > 0).then(|| {
            let plural = if self.turns == 1 { "turn" } else { "turns" };
            let lasted = self.first.map(|first| span(end.saturating_sub(first)));
            said(vec![Some(format!("{} {plural}", self.turns)), lasted])
        });
        Lines { now: now_line, context, turns }
    }
}

/// `weight` as of `then`, halved every `HALF_LIFE_MS` until `now`.
fn decayed(weight: f64, then: u64, now: u64) -> f64 {
    weight * 0.5f64.powf(now.saturating_sub(then) as f64 / HALF_LIFE_MS as f64)
}

/// A running time: `0:14`, `12:05`, `1:02:03`.
fn clock(ms: u64) -> String {
    let s = ms / 1000;
    match s {
        0..3600 => format!("{}:{:02}", s / 60, s % 60),
        _ => format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60),
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

fn ring_tip(context: Option<u64>, window: Option<u64>) -> Option<String> {
    match (context, window) {
        (Some(context), Some(window)) if window > 0 => {
            Some(format!("{} of {} tokens", tokens(context), tokens(window)))
        }
        (Some(context), _) => Some(format!("{} tokens", tokens(context))),
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

/// A stop on the route, oldest first.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Leg {
    pub station: Station,
    #[serde(skip)]
    pub age_ms: u64,
}

/// The lines beside the dial; each may be missing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lines {
    /// What it does now (`Shell · cargo test · 0:14`), else the last thing and when.
    pub now: Option<String>,
    /// `62% of context`.
    pub context: Option<String>,
    /// `4 turns · 12m`.
    pub turns: Option<String>,
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
    pub route: Vec<Leg>,
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

/// The dial shows its lines only when there is room: the drawer is at least this wide.
pub const LINES_MIN_WIDTH: f32 = 360.;

#[cfg(test)]
mod tests {
    use super::*;

    /// Why, what is done, as it is then, when, and the three lines.
    type Case = (&'static str, Box<dyn Fn(&mut Activity)>, Is<'static>, u64, [Option<&'static str>; 3]);

    fn live<'a>() -> Is<'a> {
        Is { live: true, timed: true, ..Is::default() }
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
        assert_eq!(dial.lines.now.as_deref(), Some("Shell · cargo test · 0:14"));
        assert_eq!((dial.pulse, dial.lit, dial.failed), (Pulse::Running, true, false));
        // One model request for the step that asked for the call.
        assert_eq!(dial.spot(Station::Model).map(|s| (s.calls, s.lit)), Some((1, false)));
        a.end("1", true, 16_000);
        let dial = a.dial(17_000, &is);
        assert_eq!(dial.spot(Station::Shell).map(|s| (s.failed, s.failing, s.lit)), Some((1, 1, false)));
        assert!(dial.failed);
        // The model has the turn again: lit, since the call ended.
        assert_eq!(dial.spot(Station::Model).map(|s| (s.lit, s.for_ms)), Some((true, Some(1_000))));
        assert_eq!(dial.lines.now.as_deref(), Some("Model · 0:01"));
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
                [Some("Needs you · git push"), Some("62% of context"), Some("1 turn · 5s")],
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
                [Some("Files · src/main.rs · 3m ago"), None, Some("1 turn · 3m")],
            ),
            (
                "a label that only names its station is not said twice",
                Box::new(|a| a.start("3", Station::Shell, "shell", 200_000)),
                Is { turn: true, ..live() },
                200_000,
                [Some("Shell · 0:00"), None, Some("1 turn · 3m")],
            ),
            (
                "ended: no clock runs on",
                Box::new(|a| a.settle(None, 210_000)),
                Is::default(),
                900_000,
                [Some("Shell · 11m ago"), None, Some("1 turn · 3m")],
            ),
            (
                "failed",
                Box::new(|a| a.settle(Some("rate_limit".into()), 220_000)),
                Is::default(),
                400_000,
                [Some("Failed · rate_limit · 3m ago"), None, Some("1 turn · 3m")],
            ),
        ];
        for (why, step, is, now, want) in cases {
            step(&mut a);
            let lines = a.dial(now, &is).lines;
            assert_eq!([lines.now.as_deref(), lines.context.as_deref(), lines.turns.as_deref()], want, "{why}");
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
            (Some(69), Some(100), Some(0.69), Level::Calm, Some("69 of 100 tokens")),
            (Some(75_000), Some(100_000), Some(0.75), Level::Amber, Some("75k of 100k tokens")),
            (Some(920_000), Some(1_000_000), Some(0.92), Level::Red, Some("920k of 1.0M tokens")),
            (Some(5), Some(0), None, Level::Calm, Some("5 tokens")),
            (Some(300), Some(200), Some(1.), Level::Red, Some("300 of 200 tokens")),
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
    fn spokes_weigh_recent_use_and_the_route_fades_in_a_minute() {
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
        let route: Vec<Station> = a.dial(HALF_LIFE_MS, &live()).route.iter().map(|leg| leg.station).collect();
        assert_eq!(route, [Station::Shell], "the calls of ten minutes ago are off the route");
        // The route keeps the last few, a station once in a row.
        let mut b = Activity::default();
        for (n, station) in Station::DIAL.iter().enumerate() {
            b.start(&n.to_string(), *station, "", n as u64);
        }
        let route: Vec<Station> = b.dial(20, &live()).route.iter().map(|leg| leg.station).collect();
        use Station::*;
        assert_eq!(route, [Apps, Agents, Skills, Memory, Hooks, Other]);
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
