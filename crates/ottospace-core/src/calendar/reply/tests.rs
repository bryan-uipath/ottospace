//! Made-up meetings on fixed days.

use super::*;
use crate::calendar::Calendar;

/// A workday of the whole day: free time is every gap.
const WHOLE: Workday = Workday { start: 0, end: 24 * 60 };

const H: u64 = 3_600_000;
const M: u64 = 60_000;

/// Midnight UTC of a date, in seconds.
fn midnight(y: i64, m: i64, d: i64) -> i64 {
    ics::days_from_civil(y, m, d) * DAY
}

/// Monday 2026-11-16, 00:00 UTC, in ms.
fn monday() -> u64 {
    ms(midnight(2026, 11, 16))
}

fn event(id: u64, start: u64, minutes: u64) -> Event {
    Event {
        id,
        start,
        end: start + minutes * M,
        all_day: false,
        title: format!("Meeting {id}"),
        location: String::new(),
        link: None,
        tentative: false,
    }
}

fn calendar(events: Vec<Event>) -> Calendar {
    let mut c = Calendar { on: true, ..Calendar::default() };
    let day = monday();
    c.read(Some(events), [day, day + 24 * H, day + 48 * H], day + 8 * H);
    c
}

fn utc() -> Table {
    Table::fixed(0)
}

/// The busy and free spans of Monday, as `HH:MM` pairs.
fn day_spans(events: Vec<Event>) -> (Vec<String>, Vec<String>) {
    let c = calendar(events);
    let a = answer(&c, (midnight(2026, 11, 16), midnight(2026, 11, 17)), WHOLE, &utc());
    let short = |list: &[Span]| list.iter().map(|s| format!("{}-{}", &s.start[11..16], &s.end[11..16])).collect();
    (short(&a.busy), short(&a.free))
}

/// A case: its name, the day's events, then the busy and the free spans expected.
type Case = (&'static str, Vec<Event>, &'static [&'static str], &'static [&'static str]);

#[test]
fn busy_and_free_time() {
    let day = monday();
    let all_day = Event { all_day: true, ..event(9, day, 24 * 60) };
    let cases: Vec<Case> = vec![
        ("empty day", vec![], &[], &["00:00-00:00"]),
        ("one", vec![event(1, day + 9 * H, 30)], &["09:00-09:30"], &["00:00-09:00", "09:30-00:00"]),
        (
            "overlapping",
            vec![event(1, day + 9 * H, 60), event(2, day + 9 * H + 30 * M, 60), event(3, day + 9 * H + 40 * M, 10)],
            &["09:00-10:30"],
            &["00:00-09:00", "10:30-00:00"],
        ),
        (
            "back to back",
            vec![event(1, day + 9 * H, 30), event(2, day + 9 * H + 30 * M, 30)],
            &["09:00-10:00"],
            &["00:00-09:00", "10:00-00:00"],
        ),
        ("all-day is not busy", vec![all_day], &[], &["00:00-00:00"]),
        ("no length is not busy", vec![event(1, day + 9 * H, 0)], &[], &["00:00-00:00"]),
        (
            "cut to the day",
            vec![event(1, day - H, 120), event(2, day + 23 * H, 120)],
            &["00:00-01:00", "23:00-00:00"],
            &["01:00-23:00"],
        ),
        ("all day busy", vec![event(1, day, 24 * 60)], &["00:00-00:00"], &[]),
    ];
    for (name, events, busy, free) in cases {
        let (got_busy, got_free) = day_spans(events);
        assert!(got_busy == busy && got_free == free, "{name}: {got_busy:?} {got_free:?}");
    }
}

#[test]
fn free_time_is_of_each_days_workday() {
    let day = monday();
    let c = calendar(vec![event(1, day + 8 * H, 90), event(2, day + 12 * H, 60), event(3, day + 40 * H + 30 * M, 90)]);
    let a = answer(&c, (midnight(2026, 11, 16), midnight(2026, 11, 18)), Workday::default(), &utc());
    let free: Vec<String> =
        a.free.iter().map(|s| format!("{} {}-{}", &s.start[8..10], &s.start[11..16], &s.end[11..16])).collect();
    assert_eq!(free, ["16 09:30-12:00", "16 13:00-17:00", "17 09:00-16:30"]);
    assert_eq!((a.busy.len(), a.workday.as_str()), (3, "09:00-17:00"), "busy is not cut to the workday");
    // On a clock change the workday keeps to the clock: 09:00 is after it, at the new offset.
    let change = midnight(2026, 3, 8) + 10 * 3_600;
    let la = Table(vec![(i64::MIN, -8 * 3_600), (change, -7 * 3_600)]);
    let span = span(None, None, change + 3_600, &la).expect("today");
    let a = answer(&Calendar { on: true, ..Calendar::default() }, span, Workday::default(), &la);
    assert_eq!(
        (a.free[0].start.as_str(), a.free[0].end.as_str()),
        ("2026-03-08T09:00:00-07:00", "2026-03-08T17:00:00-07:00")
    );
}

#[test]
fn a_day_with_a_clock_change() {
    // Los Angeles, 2026-03-08: 02:00 PST (10:00 UTC) becomes 03:00 PDT. A day of 23 hours.
    let change = midnight(2026, 3, 8) + 10 * 3_600;
    let la = Table(vec![(i64::MIN, -8 * 3_600), (change, -7 * 3_600)]);
    let now = midnight(2026, 3, 8) + 20 * 3_600; // 12:00 PDT
    let (from, to) = span(None, None, now, &la).expect("today");
    assert_eq!(to - from, 23 * 3_600);
    assert_eq!(rfc3339(ms(from), &la), "2026-03-08T00:00:00-08:00");
    assert_eq!(rfc3339(ms(to), &la), "2026-03-09T00:00:00-07:00");
    // A meeting each side of the change keeps its own offset; the free time spans it.
    let mut c = Calendar { on: true, ..Calendar::default() };
    let (before, after) = (ms(from) + H, ms(from) + 10 * H);
    c.read(Some(vec![event(1, before, 30), event(2, after, 30)]), [0; 3], ms(now));
    let a = answer(&c, (from, to), WHOLE, &la);
    assert_eq!(a.meetings[0].start, "2026-03-08T01:00:00-08:00");
    assert_eq!(a.meetings[1].start, "2026-03-08T11:00:00-07:00");
    assert_eq!(
        (a.free[1].start.as_str(), a.free[1].end.as_str()),
        ("2026-03-08T01:30:00-08:00", "2026-03-08T11:00:00-07:00")
    );
}

#[test]
fn the_span_asked_for() {
    let now = midnight(2026, 11, 16) + 15 * 3_600;
    let la = Table::fixed(-8 * 3_600); // Monday 07:00 there
    let days = |s: Result<(i64, i64), &'static str>| {
        s.map(|(from, to)| ((from - midnight(2026, 11, 16)) / 3_600, (to - from) / DAY))
    };
    assert_eq!(days(span(None, None, now, &la)), Ok((8, 1)), "today, local midnight");
    assert_eq!(days(span(Some("today"), Some(7), now, &la)), Ok((8, 7)));
    assert_eq!(days(span(Some("tomorrow"), None, now, &la)), Ok((32, 1)));
    assert_eq!(days(span(Some("2026-11-23"), None, now, &la)), Ok((8 + 7 * 24, 1)), "the last day held");
    assert_eq!(days(span(Some("2026-11-17"), Some(7), now, &la)), Ok((32, 7)));
    let not_held = "the calendar holds today and the next 7 days only";
    assert_eq!(span(Some("2026-11-24"), None, now, &la), Err(not_held));
    assert_eq!(span(Some("2026-11-18"), Some(7), now, &la), Err(not_held));
    assert_eq!(span(Some("2026-11-15"), None, now, &la), Err(not_held), "yesterday is not held");
    for count in [0, 8, 400] {
        assert_eq!(span(None, Some(count), now, &la), Err("days is 1 to 7"));
    }
    for bad in ["", "Monday", "2026-02-30", "2026-13-01", "2026-1-05", "2026/11/16", "+026-11-16", "２０２６-11-16"]
    {
        assert!(span(Some(bad), None, now, &la).is_err_and(|e| e.starts_with("day is")), "{bad}");
    }
    let (from, to) = held(now, &la);
    assert_eq!(((from - midnight(2026, 11, 16)) / 3_600, (to - from) / DAY), (8, HELD_DAYS));
}

#[test]
fn meetings_are_cleaned_capped_and_carry_no_link() {
    let day = monday();
    let zoom = Event {
        title: "Design\nreview\u{202e}\u{7} https://example.zoom.us/j/1?pwd=s3cret".into(),
        location: "Room 4 zoom.us/j/9?pwd=s3cret".into(),
        link: Some("https://example.zoom.us/j/1?pwd=s3cret".into()),
        tentative: true,
        ..event(1, day + 9 * H, 30)
    };
    let link_only = Event {
        title: "www.example.com/x".into(),
        location: "https://meet.google.com/abc".into(),
        link: Some("https://docs.example.com/agenda".into()),
        ..event(2, day + 10 * H, 30)
    };
    let long = Event { title: "x".repeat(500), ..event(3, day + 11 * H, 30) };
    let c = calendar(vec![zoom, link_only, long]);
    let a = answer(&c, (midnight(2026, 11, 16), midnight(2026, 11, 17)), WHOLE, &utc());
    let m = &a.meetings;
    assert_eq!((m[0].title.as_str(), m[0].location.as_deref()), ("Design review", Some("Room 4")));
    assert_eq!((m[0].has_call_link, m[0].status), (true, Some("tentative")));
    assert_eq!((m[1].title.as_str(), m[1].location.as_deref()), ("(no title)", None));
    assert_eq!((m[1].has_call_link, m[1].status), (false, None), "a link, but no call's");
    assert_eq!(m[2].title.chars().count(), MAX_TITLE + 1, "cut, with an ellipsis");
    let json = serde_json::to_string(&a).unwrap();
    for never in ["s3cret", "zoom.us", "docs.example.com", "meet.google", "https", "pwd"] {
        assert!(!json.contains(never), "{never} in {json}");
    }
    assert!(!json.contains("\"status\":null"), "no status when none is known");
}

#[test]
fn at_most_200_meetings_and_the_rest_counted() {
    let day = monday();
    let c = calendar((0..300).map(|n| event(n, day + n * M, 1)).collect());
    let a = answer(&c, (midnight(2026, 11, 16), midnight(2026, 11, 17)), WHOLE, &utc());
    assert_eq!((a.meetings.len(), a.more), (MAX_MEETINGS, 100));
    assert_eq!(a.busy.len(), 1, "busy counts every meeting: {:?}", a.busy);
}

#[test]
fn only_the_days_asked_for_and_how_fresh() {
    let day = monday();
    let all_day = Event { all_day: true, ..event(5, day + 24 * H, 24 * 60) };
    let mut c = calendar(vec![event(1, day + 9 * H, 30), all_day, event(6, day + 33 * H, 30)]);
    c.call_seen(Some(true), day);
    let tuesday = answer(&c, (midnight(2026, 11, 17), midnight(2026, 11, 18)), WHOLE, &utc());
    let ids: Vec<&str> = tuesday.meetings.iter().map(|m| m.title.as_str()).collect();
    assert_eq!(ids, ["Meeting 5", "Meeting 6"]);
    assert_eq!((tuesday.meetings[0].all_day, tuesday.meetings[0].end.as_str()), (true, "2026-11-18T00:00:00+00:00"));
    assert_eq!((tuesday.updated.as_deref(), tuesday.stale), (Some("2026-11-16T08:00:00+00:00"), false));
    assert_eq!((tuesday.in_call, tuesday.from.as_str()), (Some(true), "2026-11-17T00:00:00+00:00"));
    // A failed read keeps the last events and says so.
    c.read(None, c.days, day + 9 * H);
    assert!(answer(&c, (midnight(2026, 11, 17), midnight(2026, 11, 18)), WHOLE, &utc()).stale);
}

#[test]
fn no_feed_says_only_that() {
    let reply = reply(&Calendar::default(), Some("tomorrow"), Some(99), (monday(), WHOLE), &utc());
    assert_eq!(reply, Ok(serde_json::json!({ "configured": false })));
    let on = super::reply(&calendar(vec![]), None, Some(99), (monday(), WHOLE), &utc());
    assert_eq!(on, Err("days is 1 to 7"));
}

#[test]
fn a_declined_meeting_is_not_in_the_reply() {
    let me = "me@example.com";
    let feed = format!(
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\nSUMMARY:Sync\r\nDTSTART:20261116T090000Z\r\n\
         DTEND:20261116T093000Z\r\nSTATUS:TENTATIVE\r\nATTENDEE;CN=Ana:mailto:ana@example.com\r\nEND:VEVENT\r\n\
         BEGIN:VEVENT\r\nUID:b\r\nSUMMARY:Skipped\r\nDTSTART:20261116T100000Z\r\nDTEND:20261116T103000Z\r\n\
         ATTENDEE;PARTSTAT=DECLINED:mailto:{me}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    );
    let (from, to) = (midnight(2026, 11, 16), midnight(2026, 11, 17));
    let zones = ics::Zones { local: utc(), named: vec![] };
    let events = super::super::events(&ics::expand(&ics::parse_for(&feed, Some(me)), from, to, &zones));
    let mut c = Calendar { on: true, ..Calendar::default() };
    c.read(Some(events), [0; 3], monday());
    let json = serde_json::to_string(&answer(&c, (from, to), WHOLE, &utc())).unwrap();
    assert!(json.contains("\"title\":\"Sync\"") && json.contains("\"status\":\"tentative\""), "{json}");
    for never in ["Skipped", me, "ana@example.com", "Ana"] {
        assert!(!json.contains(never), "{never} in {json}");
    }
}

#[test]
fn a_reply_serializes_as_the_sdk_fixture_says() {
    // Monday in Los Angeles: an all-day event, a call in a room, then a tentative meeting.
    let la = Table::fixed(-8 * 3_600);
    let (from, to) = span(None, None, midnight(2026, 11, 16) + 17 * 3_600, &la).expect("today");
    let day = ms(from);
    let mut c = calendar(vec![
        Event { all_day: true, title: "Offsite week".into(), link: None, ..event(1, day, 24 * 60) },
        Event {
            title: "Design review".into(),
            location: "Room 4".into(),
            link: Some("https://example.zoom.us/j/1".into()),
            ..event(2, day + 9 * H, 30)
        },
        Event { title: "Planning".into(), tentative: true, ..event(3, day + 9 * H + 30 * M, 60) },
    ]);
    c.call_seen(Some(false), day);
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../../../sdk/test/calendar.json")).unwrap();
    let got = serde_json::to_value(answer(&c, (from, to), Workday::default(), &la)).unwrap();
    assert_eq!(got, fixture, "{}", serde_json::to_string_pretty(&got).unwrap());
}
