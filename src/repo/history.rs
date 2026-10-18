//! Commit history with a branch graph.

use super::diff_view::DiffLocation;
use super::{DetailState, Nav, RepoView, ToastKind};
use crate::actions::{SelectFirst, SelectLast, SelectNext, SelectPrev};
use crate::assets::Icon;
use crate::git::graph::{Anchor, GraphBuilder, GraphRow};
use crate::git::{Commit, LogScope, Oid, RefKind, RefLabel};
use crate::settings::Settings;
use crate::theme::{mono_font, ActiveTheme, Theme};
use crate::ui::text_input::TextInput;
use crate::ui::{self, empty_state, icon, icon_sized, Checkbox, SplitDrag};
use gpui::{
    canvas, div, point, prelude::*, px, quad, relative, size, uniform_list, AnyElement, Bounds, Context, Corners, DragMoveEvent, Entity, FocusHandle, MouseButton,
    PathBuilder, Pixels, ScrollStrategy, SharedString, UniformListScrollHandle, Window,
};
use std::ops::Range;
use std::time::Duration;

const PAGE: usize = 400;
pub const ROW_HEIGHT: f32 = 26.;
const LANE_W: f32 = 14.;
const MAX_LANES: u16 = 18;

pub struct HistoryState {
    pub scope: LogScope,
    pub commits: Vec<Commit>,
    pub graph: Vec<GraphRow>,
    builder: GraphBuilder,
    pub max_lanes: u16,
    pub loading: bool,
    pub has_more: bool,
    generation: u64,
    pub selected: Option<usize>,
    pub selected_oid: Option<Oid>,
    pub scroll: UniformListScrollHandle,
    pub search: Entity<TextInput>,
    pub search_results: Option<Vec<Commit>>,
    search_generation: u64,
    pub searching: bool,
    pub detail: DetailState,
    pub list_focus: FocusHandle,
    pending_select: Option<Oid>,
}

impl HistoryState {
    pub fn new(scope: LogScope, search: Entity<TextInput>, cx: &mut Context<RepoView>) -> Self {
        Self {
            scope,
            commits: Vec::new(),
            graph: Vec::new(),
            builder: GraphBuilder::new(),
            max_lanes: 1,
            loading: false,
            has_more: false,
            generation: 0,
            selected: None,
            selected_oid: None,
            scroll: UniformListScrollHandle::new(),
            search,
            search_results: None,
            search_generation: 0,
            searching: false,
            detail: DetailState::new(cx),
            list_focus: cx.focus_handle(),
            pending_select: None,
        }
    }

    pub fn visible(&self) -> &[Commit] {
        match &self.search_results {
            Some(r) => r,
            None => &self.commits,
        }
    }

    fn rebuild_graph(&mut self) {
        self.builder = GraphBuilder::new();
        self.graph = self.commits.iter().map(|c| self.builder.push(c)).collect();
        self.max_lanes = self.graph.iter().map(|r| r.width).max().unwrap_or(1).clamp(1, MAX_LANES);
    }

    fn append(&mut self, more: Vec<Commit>) {
        for c in &more {
            let row = self.builder.push(c);
            self.max_lanes = self.max_lanes.max(row.width).min(MAX_LANES);
            self.graph.push(row);
        }
        self.commits.extend(more);
    }
}

impl RepoView {
    pub fn reload_history(&mut self, cx: &mut Context<Self>) {
        self.history.generation += 1;
        let generation = self.history.generation;
        self.history.loading = true;
        let scope = self.history.scope.clone();
        let repo = self.repo.clone();
        let count = self.history.commits.len().clamp(PAGE, 5000);
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { repo.log(&scope, 0, count) }).await;
            this.update(cx, |this, cx| {
                if this.history.generation != generation {
                    return;
                }
                this.history.loading = false;
                match result {
                    Ok(commits) => {
                        this.history.has_more = commits.len() == count;
                        this.history.commits = commits;
                        this.history.rebuild_graph();
                        this.restore_history_selection(cx);
                    }
                    Err(e) => {
                        this.history.commits.clear();
                        this.history.graph.clear();
                        this.toast(ToastKind::Error, "Could not load history", Some(e.to_string()), cx);
                    }
                }
                if !this.history.search.read(cx).text().is_empty() {
                    this.on_search_changed(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn restore_history_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(oid) = self.history.pending_select.take() {
            if let Some(ix) = self.history.commits.iter().position(|c| c.oid == oid) {
                self.select_commit(ix, true, cx);
                return;
            }
        }
        let wanted = self.history.selected_oid.clone();
        let ix = wanted.and_then(|oid| self.history.visible().iter().position(|c| c.oid == oid));
        match ix {
            Some(ix) => {
                self.history.selected = Some(ix);
                // Commit content is immutable, but refresh in case refs changed.
                cx.notify();
            }
            None if !self.history.visible().is_empty() => self.select_commit(0, false, cx),
            None => {
                self.history.selected = None;
                self.history.selected_oid = None;
                self.history.detail.clear();
            }
        }
    }

    pub fn load_more_history(&mut self, cx: &mut Context<Self>) {
        if self.history.loading || !self.history.has_more || self.history.search_results.is_some() {
            return;
        }
        self.history.loading = true;
        let generation = self.history.generation;
        let scope = self.history.scope.clone();
        let skip = self.history.commits.len();
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { repo.log(&scope, skip, PAGE) }).await;
            this.update(cx, |this, cx| {
                if this.history.generation != generation {
                    return;
                }
                this.history.loading = false;
                if let Ok(more) = result {
                    this.history.has_more = more.len() == PAGE;
                    this.history.append(more);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn on_search_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.history.search.read(cx).text().trim().to_string();
        self.history.search_generation += 1;
        let generation = self.history.search_generation;
        if query.is_empty() {
            self.history.search_results = None;
            self.history.searching = false;
            let oid = self.history.selected_oid.clone();
            self.history.selected = oid.and_then(|o| self.history.commits.iter().position(|c| c.oid == o));
            if let Some(ix) = self.history.selected {
                self.history.scroll.scroll_to_item(ix, ScrollStrategy::Center);
            }
            cx.notify();
            return;
        }
        self.history.searching = true;
        let scope = self.history.scope.clone();
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(220)).await;
            let stale = this.update(cx, |this, _| this.history.search_generation != generation).unwrap_or(true);
            if stale {
                return;
            }
            let result = cx.background_executor().spawn(async move { repo.search_log(&scope, &query, 1000) }).await;
            this.update(cx, |this, cx| {
                if this.history.search_generation != generation {
                    return;
                }
                this.history.searching = false;
                let results = result.unwrap_or_default();
                this.history.search_results = Some(results);
                let oid = this.history.selected_oid.clone();
                this.history.selected = oid.and_then(|o| this.history.visible().iter().position(|c| c.oid == o));
                if this.history.selected.is_none() && !this.history.visible().is_empty() {
                    this.select_commit(0, false, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn select_commit(&mut self, ix: usize, scroll: bool, cx: &mut Context<Self>) {
        let Some(commit) = self.history.visible().get(ix).cloned() else { return };
        self.history.selected = Some(ix);
        if scroll {
            self.history.scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
        if self.history.selected_oid.as_deref() != Some(&commit.oid) || self.history.detail.details.is_none() {
            self.history.selected_oid = Some(commit.oid.clone());
            self.load_commit_detail(DiffLocation::History, commit.oid, cx);
        }
        cx.notify();
    }

    /// Selects a commit by id, switching to the history view. Loads it if needed.
    pub fn reveal_commit(&mut self, oid: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.set_nav(Nav::History, window, cx);
        if self.history.search_results.is_some() {
            self.history.search.update(cx, |s, cx| s.clear(cx));
            self.history.search_results = None;
        }
        if let Some(ix) = self.history.commits.iter().position(|c| c.oid == oid) {
            self.select_commit(ix, true, cx);
        } else {
            // Not loaded in this scope: show all branches and select it once loaded.
            self.history.pending_select = Some(oid.to_string());
            self.history.scope = LogScope::All;
            self.history.commits.clear();
            self.reload_history(cx);
        }
    }

    fn history_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.history.visible().len();
        if len == 0 {
            return;
        }
        let cur = self.history.selected.unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, len as isize - 1) as usize;
        self.select_commit(next, true, cx);
        if next + 50 > self.history.commits.len() {
            self.load_more_history(cx);
        }
    }

    pub fn render_history(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let scope_title = match &self.history.scope {
            LogScope::All => "All Branches".to_string(),
            LogScope::Head => format!("History of {}", self.snapshot.head.display_name()),
            LogScope::Ref(r) => format!("History of {r}"),
            LogScope::File(p) => format!("History of {p}"),
        };
        let show_all = self.history.scope == LogScope::All;
        let count = self.history.visible().len();
        let count_label = if self.history.search_results.is_some() {
            format!("{count} match{}", if count == 1 { "" } else { "es" })
        } else {
            format!("{count}{} commits", if self.history.has_more { "+" } else { "" })
        };

        let header = div()
            .flex_none()
            .h(px(42.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(12.))
            .border_b_1()
            .border_color(theme.border_subtle)
            .child(icon(Icon::History).text_color(theme.text_muted))
            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).whitespace_nowrap().overflow_hidden().text_ellipsis().child(scope_title))
            .child(div().text_size(px(12.)).text_color(theme.text_faint).whitespace_nowrap().child(count_label))
            .when(matches!(self.history.scope, LogScope::File(_) | LogScope::Ref(_)), |d| {
                d.child(ui::Button::new("history-back", "Show HEAD").small().ghost().on_click(cx.listener(|this, _, window, cx| this.show_history_scope(LogScope::Head, window, cx))))
            })
            .child(div().flex_1())
            .when(self.history.searching || self.history.loading, |d| d.child(super::spinner(cx)))
            .child(
                Checkbox::new("show-all-branches", show_all).label("All branches").on_toggle(cx.listener(|this, checked: &bool, window, cx| {
                    let checked = *checked;
                    cx.update_global::<Settings, _>(|s, _| {
                        s.show_all_branches = checked;
                        s.save();
                    });
                    this.show_history_scope(if checked { LogScope::All } else { LogScope::Head }, window, cx);
                })),
            )
            .child(
                div()
                    .w(px(300.))
                    .flex()
                    .items_center()
                    .relative()
                    .child(self.history.search.clone())
                    .child(div().absolute().right(px(8.)).top(px(7.)).child(icon(Icon::Search).text_color(theme.text_faint))),
            );

        let list: AnyElement = if self.history.visible().is_empty() && !self.history.loading {
            if self.history.search_results.is_some() {
                empty_state(Icon::Search, "No matching commits", "Try a different search term.", cx)
            } else {
                empty_state(Icon::Commit, "No commits yet", "Commit your changes to start the history of this branch.", cx)
            }
        } else {
            self.render_commit_list(window, cx)
        };

        let split = self.history_split;
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .id("history-split")
                    .flex_1()
                    .min_h_0()
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
                    .child(ui::splitter_handle("history-hsplit", false, cx).on_drag(SplitDrag(1), |_, _, _, cx| cx.new(|_| ui::DragGhost)))
                    .child(div().flex_1().min_h_0().child(self.render_commit_detail(DiffLocation::History, window, cx))),
            )
    }

    fn render_commit_list(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let searching = self.history.search_results.is_some();
        let graph_w = if searching { px(8.) } else { px(LANE_W * self.history.max_lanes as f32 + 12.) };
        let focused = self.history.list_focus.is_focused(window);
        let col_header = div()
            .flex_none()
            .h(px(24.))
            .flex()
            .items_center()
            .text_size(px(11.))
            .text_color(theme.text_faint)
            .border_b_1()
            .border_color(theme.border_subtle)
            .child(div().w(graph_w).flex_none())
            .child(div().flex_1().child("Description"))
            .child(div().w(px(150.)).flex_none().child("Author"))
            .child(div().w(px(120.)).flex_none().child("Date"))
            .child(div().w(px(76.)).flex_none().child("SHA"));
        let count = self.history.visible().len();
        let list = uniform_list(
            "commit-list",
            count,
            cx.processor(move |this: &mut RepoView, range: Range<usize>, _window, cx| {
                if !this.history.loading && this.history.has_more && range.end + 60 > this.history.commits.len() && this.history.search_results.is_none() {
                    this.load_more_history(cx);
                }
                range.map(|ix| this.render_commit_row(ix, graph_w, focused, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.history.scroll.clone())
        .flex_1();
        div()
            .id("commit-list-container")
            .key_context("List")
            .track_focus(&self.history.list_focus)
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.history_move(-1, cx)))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.history_move(1, cx)))
            .on_action(cx.listener(|this, _: &SelectFirst, _, cx| this.history_move(-1_000_000, cx)))
            .on_action(cx.listener(|this, _: &SelectLast, _, cx| this.history_move(1_000_000, cx)))
            .size_full()
            .flex()
            .flex_col()
            .text_size(px(13.))
            .child(col_header)
            .child(list)
            .into_any_element()
    }

    fn render_commit_row(&self, ix: usize, graph_w: Pixels, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let commits = self.history.visible();
        let commit = &commits[ix];
        let selected = self.history.selected == Some(ix);
        let is_head = self.snapshot.head.oid.as_deref() == Some(&commit.oid);
        let graph_row = if self.history.search_results.is_none() { self.history.graph.get(ix).cloned() } else { None };
        let labels = self.refs_by_oid.get(&commit.oid).cloned().unwrap_or_default();
        let unpushed = self.unpushed.contains(&commit.oid);
        let oid = commit.oid.clone();

        let graph_cell: AnyElement = match graph_row {
            Some(row) => {
                let theme2 = theme.clone();
                canvas(move |_, _, _| (), move |bounds, _, window, _| paint_graph_row(&row, is_head, bounds, &theme2, window)).w(graph_w).h_full().flex_none().into_any_element()
            }
            None => div().w(graph_w).flex_none().into_any_element(),
        };

        let mut desc = div().flex_1().min_w_0().flex().items_center().gap(px(5.)).overflow_hidden();
        let shown = labels.len().min(3);
        for label in labels.iter().take(shown) {
            desc = desc.child(ref_badge(label, &theme));
        }
        if labels.len() > shown {
            desc = desc.child(div().text_size(px(11.)).text_color(theme.text_faint).child(format!("+{}", labels.len() - shown)));
        }
        if unpushed {
            desc = desc.child(icon_sized(Icon::ArrowUp, px(12.)).text_color(theme.info));
        }
        desc = desc.child(
            div()
                .min_w_0()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .text_color(if commit.parents.len() > 1 { theme.text_muted } else { theme.text })
                .when(is_head, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                .child(commit.subject.clone()),
        );

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
            .id(("commit", ix))
            .h(px(ROW_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .bg(bg)
            .when(!selected, |d| d.hover(|s| s.bg(theme.hover)))
            .child(graph_cell)
            .child(desc)
            .child(div().w(px(150.)).flex_none().pl(px(8.)).flex().items_center().gap(px(6.)).text_color(theme.text_muted).overflow_hidden().child(ui::avatar(&commit.author_name, px(16.), cx)).child(div().whitespace_nowrap().overflow_hidden().text_ellipsis().child(commit.author_name.clone())))
            .child(div().w(px(120.)).flex_none().text_color(theme.text_faint).whitespace_nowrap().overflow_hidden().child(ui::relative_time(commit.author_time)))
            .child(div().w(px(76.)).flex_none().font_family(mono_font()).text_size(px(12.)).text_color(theme.text_faint).child(commit.short().to_string()))
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.history.list_focus);
                this.select_commit(ix, false, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                    window.focus(&this.history.list_focus);
                    this.select_commit(ix, false, cx);
                    let menu = this.commit_menu(&oid, e.position, cx);
                    this.open_menu(menu, cx);
                }),
            )
            .into_any_element()
    }
}

pub fn ref_badge(label: &RefLabel, theme: &Theme) -> AnyElement {
    let (color, icon_kind) = match label.kind {
        RefKind::Head => (theme.ref_head, Icon::Commit),
        RefKind::Local if label.is_head => (theme.ref_head, Icon::Check),
        RefKind::Local => (theme.ref_local, Icon::Branch),
        RefKind::Remote => (theme.ref_remote, Icon::Cloud),
        RefKind::Tag => (theme.ref_tag, Icon::Tag),
    };
    div()
        .flex_none()
        .h(px(18.))
        .px(px(6.))
        .flex()
        .items_center()
        .gap(px(4.))
        .rounded(px(4.))
        .bg(color.opacity(0.22))
        .border_1()
        .border_color(color.opacity(0.65))
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.text)
        .max_w(px(220.))
        .child(icon_sized(icon_kind, px(11.)).text_color(color))
        .child(div().whitespace_nowrap().overflow_hidden().text_ellipsis().child(SharedString::from(label.name.clone())))
        .into_any_element()
}

fn paint_graph_row(row: &GraphRow, is_head: bool, bounds: Bounds<Pixels>, theme: &Theme, window: &mut Window) {
    let lane_x = |lane: u16| bounds.left() + px(6. + LANE_W * lane as f32 + LANE_W / 2.);
    let y_of = |a: Anchor| match a {
        Anchor::Top => bounds.top(),
        Anchor::Middle => bounds.top() + bounds.size.height * 0.5,
        Anchor::Bottom => bounds.bottom(),
    };
    let max_x = bounds.right();
    for seg in &row.segments {
        if seg.from_lane >= MAX_LANES || seg.to_lane >= MAX_LANES {
            continue;
        }
        let color = theme.graph_color(seg.color);
        let (x1, y1, x2, y2) = (lane_x(seg.from_lane), y_of(seg.from), lane_x(seg.to_lane), y_of(seg.to));
        if x1 > max_x || x2 > max_x {
            continue;
        }
        if seg.from_lane == seg.to_lane {
            window.paint_quad(gpui::fill(Bounds::from_corners(point(x1 - px(1.), y1), point(x1 + px(1.), y2)), color));
        } else {
            let mut path = PathBuilder::stroke(px(2.));
            let p1 = point(x1, y1);
            let p2 = point(x2, y2);
            path.move_to(p1);
            let my = (y1 + y2) * 0.5;
            path.cubic_bezier_to(p2, point(x1, my), point(x2, my));
            if let Ok(built) = path.build() {
                window.paint_path(built, color);
            }
        }
    }
    if row.lane < MAX_LANES {
        let cx_ = lane_x(row.lane);
        let cy = y_of(Anchor::Middle);
        let color = theme.graph_color(row.color);
        let r = if is_head { px(5.5) } else { px(4.5) };
        let node = Bounds::new(point(cx_ - r, cy - r), size(r * 2., r * 2.));
        if row.is_merge {
            window.paint_quad(quad(node, Corners::all(r), theme.app_bg, px(2.), color, gpui::BorderStyle::Solid));
        } else {
            window.paint_quad(quad(node, Corners::all(r), color, px(if is_head { 2. } else { 0. }), theme.text, gpui::BorderStyle::Solid));
        }
    }
}

