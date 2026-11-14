//! Made-up feeds only: no real calendar's data.

use super::*;
use crate::fuzz::{Rng, caught, hostile};

const H: i64 = 3_600;

/// UTC seconds of a date and time.
fn at(y: i64, m: i64, d: i64, h: i64, min: i64) -> i64 {
    days_from_civil(y, m, d) * DAY + h * H + min * 60
}

fn feed(events: &str) -> String {
    format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{events}END:VCALENDAR\r\n")
}

fn event(lines: &str) -> String {
    format!("BEGIN:VEVENT\r\n{lines}END:VEVENT\r\n")
}

/// The occurrences on a UTC day, as (summary, start, end).
fn on(text: &str, y: i64, m: i64, d: i64, zones: &Zones) -> Vec<(String, i64, i64)> {
    let from = at(y, m, d, 0, 0);
    expand(&parse(text), from, from + DAY, zones).into_iter().map(|o| (o.summary, o.start, o.end)).collect()
}

fn utc() -> Zones {
    Zones { local: Table::fixed(0), named: Vec::new() }
}

#[test]
fn dates_and_weekdays_are_right() {
    assert_eq!(civil(days_from_civil(2026, 11, 13)), (2026, 11, 13));
    assert_eq!(weekday(days_from_civil(2026, 11, 13)), 4, "a Friday");
    assert_eq!(civil(days_from_civil(2024, 2, 29)), (2024, 2, 29));
    assert_eq!(days_from_civil(1970, 1, 1), 0);
}

#[test]
fn a_utc_event_with_folded_and_escaped_text() {
    let text = feed(&event(
        "UID:a@example\r\nSUMMARY:Design\r\n  review\\, part 2\\nnotes\r\nLOCATION:Room 4\\; east\r\n\
         DTSTART:20261113T140000Z\r\nDTEND:20261113T143000Z\r\n",
    ));
    let feed = parse(&text);
    assert_eq!(feed.events[0].summary, "Design review, part 2\nnotes");
    assert_eq!(feed.events[0].location, "Room 4; east");
    assert_eq!(
        on(&text, 2026, 11, 13, &utc()),
        [("Design review, part 2\nnotes".into(), at(2026, 11, 13, 14, 0), at(2026, 11, 13, 14, 30))]
    );
    assert!(on(&text, 2026, 11, 14, &utc()).is_empty());
}

#[test]
fn bare_newlines_and_a_duration_are_read() {
    let text = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nSUMMARY:Standup\nDTSTART:20261113T090000Z\nDURATION:PT15M\nEND:VEVENT\nEND:VCALENDAR\n";
    assert_eq!(on(text, 2026, 11, 13, &utc()), [("Standup".into(), at(2026, 11, 13, 9, 0), at(2026, 11, 13, 9, 15))]);
}

#[test]
fn floating_and_all_day_events_are_in_the_local_zone() {
    // UTC-5 all day.
    let zones = Zones { local: Table::fixed(-5 * H), named: Vec::new() };
    let text = feed(&format!(
        "{}{}",
        event("SUMMARY:Lunch\r\nDTSTART:20261113T120000\r\nDTEND:20261113T130000\r\n"),
        event("SUMMARY:Holiday\r\nDTSTART;VALUE=DATE:20261113\r\nDTEND;VALUE=DATE:20261114\r\n"),
    ));
    let feed = parse(&text);
    let from = at(2026, 11, 13, 5, 0); // local midnight
    let got = expand(&feed, from, from + DAY, &zones);
    assert_eq!(got.len(), 2);
    assert_eq!(
        (got[0].summary.as_str(), got[0].start, got[0].end, got[0].all_day),
        ("Holiday", from, from + DAY, true)
    );
    assert_eq!((got[1].start, got[1].all_day), (at(2026, 11, 13, 17, 0), false));
}

#[test]
fn a_named_zone_uses_the_shells_offsets() {
    let zones = Zones { local: Table::fixed(0), named: vec![("Asia/Tokyo".into(), Table::fixed(9 * H))] };
    let text = feed(&event("SUMMARY:Sync\r\nDTSTART;TZID=Asia/Tokyo:20261113T100000\r\nDURATION:PT1H\r\n"));
    assert_eq!(named_zones(&parse(&text)), ["Asia/Tokyo"]);
    assert_eq!(on(&text, 2026, 11, 13, &zones)[0].1, at(2026, 11, 13, 1, 0));
    // A zone nobody knows is read as local time.
    assert_eq!(on(&text, 2026, 11, 13, &utc())[0].1, at(2026, 11, 13, 10, 0));
}

const BERLIN: &str = "BEGIN:VTIMEZONE\r\nTZID:W. Europe Standard Time\r\n\
    BEGIN:STANDARD\r\nDTSTART:16010101T030000\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\n\
    RRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=10\r\nEND:STANDARD\r\n\
    BEGIN:DAYLIGHT\r\nDTSTART:16010101T020000\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0200\r\n\
    RRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=3\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\n";

#[test]
fn a_vtimezone_of_the_feed_gives_summer_and_winter_offsets() {
    let weekly = event(
        "UID:w\r\nSUMMARY:Weekly\r\nDTSTART;TZID=W. Europe Standard Time:20261020T100000\r\n\
         DTEND;TZID=W. Europe Standard Time:20261020T110000\r\nRRULE:FREQ=WEEKLY;BYDAY=TU\r\n",
    );
    let text = feed(&format!("{BERLIN}{weekly}"));
    assert!(named_zones(&parse(&text)).is_empty(), "defined in the feed");
    // 2026-10-20 is summer time (+2); 2026-10-27 is after the last Sunday of October (+1).
    assert_eq!(on(&text, 2026, 10, 20, &utc())[0].1, at(2026, 10, 20, 8, 0));
    assert_eq!(on(&text, 2026, 10, 27, &utc())[0].1, at(2026, 10, 27, 9, 0));
    // Summer again after the last Sunday of March.
    assert_eq!(on(&text, 2027, 3, 30, &utc())[0].1, at(2027, 3, 30, 8, 0));
}

#[test]
fn local_time_in_a_changing_zone() {
    let table = Table(vec![(i64::MIN, H), (at(2026, 10, 25, 1, 0), H), (at(2026, 3, 29, 1, 0), 2 * H)]);
    let mut sorted = table.0.clone();
    sorted.sort();
    let table = Table(sorted);
    assert_eq!(table.to_utc(at(2026, 6, 1, 12, 0)), at(2026, 6, 1, 10, 0));
    assert_eq!(table.to_utc(at(2026, 12, 1, 12, 0)), at(2026, 12, 1, 11, 0));
    // 02:30 on 29 March does not exist: the clock before the change.
    assert_eq!(table.to_utc(at(2026, 3, 29, 2, 30)), at(2026, 3, 29, 1, 30));
}

#[test]
fn daily_and_weekly_rules_with_interval_until_and_count() {
    let text = feed(&format!(
        "{}{}{}{}",
        event(
            "UID:d\r\nSUMMARY:Daily\r\nDTSTART:20261101T090000Z\r\nDTEND:20261101T091500Z\r\nRRULE:FREQ=DAILY;INTERVAL=2\r\n"
        ),
        event(
            "UID:w\r\nSUMMARY:MWF\r\nDTSTART:20261102T100000Z\r\nDURATION:PT30M\r\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR;UNTIL=20261112T235959Z\r\n"
        ),
        event("UID:c\r\nSUMMARY:Five\r\nDTSTART:20261109T110000Z\r\nDURATION:PT30M\r\nRRULE:FREQ=DAILY;COUNT=5\r\n"),
        event(
            "UID:b\r\nSUMMARY:Biweekly\r\nDTSTART:20261030T150000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=WEEKLY;INTERVAL=2\r\n"
        ),
    ));
    let names = |d| on(&text, 2026, 11, d, &utc()).into_iter().map(|o| o.0).collect::<Vec<_>>();
    // Every other day from the 1st; five days from the 9th; MWF until the 12th; every other Friday.
    assert_eq!(names(13), ["Daily", "Five", "Biweekly"]);
    assert_eq!(names(11), ["Daily", "MWF", "Five"]);
    assert_eq!(names(14), Vec::<String>::new());
    assert_eq!(names(27), ["Daily", "Biweekly"]);
}

#[test]
fn monthly_and_yearly_rules() {
    let text = feed(&format!(
        "{}{}{}",
        event(
            "UID:m\r\nSUMMARY:Second Friday\r\nDTSTART:20260109T160000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=MONTHLY;BYDAY=2FR\r\n"
        ),
        event(
            "UID:l\r\nSUMMARY:Last day\r\nDTSTART:20260131T080000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=MONTHLY;BYMONTHDAY=-1\r\n"
        ),
        event("UID:y\r\nSUMMARY:Birthday\r\nDTSTART;VALUE=DATE:20201113\r\nRRULE:FREQ=YEARLY\r\n"),
    ));
    let names = |m, d| on(&text, 2026, m, d, &utc()).into_iter().map(|o| o.0).collect::<Vec<_>>();
    assert_eq!(names(11, 13), ["Birthday", "Second Friday"]);
    assert_eq!(names(11, 30), ["Last day"]);
    assert_eq!(names(2, 28), ["Last day"]);
    assert_eq!(names(11, 6), Vec::<String>::new());
}

#[test]
fn exdates_overrides_and_cancelled_events() {
    let text = feed(&format!(
        "{}{}{}",
        event(
            "UID:s\r\nSUMMARY:Standup\r\nDTSTART:20261109T090000Z\r\nDURATION:PT15M\r\nRRULE:FREQ=DAILY\r\n\
             EXDATE:20261111T090000Z,20261112T090000Z\r\n"
        ),
        event(
            "UID:s\r\nRECURRENCE-ID:20261113T090000Z\r\nSUMMARY:Standup (moved)\r\nDTSTART:20261113T100000Z\r\nDURATION:PT15M\r\n"
        ),
        event("UID:x\r\nSUMMARY:Gone\r\nSTATUS:CANCELLED\r\nDTSTART:20261113T120000Z\r\nDURATION:PT15M\r\n"),
    ));
    assert!(on(&text, 2026, 11, 11, &utc()).is_empty());
    assert!(on(&text, 2026, 11, 12, &utc()).is_empty());
    assert_eq!(
        on(&text, 2026, 11, 13, &utc()),
        [("Standup (moved)".into(), at(2026, 11, 13, 10, 0), at(2026, 11, 13, 10, 15))]
    );
    assert_eq!(on(&text, 2026, 11, 14, &utc())[0].0, "Standup");
    // A cancelled occurrence of a series is an override with STATUS:CANCELLED.
    let cancelled = text.replace("SUMMARY:Standup (moved)", "STATUS:CANCELLED");
    assert!(on(&cancelled, 2026, 11, 13, &utc()).is_empty());
}

/// Made-up addresses only.
const ME: &str = "me@example.com";

/// The summaries on 13 November of a feed read as `ME`.
fn mine(text: &str) -> Vec<String> {
    let from = at(2026, 11, 13, 0, 0);
    expand(&parse_for(text, Some(ME)), from, from + DAY, &utc()).into_iter().map(|o| o.summary).collect()
}

#[test]
fn a_meeting_the_user_declined_is_left_out() {
    let one = |attendees: &str| {
        feed(&event(&format!("UID:a\r\nSUMMARY:Sync\r\nDTSTART:20261113T090000Z\r\nDURATION:PT30M\r\n{attendees}")))
    };
    let other = "ATTENDEE;PARTSTAT=ACCEPTED:mailto:other@example.com\r\n";
    let cases: [(&str, String, bool); 13] = [
        ("declined", "ATTENDEE;CN=Me;PARTSTAT=DECLINED:mailto:me@example.com\r\n".into(), false),
        ("accepted", "ATTENDEE;PARTSTAT=ACCEPTED:mailto:me@example.com\r\n".into(), true),
        ("tentative", "ATTENDEE;PARTSTAT=TENTATIVE:mailto:me@example.com\r\n".into(), true),
        ("needs action", "ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:me@example.com\r\n".into(), true),
        ("no PARTSTAT", "ATTENDEE:mailto:me@example.com\r\n".into(), true),
        ("not the user's line", "ATTENDEE;PARTSTAT=DECLINED:mailto:other@example.com\r\n".into(), true),
        ("no attendees", String::new(), true),
        ("case", "ATTENDEE;partstat=declined:MAILTO:Me@Example.COM\r\n".into(), false),
        ("EMAIL=", "ATTENDEE;EMAIL=\"me@example.com\";PARTSTAT=DECLINED:urn:uuid:1\r\n".into(), false),
        ("folded", "ATTENDEE;PARTSTAT=DECLINED:mailto:me@exa\r\n mple.com\r\n".into(), false),
        // A quoted CN cannot fake a parameter or the address.
        (
            "CN with PARTSTAT",
            "ATTENDEE;CN=\"x;PARTSTAT=DECLINED\";PARTSTAT=ACCEPTED:mailto:me@example.com\r\n".into(),
            true,
        ),
        (
            "CN with an address",
            "ATTENDEE;CN=\"a:mailto:me@example.com\";PARTSTAT=DECLINED:mailto:o@example.com\r\n".into(),
            true,
        ),
        // Past the cap of lines looked at, nothing is hidden.
        ("cap", format!("{}ATTENDEE;PARTSTAT=DECLINED:mailto:me@example.com\r\n", other.repeat(MAX_ATTENDEES)), true),
    ];
    for (name, attendees, shown) in cases {
        assert_eq!(mine(&one(&attendees)).len(), usize::from(shown), "{name}");
        // With no address set, nothing is hidden.
        assert_eq!(on(&one(&attendees), 2026, 11, 13, &utc()).len(), 1, "{name}, no address");
    }
    // The first line that is the user's counts.
    let twice =
        "ATTENDEE;PARTSTAT=ACCEPTED:mailto:me@example.com\r\nATTENDEE;PARTSTAT=DECLINED:mailto:me@example.com\r\n";
    assert_eq!(mine(&one(twice)).len(), 1);
    // No attendee's name or address is kept.
    assert_eq!(format!("{:?}", parse_for(&one(other), Some(ME))).contains("other@"), false);
}

#[test]
fn a_declined_occurrence_or_series_hides_only_what_was_declined() {
    let attendee = |status: &str| format!("ATTENDEE;PARTSTAT={status}:mailto:me@example.com\r\n");
    let series = |status: &str| {
        event(&format!(
            "UID:s\r\nSUMMARY:Standup\r\nDTSTART:20261112T090000Z\r\nDURATION:PT15M\r\nRRULE:FREQ=DAILY\r\n{}",
            attendee(status)
        ))
    };
    let moved = |status: &str| {
        event(&format!(
            "UID:s\r\nRECURRENCE-ID:20261113T090000Z\r\nSUMMARY:Standup (moved)\r\nDTSTART:20261113T100000Z\r\n\
             DURATION:PT15M\r\n{}",
            attendee(status)
        ))
    };
    let mine_on = |text: &str, d: i64| {
        let from = at(2026, 11, d, 0, 0);
        expand(&parse_for(text, Some(ME)), from, from + DAY, &utc()).len()
    };
    // One occurrence of an accepted series declined: only that one goes.
    let one_declined = feed(&format!("{}{}", series("ACCEPTED"), moved("DECLINED")));
    assert_eq!((mine_on(&one_declined, 12), mine_on(&one_declined, 13), mine_on(&one_declined, 14)), (1, 0, 1));
    // The series declined, one occurrence accepted: that one stays.
    let one_accepted = feed(&format!("{}{}", series("DECLINED"), moved("ACCEPTED")));
    assert_eq!(mine(&one_accepted), ["Standup (moved)"]);
    assert_eq!((mine_on(&one_accepted, 12), mine_on(&one_accepted, 14)), (0, 0));
    // An override with no line of the user's answers as the series does.
    let silent = moved("X").replace("ATTENDEE;PARTSTAT=X:mailto:me@example.com\r\n", "");
    assert_eq!(mine_on(&feed(&format!("{}{silent}", series("DECLINED"))), 13), 0);
    assert_eq!(mine_on(&feed(&format!("{}{silent}", series("ACCEPTED"))), 13), 1);
}

#[test]
fn month_days_and_an_ordinal_weekday_meet() {
    // The first Monday, if it is the 1st, 8th, 15th, 22nd or 29th: 1 June 2026 is a Monday.
    let text = feed(&event(
        "SUMMARY:First Monday\r\nDTSTART:20260601T090000Z\r\nRRULE:FREQ=MONTHLY;BYMONTHDAY=1,8,15,22,29;BYDAY=1MO\r\n",
    ));
    assert_eq!(on(&text, 2026, 6, 1, &utc()).len(), 1);
    assert!(on(&text, 2026, 6, 8, &utc()).is_empty(), "the second Monday");
}

#[test]
fn an_unsupported_rule_gives_its_first_occurrence_only() {
    let text = feed(&event("SUMMARY:Odd\r\nDTSTART:20261113T090000Z\r\nRRULE:FREQ=YEARLY;BYWEEKNO=20;BYDAY=MO,TU\r\n"));
    assert_eq!(on(&text, 2026, 11, 13, &utc()).len(), 1);
    assert!(on(&text, 2026, 12, 31, &utc()).is_empty());
    let hourly = feed(&event("SUMMARY:Hourly\r\nDTSTART:20261112T090000Z\r\nRRULE:FREQ=HOURLY\r\n"));
    assert!(on(&hourly, 2026, 11, 13, &utc()).is_empty());
    // The last Monday of the year, not of January: needs the whole year's days, not read.
    let yearly = feed(&event("SUMMARY:Y\r\nDTSTART:20260126T090000Z\r\nRRULE:FREQ=YEARLY;BYDAY=MO;BYSETPOS=-1\r\n"));
    assert_eq!(dates_in(&yearly, (2026, 1, 1), (2028, 1, 1)), [(2026, 1, 26)]);
}

#[test]
fn days_are_local_midnights() {
    let local = Table::fixed(-5 * H);
    let now = at(2026, 11, 13, 3, 0); // 22:00 on the 12th, local
    assert_eq!(days(now, &local), [at(2026, 11, 12, 5, 0), at(2026, 11, 13, 5, 0), at(2026, 11, 14, 5, 0)]);
}

#[test]
fn broken_lines_and_values_are_left_out() {
    let text = feed(&format!(
        "{}{}{}",
        event("SUMMARY:No start\r\n"),
        event("SUMMARY:Bad date\r\nDTSTART:20261340T250000Z\r\n"),
        event("garbage without a colon\r\nSUMMARY:Fine\r\nDTSTART:20261113T090000Z\r\nRRULE:FREQ=DAILY;INTERVAL=x\r\n"),
    ));
    assert_eq!(on(&text, 2026, 11, 13, &utc()).into_iter().map(|o| o.0).collect::<Vec<_>>(), ["Fine"]);
}

/// Arbitrary bytes, made-up feeds cut and spliced: never a panic, and the caps hold.
#[test]
fn any_text_is_read_without_a_panic() {
    let pieces = [
        "BEGIN:VCALENDAR\r\n",
        "BEGIN:VEVENT\r\n",
        "END:VEVENT\r\n",
        "BEGIN:VTIMEZONE\r\nTZID:Z\r\n",
        "BEGIN:STANDARD\r\nDTSTART:16010101T030000\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\n",
        "RRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=10\r\n",
        "END:STANDARD\r\n",
        "END:VTIMEZONE\r\n",
        "DTSTART:20261113T090000Z\r\n",
        "DTSTART;TZID=Z:20261113T090000\r\n",
        "DTSTART;VALUE=DATE:20261113\r\n",
        "DTEND:20261113T100000Z\r\n",
        "DURATION:P1W2DT3H4M5S\r\n",
        "RRULE:FREQ=WEEKLY;BYDAY=MO,-1FR;COUNT=99999;INTERVAL=1\r\n",
        "RRULE:FREQ=MONTHLY;BYMONTHDAY=-31,31;BYMONTH=2\r\n",
        "RRULE:FREQ=DAILY;UNTIL=20261113\r\n",
        "EXDATE:20261113T090000Z,x\r\n",
        "RRULE:FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=1,-1;COUNT=99999\r\n",
        "RRULE:FREQ=YEARLY;BYMONTH=1,6;BYDAY=SU;BYSETPOS=-366,2,366\r\n",
        "RRULE:FREQ=WEEKLY;BYSETPOS=-1;WKST=SU\r\n",
        "RDATE;VALUE=PERIOD:20261113T090000Z/PT1H,20261114/x\r\n",
        "RDATE;TZID=Z:20261113T100000,20261113T090000Z\r\n",
        "RDATE;VALUE=DATE:20261114\r\n",
        "RECURRENCE-ID:20261113T090000Z\r\n",
        "SUMMARY:a\\,b\\\r\n",
        " folded\r\n",
        "END:VCALENDAR\r\n",
    ];
    for seed in 0..400 {
        let mut rng = Rng::new(seed);
        let mut text = String::new();
        for _ in 0..rng.below(60) {
            match rng.below(4) {
                0 => text.push_str(&hostile(&mut rng)),
                1 => {
                    let bytes: Vec<u8> = (0..rng.below(40)).map(|_| rng.next() as u8).collect();
                    text.push_str(&String::from_utf8_lossy(&bytes));
                }
                _ => text.push_str(rng.pick(&pieces)),
            }
        }
        let from = at(2026, 11, 13, 0, 0) + rng.below(1000) as i64 * DAY - 500 * DAY;
        let zones = Zones { local: Table::fixed(rng.below(50_000) as i64 - 25_000), named: Vec::new() };
        let got = caught(|| expand(&parse(&text), from, from + 2 * DAY, &zones))
            .unwrap_or_else(|p| panic!("seed {seed}: {p}"));
        assert!(got.len() <= MAX_OCCURRENCES);
        assert!(got.iter().all(|o| o.end - o.start <= MAX_DURATION && o.summary.chars().count() <= MAX_VALUE));
    }
}

#[test]
fn a_huge_feed_is_capped() {
    let one = event("SUMMARY:Many\r\nDTSTART:20261113T090000Z\r\n");
    let text = feed(&one.repeat(30_000));
    let feed = parse(&text);
    assert!(feed.events.len() <= MAX_COMPONENTS);
    let from = at(2026, 11, 13, 0, 0);
    assert_eq!(expand(&feed, from, from + DAY, &utc()).len(), MAX_OCCURRENCES);
}

/// A feed is other people's text: none of these may cost more than the scan budget.
#[test]
fn hostile_zones_and_rules_cost_no_more_than_the_budget() {
    let rule_part = "BEGIN:DAYLIGHT\r\nDTSTART:16010101T020000\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0200\r\n\
                     RRULE:FREQ=YEARLY;BYMONTH=1,2,3,4,5,6,7,8,9,10,11,12;BYMONTHDAY=1,2,3\r\nEND:DAYLIGHT\r\n";
    let big_zone = format!("BEGIN:VTIMEZONE\r\nTZID:Big\r\n{}END:VTIMEZONE\r\n", rule_part.repeat(10_000));
    let uses_big = event("SUMMARY:x\r\nDTSTART;TZID=Big:20261113T090000\r\nRRULE:FREQ=DAILY\r\n");
    let parsed = parse(&feed(&format!("{big_zone}{uses_big}")));
    assert_eq!(parsed.zones[0].parts.len(), MAX_PARTS);
    let from = at(2026, 11, 13, 0, 0);
    let (_, work) = expand_counted(&parsed, from, from + 2 * DAY, &utc());
    assert!(work <= 16 * 2 * 366 + 100, "parts capped: {work}");

    // BYMONTH written 5,000 times over: read once each.
    let months = (0..5_000).map(|n| (n % 12 + 1).to_string()).collect::<Vec<_>>().join(",");
    let long = event(&format!("SUMMARY:y\r\nDTSTART:20261113T090000Z\r\nRRULE:FREQ=MONTHLY;BYMONTH={months}\r\n"));
    let rule = parse(&feed(&long)).events[0].rule.clone().unwrap();
    assert!(rule.by_month.len() <= 12 && rule.by_month.windows(2).all(|w| w[0] < w[1]), "{:?}", rule.by_month);

    // 500 zones no event names: none is worked out.
    let unused: String =
        (0..500).map(|n| format!("BEGIN:VTIMEZONE\r\nTZID:Z{n}\r\n{rule_part}END:VTIMEZONE\r\n")).collect();
    let plain = event("SUMMARY:z\r\nDTSTART:20261113T090000Z\r\n");
    let (got, work) = expand_counted(&parse(&feed(&format!("{unused}{plain}"))), from, from + 2 * DAY, &utc());
    assert_eq!((got.len(), work), (1, 0));

    // Thousands of recurring events with long exception lists: the budget holds.
    let exdates =
        (1..=999).map(|n| format!("2025{:02}{:02}T090000Z", n % 12 + 1, n % 28 + 1)).collect::<Vec<_>>().join(",");
    let one = event(&format!(
        "SUMMARY:w\r\nDTSTART:20200101T090000Z\r\nRRULE:FREQ=DAILY;COUNT=99999\r\nEXDATE:{exdates}\r\n"
    ));
    let (_, work) = expand_counted(&parse(&feed(&one.repeat(3_000))), from, from + 2 * DAY, &utc());
    assert!(work <= SCAN_BUDGET + MAX_SCAN + 1_100, "{work}");

    // Set positions over whole years, and long RDATE lists: each year's days are counted.
    let rdates = exdates.replace("2025", "2026");
    let yearly = event(&format!(
        "SUMMARY:v\r\nDTSTART:20000101T090000Z\r\nRRULE:FREQ=YEARLY;BYMONTH=1,2,3,4,5,6,7,8,9,10,11,12;BYSETPOS=-1,1;COUNT=99999\r\nRDATE:{rdates}\r\n"
    ));
    let parsed = parse(&feed(&yearly));
    let (_, one_work) = expand_counted(&parsed, from, from + 2 * DAY, &utc());
    assert!(one_work >= MAX_SCAN, "the years scanned are charged: {one_work}");
    let (_, work) = expand_counted(&parse(&feed(&yearly.repeat(3_000))), from, from + 2 * DAY, &utc());
    assert!(work <= SCAN_BUDGET + MAX_SCAN + 1_500, "{work}");

    // A span of millennia: the years' positions are charged as they are worked out.
    let old =
        event("SUMMARY:o\r\nDTSTART:00010101T090000Z\r\nRRULE:FREQ=YEARLY;BYMONTH=1,6;BYMONTHDAY=1;BYSETPOS=1\r\n");
    let (_, work) = expand_counted(&parse(&feed(&old)), at(1, 1, 1, 0, 0), at(5000, 1, 1, 0, 0), &utc());
    assert!(work <= SCAN_BUDGET + 400, "{work}");
}

#[test]
fn many_rdates_keep_the_earliest_occurrences() {
    // 300 events of 1,000 RDATEs a minute apart, the last event's earliest.
    let rdates = |base: i64| {
        (0..1_000).map(|n| format!("20261113T{:02}{:02}00Z", (base + n) / 60 % 24, (base + n) % 60)).collect::<Vec<_>>()
    };
    let events: String = (0..300)
        .map(|n| {
            let (summary, base) = if n == 299 { ("First", 0) } else { ("Later", 120) };
            let list = rdates(base);
            event(&format!(
                "UID:{n}\r\nSUMMARY:{summary}\r\nDTSTART:20261113T230000Z\r\nRDATE:{}\r\nRDATE:{}\r\n",
                list[..500].join(","),
                list[500..].join(",")
            ))
        })
        .collect();
    let got = expand(&parse(&feed(&events)), at(2026, 11, 13, 0, 0), at(2026, 11, 14, 0, 0), &utc());
    assert_eq!(got.len(), MAX_OCCURRENCES);
    assert!(got[..120].iter().all(|o| o.summary == "First"), "{:?}", got[0].summary);
    assert!(got.windows(2).all(|w| w[0].start <= w[1].start));
}

/// Year, month, day.
type Ymd = (i64, i64, i64);

/// The UTC dates occurrences start on, from one date up to another.
fn dates_in(text: &str, from: Ymd, to: Ymd) -> Vec<Ymd> {
    let (from, to) = (at(from.0, from.1, from.2, 0, 0), at(to.0, to.1, to.2, 0, 0));
    expand(&parse(text), from, to, &utc()).iter().map(|o| civil(o.start.div_euclid(DAY))).collect()
}

/// Dates checked by hand against `cal`.
#[test]
fn set_positions_pick_among_a_periods_days() {
    const WEEKDAYS: &str = "BYDAY=MO,TU,WE,TH,FR";
    let cases: [(&str, &str, &str, Ymd, &[Ymd]); 13] = [
        (
            "first weekday",
            "20261102T090000Z",
            &format!("FREQ=MONTHLY;{WEEKDAYS};BYSETPOS=1"),
            (2027, 3, 1),
            &[(2026, 11, 2), (2026, 12, 1), (2027, 1, 1), (2027, 2, 1)],
        ),
        (
            "last weekday",
            "20261130T090000Z",
            &format!("FREQ=MONTHLY;{WEEKDAYS};BYSETPOS=-1"),
            (2027, 3, 1),
            &[(2026, 11, 30), (2026, 12, 31), (2027, 1, 29), (2027, 2, 26)],
        ),
        (
            "last weekend day",
            "20261129T090000Z",
            "FREQ=MONTHLY;BYDAY=SA,SU;BYSETPOS=-1",
            (2027, 3, 1),
            &[(2026, 11, 29), (2026, 12, 27), (2027, 1, 31), (2027, 2, 28)],
        ),
        (
            "second-to-last weekday",
            "20261127T090000Z",
            &format!("FREQ=MONTHLY;{WEEKDAYS};BYSETPOS=-2"),
            (2027, 3, 1),
            &[(2026, 11, 27), (2026, 12, 30), (2027, 1, 28), (2027, 2, 25)],
        ),
        (
            "first and last weekday",
            "20261102T090000Z",
            &format!("FREQ=MONTHLY;{WEEKDAYS};BYSETPOS=1,-1,1"),
            (2027, 1, 15),
            &[(2026, 11, 2), (2026, 11, 30), (2026, 12, 1), (2026, 12, 31), (2027, 1, 1)],
        ),
        (
            "every other month",
            "20261102T090000Z",
            &format!("FREQ=MONTHLY;INTERVAL=2;{WEEKDAYS};BYSETPOS=1"),
            (2027, 4, 1),
            &[(2026, 11, 2), (2027, 1, 1), (2027, 3, 1)],
        ),
        (
            "count: RFC 5545's third Tuesday, Wednesday or Thursday",
            "19970904T090000Z",
            "FREQ=MONTHLY;COUNT=3;BYDAY=TU,WE,TH;BYSETPOS=3",
            (1998, 1, 1),
            &[(1997, 9, 4), (1997, 10, 7), (1997, 11, 6)],
        ),
        (
            "until",
            "20261130T090000Z",
            &format!("FREQ=MONTHLY;{WEEKDAYS};BYSETPOS=-1;UNTIL=20270115T000000Z"),
            (2027, 3, 1),
            &[(2026, 11, 30), (2026, 12, 31)],
        ),
        (
            "a fifth Monday, where there is one",
            "20261130T090000Z",
            "FREQ=MONTHLY;BYDAY=MO;BYSETPOS=5",
            (2027, 4, 1),
            &[(2026, 11, 30), (2027, 3, 29)],
        ),
        (
            "the last Sunday of March and October together: October's",
            "20261025T090000Z",
            "FREQ=YEARLY;BYMONTH=3,10;BYDAY=SU;BYSETPOS=-1",
            (2028, 1, 1),
            &[(2026, 10, 25), (2027, 10, 31)],
        ),
        (
            "fourth Thursday of November",
            "20261126T090000Z",
            "FREQ=YEARLY;BYMONTH=11;BYDAY=TH;BYSETPOS=4",
            (2028, 1, 1),
            &[(2026, 11, 26), (2027, 11, 25)],
        ),
        (
            "last of Monday, Wednesday and Friday: each week's Friday",
            "20261106T090000Z",
            "FREQ=WEEKLY;BYDAY=MO,WE,FR;BYSETPOS=-1",
            (2026, 11, 16),
            &[(2026, 11, 6), (2026, 11, 13)],
        ),
        (
            "a day's only occurrence is its first and last, never its second",
            "20261101T090000Z",
            "FREQ=DAILY;BYDAY=SU;BYSETPOS=-1,2",
            (2026, 11, 16),
            &[(2026, 11, 1), (2026, 11, 8), (2026, 11, 15)],
        ),
    ];
    for (name, start, rule, to, want) in cases {
        let text = feed(&event(&format!("SUMMARY:{name}\r\nDTSTART:{start}\r\nDURATION:PT1H\r\nRRULE:{rule}\r\n")));
        let first = |range: std::ops::Range<usize>| start[range].parse::<i64>().unwrap();
        assert_eq!(dates_in(&text, (first(0..4), first(4..6), first(6..8)), to), want, "{name}");
    }
}

#[test]
fn an_exdate_on_a_set_position_removes_it_and_moves_nothing() {
    let text = feed(&event(
        "SUMMARY:Last weekday\r\nDTSTART:20261130T090000Z\r\nDURATION:PT1H\r\n\
         RRULE:FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1\r\nEXDATE:20261231T090000Z\r\n",
    ));
    assert_eq!(dates_in(&text, (2026, 11, 1), (2027, 3, 1)), [(2026, 11, 30), (2027, 1, 29), (2027, 2, 26)]);
}

#[test]
fn a_bad_set_position_rejects_the_rule() {
    for bad in ["0", "400", "-367", "x", "1,,2", ""] {
        let text = feed(&event(&format!(
            "SUMMARY:Bad\r\nDTSTART:20261102T090000Z\r\nRRULE:FREQ=MONTHLY;BYDAY=MO;BYSETPOS={bad}\r\n"
        )));
        assert_eq!(parse(&text).events[0].rule, None, "{bad}");
        assert_eq!(dates_in(&text, (2026, 11, 1), (2027, 3, 1)), [(2026, 11, 2)], "{bad}");
    }
    // A long list is read once each, as the other lists are.
    let many = (0..5_000).map(|n| (n % 3 + 1).to_string()).collect::<Vec<_>>().join(",");
    let rule = rule(&format!("FREQ=MONTHLY;BYDAY=MO;BYSETPOS={many}")).unwrap();
    assert_eq!(rule.by_set_pos, [1, 2, 3]);
}

#[test]
fn a_vtimezone_part_may_name_a_set_position() {
    let berlin = BERLIN.replace("BYDAY=-1SU;BYMONTH=10", "BYMONTH=10;BYDAY=SU;BYSETPOS=-1");
    let weekly =
        event("UID:w\r\nSUMMARY:W\r\nDTSTART;TZID=W. Europe Standard Time:20261020T100000\r\nRRULE:FREQ=WEEKLY\r\n");
    let text = feed(&format!("{berlin}{weekly}"));
    assert_eq!(on(&text, 2026, 10, 20, &utc())[0].1, at(2026, 10, 20, 8, 0));
    assert_eq!(on(&text, 2026, 10, 27, &utc())[0].1, at(2026, 10, 27, 9, 0));
}

#[test]
fn rdates_add_occurrences() {
    let zones = Zones { local: Table::fixed(0), named: vec![("Asia/Tokyo".into(), Table::fixed(9 * H))] };
    let text = feed(&event(
        "UID:r\r\nSUMMARY:Extra\r\nDTSTART:20261102T090000Z\r\nDURATION:PT30M\r\n\
         RDATE:20261103T090000Z,20261102T090000Z\r\nRDATE;TZID=Asia/Tokyo:20261104T100000\r\n\
         RDATE;VALUE=PERIOD:20261105T150000Z/PT2H,20261106T150000Z/20261106T160000Z\r\n\
         RDATE;VALUE=DATE:20261107\r\nRDATE:bad,20261108T090000\r\n",
    ));
    assert_eq!(named_zones(&parse(&text)), ["Asia/Tokyo"]);
    let from = at(2026, 11, 1, 0, 0);
    let got: Vec<_> = expand(&parse(&text), from, from + 10 * DAY, &zones).iter().map(|o| (o.start, o.end)).collect();
    // DTSTART once, though an RDATE names it too; a whole day for a timed event is left out.
    let want = [
        at(2026, 11, 2, 9, 0),
        at(2026, 11, 3, 9, 0),
        at(2026, 11, 4, 1, 0),
        at(2026, 11, 5, 15, 0),
        at(2026, 11, 6, 15, 0),
        at(2026, 11, 8, 9, 0),
    ];
    assert_eq!(got, want.map(|s| (s, s + 30 * 60)));
}

#[test]
fn whole_day_rdates_of_a_whole_day_event() {
    let text = feed(&event(
        "SUMMARY:Off\r\nDTSTART;VALUE=DATE:20261113\r\nRDATE;VALUE=DATE:20261120,20261127\r\nRDATE:20261124T090000Z\r\n",
    ));
    let from = at(2026, 11, 1, 0, 0);
    let got = expand(&parse(&text), from, from + 30 * DAY, &utc());
    assert!(got.iter().all(|o| o.all_day && o.end - o.start == DAY));
    let days: Vec<_> = got.iter().map(|o| civil(o.start / DAY)).collect();
    assert_eq!(days, [(2026, 11, 13), (2026, 11, 20), (2026, 11, 27)]);
}

#[test]
fn rdates_meet_a_rule_exdates_and_overrides() {
    let text = feed(&format!(
        "{}{}",
        event(
            "UID:s\r\nSUMMARY:Sync\r\nDTSTART:20261102T090000Z\r\nDURATION:PT30M\r\n\
             RRULE:FREQ=WEEKLY;UNTIL=20261110T000000Z\r\n\
             RDATE:20261109T090000Z,20261111T090000Z,20261112T090000Z,20261113T090000Z,20261120T090000Z\r\n\
             EXDATE:20261111T090000Z\r\n"
        ),
        event(
            "UID:s\r\nRECURRENCE-ID:20261112T090000Z\r\nSUMMARY:Sync (moved)\r\nDTSTART:20261112T140000Z\r\nDURATION:PT30M\r\n"
        ),
    ));
    let from = at(2026, 11, 1, 0, 0);
    let got: Vec<_> =
        expand(&parse(&text), from, from + 30 * DAY, &utc()).into_iter().map(|o| (o.summary, o.start)).collect();
    // The 9th is the rule's and an RDATE: once. The 11th is excluded; the 12th moved. RDATEs pass UNTIL.
    let want = [
        ("Sync", at(2026, 11, 2, 9, 0)),
        ("Sync", at(2026, 11, 9, 9, 0)),
        ("Sync (moved)", at(2026, 11, 12, 14, 0)),
        ("Sync", at(2026, 11, 13, 9, 0)),
        ("Sync", at(2026, 11, 20, 9, 0)),
    ];
    assert_eq!(got, want.map(|(s, t)| (s.to_string(), t)));
    // With no RRULE, an EXDATE may remove DTSTART itself.
    let alone = feed(&event(
        "SUMMARY:Once\r\nDTSTART:20261102T090000Z\r\nRDATE:20261103T090000Z\r\nEXDATE:20261102T090000Z\r\n",
    ));
    assert_eq!(dates_in(&alone, (2026, 11, 1), (2026, 12, 1)), [(2026, 11, 3)]);
}

#[test]
fn rdates_are_capped() {
    let list = |n: i64| (0..n).map(|d| format!("2026{:02}{:02}T090000Z", d % 12 + 1, d % 28 + 1)).collect::<Vec<_>>();
    let one = list(1_500).join(",");
    let text = feed(&event(&format!("SUMMARY:Many\r\nDTSTART:20261102T090000Z\r\nRDATE:{one}\r\nRDATE:{one}\r\n")));
    assert_eq!(parse(&text).events[0].rdates.len(), MAX_RDATES);
}
