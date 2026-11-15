//! The meeting timeline's shell: reads the calendar feed off the main thread, draws the band
//! and the banner, looks whether a call is on, and alerts. The decisions are the core's
//! (`ottospace_core::calendar`).
//!
//! The feed is ottospace's only network access: `/usr/bin/curl`, `https:` only, its link on curl's
//! standard input (not in its arguments), never in a log, the state or an error.

use std::io::{Read as _, Write as _};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use gpui::{AnyElement, AnyView, Context, Div, Window, div, prelude::*, px, relative, rgb};
use ottospace_core::calendar::{self, Event, Stage, Workday};
use ottospace_core::ics::{self, Table, Zones};
use ottospace_core::list::ListRow;
use ottospace_core::settings::expand_home;
use ottospace_core::{Action, Effect, Origin, theme};
use ottospace_mac::meeting;

use crate::{Mark, Tooltip, Workspace, config, home, list_button, mark, now_ms, progress, words};

/// The band's height above the status bar, its 1-point top line included; hover takes all of it.
const BAND: f32 = 12.;
/// Below the top line: what lanes and hover slices divide.
const INNER: f32 = BAND - 1.;
/// Calls and blocks are drawn between these margins; overlapping calls split the room into lanes
/// `LANE_GAP` apart.
const LANES_TOP: f32 = 2.5;
const LANES_BOTTOM: f32 = 1.;
const LANE_GAP: f32 = 0.5;
/// Blocks are this much fainter than a call of the same strength: a layer under the calls.
const BLOCK_STRENGTH: f32 = 0.35;
/// A block is drawn at least this wide, and hovered over this much more on each side.
const MIN_BLOCK: f32 = 3.;
const HOVER_PAD: f32 = 3.;
/// The band's ends keep this far from the window's sides.
const INSET: f32 = 6.;
/// How often the thread wakes to see whether the feed or the day is due.
const WAKE: Duration = Duration::from_secs(20);

/// Where the events come from (`calendar_ics`).
enum Source {
    Link(String),
    File(PathBuf),
}

/// One read of the feed, for `Action::CalendarRead`.
pub struct Read {
    events: Option<Vec<Event>>,
    days: [u64; calendar::DAYS],
    at: u64,
    held_from: u64,
    /// Why the read failed, when the user can do something about it. Never the link.
    problem: Option<&'static str>,
}

/// What a failed read says in the status bar, never naming the link.
const MOVED: &str = "calendar feed redirects: put its final https address in calendar_ics";
const PRIVATE: &str = "calendar feed: its host is not on the internet";

/// The timeline's shell state. Inert when no feed is set.
#[derive(Default)]
pub struct Timeline {
    reads: Option<Receiver<Read>>,
    /// `calendar_workday`: the band's span.
    workday: Workday,
    reduce_motion: bool,
    /// When the reduced-motion setting, the band and the call were last looked at (ms).
    motion_read: u64,
    slid: u64,
    call_looked: u64,
    /// Links a test run's Join would have opened.
    joins: u32,
}

impl Timeline {
    /// Starts the reader if the settings name a feed.
    pub fn start() -> Self {
        let settings = &config::get().settings;
        let workday = settings.workday();
        let Some(feed) = settings.calendar_ics.as_deref() else { return Self { workday, ..Self::default() } };
        let source = if feed.starts_with("https://") {
            Source::Link(feed.to_string())
        } else {
            Source::File(PathBuf::from(expand_home(feed, &config::user_home())))
        };
        let every = Duration::from_secs(60 * u64::from(settings.calendar_refresh_minutes.unwrap_or(5).max(1)));
        // Only to match while parsing: never sent, stored in the model, or logged.
        let me = settings.calendar_email.clone();
        let (tx, rx) = mpsc::channel();
        // The feed is parsed once a read, and its events worked out again only then or when the
        // day turns: nothing that grows with the feed runs on the timer.
        std::thread::spawn(move || {
            let (mut feed, mut fetched, mut days) = (ics::Feed::default(), None::<Instant>, [0u64; calendar::DAYS]);
            // When the feed was last read whole, and whether the last fetch failed: a new day's
            // re-evaluation is no read, so it changes neither.
            let (mut read_at, mut failed) = (None::<u64>, false);
            loop {
                let now = now_ms();
                let due = fetched.is_none_or(|at| at.elapsed() >= every);
                let mut problem = None;
                if due {
                    match fetch(&source) {
                        Ok(text) => (feed, read_at, failed) = (ics::parse_for(&text, me.as_deref()), Some(now), false),
                        Err(why) => (failed, problem) = (true, why),
                    }
                    fetched = Some(Instant::now());
                }
                if due || now >= days[calendar::BACK + 1] {
                    let (events, today, held_from) = evaluate(&feed, now);
                    days = today;
                    let events = read_at.filter(|_| !failed).map(|_| events);
                    let read = Read { events, days, at: read_at.unwrap_or(now), held_from, problem };
                    if tx.send(read).is_err() {
                        return;
                    }
                }
                std::thread::sleep(WAKE);
            }
        });
        Self { reads: Some(rx), workday, ..Self::default() }
    }
}

/// The feed's text, or what the status bar may say of the failure (`None`: nothing more than
/// "not updated"). The link is never in a message.
fn fetch(source: &Source) -> Result<String, Option<&'static str>> {
    match source {
        Source::File(path) => {
            let file = std::fs::File::open(path).map_err(|_| None)?;
            let mut bytes = Vec::new();
            file.take(ics::MAX_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|_| None)?;
            let fits = bytes.len() <= ics::MAX_BYTES;
            fits.then(|| String::from_utf8_lossy(&bytes).into_owned()).ok_or(None)
        }
        Source::Link(link) => curl(link),
    }
}

/// `curl -q --config -`, the link and every limit in the config on its standard input
/// (`calendar::curl_config`). `-q` first: no `.curlrc` of the user's applies. The host is looked
/// up here, refused unless every address it has is public, and curl is pinned to the first (`resolve`). A proxy in
/// the user's environment (`https_proxy`) still applies; the proxy then looks the host up.
/// A test run may name another program in `OTTOSPACE_CURL`, which stands for curl, reads the same
/// config and is given no pinned address (a test looks nothing up).
fn curl(link: &str) -> Result<String, Option<&'static str>> {
    let test = std::env::var_os("OTTOSPACE_E2E").and(std::env::var_os("OTTOSPACE_CURL"));
    let pin = match test {
        Some(_) => None,
        None => {
            use std::net::ToSocketAddrs;
            let (host, port) = calendar::feed_host(link).ok_or(None)?;
            let found: Vec<_> = (host, port).to_socket_addrs().map_err(|_| None)?.map(|a| a.ip()).collect();
            Some(calendar::pin_address(&found).ok_or(Some(PRIVATE))?)
        }
    };
    let config = calendar::curl_config(link, pin).ok_or(None)?;
    let program = test.map_or_else(|| PathBuf::from("/usr/bin/curl"), PathBuf::from);
    let mut child = Command::new(program)
        .args(["-q", "--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Only the status code: `silent`, and no `show-error`, whose messages may quote the link.
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| None)?;
    child.stdin.take().ok_or(None)?.write_all(config.as_bytes()).map_err(|_| None)?;
    let mut bytes = Vec::new();
    let read = child.stdout.take().ok_or(None)?.take(ics::MAX_BYTES as u64 + 1).read_to_end(&mut bytes);
    if read.is_err() || bytes.len() > ics::MAX_BYTES {
        let _ = child.kill();
    }
    let mut code = String::new();
    let _ = child.stderr.take().ok_or(None)?.take(16).read_to_string(&mut code);
    let ok = child.wait().map_err(|_| None)?.success();
    match calendar::feed_answer(&code) {
        calendar::Answer::Moved => Err(Some(MOVED)),
        calendar::Answer::Read if ok && read.is_ok() && bytes.len() <= ics::MAX_BYTES => {
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        }
        _ => Err(None),
    }
}

/// The events of the days held (a week back to a week on) of a feed's text, in the user's zone;
/// and their local midnights (ms). Today on first: the 500 kept are today's and the week ahead's
/// before the week back's, within one expansion's work.
fn evaluate(feed: &ics::Feed, now: u64) -> (Vec<Event>, [u64; calendar::DAYS], u64) {
    let now = (now / 1_000) as i64;
    let local = local_zone(now);
    let (from, to) = zone_span(now);
    let days: [i64; calendar::DAYS] = ics::days(now, &local, calendar::BACK as i64);
    let named = ics::named_zones(feed)
        .into_iter()
        .filter_map(|name| Some((name.clone(), Table(meeting::zone_table(&name, from, to)?))))
        .collect();
    let (_, today, end) = calendar::reply::held(now, &local);
    let zones = Zones { local, named };
    let (mut held, work) = ics::expand_counted(feed, today, end, &zones);
    let mut budget = ics::SCAN_BUDGET - work;
    // The week back a whole day at a time, yesterday first, while the work and the 500 allow:
    // a day that does not fit is left out, and so is every day before it.
    let mut held_from = today;
    for back in (0..calendar::BACK).rev() {
        let (from, to) = (days[back], days[back + 1]);
        let (day, used) = ics::expand_within(feed, (from, to), &zones, budget);
        budget -= used;
        let day: Vec<_> = day.into_iter().filter(|o| o.start >= from).collect();
        if budget <= 0 || held.len() + day.len() > ics::MAX_OCCURRENCES {
            break;
        }
        held.splice(0..0, day);
        held_from = from;
    }
    let ms = |at: i64| (at.max(0) as u64) * 1_000;
    (calendar::events(&held), days.map(ms), ms(held_from))
}

/// Zone tables cover 10 days before `now` (seconds) to 11 after: the days held and more.
fn zone_span(now: i64) -> (i64, i64) {
    (now - 10 * 86_400, now + 11 * 86_400)
}

/// The user's zone's offsets over `zone_span`.
fn local_zone(now: i64) -> Table {
    let (from, to) = zone_span(now);
    Table(meeting::local_table(from, to))
}

impl Workspace {
    /// The `calendar` request: what the model holds, never a fetch. Titles and places, cleaned;
    /// no link, description, attendee, feed address or the user's own address.
    pub(crate) fn calendar_reply(&self, day: Option<&str>, days: Option<u32>) -> String {
        let now = now_ms();
        let (calendar, local) = (&self.model.calendar, local_zone((now / 1_000) as i64));
        match calendar::reply::reply(calendar, day, days, (now, self.timeline.workday), &local) {
            Ok(serde_json::Value::Object(mut reply)) => {
                reply.insert("ok".into(), true.into());
                serde_json::Value::Object(reply).to_string()
            }
            Ok(_) => crate::control::error("the calendar could not be written"),
            Err(why) => crate::control::error(why),
        }
    }

    /// From the main loop: the feed's reads, the call while a meeting is near, and a redraw while the
    /// band pulses (10 a second), else every 20 seconds, as now moves along it.
    pub(crate) fn tick_timeline(&mut self, window: &Window, cx: &mut Context<Self>) {
        let reads: Vec<Read> = self.timeline.reads.iter().flat_map(|rx| rx.try_iter()).collect();
        for Read { events, days, at, held_from, problem } in reads {
            if problem.is_some() {
                self.error = problem.map(String::from);
            }
            self.dispatch(Action::CalendarRead { events, days, at, held_from }, Origin::System, window, cx);
        }
        if !self.model.calendar.on {
            return;
        }
        let now = now_ms();
        // Not in a test run: a test says whether a call is on (`calendar_call`).
        let watching = self.model.calendar.watching(now);
        let test = std::env::var_os("OTTOSPACE_E2E").is_some();
        if !test && watching && now.saturating_sub(self.timeline.call_looked) >= 2_000 {
            self.timeline.call_looked = now;
            let in_call = match meeting::call() {
                meeting::Call::InCall => Some(true),
                meeting::Call::NotInCall => Some(false),
                meeting::Call::Unknown => None,
            };
            if self.model.calendar.in_call != in_call {
                self.dispatch(Action::CallSeen { in_call, at: now }, Origin::System, window, cx);
            }
        }
        let calendar = &self.model.calendar;
        if now.saturating_sub(self.timeline.motion_read) >= 10_000 {
            self.timeline.motion_read = now;
            self.timeline.reduce_motion = meeting::reduce_motion();
        }
        let pulses =
            calendar.near(now).any(|e| matches!(calendar::stage(e, now), Stage::Approaching | Stage::Imminent));
        let pulse = pulses && !self.timeline.reduce_motion;
        if pulse || now.saturating_sub(self.timeline.slid) >= 20_000 {
            self.timeline.slid = now;
            cx.notify();
        }
    }

    /// Carries out the timeline's effects. A test run posts no notification and opens no link.
    pub(crate) fn meeting_effect(&mut self, effect: &Effect) {
        let test = std::env::var_os("OTTOSPACE_E2E").is_some();
        match effect {
            Effect::MeetingAlert { title, started } if !test => {
                let notice = calendar::Notice { title: title.clone(), started: *started };
                notify(&notice.body(now_ms()));
                meeting::beep();
            }
            // `joinable` again, at the door: the one allow-function.
            Effect::OpenLink { url } if calendar::joinable(url) => {
                if test {
                    self.timeline.joins += 1;
                } else {
                    meeting::open_url(url);
                }
            }
            _ => {}
        }
    }

    /// The band above the status bar: a hairline along its top, the workday from its left end to
    /// its right, a mark at its start and end, blocks of time held under the calls, calls in lanes,
    /// a tick for now, a dot for all-day events and for blocks outside the band. Hover only: a
    /// card for each, no click.
    pub(crate) fn render_timeline(&self) -> Option<AnyElement> {
        let gone = |m: &Mark| {
            matches!(m, Mark::BandBlock(_) | Mark::BandOut(_) | Mark::CalendarAllDay | Mark::CalendarFirstRow)
        };
        self.marks.borrow_mut().retain(|m, _| !gone(m));
        let calendar = &self.model.calendar;
        // Not before the first read: there is no day yet.
        if !calendar.on || calendar.span(0).1 == 0 {
            return None;
        }
        let now = now_ms();
        let offset = zone_offset(now);
        let layout = calendar.band(self.timeline.workday, now, &zone_offset);
        let band = layout.band;
        let reduced = self.timeline.reduce_motion;
        let event = |id: u64| calendar.events.iter().find(|e| e.id == id);
        let card = |tip: String| {
            move |_: &mut Window, cx: &mut gpui::App| AnyView::from(cx.new(|_| Tooltip(tip.clone().into())))
        };
        // A hover area `HOVER_PAD` wider than what it holds on each side, as tall as the band.
        let area =
            |id: gpui::ElementId| div().id(id).absolute().top_0().bottom_0().left(px(-HOVER_PAD)).right(px(-HOVER_PAD));
        let hover = |id: gpui::ElementId, tip: String| area(id).tooltip(card(tip));
        // A meeting's: the card the Calendar section's rows show.
        let hover_card = |id: gpui::ElementId, tip: progress::MeetingTip| area(id).tooltip(tip);
        let at = |from: f32, to: f32| div().absolute().top_0().bottom_0().left(relative(from)).w(relative(to - from));
        // Inset, so a mark or block at either end is drawn whole.
        let mut out = div().absolute().top_0().bottom_0().left(px(INSET)).right(px(INSET));
        out = out.child(mark(&self.marks, Mark::Band));
        for (n, (t, what)) in
            [(band.workday.0, "Workday starts"), (band.workday.1, "Workday ends")].into_iter().enumerate()
        {
            let x = band.at(t);
            let tick = div().absolute().top(px(LANES_TOP)).bottom(px(LANES_BOTTOM)).w(px(1.)).bg(rgb(theme::MUTED));
            let tip = format!("{what} {}", calendar::clock(t, zone_offset(t)));
            out = out.child(
                at(x, x)
                    .child(tick.opacity(0.6).child(mark(&self.marks, Mark::BandEdge(n))))
                    .child(hover(("workday-mark", n).into(), tip)),
            );
        }
        // In the insets, over the workday marks: all-day events at the left's top; blocks wholly before
        // or after the band at the bottom of its end (a reminder at 00:00 does not stretch it).
        let dot = |left: bool, top: bool, tip: String, id: &'static str| {
            let d = div().absolute().size(px(4.)).rounded_full().bg(rgb(theme::MUTED)).child(hover(id.into(), tip));
            let d = if left { d.left(px(1. - INSET)) } else { d.right(px(1. - INSET)) };
            if top { d.top(px(LANES_TOP)) } else { d.bottom(px(LANES_BOTTOM)) }
        };
        let all_day: Vec<String> =
            calendar.today().filter(|e| e.all_day).map(|e| calendar::describe(e, offset)).collect();
        if !all_day.is_empty() {
            out = out.child(dot(true, true, all_day.join("\n"), "meeting-all-day"));
        }
        for (n, ids) in [&layout.early, &layout.late].into_iter().enumerate() {
            let tips: Vec<String> =
                ids.iter().filter_map(|id| event(*id)).map(|e| calendar::describe(e, offset)).collect();
            if !tips.is_empty() {
                let id = if n == 0 { "meeting-early" } else { "meeting-late" };
                out = out.child(dot(n == 0, false, tips.join("\n"), id).child(mark(&self.marks, Mark::BandOut(n))));
            }
        }
        // Blocks first, under the calls: time held with no call takes no lane.
        let mut drawn = 0;
        for block in &layout.blocks {
            let Some(e) = event(block.id) else { continue };
            let paint = calendar.paint(e, now, reduced);
            let fill = div().absolute().left_0().right_0().top(px(LANES_TOP)).bottom(px(LANES_BOTTOM)).rounded_sm();
            let fill = fill.bg(rgb(paint.color)).opacity(paint.alpha * BLOCK_STRENGTH);
            let card = progress::meeting_tip(calendar::facts(e, &[], offset), paint.color);
            let hovered = hover_card(("meeting-block", block.id as usize).into(), card);
            out = out.child(
                at(block.from, block.to)
                    .min_w(px(MIN_BLOCK))
                    .child(fill.child(mark(&self.marks, Mark::BandBlock(drawn))))
                    .child(hovered),
            );
            drawn += 1;
        }
        let room = INNER - LANES_TOP - LANES_BOTTOM;
        for bar in &layout.calls {
            let Some(e) = event(bar.id) else { continue };
            let lanes = f32::from(bar.lanes.max(1));
            let height = (room - LANE_GAP * (lanes - 1.)) / lanes;
            let top = LANES_TOP + f32::from(bar.lane) * (height + LANE_GAP);
            // One colour per kind or rule; the pulse and the stage are its strength.
            let paint = calendar.paint(e, now, reduced);
            let block = div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(top))
                .h(px(height))
                .rounded_sm()
                .bg(rgb(paint.color))
                .opacity(paint.alpha)
                .child(mark(&self.marks, Mark::BandBlock(drawn)));
            drawn += 1;
            // A lane is hovered from half way to the lane above to half way to the one below, the
            // first and last to the band's edges: every overlapping call can be, where it is drawn.
            let more: Vec<&Event> = bar.more.iter().filter_map(|id| event(*id)).collect();
            let first = bar.lane == 0;
            let last = f32::from(bar.lane) + 1. >= lanes;
            let from = if first { 0. } else { top - LANE_GAP / 2. };
            let to = if last { INNER } else { top + height + LANE_GAP / 2. };
            let card = progress::meeting_tip(calendar::facts(e, &more, offset), paint.color);
            let hovered =
                hover_card(("meeting-bar", bar.id as usize).into(), card).top(px(from)).bottom(px(INNER - to));
            out = out.child(at(bar.from, bar.to).min_w(px(MIN_BLOCK)).child(block).child(hovered));
        }
        // Now, last: over the blocks.
        let x = band.at(now);
        let tick = div().absolute().top_0().bottom_0().w(px(1.5)).ml(px(-0.75)).bg(rgb(theme::TEXT));
        out = out.child(
            at(x, x)
                .child(tick.child(mark(&self.marks, Mark::BandNow)))
                .child(hover("timeline-now".into(), calendar.now_card(now, offset))),
        );
        // A hairline along the top, as the status bar's and the panes' edges: the band reads as
        // part of the chrome, not empty space.
        let band = div().flex_none().relative().h(px(BAND)).w_full().bg(rgb(theme::BACKGROUND));
        let band = band.border_t_1().border_color(rgb(theme::BORDER));
        Some(band.child(out).into_any_element())
    }

    /// Across the top, under the title strip, while a meeting runs and no call is on:
    /// its title, how long ago it started, Join (an `https:` link only) and Dismiss.
    pub(crate) fn render_meeting_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (id, line, link) = self.model.calendar.banner_line(now_ms())?;
        let button = |control: String, label: &'static str| {
            div()
                .id(gpui::ElementId::Name(control.clone().into()))
                .relative()
                .px_2()
                .py_0p5()
                .rounded_sm()
                .cursor_pointer()
                .text_color(rgb(theme::TEXT))
                .bg(rgb(theme::SELECTED))
                .hover(|s| s.bg(rgb(theme::BORDER_STRONG)))
                .child(label)
                .child(self.controls.mark(control))
        };
        let join = link.then(|| {
            button(format!("meeting-join:{id}"), "Join").on_click(cx.listener(move |this, _, window, cx| {
                this.dispatch(Action::CalendarJoin { id }, this.input, window, cx);
            }))
        });
        let dismiss =
            button(format!("meeting-dismiss:{id}"), "Dismiss").on_click(cx.listener(move |this, _, window, cx| {
                this.dispatch(Action::CalendarDismiss { id }, this.input, window, cx);
            }));
        let banner = div()
            .id("meeting-banner")
            .absolute()
            // Under the notice bar, if one shows.
            .top(px(crate::TOP_BAR + self.bar.height))
            .left_0()
            .right_0()
            .h(px(32.))
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .bg(rgb(theme::RAISED))
            .border_b_1()
            .border_l_4()
            .border_color(rgb(theme::SNEAK))
            .text_xs()
            .child(div().flex_none().text_color(rgb(theme::SNEAK)).child("Meeting started"))
            .child(div().min_w_0().truncate().text_color(rgb(theme::TEXT)).child(line))
            .child(div().flex_1())
            .children(join)
            .child(dismiss)
            // A click here is no click on the panes beneath.
            .occlude();
        Some(banner.into_any_element())
    }

    /// What the state says of the timeline: counts, times and stages, never a title or a place,
    /// and where the banner's buttons are. Every reply carries the state: titles are only in the
    /// `calendar` request's own reply (`calendar_reply`).
    pub(crate) fn calendar_snapshot(&self) -> serde_json::Value {
        let calendar = &self.model.calendar;
        let now = now_ms();
        let today: Vec<&Event> = calendar.today().collect();
        let next = calendar.next(now);
        let banner = calendar.banner(now).map(|e| e.id);
        let at = |control: String| self.controls.bounds(&control).map(crate::rect);
        let layout = calendar.band(self.timeline.workday, now, &zone_offset);
        let band = layout.band;
        let marks = self.marks.borrow();
        let drawn = |m: Mark| marks.get(&m).copied().map(crate::rect);
        // Blocks, then calls: the order they are drawn and their marks numbered.
        let block = |n: usize, id: u64, lane: Option<(u8, u8)>, more: usize| {
            let e = calendar.events.iter().find(|e| e.id == id)?;
            let paint = calendar.paint(e, now, false);
            Some(serde_json::json!({
                "start": e.start, "end": e.end, "rect": drawn(Mark::BandBlock(n)),
                "kind": calendar::kind(e).name(), "show": e.show.name(), "color": paint.name,
                "lane": lane.map(|l| l.0), "lanes": lane.map(|l| l.1), "more": more,
            }))
        };
        let under = layout.blocks.iter().map(|b| (b.id, None, 0));
        let calls = layout.calls.iter().map(|b| (b.id, Some((b.lane, b.lanes)), b.more.len()));
        let blocks: Vec<serde_json::Value> =
            under.chain(calls).enumerate().filter_map(|(n, (id, lane, more))| block(n, id, lane, more)).collect();
        serde_json::json!({
            "on": calendar.on,
            "events_today": today.iter().filter(|e| !e.all_day).count(),
            "all_day_today": today.iter().filter(|e| e.all_day).count(),
            "events_tomorrow": calendar.tomorrow().count(),
            "next_start": next.map(|(e, _)| e.start),
            "stage": next.map(|(_, s)| s.name()),
            "updated": calendar.updated,
            "stale": calendar.stale,
            "in_call": calendar.in_call,
            "bars": layout.blocks.len() + layout.calls.len(),
            // The band above the status bar, in window points: its rect, the times at its ends and
            // the workday's (ms), the workday's marks, now's tick, each block's times and rect, kind,
            // status, colour name and lane (blocks under the calls have none), and the markers of
            // blocks wholly before and after it (`outside`: counts and rects).
            "band": (calendar.on && calendar.span(0).1 > 0).then(|| serde_json::json!({
                "rect": drawn(Mark::Band),
                "span": [band.from, band.to],
                "workday": [band.workday.0, band.workday.1],
                "marks": [drawn(Mark::BandEdge(0)), drawn(Mark::BandEdge(1))],
                "now": drawn(Mark::BandNow),
                "blocks": blocks,
                "outside": [
                    { "count": layout.early.len(), "rect": drawn(Mark::BandOut(0)) },
                    { "count": layout.late.len(), "rect": drawn(Mark::BandOut(1)) },
                ],
                "status_bar": drawn(Mark::Status),
            })),
            "banner": banner.map(|id| serde_json::json!({
                "join": at(format!("meeting-join:{id}")),
                "dismiss": at(format!("meeting-dismiss:{id}")),
            })),
            "alerts": calendar.given,
            // The Calendar section: its day (0 today, -1 yesterday) and that day's local midnight,
            // how many rows, the rail cell's mark, where the day's controls are (`today` while on
            // another day).
            "rows": calendar.agenda().len(),
            "rail": match calendar.rail(now) {
                calendar::RailMark::None => "none",
                calendar::RailMark::Near => "near",
                calendar::RailMark::Alert => "alert",
            },
            "day": calendar.day,
            "day_start": calendar.span(calendar.day).0,
            // Where its all-day line and first row are drawn, and the open section's header.
            "all_day_line": drawn(Mark::CalendarAllDay),
            "first_row": drawn(Mark::CalendarFirstRow),
            "nav": serde_json::json!({
                "back": at("calendar-day:back".into()),
                "forward": at("calendar-day:forward".into()),
                "today": at("calendar-day:today".into()),
            }),
            "joins": std::env::var_os("OTTOSPACE_E2E").map(|_| self.timeline.joins),
        })
    }
}

/// The local zone's offset at `ms`, seconds east of UTC.
fn zone_offset(ms: u64) -> i64 {
    ottospace_mac::time::utc_offset((ms / 1_000) as i64)
}

/// The system's notification, from `osascript`: a binary started from a terminal has no bundle
/// for the UserNotifications API. The text is passed as an argument, never put in the script, and
/// starts with our own word, so a title like `-e …` is no option of osascript's.
fn notify(body: &str) {
    let script = ["on run argv", "display notification (item 1 of argv) with title \"Meeting started\"", "end run"];
    let mut command = Command::new("/usr/bin/osascript");
    for line in script {
        command.args(["-e", line]);
    }
    let _ = command.arg(body).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

impl Workspace {
    /// The Calendar section: the day's all-day events on one line, then one row per meeting:
    /// its colour, start, title and at most one quiet fact (`now`, `in 12m`). The rest is on the
    /// row's hover card, the band's. Join on hover or selection. Display only: titles and places
    /// are other people's text.
    pub(crate) fn render_calendar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let calendar = &self.model.calendar;
        let now = now_ms();
        let offset = ottospace_mac::time::utc_offset((now / 1_000) as i64);
        let mut children: Vec<AnyElement> = Vec::new();
        let all_day: Vec<&Event> = calendar.on(calendar.day).filter(|e| e.all_day).collect();
        if !all_day.is_empty() {
            let titles: Vec<&str> = all_day.iter().map(|e| e.title.as_str()).collect();
            let tip = all_day.iter().map(|e| calendar::describe(e, offset)).collect::<Vec<_>>().join("\n");
            // Not shrunk away when the rows overflow: its text is cut, so it has no height of its own.
            let line = div().id("meetings-all-day").flex_none().relative().px_2().pb_1().truncate();
            let line = line.text_xs().text_color(rgb(theme::MUTED)).child(mark(&self.marks, Mark::CalendarAllDay));
            let line = line.child(format!("All day · {}", titles.join(", ")));
            children.push(
                line.tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone().into())))).into_any_element(),
            );
        }
        let rows = calendar.agenda();
        if rows.is_empty() {
            children.push(div().px_2().text_xs().text_color(rgb(theme::MUTED)).child("No meetings").into_any_element());
        }
        for (index, row) in rows.iter().enumerate() {
            let ListRow::Meeting(id) = row else { continue };
            let Some(event) = calendar.events.iter().find(|e| e.id == *id) else { continue };
            let row = self.meeting_row(event, (index, self.model.list == Some(index)), now, cx);
            let first = (index == 0).then(|| mark(&self.marks, Mark::CalendarFirstRow));
            children.push(div().flex_none().relative().children(first).child(row).into_any_element());
        }
        // Its own scroll, at the top when the section opens (it was not drawn the frame before)
        // or its day changes; never another section's.
        let (frame, day) = self.calendar_shown.get();
        if frame + 1 != self.renders || day != calendar.day {
            self.calendar_scroll.set_offset(gpui::point(px(0.), px(0.)));
        }
        self.calendar_shown.set((self.renders, calendar.day));
        let list = div().id("calendar").flex_1().flex().flex_col().gap_1().p_2().overflow_y_scroll();
        let list = list.track_scroll(&self.calendar_scroll).children(children);
        div().flex_1().min_h_0().flex().flex_col().child(self.bars.tracked("calendar", list, &self.calendar_scroll))
    }

    /// The header's day: `‹ Today ›`, `‹ Mon 16 Nov ›`, and Today while on another day. Its
    /// tooltip says when the feed was read.
    pub(crate) fn calendar_day_switch(&self, cx: &mut Context<Self>) -> Div {
        let calendar = &self.model.calendar;
        let now = now_ms();
        let offset = ottospace_mac::time::utc_offset((now / 1_000) as i64);
        // Noon of the day shown, in its own offset: a clock change at midnight moves no date.
        let noon = calendar.span(calendar.day).0 + 12 * 3_600_000;
        let label = match calendar.day {
            0 => "Today".to_string(),
            _ => calendar::day_name(noon, ottospace_mac::time::utc_offset((noon / 1_000) as i64)),
        };
        let read = format!("calendar {}", calendar.read_status(offset));
        // As tall as the header's + and ×, with room inside each: none sits against the next.
        let step = |name: &'static str, text: &'static str, tip: &'static str, step: calendar::DayStep| {
            let control = format!("calendar-day:{name}");
            div()
                .id(gpui::ElementId::Name(control.clone().into()))
                .relative()
                .h(px(28.))
                .min_w(px(20.))
                .px_1()
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .cursor_pointer()
                .text_color(rgb(theme::MUTED))
                .hover(|s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)))
                .child(text)
                .child(self.controls.mark(control))
                .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.into()))))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dispatch(Action::CalendarDay { step }, this.input, window, cx);
                }))
        };
        let date = div().id("calendar-date").min_w(px(64.)).flex().justify_center().text_color(rgb(theme::TEXT));
        let date = date.child(label).tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(read.clone().into()))));
        div()
            .ml_1()
            .flex()
            .items_center()
            .child(step("back", "‹", "The day before (←)", calendar::DayStep::Back))
            .child(date)
            .child(step("forward", "›", "The day after (→)", calendar::DayStep::Forward))
            .children(
                (calendar.day != 0)
                    .then(|| step("today", "Today", "Back to today (t)", calendar::DayStep::Today).px_2()),
            )
    }

    /// One meeting on one line, as a Tasks row (`list_button`): a dot and a faint tint in its
    /// colour (the band's), its start, its title and one quiet fact; the band's card on hover.
    /// Past ones faded, the one that runs stronger, with now's colour at its edge.
    fn meeting_row(
        &self,
        event: &Event,
        (index, selected): (usize, bool),
        now: u64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = event.id;
        let calendar = &self.model.calendar;
        let stage = calendar::stage(event, now);
        let paint = calendar.paint(event, now, true);
        // Each time in the offset it has: another day may be across a clock change.
        let clock = |ms: u64| calendar::clock(ms, ottospace_mac::time::utc_offset((ms / 1_000) as i64));
        let card = calendar::facts(event, &[], ottospace_mac::time::utc_offset((event.start / 1_000) as i64));
        let card = progress::meeting_tip(card, paint.color);
        let group: &'static str = "meeting-row";
        let join = event.link.as_ref().map(|_| {
            words::words(group, selected).top(px(2.)).child(self.word(
                format!("meeting-row-join:{id}"),
                ("Join", "Open the meeting's link"),
                move |this, window, cx| drop(this.dispatch(Action::CalendarJoin { id }, this.input, window, cx)),
                cx,
            ))
        });
        let running = stage == Stage::Running;
        let tint = div().absolute().inset_0().rounded_md().bg(rgb(paint.color));
        let tint = tint.opacity(if running { 0.22 } else { 0.1 * paint.alpha.max(0.4) });
        let fact = calendar.row_fact(event, now);
        let muted = || div().flex_none().text_xs().text_color(rgb(theme::MUTED));
        // Now's colour, as the band's tick: a bar at the edge of the meeting that runs, where a
        // Tasks row marks the task on show.
        let bar = div().absolute().left(px(-1.)).top(px(5.)).bottom(px(5.)).w(px(3.)).rounded_full();
        list_button(("meeting", index), selected)
            .group(group)
            .relative()
            .when(stage == Stage::Past, |d| d.opacity(0.5))
            .child(tint)
            .children(running.then(|| bar.bg(rgb(theme::TEXT))))
            .child(div().h(px(20.)).flex().items_center().child(home::dot(paint.color).opacity(paint.alpha.max(0.5))))
            .child(muted().relative().w(px(34.)).child(clock(event.start)))
            .child(div().relative().flex_1().min_w_0().truncate().child(event.title.clone()))
            .children(fact.map(|fact| muted().relative().child(fact)))
            .children(join)
            .tooltip(card)
            .into_any_element()
    }

    /// The Calendar rail cell's mark: a steady dot while a meeting is imminent or runs, a filled
    /// badge while one has started and the user is in no call. Never pulsing.
    pub(crate) fn calendar_rail_mark(&self) -> Option<Div> {
        let (size, ring) = match self.model.calendar.rail(now_ms()) {
            calendar::RailMark::None => return None,
            calendar::RailMark::Near => (6., false),
            calendar::RailMark::Alert => (10., true),
        };
        let mark = div().absolute().top(px(8.)).right(px(12.)).size(px(size)).rounded_full().bg(rgb(theme::SNEAK));
        Some(mark.when(ring, |d| d.border_2().border_color(rgb(theme::BACKGROUND))))
    }
}
