use gpui::{rgb, rgba, App, Global, Hsla};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub mode: ThemeMode,
    pub app_bg: Hsla,
    pub sidebar_bg: Hsla,
    pub toolbar_bg: Hsla,
    pub panel_bg: Hsla,
    pub surface: Hsla,
    pub elevated: Hsla,
    pub input_bg: Hsla,
    pub border: Hsla,
    pub border_subtle: Hsla,
    pub border_focus: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_faint: Hsla,
    pub text_on_accent: Hsla,
    pub accent: Hsla,
    pub accent_hover: Hsla,
    pub accent_subtle: Hsla,
    pub hover: Hsla,
    pub active: Hsla,
    pub selection: Hsla,
    pub selection_inactive: Hsla,
    pub text_selection: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
    pub danger_hover: Hsla,
    pub info: Hsla,
    pub status_added: Hsla,
    pub status_modified: Hsla,
    pub status_deleted: Hsla,
    pub status_renamed: Hsla,
    pub status_untracked: Hsla,
    pub status_conflict: Hsla,
    pub diff_add_bg: Hsla,
    pub diff_add_strong: Hsla,
    pub diff_del_bg: Hsla,
    pub diff_del_strong: Hsla,
    pub diff_hunk_bg: Hsla,
    pub diff_gutter: Hsla,
    pub diff_selected: Hsla,
    pub ref_local: Hsla,
    pub ref_remote: Hsla,
    pub ref_tag: Hsla,
    pub ref_head: Hsla,
    pub overlay: Hsla,
    pub graph: [Hsla; 10],
}

impl Global for Theme {}

impl Theme {
    pub fn new(mode: ThemeMode) -> Theme {
        match mode {
            ThemeMode::Dark => Theme::dark(),
            ThemeMode::Light => Theme::light(),
        }
    }

    pub fn dark() -> Theme {
        Theme {
            mode: ThemeMode::Dark,
            app_bg: rgb(0x1b1c1f).into(),
            sidebar_bg: rgb(0x212226).into(),
            toolbar_bg: rgb(0x26282c).into(),
            panel_bg: rgb(0x1b1c1f).into(),
            surface: rgb(0x26282c).into(),
            elevated: rgb(0x2c2e33).into(),
            input_bg: rgb(0x16171a).into(),
            border: rgb(0x34363c).into(),
            border_subtle: rgb(0x2a2c31).into(),
            border_focus: rgb(0x4c8df6).into(),
            text: rgb(0xe4e6ea).into(),
            text_muted: rgb(0xa0a6af).into(),
            text_faint: rgb(0x6c727c).into(),
            text_on_accent: rgb(0xffffff).into(),
            accent: rgb(0x3d7fe6).into(),
            accent_hover: rgb(0x4c8df6).into(),
            accent_subtle: rgba(0x3d7fe633).into(),
            hover: rgba(0xffffff0d).into(),
            active: rgba(0xffffff17).into(),
            selection: rgba(0x3d7fe659).into(),
            selection_inactive: rgba(0xffffff1a).into(),
            text_selection: rgba(0x3d7fe680).into(),
            success: rgb(0x3fb950).into(),
            warning: rgb(0xd8a032).into(),
            danger: rgb(0xe5534b).into(),
            danger_hover: rgb(0xf06a62).into(),
            info: rgb(0x58a6ff).into(),
            status_added: rgb(0x3fb950).into(),
            status_modified: rgb(0xd8a032).into(),
            status_deleted: rgb(0xe5534b).into(),
            status_renamed: rgb(0xa371f7).into(),
            status_untracked: rgb(0x8b949e).into(),
            status_conflict: rgb(0xff6b5e).into(),
            diff_add_bg: rgba(0x2ea04326).into(),
            diff_add_strong: rgba(0x2ea04366).into(),
            diff_del_bg: rgba(0xf8514926).into(),
            diff_del_strong: rgba(0xf8514966).into(),
            diff_hunk_bg: rgba(0x388bfd1a).into(),
            diff_gutter: rgb(0x1f2024).into(),
            diff_selected: rgba(0x3d7fe673).into(),
            ref_local: rgb(0x2f6fd0).into(),
            ref_remote: rgb(0x5a4fbf).into(),
            ref_tag: rgb(0xb07a1c).into(),
            ref_head: rgb(0x2c9a48).into(),
            overlay: rgba(0x0000008c).into(),
            graph: [
                rgb(0x4c8df6).into(),
                rgb(0xe0823d).into(),
                rgb(0x3fb950).into(),
                rgb(0xc75bd6).into(),
                rgb(0xe5534b).into(),
                rgb(0x36b3c2).into(),
                rgb(0xd8b432).into(),
                rgb(0x8a7cf0).into(),
                rgb(0xe36aa0).into(),
                rgb(0x7fb347).into(),
            ],
        }
    }

    pub fn light() -> Theme {
        Theme {
            mode: ThemeMode::Light,
            app_bg: rgb(0xffffff).into(),
            sidebar_bg: rgb(0xf3f4f6).into(),
            toolbar_bg: rgb(0xeceef1).into(),
            panel_bg: rgb(0xffffff).into(),
            surface: rgb(0xf6f7f9).into(),
            elevated: rgb(0xffffff).into(),
            input_bg: rgb(0xffffff).into(),
            border: rgb(0xd5d9de).into(),
            border_subtle: rgb(0xe6e8eb).into(),
            border_focus: rgb(0x2f6fd0).into(),
            text: rgb(0x1f2328).into(),
            text_muted: rgb(0x57606a).into(),
            text_faint: rgb(0x8c959f).into(),
            text_on_accent: rgb(0xffffff).into(),
            accent: rgb(0x2f6fd0).into(),
            accent_hover: rgb(0x3b7ee0).into(),
            accent_subtle: rgba(0x2f6fd022).into(),
            hover: rgba(0x0000000a).into(),
            active: rgba(0x00000014).into(),
            selection: rgba(0x2f6fd033).into(),
            selection_inactive: rgba(0x00000012).into(),
            text_selection: rgba(0x2f6fd04d).into(),
            success: rgb(0x1a7f37).into(),
            warning: rgb(0x9a6700).into(),
            danger: rgb(0xcf222e).into(),
            danger_hover: rgb(0xdd3540).into(),
            info: rgb(0x0969da).into(),
            status_added: rgb(0x1a7f37).into(),
            status_modified: rgb(0x9a6700).into(),
            status_deleted: rgb(0xcf222e).into(),
            status_renamed: rgb(0x8250df).into(),
            status_untracked: rgb(0x6e7781).into(),
            status_conflict: rgb(0xd1242f).into(),
            diff_add_bg: rgba(0x2da44e1f).into(),
            diff_add_strong: rgba(0x2da44e4d).into(),
            diff_del_bg: rgba(0xcf222e1a).into(),
            diff_del_strong: rgba(0xcf222e40).into(),
            diff_hunk_bg: rgba(0x0969da14).into(),
            diff_gutter: rgb(0xf6f8fa).into(),
            diff_selected: rgba(0x2f6fd040).into(),
            ref_local: rgb(0x2f6fd0).into(),
            ref_remote: rgb(0x6b5bd6).into(),
            ref_tag: rgb(0xb07a1c).into(),
            ref_head: rgb(0x1f8a3f).into(),
            overlay: rgba(0x00000040).into(),
            graph: [
                rgb(0x2f6fd0).into(),
                rgb(0xd0691e).into(),
                rgb(0x1f8a3f).into(),
                rgb(0xa53bb8).into(),
                rgb(0xcf222e).into(),
                rgb(0x13889a).into(),
                rgb(0xb08a00).into(),
                rgb(0x6b5bd6).into(),
                rgb(0xc8457e).into(),
                rgb(0x5c8f22).into(),
            ],
        }
    }

    pub fn graph_color(&self, ix: u8) -> Hsla {
        self.graph[ix as usize % self.graph.len()]
    }
}

pub trait ActiveTheme {
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Theme {
        self.global::<Theme>()
    }
}

pub fn ui_font() -> &'static str {
    if cfg!(target_os = "windows") {
        "Segoe UI"
    } else if cfg!(target_os = "macos") {
        ".SystemUIFont"
    } else {
        "Ubuntu"
    }
}

pub fn mono_font() -> &'static str {
    if cfg!(target_os = "windows") {
        "Consolas"
    } else if cfg!(target_os = "macos") {
        "Menlo"
    } else {
        "DejaVu Sans Mono"
    }
}
