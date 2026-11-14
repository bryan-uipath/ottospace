//! The sessions of some directories as JSON: derived facts only.

use std::io;
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::snapshot::Snapshot;
use crate::{SessionReader, SessionRef, read_all};

/// A session counts as active this long after its file last changed.
pub const IDLE_AFTER_MS: u64 = 30_000;

/// Every runner's sessions under `dirs`, newest first. The newest `snapshots` are read in full
/// for what they are doing; the rest are only listed. `now_ms` is the caller's clock.
pub fn report(runners: &[&dyn SessionReader], dirs: &[PathBuf], now_ms: u64, snapshots: usize) -> io::Result<Value> {
    let mut sessions: Vec<(&dyn SessionReader, SessionRef)> = Vec::new();
    for runner in runners {
        for session in runner.sessions(dirs)? {
            // One file is one session, however many directories or runners led to it.
            if !sessions.iter().any(|(_, s)| (s.runner, &s.path) == (session.runner, &session.path)) {
                sessions.push((*runner, session));
            }
        }
    }
    sessions.sort_by_key(|s| std::cmp::Reverse(s.1.last_activity));
    let mut out = Vec::new();
    for (i, (runner, session)) in sessions.iter().enumerate() {
        let events = if i < snapshots { Some(read_all(*runner, session)) } else { None };
        let snapshot = match events {
            None => None,
            Some(Ok(events)) => Some(Snapshot::fold(&events)),
            // Deleted since it was listed: not a session any more.
            Some(Err(err)) if err.kind() == io::ErrorKind::NotFound => continue,
            Some(Err(err)) => return Err(err),
        };
        let age = session.last_activity.map(|at| now_ms.saturating_sub(at));
        let model = snapshot.as_ref().and_then(|s| s.model.clone()).or(session.model.clone());
        let mut entry = json!({
            "kind": session.runner.name(),
            "id": session.id,
            "cwd": session.cwd,
            "started": session.started,
            "last_activity": session.last_activity,
            "active": age.is_some_and(|age| age <= IDLE_AFTER_MS),
            "busy": session.busy,
            "pid": session.pid,
            "parent": session.parent,
            "model": model,
        });
        if let Some(snapshot) = snapshot {
            let running: Vec<Value> = snapshot
                .running
                .iter()
                .map(|c| json!({"tool": c.tool, "station": c.station, "summary": c.summary, "started": c.started}))
                .collect();
            let recent: Vec<Value> = snapshot
                .recent
                .iter()
                .map(|c| json!({"station": c.station, "summary": c.summary, "failed": c.failed}))
                .collect();
            entry["snapshot"] = json!({
                "station": snapshot.station(),
                "running": running,
                "recent": recent,
                "counts": snapshot.counts,
                "subagents": snapshot.subagents.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(),
                "errors": snapshot.errors,
                "last_error": snapshot.last_error.as_ref().map(|(_, what)| what),
                "context_tokens": snapshot.context_tokens,
                "context_window": snapshot.context_window,
                "context_percent": snapshot.context_percent(),
                "compactions": snapshot.compactions,
            });
        }
        out.push(entry);
    }
    Ok(json!({ "sessions": out }))
}
