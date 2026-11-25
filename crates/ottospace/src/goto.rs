//! Go to's file lists (⌘P, `ottospace_core::goto`): a checkout's files, read off the main thread
//! as the code city's are (`city::checkout_files`), only for a folder the user went into.

use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};

use gpui::{Context, Window};
use ottospace_core::{Action, Origin};

use crate::Workspace;

/// Files listed at most, and bytes of names: past either, the first ones, and the list says so.
pub const MOST_FILES: usize = 50_000;
pub(crate) const MOST_BYTES: u64 = 16 << 20;

/// A folder's files: its paths (`None`: git could not list them), and whether there were more.
type Read = (String, Option<Vec<String>>, bool);

/// Reads each folder asked, in turn, on a thread of its own.
pub struct Reader {
    jobs: Sender<(String, Vec<String>)>,
    reads: Receiver<Read>,
}

impl Reader {
    pub fn new() -> Self {
        let (jobs, inbox) = channel::<(String, Vec<String>)>();
        let (done, reads) = channel();
        std::thread::spawn(move || {
            for (folder, roots) in inbox {
                let read = crate::city::checkout_files(
                    Path::new(&folder),
                    &roots,
                    (MOST_FILES, MOST_BYTES, std::time::Duration::from_secs(10)),
                    false,
                    &[],
                    &|_| true,
                    &|| false,
                );
                let cut = read.as_ref().is_some_and(|(_, cut)| *cut);
                let paths = read.map(|(files, _)| files.into_iter().map(|(path, _)| path).collect());
                if done.send((folder, paths, cut)).is_err() {
                    return;
                }
                ottospace_mac::wake::post();
            }
        });
        Self { jobs, reads }
    }
}

impl Workspace {
    /// `ListFiles`: read the checkout's files, if it is one the Repos section lists.
    pub(crate) fn list_files(&self, folder: &str) {
        let _ = self.goto_files.jobs.send((folder.to_string(), self.model.checkout_roots()));
    }

    /// The files read since: to the palette, which keeps them while it is open, else nowhere.
    pub(crate) fn take_files(&mut self, window: &Window, cx: &mut Context<Self>) {
        let came: Vec<Read> = self.goto_files.reads.try_iter().collect();
        for (folder, paths, cut) in came {
            self.dispatch(Action::GoToFiles { folder, paths, cut }, Origin::System, window, cx);
        }
    }
}
