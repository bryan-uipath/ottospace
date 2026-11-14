//! The meeting timeline: today's events from the user's calendar feed, the hairline across the top
//! of the window, the next-meeting chip, the pulse as one comes near, and the alert when one has
//! started and the user is in no call. Pure: the shell passes the time and whether a call is on.
//!
//! Titles, places and descriptions are other people's text: kept cleaned and short, drawn only,
//! never in the control socket's state, the trace or a brief.

use std::net::IpAddr;

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
const MAX_TITLE: usize = 120;
const MAX_LINK: usize = 2_000;
/// Events kept of two days.
const MAX_EVENTS: usize = 200;

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

/// Which band of the hairline a block takes: all of it, or a half when events overlap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Full,
    Top,
    Bottom,
}

/// A block on the hairline: from and to as fractions of its width, 0 at its left edge (now).
#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    pub id: u64,
    pub from: f32,
    pub to: f32,
    pub lane: Lane,
    pub stage: Stage,
}

/// The hairline's blocks: the next `window` milliseconds from `now`, which is its left edge.
/// What has ended is not drawn; a running event starts at the left edge.
pub fn bars(events: &[Event], now: u64, window: u64) -> Vec<Bar> {
    let window = window.max(MINUTE);
    let at = |t: u64| (t.saturating_sub(now) as f64 / window as f64).min(1.) as f32;
    let mut shown: Vec<&Event> =
        events.iter().filter(|e| !e.all_day && e.end > now && e.start < now.saturating_add(window)).collect();
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
            out.push(Bar { id: event.id, from: at(event.start), to: at(event.end), lane, stage: stage(event, now) });
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

/// The whole clock hours in the hairline's window: where each is (a fraction of its width) and
/// the hour, 0 to 23. `offset`: the local zone's, in seconds east of UTC.
pub fn ticks(now: u64, window: u64, offset: i64) -> Vec<(f32, u8)> {
    const HOUR: i64 = 3_600_000;
    let local = now.min(1 << 50) as i64 + offset.clamp(-86_400, 86_400) * 1_000;
    let mut hour = local.div_euclid(HOUR) + 1;
    let mut out = Vec::new();
    while hour * HOUR - local <= window as i64 && out.len() < 48 {
        out.push((((hour * HOUR - local) as f64 / window.max(1) as f64) as f32, hour.rem_euclid(24) as u8));
        hour += 1;
    }
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

/// `Design review`, `09:00–09:30 · Room 4`: what a block's tooltip says.
pub fn describe(event: &Event, offset: i64) -> String {
    let when = if event.all_day {
        "all day".to_string()
    } else {
        format!("{}–{}", clock(event.start, offset), clock(event.end, offset))
    };
    let place = Some(event.location.as_str()).filter(|l| !l.is_empty()).map(|l| format!(" · {l}")).unwrap_or_default();
    format!("{}\n{when}{place}", event.title)
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
    let meeting = found.iter().find(|url| host(url).is_some_and(meeting_host));
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
    event.link.as_deref().and_then(host).is_some_and(meeting_host)
}

fn meeting_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let under = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    under("zoom.us")
        || under("zoomgov.com")
        || host == "meet.google.com"
        || under("teams.microsoft.com")
        || host == "teams.live.com"
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

/// An address a feed may come from: not this machine, a private or link-local network, carrier
/// NAT, multicast, a documentation or unspecified one (IPv4, IPv6, and IPv4 inside IPv6).
pub fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            let special = v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_broadcast()
                || v4.is_documentation()
                || a == 0
                || (a == 100 && (64..128).contains(&b))
                || (a == 198 && (b == 18 || b == 19));
            !special
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped().or_else(|| v6.to_ipv4().filter(|_| v6.segments()[..6] == [0; 6])) {
                return public_address(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            let local = (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80;
            !(v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || local)
        }
    }
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

/// The timeline's state in the model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Calendar {
    /// A feed is set (`calendar_ics`).
    pub on: bool,
    /// `calendar_notify`.
    pub notify: bool,
    /// `calendar_notify_for: "links"`: only an event with a Zoom, Meet or Teams link alerts.
    pub links_only: bool,
    /// Today's and tomorrow's events, by start.
    pub events: Vec<Event>,
    /// Local midnight of today, tomorrow and the day after, in milliseconds.
    pub days: [u64; 3],
    /// When the feed was last read whole.
    pub updated: Option<u64>,
    /// The last read failed: `events` are of the one before.
    pub stale: bool,
    /// Whether the user is in a call (a microphone is in use), as last seen. `None`: not known.
    pub in_call: Option<bool>,
    alerts: Vec<Alert>,
    /// Alerts given this run (both of an event count), for a test.
    pub given: u32,
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
        self.events.iter().filter(move |e| e.start < end && (e.end > start || e.start >= start))
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

    /// The chip's text: `Design review · in 12m`, `Design review · now`.
    pub fn chip(&self, now: u64) -> Option<(String, Stage)> {
        let (event, stage) = self.next(now)?;
        let when = if stage == Stage::Running { "now".into() } else { format!("in {}", span(event.start - now)) };
        Some((format!("{} · {when}", event.title), stage))
    }

    /// The status bar's line from `SOON` before a start: `Design review in 4m`.
    pub fn soon(&self, now: u64) -> Option<String> {
        let event = self.today().filter(|e| !e.all_day).find(|e| e.start > now && e.start - now <= SOON)?;
        Some(format!("{} in {}", event.title, span(event.start - now)))
    }

    /// Whether the shell should look for a call now: an alert could come soon or is up.
    pub fn watching(&self, now: u64) -> bool {
        self.on
            && self.notify
            && self.today().any(|e| !e.all_day && e.start <= now.saturating_add(WATCH_BEFORE) && now < e.end)
    }

    /// The alerts due now: (event, whether it is the second).
    fn due_alerts(&self, now: u64) -> Vec<(u64, bool)> {
        if !self.on || !self.notify || self.in_call != Some(false) {
            return Vec::new();
        }
        let wanted = |e: &&Event| !self.links_only || has_meeting_link(e);
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
            alerted && stage(e, now) == Stage::Running && self.in_call != Some(true)
        };
        self.events.iter().rfind(shown)
    }

    /// The banner's line: `Design review · started 3m ago`.
    pub fn banner_line(&self, now: u64) -> Option<(u64, String, bool)> {
        let event = self.banner(now)?;
        let ago = now.saturating_sub(event.start) / MINUTE;
        Some((event.id, format!("{} · started {ago}m ago", event.title), event.link.is_some()))
    }

    /// Whether a call is on, as seen. A call that starts after an alert ends it: the user joined,
    /// so leaving later is not missing the meeting. A call on before any alert ends nothing.
    pub fn call_seen(&mut self, in_call: Option<bool>) {
        self.in_call = in_call;
        if in_call == Some(true) {
            self.alerts.iter_mut().for_each(|alert| alert.done = true);
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
