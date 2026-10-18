//! GitHub pull requests: the list, a detail pane with the description and the
//! changed files (reusing the commit detail and diff components), checking out
//! a PR, and creating one for the current branch. Everything goes through the
//! `gh` CLI (see `crate::github`).

use super::diff_view::DiffLocation;
use super::dialogs::{field_label, Modal, ModalKind};
use super::undo::UndoAction;
use super::{DetailState, Nav, RepoView, ToastKind};
use crate::actions::{SelectFirst, SelectLast, SelectNext, SelectPrev};
use crate::assets::Icon;
use crate::git::{CommitDetails, FileDiff};
use crate::github::{self, CheckOutcome, GhStatus, GitHubRepo, NewPullRequest, PrFilter, PrState, PullRequest, ReviewDecision};
use crate::theme::{ActiveTheme, Theme, ThemeMode};
use crate::ui::menu::ContextMenu;
use crate::ui::text_input::{InputEvent, TextInput};
use crate::ui::{self, badge, empty_state, icon, icon_sized, Button, Checkbox};
use gpui::{div, point, prelude::*, px, relative, uniform_list, AnyElement, App, Context, Entity, FocusHandle, Focusable, Hsla, MouseButton, Pixels, Point, ScrollStrategy, SharedString, UniformListScrollHandle, Window};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

const LIST_LIMIT: usize = 100;
const ROW_HEIGHT: f32 = 46.;
/// Re-list when coming back to the view after this long.
const STALE_AFTER: Duration = Duration::from_secs(60);

/// A pull request with everything the detail pane shows.
pub struct LoadedPr {
    pub pr: PullRequest,
    pub diffs: Option<Arc<Vec<FileDiff>>>,
    /// Why the diff couldn't be loaded (GitHub refuses very large diffs).
    pub diff_error: Option<String>,
}

pub struct PullRequestsState {
    /// The GitHub repository the remotes point at, if any.
    pub repo: Option<GitHubRepo>,
    /// Result of checking the gh CLI; `None` until checked.
    pub gh: Option<GhStatus>,
    pub checking: bool,
    pub default_branch: Option<String>,
    pub filter: PrFilter,
    pub prs: Vec<PullRequest>,
    pub loading: bool,
    pub loaded: bool,
    pub error: Option<String>,
    /// Open PR count for the sidebar badge.
    pub open_count: Option<usize>,
    pub selected: Option<usize>,
    pub selected_number: Option<u64>,
    pub current: Option<Arc<LoadedPr>>,
    pub detail_error: Option<String>,
    pub detail: DetailState,
    pub list_focus: FocusHandle,
    pub scroll: UniformListScrollHandle,
    cache: HashMap<u64, Arc<LoadedPr>>,
    pending_select: Option<u64>,
    /// Bumped when the GitHub repository changes, to drop stale results.
    generation: u64,
    list_generation: u64,
    last_loaded: Option<Instant>,
}

impl PullRequestsState {
    pub fn new(cx: &mut Context<RepoView>) -> Self {
        Self {
            repo: None,
            gh: None,
            checking: false,
            default_branch: None,
            filter: PrFilter::Open,
            prs: Vec::new(),
            loading: false,
            loaded: false,
            error: None,
            open_count: None,
            selected: None,
            selected_number: None,
            current: None,
            detail_error: None,
            detail: DetailState::new(cx),
            list_focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            cache: HashMap::new(),
            pending_select: None,
            generation: 0,
            list_generation: 0,
            last_loaded: None,
        }
    }

    fn reset(&mut self, repo: Option<GitHubRepo>) {
        self.repo = repo;
        self.gh = None;
        self.checking = false;
        self.default_branch = None;
        self.prs.clear();
        self.loading = false;
        self.loaded = false;
        self.error = None;
        self.open_count = None;
        self.selected = None;
        self.selected_number = None;
        self.current = None;
        self.detail_error = None;
        self.detail.clear();
        self.cache.clear();
        self.pending_select = None;
        self.generation += 1;
        self.list_generation += 1;
        self.last_loaded = None;
    }

    fn ready(&self) -> bool {
        self.repo.is_some() && self.gh == Some(GhStatus::Ready)
    }
}

/// Where and how the head branch gets pushed before the PR is opened.
#[derive(Clone)]
pub struct PushPlan {
    pub remote: String,
    pub remote_branch: String,
    pub set_upstream: bool,
}

pub struct CreatePrForm {
    pub repo: GitHubRepo,
    pub branch: String,
    /// What `gh pr create --head` gets: the branch, or `owner:branch` from a fork.
    pub head: String,
    pub push: Option<PushPlan>,
    pub bases: Vec<String>,
    pub base: usize,
    pub base_open: bool,
    pub base_filter: Entity<TextInput>,
    pub default_branch: Option<String>,
    pub title: Entity<TextInput>,
    pub body: Entity<TextInput>,
    pub draft: bool,
}

fn error_message(e: &anyhow::Error) -> String {
    match e.downcast_ref::<crate::git::GitError>() {
        Some(g) => g.message.clone(),
        None => e.to_string(),
    }
}

/// Pre-shortens text for places where GPUI's ellipsis can't be relied on.
fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// "feature/add-login" → "Add login", like `gh pr create --fill` does.
fn humanize_branch(branch: &str) -> String {
    let last = branch.rsplit('/').next().unwrap_or(branch).replace(['-', '_'], " ");
    let mut chars = last.trim().chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => branch.to_string(),
    }
}

fn merged_color(theme: &Theme) -> Hsla {
    match theme.mode {
        ThemeMode::Dark => gpui::rgb(0xa371f7).into(),
        ThemeMode::Light => gpui::rgb(0x8250df).into(),
    }
}

fn state_style(state: PrState, theme: &Theme) -> (Hsla, Icon) {
    match state {
        PrState::Open => (theme.success, Icon::PullRequest),
        PrState::Draft => (theme.text_faint, Icon::PullRequest),
        PrState::Merged => (merged_color(theme), Icon::Merge),
        PrState::Closed => (theme.danger, Icon::PullRequest),
    }
}

fn check_style(outcome: CheckOutcome, theme: &Theme) -> (Icon, Hsla) {
    match outcome {
        CheckOutcome::Success => (Icon::Check, theme.success),
        CheckOutcome::Failure => (Icon::Close, theme.danger),
        CheckOutcome::Pending => (Icon::Dot, theme.warning),
        CheckOutcome::Neutral => (Icon::Minus, theme.text_faint),
    }
}

fn review_badge(decision: ReviewDecision, theme: &Theme) -> impl IntoElement {
    let (bg, fg) = match decision {
        ReviewDecision::Approved => (theme.success.opacity(0.18), theme.success),
        ReviewDecision::ChangesRequested => (theme.danger.opacity(0.18), theme.danger),
        ReviewDecision::ReviewRequired => (theme.active, theme.text_muted),
    };
    badge(decision.label(), bg, fg)
}

fn label_chip(name: &str, color: &str, theme: &Theme) -> impl IntoElement {
    let c: Hsla = u32::from_str_radix(color, 16).map(|v| gpui::rgb(v).into()).unwrap_or(theme.text_faint);
    div()
        .flex_none()
        .h(px(17.))
        .px(px(6.))
        .flex()
        .items_center()
        .rounded(px(9.))
        .bg(c.opacity(0.22))
        .border_1()
        .border_color(c.opacity(0.6))
        .text_size(px(11.))
        .text_color(theme.text)
        .child(shorten(name, 24))
}

fn branch_chip(name: &str, theme: &Theme) -> impl IntoElement {
    div()
        .flex_none()
        .h(px(18.))
        .px(px(6.))
        .flex()
        .items_center()
        .gap(px(4.))
        .rounded(px(4.))
        .bg(theme.ref_local.opacity(0.18))
        .border_1()
        .border_color(theme.ref_local.opacity(0.5))
        .text_size(px(11.))
        .text_color(theme.text)
        .child(icon_sized(Icon::Branch, px(11.)).text_color(theme.ref_local))
        .child(shorten(name, 40))
}

/// An empty state with action buttons underneath.
fn notice(icon_kind: Icon, title: impl Into<SharedString>, detail: impl Into<SharedString>, actions: Vec<AnyElement>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .p(px(16.))
        .text_color(theme.text_faint)
        .child(icon_sized(icon_kind, px(36.)).text_color(theme.text_faint))
        .child(div().text_size(px(14.)).text_color(theme.text_muted).child(title.into()))
        .child(div().text_size(px(12.)).max_w(px(460.)).text_center().child(detail.into()))
        .when(!actions.is_empty(), |d| d.child(div().pt(px(6.)).flex().gap(px(8.)).children(actions)))
        .into_any_element()
}

impl RepoView {
    // ---------------------------------------------------------------------
    // Loading
    // ---------------------------------------------------------------------

    /// Follows the GitHub repository the remotes point at; checks gh again when it changes.
    pub fn sync_github_repo(&mut self, cx: &mut Context<Self>) {
        let found = github::find_github_repo(&self.snapshot.remotes);
        if found == self.prs.repo {
            return;
        }
        self.prs.reset(found);
        self.check_github(cx);
    }

    /// Checks that gh is installed and logged in, then loads the open PRs (for the sidebar count).
    pub fn check_github(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.prs.repo.clone() else { return };
        if self.prs.checking {
            return;
        }
        self.prs.checking = true;
        let generation = self.prs.generation;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (status, info) = cx
                .background_executor()
                .spawn(async move {
                    let status = github::gh_status(&repo.host);
                    let info = if status == GhStatus::Ready { github::repo_info(&repo).ok() } else { None };
                    (status, info)
                })
                .await;
            this.update(cx, |this, cx| {
                if this.prs.generation != generation {
                    return;
                }
                this.prs.checking = false;
                this.prs.default_branch = info.and_then(|i| i.default_branch);
                let ready = status == GhStatus::Ready;
                this.prs.gh = Some(status);
                if ready {
                    this.reload_pull_requests(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn reload_pull_requests(&mut self, cx: &mut Context<Self>) {
        if !self.prs.ready() {
            return;
        }
        let Some(repo) = self.prs.repo.clone() else { return };
        self.prs.list_generation += 1;
        let generation = self.prs.list_generation;
        self.prs.loading = true;
        let filter = self.prs.filter;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { github::list_pull_requests(&repo, filter, LIST_LIMIT) }).await;
            this.update(cx, |this, cx| {
                if this.prs.list_generation != generation {
                    return;
                }
                this.prs.loading = false;
                this.prs.last_loaded = Some(Instant::now());
                match result {
                    Ok(prs) => {
                        this.prs.error = None;
                        this.prs.loaded = true;
                        if filter == PrFilter::Open {
                            this.prs.open_count = Some(prs.len());
                        }
                        this.prs.prs = prs;
                        this.restore_pr_selection(cx);
                    }
                    Err(e) => this.prs.error = Some(error_message(&e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn reload_pull_requests_if_stale(&mut self, cx: &mut Context<Self>) {
        let stale = self.prs.last_loaded.is_none_or(|t| t.elapsed() > STALE_AFTER);
        if stale && !self.prs.loading {
            self.reload_pull_requests(cx);
        }
    }

    /// Refresh button and F5: reload the list and the selected PR.
    pub fn refresh_pull_requests(&mut self, cx: &mut Context<Self>) {
        if !self.prs.ready() {
            self.check_github(cx);
            return;
        }
        self.prs.cache.clear();
        self.prs.current = None;
        self.reload_pull_requests(cx);
    }

    /// Entering the view. gh is checked again when it wasn't ready, since the
    /// user may have just installed it or logged in.
    pub fn on_show_pull_requests(&mut self, cx: &mut Context<Self>) {
        if self.prs.repo.is_none() || self.prs.checking {
            return;
        }
        if !self.prs.ready() {
            self.check_github(cx);
            return;
        }
        self.reload_pull_requests_if_stale(cx);
        if self.prs.selected.is_none() && !self.prs.prs.is_empty() {
            self.select_pull_request(0, cx);
        }
    }

    fn set_pr_filter(&mut self, filter: PrFilter, cx: &mut Context<Self>) {
        if self.prs.filter == filter {
            return;
        }
        self.prs.filter = filter;
        self.prs.prs.clear();
        self.prs.loaded = false;
        self.prs.selected = None;
        self.prs.selected_number = None;
        self.prs.current = None;
        self.prs.detail.clear();
        self.reload_pull_requests(cx);
    }

    fn restore_pr_selection(&mut self, cx: &mut Context<Self>) {
        let wanted = self.prs.pending_select.take().or(self.prs.selected_number);
        match wanted.and_then(|n| self.prs.prs.iter().position(|p| p.number == n)) {
            Some(ix) => self.select_pull_request(ix, cx),
            None if self.nav == Nav::PullRequests && !self.prs.prs.is_empty() => self.select_pull_request(0, cx),
            None => {
                self.prs.selected = None;
                self.prs.selected_number = None;
                self.prs.current = None;
                self.prs.detail.clear();
            }
        }
    }

    pub fn select_pull_request(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(pr) = self.prs.prs.get(ix).cloned() else { return };
        self.prs.selected = Some(ix);
        self.prs.selected_number = Some(pr.number);
        self.prs.scroll.scroll_to_item(ix, ScrollStrategy::Top);
        cx.notify();
        let fresh = |l: &LoadedPr| l.pr.number == pr.number && l.pr.head_ref_oid == pr.head_ref_oid && l.pr.updated_at == pr.updated_at;
        if self.prs.current.as_deref().is_some_and(fresh) {
            return;
        }
        if let Some(loaded) = self.prs.cache.get(&pr.number).filter(|l| fresh(l)).cloned() {
            self.show_loaded_pr(loaded, cx);
            return;
        }
        let Some(repo) = self.prs.repo.clone() else { return };
        self.prs.current = None;
        self.prs.detail_error = None;
        self.prs.detail.clear();
        self.prs.detail.loading = true;
        let generation = self.prs.detail.generation;
        let number = pr.number;
        cx.spawn(async move |this, cx| {
            // Let quick keyboard navigation settle before going to the network.
            cx.background_executor().timer(Duration::from_millis(150)).await;
            let stale = this.update(cx, |this, _| this.prs.detail.generation != generation).unwrap_or(true);
            if stale {
                return;
            }
            let repo2 = repo.clone();
            let view = cx.background_executor().spawn(async move { github::view_pull_request(&repo, number) });
            let diff = cx.background_executor().spawn(async move { github::pull_request_diff(&repo2, number) });
            let (view, diff) = (view.await, diff.await);
            this.update(cx, |this, cx| {
                if this.prs.detail.generation != generation {
                    return;
                }
                this.prs.detail.loading = false;
                match view {
                    Ok(pr) => {
                        let (diffs, diff_error) = match diff {
                            Ok(d) => (Some(Arc::new(d)), None),
                            Err(e) => (None, Some(error_message(&e))),
                        };
                        let loaded = Arc::new(LoadedPr { pr, diffs, diff_error });
                        this.prs.cache.insert(number, loaded.clone());
                        this.show_loaded_pr(loaded, cx);
                    }
                    Err(e) => this.prs.detail_error = Some(error_message(&e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Feeds a loaded PR into the shared detail pane: its changed files become
    /// the file list, and GitHub's diff is used for each file.
    fn show_loaded_pr(&mut self, loaded: Arc<LoadedPr>, cx: &mut Context<Self>) {
        let pr = &loaded.pr;
        let files = github::changed_files(pr, loaded.diffs.as_deref().map(|d| d.as_slice()));
        let has_files = !files.is_empty();
        let time = pr.created_time();
        let author = pr.author.display().to_string();
        let state = &mut self.prs.detail;
        state.generation += 1;
        state.loading = false;
        state.selected_file = None;
        state.diff = None;
        state.prepared_diffs = Some(loaded.diffs.clone().unwrap_or_default());
        state.info_scroll.set_offset(point(px(0.), px(0.)));
        state.details = Some(CommitDetails {
            oid: pr.head_ref_oid.clone(),
            parents: vec![pr.base_ref_oid.clone()],
            author_name: author.clone(),
            author_email: String::new(),
            author_time: time,
            committer_name: author,
            committer_email: String::new(),
            committer_time: time,
            message: pr.title.clone(),
            files,
        });
        self.prs.detail_error = None;
        self.prs.current = Some(loaded);
        if has_files {
            self.select_detail_file(DiffLocation::PullRequest, 0, cx);
        }
        cx.notify();
    }

    fn pr_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.prs.prs.len();
        if len == 0 {
            return;
        }
        let cur = self.prs.selected.unwrap_or(0) as isize;
        self.select_pull_request((cur + delta).clamp(0, len as isize - 1) as usize, cx);
    }

    /// The open PR whose head is `branch` in this repository, if listed.
    fn pr_for_branch(&self, branch: &str) -> Option<usize> {
        self.prs.prs.iter().position(|p| p.head_ref_name == branch && !p.is_cross_repository && matches!(p.pr_state(), PrState::Open | PrState::Draft))
    }

    // ---------------------------------------------------------------------
    // Actions
    // ---------------------------------------------------------------------

    pub fn checkout_pull_request(&mut self, number: u64, cx: &mut Context<Self>) {
        let Some(gh_repo) = self.prs.repo.clone() else { return };
        let previous = self.previous_checkout();
        let existing: Vec<String> = self.snapshot.branches.iter().map(|b| b.name.clone()).collect();
        let label = format!("Checking out #{number}");
        let label2 = label.clone();
        self.run_op(
            label,
            move |repo| {
                github::checkout_pull_request(&repo.workdir, &gh_repo, number)?;
                Ok(repo.run(&["rev-parse", "--abbrev-ref", "HEAD"])?.trim().to_string())
            },
            move |this, r, cx| match r {
                Ok(branch) => {
                    if let Some(p) = previous {
                        if !existing.contains(&branch) {
                            this.push_undo(format!("Create {branch}"), UndoAction::CreateBranch { name: branch.clone() });
                        }
                        if p != branch {
                            this.push_undo(format!("Check out {branch}"), UndoAction::Checkout { previous: p });
                        }
                    }
                    this.toast_undoable(format!("Switched to {branch} (#{number})"), cx);
                }
                Err(e) => this.show_error(&label2, &e, cx),
            },
            cx,
        );
    }

    fn pr_menu(&self, ix: usize, pos: Point<Pixels>, cx: &mut Context<Self>) -> ContextMenu {
        let Some(pr) = self.prs.prs.get(ix).cloned() else { return ContextMenu::new(pos) };
        let number = pr.number;
        let (url1, url2) = (pr.url.clone(), pr.url.clone());
        let branch = pr.head_ref_name.clone();
        ContextMenu::new(pos)
            .header(format!("#{number} · {}", shorten(&pr.title, 40)))
            .item(format!("Check Out “{}”", shorten(&pr.head_ref_name, 40)), Some(Icon::Check), self.handler(cx, move |this, _, cx| this.checkout_pull_request(number, cx)))
            .item("Open in Browser", Some(Icon::Globe), self.handler(cx, move |_, _, cx| cx.open_url(&url1)))
            .separator()
            .item("Copy URL", Some(Icon::Copy), self.handler(cx, move |this, _, cx| this.copy_to_clipboard(url2.clone(), cx)))
            .item("Copy Branch Name", Some(Icon::Copy), self.handler(cx, move |this, _, cx| this.copy_to_clipboard(branch.clone(), cx)))
    }

    // ---------------------------------------------------------------------
    // Create pull request
    // ---------------------------------------------------------------------

    pub fn open_create_pull_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.prs.repo.clone() else {
            self.toast(ToastKind::Info, "This repository has no GitHub remote", None, cx);
            return;
        };
        match &self.prs.gh {
            Some(GhStatus::Ready) => {}
            Some(GhStatus::NotInstalled) => {
                self.toast(ToastKind::Info, "Install the GitHub CLI to create pull requests", Some("Get it from https://cli.github.com, then sign in with “gh auth login”.".into()), cx);
                return;
            }
            Some(GhStatus::NotLoggedIn(_)) => {
                self.toast(ToastKind::Info, "Sign in to GitHub first", Some("Run “gh auth login” in a terminal.".into()), cx);
                return;
            }
            None => {
                self.toast(ToastKind::Info, "Still connecting to GitHub…", None, cx);
                return;
            }
        }
        let Some(branch) = self.snapshot.head.branch.clone() else {
            self.toast(ToastKind::Info, "Check out a branch to create a pull request", None, cx);
            return;
        };
        if let Some(ix) = self.pr_for_branch(&branch) {
            let number = self.prs.prs[ix].number;
            self.set_nav(Nav::PullRequests, window, cx);
            self.select_pull_request(ix, cx);
            self.toast(ToastKind::Info, format!("Pull request #{number} already exists for {branch}"), None, cx);
            return;
        }

        // The branch is pushed to its upstream, or to origin (a fork, in the usual
        // fork setup) when it has none yet.
        let local = self.snapshot.branches.iter().find(|b| b.name == branch).cloned();
        let upstream = local.as_ref().and_then(|b| b.upstream.clone()).and_then(|up| up.split_once('/').map(|(r, b)| (r.to_string(), b.to_string())));
        let push_remote = match &upstream {
            Some((r, _)) => r.clone(),
            None if self.snapshot.remotes.iter().any(|r| r.name == "origin") => "origin".to_string(),
            None => repo.remote.clone(),
        };
        let remote_branch = upstream.as_ref().map(|(_, b)| b.clone()).unwrap_or_else(|| branch.clone());
        let needs_push = upstream.is_none() || local.as_ref().is_some_and(|b| b.ahead > 0 || b.upstream_gone);
        let push = needs_push.then(|| PushPlan { remote: push_remote.clone(), remote_branch: remote_branch.clone(), set_upstream: upstream.is_none() });
        let head_repo = self.snapshot.remotes.iter().find(|r| r.name == push_remote).and_then(|r| github::parse_github_url(&r.fetch_url));
        let head = match head_repo {
            Some((_, owner, name)) if owner != repo.owner || name != repo.name => format!("{owner}:{remote_branch}"),
            _ => remote_branch.clone(),
        };

        // Base candidates: the target's branches, default branch first.
        let same_repo = !head.contains(':');
        let default = self.prs.default_branch.clone();
        if same_repo && default.as_deref() == Some(remote_branch.as_str()) {
            self.toast(ToastKind::Info, format!("“{branch}” is the default branch"), Some("Create a branch for your changes, then open a pull request from it.".into()), cx);
            return;
        }
        let mut bases: Vec<String> = self.snapshot.remote_branches.iter().filter(|r| r.remote == repo.remote && !(same_repo && r.name == remote_branch)).map(|r| r.name.clone()).collect();
        let rank = |b: &str| {
            if Some(b) == default.as_deref() {
                0
            } else {
                match b {
                    "main" => 1,
                    "master" => 2,
                    "develop" | "dev" => 3,
                    _ => 4,
                }
            }
        };
        bases.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.to_lowercase().cmp(&b.to_lowercase())));
        if bases.is_empty() {
            bases.push(default.clone().unwrap_or_else(|| "main".into()));
        }

        // Prefill like `gh pr create --fill`: one commit gives title and body,
        // several give a humanized branch name and a list of their subjects.
        let range = format!("{}/{}..{}", repo.remote, bases[0], branch);
        let log = self.repo.run(&["log", "--reverse", "--format=%s%x1f%b%x1e", &range]).unwrap_or_default();
        let commits: Vec<(String, String)> = log
            .split('\x1e')
            .map(|e| e.trim_matches('\n'))
            .filter(|e| !e.trim().is_empty())
            .map(|e| {
                let (s, b) = e.split_once('\x1f').unwrap_or((e, ""));
                (s.trim().to_string(), b.trim().to_string())
            })
            .collect();
        let (title_text, body_text) = match commits.as_slice() {
            [(s, b)] => (s.clone(), b.clone()),
            _ => (humanize_branch(&branch), commits.iter().map(|(s, _)| format!("- {s}")).collect::<Vec<_>>().join("\n")),
        };

        let (title, sub) = self.new_input("Title", &title_text, window, cx);
        let body = cx.new(|cx| TextInput::new(cx).multiline().with_placeholder("Describe your changes (optional)").with_text(body_text));
        // Enter in the filter picks a branch; it must never submit the form.
        let base_filter = cx.new(|cx| TextInput::new(cx).with_placeholder("Filter branches"));
        let filter_sub = cx.subscribe_in(&base_filter, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::Changed => cx.notify(),
            InputEvent::Submit => this.pick_filtered_base(window, cx),
            InputEvent::Focused | InputEvent::Blurred => {}
        });
        let form = CreatePrForm { repo, branch, head, push, bases, base: 0, base_open: false, base_filter, default_branch: default, title: title.clone(), body, draft: false };
        self.show_modal(ModalKind::CreatePullRequest(Box::new(form)), vec![sub, filter_sub], Some(&title), window, cx);
    }

    fn filtered_bases(form: &CreatePrForm, cx: &App) -> Vec<usize> {
        let query = form.base_filter.read(cx).text().trim().to_lowercase();
        form.bases.iter().enumerate().filter(|(_, b)| query.is_empty() || b.to_lowercase().contains(&query)).map(|(i, _)| i).collect()
    }

    fn pick_filtered_base(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Modal { kind: ModalKind::CreatePullRequest(form), .. }) = self.modal.as_ref() else { return };
        let Some(&ix) = Self::filtered_bases(form, cx).first() else { return };
        self.choose_base(ix, window, cx);
    }

    fn choose_base(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Modal { kind: ModalKind::CreatePullRequest(form), .. }) = self.modal.as_mut() else { return };
        form.base = ix;
        form.base_open = false;
        let title = form.title.clone();
        window.focus(&title.focus_handle(cx));
        cx.notify();
    }

    pub fn submit_create_pull_request(&mut self, modal: Modal, window: &mut Window, cx: &mut Context<Self>) {
        let ModalKind::CreatePullRequest(form) = &modal.kind else { return };
        let title = form.title.read(cx).text().trim().to_string();
        if title.is_empty() {
            self.toast(ToastKind::Error, "Enter a title for the pull request", None, cx);
            self.modal = Some(modal);
            return;
        }
        let Some(base) = form.bases.get(form.base).cloned() else {
            self.modal = Some(modal);
            return;
        };
        let body = form.body.read(cx).text().trim().to_string();
        let (repo, branch, head, push, draft) = (form.repo.clone(), form.branch.clone(), form.head.clone(), form.push.clone(), form.draft);
        self.close_modal(window, cx);
        let label = match &push {
            Some(p) => format!("Pushing {branch} to {} and creating pull request", p.remote),
            None => "Creating pull request".to_string(),
        };
        self.run_op(
            label,
            move |git| {
                if let Some(p) = &push {
                    git.push_branch_to(&p.remote, &branch, &p.remote_branch, p.set_upstream, false, |_| {})?;
                }
                github::create_pull_request(&git.workdir, &repo, &NewPullRequest { head, base, title, body, draft })
            },
            move |this, r, cx| match r {
                Ok(url) => {
                    let number = github::number_from_url(&url);
                    let title = match number {
                        Some(n) => format!("Created pull request #{n}"),
                        None => "Created pull request".to_string(),
                    };
                    this.toast(ToastKind::Success, title, Some(url), cx);
                    if this.prs.filter == PrFilter::Closed {
                        this.prs.filter = PrFilter::Open;
                    }
                    this.prs.pending_select = number;
                    this.nav = Nav::PullRequests;
                    this.reload_pull_requests(cx);
                }
                Err(e) => this.show_error("Create pull request", &e, cx),
            },
            cx,
        );
    }

    pub fn render_create_pull_request(&self, form: &CreatePrForm, cx: &mut Context<Self>) -> (Option<SharedString>, f32, AnyElement, Option<AnyElement>) {
        let theme = cx.theme().clone();
        let base = form.bases.get(form.base).cloned().unwrap_or_default();
        let toggle = Button::new("pr-base", format!("into {}", shorten(&base, 36))).small().icon(if form.base_open { Icon::ChevronDown } else { Icon::ChevronRight }).on_click(cx.listener(|this, _, window, cx| {
            if let Some(Modal { kind: ModalKind::CreatePullRequest(f), .. }) = this.modal.as_mut() {
                f.base_open = !f.base_open;
                let focus = if f.base_open { f.base_filter.focus_handle(cx) } else { f.title.focus_handle(cx) };
                window.focus(&focus);
            }
            cx.notify();
        }));
        let branches = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .child(branch_chip(&form.head, &theme))
            .child(div().text_color(theme.text_faint).child("→"))
            .child(toggle);
        let shown = Self::filtered_bases(form, cx);
        let mut list = div().id("pr-base-list").max_h(px(180.)).overflow_y_scroll().flex().flex_col().py(px(2.)).rounded(px(5.)).border_1().border_color(theme.border).bg(theme.input_bg);
        if shown.is_empty() {
            list = list.child(div().h(px(24.)).px(px(8.)).flex().items_center().text_size(px(12.)).text_color(theme.text_faint).child("No matching branches"));
        }
        for i in shown {
            let b = &form.bases[i];
            let selected = i == form.base;
            let is_default = form.default_branch.as_deref() == Some(b.as_str());
            list = list.child(
                div()
                    .id(("pr-base-item", i))
                    .flex_none()
                    .h(px(24.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(theme.selection))
                    .when(!selected, |d| d.hover(|s| s.bg(theme.hover)))
                    .child(icon(Icon::Branch).text_color(theme.text_muted))
                    .child(shorten(b, 76))
                    .when(is_default, |d| d.child(div().text_color(theme.text_faint).child("default")))
                    .on_click(cx.listener(move |this, _, window, cx| this.choose_base(i, window, cx))),
            );
        }
        let push_note = form.push.as_ref().map(|p| {
            let text = if p.set_upstream {
                format!("“{}” isn't on {} yet. It will be pushed there (and tracked) before the pull request is created.", form.branch, p.remote)
            } else {
                format!("“{}” has commits that aren't on {} yet. They will be pushed before the pull request is created.", form.branch, p.remote)
            };
            div()
                .flex()
                .gap(px(8.))
                .p(px(8.))
                .rounded(px(5.))
                .bg(theme.info.opacity(0.12))
                .border_1()
                .border_color(theme.info.opacity(0.35))
                .text_size(px(12.))
                .child(icon(Icon::Push).text_color(theme.info))
                .child(div().flex_1().min_w_0().whitespace_normal().child(text))
        });
        let draft = form.draft;
        let body = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(div().text_size(px(12.)).text_color(theme.text_faint).child(format!("On {}", form.repo.full_name())))
            .child(branches)
            .when(form.base_open, |d| d.child(form.base_filter.clone()).child(list))
            .child(field_label("Title", cx))
            .child(form.title.clone())
            .child(field_label("Description", cx))
            .child(div().h(px(140.)).child(form.body.clone()))
            .child(Checkbox::new("pr-draft", draft).label("Create as draft").on_toggle(cx.listener(|this, v: &bool, _, cx| {
                if let Some(Modal { kind: ModalKind::CreatePullRequest(f), .. }) = this.modal.as_mut() {
                    f.draft = *v;
                }
                cx.notify();
            })))
            .children(push_note)
            .into_any_element();
        (Some("Create Pull Request".into()), 560., body, Some(self.footer(if draft { "Create Draft Pull Request" } else { "Create Pull Request" }, false, true, cx)))
    }

    // ---------------------------------------------------------------------
    // Rendering
    // ---------------------------------------------------------------------

    pub fn render_pull_requests(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.render_pr_header(cx);
        let body = self.render_pr_body(window, cx);
        div().size_full().flex().flex_col().child(header).child(div().flex_1().min_h_0().child(body))
    }

    fn render_pr_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let ready = self.prs.ready();
        let busy = self.prs.loading || self.prs.checking || self.prs.detail.loading;
        let mut filters = div().flex().items_center().gap(px(2.));
        for (i, f) in [PrFilter::Open, PrFilter::Closed, PrFilter::All].into_iter().enumerate() {
            filters = filters.child(Button::new(("pr-filter", i), f.label()).small().ghost().selected(self.prs.filter == f).on_click(cx.listener(move |this, _, _, cx| this.set_pr_filter(f, cx))));
        }
        let branch = self.snapshot.head.branch.clone();
        let action: AnyElement = match branch.as_deref().and_then(|b| self.pr_for_branch(b)) {
            Some(ix) => {
                let n = self.prs.prs[ix].number;
                Button::new("show-branch-pr", format!("Show #{n}")).small().icon(Icon::PullRequest).tooltip("The current branch already has a pull request").on_click(cx.listener(move |this, _, _, cx| this.select_pull_request(ix, cx))).into_any_element()
            }
            None => Button::new("create-pr", "Create Pull Request…")
                .small()
                .primary()
                .icon(Icon::Plus)
                .disabled(branch.is_none() || self.is_busy())
                .tooltip(if branch.is_some() { "Open a pull request for the current branch" } else { "Check out a branch first" })
                .on_click(cx.listener(|this, _, window, cx| this.open_create_pull_request(window, cx)))
                .into_any_element(),
        };
        let count = self.prs.prs.len();
        div()
            .flex_none()
            .h(px(42.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(theme.border_subtle)
            .child(icon(Icon::PullRequest).text_color(theme.text_muted))
            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).whitespace_nowrap().child("Pull Requests"))
            .when_some(self.prs.repo.as_ref(), |d, r| d.child(div().text_size(px(12.)).text_color(theme.text_faint).whitespace_nowrap().child(r.full_name())))
            .when(ready && self.prs.loaded, |d| d.child(div().text_size(px(12.)).text_color(theme.text_faint).child(format!("{count}{}", if count >= LIST_LIMIT { "+" } else { "" }))))
            .when(ready, |d| d.child(filters))
            .child(div().flex_1())
            .when(busy, |d| d.child(super::spinner(cx)))
            .when(ready, |d| {
                d.child(Button::icon_only("reload-prs", Icon::Refresh).small().tooltip("Reload").on_click(cx.listener(|this, _, _, cx| this.refresh_pull_requests(cx))))
                    .child(action)
            })
            .into_any_element()
    }

    fn render_pr_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(repo) = self.prs.repo.clone() else {
            return empty_state(Icon::PullRequest, "No GitHub remote", "Pull requests are available when one of this repository's remotes points at GitHub.", cx);
        };
        let check_again = || Button::new("gh-check", "Check Again").small().on_click(cx.listener(|this, _, _, cx| this.check_github(cx))).into_any_element();
        match self.prs.gh.clone() {
            None => div().size_full().flex().items_center().justify_center().child(super::spinner(cx)).into_any_element(),
            Some(GhStatus::NotInstalled) => notice(
                Icon::Package,
                "GitHub CLI not found",
                "Gitter uses the GitHub CLI (gh) for pull requests, so your GitHub credentials stay with gh. Install it, sign in with “gh auth login”, then check again.",
                vec![Button::new("gh-install", "Get the GitHub CLI").small().primary().icon(Icon::Globe).on_click(|_, _, cx| cx.open_url("https://cli.github.com")).into_any_element(), check_again()],
                cx,
            ),
            Some(GhStatus::NotLoggedIn(message)) => {
                let detail = format!("The GitHub CLI isn't signed in to {}. Run “gh auth login” in a terminal, then check again.\n\n{}", repo.host, shorten(&message, 200));
                notice(
                    Icon::User,
                    "Sign in to GitHub",
                    detail,
                    vec![Button::new("gh-terminal", "Open Terminal").small().primary().icon(Icon::Terminal).on_click(cx.listener(|this, _, _, cx| this.open_terminal(cx))).into_any_element(), check_again()],
                    cx,
                )
            }
            Some(GhStatus::Ready) => {
                if let Some(err) = self.prs.error.clone() {
                    let retry = Button::new("prs-retry", "Try Again").small().on_click(cx.listener(|this, _, _, cx| this.refresh_pull_requests(cx))).into_any_element();
                    return notice(Icon::Warning, "Could not load pull requests", shorten(&err, 400), vec![retry], cx);
                }
                if !self.prs.loaded {
                    return div().size_full().flex().items_center().justify_center().child(super::spinner(cx)).into_any_element();
                }
                if self.prs.prs.is_empty() {
                    let (title, detail) = match self.prs.filter {
                        PrFilter::Open => ("No open pull requests", format!("There are no open pull requests on {}.", repo.full_name())),
                        PrFilter::Closed => ("No closed pull requests", String::new()),
                        PrFilter::All => ("No pull requests yet", format!("{} doesn't have any pull requests.", repo.full_name())),
                    };
                    return notice(Icon::PullRequest, title, detail, Vec::new(), cx);
                }
                let focused = self.prs.list_focus.is_focused(window);
                let count = self.prs.prs.len();
                let list = div()
                    .id("pr-list-container")
                    .key_context("List")
                    .track_focus(&self.prs.list_focus)
                    .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.pr_move(-1, cx)))
                    .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.pr_move(1, cx)))
                    .on_action(cx.listener(|this, _: &SelectFirst, _, cx| this.pr_move(-1_000_000, cx)))
                    .on_action(cx.listener(|this, _: &SelectLast, _, cx| this.pr_move(1_000_000, cx)))
                    .size_full()
                    .child(
                        uniform_list("pr-list", count, cx.processor(move |this: &mut RepoView, range: Range<usize>, _window, cx| range.map(|ix| this.render_pr_row(ix, focused, cx)).collect::<Vec<_>>()))
                            .track_scroll(self.prs.scroll.clone())
                            .size_full(),
                    )
                    .into_any_element();
                self.split_view(list, DiffLocation::PullRequest, window, cx)
            }
        }
    }

    fn render_pr_row(&self, ix: usize, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(pr) = self.prs.prs.get(ix) else { return div().into_any_element() };
        let selected = self.prs.selected == Some(ix);
        let bg = match (selected, focused) {
            (true, true) => theme.selection,
            (true, false) => theme.selection_inactive,
            _ => gpui::transparent_black(),
        };
        let state = pr.pr_state();
        let (state_color, state_icon) = state_style(state, &theme);
        let checks = pr.checks();
        let mut title_row = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .min_w_0()
            .child(div().min_w_0().whitespace_nowrap().overflow_hidden().text_ellipsis().text_color(theme.text).child(pr.title.clone()))
            .when(state == PrState::Draft, |d| d.child(badge("Draft", theme.active, theme.text_muted)));
        for l in pr.labels.iter().take(2) {
            title_row = title_row.child(label_chip(&l.name, &l.color, &theme));
        }
        let meta = div()
            .flex()
            .items_center()
            .gap(px(5.))
            .min_w_0()
            .overflow_hidden()
            .text_size(px(12.))
            .text_color(theme.text_faint)
            .child(div().flex_none().child(format!("#{}", pr.number)))
            .child(icon_sized(Icon::Branch, px(11.)))
            .child(div().min_w_0().whitespace_nowrap().overflow_hidden().text_ellipsis().child(format!("{} → {}", pr.head_label(), pr.base_ref_name)));
        let checks_cell = div().id(("pr-checks", ix)).w(px(22.)).flex_none().flex().justify_center().when_some(checks.overall(), |d, o| {
            let (i, c) = check_style(o, &theme);
            d.child(icon(i).text_color(c)).tooltip(ui::tooltip(checks.describe()))
        });
        let number = pr.number;
        div()
            .id(("pr", ix))
            .w_full()
            .h(px(ROW_HEIGHT))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(10.))
            .bg(bg)
            .when(!selected, |d| d.hover(|s| s.bg(theme.hover)))
            .text_size(px(13.))
            .child(icon(state_icon).text_color(state_color))
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(3.)).child(title_row).child(meta))
            .child(checks_cell)
            .child(div().w(px(124.)).flex_none().flex().when_some(pr.review_decision(), |d, r| d.child(review_badge(r, &theme))))
            .child(
                div()
                    .w(px(140.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .overflow_hidden()
                    .text_color(theme.text_muted)
                    .child(ui::avatar(pr.author.display(), px(16.), cx))
                    .child(div().whitespace_nowrap().overflow_hidden().text_ellipsis().child(pr.author.display().to_string())),
            )
            .child(div().w(px(96.)).flex_none().text_size(px(12.)).text_color(theme.text_faint).whitespace_nowrap().child(ui::relative_time(pr.updated_time())))
            .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                window.focus(&this.prs.list_focus);
                this.select_pull_request(ix, cx);
                if e.click_count() >= 2 {
                    this.checkout_pull_request(number, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                    window.focus(&this.prs.list_focus);
                    this.select_pull_request(ix, cx);
                    let menu = this.pr_menu(ix, e.position, cx);
                    this.open_menu(menu, cx);
                }),
            )
            .into_any_element()
    }

    /// The detail pane's info block for the selected PR (in place of commit info).
    pub fn render_pr_info(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(loaded) = self.prs.current.clone() else { return div().into_any_element() };
        let pr = &loaded.pr;
        let state = pr.pr_state();
        let (state_color, state_icon) = state_style(state, &theme);
        let checks = pr.checks();
        let body = github::display_body(&pr.body);
        let commits = pr.commits.len();
        let meta_row = |label: &'static str, value: AnyElement| {
            div().flex().items_start().gap(px(8.)).child(div().w(px(64.)).flex_none().text_color(theme.text_faint).child(label)).child(div().flex_1().min_w_0().flex().flex_col().gap(px(4.)).child(value))
        };

        let state_pill = div()
            .flex_none()
            .h(px(20.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(4.))
            .rounded(px(10.))
            .bg(state_color.opacity(0.2))
            .text_color(state_color)
            .text_size(px(11.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .child(icon_sized(state_icon, px(12.)).text_color(state_color))
            .child(state.label());
        let plural = if commits == 1 { "" } else { "s" };
        let who = pr.author.display();
        let what = match state {
            PrState::Merged => format!("#{} · {commits} commit{plural} by {who} merged into", pr.number),
            PrState::Closed => format!("#{} · {who} wanted to merge {commits} commit{plural} into", pr.number),
            _ => format!("#{} · {who} wants to merge {commits} commit{plural} into", pr.number),
        };
        let summary = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(5.))
            .text_color(theme.text_muted)
            .child(what)
            .child(branch_chip(&pr.base_ref_name, &theme))
            .child("from")
            .child(branch_chip(&pr.head_label(), &theme));

        let number = pr.number;
        let (url1, url2) = (pr.url.clone(), pr.url.clone());
        let actions = div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .child(Button::new("pr-checkout", "Check Out").small().primary().icon(Icon::Check).disabled(self.is_busy()).tooltip(format!("Check out {} locally", shorten(&pr.head_ref_name, 40))).on_click(cx.listener(move |this, _, _, cx| this.checkout_pull_request(number, cx))))
            .child(Button::new("pr-browser", "Open in Browser").small().icon(Icon::Globe).on_click(move |_, _, cx| cx.open_url(&url1)))
            .child(Button::icon_only("pr-copy-url", Icon::Copy).small().tooltip("Copy URL").on_click(cx.listener(move |this, _, _, cx| this.copy_to_clipboard(url2.clone(), cx))));

        // Only failing and pending checks are listed; passing ones are in the summary.
        let mut check_items: Vec<_> = pr.status_check_rollup.iter().filter(|c| matches!(c.outcome(), CheckOutcome::Failure | CheckOutcome::Pending)).collect();
        check_items.sort_by_key(|c| c.outcome() != CheckOutcome::Failure);
        let mut checks_el = div().flex().flex_col().gap(px(3.));
        if let Some(o) = checks.overall() {
            let (i, c) = check_style(o, &theme);
            checks_el = checks_el.child(div().flex().items_center().gap(px(5.)).text_color(theme.text).child(icon(i).text_color(c)).child(checks.describe()));
        } else {
            checks_el = checks_el.child(div().text_color(theme.text_muted).child("No checks"));
        }
        const SHOWN_CHECKS: usize = 6;
        for (i, c) in check_items.iter().take(SHOWN_CHECKS).enumerate() {
            let (ic, color) = check_style(c.outcome(), &theme);
            let url = c.url().to_string();
            let label = if c.workflow_name.is_empty() { c.label().to_string() } else { format!("{} / {}", c.workflow_name, c.label()) };
            checks_el = checks_el.child(
                div()
                    .id(("pr-check", i))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_color(theme.text_muted)
                    .child(icon_sized(ic, px(12.)).text_color(color))
                    .child(shorten(&label, 56))
                    .when(!url.is_empty(), |d| d.cursor_pointer().hover(|s| s.text_color(theme.accent_hover)).on_click(move |_, _, cx| cx.open_url(&url))),
            );
        }
        if check_items.len() > SHOWN_CHECKS {
            checks_el = checks_el.child(div().text_color(theme.text_faint).child(format!("and {} more", check_items.len() - SHOWN_CHECKS)));
        }

        let mut reviews_el = div().flex().flex_col().gap(px(3.));
        if let Some(d) = pr.review_decision() {
            reviews_el = reviews_el.child(div().flex().child(review_badge(d, &theme)));
        }
        for r in &pr.latest_reviews {
            let (text, color) = match r.state.as_str() {
                "APPROVED" => ("approved", theme.success),
                "CHANGES_REQUESTED" => ("requested changes", theme.danger),
                "DISMISSED" => ("review dismissed", theme.text_faint),
                _ => ("commented", theme.text_faint),
            };
            reviews_el = reviews_el.child(div().flex().items_center().gap(px(5.)).child(ui::avatar(r.author.display(), px(16.), cx)).child(div().text_color(theme.text).child(r.author.display().to_string())).child(div().text_color(color).child(text)));
        }
        for a in &pr.review_requests {
            reviews_el = reviews_el.child(div().flex().items_center().gap(px(5.)).child(ui::avatar(a.display(), px(16.), cx)).child(div().text_color(theme.text).child(a.display().to_string())).child(div().text_color(theme.text_faint).child("review requested")));
        }
        let has_reviews = pr.review_decision().is_some() || !pr.latest_reviews.is_empty() || !pr.review_requests.is_empty();

        let mut labels = div().flex().flex_wrap().gap(px(4.));
        for l in &pr.labels {
            labels = labels.child(label_chip(&l.name, &l.color, &theme));
        }

        div()
            .id("pr-info")
            .flex_none()
            // Leave room for the changed files below.
            .max_h(relative(0.6))
            .overflow_y_scroll()
            .track_scroll(&self.prs.detail.info_scroll)
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .border_b_1()
            .border_color(theme.border_subtle)
            .text_size(px(12.))
            .child(div().flex().items_start().gap(px(8.)).child(state_pill).child(div().flex_1().min_w_0().text_size(px(14.)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(theme.text).child(pr.title.clone())))
            .child(summary)
            .child(actions)
            .child(meta_row(
                "Author",
                div().flex().items_center().gap(px(6.)).child(ui::avatar(pr.author.display(), px(18.), cx)).child(div().text_color(theme.text).child(pr.author.display().to_string())).child(div().text_color(theme.text_faint).child(format!("opened {}", ui::relative_time(pr.created_time())))).into_any_element(),
            ))
            .child(meta_row("Checks", checks_el.into_any_element()))
            .when(has_reviews, |d| d.child(meta_row("Reviews", reviews_el.into_any_element())))
            .when(!pr.labels.is_empty(), |d| d.child(meta_row("Labels", labels.into_any_element())))
            .child(div().h(px(1.)).bg(theme.border_subtle))
            .child(if body.is_empty() {
                div().text_color(theme.text_faint).child("No description provided.")
            } else {
                div().text_color(theme.text_muted).whitespace_normal().child(body)
            })
            .into_any_element()
    }

    /// What the PR detail pane shows when there's no loaded PR.
    pub fn render_pr_placeholder(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.prs.detail_error {
            Some(err) => {
                let retry = Button::new("pr-detail-retry", "Try Again").small().on_click(cx.listener(|this, _, _, cx| {
                    if let Some(ix) = this.prs.selected {
                        this.prs.current = None;
                        this.prs.cache.clear();
                        this.select_pull_request(ix, cx);
                    }
                }));
                notice(Icon::Warning, "Could not load this pull request", shorten(err, 400), vec![retry.into_any_element()], cx)
            }
            None => empty_state(Icon::PullRequest, "No pull request selected", "Select a pull request to see its description and changes.", cx),
        }
    }

    /// Replaces the diff pane when GitHub didn't return a diff for the PR.
    pub fn pr_diff_unavailable(&self, loc: DiffLocation, cx: &mut Context<Self>) -> Option<AnyElement> {
        if loc != DiffLocation::PullRequest {
            return None;
        }
        let current = self.prs.current.as_ref()?;
        let err = current.diff_error.clone()?;
        let url = format!("{}/files", current.pr.url);
        let open = Button::new("pr-diff-web", "View Changes on GitHub").small().icon(Icon::Globe).on_click(move |_, _, cx| cx.open_url(&url)).into_any_element();
        Some(notice(Icon::File, "Diff not available", shorten(&err, 300), vec![open], cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanizes_branch_names() {
        assert_eq!(humanize_branch("feature/add-login_page"), "Add login page");
        assert_eq!(humanize_branch("fix"), "Fix");
        assert_eq!(shorten("abcdef", 4), "abc…");
        assert_eq!(shorten("abc", 4), "abc");
    }
}
