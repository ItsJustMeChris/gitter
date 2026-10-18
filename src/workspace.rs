//! The window root: title bar with repository tabs, and the repository manager.

use crate::actions::*;
use crate::assets::Icon;
use crate::git::{self, Repo};
use crate::repo::{RepoEvent, RepoView};
use crate::settings::Settings;
use crate::theme::{ui_font, ActiveTheme, Theme, ThemeMode};
use crate::ui::text_input::{InputEvent, TextInput};
use crate::ui::{self, icon, icon_sized, Button};
use gpui::{
    anchored, deferred, div, point, prelude::*, px, AnyElement, App, Context, Entity, FocusHandle, Focusable, MouseButton, PathPromptOptions, SharedString, Subscription,
    Window, WindowControlArea,
};
use std::path::PathBuf;

struct CloneDialog {
    url: Entity<TextInput>,
    dest: Entity<TextInput>,
    name: Entity<TextInput>,
    running: bool,
    progress: Option<String>,
    error: Option<String>,
    name_edited: bool,
    _subs: Vec<Subscription>,
}

pub struct Workspace {
    tabs: Vec<Entity<RepoView>>,
    active: Option<usize>,
    focus_handle: FocusHandle,
    search: Entity<TextInput>,
    clone_dialog: Option<CloneDialog>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextInput::new(cx).with_placeholder("Filter repositories"));
        let sub = cx.subscribe(&search, |_, _, ev: &InputEvent, cx| {
            if *ev == InputEvent::Changed {
                cx.notify();
            }
        });
        let mut this = Self { tabs: Vec::new(), active: None, focus_handle: cx.focus_handle(), search, clone_dialog: None, error: None, _subscriptions: vec![sub] };

        // Restore the tabs from last session, or open a repository passed on the command line.
        let settings = cx.global::<Settings>().clone();
        let arg = std::env::args().nth(1).map(PathBuf::from);
        for path in &settings.open_tabs {
            if path.exists() {
                this.open_repo(path.clone(), false, window, cx);
            }
        }
        if let Some(i) = settings.active_tab.filter(|i| *i < this.tabs.len()) {
            this.active = Some(i);
        }
        if let Some(arg) = arg {
            this.open_repo(arg, true, window, cx);
        }
        this.focus_active(window, cx);
        this.update_title(window, cx);
        this
    }

    fn focus_active(&self, window: &mut Window, cx: &mut Context<Self>) {
        match self.active.and_then(|i| self.tabs.get(i)) {
            Some(tab) => window.focus(&tab.focus_handle(cx)),
            None => window.focus(&self.focus_handle),
        }
    }

    fn update_title(&self, window: &mut Window, cx: &mut Context<Self>) {
        let title = match self.active.and_then(|i| self.tabs.get(i)) {
            Some(tab) => format!("{} — Gitter", tab.read(cx).title()),
            None => "Gitter".to_string(),
        };
        window.set_window_title(&title);
    }

    fn persist_tabs(&self, cx: &mut Context<Self>) {
        let tabs: Vec<PathBuf> = self.tabs.iter().map(|t| t.read(cx).path().to_path_buf()).collect();
        let active = self.active;
        cx.update_global::<Settings, _>(|s, _| {
            s.open_tabs = tabs;
            s.active_tab = active;
            s.save();
        });
    }

    pub fn open_repo(&mut self, path: PathBuf, activate: bool, window: &mut Window, cx: &mut Context<Self>) {
        let repo = match Repo::open(&path) {
            Ok(r) => r,
            Err(e) => {
                self.error = Some(e.to_string());
                self.active = None;
                cx.notify();
                return;
            }
        };
        if let Some(ix) = self.tabs.iter().position(|t| t.read(cx).path() == repo.workdir) {
            if activate {
                self.activate(Some(ix), window, cx);
            }
            return;
        }
        let name = repo.name();
        let workdir = repo.workdir.clone();
        let view = cx.new(|cx| RepoView::new(repo, window, cx));
        let sub = cx.subscribe_in(&view, window, |this, _, ev: &RepoEvent, window, cx| match ev {
            RepoEvent::TitleChanged => {
                this.update_title(window, cx);
                cx.notify();
            }
            RepoEvent::OpenRepository(path) => this.open_repo(path.clone(), true, window, cx),
        });
        self._subscriptions.push(sub);
        self.tabs.push(view);
        self.error = None;
        cx.update_global::<Settings, _>(|s, _| {
            s.touch_recent(&workdir, &name);
            s.save();
        });
        if activate || self.active.is_none() {
            self.activate(Some(self.tabs.len() - 1), window, cx);
        }
        self.persist_tabs(cx);
    }

    fn activate(&mut self, ix: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.active = ix.filter(|i| *i < self.tabs.len());
        if let Some(tab) = self.active.and_then(|i| self.tabs.get(i)) {
            tab.update(cx, |t, cx| t.refresh(cx));
        }
        self.focus_active(window, cx);
        self.update_title(window, cx);
        self.persist_tabs(cx);
        cx.notify();
    }

    fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.tabs.remove(ix);
        let next = match self.active {
            Some(a) if a == ix => {
                if self.tabs.is_empty() {
                    None
                } else {
                    Some(ix.min(self.tabs.len() - 1))
                }
            }
            Some(a) if a > ix => Some(a - 1),
            other => other,
        };
        self.activate(next, window, cx);
    }

    fn prompt_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Open Repository".into()) });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(path) = paths.into_iter().next() {
                    this.update_in(cx, |this, window, cx| this.open_repo(path, true, window, cx)).ok();
                }
            }
        })
        .detach();
    }

    fn prompt_init(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Create Repository Here".into()) });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(path) = paths.into_iter().next() {
                    let result = cx.background_executor().spawn({
                        let path = path.clone();
                        async move { Repo::init(&path) }
                    });
                    let result = result.await;
                    this.update_in(cx, |this, window, cx| match result {
                        Ok(repo) => this.open_repo(repo.workdir, true, window, cx),
                        Err(e) => {
                            this.error = Some(e.to_string());
                            cx.notify();
                        }
                    })
                    .ok();
                }
            }
        })
        .detach();
    }

    fn open_clone_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active = None;
        let default_dir = cx.global::<Settings>().default_clone_dir();
        let url = cx.new(|cx| TextInput::new(cx).with_placeholder("https://github.com/user/repository.git"));
        let dest = cx.new(|cx| TextInput::new(cx).with_text(default_dir.display().to_string()));
        let name = cx.new(|cx| TextInput::new(cx).with_placeholder("Folder name"));
        let s1 = cx.subscribe_in(&url, window, |this, input, ev: &InputEvent, window, cx| match ev {
            InputEvent::Changed => {
                let text = input.read(cx).text().to_string();
                if let Some(d) = this.clone_dialog.as_ref() {
                    if !d.name_edited {
                        let guess = guess_repo_name(&text);
                        let name = d.name.clone();
                        name.update(cx, |n, cx| n.set_text(guess, cx));
                        if let Some(d) = this.clone_dialog.as_mut() {
                            d.name_edited = false;
                        }
                    }
                }
            }
            InputEvent::Submit => this.start_clone(window, cx),
            _ => {}
        });
        let s2 = cx.subscribe_in(&name, window, |this, input, ev: &InputEvent, window, cx| match ev {
            InputEvent::Changed => {
                if input.focus_handle(cx).is_focused(window) {
                    if let Some(d) = this.clone_dialog.as_mut() {
                        d.name_edited = true;
                    }
                }
            }
            InputEvent::Submit => this.start_clone(window, cx),
            _ => {}
        });
        let s3 = cx.subscribe_in(&dest, window, |this, _, ev: &InputEvent, window, cx| {
            if *ev == InputEvent::Submit {
                this.start_clone(window, cx)
            }
        });
        window.focus(&url.focus_handle(cx));
        self.clone_dialog = Some(CloneDialog { url, dest, name, running: false, progress: None, error: None, name_edited: false, _subs: vec![s1, s2, s3] });
        cx.notify();
    }

    fn start_clone(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.clone_dialog.as_mut() else { return };
        if d.running {
            return;
        }
        let url = d.url.read(cx).text().trim().to_string();
        let dest = d.dest.read(cx).text().trim().to_string();
        let name = d.name.read(cx).text().trim().to_string();
        if url.is_empty() || dest.is_empty() || name.is_empty() {
            d.error = Some("Enter a URL, a destination folder and a name.".into());
            cx.notify();
            return;
        }
        let target = PathBuf::from(&dest).join(&name);
        if target.exists() && std::fs::read_dir(&target).map(|mut r| r.next().is_some()).unwrap_or(false) {
            d.error = Some(format!("{} already exists and is not empty.", target.display()));
            cx.notify();
            return;
        }
        d.running = true;
        d.error = None;
        d.progress = Some("Starting…".into());
        cx.notify();
        let (tx, rx) = smol::channel::unbounded::<String>();
        let parent = PathBuf::from(dest);
        cx.update_global::<Settings, _>(|s, _| {
            s.clone_directory = Some(parent.clone());
            s.save();
        });
        cx.spawn_in(window, async move |this, cx| {
            let task = cx.background_executor().spawn(async move {
                git::clone(&url, &target, |line| {
                    let _ = tx.try_send(line.to_string());
                })
            });
            while let Ok(mut line) = rx.recv().await {
                while let Ok(l) = rx.try_recv() {
                    line = l;
                }
                this.update(cx, |this, cx| {
                    if let Some(d) = this.clone_dialog.as_mut() {
                        d.progress = Some(line);
                    }
                    cx.notify();
                })
                .ok();
            }
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(repo) => {
                        this.clone_dialog = None;
                        this.open_repo(repo.workdir, true, window, cx);
                    }
                    Err(e) => {
                        if let Some(d) = this.clone_dialog.as_mut() {
                            d.running = false;
                            d.progress = None;
                            d.error = Some(e.to_string());
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn browse_clone_dest(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Choose Folder".into()) });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(path) = paths.into_iter().next() {
                    this.update(cx, |this, cx| {
                        if let Some(d) = this.clone_dialog.as_ref() {
                            d.dest.update(cx, |i, cx| i.set_text(path.display().to_string(), cx));
                        }
                    })
                    .ok();
                }
            }
        })
        .detach();
    }

    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        let mode = match cx.theme().mode {
            ThemeMode::Dark => ThemeMode::Light,
            ThemeMode::Light => ThemeMode::Dark,
        };
        set_theme(mode, cx);
    }

    // ---------------------------------------------------------------------
    // Rendering
    // ---------------------------------------------------------------------

    fn render_title_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mut tabs = div().flex().items_end().h_full().gap(px(2.)).min_w_0().overflow_hidden();
        for (ix, tab) in self.tabs.iter().enumerate() {
            let active = self.active == Some(ix);
            let view = tab.read(cx);
            let name = view.title();
            let changes = view.snapshot.status.change_count();
            let busy = view.is_busy();
            tabs = tabs.child(
                div()
                    .id(("tab", ix))
                    .group(SharedString::from(format!("tab-{ix}")))
                    .h(px(30.))
                    .min_w(px(110.))
                    .max_w(px(220.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .rounded_t(px(7.))
                    .text_size(px(12.))
                    .cursor_pointer()
                    .when(active, |d| d.bg(theme.toolbar_bg).text_color(theme.text).border_1().border_b_0().border_color(theme.border))
                    .when(!active, |d| d.text_color(theme.text_muted).hover(|s| s.bg(theme.hover)))
                    .child(if busy { crate::repo::spinner(cx).into_any_element() } else { icon(Icon::Repo).into_any_element() })
                    .child(div().flex_1().min_w_0().whitespace_nowrap().overflow_hidden().text_ellipsis().child(name))
                    .when(changes > 0, |d| d.child(div().size(px(6.)).rounded_full().bg(theme.warning)))
                    .child(
                        div()
                            .id(("tab-close", ix))
                            .size(px(16.))
                            .rounded(px(3.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .invisible()
                            .group_hover(SharedString::from(format!("tab-{ix}")), |s| s.visible())
                            .when(active, |d| d.visible())
                            .hover(|s| s.bg(theme.active))
                            .child(icon_sized(Icon::Close, px(11.)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.close_tab(ix, window, cx);
                            })),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.activate(Some(ix), window, cx)))
                    .on_mouse_down(MouseButton::Middle, cx.listener(move |this, _, window, cx| this.close_tab(ix, window, cx))),
            );
        }
        let manager_active = self.active.is_none();
        tabs = tabs.child(
            div()
                .id("tab-new")
                .h(px(30.))
                .px(px(9.))
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded_t(px(7.))
                .cursor_pointer()
                .text_size(px(12.))
                .when(manager_active, |d| d.bg(theme.app_bg).text_color(theme.text).border_1().border_b_0().border_color(theme.border))
                .when(!manager_active, |d| d.text_color(theme.text_muted).hover(|s| s.bg(theme.hover)))
                .child(icon(Icon::Plus))
                .when(self.tabs.is_empty() || manager_active, |d| d.child("Repositories"))
                .tooltip(ui::tooltip("Repositories (Ctrl+T)"))
                .on_click(cx.listener(|this, _, window, cx| this.activate(None, window, cx))),
        );

        let is_mac = cfg!(target_os = "macos");
        let is_windows = cfg!(target_os = "windows");
        let csd = matches!(window.window_decorations(), gpui::Decorations::Client { .. });
        let show_controls = is_windows || (cfg!(target_os = "linux") && csd);
        let maximized = window.is_maximized();

        let control = |id: &'static str, ic: Icon, area: WindowControlArea, close: bool, cx: &mut Context<Self>| {
            let theme = cx.theme().clone();
            let hover = if close { gpui::rgb(0xe81123).into() } else { theme.hover };
            let mut el = div()
                .id(id)
                .w(px(46.))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.text_muted)
                .hover(move |s| s.bg(hover).text_color(if close { gpui::white() } else { theme.text }))
                .window_control_area(area)
                .child(icon_sized(ic, px(16.)));
            if cfg!(target_os = "linux") {
                el = el.on_click(move |_, window, _| match area {
                    WindowControlArea::Min => window.minimize_window(),
                    WindowControlArea::Max => window.zoom_window(),
                    WindowControlArea::Close => window.remove_window(),
                    WindowControlArea::Drag => {}
                });
            }
            el
        };

        let mut bar = div()
            .id("title-bar")
            .flex_none()
            .h(px(38.))
            .w_full()
            .flex()
            .items_end()
            .bg(theme.sidebar_bg)
            .border_b_1()
            .border_color(theme.border)
            .child(div().w(px(if is_mac { 78. } else { 10. })).h_full().window_control_area(WindowControlArea::Drag))
            .when(!is_mac, |d| {
                d.child(
                    div()
                        .h_full()
                        .pr(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .window_control_area(WindowControlArea::Drag)
                        .child(logo(px(18.), &theme))
                        .child(div().text_size(px(12.)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(theme.text_muted).child("Gitter")),
                )
            })
            .child(tabs)
            .child(
                div()
                    .id("title-drag")
                    .flex_1()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag)
                    .on_click(|e, window, _| {
                        if e.click_count() == 2 {
                            window.titlebar_double_click();
                        }
                    })
                    .when(cfg!(target_os = "linux"), |d| d.on_mouse_down(MouseButton::Left, |e, window, _| {
                        if e.click_count == 1 {
                            window.start_window_move();
                        }
                    })),
            )
            .child(
                div()
                    .id("theme-toggle")
                    .h_full()
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .text_color(theme.text_faint)
                    .hover(|s| s.text_color(theme.text))
                    .child(icon(if theme.mode == ThemeMode::Dark { Icon::Sun } else { Icon::Moon }))
                    .tooltip(ui::tooltip("Toggle light/dark theme"))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_theme(cx))),
            );
        if show_controls {
            bar = bar
                .child(control("win-min", Icon::WinMinimize, WindowControlArea::Min, false, cx))
                .child(control("win-max", if maximized { Icon::WinRestore } else { Icon::WinMaximize }, WindowControlArea::Max, false, cx))
                .child(control("win-close", Icon::WinClose, WindowControlArea::Close, true, cx));
        }
        bar
    }

    fn render_manager(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let filter = self.search.read(cx).text().trim().to_lowercase();
        let recents: Vec<_> = cx.global::<Settings>().recent_repos.iter().filter(|r| filter.is_empty() || r.name.to_lowercase().contains(&filter) || r.path.to_string_lossy().to_lowercase().contains(&filter)).cloned().collect();
        let git_version = git::git_version().unwrap_or_else(|_| "git not found — install Git and restart".into());

        let action_card = |id: &'static str, ic: Icon, title: &'static str, detail: &'static str, cx: &mut Context<Self>| {
            let theme = cx.theme().clone();
            div()
                .id(id)
                .flex_1()
                .p(px(16.))
                .rounded(px(10.))
                .bg(theme.surface)
                .border_1()
                .border_color(theme.border)
                .flex()
                .flex_col()
                .gap(px(6.))
                .cursor_pointer()
                .hover(|s| s.border_color(theme.accent).bg(theme.elevated))
                .child(div().size(px(34.)).rounded(px(8.)).bg(theme.accent_subtle).flex().items_center().justify_center().child(icon_sized(ic, px(18.)).text_color(theme.accent_hover)))
                .child(div().mt(px(4.)).font_weight(gpui::FontWeight::SEMIBOLD).child(title))
                .child(div().text_size(px(12.)).text_color(theme.text_muted).child(detail))
        };

        // A block parent sizes a flex child to its max-content width unless told otherwise.
        let mut list = div().w_full().flex().flex_col().gap(px(2.));
        if recents.is_empty() {
            list = list.child(div().p(px(24.)).flex().justify_center().text_color(theme.text_faint).child(if filter.is_empty() { "Repositories you open will appear here." } else { "No repositories match your filter." }));
        }
        for (ix, r) in recents.iter().enumerate() {
            let exists = r.path.exists();
            let path = r.path.clone();
            let path2 = r.path.clone();
            let path3 = r.path.clone();
            let pinned = r.pinned;
            let open = self.tabs.iter().any(|t| t.read(cx).path() == r.path);
            list = list.child(
                div()
                    .id(("recent", ix))
                    .group(SharedString::from(format!("recent-{ix}")))
                    .min_w_0()
                    .h(px(46.))
                    .px(px(12.))
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.hover))
                    .when(!exists, |d| d.opacity(0.5))
                    .child(div().size(px(30.)).rounded(px(7.)).bg(theme.graph_color((ix % 10) as u8).opacity(0.2)).flex().items_center().justify_center().child(icon(Icon::Repo).text_color(theme.graph_color((ix % 10) as u8))))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .child(div().flex().items_center().gap(px(6.)).child(div().font_weight(gpui::FontWeight::MEDIUM).child(r.name.clone())).when(open, |d| d.child(ui::badge("open", theme.accent_subtle, theme.accent_hover))).when(!exists, |d| d.child(ui::badge("missing", theme.danger.opacity(0.2), theme.danger))))
                            .child(div().w_full().flex().text_size(px(12.)).text_color(theme.text_faint).whitespace_nowrap().overflow_hidden().text_ellipsis().child(display_path(&r.path, 72))),
                    )
                    .child(div().text_size(px(12.)).text_color(theme.text_faint).child(ui::relative_time(r.last_opened)))
                    .child(
                        div()
                            .flex()
                            .gap(px(2.))
                            .invisible()
                            .group_hover(SharedString::from(format!("recent-{ix}")), |s| s.visible())
                            .when(pinned, |d| d.visible())
                            .child(Button::icon_only(("pin", ix), if pinned { Icon::Dot } else { Icon::Plus }).small().tooltip(if pinned { "Unpin" } else { "Pin to top" }).on_click(cx.listener(move |_, _, _, cx| {
                                let p = path2.clone();
                                cx.update_global::<Settings, _>(|s, _| {
                                    if let Some(r) = s.recent_repos.iter_mut().find(|r| r.path == p) {
                                        r.pinned = !r.pinned;
                                    }
                                    s.recent_repos.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.last_opened.cmp(&a.last_opened)));
                                    s.save();
                                });
                                cx.notify();
                            })))
                            .child(Button::icon_only(("forget", ix), Icon::Close).small().tooltip("Remove from list").on_click(cx.listener(move |_, _, _, cx| {
                                let p = path3.clone();
                                cx.update_global::<Settings, _>(|s, _| {
                                    s.remove_recent(&p);
                                    s.save();
                                });
                                cx.notify();
                            }))),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.open_repo(path.clone(), true, window, cx))),
            );
        }

        let _ = window;
        div()
            .id("manager")
            .size_full()
            .overflow_y_scroll()
            .overflow_x_hidden()
            .bg(theme.app_bg)
            .child(
                div()
                    .mx_auto()
                    .w_full()
                    .max_w(px(820.))
                    .px(px(32.))
                    .py(px(40.))
                    .flex()
                    .flex_col()
                    .gap(px(22.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(14.))
                            .child(logo(px(44.), &theme))
                            .child(div().flex().flex_col().child(div().text_size(px(24.)).font_weight(gpui::FontWeight::BOLD).child("Gitter")).child(div().text_color(theme.text_muted).child("A fast, native Git client"))),
                    )
                    .when_some(self.error.clone(), |d, e| {
                        d.child(
                            div()
                                .p(px(12.))
                                .rounded(px(8.))
                                .bg(theme.danger.opacity(0.12))
                                .border_1()
                                .border_color(theme.danger.opacity(0.4))
                                .flex()
                                .gap(px(8.))
                                .child(icon(Icon::Warning).text_color(theme.danger))
                                .child(div().flex_1().child(e))
                                .child(Button::icon_only("dismiss-error", Icon::Close).small().on_click(cx.listener(|this, _, _, cx| {
                                    this.error = None;
                                    cx.notify();
                                }))),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .gap(px(12.))
                            .child(action_card("open-repo", Icon::FolderOpen, "Open Repository", "Add an existing local repository", cx).on_click(cx.listener(|this, _, window, cx| this.prompt_open(window, cx))))
                            .child(action_card("clone-repo", Icon::Download, "Clone Repository", "Copy a remote repository to your machine", cx).on_click(cx.listener(|this, _, window, cx| this.open_clone_dialog(window, cx))))
                            .child(action_card("init-repo", Icon::Plus, "New Repository", "Start a new repository in a folder", cx).on_click(cx.listener(|this, _, window, cx| this.prompt_init(window, cx)))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(10.))
                            .child(div().flex().items_center().justify_between().child(ui::section_label("RECENT REPOSITORIES", cx)).child(div().w(px(260.)).child(self.search.clone())))
                            .child(div().flex().flex_col().p(px(6.)).rounded(px(10.)).border_1().border_color(theme.border_subtle).bg(theme.panel_bg).child(list)),
                    )
                    .child(div().text_size(px(11.)).text_color(theme.text_faint).child(format!("{git_version} · Gitter {}", env!("CARGO_PKG_VERSION")))),
            )
            .into_any_element()
    }

    fn render_clone_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let d = self.clone_dialog.as_ref()?;
        let theme = cx.theme().clone();
        let viewport = window.viewport_size();
        let running = d.running;
        let panel = div()
            .id("clone-panel")
            .occlude()
            .w(px(520.))
            .p(px(18.))
            .rounded(px(10.))
            .bg(theme.elevated)
            .border_1()
            .border_color(theme.border)
            .shadow_lg()
            .flex()
            .flex_col()
            .gap(px(10.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().text_size(px(15.)).font_weight(gpui::FontWeight::SEMIBOLD).child("Clone Repository"))
            .child(crate::repo::dialogs::field_label("Repository URL", cx))
            .child(d.url.clone())
            .child(crate::repo::dialogs::field_label("Clone into folder", cx))
            .child(div().flex().gap(px(6.)).child(div().flex_1().child(d.dest.clone())).child(Button::new("browse", "Browse…").on_click(cx.listener(|this, _, window, cx| this.browse_clone_dest(window, cx)))))
            .child(crate::repo::dialogs::field_label("Name", cx))
            .child(d.name.clone())
            .when_some(d.progress.clone(), |el, p| el.child(div().flex().items_center().gap(px(8.)).text_size(px(12.)).text_color(theme.text_muted).child(crate::repo::spinner(cx)).child(div().flex_1().whitespace_nowrap().overflow_hidden().text_ellipsis().child(p))))
            .when_some(d.error.clone(), |el, e| el.child(div().p(px(8.)).rounded(px(5.)).bg(theme.danger.opacity(0.12)).text_size(px(12.)).text_color(theme.danger).child(e)))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .pt(px(4.))
                    .child(Button::new("clone-cancel", "Cancel").disabled(running).on_click(cx.listener(|this, _, window, cx| {
                        this.clone_dialog = None;
                        window.focus(&this.focus_handle);
                        cx.notify();
                    })))
                    .child(Button::new("clone-go", if running { "Cloning…" } else { "Clone" }).primary().disabled(running).on_click(cx.listener(|this, _, window, cx| this.start_clone(window, cx)))),
            );
        Some(
            deferred(
                anchored().position(point(px(0.), px(0.))).child(
                    div()
                        .id("clone-backdrop")
                        .w(viewport.width)
                        .h(viewport.height)
                        .bg(theme.overlay)
                        .occlude()
                        .flex()
                        .justify_center()
                        .items_start()
                        .pt(px(110.))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                if !this.clone_dialog.as_ref().map(|d| d.running).unwrap_or(false) {
                                    this.clone_dialog = None;
                                    cx.notify();
                                }
                            }),
                        )
                        .child(panel),
                ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let title_bar = self.render_title_bar(window, cx);
        let content: AnyElement = match self.active.and_then(|i| self.tabs.get(i)) {
            Some(tab) => tab.clone().into_any_element(),
            None => self.render_manager(window, cx),
        };
        let clone_dialog = self.render_clone_dialog(window, cx);
        div()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .font_family(ui_font())
            .text_size(px(13.))
            .text_color(theme.text)
            .bg(theme.app_bg)
            .on_action(cx.listener(|this, _: &OpenRepository, window, cx| this.prompt_open(window, cx)))
            .on_action(cx.listener(|this, _: &CloneRepository, window, cx| this.open_clone_dialog(window, cx)))
            .on_action(cx.listener(|this, _: &InitRepository, window, cx| this.prompt_init(window, cx)))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| this.activate(None, window, cx)))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                if let Some(ix) = this.active {
                    this.close_tab(ix, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| {
                let n = this.tabs.len() + 1;
                let cur = this.active.map(|i| i + 1).unwrap_or(0);
                let next = (cur + 1) % n;
                this.activate(if next == 0 { None } else { Some(next - 1) }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &PrevTab, window, cx| {
                let n = this.tabs.len() + 1;
                let cur = this.active.map(|i| i + 1).unwrap_or(0);
                let prev = (cur + n - 1) % n;
                this.activate(if prev == 0 { None } else { Some(prev - 1) }, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleTheme, _, cx| this.toggle_theme(cx)))
            .on_action(cx.listener(|this, _: &Cancel, window, cx| {
                if this.clone_dialog.as_ref().map(|d| !d.running).unwrap_or(false) {
                    this.clone_dialog = None;
                    window.focus(&this.focus_handle);
                    cx.notify();
                }
            }))
            .child(title_bar)
            .child(div().flex_1().min_h_0().child(content))
            .children(clone_dialog)
    }
}

pub fn set_theme(mode: ThemeMode, cx: &mut App) {
    cx.set_global(Theme::new(mode));
    cx.update_global::<Settings, _>(|s, _| {
        s.theme = mode;
        s.save();
    });
    cx.refresh_windows();
}

/// Shortens a path for display: the home directory becomes `~`, and long
/// paths keep their start and end with an ellipsis in the middle.
pub fn display_path(path: &std::path::Path, max_chars: usize) -> String {
    let mut s = path.display().to_string();
    if let Some(home) = dirs::home_dir() {
        let home = home.display().to_string();
        if let Some(rest) = s.strip_prefix(&home) {
            s = format!("~{rest}");
        }
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        return s;
    }
    let tail = max_chars * 2 / 3;
    let head = max_chars - tail - 1;
    let start: String = chars[..head].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{start}…{end}")
}

fn guess_repo_name(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let last = trimmed.rsplit(['/', ':', '\\']).next().unwrap_or("");
    last.trim_end_matches(".git").to_string()
}

/// The app's mark: a small branching graph in a rounded square.
pub fn logo(size: gpui::Pixels, theme: &Theme) -> impl IntoElement {
    div()
        .size(size)
        .flex_none()
        .rounded(size * 0.24)
        .bg(theme.accent)
        .flex()
        .items_center()
        .justify_center()
        .child(icon_sized(Icon::Branch, size * 0.62).text_color(theme.text_on_accent))
}
