//! Quick Actions: a fuzzy finder over branches and commands.

use super::dialogs::{Modal, ModalKind};
use super::{Nav, RepoView};
use crate::assets::Icon;
use crate::theme::ActiveTheme;
use crate::ui::text_input::{self, TextInput};
use crate::ui::{icon, kbd};
use gpui::{div, prelude::*, px, AnyElement, Context, Entity, ScrollHandle, SharedString, Window};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteMode {
    All,
    Merge,
    Rebase,
    Checkout,
}

type PaletteFn = Rc<dyn Fn(&mut RepoView, &mut Window, &mut Context<RepoView>)>;

#[derive(Clone)]
pub struct PaletteItem {
    pub label: SharedString,
    pub detail: Option<SharedString>,
    pub icon: Icon,
    pub shortcut: Option<&'static str>,
    pub run: PaletteFn,
}

pub struct PaletteState {
    pub mode: PaletteMode,
    pub query: Entity<TextInput>,
    pub items: Vec<PaletteItem>,
    pub selected: usize,
    pub scroll: ScrollHandle,
}

impl PaletteState {
    pub fn new(mode: PaletteMode, query: Entity<TextInput>) -> Self {
        Self { mode, query, items: Vec::new(), selected: 0, scroll: ScrollHandle::new() }
    }
}

/// Subsequence match score; higher is better. `None` when not a match.
fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let text_l = text.to_lowercase();
    let query_l = query.to_lowercase();
    if let Some(pos) = text_l.find(&query_l) {
        return Some(1000 - pos as i32 * 2 - text.len() as i32);
    }
    let mut score = 0;
    let mut last = None;
    let mut chars = text_l.char_indices();
    for q in query_l.chars() {
        let mut found = false;
        for (i, c) in chars.by_ref() {
            if c == q {
                score += match last {
                    Some(l) if l + 1 == i => 8,
                    _ => 1,
                };
                last = Some(i);
                found = true;
                break;
            }
        }
        if !found {
            return None;
        }
    }
    Some(score - text.len() as i32 / 4)
}

impl RepoView {
    fn palette_items(&self, mode: PaletteMode) -> Vec<PaletteItem> {
        let mut items = Vec::new();
        let item = |label: String, detail: Option<String>, icon: Icon, shortcut: Option<&'static str>, run: PaletteFn| PaletteItem { label: label.into(), detail: detail.map(Into::into), icon, shortcut, run };
        let head = self.snapshot.head.branch.clone();
        match mode {
            PaletteMode::All => {
                let cmd = |label: &str, icon: Icon, shortcut: Option<&'static str>, f: fn(&mut RepoView, &mut Window, &mut Context<RepoView>)| item(label.to_string(), None, icon, shortcut, Rc::new(f));
                items.push(cmd("Fetch", Icon::Fetch, Some("Ctrl+Shift+F"), |this, _, cx| this.fetch(None, cx)));
                items.push(cmd("Pull", Icon::Pull, Some("Ctrl+Shift+L"), |this, _, cx| this.pull(cx)));
                items.push(cmd("Push", Icon::Push, Some("Ctrl+Shift+P"), |this, window, cx| this.push(window, cx)));
                items.push(cmd("New Branch…", Icon::Branch, Some("Ctrl+B"), |this, window, cx| this.open_create_branch(None, window, cx)));
                items.push(cmd("Merge Branch…", Icon::Merge, Some("Ctrl+Shift+M"), |this, window, cx| this.open_palette(PaletteMode::Merge, window, cx)));
                items.push(cmd("Rebase onto Branch…", Icon::Rebase, None, |this, window, cx| this.open_palette(PaletteMode::Rebase, window, cx)));
                items.push(cmd("Stash Changes…", Icon::Stash, Some("Ctrl+Shift+S"), |this, window, cx| this.open_stash_dialog(window, cx)));
                items.push(cmd("Apply Latest Stash", Icon::StashPop, None, |this, _, cx| this.pop_latest_stash(cx)));
                items.push(cmd("Show Working Copy", Icon::WorkingCopy, Some("Ctrl+1"), |this, window, cx| this.set_nav(Nav::WorkingCopy, window, cx)));
                items.push(cmd("Show History", Icon::History, Some("Ctrl+2"), |this, window, cx| this.set_nav(Nav::History, window, cx)));
                items.push(cmd("Show Stashes", Icon::Stash, Some("Ctrl+3"), |this, window, cx| this.set_nav(Nav::Stashes, window, cx)));
                items.push(cmd("Show Reflog", Icon::List, Some("Ctrl+4"), |this, window, cx| this.set_nav(Nav::Reflog, window, cx)));
                items.push(cmd("Stage All Changes", Icon::Plus, None, |this, _, cx| this.stage_all(cx)));
                items.push(cmd("Unstage All Changes", Icon::Minus, None, |this, _, cx| this.unstage_all(cx)));
                items.push(cmd("Add Remote…", Icon::Globe, None, |this, window, cx| this.open_add_remote(window, cx)));
                items.push(cmd("Open in Terminal", Icon::Terminal, Some("Ctrl+Shift+T"), |this, _, cx| this.open_terminal(cx)));
                items.push(cmd("Show in File Manager", Icon::Folder, None, |this, _, cx| this.reveal_in_files(cx)));
                items.push(cmd("Refresh", Icon::Refresh, Some("F5"), |this, _, cx| this.refresh(cx)));
                items.push(cmd("Toggle Light/Dark Theme", Icon::Moon, None, |_, window, cx| window.dispatch_action(Box::new(crate::actions::ToggleTheme), cx)));
                items.push(cmd("Settings…", Icon::Settings, Some("Ctrl+,"), |_, window, cx| window.dispatch_action(Box::new(crate::actions::OpenSettings), cx)));
                for b in &self.snapshot.branches {
                    if Some(&b.name) == head.as_ref() {
                        continue;
                    }
                    let n = b.name.clone();
                    items.push(item(format!("Check Out {}", b.name), Some("local branch".into()), Icon::Branch, None, Rc::new(move |this, _, cx| this.checkout_branch(n.clone(), cx))));
                }
                for r in &self.snapshot.remote_branches {
                    if self.snapshot.branches.iter().any(|b| b.name == r.name) {
                        continue;
                    }
                    let n = r.full_name.clone();
                    items.push(item(format!("Check Out {}", r.full_name), Some("remote branch".into()), Icon::Cloud, None, Rc::new(move |this, _, cx| this.checkout_remote_branch(n.clone(), cx))));
                }
                for t in self.snapshot.tags.iter().take(200) {
                    let oid = t.oid.clone();
                    items.push(item(format!("Show Tag {}", t.name), Some("tag".into()), Icon::Tag, None, Rc::new(move |this, window, cx| this.reveal_commit(&oid, window, cx))));
                }
            }
            PaletteMode::Merge | PaletteMode::Rebase | PaletteMode::Checkout => {
                let names: Vec<(String, Icon, &str)> = self
                    .snapshot
                    .branches
                    .iter()
                    .filter(|b| Some(&b.name) != head.as_ref())
                    .map(|b| (b.name.clone(), Icon::Branch, "local"))
                    .chain(self.snapshot.remote_branches.iter().map(|r| (r.full_name.clone(), Icon::Cloud, "remote")))
                    .collect();
                for (name, ic, kind) in names {
                    let n = name.clone();
                    let run: PaletteFn = match mode {
                        PaletteMode::Merge => Rc::new(move |this, window, cx| this.open_merge_dialog(n.clone(), window, cx)),
                        PaletteMode::Rebase => Rc::new(move |this, _, cx| this.rebase_current_onto(n.clone(), cx)),
                        _ => {
                            if kind == "remote" {
                                Rc::new(move |this, _, cx| this.checkout_remote_branch(n.clone(), cx))
                            } else {
                                Rc::new(move |this, _, cx| this.checkout_branch(n.clone(), cx))
                            }
                        }
                    };
                    items.push(item(name, Some(format!("{kind} branch")), ic, None, run));
                }
            }
        }
        items
    }

    pub fn update_palette(&mut self, cx: &mut Context<Self>) {
        let Some(Modal { kind: ModalKind::Palette(state), .. }) = self.modal.as_ref() else { return };
        let mode = state.mode;
        let query = state.query.read(cx).text().trim().to_string();
        let all = self.palette_items(mode);
        let mut scored: Vec<(i32, usize, PaletteItem)> = all.into_iter().enumerate().filter_map(|(i, it)| fuzzy_score(&query, &it.label).map(|s| (s, i, it))).collect();
        if !query.is_empty() {
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        }
        if let Some(Modal { kind: ModalKind::Palette(state), .. }) = self.modal.as_mut() {
            state.items = scored.into_iter().map(|(_, _, it)| it).take(200).collect();
            state.selected = 0;
            state.scroll.scroll_to_item(0);
        }
        cx.notify();
    }

    fn palette_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(Modal { kind: ModalKind::Palette(state), .. }) = self.modal.as_mut() {
            if state.items.is_empty() {
                return;
            }
            let n = state.items.len() as isize;
            state.selected = ((state.selected as isize + delta).rem_euclid(n)) as usize;
            state.scroll.scroll_to_item(state.selected);
            cx.notify();
        }
    }

    pub fn run_palette_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let run = match self.modal.as_ref() {
            Some(Modal { kind: ModalKind::Palette(state), .. }) => state.items.get(state.selected).map(|i| i.run.clone()),
            _ => None,
        };
        self.close_modal(window, cx);
        if let Some(run) = run {
            run(self, window, cx);
        }
    }

    pub fn render_palette(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(Modal { kind: ModalKind::Palette(state), .. }) = self.modal.as_ref() else { return div().into_any_element() };
        let title = match state.mode {
            PaletteMode::All => "Quick Actions",
            PaletteMode::Merge => "Merge into Current Branch",
            PaletteMode::Rebase => "Rebase Current Branch",
            PaletteMode::Checkout => "Check Out Branch",
        };
        let mut list = div().id("palette-list").max_h(px(380.)).overflow_y_scroll().track_scroll(&state.scroll).py(px(4.)).flex().flex_col();
        if state.items.is_empty() {
            list = list.child(div().p(px(16.)).text_color(theme.text_faint).child("No matches"));
        }
        for (ix, it) in state.items.iter().enumerate() {
            let selected = ix == state.selected;
            list = list.child(
                div()
                    .id(("palette-item", ix))
                    .mx(px(6.))
                    .px(px(10.))
                    .h(px(32.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(theme.accent).text_color(theme.text_on_accent))
                    .when(!selected, |d| d.hover(|s| s.bg(theme.hover)))
                    .child(icon(it.icon).text_color(if selected { theme.text_on_accent } else { theme.text_muted }))
                    .child(div().flex_1().min_w_0().whitespace_nowrap().overflow_hidden().text_ellipsis().child(it.label.clone()))
                    .when_some(it.detail.clone(), |d, det| d.child(div().text_size(px(11.)).text_color(if selected { theme.text_on_accent } else { theme.text_faint }).child(det)))
                    .when_some(it.shortcut, |d, s| d.child(kbd(s, cx)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(Modal { kind: ModalKind::Palette(state), .. }) = this.modal.as_mut() {
                            state.selected = ix;
                        }
                        this.run_palette_selection(window, cx);
                    })),
            );
        }
        div()
            .flex()
            .flex_col()
            .on_action(cx.listener(|this, _: &text_input::Up, _, cx| this.palette_move(-1, cx)))
            .on_action(cx.listener(|this, _: &text_input::Down, _, cx| this.palette_move(1, cx)))
            .child(
                div()
                    .p(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .border_b_1()
                    .border_color(theme.border_subtle)
                    .child(div().text_size(px(11.)).font_weight(gpui::FontWeight::BOLD).text_color(theme.text_faint).child(title.to_uppercase()))
                    .child(state.query.clone()),
            )
            .child(list)
            .into_any_element()
    }
}
