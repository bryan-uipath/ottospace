//! Fixed clocks and made-up events.

use super::*;

pub(super) const M: u64 = MINUTE;
pub(super) const H: u64 = 60 * M;
/// 09:00 of a day, in ms.
pub(super) const NINE: u64 = 1_794_560_400_000;

pub(super) fn event(id: u64, start: u64, minutes: u64) -> Event {
    Event {
        id,
        start,
        end: start + minutes * M,
        all_day: false,
        title: format!("Meeting {id}"),
        location: String::new(),
        link: Some("https://example.zoom.us/j/123".into()),
        show: crate::ics::Show::Busy,
    }
}

/// A block: no call link.
fn block(id: u64, start: u64, minutes: u64) -> Event {
    Event { link: None, ..event(id, start, minutes) }
}

/// The calls on `band` at `now`.
fn bars(events: &[Event], day: (u64, u64), band: &Band, now: u64) -> Vec<Bar> {
    layout(events, day, *band, now).calls
}

/// Local midnights from a week before the day of `nine` (UTC as local).
fn days_of(nine: u64) -> [u64; DAYS] {
    std::array::from_fn(|n| nine - 9 * H + n as u64 * 24 * H - BACK as u64 * 24 * H)
}

fn calendar(events: Vec<Event>) -> Calendar {
    let mut c = Calendar { on: true, notify: true, ..Calendar::default() };
    c.read(Some(events), days_of(NINE), NINE - H);
    c
}

#[test]
fn a_meeting_just_after_midnight_is_near_alerts_and_is_watched_from_the_day_before() {
    // 00:02 tomorrow, seen at 23:58 and at 00:03, before the day's next read moves `days`.
    let midnight = DAY.1;
    let mut c = calendar(vec![event(1, midnight + 2 * M, 30)]);
    let before = midnight - 2 * M;
    assert_eq!(c.next(before).map(|(e, s)| (e.id, s)), Some((1, Stage::Approaching)));
    assert_eq!(c.rail(midnight + M), RailMark::Near, "imminent at 00:01");
    assert!(c.watching(midnight + M), "the call is looked for from a minute before");
    c.call_seen(Some(false), midnight + M);
    assert_eq!(c.tick(midnight + 3 * M).len(), 1, "it alerts at its start, in the new day");
    // Far off tomorrow is not near today.
    assert_eq!(calendar(vec![event(2, midnight + 9 * H, 30)]).next(before), None);
}

#[test]
fn every_stage_boundary() {
    let e = event(1, NINE, 30);
    assert_eq!(stage(&e, NINE - APPROACHING - 1), Stage::Far);
    assert_eq!(stage(&e, NINE - APPROACHING), Stage::Approaching);
    assert_eq!(stage(&e, NINE - IMMINENT - 1), Stage::Approaching);
    assert_eq!(stage(&e, NINE - IMMINENT), Stage::Imminent);
    assert_eq!(stage(&e, NINE - 1), Stage::Imminent);
    assert_eq!(stage(&e, NINE), Stage::Running);
    assert_eq!(stage(&e, NINE + 30 * M - 1), Stage::Running);
    assert_eq!(stage(&e, NINE + 30 * M), Stage::Past);
    // An event with no length runs for its first millisecond.
    let zero = event(2, NINE, 0);
    assert_eq!((stage(&zero, NINE), stage(&zero, NINE + 1)), (Stage::Running, Stage::Past));
}

#[test]
fn the_glow_eases_and_holds_still_with_reduced_motion() {
    assert_eq!(glow(Stage::Far, 123, false), 0.);
    assert_eq!(glow(Stage::Past, 123, false), 0.);
    assert_eq!(glow(Stage::Running, 123, false), 1.);
    // A soft wave: the lowest at the start of a period, the highest half way.
    assert!((glow(Stage::Approaching, 0, false) - 0.15).abs() < 1e-4);
    assert!((glow(Stage::Approaching, 1_200, false) - 0.6).abs() < 1e-4);
    assert!((glow(Stage::Imminent, 450, false) - 1.).abs() < 1e-4);
    // No jump from one frame to the next: 100 ms apart differ by little.
    for t in (0..10_000).step_by(100) {
        assert!((glow(Stage::Imminent, t, false) - glow(Stage::Imminent, t + 100, false)).abs() < 0.35);
    }
    // Reduced motion: the same value whatever the time, stronger stage by stage.
    let steady = |s| (glow(s, 0, true), glow(s, 777, true));
    assert_eq!(steady(Stage::Approaching), (0.45, 0.45));
    assert!(glow(Stage::Imminent, 0, true) > glow(Stage::Approaching, 0, true));
}

/// Today's midnight and the next, UTC.
pub(super) const DAY: (u64, u64) = (NINE - 9 * H, NINE + 15 * H);
pub(super) const UTC: &dyn Fn(u64) -> i64 = &|_| 0;

fn workday(text: &str) -> Workday {
    Workday::parse(text).unwrap()
}

#[test]
fn the_workday_setting() {
    let table = [
        ("09:00-17:00", Some((540, 1_020))),
        ("06:00-22:00", Some((360, 1_320))),
        (" 08:30 - 18:15 ", Some((510, 1_095))),
        ("00:00-24:00", Some((0, 1_440))),
        ("09:00-09:59", None),
        ("09:00-10:00", Some((540, 600))),
        ("17:00-09:00", None),
        ("9:00-17:00", None),
        ("09:60-17:00", None),
        ("24:30-25:00", None),
        ("09:00", None),
        ("", None),
        ("+9:00-17:00", None),
    ];
    for (text, want) in table {
        assert_eq!(Workday::parse(text).map(|w| (w.start, w.end)), want, "{text:?}");
    }
    assert_eq!(Workday::default(), workday("09:00-17:00"));
}

#[test]
fn a_band_spans_the_workday_and_meetings_sit_at_their_clock_places() {
    // 06:00 to 22:00: 16 hours. A meeting 09:00–09:30 starts 3/16 in.
    let w = workday("06:00-22:00");
    let e = [event(1, NINE, 30), event(2, NINE + 5 * H, 60)];
    let band = band(&e, DAY, w, UTC);
    assert_eq!((band.from, band.to), (NINE - 3 * H, NINE + 13 * H));
    assert_eq!(band.workday, (band.from, band.to));
    let bars = bars(&e, DAY, &band, NINE - 4 * H);
    assert!((bars[0].from - 3. / 16.).abs() < 1e-6 && (bars[0].to - 3.5 / 16.).abs() < 1e-6);
    assert!((bars[1].from - 8. / 16.).abs() < 1e-6 && (bars[1].to - 9. / 16.).abs() < 1e-6);
    // The band does not move with the clock; past meetings stay, as past.
    let later = layout(&e, DAY, band, NINE + 7 * H).calls;
    assert_eq!((later[0].from, later[0].stage, later[1].stage), (bars[0].from, Stage::Past, Stage::Past));
}

#[test]
fn now_moves_along_the_band_and_pins_to_its_ends_out_of_hours() {
    let band = band(&[], DAY, Workday::default(), UTC);
    let table = [(NINE - 2 * H, 0.), (NINE, 0.), (NINE + 2 * H, 0.25), (NINE + 8 * H, 1.), (NINE + 12 * H, 1.)];
    for (now, at) in table {
        assert!((band.at(now) - at).abs() < 1e-6, "{now}: {}", band.at(now));
    }
}

#[test]
fn a_meeting_out_of_hours_widens_the_band_never_past_the_day() {
    let w = Workday::default();
    let table: [(&str, Vec<Event>, (u64, u64)); 5] = [
        ("inside", vec![event(1, NINE + H, 30)], (NINE, NINE + 8 * H)),
        ("spans the start", vec![event(1, NINE - 30 * M, 60)], (NINE - 30 * M, NINE + 8 * H)),
        ("after hours", vec![event(1, NINE + 10 * H, 60)], (NINE, NINE + 11 * H)),
        // 23:00 to 01:00: cut at midnight.
        ("overnight", vec![event(1, NINE + 14 * H, 120)], (NINE, DAY.1)),
        // Yesterday's 23:00 to 01:00 today: from midnight.
        ("from yesterday", vec![event(1, DAY.0 - H, 120)], (DAY.0, NINE + 8 * H)),
    ];
    for (name, events, want) in table {
        let band = band(&events, DAY, w, UTC);
        assert_eq!((band.from, band.to), want, "{name}");
        assert_eq!(band.workday, (NINE, NINE + 8 * H), "{name}: the marks stay at the workday");
        let bar = &bars(&events, DAY, &band, NINE)[0];
        assert!(bar.from >= 0. && bar.to <= 1. && bar.to > bar.from, "{name}: {bar:?}");
    }
    // Tomorrow's and all-day events neither widen nor show.
    let other = [event(1, DAY.1 + H, 60), Event { all_day: true, ..event(2, DAY.0, 24 * 60) }];
    let band = band(&other, DAY, w, UTC);
    assert_eq!((band.from, band.to), (NINE, NINE + 8 * H));
    assert!(bars(&other, DAY, &band, NINE).is_empty());
}

#[test]
fn a_zero_length_event_is_a_point() {
    let e = [event(1, NINE + 2 * H, 0)];
    let band = band(&e, DAY, Workday::default(), UTC);
    let bar = &bars(&e, DAY, &band, NINE)[0];
    assert_eq!((bar.from, bar.to, bar.lane, bar.lanes), (0.25, 0.25, 0, 1));
}

#[test]
fn the_workday_keeps_its_clock_on_a_day_the_clocks_change() {
    // Clocks go forward an hour at 02:00 local: from UTC-5 to UTC-4. Midnight is 05:00 UTC.
    let midnight = NINE - 4 * H;
    let change = midnight + 2 * H;
    let offset: &dyn Fn(u64) -> i64 = &|t| if t < change { -5 * 3_600 } else { -4 * 3_600 };
    let day = (midnight, midnight + 23 * H);
    let band = band(&[], day, Workday::default(), offset);
    // 09:00 local is 13:00 UTC, 8 hours before 17:00 local: not 14:00 and 22:00.
    assert_eq!(band.workday, (midnight + 8 * H, midnight + 16 * H));
    assert_eq!(clock(band.from, -4 * 3_600), "09:00");
    // Clocks go forward at midnight itself (Santiago): the day starts at 01:00, at the change
    // (04:00 UTC).
    let change = NINE - 5 * H;
    let offset: &dyn Fn(u64) -> i64 = &|t| if t < change { -4 * 3_600 } else { -3 * 3_600 };
    let band = super::band(&[], (change, change + 23 * H), Workday::default(), offset);
    assert_eq!(
        (clock(band.workday.0, -3 * 3_600), clock(band.workday.1, -3 * 3_600)),
        ("09:00".into(), "17:00".into())
    );
}

#[test]
fn overlapping_calls_get_as_many_lanes_as_they_need() {
    // (calls, each call's (id, lane, lanes), merged into a call)
    let table: [(&str, Vec<Event>, Vec<(u64, u8, u8)>, Vec<(u64, Vec<u64>)>); 7] = [
        ("alone: the full height", vec![event(1, NINE, 60)], vec![(1, 0, 1)], vec![]),
        (
            "back to back: no overlap",
            vec![event(1, NINE, 30), event(2, NINE + 30 * M, 30)],
            vec![(1, 0, 1), (2, 0, 1)],
            vec![],
        ),
        (
            "three at 09:45, then one alone",
            vec![
                event(1, NINE, 60),
                event(2, NINE + 30 * M, 60),
                event(3, NINE + 3 * H, 30),
                event(4, NINE + 45 * M, 10),
            ],
            vec![(1, 0, 3), (2, 1, 3), (4, 2, 3), (3, 0, 1)],
            vec![],
        ),
        (
            "three at once",
            vec![event(1, NINE, 60), event(2, NINE + 10 * M, 30), event(3, NINE + 20 * M, 30)],
            vec![(1, 0, 3), (2, 1, 3), (3, 2, 3)],
            vec![],
        ),
        (
            "a lane freed is used again",
            vec![event(1, NINE, 120), event(2, NINE, 30), event(3, NINE + 10 * M, 30), event(4, NINE + 30 * M, 30)],
            vec![(2, 0, 3), (1, 1, 3), (3, 2, 3), (4, 0, 3)],
            vec![],
        ),
        (
            "five at once: four lanes, the fifth in the lane that frees first",
            (1..=5).map(|n| event(n, NINE + n * M, 60 - n * 5)).collect(),
            vec![(1, 0, 4), (2, 1, 4), (3, 2, 4), (4, 3, 4)],
            vec![(1, vec![5])],
        ),
        (
            "five at once, the fifth the longest: it is drawn, the one it took the lane of is named",
            (1..=4).map(|n| event(n, NINE, 30)).chain([event(5, NINE, 180)]).collect(),
            vec![(1, 0, 4), (2, 1, 4), (3, 2, 4), (5, 3, 4)],
            vec![(5, vec![4])],
        ),
    ];
    for (name, events, want, merged) in table {
        let band = band(&events, DAY, Workday::default(), UTC);
        let bars = bars(&events, DAY, &band, NINE - H);
        let lanes: Vec<(u64, u8, u8)> = bars.iter().map(|b| (b.id, b.lane, b.lanes)).collect();
        assert_eq!(lanes, want, "{name}");
        let more: Vec<(u64, Vec<u64>)> =
            bars.iter().filter(|b| !b.more.is_empty()).map(|b| (b.id, b.more.clone())).collect();
        assert_eq!(more, merged, "{name}");
    }
}

#[test]
fn blocks_go_under_the_calls_and_never_widen_the_band() {
    let w = workday("06:00-22:00");
    // A 4-hour block under three calls, a reminder at 00:00, a block after hours, a call at 05:00.
    let events = [
        block(1, DAY.0, 15),
        event(2, NINE - 4 * H, 30),
        block(3, NINE, 240),
        event(4, NINE + H, 30),
        event(5, NINE + H, 60),
        event(6, NINE + H + 15 * M, 30),
        block(7, NINE + 14 * H, 30),
        block(8, NINE - 5 * H, 120),
    ];
    let layout = layout(&events, DAY, band(&events, DAY, w, UTC), NINE);
    // Widened to the 05:00 call only, not to the reminder or the late block.
    assert_eq!((layout.band.from, layout.band.to), (NINE - 4 * H, NINE + 13 * H));
    assert_eq!((layout.early.clone(), layout.late.clone()), (vec![1], vec![7]));
    // Under: the long block, and the one that runs into the band from before it.
    let under: Vec<(u64, f32)> = layout.blocks.iter().map(|b| (b.id, b.from)).collect();
    assert_eq!(under, [(8, 0.), (3, 4. / 17.)]);
    // The three calls at 10:00 take three lanes, the block took none.
    let lanes: Vec<(u64, u8, u8)> = layout.calls.iter().map(|b| (b.id, b.lane, b.lanes)).collect();
    assert_eq!(lanes, [(2, 0, 1), (4, 0, 3), (5, 1, 3), (6, 2, 3)]);
}

#[test]
fn a_cards_facts_name_the_place_without_links_the_link_the_status_and_the_calls_merged() {
    let one = Event {
        location: "Room 4 https://example.zoom.us/j/1?pwd=x".into(),
        show: Show::Tentative,
        ..event(1, NINE, 30)
    };
    let (two, three) = (event(2, NINE, 30), event(3, NINE, 30));
    let f = facts(&one, &[&two, &three], 0);
    assert_eq!(
        (f.title.as_str(), f.when.as_str(), f.place.as_deref()),
        ("Meeting 1", "09:00–09:30 · 30m", Some("Room 4"))
    );
    assert_eq!(
        (f.link, f.status, f.more),
        (Some("Zoom"), Some("tentative"), vec!["Meeting 2".to_string(), "Meeting 3".into()])
    );
    let all_day = Event { all_day: true, link: None, ..event(4, NINE, 0) };
    assert_eq!((facts(&all_day, &[], 0).when.as_str(), facts(&all_day, &[], 0).link), ("all day", None));
}

#[test]
fn a_focus_block_starts_with_focus_or_deep_work_and_has_no_call_link() {
    // (title, a focus block when it has no call link)
    let table = [
        ("Focus", true),
        ("Focus time", true),
        ("focus: writing", true),
        ("FOCUS - api", true),
        ("  Focus time", true),
        ("Focus/admin", true),
        ("Deep Work", true),
        ("Deep work - API", true),
        ("deep  work: parser", true),
        ("Focusing", false),
        ("Focused review", false),
        ("Focus2", false),
        ("Refocus", false),
        ("Unfocused sync", false),
        ("Time to focus", false),
        ("Deepwork", false),
        ("Deep", false),
        ("Deep workshop", false),
        ("Deep dive", false),
        ("Design review", false),
        ("", false),
    ];
    for (title, is) in table {
        let titled = |e: Event| Event { title: title.into(), ..e };
        assert_eq!(focus(&titled(block(1, NINE, 30))), is, "{title:?}");
        // The same title on a call is a call.
        assert!(!focus(&titled(event(1, NINE, 30))), "{title:?} with a Zoom link");
    }
    // A link that is no call's (a document's) leaves it a block, as on the band.
    let doc = Event { title: "Focus time".into(), link: Some("https://example.com/doc".into()), ..block(1, NINE, 30) };
    assert!(focus(&doc) && kind(&doc) == Kind::Block);
}

#[test]
fn colours_by_kind_and_by_rule_and_strength_by_status_and_stage() {
    use crate::theme;
    let rules = vec![ColorRule::new("1:1", "teal").unwrap(), ColorRule::new("SYNC", "pink").unwrap()];
    let titled = |title: &str, e: Event| Event { title: title.into(), ..e };
    let call = event(1, NINE, 30);
    let away = Event { show: Show::Away, ..block(2, NINE, 30) };
    // (what, event, stage, colour, name, strength)
    let focus = titled("Focus time", block(3, NINE, 30));
    let lit = |show: Show| Event { show, ..focus.clone() };
    let table: [(&str, Event, Stage, u32, &str, f32); 16] = [
        ("a call", call.clone(), Stage::Running, theme::MEETING, "meeting", 1.),
        ("a block", block(1, NINE, 30), Stage::Running, theme::MEETING_BLOCK, "block", 1.),
        ("away", away.clone(), Stage::Running, theme::MEETING_AWAY, "away", 1.),
        ("far off", call.clone(), Stage::Far, theme::MEETING, "meeting", 0.75),
        ("past", call.clone(), Stage::Past, theme::MEETING, "meeting", 0.35),
        ("tentative", Event { show: Show::Tentative, ..call.clone() }, Stage::Running, theme::MEETING, "meeting", 0.55),
        ("free", Event { show: Show::Free, ..call.clone() }, Stage::Running, theme::MEETING, "meeting", 0.3),
        // A rule takes the hue, case ignored, first match winning; never the treatment.
        ("a rule", titled("Ana / Bo 1:1", call.clone()), Stage::Running, theme::IDENTITY[7], "teal", 1.),
        ("first wins", titled("1:1 sync", block(1, NINE, 30)), Stage::Past, theme::IDENTITY[7], "teal", 0.35),
        (
            "case ignored, tentative kept",
            Event { show: Show::Tentative, ..titled("Weekly Sync", away) },
            Stage::Running,
            theme::IDENTITY[3],
            "pink",
            0.55,
        ),
        // A focus block: its own hue, lit by status and stage as any block.
        ("focus", focus.clone(), Stage::Running, theme::MEETING_FOCUS, "focus", 1.),
        ("focus, tentative", lit(Show::Tentative), Stage::Far, theme::MEETING_FOCUS, "focus", 0.55 * 0.75),
        ("focus, free", lit(Show::Free), Stage::Running, theme::MEETING_FOCUS, "focus", 0.3),
        ("focus, away", lit(Show::Away), Stage::Running, theme::MEETING_AWAY, "away", 1.),
        ("focus with a call link", titled("Focus time", call.clone()), Stage::Running, theme::MEETING, "meeting", 1.),
        // The user's rule is more specific than the built-in colour.
        ("a rule over focus", titled("Focus: 1:1 prep", focus.clone()), Stage::Running, theme::IDENTITY[7], "teal", 1.),
    ];
    for (what, e, stage, color, name, alpha) in table {
        let p = paint(&e, stage, 0., &rules);
        assert_eq!((p.color, p.name), (color, name), "{what}");
        assert!((p.alpha - alpha).abs() < 1e-6, "{what}: {}", p.alpha);
    }
    // Near: from the far strength up to full with the glow.
    assert!((paint(&call, Stage::Imminent, 1., &[]).alpha - 1.).abs() < 1e-6);
}

#[test]
fn a_color_rule_names_a_colour_of_the_palette() {
    assert_eq!(ColorRule::new(" Focus ", "Teal"), Ok(ColorRule { text: "focus".into(), color: 7 }));
    let wrong = ColorRule::new("x", "blue").unwrap_err();
    assert_eq!(wrong, "color is one of red, indigo, orange, pink, lime, purple, tan, teal");
    assert!(ColorRule::new("  ", "red").is_err() && ColorRule::new(&"x".repeat(101), "red").is_err());
    assert_eq!(theme_names_match(), true);
}

fn theme_names_match() -> bool {
    crate::theme::IDENTITY_NAMES.len() == crate::theme::IDENTITY.len()
}

#[test]
fn free_entries_are_left_out_while_the_setting_says() {
    let free = Event { show: Show::Free, ..event(2, NINE + H, 30) };
    let mut c = calendar(vec![event(1, NINE, 30), free]);
    assert_eq!(c.events.len(), 2);
    c.options(true, Vec::new());
    assert_eq!(c.events.iter().map(|e| e.id).collect::<Vec<_>>(), [1]);
    // A read while hidden stays hidden; turned off, it is back without a read.
    c.read(Some(c.received.clone()), c.days, NINE);
    assert_eq!(c.events.len(), 1);
    c.options(false, Vec::new());
    assert_eq!(c.events.len(), 2);
}

#[test]
fn the_now_card_names_the_time_and_the_next_meeting() {
    let c = calendar(vec![event(1, NINE, 30), event(2, NINE + 2 * H, 60)]);
    assert_eq!(c.now_card(NINE - 55 * M, 0), "08:05\nMeeting 1 · 09:00\ncalendar updated 08:00");
    assert_eq!(c.now_card(NINE + 10 * M, 0), "09:10\nMeeting 1 · now\ncalendar updated 08:00");
    assert_eq!(c.now_card(NINE + 4 * H, 0), "13:00\nNo more meetings today\ncalendar updated 08:00");
    // One coming near is named over one that runs; tomorrow's do not count.
    let back_to_back = calendar(vec![event(1, NINE, 30), event(2, NINE + 30 * M, 30)]);
    assert!(back_to_back.now_card(NINE + 25 * M, 0).contains("\nMeeting 2 · 09:30\n"));
    let tomorrow = calendar(vec![event(1, NINE + 20 * H, 30)]);
    assert!(tomorrow.now_card(NINE, 0).contains("\nNo more meetings today\n"));
    // A failed read says so, quietly, on the same card.
    let mut failed = c.clone();
    failed.read(None, failed.days, NINE);
    assert!(failed.now_card(NINE, 0).ends_with("\ncalendar not updated since 08:00"));
    let layout = c.band(Workday::default(), NINE, UTC);
    assert_eq!((layout.band.from, layout.calls.len()), (NINE, 2));
}

#[test]
fn one_alert_at_the_start_and_one_more_two_minutes_later_never_a_third() {
    let mut c = calendar(vec![event(1, NINE, 30)]);
    c.in_call = Some(false);
    assert!(c.tick(NINE - 1).is_empty(), "not before the start");
    assert!(c.due(NINE));
    assert_eq!(c.tick(NINE), [Notice { title: "Meeting 1".into(), started: NINE }]);
    assert_eq!(c.banner(NINE).map(|e| e.id), Some(1));
    assert!(c.tick(NINE + 1).is_empty(), "once");
    assert!(c.tick(NINE + REPEAT - 1).is_empty());
    assert_eq!(c.tick(NINE + REPEAT).len(), 1, "the repeat");
    assert!(!c.due(NINE + 20 * M));
    assert!(c.tick(NINE + 20 * M).is_empty(), "never a third");
    assert_eq!(c.given, 2);
    // The banner stays until the end.
    assert!(c.banner(NINE + 30 * M - 1).is_some());
    assert!(c.banner(NINE + 30 * M).is_none());
}

#[test]
fn no_alert_in_a_call_when_unknown_or_switched_off() {
    let mut c = calendar(vec![event(1, NINE, 30)]);
    c.in_call = Some(true);
    assert!(c.tick(NINE).is_empty());
    c.in_call = None;
    assert!(c.tick(NINE).is_empty(), "not looked at yet: no alert on a guess");
    c.in_call = Some(false);
    c.notify = false;
    assert!(c.tick(NINE).is_empty());
    c.notify = true;
    // A call ended after the first ten minutes: a meeting left early, no alert.
    assert!(c.tick(NINE + ALERT_WINDOW).is_empty());
    assert_eq!(c.tick(NINE + ALERT_WINDOW - 1).len(), 1);
    // An all-day event is never alerted.
    let mut all_day = calendar(vec![Event { all_day: true, ..event(2, NINE, 24 * 60) }]);
    all_day.in_call = Some(false);
    assert!(all_day.tick(NINE).is_empty());
}

#[test]
fn the_repeat_keeps_to_the_first_ten_minutes_and_a_call_ends_the_alerts() {
    // First alert at T+9m: the repeat at T+11m would be past the window.
    let mut late = calendar(vec![event(1, NINE, 60)]);
    late.in_call = Some(false);
    assert_eq!(late.tick(NINE + 9 * M).len(), 1);
    assert!(late.tick(NINE + 11 * M).is_empty());
    // Alerted, then in a call, then out of it: nothing more, and no banner.
    let mut c = calendar(vec![event(2, NINE, 60)]);
    c.in_call = Some(false);
    c.tick(NINE);
    c.call_seen(Some(true), NINE + M);
    c.call_seen(Some(false), NINE + 2 * M);
    assert!(c.tick(NINE + 3 * M).is_empty());
    assert_eq!(c.banner(NINE + 3 * M), None);
    // In a call first seen at the start, out of it soon after: this meeting is still missed.
    let mut other = calendar(vec![event(3, NINE, 60)]);
    other.call_seen(Some(true), NINE);
    assert!(other.tick(NINE).is_empty());
    other.call_seen(Some(false), NINE + 2 * M);
    assert_eq!(other.tick(NINE + 3 * M).len(), 1);
}

#[test]
fn a_meeting_that_starts_in_another_call_alerts_once_and_twice_at_most() {
    // Back to back: in the 08:00 meeting's call since 08:00, the 09:00 one alerts at its start.
    let mut c = calendar(vec![event(1, NINE - H, 60), event(2, NINE, 30)]);
    c.call_seen(Some(true), NINE - H);
    assert!(c.watching(NINE - 30 * M), "a call on is watched until it ends");
    assert!(c.tick(NINE - H + M).is_empty(), "the 08:00 one was joined");
    assert_eq!(c.tick(NINE), [Notice { title: "Meeting 2".into(), started: NINE }]);
    assert_eq!(c.banner(NINE + M).map(|e| e.id), Some(2), "the banner, in the other call");
    // The same call going on, read again, or a feed read: no more than the one repeat.
    c.call_seen(Some(true), NINE + M);
    c.read(Some(vec![event(1, NINE - H, 60), event(2, NINE, 30)]), c.days, NINE + M);
    assert!(c.tick(NINE + M).is_empty());
    assert_eq!(c.tick(NINE + REPEAT).len(), 1, "the repeat");
    assert!(c.tick(NINE + 5 * M).is_empty(), "never a third");
    // Out of that call and into the new one: the alert ends.
    let mut joined = calendar(vec![event(1, NINE - H, 60), event(2, NINE, 30)]);
    joined.call_seen(Some(true), NINE - H);
    assert_eq!(joined.tick(NINE).len(), 1);
    joined.call_seen(Some(false), NINE + M);
    joined.call_seen(Some(true), NINE + M + 4_000);
    assert!(joined.banner(NINE + M + 4_000).is_none());
    assert!(joined.tick(NINE + REPEAT).is_empty());
    // A call first seen less than `JOINED_EARLY` before the start, or after it, is this meeting's.
    for since in [NINE - JOINED_EARLY + 1, NINE - WATCH_BEFORE, NINE + M] {
        let mut early = calendar(vec![event(3, NINE, 30)]);
        early.call_seen(Some(true), since);
        assert!(early.tick(NINE.max(since)).is_empty(), "{since}");
        assert!(early.tick(NINE + REPEAT + M).is_empty(), "{since}");
    }
    // A short meeting before, running over: its call, first seen 5 minutes before, is another.
    let mut short = calendar(vec![event(1, NINE - 5 * M, 5), event(2, NINE, 30)]);
    short.call_seen(Some(true), NINE - 4 * M);
    assert_eq!(short.tick(NINE).len(), 1);
    // In that call, the banner stays only for the alert window: a switch unseen reads as one call.
    assert!(short.banner(NINE + ALERT_WINDOW - 1).is_some());
    assert!(short.banner(NINE + ALERT_WINDOW).is_none());
    let mut before = calendar(vec![event(3, NINE, 30)]);
    before.call_seen(Some(true), NINE - JOINED_EARLY);
    assert_eq!(before.tick(NINE).len(), 1);
    // Not known for a while: the call's start is forgotten, so it counts as this meeting's.
    let mut unknown = calendar(vec![event(3, NINE, 30)]);
    unknown.call_seen(Some(true), NINE - H);
    unknown.call_seen(None, NINE - M);
    assert!(unknown.tick(NINE).is_empty(), "no alert on a guess");
    unknown.call_seen(Some(true), NINE);
    assert!(unknown.tick(NINE + M).is_empty());
}

#[test]
fn the_same_occurrence_twice_alerts_once() {
    let mut c = calendar(vec![event(1, NINE, 30), event(1, NINE, 30)]);
    assert_eq!(c.events.len(), 1);
    c.in_call = Some(false);
    assert_eq!(c.tick(NINE).len(), 1);
    assert_eq!(c.tick(NINE + REPEAT).len(), 1);
    assert_eq!(c.given, 2);
}

#[test]
fn with_links_only_an_event_without_a_meeting_link_is_not_alerted() {
    let plain = Event { link: Some("https://docs.example.com/agenda".into()), ..event(2, NINE, 30) };
    let mut c = calendar(vec![event(1, NINE, 30), plain, Event { link: None, ..event(3, NINE, 30) }]);
    c.in_call = Some(false);
    c.links_only = true;
    assert_eq!(c.tick(NINE).len(), 1, "the Zoom one only");
    c.links_only = false;
    assert_eq!(c.tick(NINE).len(), 2, "the default: every meeting, with a link or none");
}

#[test]
fn a_meeting_joined_or_dismissed_alerts_no_more() {
    let mut c = calendar(vec![event(1, NINE, 30), event(2, NINE, 30)]);
    c.in_call = Some(false);
    assert_eq!(c.tick(NINE).len(), 2);
    c.dismiss(1);
    assert_eq!(c.join(2), Some("https://example.zoom.us/j/123".into()));
    assert!(c.banner(NINE + M).is_none());
    assert!(c.tick(NINE + REPEAT).is_empty(), "no repeat after either");
    // A call coming on hides the banner.
    let mut z = calendar(vec![event(3, NINE, 30)]);
    z.in_call = Some(false);
    z.tick(NINE);
    z.in_call = Some(true);
    assert!(z.banner(NINE + M).is_none());
    // Dismissed before its start: no alert at all.
    let mut early = calendar(vec![event(4, NINE, 30)]);
    early.in_call = Some(false);
    early.dismiss(4);
    assert!(early.tick(NINE).is_empty());
}

#[test]
fn join_opens_only_an_allowed_link() {
    let mut c = calendar(vec![Event { link: Some("http://example.com".into()), ..event(1, NINE, 30) }]);
    assert_eq!(c.join(1), None);
    let mut none = calendar(vec![Event { link: None, ..event(1, NINE, 30) }]);
    assert_eq!(none.join(1), None);
    assert_eq!(calendar(vec![event(1, NINE, 30)]).join(9), None, "no such event");
}

#[test]
fn the_allow_function() {
    for good in [
        "https://example.zoom.us/j/123?pwd=abc",
        "https://meet.google.com/abc-defg-hij",
        "https://teams.microsoft.com/l/meetup-join/19%3a",
    ] {
        assert!(joinable(good), "{good}");
    }
    for bad in [
        "http://example.zoom.us/j/1",
        "zoommtg://zoom.us/join",
        "javascript:alert(1)",
        "file:///etc/passwd",
        "https://user@evil.example/",
        "https://evil.example/a b",
        "https://evil.example/\"",
        "https://",
        "https:///path",
        "https://exa_mple.com/",
        "HTTPS://example.com/",
        "https://example.com/\u{202e}",
    ] {
        assert!(!joinable(bad), "{bad}");
    }
    assert!(!joinable(&format!("https://example.com/{}", "a".repeat(MAX_LINK))));
}

#[test]
fn the_feed_link_is_refused_without_being_named() {
    assert_eq!(feed_refusal("https://calendar.example.com/private-abc/basic.ics"), None);
    for bad in [
        "http://calendar.example.com/x.ics",
        "https://localhost/x.ics",
        "https://127.0.0.1/x.ics",
        "https://10.0.0.1:8443/x.ics",
        "https://2130706433/x.ics",
        "https://printer.local/x.ics",
        "https://intranet/x.ics",
        "https://a@calendar.example.com/x.ics",
        "https://[::1]/x.ics",
    ] {
        let why = feed_refusal(bad).unwrap_or_else(|| panic!("{bad}"));
        assert!(!why.contains("x.ics") && !why.contains("example"), "{why}");
    }
}

#[test]
fn curls_config_holds_the_link_and_the_limits_and_follows_no_redirect() {
    let config = curl_config("https://calendar.example.com/private-abc/basic.ics", None).unwrap();
    for line in ["proto = \"=https\"", "max-redirs = 0", "globoff", "fail", "max-time = 30", "connect-timeout = 10"] {
        assert!(config.lines().any(|l| l == line), "{line}: {config}");
    }
    for never in ["location", "proto-redir", "cookie", "netrc", "show-error", "resolve"] {
        assert!(!config.contains(never), "{never}: {config}");
    }
    assert!(config.contains("write-out = \"%{stderr}%{http_code}\""));
    // Pinned to the address that was checked.
    let ip = Some("93.184.215.14".parse().unwrap());
    let pinned = curl_config("https://calendar.example.com:8443/a.ics", ip).unwrap();
    assert!(pinned.contains("resolve = \"calendar.example.com:8443:93.184.215.14\"\n"), "{pinned}");
    let v6 = curl_config("https://calendar.example.com/a.ics", Some("2606:2800:21f:cb07::1".parse().unwrap())).unwrap();
    assert!(v6.contains("resolve = \"calendar.example.com:443:[2606:2800:21f:cb07::1]\"\n"), "{v6}");
    assert!(config.contains("max-filesize = 5242880"));
    assert!(config.ends_with("url = \"https://calendar.example.com/private-abc/basic.ics\"\n"));
    assert_eq!(curl_config("https://calendar.example.com/a\"\nupload-file = /etc/passwd", None), None);
}

/// Every refused range: its first and last address are refused, the neighbours outside are not.
#[test]
fn every_refused_range_and_its_neighbours() {
    for (net, len) in V4_REFUSED {
        let (first, last) = (net, net | (u32::MAX >> len));
        for ip in [first, last] {
            assert!(!public_address(IpAddr::V4(ip.into())), "{} in {net:#x}/{len}", std::net::Ipv4Addr::from(ip));
        }
        for ip in [first.checked_sub(1), last.checked_add(1)].into_iter().flatten() {
            let inside_another = V4_REFUSED.iter().any(|&(n, l)| ip >> (32 - l) == n >> (32 - l));
            assert_eq!(public_address(IpAddr::V4(ip.into())), !inside_another, "{}", std::net::Ipv4Addr::from(ip));
        }
    }
    for (net, len) in V6_REFUSED {
        let last = if len == 128 { net } else { net | (u128::MAX >> len) };
        for ip in [net, last] {
            assert!(!public_address(IpAddr::V6(ip.into())), "{}", std::net::Ipv6Addr::from(ip));
        }
    }
    // Neighbours of IPv6 ranges with nothing embedded.
    for ip in ["2001:db9::1", "2001:db7:ffff::1", "fbff::1", "fe00::1"] {
        assert!(public_address(ip.parse().unwrap()), "{ip}");
    }
    // An IPv4 inside IPv6 is judged as that IPv4: mapped, compatible, NAT64, 6to4.
    let wrapped = [
        "::ffff:10.0.0.1",
        "::10.0.0.1",
        "64:ff9b::a9fe:a9fe",
        "64:ff9b::7f00:1",
        "2002:c0a8:0101::1",
        "2002:7f00:1::",
    ];
    for ip in wrapped {
        assert!(!public_address(ip.parse().unwrap()), "{ip}");
    }
    for ip in ["::ffff:93.184.215.14", "64:ff9b::5db8:d70e", "2002:5db8:d70e::1", "2606:4700::1111"] {
        assert!(public_address(ip.parse().unwrap()), "{ip}");
    }
    // Every address a host resolved to must pass: one that does not refuses the fetch.
    let ip = |s: &str| -> IpAddr { s.parse().unwrap() };
    assert_eq!(pin_address(&[ip("93.184.215.14"), ip("8.8.8.8")]), Some(ip("93.184.215.14")));
    assert_eq!(pin_address(&[ip("93.184.215.14"), ip("10.0.0.1")]), None);
    assert_eq!(pin_address(&[ip("127.0.0.1"), ip("93.184.215.14")]), None);
    assert_eq!(pin_address(&[]), None);
}

#[test]
fn only_a_public_address_is_fetched_from_and_a_redirect_is_no_read() {
    let private = [
        "127.0.0.1",
        "10.1.2.3",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.169.254",
        "100.64.0.1",
        "0.0.0.0",
        "224.0.0.1",
        "::1",
        "::",
        "fe80::1",
        "fd00::1",
        "::ffff:127.0.0.1",
        "::ffff:169.254.169.254",
    ];
    for ip in private {
        assert!(!public_address(ip.parse().unwrap()), "{ip}");
    }
    for ip in ["93.184.215.14", "8.8.8.8", "2606:4700::1111"] {
        assert!(public_address(ip.parse().unwrap()), "{ip}");
    }
    assert_eq!(feed_host("https://calendar.example.com:8443/x"), Some(("calendar.example.com", 8443)));
    assert_eq!(feed_host("https://calendar.example.com/x"), Some(("calendar.example.com", 443)));
}

#[test]
fn events_are_cleaned_capped_and_linked() {
    let occurrence = |summary: &str, location: &str, description: &str, url: &str| crate::ics::Occurrence {
        uid: "u".into(),
        start: 1_794_560_400,
        end: 1_794_562_200,
        all_day: false,
        summary: summary.into(),
        location: location.into(),
        description: description.into(),
        url: url.into(),
        show: crate::ics::Show::Busy,
    };
    let got = events(&[
        occurrence(
            "Design\nreview\u{202e}\u{7}",
            "Room 4",
            "Join: https://example.zoom.us/j/1?pwd=x. Notes https://docs.example.com/a",
            "",
        ),
        occurrence(&"x".repeat(500), "", "", "https://example.com/event"),
        occurrence("", "https://meet.google.com/abc-defg-hij", "", "http://plain.example.com"),
    ]);
    assert_eq!(got[0].title, "Design review");
    assert_eq!(got[0].link.as_deref(), Some("https://example.zoom.us/j/1?pwd=x"));
    assert_eq!((got[0].start, got[0].end), (1_794_560_400_000, 1_794_562_200_000));
    assert_eq!(got[1].title.chars().count(), MAX_TITLE + 1, "cut, with an ellipsis");
    assert_eq!(got[1].link.as_deref(), Some("https://example.com/event"));
    assert_eq!(
        (got[2].title.as_str(), got[2].link.as_deref()),
        ("(no title)", Some("https://meet.google.com/abc-defg-hij"))
    );
    // The same event read again keeps its id.
    assert_eq!(events(&[occurrence("a", "", "", "")])[0].id, got[0].id);
}

#[test]
fn a_failed_read_keeps_the_last_events() {
    let mut c = calendar(vec![event(1, NINE, 30)]);
    c.read(None, c.days, NINE);
    assert!(c.stale);
    assert_eq!((c.events.len(), c.updated), (1, Some(NINE - H)));
    c.read(Some(Vec::new()), c.days, NINE + M);
    assert!(!c.stale && c.events.is_empty());
}

#[test]
fn a_day_is_named_by_its_weekday_date_and_month() {
    // NINE is 2026-11-13, a Friday, 09:00 UTC.
    assert_eq!(day_name(NINE, 0), "Fri 13 Nov");
    assert_eq!(day_name(NINE + 3 * 24 * H, 0), "Mon 16 Nov");
    assert_eq!(day_name(NINE, -10 * 3_600), "Thu 12 Nov", "23:00 the day before, ten hours west");
    assert_eq!(day_name(NINE + 50 * 24 * H, 0), "Sat 2 Jan");
}

#[test]
fn clock_times_and_the_banner_line() {
    assert_eq!(clock(NINE, 0), "09:00");
    assert_eq!(clock(NINE + 5 * M, -5 * 3_600), "04:05");
    assert_eq!(clock(NINE, 16 * 3_600), "01:00");
    let e = Event { location: "Room 4".into(), ..event(1, NINE, 30) };
    assert_eq!(describe(&e, 0), "Meeting 1\n09:00–09:30 · 30m · Room 4\nZoom link");
    let plain = Event { link: None, ..event(2, NINE, 90) };
    assert_eq!(describe(&plain, 0), "Meeting 2\n09:00–10:30 · 1h 30m");
    // A link in the place is left out (a join link may hold a passcode); the status is said.
    let linked =
        Event { location: "Room 4 https://example.zoom.us/j/1?pwd=x".into(), show: Show::Tentative, ..e.clone() };
    assert_eq!(describe(&linked, 0), "Meeting 1\n09:00–09:30 · 30m · Room 4\nZoom link · tentative");
    let away = Event { location: "https://example.zoom.us/j/1".into(), show: Show::Away, ..plain.clone() };
    assert_eq!(describe(&away, 0), "Meeting 2\n09:00–10:30 · 1h 30m\nout of office");
    assert!(describe(&Event { show: Show::Free, ..plain.clone() }, 0).ends_with("\nfree"));
    let mut c = calendar(vec![e]);
    c.in_call = Some(false);
    c.tick(NINE + 30_000);
    assert_eq!(c.banner_line(NINE + 3 * M + 1), Some((1, "Meeting 1 · started 3m ago".into(), true)));
    assert_eq!(started_line("Meeting 1", NINE, NINE + 59_999), "Meeting 1 · just started");
    let notice = |title: &str| Notice { title: title.into(), started: NINE };
    assert_eq!(notice("Meeting 1").body(NINE + 3 * M + 1), "Started 3m ago: Meeting 1");
    assert_eq!(notice("-e do shell script").body(NINE), "Started just now: -e do shell script");
}

#[test]
fn the_read_status_says_when() {
    let mut c = calendar(Vec::new());
    assert_eq!(c.read_status(0), "updated 08:00");
    c.read(None, c.days, NINE);
    assert_eq!(c.read_status(0), "not updated since 08:00");
    assert_eq!(Calendar::default().read_status(0), "not read yet");
}

/// Bryan, 2026-11-17: "shows no meetings but i definitely have some today". Before the first
/// read landed, and after a first read failed, the section said "No meetings", as for an empty day.
#[test]
fn the_section_says_loading_or_what_failed_and_draws_its_day_only_after_a_good_read() {
    use crate::model::Model;
    use crate::{Action, Origin};
    use fetch::{Failure, Fetch};
    let mut m = Model::default();
    m.update(Action::SetCalendar { on: true, notify: true, links_only: false, hide_free: false, colors: Vec::new() });
    let line = |m: &Model, now: u64| (m.calendar.draws_day(), m.calendar.fetch_line(now, 0));
    assert_eq!(line(&m, NINE), (false, "Loading the calendar…".into()), "before the thread's first word");
    m.update(Action::CalendarFetching { since: NINE, bytes: 0 });
    assert_eq!(line(&m, NINE + 3_000), (false, "Loading the calendar… 3 s".into()));
    m.update(Action::CalendarFetching { since: NINE, bytes: 300 * 1024 });
    assert_eq!(line(&m, NINE + 4_000), (false, "Loading the calendar… 300 KB".into()));
    // A first read that fails says why, and draws no day.
    m.update(Action::CalendarFetched { at: NINE + 30_000, events: 0, failure: Some(Failure::TimedOut { secs: 30 }) });
    m.update(Action::CalendarRead { events: None, days: days_of(NINE), at: NINE + 30_000, held_from: 0 });
    let failed = "Couldn't reach the calendar (timed out after 30 s)".to_string();
    assert_eq!(line(&m, NINE + 31_000), (false, failed));
    // The user's Retry fetches; a client's, or the system's, does not.
    for origin in [Origin::Socket, Origin::System] {
        assert_eq!(m.refusal(&Action::CalendarRetry, origin), Some(USERS_RETRY));
    }
    assert_eq!(m.update(Action::CalendarRetry.sent_by(Origin::User)), [crate::Effect::CalendarRetry]);
    // Read: an empty day is drawn (its hours, one free band), with when and how many.
    m.update(Action::CalendarFetching { since: NINE + 40_000, bytes: 0 });
    assert!(m.update(Action::CalendarRetry.sent_by(Origin::User)).is_empty(), "not while one runs");
    m.update(Action::CalendarFetched { at: NINE + 45_000, events: 1858, failure: None });
    m.update(Action::CalendarRead { events: Some(Vec::new()), days: days_of(NINE), at: NINE + 45_000, held_from: 0 });
    assert_eq!(m.calendar.fetch, Fetch::Read { at: NINE + 45_000, events: 1858 });
    let read = (true, "Updated 2 min ago · 1858 events".to_string());
    assert_eq!(line(&m, NINE + 45_000 + 2 * M), read);
    // A later failure keeps what was read, and says when that was.
    m.update(Action::CalendarFetched { at: NINE + H, events: 0, failure: Some(Failure::Http(403)) });
    m.update(Action::CalendarRead { events: None, days: days_of(NINE), at: NINE + 45_000, held_from: 0 });
    let after = (true, "The calendar answered 403 · last read 09:00".to_string());
    assert_eq!(line(&m, NINE + H), after);
}

/// Bryan, 2026-11-18: "it is a little jarring when it pops in". The band was drawn only once a read
/// landed; now it is there while a feed is set, loading, failed, then the meetings.
#[test]
fn the_band_is_there_from_the_start_loading_or_failed_until_a_read_lands() {
    use crate::Action;
    use crate::Origin;
    use crate::model::Model;
    use fetch::Failure;
    let mut m = Model::default();
    assert_eq!(m.calendar.band_shows(), BandShows::Absent, "no feed, no band");
    m.update(Action::SetCalendar { on: true, notify: true, links_only: false, hide_free: false, colors: Vec::new() });
    assert_eq!(m.calendar.band_shows(), BandShows::Loading, "before the thread's first word");
    m.update(Action::CalendarFetching { since: NINE, bytes: 0 });
    let shows = m.calendar.band_shows();
    assert_eq!((shows, shows.slides(false), shows.line(false)), (BandShows::Loading, true, None));
    // Reduce motion: still, a word in place of the bar.
    assert_eq!((shows.slides(true), shows.line(true)), (false, Some("Loading…")));
    // A first read that fails: a line, nothing moves. A read with no events (the failed one's) is no read.
    m.update(Action::CalendarRead { events: None, days: days_of(NINE), at: NINE, held_from: 0 });
    m.update(Action::CalendarFetched { at: NINE + 1_000, events: 0, failure: Some(Failure::Http(403)) });
    let shows = m.calendar.band_shows();
    assert_eq!(
        (shows, shows.slides(false), shows.line(false)),
        (BandShows::Failed, false, Some("Couldn't read the calendar"))
    );
    // Tried again by itself (15 s, 30 s..): quiet, the line stays and nothing slides.
    m.update(Action::CalendarFetching { since: NINE + 15_000, bytes: 0 });
    assert_eq!(m.calendar.band_shows(), BandShows::Failed);
    let tip = "The calendar answered 403 · trying again";
    assert_eq!(m.calendar.band_tip(NINE + 18_000, 0), tip, "why, not the retry's seconds");
    // The user's Retry while that one runs wakes nothing, but the band slides for it.
    assert!(m.update(Action::CalendarRetry.sent_by(Origin::User)).is_empty());
    assert_eq!(m.calendar.band_shows(), BandShows::Loading);
    m.update(Action::CalendarFetched { at: NINE + 20_000, events: 0, failure: Some(Failure::Http(403)) });
    assert_eq!(m.calendar.band_shows(), BandShows::Failed);
    // A client's Retry is refused, so it slides nothing; the user's fetches and slides.
    m.update(Action::CalendarRetry.sent_by(Origin::Socket));
    m.update(Action::CalendarFetching { since: NINE + 30_000, bytes: 0 });
    assert_eq!(m.calendar.band_shows(), BandShows::Failed, "a client's Retry");
    m.update(Action::CalendarFetched { at: NINE + 31_000, events: 0, failure: Some(Failure::Http(403)) });
    assert_eq!(m.update(Action::CalendarRetry.sent_by(Origin::User)), [crate::Effect::CalendarRetry]);
    assert_eq!(m.calendar.band_shows(), BandShows::Loading, "at once, before the thread's word");
    m.update(Action::CalendarFetching { since: NINE + 32_000, bytes: 0 });
    assert_eq!(m.calendar.band_shows(), BandShows::Loading);
    assert_eq!(m.calendar.band_tip(NINE + 32_000, 0), "Loading the calendar…");
    // Read, even empty: the band, still once later fetches run or fail.
    m.update(Action::CalendarRead { events: Some(Vec::new()), days: days_of(NINE), at: NINE + 3_000, held_from: 0 });
    for failure in [None, Some(Failure::Unreachable)] {
        m.update(Action::CalendarFetching { since: NINE + H, bytes: 0 });
        assert_eq!(m.calendar.band_shows(), BandShows::Ready);
        m.update(Action::CalendarFetched { at: NINE + H, events: 0, failure });
        let shows = m.calendar.band_shows();
        assert_eq!((shows, shows.slides(false), shows.line(true)), (BandShows::Ready, false, None));
    }
    m.update(Action::SetCalendar { on: false, notify: true, links_only: false, hide_free: false, colors: Vec::new() });
    assert_eq!(m.calendar.band_shows(), BandShows::Absent);
}

#[test]
fn the_loading_bar_crosses_the_band_and_comes_round_again() {
    // Enters at the left, eased: slow at the ends, fast through the middle.
    assert_eq!(slide(0), (0., 0.));
    let (from, to) = slide(SLIDE_MS / 2);
    assert!((from - 0.4).abs() < 1e-6 && (to - 0.6).abs() < 1e-6, "{from} {to}");
    assert!(slide(SLIDE_MS / 10).1 < 0.1, "slow at first");
    let (from, to) = slide(SLIDE_MS - 1);
    assert!(from > 0.99 && to == 1., "leaving at the right: {from} {to}");
    assert_eq!(slide(SLIDE_MS + SLIDE_MS / 2), slide(SLIDE_MS / 2), "round again");
    // Always within the band, never wider than the highlight.
    for ms in (0..SLIDE_MS).step_by(7) {
        let (from, to) = slide(ms);
        assert!((0. ..=1.).contains(&from) && from <= to && to <= 1. && to - from <= SLIDE_WIDTH + 1e-6, "{ms}");
    }
}

#[test]
fn spans_round_up_to_the_minute() {
    assert_eq!(span(1), "1m");
    assert_eq!(span(12 * M), "12m");
    assert_eq!(span(60 * M), "1h");
    assert_eq!(span(80 * M + 1), "1h 21m");
}

#[test]
fn the_agenda_lists_the_days_timed_meetings_by_start() {
    use crate::list::ListRow::Meeting;
    let all_day = Event { all_day: true, ..event(1, NINE - 9 * H, 24 * 60) };
    let tomorrow = [event(4, NINE + 24 * H, 30), Event { all_day: true, ..event(5, NINE + 15 * H, 24 * 60) }];
    let yesterday = event(6, NINE - 24 * H, 30);
    let mut c = calendar(vec![
        event(2, NINE, 30),
        event(3, NINE - 2 * H, 30),
        all_day,
        tomorrow[0].clone(),
        tomorrow[1].clone(),
        yesterday,
    ]);
    // All-day events are no rows: one line above them.
    assert_eq!(c.agenda(), [Meeting(3), Meeting(2)]);
    assert_eq!(c.on(c.day).filter(|e| e.all_day).map(|e| e.id).collect::<Vec<_>>(), [1]);
    c.step(DayStep::Forward);
    assert_eq!((c.day, c.agenda()), (1, vec![Meeting(4)]));
    c.step(DayStep::Back);
    c.step(DayStep::Back);
    assert_eq!((c.day, c.agenda()), (-1, vec![Meeting(6)]));
    assert!(calendar(Vec::new()).agenda().is_empty());
}

#[test]
fn the_sections_day_stays_within_the_days_held_and_comes_back_when_the_day_turns() {
    let mut c = calendar(vec![event(1, NINE, 30)]);
    // (step, how many times, the day after)
    let table = [(DayStep::Back, 3, -3), (DayStep::Back, 10, -7), (DayStep::Today, 1, 0), (DayStep::Forward, 20, 7)];
    for (step, times, want) in table {
        (0..times).for_each(|_| c.step(step));
        assert_eq!(c.day, want, "{step:?} x{times}");
    }
    // A week on, the last day held still has its span; a read of the same day keeps the day shown.
    assert!(c.span(7).1 > c.span(7).0);
    c.read(None, c.days, NINE);
    assert_eq!(c.day, 7);
    // The day turns: back to today.
    c.read(None, days_of(NINE + 24 * H), NINE + 24 * H);
    assert_eq!(c.day, 0);
    // A past day the read could not hold whole is not shown: back stops at the first held whole.
    c.held_from = c.span(-3).0;
    (0..9).for_each(|_| c.step(DayStep::Back));
    assert_eq!(c.day, -3);
    // Before the first read there is no day to move to.
    let mut unread = Calendar { on: true, ..Calendar::default() };
    unread.step(DayStep::Forward);
    assert_eq!(unread.day, 0);
}

#[test]
fn list_mode_starts_on_the_meeting_that_runs_else_the_next_else_the_last() {
    use crate::model::{Action, Model, Origin, Section};
    let c = calendar(vec![event(1, NINE, 30), event(2, NINE + H, 30), event(3, NINE + 2 * H, 30)]);
    let table = [
        (NINE - H, 0, "before them all: the first, the next"),
        (NINE + 10 * M, 0, "the one that runs"),
        (NINE + H + 5 * M, 1, "the one that runs, not the first"),
        (NINE + 40 * M, 1, "between: the next"),
        (NINE + 5 * H, 2, "all past: the last"),
    ];
    for (now, want, name) in table {
        assert_eq!(c.first_row(now), want, "{name}");
    }
    // Two that run: the one that started first.
    let both = calendar(vec![event(1, NINE, 60), event(2, NINE + 15 * M, 30)]);
    assert_eq!(both.first_row(NINE + 20 * M), 0);
    // No meetings: row 0.
    assert_eq!(calendar(Vec::new()).first_row(NINE), 0);
    // ⌘⌃E in the open section, the time passed in a tick first.
    let mut m = Model::default();
    m.update(
        Action::SetCalendar { on: true, notify: true, links_only: false, hide_free: false, colors: Vec::new() }
            .sent_by(Origin::System),
    );
    let events = Some(vec![event(1, NINE, 30), event(2, NINE + H, 30), event(3, NINE + 2 * H, 30)]);
    m.update(Action::CalendarRead { events, days: days_of(NINE), at: NINE, held_from: 0 }.sent_by(Origin::System));
    m.update(Action::ToggleSection { section: Section::Calendar });
    m.update(Action::Tick { now: NINE + H + 5 * M }.sent_by(Origin::System));
    m.update(Action::FocusList);
    assert_eq!(m.list, Some(1));
}

#[test]
fn the_rail_shows_a_dot_near_a_meeting_and_a_badge_while_it_alerts() {
    let mut c = calendar(vec![event(1, NINE, 30)]);
    assert_eq!(c.rail(NINE - 5 * M), RailMark::None, "approaching: the pulse is the hairline's");
    assert_eq!(c.rail(NINE - IMMINENT), RailMark::Near);
    assert_eq!(c.rail(NINE + M), RailMark::Near);
    c.in_call = Some(false);
    c.tick(NINE);
    assert_eq!(c.rail(NINE + M), RailMark::Alert);
    c.dismiss(1);
    assert_eq!(c.rail(NINE + M), RailMark::Near);
    assert_eq!(c.rail(NINE + 30 * M), RailMark::None);
    c.on = false;
    assert_eq!(c.rail(NINE + M), RailMark::None);
}

#[test]
fn the_calendar_section_is_there_only_with_a_feed() {
    use crate::model::{Action, Model, Origin, Section};
    let mut m = Model::default();
    assert!(!m.sections().contains(&Section::Calendar));
    m.update(Action::ToggleSection { section: Section::Calendar });
    assert_eq!(m.sidebar, None, "no feed: no section to open");
    m.update(
        Action::SetCalendar { on: true, notify: true, links_only: false, hide_free: false, colors: Vec::new() }
            .sent_by(Origin::System),
    );
    assert_eq!(m.sections()[..3], [Section::Repos, Section::Tasks, Section::Calendar]);
    m.update(Action::ToggleSection { section: Section::Calendar });
    assert_eq!(m.sidebar, Some(Section::Calendar));
    // Not written to the layout: a build without the section could not read the file.
    assert_eq!(m.layout_file(&[], &[], 0).section, None);
    // The feed goes (a later start without it): the section closes.
    m.update(
        Action::SetCalendar { on: false, notify: true, links_only: false, hide_free: false, colors: Vec::new() }
            .sent_by(Origin::System),
    );
    assert_eq!(m.sidebar, None);
}

#[test]
fn enter_on_a_meeting_joins_it_for_the_user_only() {
    use crate::model::{Action, Effect, Model, Origin, Section};
    let mut m = Model::default();
    m.update(
        Action::SetCalendar { on: true, notify: true, links_only: false, hide_free: false, colors: Vec::new() }
            .sent_by(Origin::System),
    );
    let days = days_of(NINE);
    let read = Action::CalendarRead { events: Some(vec![event(7, NINE, 30)]), days, at: NINE, held_from: 0 };
    m.update(read.sent_by(Origin::System));
    m.update(Action::ToggleSection { section: Section::Calendar });
    m.update(Action::FocusList);
    assert_eq!(m.list, Some(0));
    let client = m.update(Action::ListChoose { axis: crate::Axis::Right }.sent_by(Origin::Socket));
    assert!(client.is_empty(), "a client's ↵ opens nothing");
    let user = m.update(Action::ListChoose { axis: crate::Axis::Right });
    assert_eq!(user, [Effect::OpenLink { url: "https://example.zoom.us/j/123".into() }]);
}

#[test]
fn the_sections_day_moves_by_key_or_click_from_anyone_and_opens_the_section() {
    use crate::model::{Action, Model, Origin, Section};
    let mut m = Model::default();
    let step = |step| Action::CalendarDay { step };
    // No feed: nothing moves, nothing opens.
    m.update(step(DayStep::Back));
    assert_eq!((m.calendar.day, m.sidebar), (0, None));
    m.update(
        Action::SetCalendar { on: true, notify: true, links_only: false, hide_free: false, colors: Vec::new() }
            .sent_by(Origin::System),
    );
    let events = vec![event(1, NINE, 30), event(2, NINE + H, 30), event(3, NINE - 24 * H, 30)];
    let read = Action::CalendarRead { events: Some(events), days: days_of(NINE), at: NINE, held_from: 0 };
    m.update(read.sent_by(Origin::System));
    // From the palette, with the section closed: it opens on the day before.
    m.update(step(DayStep::Back));
    assert_eq!((m.calendar.day, m.sidebar), (-1, Some(Section::Calendar)));
    // In list mode the first row of the day shown is selected; a client's key moves as the user's.
    m.update(Action::FocusList);
    m.update(step(DayStep::Today).sent_by(Origin::Socket));
    assert_eq!((m.calendar.day, m.list), (0, Some(0)));
    m.update(Action::ListMove { delta: 1 });
    m.update(step(DayStep::Forward));
    assert_eq!((m.calendar.day, m.list), (1, Some(0)), "no meeting tomorrow: still in list mode");
    assert!(m.calendar.agenda().is_empty());
}

/// The ids `possible` gives for a transcript started at `at`.
fn possible_ids(events: &[Event], at: u64) -> Vec<u64> {
    possible(events, at).into_iter().map(|e| e.id).collect()
}

#[test]
fn a_transcript_is_possibly_of_the_meetings_on_the_likeliest_first() {
    // 09:00–10:00, 09:50–10:20, 10:03–10:30, 10:10–10:40; started at 10:00 (09:00 + H).
    let events = [
        event(1, NINE, 60),
        event(2, NINE + 50 * M, 30),
        event(3, NINE + H + 3 * M, 27),
        event(4, NINE + H + 10 * M, 30),
    ];
    // The one about to start (3 minutes off) is nearer than the one on for 10 minutes; the one at
    // 09:00 has just ended; 10:10 is too far off.
    assert_eq!(possible_ids(&events, NINE + H), [3, 2]);
    assert_eq!(possible_ids(&events, NINE + 58 * M), [3, 2, 1]);
    // Never more than three.
    let many: Vec<Event> = (1..=5).map(|id| event(id, NINE + id * M, 60)).collect();
    assert_eq!(possible_ids(&many, NINE + 30 * M), [5, 4, 3]);
}

#[test]
fn one_that_just_ended_counts_only_when_nothing_else_does() {
    let ended = [event(1, NINE, 30), event(2, NINE - H, 89)];
    // 09:30 + 4 minutes: both ended within five, the last to end first and alone.
    assert_eq!(possible_ids(&ended, NINE + 34 * M), [1]);
    assert_eq!(possible_ids(&ended, NINE + 36 * M), Vec::<u64>::new(), "ended too long ago");
    let mut on = ended.to_vec();
    on.push(event(3, NINE + 30 * M, 30));
    assert_eq!(possible_ids(&on, NINE + 34 * M), [3]);
}

#[test]
fn no_meeting_on_is_none() {
    assert_eq!(possible_ids(&[], NINE), Vec::<u64>::new());
    assert_eq!(possible_ids(&[event(1, NINE + 6 * M, 30)], NINE), Vec::<u64>::new(), "six minutes off");
    assert_eq!(possible_ids(&[event(1, NINE + 5 * M, 30)], NINE), [1], "five minutes off");
}

#[test]
fn all_day_free_away_focus_cancelled_and_empty_entries_are_no_meetings() {
    use crate::ics::Show;
    let on = |e: Event| possible_ids(&[e], NINE + 10 * M);
    assert_eq!(on(Event { all_day: true, ..event(1, NINE, 24 * 60) }), Vec::<u64>::new());
    assert_eq!(on(Event { show: Show::Free, ..event(1, NINE, 60) }), Vec::<u64>::new());
    assert_eq!(on(Event { show: Show::Away, ..event(1, NINE, 60) }), Vec::<u64>::new());
    assert_eq!(on(Event { title: "Focus: writing".into(), ..block(1, NINE, 60) }), Vec::<u64>::new());
    for title in ["Canceled: Standup", "cancelled: Standup", " CANCELED: x"] {
        assert_eq!(on(Event { title: title.into(), ..event(1, NINE, 60) }), Vec::<u64>::new(), "{title}");
    }
    assert_eq!(on(event(1, NINE, 0)), Vec::<u64>::new(), "no length");
    // A tentative one, a block that is no focus block, and a call titled Focus are meetings.
    assert_eq!(on(Event { show: Show::Tentative, ..event(1, NINE, 60) }), [1]);
    assert_eq!(on(block(1, NINE, 60)), [1]);
    assert_eq!(on(Event { title: "Focus sync".into(), ..event(1, NINE, 60) }), [1]);
    assert_eq!(on(Event { title: "Cancellation policy review".into(), ..event(1, NINE, 60) }), [1]);
}
