//! Stash list and reflog views.

use super::diff_view::DiffLocation;
use super::{DetailState, RepoView, ToastKind};
use crate::actions::{SelectNext, SelectPrev};
use crate::assets::Icon;
use crate::git::{short_oid, ReflogEntry, Stash};
use crate::theme::{mono_font, ActiveTheme};
use crate::ui::{self, empty_state, icon, Button, SplitDrag};
use gpui::{div, prelude::*, px, relative, uniform_list, AnyElement, Context, DragMoveEvent, FocusHandle, MouseButton, ScrollStrategy, UniformListScrollHandle, Window};
use std::ops::Range;

pub struct StashViewState {
    pub selected: Option<usize>,
    pub detail: DetailState,
    pub list_focus: FocusHandle,
    pub scroll: UniformListScrollHandle,
}

impl StashViewState {
    pub fn new(cx: &mut Context<RepoView>) -> Self {
        Self { selected: None, detail: DetailState::new(cx), list_focus: cx.focus_handle(), scroll: UniformListScrollHandle::new() }
    }
}

pub struct ReflogState {
    pub entries: Vec<ReflogEntry>,
    pub loaded: bool,
    pub selected: Option<usize>,
    pub detail: DetailState,
    pub list_focus: FocusHandle,
    pub scroll: UniformListScrollHandle,
}

impl ReflogState {
    pub fn new(cx: &mut Context<RepoView>) -> Self {
        Self { entries: Vec::new(), loaded: false, selected: None, detail: DetailState::new(cx), list_focus: cx.focus_handle(), scroll: UniformListScrollHandle::new() }
    }
}

impl RepoView {
    pub fn validate_stash_selection(&mut self, cx: &mut Context<Self>) {
        let stashes = &self.snapshot.stashes;
        if stashes.is_empty() {
            self.stash_view.selected = None;
            self.stash_view.detail.clear();
            return;
        }
        let current_oid = self.stash_view.detail.stash.as_ref().map(|s| s.oid.clone());
        match current_oid.and_then(|oid| stashes.iter().position(|s| s.oid == oid)) {
            Some(ix) => {
                self.stash_view.selected = Some(ix);
                // Indices shift when stashes are added or dropped.
                if let Some(s) = self.stash_view.detail.stash.as_mut() {
                    *s = stashes[ix].clone();
                }
            }
            None => self.select_stash(0, cx),
        }
    }

    pub fn select_stash(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(stash) = self.snapshot.stashes.get(ix).cloned() else { return };
        self.stash_view.selected = Some(ix);
        self.stash_view.scroll.scroll_to_item(ix, ScrollStrategy::Top);
        self.load_stash_detail(stash, cx);
        cx.notify();
    }

    pub fn apply_stash(&mut self, stash: Stash, pop: bool, cx: &mut Context<Self>) {
        let msg = if pop { "Stash applied and removed" } else { "Stash applied" };
        self.simple_op(if pop { "Popping stash" } else { "Applying stash" }, msg, move |repo| repo.stash_apply(&stash.name, pop), cx);
    }

    pub fn confirm_drop_stash(&mut self, stash: Stash, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(
            "Delete stash?",
            format!("“{}” will be permanently deleted.", stash.message),
            "Delete Stash",
            true,
            move |this, _, cx| {
                let s = stash.clone();
                this.simple_op("Deleting stash", "Stash deleted", move |repo| repo.stash_drop(&s.name), cx);
            },
            window,
            cx,
        );
    }

    pub fn load_reflog(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { repo.reflog(1000) }).await;
            this.update(cx, |this, cx| {
                this.reflog.loaded = true;
                match result {
                    Ok(entries) => {
                        let selected_oid = this.reflog.selected.and_then(|i| this.reflog.entries.get(i)).map(|e| e.oid.clone());
                        this.reflog.entries = entries;
                        let ix = selected_oid.and_then(|o| this.reflog.entries.iter().position(|e| e.oid == o));
                        match ix {
                            Some(ix) => this.reflog.selected = Some(ix),
                            None if !this.reflog.entries.is_empty() => this.select_reflog(0, cx),
                            None => this.reflog.selected = None,
                        }
                    }
                    Err(e) => this.toast(ToastKind::Error, "Could not load reflog", Some(e.to_string()), cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn select_reflog(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.reflog.entries.get(ix).cloned() else { return };
        self.reflog.selected = Some(ix);
        self.reflog.scroll.scroll_to_item(ix, ScrollStrategy::Top);
        if self.reflog.detail.oid.as_deref() != Some(&entry.oid) {
            self.load_commit_detail(DiffLocation::Reflog, entry.oid, cx);
        }
        cx.notify();
    }

    fn split_view(&mut self, list: AnyElement, loc: DiffLocation, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let split = self.history_split;
        div()
            .id(("split-view", loc as usize))
            .size_full()
            .flex()
            .flex_col()
            .on_drag_move(cx.listener(|this, e: &DragMoveEvent<SplitDrag>, _, cx| {
                if e.drag(cx).0 == 1 {
                    let h = f32::from(e.bounds.size.height).max(1.);
                    this.history_split = (f32::from(e.event.position.y - e.bounds.top()) / h).clamp(0.12, 0.88);
                    cx.notify();
                }
            }))
            .child(div().h(relative(split)).min_h(px(80.)).child(list))
            .child(ui::splitter_handle(("list-hsplit", loc as usize), false, cx).on_drag(SplitDrag(1), |_, _, _, cx| cx.new(|_| ui::DragGhost)))
            .child(div().flex_1().min_h_0().child(self.render_commit_detail(loc, window, cx)))
            .into_any_element()
    }

    pub fn render_stashes(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let count = self.snapshot.stashes.len();
        let selected = self.stash_view.selected.and_then(|i| self.snapshot.stashes.get(i).cloned());
        let header = div()
            .flex_none()
            .h(px(42.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(theme.border_subtle)
            .child(icon(Icon::Stash).text_color(theme.text_muted))
            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("Stashes"))
            .child(div().text_size(px(12.)).text_color(theme.text_faint).child(format!("{count}")))
            .child(div().flex_1())
            .child(Button::new("new-stash", "Stash Changes…").small().icon(Icon::Plus).disabled(self.snapshot.status.is_clean()).on_click(cx.listener(|this, _, window, cx| this.open_stash_dialog(window, cx))))
            .when_some(selected.clone(), |d, s| {
                let (s1, s2, s3) = (s.clone(), s.clone(), s);
                d.child(Button::new("apply-stash", "Apply").small().on_click(cx.listener(move |this, _, _, cx| this.apply_stash(s1.clone(), false, cx))))
                    .child(Button::new("pop-stash", "Pop").small().primary().on_click(cx.listener(move |this, _, _, cx| this.apply_stash(s2.clone(), true, cx))))
                    .child(Button::icon_only("drop-stash", Icon::Trash).small().tooltip("Delete stash").on_click(cx.listener(move |this, _, window, cx| this.confirm_drop_stash(s3.clone(), window, cx))))
            });
        let body = if count == 0 {
            empty_state(Icon::Stash, "No stashes", "Stash your changes to set them aside and get a clean working copy.", cx)
        } else {
            let focused = self.stash_view.list_focus.is_focused(window);
            let list = div()
                .id("stash-list-container")
                .key_context("List")
                .track_focus(&self.stash_view.list_focus)
                .on_action(cx.listener(|this, _: &SelectPrev, _, cx| {
                    let ix = this.stash_view.selected.unwrap_or(0).saturating_sub(1);
                    this.select_stash(ix, cx);
                }))
                .on_action(cx.listener(|this, _: &SelectNext, _, cx| {
                    let ix = (this.stash_view.selected.map(|i| i + 1).unwrap_or(0)).min(this.snapshot.stashes.len().saturating_sub(1));
                    this.select_stash(ix, cx);
                }))
                .size_full()
                .child(
                    uniform_list("stash-list", count, cx.processor(move |this: &mut RepoView, range: Range<usize>, _window, cx| range.map(|ix| this.render_stash_row(ix, focused, cx)).collect::<Vec<_>>()))
                        .track_scroll(self.stash_view.scroll.clone())
                        .size_full(),
                )
                .into_any_element();
            self.split_view(list, DiffLocation::Stash, window, cx)
        };
        div().size_full().flex().flex_col().child(header).child(div().flex_1().min_h_0().child(body))
    }

    fn render_stash_row(&self, ix: usize, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(stash) = self.snapshot.stashes.get(ix) else { return div().into_any_element() };
        let selected = self.stash_view.selected == Some(ix);
        let bg = if selected {
            if focused {
                theme.selection
            } else {
                theme.selection_inactive
            }
        } else {
            gpui::transparent_black()
        };
        div()
            .id(("stash", ix))
            .w_full()
            .h(px(30.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(10.))
            .bg(bg)
            .when(!selected, |d| d.hover(|s| s.bg(theme.hover)))
            .child(icon(Icon::Stash).text_color(theme.text_muted))
            .child(div().flex_1().min_w_0().whitespace_nowrap().overflow_hidden().text_ellipsis().child(stash.message.clone()))
            .child(div().text_size(px(12.)).text_color(theme.text_faint).child(ui::relative_time(stash.time)))
            .child(div().w(px(80.)).font_family(mono_font()).text_size(px(12.)).text_color(theme.text_faint).child(stash.name.clone()))
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.stash_view.list_focus);
                this.select_stash(ix, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                    this.select_stash(ix, cx);
                    let menu = this.stash_menu(ix, e.position, cx);
                    this.open_menu(menu, cx);
                }),
            )
            .into_any_element()
    }

    pub fn render_reflog(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let count = self.reflog.entries.len();
        let header = div()
            .flex_none()
            .h(px(42.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(theme.border_subtle)
            .child(icon(Icon::List).text_color(theme.text_muted))
            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("Reflog"))
            .child(div().text_size(px(12.)).text_color(theme.text_faint).child("Every position HEAD has been in — use it to recover lost commits."))
            .child(div().flex_1())
            .child(Button::icon_only("reload-reflog", Icon::Refresh).small().tooltip("Reload").on_click(cx.listener(|this, _, _, cx| this.load_reflog(cx))));
        let body = if count == 0 {
            if self.reflog.loaded {
                empty_state(Icon::List, "Reflog is empty", "", cx)
            } else {
                div().size_full().flex().items_center().justify_center().child(super::spinner(cx)).into_any_element()
            }
        } else {
            let focused = self.reflog.list_focus.is_focused(window);
            let list = div()
                .id("reflog-list-container")
                .key_context("List")
                .track_focus(&self.reflog.list_focus)
                .on_action(cx.listener(|this, _: &SelectPrev, _, cx| {
                    let ix = this.reflog.selected.unwrap_or(0).saturating_sub(1);
                    this.select_reflog(ix, cx);
                }))
                .on_action(cx.listener(|this, _: &SelectNext, _, cx| {
                    let ix = (this.reflog.selected.map(|i| i + 1).unwrap_or(0)).min(this.reflog.entries.len().saturating_sub(1));
                    this.select_reflog(ix, cx);
                }))
                .size_full()
                .child(
                    uniform_list("reflog-list", count, cx.processor(move |this: &mut RepoView, range: Range<usize>, _window, cx| range.map(|ix| this.render_reflog_row(ix, focused, cx)).collect::<Vec<_>>()))
                        .track_scroll(self.reflog.scroll.clone())
                        .size_full(),
                )
                .into_any_element();
            self.split_view(list, DiffLocation::Reflog, window, cx)
        };
        div().size_full().flex().flex_col().child(header).child(div().flex_1().min_h_0().child(body))
    }

    fn render_reflog_row(&self, ix: usize, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(entry) = self.reflog.entries.get(ix) else { return div().into_any_element() };
        let selected = self.reflog.selected == Some(ix);
        let bg = if selected {
            if focused {
                theme.selection
            } else {
                theme.selection_inactive
            }
        } else {
            gpui::transparent_black()
        };
        let (action, rest) = entry.subject.split_once(": ").map(|(a, r)| (a.to_string(), r.to_string())).unwrap_or((String::new(), entry.subject.clone()));
        div()
            .id(("reflog", ix))
            .w_full()
            .h(px(26.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(10.))
            .bg(bg)
            .when(!selected, |d| d.hover(|s| s.bg(theme.hover)))
            .child(div().w(px(84.)).flex_none().font_family(mono_font()).text_size(px(12.)).text_color(theme.text_faint).child(entry.selector.clone()))
            .child(div().w(px(70.)).flex_none().font_family(mono_font()).text_size(px(12.)).text_color(theme.accent_hover).child(short_oid(&entry.oid).to_string()))
            .when(!action.is_empty(), |d| d.child(ui::badge(action, theme.active, theme.text_muted)))
            .child(div().flex_1().min_w_0().whitespace_nowrap().overflow_hidden().text_ellipsis().child(rest))
            .child(div().text_size(px(12.)).text_color(theme.text_faint).whitespace_nowrap().child(ui::relative_time(entry.time)))
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.reflog.list_focus);
                this.select_reflog(ix, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                    this.select_reflog(ix, cx);
                    let menu = this.reflog_menu(ix, e.position, cx);
                    this.open_menu(menu, cx);
                }),
            )
            .into_any_element()
    }
}
