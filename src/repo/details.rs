//! Commit details: metadata, changed files and the diff of the selected file.

use super::diff_view::{DiffLocation, DiffMode, DiffState};
use super::{RepoView, ToastKind};
use crate::actions::{SelectNext, SelectPrev};
use crate::assets::Icon;
use crate::git::{short_oid, ChangedFile, CommitDetails, Oid, Stash};
use crate::theme::{mono_font, ActiveTheme};
use crate::ui::{self, empty_state, SplitDrag};
use gpui::{
    div, prelude::*, px, relative, uniform_list, AnyElement, Context, DragMoveEvent, FocusHandle, MouseButton, ScrollHandle, ScrollStrategy, UniformListScrollHandle, Window,
};
use std::ops::Range;

pub struct DetailState {
    pub oid: Option<Oid>,
    pub stash: Option<Stash>,
    pub details: Option<CommitDetails>,
    pub loading: bool,
    pub selected_file: Option<usize>,
    pub diff: Option<DiffState>,
    pub files_scroll: UniformListScrollHandle,
    pub info_scroll: ScrollHandle,
    pub generation: u64,
    pub diff_generation: u64,
    pub files_focus: FocusHandle,
}

impl DetailState {
    pub fn new(cx: &mut Context<RepoView>) -> Self {
        Self {
            oid: None,
            stash: None,
            details: None,
            loading: false,
            selected_file: None,
            diff: None,
            files_scroll: UniformListScrollHandle::new(),
            info_scroll: ScrollHandle::new(),
            generation: 0,
            diff_generation: 0,
            files_focus: cx.focus_handle(),
        }
    }

    pub fn clear(&mut self) {
        self.oid = None;
        self.stash = None;
        self.details = None;
        self.selected_file = None;
        self.diff = None;
        self.generation += 1;
    }
}

impl RepoView {
    pub fn detail_state(&self, loc: DiffLocation) -> &DetailState {
        match loc {
            DiffLocation::History | DiffLocation::WorkingCopy => &self.history.detail,
            DiffLocation::Stash => &self.stash_view.detail,
            DiffLocation::Reflog => &self.reflog.detail,
        }
    }

    pub fn detail_state_mut(&mut self, loc: DiffLocation) -> &mut DetailState {
        match loc {
            DiffLocation::History | DiffLocation::WorkingCopy => &mut self.history.detail,
            DiffLocation::Stash => &mut self.stash_view.detail,
            DiffLocation::Reflog => &mut self.reflog.detail,
        }
    }

    pub fn load_commit_detail(&mut self, loc: DiffLocation, oid: Oid, cx: &mut Context<Self>) {
        self.load_detail(loc, oid, None, cx);
    }

    pub fn load_stash_detail(&mut self, stash: Stash, cx: &mut Context<Self>) {
        let oid = stash.oid.clone();
        self.load_detail(DiffLocation::Stash, oid, Some(stash), cx);
    }

    fn load_detail(&mut self, loc: DiffLocation, oid: Oid, stash: Option<Stash>, cx: &mut Context<Self>) {
        let state = self.detail_state_mut(loc);
        let previous_path = state.selected_file.and_then(|i| state.details.as_ref().and_then(|d| d.files.get(i)).map(|f| f.path.clone()));
        state.generation += 1;
        let generation = state.generation;
        state.oid = Some(oid.clone());
        state.stash = stash.clone();
        state.loading = true;
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut details = repo.commit_details(&oid)?;
                    if let Some(stash) = &stash {
                        details.files = repo.stash_files(stash)?;
                    }
                    anyhow::Ok(details)
                })
                .await;
            this.update(cx, |this, cx| {
                let state = this.detail_state_mut(loc);
                if state.generation != generation {
                    return;
                }
                state.loading = false;
                match result {
                    Ok(details) => {
                        // Keep the same file selected when moving between commits.
                        let ix = previous_path.and_then(|p| details.files.iter().position(|f| f.path == p)).or(if details.files.is_empty() { None } else { Some(0) });
                        state.details = Some(details);
                        state.selected_file = None;
                        state.diff = None;
                        state.info_scroll.set_offset(gpui::point(px(0.), px(0.)));
                        if let Some(ix) = ix {
                            this.select_detail_file(loc, ix, cx);
                        }
                    }
                    Err(e) => {
                        state.details = None;
                        this.toast(ToastKind::Error, "Could not load commit", Some(e.to_string()), cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn select_detail_file(&mut self, loc: DiffLocation, ix: usize, cx: &mut Context<Self>) {
        let state = self.detail_state_mut(loc);
        let Some(details) = state.details.as_ref() else { return };
        let Some(file) = details.files.get(ix).cloned() else { return };
        state.selected_file = Some(ix);
        state.files_scroll.scroll_to_item(ix, ScrollStrategy::Top);
        state.diff_generation += 1;
        let generation = state.diff_generation;
        let oid = details.oid.clone();
        let parent = details.parents.first().cloned();
        let stash = state.stash.clone();
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match &stash {
                        Some(s) => repo.stash_file_diff(s, &file),
                        None => repo.commit_file_diff(&oid, parent.as_deref(), &file),
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                let state = this.detail_state_mut(loc);
                if state.diff_generation != generation {
                    return;
                }
                match result {
                    Ok(diff) => state.diff = Some(DiffState::new(diff)),
                    Err(e) => {
                        state.diff = None;
                        this.toast(ToastKind::Error, "Could not load diff", Some(e.to_string()), cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn detail_file_move(&mut self, loc: DiffLocation, delta: isize, cx: &mut Context<Self>) {
        let state = self.detail_state(loc);
        let Some(details) = state.details.as_ref() else { return };
        if details.files.is_empty() {
            return;
        }
        let cur = state.selected_file.unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, details.files.len() as isize - 1) as usize;
        self.select_detail_file(loc, next, cx);
    }

    pub fn render_commit_detail(&mut self, loc: DiffLocation, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let state = self.detail_state(loc);
        let Some(details) = state.details.as_ref() else {
            if state.loading {
                return div().size_full().flex().items_center().justify_center().child(super::spinner(cx)).into_any_element();
            }
            return empty_state(Icon::Commit, "No commit selected", "Select a commit to see its details.", cx);
        };
        let split = self.details_split;
        let left = div()
            .h_full()
            .w(relative(split))
            .min_w(px(220.))
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(theme.border_subtle)
            .child(self.render_commit_info(loc, details, cx))
            .child(self.render_detail_files(loc, window, cx));
        let diff = self.render_diff(loc, DiffMode::ReadOnly, self.detail_diff_toolbar(loc, cx), cx);
        div()
            .id(("detail-split", loc as usize))
            .size_full()
            .flex()
            .on_drag_move(cx.listener(|this, e: &DragMoveEvent<SplitDrag>, _, cx| {
                if e.drag(cx).0 == 2 {
                    let w = f32::from(e.bounds.size.width).max(1.);
                    this.details_split = (f32::from(e.event.position.x - e.bounds.left()) / w).clamp(0.15, 0.8);
                    cx.notify();
                }
            }))
            .child(left)
            .child(ui::splitter_handle(("detail-vsplit", loc as usize), true, cx).on_drag(SplitDrag(2), |_, _, _, cx| cx.new(|_| ui::DragGhost)))
            .child(div().flex_1().min_w_0().h_full().child(diff))
            .into_any_element()
    }

    fn detail_diff_toolbar(&self, loc: DiffLocation, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.detail_state(loc);
        let file = state.selected_file.and_then(|i| state.details.as_ref()?.files.get(i).cloned())?;
        let path = file.path.clone();
        Some(
            div()
                .flex()
                .gap(px(4.))
                .child(ui::Button::icon_only("diff-file-history", Icon::History).small().tooltip("File history").on_click(cx.listener(move |this, _, window, cx| {
                    this.show_history_scope(crate::git::LogScope::File(path.clone()), window, cx);
                })))
                .into_any_element(),
        )
    }

    fn render_commit_info(&self, loc: DiffLocation, d: &CommitDetails, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let mut lines = d.message.lines();
        let subject = lines.next().unwrap_or("").to_string();
        let body = lines.collect::<Vec<_>>().join("\n").trim().to_string();
        let labels = self.refs_by_oid.get(&d.oid).cloned().unwrap_or_default();
        let oid = d.oid.clone();
        let oid_copy = d.oid.clone();
        let mut parents = div().flex().flex_wrap().gap(px(6.)).items_center();
        for p in &d.parents {
            let p2 = p.clone();
            parents = parents.child(
                div()
                    .id(gpui::SharedString::from(format!("parent-{p}")))
                    .font_family(mono_font())
                    .text_size(px(12.))
                    .text_color(theme.accent_hover)
                    .cursor_pointer()
                    .hover(|s| s.underline())
                    .child(short_oid(p).to_string())
                    .on_click(cx.listener(move |this, _, window, cx| this.reveal_commit(&p2, window, cx))),
            );
        }
        let committer_differs = d.committer_name != d.author_name;
        let stash_label = (loc == DiffLocation::Stash).then(|| self.stash_view.detail.stash.as_ref().map(|s| s.name.clone())).flatten();
        let meta_row = |label: &'static str, value: AnyElement| div().flex().items_center().gap(px(8.)).child(div().w(px(64.)).flex_none().text_color(theme.text_faint).child(label)).child(value);

        div()
            .id(("commit-info", loc as usize))
            .flex_none()
            .max_h(px(260.))
            .overflow_y_scroll()
            .track_scroll(&self.detail_state(loc).info_scroll)
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .border_b_1()
            .border_color(theme.border_subtle)
            .text_size(px(12.))
            .child(div().text_size(px(14.)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(theme.text).child(subject))
            .when(!body.is_empty(), |el| el.child(div().text_color(theme.text_muted).whitespace_normal().child(body)))
            .when(!labels.is_empty(), |el| {
                let mut row = div().flex().flex_wrap().gap(px(4.));
                for l in &labels {
                    row = row.child(super::history::ref_badge(l, &theme));
                }
                el.child(row)
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(ui::avatar(&d.author_name, px(28.), cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(div().text_color(theme.text).font_weight(gpui::FontWeight::MEDIUM).child(d.author_name.clone()))
                            .child(div().text_color(theme.text_faint).whitespace_nowrap().overflow_hidden().text_ellipsis().child(d.author_email.clone())),
                    ),
            )
            .child(meta_row("Date", div().text_color(theme.text_muted).child(ui::absolute_time(d.author_time)).into_any_element()))
            .when(committer_differs, |el| el.child(meta_row("Committer", div().text_color(theme.text_muted).child(format!("{} <{}> · {}", d.committer_name, d.committer_email, ui::relative_time(d.committer_time))).into_any_element())))
            .when_some(stash_label, |el, s| el.child(meta_row("Stash", div().text_color(theme.text_muted).child(s).into_any_element())))
            .child(meta_row(
                "SHA",
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().font_family(mono_font()).text_color(theme.text_muted).child(short_oid(&oid).to_string()))
                    .child(ui::Button::icon_only("copy-sha", Icon::Copy).small().tooltip("Copy full SHA").on_click(cx.listener(move |this, _, _, cx| this.copy_to_clipboard(oid_copy.clone(), cx))))
                    .into_any_element(),
            ))
            .when(!d.parents.is_empty(), |el| el.child(meta_row(if d.parents.len() > 1 { "Parents" } else { "Parent" }, parents.into_any_element())))
            .into_any_element()
    }

    fn render_detail_files(&self, loc: DiffLocation, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let state = self.detail_state(loc);
        let count = state.details.as_ref().map(|d| d.files.len()).unwrap_or(0);
        let (adds, dels) = state.details.as_ref().map(|d| d.files.iter().fold((0u32, 0u32), |(a, b), f| (a + f.additions.unwrap_or(0), b + f.deletions.unwrap_or(0)))).unwrap_or((0, 0));
        let focused = state.files_focus.is_focused(window);
        let header = div()
            .flex_none()
            .h(px(28.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(11.))
            .text_color(theme.text_faint)
            .child(format!("{count} changed file{}", if count == 1 { "" } else { "s" }))
            .child(div().flex_1())
            .child(div().text_color(theme.status_added).child(format!("+{adds}")))
            .child(div().text_color(theme.status_deleted).child(format!("−{dels}")));
        let focus = state.files_focus.clone();
        div()
            .id(("detail-files", loc as usize))
            .key_context("List")
            .track_focus(&focus)
            .on_action(cx.listener(move |this, _: &SelectPrev, _, cx| this.detail_file_move(loc, -1, cx)))
            .on_action(cx.listener(move |this, _: &SelectNext, _, cx| this.detail_file_move(loc, 1, cx)))
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(header)
            .child(
                uniform_list(
                    ("detail-file-list", loc as usize),
                    count,
                    cx.processor(move |this: &mut RepoView, range: Range<usize>, _window, cx| range.map(|ix| this.render_detail_file_row(loc, ix, focused, cx)).collect::<Vec<_>>()),
                )
                .track_scroll(state.files_scroll.clone())
                .flex_1(),
            )
            .into_any_element()
    }

    fn render_detail_file_row(&self, loc: DiffLocation, ix: usize, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let state = self.detail_state(loc);
        let Some(file) = state.details.as_ref().and_then(|d| d.files.get(ix)).cloned() else { return div().into_any_element() };
        let selected = state.selected_file == Some(ix);
        let (name, dir) = ui::split_path(&file.path);
        let bg = if selected {
            if focused {
                theme.selection
            } else {
                theme.selection_inactive
            }
        } else {
            gpui::transparent_black()
        };
        let file2 = file.clone();
        div()
            .id(("detail-file", ix))
            .w_full()
            .h(px(24.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .bg(bg)
            .when(!selected, |d| d.hover(|s| s.bg(theme.hover)))
            .text_size(px(13.))
            .child(ui::status_badge(file.status, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_baseline()
                    .gap(px(6.))
                    .overflow_hidden()
                    .child(div().whitespace_nowrap().text_color(theme.text).child(name.to_string()))
                    .child(div().whitespace_nowrap().overflow_hidden().text_ellipsis().text_size(px(11.)).text_color(theme.text_faint).child(dir.to_string())),
            )
            .when_some(file.additions.filter(|a| *a > 0), |d, a| d.child(div().text_size(px(11.)).text_color(theme.status_added).child(format!("+{a}"))))
            .when_some(file.deletions.filter(|a| *a > 0), |d, a| d.child(div().text_size(px(11.)).text_color(theme.status_deleted).child(format!("−{a}"))))
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.detail_state(loc).files_focus.clone());
                this.select_detail_file(loc, ix, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                    let menu = this.detail_file_menu(loc, &file2, e.position, cx);
                    this.open_menu(menu, cx);
                }),
            )
            .into_any_element()
    }

    pub fn checkout_file_at(&mut self, oid: Oid, file: ChangedFile, cx: &mut Context<Self>) {
        let path = file.path.clone();
        self.simple_op(format!("Restoring {path}"), format!("Restored {path} from {}", short_oid(&oid)), move |repo| repo.run(&["checkout", &oid, "--", &file.path]), cx);
    }
}
