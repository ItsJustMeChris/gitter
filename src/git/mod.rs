//! Git backend.
//!
//! Everything is driven through the `git` command line, the same way Tower and
//! most serious Git clients do it: it honours the user's config, hooks,
//! credential helpers and SSH setup, and it covers every porcelain feature.

pub mod graph;
pub mod images;
pub mod parse;
pub mod patch;

use anyhow::{anyhow, bail, Context as _, Result};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

pub use parse::*;

pub type Oid = String;

/// Path or name of the git executable. Can be overridden in settings.
static GIT_BINARY: OnceLock<String> = OnceLock::new();

pub fn set_git_binary(path: Option<String>) {
    let _ = GIT_BINARY.set(path.filter(|p| !p.trim().is_empty()).unwrap_or_else(|| "git".into()));
}

fn git_binary() -> &'static str {
    GIT_BINARY.get_or_init(|| "git".into())
}

/// Returns the installed git version string, e.g. "git version 2.49.0".
pub fn git_version() -> Result<String> {
    let out = base_command(None).arg("--version").output().context("Could not run git. Is it installed and on your PATH?")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn base_command(cwd: Option<&Path>) -> Command {
    let mut cmd = Command::new(git_binary());
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        // ":" is special-cased by git as a no-op editor, so commands that would
        // normally open an editor (revert, rebase --continue, merge) use the
        // default message instead of hanging.
        .env("GIT_EDITOR", ":")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .env_remove("GIT_PAGER")
        .env("GIT_PAGER", "cat")
        .args(["-c", "core.quotepath=false", "-c", "color.ui=false", "-c", "advice.detachedHead=false"])
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

#[derive(Debug)]
pub struct GitOutput {
    pub stdout: Vec<u8>,
}

impl GitOutput {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

/// An error from a git command, with the command line and its output.
#[derive(Debug)]
pub struct GitError {
    pub command: String,
    pub message: String,
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for GitError {}

#[derive(Clone, Debug)]
pub struct Repo {
    /// Root of the working tree.
    pub workdir: PathBuf,
    /// Absolute path to the .git directory.
    pub git_dir: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepoState {
    Clean,
    Merging,
    Rebasing,
    CherryPicking,
    Reverting,
    Bisecting,
}

impl RepoState {
    pub fn label(self) -> &'static str {
        match self {
            RepoState::Clean => "",
            RepoState::Merging => "Merge in progress",
            RepoState::Rebasing => "Rebase in progress",
            RepoState::CherryPicking => "Cherry-pick in progress",
            RepoState::Reverting => "Revert in progress",
            RepoState::Bisecting => "Bisect in progress",
        }
    }
}

/// Which commits the history view shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogScope {
    /// All local and remote branches, tags and HEAD.
    All,
    /// The history reachable from HEAD.
    Head,
    /// A specific ref (branch, remote branch, tag) or commit.
    Ref(String),
    /// History of a single file.
    File(String),
}

/// One complete snapshot of a repository, loaded in the background.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub head: HeadInfo,
    pub state: Option<RepoState>,
    pub status: WorkingStatus,
    pub branches: Vec<Branch>,
    pub remote_branches: Vec<RemoteBranch>,
    pub tags: Vec<Tag>,
    pub remotes: Vec<Remote>,
    pub stashes: Vec<Stash>,
    pub submodules: Vec<Submodule>,
    pub user_name: Option<String>,
    pub user_email: Option<String>,
}

impl Repo {
    /// Opens the repository containing `path`.
    pub fn open(path: &Path) -> Result<Repo> {
        if !path.exists() {
            bail!("{} does not exist", path.display());
        }
        let out = base_command(Some(path))
            .args(["rev-parse", "--show-toplevel", "--absolute-git-dir"])
            .output()
            .context("Could not run git. Is it installed and on your PATH?")?;
        if !out.status.success() {
            bail!("{} is not a Git repository", path.display());
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut lines = text.lines();
        let top = lines.next().ok_or_else(|| anyhow!("unexpected rev-parse output"))?;
        let git_dir = lines.next().ok_or_else(|| anyhow!("unexpected rev-parse output (is this a bare repository?)"))?;
        Ok(Repo { workdir: normalize_path(top), git_dir: normalize_path(git_dir) })
    }

    pub fn init(path: &Path) -> Result<Repo> {
        std::fs::create_dir_all(path)?;
        run_raw(Some(path), &["init"], None)?;
        Repo::open(path)
    }

    pub fn name(&self) -> String {
        self.workdir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.workdir.display().to_string())
    }

    // ---------------------------------------------------------------------
    // Command plumbing
    // ---------------------------------------------------------------------

    pub fn command(&self) -> Command {
        base_command(Some(&self.workdir))
    }

    /// Runs git and returns stdout on success, or a [`GitError`] on failure.
    pub fn run(&self, args: &[&str]) -> Result<String> {
        Ok(run_raw(Some(&self.workdir), args, None)?.text())
    }

    /// Runs git, feeding `input` to stdin.
    pub fn run_with_input(&self, args: &[&str], input: &[u8]) -> Result<String> {
        Ok(run_raw(Some(&self.workdir), args, Some(input))?.text())
    }

    /// Runs a read-only git command that must not take optional locks
    /// (so we never fight with other git processes or trigger our watcher).
    fn read(&self, args: &[&str]) -> Result<String> {
        let mut cmd = self.command();
        cmd.env("GIT_OPTIONAL_LOCKS", "0").args(args);
        Ok(exec(cmd, args, None)?.text())
    }

    fn read_bytes(&self, args: &[&str]) -> Result<Vec<u8>> {
        let mut cmd = self.command();
        cmd.env("GIT_OPTIONAL_LOCKS", "0").args(args);
        Ok(exec(cmd, args, None)?.stdout)
    }

    /// Runs git, streaming each line of stderr progress to `on_progress`.
    /// Used for network operations (fetch, pull, push, clone).
    pub fn run_with_progress(&self, args: &[&str], on_progress: impl FnMut(&str)) -> Result<String> {
        run_streaming(Some(&self.workdir), args, on_progress)
    }

    // ---------------------------------------------------------------------
    // Reading
    // ---------------------------------------------------------------------

    pub fn snapshot(&self) -> Result<Snapshot> {
        let (head, status) = self.status()?;
        let refs = self.refs()?;
        let remotes = self.remotes().unwrap_or_default();
        let stashes = self.stashes().unwrap_or_default();
        let submodules = self.submodules().unwrap_or_default();
        let user_name = self.config_get("user.name");
        let user_email = self.config_get("user.email");
        Ok(Snapshot {
            head,
            state: Some(self.state()),
            status,
            branches: refs.branches,
            remote_branches: refs.remote_branches,
            tags: refs.tags,
            remotes,
            stashes,
            submodules,
            user_name,
            user_email,
        })
    }

    pub fn state(&self) -> RepoState {
        let g = &self.git_dir;
        if g.join("rebase-merge").exists() || g.join("rebase-apply").exists() {
            RepoState::Rebasing
        } else if g.join("MERGE_HEAD").exists() {
            RepoState::Merging
        } else if g.join("CHERRY_PICK_HEAD").exists() {
            RepoState::CherryPicking
        } else if g.join("REVERT_HEAD").exists() {
            RepoState::Reverting
        } else if g.join("BISECT_LOG").exists() {
            RepoState::Bisecting
        } else {
            RepoState::Clean
        }
    }

    /// Prepared message for an in-progress merge/cherry-pick/revert.
    pub fn pending_merge_message(&self) -> Option<String> {
        std::fs::read_to_string(self.git_dir.join("MERGE_MSG"))
            .ok()
            .map(|m| m.lines().filter(|l| !l.starts_with('#')).collect::<Vec<_>>().join("\n").trim().to_string())
            .filter(|m| !m.is_empty())
    }

    pub fn status(&self) -> Result<(HeadInfo, WorkingStatus)> {
        let bytes = self.read_bytes(&["status", "--porcelain=v2", "--branch", "-z", "--untracked-files=all", "--ignore-submodules=dirty"])?;
        Ok(parse_status(&bytes))
    }

    pub fn refs(&self) -> Result<Refs> {
        let out = self.read(&[
            "for-each-ref",
            "--format=%(refname)%00%(objectname)%00%(*objectname)%00%(upstream:short)%00%(upstream:track,nobracket)%00%(HEAD)%00%(committerdate:unix)%00%(subject)%00%(symref)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ])?;
        Ok(parse_refs(&out))
    }

    pub fn remotes(&self) -> Result<Vec<Remote>> {
        let out = self.read(&["remote", "-v"])?;
        Ok(parse_remotes(&out))
    }

    pub fn stashes(&self) -> Result<Vec<Stash>> {
        let out = self.read(&["stash", "list", "--format=%gd%x1f%H%x1f%ct%x1f%gs"])?;
        Ok(parse_stashes(&out))
    }

    pub fn submodules(&self) -> Result<Vec<Submodule>> {
        if !self.workdir.join(".gitmodules").exists() {
            return Ok(Vec::new());
        }
        let out = self.read(&["submodule", "status"])?;
        Ok(parse_submodules(&out))
    }

    /// Top-level ignored directories, as absolute paths.
    pub fn ignored_directories(&self) -> Vec<PathBuf> {
        let Ok(out) = self.read(&["ls-files", "--others", "--ignored", "--exclude-standard", "--directory", "-z"]) else { return Vec::new() };
        out.split('\0')
            .filter(|p| p.ends_with('/'))
            .map(|p| self.workdir.join(normalize_path(p.trim_end_matches('/'))))
            .collect()
    }

    pub fn config_get(&self, key: &str) -> Option<String> {
        self.read(&["config", "--get", key]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }

    /// Loads up to `limit` commits starting after `skip`.
    pub fn log(&self, scope: &LogScope, skip: usize, limit: usize) -> Result<Vec<Commit>> {
        let skip_arg = format!("--skip={skip}");
        let limit_arg = format!("-n{limit}");
        let mut args: Vec<&str> = vec!["log", "--date-order", LOG_FORMAT, &skip_arg, &limit_arg];
        match scope {
            LogScope::All => args.extend(["--branches", "--remotes", "--tags", "HEAD"]),
            LogScope::Head => args.push("HEAD"),
            LogScope::Ref(r) => {
                args.push(r);
                args.push("--");
            }
            LogScope::File(path) => {
                args.extend(["--follow", "HEAD", "--", path]);
            }
        }
        match self.read(&args) {
            Ok(out) => Ok(parse_log(&out)),
            // An unborn branch has no history yet.
            Err(e) if e.to_string().contains("does not have any commits") || e.to_string().contains("unknown revision") || e.to_string().contains("bad default revision") => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// Searches the whole history for commits whose message, author or hash match.
    pub fn search_log(&self, scope: &LogScope, query: &str, limit: usize) -> Result<Vec<Commit>> {
        let limit_arg = format!("-n{limit}");
        // git ANDs --grep with --author, so search messages and authors separately.
        let filters = [format!("--grep={query}"), format!("--author={query}")];
        let mut commits: Vec<Commit> = Vec::new();
        for filter in &filters {
            let mut args: Vec<&str> = vec!["log", "--date-order", LOG_FORMAT, &limit_arg, "--regexp-ignore-case", "--fixed-strings", filter];
            match scope {
                LogScope::All => args.extend(["--branches", "--remotes", "--tags", "HEAD"]),
                LogScope::Head => args.push("HEAD"),
                LogScope::Ref(r) => args.push(r),
                LogScope::File(path) => args.extend(["HEAD", "--", path]),
            }
            for c in parse_log(&self.read(&args).unwrap_or_default()) {
                if !commits.iter().any(|x| x.oid == c.oid) {
                    commits.push(c);
                }
            }
        }
        commits.sort_by(|a, b| b.author_time.cmp(&a.author_time));
        commits.truncate(limit);
        // Hash prefix match goes first.
        if query.len() >= 4 && query.chars().all(|c| c.is_ascii_hexdigit()) {
            if let Ok(out) = self.read(&["log", LOG_FORMAT, "-n1", query, "--"]) {
                for c in parse_log(&out) {
                    commits.retain(|x| x.oid != c.oid);
                    commits.insert(0, c);
                }
            }
        }
        Ok(commits)
    }

    pub fn commit_details(&self, oid: &str) -> Result<CommitDetails> {
        let out = self.read(&["show", "-s", "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%cn%x1f%ce%x1f%ct%x1f%B", oid])?;
        let mut parts = out.splitn(9, '\x1f');
        let mut next = || parts.next().unwrap_or("").to_string();
        let oid_full = next();
        let parents: Vec<String> = next().split_whitespace().map(String::from).collect();
        let author_name = next();
        let author_email = next();
        let author_time = next().trim().parse().unwrap_or(0);
        let committer_name = next();
        let committer_email = next();
        let committer_time = next().trim().parse().unwrap_or(0);
        let message = next().trim_end().to_string();
        let files = self.changed_files(&oid_full, parents.first().map(|s| s.as_str()))?;
        Ok(CommitDetails {
            oid: oid_full,
            parents,
            author_name,
            author_email,
            author_time,
            committer_name,
            committer_email,
            committer_time,
            message,
            files,
        })
    }

    /// Files changed by a commit relative to `parent` (or the empty tree for root commits).
    pub fn changed_files(&self, oid: &str, parent: Option<&str>) -> Result<Vec<ChangedFile>> {
        let mut args = vec!["diff-tree", "-r", "-z", "-M", "--no-commit-id", "--name-status"];
        let numstat_args;
        match parent {
            Some(p) => {
                args.push(p);
                args.push(oid);
                numstat_args = vec!["diff-tree", "-r", "-z", "-M", "--no-commit-id", "--numstat", p, oid];
            }
            None => {
                args.push("--root");
                args.push(oid);
                numstat_args = vec!["diff-tree", "-r", "-z", "-M", "--no-commit-id", "--numstat", "--root", oid];
            }
        }
        let mut files = parse_name_status(&self.read_bytes(&args)?);
        if let Ok(numstat) = self.read_bytes(&numstat_args) {
            apply_numstat(&mut files, &numstat);
        }
        Ok(files)
    }

    pub fn stash_files(&self, stash: &Stash) -> Result<Vec<ChangedFile>> {
        let parent = format!("{}^1", stash.oid);
        let mut files = self.changed_files(&stash.oid, Some(&parent))?;
        // Untracked files live in the third parent of the stash commit.
        let untracked = format!("{}^3", stash.oid);
        if self.read(&["rev-parse", "--verify", "-q", &untracked]).is_ok() {
            if let Ok(extra) = self.changed_files(&untracked, None) {
                for mut f in extra {
                    f.status = FileStatus::Untracked;
                    files.push(f);
                }
            }
        }
        Ok(files)
    }

    /// Diff of a single file in a commit (against its first parent).
    pub fn commit_file_diff(&self, oid: &str, parent: Option<&str>, file: &ChangedFile) -> Result<FileDiff> {
        let mut args: Vec<String> = vec!["diff".into(), "--no-color".into(), "--no-ext-diff".into(), "-M".into(), "-U3".into()];
        match parent {
            Some(p) => {
                args.push(p.to_string());
                args.push(oid.to_string());
            }
            None => {
                // Root commit: diff against the empty tree.
                args.push(EMPTY_TREE.to_string());
                args.push(oid.to_string());
            }
        }
        args.push("--".into());
        if let Some(old) = &file.old_path {
            args.push(old.clone());
        }
        args.push(file.path.clone());
        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let out = self.read_bytes(&refs)?;
        let mut diff = parse_diff(&out).into_iter().next().unwrap_or_else(|| FileDiff::empty(&file.path));
        diff.images = self.commit_images(oid, parent, file);
        Ok(diff)
    }

    pub fn stash_file_diff(&self, stash: &Stash, file: &ChangedFile) -> Result<FileDiff> {
        if file.status == FileStatus::Untracked {
            let untracked = format!("{}^3", stash.oid);
            return self.commit_file_diff(&untracked, None, file);
        }
        let parent = format!("{}^1", stash.oid);
        self.commit_file_diff(&stash.oid, Some(&parent), file)
    }

    /// Diff of a working copy file. `staged` selects index-vs-HEAD instead of worktree-vs-index.
    pub fn working_diff(&self, entry: &StatusEntry, staged: bool, context: u32) -> Result<FileDiff> {
        let mut diff = self.working_text_diff(entry, staged, context)?;
        diff.images = self.working_images(entry, staged);
        Ok(diff)
    }

    fn working_text_diff(&self, entry: &StatusEntry, staged: bool, context: u32) -> Result<FileDiff> {
        let ctx = format!("-U{context}");
        if !staged && entry.status == FileStatus::Untracked {
            return self.untracked_diff(&entry.path);
        }
        if entry.status == FileStatus::Conflicted {
            let bytes = std::fs::read(self.workdir.join(&entry.path)).unwrap_or_default();
            return Ok(conflict_view(&entry.path, &bytes));
        }
        let mut args: Vec<&str> = vec!["diff", "--no-color", "--no-ext-diff", &ctx];
        if staged {
            args.push("--cached");
            args.push("-M");
        }
        args.push("--");
        if staged {
            if let Some(old) = &entry.old_path {
                args.push(old);
            }
        }
        args.push(&entry.path);
        let out = self.read_bytes(&args)?;
        let mut diffs = parse_diff(&out);
        Ok(diffs.pop().unwrap_or_else(|| FileDiff::empty(&entry.path)))
    }

    /// Synthesizes an "all lines added" diff for an untracked file.
    fn untracked_diff(&self, path: &str) -> Result<FileDiff> {
        let full = self.workdir.join(path);
        let meta = std::fs::metadata(&full)?;
        if meta.is_dir() {
            return Ok(FileDiff::empty(path));
        }
        if meta.len() > 8 * 1024 * 1024 {
            let mut d = FileDiff::empty(path);
            d.binary = true;
            d.new_file = true;
            return Ok(d);
        }
        let bytes = std::fs::read(&full)?;
        Ok(synthesize_added_diff(path, &bytes))
    }

    pub fn reflog(&self, limit: usize) -> Result<Vec<ReflogEntry>> {
        let n = format!("-n{limit}");
        let out = self.read(&["reflog", "show", &n, "--date=unix", "--format=%H%x1f%gd%x1f%gs"])?;
        Ok(parse_reflog(&out))
    }

    pub fn blame(&self, path: &str) -> Result<Vec<BlameLine>> {
        let out = self.read(&["blame", "--porcelain", "--", path])?;
        Ok(parse_blame(&out))
    }

    pub fn is_ancestor(&self, ancestor: &str, of: &str) -> bool {
        self.read(&["merge-base", "--is-ancestor", ancestor, of]).is_ok()
    }

    /// Commit oids reachable from HEAD but not from `upstream` (i.e. unpushed).
    pub fn unpushed(&self, upstream: &str) -> Vec<String> {
        let range = format!("{upstream}..HEAD");
        self.read(&["rev-list", &range]).map(|s| s.lines().map(String::from).collect()).unwrap_or_default()
    }

    // ---------------------------------------------------------------------
    // Staging
    // ---------------------------------------------------------------------

    pub fn stage(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        for chunk in paths.chunks(200) {
            let mut args = vec!["add", "-A", "--"];
            args.extend(chunk.iter().map(|s| s.as_str()));
            self.run(&args)?;
        }
        Ok(())
    }

    pub fn stage_all(&self) -> Result<()> {
        self.run(&["add", "-A"])?;
        Ok(())
    }

    pub fn unstage(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let has_head = self.read(&["rev-parse", "--verify", "-q", "HEAD"]).is_ok();
        for chunk in paths.chunks(200) {
            let mut args = if has_head { vec!["reset", "-q", "HEAD", "--"] } else { vec!["rm", "--cached", "-r", "-q", "--"] };
            args.extend(chunk.iter().map(|s| s.as_str()));
            self.run(&args)?;
        }
        Ok(())
    }

    pub fn unstage_all(&self) -> Result<()> {
        if self.read(&["rev-parse", "--verify", "-q", "HEAD"]).is_ok() {
            self.run(&["reset", "-q", "HEAD"])?;
        } else {
            self.run(&["rm", "--cached", "-r", "-q", "."])?;
        }
        Ok(())
    }

    /// Discards working tree changes. Untracked files are deleted.
    pub fn discard(&self, entries: &[StatusEntry]) -> Result<()> {
        let mut tracked = Vec::new();
        for e in entries {
            if e.status == FileStatus::Untracked {
                let full = self.workdir.join(&e.path);
                if full.is_dir() {
                    std::fs::remove_dir_all(&full)?;
                } else if full.exists() {
                    std::fs::remove_file(&full)?;
                }
            } else {
                tracked.push(e.path.clone());
            }
        }
        for chunk in tracked.chunks(200) {
            let mut args = vec!["checkout", "--"];
            args.extend(chunk.iter().map(|s| s.as_str()));
            self.run(&args)?;
        }
        Ok(())
    }

    /// Applies a patch to the index (staging) or to the working tree.
    pub fn apply_patch(&self, patch: &str, cached: bool, reverse: bool) -> Result<()> {
        let mut args = vec!["apply", "--recount", "--whitespace=nowarn"];
        if cached {
            args.push("--cached");
        }
        if reverse {
            args.push("-R");
        }
        args.push("-");
        self.run_with_input(&args, patch.as_bytes())?;
        Ok(())
    }

    pub fn resolve_with(&self, path: &str, ours: bool) -> Result<()> {
        self.run(&["checkout", if ours { "--ours" } else { "--theirs" }, "--", path])?;
        self.run(&["add", "--", path])?;
        Ok(())
    }

    pub fn mark_resolved(&self, paths: &[String]) -> Result<()> {
        self.stage(paths)
    }

    pub fn add_to_gitignore(&self, pattern: &str) -> Result<()> {
        let path = self.workdir.join(".gitignore");
        let mut existing = std::fs::read_to_string(&path).unwrap_or_default();
        if existing.lines().any(|l| l.trim() == pattern) {
            return Ok(());
        }
        if !existing.is_empty() && !existing.ends_with('\n') {
            existing.push('\n');
        }
        existing.push_str(pattern);
        existing.push('\n');
        std::fs::write(path, existing)?;
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Committing
    // ---------------------------------------------------------------------

    pub fn commit(&self, message: &str, amend: bool, allow_empty: bool) -> Result<String> {
        let mut args = vec!["commit", "-F", "-", "--cleanup=strip"];
        if amend {
            args.push("--amend");
        }
        if allow_empty {
            args.push("--allow-empty");
        }
        self.run_with_input(&args, message.as_bytes())
    }

    pub fn head_message(&self) -> Option<String> {
        self.read(&["log", "-1", "--format=%B"]).ok().map(|s| s.trim_end().to_string())
    }

    // ---------------------------------------------------------------------
    // Branches
    // ---------------------------------------------------------------------

    pub fn checkout(&self, name: &str) -> Result<String> {
        self.run(&["checkout", name])
    }

    pub fn checkout_detached(&self, oid: &str) -> Result<String> {
        self.run(&["checkout", "--detach", oid])
    }

    /// Creates a local tracking branch for `remote_branch` (e.g. "origin/feature") and checks it out.
    pub fn checkout_remote_branch(&self, remote_branch: &str, local_name: &str) -> Result<String> {
        if self.read(&["rev-parse", "--verify", "-q", &format!("refs/heads/{local_name}")]).is_ok() {
            // Local branch already exists: just switch to it.
            return self.run(&["checkout", local_name]);
        }
        self.run(&["checkout", "-b", local_name, "--track", remote_branch])
    }

    pub fn create_branch(&self, name: &str, start: Option<&str>, checkout: bool) -> Result<String> {
        let mut args = if checkout { vec!["checkout", "-b", name] } else { vec!["branch", name] };
        if let Some(s) = start {
            args.push(s);
        }
        self.run(&args)
    }

    pub fn rename_branch(&self, old: &str, new: &str) -> Result<String> {
        self.run(&["branch", "-m", old, new])
    }

    pub fn delete_branch(&self, name: &str, force: bool) -> Result<String> {
        self.run(&["branch", if force { "-D" } else { "-d" }, name])
    }

    pub fn delete_remote_branch(&self, remote: &str, branch: &str, on_progress: impl FnMut(&str)) -> Result<String> {
        self.run_with_progress(&["push", "--progress", remote, "--delete", branch], on_progress)
    }

    pub fn set_upstream(&self, branch: &str, upstream: Option<&str>) -> Result<String> {
        match upstream {
            Some(up) => self.run(&["branch", &format!("--set-upstream-to={up}"), branch]),
            None => self.run(&["branch", "--unset-upstream", branch]),
        }
    }

    pub fn merge(&self, what: &str, no_ff: bool, squash: bool) -> Result<String> {
        let mut args = vec!["merge"];
        if no_ff {
            args.push("--no-ff");
        }
        if squash {
            args.push("--squash");
        }
        args.push(what);
        self.run(&args)
    }

    pub fn rebase(&self, onto: &str) -> Result<String> {
        self.run(&["rebase", onto])
    }

    pub fn abort(&self, state: RepoState) -> Result<String> {
        match state {
            RepoState::Merging => self.run(&["merge", "--abort"]),
            RepoState::Rebasing => self.run(&["rebase", "--abort"]),
            RepoState::CherryPicking => self.run(&["cherry-pick", "--abort"]),
            RepoState::Reverting => self.run(&["revert", "--abort"]),
            RepoState::Bisecting => self.run(&["bisect", "reset"]),
            RepoState::Clean => Ok(String::new()),
        }
    }

    pub fn continue_op(&self, state: RepoState) -> Result<String> {
        match state {
            RepoState::Merging => self.run(&["commit", "--no-edit"]),
            RepoState::Rebasing => self.run(&["rebase", "--continue"]),
            RepoState::CherryPicking => self.run(&["cherry-pick", "--continue"]),
            RepoState::Reverting => self.run(&["revert", "--continue"]),
            RepoState::Bisecting | RepoState::Clean => Ok(String::new()),
        }
    }

    pub fn skip_rebase_commit(&self) -> Result<String> {
        self.run(&["rebase", "--skip"])
    }

    // ---------------------------------------------------------------------
    // Commits
    // ---------------------------------------------------------------------

    pub fn cherry_pick(&self, oid: &str) -> Result<String> {
        self.run(&["cherry-pick", oid])
    }

    pub fn revert(&self, oid: &str, is_merge: bool) -> Result<String> {
        if is_merge {
            self.run(&["revert", "--no-edit", "-m", "1", oid])
        } else {
            self.run(&["revert", "--no-edit", oid])
        }
    }

    pub fn reset(&self, oid: &str, mode: ResetMode) -> Result<String> {
        let flag = match mode {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
        };
        self.run(&["reset", flag, oid])
    }

    /// Runs an interactive rebase onto `base` with the given todo list.
    /// `base` of `None` rebases from the root commit.
    pub fn interactive_rebase(&self, base: Option<&str>, todo: &str) -> Result<String> {
        let dir = std::env::temp_dir().join(format!("gitter-rebase-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let todo_path = dir.join("todo");
        std::fs::write(&todo_path, todo)?;
        // git runs the sequence editor through its bundled shell, so `cp`
        // is available on every platform.
        let todo_str = todo_path.to_string_lossy().replace('\\', "/");
        let editor = format!("cp '{}'", todo_str.replace('\'', "'\\''"));
        let mut cmd = self.command();
        cmd.env("GIT_SEQUENCE_EDITOR", editor);
        let mut args = vec!["rebase", "-i", "--autostash"];
        match base {
            Some(b) => args.push(b),
            None => args.push("--root"),
        }
        cmd.args(&args);
        let result = exec(cmd, &args, None).map(|o| o.text());
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    /// Changes the message of an arbitrary commit on the current branch.
    pub fn reword(&self, oid: &str, message: &str) -> Result<String> {
        let head = self.read(&["rev-parse", "HEAD"])?.trim().to_string();
        if head == oid {
            return self.commit(message, true, true);
        }
        let dir = std::env::temp_dir().join(format!("gitter-reword-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let msg_path = dir.join("message");
        std::fs::write(&msg_path, message)?;
        let msg_str = msg_path.to_string_lossy().replace('\\', "/");
        let commits = self.rev_list_range(oid)?;
        let mut todo = String::new();
        for c in &commits {
            todo.push_str(&format!("pick {c}\n"));
            if c == oid {
                todo.push_str(&format!("exec git commit --amend --allow-empty --cleanup=strip -F '{}'\n", msg_str.replace('\'', "'\\''")));
            }
        }
        let parent = self.parent_of(oid);
        let result = self.interactive_rebase(parent.as_deref(), &todo);
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    /// Squashes `oid` into its parent, keeping both messages.
    pub fn squash_into_parent(&self, oid: &str) -> Result<String> {
        let parent = self.parent_of(oid).ok_or_else(|| anyhow!("The root commit has no parent to squash into"))?;
        let grandparent = self.parent_of(&parent);
        let commits = self.rev_list_range(&parent)?;
        // A plain `fixup` would discard this commit's message; amend with a combined one.
        let parent_msg = self.read(&["log", "-1", "--format=%B", &parent])?.trim_end().to_string();
        let msg = self.read(&["log", "-1", "--format=%B", oid])?.trim_end().to_string();
        let combined = format!("{parent_msg}\n\n{msg}\n");
        let dir = std::env::temp_dir().join(format!("gitter-squash-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let msg_path = dir.join("message");
        std::fs::write(&msg_path, combined)?;
        let msg_str = msg_path.to_string_lossy().replace('\\', "/").replace('\'', "'\\''");
        let mut todo = String::new();
        for c in &commits {
            if c == oid {
                todo.push_str(&format!("fixup {c}\nexec git commit --amend --allow-empty --cleanup=strip -F '{msg_str}'\n"));
            } else {
                todo.push_str(&format!("pick {c}\n"));
            }
        }
        let result = self.interactive_rebase(grandparent.as_deref(), &todo);
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    /// Removes a commit from the current branch.
    pub fn drop_commit(&self, oid: &str) -> Result<String> {
        let commits = self.rev_list_range(oid)?;
        let mut todo = String::new();
        for c in &commits {
            if c == oid {
                todo.push_str(&format!("drop {c}\n"));
            } else {
                todo.push_str(&format!("pick {c}\n"));
            }
        }
        let parent = self.parent_of(oid);
        self.interactive_rebase(parent.as_deref(), &todo)
    }

    /// Moves a commit one position up (towards HEAD) or down in the current branch.
    pub fn move_commit(&self, oid: &str, up: bool) -> Result<String> {
        let start = if up { oid.to_string() } else { self.parent_of(oid).ok_or_else(|| anyhow!("Commit has no parent"))? };
        let mut commits = self.rev_list_range(&start)?;
        let ix = commits.iter().position(|c| c == oid).ok_or_else(|| anyhow!("Commit is not on the current branch"))?;
        if up {
            if ix + 1 >= commits.len() {
                bail!("Commit is already at the top");
            }
            commits.swap(ix, ix + 1);
        } else {
            if ix == 0 {
                bail!("Commit cannot be moved further down");
            }
            commits.swap(ix, ix - 1);
        }
        let todo: String = commits.iter().map(|c| format!("pick {c}\n")).collect();
        let base = self.parent_of(&start);
        self.interactive_rebase(base.as_deref(), &todo)
    }

    fn parent_of(&self, oid: &str) -> Option<String> {
        self.read(&["rev-parse", "--verify", "-q", &format!("{oid}^")]).ok().map(|s| s.trim().to_string())
    }

    /// Commits from `oid` (inclusive) up to HEAD, oldest first. Fails if `oid` is not an ancestor of HEAD
    /// or if a merge commit is in the range.
    fn rev_list_range(&self, oid: &str) -> Result<Vec<String>> {
        if self.read(&["merge-base", "--is-ancestor", oid, "HEAD"]).is_err() {
            bail!("This commit is not part of the current branch");
        }
        let range = match self.parent_of(oid) {
            Some(p) => format!("{p}..HEAD"),
            None => "HEAD".to_string(),
        };
        let merges = self.read(&["rev-list", "--merges", &range])?;
        if !merges.trim().is_empty() {
            bail!("Rewriting history across merge commits is not supported");
        }
        let out = self.read(&["rev-list", "--reverse", &range])?;
        Ok(out.lines().map(String::from).collect())
    }

    // ---------------------------------------------------------------------
    // Tags
    // ---------------------------------------------------------------------

    pub fn create_tag(&self, name: &str, target: &str, message: Option<&str>) -> Result<String> {
        match message.filter(|m| !m.trim().is_empty()) {
            Some(m) => self.run_with_input(&["tag", "-a", name, target, "-F", "-"], m.as_bytes()),
            None => self.run(&["tag", name, target]),
        }
    }

    pub fn delete_tag(&self, name: &str) -> Result<String> {
        self.run(&["tag", "-d", name])
    }

    pub fn push_tag(&self, remote: &str, name: &str, on_progress: impl FnMut(&str)) -> Result<String> {
        let refspec = format!("refs/tags/{name}");
        self.run_with_progress(&["push", "--progress", remote, &refspec], on_progress)
    }

    pub fn delete_remote_tag(&self, remote: &str, name: &str, on_progress: impl FnMut(&str)) -> Result<String> {
        let refspec = format!(":refs/tags/{name}");
        self.run_with_progress(&["push", "--progress", remote, &refspec], on_progress)
    }

    // ---------------------------------------------------------------------
    // Stashes
    // ---------------------------------------------------------------------

    pub fn stash_save(&self, message: Option<&str>, include_untracked: bool, keep_index: bool) -> Result<String> {
        let mut args = vec!["stash", "push"];
        if include_untracked {
            args.push("--include-untracked");
        }
        if keep_index {
            args.push("--keep-index");
        }
        if let Some(m) = message.filter(|m| !m.trim().is_empty()) {
            args.push("-m");
            args.push(m);
        }
        self.run(&args)
    }

    pub fn stash_apply(&self, name: &str, pop: bool) -> Result<String> {
        self.run(&["stash", if pop { "pop" } else { "apply" }, "--index", name]).or_else(|_| self.run(&["stash", if pop { "pop" } else { "apply" }, name]))
    }

    pub fn stash_drop(&self, name: &str) -> Result<String> {
        self.run(&["stash", "drop", name])
    }

    // ---------------------------------------------------------------------
    // Remotes
    // ---------------------------------------------------------------------

    pub fn fetch(&self, remote: Option<&str>, on_progress: impl FnMut(&str)) -> Result<String> {
        let mut args = vec!["fetch", "--progress", "--prune", "--tags"];
        match remote {
            Some(r) => args.push(r),
            None => args.push("--all"),
        }
        self.run_with_progress(&args, on_progress)
    }

    pub fn pull(&self, rebase: bool, on_progress: impl FnMut(&str)) -> Result<String> {
        let args = vec!["pull", "--progress", if rebase { "--rebase" } else { "--no-rebase" }, "--autostash"];
        self.run_with_progress(&args, on_progress)
    }

    pub fn push(&self, remote: &str, branch: &str, set_upstream: bool, force: bool, tags: bool, on_progress: impl FnMut(&str)) -> Result<String> {
        let mut args = vec!["push", "--progress"];
        if set_upstream {
            args.push("-u");
        }
        if force {
            args.push("--force-with-lease");
        }
        if tags {
            args.push("--follow-tags");
        }
        args.push(remote);
        let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
        args.push(&refspec);
        self.run_with_progress(&args, on_progress)
    }

    pub fn push_branch_to(&self, remote: &str, local: &str, remote_name: &str, set_upstream: bool, force: bool, on_progress: impl FnMut(&str)) -> Result<String> {
        let mut args = vec!["push", "--progress"];
        if set_upstream {
            args.push("-u");
        }
        if force {
            args.push("--force-with-lease");
        }
        args.push(remote);
        let refspec = format!("refs/heads/{local}:refs/heads/{remote_name}");
        args.push(&refspec);
        self.run_with_progress(&args, on_progress)
    }

    pub fn add_remote(&self, name: &str, url: &str) -> Result<String> {
        self.run(&["remote", "add", name, url])
    }

    pub fn remove_remote(&self, name: &str) -> Result<String> {
        self.run(&["remote", "remove", name])
    }

    pub fn rename_remote(&self, old: &str, new: &str) -> Result<String> {
        self.run(&["remote", "rename", old, new])
    }

    pub fn set_remote_url(&self, name: &str, url: &str) -> Result<String> {
        self.run(&["remote", "set-url", name, url])
    }

    // ---------------------------------------------------------------------
    // Submodules
    // ---------------------------------------------------------------------

    pub fn submodule_update(&self, on_progress: impl FnMut(&str)) -> Result<String> {
        self.run_with_progress(&["submodule", "update", "--init", "--recursive", "--progress"], on_progress)
    }

    // ---------------------------------------------------------------------
    // Misc
    // ---------------------------------------------------------------------

    pub fn is_valid_ref_name(name: &str) -> bool {
        if name.is_empty() {
            return false;
        }
        base_command(None).args(["check-ref-format", "--branch", name]).output().map(|o| o.status.success()).unwrap_or(false)
    }
}

/// Reads a value from the user's global git config.
pub fn global_config_get(key: &str) -> Option<String> {
    let out = base_command(None).args(["config", "--global", "--get", key]).output().ok()?;
    let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

/// Sets (or, when `value` is empty, unsets) a value in the user's global git config.
pub fn global_config_set(key: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        let _ = base_command(None).args(["config", "--global", "--unset", key]).output();
        return Ok(());
    }
    run_raw(None, &["config", "--global", key, value], None)?;
    Ok(())
}

/// Clones `url` into `dest`, reporting progress lines.
pub fn clone(url: &str, dest: &Path, on_progress: impl FnMut(&str)) -> Result<Repo> {
    let parent = dest.parent().ok_or_else(|| anyhow!("Invalid destination"))?;
    std::fs::create_dir_all(parent)?;
    let dest_str = dest.to_string_lossy().to_string();
    run_streaming(Some(parent), &["clone", "--progress", "--recurse-submodules", url, &dest_str], on_progress)?;
    Repo::open(dest)
}

pub const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

const LOG_FORMAT: &str = "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s";

fn normalize_path(p: &str) -> PathBuf {
    let p = p.trim();
    #[cfg(windows)]
    {
        // git prints forward slashes on Windows; normalize for display.
        PathBuf::from(p.replace('/', "\\"))
    }
    #[cfg(not(windows))]
    {
        PathBuf::from(p)
    }
}

fn run_raw(cwd: Option<&Path>, args: &[&str], input: Option<&[u8]>) -> Result<GitOutput> {
    let mut cmd = base_command(cwd);
    cmd.args(args);
    exec(cmd, args, input)
}

fn exec(mut cmd: Command, args: &[&str], input: Option<&[u8]>) -> Result<GitOutput> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    if input.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let mut child = cmd.spawn().with_context(|| format!("Failed to run git {}", args.join(" ")))?;
    if let Some(input) = input {
        if let Some(mut stdin) = child.stdin.take() {
            let data = input.to_vec();
            // Write on a thread so large patches can't deadlock against a full stdout pipe.
            std::thread::spawn(move || {
                let _ = stdin.write_all(&data);
            });
        }
    }
    let out = child.wait_with_output()?;
    let code = out.status.code().unwrap_or(-1);
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let mut message = stderr.trim().to_string();
        if message.is_empty() {
            message = stdout.trim().to_string();
        } else if !stdout.trim().is_empty() && args.first().is_some_and(|a| matches!(*a, "merge" | "rebase" | "cherry-pick" | "revert" | "pull" | "stash" | "commit")) {
            message = format!("{}\n{}", stdout.trim(), message);
        }
        if message.is_empty() {
            message = format!("git exited with code {code}");
        }
        return Err(GitError { command: format!("git {}", args.join(" ")), message }.into());
    }
    Ok(GitOutput { stdout: out.stdout })
}

fn run_streaming(cwd: Option<&Path>, args: &[&str], mut on_progress: impl FnMut(&str)) -> Result<String> {
    let mut cmd = base_command(cwd);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().with_context(|| format!("Failed to run git {}", args.join(" ")))?;
    let mut stdout = child.stdout.take().unwrap();
    let stdout_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let mut stderr = child.stderr.take().unwrap();
    let mut all_err = String::new();
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while let Ok(1) = stderr.read(&mut byte) {
        if byte[0] == b'\r' || byte[0] == b'\n' {
            if !line.is_empty() {
                let s = String::from_utf8_lossy(&line).into_owned();
                on_progress(s.trim());
                all_err.push_str(&s);
                all_err.push('\n');
                line.clear();
            }
        } else {
            line.push(byte[0]);
        }
    }
    if !line.is_empty() {
        let s = String::from_utf8_lossy(&line).into_owned();
        all_err.push_str(&s);
    }
    let status = child.wait()?;
    let stdout = stdout_thread.join().unwrap_or_default();
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    if !status.success() {
        // Drop the noisy progress lines from the error message.
        let msg: Vec<&str> = all_err
            .lines()
            .filter(|l| {
                let l = l.trim();
                !(l.starts_with("remote: Counting") || l.starts_with("remote: Compressing") || l.starts_with("Receiving objects") || l.starts_with("Resolving deltas") || l.starts_with("Counting objects") || l.starts_with("Compressing objects") || l.starts_with("Writing objects") || l.starts_with("remote: Enumerating") || l.starts_with("Enumerating objects") || l.starts_with("remote: Total") || l.starts_with("Updating files"))
            })
            .collect();
        let mut message = msg.join("\n").trim().to_string();
        if message.is_empty() {
            message = format!("git exited with code {}", status.code().unwrap_or(-1));
        }
        return Err(GitError { command: format!("git {}", args.join(" ")), message }.into());
    }
    Ok(if stdout.trim().is_empty() { all_err } else { stdout })
}

/// Groups refs by the commit they point to, for drawing labels in the history.
pub fn refs_by_commit(snapshot: &Snapshot) -> HashMap<Oid, Vec<RefLabel>> {
    let mut map: HashMap<Oid, Vec<RefLabel>> = HashMap::new();
    if let Some(oid) = &snapshot.head.oid {
        if snapshot.head.detached {
            map.entry(oid.clone()).or_default().push(RefLabel { name: "HEAD".into(), kind: RefKind::Head, is_head: true });
        }
    }
    for b in &snapshot.branches {
        map.entry(b.oid.clone()).or_default().push(RefLabel { name: b.name.clone(), kind: RefKind::Local, is_head: b.is_head });
    }
    for r in &snapshot.remote_branches {
        map.entry(r.oid.clone()).or_default().push(RefLabel { name: r.full_name.clone(), kind: RefKind::Remote, is_head: false });
    }
    for t in &snapshot.tags {
        map.entry(t.oid.clone()).or_default().push(RefLabel { name: t.name.clone(), kind: RefKind::Tag, is_head: false });
    }
    for labels in map.values_mut() {
        labels.sort_by_key(|l| (!l.is_head, l.kind as u8));
    }
    map
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    fn temp_repo(name: &str) -> Repo {
        let dir = std::env::temp_dir().join(format!("gitter-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let repo = Repo::init(&dir).unwrap();
        repo.run(&["config", "user.name", "Test"]).unwrap();
        repo.run(&["config", "user.email", "test@example.com"]).unwrap();
        repo.run(&["config", "core.autocrlf", "false"]).unwrap();
        repo
    }

    #[test]
    fn line_staging_commit_branch_stash() {
        let repo = temp_repo("flow");
        std::fs::write(repo.workdir.join("a.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        repo.stage_all().unwrap();
        repo.commit("Initial commit", false, false).unwrap();

        std::fs::write(repo.workdir.join("a.txt"), "ONE\ntwo\nthree\nFOUR\n").unwrap();
        let (_, status) = repo.status().unwrap();
        assert_eq!(status.unstaged.len(), 1);
        let diff = repo.working_diff(&status.unstaged[0], false, 3).unwrap();
        // Stage only the first change (-one/+ONE).
        let patch = patch::build_patch(&diff, |_, l| l <= 1, false).unwrap();
        repo.apply_patch(&patch, true, false).unwrap();
        let staged = repo.run(&["diff", "--cached"]).unwrap();
        assert!(staged.contains("+ONE") && !staged.contains("+FOUR"), "{staged}");

        // Unstage it again via a reverse patch built from the staged diff.
        let (_, status) = repo.status().unwrap();
        let sdiff = repo.working_diff(&status.staged[0], true, 3).unwrap();
        let patch = patch::build_patch(&sdiff, |_, _| true, true).unwrap();
        repo.apply_patch(&patch, true, true).unwrap();
        assert!(repo.run(&["diff", "--cached"]).unwrap().is_empty());

        // Discard only the FOUR change from the working tree.
        let (_, status) = repo.status().unwrap();
        let diff = repo.working_diff(&status.unstaged[0], false, 3).unwrap();
        let last = diff.hunks[0].lines.len() - 1;
        let patch = patch::build_patch(&diff, |_, l| l + 1 >= last, true).unwrap();
        repo.apply_patch(&patch, false, true).unwrap();
        assert_eq!(std::fs::read_to_string(repo.workdir.join("a.txt")).unwrap(), "ONE\ntwo\nthree\nfour\n");

        repo.stage_all().unwrap();
        repo.commit("Second", false, false).unwrap();
        repo.create_branch("feature/x", None, true).unwrap();
        let snap = repo.snapshot().unwrap();
        assert_eq!(snap.head.branch.as_deref(), Some("feature/x"));
        assert_eq!(snap.branches.len(), 2);

        std::fs::write(repo.workdir.join("b.txt"), "new\n").unwrap();
        repo.stash_save(Some("wip"), true, false).unwrap();
        let stashes = repo.stashes().unwrap();
        assert_eq!(stashes.len(), 1);
        let files = repo.stash_files(&stashes[0]).unwrap();
        assert!(files.iter().any(|f| f.path == "b.txt"), "{files:?}");
        repo.stash_apply(&stashes[0].name, true).unwrap();
        assert!(repo.workdir.join("b.txt").exists());

        let log = repo.log(&LogScope::All, 0, 100).unwrap();
        assert_eq!(log.len(), 2);
        let details = repo.commit_details(&log[0].oid).unwrap();
        assert_eq!(details.message, "Second");
        assert_eq!(details.files.len(), 1);
        let d = repo.commit_file_diff(&details.oid, details.parents.first().map(|s| s.as_str()), &details.files[0]).unwrap();
        assert_eq!(d.stats(), (1, 1));
        let root = repo.commit_details(&log[1].oid).unwrap();
        let d = repo.commit_file_diff(&root.oid, None, &root.files[0]).unwrap();
        assert_eq!(d.stats(), (4, 0));
        let _ = std::fs::remove_dir_all(&repo.workdir);
    }

    #[test]
    fn rewrite_history() {
        let repo = temp_repo("rewrite");
        for (i, name) in ["a", "b", "c"].iter().enumerate() {
            std::fs::write(repo.workdir.join(name), format!("{i}\n")).unwrap();
            repo.stage_all().unwrap();
            repo.commit(&format!("Add {name}"), false, false).unwrap();
        }
        let log = repo.log(&LogScope::Head, 0, 10).unwrap();
        repo.reword(&log[1].oid, "Add b (reworded)").unwrap();
        let log = repo.log(&LogScope::Head, 0, 10).unwrap();
        assert_eq!(log[1].subject, "Add b (reworded)");
        repo.squash_into_parent(&log[0].oid).unwrap();
        let log = repo.log(&LogScope::Head, 0, 10).unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].subject, "Add b (reworded)");
        repo.drop_commit(&log[0].oid).unwrap();
        let log = repo.log(&LogScope::Head, 0, 10).unwrap();
        assert_eq!(log.len(), 1);
        let _ = std::fs::remove_dir_all(&repo.workdir);
    }
}
