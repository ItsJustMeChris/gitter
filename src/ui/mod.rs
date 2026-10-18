//! Reusable widgets.

pub mod menu;
pub mod text_input;

use crate::assets::Icon;
use crate::theme::ActiveTheme;
use gpui::{
    div, prelude::*, px, svg, AnyElement, AnyView, App, ClickEvent, Context, ElementId, Hsla, IntoElement, ParentElement, Pixels, RenderOnce, SharedString, Styled, Svg,
    Window,
};

pub fn icon(icon: Icon) -> IconElement {
    IconElement { svg: svg().path(icon.path()).size(px(14.)).flex_none() }
}

pub fn icon_sized(icon: Icon, size: Pixels) -> IconElement {
    IconElement { svg: svg().path(icon.path()).size(size).flex_none() }
}

/// An SVG icon that, unlike GPUI's `svg()`, falls back to the inherited
/// text color when no color is set (like CSS `currentColor`).
pub struct IconElement {
    svg: Svg,
}

impl gpui::Styled for IconElement {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        self.svg.style()
    }
}

impl IntoElement for IconElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for IconElement {
    type RequestLayoutState = <Svg as gpui::Element>::RequestLayoutState;
    type PrepaintState = <Svg as gpui::Element>::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        gpui::Element::id(&self.svg)
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        gpui::Element::source_location(&self.svg)
    }

    fn request_layout(&mut self, id: Option<&gpui::GlobalElementId>, inspector_id: Option<&gpui::InspectorElementId>, window: &mut Window, cx: &mut App) -> (gpui::LayoutId, Self::RequestLayoutState) {
        self.svg.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.svg.prepaint(id, inspector_id, bounds, state, window, cx)
    }

    fn paint(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let inherited = window.text_style().color;
        let text = self.svg.style().text.get_or_insert_with(Default::default);
        if text.color.is_none() {
            text.color = Some(inherited);
        }
        self.svg.paint(id, inspector_id, bounds, state, prepaint, window, cx)
    }
}

// -------------------------------------------------------------------------
// Tooltip
// -------------------------------------------------------------------------

pub struct Tooltip {
    text: SharedString,
    meta: Option<SharedString>,
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .pl(px(10.))
            .pt(px(14.))
            .child(
                div()
                    .px(px(8.))
                    .py(px(4.))
                    .max_w(px(420.))
                    .rounded(px(5.))
                    .bg(theme.elevated)
                    .border_1()
                    .border_color(theme.border)
                    .shadow_md()
                    .text_size(px(12.))
                    .text_color(theme.text)
                    .child(self.text.clone())
                    .when_some(self.meta.clone(), |d, meta| d.child(div().text_color(theme.text_muted).child(meta))),
            )
    }
}

pub fn tooltip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
    let text = text.into();
    move |_, cx| cx.new(|_| Tooltip { text: text.clone(), meta: None }).into()
}

pub fn tooltip_with_meta(text: impl Into<SharedString>, meta: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
    let text = text.into();
    let meta = meta.into();
    move |_, cx| cx.new(|_| Tooltip { text: text.clone(), meta: Some(meta.clone()) }).into()
}

// -------------------------------------------------------------------------
// Button
// -------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: Option<SharedString>,
    icon: Option<Icon>,
    style: ButtonStyle,
    disabled: bool,
    selected: bool,
    small: bool,
    tooltip: Option<SharedString>,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: Some(label.into()),
            icon: None,
            style: ButtonStyle::Secondary,
            disabled: false,
            selected: false,
            small: false,
            tooltip: None,
            on_click: None,
        }
    }

    pub fn icon_only(id: impl Into<ElementId>, icon: Icon) -> Self {
        Self { label: None, icon: Some(icon), style: ButtonStyle::Ghost, ..Self::new(id, "") }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn primary(self) -> Self {
        self.style(ButtonStyle::Primary)
    }

    pub fn ghost(self) -> Self {
        self.style(ButtonStyle::Ghost)
    }

    pub fn danger(self) -> Self {
        self.style(ButtonStyle::Danger)
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }

    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme().clone();
        let (bg, hover_bg, fg, border) = match self.style {
            _ if self.selected => (theme.accent_subtle, theme.accent_subtle, theme.text, theme.accent),
            ButtonStyle::Primary => (theme.accent, theme.accent_hover, theme.text_on_accent, theme.accent),
            ButtonStyle::Secondary => (theme.surface, theme.active, theme.text, theme.border),
            ButtonStyle::Ghost => (gpui::transparent_black(), theme.hover, theme.text_muted, gpui::transparent_black()),
            ButtonStyle::Danger => (theme.danger, theme.danger_hover, theme.text_on_accent, theme.danger),
        };
        let icon_only = self.label.is_none();
        let height = if self.small { px(22.) } else { px(28.) };
        let mut el = div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .h(height)
            .rounded(px(5.))
            .text_size(if self.small { px(12.) } else { px(13.) })
            .text_color(fg)
            .bg(bg)
            .border_1()
            .border_color(border)
            .when(icon_only, |d| d.w(height))
            .when(!icon_only, |d| d.px(if self.small { px(8.) } else { px(12.) }))
            .when_some(self.icon, |d, i| d.child(icon(i).text_color(fg)))
            .when_some(self.label, |d, l| d.child(div().whitespace_nowrap().child(l)));
        if self.disabled {
            el = el.opacity(0.45).cursor_default();
        } else {
            let hover_fg = if self.style == ButtonStyle::Ghost { theme.text } else { fg };
            el = el.cursor_pointer().hover(move |s| s.bg(hover_bg).text_color(hover_fg)).active(|s| s.opacity(0.85));
            if let Some(handler) = self.on_click {
                el = el.on_click(move |e, w, cx| {
                    cx.stop_propagation();
                    handler(e, w, cx)
                });
            }
        }
        if let Some(t) = self.tooltip {
            el = el.tooltip(tooltip(t));
        }
        el
    }
}

// -------------------------------------------------------------------------
// Checkbox
// -------------------------------------------------------------------------

#[derive(IntoElement)]
pub struct Checkbox {
    id: ElementId,
    checked: bool,
    label: Option<SharedString>,
    disabled: bool,
    on_toggle: Option<Box<dyn Fn(&bool, &mut Window, &mut App) + 'static>>,
}

impl Checkbox {
    pub fn new(id: impl Into<ElementId>, checked: bool) -> Self {
        Self { id: id.into(), checked, label: None, disabled: false, on_toggle: None }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn on_toggle(mut self, f: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Box::new(f));
        self
    }
}

impl RenderOnce for Checkbox {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme().clone();
        let checked = self.checked;
        let box_el = div()
            .size(px(15.))
            .flex_none()
            .rounded(px(3.))
            .border_1()
            .flex()
            .items_center()
            .justify_center()
            .border_color(if checked { theme.accent } else { theme.border })
            .bg(if checked { theme.accent } else { theme.input_bg })
            .when(checked, |d| d.child(icon_sized(Icon::Check, px(11.)).text_color(theme.text_on_accent)));
        let mut el = div()
            .id(self.id)
            .flex()
            .items_center()
            .gap(px(7.))
            .text_size(px(13.))
            .text_color(theme.text)
            .child(box_el)
            .when_some(self.label, |d, l| d.child(l));
        if self.disabled {
            el = el.opacity(0.5);
        } else {
            el = el.cursor_pointer();
            if let Some(f) = self.on_toggle {
                el = el.on_click(move |_, w, cx| {
                    cx.stop_propagation();
                    f(&!checked, w, cx)
                });
            }
        }
        el
    }
}

// -------------------------------------------------------------------------
// Small pieces
// -------------------------------------------------------------------------

pub fn badge(text: impl Into<SharedString>, bg: Hsla, fg: Hsla) -> impl IntoElement {
    div()
        .flex_none()
        .px(px(6.))
        .h(px(17.))
        .flex()
        .items_center()
        .rounded(px(9.))
        .bg(bg)
        .text_color(fg)
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(text.into())
}

pub fn kbd(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .px(px(5.))
        .py(px(1.))
        .rounded(px(3.))
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .text_size(px(11.))
        .text_color(theme.text_muted)
        .child(text.into())
}

pub fn section_label(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    div()
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(cx.theme().text_faint)
        .child(text.into())
}

pub fn empty_state(icon_kind: Icon, title: impl Into<SharedString>, detail: impl Into<SharedString>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .text_color(theme.text_faint)
        .child(icon_sized(icon_kind, px(36.)).text_color(theme.text_faint))
        .child(div().text_size(px(14.)).text_color(theme.text_muted).child(title.into()))
        .child(div().text_size(px(12.)).max_w(px(360.)).text_center().child(detail.into()))
        .into_any_element()
}

pub fn status_color(status: crate::git::FileStatus, cx: &App) -> Hsla {
    use crate::git::FileStatus::*;
    let t = cx.theme();
    match status {
        Added => t.status_added,
        Modified | TypeChanged | Unknown => t.status_modified,
        Deleted => t.status_deleted,
        Renamed | Copied => t.status_renamed,
        Untracked => t.status_untracked,
        Conflicted => t.status_conflict,
    }
}

pub fn status_badge(status: crate::git::FileStatus, cx: &App) -> impl IntoElement {
    let color = status_color(status, cx);
    div()
        .flex_none()
        .size(px(16.))
        .rounded(px(3.))
        .flex()
        .items_center()
        .justify_center()
        .bg(color.opacity(0.18))
        .text_color(color)
        .text_size(px(10.))
        .font_weight(gpui::FontWeight::BOLD)
        .child(status.letter())
}

/// Vertical or horizontal 1px divider with a wider invisible grab area.
pub fn splitter_handle(id: impl Into<ElementId>, vertical: bool, cx: &App) -> gpui::Stateful<gpui::Div> {
    let theme = cx.theme();
    let line = div().bg(theme.border_subtle);
    let grab = div().id(id).flex_none().flex().justify_center().items_center();
    if vertical {
        grab.w(px(5.)).h_full().mx(px(-2.)).cursor_col_resize().child(line.w(px(1.)).h_full())
    } else {
        grab.h(px(5.)).w_full().my(px(-2.)).cursor_row_resize().child(line.h(px(1.)).w_full())
    }
}

/// Marker types for splitter drags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplitDrag(pub u8);

pub struct DragGhost;

impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// Formats a unix timestamp relative to now ("5 min ago") or as a date.
pub fn relative_time(ts: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let d = now - ts;
    if d < 0 {
        return absolute_time(ts);
    }
    match d {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86399 => {
            let h = d / 3600;
            format!("{h} hour{} ago", if h == 1 { "" } else { "s" })
        }
        86400..=604799 => {
            let days = d / 86400;
            format!("{days} day{} ago", if days == 1 { "" } else { "s" })
        }
        _ => absolute_date(ts),
    }
}

pub fn absolute_date(ts: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_opt(ts, 0) {
        chrono::LocalResult::Single(t) => t.format("%b %-d, %Y").to_string(),
        _ => String::new(),
    }
}

pub fn absolute_time(ts: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_opt(ts, 0) {
        chrono::LocalResult::Single(t) => t.format("%a, %b %-d %Y, %H:%M").to_string(),
        _ => String::new(),
    }
}

/// Up to two initials for an author avatar.
pub fn initials(name: &str) -> String {
    let mut parts = name.split_whitespace().filter(|p| !p.is_empty());
    let first = parts.next().and_then(|p| p.chars().next());
    let last = parts.last().and_then(|p| p.chars().next());
    match (first, last) {
        (Some(a), Some(b)) => format!("{}{}", a.to_uppercase(), b.to_uppercase()),
        (Some(a), None) => a.to_uppercase().to_string(),
        _ => "?".into(),
    }
}

pub fn avatar(name: &str, size: Pixels, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let hash = name.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    let color = theme.graph_color((hash % 10) as u8);
    div()
        .flex_none()
        .size(size)
        .rounded_full()
        .bg(color.opacity(0.85))
        .flex()
        .items_center()
        .justify_center()
        .text_size(size * 0.42)
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(gpui::white())
        .child(initials(name))
}

pub fn split_path(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(i) => (&path[i + 1..], &path[..i]),
        None => (path, ""),
    }
}
