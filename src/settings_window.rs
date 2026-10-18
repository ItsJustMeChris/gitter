//! Preferences, shown in their own window.

use crate::git;
use crate::settings::Settings;
use crate::theme::{ui_font, ActiveTheme, ThemeMode};
use crate::ui::text_input::TextInput;
use crate::ui::{section_label, Button, Checkbox};
use crate::workspace::set_theme;
use gpui::{div, prelude::*, px, size, App, Bounds, Context, Entity, FocusHandle, Focusable, SharedString, TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions};
use std::cell::RefCell;

thread_local! {
    static OPEN: RefCell<Option<WindowHandle<SettingsWindow>>> = const { RefCell::new(None) };
}

pub fn open(cx: &mut App) {
    let existing = OPEN.with(|o| *o.borrow());
    if let Some(handle) = existing {
        if handle.update(cx, |_, window, _| window.activate_window()).is_ok() {
            return;
        }
    }
    let bounds = Bounds::centered(None, size(px(560.), px(620.)), cx);
    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("Gitter Settings".into()), appears_transparent: false, traffic_light_position: None }),
            window_min_size: Some(size(px(440.), px(420.))),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| SettingsWindow::new(window, cx)),
    );
    if let Ok(handle) = handle {
        OPEN.with(|o| *o.borrow_mut() = Some(handle));
    }
}

pub struct SettingsWindow {
    focus: FocusHandle,
    name: Entity<TextInput>,
    email: Entity<TextInput>,
    git_path: Entity<TextInput>,
    saved: Option<SharedString>,
}

impl SettingsWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = cx.global::<Settings>().clone();
        let name = cx.new(|cx| TextInput::new(cx).with_placeholder("Your Name").with_text(git::global_config_get("user.name").unwrap_or_default()));
        let email = cx.new(|cx| TextInput::new(cx).with_placeholder("you@example.com").with_text(git::global_config_get("user.email").unwrap_or_default()));
        let git_path = cx.new(|cx| TextInput::new(cx).with_placeholder("git (from PATH)").with_text(settings.git_path.clone().unwrap_or_default()));
        window.focus(&name.focus_handle(cx));
        Self { focus: cx.focus_handle(), name, email, git_path, saved: None }
    }

    fn save_identity(&mut self, cx: &mut Context<Self>) {
        let name = self.name.read(cx).text().trim().to_string();
        let email = self.email.read(cx).text().trim().to_string();
        let git_path = self.git_path.read(cx).text().trim().to_string();
        let result = git::global_config_set("user.name", &name).and_then(|_| git::global_config_set("user.email", &email));
        cx.update_global::<Settings, _>(|s, _| {
            s.git_path = (!git_path.is_empty()).then(|| git_path.clone());
            s.save();
        });
        self.saved = Some(match result {
            Ok(_) => "Saved.".into(),
            Err(e) => format!("Could not save: {e}").into(),
        });
        cx.refresh_windows();
        cx.notify();
    }

    fn update(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Settings)) {
        cx.update_global::<Settings, _>(|s, _| {
            f(s);
            s.save();
        });
        cx.refresh_windows();
    }
}

impl Focusable for SettingsWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let s = cx.global::<Settings>().clone();
        let version = git::git_version().unwrap_or_else(|_| "git not found".into());

        let choice = |id: (&'static str, usize), label: String, selected: bool| Button::new(id, label).small().selected(selected);

        let mut fetch_row = div().flex().gap(px(6.));
        for (i, m) in [0u32, 5, 10, 30].into_iter().enumerate() {
            fetch_row = fetch_row.child(choice(("fetch", i), if m == 0 { "Off".into() } else { format!("{m} min") }, s.auto_fetch_minutes == m).on_click(cx.listener(move |this, _, _, cx| this.update(cx, |s| s.auto_fetch_minutes = m))));
        }
        let mut ctx_row = div().flex().gap(px(6.));
        for (i, n) in [1u32, 3, 5, 10].into_iter().enumerate() {
            ctx_row = ctx_row.child(choice(("ctx", i), format!("{n} line{}", if n == 1 { "" } else { "s" }), s.diff_context == n).on_click(cx.listener(move |this, _, _, cx| this.update(cx, |s| s.diff_context = n))));
        }
        let section = |title: &'static str, cx: &App| div().mt(px(8.)).child(section_label(title, cx));
        let row = |label: &'static str, control: gpui::AnyElement| div().flex().items_center().gap(px(12.)).child(div().w(px(130.)).flex_none().text_color(theme.text_muted).child(label)).child(div().flex_1().child(control));

        div()
            .id("settings")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .bg(theme.app_bg)
            .text_color(theme.text)
            .font_family(ui_font())
            .text_size(px(13.))
            .p(px(24.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(div().text_size(px(20.)).font_weight(gpui::FontWeight::BOLD).child("Settings"))
            .child(section("APPEARANCE", cx))
            .child(row(
                "Theme",
                div()
                    .flex()
                    .gap(px(6.))
                    .child(choice(("theme", 0), "Dark".into(), s.theme == ThemeMode::Dark).on_click(cx.listener(|_, _, _, cx| set_theme(ThemeMode::Dark, cx))))
                    .child(choice(("theme", 1), "Light".into(), s.theme == ThemeMode::Light).on_click(cx.listener(|_, _, _, cx| set_theme(ThemeMode::Light, cx))))
                    .into_any_element(),
            ))
            .child(section("GIT IDENTITY (GLOBAL)", cx))
            .child(row("Name", self.name.clone().into_any_element()))
            .child(row("Email", self.email.clone().into_any_element()))
            .child(row("Git executable", self.git_path.clone().into_any_element()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().w(px(130.)))
                    .child(Button::new("save-identity", "Save").primary().on_click(cx.listener(|this, _, _, cx| this.save_identity(cx))))
                    .when_some(self.saved.clone(), |d, m| d.child(div().text_size(px(12.)).text_color(theme.text_muted).child(m))),
            )
            .child(section("BEHAVIOR", cx))
            .child(row(
                "Pulling",
                Checkbox::new("pull-rebase", s.pull_rebase).label("Rebase instead of merge when pulling").on_toggle(cx.listener(|this, v: &bool, _, cx| {
                    let v = *v;
                    this.update(cx, |s| s.pull_rebase = v)
                })).into_any_element(),
            ))
            .child(row("Auto-fetch", fetch_row.into_any_element()))
            .child(row("Diff context", ctx_row.into_any_element()))
            .child(section("ABOUT", cx))
            .child(div().text_size(px(12.)).text_color(theme.text_faint).child(format!("Gitter {} · {version}", env!("CARGO_PKG_VERSION"))))
            .child(div().text_size(px(12.)).text_color(theme.text_faint).child("Git executable changes apply after restarting Gitter."))
    }
}
