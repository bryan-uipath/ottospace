//! The meeting timeline: today's events from the user's calendar feed, the band of the workday
//! above the status bar, the pulse as one comes near, and the alert when one has
//! started and the user is in no call. Pure: the shell passes the time and whether a call is on.
//!
//! Titles, places and descriptions are other people's text: kept cleaned and short, drawn, and
//! never in the control socket's state, the trace or a brief. Titles and places reach a client
//! only through the `calendar` request (`reply`); descriptions and links never.

use std::net::IpAddr;

pub mod day;
pub mod fetch;
pub mod reply;

use crate::ics::{Occurrence, Show};

/// Why a client's Join or Dismiss did nothing.
pub const USERS_ALONE: &str = "only the user joins or dismisses a meeting";
/// curl's `max-time` for one fetch, in seconds.
pub const MAX_SECS: u32 = 30;
/// The Retry fetches the feed: no client makes the app reach the network.
pub const USERS_RETRY: &str = "only the user reads the calendar again";

const MINUTE: u64 = 60_000;
/// From here before its start a meeting pulses slowly; from `IMMINENT` faster.
pub const APPROACHING: u64 = 10 * MINUTE;
pub const IMMINENT: u64 = 2 * MINUTE;
/// The second alert comes this long after the first; there is no third.
pub const REPEAT: u64 = 2 * MINUTE;
/// A first alert only this long after the start: a meeting left early is no missed one.
pub const ALERT_WINDOW: u64 = 10 * MINUTE;
/// Whether a call is on is looked at from this long before a start.
const WATCH_BEFORE: u64 = MINUTE;
/// A call first seen less than this before a start is taken for that meeting, joined early; one
/// on from before is another call (the meeting before, running over), and the meeting alerts.
const JOINED_EARLY: u64 = APPROACHING;
const MAX_TITLE: usize = 120;
const MAX_LINK: usize = 2_000;
/// Events kept of the days held (`reply::HELD_DAYS`): as many as one expansion gives.
const MAX_EVENTS: usize = crate::ics::MAX_OCCURRENCES;
/// Days held before today, and the local midnights the model knows: from `BACK` days before today
/// to the end of the last day held. Today starts at `days[BACK]`.
pub const BACK: usize = reply::BACK_DAYS as usize;
pub const DAYS: usize = BACK + reply::HELD_DAYS as usize + 1;
/// The Calendar section's day, from today: `BACK` days back to `reply::MAX_DAYS` ahead.
const AHEAD: i32 = reply::MAX_DAYS as i32;

/// A step of the Calendar section's day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DayStep {
    Back,
    Forward,
    Today,
}

/// One occurrence of an event, as shown. Times in milliseconds since 1970.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// Stable across reads of the feed: of the event's UID and start.
    pub id: u64,
    pub start: u64,
    pub end: u64,
    pub all_day: bool,
    /// One line, characters that show, at most `MAX_TITLE`.
    pub title: String,
    pub location: String,
    /// An `https:` link `joinable` allows: a Zoom, Meet or Teams link first, else the event's URL.
    pub link: Option<String>,
    /// How it holds the user's time, as the feed says.
    pub show: Show,
}

/// How near an event is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// More than `APPROACHING` away.
    Far,
    Approaching,
    Imminent,
    Running,
    Past,
}

impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Stage::Far => "far",
            Stage::Approaching => "approaching",
            Stage::Imminent => "imminent",
            Stage::Running => "running",
            Stage::Past => "past",
        }
    }
}

pub fn stage(event: &Event, now: u64) -> Stage {
    match now {
        _ if now >= event.end.max(event.start.saturating_add(1)) => Stage::Past,
        _ if now >= event.start => Stage::Running,
        _ if event.start - now <= IMMINENT => Stage::Imminent,
        _ if event.start - now <= APPROACHING => Stage::Approaching,
        _ => Stage::Far,
    }
}

/// How strongly a block and the chip are lit, 0 to 1: a soft wave, slow while approaching, faster
/// when imminent; steady while running. With reduced motion, a steady step for each stage.
pub fn glow(stage: Stage, now: u64, reduced: bool) -> f32 {
    let wave = |period: u64| (1. - (std::f32::consts::TAU * (now % period) as f32 / period as f32).cos()) / 2.;
    match (stage, reduced) {
        (Stage::Far | Stage::Past, _) => 0.,
        (Stage::Approaching, true) => 0.45,
        (Stage::Imminent, true) => 0.8,
        (Stage::Approaching, false) => 0.15 + 0.45 * wave(2_400),
        (Stage::Imminent, false) => 0.35 + 0.65 * wave(900),
        (Stage::Running, _) => 1.,
    }
}

/// A call (a Zoom, Meet or Teams link) or a block of time held with none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Call,
    Block,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Call => "call",
            Kind::Block => "block",
        }
    }
}

pub fn kind(event: &Event) -> Kind {
    if has_meeting_link(event) { Kind::Call } else { Kind::Block }
}

/// A focus block: a block (no call link) whose title starts with the word `Focus` or the words
/// `Deep Work`, case and leading space ignored: `Focus: writing`, not `Focusing` or a `Focus sync` call.
pub fn focus(event: &Event) -> bool {
    kind(event) == Kind::Block && focus_title(&event.title)
}

fn focus_title(title: &str) -> bool {
    let title = title.trim_start().to_lowercase();
    // "deep", then space, then "work".
    let deep = title.strip_prefix("deep").map(|rest| (rest, rest.trim_start()));
    let deep = deep.and_then(|(rest, work)| work.strip_prefix("work").filter(|_| work.len() < rest.len()));
    let rest = title.strip_prefix("focus").or(deep);
    // The prefix ends at a word's end: not `Focusing`.
    rest.is_some_and(|rest| !rest.starts_with(char::is_alphanumeric))
}

/// A transcript started this long before a meeting is possibly of it: people start early.
pub const SOON: u64 = 5 * MINUTE;
/// One that ended this long before a transcript started, when nothing else matches: it ran over.
pub const JUST_ENDED: u64 = 5 * MINUTE;
/// Most entries a transcript's header names.
pub const MAX_POSSIBLE: usize = 3;

/// An entry a transcript may be of: timed, held busy or tentative, not a focus block, and not one
/// its organiser cancelled that stays on the calendar (Outlook's `Canceled: Standup`).
pub fn meeting(event: &Event) -> bool {
    let title = event.title.trim_start().to_lowercase();
    let cancelled = title.starts_with("canceled:") || title.starts_with("cancelled:");
    let held = matches!(event.show, Show::Busy | Show::Tentative);
    !event.all_day && event.end > event.start && held && !focus(event) && !cancelled
}

/// The meetings a transcript started at `at` is possibly of, the likeliest first: those on at
/// `at` and those starting within `SOON`, by how near their start is; else the last to end within
/// `JUST_ENDED`. At most `MAX_POSSIBLE`; none, often.
pub fn possible<'a>(events: impl IntoIterator<Item = &'a Event>, at: u64) -> Vec<&'a Event> {
    let meetings: Vec<&Event> = events.into_iter().filter(|e| meeting(e)).collect();
    let mut near: Vec<&Event> =
        meetings.iter().copied().filter(|e| e.start <= at.saturating_add(SOON) && at < e.end).collect();
    near.sort_by_key(|e| (e.start.abs_diff(at), e.end, e.id));
    if near.is_empty() {
        let ended = meetings.into_iter().filter(|e| e.end <= at && at - e.end <= JUST_ENDED);
        near.extend(ended.max_by_key(|e| (e.end, e.start)));
    }
    near.truncate(MAX_POSSIBLE);
    near
}

/// Most lanes calls share on the band; a call more at once goes into a lane's hover card.
pub const MAX_LANES: u8 = 4;

/// A call on the band: from and to as fractions of its width, its lane of the `lanes` its
/// overlapping calls share (0 the top), and the calls merged into it when more than `MAX_LANES`
/// overlap.
#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    pub id: u64,
    pub from: f32,
    pub to: f32,
    pub lane: u8,
    pub lanes: u8,
    pub stage: Stage,
    pub more: Vec<u64>,
}

/// A block on the band, drawn under the calls.
#[derive(Clone, Debug, PartialEq)]
pub struct Under {
    pub id: u64,
    pub from: f32,
    pub to: f32,
    pub stage: Stage,
}

/// The band of a day: its span, calls in lanes, blocks under them, and the blocks wholly before
/// or after the span (a marker at that end each).
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub band: Band,
    pub calls: Vec<Bar>,
    pub blocks: Vec<Under>,
    pub early: Vec<u64>,
    pub late: Vec<u64>,
}

/// A `calendar_colors` rule: a title that holds `text` takes the colour.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorRule {
    /// Lower case: matched anywhere in the title, case ignored.
    pub text: String,
    /// Into `theme::IDENTITY`.
    pub color: u8,
}

const MAX_RULE_TEXT: usize = 100;
/// Rules read of `calendar_colors`.
pub const MAX_RULES: usize = 64;

impl ColorRule {
    /// `{"match": "1:1", "color": "teal"}`, or why not.
    pub fn new(text: &str, color: &str) -> Result<ColorRule, String> {
        let text = text.trim();
        if text.is_empty() || text.chars().count() > MAX_RULE_TEXT {
            return Err(format!("match must be 1 to {MAX_RULE_TEXT} characters"));
        }
        let names = crate::theme::IDENTITY_NAMES;
        let at = names.iter().position(|name| name.eq_ignore_ascii_case(color.trim()));
        let at = at.ok_or_else(|| format!("color is one of {}", names.join(", ")))?;
        Ok(ColorRule { text: text.to_lowercase(), color: at as u8 })
    }
}

/// The first rule whose text the title holds, case ignored.
fn rule_for<'a>(title: &str, rules: &'a [ColorRule]) -> Option<&'a ColorRule> {
    let title = title.to_lowercase();
    rules.iter().find(|rule| title.contains(&rule.text))
}

/// How an entry is drawn: its colour, how strongly (0 to 1), and the colour's name for the state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paint {
    pub color: u32,
    pub alpha: f32,
    pub name: &'static str,
}

/// An entry's colour: a rule's, else by kind (calls the meeting colour, blocks muted, away its
/// own, a focus block its own). Its strength: by how it holds the time (tentative lighter, free faint) and by stage
/// (past faded, near pulsing up to the running one's full strength).
pub fn paint(event: &Event, stage: Stage, glow: f32, rules: &[ColorRule]) -> Paint {
    use crate::theme;
    let (color, name) = match (rule_for(&event.title, rules), event.show, kind(event)) {
        (Some(rule), ..) => (theme::identity(Some(rule.color)), theme::IDENTITY_NAMES[usize::from(rule.color)]),
        (None, Show::Away, _) => (theme::MEETING_AWAY, "away"),
        (None, _, Kind::Call) => (theme::MEETING, "meeting"),
        (None, _, Kind::Block) if focus_title(&event.title) => (theme::MEETING_FOCUS, "focus"),
        (None, _, Kind::Block) => (theme::MEETING_BLOCK, "block"),
    };
    let held = match event.show {
        Show::Busy | Show::Away => 1.,
        Show::Tentative => 0.55,
        Show::Free => 0.3,
    };
    let near = match stage {
        Stage::Past => 0.35,
        Stage::Far => 0.75,
        Stage::Approaching | Stage::Imminent => 0.75 + 0.25 * glow,
        Stage::Running => 1.,
    };
    Paint { color, alpha: held * near, name }
}

/// The workday (`calendar_workday`): its start and end, in minutes after local midnight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Workday {
    pub start: u32,
    pub end: u32,
}

impl Default for Workday {
    fn default() -> Self {
        Self { start: 9 * 60, end: 17 * 60 }
    }
}

/// What a wrong `calendar_workday` says.
pub const WORKDAY_FORM: &str = "calendar_workday must be \"HH:MM-HH:MM\" in 24-hour time, the start at least an \
                                hour before the end, e.g. \"09:00-17:00\"";

impl Workday {
    /// `09:00-17:00`: both times, the start at least an hour before the end. `24:00` ends the day.
    pub fn parse(text: &str) -> Option<Workday> {
        let minutes = |t: &str| -> Option<u32> {
            let (h, m) = t.trim().split_once(':')?;
            let two = |s: &str| (s.len() == 2 && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok())?;
            let (h, m): (u32, u32) = (two(h)?, two(m)?);
            (m < 60 && (h < 24 || (h, m) == (24, 0))).then_some(h * 60 + m)
        };
        let (start, end) = text.split_once('-')?;
        let (start, end) = (minutes(start)?, minutes(end)?);
        (start + 60 <= end).then_some(Workday { start, end })
    }
}

/// The band's span (ms): the workday, widened to hold today's meetings, never past today.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Band {
    pub from: u64,
    pub to: u64,
    /// The workday's start and end, where its marks go.
    pub workday: (u64, u64),
}

impl Band {
    /// Where `t` falls, 0 at the left end, 1 at the right; outside the band, pinned to its end.
    pub fn at(&self, t: u64) -> f32 {
        let span = self.to.saturating_sub(self.from).max(1);
        (t.clamp(self.from, self.to.max(self.from)) - self.from) as f32 / span as f32
    }
}

/// The local wall time `minutes` after the midnight `day` (ms), in a zone `offset(t)` seconds east
/// of UTC at `t`: 09:00 stays 09:00 on a day the clocks change, at midnight too (the offset
/// just before `day` is the one its wall clock started from).
fn local(day: u64, minutes: u32, offset: &dyn Fn(u64) -> i64) -> u64 {
    let naive = day + u64::from(minutes) * MINUTE;
    let shift = (offset(naive) - offset(day.saturating_sub(1))) * 1_000;
    naive.saturating_add_signed(-shift)
}

/// The timed events that overlap the day from midnight `day.0` to `day.1`.
fn of_day(events: &[Event], (start, end): (u64, u64)) -> impl Iterator<Item = &Event> {
    events.iter().filter(move |e| !e.all_day && e.start < end && (e.end > start || e.start >= start))
}

/// The band of the day from midnight `day.0` to `day.1`: the workday, widened to the day's
/// earliest call and latest, so no call is hidden; one over midnight is cut there. Blocks never
/// widen it: a reminder at 00:00 would stretch it for nothing.
pub fn band(events: &[Event], day: (u64, u64), workday: Workday, offset: &dyn Fn(u64) -> i64) -> Band {
    let hours = (local(day.0, workday.start, offset), local(day.0, workday.end, offset).min(day.1));
    let mut band = Band { from: hours.0, to: hours.1, workday: hours };
    for e in of_day(events, day).filter(|e| kind(e) == Kind::Call) {
        band.from = band.from.min(e.start.max(day.0));
        band.to = band.to.max(e.end.min(day.1));
    }
    band
}

/// The day's layout on `band`: calls in as many lanes as overlap (at most `MAX_LANES`, the rest
/// merged into a lane's call), blocks under them, and blocks wholly outside it at its ends.
fn layout(events: &[Event], day: (u64, u64), band: Band, now: u64) -> Layout {
    let mut shown: Vec<&Event> = of_day(events, day).collect();
    shown.sort_by_key(|e| (e.start, e.end));
    let mut out = Layout { band, calls: Vec::new(), blocks: Vec::new(), early: Vec::new(), late: Vec::new() };
    for e in shown.iter().filter(|e| kind(e) == Kind::Block) {
        if e.start < band.from && e.end <= band.from {
            out.early.push(e.id);
        } else if e.start > band.to || (e.start == band.to && e.end > e.start) {
            out.late.push(e.id);
        } else {
            out.blocks.push(Under { id: e.id, from: band.at(e.start), to: band.at(e.end), stage: stage(e, now) });
        }
    }
    let calls: Vec<&Event> = shown.into_iter().filter(|e| kind(e) == Kind::Call).collect();
    out.calls = lanes(&calls, band, now);
    out
}

/// Events (by start) in columns, as Outlook sets them side by side: those that overlap, one after
/// another, form a cluster; each takes the first column free at its start, and all of a cluster
/// share its count. More than `MAX_LANES` at once: merged into the column that frees last. The
/// band's calls and the Calendar section's day view alike.
fn lanes(events: &[&Event], band: Band, now: u64) -> Vec<Bar> {
    let mut out: Vec<Bar> = Vec::new();
    let ends = |e: &Event| e.end.max(e.start + 1);
    let mut at = 0;
    while at < events.len() {
        let (first, mut end) = (at, ends(events[at]));
        while at + 1 < events.len() && events[at + 1].start < end {
            at += 1;
            end = end.max(ends(events[at]));
        }
        at += 1;
        // Each lane: when it frees, and its bar.
        let mut lanes: Vec<(u64, usize)> = Vec::new();
        let base = out.len();
        for e in &events[first..at] {
            let lane = match lanes.iter().position(|(until, _)| *until <= e.start) {
                Some(lane) => lane,
                None if lanes.len() < usize::from(MAX_LANES) => {
                    lanes.push((0, 0));
                    lanes.len() - 1
                }
                None => {
                    // Merged into the lane that frees last; the longer of the two is drawn, so a
                    // long call is never hidden behind a short one.
                    let lane = (0..lanes.len()).max_by_key(|l| lanes[*l].0).unwrap_or(0);
                    let bar = &mut out[lanes[lane].1];
                    if ends(e) > lanes[lane].0 {
                        let (from, to, stage) = (band.at(e.start), band.at(e.end), stage(e, now));
                        let id = std::mem::replace(&mut bar.id, e.id);
                        (bar.from, bar.to, bar.stage) = (bar.from.min(from), to, stage);
                        bar.more.push(id);
                        lanes[lane].0 = ends(e);
                    } else {
                        bar.more.push(e.id);
                    }
                    continue;
                }
            };
            lanes[lane] = (ends(e), out.len());
            let (from, to, stage) = (band.at(e.start), band.at(e.end), stage(e, now));
            out.push(Bar { id: e.id, from, to, lane: lane as u8, lanes: 0, stage, more: Vec::new() });
        }
        let count = lanes.len() as u8;
        out[base..].iter_mut().for_each(|bar| bar.lanes = count);
    }
    out
}

/// What a meeting's hover card says, the band's and the Calendar section's alike.
#[derive(Clone, Debug, PartialEq)]
pub struct Facts {
    pub title: String,
    /// `09:00–09:30 · 30m`, or `all day`.
    pub when: String,
    /// Without its links: a Zoom link often is the place, and holds a passcode.
    pub place: Option<String>,
    /// `Zoom`, `Meet` or `Teams`.
    pub link: Option<&'static str>,
    /// `tentative`, `free` or `out of office`; none when busy.
    pub status: Option<&'static str>,
    /// The titles of the calls merged into it on the band.
    pub more: Vec<String>,
}

pub fn facts(event: &Event, more: &[&Event], offset: i64) -> Facts {
    let when = if event.all_day {
        "all day".to_string()
    } else {
        let length = span(event.end.saturating_sub(event.start));
        format!("{}–{} · {length}", clock(event.start, offset), clock(event.end, offset))
    };
    let status = match event.show {
        Show::Busy => None,
        Show::Tentative => Some("tentative"),
        Show::Free => Some("free"),
        Show::Away => Some("out of office"),
    };
    Facts {
        title: event.title.clone(),
        when,
        place: Some(reply::text(&event.location)).filter(|l| !l.is_empty()),
        link: meeting_service(event),
        status,
        more: more.iter().map(|e| e.title.clone()).collect(),
    }
}

/// `12m`, `1h 5m`, `3h`: a time to come, rounded up to the minute.
pub fn span(ms: u64) -> String {
    let minutes = ms.div_ceil(MINUTE);
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// `09:05`: a time of day in a zone `offset` seconds east of UTC.
pub fn clock(ms: u64, offset: i64) -> String {
    let minutes = (ms / MINUTE) as i64 + offset / 60;
    format!("{:02}:{:02}", minutes.rem_euclid(1_440) / 60, minutes.rem_euclid(60))
}

/// `Mon 16 Nov`: the date of `ms` in a zone `offset` seconds east of UTC.
pub fn day_name(ms: u64, offset: i64) -> String {
    let day = ((ms / 1_000) as i64 + offset).div_euclid(86_400);
    let (_, month, date) = crate::ics::civil(day);
    let weekday = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][day.rem_euclid(7) as usize];
    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{weekday} {date} {}", months[(month - 1).clamp(0, 11) as usize])
}

/// A meeting's facts as text, for a tooltip that lists several (all-day events, blocks outside
/// the band): `Design review`, `09:00–09:30 · 30m · Room 4`, `Zoom link · tentative`.
pub fn describe(event: &Event, offset: i64) -> String {
    let f = facts(event, &[], offset);
    let place = f.place.map(|l| format!(" · {l}")).unwrap_or_default();
    let more: Vec<String> =
        f.link.map(|name| format!("{name} link")).into_iter().chain(f.status.map(String::from)).collect();
    let more = if more.is_empty() { String::new() } else { format!("\n{}", more.join(" · ")) };
    format!("{}\n{}{place}{more}", f.title, f.when)
}

/// The text of the events read, as shown: cleaned and capped. The last of `MAX_EVENTS` go.
pub fn events(occurrences: &[Occurrence]) -> Vec<Event> {
    let ms = |s: i64| u64::try_from(s).unwrap_or(0).saturating_mul(1_000);
    let mut out: Vec<Event> = occurrences
        .iter()
        .map(|o| {
            let title = crate::capture::plain(&o.summary, MAX_TITLE);
            Event {
                id: id(&o.uid, o.start),
                start: ms(o.start),
                end: ms(o.end.max(o.start)),
                all_day: o.all_day,
                title: if title.is_empty() { "(no title)".into() } else { title },
                location: crate::capture::plain(&o.location, MAX_TITLE),
                link: link(&[&o.location, &o.description, &o.url]),
                show: o.show,
            }
        })
        .collect();
    out.truncate(MAX_EVENTS);
    out
}

/// FNV-1a of the UID and start: an occurrence keeps its id when the feed is read again.
fn id(uid: &str, start: i64) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in uid.bytes().chain(start.to_le_bytes()) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// The meeting link of an event's text: the first Zoom, Meet or Teams one, else the first link
/// `joinable` allows.
fn link(texts: &[&str]) -> Option<String> {
    let found: Vec<&str> = texts.iter().flat_map(|text| urls(text)).filter(|url| joinable(url)).collect();
    let meeting = found.iter().find(|url| host(url).and_then(meeting_host).is_some());
    meeting.or(found.first()).map(|url| url.to_string())
}

/// The `https://` links in a text, each up to a space, a quote or a bracket.
fn urls(text: &str) -> impl Iterator<Item = &str> {
    text.match_indices("https://").map(move |(at, _)| {
        let rest = &text[at..];
        let end = rest.find(|c: char| c.is_whitespace() || "\"'<>()[]{}`\\|^".contains(c)).unwrap_or(rest.len());
        rest[..end].trim_end_matches(['.', ',', ';', ':', '!', '?'])
    })
}

/// The one test before a link is opened: `https:`, a host of letters, digits, dots and dashes,
/// no user name, no space, quote or control character. Zoom, Meet and Teams links pass.
pub fn joinable(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let clean = url.len() <= MAX_LINK && url.chars().all(|c| c.is_ascii_graphic() && !"\"'<>\\`{}|^".contains(c));
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    clean && !authority.contains('@') && host(url).is_some()
}

/// The host of an `https:` link, without its port.
pub fn host(url: &str) -> Option<&str> {
    let authority = url.strip_prefix("https://")?.split(['/', '?', '#']).next()?;
    let (host, port) = authority.split_once(':').unwrap_or((authority, ""));
    let named = !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
    (named && port.bytes().all(|b| b.is_ascii_digit()) && port.len() <= 5).then_some(host)
}

/// The event's link is a Zoom, Meet or Teams one.
pub fn has_meeting_link(event: &Event) -> bool {
    meeting_service(event).is_some()
}

/// `Zoom`, `Meet` or `Teams`: whose meeting the event's link is.
pub fn meeting_service(event: &Event) -> Option<&'static str> {
    event.link.as_deref().and_then(host).and_then(meeting_host)
}

fn meeting_host(host: &str) -> Option<&'static str> {
    let host = host.to_ascii_lowercase();
    let under = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    if under("zoom.us") || under("zoomgov.com") {
        Some("Zoom")
    } else if host == "meet.google.com" {
        Some("Meet")
    } else if under("teams.microsoft.com") || host == "teams.live.com" {
        Some("Teams")
    } else {
        None
    }
}

/// Why a feed link is refused: not `https:`, a user name in it, or a host that is a number or this
/// machine's network (`localhost`, `*.local`). A name that resolves to such an address is not
/// checked: the link is the user's own setting. Never says the link itself: it is a secret.
pub fn feed_refusal(url: &str) -> Option<&'static str> {
    if !joinable(url) {
        return Some("calendar_ics must be an https:// link or a file's path");
    }
    let host = host(url)?.to_ascii_lowercase();
    let last = host.rsplit('.').next().unwrap_or_default();
    let number = last.bytes().all(|b| b.is_ascii_digit()) || last.starts_with("0x");
    let local = ["localhost", "local", "internal", "home", "lan", "arpa"].contains(&last);
    (number || local || !host.contains('.')).then_some("calendar_ics must name a host on the internet, not an address")
}

/// The host and port a feed link names (443 unless it says).
pub fn feed_host(url: &str) -> Option<(&str, u16)> {
    let authority = url.strip_prefix("https://")?.split(['/', '?', '#']).next()?;
    let port = match authority.split_once(':') {
        Some((_, port)) => port.parse().ok()?,
        None => 443,
    };
    Some((host(url)?, port))
}

/// IPv4 networks a feed may not come from, as (network, prefix length): this network, private,
/// carrier NAT, loopback, link-local, IETF protocol, documentation, benchmarking, multicast,
/// reserved and broadcast.
pub const V4_REFUSED: [(u32, u32); 14] = [
    (0x0000_0000, 8),
    (0x0a00_0000, 8),
    (0x6440_0000, 10),
    (0x7f00_0000, 8),
    (0xa9fe_0000, 16),
    (0xac10_0000, 12),
    (0xc000_0000, 24),
    (0xc000_0200, 24),
    (0xc0a8_0000, 16),
    (0xc612_0000, 15),
    (0xc633_6400, 24),
    (0xcb00_7100, 24),
    (0xe000_0000, 4),
    (0xf000_0000, 4),
];

/// IPv6 networks refused outright: unspecified, loopback, local-use NAT64, Teredo (its IPv4 is
/// obscured), documentation, unique local, link-local, site-local and multicast.
pub const V6_REFUSED: [(u128, u32); 9] = [
    (0, 128),
    (1, 128),
    (0x0064_ff9b_0001 << 80, 48),
    (0x2001_0000 << 96, 32),
    (0x2001_0db8 << 96, 32),
    (0xfc00 << 112, 7),
    (0xfe80 << 112, 10),
    (0xfec0 << 112, 10),
    (0xff00 << 112, 8),
];

/// An address a feed may come from: none of `V4_REFUSED` or `V6_REFUSED`, and an IPv4 inside IPv6
/// (mapped, compatible, NAT64 `64:ff9b::/96`, 6to4 `2002::/16`) is judged as that IPv4.
pub fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let ip = u32::from(v4);
            !V4_REFUSED.iter().any(|&(net, len)| ip >> (32 - len) == net >> (32 - len))
        }
        IpAddr::V6(v6) => {
            let ip = u128::from(v6);
            let within = |net: u128, len: u32| len == 0 || ip >> (128 - len) == net >> (128 - len);
            if V6_REFUSED.iter().any(|&(net, len)| within(net, len)) {
                return false;
            }
            let embedded = if within(0xffff << 32, 96) || within(0, 96) || within(0x0064_ff9b << 96, 96) {
                Some(ip as u32)
            } else if within(0x2002 << 112, 16) {
                Some((ip >> 80) as u32)
            } else {
                None
            };
            embedded.is_none_or(|v4| public_address(IpAddr::V4(v4.into())))
        }
    }
}

/// The address to pin a fetch to: the first the host resolved to, if every one is public. One
/// that is not refuses the fetch, rather than picking the public one among them.
pub fn pin_address(found: &[IpAddr]) -> Option<IpAddr> {
    (found.iter().all(|ip| public_address(*ip))).then(|| found.first().copied()).flatten()
}

/// The config `curl --config -` reads on its standard input: the link is not in its arguments,
/// so `ps` does not show it. `https:` only, no redirect (one could lead to this machine's
/// network), no globbing of `[]{}`, a size cap and time limits. The status code goes to stderr
/// (`fetch::Failure::of_curl`). `pin`: the address the host was resolved to and checked (`public_address`),
/// which curl then connects to, so a second lookup cannot answer otherwise. `None` for a link
/// `feed_refusal` refuses.
pub fn curl_config(url: &str, pin: Option<IpAddr>) -> Option<String> {
    if feed_refusal(url).is_some() {
        return None;
    }
    let (host, port) = feed_host(url)?;
    let resolve = match pin {
        Some(IpAddr::V6(ip)) => format!("resolve = \"{host}:{port}:[{ip}]\"\n"),
        Some(ip) => format!("resolve = \"{host}:{port}:{ip}\"\n"),
        None => String::new(),
    };
    // `joinable` leaves no quote, backslash or space in the link to escape.
    Some(format!(
        "silent\nfail\ngloboff\nproto = \"=https\"\nmax-redirs = 0\nmax-filesize = {}\nconnect-timeout = 10\n\
         max-time = {MAX_SECS}\nwrite-out = \"%{{stderr}}%{{http_code}}\"\n{resolve}url = \"{url}\"\n",
        crate::ics::MAX_BYTES
    ))
}

/// The alert of one occurrence: how many were given (at most 2) and when the last was.
#[derive(Clone, Debug, PartialEq)]
struct Alert {
    id: u64,
    count: u8,
    at: u64,
    /// Dismissed, or joined: nothing more is said of it.
    done: bool,
}

/// What the alert says, for the system's notification. Other people's text: drawn, never logged.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub title: String,
    pub started: u64,
}

/// The banner's line of a meeting that started: `Design review · started 3m ago`,
/// `Design review · just started` in its first minute.
pub(crate) fn started_line(title: &str, start: u64, now: u64) -> String {
    match now.saturating_sub(start) / MINUTE {
        0 => format!("{title} · just started"),
        ago => format!("{title} · started {ago}m ago"),
    }
}

impl Notice {
    /// The notification's text: `Started 3m ago: Design review`. Our word first, so a title like
    /// `-e …` is no option of the program that shows it.
    pub fn body(&self, now: u64) -> String {
        match now.saturating_sub(self.started) / MINUTE {
            0 => format!("Started just now: {}", self.title),
            ago => format!("Started {ago}m ago: {}", self.title),
        }
    }
}

/// The event overlaps `start..end` (ms), or starts in it with no length.
fn overlaps(e: &Event, start: u64, end: u64) -> bool {
    e.start < end && (e.end > start || e.start >= start)
}

/// The timeline's state in the model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Calendar {
    /// A feed is set (`calendar_ics`).
    pub on: bool,
    /// `calendar_notify`.
    pub notify: bool,
    /// `calendar_notify_for: "links"`: only an event with a Zoom, Meet or Teams link alerts.
    pub links_only: bool,
    /// The events of the 7 days before today, today and the next 7 (`reply::HELD_DAYS`), by
    /// start; without free ones while `hide_free`.
    pub events: Vec<Event>,
    /// As read, free ones too.
    received: Vec<Event>,
    /// `calendar_hide_free`: entries that leave the time free are left out.
    pub hide_free: bool,
    /// `calendar_colors`, in order.
    pub rules: Vec<ColorRule>,
    /// Local midnights in milliseconds, `BACK` days before today on (`DAYS`).
    pub days: [u64; DAYS],
    /// When the feed was last read whole.
    pub updated: Option<u64>,
    /// The last read failed: `events` are of the one before.
    pub stale: bool,
    /// The feed's fetch as it goes: loading, read or failed.
    pub fetch: fetch::Fetch,
    /// The last fetch that failed: the band says so through the retries that follow.
    last_failure: Option<fetch::Failure>,
    /// The user's Retry asked for the fetch that runs or comes next: the band slides for it.
    retried: bool,
    /// Whether the user is in a call (a microphone is in use), as last seen. `None`: not known.
    pub in_call: Option<bool>,
    /// When the call that is on was first seen (ms).
    call_since: Option<u64>,
    alerts: Vec<Alert>,
    /// Alerts given this run (both of an event count), for a test.
    pub given: u32,
    /// The Calendar section's day, from today: 0 is today, -1 yesterday. Back to today when the
    /// day turns.
    pub day: i32,
    /// The first local midnight the events are whole from (ms): a past day before it was cut by
    /// the cap on events, and is neither shown nor answered for.
    pub held_from: u64,
}

/// What the Calendar's rail cell shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailMark {
    None,
    /// A meeting is imminent or runs: a steady dot.
    Near,
    /// A meeting started and the user is in no call (the banner is up): a filled badge.
    Alert,
}

/// What the band above the status bar shows. Its room is kept from the first frame while a feed
/// is set: nothing moves when the meetings come.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandShows {
    /// No feed: no band.
    Absent,
    /// No good read yet, and none failed last: a bar slides along it.
    Loading,
    /// No good read yet, the last fetch failed: one quiet line and the user's Retry.
    Failed,
    /// A read landed: the workday and its meetings.
    Ready,
}

/// One crossing of the loading bar's highlight (ms), and its width (of the band's).
const SLIDE_MS: u64 = 1_600;
const SLIDE_WIDTH: f32 = 0.2;

impl BandShows {
    pub fn name(self) -> &'static str {
        match self {
            BandShows::Absent => "absent",
            BandShows::Loading => "loading",
            BandShows::Failed => "failed",
            BandShows::Ready => "ready",
        }
    }

    /// Its quiet line, if any: a failure; loading with Reduce motion, in place of the bar.
    pub fn line(self, reduced: bool) -> Option<&'static str> {
        match self {
            BandShows::Loading if reduced => Some("Loading…"),
            BandShows::Failed => Some("Couldn't read the calendar"),
            _ => None,
        }
    }

    /// The bar slides: a frame each tick while it does, none once the read lands or fails.
    pub fn slides(self, reduced: bool) -> bool {
        self == BandShows::Loading && !reduced
    }
}

/// The loading bar's highlight at `ms`, from and to across the band (0 to 1): a short highlight
/// enters at the left and leaves at the right every `SLIDE_MS`, eased in and out.
pub fn slide(ms: u64) -> (f32, f32) {
    let p = (ms % SLIDE_MS) as f32 / SLIDE_MS as f32;
    let start = -SLIDE_WIDTH + (1. + SLIDE_WIDTH) * p * p * (3. - 2. * p);
    (start.max(0.), (start + SLIDE_WIDTH).min(1.))
}

impl Calendar {
    /// What the band shows: absent without a feed; until a read lands, loading, or failed once a
    /// fetch failed: the retries after it stay quiet, but for the user's own.
    pub fn band_shows(&self) -> BandShows {
        match (self.on, self.updated, self.fetch) {
            (false, ..) => BandShows::Absent,
            (true, Some(_), _) => BandShows::Ready,
            (true, None, _) if self.retried => BandShows::Loading,
            (true, None, fetch::Fetch::Failed { .. }) => BandShows::Failed,
            (true, None, _) if self.last_failure.is_some() => BandShows::Failed,
            (true, None, _) => BandShows::Loading,
        }
    }

    /// The band's line's hover: the section's line, or while a retry of its own runs, what failed.
    pub fn band_tip(&self, now: u64, offset: i64) -> String {
        match (self.fetch, self.last_failure) {
            (fetch::Fetch::Loading { .. }, Some(why)) if !self.retried => format!("{} · trying again", why.words()),
            _ => self.fetch_line(now, offset),
        }
    }

    /// A fetch ended: failed or read.
    pub fn fetched(&mut self, at: u64, events: u32, failure: Option<fetch::Failure>) {
        self.fetch = match failure {
            None => fetch::Fetch::Read { at, events },
            Some(why) => fetch::Fetch::Failed { at, why },
        };
        self.last_failure = failure.or(self.last_failure);
        self.retried = false;
    }

    /// The user's Retry: whether to wake the feed thread. Not while a fetch runs (a second would
    /// only wait behind it), though the band slides for that one.
    pub fn retry(&mut self) -> bool {
        self.retried = self.on;
        self.on && !matches!(self.fetch, fetch::Fetch::Loading { .. })
    }

    /// A read of the feed: its events, or `None` when it failed (the last ones stay).
    pub fn read(&mut self, events: Option<Vec<Event>>, days: [u64; DAYS], at: u64) {
        if days[BACK] != self.days[BACK] {
            self.day = 0;
        }
        self.days = days;
        match events {
            Some(mut events) => {
                events.sort_by_key(|e| (e.start, e.end));
                // The same occurrence written twice is one: one id, one alert.
                let mut seen = std::collections::HashSet::new();
                events.retain(|e| seen.insert(e.id));
                events.truncate(MAX_EVENTS);
                self.received = events;
                self.updated = Some(at);
                self.stale = false;
                self.shown();
            }
            None => self.stale = true,
        }
    }

    /// `calendar_hide_free` and `calendar_colors`, as the settings say now.
    pub fn options(&mut self, hide_free: bool, rules: Vec<ColorRule>) {
        self.rules = rules;
        if hide_free != self.hide_free {
            self.hide_free = hide_free;
            self.shown();
        }
    }

    /// `events` again from what was read.
    fn shown(&mut self) {
        let hide = self.hide_free;
        self.events = self.received.iter().filter(|e| !(hide && e.show == Show::Free)).cloned().collect();
        let events = &self.events;
        self.alerts.retain(|a| events.iter().any(|e| e.id == a.id));
    }

    /// An event's paint now.
    pub fn paint(&self, event: &Event, now: u64, reduced: bool) -> Paint {
        let stage = stage(event, now);
        paint(event, stage, glow(stage, now, reduced), &self.rules)
    }

    /// The day `from` today, midnight to midnight (ms): 0 today. Before the first read, nothing.
    pub fn span(&self, from: i32) -> (u64, u64) {
        let at = |n: i32| usize::try_from(BACK as i32 + n).ok().and_then(|n| self.days.get(n)).copied().unwrap_or(0);
        (at(from), at(from + 1))
    }

    /// The events that overlap the day `from` today, by start.
    pub fn on(&self, from: i32) -> impl Iterator<Item = &Event> {
        let (start, end) = self.span(from);
        self.events.iter().filter(move |e| overlaps(e, start, end))
    }

    /// Today's events, by start: those that overlap today.
    pub fn today(&self) -> impl Iterator<Item = &Event> {
        self.on(0)
    }

    pub fn tomorrow(&self) -> impl Iterator<Item = &Event> {
        let (start, end) = self.span(1);
        self.events.iter().filter(move |e| e.start < end && e.start >= start)
    }

    /// The first day back held whole (`held_from`), from today.
    pub fn first_day(&self) -> i32 {
        (1..=BACK as i32).rev().map(|n| -n).find(|n| self.span(*n).0 >= self.held_from).unwrap_or(0)
    }

    /// The Calendar section's day moved a step, within the days held.
    pub fn step(&mut self, step: DayStep) {
        // No day before the first read.
        if self.span(0).1 == 0 {
            return;
        }
        self.day = match step {
            DayStep::Back => (self.day - 1).max(self.first_day()),
            DayStep::Forward => (self.day + 1).min(AHEAD),
            DayStep::Today => 0,
        };
    }

    /// Today's events, and those about to start or running though the day has turned since the
    /// last read: a meeting at 00:02 is near at 23:58, and runs at 00:01 before the next read.
    pub fn near(&self, now: u64) -> impl Iterator<Item = &Event> {
        let (start, end) = self.span(0);
        let end = end.max(now.saturating_add(APPROACHING + 1));
        self.events.iter().filter(move |e| overlaps(e, start, end))
    }

    /// The event the now marker's card names: one that starts within `APPROACHING`, else one that
    /// runs, else the next today. `None` when nothing is left today.
    pub fn next(&self, now: u64) -> Option<(&Event, Stage)> {
        let timed: Vec<(&Event, Stage)> = self
            .near(now)
            .filter(|e| !e.all_day)
            .map(|e| (e, stage(e, now)))
            .filter(|(_, s)| *s != Stage::Past)
            .collect();
        let near = timed.iter().find(|(_, s)| matches!(s, Stage::Approaching | Stage::Imminent));
        let running = timed.iter().find(|(_, s)| *s == Stage::Running);
        near.or(running).or(timed.first()).copied()
    }

    /// The Calendar section's rows: the timed meetings of its day, by start. Its all-day events
    /// are one line above them, no row.
    pub fn agenda(&self) -> Vec<crate::list::ListRow> {
        self.on(self.day).filter(|e| !e.all_day).map(|e| crate::list::ListRow::Meeting(e.id)).collect()
    }

    /// The row list mode starts on: the meeting that runs, else the next, else the day's last.
    /// Another day falls out alike: a past one's last, a day to come's first.
    pub fn first_row(&self, now: u64) -> usize {
        let timed: Vec<&Event> = self.on(self.day).filter(|e| !e.all_day).collect();
        let at = |want: Stage| timed.iter().position(|e| stage(e, now) == want);
        at(Stage::Running)
            .or_else(|| timed.iter().position(|e| stage(e, now) != Stage::Past))
            .unwrap_or(timed.len().saturating_sub(1))
    }

    /// The rail cell's mark: the alert's badge, else a dot while a meeting is imminent or runs.
    pub fn rail(&self, now: u64) -> RailMark {
        let near = |e: &&Event| !e.all_day && matches!(stage(e, now), Stage::Imminent | Stage::Running);
        match (self.on, self.banner(now).is_some(), self.near(now).any(|e| near(&e))) {
            (false, ..) => RailMark::None,
            (true, true, _) => RailMark::Alert,
            (true, false, true) => RailMark::Near,
            _ => RailMark::None,
        }
    }

    /// The now marker's hover card: `14:05`, `Design review · 15:00`, and when the feed was read
    /// (`calendar updated 14:00`): the one place that shows it outside the Calendar section.
    pub fn now_card(&self, now: u64, offset: i64) -> String {
        let next = match self.next(now) {
            Some((e, Stage::Running)) => format!("{} · now", e.title),
            Some((e, _)) => format!("{} · {}", e.title, clock(e.start, offset)),
            None => "No more meetings today".into(),
        };
        format!("{}\n{next}\ncalendar {}", clock(now, offset), self.read_status(offset))
    }

    /// Today's band and what is on it. `offset(t)`: the local zone's at `t`, seconds east of UTC.
    pub fn band(&self, workday: Workday, now: u64, offset: &dyn Fn(u64) -> i64) -> Layout {
        let today = self.span(0);
        layout(&self.events, today, band(&self.events, today, workday, offset), now)
    }

    /// The Calendar section's day on its axis: the band's span of that day, and its lanes.
    pub fn day_view(&self, workday: Workday, now: u64, offset: &dyn Fn(u64) -> i64) -> day::DayView {
        let shown = self.span(self.day);
        day::day_view(&self.events, shown, band(&self.events, shown, workday, offset), now, offset)
    }

    /// When the feed was read: `updated 09:40`, `not updated since 09:40`, `not read yet`.
    pub fn read_status(&self, offset: i64) -> String {
        match (self.updated, self.stale) {
            (Some(at), false) => format!("updated {}", clock(at, offset)),
            (Some(at), true) => format!("not updated since {}", clock(at, offset)),
            (None, _) => "not read yet".to_string(),
        }
    }

    /// The Calendar section draws its day, an empty one too (one `free` band), only once a read
    /// succeeded: before it, the fetch's line says loading or what failed.
    pub fn draws_day(&self) -> bool {
        self.updated.is_some()
    }

    /// The section's muted line: the fetch as it goes (`fetch::Fetch::line`), the last good read's
    /// time after a failure. `offset`: the local zone's, for that time.
    pub fn fetch_line(&self, now: u64, offset: i64) -> String {
        self.fetch.line(now, self.updated.map(|at| clock(at, offset)))
    }

    /// Whether the shell should look for a call now: an alert could come soon or is up, or a call
    /// is on (watched until it ends, so `call_since` is of this call).
    pub fn watching(&self, now: u64) -> bool {
        self.on
            && self.notify
            && (self.in_call == Some(true)
                || self.near(now).any(|e| !e.all_day && e.start <= now.saturating_add(WATCH_BEFORE) && now < e.end))
    }

    /// A call is on that the user was in before `event` came near: not this meeting. Either first
    /// seen `JOINED_EARLY` before it, or before its watch while another meeting ran (one overrunning).
    pub fn in_other_call(&self, event: &Event) -> bool {
        let Some(since) = self.call_since.filter(|_| self.in_call == Some(true)) else { return false };
        let other = |e: &Event| {
            e.id != event.id && !e.all_day && e.start.saturating_sub(WATCH_BEFORE) <= since && since < e.end
        };
        since.saturating_add(JOINED_EARLY) <= event.start
            || (since.saturating_add(WATCH_BEFORE) <= event.start && self.today().any(other))
    }

    /// The alerts due now: (event, whether it is the second).
    fn due_alerts(&self, now: u64) -> Vec<(u64, bool)> {
        if !self.on || !self.notify || self.in_call.is_none() {
            return Vec::new();
        }
        let wanted = |e: &&Event| {
            (!self.links_only || has_meeting_link(e)) && (self.in_call == Some(false) || self.in_other_call(e))
        };
        let running = self.near(now).filter(|e| !e.all_day && stage(e, now) == Stage::Running).filter(wanted);
        // Both alerts only in the first minutes: a meeting left early is no missed one.
        let early = |e: &&Event| now < e.start.saturating_add(ALERT_WINDOW);
        running
            .filter(early)
            .filter_map(|e| match self.alerts.iter().find(|a| a.id == e.id) {
                None => Some((e.id, false)),
                Some(a) if !a.done && a.count == 1 && now >= a.at.saturating_add(REPEAT) => Some((e.id, true)),
                _ => None,
            })
            .collect()
    }

    pub fn due(&self, now: u64) -> bool {
        !self.due_alerts(now).is_empty()
    }

    /// The time passed: the alerts it brings, each once.
    pub fn tick(&mut self, now: u64) -> Vec<Notice> {
        let mut out = Vec::new();
        for (id, again) in self.due_alerts(now) {
            let Some(event) = self.events.iter().find(|e| e.id == id) else { continue };
            out.push(Notice { title: event.title.clone(), started: event.start });
            match self.alerts.iter_mut().find(|a| a.id == id) {
                Some(alert) if again => (alert.count, alert.at) = (2, now),
                _ => self.alerts.push(Alert { id, count: 1, at: now, done: false }),
            }
            self.given += 1;
        }
        out
    }

    /// The banner's event: alerted, not dismissed or joined, still running, and no call on.
    pub fn banner(&self, now: u64) -> Option<&Event> {
        let shown = |e: &&Event| {
            let alerted = self.alerts.iter().any(|a| a.id == e.id && !a.done);
            // In another call, only in the first minutes: a switch between two looks reads as one call.
            let other = self.in_other_call(e) && now < e.start.saturating_add(ALERT_WINDOW);
            alerted && stage(e, now) == Stage::Running && (self.in_call != Some(true) || other)
        };
        self.events.iter().rfind(shown)
    }

    /// The banner's line: `Design review · started 3m ago`.
    pub fn banner_line(&self, now: u64) -> Option<(u64, String, bool)> {
        let event = self.banner(now)?;
        Some((event.id, started_line(&event.title, event.start, now), event.link.is_some()))
    }

    /// Whether a call is on, as seen at `at`. A call that starts after an alert ends it: the user
    /// joined, so leaving later is not missing the meeting. A call on before any alert ends nothing.
    pub fn call_seen(&mut self, in_call: Option<bool>, at: u64) {
        self.in_call = in_call;
        match in_call {
            Some(true) if self.call_since.is_none() => {
                self.call_since = Some(at);
                self.alerts.iter_mut().for_each(|alert| alert.done = true);
            }
            Some(true) => {}
            _ => self.call_since = None,
        }
    }

    pub fn dismiss(&mut self, id: u64) {
        match self.alerts.iter_mut().find(|a| a.id == id) {
            Some(alert) => alert.done = true,
            None if self.events.iter().any(|e| e.id == id) => {
                self.alerts.push(Alert { id, count: 2, at: 0, done: true })
            }
            None => {}
        }
    }

    /// The link to open for the user's Join, if the event has one `joinable` allows. Its alert ends.
    pub fn join(&mut self, id: u64) -> Option<String> {
        let link = self.events.iter().find(|e| e.id == id)?.link.clone().filter(|l| joinable(l))?;
        self.dismiss(id);
        Some(link)
    }
}

#[cfg(test)]
mod tests;
