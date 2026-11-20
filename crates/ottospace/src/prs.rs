//! Each listed GitHub repository's open pull requests, read with `gh pr list` on a thread of its
//! own while the Repos section is open: read-only, with a fixed argv, and nothing gh says is
//! ever an argument or a path. What they mean, and when they are read, is the core's (`prs`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use ottospace_core::prs::{self, Branch, Pr};

/// How long gh and git may take for one repository.
const TIMEOUT: Duration = Duration::from_secs(20);
/// Why there is no gh to run: none found, or (a test run) no `OTTOSPACE_GH`.
pub const NO_GH: &str = "gh is not installed";
/// How long a missing or logged-out gh is believed before it is looked for again.
const RECHECK: Duration = Duration::from_secs(5 * 60);

/// One repository to read: its path, and each checkout's folder and branch.
struct Job {
    repo: String,
    checkouts: Vec<(String, String)>,
}

/// What a read came to.
pub struct Read {
    pub repo: String,
    pub result: Result<(Vec<Pr>, Vec<Branch>), &'static str>,
}

/// The reader thread, and when each repository was last asked for.
pub struct Reader {
    jobs: Sender<Job>,
    pub reads: Receiver<Read>,
    last: HashMap<String, u64>,
    /// The Repos section was open at the last look: its opening reads again.
    open: bool,
}

impl Reader {
    pub fn new() -> Self {
        let (jobs, inbox) = channel::<Job>();
        let (done, reads) = channel();
        std::thread::spawn(move || {
            let mut ready: Option<(Result<PathBuf, &'static str>, Instant)> = None;
            for job in inbox {
                if ready.as_ref().is_none_or(|(gh, at)| gh.is_err() && at.elapsed() >= RECHECK) {
                    ready = Some((gh(), Instant::now()));
                }
                let result = match ready.as_ref().map(|(gh, _)| gh.clone()) {
                    Some(Ok(gh)) => read(&gh, &job),
                    Some(Err(why)) => Err(why),
                    None => Err(NO_GH),
                };
                if done.send(Read { repo: job.repo, result }).is_err() {
                    return;
                }
                ottospace_mac::wake::post();
            }
        });
        Self { jobs, reads, last: HashMap::new(), open: false }
    }

    /// Asks for each repository of `repos` (path, checkouts) that is due (`prs::due`) and on github.com.
    pub fn ask(&mut self, open: bool, repos: Vec<(String, Vec<(String, String)>)>, now: u64) {
        let opened = open && !self.open;
        self.open = open;
        for (repo, checkouts) in repos {
            if !prs::due(open, opened, self.last.get(&repo).copied(), now) {
                continue;
            }
            self.last.insert(repo.clone(), now);
            if crate::repos::small_text(&Path::new(&repo).join(".git/config"), 64 * 1024)
                .is_some_and(|config| prs::on_github(&config))
            {
                let _ = self.jobs.send(Job { repo, checkouts });
            }
        }
    }
}

/// gh, if it is there and logged in to github.com. A test run uses only `OTTOSPACE_GH`, a
/// stand-in: never the user's gh.
pub fn gh() -> Result<PathBuf, &'static str> {
    let found = match std::env::var_os("OTTOSPACE_E2E") {
        Some(_) => std::env::var_os("OTTOSPACE_GH").map(PathBuf::from).filter(|p| p.is_file()),
        // An app started from the Dock has a short PATH: Homebrew's folders too.
        None => {
            let path: Vec<PathBuf> =
                std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
            let dirs = path
                .into_iter()
                .filter(|dir| dir.is_absolute())
                .chain(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
            dirs.map(|dir| dir.join("gh")).find(|p| p.is_file())
        }
    };
    let gh = found.ok_or(NO_GH)?;
    let mut status = command(&gh);
    status.args(["auth", "status", "--hostname", "github.com"]);
    let said = crate::child::run(status, TIMEOUT, 64 * 1024).map_err(|_| "gh did not answer")?;
    if said.ok { Ok(gh) } else { Err("gh is not logged in (gh auth login)") }
}

/// gh with no prompt, no colour, no update check.
pub fn command(gh: &Path) -> Command {
    let mut command = Command::new(gh);
    for (name, value) in
        [("GH_PROMPT_DISABLED", "1"), ("GH_NO_UPDATE_NOTIFIER", "1"), ("NO_COLOR", "1"), ("GH_SPINNER_DISABLED", "1")]
    {
        command.env(name, value);
    }
    command
}

/// One repository: where each checkout's branch pushes and its commit (git), then the open pull
/// requests (gh, in the repository's folder).
fn read(gh: &Path, job: &Job) -> Result<(Vec<Pr>, Vec<Branch>), &'static str> {
    let git = |dir: &str, args: &[&str]| {
        let mut command = crate::child::git_read(Path::new(dir));
        command.args(args);
        crate::child::run(command, TIMEOUT, 256 * 1024).ok().filter(|said| said.ok).map(|said| said.out)
    };
    let config =
        git(&job.repo, &["config", "--get-regexp", r"^(branch\..*\.remote|remote\..*\.url)$"]).unwrap_or_default();
    let branches = job
        .checkouts
        .iter()
        .map(|(path, name)| {
            let (slug, tracked) = prs::push_slug(name, &config);
            let oid = git(path, &["rev-parse", "HEAD"]).map(|out| out.trim().to_string());
            Branch { path: path.clone(), name: name.clone(), slug, tracked, oid }
        })
        .collect();
    let mut list = command(gh);
    list.current_dir(&job.repo).args([
        "pr",
        "list",
        "--state",
        "open",
        "--limit",
        "100",
        "--json",
        "number,title,isDraft,headRefName,headRefOid,headRepository,headRepositoryOwner,isCrossRepository,\
         baseRefName,mergeable,mergeStateStatus,reviewDecision,statusCheckRollup,url",
    ]);
    let said = crate::child::run(list, TIMEOUT, prs::MAX_BYTES as u64 + 1).map_err(|_| "gh did not answer")?;
    if !said.ok {
        return Err("gh could not list pull requests");
    }
    Ok((prs::parse(&said.out).map_err(|_| "gh's answer does not read")?, branches))
}
