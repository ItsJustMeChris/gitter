//! Line-by-line blame for a file.

use super::{Nav, RepoView};
use crate::assets::Icon;
use crate::git::{short_oid, BlameLine};
use crate::theme::{mono_font, ActiveTheme};
use crate::ui::{self, empty_state, icon, Button};
use gpui::{div, prelude::*, px, uniform_list, AnyElement, Context, ListHorizontalSizingBehavior, UniformListScrollHandle, Window};
use std::ops::Range;

pub struct BlameState {
    pub path: String,
    pub lines: Vec<BlameLine>,
    pub loading: bool,
    pub error: Option<String>,
    pub scroll: UniformListScrollHandle,
    pub previous_nav: Nav,
    longest: usize,
}

impl RepoView {
    pub fn open_blame(&mut self, path: String, _window: &mut Window, cx: &mut Context<Self>) {
        let previous_nav = if matches!(self.nav, Nav::Blame(_)) { Nav::History } else { self.nav.clone() };
        self.blame = Some(BlameState { path: path.clone(), lines: Vec::new(), loading: true, error: None, scroll: UniformListScrollHandle::new(), previous_nav, longest: 0 });
        self.nav = Nav::Blame(path.clone());
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let p = path.clone();
            let result = cx.background_executor().spawn(async move { repo.blame(&p) }).await;
            this.update(cx, |this, cx| {
                if let Some(state) = this.blame.as_mut().filter(|b| b.path == path) {
                    state.loading = false;
                    match result {
                        Ok(lines) => {
                            state.longest = lines.iter().enumerate().max_by_key(|(_, l)| l.content.len()).map(|(i, _)| i).unwrap_or(0);
                            state.lines = lines;
                        }
                        Err(e) => state.error = Some(e.to_string()),
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn render_blame(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let Some(state) = self.blame.as_ref() else { return div().into_any_element() };
        let header = div()
            .flex_none()
            .h(px(42.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(theme.border_subtle)
            .child(icon(Icon::User).text_color(theme.text_muted))
            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(format!("Blame: {}", state.path)))
            .child(div().flex_1())
            .when(state.loading, |d| d.child(super::spinner(cx)))
            .child(Button::new("close-blame", "Close").small().icon(Icon::Close).on_click(cx.listener(|this, _, window, cx| {
                let back = this.blame.as_ref().map(|b| b.previous_nav.clone()).unwrap_or(Nav::History);
                this.blame = None;
                this.set_nav(back, window, cx);
            })));
        let body: AnyElement = if let Some(err) = &state.error {
            empty_state(Icon::Warning, "Blame failed", err.clone(), cx)
        } else if state.lines.is_empty() && !state.loading {
            empty_state(Icon::File, "Nothing to blame", "This file has no committed lines.", cx)
        } else {
            uniform_list("blame-lines", state.lines.len(), cx.processor(|this: &mut RepoView, range: Range<usize>, _window, cx| range.map(|ix| this.render_blame_row(ix, cx)).collect::<Vec<_>>()))
                .track_scroll(state.scroll.clone())
                .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
                .with_width_from_item(Some(state.longest))
                .size_full()
                .font_family(mono_font())
                .text_size(px(12.5))
                .into_any_element()
        };
        div().size_full().flex().flex_col().child(header).child(div().flex_1().min_h_0().child(body)).into_any_element()
    }

    fn render_blame_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(state) = self.blame.as_ref() else { return div().into_any_element() };
        let line = &state.lines[ix];
        let first_of_block = ix == 0 || state.lines[ix - 1].oid != line.oid;
        let uncommitted = line.oid.chars().all(|c| c == '0');
        let hash = line.oid.bytes().take(6).fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
        let band = theme.graph_color((hash % 10) as u8);
        let oid = line.oid.clone();
        let info = if first_of_block {
            div()
                .flex()
                .gap(px(8.))
                .child(div().w(px(64.)).text_color(if uncommitted { theme.text_faint } else { theme.accent_hover }).child(if uncommitted { "local".to_string() } else { short_oid(&line.oid).to_string() }))
                .child(div().w(px(130.)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(theme.text_muted).font_family(crate::theme::ui_font()).child(line.author.clone()))
                .child(div().w(px(96.)).text_color(theme.text_faint).font_family(crate::theme::ui_font()).child(ui::absolute_date(line.time)))
        } else {
            div().w(px(306.))
        };
        div()
            .id(("blame", ix))
            .h(px(20.))
            .w_full()
            .flex()
            .items_center()
            .whitespace_nowrap()
            .when(first_of_block && ix > 0, |d| d.border_t_1().border_color(theme.border_subtle))
            .child(div().w(px(3.)).h_full().bg(band.opacity(0.7)))
            .child(
                div()
                    .id(("blame-info", ix))
                    .w(px(320.))
                    .flex_none()
                    .px(px(8.))
                    .bg(theme.diff_gutter)
                    .h_full()
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .when(!uncommitted, |d| d.tooltip(ui::tooltip(line.summary.clone())))
                    .child(info)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if !oid.chars().all(|c| c == '0') {
                            this.reveal_commit(&oid, window, cx);
                        }
                    })),
            )
            .child(div().w(px(48.)).flex_none().pr(px(8.)).flex().justify_end().text_color(theme.text_faint).child(line.line_no.to_string()))
            .child(div().pl(px(6.)).child(super::diff_view::display_text(&line.content)))
            .into_any_element()
    }
}
