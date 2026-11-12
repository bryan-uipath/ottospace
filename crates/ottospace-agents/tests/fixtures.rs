//! The runners against hand-written session files in `tests/fixtures`: made-up content in the
//! real schema. No test reads a real config dir.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use ottospace_agents::claude::Claude;
use ottospace_agents::codex::Codex;
use ottospace_agents::git::checkouts;
use ottospace_agents::report::report;
use ottospace_agents::snapshot::Snapshot;
use ottospace_agents::station::Station;
use ottospace_agents::{Cursor, Kind, Runner, SessionRef, What};

const CLAUDE_MAIN: &str = "11111111-1111-4111-8111-111111111111";
const CLAUDE_WORKTREE: &str = "22222222-2222-4222-8222-222222222222";
const CODEX_NEW: &str = "c0de0001-0000-4000-8000-000000000001";
const CODEX_CHILD: &str = "c0de0002-0000-4000-8000-000000000002";
const CODEX_OLD: &str = "c0de0004-0000-4000-8000-000000000004";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn claude() -> Claude {
    // A config dir that does not exist is skipped.
    Claude::new(vec![fixtures().join("claude"), fixtures().join("no-such-dir")])
}

fn codex() -> Codex {
    Codex::new(fixtures().join("codex"))
}

/// A fresh folder under the system temp dir.
fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ottospace-agents-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn ids(sessions: &[SessionRef]) -> Vec<&str> {
    let mut ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
    ids.sort();
    ids
}

fn find(runner: &dyn Runner, dir: &str, id: &str) -> SessionRef {
    runner.sessions(Path::new(dir)).unwrap().into_iter().find(|s| s.id == id).unwrap()
}

fn snapshot(runner: &dyn Runner, dir: &str, id: &str) -> Snapshot {
    let (events, _) = runner.read(&find(runner, dir, id), Cursor::default()).unwrap();
    Snapshot::fold(&events)
}

fn counts(snapshot: &Snapshot) -> Vec<(Station, u32)> {
    snapshot.counts.iter().map(|(station, n)| (*station, *n)).collect()
}

#[test]
fn claude_lists_sessions_started_in_a_directory_or_below() {
    let claude = claude();
    let sessions = claude.sessions(Path::new("/work/app")).unwrap();
    assert_eq!(ids(&sessions), [CLAUDE_MAIN, CLAUDE_WORKTREE], "the worktree's too, not `/work/app-other`");

    let main = find(&claude, "/work/app", CLAUDE_MAIN);
    assert_eq!(main.kind, Kind::Claude);
    assert_eq!(main.cwd, Path::new("/work/app"));
    assert_eq!(main.started, Some(1_794_474_000_000));
    assert!(main.last_activity.is_some());
    assert_eq!(main.model.as_deref(), Some("claude-fable-5-2"));
    assert_eq!((main.pid, main.busy), (Some(4242), Some(true)), "from sessions/4242.json");

    let worktree = find(&claude, "/work/app/.claude/worktrees/fix", CLAUDE_WORKTREE);
    assert_eq!((worktree.pid, worktree.busy), (None, None));
    assert_eq!(ids(&claude.sessions(Path::new("/work/app/.claude/worktrees/fix")).unwrap()), [CLAUDE_WORKTREE]);
    assert_eq!(ids(&claude.sessions(Path::new("/work/app-other")).unwrap()).len(), 1);
    assert!(claude.sessions(Path::new("/work/ap")).unwrap().is_empty());
    assert!(claude.sessions(Path::new("/nowhere")).unwrap().is_empty());
}

#[test]
fn claude_snapshot() {
    let s = snapshot(&claude(), "/work/app", CLAUDE_MAIN);
    let running: Vec<_> = s.running.iter().map(|c| (c.tool.as_str(), c.station, c.summary.as_str())).collect();
    assert_eq!(running, [("Bash", Station::Network, "git push")]);
    assert_eq!(s.station(), Some(Station::Network));
    let recent: Vec<_> = s.recent.iter().map(|c| (c.summary.as_str(), c.failed)).collect();
    assert_eq!(recent, [("src/main.rs", false), ("Explore", false), ("cargo test", true), ("agent", false)]);
    assert_eq!(
        counts(&s),
        [
            (Station::You, 1),
            (Station::Model, 3),
            (Station::Files, 1),
            (Station::Shell, 1),
            (Station::Network, 1),
            (Station::Agents, 2)
        ],
        "one model step per message id; the summary, the reminder and the sidechain record count nowhere"
    );
    assert_eq!(s.errors, 2);
    assert_eq!(s.last_error.as_ref().map(|e| e.1.as_str()), Some("rate_limit"));
    let subagents: Vec<_> = s.subagents.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(subagents, ["agent"], "`Explore` finished, by its meta file");
    assert_eq!(s.model.as_deref(), Some("claude-fable-5-2"));
    assert_eq!((s.context_tokens, s.context_window, s.context_percent()), (Some(61_000), Some(200_000), Some(30)));
    assert_eq!(s.compactions, 1);
}

#[test]
fn claude_events_name_the_model_change() {
    let claude = claude();
    let (events, _) = claude.read(&find(&claude, "/work/app", CLAUDE_MAIN), Cursor::default()).unwrap();
    let change = events.iter().find_map(|e| match &e.what {
        What::ModelChange { from, to } => Some((from.as_str(), to.as_str())),
        _ => None,
    });
    assert_eq!(change, Some(("claude-opus-5-5", "claude-fable-5-2")));
}

#[test]
fn read_is_incremental_and_waits_for_a_whole_line() {
    let claude = claude();
    let original = find(&claude, "/work/app", CLAUDE_MAIN);
    let text = fs::read_to_string(&original.path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let (first, last) = (lines[..lines.len() - 1].join("\n"), lines[lines.len() - 1]);

    // All lines but the last, then half of the last one, as a runner in the middle of a write leaves it.
    let path = temp("partial").join("session.jsonl");
    let (half, rest) = last.split_at(last.len() / 2);
    fs::write(&path, format!("{first}\n{half}")).unwrap();
    let session = SessionRef { path: path.clone(), ..original };

    let (events, cursor) = claude.read(&session, Cursor::default()).unwrap();
    assert_eq!(cursor.offset(), first.len() as u64 + 1, "stops before the half line");
    let mut s = Snapshot::fold(&events);
    assert_eq!(s.running.len(), 2, "the last tool result is not in yet");

    let (events, same) = claude.read(&session, cursor.clone()).unwrap();
    assert!(events.is_empty());
    assert_eq!(same, cursor);

    fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(format!("{rest}\n").as_bytes()).unwrap();
    let (events, cursor) = claude.read(&session, cursor).unwrap();
    assert_eq!(events.len(), 1);
    events.iter().for_each(|e| s.apply(e));
    assert_eq!(s.running.len(), 1);
    assert_eq!(cursor.offset(), text.len() as u64);

    // A replaced, shorter file is read from its start.
    fs::write(&path, format!("{}\n", lines[3])).unwrap();
    let (events, cursor) = claude.read(&session, cursor).unwrap();
    assert_eq!(events.iter().map(|e| &e.what).collect::<Vec<_>>(), [&What::UserMessage]);
    assert_eq!(cursor.offset(), lines[3].len() as u64 + 1);
}

#[test]
fn codex_lists_sessions_started_in_a_directory_or_below() {
    let mut codex = codex();
    let sessions = codex.sessions(Path::new("/work/app")).unwrap();
    assert_eq!(ids(&sessions), [CODEX_NEW, CODEX_CHILD, CODEX_OLD]);
    assert_eq!(ids(&codex.sessions(Path::new("/work/app/crates")).unwrap()), [CODEX_NEW, CODEX_CHILD]);
    assert!(codex.sessions(Path::new("/work/ap")).unwrap().is_empty());

    let new = find(&codex, "/work/app", CODEX_NEW);
    assert_eq!(new.kind, Kind::Codex);
    assert_eq!(new.cwd, Path::new("/work/app/crates"));
    assert_eq!(new.started, Some(1_794_475_800_000));
    assert_eq!(new.model.as_deref(), Some("gpt-6-astra"));
    assert_eq!((new.pid, new.busy, new.parent.as_deref()), (None, None, None));
    assert_eq!(find(&codex, "/work/app", CODEX_CHILD).parent.as_deref(), Some(CODEX_NEW));

    // Only the newest rollouts are opened.
    codex.max_files = 2;
    assert_eq!(ids(&codex.sessions(Path::new("/work/app")).unwrap()), [CODEX_NEW, CODEX_CHILD]);
}

#[test]
fn codex_snapshot() {
    let s = snapshot(&codex(), "/work/app", CODEX_NEW);
    let running: Vec<_> = s.running.iter().map(|c| (c.tool.as_str(), c.station, c.summary.as_str())).collect();
    assert_eq!(running, [("exec", Station::Shell, "exec")]);
    let recent: Vec<_> = s.recent.iter().map(|c| (c.station, c.summary.as_str(), c.failed)).collect();
    assert_eq!(
        recent,
        [
            (Station::Network, "curl", false),
            (Station::Agents, "spawn_agent", false),
            (Station::Shell, "cargo test", true)
        ],
        "`exec` is named by the command it turned out to run"
    );
    assert_eq!(
        counts(&s),
        [(Station::You, 1), (Station::Model, 1), (Station::Shell, 2), (Station::Network, 1), (Station::Agents, 1)]
    );
    assert_eq!(s.errors, 1);
    assert_eq!(s.last_error.as_ref().map(|e| e.1.as_str()), Some("cargo test"));
    assert_eq!(s.subagents.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(), ["reviewer"]);
    assert_eq!(s.model.as_deref(), Some("gpt-6-nova"));
    assert_eq!((s.context_tokens, s.context_window, s.context_percent()), (Some(32_000), Some(400_000), Some(8)));
    assert_eq!(s.compactions, 1);
}

#[test]
fn codex_snapshot_of_an_older_rollout() {
    let s = snapshot(&codex(), "/work/app", CODEX_OLD);
    assert!(s.running.is_empty(), "a web search starts and ends in one record");
    let recent: Vec<_> = s.recent.iter().map(|c| (c.tool.as_str(), c.station, c.summary.as_str())).collect();
    assert_eq!(
        recent,
        [
            ("exec_command", Station::Files, "rg"),
            ("apply_patch", Station::Files, "src/lib.rs"),
            ("web_search_call", Station::Network, "web_search_call")
        ]
    );
    assert_eq!(counts(&s), [(Station::You, 1), (Station::Model, 1), (Station::Files, 2), (Station::Network, 1)]);
    assert_eq!(
        (s.errors, s.last_error.as_ref().map(|e| e.1.as_str())),
        (1, Some("usage_limit_exceeded")),
        "Esc is no error"
    );
    assert_eq!(s.context_percent(), Some(40));
}

#[test]
fn report_lists_sessions_and_prints_only_derived_facts() {
    let (claude, codex) = (claude(), codex());
    let dirs = [PathBuf::from("/work/app"), PathBuf::from("/work/app/.claude/worktrees/fix")];
    let newest = claude
        .sessions(&dirs[0])
        .unwrap()
        .iter()
        .chain(&codex.sessions(&dirs[0]).unwrap())
        .filter_map(|s| s.last_activity)
        .max();
    let report = report(&[&claude, &codex], &dirs, newest.unwrap() + 1000, 10).unwrap();
    let text = serde_json::to_string_pretty(&report).unwrap();
    // `cargo test -p ottospace-agents -- --nocapture report` shows it.
    println!("{text}");

    let sessions = report["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 5, "the worktree session is found from both directories and listed once");
    let main = sessions.iter().find(|s| s["id"] == CLAUDE_MAIN).unwrap();
    assert_eq!(main["kind"], "claude");
    assert_eq!(main["busy"], true);
    assert_eq!(main["snapshot"]["station"], "network");
    assert_eq!(main["snapshot"]["running"][0]["summary"], "git push");
    assert_eq!(main["snapshot"]["context_percent"], 30);
    assert_eq!(main["snapshot"]["counts"]["agents"], 2);
    assert!(sessions.iter().any(|s| s["active"] == true));

    // Every prompt, output and script in the fixtures says "made-up"; none of it may come out.
    for private in ["made-up", "greeting", "TODO", "health", "--workspace", "origin", "site is up", "fn main"] {
        assert!(!text.contains(private), "the report leaks {private:?}");
    }
}

#[test]
fn report_snapshots_only_the_newest_sessions() {
    let codex = codex();
    let report = report(&[&codex], &[PathBuf::from("/work/app")], 0, 1).unwrap();
    let with: Vec<bool> = report["sessions"].as_array().unwrap().iter().map(|s| s.get("snapshot").is_some()).collect();
    assert_eq!(with, [true, false, false]);
}

#[test]
fn checkouts_are_the_main_checkout_and_its_worktrees() {
    let root = temp("git");
    let (main, linked) = (root.join("app"), root.join("elsewhere/app-fix"));
    let admin = main.join(".git/worktrees/app-fix");
    fs::create_dir_all(&admin).unwrap();
    fs::create_dir_all(linked.join("src")).unwrap();
    fs::create_dir_all(main.join("src")).unwrap();
    fs::write(admin.join("gitdir"), format!("{}\n", linked.join(".git").display())).unwrap();
    fs::write(admin.join("commondir"), "../..\n").unwrap();
    fs::write(linked.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();

    let both = [main.clone(), linked.clone()];
    assert_eq!(checkouts(&main), both);
    assert_eq!(checkouts(&main.join("src")), both);
    assert_eq!(checkouts(&linked.join("src")), both, "from a worktree's subdirectory");
    let plain = root.join("plain");
    assert_eq!(checkouts(&plain), std::slice::from_ref(&plain), "not a repository");
}
