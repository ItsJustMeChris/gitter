//! Modal dialogs.

use super::palette::PaletteState;
use super::{RepoView, ToastKind};
use crate::actions::Confirm;
use crate::assets::Icon;
use crate::git::{short_oid, Repo, ResetMode};
use crate::theme::{mono_font, ActiveTheme};
use crate::ui::text_input::{InputEvent, TextInput};
use crate::ui::{icon, Button, Checkbox};
use gpui::{anchored, deferred, div, point, prelude::*, px, AnyElement, Context, Entity, FocusHandle, Focusable, MouseButton, SharedString, Subscription, Window};
use std::rc::Rc;

pub type ConfirmFn = Rc<dyn Fn(&mut RepoView, &mut Window, &mut Context<RepoView>)>;

pub struct ResetTarget {
    pub oid: String,
}

pub enum ModalKind {
    Confirm { title: SharedString, message: SharedString, confirm_label: SharedString, danger: bool, on_confirm: ConfirmFn },
    Error { title: SharedString, message: String },
    CreateBranch { name: Entity<TextInput>, start: Option<String>, checkout: bool },
    RenameBranch { old: String, name: Entity<TextInput> },
    CreateTag { target: String, name: Entity<TextInput>, message: Entity<TextInput>, push: bool },
    Stash { message: Entity<TextInput>, include_untracked: bool, keep_index: bool },
    Push { branch: String, remote: usize, remote_branch: Entity<TextInput>, set_upstream: bool, force: bool, tags: bool },
    Merge { what: String, no_ff: bool, squash: bool },
    Reset { oid: String, mode: ResetMode },
    Reword { oid: String, subject: Entity<TextInput>, body: Entity<TextInput> },
    Remote { old: Option<String>, name: Entity<TextInput>, url: Entity<TextInput> },
    Palette(PaletteState),
}

pub struct Modal {
    pub kind: ModalKind,
    pub focus: FocusHandle,
    pub _subscriptions: Vec<Subscription>,
}

impl RepoView {
    fn new_input(&self, placeholder: &str, text: &str, window: &mut Window, cx: &mut Context<Self>) -> (Entity<TextInput>, Subscription) {
        let placeholder = placeholder.to_string();
        let text = text.to_string();
        let input = cx.new(|cx| TextInput::new(cx).with_placeholder(placeholder).with_text(text));
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| {
            if *ev == InputEvent::Submit {
                this.submit_modal(window, cx);
            }
        });
        (input, sub)
    }

    fn show_modal(&mut self, kind: ModalKind, subs: Vec<Subscription>, focus_input: Option<&Entity<TextInput>>, window: &mut Window, cx: &mut Context<Self>) {
        let focus = cx.focus_handle();
        match focus_input {
            Some(input) => {
                input.update(cx, |i, cx| i.select_all_text(cx));
                window.focus(&input.focus_handle(cx));
            }
            None => window.focus(&focus),
        }
        self.menu = None;
        self.modal = Some(Modal { kind, focus, _subscriptions: subs });
        cx.notify();
    }

    pub fn close_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.modal = None;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    pub fn confirm(
        &mut self,
        title: impl Into<SharedString>,
        message: impl Into<SharedString>,
        confirm_label: impl Into<SharedString>,
        danger: bool,
        on_confirm: impl Fn(&mut RepoView, &mut Window, &mut Context<RepoView>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let kind = ModalKind::Confirm { title: title.into(), message: message.into(), confirm_label: confirm_label.into(), danger, on_confirm: Rc::new(on_confirm) };
        self.show_modal(kind, Vec::new(), None, window, cx);
    }

    pub fn open_error(&mut self, title: String, message: String, window: &mut Window, cx: &mut Context<Self>) {
        self.show_modal(ModalKind::Error { title: title.into(), message }, Vec::new(), None, window, cx);
    }

    pub fn open_create_branch(&mut self, start: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let (name, sub) = self.new_input("feature/my-branch", "", window, cx);
        self.show_modal(ModalKind::CreateBranch { name: name.clone(), start, checkout: true }, vec![sub], Some(&name), window, cx);
    }

    pub fn open_rename_branch(&mut self, old: String, window: &mut Window, cx: &mut Context<Self>) {
        let (name, sub) = self.new_input("New name", &old, window, cx);
        self.show_modal(ModalKind::RenameBranch { old, name: name.clone() }, vec![sub], Some(&name), window, cx);
    }

    pub fn open_create_tag(&mut self, target: String, window: &mut Window, cx: &mut Context<Self>) {
        let (name, s1) = self.new_input("v1.0.0", "", window, cx);
        let message = cx.new(|cx| TextInput::new(cx).multiline().with_placeholder("Optional message (creates an annotated tag)"));
        self.show_modal(ModalKind::CreateTag { target, name: name.clone(), message, push: false }, vec![s1], Some(&name), window, cx);
    }

    pub fn open_stash_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.snapshot.status.is_clean() {
            self.toast(ToastKind::Info, "There are no local changes to stash", None, cx);
            return;
        }
        let (message, sub) = self.new_input("Describe these changes (optional)", "", window, cx);
        let has_untracked = self.snapshot.status.unstaged.iter().any(|e| e.status == crate::git::FileStatus::Untracked);
        self.show_modal(ModalKind::Stash { message: message.clone(), include_untracked: has_untracked, keep_index: false }, vec![sub], Some(&message), window, cx);
    }

    pub fn open_push_dialog(&mut self, branch: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.snapshot.remotes.is_empty() {
            self.toast(ToastKind::Info, "This repository has no remotes", Some("Add a remote from the sidebar first.".into()), cx);
            return;
        }
        let tracking = self.snapshot.branches.iter().find(|b| b.name == branch).and_then(|b| b.upstream.clone());
        let (remote_name, remote_branch) = match &tracking {
            Some(up) => up.split_once('/').map(|(r, b)| (r.to_string(), b.to_string())).unwrap_or(("origin".into(), branch.clone())),
            None => ("origin".into(), branch.clone()),
        };
        let remote = self.snapshot.remotes.iter().position(|r| r.name == remote_name).unwrap_or(0);
        let (rb, sub) = self.new_input("Remote branch name", &remote_branch, window, cx);
        self.show_modal(ModalKind::Push { branch, remote, remote_branch: rb, set_upstream: tracking.is_none(), force: false, tags: false }, vec![sub], None, window, cx);
    }

    pub fn open_merge_dialog(&mut self, what: String, window: &mut Window, cx: &mut Context<Self>) {
        self.show_modal(ModalKind::Merge { what, no_ff: false, squash: false }, Vec::new(), None, window, cx);
    }

    pub fn open_reset(&mut self, target: ResetTarget, window: &mut Window, cx: &mut Context<Self>) {
        self.show_modal(ModalKind::Reset { oid: target.oid, mode: ResetMode::Mixed }, Vec::new(), None, window, cx);
    }

    pub fn open_reword(&mut self, oid: String, window: &mut Window, cx: &mut Context<Self>) {
        let message = self.repo.run(&["log", "-1", "--format=%B", &oid]).unwrap_or_default();
        let mut lines = message.trim_end().lines();
        let subject_text = lines.next().unwrap_or("").to_string();
        let body_text = lines.collect::<Vec<_>>().join("\n").trim().to_string();
        let (subject, sub) = self.new_input("Summary", &subject_text, window, cx);
        let body = cx.new(|cx| TextInput::new(cx).multiline().with_placeholder("Description").with_text(body_text));
        self.show_modal(ModalKind::Reword { oid, subject: subject.clone(), body }, vec![sub], Some(&subject), window, cx);
    }

    pub fn open_add_remote(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let default_name = if self.snapshot.remotes.iter().any(|r| r.name == "origin") { "" } else { "origin" };
        let (name, s1) = self.new_input("origin", default_name, window, cx);
        let (url, s2) = self.new_input("https://github.com/user/repo.git", "", window, cx);
        let focus = if default_name.is_empty() { name.clone() } else { url.clone() };
        self.show_modal(ModalKind::Remote { old: None, name, url }, vec![s1, s2], Some(&focus), window, cx);
    }

    pub fn open_edit_remote(&mut self, old: String, old_url: String, window: &mut Window, cx: &mut Context<Self>) {
        let (name, s1) = self.new_input("Name", &old, window, cx);
        let (url, s2) = self.new_input("URL", &old_url, window, cx);
        self.show_modal(ModalKind::Remote { old: Some(old), name, url: url.clone() }, vec![s1, s2], Some(&url), window, cx);
    }

    pub fn open_palette(&mut self, mode: super::palette::PaletteMode, window: &mut Window, cx: &mut Context<Self>) {
        let placeholder = match mode {
            super::palette::PaletteMode::All => "Search branches and commands…",
            super::palette::PaletteMode::Merge => "Choose a branch to merge into the current branch…",
            super::palette::PaletteMode::Rebase => "Choose a branch to rebase onto…",
            super::palette::PaletteMode::Checkout => "Choose a branch to check out…",
        };
        let (query, sub) = self.new_input(placeholder, "", window, cx);
        let sub2 = cx.subscribe(&query, |this, _, ev: &InputEvent, cx| {
            if *ev == InputEvent::Changed {
                this.update_palette(cx);
            }
        });
        let state = PaletteState::new(mode, query.clone());
        self.show_modal(ModalKind::Palette(state), vec![sub, sub2], Some(&query), window, cx);
        self.update_palette(cx);
    }

    pub fn confirm_delete_branch(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let n = name.clone();
        self.confirm(
            "Delete branch?",
            format!("The local branch “{name}” will be deleted. If it has unmerged commits, they'll only be reachable through the reflog."),
            "Delete Branch",
            true,
            move |this, _, cx| {
                let n = n.clone();
                let oid = this.snapshot.branches.iter().find(|b| b.name == n).map(|b| b.oid.clone());
                let label = format!("Deleting {n}");
                this.run_op(
                    label,
                    {
                        let n = n.clone();
                        move |repo| repo.delete_branch(&n, false).or_else(|e| if e.to_string().contains("not fully merged") { repo.delete_branch(&n, true) } else { Err(e) })
                    },
                    move |this, r, cx| match r {
                        Ok(_) => {
                            if let Some(oid) = oid.clone() {
                                this.push_undo(format!("Delete {n}"), super::undo::UndoAction::DeleteBranch { name: n.clone(), oid });
                            }
                            this.toast_undoable(format!("Deleted branch {n}"), cx);
                        }
                        Err(e) => this.show_error("Delete branch", &e, cx),
                    },
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub fn confirm_checkout_detached(&mut self, rev: String, window: &mut Window, cx: &mut Context<Self>) {
        let short = if rev.len() == 40 { short_oid(&rev).to_string() } else { rev.clone() };
        self.confirm(
            "Check out in detached HEAD?",
            format!("You'll be on {short} without a branch. New commits won't belong to any branch unless you create one."),
            "Check Out",
            false,
            move |this, _, cx| {
                let r = rev.clone();
                this.simple_op("Checking out", format!("Checked out {short}"), move |repo| repo.checkout_detached(&r), cx);
            },
            window,
            cx,
        );
    }

    /// Primary action of the open dialog (Enter / primary button).
    pub fn submit_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.modal.take() else { return };
        let keep_open = |this: &mut Self, modal: Modal| {
            this.modal = Some(modal);
        };
        match &modal.kind {
            ModalKind::Confirm { on_confirm, .. } => {
                let f = on_confirm.clone();
                self.close_modal(window, cx);
                f(self, window, cx);
            }
            ModalKind::Error { .. } => self.close_modal(window, cx),
            ModalKind::CreateBranch { name, start, checkout } => {
                let n = name.read(cx).text().trim().to_string();
                if !Repo::is_valid_ref_name(&n) {
                    self.toast(ToastKind::Error, "That's not a valid branch name", None, cx);
                    keep_open(self, modal);
                    return;
                }
                let (start, checkout) = (start.clone(), *checkout);
                self.close_modal(window, cx);
                let n2 = n.clone();
                let previous = self.previous_checkout();
                self.run_op(
                    format!("Creating {n}"),
                    move |repo| repo.create_branch(&n2, start.as_deref(), checkout),
                    move |this, r, cx| match r {
                        Ok(_) => {
                            this.push_undo(format!("Create {n}"), super::undo::UndoAction::CreateBranch { name: n.clone() });
                            if checkout {
                                if let Some(p) = previous.clone() {
                                    // Undo switches back first, then deletes the branch.
                                    this.push_undo(format!("Switch to {n}"), super::undo::UndoAction::Checkout { previous: p });
                                }
                            }
                            this.toast_undoable(if checkout { format!("Created and switched to {n}") } else { format!("Created branch {n}") }, cx);
                        }
                        Err(e) => this.show_error("Create branch", &e, cx),
                    },
                    cx,
                );
            }
            ModalKind::RenameBranch { old, name } => {
                let n = name.read(cx).text().trim().to_string();
                if !Repo::is_valid_ref_name(&n) {
                    self.toast(ToastKind::Error, "That's not a valid branch name", None, cx);
                    keep_open(self, modal);
                    return;
                }
                let old = old.clone();
                self.close_modal(window, cx);
                let n2 = n.clone();
                self.simple_op("Renaming branch", format!("Renamed {old} to {n}"), move |repo| repo.rename_branch(&old, &n2), cx);
            }
            ModalKind::CreateTag { target, name, message, push } => {
                let n = name.read(cx).text().trim().to_string();
                if n.is_empty() || n.contains(' ') {
                    self.toast(ToastKind::Error, "Enter a tag name without spaces", None, cx);
                    keep_open(self, modal);
                    return;
                }
                let msg = message.read(cx).text().to_string();
                let (target, push) = (target.clone(), *push);
                let remote = self.snapshot.remotes.first().map(|r| r.name.clone());
                self.close_modal(window, cx);
                let n2 = n.clone();
                self.run_op(
                    format!("Creating tag {n}"),
                    move |repo| {
                        let out = repo.create_tag(&n2, &target, Some(&msg))?;
                        if let (true, Some(r)) = (push, remote) {
                            repo.push_tag(&r, &n2, |_| {})?;
                        }
                        Ok(out)
                    },
                    move |this, r, cx| match r {
                        Ok(_) => {
                            if !push {
                                this.push_undo(format!("Create tag {n}"), super::undo::UndoAction::CreateTag { name: n.clone() });
                            }
                            this.toast_undoable(format!("Created tag {n}"), cx);
                        }
                        Err(e) => this.show_error("Create tag", &e, cx),
                    },
                    cx,
                );
            }
            ModalKind::Stash { message, include_untracked, keep_index } => {
                let msg = message.read(cx).text().to_string();
                let (u, k) = (*include_untracked, *keep_index);
                self.close_modal(window, cx);
                self.run_op("Stashing changes", move |repo| repo.stash_save(Some(&msg), u, k), |this, r, cx| match r {
                    Ok(_) => {
                        this.push_undo("Stash changes", super::undo::UndoAction::StashSave);
                        this.toast_undoable("Changes stashed", cx);
                    }
                    Err(e) => this.show_error("Stash", &e, cx),
                }, cx);
            }
            ModalKind::Push { branch, remote, remote_branch, set_upstream, force, tags } => {
                let Some(remote) = self.snapshot.remotes.get(*remote).map(|r| r.name.clone()) else {
                    keep_open(self, modal);
                    return;
                };
                let rb = remote_branch.read(cx).text().trim().to_string();
                let rb = if rb.is_empty() { branch.clone() } else { rb };
                let (b, up, force, tags) = (branch.clone(), *set_upstream, *force, *tags);
                self.close_modal(window, cx);
                self.run_progress_op(format!("Pushing {b}"), format!("Pushed {b} to {remote}/{rb}"), move |repo, p| {
                    let out = repo.push_branch_to(&remote, &b, &rb, up, force, &mut *p)?;
                    if tags {
                        repo.run_with_progress(&["push", "--progress", "--tags", &remote], &mut *p)?;
                    }
                    Ok(out)
                }, cx);
            }
            ModalKind::Merge { what, no_ff, squash } => {
                let (w, n, s) = (what.clone(), *no_ff, *squash);
                self.close_modal(window, cx);
                self.merge_into_current(w, n, s, cx);
            }
            ModalKind::Reset { oid, mode } => {
                let (o, m) = (oid.clone(), *mode);
                self.close_modal(window, cx);
                let short = short_oid(&o).to_string();
                self.head_op("Resetting", format!("Reset to {short}"), m == ResetMode::Soft, move |repo| repo.reset(&o, m), cx);
            }
            ModalKind::Reword { oid, subject, body } => {
                let s = subject.read(cx).text().trim().to_string();
                let b = body.read(cx).text().trim().to_string();
                if s.is_empty() {
                    keep_open(self, modal);
                    return;
                }
                let msg = if b.is_empty() { s } else { format!("{s}\n\n{b}") };
                let o = oid.clone();
                self.close_modal(window, cx);
                self.head_op("Rewording commit", "Commit message updated", false, move |repo| repo.reword(&o, &msg), cx);
            }
            ModalKind::Remote { old, name, url } => {
                let n = name.read(cx).text().trim().to_string();
                let u = url.read(cx).text().trim().to_string();
                if n.is_empty() || u.is_empty() {
                    self.toast(ToastKind::Error, "Enter both a name and a URL", None, cx);
                    keep_open(self, modal);
                    return;
                }
                let old = old.clone();
                self.close_modal(window, cx);
                match old {
                    None => {
                        let n2 = n.clone();
                        self.run_progress_op(format!("Adding remote {n}"), format!("Added remote {n}"), move |repo, p| {
                            repo.add_remote(&n2, &u)?;
                            repo.fetch(Some(&n2), p)
                        }, cx);
                    }
                    Some(old) => {
                        self.simple_op("Updating remote", format!("Updated remote {n}"), move |repo| {
                            if old != n {
                                repo.rename_remote(&old, &n)?;
                            }
                            repo.set_remote_url(&n, &u)
                        }, cx);
                    }
                }
            }
            ModalKind::Palette(_) => {
                self.modal = Some(modal);
                self.run_palette_selection(window, cx);
            }
        }
    }

    pub fn render_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let modal = self.modal.as_ref()?;
        let theme = cx.theme().clone();
        let (title, width, body, footer): (Option<SharedString>, f32, AnyElement, Option<AnyElement>) = match &modal.kind {
            ModalKind::Palette(_) => (None, 600., self.render_palette(window, cx), None),
            ModalKind::Confirm { title, message, confirm_label, danger, .. } => (
                Some(title.clone()),
                440.,
                div().text_color(theme.text_muted).child(message.clone()).into_any_element(),
                Some(self.footer(confirm_label.clone(), *danger, true, cx)),
            ),
            ModalKind::Error { title, message } => (
                Some(title.clone()),
                640.,
                div()
                    .id("error-text")
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .p(px(10.))
                    .rounded(px(5.))
                    .bg(theme.input_bg)
                    .border_1()
                    .border_color(theme.border)
                    .font_family(mono_font())
                    .text_size(px(12.))
                    .text_color(theme.text)
                    .child(message.clone())
                    .into_any_element(),
                Some(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(Button::new("copy-error", "Copy").on_click({
                            let m = message.clone();
                            cx.listener(move |this, _, _, cx| this.copy_to_clipboard(m.clone(), cx))
                        }))
                        .child(Button::new("close-error", "Close").primary().on_click(cx.listener(|this, _, window, cx| this.close_modal(window, cx))))
                        .into_any_element(),
                ),
            ),
            ModalKind::CreateBranch { name, start, checkout } => {
                let from = start.clone().map(|s| if s.len() == 40 { short_oid(&s).to_string() } else { s }).unwrap_or_else(|| self.snapshot.head.display_name());
                let checkout = *checkout;
                (
                    Some("Create Branch".into()),
                    440.,
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .child(field_label("Branch name", cx))
                        .child(name.clone())
                        .child(div().text_size(px(12.)).text_color(theme.text_faint).child(format!("Based on {from}")))
                        .child(Checkbox::new("checkout-new", checkout).label("Check out after creating").on_toggle(cx.listener(|this, v: &bool, _, cx| {
                            if let Some(Modal { kind: ModalKind::CreateBranch { checkout, .. }, .. }) = this.modal.as_mut() {
                                *checkout = *v;
                            }
                            cx.notify();
                        })))
                        .into_any_element(),
                    Some(self.footer("Create Branch", false, true, cx)),
                )
            }
            ModalKind::RenameBranch { old, name } => (
                Some(format!("Rename “{old}”").into()),
                420.,
                div().flex().flex_col().gap(px(10.)).child(field_label("New name", cx)).child(name.clone()).into_any_element(),
                Some(self.footer("Rename", false, true, cx)),
            ),
            ModalKind::CreateTag { target, name, message, push } => {
                let push = *push;
                let short = if target.len() == 40 { short_oid(target).to_string() } else { target.clone() };
                (
                    Some("Create Tag".into()),
                    460.,
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .child(field_label("Tag name", cx))
                        .child(name.clone())
                        .child(field_label("Message", cx))
                        .child(div().h(px(80.)).child(message.clone()))
                        .child(div().text_size(px(12.)).text_color(theme.text_faint).child(format!("Tags commit {short}")))
                        .when(!self.snapshot.remotes.is_empty(), |d| {
                            d.child(Checkbox::new("push-tag", push).label("Push tag to remote").on_toggle(cx.listener(|this, v: &bool, _, cx| {
                                if let Some(Modal { kind: ModalKind::CreateTag { push, .. }, .. }) = this.modal.as_mut() {
                                    *push = *v;
                                }
                                cx.notify();
                            })))
                        })
                        .into_any_element(),
                    Some(self.footer("Create Tag", false, true, cx)),
                )
            }
            ModalKind::Stash { message, include_untracked, keep_index } => {
                let (u, k) = (*include_untracked, *keep_index);
                (
                    Some("Stash Changes".into()),
                    440.,
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .child(field_label("Message", cx))
                        .child(message.clone())
                        .child(Checkbox::new("stash-untracked", u).label("Include untracked files").on_toggle(cx.listener(|this, v: &bool, _, cx| {
                            if let Some(Modal { kind: ModalKind::Stash { include_untracked, .. }, .. }) = this.modal.as_mut() {
                                *include_untracked = *v;
                            }
                            cx.notify();
                        })))
                        .child(Checkbox::new("stash-keep-index", k).label("Keep staged changes in the working copy").on_toggle(cx.listener(|this, v: &bool, _, cx| {
                            if let Some(Modal { kind: ModalKind::Stash { keep_index, .. }, .. }) = this.modal.as_mut() {
                                *keep_index = *v;
                            }
                            cx.notify();
                        })))
                        .into_any_element(),
                    Some(self.footer("Stash", false, true, cx)),
                )
            }
            ModalKind::Push { branch, remote, remote_branch, set_upstream, force, tags } => {
                let (remote_ix, up, force, tags) = (*remote, *set_upstream, *force, *tags);
                let mut remotes = div().flex().flex_wrap().gap(px(6.));
                for (i, r) in self.snapshot.remotes.iter().enumerate() {
                    remotes = remotes.child(Button::new(("push-remote", i), r.name.clone()).small().selected(i == remote_ix).icon(Icon::Globe).on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(Modal { kind: ModalKind::Push { remote, .. }, .. }) = this.modal.as_mut() {
                            *remote = i;
                        }
                        cx.notify();
                    })));
                }
                (
                    Some(format!("Push “{branch}”").into()),
                    460.,
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .child(field_label("Remote", cx))
                        .child(remotes)
                        .child(field_label("Remote branch", cx))
                        .child(remote_branch.clone())
                        .child(Checkbox::new("push-upstream", up).label("Track this remote branch (set upstream)").on_toggle(cx.listener(|this, v: &bool, _, cx| {
                            if let Some(Modal { kind: ModalKind::Push { set_upstream, .. }, .. }) = this.modal.as_mut() {
                                *set_upstream = *v;
                            }
                            cx.notify();
                        })))
                        .child(Checkbox::new("push-tags", tags).label("Also push all tags").on_toggle(cx.listener(|this, v: &bool, _, cx| {
                            if let Some(Modal { kind: ModalKind::Push { tags, .. }, .. }) = this.modal.as_mut() {
                                *tags = *v;
                            }
                            cx.notify();
                        })))
                        .child(Checkbox::new("push-force", force).label("Force push (with lease)").on_toggle(cx.listener(|this, v: &bool, _, cx| {
                            if let Some(Modal { kind: ModalKind::Push { force, .. }, .. }) = this.modal.as_mut() {
                                *force = *v;
                            }
                            cx.notify();
                        })))
                        .when(force, |d| d.child(warning_note("Force pushing rewrites the remote branch. Commits others have pushed since your last fetch are protected by the lease.", cx)))
                        .into_any_element(),
                    Some(self.footer(if force { "Force Push" } else { "Push" }, force, true, cx)),
                )
            }
            ModalKind::Merge { what, no_ff, squash } => {
                let (n, s) = (*no_ff, *squash);
                let current = self.snapshot.head.display_name();
                (
                    Some("Merge".into()),
                    440.,
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .child(div().child(format!("Merge “{what}” into “{current}”.")))
                        .child(Checkbox::new("merge-no-ff", n).label("Always create a merge commit (--no-ff)").on_toggle(cx.listener(|this, v: &bool, _, cx| {
                            if let Some(Modal { kind: ModalKind::Merge { no_ff, squash, .. }, .. }) = this.modal.as_mut() {
                                *no_ff = *v;
                                if *v {
                                    *squash = false;
                                }
                            }
                            cx.notify();
                        })))
                        .child(Checkbox::new("merge-squash", s).label("Squash all changes into the working copy (--squash)").on_toggle(cx.listener(|this, v: &bool, _, cx| {
                            if let Some(Modal { kind: ModalKind::Merge { no_ff, squash, .. }, .. }) = this.modal.as_mut() {
                                *squash = *v;
                                if *v {
                                    *no_ff = false;
                                }
                            }
                            cx.notify();
                        })))
                        .into_any_element(),
                    Some(self.footer("Merge", false, true, cx)),
                )
            }
            ModalKind::Reset { oid, mode } => {
                let mode = *mode;
                let current = self.snapshot.head.display_name();
                let option = |m: ResetMode, label: &'static str, detail: &'static str, cx: &mut Context<Self>| {
                    let theme = cx.theme().clone();
                    let selected = m == mode;
                    div()
                        .id(label)
                        .p(px(10.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(if selected { theme.accent } else { theme.border })
                        .bg(if selected { theme.accent_subtle } else { theme.surface })
                        .cursor_pointer()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(label))
                        .child(div().text_size(px(12.)).text_color(theme.text_muted).child(detail))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(Modal { kind: ModalKind::Reset { mode, .. }, .. }) = this.modal.as_mut() {
                                *mode = m;
                            }
                            cx.notify();
                        }))
                };
                (
                    Some(format!("Reset “{current}” to {}", short_oid(oid)).into()),
                    480.,
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(option(ResetMode::Soft, "Soft", "Keep all changes staged.", cx))
                        .child(option(ResetMode::Mixed, "Mixed", "Keep changes in the working copy, unstaged.", cx))
                        .child(option(ResetMode::Hard, "Hard", "Discard all changes. Uncommitted work will be lost.", cx))
                        .when(mode == ResetMode::Hard, |d| d.child(warning_note("A hard reset permanently discards uncommitted changes.", cx)))
                        .into_any_element(),
                    Some(self.footer("Reset", mode == ResetMode::Hard, true, cx)),
                )
            }
            ModalKind::Reword { oid, subject, body } => (
                Some(format!("Edit Message of {}", short_oid(oid)).into()),
                520.,
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(field_label("Summary", cx))
                    .child(subject.clone())
                    .child(field_label("Description", cx))
                    .child(div().h(px(140.)).child(body.clone()))
                    .child(warning_note("Commits after this one will be rewritten. Avoid this on branches others have already pulled.", cx))
                    .into_any_element(),
                Some(self.footer("Save Message", false, true, cx)),
            ),
            ModalKind::Remote { old, name, url } => (
                Some(if old.is_some() { "Edit Remote".into() } else { "Add Remote".into() }),
                480.,
                div().flex().flex_col().gap(px(10.)).child(field_label("Name", cx)).child(name.clone()).child(field_label("URL", cx)).child(url.clone()).into_any_element(),
                Some(self.footer(if old.is_some() { "Save" } else { "Add Remote" }, false, true, cx)),
            ),
        };

        let viewport = window.viewport_size();
        let focus = modal.focus.clone();
        let is_palette = matches!(modal.kind, ModalKind::Palette(_));
        let panel = div()
            .id("modal-panel")
            .key_context("Modal")
            .track_focus(&focus)
            .on_action(cx.listener(|this, _: &Confirm, window, cx| this.submit_modal(window, cx)))
            .occlude()
            .w(px(width))
            .max_w(viewport.width - px(40.))
            .rounded(px(10.))
            .bg(theme.elevated)
            .border_1()
            .border_color(theme.border)
            .shadow_lg()
            .flex()
            .flex_col()
            .text_size(px(13.))
            .when_some(title, |d, t| d.child(div().px(px(18.)).pt(px(16.)).pb(px(4.)).text_size(px(15.)).font_weight(gpui::FontWeight::SEMIBOLD).child(t)))
            .child(div().flex().flex_col().when(!is_palette, |d| d.px(px(18.)).py(px(12.))).child(body))
            .when_some(footer, |d, f| d.child(div().flex().flex_col().px(px(18.)).pb(px(16.)).pt(px(4.)).child(f)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());

        Some(
            deferred(
                anchored().position(point(px(0.), px(0.))).child(
                    div()
                        .id("modal-backdrop")
                        .w(viewport.width)
                        .h(viewport.height)
                        .bg(theme.overlay)
                        .flex()
                        .justify_center()
                        .items_start()
                        .pt(px(if is_palette { 90. } else { 120. }))
                        .occlude()
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_modal(window, cx)))
                        .child(panel),
                ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    fn footer(&self, label: impl Into<SharedString>, danger: bool, cancel: bool, cx: &mut Context<Self>) -> AnyElement {
        let mut primary = Button::new("modal-primary", label).on_click(cx.listener(|this, _, window, cx| this.submit_modal(window, cx)));
        primary = if danger { primary.danger() } else { primary.primary() };
        div()
            .flex()
            .justify_end()
            .gap(px(8.))
            .when(cancel, |d| d.child(Button::new("modal-cancel", "Cancel").on_click(cx.listener(|this, _, window, cx| this.close_modal(window, cx)))))
            .child(primary)
            .into_any_element()
    }
}

pub fn field_label(text: &'static str, cx: &gpui::App) -> impl IntoElement {
    div().text_size(px(12.)).font_weight(gpui::FontWeight::MEDIUM).text_color(cx.theme().text_muted).child(text)
}

pub fn warning_note(text: &'static str, cx: &gpui::App) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .flex()
        .gap(px(8.))
        .p(px(8.))
        .rounded(px(5.))
        .bg(theme.warning.opacity(0.12))
        .border_1()
        .border_color(theme.warning.opacity(0.35))
        .text_size(px(12.))
        .text_color(theme.text)
        .child(icon(Icon::Warning).text_color(theme.warning))
        .child(div().flex_1().child(text))
}
