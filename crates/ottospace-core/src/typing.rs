//! A timed run of typing in a terminal (Developer ▸ "Time Typing in This Terminal"): each key's
//! way to the screen, by stage, summed up. The shell stamps the stages; the numbers are made here.

/// One key's stages, each in µs from its key event: its bytes sent to the pty, its echo read and
/// parsed, the main thread taking that output (and asking for a frame), a frame painting it.
/// `None`: the stage never came before the next key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Key {
    pub write: Option<u64>,
    pub echo: Option<u64>,
    pub taken: Option<u64>,
    pub painted: Option<u64>,
}

/// The app's own share of a key's latency must stay under one 120 Hz frame (ms): its p99, from
/// key event to pty write plus from echo parsed to frame asked for.
pub const APP_BUDGET_MS: f32 = 8.3;

/// A stage's times over the run (ms): nearest-rank percentiles.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct Dist {
    pub n: usize,
    pub p50: f32,
    pub p95: f32,
    pub p99: f32,
    pub max: f32,
}

impl Dist {
    fn of(mut us: Vec<u64>) -> Self {
        us.sort_unstable();
        let n = us.len();
        let at = |p: usize| if n == 0 { 0. } else { us[(n * p).div_ceil(100).clamp(1, n) - 1] as f32 / 1e3 };
        Dist { n, p50: at(50), p95: at(95), p99: at(99), max: at(100) }
    }
}

/// A run's numbers, as the toast, the event log and a test run's state give them: times only.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct Typed {
    /// Keys timed, and those whose echo no frame painted before the next key.
    pub keys: usize,
    pub missed: usize,
    /// Key event to bytes written to the pty.
    pub key_write: Dist,
    /// Written to echo read and parsed: the pty, the program, the reader.
    pub write_echo: Dist,
    /// Parsed to the main thread taking it and asking for a frame.
    pub echo_taken: Dist,
    /// Asked for to painted: the wait for the display's next frame, then the frame's own work.
    pub taken_painted: Dist,
    /// The app's own share: `key_write` plus `echo_taken`, per key.
    pub app: Dist,
    /// Key event to painted.
    pub total: Dist,
}

/// Sums up a run's keys.
pub fn summary(keys: &[Key]) -> Typed {
    let each = |f: &dyn Fn(&Key) -> Option<u64>| Dist::of(keys.iter().filter_map(f).collect());
    let gap = |a: fn(&Key) -> Option<u64>, b: fn(&Key) -> Option<u64>| {
        each(&move |k: &Key| Some(b(k)?.saturating_sub(a(k)?)))
    };
    Typed {
        keys: keys.len(),
        missed: keys.iter().filter(|k| k.painted.is_none()).count(),
        key_write: each(&|k| k.write),
        write_echo: gap(|k| k.write, |k| k.echo),
        echo_taken: gap(|k| k.echo, |k| k.taken),
        taken_painted: gap(|k| k.taken, |k| k.painted),
        app: each(&|k| Some(k.write? + k.taken?.saturating_sub(k.echo?))),
        total: each(&|k| k.painted),
    }
}

/// The made-up character typed as the `i`th key: letters and digits, never a control key.
pub fn key_char(i: usize) -> char {
    const KEYS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    KEYS[i % KEYS.len()] as char
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(write: u64, echo: u64, taken: u64, painted: Option<u64>) -> Key {
        Key { write: Some(write), echo: Some(echo), taken: Some(taken), painted }
    }

    /// Each stage is the time from the stage before; the app's share is its two own stages.
    #[test]
    fn a_run_is_summed_up_by_stage() {
        let keys = [key(100, 1_100, 9_100, Some(15_000)), key(200, 700, 1_700, Some(5_000))];
        let t = summary(&keys);
        assert_eq!((t.keys, t.missed), (2, 0));
        assert_eq!((t.key_write.p50, t.key_write.max), (0.1, 0.2));
        assert_eq!((t.write_echo.p50, t.write_echo.max), (0.5, 1.0));
        assert_eq!((t.echo_taken.p50, t.echo_taken.max), (1.0, 8.0));
        assert_eq!((t.taken_painted.p50, t.taken_painted.max), (3.3, 5.9));
        // 0.2 + 1.0 and 0.1 + 8.0.
        assert_eq!((t.app.p50, t.app.p99), (1.2, 8.1));
        assert_eq!((t.total.p50, t.total.max), (5.0, 15.0));
    }

    /// A key no frame painted is missed: left out of the times it lacks, counted.
    #[test]
    fn a_key_never_painted_is_missed() {
        let keys = [key(100, 1_000, 2_000, None), Key::default(), key(100, 1_000, 2_000, Some(9_000))];
        let t = summary(&keys);
        assert_eq!((t.keys, t.missed), (3, 2));
        assert_eq!((t.total.n, t.app.n, t.taken_painted.n), (1, 2, 1));
    }

    #[test]
    fn percentiles_are_nearest_rank() {
        let d = Dist::of((1..=100).map(|ms| ms * 1_000).collect());
        assert_eq!((d.n, d.p50, d.p95, d.p99, d.max), (100, 50., 95., 99., 100.));
        assert_eq!(Dist::of(Vec::new()), Dist::default());
    }

    #[test]
    fn keys_are_made_up_letters_and_digits() {
        assert!((0..100).map(key_char).all(|c| c.is_ascii_alphanumeric()));
        assert_eq!((key_char(0), key_char(36)), ('a', 'a'));
    }
}
