//! The apps installed on this Mac, for the picker to offer those that are not running. Found by
//! a scan off the main thread, kept for the session, read again when the picker opens if old.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::Context;
use ottospace_core::settings::Installed;
use ottospace_core::{Action, Axis, Origin};
use ottospace_mac::icon;

use crate::Workspace;

/// A scan older than this is done again when the picker opens.
const FRESH: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct Scan {
    apps: Vec<Installed>,
    /// When the last scan started. `None`: never.
    at: Option<Instant>,
}

/// The folders scanned: `$OTTOSPACE_APPS_DIRS` (separated by `:`), else where apps usually are.
/// An E2E run that names none scans none: a test never lists the user's apps.
fn folders() -> Vec<PathBuf> {
    match std::env::var_os("OTTOSPACE_APPS_DIRS") {
        Some(dirs) => std::env::split_paths(&dirs).filter(|dir| !dir.as_os_str().is_empty()).collect(),
        None if std::env::var_os("OTTOSPACE_E2E").is_some() => Vec::new(),
        None => icon::app_folders(),
    }
}

/// The bundle path of every running app, windows or none: an app with no window on this Space
/// (minimised, on another Space, in the menu bar) is running, not one to start.
pub fn running_bundles() -> Vec<String> {
    icon::running_bundles().into_iter().map(|path| path.to_string_lossy().into_owned()).collect()
}

impl Workspace {
    /// The picker opened: tells the core what is installed as last scanned, and scans again if
    /// that was a while ago. The scan's answer comes later.
    pub fn offer_installed(&mut self, cx: &mut Context<Self>) {
        self.tell_installed();
        if self.installed.at.is_some_and(|at| at.elapsed() < FRESH) {
            return;
        }
        self.installed.at = Some(Instant::now());
        let scan = cx.background_executor().spawn(async {
            let app = |(name, path): (String, PathBuf)| Installed { name, path: path.to_string_lossy().into_owned() };
            icon::apps_in(&folders()).into_iter().map(app).collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let apps = scan.await;
            let _ = this.update(cx, |this, cx| {
                this.installed.apps = apps;
                this.tell_installed();
                cx.notify();
            });
        })
        .detach();
    }

    /// The scan's apps and the bundles running now: the core offers those that are not.
    fn tell_installed(&mut self) {
        let (apps, running) = (self.installed.apps.clone(), running_bundles());
        self.model.update(Action::SetInstalled { apps, running }.sent_by(Origin::System));
    }

    /// Starts a picked app that is not running. Only one the scan or the settings found, and
    /// never in a test run: a real app would start.
    pub fn launch_picked(&mut self, name: &str, path: &str, axis: Axis, new_tab: bool) {
        let known = self.installed.apps.iter().chain(&self.closed).any(|app| app.path == path);
        let started = if !self.hosting || self.host_only.is_some() {
            Err("window hosting is off in e2e runs".to_string())
        } else if !known {
            Err(format!("{name} is not an app found on this Mac"))
        } else {
            self.launch(Installed { name: name.into(), path: path.into() }, axis, new_tab, None)
        };
        self.error = started.err();
    }
}
