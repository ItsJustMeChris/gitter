//! The main toolbar.

use super::palette::PaletteMode;
use super::RepoView;
use crate::actions::OpenSettings;
use crate::assets::Icon;
use crate::theme::ActiveTheme;
use crate::ui::{icon, icon_sized, kbd, tooltip_with_meta};
use gpui::{div, prelude::*, px, AnyElement, ClickEvent, Context, SharedString, Window};

impl RepoView {
    fn tool_button(
        &self,
        id: &'static str,
        label: &'static str,
        tool_icon: Icon,
        hint: &'static str,
        count: Option<u32>,
        disabled: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let mut el = div()
            .id(id)
            .relative()
            .flex_none()
            .w(px(58.))
            .h(px(46.))
            .rounded(px(6.))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(3.))
            .text_color(theme.text_muted)
            .child(icon_sized(tool_icon, px(18.)))
            .child(div().text_size(px(11.)).child(label))
            .when_some(count.filter(|c| *c > 0), |d, c| {
                d.child(
                    div()
                        .absolute()
                        .top(px(2.))
                        .right(px(8.))
                        .min_w(px(16.))
                        .h(px(15.))
                        .px(px(4.))
                        .rounded(px(8.))
                        .bg(theme.accent)
                        .text_color(theme.text_on_accent)
                        .text_size(px(10.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(c.to_string()),
                )
            })
            .tooltip(tooltip_with_meta(label, hint));
        if disabled {
            el = el.opacity(0.4);
        } else {
            el = el.cursor_pointer().hover(|s| s.bg(theme.hover).text_color(theme.text)).active(|s| s.bg(theme.active)).on_click(cx.listener(move |this, _: &ClickEvent, window, cx| on_click(this, window, cx)));
        }
        el.into_any_element()
    }

    pub fn render_toolbar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let busy = self.is_busy();
        let head = self.snapshot.head.clone();
        let has_remotes = !self.snapshot.remotes.is_empty();
        let sep = || div().w(px(1.)).h(px(30.)).mx(px(6.)).bg(theme.border);

        let repo_block = div()
            .id("repo-block")
            .flex_none()
            .w(px(220.))
            .min_w(px(140.))
            .h(px(42.))
            .px(px(10.))
            .rounded(px(6.))
            .flex()
            .flex_col()
            .justify_center()
            .cursor_pointer()
            .hover(|s| s.bg(theme.hover))
            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).text_size(px(14.)).whitespace_nowrap().overflow_hidden().text_ellipsis().child(SharedString::from(self.repo.name())))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_size(px(12.))
                    .text_color(theme.text_muted)
                    .child(icon_sized(Icon::Branch, px(12.)))
                    .child(div().whitespace_nowrap().overflow_hidden().text_ellipsis().child(head.display_name()))
                    .child(icon_sized(Icon::ChevronDown, px(10.))),
            )
            .tooltip(tooltip_with_meta("Switch branch", "Click to check out another branch"))
            .on_click(cx.listener(|this, _, window, cx| this.open_palette(PaletteMode::Checkout, window, cx)));

        let quick = div()
            .id("quick-actions")
            .flex_none()
            .w(px(240.))
            .h(px(30.))
            .px(px(10.))
            .rounded(px(6.))
            .bg(theme.input_bg)
            .border_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .text_color(theme.text_faint)
            .hover(|s| s.border_color(theme.border_focus))
            .child(icon(Icon::Search))
            .child(div().flex_1().text_size(px(12.)).child("Quick Actions…"))
            .child(kbd(if cfg!(target_os = "macos") { "⌘K" } else { "Ctrl+K" }, cx))
            .on_click(cx.listener(|this, _, window, cx| this.open_palette(PaletteMode::All, window, cx)));

        let branch = head.branch.clone();
        div()
            .flex_none()
            .h(px(54.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(2.))
            .bg(theme.toolbar_bg)
            .border_b_1()
            .border_color(theme.border)
            .child(repo_block)
            .child(sep())
            .child(self.tool_button("tb-fetch", "Fetch", Icon::Fetch, "Download new data from all remotes", None, busy || !has_remotes, cx, |this, _, cx| this.fetch(None, cx)))
            .child(self.tool_button("tb-pull", "Pull", Icon::Pull, "Fetch and integrate remote changes", Some(head.behind), busy || head.upstream.is_none(), cx, |this, _, cx| this.pull(cx)))
            .child(self.tool_button("tb-push", "Push", Icon::Push, "Upload local commits", Some(head.ahead), busy || branch.is_none() || !has_remotes, cx, |this, window, cx| this.push(window, cx)))
            .child(sep())
            .child(self.tool_button("tb-branch", "Branch", Icon::Branch, "Create a new branch", None, busy, cx, |this, window, cx| this.open_create_branch(None, window, cx)))
            .child(self.tool_button("tb-merge", "Merge", Icon::Merge, "Merge a branch into the current one", None, busy || branch.is_none(), cx, |this, window, cx| this.open_palette(PaletteMode::Merge, window, cx)))
            .child(self.tool_button("tb-rebase", "Rebase", Icon::Rebase, "Rebase the current branch", None, busy || branch.is_none(), cx, |this, window, cx| this.open_palette(PaletteMode::Rebase, window, cx)))
            .child(sep())
            .child(self.tool_button("tb-stash", "Stash", Icon::Stash, "Save local changes for later", None, busy || self.snapshot.status.is_clean(), cx, |this, window, cx| this.open_stash_dialog(window, cx)))
            .child(self.tool_button("tb-pop", "Pop", Icon::StashPop, "Apply and remove the latest stash", None, busy || self.snapshot.stashes.is_empty(), cx, |this, _, cx| this.pop_latest_stash(cx)))
            .child(div().flex_1().min_w(px(8.)))
            .child(quick)
            .child(div().w(px(6.)))
            .child(self.tool_button("tb-terminal", "Terminal", Icon::Terminal, "Open a terminal in this repository", None, false, cx, |this, _, cx| this.open_terminal(cx)))
            .child(self.tool_button("tb-folder", "Explorer", Icon::Folder, "Show the repository in your file manager", None, false, cx, |this, _, cx| this.reveal_in_files(cx)))
            .child(self.tool_button("tb-settings", "Settings", Icon::Settings, "Preferences", None, false, cx, |_, window, cx| window.dispatch_action(Box::new(OpenSettings), cx)))
    }
}
