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

impl Extent {
    /// The furthest offset: 0 when the content fits.
    pub fn max_offset(&self) -> f32 {
        (self.content - self.viewport).max(0.)
    }

    /// The thumb's start and length on a track `track` long. `None`: the content fits, so no bar.
    pub fn thumb(&self, track: f32) -> Option<(f32, f32)> {
        let max = self.max_offset();
        if max < 0.5 || track <= 0. {
            return None; // half a point of overflow is rounding, not content
        }
        let length = (track * self.viewport / self.content).max(MIN_THUMB).min(track);
        Some(((track - length) * (self.offset / max).clamp(0., 1.), length))
    }

    /// The offset that puts the thumb's start at `start`: a drag of the thumb.
    pub fn dragged(&self, track: f32, start: f32) -> f32 {
        match self.thumb(track) {
            Some((_, length)) if track > length => (start / (track - length)).clamp(0., 1.) * self.max_offset(),
            _ => 0.,
        }
    }

    /// The offset after a click on the track at `at`: one viewport toward the click. A click on
    /// the thumb scrolls nothing.
    pub fn paged(&self, track: f32, at: f32) -> f32 {
        let Some((start, length)) = self.thumb(track) else { return 0. };
        let pages = if at < start {
            -1.
        } else if at >= start + length {
            1.
        } else {
            0.
        };
        (self.offset + pages * self.viewport).clamp(0., self.max_offset())
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
            ((100., 400., 10., 5.), 0.),
        ];
        for ((viewport, content, track, start), offset) in cases {
            assert_eq!(extent(viewport, content, 0.).dragged(track, start), offset, "{viewport} {content} {start}");
        }
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
