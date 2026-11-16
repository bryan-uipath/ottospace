//! What the transcript on hears now, for the eye alone: the sound's level, and whether it is
//! words, sound with no words, or nothing. Draw-only: never in the state or the trace, never
//! dispatched, and drawn on a fixed tick, so no reply says when someone speaks
//! (`docs/ai/transcripts.md` §6a).

use std::time::{Duration, Instant};

/// Below this level (RMS of samples in -1..1, about -40 dBFS) is no sound.
const SOUND: f32 = 0.01;
/// Words came this recently: "Hearing speech".
const WORDS: Duration = Duration::from_secs(2);
/// No sound this long: "No sound for 30 s" (a muted call, or the wrong app).
pub(crate) const QUIET: Duration = Duration::from_secs(30);
/// A level older than this is none: the source stopped giving one.
const STALE: Duration = Duration::from_millis(500);

/// What the label says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hearing {
    /// Sound, no words.
    Listening,
    /// Lines arriving.
    Speech,
    /// No sound for `QUIET`.
    Silent,
}

impl Hearing {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Hearing::Listening => "Listening",
            Hearing::Speech => "Hearing speech",
            Hearing::Silent => "No sound for 30 s",
        }
    }

    /// Its control id in a test run's `listen_rects`.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Hearing::Listening => "sound",
            Hearing::Speech => "speech",
            Hearing::Silent => "silent",
        }
    }
}

pub(crate) struct Meter {
    /// The last level and when it came.
    level: (f32, Instant),
    /// Since when it has been on: silence counts from here when there was no sound yet.
    began: Instant,
    sound_at: Option<Instant>,
    words_at: Option<Instant>,
}

impl Meter {
    pub(crate) fn new(now: Instant) -> Meter {
        Meter { level: (0., now), began: now, sound_at: None, words_at: None }
    }

    /// The source's level now, 0 to 1.
    pub(crate) fn level(&mut self, level: f32, now: Instant) {
        let level = if level.is_finite() { level.clamp(0., 1.) } else { 0. };
        self.level = (level, now);
        if level >= SOUND {
            self.sound_at = Some(now);
        }
    }

    /// A line, volatile or final, came.
    pub(crate) fn words(&mut self, now: Instant) {
        self.words_at = Some(now);
    }

    /// A resume: the silence counts from now.
    pub(crate) fn restart(&mut self, now: Instant) {
        *self = Meter::new(now);
    }

    /// The level to draw, 0 to 1: louder sounds stand out (a square root of the RMS).
    pub(crate) fn shown(&self, now: Instant) -> f32 {
        let (level, at) = self.level;
        if now.saturating_duration_since(at) > STALE { 0. } else { level.sqrt().min(1.) }
    }

    pub(crate) fn hearing(&self, now: Instant) -> Hearing {
        if self.words_at.is_some_and(|at| now.saturating_duration_since(at) < WORDS) {
            return Hearing::Speech;
        }
        let since = self.sound_at.unwrap_or(self.began);
        if now.saturating_duration_since(since) >= QUIET { Hearing::Silent } else { Hearing::Listening }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_then_sound_then_nothing() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_millis(s);
        let mut m = Meter::new(t0);
        assert_eq!(m.hearing(at(1_000)), Hearing::Listening, "quiet, not yet for long");
        m.level(0.2, at(1_000));
        m.words(at(1_000));
        assert_eq!(m.hearing(at(2_000)), Hearing::Speech);
        assert!((m.shown(at(1_100)) - 0.2f32.sqrt()).abs() < 1e-6);
        assert_eq!(m.shown(at(2_000)), 0., "a level not given again goes");
        assert_eq!(m.hearing(at(3_500)), Hearing::Listening, "sound, words gone quiet");
        m.level(0.001, at(10_000));
        assert_eq!(m.hearing(at(30_999)), Hearing::Listening);
        assert_eq!(m.hearing(at(31_000)), Hearing::Silent, "30 s since the last sound");
        m.restart(at(40_000));
        assert_eq!(m.hearing(at(41_000)), Hearing::Listening);
        assert_eq!(m.hearing(at(70_000)), Hearing::Silent, "never any sound: from the start");
        m.level(f32::NAN, at(70_000));
        assert_eq!(m.shown(at(70_000)), 0.);
    }
}
