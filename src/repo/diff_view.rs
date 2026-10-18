//! Rendering of unified diffs, with hunk and line staging.

use super::{RepoView, ToastKind};
use crate::assets::Icon;
use crate::git::{patch, FileDiff, LineKind};
use crate::theme::{mono_font, ActiveTheme};
use crate::ui::{self, empty_state, icon, Button};
use gpui::{
    div, prelude::*, px, uniform_list, AnyElement, Context, HighlightStyle, ListHorizontalSizingBehavior, MouseButton, SharedString, StyledText, UniformListScrollHandle,
    Window,
};
use std::collections::{BTreeSet, HashMap};
use std::ops::Range;

const MAX_ROWS: usize = 25_000;
pub const DIFF_ROW_HEIGHT: f32 = 20.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLocation {
    WorkingCopy,
    History,
    Stash,
    Reflog,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffMode {
    ReadOnly,
    Unstaged,
    Staged,
}

#[derive(Clone, Copy, Debug)]
pub enum DiffRow {
    Hunk(usize),
    Line(usize, usize),
}

pub struct DiffState {
    pub diff: FileDiff,
    pub rows: Vec<DiffRow>,
    pub longest_row: usize,
    pub scroll: UniformListScrollHandle,
    pub selected: BTreeSet<(usize, usize)>,
    pub anchor: Option<(usize, usize)>,
    pub highlights: HashMap<(usize, usize), Range<usize>>,
    pub truncated: bool,
}

/// Text shown for a diff line: no trailing CR, tabs expanded, and very long
/// lines (minified files) cut short so shaping stays fast.
pub fn display_text(content: &str) -> String {
    const MAX_CHARS: usize = 3000;
    let trimmed = content.strip_suffix('\r').unwrap_or(content);
    let mut text = if trimmed.contains('\t') { trimmed.replace('\t', "    ") } else { trimmed.to_string() };
    if text.len() > MAX_CHARS {
        let cut = text.char_indices().nth(MAX_CHARS).map(|(i, _)| i).unwrap_or(text.len());
        text.truncate(cut);
        text.push_str(" …");
    }
    text
}

impl DiffState {
    pub fn new(diff: FileDiff) -> Self {
        let mut rows = Vec::new();
        let mut longest = (0usize, 0usize);
        let mut truncated = false;
        'outer: for (hi, hunk) in diff.hunks.iter().enumerate() {
            rows.push(DiffRow::Hunk(hi));
            for (li, line) in hunk.lines.iter().enumerate() {
                if rows.len() >= MAX_ROWS {
                    truncated = true;
                    break 'outer;
                }
                let len = line.content.len().min(3000) + if line.content.contains('\t') { 12 } else { 0 };
                if len > longest.1 {
                    longest = (rows.len(), len);
                }
                rows.push(DiffRow::Line(hi, li));
            }
        }
        let highlights = compute_word_highlights(&diff);
        DiffState { diff, rows, longest_row: longest.0, scroll: UniformListScrollHandle::new(), selected: BTreeSet::new(), anchor: None, highlights, truncated }
    }

    /// Replaces the diff, keeping the scroll position when it's the same file.
    pub fn update(&mut self, diff: FileDiff) {
        let same = self.diff == diff;
        if same {
            return;
        }
        let scroll = self.scroll.clone();
        let keep_scroll = self.diff.path == diff.path;
        *self = DiffState::new(diff);
        if keep_scroll {
            self.scroll = scroll;
        }
    }

    fn selection_in_hunk(&self, hunk: usize) -> usize {
        self.selected.range((hunk, 0)..(hunk + 1, 0)).count()
    }
}

/// Highlights the changed part of paired removed/added lines.
fn compute_word_highlights(diff: &FileDiff) -> HashMap<(usize, usize), Range<usize>> {
    let mut out = HashMap::new();
    for (hi, hunk) in diff.hunks.iter().enumerate() {
        let lines = &hunk.lines;
        let mut i = 0;
        while i < lines.len() {
            if lines[i].kind != LineKind::Removed {
                i += 1;
                continue;
            }
            let del_start = i;
            while i < lines.len() && lines[i].kind == LineKind::Removed {
                i += 1;
            }
            let add_start = i;
            while i < lines.len() && lines[i].kind == LineKind::Added {
                i += 1;
            }
            let dels = add_start - del_start;
            let adds = i - add_start;
            if adds == 0 || dels != adds || dels > 40 {
                continue;
            }
            for k in 0..dels {
                let a = display_text(&lines[del_start + k].content);
                let b = display_text(&lines[add_start + k].content);
                if let Some((ra, rb)) = changed_ranges(&a, &b) {
                    out.insert((hi, del_start + k), ra);
                    out.insert((hi, add_start + k), rb);
                }
            }
        }
    }
    out
}

fn changed_ranges(a: &str, b: &str) -> Option<(Range<usize>, Range<usize>)> {
    if a == b || a.len() > 2000 || b.len() > 2000 {
        return None;
    }
    let prefix: usize = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).map(|(x, _)| x.len_utf8()).sum();
    let a_rest = &a[prefix..];
    let b_rest = &b[prefix..];
    let suffix: usize = a_rest.chars().rev().zip(b_rest.chars().rev()).take_while(|(x, y)| x == y).map(|(x, _)| x.len_utf8()).sum();
    let ra = prefix..a.len() - suffix;
    let rb = prefix..b.len() - suffix;
    // Not worth highlighting when nearly the whole line changed.
    let changed = ra.len().max(rb.len());
    if prefix + suffix == 0 || changed * 10 > a.len().max(b.len()) * 8 {
        return None;
    }
    Some((ra, rb))
}

impl RepoView {
    pub fn diff_state(&self, loc: DiffLocation) -> Option<&DiffState> {
        match loc {
            DiffLocation::WorkingCopy => self.wc.diff.as_ref(),
            DiffLocation::History => self.history.detail.diff.as_ref(),
            DiffLocation::Stash => self.stash_view.detail.diff.as_ref(),
            DiffLocation::Reflog => self.reflog.detail.diff.as_ref(),
        }
    }

    pub fn diff_state_mut(&mut self, loc: DiffLocation) -> Option<&mut DiffState> {
        match loc {
            DiffLocation::WorkingCopy => self.wc.diff.as_mut(),
            DiffLocation::History => self.history.detail.diff.as_mut(),
            DiffLocation::Stash => self.stash_view.detail.diff.as_mut(),
            DiffLocation::Reflog => self.reflog.detail.diff.as_mut(),
        }
    }

    fn click_diff_line(&mut self, loc: DiffLocation, hunk: usize, line: usize, shift: bool, cx: &mut Context<Self>) {
        let Some(state) = self.diff_state_mut(loc) else { return };
        let Some(kind) = state.diff.hunks.get(hunk).and_then(|h| h.lines.get(line)).map(|l| l.kind) else { return };
        if kind == LineKind::Context {
            return;
        }
        match state.anchor {
            Some((ah, al)) if shift && ah == hunk => {
                let (lo, hi) = if al <= line { (al, line) } else { (line, al) };
                for l in lo..=hi {
                    if state.diff.hunks[hunk].lines[l].kind != LineKind::Context {
                        state.selected.insert((hunk, l));
                    }
                }
            }
            _ => {
                if !state.selected.remove(&(hunk, line)) {
                    state.selected.insert((hunk, line));
                }
                state.anchor = Some((hunk, line));
            }
        }
        cx.notify();
    }

    pub fn clear_diff_selection(&mut self, loc: DiffLocation, cx: &mut Context<Self>) {
        if let Some(s) = self.diff_state_mut(loc) {
            s.selected.clear();
            s.anchor = None;
        }
        cx.notify();
    }

    /// Stages, unstages or discards a hunk (`hunk: Some`) or the selected lines (`hunk: None`).
    pub fn apply_partial(&mut self, mode: DiffMode, hunk: Option<usize>, discard: bool, cx: &mut Context<Self>) {
        let Some(state) = self.wc.diff.as_ref() else { return };
        let selected = state.selected.clone();
        let patch_text = match (mode, discard) {
            (DiffMode::Unstaged, false) => patch::build_patch(&state.diff, |h, l| hunk.map(|x| x == h).unwrap_or_else(|| selected.contains(&(h, l))), false),
            (DiffMode::Unstaged, true) | (DiffMode::Staged, _) => patch::build_patch(&state.diff, |h, l| hunk.map(|x| x == h).unwrap_or_else(|| selected.contains(&(h, l))), true),
            (DiffMode::ReadOnly, _) => None,
        };
        let Some(patch_text) = patch_text else {
            self.toast(ToastKind::Info, "Nothing selected", None, cx);
            return;
        };
        let what = if hunk.is_some() { "hunk" } else { "lines" };
        let (label, cached, reverse) = match (mode, discard) {
            (DiffMode::Unstaged, false) => (format!("Staging {what}"), true, false),
            (DiffMode::Unstaged, true) => (format!("Discarding {what}"), false, true),
            _ => (format!("Unstaging {what}"), true, true),
        };
        let label2 = label.clone();
        self.run_op(
            label,
            move |repo| repo.apply_patch(&patch_text, cached, reverse),
            move |this, result, cx| {
                if let Err(e) = result {
                    this.show_error(&label2, &e, cx);
                }
                if let Some(s) = this.wc.diff.as_mut() {
                    s.selected.clear();
                    s.anchor = None;
                }
            },
            cx,
        );
    }

    pub fn render_diff(&self, loc: DiffLocation, mode: DiffMode, toolbar: Option<AnyElement>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(state) = self.diff_state(loc) else {
            return empty_state(Icon::File, "No file selected", "Select a file to see its changes.", cx);
        };
        let diff = &state.diff;
        let (adds, dels) = diff.stats();
        let (name, dir) = ui::split_path(&diff.path);

        let mut header = div()
            .flex_none()
            .min_w_0()
            .h(px(38.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_color(theme.border_subtle)
            .bg(theme.panel_bg)
            .child(icon(Icon::File).text_color(theme.text_muted))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(6.))
                    .min_w_0()
                    .overflow_hidden()
                    .child(div().font_weight(gpui::FontWeight::SEMIBOLD).whitespace_nowrap().child(name.to_string()))
                    .when(!dir.is_empty(), |d| d.child(div().text_size(px(12.)).text_color(theme.text_faint).whitespace_nowrap().overflow_hidden().text_ellipsis().child(dir.to_string())))
                    .when_some(diff.old_path.clone(), |d, old| d.child(div().text_size(px(12.)).text_color(theme.status_renamed).whitespace_nowrap().child(format!("← {old}")))),
            )
            .child(div().flex_1())
            .when(adds > 0, |d| d.child(div().text_size(px(12.)).text_color(theme.status_added).child(format!("+{adds}"))))
            .when(dels > 0, |d| d.child(div().text_size(px(12.)).text_color(theme.status_deleted).child(format!("−{dels}"))));
        if let Some(tb) = toolbar {
            header = header.child(tb);
        }

        let selection_count = state.selected.len();
        let selection_bar = (mode != DiffMode::ReadOnly && selection_count > 0).then(|| {
            let label = format!("{selection_count} line{} selected", if selection_count == 1 { "" } else { "s" });
            let mut bar = div()
                .flex_none()
                .h(px(36.))
                .px(px(12.))
                .flex()
                .items_center()
                .gap(px(8.))
                .bg(theme.accent_subtle)
                .border_b_1()
                .border_color(theme.border_subtle)
                .child(div().text_size(px(12.)).text_color(theme.text).child(label))
                .child(div().flex_1());
            match mode {
                DiffMode::Unstaged => {
                    bar = bar
                        .child(Button::new("stage-lines", "Stage Lines").small().primary().icon(Icon::Plus).on_click(cx.listener(|this, _, _, cx| this.apply_partial(DiffMode::Unstaged, None, false, cx))))
                        .when(!state.diff.new_file, |b| {
                            b.child(Button::new("discard-lines", "Discard Lines").small().icon(Icon::Undo).on_click(cx.listener(|this, _, window, cx| this.confirm_discard_partial(None, window, cx))))
                        });
                }
                DiffMode::Staged => {
                    bar = bar.child(Button::new("unstage-lines", "Unstage Lines").small().primary().icon(Icon::Minus).on_click(cx.listener(|this, _, _, cx| this.apply_partial(DiffMode::Staged, None, false, cx))));
                }
                DiffMode::ReadOnly => {}
            }
            bar.child(Button::new("clear-sel", "Clear").small().ghost().on_click(cx.listener(move |this, _, _, cx| this.clear_diff_selection(loc, cx))))
        });

        let body: AnyElement = if diff.binary {
            empty_state(Icon::File, "Binary file", "This file is binary and can't be displayed as text.", cx)
        } else if diff.hunks.is_empty() {
            let msg = if diff.old_path.is_some() {
                "File was renamed without changes."
            } else if diff.new_file {
                "Empty file."
            } else if diff.header.iter().any(|h| h.starts_with("old mode")) {
                "File mode changed."
            } else {
                "No changes to display."
            };
            empty_state(Icon::Check, msg, "", cx)
        } else {
            let row_count = state.rows.len();
            let longest = state.longest_row;
            let truncated = state.truncated;
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(
                    uniform_list(
                        "diff-rows",
                        row_count,
                        cx.processor(move |this: &mut RepoView, range: Range<usize>, _window, cx| range.map(|ix| this.render_diff_row(loc, mode, ix, cx)).collect::<Vec<_>>()),
                    )
                    .track_scroll(state.scroll.clone())
                    .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
                    .with_width_from_item(Some(longest))
                    .flex_1()
                    .font_family(mono_font())
                    .text_size(px(12.5))
                    .bg(theme.panel_bg),
                )
                .when(truncated, |d| {
                    d.child(div().flex_none().p(px(8.)).text_size(px(12.)).text_color(theme.warning).bg(theme.surface).child("Diff truncated: this file has too many changed lines to display."))
                })
                .into_any_element()
        };

        div().size_full().flex().flex_col().min_w_0().child(header).children(selection_bar).child(div().flex_1().min_h_0().child(body)).into_any_element()
    }

    fn render_diff_row(&self, loc: DiffLocation, mode: DiffMode, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(state) = self.diff_state(loc) else { return div().into_any_element() };
        let row_h = px(DIFF_ROW_HEIGHT);
        match state.rows[ix] {
            DiffRow::Hunk(hi) => {
                let hunk = &state.diff.hunks[hi];
                let header_text = format!("@@ -{},{} +{},{} @@ {}", hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count, hunk.context());
                let sel_in_hunk = state.selection_in_hunk(hi);
                let mut row = div()
                    .id(("hunk", ix))
                    .w_full()
                    .h(row_h)
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .bg(theme.diff_hunk_bg)
                    .text_color(theme.text_faint)
                    .whitespace_nowrap();
                let small_btn = |id: (&'static str, usize), label: &'static str, cx: &mut Context<Self>| {
                    let theme = cx.theme().clone();
                    div()
                        .id(id)
                        .px(px(7.))
                        .h(px(16.))
                        .flex()
                        .items_center()
                        .rounded(px(3.))
                        .bg(theme.surface)
                        .border_1()
                        .border_color(theme.border)
                        .text_size(px(11.))
                        .font_family(crate::theme::ui_font())
                        .text_color(theme.text)
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.active))
                        .child(label)
                };
                match mode {
                    DiffMode::Unstaged => {
                        row = row
                            .child(small_btn(("stage-hunk", hi), if sel_in_hunk > 0 { "Stage Lines" } else { "Stage Hunk" }, cx).on_click(cx.listener(move |this, _, _, cx| {
                                let has_sel = this.wc.diff.as_ref().map(|s| s.selection_in_hunk(hi) > 0).unwrap_or(false);
                                this.apply_partial(DiffMode::Unstaged, if has_sel { None } else { Some(hi) }, false, cx)
                            })))
                            .when(!state.diff.new_file, |r| r.child(small_btn(("discard-hunk", hi), "Discard Hunk", cx).on_click(cx.listener(move |this, _, window, cx| this.confirm_discard_partial(Some(hi), window, cx)))));
                    }
                    DiffMode::Staged => {
                        row = row.child(small_btn(("unstage-hunk", hi), if sel_in_hunk > 0 { "Unstage Lines" } else { "Unstage Hunk" }, cx).on_click(cx.listener(move |this, _, _, cx| {
                            let has_sel = this.wc.diff.as_ref().map(|s| s.selection_in_hunk(hi) > 0).unwrap_or(false);
                            this.apply_partial(DiffMode::Staged, if has_sel { None } else { Some(hi) }, false, cx)
                        })));
                    }
                    DiffMode::ReadOnly => {}
                }
                row.child(header_text).into_any_element()
            }
            DiffRow::Line(hi, li) => {
                let line = &state.diff.hunks[hi].lines[li];
                let selected = state.selected.contains(&(hi, li));
                let (bg, sign, sign_color, strong) = match (line.kind, state.diff.conflict) {
                    (LineKind::Added, false) => (theme.diff_add_bg, "+", theme.status_added, theme.diff_add_strong),
                    (LineKind::Removed, false) => (theme.diff_del_bg, "−", theme.status_deleted, theme.diff_del_strong),
                    // Conflict view: our side in blue, their side in green.
                    (LineKind::Removed, true) => (theme.info.opacity(0.14), "‹", theme.info, theme.info.opacity(0.3)),
                    (LineKind::Added, true) => (theme.diff_add_bg, "›", theme.status_added, theme.diff_add_strong),
                    (LineKind::Context, _) => (gpui::transparent_black(), " ", theme.text_faint, gpui::transparent_black()),
                };
                let is_marker = state.diff.conflict && ["<<<<<<<", "=======", ">>>>>>>", "|||||||"].iter().any(|m| line.content.starts_with(m));
                let bg = if is_marker { theme.status_conflict.opacity(0.18) } else { bg };
                let text = display_text(&line.content);
                let content: AnyElement = match state.highlights.get(&(hi, li)) {
                    Some(r) if r.end <= text.len() && text.is_char_boundary(r.start) && text.is_char_boundary(r.end) => {
                        StyledText::new(SharedString::from(text)).with_highlights(vec![(r.clone(), HighlightStyle { background_color: Some(strong), ..Default::default() })]).into_any_element()
                    }
                    _ => div().child(if text.is_empty() { SharedString::from(" ") } else { SharedString::from(text) }).into_any_element(),
                };
                let num = |n: Option<u32>| div().w(px(42.)).flex_none().pr(px(6.)).flex().justify_end().text_color(theme.text_faint).child(n.map(|n| n.to_string()).unwrap_or_default());
                let interactive = mode != DiffMode::ReadOnly && line.kind != LineKind::Context;
                let mut row = div()
                    .id(("line", ix))
                    .w_full()
                    .h(row_h)
                    .flex()
                    .items_center()
                    .whitespace_nowrap()
                    .bg(bg)
                    .child(div().flex().flex_none().h_full().items_center().bg(if selected { theme.accent } else { theme.diff_gutter.opacity(0.6) }).child(num(line.old_no)).child(num(line.new_no)))
                    .child(div().w(px(18.)).flex_none().flex().justify_center().text_color(sign_color).child(sign))
                    .child(content)
                    .when(line.no_newline, |d| d.child(div().ml(px(6.)).text_color(theme.text_faint).child("no newline at end of file")));
                if selected {
                    row = row.bg(theme.diff_selected);
                }
                if interactive {
                    row = row.cursor_pointer().hover(|s| s.opacity(0.92)).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                            this.click_diff_line(loc, hi, li, e.modifiers.shift, cx);
                        }),
                    );
                }
                row.into_any_element()
            }
        }
    }

    pub fn confirm_discard_partial(&mut self, hunk: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let what = if hunk.is_some() { "this hunk" } else { "the selected lines" };
        self.confirm(
            "Discard changes?",
            format!("This will permanently discard {what} from your working copy. This cannot be undone."),
            "Discard",
            true,
            move |this, _, cx| this.apply_partial(DiffMode::Unstaged, hunk, true, cx),
            window,
            cx,
        );
    }
}
