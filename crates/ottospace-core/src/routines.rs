//! Routines: what the ottobot daemon last said, and what the Routines section makes of it.
//! The shell talks to the daemon; grouping, order and text are decided here. Times are
//! milliseconds since 1970, passed in. Design: `docs/ai/routines.md`.

use std::path::{Component, Path};

use serde::Serialize;

const DAY_MS: u64 = 86_400_000;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Running,
    Succeeded,
    /// Why, in the daemon's words.
    Failed(String),
}

/// What a run said about what it found. A run that said nothing is unassessed (`None`), never normal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Normal,
    Critical,
    Immediate,
}

/// One firing of a routine.
#[derive(Clone, Debug, PartialEq)]
pub struct RunLine {
    pub id: String,
    pub routine: String,
    pub status: Status,
    pub started: u64,
    pub finished: Option<u64>,
    /// `runner/model@effort`, as much as is known.
    pub agent: String,
    /// The verdict's headline, else the first line of the run's last message.
    pub line: Option<String>,
    pub report: Option<String>,
    pub level: Option<Level>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Routine {
    pub name: String,
    pub description: Option<String>,
    /// e.g. `cron 0 8 * * 1-5`, or why the trigger could not be read.
    pub trigger: Result<String, String>,
    pub next_fire: Option<u64>,
    pub last_fired: Option<u64>,
    pub active_run: Option<String>,
    /// The scheduler's note, e.g. why the last fire was skipped.
    pub note: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    pub routine: String,
    pub key: String,
    pub level: Level,
    pub headline: String,
    pub summary: String,
    pub report: Option<String>,
    pub run: String,
    pub first_seen: u64,
}

/// One rate-limit window of a runner, e.g. `5h`.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub label: String,
    pub left_percent: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Quota {
    pub runner: String,
    /// The runner's version, or why it cannot run.
    pub health: Result<String, String>,
    /// Or why the quota could not be read.
    pub windows: Result<Vec<Window>, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Daemon {
    /// No socket, nothing listening, or no answer in time.
    #[default]
    Absent,
    /// Something answered, but not with a snapshot this build can read.
    Incompatible { reason: String },
    Up {
        pid: Option<u32>,
        /// The folder its routine files are in.
        root: String,
        /// The daemon's build. Empty from an old one.
        commit: String,
    },
}

/// The daemon's last snapshot. The shell's poll replaces it whole.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Routines {
    pub daemon: Daemon,
    /// Reports are under `<store_dir>/reports`.
    pub store_dir: String,
    pub list: Vec<Routine>,
    /// Newest first, as the daemon sends them.
    pub runs: Vec<RunLine>,
    pub alerts: Vec<Alert>,
    pub quota: Vec<Quota>,
    /// Routine files that could not be read: (path, why).
    pub errors: Vec<(String, String)>,
    /// Routine → the failed run the user dismissed.
    pub dismissed_failures: Vec<(String, String)>,
}

/// A heading of the section, in the order shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    /// No heading: the one row that says there is no usable daemon.
    Daemon,
    Alerts,
    Now,
    Attention,
    Next,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::Daemon => "",
            Group::Alerts => "Alerts",
            Group::Now => "Now",
            Group::Attention => "Attention",
            Group::Next => "Next",
        }
    }
}

/// How loud a row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Plain,
    Running,
    Warning,
    Urgent,
}

/// A row of the section.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Row {
    pub group: Group,
    /// The same from one snapshot to the next, so the list selection can follow its row.
    pub key: String,
    pub title: String,
    /// Short, at the right: an age, or the time to the next fire.
    pub badge: String,
    pub lines: Vec<String>,
    pub tone: Tone,
    /// What `r` runs, ↵ follows and `o` opens.
    pub routine: Option<String>,
    pub run: Option<String>,
    pub report: Option<String>,
}

impl Routines {
    /// Per routine, the latest finished run, if it failed within 24 hours and was not dismissed.
    /// ottobot's rule, restated: its daemon leaves this to the client.
    pub fn failures(&self, now: u64) -> Vec<&RunLine> {
        let mut seen: Vec<&str> = Vec::new();
        let latest = self.runs.iter().filter(|run| {
            let first = run.status != Status::Running && !seen.contains(&run.routine.as_str());
            if first {
                seen.push(&run.routine);
            }
            first
        });
        let recent =
            latest.filter(|run| matches!(run.status, Status::Failed(_)) && run.started >= now.saturating_sub(DAY_MS));
        recent
            .filter(|run| !self.dismissed_failures.iter().any(|(routine, id)| *routine == run.routine && *id == run.id))
            .collect()
    }

    /// Alerts and failures: what the rail's badge counts.
    pub fn needs_you(&self, now: u64) -> usize {
        match self.daemon {
            Daemon::Up { .. } => self.alerts.len() + self.failures(now).len(),
            _ => 0,
        }
    }

    /// The newest run of a routine.
    fn last_run(&self, routine: &str) -> Option<&RunLine> {
        self.runs.iter().find(|run| run.routine == routine)
    }

    /// Why `name` cannot be run now, if it cannot. A running routine is refused here: the
    /// daemon would reply `Ok` and skip it.
    pub fn can_run(&self, name: &str) -> Result<(), String> {
        match &self.daemon {
            Daemon::Absent => return Err("ottobot daemon not running".into()),
            Daemon::Incompatible { .. } => return Err("incompatible ottobot daemon".into()),
            Daemon::Up { .. } => {}
        }
        match self.list.iter().find(|routine| routine.name == name) {
            None => Err(format!("no routine {}", plain(name))),
            Some(routine) if routine.active_run.is_some() => Err(format!("{} is already running", plain(name))),
            Some(_) => Ok(()),
        }
    }

    /// One line about the daemon, for the top of the section: `ottobot · pid 4242 · 8bbc374`.
    pub fn daemon_line(&self) -> Option<String> {
        let Daemon::Up { pid, commit, .. } = &self.daemon else { return None };
        let pid = pid.map(|pid| format!(" · pid {pid}")).unwrap_or_default();
        let commit = if commit.is_empty() { String::new() } else { format!(" · {}", plain(commit)) };
        Some(format!("ottobot{pid}{commit}"))
    }

    /// What to say when the daemon is up and has nothing to list.
    pub fn hint(&self) -> Option<String> {
        let Daemon::Up { root, .. } = &self.daemon else { return None };
        let empty = self.list.is_empty() && self.runs.is_empty() && self.errors.is_empty() && self.alerts.is_empty();
        empty.then(|| format!("No routines. Add a skill file with `trigger:` under {}.", plain(root)))
    }

    /// One line a runner: `claude · 5h 63% left · 7d 93% left`. Empty without a usable daemon.
    pub fn quota_lines(&self) -> Vec<(String, Tone)> {
        if !matches!(self.daemon, Daemon::Up { .. }) {
            return Vec::new();
        }
        let line = |quota: &Quota| {
            let runner = plain(&quota.runner);
            match (&quota.health, &quota.windows) {
                (Err(why), _) => (format!("{runner} · not usable: {}", plain(why)), Tone::Warning),
                (_, Err(why)) => (format!("{runner} · quota unknown: {}", plain(why)), Tone::Plain),
                (_, Ok(windows)) if windows.is_empty() => (format!("{runner} · no usage data"), Tone::Plain),
                (_, Ok(windows)) => {
                    let left = |w: &Window| format!("{} {}% left", plain(&w.label), w.left_percent);
                    let tone = if windows.iter().any(|w| w.left_percent < 10) { Tone::Warning } else { Tone::Plain };
                    (format!("{runner} · {}", windows.iter().map(left).collect::<Vec<_>>().join(" · ")), tone)
                }
            }
        };
        self.quota.iter().map(line).collect()
    }

    /// The section, top to bottom: alerts (worst first, then oldest), running runs, failures and
    /// files that did not parse, then routines by next fire (none: last). Without a usable
    /// daemon: one row that says so.
    pub fn rows(&self, now: u64) -> Vec<Row> {
        let row = |group, key: String, title: &str| Row {
            group,
            key,
            title: plain(title),
            badge: String::new(),
            lines: Vec::new(),
            tone: Tone::Plain,
            routine: None,
            run: None,
            report: None,
        };
        match &self.daemon {
            Daemon::Absent => {
                let lines = vec!["Start it: otto daemon --install".to_string()];
                return vec![Row { lines, ..row(Group::Daemon, "daemon".into(), "ottobot daemon not running") }];
            }
            Daemon::Incompatible { reason } => {
                let lines = vec![plain(reason), "Update ottobot or ottospace: they speak different protocols.".into()];
                return vec![Row { lines, ..row(Group::Daemon, "daemon".into(), "Incompatible ottobot daemon") }];
            }
            Daemon::Up { .. } => {}
        }
        let ago = |then: u64| span(now.saturating_sub(then));
        let mut rows = Vec::new();

        let mut alerts: Vec<&Alert> = self.alerts.iter().collect();
        alerts.sort_by(|a, b| b.level.cmp(&a.level).then(a.first_seen.cmp(&b.first_seen)));
        rows.extend(alerts.into_iter().map(|alert| Row {
            badge: ago(alert.first_seen),
            lines: vec![plain(if alert.headline.is_empty() { &alert.summary } else { &alert.headline })],
            tone: if alert.level == Level::Immediate { Tone::Urgent } else { Tone::Warning },
            routine: Some(alert.routine.clone()),
            run: Some(alert.run.clone()),
            report: alert.report.clone(),
            ..row(Group::Alerts, format!("alert:{}/{}", alert.routine, alert.key), &alert.routine)
        }));

        rows.extend(self.runs.iter().filter(|run| run.status == Status::Running).map(|run| Row {
            badge: ago(run.started),
            lines: vec![plain(&run.agent)],
            tone: Tone::Running,
            routine: Some(run.routine.clone()),
            run: Some(run.id.clone()),
            ..row(Group::Now, format!("run:{}", run.id), &run.routine)
        }));

        rows.extend(self.failures(now).into_iter().map(|run| {
            let why = if let Status::Failed(why) = &run.status { why.as_str() } else { "" };
            Row {
                badge: format!("{} ago", ago(run.finished.unwrap_or(run.started))),
                lines: vec![plain(&format!("failed: {why}"))],
                tone: Tone::Warning,
                routine: Some(run.routine.clone()),
                run: Some(run.id.clone()),
                report: run.report.clone(),
                ..row(Group::Attention, format!("failed:{}", run.id), &run.routine)
            }
        }));
        rows.extend(self.errors.iter().map(|(path, why)| {
            let file = path.rsplit('/').next().unwrap_or(path);
            Row { lines: vec![plain(why)], tone: Tone::Warning, ..row(Group::Attention, format!("error:{path}"), file) }
        }));

        let mut next: Vec<&Routine> = self.list.iter().collect();
        next.sort_by_key(|routine| (routine.next_fire.is_none(), routine.next_fire));
        rows.extend(next.into_iter().map(|routine| {
            let last = self.last_run(&routine.name);
            let mut lines = vec![match &routine.trigger {
                Ok(trigger) => plain(trigger),
                Err(why) => plain(&format!("trigger not understood: {why}")),
            }];
            lines.push(match last {
                Some(run) => last_line(run, now),
                None => "never ran".into(),
            });
            lines.extend(Some(plain(&routine.note)).filter(|note| !note.is_empty()));
            Row {
                badge: match (&routine.active_run, routine.next_fire) {
                    (Some(_), _) => "running".into(),
                    (None, Some(at)) if at <= now => "due".into(),
                    (None, Some(at)) => format!("in {}", span(at - now)),
                    (None, None) => "—".into(),
                },
                lines,
                tone: match (&routine.active_run, &routine.trigger) {
                    (Some(_), _) => Tone::Running,
                    (None, Err(_)) => Tone::Warning,
                    _ => Tone::Plain,
                },
                routine: Some(routine.name.clone()),
                run: routine.active_run.clone().or(last.map(|run| run.id.clone())),
                report: last.and_then(|run| run.report.clone()),
                ..row(Group::Next, format!("routine:{}", routine.name), &routine.name)
            }
        }));
        rows
    }
}

/// A routine's last result and when: `last ok · 2h ago · All quiet`.
fn last_line(run: &RunLine, now: u64) -> String {
    let when = span(now.saturating_sub(run.finished.unwrap_or(run.started)));
    let result = match (&run.status, run.level) {
        (Status::Running, _) => return plain(&format!("running · {}", run.agent)),
        (Status::Failed(why), _) => return plain(&format!("last failed · {when} ago · {why}")),
        (Status::Succeeded, Some(Level::Normal)) => "ok",
        (Status::Succeeded, Some(Level::Critical)) => "critical",
        (Status::Succeeded, Some(Level::Immediate)) => "immediate",
        // No verdict: the run did not say. Never shown as ok.
        (Status::Succeeded, None) => "unassessed",
    };
    let line = run.line.as_deref().map(|line| format!(" · {line}")).unwrap_or_default();
    plain(&format!("last {result} · {when} ago{line}"))
}

/// A length of time, short: `42s`, `5m`, `3h`, `4d`.
fn span(ms: u64) -> String {
    match ms / 1000 {
        s @ 0..60 => format!("{s}s"),
        s @ 60..3600 => format!("{}m", s / 60),
        s @ 3600..172_800 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

/// Text from a run or the daemon, as one plain line: control characters (a newline, a terminal
/// escape) become spaces, and it is cut at 200 characters. Such text is data, never markup.
pub fn plain(text: &str) -> String {
    let mut line: String = text.chars().take(200).map(|c| if c.is_control() { ' ' } else { c }).collect();
    if text.chars().nth(200).is_some() {
        line.push('…');
    }
    line.trim().to_string()
}

/// True if a run id can be put on a command line as it is: `20260101-120000-nightly`.
/// The id comes from a socket reply, and following a run types it into a shell.
pub fn safe_id(id: &str) -> bool {
    let word = id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    word && !id.is_empty() && id.len() <= 128 && !id.starts_with(['-', '.'])
}

/// True if `path` may be opened as a run's report: a `.html` or `.md` file under
/// `<store_dir>/reports`. Both must be canonical already (the shell resolves them): the path
/// comes from a socket reply, and a report is an agent's own file.
pub fn report_allowed(path: &str, store_dir: &str) -> bool {
    let (path, reports) = (Path::new(path), Path::new(store_dir).join("reports"));
    let plain = |p: &Path| p.is_absolute() && p.components().all(|c| !matches!(c, Component::ParentDir));
    let kind = path.extension().and_then(|ext| ext.to_str());
    let inside = path.strip_prefix(&reports).is_ok_and(|rest| rest.components().next().is_some());
    plain(path) && plain(&reports) && inside && matches!(kind, Some("html" | "md"))
}

/// A run's output as lines that are safe to print in a terminal: control characters dropped (a
/// run's text could carry terminal escapes), then credentials masked.
pub fn output_lines(text: &str) -> Vec<String> {
    let clean = |line: &str| line.chars().filter(|c| !c.is_control() || *c == '\t').collect::<String>();
    text.lines().map(|line| redact(&clean(line))).collect()
}

/// A line of a run's output with credential-looking words masked: known token prefixes, the word
/// after `Bearer`/`token`/`authorization`, and `key=`/`secret=`/`password=`/`token=` values.
/// ottobot's rule for the stderr it keeps; its `Events` reply carries the raw lines.
pub fn redact(line: &str) -> String {
    const PREFIXES: [&str; 9] = ["sk-", "ghp_", "gho_", "ghs_", "ghu_", "github_pat_", "xox", "AKIA", "eyJ"];
    const KEYS: [&str; 5] = ["key=", "secret=", "password=", "token=", "authorization="];
    const MASK: &str = "<redacted>";
    let mut after_marker = false;
    let mut mask = |word: &str| {
        let lower = word.to_ascii_lowercase();
        let bare = word.trim_matches(|c: char| !c.is_alphanumeric());
        let token_like = word.len() >= 16 && PREFIXES.iter().any(|prefix| bare.starts_with(prefix));
        let masked = if (after_marker && word.len() >= 8) || token_like {
            MASK.to_string()
        } else if let Some(value) = KEYS.iter().find_map(|key| lower.find(key).map(|at| at + key.len())) {
            format!("{}{MASK}", &word[..value])
        } else {
            word.to_string()
        };
        after_marker = matches!(lower.trim_end_matches(':'), "bearer" | "token" | "authorization");
        masked
    };
    line.split(' ').map(&mut mask).collect::<Vec<_>>().join(" ")
}

/// An RFC 3339 time in Unix milliseconds: `2026-11-12T10:44:00.5-08:00` → 1794509040500.
/// ottobot writes local times with their offset.
pub fn parse_time(text: &str) -> Option<u64> {
    let (date, rest) = text.split_once(['T', 't', ' '])?;
    // The offset: `Z`, or `+hh:mm` / `-hh:mm` after the clock.
    let (clock, offset_minutes) = match rest.strip_suffix(['Z', 'z']) {
        Some(clock) => (clock, 0),
        None => {
            let at = rest.rfind(['+', '-'])?;
            let (hours, minutes) = rest.get(at + 1..)?.split_once(':')?;
            let offset = hours.parse::<i64>().ok()? * 60 + minutes.parse::<i64>().ok()?;
            (rest.get(..at)?, if rest[at..].starts_with('-') { -offset } else { offset })
        }
    };
    let number = |part: &str| part.bytes().all(|b| b.is_ascii_digit()).then(|| part.parse::<i64>().ok()).flatten();
    let mut date = date.split('-').map(number);
    let (year, month, day) = (date.next()??, date.next()??, date.next()??);
    let (clock, fraction) = clock.split_once('.').unwrap_or((clock, "0"));
    let mut clock = clock.split(':').map(number);
    let (hour, minute, second) = (clock.next()??, clock.next()??, clock.next()??);
    let in_range = (1970..=9999).contains(&year) && (1..=12).contains(&month) && (1..=31).contains(&day);
    if !in_range || hour > 23 || minute > 59 || second > 60 || offset_minutes.abs() > 24 * 60 {
        return None;
    }
    let millis = number(format!("{fraction:0<3}").get(..3)?)?;
    // Days since 1970-01-01, by civil-date arithmetic with years starting in March.
    let y = if month <= 2 { year - 1 } else { year };
    let (era, year_of_era) = (y / 400, y % 400);
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let local = (((days * 24 + hour) * 60 + minute) * 60 + second) * 1000 + millis;
    u64::try_from(local - offset_minutes * 60_000).ok()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const HOUR: u64 = 3_600_000;
    const NOW: u64 = 100 * HOUR;

    pub(crate) fn run(id: &str, routine: &str, status: Status, hours_ago: u64) -> RunLine {
        RunLine {
            id: id.into(),
            routine: routine.into(),
            status,
            started: NOW - hours_ago * HOUR,
            finished: None,
            agent: "claude/opus".into(),
            line: None,
            report: None,
            level: None,
        }
    }

    pub(crate) fn routine(name: &str, next_fire: Option<u64>) -> Routine {
        Routine {
            name: name.into(),
            description: None,
            trigger: Ok("cron 0 8 * * 1-5".into()),
            next_fire,
            last_fired: None,
            active_run: None,
            note: String::new(),
        }
    }

    fn alert(routine: &str, level: Level, hours_ago: u64) -> Alert {
        Alert {
            routine: routine.into(),
            key: "k".into(),
            level,
            headline: format!("{routine} is down"),
            summary: String::new(),
            report: None,
            run: format!("{routine}-1"),
            first_seen: NOW - hours_ago * HOUR,
        }
    }

    pub(crate) fn up() -> Routines {
        let daemon = Daemon::Up { pid: Some(7), root: "/skills".into(), commit: "8bbc374".into() };
        Routines { daemon, store_dir: "/store".into(), ..Routines::default() }
    }

    fn failed() -> Status {
        Status::Failed("exit 1".into())
    }

    #[test]
    fn rows_are_alerts_then_now_then_attention_then_next() {
        let routines = Routines {
            list: vec![
                routine("later", Some(NOW + 2 * HOUR)),
                routine("never", None),
                routine("soon", Some(NOW + 60_000)),
            ],
            runs: vec![run("r2", "soon", Status::Running, 0), run("r1", "later", failed(), 2)],
            alerts: vec![
                alert("new-critical", Level::Critical, 1),
                alert("immediate", Level::Immediate, 1),
                alert("old-critical", Level::Critical, 5),
            ],
            errors: vec![("/skills/broken.md".into(), "no trigger".into())],
            ..up()
        };
        let rows = routines.rows(NOW);
        let listed: Vec<(Group, &str)> = rows.iter().map(|row| (row.group, row.title.as_str())).collect();
        assert_eq!(
            listed,
            [
                (Group::Alerts, "immediate"),
                (Group::Alerts, "old-critical"),
                (Group::Alerts, "new-critical"),
                (Group::Now, "soon"),
                (Group::Attention, "later"),
                (Group::Attention, "broken.md"),
                (Group::Next, "soon"),
                (Group::Next, "later"),
                (Group::Next, "never"),
            ]
        );
        assert_eq!(
            (rows[0].tone, rows[0].badge.as_str(), rows[0].lines[0].as_str()),
            (Tone::Urgent, "1h", "immediate is down")
        );
        assert_eq!((rows[4].badge.as_str(), rows[4].lines[0].as_str()), ("2h ago", "failed: exit 1"));
        let badges: Vec<&str> = rows[6..].iter().map(|row| row.badge.as_str()).collect();
        assert_eq!(badges, ["in 1m", "in 2h", "—"]);
        // A row names what its keys act on: the alert's own run, a routine's latest run.
        assert_eq!((rows[0].routine.as_deref(), rows[0].run.as_deref()), (Some("immediate"), Some("immediate-1")));
        assert_eq!((rows[7].run.as_deref(), rows[8].run.as_deref()), (Some("r1"), None));
        assert_eq!(routines.needs_you(NOW), 4);
        assert_eq!(routines.daemon_line().as_deref(), Some("ottobot · pid 7 · 8bbc374"));
    }

    #[test]
    fn failures_are_current_and_undismissed() {
        // Newest first: a fixed after a failure; b failed twice; c failed 2 days ago.
        let mut routines = Routines {
            runs: vec![
                run("a2", "a", Status::Succeeded, 1),
                run("b2", "b", failed(), 2),
                run("a1", "a", failed(), 3),
                run("b1", "b", failed(), 4),
                run("c1", "c", failed(), 48),
            ],
            ..up()
        };
        let ids = |r: &Routines| r.failures(NOW).iter().map(|run| run.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&routines), ["b2"]);
        routines.runs.insert(0, run("b3", "b", Status::Running, 0));
        assert_eq!(ids(&routines), ["b2"], "a running run does not hide the last failure");
        routines.dismissed_failures.push(("b".into(), "b2".into()));
        assert!(ids(&routines).is_empty(), "dismissed");
        routines.runs[0].status = Status::Failed("y".into());
        assert_eq!(ids(&routines), ["b3"], "a newer failure shows again");
    }

    #[test]
    fn a_run_with_no_verdict_is_unassessed_never_ok() {
        let mut done = run("r1", "watch", Status::Succeeded, 2);
        let last = |run: &RunLine| {
            let routines = Routines { list: vec![routine("watch", None)], runs: vec![run.clone()], ..up() };
            routines.rows(NOW).pop().unwrap().lines[1].clone()
        };
        assert_eq!(last(&done), "last unassessed · 2h ago");
        (done.level, done.line) = (Some(Level::Normal), Some("All quiet".into()));
        assert_eq!(last(&done), "last ok · 2h ago · All quiet");
        done.level = Some(Level::Critical);
        assert_eq!(last(&done), "last critical · 2h ago · All quiet");
        assert_eq!(last(&run("r2", "watch", failed(), 3)), "last failed · 3h ago · exit 1");
    }

    #[test]
    fn a_routine_runs_only_if_listed_idle_and_the_daemon_is_up() {
        let mut routines = Routines { list: vec![routine("nightly", None), routine("busy", None)], ..up() };
        routines.list[1].active_run = Some("r9".into());
        assert_eq!(routines.can_run("nightly"), Ok(()));
        assert_eq!(routines.can_run("busy").unwrap_err(), "busy is already running");
        assert_eq!(routines.can_run("gone").unwrap_err(), "no routine gone");
        routines.daemon = Daemon::Absent;
        assert_eq!(routines.can_run("nightly").unwrap_err(), "ottobot daemon not running");
        routines.daemon = Daemon::Incompatible { reason: "x".into() };
        assert!(routines.can_run("nightly").is_err());
    }

    #[test]
    fn without_a_usable_daemon_there_is_one_row_that_says_so() {
        // Whatever else the value holds: nothing of it is listed.
        let stale =
            Routines { list: vec![routine("nightly", None)], alerts: vec![alert("a", Level::Critical, 1)], ..up() };
        let absent = Routines { daemon: Daemon::Absent, ..stale.clone() };
        let rows = absent.rows(NOW);
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].group, rows[0].title.as_str()), (Group::Daemon, "ottobot daemon not running"));
        assert_eq!(rows[0].lines, ["Start it: otto daemon --install"]);
        assert_eq!((rows[0].routine.as_ref(), absent.needs_you(NOW), absent.quota_lines().len()), (None, 0, 0));
        let other = Routines { daemon: Daemon::Incompatible { reason: "no `root` in the reply".into() }, ..stale };
        let rows = other.rows(NOW);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].title.as_str(), rows[0].lines[0].as_str()),
            ("Incompatible ottobot daemon", "no `root` in the reply")
        );
        assert_eq!((other.daemon_line(), other.hint()), (None, None));
    }

    #[test]
    fn quota_is_a_line_a_runner() {
        let window = |label: &str, left_percent| Window { label: label.into(), left_percent };
        let quota = |runner: &str, health: Result<&str, &str>, windows: Result<Vec<Window>, &str>| Quota {
            runner: runner.into(),
            health: health.map(String::from).map_err(String::from),
            windows: windows.map_err(String::from),
        };
        let routines = Routines {
            quota: vec![
                quota("claude", Ok("2.1"), Ok(vec![window("5h", 63), window("7d", 93)])),
                quota("codex", Err("not installed"), Ok(vec![])),
                quota("low", Ok(""), Ok(vec![window("5h", 4)])),
                quota("dark", Ok(""), Err("no token")),
            ],
            ..up()
        };
        assert_eq!(
            routines.quota_lines(),
            [
                ("claude · 5h 63% left · 7d 93% left".to_string(), Tone::Plain),
                ("codex · not usable: not installed".to_string(), Tone::Warning),
                ("low · 5h 4% left".to_string(), Tone::Warning),
                ("dark · quota unknown: no token".to_string(), Tone::Plain),
            ]
        );
        assert_eq!(up().hint().as_deref(), Some("No routines. Add a skill file with `trigger:` under /skills."));
    }

    #[test]
    fn text_from_a_run_is_one_plain_line() {
        assert_eq!(plain("  a\nb\u{1b}[31mc\t "), "a b [31mc");
        let long = "x".repeat(300);
        assert_eq!(plain(&long).chars().count(), 201);
        let mut routines = Routines { alerts: vec![alert("a", Level::Critical, 1)], ..up() };
        routines.alerts[0].headline = "line one\nline two".into();
        assert_eq!(routines.rows(NOW)[0].lines, ["line one line two"]);
    }

    #[test]
    fn only_a_plain_run_id_is_followed() {
        assert!(safe_id("20260101-120000-nightly_check.2"));
        for id in ["", "-rf", ".hidden", "a b", "a;b", "a'b", "$(x)", "a/b", "é", &"x".repeat(129)] {
            assert!(!safe_id(id), "{id}");
        }
    }

    #[test]
    fn only_a_report_under_the_store_is_opened() {
        let store = "/Users/me/.ottobot/projects/skills";
        assert!(report_allowed("/Users/me/.ottobot/projects/skills/reports/r1.html", store));
        assert!(report_allowed("/Users/me/.ottobot/projects/skills/reports/2026/r1.md", store));
        for path in [
            "/Users/me/.ottobot/projects/skills/reports/r1.sh",
            "/Users/me/.ottobot/projects/skills/reports/r1",
            "/Users/me/.ottobot/projects/skills/reports",
            "/Users/me/.ottobot/projects/skills/runs/r1.html",
            "/Users/me/.ottobot/projects/skills/reports/../../../../x.html",
            "/Users/me/.ottobot/projects/skills/reports-other/r1.html",
            "/Applications/Calculator.app",
            "reports/r1.html",
            "",
        ] {
            assert!(!report_allowed(path, store), "{path}");
        }
        assert!(!report_allowed("/reports/r1.html", ""), "no store named");
    }

    #[test]
    fn credentials_are_masked() {
        assert_eq!(redact("Authorization: Bearer abcdefgh12345678"), "Authorization: Bearer <redacted>");
        assert_eq!(redact("use sk-proj-0123456789abcdef now"), "use <redacted> now");
        assert_eq!(redact("url?token=abc123&x=1 failed"), "url?token=<redacted> failed");
        assert_eq!(redact("push with ghp_0123456789abcdefghij"), "push with <redacted>");
        assert_eq!(redact("error: 401 Unauthorized"), "error: 401 Unauthorized");
        // What `routine-tail` prints: no terminal escapes, and an escape cannot hide a token.
        assert_eq!(
            output_lines("ok\u{1b}[2J\r\nBearer abcd\u{7}efgh1234\n\tdone"),
            ["ok[2J", "Bearer <redacted>", "\tdone"]
        );
    }

    #[test]
    fn times_are_read_with_their_offset() {
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_time("2026-11-12T10:44:00.123456789Z"), Some(1_794_480_240_123));
        assert_eq!(parse_time("2026-11-12T02:44:00.123456789-08:00"), Some(1_794_480_240_123));
        assert_eq!(parse_time("2026-11-12T16:14:00.5+05:30"), Some(1_794_480_240_500));
        assert_eq!(parse_time("2026-11-12T10:44:00+00:00"), Some(1_794_480_240_000));
        for bad in
            ["", "yesterday", "2026-11-12", "2026-11-12T10:44:00", "2026-13-01T00:00:00Z", "1970-01-01T00:00:00+01:00"]
        {
            assert_eq!(parse_time(bad), None, "{bad}");
        }
        assert_eq!(parse_time("2026-11-12T10:44:00+99:00"), None);
        assert_eq!(parse_time("2026-11-12T1é:00:00Z"), None);
    }
}
