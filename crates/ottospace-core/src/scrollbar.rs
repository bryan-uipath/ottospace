//! The scrollbar's rules: where the thumb is, what a drag or a track click scrolls to, and how
//! opaque an idle bar is. Lengths are points along the scroll axis; an offset is how far the
//! content is scrolled from its start.

/// Thumb thickness at rest. apollo-ui future-dark: `::-webkit-scrollbar { width: 6px }`.
pub const THUMB: f32 = 6.;
/// Thumb thickness under the pointer, and the track's: what a press on the bar can hit.
pub const TRACK: f32 = 8.;
/// Gap between the track's ends and the area's. apollo-ui `ScrollBar`: `p-px`.
pub const INSET: f32 = 1.;
/// The thumb never gets shorter: a long list still leaves something to grab.
pub const MIN_THUMB: f32 = 24.;
/// An overlay bar stays this long after the last scroll or hover, then fades for `FADE_MS`.
pub const IDLE_MS: u64 = 1000;
pub const FADE_MS: u64 = 200;

/// A scroll area along one axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extent {
    pub viewport: f32,
    pub content: f32,
    pub offset: f32,
}

/// A step through a scroll area from the keyboard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// Down (positive) or up.
    Lines(i32),
    Pages(i32),
    Top,
    Bottom,
}

/// Keys that scroll a list that takes no other keys (the key reference): `j`/`k` or arrows a
/// line, space or Page Down / Page Up a page, Home / End (`g` / ⇧G) the ends.
pub fn key_step(key: &str, shift: bool) -> Option<Step> {
    Some(match key {
        "j" | "down" => Step::Lines(1),
        "k" | "up" => Step::Lines(-1),
        "space" if shift => Step::Pages(-1),
        "space" | "pagedown" => Step::Pages(1),
        "pageup" => Step::Pages(-1),
        "g" if shift => Step::Bottom,
        "home" | "g" => Step::Top,
        "end" => Step::Bottom,
        _ => return None,
    })
}

impl Extent {
    /// The furthest offset: 0 when the content fits, or when a size is not a number.
    pub fn max_offset(&self) -> f32 {
        let max = self.content - self.viewport;
        if max.is_finite() { max.max(0.) } else { 0. }
    }

    /// The offset, kept within the content. Not a number: the start.
    fn at(&self) -> f32 {
        if self.offset.is_finite() { self.offset.clamp(0., self.max_offset()) } else { 0. }
    }

    /// The thumb's start and length on a track `track` long. `None`: the content fits (no bar),
    /// or there is nothing to measure.
    pub fn thumb(&self, track: f32) -> Option<(f32, f32)> {
        let max = self.max_offset();
        // Half a point of overflow is rounding, not content.
        if max < 0.5 || self.viewport.is_nan() || self.viewport <= 0. || !track.is_finite() || track <= 0. {
            return None;
        }
        let length = (track * self.viewport / self.content).max(MIN_THUMB).min(track);
        Some(((track - length) * (self.at() / max), length))
    }

    /// The offset that puts the thumb's start at `start`: a drag of the thumb. A thumb that
    /// fills its track has nowhere to go: the offset stays.
    pub fn dragged(&self, track: f32, start: f32) -> f32 {
        match self.thumb(track) {
            Some((_, length)) if track > length && !start.is_nan() => {
                (start / (track - length)).clamp(0., 1.) * self.max_offset()
            }
            _ => self.at(),
        }
    }

    /// The offset after a click on the track at `at`: one viewport toward the click. A click on
    /// the thumb scrolls nothing.
    pub fn paged(&self, track: f32, at: f32) -> f32 {
        let Some((start, length)) = self.thumb(track) else { return self.at() };
        let pages = if at < start {
            -1.
        } else if at >= start + length {
            1.
        } else {
            0.
        };
        (self.at() + pages * self.viewport).clamp(0., self.max_offset())
    }

    /// The offset after a step from the keyboard. `line`: how far one line goes.
    pub fn stepped(&self, step: Step, line: f32) -> f32 {
        let to = match step {
            Step::Lines(lines) => self.at() + lines as f32 * line,
            Step::Pages(pages) => self.at() + pages as f32 * self.viewport,
            Step::Top => 0.,
            Step::Bottom => self.max_offset(),
        };
        if to.is_finite() { to.clamp(0., self.max_offset()) } else { self.at() }
    }
}

/// How opaque the bar is. `always`: the system shows scroll bars always. `engaged`: the pointer
/// is over the area, or holds the thumb. `idle_ms`: time since the last scroll or since the
/// pointer left; `None` if there was neither.
pub fn opacity(always: bool, engaged: bool, idle_ms: Option<u64>) -> f32 {
    match idle_ms {
        _ if always || engaged => 1.,
        Some(ms) if ms <= IDLE_MS => 1.,
        Some(ms) if ms < IDLE_MS + FADE_MS => 1. - (ms - IDLE_MS) as f32 / FADE_MS as f32,
        _ => 0.,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuzz::Rng;

    fn extent(viewport: f32, content: f32, offset: f32) -> Extent {
        Extent { viewport, content, offset }
    }

    #[test]
    fn the_thumb_shows_how_much_is_in_view_and_where() {
        // (viewport, content, offset, track) -> thumb (start, length)
        let cases = [
            ((100., 400., 0., 100.), Some((0., 25.))),
            ((100., 400., 150., 100.), Some((37.5, 25.))),
            ((100., 400., 300., 100.), Some((75., 25.))),
            // A track shorter than the viewport (inset): the same proportion of it.
            ((200., 400., 200., 198.), Some((99., 99.))),
            // A long list: the thumb stops shrinking, and still reaches both ends.
            ((100., 10_000., 0., 100.), Some((0., MIN_THUMB))),
            ((100., 10_000., 9_900., 100.), Some((100. - MIN_THUMB, MIN_THUMB))),
            // An offset out of range (the content shrank) stays on the track.
            ((100., 400., 900., 100.), Some((75., 25.))),
            ((100., 400., -20., 100.), Some((0., 25.))),
            // A track shorter than the least thumb: the thumb is the track.
            ((100., 400., 150., 10.), Some((0., 10.))),
            // Content that fits has no bar.
            ((100., 100., 0., 100.), None),
            ((100., 100.4, 0., 100.), None),
            ((100., 40., 0., 100.), None),
            ((100., 400., 0., 0.), None),
        ];
        for ((viewport, content, offset, track), thumb) in cases {
            assert_eq!(extent(viewport, content, offset).thumb(track), thumb, "{viewport} {content} {offset} {track}");
        }
    }

    #[test]
    fn a_drag_scrolls_in_proportion_and_stops_at_the_ends() {
        // (viewport, content, track, thumb start) -> offset
        let cases = [
            ((100., 400., 100., 0.), 0.),
            ((100., 400., 100., 37.5), 150.),
            ((100., 400., 100., 75.), 300.),
            ((100., 400., 100., 500.), 300.),
            ((100., 400., 100., -30.), 0.),
            // The least thumb leaves 76 points of travel for 9900 of content.
            ((100., 10_000., 100., 38.), 4950.),
            ((100., 100., 100., 40.), 0.),
        ];
        for ((viewport, content, track, start), offset) in cases {
            assert_eq!(extent(viewport, content, 0.).dragged(track, start), offset, "{viewport} {content} {start}");
        }
        // A thumb that fills its track (a one-row terminal) cannot move: a held move scrolls nothing.
        assert_eq!(extent(100., 400., 150.).dragged(10., 5.), 150.);
        assert_eq!(extent(100., 400., 900.).dragged(24., -3.), 300.);
        // Where the thumb is drawn is where a drag to that place scrolls.
        let at = extent(120., 1000., 333.);
        let (start, _) = at.thumb(118.).unwrap();
        assert!((at.dragged(118., start) - 333.).abs() < 0.01);
    }

    #[test]
    fn a_track_click_pages_toward_the_click() {
        // (offset, click) -> offset; viewport 100, content 400, track 100: the thumb is 25 long.
        let cases = [
            ((0., 90.), 100.),
            ((100., 90.), 200.),
            ((250., 99.), 300.), // the last page is a short one
            ((300., 10.), 200.),
            ((50., 0.), 0.),
            ((150., 40.), 150.), // on the thumb (37.5 to 62.5)
            ((150., 62.5), 250.),
        ];
        for ((offset, click), paged) in cases {
            assert_eq!(extent(100., 400., offset).paged(100., click), paged, "{offset} {click}");
        }
        assert_eq!(extent(100., 80., 0.).paged(100., 90.), 0.);
    }

    #[test]
    fn keys_step_a_line_a_page_or_to_an_end() {
        let keys = [
            ("j", false, Some(Step::Lines(1))),
            ("up", false, Some(Step::Lines(-1))),
            ("space", false, Some(Step::Pages(1))),
            ("space", true, Some(Step::Pages(-1))),
            ("pageup", false, Some(Step::Pages(-1))),
            ("pagedown", false, Some(Step::Pages(1))),
            ("home", false, Some(Step::Top)),
            ("g", false, Some(Step::Top)),
            ("g", true, Some(Step::Bottom)),
            ("end", false, Some(Step::Bottom)),
            ("escape", false, None),
        ];
        for (key, shift, step) in keys {
            assert_eq!(key_step(key, shift), step, "{key} {shift}");
        }
        // (offset, step) -> offset; viewport 100, content 400, a line is 20.
        let cases = [
            ((0., Step::Lines(1)), 20.),
            ((0., Step::Lines(-1)), 0.),
            ((290., Step::Lines(1)), 300.),
            ((50., Step::Pages(1)), 150.),
            ((50., Step::Pages(-1)), 0.),
            ((50., Step::Lines(i32::MAX)), 300.),
            ((50., Step::Bottom), 300.),
            ((50., Step::Top), 0.),
        ];
        for ((offset, step), stepped) in cases {
            assert_eq!(extent(100., 400., offset).stepped(step, 20.), stepped, "{offset} {step:?}");
        }
        assert_eq!(extent(100., 80., 0.).stepped(Step::Bottom, 20.), 0.);
    }

    /// One random area. `Some`: what does not hold for it.
    fn broken(seed: u64) -> Option<String> {
        let mut rng = Rng::new(seed);
        let mut size = |most: f32| rng.below(1_000_000) as f32 / 1_000_000. * most;
        let (viewport, track) = (1. + size(2000.), 1. + size(2000.));
        let content = viewport + size(3_000_000.);
        let area = extent(viewport, content, size(content - viewport));
        let (max, d) = (area.max_offset(), size(track));
        let Some((start, length)) = area.thumb(track) else {
            return (max >= 0.5).then(|| format!("{area:?}: content that does not fit has no thumb"));
        };
        let at = |offset: f32| extent(viewport, content, offset).thumb(track).unwrap();
        let (top, bottom) = (at(0.), at(max));
        let there = extent(viewport, content, area.dragged(track, start + d));
        let back = there.dragged(track, there.thumb(track).unwrap().0 - d);
        let (up, down) = (area.paged(track, start - 1.), area.paged(track, start + length));
        // An offset in the millions is only exact to a fraction of a point.
        let (here, page) = (area.offset, viewport + max * 1e-6 + 0.01);
        let fail = if !(start.is_finite() && start >= 0. && start + length <= track + 0.01) {
            "the thumb is off the track"
        } else if length < MIN_THUMB.min(track) {
            "the thumb is shorter than the least"
        } else if top.0 != 0. || (bottom.0 + bottom.1 - track).abs() > 0.01 {
            "the thumb is not at the track's ends at the content's"
        } else if !(0. ..=max).contains(&area.dragged(track, start + d - track / 2.)) {
            "a drag leaves the content"
        } else if area.dragged(track, start) > area.dragged(track, start + d) {
            "a drag further down scrolls further up"
        } else if start + d <= track - length && (back - here).abs() > max * 1e-3 + 0.5 {
            "a drag there and back does not return"
        } else if !(up <= here && here - up <= page && down >= here && down - here <= page) {
            "a page is not toward the click, or more than a viewport"
        } else {
            return None;
        };
        Some(format!("seed {seed}: {fail}: {area:?}, track {track}, thumb {start} + {length}, d {d}"))
    }

    /// Seeded: the same areas on every machine. `OTTOSPACE_PROP_SEED=<n>` runs one again.
    #[test]
    fn any_area_keeps_its_thumb_on_the_track_and_its_offsets_in_range() {
        let (first, count) = match std::env::var("OTTOSPACE_PROP_SEED").ok().and_then(|seed| seed.parse().ok()) {
            Some(seed) => (seed, 1),
            None => (0, 20_000),
        };
        assert_eq!((first..first + count).find_map(broken), None);
    }

    #[test]
    fn sizes_that_are_no_sizes_never_give_an_offset_that_is_no_number() {
        let odd = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1., 0., 1e-9, 0.4, 30., 1e30, f32::MAX];
        let steps = [Step::Lines(i32::MIN), Step::Lines(3), Step::Pages(i32::MAX), Step::Top, Step::Bottom];
        let mut rng = Rng::new(7);
        for _ in 0..50_000 {
            let mut any = || rng.pick(&odd);
            let (area, track, at) = (extent(any(), any(), any()), any(), any());
            if let Some((start, length)) = area.thumb(track) {
                assert!(start.is_finite() && length.is_finite() && start >= 0. && length > 0., "{area:?} {track}");
            }
            let step = rng.pick(&steps);
            for offset in [area.dragged(track, at), area.paged(track, at), area.stepped(step, at)] {
                assert!(offset.is_finite() && offset >= 0., "{area:?} track {track} at {at}: {offset}");
            }
        }
    }

    #[test]
    fn an_overlay_bar_fades_after_a_while_and_an_always_bar_never_does() {
        // (always, engaged, idle) -> opacity
        let cases = [
            ((false, false, None), 0.),
            ((false, true, None), 1.),
            ((false, false, Some(0)), 1.),
            ((false, false, Some(IDLE_MS)), 1.),
            ((false, false, Some(IDLE_MS + FADE_MS / 2)), 0.5),
            ((false, false, Some(IDLE_MS + FADE_MS)), 0.),
            ((false, false, Some(u64::MAX)), 0.),
            ((false, true, Some(u64::MAX)), 1.),
            ((true, false, None), 1.),
            ((true, false, Some(u64::MAX)), 1.),
        ];
        for ((always, engaged, idle), expected) in cases {
            assert_eq!(opacity(always, engaged, idle), expected, "{always} {engaged} {idle:?}");
        }
    }
}
