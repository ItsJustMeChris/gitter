//! The main view for one open repository.

pub mod blame;
pub mod details;
pub mod dialogs;
pub mod diff_view;
pub mod history;
pub mod menus;
pub mod palette;
pub mod pull_requests;
pub mod sidebar;
pub mod stashes;
pub mod toolbar;
pub mod undo;
pub mod working_copy;

use crate::actions::*;
use crate::assets::Icon;
use crate::git::{self, LogScope, Oid, RefLabel, Repo, RepoState, Snapshot};
use crate::settings::Settings;
use crate::theme::ActiveTheme;
use crate::ui::menu::{render_menu, ContextMenu, Handler};
use crate::ui::text_input::{InputEvent, TextInput};
use crate::ui::{self, icon, SplitDrag};
use anyhow::Result;
use gpui::{
    div, prelude::*, px, App, ClipboardItem, Context, DragMoveEvent, EventEmitter, FocusHandle, Focusable, MouseButton, Pixels, SharedString, Subscription, Task,
    Window,
};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

pub use details::DetailState;
pub use dialogs::Modal;
pub use history::HistoryState;
pub use pull_requests::PullRequestsState;
pub use sidebar::SidebarState;
pub use stashes::{ReflogState, StashViewState};
pub use working_copy::WorkingCopyState;

#[derive(Clone, Debug, PartialEq)]
pub enum Nav {
    WorkingCopy,
    History,
    Stashes,
    Reflog,
    PullRequests,
    Blame(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Success,
    Error,
    Info,
}

pub struct Toast {
    pub id: usize,
    pub kind: ToastKind,
    pub title: SharedString,
    pub detail: Option<String>,
    pub undoable: bool,
}

pub struct Busy {
    pub label: String,
    pub progress: Option<String>,
}

/// Events the workspace listens to.
pub enum RepoEvent {
    TitleChanged,
    OpenRepository(std::path::PathBuf),
}

impl EventEmitter<RepoEvent> for RepoView {}

pub struct RepoView {
    pub repo: Arc<Repo>,
    pub focus_handle: FocusHandle,
    pub snapshot: Snapshot,
    pub loaded: bool,
    pub refs_by_oid: HashMap<Oid, Vec<RefLabel>>,
    pub unpushed: HashSet<Oid>,
    pub nav: Nav,
    pub sidebar: SidebarState,
    pub history: HistoryState,
    pub wc: WorkingCopyState,
    pub stash_view: StashViewState,
    pub reflog: ReflogState,
    pub prs: PullRequestsState,
    pub blame: Option<blame::BlameState>,
    pub modal: Option<Modal>,
    pub menu: Option<ContextMenu>,
    pub toasts: Vec<Toast>,
    pub busy: Option<Busy>,
    pub undo_stack: Vec<undo::UndoEntry>,
    pub sidebar_width: Pixels,
    pub history_split: f32,
    pub details_split: f32,
    pub wc_split: f32,
    pub last_fetch: Option<std::time::Instant>,
    refreshing: bool,
    refresh_pending: bool,
    next_toast_id: usize,
    _watcher: Option<notify::RecommendedWatcher>,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl RepoView {
    pub fn new(repo: Repo, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = cx.global::<Settings>().clone();
        let subject = cx.new(|cx| TextInput::new(cx).with_placeholder("Commit summary"));
        let body = cx.new(|cx| TextInput::new(cx).multiline().with_placeholder("Detailed description (optional)"));
        let search = cx.new(|cx| TextInput::new(cx).with_placeholder("Search message, author or SHA…"));
        let filter = cx.new(|cx| TextInput::new(cx).with_placeholder("Filter branches and tags"));

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(&subject, window, |this, _, ev: &InputEvent, window, cx| {
            if *ev == InputEvent::Submit {
                this.commit(window, cx);
            }
        }));
        subscriptions.push(cx.subscribe_in(&body, window, |this, _, ev: &InputEvent, window, cx| {
            if *ev == InputEvent::Submit {
                this.commit(window, cx);
            }
        }));
        subscriptions.push(cx.subscribe(&search, |this, _, ev: &InputEvent, cx| match ev {
            InputEvent::Changed => this.on_search_changed(cx),
            _ => {}
        }));
        subscriptions.push(cx.subscribe(&filter, |_, _, ev: &InputEvent, cx| {
            if *ev == InputEvent::Changed {
                cx.notify();
            }
        }));
        subscriptions.push(cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.refresh(cx);
                if this.nav == Nav::PullRequests {
                    this.reload_pull_requests_if_stale(cx);
                }
            }
        }));

        let repo = Arc::new(repo);
        let scope = if settings.show_all_branches { LogScope::All } else { LogScope::Head };
        let mut this = Self {
            repo,
            focus_handle: cx.focus_handle(),
            snapshot: Snapshot::default(),
            loaded: false,
            refs_by_oid: HashMap::new(),
            unpushed: HashSet::new(),
            nav: Nav::WorkingCopy,
            sidebar: SidebarState::new(filter),
            history: HistoryState::new(scope, search, cx),
            wc: WorkingCopyState::new(subject, body, cx),
            stash_view: StashViewState::new(cx),
            reflog: ReflogState::new(cx),
            prs: PullRequestsState::new(cx),
            blame: None,
            modal: None,
            menu: None,
            toasts: Vec::new(),
            busy: None,
            undo_stack: Vec::new(),
            sidebar_width: px(settings.sidebar_width.clamp(160., 600.)),
            history_split: settings.history_split.clamp(0.15, 0.85),
            details_split: settings.details_split.clamp(0.15, 0.85),
            wc_split: settings.working_copy_split.clamp(0.2, 0.8),
            last_fetch: None,
            refreshing: false,
            refresh_pending: false,
            next_toast_id: 0,
            _watcher: None,
            _tasks: Vec::new(),
            _subscriptions: subscriptions,
        };
        this.start_watcher(cx);
        this.start_auto_fetch(cx);
        this.refresh(cx);
        this
    }

    pub fn title(&self) -> String {
        self.repo.name()
    }

    pub fn path(&self) -> &Path {
        &self.repo.workdir
    }

    pub fn is_busy(&self) -> bool {
        self.busy.is_some()
    }

    pub fn state(&self) -> RepoState {
        self.snapshot.state.unwrap_or(RepoState::Clean)
    }

    // ---------------------------------------------------------------------
    // Loading
    // ---------------------------------------------------------------------

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            self.refresh_pending = true;
            return;
        }
        self.refreshing = true;
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let snap = repo.snapshot()?;
                    let unpushed = match &snap.head.upstream {
                        Some(up) if snap.head.ahead > 0 => repo.unpushed(up),
                        _ => Vec::new(),
                    };
                    anyhow::Ok((snap, unpushed))
                })
                .await;
            this.update(cx, |this, cx| {
                this.refreshing = false;
                match result {
                    Ok((snap, unpushed)) => this.apply_snapshot(snap, unpushed, cx),
                    Err(e) => this.toast(ToastKind::Error, "Could not read repository", Some(e.to_string()), cx),
                }
                if this.refresh_pending {
                    this.refresh_pending = false;
                    this.refresh(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn apply_snapshot(&mut self, snap: Snapshot, unpushed: Vec<Oid>, cx: &mut Context<Self>) {
        let first = !self.loaded;
        let refs_changed = first
            || snap.head != self.snapshot.head
            || snap.branches != self.snapshot.branches
            || snap.remote_branches != self.snapshot.remote_branches
            || snap.tags != self.snapshot.tags;
        let stashes_changed = first || snap.stashes != self.snapshot.stashes;
        let state_changed = snap.state != self.snapshot.state;
        self.snapshot = snap;
        self.refs_by_oid = git::refs_by_commit(&self.snapshot);
        self.unpushed = unpushed.into_iter().collect();
        self.loaded = true;

        if first {
            // Land on history when the working copy is clean, like Tower does.
            if self.snapshot.status.is_clean() {
                self.nav = Nav::History;
            }
        }
        if refs_changed {
            self.reload_history(cx);
            if matches!(self.nav, Nav::Reflog) || self.reflog.loaded {
                self.load_reflog(cx);
            }
        }
        if stashes_changed {
            self.validate_stash_selection(cx);
        }
        if state_changed || first {
            self.prefill_merge_message(cx);
        }
        self.sync_github_repo(cx);
        self.validate_wc_selection(cx);
        cx.emit(RepoEvent::TitleChanged);
        cx.notify();
    }

    fn start_watcher(&mut self, cx: &mut Context<Self>) {
        use notify::Watcher;
        let (tx, rx) = smol::channel::unbounded::<()>();
        let git_dir = self.repo.git_dir.clone();
        let workdir = self.repo.workdir.clone();
        // Ignored directories (target/, node_modules/, ...) churn during builds;
        // their events never change what we show, so skip them.
        let ignored: Arc<std::sync::RwLock<Vec<std::path::PathBuf>>> = Arc::default();
        {
            let ignored = ignored.clone();
            let repo = self.repo.clone();
            cx.background_executor()
                .spawn(async move {
                    let dirs = repo.ignored_directories();
                    if let Ok(mut guard) = ignored.write() {
                        *guard = dirs;
                    }
                })
                .detach();
        }
        let handler = move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                let ignored = ignored.read().map(|g| g.clone()).unwrap_or_default();
                if event.paths.iter().any(|p| !ignored.iter().any(|d| p.starts_with(d)) && is_relevant_change(p, &workdir, &git_dir)) {
                    let _ = tx.try_send(());
                }
            }
        };
        let Ok(mut watcher) = notify::recommended_watcher(handler) else { return };
        if watcher.watch(&self.repo.workdir, notify::RecursiveMode::Recursive).is_err() {
            return;
        }
        if !self.repo.git_dir.starts_with(&self.repo.workdir) {
            let _ = watcher.watch(&self.repo.git_dir, notify::RecursiveMode::Recursive);
        }
        self._watcher = Some(watcher);
        let task = cx.spawn(async move |this, cx| {
            while rx.recv().await.is_ok() {
                // Debounce bursts of file system events.
                cx.background_executor().timer(Duration::from_millis(350)).await;
                while rx.try_recv().is_ok() {}
                let keep_going = this
                    .update(cx, |this, cx| {
                        if !this.is_busy() {
                            this.refresh(cx);
                        }
                    })
                    .is_ok();
                if !keep_going {
                    break;
                }
            }
        });
        self._tasks.push(task);
    }

    fn start_auto_fetch(&mut self, cx: &mut Context<Self>) {
        let task = cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(60)).await;
            let alive = this
                .update(cx, |this, cx| {
                    let minutes = cx.global::<Settings>().auto_fetch_minutes;
                    if minutes == 0 || this.is_busy() || this.snapshot.remotes.is_empty() {
                        return;
                    }
                    let due = this.last_fetch.map(|t| t.elapsed() >= Duration::from_secs(minutes as u64 * 60)).unwrap_or(true);
                    if due {
                        this.background_fetch(cx);
                    }
                })
                .is_ok();
            if !alive {
                break;
            }
        });
        self._tasks.push(task);
    }

    /// Quiet fetch used by the auto-fetch timer: no busy indicator, errors are ignored.
    fn background_fetch(&mut self, cx: &mut Context<Self>) {
        self.last_fetch = Some(std::time::Instant::now());
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let _ = cx.background_executor().spawn(async move { repo.fetch(None, |_| {}) }).await;
            this.update(cx, |this, cx| this.refresh(cx)).ok();
        })
        .detach();
    }

    // ---------------------------------------------------------------------
    // Running operations
    // ---------------------------------------------------------------------

    /// Runs a git operation in the background, then refreshes. `on_success`
    /// receives the command output.
    pub fn run_op<T: Send + 'static>(
        &mut self,
        label: impl Into<String>,
        op: impl FnOnce(&Repo) -> Result<T> + Send + 'static,
        on_done: impl FnOnce(&mut Self, Result<T>, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            self.toast(ToastKind::Info, "Please wait for the current operation to finish", None, cx);
            return;
        }
        let label = label.into();
        self.busy = Some(Busy { label: label.clone(), progress: None });
        cx.notify();
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { op(&repo) }).await;
            this.update(cx, |this, cx| {
                this.busy = None;
                on_done(this, result, cx);
                this.refresh(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Runs a network operation that reports progress lines.
    pub fn run_progress_op(
        &mut self,
        label: impl Into<String>,
        success: impl Into<String>,
        op: impl FnOnce(&Repo, &mut dyn FnMut(&str)) -> Result<String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            self.toast(ToastKind::Info, "Please wait for the current operation to finish", None, cx);
            return;
        }
        let label = label.into();
        let success = success.into();
        self.busy = Some(Busy { label: label.clone(), progress: None });
        cx.notify();
        let repo = self.repo.clone();
        let (tx, rx) = smol::channel::unbounded::<String>();
        cx.spawn(async move |this, cx| {
            let task = cx.background_executor().spawn(async move {
                let mut report = |line: &str| {
                    let _ = tx.try_send(line.to_string());
                };
                let head = |repo: &Repo| repo.run(&["rev-parse", "--verify", "-q", "HEAD"]).ok().map(|s| s.trim().to_string());
                let before = head(&repo);
                let result = op(&repo, &mut report);
                let after = head(&repo);
                result.map(|out| (out, before, after))
            });
            while let Ok(mut line) = rx.recv().await {
                while let Ok(newer) = rx.try_recv() {
                    line = newer;
                }
                let alive = this
                    .update(cx, |this, cx| {
                        if let Some(b) = this.busy.as_mut() {
                            b.progress = Some(line);
                        }
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
            let result = task.await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok((_, Some(from), Some(to))) if from != to => {
                        this.push_undo(success.clone(), undo::UndoAction::MoveHead { from, to, soft: false });
                        this.toast_undoable(success, cx);
                    }
                    Ok(_) => this.toast(ToastKind::Success, success, None, cx),
                    Err(e) => this.show_error(&label, &e, cx),
                }
                this.refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    /// The common case: run, toast on success, show an error on failure.
    pub fn simple_op(&mut self, label: impl Into<String>, success: impl Into<String>, op: impl FnOnce(&Repo) -> Result<String> + Send + 'static, cx: &mut Context<Self>) {
        let label = label.into();
        let success = success.into();
        let label2 = label.clone();
        self.run_op(
            label,
            op,
            move |this, result, cx| match result {
                Ok(_) => this.toast(ToastKind::Success, success, None, cx),
                Err(e) => this.show_error(&label2, &e, cx),
            },
            cx,
        );
    }

    pub fn show_error(&mut self, label: &str, err: &anyhow::Error, cx: &mut Context<Self>) {
        let message = match err.downcast_ref::<git::GitError>() {
            Some(g) => format!("{}\n\n$ {}", g.message, g.command),
            None => err.to_string(),
        };
        let title = if message.contains("CONFLICT") || message.contains("conflict") {
            format!("{label}: conflicts need to be resolved")
        } else {
            format!("{label} failed")
        };
        self.toast(ToastKind::Error, title, Some(message), cx);
    }

    pub fn toast(&mut self, kind: ToastKind, title: impl Into<SharedString>, detail: Option<String>, cx: &mut Context<Self>) {
        self.push_toast(kind, title.into(), detail, false, cx);
    }

    /// Shows a toast; ones that offer Undo or report errors stay up longer.
    pub fn push_toast(&mut self, kind: ToastKind, title: SharedString, detail: Option<String>, undoable: bool, cx: &mut Context<Self>) {
        let id = self.next_toast_id;
        self.next_toast_id += 1;
        self.toasts.push(Toast { id, kind, title, detail, undoable });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
        let secs = match (kind, undoable) {
            (ToastKind::Error, _) | (_, true) => 9,
            _ => 4,
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(secs)).await;
            this.update(cx, |this, cx| {
                this.toasts.retain(|t| t.id != id);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Wraps a view method as a menu handler.
    pub fn handler(&self, cx: &Context<Self>, f: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static) -> Handler {
        let weak = cx.weak_entity();
        Rc::new(move |window, cx| {
            weak.update(cx, |this, cx| f(this, window, cx)).ok();
        })
    }

    pub fn open_menu(&mut self, menu: ContextMenu, cx: &mut Context<Self>) {
        if !menu.is_empty() {
            self.menu = Some(menu);
            cx.notify();
        }
    }

    pub fn copy_to_clipboard(&mut self, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        let shown: String = text.chars().take(48).collect();
        self.toast(ToastKind::Info, format!("Copied “{shown}”"), None, cx);
    }

    pub fn set_nav(&mut self, nav: Nav, window: &mut Window, cx: &mut Context<Self>) {
        if self.nav == nav {
            return;
        }
        self.nav = nav;
        match &self.nav {
            Nav::Reflog if !self.reflog.loaded => self.load_reflog(cx),
            Nav::Stashes => self.validate_stash_selection(cx),
            Nav::History => window.focus(&self.history.list_focus),
            Nav::WorkingCopy => window.focus(&self.wc.list_focus),
            Nav::PullRequests => {
                window.focus(&self.prs.list_focus);
                self.on_show_pull_requests(cx);
            }
            _ => {}
        }
        cx.notify();
    }

    pub fn show_history_scope(&mut self, scope: LogScope, window: &mut Window, cx: &mut Context<Self>) {
        self.nav = Nav::History;
        if self.history.scope != scope {
            self.history.scope = scope;
            self.reload_history(cx);
        }
        window.focus(&self.history.list_focus);
        cx.notify();
    }

    fn persist_layout(&self, cx: &mut Context<Self>) {
        let (w, h, d, wc) = (f32::from(self.sidebar_width), self.history_split, self.details_split, self.wc_split);
        cx.update_global::<Settings, _>(|s, _| {
            s.sidebar_width = w;
            s.history_split = h;
            s.details_split = d;
            s.working_copy_split = wc;
            s.save();
        });
    }

    // ---------------------------------------------------------------------
    // Common operations used by the toolbar, menus and palette
    // ---------------------------------------------------------------------

    pub fn fetch(&mut self, remote: Option<String>, cx: &mut Context<Self>) {
        self.last_fetch = Some(std::time::Instant::now());
        let label = match &remote {
            Some(r) => format!("Fetching {r}"),
            None => "Fetching".to_string(),
        };
        self.run_progress_op(label, "Fetch complete", move |repo, p| repo.fetch(remote.as_deref(), p), cx);
    }

    pub fn pull(&mut self, cx: &mut Context<Self>) {
        if self.snapshot.head.upstream.is_none() {
            self.toast(ToastKind::Info, "The current branch has no upstream to pull from", Some("Push it first, or set an upstream from the branch's context menu.".into()), cx);
            return;
        }
        let rebase = cx.global::<Settings>().pull_rebase;
        self.run_progress_op("Pulling", "Pull complete", move |repo, p| repo.pull(rebase, p), cx);
    }

    pub fn push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(branch) = self.snapshot.head.branch.clone() else {
            self.toast(ToastKind::Info, "Check out a branch to push", None, cx);
            return;
        };
        match self.snapshot.head.upstream.clone() {
            Some(upstream) => {
                let (remote, remote_branch) = upstream.split_once('/').map(|(r, b)| (r.to_string(), b.to_string())).unwrap_or(("origin".into(), branch.clone()));
                self.run_progress_op(format!("Pushing {branch}"), format!("Pushed {branch} to {remote}"), move |repo, p| repo.push_branch_to(&remote, &branch, &remote_branch, false, false, p), cx);
            }
            None => self.open_push_dialog(branch, window, cx),
        }
    }

    pub fn checkout_branch(&mut self, name: String, cx: &mut Context<Self>) {
        let n = name.clone();
        let previous = self.previous_checkout();
        self.run_op(
            format!("Checking out {name}"),
            move |repo| repo.checkout(&n),
            move |this, r, cx| match r {
                Ok(_) => {
                    if let Some(p) = previous {
                        this.push_undo(format!("Check out {name}"), undo::UndoAction::Checkout { previous: p });
                    }
                    this.toast_undoable(format!("Switched to {name}"), cx);
                }
                Err(e) => this.show_error(&format!("Checking out {name}"), &e, cx),
            },
            cx,
        );
    }

    /// What to go back to when undoing a checkout.
    pub fn previous_checkout(&self) -> Option<String> {
        self.snapshot.head.branch.clone().or_else(|| self.snapshot.head.oid.clone())
    }

    pub fn checkout_remote_branch(&mut self, full: String, cx: &mut Context<Self>) {
        let local = full.split_once('/').map(|(_, b)| b.to_string()).unwrap_or(full.clone());
        let l = local.clone();
        let previous = self.previous_checkout();
        let created = !self.snapshot.branches.iter().any(|b| b.name == local);
        self.run_op(
            format!("Checking out {full}"),
            move |repo| repo.checkout_remote_branch(&full, &l),
            move |this, r, cx| match r {
                Ok(_) => {
                    if let Some(p) = previous {
                        if created {
                            this.push_undo(format!("Create {local}"), undo::UndoAction::CreateBranch { name: local.clone() });
                        }
                        this.push_undo(format!("Check out {local}"), undo::UndoAction::Checkout { previous: p });
                    }
                    this.toast_undoable(format!("Switched to {local}"), cx);
                }
                Err(e) => this.show_error("Checkout", &e, cx),
            },
            cx,
        );
    }

    pub fn merge_into_current(&mut self, what: String, no_ff: bool, squash: bool, cx: &mut Context<Self>) {
        let current = self.snapshot.head.display_name();
        let w = what.clone();
        self.head_op(format!("Merging {what}"), format!("Merged {what} into {current}"), false, move |repo| repo.merge(&w, no_ff, squash), cx);
    }

    pub fn rebase_current_onto(&mut self, onto: String, cx: &mut Context<Self>) {
        let current = self.snapshot.head.display_name();
        let o = onto.clone();
        self.head_op(format!("Rebasing onto {onto}"), format!("Rebased {current} onto {onto}"), false, move |repo| repo.rebase(&o), cx);
    }

    pub fn pop_latest_stash(&mut self, cx: &mut Context<Self>) {
        let Some(stash) = self.snapshot.stashes.first().cloned() else {
            self.toast(ToastKind::Info, "There are no stashes to apply", None, cx);
            return;
        };
        self.simple_op("Applying stash", "Stash applied and removed", move |repo| repo.stash_apply(&stash.name, true), cx);
    }

    pub fn abort_operation(&mut self, cx: &mut Context<Self>) {
        let state = self.state();
        self.simple_op(format!("Aborting {}", state.label().to_lowercase()), "Aborted", move |repo| repo.abort(state), cx);
    }

    pub fn continue_operation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state();
        if self.snapshot.status.has_conflicts() {
            self.toast(ToastKind::Error, "Resolve all conflicts before continuing", None, cx);
            return;
        }
        if state == RepoState::Merging {
            self.commit(window, cx);
            return;
        }
        self.simple_op("Continuing", "Continued", move |repo| repo.continue_op(state), cx);
    }

    pub fn open_terminal(&mut self, cx: &mut Context<Self>) {
        let dir = self.repo.workdir.clone();
        if let Err(e) = open_terminal_at(&dir) {
            self.toast(ToastKind::Error, "Could not open a terminal", Some(e.to_string()), cx);
        }
    }

    pub fn reveal_in_files(&mut self, cx: &mut Context<Self>) {
        cx.open_with_system(&self.repo.workdir);
    }

    pub fn open_file(&mut self, path: &str, cx: &mut Context<Self>) {
        cx.open_with_system(&self.repo.workdir.join(path));
    }

    pub fn reveal_file(&mut self, path: &str, cx: &mut Context<Self>) {
        cx.reveal_path(&self.repo.workdir.join(path));
    }

    // ---------------------------------------------------------------------
    // Actions
    // ---------------------------------------------------------------------

    fn on_cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
            return;
        }
        if self.modal.take().is_some() {
            window.focus(&self.focus_handle);
            cx.notify();
            return;
        }
        if !self.history.search.read(cx).text().is_empty() && self.nav == Nav::History {
            self.history.search.update(cx, |s, cx| s.clear(cx));
            window.focus(&self.history.list_focus);
            return;
        }
        cx.propagate();
    }

    fn register_actions(&self, el: gpui::Div, cx: &mut Context<Self>) -> gpui::Div {
        el.on_action(cx.listener(|this, _: &Refresh, _, cx| {
            this.refresh(cx);
            if this.nav == Nav::PullRequests {
                this.refresh_pull_requests(cx);
            }
            this.toast(ToastKind::Info, "Refreshed", None, cx);
        }))
        .on_action(cx.listener(|this, _: &Fetch, _, cx| this.fetch(None, cx)))
        .on_action(cx.listener(|this, _: &Pull, _, cx| this.pull(cx)))
        .on_action(cx.listener(|this, _: &Push, window, cx| this.push(window, cx)))
        .on_action(cx.listener(|this, _: &NewBranch, window, cx| this.open_create_branch(None, window, cx)))
        .on_action(cx.listener(|this, _: &MergeBranch, window, cx| this.open_palette(palette::PaletteMode::Merge, window, cx)))
        .on_action(cx.listener(|this, _: &RebaseBranch, window, cx| this.open_palette(palette::PaletteMode::Rebase, window, cx)))
        .on_action(cx.listener(|this, _: &StashChanges, window, cx| this.open_stash_dialog(window, cx)))
        .on_action(cx.listener(|this, _: &PopStash, _, cx| this.pop_latest_stash(cx)))
        .on_action(cx.listener(|this, _: &QuickActions, window, cx| this.open_palette(palette::PaletteMode::All, window, cx)))
        .on_action(cx.listener(|this, _: &ShowWorkingCopy, window, cx| this.set_nav(Nav::WorkingCopy, window, cx)))
        .on_action(cx.listener(|this, _: &ShowHistory, window, cx| this.set_nav(Nav::History, window, cx)))
        .on_action(cx.listener(|this, _: &ShowStashes, window, cx| this.set_nav(Nav::Stashes, window, cx)))
        .on_action(cx.listener(|this, _: &ShowReflog, window, cx| this.set_nav(Nav::Reflog, window, cx)))
        .on_action(cx.listener(|this, _: &ShowPullRequests, window, cx| this.set_nav(Nav::PullRequests, window, cx)))
        .on_action(cx.listener(|this, _: &CreatePullRequest, window, cx| this.open_create_pull_request(window, cx)))
        .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
            this.set_nav(Nav::History, window, cx);
            let search = this.history.search.clone();
            search.update(cx, |s, cx| s.select_all_text(cx));
            window.focus(&search.focus_handle(cx));
        }))
        .on_action(cx.listener(|this, _: &Commit, window, cx| {
            if this.nav == Nav::WorkingCopy {
                this.commit(window, cx)
            }
        }))
        .on_action(cx.listener(|this, _: &Undo, _, cx| this.undo(cx)))
        .on_action(cx.listener(|this, _: &OpenTerminal, _, cx| this.open_terminal(cx)))
        .on_action(cx.listener(|this, _: &RevealInFiles, _, cx| this.reveal_in_files(cx)))
        .on_action(cx.listener(Self::on_cancel))
    }

    // ---------------------------------------------------------------------
    // Rendering
    // ---------------------------------------------------------------------

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let head = &self.snapshot.head;
        let mut left = div().flex().items_center().gap(px(10.)).min_w_0();
        left = left.child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(icon(if head.detached { Icon::Commit } else { Icon::Branch }).text_color(theme.text_muted))
                .child(div().text_color(theme.text).child(head.display_name())),
        );
        if let Some(up) = &head.upstream {
            left = left.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_color(theme.text_faint)
                    .child(icon(Icon::Cloud))
                    .child(up.clone())
                    .when(head.ahead > 0, |d| d.child(div().text_color(theme.info).child(format!("↑{}", head.ahead))))
                    .when(head.behind > 0, |d| d.child(div().text_color(theme.warning).child(format!("↓{}", head.behind)))),
            );
        }
        let state = self.state();
        if state != RepoState::Clean {
            left = left.child(ui::badge(state.label(), theme.warning.opacity(0.2), theme.warning));
        }
        let mut right = div().flex().items_center().gap(px(8.)).min_w_0().overflow_hidden();
        if let Some(busy) = &self.busy {
            right = right
                .child(spinner(cx))
                .child(div().text_color(theme.text).whitespace_nowrap().child(busy.label.clone()))
                .when_some(busy.progress.clone(), |d, p| d.child(div().text_color(theme.text_faint).whitespace_nowrap().overflow_hidden().text_ellipsis().child(p)));
        } else {
            let changes = self.snapshot.status.change_count();
            right = right.child(div().text_color(theme.text_faint).child(if changes == 0 { "Working copy clean".to_string() } else { format!("{changes} changed file{}", if changes == 1 { "" } else { "s" }) }));
        }
        div()
            .flex_none()
            .h(px(26.))
            .px(px(10.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .bg(theme.toolbar_bg)
            .border_t_1()
            .border_color(theme.border_subtle)
            .text_size(px(12.))
            .child(left)
            .child(right)
    }

    fn render_toasts(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mut col = div().absolute().bottom(px(36.)).right(px(16.)).flex().flex_col().gap(px(8.)).w(px(380.));
        for toast in &self.toasts {
            let (accent, icon_kind) = match toast.kind {
                ToastKind::Success => (theme.success, Icon::Check),
                ToastKind::Error => (theme.danger, Icon::Warning),
                ToastKind::Info => (theme.info, Icon::Info),
            };
            let id = toast.id;
            let detail = toast.detail.clone();
            let title = toast.title.clone();
            col = col.child(
                div()
                    .id(("toast", id))
                    .occlude()
                    .flex()
                    .gap(px(10.))
                    .p(px(12.))
                    .rounded(px(8.))
                    .bg(theme.elevated)
                    .border_1()
                    .border_color(theme.border)
                    .shadow_lg()
                    .child(div().pt(px(1.)).child(icon(icon_kind).text_color(accent)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(div().text_size(px(13.)).font_weight(gpui::FontWeight::MEDIUM).text_color(theme.text).child(title.clone()))
                            .when_some(detail.clone(), |d, det| {
                                let short: String = det.lines().take(4).collect::<Vec<_>>().join("\n");
                                d.child(div().text_size(px(12.)).text_color(theme.text_muted).max_h(px(72.)).overflow_hidden().child(short))
                            })
                            .when(toast.undoable, |d| {
                                d.child(
                                    div()
                                        .id(("toast-undo", id))
                                        .text_size(px(12.))
                                        .text_color(theme.accent_hover)
                                        .cursor_pointer()
                                        .child(if cfg!(target_os = "macos") { "Undo (⌘Z)" } else { "Undo (Ctrl+Z)" })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.toasts.retain(|t| t.id != id);
                                            this.undo(cx);
                                        })),
                                )
                            })
                            .when(toast.kind == ToastKind::Error && detail.is_some(), |d| {
                                let det = detail.clone().unwrap_or_default();
                                let title = title.clone();
                                d.child(
                                    div()
                                        .id(("toast-details", id))
                                        .text_size(px(12.))
                                        .text_color(theme.accent_hover)
                                        .cursor_pointer()
                                        .child("Show details")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.toasts.retain(|t| t.id != id);
                                            this.open_error(title.to_string(), det.clone(), window, cx);
                                        })),
                                )
                            }),
                    )
                    .child(
                        div()
                            .id(("toast-close", id))
                            .cursor_pointer()
                            .text_color(theme.text_faint)
                            .hover(|s| s.text_color(theme.text))
                            .child(icon(Icon::Close))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toasts.retain(|t| t.id != id);
                                cx.notify();
                            })),
                    ),
            );
        }
        col
    }

    fn render_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        if !self.loaded {
            return div().size_full().flex().items_center().justify_center().child(spinner(cx)).into_any_element();
        }
        match self.nav.clone() {
            Nav::WorkingCopy => self.render_working_copy(window, cx).into_any_element(),
            Nav::History => self.render_history(window, cx).into_any_element(),
            Nav::Stashes => self.render_stashes(window, cx).into_any_element(),
            Nav::Reflog => self.render_reflog(window, cx).into_any_element(),
            Nav::PullRequests => self.render_pull_requests(window, cx).into_any_element(),
            Nav::Blame(_) => self.render_blame(window, cx).into_any_element(),
        }
    }
}

impl Focusable for RepoView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RepoView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let toolbar = self.render_toolbar(window, cx);
        let sidebar = self.render_sidebar(window, cx);
        let content = self.render_content(window, cx);
        let status = self.render_status_bar(cx);
        let toasts = self.render_toasts(cx);
        let menu = self.menu.as_ref().map(|m| {
            render_menu(m, cx, |this: &mut RepoView, _| {
                this.menu = None;
            })
        });
        let modal = self.render_modal(window, cx);

        let root = div()
            .key_context("RepoView")
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme.app_bg)
            .text_color(theme.text)
            .on_drag_move(cx.listener(|this, e: &DragMoveEvent<SplitDrag>, _, cx| {
                if e.drag(cx).0 == 0 {
                    let w = (e.event.position.x - e.bounds.left()).clamp(px(160.), px(600.));
                    this.sidebar_width = w;
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if cx.has_active_drag() {
                        this.persist_layout(cx);
                    }
                }),
            );
        let root = self.register_actions(root, cx);
        root.child(toolbar)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .child(div().flex_none().w(self.sidebar_width).h_full().child(sidebar))
                    .child(ui::splitter_handle("sidebar-split", true, cx).on_drag(SplitDrag(0), |_, _, _, cx| cx.new(|_| ui::DragGhost)))
                    .child(div().flex_1().min_w_0().h_full().child(content)),
            )
            .child(status)
            .child(toasts)
            .children(menu)
            .children(modal)
    }
}

pub fn spinner(cx: &App) -> impl IntoElement {
    use gpui::{Animation, AnimationExt, Transformation};
    let theme = cx.theme();
    gpui::svg().path(Icon::Refresh.path()).size(px(14.)).flex_none().text_color(theme.accent_hover).with_animation("spinner", Animation::new(Duration::from_millis(900)).repeat(), |svg, delta| {
        svg.with_transformation(Transformation::rotate(gpui::percentage(delta)))
    })
}

fn is_relevant_change(path: &Path, workdir: &Path, git_dir: &Path) -> bool {
    if let Ok(rel) = path.strip_prefix(git_dir) {
        let s = rel.to_string_lossy().replace('\\', "/");
        if s.ends_with(".lock") {
            return false;
        }
        return s == "HEAD"
            || s == "index"
            || s == "packed-refs"
            || s == "MERGE_HEAD"
            || s == "CHERRY_PICK_HEAD"
            || s == "REVERT_HEAD"
            || s == "FETCH_HEAD"
            || s.starts_with("refs/")
            || s.starts_with("rebase-merge")
            || s.starts_with("rebase-apply")
            || s == "logs/refs/stash";
    }
    if let Ok(rel) = path.strip_prefix(workdir) {
        let first = rel.components().next().map(|c| c.as_os_str().to_string_lossy().to_string()).unwrap_or_default();
        return first != ".git";
    }
    true
}

pub fn open_terminal_at(dir: &Path) -> Result<()> {
    use std::process::Command;
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // Prefer Windows Terminal, fall back to a plain console.
        if Command::new("wt.exe").arg("-d").arg(dir).spawn().is_ok() {
            return Ok(());
        }
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        Command::new("cmd.exe").arg("/K").current_dir(dir).creation_flags(CREATE_NEW_CONSOLE).spawn()?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").args(["-a", "Terminal"]).arg(dir).spawn()?;
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for term in ["x-terminal-emulator", "gnome-terminal", "konsole", "xfce4-terminal", "alacritty", "kitty", "xterm"] {
            if Command::new(term).current_dir(dir).spawn().is_ok() {
                return Ok(());
            }
        }
        anyhow::bail!("No terminal emulator found")
    }
}

