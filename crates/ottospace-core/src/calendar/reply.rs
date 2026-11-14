//! The `calendar` request's reply: the meetings of the days asked for, busy and free time. Pure:
//! the shell passes the time and the local zone. Only what the feed's own reads left in the model:
//! the request never makes the app fetch.
//!
//! Titles and places are other people's text: cleaned again, links taken out. Never in it: links,
//! descriptions, attendees, the feed's address or the user's own.

use serde::Serialize;

use super::{Calendar, Event, MAX_TITLE, has_meeting_link};
use crate::ics::{self, Table};

const DAY: i64 = 86_400;
/// Days one request spans at most.
pub const MAX_DAYS: i64 = 7;
/// Days the feed's reads hold, from the start of today: today and the next `MAX_DAYS`.
pub const HELD_DAYS: i64 = MAX_DAYS + 1;
/// Meetings in one reply; `more` counts those left out.
pub const MAX_MEETINGS: usize = 200;

/// The reply with a feed set.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Answer {
    pub configured: bool,
    /// When the feed was last read whole (RFC 3339), and whether the last read failed.
    pub updated: Option<String>,
    pub stale: bool,
    pub in_call: Option<bool>,
    /// The span asked for: local midnight to local midnight.
    pub from: String,
    pub to: String,
    pub meetings: Vec<Meeting>,
    pub more: usize,
    /// Timed meetings merged, cut to the span; `free` is the rest of the span, nights included.
    pub busy: Vec<Span>,
    pub free: Vec<Span>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Meeting {
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub title: String,
    pub location: Option<String>,
    /// A Zoom, Meet or Teams link: its address is not given (it may hold a passcode).
    pub has_call_link: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Span {
    pub start: String,
    pub end: String,
}

/// The whole reply: `{"configured":false}` alone with no feed set.
pub fn reply(
    calendar: &Calendar,
    day: Option<&str>,
    days: Option<u32>,
    now: u64,
    local: &Table,
) -> Result<serde_json::Value, &'static str> {
    if !calendar.on {
        return Ok(serde_json::json!({ "configured": false }));
    }
    let span = span(day, days, seconds(now), local)?;
    serde_json::to_value(answer(calendar, span, local)).map_err(|_| "the calendar could not be written")
}

/// Local midnight of today and of `HELD_DAYS` later (UTC seconds): the span the feed's reads hold.
pub fn held(now: i64, local: &Table) -> (i64, i64) {
    let today = today(now, local);
    (local.to_utc(today * DAY), local.to_utc((today + HELD_DAYS) * DAY))
}

fn today(now: i64, local: &Table) -> i64 {
    now.saturating_add(local.at(now)).div_euclid(DAY)
}

/// The span asked for, in UTC seconds: local midnight of `day` (`today`, the default, `tomorrow`
/// or `YYYY-MM-DD`) to that of `days` (1 to 7, default 1) later. Only within the days held.
pub fn span(day: Option<&str>, days: Option<u32>, now: i64, local: &Table) -> Result<(i64, i64), &'static str> {
    let today = today(now, local);
    let first = match day.unwrap_or("today") {
        "today" => today,
        "tomorrow" => today + 1,
        date => date_day(date).ok_or("day is today, tomorrow or a date such as 2026-11-16")?,
    };
    let count = i64::from(days.unwrap_or(1));
    if !(1..=MAX_DAYS).contains(&count) {
        return Err("days is 1 to 7");
    }
    if first < today || first + count > today + HELD_DAYS {
        return Err("the calendar holds today and the next 7 days only");
    }
    Ok((local.to_utc(first * DAY), local.to_utc((first + count) * DAY)))
}

/// `2026-11-16` as days since 1970; `None` for anything else, `2026-02-30` included.
fn date_day(text: &str) -> Option<i64> {
    let number = |at: std::ops::Range<usize>| {
        let part = text.get(at)?;
        part.bytes().all(|b| b.is_ascii_digit()).then(|| part.parse::<i64>().ok()).flatten()
    };
    let dashes = text.len() == 10 && text.get(4..5) == Some("-") && text.get(7..8) == Some("-");
    let (y, m, d) = (number(0..4).filter(|_| dashes)?, number(5..7)?, number(8..10)?);
    let day = ics::days_from_civil(y, m, d);
    (ics::civil(day) == (y, m, d)).then_some(day)
}

/// The meetings of `from..to` (UTC seconds), busy and free time, and how fresh it is.
pub fn answer(calendar: &Calendar, (from, to): (i64, i64), local: &Table) -> Answer {
    let (from_ms, to_ms) = (ms(from), ms(to));
    // As `Calendar::today`: what overlaps the span, and what starts in it with no length.
    let within: Vec<&Event> =
        calendar.events.iter().filter(|e| e.start < to_ms && (e.end > from_ms || e.start >= from_ms)).collect();
    let at = |ms: u64| rfc3339(ms, local);
    let meetings = within
        .iter()
        .take(MAX_MEETINGS)
        .map(|e| Meeting {
            start: at(e.start),
            end: at(e.end),
            all_day: e.all_day,
            title: Some(text(&e.title)).filter(|t| !t.is_empty()).unwrap_or_else(|| "(no title)".into()),
            location: Some(text(&e.location)).filter(|l| !l.is_empty()),
            has_call_link: has_meeting_link(e),
            status: e.tentative.then_some("tentative"),
        })
        .collect();
    let busy = busy(&within, from_ms, to_ms);
    let spans = |list: &[(u64, u64)]| list.iter().map(|&(s, e)| Span { start: at(s), end: at(e) }).collect();
    Answer {
        configured: true,
        updated: calendar.updated.map(at),
        stale: calendar.stale,
        in_call: calendar.in_call,
        from: at(from_ms),
        to: at(to_ms),
        meetings,
        more: within.len().saturating_sub(MAX_MEETINGS),
        free: spans(&free(&busy, from_ms, to_ms)),
        busy: spans(&busy),
    }
}

/// Timed events cut to `from..to` and merged where they overlap or meet. All-day events and
/// those with no length are no busy time.
fn busy(events: &[&Event], from: u64, to: u64) -> Vec<(u64, u64)> {
    let mut spans: Vec<(u64, u64)> = events
        .iter()
        .filter(|e| !e.all_day)
        .map(|e| (e.start.max(from), e.end.min(to)))
        .filter(|(start, end)| start < end)
        .collect();
    spans.sort_unstable();
    let mut out: Vec<(u64, u64)> = Vec::new();
    for (start, end) in spans {
        match out.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => out.push((start, end)),
        }
    }
    out
}

/// The gaps of `from..to` between `busy` (merged, by start).
fn free(busy: &[(u64, u64)], from: u64, to: u64) -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    let mut at = from;
    for &(start, end) in busy {
        if start > at {
            out.push((at, start));
        }
        at = at.max(end);
    }
    if at < to {
        out.push((at, to));
    }
    out
}

/// A title or place as a client gets it: one line of characters that show, capped, and no word
/// that looks like a link (`https://…`, `zoom.us/j/1?pwd=…`, `www.…`): a join link holds a passcode.
fn text(raw: &str) -> String {
    let link = |w: &&str| {
        let host = w.split('/').next().unwrap_or_default();
        w.contains("://") || w.to_ascii_lowercase().starts_with("www.") || (w.contains('/') && host.contains('.'))
    };
    let words: Vec<&str> = raw.split_whitespace().filter(|w| !link(w)).collect();
    crate::capture::plain(&words.join(" "), MAX_TITLE)
}

/// `2026-11-16T09:00:00-08:00`: a time in the local zone, with its offset then.
pub fn rfc3339(ms: u64, local: &Table) -> String {
    let utc = seconds(ms);
    let offset = local.at(utc).clamp(-86_399, 86_399);
    let t = utc + offset;
    let (y, m, d) = ics::civil(t.div_euclid(DAY));
    let (s, o) = (t.rem_euclid(DAY), offset.abs());
    let sign = if offset < 0 { '-' } else { '+' };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}{sign}{:02}:{:02}",
        s / 3600,
        s / 60 % 60,
        s % 60,
        o / 3600,
        o / 60 % 60
    )
}

fn seconds(ms: u64) -> i64 {
    (ms.min(1 << 50) / 1_000) as i64
}

fn ms(seconds: i64) -> u64 {
    u64::try_from(seconds).unwrap_or(0).saturating_mul(1_000)
}

#[cfg(test)]
mod tests;
