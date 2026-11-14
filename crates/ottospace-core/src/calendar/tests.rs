//! Fixed clocks and made-up events.

use super::*;

const M: u64 = MINUTE;
const H: u64 = 60 * M;
/// 09:00 of a day, in ms.
const NINE: u64 = 1_794_560_400_000;

fn event(id: u64, start: u64, minutes: u64) -> Event {
    Event {
        id,
        start,
        end: start + minutes * M,
        all_day: false,
        title: format!("Meeting {id}"),
        location: String::new(),
        link: Some("https://example.zoom.us/j/123".into()),
    }
}

fn calendar(events: Vec<Event>) -> Calendar {
    let mut c = Calendar { on: true, notify: true, ..Calendar::default() };
    c.read(Some(events), [NINE - 9 * H, NINE + 15 * H, NINE + 39 * H], NINE - H);
    c
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

#[test]
fn a_block_enters_from_the_right_crosses_and_leaves_at_the_left() {
    let window = 8 * H;
    let e = [event(1, NINE, 60)];
    // Beyond the window: nothing.
    assert!(bars(&e, NINE - 8 * H - 1, window).is_empty());
    // Entering: its start at the right edge, cut there.
    let entering = &bars(&e, NINE - 8 * H + 30 * M, window)[0];
    assert!((entering.from - (450. / 480.)).abs() < 1e-6 && entering.to == 1.);
    // Crossing: its width is its length.
    let crossing = &bars(&e, NINE - 4 * H, window)[0];
    assert!((crossing.from - 0.5).abs() < 1e-6 && (crossing.to - 0.5 - 1. / 8.).abs() < 1e-6);
    assert_eq!(crossing.stage, Stage::Far);
    // Running: from the left edge, as wide as the time left.
    let running = &bars(&e, NINE + 15 * M, window)[0];
    assert_eq!((running.from, running.stage), (0., Stage::Running));
    assert!((running.to - 45. / 480.).abs() < 1e-6);
    // Ended: gone.
    assert!(bars(&e, NINE + 60 * M, window).is_empty());
}

#[test]
fn an_event_longer_than_the_window_fills_it() {
    let b = &bars(&[event(1, NINE, 12 * 60)], NINE + H, 8 * H)[0];
    assert_eq!((b.from, b.to, b.lane), (0., 1., Lane::Full));
}

#[test]
fn overlapping_events_share_two_half_lanes() {
    let events =
        [event(1, NINE, 60), event(2, NINE + 30 * M, 60), event(3, NINE + 3 * H, 30), event(4, NINE + 45 * M, 10)];
    let lanes: Vec<(u64, Lane)> = bars(&events, NINE - H, 8 * H).iter().map(|b| (b.id, b.lane)).collect();
    assert_eq!(lanes, [(1, Lane::Top), (2, Lane::Bottom), (4, Lane::Top), (3, Lane::Full)]);
    // All-day events are no blocks.
    let all_day = Event { all_day: true, ..event(5, NINE, 24 * 60) };
    assert!(bars(&[all_day], NINE, 8 * H).is_empty());
}

#[test]
fn hour_ticks_slide_with_the_clock() {
    // 09:15 UTC, a zone 1 hour ahead: 10:15 local. The first tick is 11:00, 45 minutes in.
    let ticks = ticks(NINE + 15 * M, 8 * H, 3_600);
    assert_eq!(ticks.len(), 8);
    assert_eq!(ticks[0].1, 11);
    assert!((ticks[0].0 - 45. / 480.).abs() < 1e-6);
    assert_eq!(ticks[7].1, 18);
    // A tick at the right edge itself counts; past midnight hours start again at 0.
    assert_eq!(super::ticks(NINE + 14 * H, 3 * H, 0).iter().map(|t| t.1).collect::<Vec<_>>(), [0, 1, 2]);
}

#[test]
fn the_chip_and_the_status_line() {
    let c = calendar(vec![event(1, NINE, 30), event(2, NINE + 2 * H, 60)]);
    assert_eq!(c.chip(NINE - 12 * M), Some(("Meeting 1 · in 12m".into(), Stage::Far)));
    assert_eq!(c.chip(NINE - 8 * M), Some(("Meeting 1 · in 8m".into(), Stage::Approaching)));
    assert_eq!(c.chip(NINE - 3 * H).map(|c| c.0), Some("Meeting 1 · in 3h".into()));
    assert_eq!(c.chip(NINE + 5 * M), Some(("Meeting 1 · now".into(), Stage::Running)));
    // One coming near is named over one that runs.
    let back_to_back = calendar(vec![event(1, NINE, 30), event(2, NINE + 30 * M, 30)]);
    assert_eq!(back_to_back.chip(NINE + 25 * M).map(|c| c.0), Some("Meeting 2 · in 5m".into()));
    assert_eq!(c.chip(NINE + 40 * M).map(|c| c.0), Some("Meeting 2 · in 1h 20m".into()));
    // Nothing left today: no chip. Tomorrow's events do not count.
    let tomorrow = calendar(vec![event(1, NINE + 20 * H, 30)]);
    assert_eq!(tomorrow.chip(NINE), None);
    assert_eq!(c.chip(NINE + 4 * H), None);
    // The status bar from five minutes before.
    assert_eq!(c.soon(NINE - 5 * M - 1), None);
    assert_eq!(c.soon(NINE - 5 * M), Some("Meeting 1 in 5m".into()));
    assert_eq!(c.soon(NINE - 3 * M - 30_000), Some("Meeting 1 in 4m".into()));
    assert_eq!(c.soon(NINE), None);
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
    c.call_seen(Some(true));
    c.call_seen(Some(false));
    assert!(c.tick(NINE + 3 * M).is_empty());
    assert_eq!(c.banner(NINE + 3 * M), None);
    // In another call at the start, out of it soon after: this meeting is still missed.
    let mut other = calendar(vec![event(3, NINE, 60)]);
    other.call_seen(Some(true));
    assert!(other.tick(NINE).is_empty());
    other.call_seen(Some(false));
    assert_eq!(other.tick(NINE + 3 * M).len(), 1);
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
    let answers = [feed_answer("200"), feed_answer("302"), feed_answer("404"), feed_answer("")];
    assert_eq!(answers, [Answer::Read, Answer::Moved, Answer::Failed, Answer::Failed]);
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
fn clock_times_and_the_banner_line() {
    assert_eq!(clock(NINE, 0), "09:00");
    assert_eq!(clock(NINE + 5 * M, -5 * 3_600), "04:05");
    assert_eq!(clock(NINE, 16 * 3_600), "01:00");
    let e = Event { location: "Room 4".into(), ..event(1, NINE, 30) };
    assert_eq!(describe(&e, 0), "Meeting 1\n09:00–09:30 · Room 4");
    let mut c = calendar(vec![e]);
    c.in_call = Some(false);
    c.tick(NINE + 30_000);
    assert_eq!(c.banner_line(NINE + 3 * M + 1), Some((1, "Meeting 1 · started 3m ago".into(), true)));
}

#[test]
fn spans_round_up_to_the_minute() {
    assert_eq!(span(1), "1m");
    assert_eq!(span(12 * M), "12m");
    assert_eq!(span(60 * M), "1h");
    assert_eq!(span(80 * M + 1), "1h 21m");
}

#[test]
fn the_agenda_lists_all_day_first_then_by_start_then_a_folded_tomorrow() {
    use crate::list::ListRow::{Meeting, MeetingsTomorrow};
    let all_day = Event { all_day: true, ..event(1, NINE - 9 * H, 24 * 60) };
    let tomorrow = [event(4, NINE + 24 * H, 30), Event { all_day: true, ..event(5, NINE + 15 * H, 24 * 60) }];
    let mut c = calendar(vec![
        event(2, NINE, 30),
        event(3, NINE - 2 * H, 30),
        all_day,
        tomorrow[0].clone(),
        tomorrow[1].clone(),
    ]);
    assert_eq!(c.agenda(), [Meeting(1), Meeting(3), Meeting(2), MeetingsTomorrow]);
    c.tomorrow_open = true;
    assert_eq!(c.agenda(), [Meeting(1), Meeting(3), Meeting(2), MeetingsTomorrow, Meeting(5), Meeting(4)]);
    // No tomorrow: no group. Nothing at all: no rows.
    assert_eq!(calendar(vec![event(2, NINE, 30)]).agenda(), [Meeting(2)]);
    assert!(calendar(Vec::new()).agenda().is_empty());
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
    m.update(Action::SetCalendar { on: true, notify: true, links_only: false }.sent_by(Origin::System));
    assert_eq!(m.sections()[..2], [Section::Tasks, Section::Calendar]);
    m.update(Action::ToggleSection { section: Section::Calendar });
    assert_eq!(m.sidebar, Some(Section::Calendar));
    // The feed goes (a later start without it): the section closes.
    m.update(Action::SetCalendar { on: false, notify: true, links_only: false }.sent_by(Origin::System));
    assert_eq!(m.sidebar, None);
}

#[test]
fn enter_on_a_meeting_joins_it_for_the_user_only() {
    use crate::model::{Action, Effect, Model, Origin, Section};
    let mut m = Model::default();
    m.update(Action::SetCalendar { on: true, notify: true, links_only: false }.sent_by(Origin::System));
    let days = [NINE - 9 * H, NINE + 15 * H, NINE + 39 * H];
    m.update(Action::CalendarRead { events: Some(vec![event(7, NINE, 30)]), days, at: NINE }.sent_by(Origin::System));
    m.update(Action::ToggleSection { section: Section::Calendar });
    m.update(Action::FocusList);
    assert_eq!(m.list, Some(0));
    let client = m.update(Action::ListChoose { axis: crate::Axis::Right }.sent_by(Origin::Socket));
    assert!(client.is_empty(), "a client's ↵ opens nothing");
    let user = m.update(Action::ListChoose { axis: crate::Axis::Right });
    assert_eq!(user, [Effect::OpenLink { url: "https://example.zoom.us/j/123".into() }]);
}
