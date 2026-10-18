// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod actions;
mod assets;
mod git;
mod repo;
mod settings;
mod settings_window;
mod theme;
mod ui;
mod workspace;

use actions::*;
use gpui::{point, px, size, App, AppContext, Application, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds, WindowDecorations, WindowOptions};
use settings::Settings;
use theme::Theme;
use workspace::Workspace;

fn main() {
    let settings = Settings::load();
    git::set_git_binary(settings.git_path.clone());

    Application::new().with_assets(assets::Assets).run(move |cx: &mut App| {
        cx.set_global(Theme::new(settings.theme));
        cx.set_global(settings);

        actions::bind_keys(cx);
        ui::text_input::bind_keys(cx);
        cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &OpenSettings, cx| settings_window::open(cx));
        cx.on_action(|_: &ToggleTheme, cx| {
            let mode = match cx.global::<Theme>().mode {
                theme::ThemeMode::Dark => theme::ThemeMode::Light,
                theme::ThemeMode::Light => theme::ThemeMode::Dark,
            };
            workspace::set_theme(mode, cx);
        });

        cx.set_menus(vec![
            Menu {
                name: "Gitter".into(),
                items: vec![MenuItem::action("Settings…", OpenSettings), MenuItem::separator(), MenuItem::action("Quit Gitter", Quit)],
            },
            Menu {
                name: "File".into(),
                items: vec![
                    MenuItem::action("Open Repository…", OpenRepository),
                    MenuItem::action("Clone Repository…", CloneRepository),
                    MenuItem::action("New Repository…", InitRepository),
                    MenuItem::separator(),
                    MenuItem::action("Repositories", NewTab),
                    MenuItem::action("Close Tab", CloseTab),
                ],
            },
            Menu {
                name: "Repository".into(),
                items: vec![
                    MenuItem::action("Fetch", Fetch),
                    MenuItem::action("Pull", Pull),
                    MenuItem::action("Push", Push),
                    MenuItem::separator(),
                    MenuItem::action("New Branch…", NewBranch),
                    MenuItem::action("Merge…", MergeBranch),
                    MenuItem::action("Stash Changes…", StashChanges),
                    MenuItem::separator(),
                    MenuItem::action("Quick Actions…", QuickActions),
                    MenuItem::action("Refresh", Refresh),
                ],
            },
            Menu {
                name: "View".into(),
                items: vec![
                    MenuItem::action("Working Copy", ShowWorkingCopy),
                    MenuItem::action("History", ShowHistory),
                    MenuItem::action("Stashes", ShowStashes),
                    MenuItem::action("Reflog", ShowReflog),
                    MenuItem::separator(),
                    MenuItem::action("Toggle Theme", ToggleTheme),
                ],
            },
        ]);

        let bounds = Bounds::centered(None, size(px(1360.), px(860.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions { title: Some("Gitter".into()), appears_transparent: true, traffic_light_position: Some(point(px(12.), px(12.))) }),
                    window_min_size: Some(size(px(900.), px(560.))),
                    window_decorations: Some(WindowDecorations::Server),
                    app_id: Some("gitter".into()),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Workspace::new(window, cx)),
            )
            .expect("failed to open window");
        window.update(cx, |_, window, _| window.activate_window()).ok();
        cx.activate(true);

        // Quit when the main window closes, even if the settings window is still open.
        cx.on_window_closed(move |cx| {
            if !cx.windows().iter().any(|w| w.window_id() == window.window_id()) {
                cx.quit();
            }
        })
        .detach();
    });
}
