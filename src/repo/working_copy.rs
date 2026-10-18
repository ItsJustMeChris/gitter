//! Working copy: staging area, conflict resolution and the commit composer.

use super::diff_view::{DiffLocation, DiffMode, DiffState};
use super::{RepoView, ToastKind};
use crate::actions::{DiscardSelected, SelectNext, SelectPrev, ToggleStage};
use crate::assets::Icon;
use crate::git::{FileStatus, RepoState, StatusEntry};
use crate::settings::Settings;
use crate::theme::ActiveTheme;
use crate::ui::text_input::TextInput;
use crate::ui::{self, empty_state, icon, Button, Checkbox, SplitDrag};
use gpui::{
    Focusable,
    div, prelude::*, px, relative, uniform_list, AnyElement, ClickEvent, Context, DragMoveEvent, Entity, FocusHandle, MouseButton, ScrollStrategy, UniformListScrollHandle,
    Window,
};
use std::ops::Range;

pub struct WorkingCopyState {
    /// (staged, path)
    pub selected: Option<(bool, String)>,
    pub diff: Option<DiffState>,
    pub diff_generation: u64,
    pub subject: Entity<TextInput>,
    pub body: Entity<TextInput>,
    pub amend: bool,
    pub unstaged_scroll: UniformListScrollHandle,
    pub staged_scroll: UniformListScrollHandle,
    pub list_focus: FocusHandle,
    pub committing: bool,
}

impl WorkingCopyState {
    pub fn new(subject: Entity<TextInput>, body: Entity<TextInput>, cx: &mut Context<RepoView>) -> Self {
        Self {
            selected: None,
            diff: None,
            diff_generation: 0,
            subject,
            body,
            amend: false,
            unstaged_scroll: UniformListScrollHandle::new(),
            staged_scroll: UniformListScrollHandle::new(),
            list_focus: cx.focus_handle(),
            committing: false,
        }
    }
}

impl RepoView {
    fn wc_list(&self, staged: bool) -> &[StatusEntry] {
        if staged {
            &self.snapshot.status.staged
        } else {
            &self.snapshot.status.unstaged
        }
    }

    pub fn selected_wc_entry(&self) -> Option<(bool, StatusEntry)> {
        let (staged, path) = self.wc.selected.as_ref()?;
        self.wc_list(*staged).iter().find(|e| &e.path == path).map(|e| (*staged, e.clone()))
    }

    pub fn validate_wc_selection(&mut self, cx: &mut Context<Self>) {
        let still_there = self.selected_wc_entry().is_some();
        if !still_there {
            let next = match self.wc.selected.clone() {
                Some((staged, path)) if self.wc_list(!staged).iter().any(|e| e.path == path) => Some((!staged, path)),
                _ => self.snapshot.status.unstaged.first().map(|e| (false, e.path.clone())).or_else(|| self.snapshot.status.staged.first().map(|e| (true, e.path.clone()))),
            };
            self.wc.selected = next;
        }
        self.load_wc_diff(cx);
    }

    pub fn select_wc_file(&mut self, staged: bool, path: String, cx: &mut Context<Self>) {
        let changed = self.wc.selected.as_ref() != Some(&(staged, path.clone()));
        self.wc.selected = Some((staged, path.clone()));
        if let Some(ix) = self.wc_list(staged).iter().position(|e| e.path == path) {
            let handle = if staged { &self.wc.staged_scroll } else { &self.wc.unstaged_scroll };
            handle.scroll_to_item(ix, ScrollStrategy::Top);
        }
        if changed {
            self.wc.diff = None;
            self.load_wc_diff(cx);
        }
        cx.notify();
    }

    pub fn load_wc_diff(&mut self, cx: &mut Context<Self>) {
        let Some((staged, entry)) = self.selected_wc_entry() else {
            self.wc.diff = None;
            cx.notify();
            return;
        };
        self.wc.diff_generation += 1;
        let generation = self.wc.diff_generation;
        let repo = self.repo.clone();
        let context = cx.global::<Settings>().diff_context;
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { repo.working_diff(&entry, staged, context) }).await;
            this.update(cx, |this, cx| {
                if this.wc.diff_generation != generation {
                    return;
                }
                match result {
                    Ok(diff) => match this.wc.diff.as_mut() {
                        Some(state) => state.update(diff),
                        None => this.wc.diff = Some(DiffState::new(diff)),
                    },
                    Err(_) => this.wc.diff = None,
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Flattened (staged, path) list used for keyboard navigation.
    fn wc_nav_list(&self) -> Vec<(bool, String)> {
        self.snapshot.status.unstaged.iter().map(|e| (false, e.path.clone())).chain(self.snapshot.status.staged.iter().map(|e| (true, e.path.clone()))).collect()
    }

    fn wc_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        let list = self.wc_nav_list();
        if list.is_empty() {
            return;
        }
        let cur = self.wc.selected.as_ref().and_then(|s| list.iter().position(|x| x == s)).map(|i| i as isize).unwrap_or(-1);
        let next = (cur + delta).clamp(0, list.len() as isize - 1) as usize;
        let (staged, path) = list[next].clone();
        self.select_wc_file(staged, path, cx);
    }

    pub fn stage_paths(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        let n = paths.len();
        let undo_paths = paths.clone();
        self.run_op(
            if n == 1 { format!("Staging {}", paths[0]) } else { format!("Staging {n} files") },
            move |repo| repo.stage(&paths),
            move |this, r, cx| match r {
                Ok(_) => this.push_undo(if n == 1 { format!("Stage {}", undo_paths[0]) } else { format!("Stage {n} files") }, super::undo::UndoAction::Stage { paths: undo_paths }),
                Err(e) => this.show_error("Stage", &e, cx),
            },
            cx,
        );
    }

    pub fn unstage_paths(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        let n = paths.len();
        let undo_paths = paths.clone();
        self.run_op(
            if n == 1 { format!("Unstaging {}", paths[0]) } else { format!("Unstaging {n} files") },
            move |repo| repo.unstage(&paths),
            move |this, r, cx| match r {
                Ok(_) => this.push_undo(if n == 1 { format!("Unstage {}", undo_paths[0]) } else { format!("Unstage {n} files") }, super::undo::UndoAction::Unstage { paths: undo_paths }),
                Err(e) => this.show_error("Unstage", &e, cx),
            },
            cx,
        );
    }

    pub fn stage_all(&mut self, cx: &mut Context<Self>) {
        self.run_op(
            "Staging all changes",
            |repo| repo.stage_all(),
            |this, r, cx| {
                if let Err(e) = r {
                    this.show_error("Stage all", &e, cx);
                }
            },
            cx,
        );
    }

    pub fn unstage_all(&mut self, cx: &mut Context<Self>) {
        self.run_op(
            "Unstaging all changes",
            |repo| repo.unstage_all(),
            |this, r, cx| {
                if let Err(e) = r {
                    this.show_error("Unstage all", &e, cx);
                }
            },
            cx,
        );
    }

    /// Stages or unstages the selected file, then moves the selection to the next file in the same list.
    pub fn toggle_stage_selected(&mut self, cx: &mut Context<Self>) {
        let Some((staged, entry)) = self.selected_wc_entry() else { return };
        let list = self.wc_list(staged);
        let ix = list.iter().position(|e| e.path == entry.path).unwrap_or(0);
        let next = list.get(ix + 1).or_else(|| if ix > 0 { list.get(ix - 1) } else { None }).map(|e| (staged, e.path.clone()));
        self.wc.selected = Some(next.unwrap_or((!staged, entry.path.clone())));
        if staged {
            self.unstage_paths(vec![entry.path], cx);
        } else {
            self.stage_paths(vec![entry.path], cx);
        }
    }

    pub fn confirm_discard(&mut self, entries: Vec<StatusEntry>, window: &mut Window, cx: &mut Context<Self>) {
        if entries.is_empty() {
            return;
        }
        let untracked = entries.iter().filter(|e| e.status == FileStatus::Untracked).count();
        let what = if entries.len() == 1 { format!("“{}”", entries[0].path) } else { format!("{} files", entries.len()) };
        let detail = if untracked > 0 {
            format!("Changes to {what} will be discarded. Untracked files will be deleted. This cannot be undone.")
        } else {
            format!("Changes to {what} will be discarded. This cannot be undone.")
        };
        self.confirm(
            "Discard changes?",
            detail,
            "Discard",
            true,
            move |this, _, cx| {
                let entries = entries.clone();
                this.run_op(
                    "Discarding changes",
                    move |repo| repo.discard(&entries),
                    |this, r, cx| match r {
                        Ok(_) => this.toast(ToastKind::Success, "Changes discarded", None, cx),
                        Err(e) => this.show_error("Discard", &e, cx),
                    },
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub fn discard_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entries: Vec<StatusEntry> = self.snapshot.status.unstaged.iter().filter(|e| e.status != FileStatus::Conflicted).cloned().collect();
        self.confirm_discard(entries, window, cx);
    }

    pub fn resolve_conflict(&mut self, path: String, ours: bool, cx: &mut Context<Self>) {
        let side = if ours { "our" } else { "their" };
        let p = path.clone();
        self.simple_op(format!("Resolving {path}"), format!("Resolved {path} using {side} version"), move |repo| repo.resolve_with(&p, ours).map(|_| String::new()), cx);
    }

    pub fn mark_resolved(&mut self, path: String, cx: &mut Context<Self>) {
        let p = path.clone();
        self.simple_op(format!("Marking {path} resolved"), format!("Marked {path} as resolved"), move |repo| repo.mark_resolved(&[p]).map(|_| String::new()), cx);
    }

    pub fn ignore_path(&mut self, pattern: String, cx: &mut Context<Self>) {
        let p = pattern.clone();
        self.simple_op("Updating .gitignore", format!("Added {pattern} to .gitignore"), move |repo| repo.add_to_gitignore(&p).map(|_| String::new()), cx);
    }

    pub fn prefill_merge_message(&mut self, cx: &mut Context<Self>) {
        if self.state() == RepoState::Clean || !self.wc.subject.read(cx).text().is_empty() {
            return;
        }
        if let Some(msg) = self.repo.pending_merge_message() {
            let mut lines = msg.lines();
            let subject = lines.next().unwrap_or("").to_string();
            let body = lines.collect::<Vec<_>>().join("\n").trim().to_string();
            self.wc.subject.update(cx, |s, cx| s.set_text(subject, cx));
            self.wc.body.update(cx, |s, cx| s.set_text(body, cx));
        }
    }

    pub fn set_amend(&mut self, amend: bool, cx: &mut Context<Self>) {
        self.wc.amend = amend;
        if amend && self.wc.subject.read(cx).text().trim().is_empty() {
            if let Some(msg) = self.repo.head_message() {
                let mut lines = msg.lines();
                let subject = lines.next().unwrap_or("").to_string();
                let body = lines.collect::<Vec<_>>().join("\n").trim().to_string();
                self.wc.subject.update(cx, |s, cx| s.set_text(subject, cx));
                self.wc.body.update(cx, |s, cx| s.set_text(body, cx));
            }
        }
        cx.notify();
    }

    pub fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let subject = self.wc.subject.read(cx).text().trim().to_string();
        let body = self.wc.body.read(cx).text().trim().to_string();
        let amend = self.wc.amend;
        let merging = matches!(self.state(), RepoState::Merging | RepoState::CherryPicking | RepoState::Reverting);
        if subject.is_empty() {
            self.toast(ToastKind::Info, "Enter a commit summary first", None, cx);
            window.focus(&self.wc.subject.focus_handle(cx));
            return;
        }
        if self.snapshot.status.has_conflicts() {
            self.toast(ToastKind::Error, "Resolve all conflicts before committing", None, cx);
            return;
        }
        if self.snapshot.status.staged.is_empty() && !amend && !merging {
            self.toast(ToastKind::Info, "Nothing staged", Some("Stage some changes, or enable “Amend” to only change the message.".into()), cx);
            return;
        }
        let message = if body.is_empty() { subject.clone() } else { format!("{subject}\n\n{body}") };
        let message_for_undo = message.clone();
        let branch = self.snapshot.head.display_name();
        self.wc.committing = true;
        let state = self.state();
        self.run_op(
            if amend { "Amending commit" } else { "Committing" },
            move |repo| {
                let head = |repo: &crate::git::Repo| repo.run(&["rev-parse", "--verify", "-q", "HEAD"]).ok().map(|s| s.trim().to_string());
                let before = head(repo);
                if state == RepoState::CherryPicking || state == RepoState::Reverting {
                    // Let the sequencer finish with our message.
                    repo.commit(&message, false, false)?;
                    let _ = repo.continue_op(state);
                } else {
                    repo.commit(&message, amend, amend)?;
                }
                Ok((before, head(repo)))
            },
            move |this, r, cx| {
                this.wc.committing = false;
                match r {
                    Ok((before, after)) => {
                        this.wc.subject.update(cx, |s, cx| s.clear(cx));
                        this.wc.body.update(cx, |s, cx| s.clear(cx));
                        this.wc.amend = false;
                        if let (Some(from), Some(to)) = (before, after) {
                            this.push_undo(if amend { "Amend commit" } else { "Commit" }, super::undo::UndoAction::Commit { from, to, message: message_for_undo.clone() });
                        }
                        let title = if amend { format!("Amended last commit on {branch}") } else { format!("Committed to {branch}") };
                        this.push_toast(ToastKind::Success, title.into(), Some(subject.clone()), true, cx);
                    }
                    Err(e) => this.show_error("Commit", &e, cx),
                }
            },
            cx,
        );
    }

    // ---------------------------------------------------------------------
    // Rendering
    // ---------------------------------------------------------------------

    pub fn render_working_copy(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let split = self.wc_split;
        let unstaged_n = self.snapshot.status.unstaged.len();
        let staged_n = self.snapshot.status.staged.len();
        let focused = self.wc.list_focus.is_focused(window);
        let has_conflicts = self.snapshot.status.has_conflicts();

        let unstaged_header = self.list_header(
            "Unstaged Files",
            unstaged_n,
            div()
                .flex()
                .gap(px(4.))
                .child(Button::icon_only("discard-all", Icon::Undo).small().tooltip("Discard all changes").disabled(unstaged_n == 0).on_click(cx.listener(|this, _, window, cx| this.discard_all(window, cx))))
                .child(Button::new("stage-all", "Stage All").small().disabled(unstaged_n == 0).on_click(cx.listener(|this, _, _, cx| this.stage_all(cx))))
                .into_any_element(),
            cx,
        );
        let staged_header = self.list_header(
            "Staged Files",
            staged_n,
            Button::new("unstage-all", "Unstage All").small().disabled(staged_n == 0).on_click(cx.listener(|this, _, _, cx| this.unstage_all(cx))).into_any_element(),
            cx,
        );

        let unstaged_list: AnyElement = if unstaged_n == 0 {
            div().flex_1().flex().items_center().justify_center().text_size(px(12.)).text_color(theme.text_faint).child("No unstaged changes").into_any_element()
        } else {
            uniform_list("unstaged-list", unstaged_n, cx.processor(move |this: &mut RepoView, range: Range<usize>, _window, cx| range.map(|ix| this.render_wc_row(false, ix, focused, cx)).collect::<Vec<_>>()))
                .track_scroll(self.wc.unstaged_scroll.clone())
                .flex_1()
                .into_any_element()
        };
        let staged_list: AnyElement = if staged_n == 0 {
            div().flex_1().flex().items_center().justify_center().text_size(px(12.)).text_color(theme.text_faint).child("Stage files to include them in the next commit").into_any_element()
        } else {
            uniform_list("staged-list", staged_n, cx.processor(move |this: &mut RepoView, range: Range<usize>, _window, cx| range.map(|ix| this.render_wc_row(true, ix, focused, cx)).collect::<Vec<_>>()))
                .track_scroll(self.wc.staged_scroll.clone())
                .flex_1()
                .into_any_element()
        };

        let lists = div()
            .id("wc-lists")
            .key_context("List FileList")
            .track_focus(&self.wc.list_focus)
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.wc_move(-1, cx)))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.wc_move(1, cx)))
            .on_action(cx.listener(|this, _: &ToggleStage, _, cx| this.toggle_stage_selected(cx)))
            .on_action(cx.listener(|this, _: &DiscardSelected, window, cx| {
                if let Some((false, e)) = this.selected_wc_entry() {
                    this.confirm_discard(vec![e], window, cx);
                }
            }))
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .text_size(px(13.))
            .child(unstaged_header)
            .child(div().flex_1().min_h(px(60.)).flex().flex_col().child(unstaged_list))
            .child(staged_header)
            .child(div().flex_1().min_h(px(60.)).flex().flex_col().child(staged_list));

        let left = div()
            .h_full()
            .w(relative(split))
            .min_w(px(280.))
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(theme.border_subtle)
            .children(self.render_state_banner(cx))
            .child(lists)
            .child(self.render_commit_composer(window, cx));

        let mode = match &self.wc.selected {
            Some((true, _)) => DiffMode::Staged,
            _ => DiffMode::Unstaged,
        };
        let right: AnyElement = if self.snapshot.status.is_clean() {
            empty_state(Icon::Check, "Working copy is clean", "There are no local changes. Edit files in your project and they'll show up here.", cx)
        } else if has_conflicts && self.selected_wc_entry().map(|(_, e)| e.status == FileStatus::Conflicted).unwrap_or(false) {
            self.render_conflict_panel(cx)
        } else {
            let toolbar = self.wc_diff_toolbar(cx);
            self.render_diff(DiffLocation::WorkingCopy, mode, toolbar, cx)
        };

        div()
            .id("wc-split")
            .size_full()
            .flex()
            .on_drag_move(cx.listener(|this, e: &DragMoveEvent<SplitDrag>, _, cx| {
                if e.drag(cx).0 == 3 {
                    let w = f32::from(e.bounds.size.width).max(1.);
                    this.wc_split = (f32::from(e.event.position.x - e.bounds.left()) / w).clamp(0.18, 0.8);
                    cx.notify();
                }
            }))
            .child(left)
            .child(ui::splitter_handle("wc-vsplit", true, cx).on_drag(SplitDrag(3), |_, _, _, cx| cx.new(|_| ui::DragGhost)))
            .child(div().flex_1().min_w_0().h_full().child(right))
    }

    fn list_header(&self, title: &'static str, count: usize, actions: AnyElement, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        div()
            .flex_none()
            .h(px(34.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .bg(theme.surface.opacity(0.5))
            .border_b_1()
            .border_t_1()
            .border_color(theme.border_subtle)
            .child(div().text_size(px(12.)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(theme.text_muted).child(title))
            .child(ui::badge(count.to_string(), theme.active, theme.text_muted))
            .child(div().flex_1())
            .child(actions)
    }

    fn render_wc_row(&self, staged: bool, ix: usize, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(entry) = self.wc_list(staged).get(ix).cloned() else { return div().into_any_element() };
        let selected = self.wc.selected.as_ref().map(|(s, p)| *s == staged && p == &entry.path).unwrap_or(false);
        let (name, dir) = ui::split_path(&entry.path);
        let bg = if selected {
            if focused {
                theme.selection
            } else {
                theme.selection_inactive
            }
        } else {
            gpui::transparent_black()
        };
        let path = entry.path.clone();
        let path2 = entry.path.clone();
        let path3 = entry.path.clone();
        let entry2 = entry.clone();
        let group = gpui::SharedString::from(format!("wc-row-{staged}-{ix}"));
        let conflict_label = entry.conflict.map(|c| c.label());
        div()
            .id((if staged { "staged-row" } else { "unstaged-row" }, ix))
            .group(group.clone())
            .w_full()
            .h(px(26.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .bg(bg)
            .when(!selected, |d| d.hover(|s| s.bg(theme.hover)))
            .child(div().id(("status", ix)).child(ui::status_badge(entry.status, cx)).tooltip(ui::tooltip(entry.status.label())))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_baseline()
                    .gap(px(6.))
                    .overflow_hidden()
                    .child(div().whitespace_nowrap().text_color(theme.text).child(name.to_string()))
                    .when_some(entry.old_path.clone(), |d, old| d.child(div().whitespace_nowrap().text_size(px(11.)).text_color(theme.status_renamed).child(format!("← {old}"))))
                    .child(div().whitespace_nowrap().overflow_hidden().text_ellipsis().text_size(px(11.)).text_color(theme.text_faint).child(dir.to_string())),
            )
            .when_some(conflict_label, |d, l| d.child(div().text_size(px(11.)).text_color(theme.status_conflict).whitespace_nowrap().child(l)))
            .child(
                div()
                    .invisible()
                    .group_hover(group, |s| s.visible())
                    .when(selected, |d| d.visible())
                    .child(Button::icon_only(("toggle-stage", ix), if staged { Icon::Minus } else { Icon::Plus }).small().tooltip(if staged { "Unstage" } else { "Stage" }).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if staged {
                            this.unstage_paths(vec![path.clone()], cx);
                        } else {
                            this.stage_paths(vec![path.clone()], cx);
                        }
                    }))),
            )
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                window.focus(&this.wc.list_focus);
                if e.click_count() >= 2 {
                    if staged {
                        this.unstage_paths(vec![path2.clone()], cx);
                    } else {
                        this.stage_paths(vec![path2.clone()], cx);
                    }
                } else {
                    this.select_wc_file(staged, path2.clone(), cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                    window.focus(&this.wc.list_focus);
                    this.select_wc_file(staged, path3.clone(), cx);
                    let menu = this.wc_file_menu(staged, &entry2, e.position, cx);
                    this.open_menu(menu, cx);
                }),
            )
            .into_any_element()
    }

    fn wc_diff_toolbar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (staged, entry) = self.selected_wc_entry()?;
        let path = entry.path.clone();
        let path2 = entry.path.clone();
        let mut bar = div().flex().items_center().gap(px(4.));
        if staged {
            bar = bar.child(Button::new("unstage-file", "Unstage File").small().icon(Icon::Minus).on_click(cx.listener(move |this, _, _, cx| this.unstage_paths(vec![path.clone()], cx))));
        } else {
            let e = entry.clone();
            bar = bar
                .child(Button::new("stage-file", "Stage File").small().primary().icon(Icon::Plus).on_click(cx.listener(move |this, _, _, cx| this.stage_paths(vec![path.clone()], cx))))
                .child(Button::icon_only("discard-file", Icon::Undo).small().tooltip("Discard changes").on_click(cx.listener(move |this, _, window, cx| this.confirm_discard(vec![e.clone()], window, cx))));
        }
        bar = bar.child(Button::icon_only("open-file", Icon::Reveal).small().tooltip("Open file").on_click(cx.listener(move |this, _, _, cx| this.open_file(&path2, cx))));
        Some(bar.into_any_element())
    }

    fn render_state_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = cx.theme().clone();
        let state = self.state();
        if state == RepoState::Clean {
            return None;
        }
        let conflicts = self.snapshot.status.conflicted().count();
        let detail = if conflicts > 0 {
            format!("{conflicts} conflicted file{} — resolve them, then continue.", if conflicts == 1 { "" } else { "s" })
        } else {
            "All conflicts resolved. Continue to finish.".to_string()
        };
        Some(
            div()
                .flex_none()
                .p(px(10.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .bg(theme.warning.opacity(0.12))
                .border_b_1()
                .border_color(theme.warning.opacity(0.4))
                .child(div().flex().items_center().gap(px(8.)).child(icon(Icon::Warning).text_color(theme.warning)).child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(state.label())))
                .child(div().text_size(px(12.)).text_color(theme.text_muted).child(detail))
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .when(state == RepoState::Rebasing, |d| d.child(Button::new("skip-rebase", "Skip Commit").small().on_click(cx.listener(|this, _, _, cx| this.simple_op("Skipping commit", "Skipped", |repo| repo.skip_rebase_commit(), cx)))))
                        .child(Button::new("abort-op", "Abort").small().danger().on_click(cx.listener(|this, _, _, cx| this.abort_operation(cx))))
                        .child(Button::new("continue-op", "Continue").small().primary().disabled(conflicts > 0).on_click(cx.listener(|this, _, window, cx| this.continue_operation(window, cx)))),
                )
                .into_any_element(),
        )
    }

    fn render_conflict_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some((_, entry)) = self.selected_wc_entry() else { return div().into_any_element() };
        let path = entry.path.clone();
        let (p1, p2, p3, p4) = (path.clone(), path.clone(), path.clone(), path.clone());
        let kind = entry.conflict.map(|c| c.label()).unwrap_or("Conflict");
        let diff = self.render_diff(DiffLocation::WorkingCopy, DiffMode::ReadOnly, None, cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .p(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .border_b_1()
                    .border_color(theme.border_subtle)
                    .bg(theme.status_conflict.opacity(0.08))
                    .child(div().flex().items_center().gap(px(8.)).child(icon(Icon::Warning).text_color(theme.status_conflict)).child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(format!("{path} — {kind}"))))
                    .child(div().text_size(px(12.)).text_color(theme.text_muted).child("Choose a version to keep, or edit the file to resolve the conflict markers yourself and mark it as resolved."))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(6.))
                            .child(Button::new("use-ours", "Use My Version").small().on_click(cx.listener(move |this, _, _, cx| this.resolve_conflict(p1.clone(), true, cx))))
                            .child(Button::new("use-theirs", "Use Their Version").small().on_click(cx.listener(move |this, _, _, cx| this.resolve_conflict(p2.clone(), false, cx))))
                            .child(Button::new("open-conflict", "Open in Editor").small().icon(Icon::Reveal).on_click(cx.listener(move |this, _, _, cx| this.open_file(&p3, cx))))
                            .child(Button::new("mark-resolved", "Mark as Resolved").small().primary().icon(Icon::Check).on_click(cx.listener(move |this, _, _, cx| this.mark_resolved(p4.clone(), cx)))),
                    ),
            )
            .child(div().flex_1().min_h_0().child(diff))
            .into_any_element()
    }

    fn render_commit_composer(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let staged_n = self.snapshot.status.staged.len();
        let subject_len = self.wc.subject.read(cx).text().chars().count();
        let branch = self.snapshot.head.display_name();
        let merging = matches!(self.state(), RepoState::Merging | RepoState::CherryPicking | RepoState::Reverting);
        let can_commit = subject_len > 0 && (staged_n > 0 || self.wc.amend || merging) && !self.is_busy() && !self.snapshot.status.has_conflicts();
        let label = if self.wc.amend {
            "Amend Commit".to_string()
        } else if staged_n > 0 {
            format!("Commit {staged_n} file{}", if staged_n == 1 { "" } else { "s" })
        } else {
            "Commit".to_string()
        };
        let author = match (&self.snapshot.user_name, &self.snapshot.user_email) {
            (Some(n), Some(e)) => format!("{n} <{e}>"),
            (Some(n), None) => n.clone(),
            _ => "No author configured — set it in Settings".into(),
        };
        let _ = window;
        div()
            .flex_none()
            .p(px(10.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.surface.opacity(0.4))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_size(px(12.)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(theme.text_muted).child(format!("Commit to {branch}")))
                    .child(div().text_size(px(11.)).text_color(if subject_len > 72 { theme.danger } else if subject_len > 50 { theme.warning } else { theme.text_faint }).child(format!("{subject_len}"))),
            )
            .child(self.wc.subject.clone())
            .child(div().h(px(84.)).child(self.wc.body.clone()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(Checkbox::new("amend", self.wc.amend).label("Amend").on_toggle(cx.listener(|this, v: &bool, _, cx| this.set_amend(*v, cx))))
                    .child(div().flex_1().min_w_0().text_size(px(11.)).text_color(theme.text_faint).whitespace_nowrap().overflow_hidden().text_ellipsis().child(author))
                    .child(Button::new("commit", label).primary().disabled(!can_commit).tooltip("Commit (Ctrl/⌘+Enter)").on_click(cx.listener(|this, _, window, cx| this.commit(window, cx)))),
            )
    }
}
