//! A small iCalendar (RFC 5545) reader: a feed's events, and when each happens in a span of time.
//! Pure: the shell reads the feed and passes in the offsets of the time zones it names.
//!
//! Read: VEVENT with DTSTART, DTEND or DURATION (UTC, floating, `TZID=`, `VALUE=DATE`), SUMMARY,
//! LOCATION, DESCRIPTION, URL, UID, STATUS, RRULE (DAILY, WEEKLY, MONTHLY, YEARLY with INTERVAL,
//! UNTIL, COUNT, BYDAY, BYMONTHDAY, BYMONTH, WKST), EXDATE, RECURRENCE-ID; VTIMEZONE with
//! STANDARD and DAYLIGHT. A rule with another part (BYSETPOS, BYWEEKNO, BYYEARDAY, BYHOUR...)
//! gives its first occurrence only. RDATE, VTODO, VJOURNAL and attendees are not read.

/// The most of a feed that is read: the rest is left out.
pub const MAX_BYTES: usize = 5 * 1024 * 1024;
/// Events and time zones read from one feed.
const MAX_COMPONENTS: usize = 20_000;
/// Characters kept of a property's value.
const MAX_VALUE: usize = 4_000;
/// Dates of one EXDATE list, and of all of an event's.
const MAX_EXDATES: usize = 1_000;
/// Occurrences one expansion returns.
pub const MAX_OCCURRENCES: usize = 500;
/// The longest event: a longer one is cut to this.
const MAX_DURATION: i64 = 14 * DAY;
/// Days a rule with COUNT is followed from its start, and all rules of a feed together.
const MAX_SCAN: i64 = 7_400;
const SCAN_BUDGET: i64 = 2_000_000;
const DAY: i64 = 86_400;

/// A time as the feed writes it. Seconds since 1970; `Local` and `Date` count as if the local
/// clock were UTC.
#[derive(Clone, Debug, PartialEq)]
pub enum When {
    Utc(i64),
    /// Floating (no zone) or in the named zone.
    Local {
        at: i64,
        zone: Option<String>,
    },
    /// A whole day: days since 1970.
    Date(i64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Clone, Debug, PartialEq)]
struct Rule {
    freq: Freq,
    interval: i64,
    until: Option<When>,
    count: Option<i64>,
    /// (ordinal, weekday): 0 is every such weekday; 1 the first, -1 the last. Monday is 0.
    by_day: Vec<(i64, i64)>,
    by_month_day: Vec<i64>,
    by_month: Vec<i64>,
    week_start: i64,
    /// A part this reader does not follow: only the first occurrence is given.
    unsupported: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Event {
    pub uid: String,
    start: Option<When>,
    end: Option<When>,
    duration: Option<i64>,
    pub summary: String,
    pub location: String,
    pub description: String,
    pub url: String,
    cancelled: bool,
    rule: Option<Rule>,
    exdates: Vec<When>,
    recurrence_id: Option<When>,
}

/// One STANDARD or DAYLIGHT part of a VTIMEZONE.
#[derive(Clone, Debug, Default, PartialEq)]
struct ZonePart {
    start: Option<i64>,
    from: Option<i64>,
    to: Option<i64>,
    rule: Option<Rule>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Zone {
    id: String,
    parts: Vec<ZonePart>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Feed {
    pub events: Vec<Event>,
    zones: Vec<Zone>,
}

/// A zone's offsets: from each UTC second on (sorted), seconds east of UTC. The first also
/// applies before it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table(pub Vec<(i64, i64)>);

impl Table {
    pub fn fixed(offset: i64) -> Self {
        Self(vec![(i64::MIN, offset)])
    }

    /// The offset at a UTC second.
    pub fn at(&self, utc: i64) -> i64 {
        let after = self.0.partition_point(|(from, _)| *from <= utc);
        self.0.get(after.saturating_sub(1)).map_or(0, |(_, offset)| *offset)
    }

    /// The UTC second of a local one: the earlier of two in an hour that repeats; in an hour
    /// that is skipped, as the clock before the change would have it.
    pub fn to_utc(&self, local: i64) -> i64 {
        let mut fits = self.0.iter().map(|(_, o)| local.saturating_sub(*o)).filter(|utc| self.at(*utc) == local - utc);
        let first = fits.next();
        match first {
            Some(utc) => fits.fold(utc, i64::min),
            None => local.saturating_sub(self.at(local.saturating_sub(self.at(local)) - 3_600)),
        }
    }
}

/// The offsets the shell knows: the user's own zone (for floating times and whole days) and those
/// of named zones the feed does not define (`named_zones`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Zones {
    pub local: Table,
    pub named: Vec<(String, Table)>,
}

/// An event as it happens: UTC seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Occurrence {
    pub uid: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub summary: String,
    pub location: String,
    pub description: String,
    pub url: String,
}

/// Reads a feed. Anything it cannot read is left out; it never fails.
pub fn parse(text: &str) -> Feed {
    let text = cut(text, MAX_BYTES);
    let mut feed = Feed::default();
    let mut path: Vec<String> = Vec::new();
    let mut event: Option<Event> = None;
    let mut zone: Option<Zone> = None;
    let mut part: Option<ZonePart> = None;
    for line in unfold(text) {
        let Some((name, params, value)) = content(&line) else { continue };
        match name.as_str() {
            "BEGIN" => {
                let kind = value.trim().to_ascii_uppercase();
                let parent = path.last().map(String::as_str);
                let room = feed.events.len() + feed.zones.len() < MAX_COMPONENTS;
                match (kind.as_str(), parent) {
                    ("VEVENT", Some("VCALENDAR")) if room => event = Some(Event::default()),
                    ("VTIMEZONE", Some("VCALENDAR")) if room => zone = Some(Zone::default()),
                    ("STANDARD" | "DAYLIGHT", Some("VTIMEZONE")) if zone.is_some() => part = Some(ZonePart::default()),
                    _ => {}
                }
                // A file that only opens components must not grow this without end.
                if path.len() < 16 {
                    path.push(kind);
                }
            }
            "END" => {
                let kind = value.trim().to_ascii_uppercase();
                if path.last() != Some(&kind) {
                    continue;
                }
                path.pop();
                match kind.as_str() {
                    "VEVENT" => feed.events.extend(event.take().filter(|e| e.start.is_some())),
                    "VTIMEZONE" => feed.zones.extend(zone.take().filter(|z| !z.id.is_empty())),
                    "STANDARD" | "DAYLIGHT" => {
                        if let (Some(zone), Some(part)) = (zone.as_mut(), part.take()) {
                            zone.parts.push(part);
                        }
                    }
                    _ => {}
                }
            }
            _ => {
                let here = path.last().map(String::as_str);
                match here {
                    Some("VEVENT") => {
                        if let Some(event) = event.as_mut() {
                            event_property(event, &name, &params, value);
                        }
                    }
                    Some("VTIMEZONE") if name == "TZID" => {
                        if let Some(zone) = zone.as_mut() {
                            zone.id = cut(value.trim(), 200).to_string();
                        }
                    }
                    Some("STANDARD" | "DAYLIGHT") => {
                        if let Some(part) = part.as_mut() {
                            zone_property(part, &name, value);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    feed
}

/// The named zones the feed's events use and do not define: the shell looks them up.
pub fn named_zones(feed: &Feed) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let defined = |name: &str| feed.zones.iter().any(|z| z.id == name);
    let times = feed.events.iter().flat_map(|e| [&e.start, &e.end, &e.recurrence_id].into_iter().flatten());
    for when in times {
        if let When::Local { zone: Some(name), .. } = when
            && !defined(name)
            && !names.contains(name)
            && names.len() < 64
        {
            names.push(name.clone());
        }
    }
    names
}

/// The occurrences that overlap `from..to` (UTC seconds), by start, at most `MAX_OCCURRENCES`.
pub fn expand(feed: &Feed, from: i64, to: i64, zones: &Zones) -> Vec<Occurrence> {
    let years = year_of(from.saturating_sub(DAY))..=year_of(to.saturating_add(DAY));
    let tables: Vec<(&str, Table)> = feed.zones.iter().map(|z| (z.id.as_str(), z.table(years.clone()))).collect();
    let table = |zone: Option<&str>| -> &Table {
        let Some(name) = zone else { return &zones.local };
        let defined = tables.iter().find(|(id, _)| *id == name).map(|(_, t)| t);
        defined.or_else(|| zones.named.iter().find(|(id, _)| id == name).map(|(_, t)| t)).unwrap_or(&zones.local)
    };
    let utc = |when: &When| -> i64 {
        match when {
            When::Utc(at) => *at,
            When::Local { at, zone } => table(zone.as_deref()).to_utc(*at),
            When::Date(day) => zones.local.to_utc(day.saturating_mul(DAY)),
        }
    };
    // Occurrences moved or changed by an event of their own: (uid, original start).
    let overrides: Vec<(&str, i64)> =
        feed.events.iter().filter_map(|e| Some((e.uid.as_str(), utc(e.recurrence_id.as_ref()?)))).collect();
    let mut budget = SCAN_BUDGET;
    let mut out = Vec::new();
    for event in &feed.events {
        let Some(start) = event.start.as_ref() else { continue };
        if event.cancelled {
            continue;
        }
        let all_day = matches!(start, When::Date(_));
        let first_utc = utc(start);
        let duration = match (&event.end, event.duration) {
            (Some(When::Date(end)), _) if all_day => match start {
                When::Date(day) => (end - day).saturating_mul(DAY),
                _ => DAY,
            },
            (Some(end), _) => utc(end).saturating_sub(first_utc),
            (None, Some(duration)) => duration,
            (None, None) if all_day => DAY,
            (None, None) => 0,
        }
        .clamp(0, MAX_DURATION);
        let mut push = |begin: i64, end: i64| {
            if begin < to && (end > from || (end == begin && begin >= from)) {
                out.push(Occurrence {
                    uid: event.uid.clone(),
                    start: begin,
                    end,
                    all_day,
                    summary: event.summary.clone(),
                    location: event.location.clone(),
                    description: event.description.clone(),
                    url: event.url.clone(),
                });
            }
        };
        let rule = event.rule.as_ref().filter(|r| !r.unsupported && event.recurrence_id.is_none());
        let Some(rule) = rule else {
            let end = if all_day { day_end(start, duration, &zones.local) } else { first_utc + duration };
            push(first_utc, end);
            continue;
        };
        // The rule counts days in the zone of its start: UTC for a `Z` time.
        let (zone_table, local_first) = match start {
            When::Utc(at) => (Table::fixed(0), *at),
            When::Local { at, zone } => (table(zone.as_deref()).clone(), *at),
            When::Date(day) => (zones.local.clone(), day.saturating_mul(DAY)),
        };
        let first_day = local_first.div_euclid(DAY);
        let time_of_day = local_first.rem_euclid(DAY);
        let last_day = match rule.count {
            Some(count) => {
                let mut seen = 0;
                let mut day = first_day;
                let stop = (to.div_euclid(DAY) + 2).min(first_day + MAX_SCAN);
                while day <= stop && seen < count && budget > 0 {
                    seen += i64::from(occurs(rule, first_day, day));
                    day += 1;
                    budget -= 1;
                }
                if seen < count && day <= stop {
                    continue; // out of budget: left out rather than wrong
                }
                day - 1
            }
            None => i64::MAX,
        };
        let until = rule.until.as_ref().map(|until| match until {
            When::Date(day) => zone_table.to_utc((day + 1).saturating_mul(DAY)) - 1,
            When::Local { at, zone: None } => zone_table.to_utc(*at),
            other => utc(other),
        });
        let excluded = |begin: i64, day: i64| {
            event.exdates.iter().any(|ex| match ex {
                When::Date(ex) => *ex == day,
                other => utc(other) == begin,
            })
        };
        let span_days = duration / DAY + 1;
        let low = (from.saturating_sub(DAY)).div_euclid(DAY) - span_days;
        let high = to.saturating_add(DAY).div_euclid(DAY);
        for day in low.max(first_day)..=high.min(last_day) {
            if !occurs(rule, first_day, day) {
                continue;
            }
            let begin = zone_table.to_utc(day * DAY + time_of_day);
            if until.is_some_and(|until| begin > until)
                || excluded(begin, day)
                || overrides.contains(&(event.uid.as_str(), begin))
            {
                continue;
            }
            let end = if all_day { day_end(&When::Date(day), duration, &zones.local) } else { begin + duration };
            push(begin, end);
        }
    }
    out.sort_by(|a, b| (a.start, a.end, &a.summary).cmp(&(b.start, b.end, &b.summary)));
    out.truncate(MAX_OCCURRENCES);
    out
}

/// The end of a whole-day event that starts at `start` and lasts `duration`: a local midnight.
fn day_end(start: &When, duration: i64, local: &Table) -> i64 {
    let When::Date(day) = start else { return 0 };
    local.to_utc(day.saturating_mul(DAY) + duration.max(DAY))
}

/// Local midnight of the day `now` is in, of the next and of the one after: UTC seconds.
pub fn days(now: i64, local: &Table) -> [i64; 3] {
    let day = now.saturating_add(local.at(now)).div_euclid(DAY);
    [0, 1, 2].map(|n| local.to_utc((day + n) * DAY))
}

impl Zone {
    /// The zone's offsets over these years.
    fn table(&self, years: std::ops::RangeInclusive<i64>) -> Table {
        let mut changes: Vec<(i64, i64, i64)> = Vec::new(); // (utc, from, to)
        for part in &self.parts {
            let (Some(start), Some(from), Some(to)) = (part.start, part.from, part.to) else { continue };
            match part.rule.as_ref().filter(|r| !r.unsupported) {
                None => changes.push((start - from, from, to)),
                Some(rule) => {
                    let (first_day, time) = (start.div_euclid(DAY), start.rem_euclid(DAY));
                    for year in years.clone() {
                        let months =
                            if rule.by_month.is_empty() { vec![civil(first_day).1] } else { rule.by_month.clone() };
                        for month in months {
                            for d in 1..=month_len(year, month) {
                                let day = days_from_civil(year, month, d);
                                let local = day * DAY + time;
                                let until = rule.until.as_ref().map(|u| match u {
                                    When::Utc(at) => *at,
                                    When::Local { at, .. } => at - from,
                                    When::Date(day) => day * DAY,
                                });
                                if occurs(rule, first_day, day) && until.is_none_or(|u| local - from <= u) {
                                    changes.push((local - from, from, to));
                                }
                            }
                        }
                    }
                }
            }
        }
        changes.sort_unstable();
        let Some(&(_, first, _)) = changes.first() else { return Table::fixed(0) };
        let mut table = vec![(i64::MIN, first)];
        table.extend(changes.iter().map(|(at, _, to)| (*at, *to)));
        Table(table)
    }
}

/// Whether a rule that started on `first` has an occurrence on `day` (days since 1970). COUNT
/// and UNTIL are the caller's.
fn occurs(rule: &Rule, first: i64, day: i64) -> bool {
    if day < first {
        return false;
    }
    let ((fy, fm, fd), (y, m, d)) = (civil(first), civil(day));
    let wd = weekday(day);
    if !rule.by_month.is_empty() && !rule.by_month.contains(&m) {
        return false;
    }
    let on_weekday = || rule.by_day.iter().any(|(_, w)| *w == wd);
    match rule.freq {
        Freq::Daily => {
            (day - first) % rule.interval == 0
                && (rule.by_day.is_empty() || on_weekday())
                && (rule.by_month_day.is_empty() || month_day(&rule.by_month_day, y, m, d))
        }
        Freq::Weekly => {
            let week = |day: i64| day - (weekday(day) + 7 - rule.week_start) % 7;
            let weeks = (week(day) - week(first)) / 7;
            weeks % rule.interval == 0 && if rule.by_day.is_empty() { wd == weekday(first) } else { on_weekday() }
        }
        Freq::Monthly => ((y - fy) * 12 + m - fm) % rule.interval == 0 && in_month(rule, y, m, d, wd, fd),
        Freq::Yearly => {
            let month = !rule.by_month.is_empty() || m == fm;
            (y - fy) % rule.interval == 0 && month && in_month(rule, y, m, d, wd, fd)
        }
    }
}

/// Whether day `d` of the month is one the rule's BYMONTHDAY or BYDAY names, or `default`.
fn in_month(rule: &Rule, y: i64, m: i64, d: i64, weekday: i64, default: i64) -> bool {
    let len = month_len(y, m);
    if !rule.by_month_day.is_empty() {
        return month_day(&rule.by_month_day, y, m, d)
            && (rule.by_day.is_empty() || rule.by_day.iter().any(|(_, w)| *w == weekday));
    }
    if !rule.by_day.is_empty() {
        return rule.by_day.iter().any(|&(n, w)| {
            w == weekday && (n == 0 || (n > 0 && (d - 1) / 7 + 1 == n) || (n < 0 && (len - d) / 7 + 1 == -n))
        });
    }
    d == default
}

fn month_day(days: &[i64], y: i64, m: i64, d: i64) -> bool {
    let len = month_len(y, m);
    days.iter().any(|&n| (n > 0 && n == d) || (n < 0 && len + 1 + n == d))
}

/// Monday is 0. 1970-01-01 was a Thursday.
fn weekday(day: i64) -> i64 {
    (day + 3).rem_euclid(7)
}

/// Days since 1970 of a date (proleptic Gregorian), and back.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn year_of(seconds: i64) -> i64 {
    civil(seconds.div_euclid(DAY)).0
}

fn month_len(y: i64, m: i64) -> i64 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn event_property(event: &mut Event, name: &str, params: &[(String, String)], value: &str) {
    let zone = || param(params, "TZID").map(|z| cut(z.trim_start_matches('/'), 200).to_string());
    let date = param(params, "VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE"));
    match name {
        "UID" => event.uid = cut(value, 400).to_string(),
        "SUMMARY" => event.summary = unescape(value),
        "LOCATION" => event.location = unescape(value),
        "DESCRIPTION" => event.description = unescape(value),
        "URL" => event.url = cut(value.trim(), 2_000).to_string(),
        "STATUS" => event.cancelled = value.trim().eq_ignore_ascii_case("CANCELLED"),
        "DTSTART" => event.start = when(value, zone(), date),
        "DTEND" => event.end = when(value, zone(), date),
        "DURATION" => event.duration = duration(value),
        "RRULE" => event.rule = rule(value),
        "RECURRENCE-ID" => event.recurrence_id = when(value, zone(), date),
        "EXDATE" => {
            for one in value.split(',').take(MAX_EXDATES) {
                if event.exdates.len() < MAX_EXDATES
                    && let Some(at) = when(one, zone(), date)
                {
                    event.exdates.push(at);
                }
            }
        }
        _ => {}
    }
}

fn zone_property(part: &mut ZonePart, name: &str, value: &str) {
    match name {
        "DTSTART" => {
            part.start = match when(value, None, false) {
                Some(When::Local { at, .. } | When::Utc(at)) => Some(at),
                _ => None,
            }
        }
        "TZOFFSETFROM" => part.from = offset(value),
        "TZOFFSETTO" => part.to = offset(value),
        "RRULE" => part.rule = rule(value),
        _ => {}
    }
}

/// `+0100`, `-0530`, `+013045`: seconds east of UTC.
fn offset(value: &str) -> Option<i64> {
    let value = value.trim();
    let (sign, digits) = match value.as_bytes().first()? {
        b'+' => (1, &value[1..]),
        b'-' => (-1, &value[1..]),
        _ => return None,
    };
    if !(digits.len() == 4 || digits.len() == 6) || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n = |range: std::ops::Range<usize>| digits.get(range).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
    let (h, m, s) = (n(0..2), n(2..4), n(4..6));
    (h < 24 && m < 60 && s < 60).then_some(sign * (h * 3600 + m * 60 + s))
}

/// `20261113`, `20261113T090000`, `20261113T090000Z`.
fn when(value: &str, zone: Option<String>, date: bool) -> Option<When> {
    let value = value.trim();
    let b = value.as_bytes();
    let digits = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = value.get(range)?;
        part.bytes().all(|b| b.is_ascii_digit()).then(|| part.parse().ok())?
    };
    let (y, m, d) = (digits(0..4)?, digits(4..6)?, digits(6..8)?);
    if !(1..=12).contains(&m) || !(1..=month_len(y, m)).contains(&d) {
        return None;
    }
    let day = days_from_civil(y, m, d);
    if date || b.len() == 8 {
        return (b.len() == 8).then_some(When::Date(day));
    }
    if b.get(8) != Some(&b'T') {
        return None;
    }
    let (h, min, s) = (digits(9..11)?, digits(11..13)?, digits(13..15)?);
    if h > 23 || min > 59 || s > 60 {
        return None;
    }
    let at = day * DAY + h * 3600 + min * 60 + s.min(59);
    match &value[15..] {
        "Z" | "z" => Some(When::Utc(at)),
        "" => Some(When::Local { at, zone }),
        _ => None,
    }
}

/// `PT1H30M`, `P1D`, `P2W`: seconds.
fn duration(value: &str) -> Option<i64> {
    let value = value.trim();
    let (sign, rest) = match value.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, value.strip_prefix('+').unwrap_or(value)),
    };
    let rest = rest.strip_prefix('P')?;
    let (mut total, mut number, mut time) = (0i64, None::<i64>, false);
    for c in rest.chars() {
        match c {
            '0'..='9' => number = Some(number.unwrap_or(0).checked_mul(10)?.checked_add(i64::from(c as u8 - b'0'))?),
            'T' => time = true,
            unit => {
                let n = number.take()?;
                let scale = match (unit, time) {
                    ('W', false) => 7 * DAY,
                    ('D', false) => DAY,
                    ('H', true) => 3600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                };
                total = total.checked_add(n.checked_mul(scale)?)?;
            }
        }
    }
    (number.is_none()).then_some(sign * total.min(MAX_DURATION))
}

fn rule(value: &str) -> Option<Rule> {
    let mut rule = Rule {
        freq: Freq::Daily,
        interval: 1,
        until: None,
        count: None,
        by_day: Vec::new(),
        by_month_day: Vec::new(),
        by_month: Vec::new(),
        week_start: 0,
        unsupported: false,
    };
    let mut freq = None;
    for part in value.trim().split(';').take(32) {
        let (key, value) = part.split_once('=')?;
        let list = || value.split(',').take(64);
        match key.to_ascii_uppercase().as_str() {
            "FREQ" => {
                freq = Some(match value.to_ascii_uppercase().as_str() {
                    "DAILY" => Freq::Daily,
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    "YEARLY" => Freq::Yearly,
                    _ => {
                        rule.unsupported = true;
                        Freq::Daily
                    }
                })
            }
            "INTERVAL" => rule.interval = value.parse::<i64>().ok()?.clamp(1, 1_000),
            "COUNT" => rule.count = Some(value.parse::<i64>().ok()?.clamp(0, 100_000)),
            "UNTIL" => rule.until = Some(when(value, None, false)?),
            "BYDAY" => rule.by_day = list().map(day_of).collect::<Option<_>>()?,
            "BYMONTHDAY" => {
                rule.by_month_day = list()
                    .map(|n| n.parse::<i64>().ok().filter(|n| (-31..=31).contains(n) && *n != 0))
                    .collect::<Option<_>>()?
            }
            "BYMONTH" => {
                rule.by_month =
                    list().map(|n| n.parse::<i64>().ok().filter(|n| (1..=12).contains(n))).collect::<Option<_>>()?
            }
            "WKST" => rule.week_start = day_of(value).filter(|(n, _)| *n == 0)?.1,
            _ => rule.unsupported = true,
        }
    }
    rule.freq = freq?;
    // An ordinal BYDAY of a year with no month counts weeks of the year: not followed.
    let ordinal = rule.by_day.iter().any(|(n, _)| *n != 0);
    if (rule.freq == Freq::Yearly && ordinal && rule.by_month.is_empty())
        || (ordinal && matches!(rule.freq, Freq::Daily | Freq::Weekly))
    {
        rule.unsupported = true;
    }
    Some(rule)
}

/// `MO`, `2TU`, `-1FR`.
fn day_of(text: &str) -> Option<(i64, i64)> {
    let text = text.trim();
    let split = text.len().checked_sub(2)?;
    let (n, name) = (text.get(..split)?, text.get(split..)?);
    let n = if n.is_empty() { 0 } else { n.parse::<i64>().ok().filter(|n| (-53..=53).contains(n) && *n != 0)? };
    let day = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"].iter().position(|d| d.eq_ignore_ascii_case(name))?;
    Some((n, day as i64))
}

/// Lines with their continuations joined (a line that starts with a space or a tab goes on the
/// one before). Each at most `MAX_VALUE` * 4 bytes.
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        match (raw.strip_prefix([' ', '\t']), lines.last_mut()) {
            (Some(rest), Some(last)) => {
                if last.len() + rest.len() <= MAX_VALUE * 4 {
                    last.push_str(rest);
                }
            }
            _ => lines.push(cut(raw, MAX_VALUE * 4).to_string()),
        }
    }
    lines
}

/// `NAME;PARAM=a;PARAM="b:c":value`: the name in capitals, the parameters and the value.
/// A content line: its name, its parameters and its value.
type Line<'a> = (String, Vec<(String, String)>, &'a str);

fn content(line: &str) -> Option<Line<'_>> {
    let mut quoted = false;
    let colon = line.char_indices().find(|(_, c)| {
        if *c == '"' {
            quoted = !quoted;
        }
        *c == ':' && !quoted
    })?;
    let (head, value) = (&line[..colon.0], &line[colon.0 + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_ascii_uppercase();
    if name.is_empty() || name.len() > 64 {
        return None;
    }
    let params = parts
        .take(32)
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.trim().to_ascii_uppercase(), v.trim_matches('"').to_string()))
        .collect();
    Some((name, params, value))
}

fn param<'a>(params: &'a [(String, String)], name: &str) -> Option<&'a str> {
    params.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

/// TEXT's escapes undone: `\,` `\;` `\\` `\n`. At most `MAX_VALUE` characters.
fn unescape(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars().take(MAX_VALUE);
    while let Some(c) = chars.next() {
        match (c, c == '\\') {
            (_, true) => match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            },
            (c, false) => out.push(c),
        }
    }
    out
}

/// At most `max` bytes of `text`, cut at a character boundary.
fn cut(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests;
