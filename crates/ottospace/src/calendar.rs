//! The meeting timeline's shell: reads the calendar feed off the main thread, draws the hairline,
//! the next-meeting chip and the banner, looks whether a call is on, and alerts. The decisions are the core's
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
use ottospace_core::calendar::{self, Event, Lane, Stage};
use ottospace_core::ics::{self, Table, Zones};
use ottospace_core::list::ListRow;
use ottospace_core::settings::expand_home;
use ottospace_core::{Action, Effect, Origin, theme};
use ottospace_mac::meeting;

use crate::{Tooltip, Workspace, config, list_row, now_ms, words};

/// Where the line for now goes on a meeting's row.
#[derive(Clone, Copy)]
enum Now {
    Above,
    Below,
}

/// The hairline's height: above the window's buttons, so it costs the title strip nothing.
pub const HAIRLINE: f32 = 6.;
/// The hairline keeps clear of the window's rounded corners by this much.
const INSET: f32 = 10.;
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
    days: [u64; 3],
    at: u64,
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
    /// The hairline's span, in milliseconds.
    window: u64,
    reduce_motion: bool,
    /// When the reduced-motion setting, the hairline and the call were last looked at (ms).
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
        let window = u64::from(settings.calendar_window_hours.unwrap_or(8)) * 3_600_000;
        let Some(feed) = settings.calendar_ics.as_deref() else { return Self { window, ..Self::default() } };
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
            let (mut feed, mut fetched, mut days) = (ics::Feed::default(), None::<Instant>, [0u64; 3]);
            loop {
                let now = now_ms();
                let due = fetched.is_none_or(|at| at.elapsed() >= every);
                let mut problem = None;
                let mut failed = false;
                if due {
                    match fetch(&source) {
                        Ok(text) => feed = ics::parse_for(&text, me.as_deref()),
                        Err(why) => (failed, problem) = (true, why),
                    }
                    fetched = Some(Instant::now());
                }
                if due || now >= days[1] {
                    let (events, today) = evaluate(&feed, now);
                    days = today;
                    let read = Read { events: (!failed).then_some(events), days, at: now, problem };
                    if tx.send(read).is_err() {
                        return;
                    }
                }
                std::thread::sleep(WAKE);
            }
        });
        Self { reads: Some(rx), window, ..Self::default() }
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

/// Today's and tomorrow's events of a feed's text, in the user's zone; and local midnight of
/// today, tomorrow and the day after (ms).
fn evaluate(feed: &ics::Feed, now: u64) -> (Vec<Event>, [u64; 3]) {
    let now = (now / 1_000) as i64;
    let (from, to) = (now - 3 * 86_400, now + 4 * 86_400);
    let local = Table(meeting::local_table(from, to));
    let days = ics::days(now, &local);
    let named = ics::named_zones(feed)
        .into_iter()
        .filter_map(|name| Some((name.clone(), Table(meeting::zone_table(&name, from, to)?))))
        .collect();
    let occurrences = ics::expand(feed, days[0], days[2], &Zones { local, named });
    (calendar::events(&occurrences), days.map(|day| (day.max(0) as u64) * 1_000))
}

impl Workspace {
    /// From the main loop: the feed's reads, the call while a meeting is near, and a redraw while the
    /// hairline pulses (10 a second) or slides (every 20 seconds).
    pub(crate) fn tick_timeline(&mut self, window: &Window, cx: &mut Context<Self>) {
        let reads: Vec<Read> = self.timeline.reads.iter().flat_map(|rx| rx.try_iter()).collect();
        for Read { events, days, at, problem } in reads {
            if problem.is_some() {
                self.error = problem.map(String::from);
            }
            self.dispatch(Action::CalendarRead { events, days, at }, Origin::System, window, cx);
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
        let pulses = calendar.today().any(|e| matches!(calendar::stage(e, now), Stage::Approaching | Stage::Imminent));
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
                let ago = now_ms().saturating_sub(*started) / 60_000;
                notify(&format!("Started {ago}m ago: {title}"));
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

    /// The hairline across the top of the window: now at its left edge, the next hours to the
    /// right. Blocks for events, faint ticks for whole hours, a dot for all-day events.
    pub(crate) fn render_hairline(&self) -> Option<AnyElement> {
        let calendar = &self.model.calendar;
        if !calendar.on {
            return None;
        }
        let now = now_ms();
        let offset = ottospace_mac::time::utc_offset((now / 1_000) as i64);
        let mut band = div().id("hairline").absolute().top_0().left(px(INSET)).right(px(INSET)).h(px(HAIRLINE));
        band = band.child(div().absolute().left_0().right_0().top(px(2.5)).h(px(1.)).bg(rgb(theme::BORDER_STRONG)));
        for (at, hour) in calendar::ticks(now, self.timeline.window, offset) {
            let tick = div().w(px(1.)).h(px(4.)).mt(px(1.)).bg(rgb(theme::MUTED)).opacity(0.5);
            band = band.child(
                div()
                    .id(("hour", usize::from(hour)))
                    .absolute()
                    .left(relative(at))
                    .ml(px(-4.))
                    .w(px(8.))
                    .h_full()
                    .flex()
                    .justify_center()
                    .child(tick)
                    .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(format!("{hour:02}:00").into())))),
            );
        }
        for bar in calendar::bars(&calendar.events, now, self.timeline.window) {
            let Some(event) = calendar.events.iter().find(|e| e.id == bar.id) else { continue };
            let tip = calendar::describe(event, offset);
            let (top, height) = match bar.lane {
                Lane::Full => (1., 4.),
                Lane::Top => (0.5, 2.25),
                Lane::Bottom => (3.25, 2.25),
            };
            let glow = calendar::glow(bar.stage, now, self.timeline.reduce_motion);
            let (color, opacity) = match bar.stage {
                Stage::Far => (theme::MUTED, 0.9),
                _ => (theme::SNEAK, 0.35 + 0.65 * glow),
            };
            band = band.child(
                div()
                    .id(("meeting-bar", bar.id as usize))
                    .absolute()
                    .left(relative(bar.from))
                    .w(relative(bar.to - bar.from))
                    .min_w(px(2.))
                    .top(px(top))
                    .h(px(height))
                    .rounded_sm()
                    .bg(rgb(color))
                    .opacity(opacity)
                    .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone().into())))),
            );
        }
        let all_day: Vec<String> =
            calendar.today().filter(|e| e.all_day).map(|e| calendar::describe(e, offset)).collect();
        if !all_day.is_empty() {
            let tip = all_day.join("\n");
            band = band.child(
                div()
                    .id("meeting-all-day")
                    .absolute()
                    .left(px(2.))
                    .top(px(1.))
                    .size(px(4.))
                    .rounded_full()
                    .bg(rgb(theme::MUTED))
                    .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone().into())))),
            );
        }
        Some(band.into_any_element())
    }

    /// The chip in the title strip that names the next event today: `Design review · in 12m`.
    /// It pulses with the hairline. Its tooltip says when the feed was last read.
    pub(crate) fn render_meeting_chip(&self) -> Option<Div> {
        let calendar = &self.model.calendar;
        let now = now_ms();
        let (text, stage) = calendar.chip(now).filter(|_| calendar.on)?;
        let offset = ottospace_mac::time::utc_offset((now / 1_000) as i64);
        let read = format!("calendar: {}", calendar.read_status(offset));
        let tip = calendar.next(now).map(|(e, _)| format!("{}\n{read}", calendar::describe(e, offset))).unwrap_or(read);
        let glow = calendar::glow(stage, now, self.timeline.reduce_motion);
        let lit = stage != Stage::Far;
        let chip = div()
            .id("meeting-chip")
            .relative()
            .h(px(22.))
            .px_2()
            .flex()
            .items_center()
            .rounded_md()
            .min_w_0()
            .max_w(px(260.))
            .text_xs()
            .text_color(rgb(if lit { theme::SNEAK } else { theme::MUTED }))
            // The glow behind the text, so the text itself stays readable.
            .child(div().absolute().inset_0().rounded_md().bg(rgb(theme::SNEAK)).opacity(0.25 * glow))
            .child(div().relative().truncate().child(text))
            .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone().into()))));
        // It shrinks, text truncated, when the title strip is short of room.
        Some(div().min_w_0().flex().items_center().child(chip))
    }

    /// The status bar's `Design review in 4m`, from five minutes before.
    pub(crate) fn meeting_soon(&self) -> Option<Div> {
        let line = self.model.calendar.soon(now_ms()).filter(|_| self.model.calendar.on)?;
        Some(div().flex_none().text_color(rgb(theme::SNEAK)).child(line))
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
            .top(px(crate::TOP_BAR))
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
            .child(div().flex_none().text_color(rgb(theme::MUTED)).child("you are in no call"))
            .child(div().flex_1())
            .children(join)
            .child(dismiss)
            // A click here is no click on the panes beneath.
            .occlude();
        Some(banner.into_any_element())
    }

    /// What the state says of the timeline: counts, times and stages, never a title or a place,
    /// and where the banner's buttons are. Other people's text stays out of reach of agents.
    pub(crate) fn calendar_snapshot(&self) -> serde_json::Value {
        let calendar = &self.model.calendar;
        let now = now_ms();
        let today: Vec<&Event> = calendar.today().collect();
        let next = calendar.next(now);
        let banner = calendar.banner(now).map(|e| e.id);
        let at = |control: String| self.controls.bounds(&control).map(crate::rect);
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
            "soon": calendar.soon(now).is_some(),
            "bars": calendar::bars(&calendar.events, now, self.timeline.window).len(),
            "chip": calendar.chip(now).is_some(),
            "banner": banner.map(|id| serde_json::json!({
                "join": at(format!("meeting-join:{id}")),
                "dismiss": at(format!("meeting-dismiss:{id}")),
            })),
            "alerts": calendar.given,
            // The Calendar section: how many rows, the rail cell's mark, "Tomorrow" unfolded.
            "rows": calendar.agenda().len(),
            "rail": match calendar.rail(now) {
                calendar::RailMark::None => "none",
                calendar::RailMark::Near => "near",
                calendar::RailMark::Alert => "alert",
            },
            "tomorrow_open": calendar.tomorrow_open,
            "joins": std::env::var_os("OTTOSPACE_E2E").map(|_| self.timeline.joins),
        })
    }
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
    /// The Calendar section: today's meetings (all-day first, then by start), the running one
    /// lit, past ones dimmed, a line where now falls; a folded "Tomorrow · n"; when the feed was
    /// read. Join on a row with an `https:` link, on hover or selection. Display only: titles and
    /// places are other people's text.
    pub(crate) fn render_calendar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let calendar = &self.model.calendar;
        let now = now_ms();
        let offset = ottospace_mac::time::utc_offset((now / 1_000) as i64);
        let rows = calendar.agenda();
        let today = calendar.today().count();
        // The line for now: above today's first meeting still to come, else below its last.
        let timed: Vec<&Event> = calendar.today().filter(|e| !e.all_day).collect();
        let now_line = |id: u64| match timed.iter().find(|e| e.start > now) {
            Some(next) => (next.id == id).then_some(Now::Above),
            None => timed.last().filter(|last| last.id == id).map(|_| Now::Below),
        };
        let mut children: Vec<AnyElement> = Vec::new();
        if today == 0 {
            children
                .push(div().px_2().text_xs().text_color(rgb(theme::MUTED)).child("Nothing today").into_any_element());
        }
        for (index, row) in rows.iter().enumerate() {
            let selected = self.model.list == Some(index);
            match row {
                ListRow::MeetingsTomorrow => {
                    // As the Apps section's fold lines.
                    let count = calendar.tomorrow().count();
                    let fold = if calendar.tomorrow_open { "▾" } else { "▸" };
                    let heading = div()
                        .id("meetings-tomorrow")
                        .relative()
                        .mt_2()
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .text_xs()
                        .text_color(rgb(theme::MUTED))
                        .cursor_pointer()
                        .hover(|s| s.text_color(rgb(theme::TEXT)))
                        .when(selected, |d| d.pb_0p5().rounded_md().bg(rgb(theme::SELECTED)))
                        .child(div().w(px(8.)).child(fold))
                        .child(format!("Tomorrow · {count}"))
                        .child(self.controls.mark("meetings-tomorrow"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.dispatch(Action::CalendarTomorrow, this.input, window, cx);
                        }));
                    children.push(heading.into_any_element());
                }
                ListRow::Meeting(id) => {
                    let Some(event) = calendar.events.iter().find(|e| e.id == *id) else { continue };
                    children.push(self.meeting_row(event, (index, selected), now, now_line(*id), cx));
                }
                _ => {}
            }
        }
        let footer = calendar.read_status(offset);
        let list = div().id("calendar").flex_1().flex().flex_col().gap_1().p_2().overflow_y_scroll();
        let list = list.track_scroll(&self.list_scroll).children(children);
        let footer = div().px_3().py_2().text_xs().text_color(rgb(theme::BORDER_STRONG)).child(footer);
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.bars.tracked("calendar", list, &self.list_scroll))
            .child(footer)
    }

    /// One meeting: its time and title, and its place or its link's host below.
    fn meeting_row(
        &self,
        event: &Event,
        (index, selected): (usize, bool),
        now: u64,
        now_line: Option<Now>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = event.id;
        let stage = calendar::stage(event, now);
        // Each time in the offset it has: tomorrow may be after a clock change.
        let clock = |ms: u64| calendar::clock(ms, ottospace_mac::time::utc_offset((ms / 1_000) as i64));
        let when = if event.all_day {
            "all day".to_string()
        } else {
            format!("{}–{}", clock(event.start), clock(event.end))
        };
        let place = Some(event.location.clone())
            .filter(|l| !l.is_empty())
            .or_else(|| event.link.as_deref().and_then(calendar::host).map(String::from));
        let group: &'static str = "meeting-row";
        let join = event.link.as_ref().map(|_| {
            words::words(group, selected).top(px(4.)).child(self.word(
                format!("meeting-row-join:{id}"),
                ("Join", "Open the meeting's link"),
                move |this, window, cx| drop(this.dispatch(Action::CalendarJoin { id }, this.input, window, cx)),
                cx,
            ))
        });
        // An all-day event is no meeting in progress.
        let running = !event.all_day && stage == Stage::Running;
        let line = |d: Div| d.absolute().left_0().right_0().h(px(1.)).bg(rgb(theme::SNEAK));
        list_row(("meeting", index), selected)
            .group(group)
            .pl_2()
            .py_1()
            .flex()
            .flex_col()
            .text_color(rgb(theme::TEXT))
            .when(stage == Stage::Past, |d| d.opacity(0.45))
            .when(running && !selected, |d| d.bg(rgb(theme::SELECTED)).border_color(rgb(theme::SNEAK)))
            // Now falls between this meeting and the one before or after.
            .children(now_line.map(|at| match at {
                Now::Above => line(div()).top(px(-3.)),
                Now::Below => line(div()).bottom(px(-3.)),
            }))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_none().w(px(78.)).text_xs().text_color(rgb(theme::MUTED)).child(when))
                    .child(div().min_w_0().truncate().child(event.title.clone())),
            )
            .children(place.map(|place| {
                div().pl(px(86.)).min_w_0().truncate().text_xs().text_color(rgb(theme::MUTED)).child(place)
            }))
            .children(join)
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
