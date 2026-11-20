//! Open pull requests of the listed repositories, as `gh pr list` tells them: what a worktree's
//! row shows. The shell runs gh; what its answer means is decided here. Titles, check names and
//! URLs are other people's text: shown cleaned on the user's hover, never in the state.

use serde::{Deserialize, Serialize};

/// Why a client's click opens no pull request: a link opens in the user's browser.
pub const USERS_ALONE: &str = "only the user opens a pull request";
/// The most of gh's answer read, in bytes, and the most pull requests kept of a repository.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PRS: usize = 100;
/// How often a repository's pull requests are read while the Repos section is open, and at most
/// how often opening the section reads them again (ms).
pub const EVERY_MS: u64 = 5 * 60 * 1000;
pub const OPENED_MS: u64 = 60 * 1000;

/// `repos_prs`: `auto` (default) reads pull requests when gh is there and logged in; `on` as well,
/// and says why not when it cannot; `off` never runs gh.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Setting {
    #[default]
    Auto,
    On,
    Off,
}

/// One open pull request, as `gh pr list --json …` writes it. A field gh leaves out or sets to
/// null is empty; one it adds is ignored.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Pr {
    pub number: u32,
    pub title: Option<String>,
    pub is_draft: bool,
    pub head_ref_name: Option<String>,
    pub base_ref_name: Option<String>,
    pub mergeable: Option<String>,
    pub merge_state_status: Option<String>,
    pub review_decision: Option<String>,
    pub status_check_rollup: Option<Vec<Check>>,
    pub url: Option<String>,
    /// Where its branch is: the repository it was pushed to, and that branch's commit.
    pub head_repository_owner: Option<Login>,
    pub head_repository: Option<Named>,
    pub is_cross_repository: bool,
    pub head_ref_oid: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Login {
    pub login: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Named {
    pub name: String,
}

/// One check of a pull request: a check run (`name`, `status`, `conclusion`) or a commit status
/// (`context`, `state`).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Check {
    pub name: Option<String>,
    pub context: Option<String>,
    pub status: Option<String>,
    pub conclusion: Option<String>,
    pub state: Option<String>,
}

/// What a check came to.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Outcome {
    Pass,
    Running,
    Fail,
}

impl Check {
    fn outcome(&self) -> Outcome {
        let is = |value: &Option<String>, of: &[&str]| value.as_deref().is_some_and(|v| of.contains(&v));
        match () {
            // A commit status.
            _ if is(&self.state, &["FAILURE", "ERROR"]) => Outcome::Fail,
            _ if is(&self.state, &["PENDING", "EXPECTED"]) => Outcome::Running,
            _ if self.state.is_some() => Outcome::Pass,
            // A check run: not completed yet, or what it concluded.
            _ if !is(&self.status, &["COMPLETED"]) => Outcome::Running,
            _ if is(&self.conclusion, &["SUCCESS", "NEUTRAL", "SKIPPED"]) => Outcome::Pass,
            _ => Outcome::Fail,
        }
    }

    fn name(&self) -> &str {
        self.name.as_deref().or(self.context.as_deref()).unwrap_or("check")
    }
}

/// The row's one mark, by precedence: `Failing` (a check failed, changes are requested, or a
/// merge conflict), then `Draft` (muted, no dot), `Pending` (checks running, a review required),
/// `Ready` (approved, checks passing or none, mergeable), else `Open`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Look {
    Failing,
    Draft,
    Pending,
    Ready,
    Open,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Checks {
    None,
    Passing,
    Running,
    Failing,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Review {
    None,
    Approved,
    ChangesRequested,
    Required,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Merge {
    Clean,
    Conflicts,
    Behind,
    Blocked,
    Unknown,
}

/// A pull request as the state tells it: numbers and kinds only, no one's text.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Summary {
    pub number: u32,
    pub draft: bool,
    pub look: Look,
    pub checks: Checks,
    /// How many checks failed.
    pub failed: usize,
    pub review: Review,
    pub merge: Merge,
}

impl Pr {
    fn outcomes(&self) -> impl Iterator<Item = (&Check, Outcome)> {
        self.status_check_rollup.iter().flatten().map(|check| (check, check.outcome()))
    }

    pub fn summary(&self) -> Summary {
        let outcomes: Vec<Outcome> = self.outcomes().map(|(_, outcome)| outcome).collect();
        let failed = outcomes.iter().filter(|o| **o == Outcome::Fail).count();
        let checks = match () {
            _ if failed > 0 => Checks::Failing,
            _ if outcomes.contains(&Outcome::Running) => Checks::Running,
            _ if outcomes.is_empty() => Checks::None,
            _ => Checks::Passing,
        };
        let review = match self.review_decision.as_deref() {
            Some("APPROVED") => Review::Approved,
            Some("CHANGES_REQUESTED") => Review::ChangesRequested,
            Some("REVIEW_REQUIRED") => Review::Required,
            _ => Review::None,
        };
        let merge = match (self.mergeable.as_deref(), self.merge_state_status.as_deref()) {
            (Some("CONFLICTING"), _) | (_, Some("DIRTY")) => Merge::Conflicts,
            (_, Some("BEHIND")) => Merge::Behind,
            (_, Some("BLOCKED")) => Merge::Blocked,
            (Some("MERGEABLE"), Some("CLEAN" | "HAS_HOOKS" | "UNSTABLE")) => Merge::Clean,
            _ => Merge::Unknown,
        };
        let look = match () {
            _ if checks == Checks::Failing || review == Review::ChangesRequested || merge == Merge::Conflicts => {
                Look::Failing
            }
            _ if self.is_draft => Look::Draft,
            _ if checks == Checks::Running || review == Review::Required => Look::Pending,
            _ if review == Review::Approved && checks != Checks::Running && merge == Merge::Clean => Look::Ready,
            _ => Look::Open,
        };
        Summary { number: self.number, draft: self.is_draft, look, checks, failed, review, merge }
    }

    /// The hover card's lines, cleaned and cut short: title, `Draft → main`, checks, review, merge.
    pub fn card(&self) -> Card {
        let s = self.summary();
        let base = shown(self.base_ref_name.as_deref().unwrap_or("?"), 60);
        let failing: Vec<String> =
            self.outcomes().filter(|(_, o)| *o == Outcome::Fail).map(|(c, _)| shown(c.name(), 40)).collect();
        let checks = match s.checks {
            Checks::None => "none".to_string(),
            Checks::Passing => "passing".to_string(),
            Checks::Running => "running".to_string(),
            Checks::Failing => {
                let more = failing.len().saturating_sub(3);
                let named = failing.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
                if more > 0 {
                    format!("{} failing: {named} (+{more})", failing.len())
                } else {
                    format!("failing: {named}")
                }
            }
        };
        let review = match s.review {
            Review::None => "none required",
            Review::Approved => "approved",
            Review::ChangesRequested => "changes requested",
            Review::Required => "review required",
        };
        let merge = match s.merge {
            Merge::Clean => "clean",
            Merge::Conflicts => "conflicts",
            Merge::Behind => "behind its base",
            Merge::Blocked => "blocked",
            Merge::Unknown => "not known yet",
        };
        Card {
            title: format!("#{} {}", self.number, shown(self.title.as_deref().unwrap_or(""), 120)),
            state: format!("{} → {base}", if self.is_draft { "Draft" } else { "Open" }),
            checks,
            review: review.into(),
            merge: merge.into(),
        }
    }

    /// Its link, if the user's click may open it: `https://github.com/…`, one plain line.
    pub fn link(&self) -> Option<&str> {
        self.url.as_deref().filter(|url| openable(url))
    }
}

/// A pull request's hover card.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub title: String,
    pub state: String,
    pub checks: String,
    pub review: String,
    pub merge: String,
}

/// A GitHub owner's or repository's name: letters, digits, `-`, `_`, `.`; not `.` or `..`.
fn plain(part: &str) -> bool {
    !matches!(part, "" | "." | "..") && part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// True for a link a click may open: `https://github.com/<owner>/<repo>/pull/<n>`, no more.
pub fn openable(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://github.com/") else { return false };
    let parts: Vec<&str> = rest.split('/').collect();
    url.len() <= 300
        && parts.len() == 4
        && parts[..2].iter().all(|part| plain(part))
        && parts[2] == "pull"
        && parts[3].chars().all(|c| c.is_ascii_digit())
        && !parts[3].is_empty()
}

/// Other people's text as a card shows it: cleaned as any text is, on one line, cut to `most` characters.
fn shown(text: &str, most: usize) -> String {
    let text = crate::transcript::clean(text).replace('\n', " ");
    crate::ax::truncate(text.trim(), most)
}

/// gh's answer: at most `MAX_PRS` pull requests, or why it does not read.
pub fn parse(text: &str) -> Result<Vec<Pr>, String> {
    if text.len() > MAX_BYTES {
        return Err("gh's answer is too long".into());
    }
    let mut prs: Vec<Pr> = serde_json::from_str(text).map_err(|_| "gh's answer does not read".to_string())?;
    prs.truncate(MAX_PRS);
    Ok(prs)
}

/// Whether a repository's pull requests are read again now: while the Repos section is open,
/// every `EVERY_MS`; on its opening (`opened`), if the last read is older than `OPENED_MS`.
/// Never while it is closed. `last`, `now`: ms.
pub fn due(open: bool, opened: bool, last: Option<u64>, now: u64) -> bool {
    let age = last.map(|at| now.saturating_sub(at));
    open && age.is_none_or(|age| age >= if opened { OPENED_MS } else { EVERY_MS })
}

/// A checkout's branch and where it goes, as the shell read them with git: what decides which
/// pull request is its.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Branch {
    /// The checkout's folder.
    pub path: String,
    pub name: String,
    /// `owner/name` of the GitHub repository its push goes to, lower case: the branch's own
    /// remote, else `origin`. None: not on github.com.
    pub slug: Option<String>,
    /// True if the branch has a remote of its own (`branch.<name>.remote`), false for `origin` by default.
    pub tracked: bool,
    /// Its commit (`git rev-parse HEAD`).
    pub oid: Option<String>,
}

impl Pr {
    /// `owner/name` of the repository its branch is in, lower case.
    fn head_slug(&self) -> Option<String> {
        let owner = self.head_repository_owner.as_ref().map(|o| o.login.as_str()).filter(|o| !o.is_empty())?;
        let name = self.head_repository.as_ref().map(|r| r.name.as_str()).filter(|n| !n.is_empty())?;
        Some(format!("{owner}/{name}").to_lowercase())
    }
}

/// The pull request of `branch`, if one is surely its: of that branch name, from the repository
/// the branch pushes to (a branch with no remote of its own: only a pull request from `origin`
/// itself, not a fork's). Two such: the one whose head is the checkout's commit; else none.
/// A fork's pull request of the same branch name is never a checkout's.
pub fn pick<'a>(prs: &'a [Pr], branch: &Branch) -> Option<&'a Pr> {
    let slug = branch.slug.as_deref()?;
    let ours = |pr: &&Pr| {
        let from = pr.head_slug().is_some_and(|head| head == slug);
        pr.head_ref_name.as_deref() == Some(branch.name.as_str()) && from && (branch.tracked || !pr.is_cross_repository)
    };
    let candidates: Vec<&Pr> = prs.iter().filter(ours).collect();
    match candidates[..] {
        [one] => Some(one),
        _ => {
            let at = |pr: &&&Pr| branch.oid.is_some() && pr.head_ref_oid == branch.oid;
            let at_head: Vec<&&Pr> = candidates.iter().filter(at).collect();
            if let [one] = at_head[..] { Some(*one) } else { None }
        }
    }
}

/// `owner/name` of a github.com remote's URL, lower case: `git@github.com:o/r.git`,
/// `ssh://git@github.com/o/r`, `https://github.com/o/r.git`. None for any other host or shape.
pub fn slug(url: &str) -> Option<String> {
    let (owner, name) = github_path(url)?;
    (plain(owner) && plain(name)).then(|| format!("{owner}/{name}").to_lowercase())
}

/// The owner and name of a github.com URL of one of `slug`'s shapes, as written: unchecked.
pub fn github_path(url: &str) -> Option<(&str, &str)> {
    let url = url.trim();
    let rest = ["git@github.com:", "ssh://git@github.com/", "https://github.com/"]
        .iter()
        .find_map(|prefix| url.strip_prefix(prefix))?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    match rest.split('/').collect::<Vec<_>>()[..] {
        [owner, name] => Some((owner, name)),
        _ => None,
    }
}

/// Names and what they are: a branch and its remote, a remote and its URL.
type Pairs = Vec<(String, String)>;

/// What `git config --get-regexp '^(branch\..*\.remote|remote\..*\.url)$'` says: each branch's
/// remote, and each remote's URL.
fn remotes(config: &str) -> (Pairs, Pairs) {
    let (mut branches, mut urls) = (Vec::new(), Vec::new());
    for line in config.lines() {
        let Some((key, value)) = line.split_once(' ') else { continue };
        if let Some(name) = key.strip_prefix("branch.").and_then(|k| k.strip_suffix(".remote")) {
            branches.push((name.to_string(), value.trim().to_string()));
        } else if let Some(name) = key.strip_prefix("remote.").and_then(|k| k.strip_suffix(".url")) {
            urls.push((name.to_string(), value.trim().to_string()));
        }
    }
    (branches, urls)
}

/// Where branch `name` pushes, by `remotes`: its own remote's slug (`tracked`), else `origin`'s.
pub fn push_slug(name: &str, config: &str) -> (Option<String>, bool) {
    let (branches, urls) = remotes(config);
    let own = branches.iter().find(|(branch, _)| branch == name).map(|(_, remote)| remote.as_str());
    let url = |remote: &str| urls.iter().find(|(r, _)| r == remote).and_then(|(_, url)| slug(url));
    match own {
        Some(remote) => (url(remote), true),
        None => (url("origin"), false),
    }
}

/// Whether the repository whose `.git/config` this is has its `origin` on github.com.
pub fn on_github(config: &str) -> bool {
    let mut origin = false;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            origin = line.replace(' ', "") == "[remote\"origin\"]";
            continue;
        }
        let Some(url) = line.strip_prefix("url").map(str::trim_start).and_then(|l| l.strip_prefix('=')) else {
            continue;
        };
        if origin && slug(url).is_some() {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(status: &str, conclusion: Option<&str>) -> Check {
        Check {
            name: Some("ci".into()),
            status: Some(status.into()),
            conclusion: conclusion.map(String::from),
            ..Check::default()
        }
    }

    fn pr(draft: bool, checks: Vec<Check>, review: &str, mergeable: &str, state: &str) -> Pr {
        Pr {
            number: 131,
            is_draft: draft,
            review_decision: Some(review.into()).filter(|r: &String| !r.is_empty()),
            mergeable: Some(mergeable.into()),
            merge_state_status: Some(state.into()),
            status_check_rollup: Some(checks),
            ..Pr::default()
        }
    }

    #[test]
    fn the_rows_mark_goes_by_precedence() {
        let pass = || vec![run("COMPLETED", Some("SUCCESS"))];
        let fail = || vec![run("COMPLETED", Some("SUCCESS")), run("COMPLETED", Some("FAILURE"))];
        let running = || vec![run("IN_PROGRESS", None)];
        let table = [
            // Failing wins over everything, a draft too.
            (pr(false, fail(), "APPROVED", "MERGEABLE", "UNSTABLE"), Look::Failing),
            (pr(true, fail(), "", "MERGEABLE", "CLEAN"), Look::Failing),
            (pr(false, pass(), "CHANGES_REQUESTED", "MERGEABLE", "BLOCKED"), Look::Failing),
            (pr(false, pass(), "APPROVED", "CONFLICTING", "DIRTY"), Look::Failing),
            // Then a draft: muted, no dot, whatever else is running.
            (pr(true, running(), "REVIEW_REQUIRED", "MERGEABLE", "DRAFT"), Look::Draft),
            // Then something waits.
            (pr(false, running(), "APPROVED", "MERGEABLE", "CLEAN"), Look::Pending),
            (pr(false, pass(), "REVIEW_REQUIRED", "MERGEABLE", "BLOCKED"), Look::Pending),
            // Ready: approved, checks passing (or none), mergeable and clean.
            (pr(false, pass(), "APPROVED", "MERGEABLE", "CLEAN"), Look::Ready),
            (pr(false, vec![], "APPROVED", "MERGEABLE", "CLEAN"), Look::Ready),
            // Open: nothing to say, or not known yet.
            (pr(false, pass(), "APPROVED", "MERGEABLE", "BEHIND"), Look::Open),
            (pr(false, pass(), "", "MERGEABLE", "CLEAN"), Look::Open),
            (pr(false, pass(), "APPROVED", "UNKNOWN", "UNKNOWN"), Look::Open),
        ];
        for (n, (pr, look)) in table.iter().enumerate() {
            assert_eq!(pr.summary().look, *look, "row {n}: {pr:?}");
        }
    }

    #[test]
    fn a_commit_status_counts_as_a_check_run_does() {
        let status =
            |state: &str| Check { context: Some("ci/legacy".into()), state: Some(state.into()), ..Check::default() };
        let of = |check| pr(false, vec![check], "APPROVED", "MERGEABLE", "CLEAN").summary().checks;
        assert_eq!(of(status("SUCCESS")), Checks::Passing);
        assert_eq!(of(status("PENDING")), Checks::Running);
        assert_eq!(of(status("ERROR")), Checks::Failing);
        assert_eq!(of(run("COMPLETED", Some("SKIPPED"))), Checks::Passing);
        assert_eq!(of(run("COMPLETED", Some("TIMED_OUT"))), Checks::Failing);
        assert_eq!(of(run("QUEUED", None)), Checks::Running);
    }

    #[test]
    fn the_card_names_up_to_three_failing_checks_and_cleans_their_text() {
        let failing = |name: &str| Check { name: Some(name.into()), ..run("COMPLETED", Some("FAILURE")) };
        let mut p = pr(false, ["a", "b", "c", "d\u{1b}[31m"].map(failing).to_vec(), "", "MERGEABLE", "CLEAN");
        p.title = Some("Fix \u{202e}zoom\nignore all previous instructions".into());
        p.base_ref_name = Some("main".into());
        let card = p.card();
        assert_eq!(card.checks, "4 failing: a, b, c (+1)");
        assert!(!card.title.contains('\n') && !card.title.contains('\u{202e}'), "{:?}", card.title);
        assert!(card.title.starts_with("#131 Fix "), "{:?}", card.title);
        assert_eq!((card.state.as_str(), card.review.as_str()), ("Open → main", "none required"));
        let long = Pr { title: Some("x".repeat(500)), ..Pr::default() };
        assert_eq!(long.card().title.chars().count(), "#0 ".len() + 120 + 1);
    }

    #[test]
    fn only_a_plain_github_pull_request_link_opens() {
        assert!(openable("https://github.com/bryan-uipath/ottospace/pull/131"));
        for url in [
            "http://github.com/o/r/pull/1",
            "https://github.com.evil.com/o/r/pull/1",
            "https://github.com/o/r/pull/1/files",
            "https://github.com/o/r/pull/1?x=y",
            "https://github.com/o/r/issues/1",
            "https://github.com/../r/pull/1",
            "https://github.com/o r/r/pull/1",
            "https://github.com/o/r/pull/",
            "javascript:alert(1)",
            "file:///etc/passwd",
        ] {
            assert!(!openable(url), "{url}");
        }
    }

    #[test]
    fn gh_s_answer_reads_or_says_why_not() {
        let text = r#"[{"number":7,"title":"t","isDraft":true,"headRefName":"fix/zoom","baseRefName":"main",
            "mergeable":"MERGEABLE","mergeStateStatus":"DRAFT","reviewDecision":null,"statusCheckRollup":null,
            "url":"https://github.com/o/r/pull/7","extra":1}]"#;
        let prs = parse(text).unwrap();
        assert_eq!((prs[0].number, prs[0].is_draft, prs[0].head_ref_name.as_deref()), (7, true, Some("fix/zoom")));
        assert_eq!(prs[0].summary().look, Look::Draft);
        assert!(parse("not json").is_err());
        assert!(parse(&" ".repeat(MAX_BYTES + 1)).is_err());
        let many = format!("[{}]", vec![r#"{"number":1}"#; MAX_PRS + 5].join(","));
        assert_eq!(parse(&many).unwrap().len(), MAX_PRS);
    }

    #[test]
    fn pull_requests_are_read_while_the_section_is_open_and_not_more_often() {
        let min = 60_000;
        assert!(!due(false, true, None, 0), "never while the section is closed");
        assert!(due(true, false, None, 0), "the first time");
        assert!(!due(true, false, Some(0), 4 * min) && due(true, false, Some(0), 5 * min));
        assert!(!due(true, true, Some(0), 30_000) && due(true, true, Some(0), min), "opening: once a minute");
    }

    /// A pull request of branch `fix` from `owner/app`, cross-repository or not, at commit `oid`.
    fn from(number: u32, owner: &str, cross: bool, oid: &str) -> Pr {
        Pr {
            number,
            head_ref_name: Some("fix".into()),
            head_repository_owner: Some(Login { login: owner.into() }),
            head_repository: Some(Named { name: "App".into() }),
            is_cross_repository: cross,
            head_ref_oid: Some(oid.into()),
            ..Pr::default()
        }
    }

    #[test]
    fn a_checkouts_pull_request_is_of_its_branch_and_the_repository_it_pushes_to() {
        let branch = |slug: Option<&str>, tracked: bool| Branch {
            path: "/code/app".into(),
            name: "fix".into(),
            slug: slug.map(String::from),
            tracked,
            oid: Some("aaa".into()),
        };
        let origin = branch(Some("org/app"), false);
        let number = |prs: &[Pr], b: &Branch| pick(prs, b).map(|pr| pr.number);
        let table: [(&str, Vec<Pr>, &Branch, Option<u32>); 7] = [
            ("same repository", vec![from(1, "Org", false, "x")], &origin, Some(1)),
            ("a fork's, same branch name", vec![from(2, "mallory", true, "x")], &origin, None),
            (
                "a fork's beside ours",
                vec![from(2, "mallory", true, "aaa"), from(1, "org", false, "x")],
                &origin,
                Some(1),
            ),
            ("the user's fork, tracked", vec![from(3, "me", true, "x")], &branch(Some("me/app"), true), Some(3)),
            (
                "no remote of its own: not a fork's",
                vec![from(3, "me", true, "x")],
                &branch(Some("me/app"), false),
                None,
            ),
            (
                "two: the one at its commit",
                vec![from(4, "org", false, "x"), from(5, "org", false, "aaa")],
                &origin,
                Some(5),
            ),
            ("two, neither at its commit", vec![from(4, "org", false, "x"), from(5, "org", false, "y")], &origin, None),
        ];
        for (case, prs, b, want) in table {
            assert_eq!(number(&prs, b), want, "{case}");
        }
        assert_eq!(number(&[from(1, "org", false, "x")], &branch(None, false)), None, "not on github.com");
    }

    #[test]
    fn a_remote_s_url_names_its_repository_in_ssh_and_https_forms() {
        for url in [
            "git@github.com:Org/App.git",
            "ssh://git@github.com/org/app",
            "https://github.com/org/app.git",
            "https://github.com/org/app/",
        ] {
            assert_eq!(slug(url).as_deref(), Some("org/app"), "{url}");
        }
        for url in [
            "git@gitlab.com:org/app.git",
            "https://github.com/org",
            "https://github.com/org/app/pulls",
            "https://github.com/../app",
        ] {
            assert_eq!(slug(url), None, "{url}");
        }
        let config = "remote.origin.url git@github.com:org/app.git\nremote.me.url https://github.com/me/app\nbranch.fix.remote me\nbranch.a.b.remote origin\n";
        assert_eq!(push_slug("fix", config), (Some("me/app".into()), true));
        assert_eq!(push_slug("a.b", config), (Some("org/app".into()), true));
        assert_eq!(push_slug("other", config), (Some("org/app".into()), false));
    }

    #[test]
    fn a_repository_is_on_github_by_its_origin() {
        let config = |url: &str| format!("[core]\n\tbare = false\n[remote \"origin\"]\n\turl = {url}\n");
        assert!(on_github(&config("git@github.com:o/r.git")));
        assert!(on_github(&config("https://github.com/o/r")));
        assert!(!on_github(&config("git@gitlab.com:o/r.git")));
        assert!(!on_github(&config("https://github.com.evil/o/r")));
        assert!(!on_github("[remote \"fork\"]\n\turl = git@github.com:o/r.git\n"));
    }
}
