//! The left sidebar: workspace views, branches, remotes, tags and submodules.
//!
//! The tree is flattened into rows and rendered with a virtualized list, so
//! repositories with thousands of branches stay fast.

use super::{Nav, RepoEvent, RepoView};
use crate::assets::Icon;
use crate::git::LogScope;
use crate::theme::ActiveTheme;
use crate::ui::menu::ContextMenu;
use crate::ui::text_input::TextInput;
use crate::ui::{self, badge, icon, icon_sized};
use gpui::{div, prelude::*, px, uniform_list, AnyElement, ClickEvent, Context, Entity, MouseButton, Pixels, Point, SharedString, UniformListScrollHandle, Window};
use std::collections::{BTreeMap, HashSet};
use std::ops::Range;

const ROW_HEIGHT: f32 = 26.;

pub struct SidebarState {
    pub collapsed: HashSet<String>,
    pub filter: Entity<TextInput>,
    pub scroll: UniformListScrollHandle,
    rows: Vec<Row>,
}

impl SidebarState {
    pub fn new(filter: Entity<TextInput>) -> Self {
        let mut collapsed = HashSet::new();
        collapsed.insert("section:tags".to_string());
        Self { collapsed, filter, scroll: UniformListScrollHandle::new(), rows: Vec::new() }
    }

    fn toggle(&mut self, key: &str) {
        if !self.collapsed.remove(key) {
            self.collapsed.insert(key.to_string());
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RefDragKind {
    Local,
    Remote,
}

/// Payload when dragging a branch in the sidebar.
#[derive(Clone, Debug)]
pub struct DraggedRef {
    pub name: String,
    pub kind: RefDragKind,
}

pub struct DragPreview {
    label: SharedString,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div().pl(px(14.)).pt(px(4.)).child(
            div()
                .px(px(8.))
                .py(px(3.))
                .rounded(px(5.))
                .bg(theme.accent)
                .text_color(theme.text_on_accent)
                .text_size(px(12.))
                .shadow_md()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(icon(Icon::Branch).text_color(theme.text_on_accent))
                .child(self.label.clone()),
        )
    }
}

#[derive(Default)]
struct TreeNode {
    children: BTreeMap<String, TreeNode>,
    leaf: Option<String>,
}

impl TreeNode {
    fn insert(&mut self, path: &str, full: String) {
        let mut node = self;
        let parts: Vec<&str> = path.split('/').collect();
        for (i, part) in parts.iter().enumerate() {
            // Leaves sort after folders; \u{1} marks a leaf key.
            let key = if i + 1 == parts.len() { format!("\u{1}{part}") } else { part.to_string() };
            node = node.children.entry(key).or_default();
        }
        node.leaf = Some(full);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum LeafKind {
    Local,
    Remote,
    Tag,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum SectionAdd {
    Branch,
    Remote,
}

#[derive(Clone, Debug)]
enum Row {
    Label(&'static str),
    Nav { id: &'static str, label: &'static str, icon: Icon, nav: Nav, count: usize },
    Section { key: &'static str, title: &'static str, count: usize, add: Option<SectionAdd> },
    Folder { key: String, name: String, depth: usize },
    Leaf { label: String, full: String, depth: usize, kind: LeafKind },
    Remote { name: String, url: String },
    Submodule { path: String, state: char },
    Empty(&'static str),
}

impl RepoView {
    /// Rebuilds the flattened sidebar rows from the snapshot, filter and collapsed state.
    fn build_sidebar_rows(&self, filter: &str) -> Vec<Row> {
        let mut rows = Vec::new();
        let sb = &self.sidebar;
        let is_open = |key: &str| !sb.collapsed.contains(key) || !filter.is_empty();
        if filter.is_empty() {
            rows.push(Row::Label("WORKSPACE"));
            rows.push(Row::Nav { id: "nav-wc", label: "Working Copy", icon: Icon::WorkingCopy, nav: Nav::WorkingCopy, count: self.snapshot.status.change_count() });
            rows.push(Row::Nav { id: "nav-history", label: "History", icon: Icon::History, nav: Nav::History, count: 0 });
            rows.push(Row::Nav { id: "nav-stashes", label: "Stashes", icon: Icon::Stash, nav: Nav::Stashes, count: self.snapshot.stashes.len() });
            rows.push(Row::Nav { id: "nav-reflog", label: "Reflog", icon: Icon::List, nav: Nav::Reflog, count: 0 });
            if self.prs.repo.is_some() {
                rows.push(Row::Nav { id: "nav-prs", label: "Pull Requests", icon: Icon::PullRequest, nav: Nav::PullRequests, count: self.prs.open_count.unwrap_or(0) });
            }
        }

        let matches = |s: &str| filter.is_empty() || s.to_lowercase().contains(filter);

        let branches: Vec<_> = self.snapshot.branches.iter().filter(|b| matches(&b.name)).collect();
        rows.push(Row::Section { key: "branches", title: "BRANCHES", count: branches.len(), add: Some(SectionAdd::Branch) });
        if is_open("section:branches") {
            if filter.is_empty() {
                let mut tree = TreeNode::default();
                for b in &branches {
                    tree.insert(&b.name, b.name.clone());
                }
                self.flatten_tree(&tree, "b:", 0, LeafKind::Local, &mut rows);
            } else {
                rows.extend(branches.iter().map(|b| Row::Leaf { label: b.name.clone(), full: b.name.clone(), depth: 0, kind: LeafKind::Local }));
            }
            if branches.is_empty() {
                rows.push(Row::Empty(if filter.is_empty() { "No branches yet" } else { "No matching branches" }));
            }
        }

        rows.push(Row::Section { key: "remotes", title: "REMOTES", count: self.snapshot.remotes.len(), add: Some(SectionAdd::Remote) });
        if is_open("section:remotes") {
            for remote in &self.snapshot.remotes {
                let rbs: Vec<_> = self.snapshot.remote_branches.iter().filter(|r| r.remote == remote.name && matches(&r.full_name)).collect();
                if !filter.is_empty() && rbs.is_empty() {
                    continue;
                }
                rows.push(Row::Remote { name: remote.name.clone(), url: remote.fetch_url.clone() });
                if is_open(&format!("r:{}", remote.name)) {
                    if filter.is_empty() {
                        let mut tree = TreeNode::default();
                        for rb in &rbs {
                            tree.insert(&rb.name, rb.full_name.clone());
                        }
                        self.flatten_tree(&tree, &format!("r:{}/", remote.name), 1, LeafKind::Remote, &mut rows);
                    } else {
                        rows.extend(rbs.iter().map(|rb| Row::Leaf { label: rb.full_name.clone(), full: rb.full_name.clone(), depth: 1, kind: LeafKind::Remote }));
                    }
                }
            }
            if self.snapshot.remotes.is_empty() && filter.is_empty() {
                rows.push(Row::Empty("No remotes"));
            }
        }

        let tags: Vec<_> = self.snapshot.tags.iter().filter(|t| matches(&t.name)).collect();
        rows.push(Row::Section { key: "tags", title: "TAGS", count: tags.len(), add: None });
        if is_open("section:tags") {
            rows.extend(tags.iter().map(|t| Row::Leaf { label: t.name.clone(), full: t.name.clone(), depth: 0, kind: LeafKind::Tag }));
        }

        if !self.snapshot.submodules.is_empty() && filter.is_empty() {
            rows.push(Row::Section { key: "submodules", title: "SUBMODULES", count: self.snapshot.submodules.len(), add: None });
            if is_open("section:submodules") {
                rows.extend(self.snapshot.submodules.iter().map(|s| Row::Submodule { path: s.path.clone(), state: s.state }));
            }
        }
        rows
    }

    fn flatten_tree(&self, node: &TreeNode, prefix: &str, depth: usize, kind: LeafKind, out: &mut Vec<Row>) {
        let mut folders: Vec<(&String, &TreeNode)> = node.children.iter().filter(|(k, _)| !k.starts_with('\u{1}')).collect();
        folders.sort_by_key(|(k, _)| k.to_lowercase());
        for (name, child) in folders {
            let key = format!("{prefix}{name}");
            let collapsed = self.sidebar.collapsed.contains(&key);
            out.push(Row::Folder { key: key.clone(), name: name.clone(), depth });
            if !collapsed {
                self.flatten_tree(child, &format!("{key}/"), depth + 1, kind, out);
            }
        }
        let mut leaves: Vec<(&String, &TreeNode)> = node.children.iter().filter(|(k, _)| k.starts_with('\u{1}')).collect();
        leaves.sort_by_key(|(k, _)| k.to_lowercase());
        for (name, child) in leaves {
            let label = name.trim_start_matches('\u{1}');
            if let Some(full) = &child.leaf {
                out.push(Row::Leaf { label: label.to_string(), full: full.clone(), depth, kind });
            }
            if !child.children.is_empty() {
                // A branch named like a folder ("a" next to "a/b").
                self.flatten_tree(child, &format!("{prefix}{label}/"), depth + 1, kind, out);
            }
        }
    }

    pub fn render_sidebar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let filter = self.sidebar.filter.read(cx).text().trim().to_lowercase();
        self.sidebar.rows = self.build_sidebar_rows(&filter);
        let count = self.sidebar.rows.len();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.sidebar_bg)
            .child(div().flex_none().p(px(8.)).child(self.sidebar.filter.clone()))
            .child(
                uniform_list("sidebar-rows", count, cx.processor(|this: &mut RepoView, range: Range<usize>, _window, cx| range.map(|ix| this.render_sidebar_row(ix, cx)).collect::<Vec<_>>()))
                    .track_scroll(self.sidebar.scroll.clone())
                    .flex_1()
                    .pb(px(12.)),
            )
    }

    /// The outer cell has the fixed row height; the inner pill carries hover and selection.
    fn row_shell(&self, id: impl Into<SharedString>, depth: usize, selected: bool, cx: &Context<Self>) -> gpui::Stateful<gpui::Div> {
        let theme = cx.theme();
        let id: SharedString = id.into();
        div()
            .id(id)
            .h_full()
            .w_full()
            .pl(px(8. + depth as f32 * 14.))
            .pr(px(8.))
            .rounded(px(5.))
            .flex()
            .items_center()
            .gap(px(7.))
            .text_size(px(13.))
            .cursor_pointer()
            .when(selected, |d| d.bg(theme.selection))
            .when(!selected, |d| d.hover(|s| s.bg(theme.hover)))
    }

    fn render_sidebar_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(row) = self.sidebar.rows.get(ix).cloned() else { return div().into_any_element() };
        let inner: AnyElement = match row {
            Row::Label(text) => div().h_full().px(px(6.)).flex().items_end().pb(px(3.)).text_size(px(11.)).font_weight(gpui::FontWeight::BOLD).text_color(theme.text_faint).child(text).into_any_element(),
            Row::Empty(text) => div().h_full().pl(px(30.)).flex().items_center().text_size(px(12.)).text_color(theme.text_faint).child(text).into_any_element(),
            Row::Nav { id, label, icon: nav_icon, nav, count } => {
                let selected = match (&nav, &self.nav) {
                    (Nav::History, Nav::History) => matches!(self.history.scope, LogScope::Head | LogScope::All),
                    (a, b) => a == b,
                };
                let is_wc = nav == Nav::WorkingCopy;
                self.row_shell(id, 0, selected, cx)
                    .child(icon(nav_icon).text_color(if selected { theme.text } else { theme.text_muted }))
                    .child(div().flex_1().child(label))
                    .when(count > 0, |d| d.child(badge(count.to_string(), if is_wc { theme.accent } else { theme.active }, if is_wc { theme.text_on_accent } else { theme.text_muted })))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if nav == Nav::History {
                            let scope = if cx.global::<crate::settings::Settings>().show_all_branches { LogScope::All } else { LogScope::Head };
                            this.show_history_scope(scope, window, cx);
                        } else {
                            this.set_nav(nav.clone(), window, cx);
                        }
                    }))
                    .into_any_element()
            }
            Row::Section { key, title, count, add } => {
                let collapsed = self.sidebar.collapsed.contains(&format!("section:{key}"));
                let group = SharedString::from(format!("section-{key}"));
                let add_button = add.map(|a| {
                    let (id, tip) = match a {
                        SectionAdd::Branch => ("add-branch", "New branch"),
                        SectionAdd::Remote => ("add-remote", "Add remote"),
                    };
                    ui::Button::icon_only(id, Icon::Plus).small().tooltip(tip).on_click(cx.listener(move |this, _, window, cx| match a {
                        SectionAdd::Branch => this.open_create_branch(None, window, cx),
                        SectionAdd::Remote => this.open_add_remote(window, cx),
                    }))
                });
                div()
                    .id(SharedString::from(format!("section-{key}")))
                    .group(group.clone())
                    .h_full()
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .cursor_pointer()
                    .border_t_1()
                    .border_color(theme.border_subtle)
                    .child(icon_sized(if collapsed { Icon::ChevronRight } else { Icon::ChevronDown }, px(11.)).text_color(theme.text_faint))
                    .child(div().text_size(px(11.)).font_weight(gpui::FontWeight::BOLD).text_color(theme.text_faint).child(title))
                    .child(div().text_size(px(11.)).text_color(theme.text_faint).child(count.to_string()))
                    .child(div().flex_1())
                    .when_some(add_button, |d, b| d.child(div().invisible().group_hover(group, |s| s.visible()).child(b)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sidebar.toggle(&format!("section:{key}"));
                        cx.notify();
                    }))
                    .into_any_element()
            }
            Row::Folder { key, name, depth } => {
                let collapsed = self.sidebar.collapsed.contains(&key);
                self.row_shell(format!("folder-{key}"), depth, false, cx)
                    .child(icon_sized(if collapsed { Icon::ChevronRight } else { Icon::ChevronDown }, px(11.)).text_color(theme.text_faint))
                    .child(icon(if collapsed { Icon::Folder } else { Icon::FolderOpen }).text_color(theme.text_muted))
                    .child(div().flex_1().whitespace_nowrap().overflow_hidden().text_ellipsis().text_color(theme.text_muted).child(name))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sidebar.toggle(&key);
                        cx.notify();
                    }))
                    .into_any_element()
            }
            Row::Remote { name, url } => {
                let key = format!("r:{name}");
                let collapsed = self.sidebar.collapsed.contains(&key);
                let name2 = name.clone();
                let name3 = name.clone();
                self.row_shell(format!("remote-{name}"), 0, false, cx)
                    .child(icon_sized(if collapsed { Icon::ChevronRight } else { Icon::ChevronDown }, px(11.)).text_color(theme.text_faint))
                    .child(icon(Icon::Globe).text_color(theme.text_muted))
                    .child(div().flex_1().whitespace_nowrap().overflow_hidden().text_ellipsis().child(name))
                    .tooltip(ui::tooltip(url))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sidebar.toggle(&key);
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                            let menu = this.remote_menu(&name2, e.position, cx);
                            this.open_menu(menu, cx);
                        }),
                    )
                    .drag_over::<DraggedRef>(|s, _, _, cx| s.bg(cx.theme().accent_subtle))
                    .on_drop(cx.listener(move |this, dragged: &DraggedRef, window, cx| this.on_drop_on_remote(dragged.clone(), name3.clone(), window.mouse_position(), cx)))
                    .into_any_element()
            }
            Row::Submodule { path, state } => {
                let hint = match state {
                    '-' => Some("not initialized"),
                    '+' => Some("modified"),
                    'U' => Some("conflict"),
                    _ => None,
                };
                let (p1, p2) = (path.clone(), path.clone());
                self.row_shell(format!("sub-{path}"), 0, false, cx)
                    .child(icon(Icon::Package).text_color(theme.text_muted))
                    .child(div().flex_1().whitespace_nowrap().overflow_hidden().text_ellipsis().child(path))
                    .when_some(hint, |d, h| d.child(div().text_size(px(11.)).text_color(theme.warning).child(h)))
                    .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                        if e.click_count() >= 2 {
                            cx.emit(RepoEvent::OpenRepository(this.repo.workdir.join(&p1)));
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                            let menu = this.submodule_menu(&p2, e.position, cx);
                            this.open_menu(menu, cx);
                        }),
                    )
                    .into_any_element()
            }
            Row::Leaf { label, full, depth, kind } => self.render_leaf(&label, &full, depth, kind, cx),
        };
        div().w_full().h(px(ROW_HEIGHT)).px(px(6.)).py(px(1.)).child(inner).into_any_element()
    }

    fn render_leaf(&self, label: &str, full: &str, depth: usize, kind: LeafKind, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let full = full.to_string();
        let scope = match kind {
            LeafKind::Local | LeafKind::Remote => LogScope::Ref(full.clone()),
            LeafKind::Tag => LogScope::Ref(format!("refs/tags/{full}")),
        };
        let selected = self.nav == Nav::History && self.history.scope == scope;
        let local = (kind == LeafKind::Local).then(|| self.snapshot.branches.iter().find(|b| b.name == full)).flatten();
        let is_head = local.map(|b| b.is_head).unwrap_or(false);
        let leaf_icon = match kind {
            LeafKind::Local if is_head => Icon::Check,
            LeafKind::Local | LeafKind::Remote => Icon::Branch,
            LeafKind::Tag => Icon::Tag,
        };
        let id_prefix = match kind {
            LeafKind::Local => "lb",
            LeafKind::Remote => "rb",
            LeafKind::Tag => "tag",
        };
        let mut row = self
            .row_shell(format!("{id_prefix}-{full}"), depth, selected, cx)
            .child(icon(leaf_icon).text_color(if is_head { theme.success } else { theme.text_muted }))
            .child(div().flex_1().min_w_0().whitespace_nowrap().overflow_hidden().text_ellipsis().when(is_head, |d| d.font_weight(gpui::FontWeight::SEMIBOLD)).child(label.to_string()));
        if let Some(b) = local {
            if b.upstream_gone {
                row = row.child(div().text_size(px(10.)).text_color(theme.text_faint).child("gone"));
            }
            if b.ahead > 0 {
                row = row.child(div().flex().items_center().text_size(px(11.)).text_color(theme.info).child(icon_sized(Icon::ArrowUp, px(10.))).child(b.ahead.to_string()));
            }
            if b.behind > 0 {
                row = row.child(div().flex().items_center().text_size(px(11.)).text_color(theme.warning).child(icon_sized(Icon::ArrowDown, px(10.))).child(b.behind.to_string()));
            }
        }
        let f1 = full.clone();
        let f2 = full.clone();
        row = row
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                if e.click_count() >= 2 {
                    match kind {
                        LeafKind::Local => this.checkout_branch(f1.clone(), cx),
                        LeafKind::Remote => this.checkout_remote_branch(f1.clone(), cx),
                        LeafKind::Tag => {}
                    }
                } else {
                    this.show_history_scope(scope.clone(), window, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                    let menu = match kind {
                        LeafKind::Local => this.branch_menu(&f2, e.position, cx),
                        LeafKind::Remote => this.remote_branch_menu(&f2, e.position, cx),
                        LeafKind::Tag => this.tag_menu(&f2, e.position, cx),
                    };
                    this.open_menu(menu, cx);
                }),
            );
        if kind != LeafKind::Tag {
            let drag = DraggedRef { name: full.clone(), kind: if kind == LeafKind::Local { RefDragKind::Local } else { RefDragKind::Remote } };
            row = row.on_drag(drag, |d: &DraggedRef, _, _, cx| cx.new(|_| DragPreview { label: d.name.clone().into() }));
        }
        if kind == LeafKind::Local {
            let target = full.clone();
            row = row.drag_over::<DraggedRef>(|s, _, _, cx| s.bg(cx.theme().accent_subtle)).on_drop(cx.listener(move |this, dragged: &DraggedRef, window, cx| this.on_drop_on_branch(dragged.clone(), target.clone(), window.mouse_position(), cx)));
        }
        row.into_any_element()
    }

    fn on_drop_on_branch(&mut self, dragged: DraggedRef, target: String, position: Point<Pixels>, cx: &mut Context<Self>) {
        if dragged.name == target {
            return;
        }
        let current = self.snapshot.head.branch.clone();
        let source = dragged.name.clone();
        let mut menu = ContextMenu::new(position).header(format!("{source} → {target}"));
        if current.as_deref() == Some(target.as_str()) {
            let s1 = source.clone();
            let s2 = source.clone();
            menu = menu
                .item(format!("Merge {source} into {target}"), Some(Icon::Merge), self.handler(cx, move |this, window, cx| this.open_merge_dialog(s1.clone(), window, cx)))
                .item(format!("Rebase {target} onto {source}"), Some(Icon::Rebase), self.handler(cx, move |this, _, cx| this.rebase_current_onto(s2.clone(), cx)));
        } else {
            let t = target.clone();
            menu = menu.item(format!("Check out {target} first"), Some(Icon::Branch), self.handler(cx, move |this, _, cx| this.checkout_branch(t.clone(), cx)));
        }
        self.open_menu(menu, cx);
    }

    fn on_drop_on_remote(&mut self, dragged: DraggedRef, remote: String, position: Point<Pixels>, cx: &mut Context<Self>) {
        if dragged.kind != RefDragKind::Local {
            return;
        }
        let branch = dragged.name.clone();
        let b = branch.clone();
        let r = remote.clone();
        let menu = ContextMenu::new(position).item(
            format!("Push {branch} to {remote}"),
            Some(Icon::Push),
            self.handler(cx, move |this, _, cx| {
                let (b, r) = (b.clone(), r.clone());
                this.run_progress_op(format!("Pushing {b}"), format!("Pushed {b} to {r}"), move |repo, p| repo.push(&r, &b, true, false, false, p), cx);
            }),
        );
        self.open_menu(menu, cx);
    }
}
