//! Test support for the property tests: a seeded generator, hostile strings, a shrinker. No
//! crate for it: the core depends on std and serde only, and this is 100 lines.

use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

/// splitmix64: the same numbers for the same seed, on every machine.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// 0 up to, not including, `n`. `n` 0: 0.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    /// True one time in `n`.
    pub fn one_in(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }

    pub fn coin(&mut self) -> bool {
        self.one_in(2)
    }

    pub fn pick<T: Clone>(&mut self, from: &[T]) -> T {
        from[self.below(from.len())].clone()
    }

    /// `some()` half the time.
    pub fn maybe<T>(&mut self, some: impl FnOnce(&mut Self) -> T) -> Option<T> {
        self.coin().then(|| some(self))
    }

    /// Up to `most` of `one()`.
    pub fn some<T>(&mut self, most: usize, mut one: impl FnMut(&mut Self) -> T) -> Vec<T> {
        (0..self.below(most + 1)).map(|_| one(self)).collect()
    }
}

/// Pieces of text that break code which counts bytes, splits at spaces or trusts a name.
pub const HOSTILE: &[&str] = &[
    "👨‍👩‍👧‍👦",
    "🏳️‍🌈",
    "e\u{301}",
    "a\u{300}\u{301}\u{302}",
    "שלום",
    "مرحبا",
    "\u{202e}abc",
    "\0",
    "\u{1b}[31m",
    "\r\n",
    "\t",
    "\u{7f}",
    " ",
    "  ",
    "\\ ",
    "\\",
    "/",
    "~/",
    "/src/export,",
    "../..",
    "#x",
    ">y",
    "@z",
    "/p",
    "$s",
    "p1",
    "#",
    ">",
    "@",
    "$",
    "#123",
    "ß",
    "İ",
    "ǅ",
    "\u{feff}",
    "\u{200b}",
    "a",
    "S",
    "q",
    "\"",
    "'",
    ".",
    ",",
];

/// A hostile string: empty one time in 8, very long one in 16, else a few pieces.
pub fn hostile(rng: &mut Rng) -> String {
    match rng.below(16) {
        0 | 1 => String::new(),
        2 => rng.pick(HOSTILE).repeat(600 + rng.below(600)),
        _ => (0..1 + rng.below(4)).map(|_| rng.pick(HOSTILE)).collect(),
    }
}

/// `run`'s result, or the message of its panic. Nothing is printed for the panic: a shrink
/// makes hundreds.
pub fn caught<R>(run: impl FnOnce() -> R) -> Result<R, String> {
    thread_local!(static QUIET: Cell<bool> = const { Cell::new(false) });
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let before = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if !QUIET.with(Cell::get) {
                before(info);
            }
        }));
    });
    QUIET.with(|quiet| quiet.set(true));
    let result = panic::catch_unwind(AssertUnwindSafe(run));
    QUIET.with(|quiet| quiet.set(false));
    result.map_err(|panic| {
        let text =
            panic.downcast_ref::<String>().cloned().or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()));
        format!("panicked: {}", text.unwrap_or_default())
    })
}

/// The steps of `steps` that still fail: runs of them are dropped, long runs first, while
/// `fails` goes on saying so. Not the least possible, but every step left is needed.
pub fn shrink<T: Clone>(mut steps: Vec<T>, fails: impl Fn(&[T]) -> bool) -> Vec<T> {
    let mut run = (steps.len() / 2).max(1);
    loop {
        let (mut at, mut dropped) = (0, false);
        while at < steps.len() {
            let end = (at + run).min(steps.len());
            let without = [&steps[..at], &steps[end..]].concat();
            if fails(&without) {
                (steps, dropped) = (without, true);
            } else {
                at = end;
            }
        }
        if run == 1 && !dropped {
            return steps;
        }
        run = (run / 2).max(1);
    }
}

/// How many sequences run at once.
pub fn workers() -> usize {
    std::thread::available_parallelism().map_or(1, usize::from)
}

/// Runs `one(seed)` for `count` seeds from `first`, on every core. The failure of the lowest
/// seed, if any failed.
pub fn each_seed<F: Send>(first: u64, count: u64, one: impl Fn(u64) -> Option<F> + Sync) -> Option<F> {
    let workers = workers() as u64;
    let failures: Vec<(u64, F)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|worker| {
                let one = &one;
                scope.spawn(move || {
                    let seeds = (0..count).filter(|n| n % workers == worker).map(|n| first + n);
                    seeds.filter_map(|seed| Some((seed, one(seed)?))).next()
                })
            })
            .collect();
        handles.into_iter().filter_map(|handle| handle.join().ok().flatten()).collect()
    });
    failures.into_iter().min_by_key(|(seed, _)| *seed).map(|(_, failure)| failure)
}
