//! Fixed clocks and made-up meetings, UTC as local.

use super::*;
use crate::calendar::tests::{DAY, H, M, NINE, UTC, event};
use crate::calendar::{Workday, band};

fn view(events: &[Event], now: u64) -> DayView {
    day_view(events, DAY, band(events, DAY, Workday::default(), UTC), now, UTC)
}

fn block(view: &DayView, id: u64) -> &Block {
    view.blocks.iter().find(|b| b.id == id).expect("a block")
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn a_workday_of_meetings_is_a_point_a_minute_and_fits_a_laptops_sidebar() {
    // 09:00 to 17:00, back to back: 16 half hours.
    let events: Vec<Event> = (0..16).map(|n| event(n, NINE + n * 30 * M, 30)).collect();
    let v = view(&events, NINE);
    assert_eq!((v.from, v.to), (NINE, NINE + 8 * H));
    assert!(near(v.height, 480.), "{}", v.height);
    assert!(v.blocks.iter().all(|b| near(b.height, 30.)), "{:?}", v.blocks);
    // A 14-inch laptop's window leaves its sidebar about 600 points under the header.
    assert!(v.height <= 600.);
    // Heights by duration: an hour is twice half an hour.
    let v = view(&[event(1, NINE, 60), event(2, NINE + H, 30)], NINE);
    assert!(near(block(&v, 1).height, 2. * block(&v, 2).height));
}

#[test]
fn a_short_meeting_keeps_room_for_its_title_and_start_and_pushes_the_next_down() {
    // Standup 09:00–09:15, then a half hour at 09:15.
    let v = view(&[event(1, NINE, 15), event(2, NINE + 15 * M, 30)], NINE);
    let (standup, next) = (block(&v, 1), block(&v, 2));
    assert!(near(standup.top, 0.) && near(standup.height, MIN_BLOCK), "{standup:?}");
    assert!(near(next.top, MIN_BLOCK) && near(next.height, 30.), "{next:?}");
    // A meeting with no length is a block too.
    let v = view(&[event(1, NINE + H, 0)], NINE);
    assert!(near(block(&v, 1).height, MIN_BLOCK));
    assert_eq!(short_clock(NINE, 0), "9:00");
    assert_eq!(short_clock(NINE + 5 * H + 5 * M, 0), "14:05");
    assert_eq!(short_clock(NINE, -10 * 3_600), "23:00");
}

#[test]
fn free_time_over_an_hour_is_a_short_band_that_says_how_long() {
    // 09:00–10:00, two hours free, 12:00–12:30, an hour free, 13:30–14:00, then free to 17:00.
    let events = [event(1, NINE, 60), event(2, NINE + 3 * H, 30), event(3, NINE + 4 * H + 30 * M, 30)];
    let v = view(&events, NINE);
    let gaps: Vec<(u64, u64, f32)> = v.gaps().map(|g| (g.from, g.to, g.height)).collect();
    assert_eq!(gaps, [(NINE + H, NINE + 3 * H, GAP_BAND), (NINE + 5 * H, NINE + 8 * H, GAP_BAND)]);
    // The hour between is not pressed: an hour of empty space.
    let between = v.segments.iter().find(|s| s.from == NINE + 3 * H + 30 * M).unwrap();
    assert!(!between.gap && near(between.height, 60.), "{between:?}");
    // 60 + 24 + 30 + 60 + 30 + 24.
    assert!(near(v.height, 228.), "{}", v.height);
    assert!(near(block(&v, 2).top, 84.));
    // A minute over the hour is pressed.
    let v = view(&[event(1, NINE, 60), event(2, NINE + 2 * H + M, 30)], NINE);
    assert_eq!(v.gaps().next().map(|g| g.to - g.from), Some(61 * M));
    for (ms, said) in [(2 * H, "2 h free"), (90 * M, "1 h 30 m free"), (61 * M, "1 h 1 m free"), (45 * M, "45 m free")]
    {
        assert_eq!(free_label(ms), said);
    }
}

#[test]
fn overlapping_meetings_sit_side_by_side_in_columns() {
    // (meetings, each one's (id, column, columns))
    let table: [(&str, Vec<Event>, Vec<(u64, u8, u8)>); 5] = [
        ("two at once", vec![event(1, NINE, 60), event(2, NINE + 30 * M, 60)], vec![(1, 0, 2), (2, 1, 2)]),
        (
            "three at once",
            vec![event(1, NINE, 60), event(2, NINE + 10 * M, 30), event(3, NINE + 20 * M, 30)],
            vec![(1, 0, 3), (2, 1, 3), (3, 2, 3)],
        ),
        (
            "a chain: each overlaps the next, the first column comes free for the third",
            vec![event(1, NINE, 60), event(2, NINE + 30 * M, 60), event(3, NINE + H, 60)],
            vec![(1, 0, 2), (2, 1, 2), (3, 0, 2)],
        ),
        (
            "one over two that do not overlap: they share a column beside it",
            vec![event(1, NINE, 120), event(2, NINE, 60), event(3, NINE + H, 60)],
            vec![(2, 0, 2), (1, 1, 2), (3, 0, 2)],
        ),
        (
            "back to back is no overlap",
            vec![event(1, NINE, 30), event(2, NINE + 30 * M, 30)],
            vec![(1, 0, 1), (2, 0, 1)],
        ),
    ];
    for (name, events, want) in table {
        let v = view(&events, NINE);
        let got: Vec<(u64, u8, u8)> = v.blocks.iter().map(|b| (b.id, b.column, b.columns)).collect();
        assert_eq!(got, want, "{name}");
    }
    // A block of time held with no call takes a column too, unlike on the band.
    let held = Event { link: None, ..event(2, NINE, 60) };
    let v = view(&[event(1, NINE, 60), held], NINE);
    assert!(v.blocks.iter().all(|b| b.columns == 2), "{:?}", v.blocks);
}

#[test]
fn the_now_line_sits_at_its_time_inside_a_meeting_or_a_pressed_gap() {
    let events = [event(1, NINE, 60), event(2, NINE + 3 * H, 60)];
    let v = view(&events, NINE);
    // A quarter into the first meeting.
    assert_eq!(v.now(NINE + 15 * M), Some(15.));
    // Half way through the two free hours: half way down their band.
    let gap = *v.gaps().next().unwrap();
    assert_eq!(v.now(NINE + 2 * H), Some(gap.top + GAP_BAND / 2.));
    // Between the blocks that end before it and start after it.
    let y = v.now(NINE + 2 * H).unwrap();
    assert!(block(&v, 1).top + block(&v, 1).height <= y && y <= block(&v, 2).top);
    // Off the axis (before 09:00, or another day): none.
    assert_eq!((v.now(NINE - H), v.now(DAY.1 + H)), (None, None));
    // It moves down as time goes.
    let ys: Vec<f32> = (0..=8).filter_map(|h| v.now(NINE + h * H)).collect();
    assert!(ys.windows(2).all(|w| w[1] >= w[0]), "{ys:?}");
}

#[test]
fn hour_lines_mark_whole_hours_but_not_inside_a_pressed_gap() {
    let events = [event(1, NINE + 30 * M, 60), event(2, NINE + 4 * H, 30)];
    let v = view(&events, NINE);
    let hours: Vec<(u8, f32)> = v.hours.iter().map(|h| (h.hour, h.y)).collect();
    // 30 free, the hour's meeting, 10:30–13:00 pressed, the half hour, 13:30–17:00 pressed: 11, 12
    // and 14 to 16 are inside a band; 13:00 and 17:00 are at their ends.
    assert_eq!(hours, [(9, 0.), (10, 60.), (13, 90. + GAP_BAND), (17, 120. + 2. * GAP_BAND)]);
}

#[test]
fn the_day_spans_the_bands_hours_widened_to_every_meeting_out_to_whole_hours() {
    let w = Workday::default();
    // A reminder at 00:00 and a block after hours widen the day, not the band.
    let reminder = Event { link: None, ..event(1, DAY.0, 15) };
    let late = Event { link: None, ..event(2, NINE + 9 * H + 20 * M, 30) };
    let call = event(3, NINE + H, 30);
    let events = [reminder, late, call];
    let b = band(&events, DAY, w, UTC);
    let v = day_view(&events, DAY, b, NINE, UTC);
    assert_eq!((b.from, b.to), (NINE, NINE + 8 * H));
    assert_eq!((v.from, v.to), (DAY.0, NINE + 10 * H));
    // 00:15 to 10:00 is a band, so the day still fits.
    assert_eq!(v.gaps().next().map(|g| (g.from, g.to)), Some((DAY.0 + 15 * M, NINE + H)));
    // The call is where the band has it: the same times, its lane alone.
    let c = block(&v, 3);
    assert_eq!((c.start, c.end, c.column, c.columns), (NINE + H, NINE + H + 30 * M, 0, 1));
    // A meeting from yesterday is cut at midnight; no meetings: the workday, one band.
    let v = view(&[event(1, DAY.0 - H, 120)], NINE);
    assert_eq!((v.from, block(&v, 1).start), (DAY.0, DAY.0));
    let v = view(&[], NINE);
    assert_eq!((v.from, v.to, v.segments.len(), v.height), (NINE, NINE + 8 * H, 1, GAP_BAND));
    // An empty evening: the axis reaches now, one free band, the line in it.
    let v = view(&[], NINE + 11 * H + 30 * M);
    assert_eq!((v.from, v.to, v.gaps().count()), (NINE, NINE + 12 * H, 1));
    assert_eq!(v.now(NINE + 11 * H + 30 * M), Some(GAP_BAND * 11.5 / 12.));
    // Another day's view is not widened to now.
    let other = (DAY.1, DAY.1 + 24 * H);
    let b = band(&[], other, Workday::default(), UTC);
    assert_eq!(day_view(&[], other, b, NINE + 11 * H, UTC).to, DAY.1 + 17 * H);
}

#[test]
fn the_hours_keep_their_clock_on_a_day_the_clocks_change() {
    // Forward an hour at 02:00 local, UTC-5 to UTC-4: midnight is 05:00 UTC, the day 23 hours.
    let midnight = NINE - 4 * H;
    let change = midnight + 2 * H;
    let offset: &dyn Fn(u64) -> i64 = &|t| if t < change { -5 * 3_600 } else { -4 * 3_600 };
    let day = (midnight, midnight + 23 * H);
    let b = band(&[], day, Workday::default(), offset);
    let v = day_view(&[event(1, midnight + 8 * H, 60)], day, b, day.1 + H, offset);
    // 09:00 local is 8 hours after midnight.
    assert_eq!((v.from, v.hours[0].hour, v.hours[1].hour), (midnight + 8 * H, 9, 10));
}

#[test]
fn the_section_opens_with_now_a_third_down_and_a_selection_scrolls_into_view() {
    let events: Vec<Event> = (0..16).map(|n| event(n, NINE + n * 30 * M, 30)).collect();
    let v = view(&events, NINE);
    // 13:00 is 240 points down: a 300-point view opens 140 down, the all-day strip's 20 above it.
    assert!(near(v.opening_scroll(NINE + 4 * H, 300., 20.), 160.));
    // Early: the top; late: no further than the end; another day: the top.
    assert_eq!(v.opening_scroll(NINE, 300., 0.), 0.);
    assert!(near(v.opening_scroll(NINE + 8 * H, 300., 0.), 180.));
    assert_eq!(v.opening_scroll(DAY.1 + H, 300., 0.), 0.);
    // A block above the view scrolls up to it, one below just into it, one shown not at all.
    assert_eq!(reveal(100., 300., (40., 70.)), 40.);
    assert_eq!(reveal(100., 300., (420., 450.)), 150.);
    assert_eq!(reveal(100., 300., (200., 230.)), 100.);
    // Taller than the view: its top.
    assert_eq!(reveal(0., 100., (300., 500.)), 300.);
}
