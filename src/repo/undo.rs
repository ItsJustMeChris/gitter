//! Undo for the most common operations, Tower style (Ctrl/Cmd+Z).
//!
//! Each successful operation that knows how to reverse itself pushes an
//! entry. Undo re-validates the repository state before acting so that a
//! stale entry can't clobber newer work.

use super::{RepoView, ToastKind};
use crate::git::{Oid, Repo};
use anyhow::{bail, Result};
use gpui::Context;

#[derive(Clone, Debug)]
pub enum UndoAction {
    /// HEAD moved from `from` to `to` (commit, merge, rebase, reset, pull…).
    /// Undo resets back with `--keep`, which preserves uncommitted changes.
    MoveHead { from: Oid, to: Oid, soft: bool },
    /// A commit (or amend); undo also restores the message to the composer.
    Commit { from: Oid, to: Oid, message: String },
    CreateBranch { name: String },
    DeleteBranch { name: String, oid: Oid },
    Checkout { previous: String },
    CreateTag { name: String },
    DeleteTag { name: String, oid: Oid },
    Stage { paths: Vec<String> },
    Unstage { paths: Vec<String> },
    StashSave,
}

#[derive(Clone, Debug)]
pub struct UndoEntry {
    pub label: String,
    pub action: UndoAction,
}

const MAX_UNDO: usize = 30;

fn head(repo: &Repo) -> Option<Oid> {
    repo.run(&["rev-parse", "--verify", "-q", "HEAD"]).ok().map(|s| s.trim().to_string())
}

fn apply(repo: &Repo, action: &UndoAction) -> Result<String> {
    match action {
        UndoAction::MoveHead { from, to, soft } => {
            if head(repo).as_deref() != Some(to.as_str()) {
                bail!("The branch has moved since then, so this can no longer be undone safely");
            }
            repo.run(&["reset", if *soft { "--soft" } else { "--keep" }, from])
        }
        UndoAction::Commit { from, to, .. } => apply(repo, &UndoAction::MoveHead { from: from.clone(), to: to.clone(), soft: true }),
        UndoAction::CreateBranch { name } => repo.delete_branch(name, true),
        UndoAction::DeleteBranch { name, oid } => repo.run(&["branch", name, oid]),
        UndoAction::Checkout { previous } => repo.checkout(previous),
        UndoAction::CreateTag { name } => repo.delete_tag(name),
        UndoAction::DeleteTag { name, oid } => repo.run(&["tag", name, oid]),
        UndoAction::Stage { paths } => repo.unstage(paths).map(|_| String::new()),
        UndoAction::Unstage { paths } => repo.stage(paths).map(|_| String::new()),
        UndoAction::StashSave => repo.stash_apply("stash@{0}", true),
    }
}

impl RepoView {
    pub fn push_undo(&mut self, label: impl Into<String>, action: UndoAction) {
        self.undo_stack.push(UndoEntry { label: label.into(), action });
        if self.undo_stack.len() > MAX_UNDO {
            self.undo_stack.remove(0);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.undo_stack.pop() else {
            self.toast(ToastKind::Info, "Nothing to undo", None, cx);
            return;
        };
        let label = entry.label.clone();
        let restore = match &entry.action {
            UndoAction::Commit { message, .. } => Some(message.clone()),
            _ => None,
        };
        self.run_op(
            format!("Undoing {}", label.to_lowercase()),
            move |repo| apply(repo, &entry.action),
            move |this, result, cx| match result {
                Ok(_) => {
                    if let Some(message) = restore {
                        let mut lines = message.lines();
                        let subject = lines.next().unwrap_or("").to_string();
                        let body = lines.collect::<Vec<_>>().join("\n").trim().to_string();
                        this.wc.subject.update(cx, |s, cx| s.set_text(subject, cx));
                        this.wc.body.update(cx, |s, cx| s.set_text(body, cx));
                    }
                    this.toast(ToastKind::Success, format!("Undid: {label}"), None, cx)
                }
                Err(e) => this.show_error("Undo", &e, cx),
            },
            cx,
        );
    }

    /// Runs an operation that moves HEAD and makes it undoable.
    pub fn head_op(&mut self, label: impl Into<String>, success: impl Into<String>, soft_undo: bool, op: impl FnOnce(&Repo) -> Result<String> + Send + 'static, cx: &mut Context<Self>) {
        let label = label.into();
        let success = success.into();
        let label2 = label.clone();
        self.run_op(
            label,
            move |repo| {
                let before = head(repo);
                let result = op(repo);
                let after = head(repo);
                result.map(|out| (out, before, after))
            },
            move |this, result, cx| match result {
                Ok((_, before, after)) => {
                    if let (Some(from), Some(to)) = (before, after) {
                        if from != to {
                            this.push_undo(success.clone(), UndoAction::MoveHead { from, to, soft: soft_undo });
                        }
                    }
                    this.toast_undoable(success.clone(), cx);
                }
                Err(e) => {
                    this.show_error(&label2, &e, cx);
                }
            },
            cx,
        );
    }

    /// A success toast that offers to undo the operation that was just recorded.
    pub fn toast_undoable(&mut self, title: impl Into<gpui::SharedString>, cx: &mut Context<Self>) {
        let undoable = self.can_undo();
        self.push_toast(ToastKind::Success, title.into(), None, undoable, cx);
    }
}
