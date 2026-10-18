//! Context menus.

use super::icon;
use crate::assets::Icon;
use crate::theme::ActiveTheme;
use gpui::{anchored, deferred, div, prelude::*, px, AnyElement, App, Context, Entity, MouseButton, Pixels, Point, SharedString, Window};
use std::rc::Rc;

pub type Handler = Rc<dyn Fn(&mut Window, &mut App)>;

pub enum MenuEntry {
    Item { label: SharedString, icon: Option<Icon>, detail: Option<SharedString>, disabled: bool, danger: bool, handler: Option<Handler> },
    Header(SharedString),
    Separator,
}

pub struct ContextMenu {
    pub position: Point<Pixels>,
    pub entries: Vec<MenuEntry>,
}

impl ContextMenu {
    pub fn new(position: Point<Pixels>) -> Self {
        Self { position, entries: Vec::new() }
    }

    pub fn item(mut self, label: impl Into<SharedString>, icon: Option<Icon>, handler: Handler) -> Self {
        self.entries.push(MenuEntry::Item { label: label.into(), icon, detail: None, disabled: false, danger: false, handler: Some(handler) });
        self
    }

    pub fn item_if(self, cond: bool, label: impl Into<SharedString>, icon: Option<Icon>, handler: Handler) -> Self {
        if cond {
            self.item(label, icon, handler)
        } else {
            self
        }
    }

    pub fn danger_item(mut self, label: impl Into<SharedString>, icon: Option<Icon>, handler: Handler) -> Self {
        self.entries.push(MenuEntry::Item { label: label.into(), icon, detail: None, disabled: false, danger: true, handler: Some(handler) });
        self
    }

    pub fn header(mut self, label: impl Into<SharedString>) -> Self {
        self.entries.push(MenuEntry::Header(label.into()));
        self
    }

    pub fn separator(mut self) -> Self {
        if !matches!(self.entries.last(), None | Some(MenuEntry::Separator)) {
            self.entries.push(MenuEntry::Separator);
        }
        self
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Renders an open menu. `close` is invoked before the chosen item's handler runs.
pub fn render_menu<V: 'static>(menu: &ContextMenu, cx: &mut Context<V>, close: impl Fn(&mut V, &mut Context<V>) + 'static) -> AnyElement {
    let theme = cx.theme().clone();
    let close = Rc::new(close);
    let view: Entity<V> = cx.entity();
    let mut panel = div()
        .id("context-menu")
        .occlude()
        .min_w(px(220.))
        .max_w(px(420.))
        .py(px(4.))
        .rounded(px(7.))
        .bg(theme.elevated)
        .border_1()
        .border_color(theme.border)
        .shadow_lg()
        .text_size(px(13.))
        .flex()
        .flex_col()
        .on_mouse_down_out({
            let close = close.clone();
            cx.listener(move |this, _, _, cx| {
                close(this, cx);
                cx.notify();
            })
        });
    for (ix, entry) in menu.entries.iter().enumerate() {
        match entry {
            MenuEntry::Separator => {
                panel = panel.child(div().h(px(1.)).mx(px(6.)).my(px(4.)).bg(theme.border_subtle));
            }
            MenuEntry::Header(label) => {
                panel = panel.child(div().px(px(12.)).pt(px(6.)).pb(px(2.)).text_size(px(11.)).text_color(theme.text_faint).child(label.clone()));
            }
            MenuEntry::Item { label, icon: item_icon, detail, disabled, danger, handler } => {
                let fg = if *danger { theme.danger } else { theme.text };
                let mut row = div()
                    .id(ix)
                    .mx(px(4.))
                    .px(px(8.))
                    .h(px(26.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_color(fg)
                    .child(div().w(px(14.)).flex_none().when_some(*item_icon, |d, i| d.child(icon(i).text_color(if *danger { theme.danger } else { theme.text_muted }))))
                    .child(div().flex_1().whitespace_nowrap().overflow_hidden().text_ellipsis().child(label.clone()))
                    .when_some(detail.clone(), |d, det| d.child(div().text_color(theme.text_faint).text_size(px(12.)).child(det)));
                if *disabled {
                    row = row.opacity(0.4);
                } else {
                    let hover = if *danger { theme.danger.opacity(0.15) } else { theme.accent };
                    let hover_fg = if *danger { theme.danger } else { theme.text_on_accent };
                    row = row.cursor_pointer().hover(move |s| s.bg(hover).text_color(hover_fg));
                    if let Some(handler) = handler.clone() {
                        let close = close.clone();
                        let view = view.clone();
                        row = row.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            view.update(cx, |this, cx| {
                                close(this, cx);
                                cx.notify();
                            });
                            handler(window, cx);
                        });
                    }
                }
                panel = panel.child(row);
            }
        }
    }
    deferred(anchored().position(menu.position).snap_to_window_with_margin(px(8.)).child(panel)).with_priority(2).into_any_element()
}
