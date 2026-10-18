//! GitHub integration through the GitHub CLI (`gh`).
//!
//! Gitter never stores tokens: every request goes through the user's existing
//! `gh` login, the same way the git backend relies on the user's credential
//! helpers. Read requests use `--json` output; checkout and create shell out to
//! `gh pr checkout` / `gh pr create` so they behave exactly like the CLI.

use crate::git::{parse_diff, ChangedFile, FileDiff, FileStatus, GitError, Remote};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Deserializer};
use std::path::Path;
use std::process::{Command, Stdio};

/// A repository on GitHub (or a GitHub Enterprise host).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubRepo {
    pub host: String,
    pub owner: String,
    pub name: String,
    /// The local remote that points at it.
    pub remote: String,
}

impl GitHubRepo {
    /// `HOST/OWNER/REPO`, the form `gh --repo` accepts.
    pub fn spec(&self) -> String {
        format!("{}/{}/{}", self.host, self.owner, self.name)
    }

    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// Maps a remote host to the GitHub host `gh` knows it by.
fn github_host(host: &str) -> Option<String> {
    let h = host.to_ascii_lowercase();
    if matches!(h.as_str(), "github.com" | "www.github.com" | "ssh.github.com") {
        return Some("github.com".into());
    }
    // SSH config aliases for multiple accounts, e.g. git@github-work:owner/repo.
    if h.starts_with("github-") || h.starts_with("github_") || h.starts_with("github.com-") {
        return Some("github.com".into());
    }
    // GitHub Enterprise Cloud (*.ghe.com) and Enterprise Server installs.
    if h.ends_with(".ghe.com") || (h.contains("github") && h.contains('.')) {
        return Some(h);
    }
    None
}

/// Parses a remote URL into `(host, owner, repo)` when it points at GitHub.
pub fn parse_github_url(url: &str) -> Option<(String, String, String)> {
    let web = crate::repo::menus::web_url(url)?;
    let rest = web.split_once("://")?.1;
    let (host, path) = rest.split_once('/')?;
    let host = github_host(host.split(':').next().unwrap_or(host))?;
    let mut parts = path.trim_matches('/').split('/').filter(|p| !p.is_empty());
    let owner = parts.next()?.to_string();
    let name = parts.next()?.trim_end_matches(".git").to_string();
    if name.is_empty() {
        return None;
    }
    Some((host, owner, name))
}

/// Picks the GitHub repository pull requests belong to: `upstream` for the
/// usual fork setup, then `origin`, then any other GitHub remote.
pub fn find_github_repo(remotes: &[Remote]) -> Option<GitHubRepo> {
    let rank = |name: &str| match name {
        "upstream" => 0,
        "origin" => 1,
        _ => 2,
    };
    let mut candidates: Vec<&Remote> = remotes.iter().collect();
    candidates.sort_by_key(|r| rank(&r.name));
    candidates.into_iter().find_map(|r| {
        let (host, owner, name) = parse_github_url(&r.fetch_url)?;
        Some(GitHubRepo { host, owner, name, remote: r.name.clone() })
    })
}

// -------------------------------------------------------------------------
// Running gh
// -------------------------------------------------------------------------

fn gh_command(cwd: Option<&Path>) -> Command {
    let mut cmd = Command::new("gh");
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    cmd.env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_SPINNER_DISABLED", "1")
        .env("NO_COLOR", "1")
        .env("CLICOLOR", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Runs gh and returns stdout, or a [`GitError`] carrying gh's message.
fn run_gh(cwd: Option<&Path>, args: &[&str]) -> Result<String> {
    let out = gh_command(cwd).args(args).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow!("The GitHub CLI (gh) is not installed or not on your PATH.")
        } else {
            anyhow!("Failed to run gh: {e}")
        }
    })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let mut message = stderr.trim().to_string();
        if message.is_empty() {
            message = stdout.trim().to_string();
        }
        if message.is_empty() {
            message = format!("gh exited with code {}", out.status.code().unwrap_or(-1));
        }
        return Err(GitError { command: format!("gh {}", args.join(" ")), message }.into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GhStatus {
    /// `gh` isn't installed (or isn't on PATH).
    NotInstalled,
    /// `gh` runs but has no working login for the host; carries gh's explanation.
    NotLoggedIn(String),
    Ready,
}

/// Checks that gh is installed and logged in to `host`.
pub fn gh_status(host: &str) -> GhStatus {
    match gh_command(None).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status() {
        Ok(s) if s.success() => {}
        _ => return GhStatus::NotInstalled,
    }
    match run_gh(None, &["auth", "status", "--hostname", host]) {
        Ok(_) => GhStatus::Ready,
        Err(e) => {
            let message = e.downcast_ref::<GitError>().map(|g| g.message.clone()).unwrap_or_else(|| e.to_string());
            auth_failure(host, &message)
        }
    }
}

/// Interprets a failed `gh auth status`. Being offline isn't a login problem:
/// that reports Ready so the next request shows the real network error.
fn auth_failure(host: &str, message: &str) -> GhStatus {
    let lower = message.to_lowercase();
    if lower.contains("timeout") || lower.contains("error connecting") {
        return GhStatus::Ready;
    }
    // gh prints the host name as a heading, then indented "X ..." / "- ..." details.
    let fields = ["Active account", "Git operations protocol", "Token"];
    let summary: Vec<&str> = message
        .lines()
        .map(|l| l.trim().trim_start_matches(['X', '-', '✓']).trim())
        .filter(|l| !l.is_empty() && *l != host && !fields.iter().any(|f| l.starts_with(f)))
        .take(2)
        .collect();
    GhStatus::NotLoggedIn(summary.join(" "))
}

// -------------------------------------------------------------------------
// Data
// -------------------------------------------------------------------------

/// Treats JSON `null` like a missing field.
fn nullable<'de, D: Deserializer<'de>, T: Default + Deserialize<'de>>(d: D) -> std::result::Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Actor {
    #[serde(deserialize_with = "nullable")]
    pub login: String,
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    #[serde(deserialize_with = "nullable")]
    pub slug: String,
}

impl Actor {
    /// Login for users, slug or name for teams.
    pub fn display(&self) -> &str {
        [&self.login, &self.slug, &self.name].into_iter().find(|s| !s.is_empty()).map(|s| s.as_str()).unwrap_or("ghost")
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Label {
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    #[serde(deserialize_with = "nullable")]
    pub color: String,
}

/// A CheckRun (GitHub Actions and apps) or a commit StatusContext.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct CheckItem {
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    #[serde(deserialize_with = "nullable")]
    pub context: String,
    #[serde(deserialize_with = "nullable")]
    pub workflow_name: String,
    #[serde(deserialize_with = "nullable")]
    pub status: String,
    #[serde(deserialize_with = "nullable")]
    pub conclusion: String,
    #[serde(deserialize_with = "nullable")]
    pub state: String,
    #[serde(deserialize_with = "nullable")]
    pub details_url: String,
    #[serde(deserialize_with = "nullable")]
    pub target_url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    Success,
    Failure,
    Pending,
    /// Skipped or neutral: neither passes nor fails the PR.
    Neutral,
}

impl CheckItem {
    pub fn label(&self) -> &str {
        if self.name.is_empty() {
            &self.context
        } else {
            &self.name
        }
    }

    pub fn url(&self) -> &str {
        if self.details_url.is_empty() {
            &self.target_url
        } else {
            &self.details_url
        }
    }

    pub fn outcome(&self) -> CheckOutcome {
        if !self.state.is_empty() {
            // StatusContext
            return match self.state.as_str() {
                "SUCCESS" => CheckOutcome::Success,
                "FAILURE" | "ERROR" => CheckOutcome::Failure,
                _ => CheckOutcome::Pending,
            };
        }
        if self.status != "COMPLETED" {
            return CheckOutcome::Pending;
        }
        match self.conclusion.as_str() {
            "SUCCESS" => CheckOutcome::Success,
            "NEUTRAL" | "SKIPPED" | "STALE" => CheckOutcome::Neutral,
            _ => CheckOutcome::Failure,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChecksSummary {
    pub passed: usize,
    pub failed: usize,
    pub pending: usize,
    pub neutral: usize,
}

impl ChecksSummary {
    pub fn total(&self) -> usize {
        self.passed + self.failed + self.pending + self.neutral
    }

    /// The overall state GitHub shows next to a PR: any failure wins, then pending.
    pub fn overall(&self) -> Option<CheckOutcome> {
        if self.total() == 0 {
            None
        } else if self.failed > 0 {
            Some(CheckOutcome::Failure)
        } else if self.pending > 0 {
            Some(CheckOutcome::Pending)
        } else {
            Some(CheckOutcome::Success)
        }
    }

    pub fn describe(&self) -> String {
        match self.overall() {
            None => "No checks".into(),
            Some(CheckOutcome::Failure) => format!("{} of {} checks failed", self.failed, self.total()),
            Some(CheckOutcome::Pending) => format!("{} of {} checks pending", self.pending, self.total()),
            _ => {
                let ok = self.passed + self.neutral;
                if self.neutral > 0 {
                    format!("{ok} checks passed ({} skipped)", self.neutral)
                } else {
                    format!("{ok} checks passed")
                }
            }
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Review {
    pub author: Actor,
    #[serde(deserialize_with = "nullable")]
    pub state: String,
    #[serde(deserialize_with = "nullable")]
    pub submitted_at: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct PrFile {
    #[serde(deserialize_with = "nullable")]
    pub path: String,
    pub additions: u32,
    pub deletions: u32,
    #[serde(rename = "changeType", deserialize_with = "nullable")]
    pub change_type: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct PrCommit {
    #[serde(deserialize_with = "nullable")]
    pub oid: String,
}

/// A pull request. List queries fill a subset of the fields; `gh pr view`
/// fills the rest (body, files, reviews, commits).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u64,
    #[serde(deserialize_with = "nullable")]
    pub title: String,
    #[serde(deserialize_with = "nullable")]
    pub body: String,
    #[serde(deserialize_with = "nullable")]
    pub author: Actor,
    #[serde(deserialize_with = "nullable")]
    pub state: String,
    pub is_draft: bool,
    #[serde(deserialize_with = "nullable")]
    pub url: String,
    #[serde(deserialize_with = "nullable")]
    pub head_ref_name: String,
    #[serde(deserialize_with = "nullable")]
    pub head_ref_oid: String,
    #[serde(deserialize_with = "nullable")]
    pub base_ref_name: String,
    #[serde(deserialize_with = "nullable")]
    pub base_ref_oid: String,
    pub is_cross_repository: bool,
    #[serde(deserialize_with = "nullable")]
    pub head_repository_owner: Actor,
    #[serde(deserialize_with = "nullable")]
    pub review_decision: String,
    #[serde(deserialize_with = "nullable")]
    pub status_check_rollup: Vec<CheckItem>,
    #[serde(deserialize_with = "nullable")]
    pub labels: Vec<Label>,
    #[serde(deserialize_with = "nullable")]
    pub created_at: String,
    #[serde(deserialize_with = "nullable")]
    pub updated_at: String,
    pub additions: u32,
    pub deletions: u32,
    #[serde(deserialize_with = "nullable")]
    pub files: Vec<PrFile>,
    #[serde(deserialize_with = "nullable")]
    pub commits: Vec<PrCommit>,
    #[serde(deserialize_with = "nullable")]
    pub latest_reviews: Vec<Review>,
    #[serde(deserialize_with = "nullable")]
    pub review_requests: Vec<Actor>,
    #[serde(deserialize_with = "nullable")]
    pub mergeable: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
}

impl PrState {
    pub fn label(self) -> &'static str {
        match self {
            PrState::Open => "Open",
            PrState::Draft => "Draft",
            PrState::Merged => "Merged",
            PrState::Closed => "Closed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewDecision {
    Approved,
    ChangesRequested,
    ReviewRequired,
}

impl ReviewDecision {
    pub fn label(self) -> &'static str {
        match self {
            ReviewDecision::Approved => "Approved",
            ReviewDecision::ChangesRequested => "Changes requested",
            ReviewDecision::ReviewRequired => "Review required",
        }
    }
}

impl PullRequest {
    pub fn pr_state(&self) -> PrState {
        match self.state.as_str() {
            "MERGED" => PrState::Merged,
            "CLOSED" => PrState::Closed,
            _ if self.is_draft => PrState::Draft,
            _ => PrState::Open,
        }
    }

    pub fn review_decision(&self) -> Option<ReviewDecision> {
        match self.review_decision.as_str() {
            "APPROVED" => Some(ReviewDecision::Approved),
            "CHANGES_REQUESTED" => Some(ReviewDecision::ChangesRequested),
            "REVIEW_REQUIRED" => Some(ReviewDecision::ReviewRequired),
            _ => None,
        }
    }

    pub fn checks(&self) -> ChecksSummary {
        let mut s = ChecksSummary::default();
        for c in &self.status_check_rollup {
            match c.outcome() {
                CheckOutcome::Success => s.passed += 1,
                CheckOutcome::Failure => s.failed += 1,
                CheckOutcome::Pending => s.pending += 1,
                CheckOutcome::Neutral => s.neutral += 1,
            }
        }
        s
    }

    /// The head branch as shown on GitHub: `owner:branch` for forks.
    pub fn head_label(&self) -> String {
        if self.is_cross_repository && !self.head_repository_owner.login.is_empty() {
            format!("{}:{}", self.head_repository_owner.login, self.head_ref_name)
        } else {
            self.head_ref_name.clone()
        }
    }

    pub fn updated_time(&self) -> i64 {
        parse_time(&self.updated_at)
    }

    pub fn created_time(&self) -> i64 {
        parse_time(&self.created_at)
    }
}

pub fn parse_time(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s).map(|t| t.timestamp()).unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrFilter {
    Open,
    Closed,
    All,
}

impl PrFilter {
    fn arg(self) -> &'static str {
        match self {
            PrFilter::Open => "open",
            PrFilter::Closed => "closed",
            PrFilter::All => "all",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PrFilter::Open => "Open",
            PrFilter::Closed => "Closed",
            PrFilter::All => "All",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RepoInfo {
    pub default_branch: Option<String>,
}

// -------------------------------------------------------------------------
// Requests
// -------------------------------------------------------------------------

const LIST_FIELDS: &str = "number,title,author,state,isDraft,url,headRefName,headRefOid,baseRefName,isCrossRepository,headRepositoryOwner,reviewDecision,labels,createdAt,updatedAt";
const VIEW_FIELDS: &str = "number,title,body,author,state,isDraft,url,headRefName,headRefOid,baseRefName,baseRefOid,isCrossRepository,headRepositoryOwner,reviewDecision,statusCheckRollup,labels,createdAt,updatedAt,additions,deletions,files,commits,latestReviews,reviewRequests,mergeable";

pub fn parse_pull_requests(json: &str) -> Result<Vec<PullRequest>> {
    Ok(serde_json::from_str(json)?)
}

pub fn list_pull_requests(repo: &GitHubRepo, filter: PrFilter, limit: usize) -> Result<Vec<PullRequest>> {
    let limit = limit.to_string();
    let list = |fields: &str| -> Result<Vec<PullRequest>> {
        let out = run_gh(None, &["pr", "list", "--repo", &repo.spec(), "--state", filter.arg(), "--limit", &limit, "--json", fields])?;
        parse_pull_requests(&out)
    };
    // Check rollups are expensive for GitHub to compute: busy repositories time
    // out (HTTP 502/504) when asked for them on a page of closed PRs. Only open
    // PRs get them in the list, with a retry without them; the detail view
    // always shows the checks.
    if filter != PrFilter::Open {
        return list(LIST_FIELDS);
    }
    match list(&format!("{LIST_FIELDS},statusCheckRollup")) {
        Err(e) if is_gateway_timeout(&e) => list(LIST_FIELDS),
        result => result,
    }
}

fn is_gateway_timeout(e: &anyhow::Error) -> bool {
    let text = e.to_string();
    ["HTTP 502", "HTTP 503", "HTTP 504", "timed out", "timeout"].iter().any(|s| text.contains(s))
}

pub fn view_pull_request(repo: &GitHubRepo, number: u64) -> Result<PullRequest> {
    let n = number.to_string();
    let out = run_gh(None, &["pr", "view", &n, "--repo", &repo.spec(), "--json", VIEW_FIELDS])?;
    Ok(serde_json::from_str(&out)?)
}

/// The PR's changes as GitHub shows them (merge base to head), one entry per file.
pub fn pull_request_diff(repo: &GitHubRepo, number: u64) -> Result<Vec<FileDiff>> {
    let n = number.to_string();
    let out = run_gh(None, &["pr", "diff", &n, "--repo", &repo.spec(), "--color=never"])?;
    Ok(parse_diff(out.as_bytes()))
}

pub fn repo_info(repo: &GitHubRepo) -> Result<RepoInfo> {
    #[derive(Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    struct Raw {
        #[serde(deserialize_with = "nullable")]
        default_branch_ref: Actor,
    }
    let out = run_gh(None, &["repo", "view", &repo.spec(), "--json", "defaultBranchRef"])?;
    let raw: Raw = serde_json::from_str(&out)?;
    Ok(RepoInfo { default_branch: Some(raw.default_branch_ref.name).filter(|n| !n.is_empty()) })
}

/// Checks out a PR's head into a local branch (handles forks), from inside the working tree.
pub fn checkout_pull_request(workdir: &Path, repo: &GitHubRepo, number: u64) -> Result<String> {
    let n = number.to_string();
    run_gh(Some(workdir), &["pr", "checkout", &n, "--repo", &repo.spec()])
}

pub struct NewPullRequest {
    /// `branch`, or `owner:branch` when the branch lives in a fork.
    pub head: String,
    pub base: String,
    pub title: String,
    pub body: String,
    pub draft: bool,
}

/// Opens a pull request and returns its URL. The head branch must already be pushed.
pub fn create_pull_request(workdir: &Path, repo: &GitHubRepo, pr: &NewPullRequest) -> Result<String> {
    let spec = repo.spec();
    let mut args = vec!["pr", "create", "--repo", &spec, "--head", &pr.head, "--base", &pr.base, "--title", &pr.title, "--body", &pr.body];
    if pr.draft {
        args.push("--draft");
    }
    let out = run_gh(Some(workdir), &args)?;
    out.lines().rev().map(str::trim).find(|l| l.starts_with("http")).map(String::from).ok_or_else(|| anyhow!("gh did not report the new pull request's URL:\n{}", out.trim()))
}

/// The PR number at the end of a pull request URL.
pub fn number_from_url(url: &str) -> Option<u64> {
    url.trim().trim_end_matches('/').rsplit('/').next()?.parse().ok()
}

/// Files changed by a PR, from its diff when available (it knows renames),
/// filled in with GitHub's per-file line counts and any files the diff left out.
pub fn changed_files(pr: &PullRequest, diffs: Option<&[FileDiff]>) -> Vec<ChangedFile> {
    let stats = |path: &str| pr.files.iter().find(|f| f.path == path);
    let mut files: Vec<ChangedFile> = Vec::new();
    for d in diffs.unwrap_or_default() {
        let status = if d.new_file {
            FileStatus::Added
        } else if d.deleted {
            FileStatus::Deleted
        } else if d.old_path.is_some() {
            FileStatus::Renamed
        } else {
            FileStatus::Modified
        };
        let (additions, deletions) = match stats(&d.path) {
            Some(f) => (Some(f.additions), Some(f.deletions)),
            None if d.binary => (None, None),
            None => {
                let (a, r) = d.stats();
                (Some(a as u32), Some(r as u32))
            }
        };
        files.push(ChangedFile { path: d.path.clone(), old_path: d.old_path.clone(), status, additions, deletions });
    }
    for f in &pr.files {
        if files.iter().any(|x| x.path == f.path) {
            continue;
        }
        let status = match f.change_type.as_str() {
            "ADDED" => FileStatus::Added,
            "DELETED" => FileStatus::Deleted,
            "RENAMED" => FileStatus::Renamed,
            "COPIED" => FileStatus::Copied,
            _ => FileStatus::Modified,
        };
        files.push(ChangedFile { path: f.path.clone(), old_path: None, status, additions: Some(f.additions), deletions: Some(f.deletions) });
    }
    files
}

/// PR descriptions often carry template comments; hide them like GitHub does.
pub fn display_body(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    let text = out.replace("\r\n", "\n");
    // Collapse the blank runs that removed comments leave behind.
    let mut lines: Vec<&str> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() && lines.last().is_none_or(|l| l.trim().is_empty()) {
            continue;
        }
        lines.push(line);
    }
    lines.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(name: &str, url: &str) -> Remote {
        Remote { name: name.into(), fetch_url: url.into(), push_url: url.into() }
    }

    #[test]
    fn parses_github_urls() {
        let expect = Some(("github.com".to_string(), "octo".to_string(), "hello".to_string()));
        assert_eq!(parse_github_url("https://github.com/octo/hello.git"), expect);
        assert_eq!(parse_github_url("https://github.com/octo/hello"), expect);
        assert_eq!(parse_github_url("https://user:token@github.com/octo/hello.git"), expect);
        assert_eq!(parse_github_url("git@github.com:octo/hello.git"), expect);
        assert_eq!(parse_github_url("ssh://git@github.com/octo/hello.git"), expect);
        assert_eq!(parse_github_url("ssh://git@ssh.github.com:443/octo/hello.git"), expect);
        assert_eq!(parse_github_url("git@github-work:octo/hello.git"), expect);
        assert_eq!(parse_github_url("https://github.example.com/team/app.git"), Some(("github.example.com".into(), "team".into(), "app".into())));
        assert_eq!(parse_github_url("https://gitlab.com/octo/hello.git"), None);
        assert_eq!(parse_github_url("C:/repos/bare.git"), None);
        assert_eq!(parse_github_url("https://github.com/octo"), None);
    }

    #[test]
    fn prefers_upstream_then_origin() {
        let remotes = vec![remote("mine", "git@github.com:me/app.git"), remote("origin", "git@github.com:me/app.git"), remote("upstream", "https://github.com/org/app.git")];
        let repo = find_github_repo(&remotes).unwrap();
        assert_eq!((repo.remote.as_str(), repo.spec().as_str()), ("upstream", "github.com/org/app"));
        let remotes = vec![remote("gitlab", "https://gitlab.com/a/b.git"), remote("origin", "git@github.com:me/app.git")];
        assert_eq!(find_github_repo(&remotes).unwrap().full_name(), "me/app");
        assert_eq!(find_github_repo(&[remote("origin", "https://gitlab.com/a/b.git")]), None);
    }

    #[test]
    fn parses_pr_list_json() {
        let json = r#"[{"author":{"id":"U_1","is_bot":false,"login":"octocat","name":"Mona"},"baseRefName":"main","headRefName":"feature/x","headRefOid":"abc","headRepositoryOwner":{"id":"U_2","login":"fork-owner"},"isCrossRepository":true,"isDraft":false,"labels":[{"id":"L","name":"bug","description":"","color":"d73a4a"}],"number":42,"reviewDecision":"CHANGES_REQUESTED","state":"OPEN","statusCheckRollup":[
            {"__typename":"CheckRun","conclusion":"SUCCESS","name":"build","status":"COMPLETED","workflowName":"CI","detailsUrl":"https://x/1"},
            {"__typename":"CheckRun","conclusion":null,"name":"test","status":"IN_PROGRESS","workflowName":"CI"},
            {"__typename":"CheckRun","conclusion":"SKIPPED","name":"deploy","status":"COMPLETED"},
            {"__typename":"StatusContext","context":"ci/legacy","state":"FAILURE","targetUrl":"https://x/2"}],
            "title":"Fix it","url":"https://github.com/o/r/pull/42","createdAt":"2026-10-01T10:00:00Z","updatedAt":"2026-10-02T10:00:00Z"},
            {"author":null,"number":7,"title":"Draft","state":"OPEN","isDraft":true,"reviewDecision":"","statusCheckRollup":null,"labels":[]}]"#;
        let prs = parse_pull_requests(json).unwrap();
        assert_eq!(prs.len(), 2);
        let pr = &prs[0];
        assert_eq!(pr.number, 42);
        assert_eq!(pr.author.display(), "octocat");
        assert_eq!(pr.head_label(), "fork-owner:feature/x");
        assert_eq!(pr.pr_state(), PrState::Open);
        assert_eq!(pr.review_decision(), Some(ReviewDecision::ChangesRequested));
        let checks = pr.checks();
        assert_eq!(checks, ChecksSummary { passed: 1, failed: 1, pending: 1, neutral: 1 });
        assert_eq!(checks.overall(), Some(CheckOutcome::Failure));
        assert_eq!(pr.status_check_rollup[3].label(), "ci/legacy");
        assert_eq!(pr.status_check_rollup[3].url(), "https://x/2");
        assert_eq!(pr.updated_time() - pr.created_time(), 86400);
        assert_eq!(prs[1].pr_state(), PrState::Draft);
        assert_eq!(prs[1].review_decision(), None);
        assert_eq!(prs[1].checks().overall(), None);
        assert_eq!(prs[1].author.display(), "ghost");
    }

    #[test]
    fn explains_auth_failures() {
        let invalid = "github.com
  X Failed to log in to github.com account octo (keyring)
  - Active account: true
  - The token in keyring is invalid.
";
        assert_eq!(auth_failure("github.com", invalid), GhStatus::NotLoggedIn("Failed to log in to github.com account octo (keyring) The token in keyring is invalid.".into()));
        assert_eq!(auth_failure("github.com", "You are not logged into any GitHub hosts. To log in, run: gh auth login"), GhStatus::NotLoggedIn("You are not logged into any GitHub hosts. To log in, run: gh auth login".into()));
        assert_eq!(auth_failure("github.com", "github.com
  X Timeout trying to log in to github.com account octo (keyring)"), GhStatus::Ready);
    }

    #[test]
    fn summarizes_checks() {
        let s = ChecksSummary { passed: 3, failed: 0, pending: 0, neutral: 1 };
        assert_eq!(s.overall(), Some(CheckOutcome::Success));
        assert_eq!(s.describe(), "4 checks passed (1 skipped)");
        let s = ChecksSummary { passed: 1, failed: 0, pending: 2, neutral: 0 };
        assert_eq!(s.describe(), "2 of 3 checks pending");
    }

    #[test]
    fn builds_changed_files_from_diff_and_stats() {
        let diff = "diff --git a/old.txt b/new.txt\nsimilarity index 90%\nrename from old.txt\nrename to new.txt\n--- a/old.txt\n+++ b/new.txt\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/added.rs b/added.rs\nnew file mode 100644\n--- /dev/null\n+++ b/added.rs\n@@ -0,0 +1,2 @@\n+x\n+y\n";
        let diffs = parse_diff(diff.as_bytes());
        let pr = PullRequest {
            files: vec![
                PrFile { path: "new.txt".into(), additions: 1, deletions: 1, change_type: "RENAMED".into() },
                PrFile { path: "big.bin".into(), additions: 0, deletions: 0, change_type: "ADDED".into() },
            ],
            ..Default::default()
        };
        let files = changed_files(&pr, Some(&diffs));
        assert_eq!(files.len(), 3);
        assert_eq!((files[0].path.as_str(), files[0].old_path.as_deref(), files[0].status), ("new.txt", Some("old.txt"), FileStatus::Renamed));
        assert_eq!((files[1].status, files[1].additions, files[1].deletions), (FileStatus::Added, Some(2), Some(0)));
        assert_eq!((files[2].path.as_str(), files[2].status), ("big.bin", FileStatus::Added));
        // Without a diff, GitHub's file list is used.
        let files = changed_files(&pr, None);
        assert_eq!(files.iter().map(|f| f.status).collect::<Vec<_>>(), vec![FileStatus::Renamed, FileStatus::Added]);
    }

    #[test]
    fn cleans_up_descriptions() {
        assert_eq!(display_body("<!-- template -->\r\n\r\nFixes #1\r\n\r\n\r\n<!-- more\nlines -->\nDone"), "Fixes #1\n\nDone");
        assert_eq!(display_body("Unclosed <!-- comment"), "Unclosed");
        assert_eq!(number_from_url("https://github.com/o/r/pull/123\n"), Some(123));
        assert_eq!(number_from_url("https://github.com/o/r/pulls"), None);
    }
}
