//! The Calendar section's day, as Outlook draws one: a time axis with hour lines, each meeting a
//! block as tall as it lasts, side by side where they overlap (`lanes`, the band's own), and a
//! long stretch of free time pressed into a short band (`2 h free`). Points from the view's top;
//! titles stay in the shell.

use super::{Band, Event, MINUTE, Stage, lanes, local, of_day};

/// Points a minute: a 09:00 to 17:00 day with no gap is 480 points, within a laptop's sidebar.
pub const SCALE: f32 = 1.0;
/// A block is at least this tall: its title and start on one line (`Standup 9:00`).
pub const MIN_BLOCK: f32 = 20.;
/// Free time longer than this is pressed into a band of `GAP_BAND` points that says how long.
pub const GAP_MAX: u64 = 60 * MINUTE;
pub const GAP_BAND: f32 = 24.;

/// A stretch of the day and its height. `gap`: free time longer than `GAP_MAX`, pressed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub from: u64,
    pub to: u64,
    pub top: f32,
    pub height: f32,
    pub gap: bool,
}

/// A meeting's block: its times (cut to the day), its column of those it overlaps, where it is
/// drawn, and the meetings merged into it past `MAX_LANES` at once.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub id: u64,
    pub start: u64,
    pub end: u64,
    pub column: u8,
    pub columns: u8,
    pub top: f32,
    pub height: f32,
    pub stage: Stage,
    pub more: Vec<u64>,
}

/// An hour's line: its time, its local hour (`9`) and where it is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hour {
    pub at: u64,
    pub hour: u8,
    pub y: f32,
}

/// The day's geometry: from and to a whole hour, its stretches top to bottom, the blocks by start,
/// the hour lines, and its height.
#[derive(Clone, Debug, PartialEq)]
pub struct DayView {
    pub from: u64,
    pub to: u64,
    pub segments: Vec<Segment>,
    pub blocks: Vec<Block>,
    pub hours: Vec<Hour>,
    pub height: f32,
}

impl DayView {
    /// Where `t` is drawn: along its stretch in proportion, a pressed gap too; pinned to the ends.
    pub fn y(&self, t: u64) -> f32 {
        let Some(s) = self.segments.iter().find(|s| t < s.to).or(self.segments.last()) else { return 0. };
        let t = t.clamp(s.from, s.to);
        s.top + s.height * (t - s.from) as f32 / (s.to - s.from).max(1) as f32
    }

    /// The now line's place, if now is on this day's axis.
    pub fn now(&self, now: u64) -> Option<f32> {
        (self.from <= now && now <= self.to && !self.segments.is_empty()).then(|| self.y(now))
    }

    /// The free stretches pressed into a band.
    pub fn gaps(&self) -> impl Iterator<Item = &Segment> {
        self.segments.iter().filter(|s| s.gap)
    }

    /// The scroll (points down) to open at: now a third of the way down `viewport`, or the top
    /// when now is not on the day. `above`: the room over the view inside the scroll.
    pub fn opening_scroll(&self, now: u64, viewport: f32, above: f32) -> f32 {
        let most = (self.height + above - viewport).max(0.);
        self.now(now).map_or(0., |y| (above + y - viewport / 3.).clamp(0., most))
    }
}

/// The scroll that shows `top..bottom` in `viewport`, moved as little as it can from `scroll`.
pub fn reveal(scroll: f32, viewport: f32, (top, bottom): (f32, f32)) -> f32 {
    if top < scroll {
        top
    } else if bottom > scroll + viewport {
        (bottom - viewport).min(top)
    } else {
        scroll
    }
}

/// `2 h free`, `1 h 30 m free`: a pressed gap's words, to the minute.
pub fn free_label(ms: u64) -> String {
    match (ms / MINUTE / 60, ms / MINUTE % 60) {
        (0, m) => format!("{m} m free"),
        (h, 0) => format!("{h} h free"),
        (h, m) => format!("{h} h {m} m free"),
    }
}

/// `9:00`, `14:05`: a block's start, `clock` without its hour's leading zero.
pub fn short_clock(ms: u64, offset: i64) -> String {
    let at = super::clock(ms, offset);
    at.strip_prefix('0').map_or(at.clone(), String::from)
}

/// The day from midnight `day.0` to `day.1` on its axis. It spans the band (`band`, the same
/// workday and calls) widened to every timed event of the day and to now, out to whole hours. Free time over
/// `GAP_MAX` is a `GAP_BAND`; a block shorter than `MIN_BLOCK` is stretched to it at its end,
/// moving what follows down. Columns are the band's lanes, of every event, not calls only.
pub fn day_view(events: &[Event], day: (u64, u64), band: Band, now: u64, offset: &dyn Fn(u64) -> i64) -> DayView {
    let mut shown: Vec<&Event> = of_day(events, day).collect();
    shown.sort_by_key(|e| (e.start, e.end));
    // Each cut to the day; one with no length takes a millisecond, so it has a place.
    let span = |e: &Event| (e.start.max(day.0), e.end.max(e.start + 1).min(day.1).max(e.start.max(day.0)));
    // Now too, on its day: the line is there out of hours and on an empty day.
    let today = (day.0..day.1).contains(&now).then_some(now);
    let lo = shown.iter().map(|e| span(e).0).chain(today).fold(band.from, u64::min);
    let hi = shown.iter().map(|e| span(e).1).chain(today).fold(band.to, u64::max);
    let marks: Vec<(u8, u64)> =
        (0..=24u8).map(|h| (h, local(day.0, u32::from(h) * 60, offset))).filter(|(_, t)| *t <= day.1).collect();
    let from = marks.iter().rev().map(|m| m.1).find(|t| *t <= lo).unwrap_or(day.0);
    let to = marks.iter().map(|m| m.1).find(|t| *t >= hi).unwrap_or(day.1).max(from);
    let mut cuts: Vec<u64> = vec![from, to];
    for e in &shown {
        let (s, e) = span(e);
        cuts.extend([s.clamp(from, to), e.clamp(from, to)]);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let minutes = |ms: u64| ms as f32 / MINUTE as f32;
    let mut segments: Vec<Segment> = cuts
        .windows(2)
        .map(|w| {
            let held = shown.iter().any(|e| span(e).0 < w[1] && span(e).1 > w[0]);
            let gap = !held && w[1] - w[0] > GAP_MAX;
            let height = if gap { GAP_BAND } else { minutes(w[1] - w[0]) * SCALE };
            Segment { from: w[0], to: w[1], top: 0., height, gap }
        })
        .collect();
    let stack = |segments: &mut Vec<Segment>| {
        let mut top = 0.;
        for s in segments.iter_mut() {
            s.top = top;
            top += s.height;
        }
    };
    stack(&mut segments);
    let mut view = DayView { from, to, segments, blocks: Vec::new(), hours: Vec::new(), height: 0. };
    // Too short to read: the stretch it ends with grows, and what is below moves down.
    for e in &shown {
        let (s, e) = span(e);
        let short = MIN_BLOCK - (view.y(e) - view.y(s));
        if let Some(last) = view.segments.iter_mut().find(|seg| seg.to == e).filter(|_| short > 0.) {
            last.height += short;
            stack(&mut view.segments);
        }
    }
    view.height = view.segments.last().map_or(0., |s| s.top + s.height);
    let axis = Band { from, to, workday: band.workday };
    for bar in lanes(&shown, axis, now) {
        let of = |id: u64| shown.iter().find(|e| e.id == id).map(|e| span(e));
        let times: Vec<(u64, u64)> = std::iter::once(bar.id).chain(bar.more.iter().copied()).filter_map(of).collect();
        let start = times.iter().map(|t| t.0).min().unwrap_or(from);
        let end = times.iter().map(|t| t.1).max().unwrap_or(start);
        let top = view.y(start);
        view.blocks.push(Block {
            id: bar.id,
            start,
            end,
            column: bar.lane,
            columns: bar.lanes,
            top,
            height: view.y(end) - top,
            stage: bar.stage,
            more: bar.more,
        });
    }
    view.blocks.sort_by_key(|b| (b.start, b.column));
    // Hour lines on the axis, none inside a pressed gap.
    let inside_gap = |t: u64| view.gaps().any(|g| g.from < t && t < g.to);
    let mut hours: Vec<Hour> = marks
        .iter()
        .filter(|(_, t)| from <= *t && *t <= to && !inside_gap(*t))
        .map(|&(hour, at)| Hour { at, hour: hour % 24, y: view.y(at) })
        .collect();
    hours.dedup_by_key(|h| h.at);
    view.hours = hours;
    view
}

#[cfg(test)]
mod tests;
