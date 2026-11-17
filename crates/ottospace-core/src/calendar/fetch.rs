//! How the feed's fetch goes: loading (with bytes so far), read (with its count of events), or
//! failed (with why, in plain words). Pure: the shell passes the times. Counts, codes and kinds
//! only: never the link or a title, so it may be in the state.

const SECOND: u64 = 1_000;
const MINUTE: u64 = 60 * SECOND;
/// The first retry after a failed read (ms); each failure in a row doubles it.
const RETRY: u64 = 15 * SECOND;

/// When to fetch again after `failures` in a row (ms): 15 s, 30 s, 1 min.., at most `every`, the
/// refresh interval. A first read that fails is not left for the whole interval.
pub fn retry_after(failures: u32, every: u64) -> u64 {
    match failures {
        0 => every,
        n => RETRY.saturating_mul(1 << (n - 1).min(16)).min(every),
    }
}

/// Why a fetch failed. Never the link, never curl's own message (it may quote the link).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// curl's `connect-timeout` or `max-time` passed (exit 28), after `secs`.
    TimedOut { secs: u32 },
    /// The host was not found or did not answer (a lookup that failed, curl's exit 6 or 7).
    Unreachable,
    /// The server answered with this status (400 and up).
    Http(u16),
    /// A redirect (3xx): not followed.
    Moved,
    /// The host's addresses are not all public.
    NotPublic,
    /// Over `ics::MAX_BYTES`.
    TooBig,
    /// Read, but no `BEGIN:VCALENDAR`: not an iCalendar feed (a sign-in page, say).
    NotCalendar,
    /// The feed's file could not be opened or read.
    File,
    /// curl failed otherwise: its exit code.
    Curl(i32),
}

impl Failure {
    /// What the Calendar section says.
    pub fn words(self) -> String {
        match self {
            Failure::TimedOut { secs } => format!("Couldn't reach the calendar (timed out after {secs} s)"),
            Failure::Unreachable => "Couldn't reach the calendar's host".into(),
            Failure::Http(code) => format!("The calendar answered {code}"),
            Failure::Moved => "The calendar feed redirects: put its final https address in calendar_ics".into(),
            Failure::NotPublic => "The calendar's host is not on the internet".into(),
            Failure::TooBig => format!("Couldn't read the calendar: over {} MB", crate::ics::MAX_BYTES >> 20),
            Failure::NotCalendar => "Couldn't read the calendar: not an iCalendar feed (no BEGIN:VCALENDAR)".into(),
            Failure::File => "Couldn't open the calendar file".into(),
            Failure::Curl(code) => format!("Couldn't reach the calendar (curl error {code})"),
        }
    }

    /// Its name in the state and the app's log.
    pub fn kind(self) -> &'static str {
        match self {
            Failure::TimedOut { .. } => "timed_out",
            Failure::Unreachable => "unreachable",
            Failure::Http(_) => "http",
            Failure::Moved => "moved",
            Failure::NotPublic => "not_public",
            Failure::TooBig => "too_big",
            Failure::NotCalendar => "not_calendar",
            Failure::File => "file",
            Failure::Curl(_) => "curl",
        }
    }

    /// From curl's exit code and the status it wrote (`write-out`), after `secs`. `None`: no failure.
    pub fn of_curl(exit: Option<i32>, http: Option<u16>, secs: u32) -> Option<Failure> {
        match (exit, http) {
            (_, Some(300..=399)) => Some(Failure::Moved),
            (_, Some(code @ 400..)) => Some(Failure::Http(code)),
            (Some(0), Some(200..=299)) => None,
            (Some(28), _) => Some(Failure::TimedOut { secs }),
            (Some(6 | 7), _) => Some(Failure::Unreachable),
            (Some(63), _) => Some(Failure::TooBig),
            (Some(code), _) => Some(Failure::Curl(code)),
            (None, _) => Some(Failure::Curl(-1)),
        }
    }
}

/// The feed's fetch, as last heard from the shell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fetch {
    /// No word from the feed thread yet: it starts at once.
    #[default]
    Idle,
    /// A fetch runs since `since` (ms), `bytes` received so far.
    Loading { since: u64, bytes: u64 },
    /// The last fetch was read at `at`: `events` VEVENTs in it.
    Read { at: u64, events: u32 },
    /// The last fetch failed at `at`.
    Failed { at: u64, why: Failure },
}

impl Fetch {
    pub fn name(self) -> &'static str {
        match self {
            Fetch::Idle => "idle",
            Fetch::Loading { .. } => "loading",
            Fetch::Read { .. } => "read",
            Fetch::Failed { .. } => "failed",
        }
    }

    /// The Calendar section's muted line: `Loading the calendar… 412 KB` (or `… 3 s` before a
    /// byte), `Updated 2 min ago · 1858 events`, or what failed and, when one was, the last good
    /// read: `… · last read 09:40` (`last_read` already put as a clock).
    pub fn line(self, now: u64, last_read: Option<String>) -> String {
        match self {
            Fetch::Idle => "Loading the calendar…".into(),
            Fetch::Loading { bytes: 0, since } => match now.saturating_sub(since) / SECOND {
                0 => "Loading the calendar…".into(),
                secs => format!("Loading the calendar… {secs} s"),
            },
            Fetch::Loading { bytes, .. } => format!("Loading the calendar… {}", size(bytes)),
            Fetch::Read { at, events } => {
                let ago = match now.saturating_sub(at) / MINUTE {
                    0 => "just now".to_string(),
                    m if m < 60 => format!("{m} min ago"),
                    m => format!("{} h ago", m / 60),
                };
                let s = if events == 1 { "" } else { "s" };
                format!("Updated {ago} · {events} event{s}")
            }
            Fetch::Failed { why, .. } => match last_read {
                Some(clock) => format!("{} · last read {clock}", why.words()),
                None => why.words(),
            },
        }
    }
}

/// `412 KB`, `1.5 MB`.
fn size(bytes: u64) -> String {
    match bytes {
        b if b < 1 << 20 => format!("{} KB", b.div_ceil(1 << 10)),
        b => format!("{}.{} MB", b >> 20, (b % (1 << 20)) * 10 >> 20),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_read_is_tried_again_soon_then_less_often() {
        // A failed read waited the whole refresh interval (5 minutes by default).
        let every = 5 * MINUTE;
        let waits: Vec<u64> = (0..7).map(|n| retry_after(n, every)).collect();
        assert_eq!(waits, [every, 15 * SECOND, 30 * SECOND, MINUTE, 2 * MINUTE, 4 * MINUTE, every]);
        assert_eq!(retry_after(u32::MAX, every), every);
        assert_eq!(retry_after(1, 10 * SECOND), 10 * SECOND, "never later than a short interval");
    }

    #[test]
    fn curl_says_what_failed() {
        let of = |exit, http| Failure::of_curl(Some(exit), http, 30);
        assert_eq!(of(0, Some(200)), None);
        assert_eq!(of(28, Some(0)), Some(Failure::TimedOut { secs: 30 }), "the time it took");
        assert_eq!(of(22, Some(403)), Some(Failure::Http(403)));
        assert_eq!(of(0, Some(302)), Some(Failure::Moved));
        assert_eq!(of(6, Some(0)), Some(Failure::Unreachable));
        assert_eq!(of(63, None), Some(Failure::TooBig));
        assert_eq!(of(35, Some(0)), Some(Failure::Curl(35)));
        assert_eq!(Failure::of_curl(None, None, 30), Some(Failure::Curl(-1)), "killed");
    }

    #[test]
    fn each_state_says_itself_in_plain_words() {
        let t = 1_000_000;
        assert_eq!(Fetch::Idle.line(t, None), "Loading the calendar…");
        assert_eq!(Fetch::Loading { since: t, bytes: 0 }.line(t + 3_400, None), "Loading the calendar… 3 s");
        assert_eq!(Fetch::Loading { since: t, bytes: 421_000 }.line(t, None), "Loading the calendar… 412 KB");
        assert_eq!(Fetch::Loading { since: t, bytes: 1_557_023 }.line(t, None), "Loading the calendar… 1.4 MB");
        assert_eq!(
            Fetch::Read { at: t, events: 1858 }.line(t + 2 * MINUTE + 5, None),
            "Updated 2 min ago · 1858 events"
        );
        assert_eq!(Fetch::Read { at: t, events: 1 }.line(t, None), "Updated just now · 1 event");
        let timed_out = Fetch::Failed { at: t, why: Failure::TimedOut { secs: 30 } };
        assert_eq!(timed_out.line(t, None), "Couldn't reach the calendar (timed out after 30 s)");
        let forbidden = Fetch::Failed { at: t, why: Failure::Http(403) };
        assert_eq!(forbidden.line(t, Some("09:40".into())), "The calendar answered 403 · last read 09:40");
        let bad = Fetch::Failed { at: t, why: Failure::NotCalendar };
        assert!(bad.line(t, None).starts_with("Couldn't read the calendar: not an iCalendar feed"));
    }
}
