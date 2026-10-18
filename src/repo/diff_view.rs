//! Rendering of unified diffs, with hunk and line staging.

use super::{RepoView, ToastKind};
use crate::assets::Icon;
use crate::git::images::{ImageVersion, ImageVersions};
use crate::git::{patch, DiffLine, FileDiff, LineKind};
use crate::settings::Settings;
use crate::theme::{mono_font, ActiveTheme, Theme};
use crate::ui::{self, empty_state, icon, Button};
use gpui::{
    canvas, div, fill, font, image_cache, img, point, prelude::*, px, retain_all, size, uniform_list, AnyElement, Bounds, Context, DispatchPhase, ElementId, HighlightStyle,
    HitboxBehavior, Hsla, ListHorizontalSizingBehavior, MouseButton, ScrollWheelEvent, SharedString, StyledText, UniformListScrollHandle, Window,
};
use std::collections::{BTreeSet, HashMap};
use std::ops::Range;

const MAX_ROWS: usize = 25_000;
pub const DIFF_ROW_HEIGHT: f32 = 20.;
const DIFF_FONT_SIZE: f32 = 12.5;
/// Width of one line-number column, and of the +/- sign column.
const NUMBER_WIDTH: f32 = 42.;
const SIGN_WIDTH: f32 = 18.;

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

/// A row of the side-by-side view: a hunk header, or old and new lines facing
/// each other. Context lines appear on both sides; in a block of changes the
/// removed lines pair up with the added ones and the shorter side is padded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitRow {
    Hunk(usize),
    Pair { hunk: usize, old: Option<usize>, new: Option<usize> },
}

pub struct DiffState {
    pub diff: FileDiff,
    pub rows: Vec<DiffRow>,
    pub longest_row: usize,
    pub split_rows: Vec<SplitRow>,
    /// Display length of the longest line, bounding the side-by-side horizontal scroll.
    pub split_longest: usize,
    pub split_truncated: bool,
    /// Horizontal scroll of both side-by-side columns, in pixels.
    pub split_offset: f32,
    pub scroll: UniformListScrollHandle,
    pub selected: BTreeSet<(usize, usize)>,
    pub anchor: Option<(usize, usize)>,
    pub highlights: HashMap<(usize, usize), Range<usize>>,
    pub truncated: bool,
    /// Show the text diff of an image that has one (SVG) instead of the pictures.
    pub prefer_text: bool,
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
        let (split_rows, split_longest, split_truncated) = build_split_rows(&diff);
        DiffState {
            diff,
            rows,
            longest_row: longest.0,
            split_rows,
            split_longest,
            split_truncated,
            split_offset: 0.,
            scroll: UniformListScrollHandle::new(),
            selected: BTreeSet::new(),
            anchor: None,
            highlights,
            truncated,
            prefer_text: false,
        }
    }

    /// Replaces the diff, keeping the scroll position when it's the same file.
    pub fn update(&mut self, diff: FileDiff) {
        let same = self.diff == diff;
        if same {
            return;
        }
        let scroll = self.scroll.clone();
        let (split_offset, prefer_text) = (self.split_offset, self.prefer_text);
        let keep_scroll = self.diff.path == diff.path;
        *self = DiffState::new(diff);
        if keep_scroll {
            self.scroll = scroll;
            self.split_offset = split_offset;
            self.prefer_text = prefer_text;
        }
    }

    fn selection_in_hunk(&self, hunk: usize) -> usize {
        self.selected.range((hunk, 0)..(hunk + 1, 0)).count()
    }

    fn has_text(&self) -> bool {
        !self.diff.binary && !self.diff.hunks.is_empty()
    }

    /// The images to show instead of the text diff, if any.
    fn shown_images(&self) -> Option<&ImageVersions> {
        self.diff.images.as_ref().filter(|_| !(self.prefer_text && self.has_text()))
    }
}

fn build_split_rows(diff: &FileDiff) -> (Vec<SplitRow>, usize, bool) {
    let mut rows = Vec::new();
    let mut longest = 0;
    for (hi, hunk) in diff.hunks.iter().enumerate() {
        let lines = &hunk.lines;
        longest = lines.iter().map(|l| l.content.len().min(3000) + 3 * l.content.matches('\t').count()).fold(longest, usize::max);
        rows.push(SplitRow::Hunk(hi));
        let mut i = 0;
        while i < lines.len() {
            if rows.len() >= MAX_ROWS {
                return (rows, longest, true);
            }
            if lines[i].kind == LineKind::Context {
                rows.push(SplitRow::Pair { hunk: hi, old: Some(i), new: Some(i) });
                i += 1;
                continue;
            }
            let start = i;
            while i < lines.len() && lines[i].kind != LineKind::Context {
                i += 1;
            }
            let old: Vec<usize> = (start..i).filter(|&l| lines[l].kind == LineKind::Removed).collect();
            let new: Vec<usize> = (start..i).filter(|&l| lines[l].kind == LineKind::Added).collect();
            for k in 0..old.len().max(new.len()) {
                rows.push(SplitRow::Pair { hunk: hi, old: old.get(k).copied(), new: new.get(k).copied() });
            }
        }
    }
    let truncated = rows.len() > MAX_ROWS;
    rows.truncate(MAX_ROWS);
    (rows, longest, truncated)
}

/// Background, sign, sign color and word-highlight color of a diff line.
fn line_style(line: &DiffLine, conflict: bool, theme: &Theme) -> (Hsla, &'static str, Hsla, Hsla) {
    let (bg, sign, sign_color, strong) = match (line.kind, conflict) {
        (LineKind::Added, false) => (theme.diff_add_bg, "+", theme.status_added, theme.diff_add_strong),
        (LineKind::Removed, false) => (theme.diff_del_bg, "−", theme.status_deleted, theme.diff_del_strong),
        // Conflict view: our side in blue, their side in green.
        (LineKind::Removed, true) => (theme.info.opacity(0.14), "‹", theme.info, theme.info.opacity(0.3)),
        (LineKind::Added, true) => (theme.diff_add_bg, "›", theme.status_added, theme.diff_add_strong),
        (LineKind::Context, _) => (gpui::transparent_black(), " ", theme.text_faint, gpui::transparent_black()),
    };
    let is_marker = conflict && ["<<<<<<<", "=======", ">>>>>>>", "|||||||"].iter().any(|m| line.content.starts_with(m));
    let bg = if is_marker { theme.status_conflict.opacity(0.18) } else { bg };
    (bg, sign, sign_color, strong)
}

/// The text of a diff line, with its changed words highlighted.
fn line_content(state: &DiffState, hi: usize, li: usize, strong: Hsla) -> AnyElement {
    let text = display_text(&state.diff.hunks[hi].lines[li].content);
    match state.highlights.get(&(hi, li)) {
        Some(r) if r.end <= text.len() && text.is_char_boundary(r.start) && text.is_char_boundary(r.end) => {
            StyledText::new(SharedString::from(text)).with_highlights(vec![(r.clone(), HighlightStyle { background_color: Some(strong), ..Default::default() })]).into_any_element()
        }
        _ => div().child(if text.is_empty() { SharedString::from(" ") } else { SharedString::from(text) }).into_any_element(),
    }
}

fn line_number(n: Option<u32>, theme: &Theme) -> gpui::Div {
    div().w(px(NUMBER_WIDTH)).flex_none().pr(px(6.)).flex().justify_end().text_color(theme.text_faint).child(n.map(|n| n.to_string()).unwrap_or_default())
}

/// A transparency grid filling its parent, painted behind image previews.
fn checkerboard(base: Hsla, square: Hsla) -> impl IntoElement {
    const CELL: f32 = 8.;
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            window.paint_quad(fill(bounds, base));
            let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            for row in 0..(h / CELL).ceil() as usize {
                for col in (row % 2..(w / CELL).ceil() as usize).step_by(2) {
                    let (x, y) = (col as f32 * CELL, row as f32 * CELL);
                    let cell = Bounds::new(bounds.origin + point(px(x), px(y)), size(px(CELL.min(w - x)), px(CELL.min(h - y))));
                    window.paint_quad(fill(cell, square));
                }
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

fn format_bytes(n: usize) -> String {
    match n {
        n if n < 1024 => format!("{n} B"),
        n if n < 1024 * 1024 => format!("{:.1} KB", n as f64 / 1024.),
        n => format!("{:.1} MB", n as f64 / (1024. * 1024.)),
    }
}

fn image_meta(v: &ImageVersion) -> String {
    match v.size {
        Some((w, h)) => format!("{} · {w} × {h} · {}", v.format, format_bytes(v.len)),
        None => format!("{} · {}", v.format, format_bytes(v.len)),
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
        let split = cx.global::<Settings>().diff_side_by_side;
        let Some(state) = self.diff_state_mut(loc) else { return };
        let Some(kind) = state.diff.hunks.get(hunk).and_then(|h| h.lines.get(line)).map(|l| l.kind) else { return };
        if kind == LineKind::Context {
            return;
        }
        match state.anchor {
            Some((ah, al)) if shift && ah == hunk => {
                let lines = &state.diff.hunks[hunk].lines;
                // A range dragged down one column of the side-by-side view stays in that column.
                let one_side = split && lines[al].kind == kind;
                let (lo, hi) = if al <= line { (al, line) } else { (line, al) };
                for l in lo..=hi {
                    if lines[l].kind != LineKind::Context && (!one_side || lines[l].kind == kind) {
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

    pub fn set_diff_side_by_side(&mut self, on: bool, cx: &mut Context<Self>) {
        cx.update_global::<Settings, _>(|s, _| {
            s.diff_side_by_side = on;
            s.save();
        });
        cx.notify();
    }

    fn set_diff_prefer_text(&mut self, loc: DiffLocation, on: bool, cx: &mut Context<Self>) {
        if let Some(s) = self.diff_state_mut(loc) {
            s.prefer_text = on;
        }
        cx.notify();
    }

    pub fn render_diff(&self, loc: DiffLocation, mode: DiffMode, toolbar: Option<AnyElement>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(state) = self.diff_state(loc) else {
            return empty_state(Icon::File, "No file selected", "Select a file to see its changes.", cx);
        };
        let split = cx.global::<Settings>().diff_side_by_side;
        let diff = &state.diff;
        let (adds, dels) = diff.stats();
        let (name, dir) = ui::split_path(&diff.path);
        let images = state.shown_images();
        let show_text = images.is_none() && state.has_text();

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
            .child(icon(if diff.images.is_some() { Icon::Image } else { Icon::File }).text_color(theme.text_muted))
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
        // SVGs have both a picture and a text diff.
        if diff.images.is_some() && state.has_text() {
            let prefer_text = state.prefer_text;
            header = header.child(
                div()
                    .flex()
                    .gap(px(2.))
                    .child(Button::new("diff-show-image", "Image").small().selected(!prefer_text).on_click(cx.listener(move |this, _, _, cx| this.set_diff_prefer_text(loc, false, cx))))
                    .child(Button::new("diff-show-text", "Text").small().selected(prefer_text).on_click(cx.listener(move |this, _, _, cx| this.set_diff_prefer_text(loc, true, cx)))),
            );
        }
        if show_text {
            header = header.child(
                div()
                    .flex()
                    .gap(px(2.))
                    .child(Button::icon_only("diff-inline", Icon::DiffInline).small().selected(!split).tooltip("Inline diff").on_click(cx.listener(|this, _, _, cx| this.set_diff_side_by_side(false, cx))))
                    .child(Button::icon_only("diff-split", Icon::DiffSplit).small().selected(split).tooltip("Side-by-side diff").on_click(cx.listener(|this, _, _, cx| this.set_diff_side_by_side(true, cx)))),
            );
        }
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

        let body: AnyElement = if let Some(images) = images {
            self.render_image_diff(images, cx)
        } else if diff.binary {
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
            let (row_count, truncated) = if split { (state.split_rows.len(), state.split_truncated) } else { (state.rows.len(), state.truncated) };
            let list = uniform_list(
                "diff-rows",
                row_count,
                cx.processor(move |this: &mut RepoView, range: Range<usize>, _window, cx| {
                    range.map(|ix| if split { this.render_split_row(loc, mode, ix, cx) } else { this.render_diff_row(loc, mode, ix, cx) }).collect::<Vec<_>>()
                }),
            )
            .track_scroll(state.scroll.clone())
            .flex_1()
            .font_family(mono_font())
            .text_size(px(DIFF_FONT_SIZE))
            .bg(theme.panel_bg);
            // Inline rows scroll sideways as a whole; side-by-side columns share an offset of their own.
            let list = if split { list } else { list.with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained).with_width_from_item(Some(state.longest_row)) };
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(div().flex_1().min_h_0().flex().flex_col().relative().child(list).when(split, |d| d.child(self.split_scroll_listener(loc, state.split_longest, cx))))
                .when(truncated, |d| {
                    d.child(div().flex_none().p(px(8.)).text_size(px(12.)).text_color(theme.warning).bg(theme.surface).child("Diff truncated: this file has too many changed lines to display."))
                })
                .into_any_element()
        };

        div().size_full().flex().flex_col().min_w_0().child(header).children(selection_bar).child(div().flex_1().min_h_0().child(body)).into_any_element()
    }

    /// An overlay that turns horizontal wheel movement (shift+wheel, tilt
    /// wheels, trackpads) over the side-by-side view into a shared column
    /// offset. It listens in the capture phase so the list doesn't turn the
    /// movement into vertical scrolling first.
    fn split_scroll_listener(&self, loc: DiffLocation, longest: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |bounds, hitbox, window, _| {
                window.on_mouse_event(move |e: &ScrollWheelEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture || !hitbox.should_handle_scroll(window) {
                        return;
                    }
                    let delta = e.delta.pixel_delta(window.line_height());
                    if delta.x.abs() <= delta.y.abs() {
                        return;
                    }
                    let text = window.text_system();
                    let char_width = text.em_advance(text.resolve_font(&font(mono_font())), px(DIFF_FONT_SIZE)).map(f32::from).unwrap_or(7.);
                    let column = (f32::from(bounds.size.width) - 1.) / 2. - NUMBER_WIDTH - SIGN_WIDTH;
                    let max = (longest as f32 * char_width + 24. - column).max(0.);
                    let _ = view.update(cx, |this, cx| {
                        if let Some(s) = this.diff_state_mut(loc) {
                            let offset = (s.split_offset - f32::from(delta.x)).clamp(0., max);
                            if offset != s.split_offset {
                                s.split_offset = offset;
                                cx.notify();
                            }
                        }
                    });
                    cx.stop_propagation();
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }

    fn render_hunk_row(&self, state: &DiffState, mode: DiffMode, ix: usize, hi: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let hunk = &state.diff.hunks[hi];
        let header_text = format!("@@ -{},{} +{},{} @@ {}", hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count, hunk.context());
        let sel_in_hunk = state.selection_in_hunk(hi);
        let mut row = div()
            .id(("hunk", ix))
            .w_full()
            .h(px(DIFF_ROW_HEIGHT))
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

    fn render_diff_row(&self, loc: DiffLocation, mode: DiffMode, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(state) = self.diff_state(loc) else { return div().into_any_element() };
        let (hi, li) = match state.rows[ix] {
            DiffRow::Hunk(hi) => return self.render_hunk_row(state, mode, ix, hi, cx),
            DiffRow::Line(hi, li) => (hi, li),
        };
        let line = &state.diff.hunks[hi].lines[li];
        let selected = state.selected.contains(&(hi, li));
        let (bg, sign, sign_color, strong) = line_style(line, state.diff.conflict, &theme);
        let interactive = mode != DiffMode::ReadOnly && line.kind != LineKind::Context;
        let mut row = div()
            .id(("line", ix))
            .w_full()
            .h(px(DIFF_ROW_HEIGHT))
            .flex()
            .items_center()
            .whitespace_nowrap()
            .bg(bg)
            .child(div().flex().flex_none().h_full().items_center().bg(if selected { theme.accent } else { theme.diff_gutter.opacity(0.6) }).child(line_number(line.old_no, &theme)).child(line_number(line.new_no, &theme)))
            .child(div().w(px(SIGN_WIDTH)).flex_none().flex().justify_center().text_color(sign_color).child(sign))
            .child(line_content(state, hi, li, strong))
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

    fn render_split_row(&self, loc: DiffLocation, mode: DiffMode, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(state) = self.diff_state(loc) else { return div().into_any_element() };
        let (hi, old, new) = match state.split_rows[ix] {
            SplitRow::Hunk(hi) => return self.render_hunk_row(state, mode, ix, hi, cx),
            SplitRow::Pair { hunk, old, new } => (hunk, old, new),
        };
        div()
            .id(("split", ix))
            .w_full()
            .h(px(DIFF_ROW_HEIGHT))
            .flex()
            .whitespace_nowrap()
            .child(self.render_split_cell(state, loc, mode, hi, old, false, cx))
            .child(div().flex_none().w(px(1.)).h_full().bg(theme.border_subtle))
            .child(self.render_split_cell(state, loc, mode, hi, new, true, cx))
            .into_any_element()
    }

    /// One side of a side-by-side row; `None` is the filler facing an unpaired change.
    #[allow(clippy::too_many_arguments)]
    fn render_split_cell(&self, state: &DiffState, loc: DiffLocation, mode: DiffMode, hi: usize, li: Option<usize>, new_side: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let cell = div().id(if new_side { "new" } else { "old" }).flex_1().min_w_0().h_full().flex().items_center().overflow_hidden();
        let Some(li) = li else {
            return cell.bg(theme.diff_gutter.opacity(0.35)).into_any_element();
        };
        let line = &state.diff.hunks[hi].lines[li];
        let selected = line.kind != LineKind::Context && state.selected.contains(&(hi, li));
        let (bg, sign, sign_color, strong) = line_style(line, state.diff.conflict, &theme);
        let interactive = mode != DiffMode::ReadOnly && line.kind != LineKind::Context;
        let text = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .flex()
            .items_center()
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .ml(px(-state.split_offset))
                    .child(line_content(state, hi, li, strong))
                    .when(line.no_newline, |d| d.child(div().ml(px(6.)).text_color(theme.text_faint).child("no newline at end of file"))),
            );
        let mut cell = cell
            .bg(if selected { theme.diff_selected } else { bg })
            .child(div().flex_none().h_full().flex().items_center().bg(if selected { theme.accent } else { theme.diff_gutter.opacity(0.6) }).child(line_number(if new_side { line.new_no } else { line.old_no }, &theme)))
            .child(div().w(px(SIGN_WIDTH)).flex_none().flex().justify_center().text_color(sign_color).child(sign))
            .child(text);
        if interactive {
            cell = cell.cursor_pointer().hover(|s| s.opacity(0.92)).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                    this.click_diff_line(loc, hi, li, e.modifiers.shift, cx);
                }),
            );
        }
        cell.into_any_element()
    }

    fn render_image_diff(&self, images: &ImageVersions, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let both = images.old.is_some() && images.new.is_some();
        let panel = |v: &ImageVersion, label: &'static str, color: Hsla| {
            let fallback_color = theme.text_faint;
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(12.))
                        .child(ui::badge(label, color.opacity(0.16), color))
                        .child(div().text_color(theme.text_muted).whitespace_nowrap().child(image_meta(v))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .relative()
                        .p(px(12.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(theme.border_subtle)
                        .overflow_hidden()
                        .child(checkerboard(theme.surface, theme.text.opacity(0.07)))
                        .child(img(v.path.clone()).max_w_full().max_h_full().with_fallback(move || div().text_size(px(12.)).text_color(fallback_color).child("Can't display this image").into_any_element())),
                )
        };
        // Images of the same name share a cache that is dropped when the preview goes away.
        let cache_id = ElementId::Name(format!("diff-images:{}", images.new.as_ref().or(images.old.as_ref()).map(|v| v.path.display().to_string()).unwrap_or_default()).into());
        image_cache(retain_all(cache_id))
            .size_full()
            .flex()
            .gap(px(12.))
            .p(px(12.))
            .bg(theme.panel_bg)
            .children(images.old.as_ref().map(|v| panel(v, if both { "Before" } else { "Deleted" }, theme.status_deleted)))
            .children(images.new.as_ref().map(|v| panel(v, if both { "After" } else { "Added" }, theme.status_added)))
            .into_any_element()
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
