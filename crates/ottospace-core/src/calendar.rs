//! The meeting timeline: today's events from the user's calendar feed, the band of the workday
//! above the status bar, the next-meeting chip, the pulse as one comes near, and the alert when one has
//! started and the user is in no call. Pure: the shell passes the time and whether a call is on.
//!
//! Titles, places and descriptions are other people's text: kept cleaned and short, drawn, and
//! never in the control socket's state, the trace or a brief. Titles and places reach a client
//! only through the `calendar` request (`reply`); descriptions and links never.

use std::net::IpAddr;

pub mod reply;

use crate::ics::Occurrence;

/// Why a client's Join or Dismiss did nothing.
pub const USERS_ALONE: &str = "only the user joins or dismisses a meeting";

const MINUTE: u64 = 60_000;
/// From here before its start a meeting pulses slowly; from `IMMINENT` faster.
pub const APPROACHING: u64 = 10 * MINUTE;
pub const IMMINENT: u64 = 2 * MINUTE;
/// From here before its start the status bar names it.
pub const SOON: u64 = 5 * MINUTE;
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
    /// `STATUS:TENTATIVE`.
    pub tentative: bool,
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

/// Which part of the band a block takes: all of it, or a half when events overlap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Full,
    Top,
    Bottom,
}

/// A block on the band: from and to as fractions of its width.
#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    pub id: u64,
    pub from: f32,
    pub to: f32,
    pub lane: Lane,
    pub stage: Stage,
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
/// earliest start and latest end, so no meeting is hidden; a meeting over midnight is cut there.
pub fn band(events: &[Event], day: (u64, u64), workday: Workday, offset: &dyn Fn(u64) -> i64) -> Band {
    let hours = (local(day.0, workday.start, offset), local(day.0, workday.end, offset).min(day.1));
    let mut band = Band { from: hours.0, to: hours.1, workday: hours };
    for e in of_day(events, day) {
        band.from = band.from.min(e.start.max(day.0));
        band.to = band.to.max(e.end.min(day.1));
    }
    band
}

/// The band's blocks: the day's timed events at their clock places, past ones too.
pub fn bars(events: &[Event], day: (u64, u64), band: &Band, now: u64) -> Vec<Bar> {
    let mut shown: Vec<&Event> = of_day(events, day).collect();
    shown.sort_by_key(|e| (e.start, e.end));
    let mut out = Vec::new();
    // Events that overlap go in two half lanes; one with none takes both.
    let mut cluster: Vec<&Event> = Vec::new();
    let mut cluster_end = 0;
    let mut flush = |cluster: &mut Vec<&Event>| {
        let alone = cluster.len() == 1;
        let (mut top_end, mut bottom_end) = (0, 0);
        for event in cluster.drain(..) {
            let lane = if alone {
                Lane::Full
            } else if event.start >= top_end || (event.start < bottom_end && top_end <= bottom_end) {
                // The free lane, else the one that frees first.
                top_end = top_end.max(event.end);
                Lane::Top
            } else {
                bottom_end = bottom_end.max(event.end);
                Lane::Bottom
            };
            let (from, to) = (band.at(event.start), band.at(event.end));
            out.push(Bar { id: event.id, from, to, lane, stage: stage(event, now) });
        }
    };
    for event in shown {
        if !cluster.is_empty() && event.start >= cluster_end {
            flush(&mut cluster);
        }
        cluster_end = if cluster.is_empty() { event.end } else { cluster_end.max(event.end) };
        cluster.push(event);
    }
    flush(&mut cluster);
    out
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

/// What a block's hover card says: `Design review`, `09:00–09:30 · 30m · Room 4`, `Zoom link`.
pub fn describe(event: &Event, offset: i64) -> String {
    let when = if event.all_day {
        "all day".to_string()
    } else {
        let length = span(event.end.saturating_sub(event.start));
        format!("{}–{} · {length}", clock(event.start, offset), clock(event.end, offset))
    };
    let place = Some(event.location.as_str()).filter(|l| !l.is_empty()).map(|l| format!(" · {l}")).unwrap_or_default();
    let link = meeting_service(event).map(|name| format!("\n{name} link")).unwrap_or_default();
    format!("{}\n{when}{place}{link}", event.title)
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
                tentative: o.tentative,
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
/// (`feed_answer`). `pin`: the address the host was resolved to and checked (`public_address`),
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
         max-time = 30\nwrite-out = \"%{{stderr}}%{{http_code}}\"\n{resolve}url = \"{url}\"\n",
        crate::ics::MAX_BYTES
    ))
}

/// What curl's status code (`write-out`) says of a read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Read,
    /// The feed moved: its final address belongs in the settings.
    Moved,
    Failed,
}

pub fn feed_answer(code: &str) -> Answer {
    match code.trim().parse::<u16>() {
        Ok(200..=299) => Answer::Read,
        Ok(300..=399) => Answer::Moved,
        _ => Answer::Failed,
    }
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
    /// The events of today and the next 7 days (`reply::HELD_DAYS`), by start.
    pub events: Vec<Event>,
    /// Local midnight of today, tomorrow and the day after, in milliseconds.
    pub days: [u64; 3],
    /// When the feed was last read whole.
    pub updated: Option<u64>,
    /// The last read failed: `events` are of the one before.
    pub stale: bool,
    /// Whether the user is in a call (a microphone is in use), as last seen. `None`: not known.
    pub in_call: Option<bool>,
    /// When the call that is on was first seen (ms).
    call_since: Option<u64>,
    alerts: Vec<Alert>,
    /// Alerts given this run (both of an event count), for a test.
    pub given: u32,
    /// The Calendar section's "Tomorrow" group is unfolded.
    pub tomorrow_open: bool,
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

impl Calendar {
    /// A read of the feed: its events, or `None` when it failed (the last ones stay).
    pub fn read(&mut self, events: Option<Vec<Event>>, days: [u64; 3], at: u64) {
        self.days = days;
        match events {
            Some(mut events) => {
                events.sort_by_key(|e| (e.start, e.end));
                // The same occurrence written twice is one: one id, one alert.
                let mut seen = std::collections::HashSet::new();
                events.retain(|e| seen.insert(e.id));
                events.truncate(MAX_EVENTS);
                self.alerts.retain(|a| events.iter().any(|e| e.id == a.id));
                self.events = events;
                self.updated = Some(at);
                self.stale = false;
            }
            None => self.stale = true,
        }
    }

    /// Today's events, by start: those that overlap today.
    pub fn today(&self) -> impl Iterator<Item = &Event> {
        let [start, end, _] = self.days;
        self.events.iter().filter(move |e| overlaps(e, start, end))
    }

    pub fn tomorrow(&self) -> impl Iterator<Item = &Event> {
        let [_, start, end] = self.days;
        self.events.iter().filter(move |e| e.start < end && e.start >= start)
    }

    /// The event the chip names: one that starts within `APPROACHING`, else one that runs, else
    /// the next today. `None` when nothing is left today.
    pub fn next(&self, now: u64) -> Option<(&Event, Stage)> {
        let timed: Vec<(&Event, Stage)> = self
            .today()
            .filter(|e| !e.all_day)
            .map(|e| (e, stage(e, now)))
            .filter(|(_, s)| *s != Stage::Past)
            .collect();
        let near = timed.iter().find(|(_, s)| matches!(s, Stage::Approaching | Stage::Imminent));
        let running = timed.iter().find(|(_, s)| *s == Stage::Running);
        near.or(running).or(timed.first()).copied()
    }

    /// The Calendar section's rows: today's all-day events, then today's others by start, then the
    /// "Tomorrow" group, and tomorrow's events (all-day first) while it is unfolded.
    pub fn agenda(&self) -> Vec<crate::list::ListRow> {
        use crate::list::ListRow;
        let ordered = |events: Vec<&Event>| {
            let (all_day, timed): (Vec<&Event>, Vec<&Event>) = events.into_iter().partition(|e| e.all_day);
            all_day.into_iter().chain(timed).map(|e| ListRow::Meeting(e.id)).collect::<Vec<_>>()
        };
        let mut rows = ordered(self.today().collect());
        let tomorrow: Vec<&Event> = self.tomorrow().collect();
        if !tomorrow.is_empty() {
            rows.push(ListRow::MeetingsTomorrow);
            if self.tomorrow_open {
                rows.extend(ordered(tomorrow));
            }
        }
        rows
    }

    /// The rail cell's mark: the alert's badge, else a dot while a meeting is imminent or runs.
    pub fn rail(&self, now: u64) -> RailMark {
        let near = |e: &&Event| !e.all_day && matches!(stage(e, now), Stage::Imminent | Stage::Running);
        match (self.on, self.banner(now).is_some(), self.today().any(|e| near(&e))) {
            (false, ..) => RailMark::None,
            (true, true, _) => RailMark::Alert,
            (true, false, true) => RailMark::Near,
            _ => RailMark::None,
        }
    }

    /// The chip's text: `Design review · in 12m`, `Design review · now`.
    pub fn chip(&self, now: u64) -> Option<(String, Stage)> {
        let (event, stage) = self.next(now)?;
        let when = if stage == Stage::Running { "now".into() } else { format!("in {}", span(event.start - now)) };
        Some((format!("{} · {when}", event.title), stage))
    }

    /// The now marker's hover card: `14:05`, then `Next: Design review at 15:00 (in 55m)`.
    pub fn now_card(&self, now: u64, offset: i64) -> String {
        let next = match self.next(now) {
            Some((e, Stage::Running)) => format!("Now: {}", e.title),
            Some((e, _)) => format!("Next: {} at {} (in {})", e.title, clock(e.start, offset), span(e.start - now)),
            None => "No more meetings today".into(),
        };
        format!("{}\n{next}", clock(now, offset))
    }

    /// Today's band and its blocks. `offset(t)`: the local zone's at `t`, seconds east of UTC.
    pub fn band(&self, workday: Workday, now: u64, offset: &dyn Fn(u64) -> i64) -> (Band, Vec<Bar>) {
        let today = (self.days[0], self.days[1]);
        let band = band(&self.events, today, workday, offset);
        (band, bars(&self.events, today, &band, now))
    }

    /// When the feed was read: `updated 09:40`, `not updated since 09:40`, `not read yet`.
    pub fn read_status(&self, offset: i64) -> String {
        match (self.updated, self.stale) {
            (Some(at), false) => format!("updated {}", clock(at, offset)),
            (Some(at), true) => format!("not updated since {}", clock(at, offset)),
            (None, _) => "not read yet".to_string(),
        }
    }

    /// The status bar's line from `SOON` before a start: `Design review in 4m`.
    pub fn soon(&self, now: u64) -> Option<String> {
        let event = self.today().filter(|e| !e.all_day).find(|e| e.start > now && e.start - now <= SOON)?;
        Some(format!("{} in {}", event.title, span(event.start - now)))
    }

    /// Whether the shell should look for a call now: an alert could come soon or is up, or a call
    /// is on (watched until it ends, so `call_since` is of this call).
    pub fn watching(&self, now: u64) -> bool {
        self.on
            && self.notify
            && (self.in_call == Some(true)
                || self.today().any(|e| !e.all_day && e.start <= now.saturating_add(WATCH_BEFORE) && now < e.end))
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
        let running = self.today().filter(|e| !e.all_day && stage(e, now) == Stage::Running).filter(wanted);
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
        let ago = now.saturating_sub(event.start) / MINUTE;
        Some((event.id, format!("{} · started {ago}m ago", event.title), event.link.is_some()))
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
