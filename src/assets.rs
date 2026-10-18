//! Embedded SVG icons. GPUI renders SVGs as alpha masks tinted with the
//! element's text color, so the stroke color here does not matter.

use anyhow::Result;
use gpui::{AssetSource, SharedString};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Branch,
    Tag,
    Cloud,
    Stash,
    StashPop,
    Folder,
    FolderOpen,
    File,
    Pull,
    Push,
    Fetch,
    Refresh,
    Merge,
    Rebase,
    Plus,
    Minus,
    Check,
    Close,
    ChevronRight,
    ChevronDown,
    Search,
    Settings,
    History,
    WorkingCopy,
    Commit,
    Warning,
    Info,
    Trash,
    Copy,
    Terminal,
    Reveal,
    Undo,
    List,
    Package,
    WinMinimize,
    WinMaximize,
    WinRestore,
    WinClose,
    Sun,
    Moon,
    Repo,
    User,
    Download,
    ArrowUp,
    ArrowDown,
    Eye,
    Cherry,
    Pencil,
    Dot,
    Globe,
    PullRequest,
    DiffInline,
    DiffSplit,
    Image,
}

impl Icon {
    pub fn path(self) -> SharedString {
        format!("icons/{}.svg", self.name()).into()
    }

    fn name(self) -> &'static str {
        match self {
            Icon::Branch => "branch",
            Icon::Tag => "tag",
            Icon::Cloud => "cloud",
            Icon::Stash => "stash",
            Icon::StashPop => "stash-pop",
            Icon::Folder => "folder",
            Icon::FolderOpen => "folder-open",
            Icon::File => "file",
            Icon::Pull => "pull",
            Icon::Push => "push",
            Icon::Fetch => "fetch",
            Icon::Refresh => "refresh",
            Icon::Merge => "merge",
            Icon::Rebase => "rebase",
            Icon::Plus => "plus",
            Icon::Minus => "minus",
            Icon::Check => "check",
            Icon::Close => "close",
            Icon::ChevronRight => "chevron-right",
            Icon::ChevronDown => "chevron-down",
            Icon::Search => "search",
            Icon::Settings => "settings",
            Icon::History => "history",
            Icon::WorkingCopy => "working-copy",
            Icon::Commit => "commit",
            Icon::Warning => "warning",
            Icon::Info => "info",
            Icon::Trash => "trash",
            Icon::Copy => "copy",
            Icon::Terminal => "terminal",
            Icon::Reveal => "reveal",
            Icon::Undo => "undo",
            Icon::List => "list",
            Icon::Package => "package",
            Icon::WinMinimize => "win-minimize",
            Icon::WinMaximize => "win-maximize",
            Icon::WinRestore => "win-restore",
            Icon::WinClose => "win-close",
            Icon::Sun => "sun",
            Icon::Moon => "moon",
            Icon::Repo => "repo",
            Icon::User => "user",
            Icon::Download => "download",
            Icon::ArrowUp => "arrow-up",
            Icon::ArrowDown => "arrow-down",
            Icon::Eye => "eye",
            Icon::Cherry => "cherry",
            Icon::Pencil => "pencil",
            Icon::Dot => "dot",
            Icon::Globe => "globe",
            Icon::PullRequest => "pull-request",
            Icon::DiffInline => "diff-inline",
            Icon::DiffSplit => "diff-split",
            Icon::Image => "image",
        }
    }
}

const OPEN: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">"##;
const OPEN_THIN: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1" stroke-linecap="round" stroke-linejoin="round">"##;

fn body(name: &str) -> Option<(&'static str, &'static str)> {
    Some(match name {
        "branch" => (OPEN, r#"<circle cx="4.5" cy="3.5" r="1.6"/><circle cx="4.5" cy="12.5" r="1.6"/><circle cx="11.5" cy="4.5" r="1.6"/><path d="M4.5 5.1v5.8M11.5 6.1c0 3.4-7 2.4-7 4.8"/>"#),
        "tag" => (OPEN, r##"<path d="M2.2 2.2h5l6.6 6.6a1 1 0 0 1 0 1.4l-3.6 3.6a1 1 0 0 1-1.4 0L2.2 7.2z"/><circle cx="5.2" cy="5.2" r="1" fill="#000"/>"##),
        "cloud" => (OPEN, r#"<path d="M4.6 12.8h6.9a3 3 0 0 0 .4-6 4.4 4.4 0 0 0-8.5 1.2 2.4 2.4 0 0 0 1.2 4.8z"/>"#),
        "stash" => (OPEN, r#"<rect x="1.8" y="2.5" width="12.4" height="3.4" rx="1"/><path d="M2.8 5.9v6.6a1 1 0 0 0 1 1h8.4a1 1 0 0 0 1-1V5.9M6.4 8.8h3.2"/>"#),
        "stash-pop" => (OPEN, r#"<path d="M2.8 7.5v5a1 1 0 0 0 1 1h8.4a1 1 0 0 0 1-1v-5M8 10.5V1.8M5.2 4.5 8 1.8l2.8 2.7"/>"#),
        "folder" => (OPEN, r#"<path d="M1.8 4.2a1 1 0 0 1 1-1h3.4l1.6 1.8h5.4a1 1 0 0 1 1 1v6.8a1 1 0 0 1-1 1H2.8a1 1 0 0 1-1-1z"/>"#),
        "folder-open" => (OPEN, r#"<path d="M1.8 12V4.2a1 1 0 0 1 1-1h3.4l1.6 1.8h4.6a1 1 0 0 1 1 1v1.2"/><path d="M1.8 12.8 3.6 7.6a1 1 0 0 1 .9-.6h9.2a.6.6 0 0 1 .6.8l-1.6 4.4a1 1 0 0 1-.9.6z"/>"#),
        "file" => (OPEN, r#"<path d="M9.2 1.8H4.4a1 1 0 0 0-1 1v10.4a1 1 0 0 0 1 1h7.2a1 1 0 0 0 1-1V5.2z"/><path d="M9.2 1.8v3.4h3.4"/>"#),
        "pull" => (OPEN, r#"<path d="M8 1.8v8.6M4.4 7 8 10.6 11.6 7M2.8 13.8h10.4"/>"#),
        "push" => (OPEN, r#"<path d="M8 13.8V5.2M4.4 8.6 8 5l3.6 3.6M2.8 2.2h10.4"/>"#),
        "fetch" => (OPEN, r#"<path d="M13.4 8.6a5.5 5.5 0 0 1-9.8 2.8M2.6 7.4a5.5 5.5 0 0 1 9.8-2.8"/><path d="M12.6 1.6v3h-3M3.4 14.4v-3h3"/>"#),
        "refresh" => (OPEN, r#"<path d="M13.4 8a5.4 5.4 0 1 1-1.6-3.8"/><path d="M13.4 1.8v2.8h-2.8"/>"#),
        "merge" => (OPEN, r#"<circle cx="4.5" cy="3.5" r="1.6"/><circle cx="4.5" cy="12.5" r="1.6"/><circle cx="11.5" cy="9.5" r="1.6"/><path d="M4.5 5.1v5.8M4.5 5.1c0 2.6 2.6 4.4 5.4 4.4"/>"#),
        "rebase" => (OPEN, r#"<circle cx="4.5" cy="12.5" r="1.6"/><circle cx="4.5" cy="3.5" r="1.6"/><circle cx="11.5" cy="3.5" r="1.6"/><path d="M4.5 5.1v5.8M11.5 5.1v2.4a2 2 0 0 1-2 2H6.4"/><path d="M7.8 7.9 6.2 9.5l1.6 1.6"/>"#),
        "plus" => (OPEN, r#"<path d="M8 3v10M3 8h10"/>"#),
        "minus" => (OPEN, r#"<path d="M3 8h10"/>"#),
        "check" => (OPEN, r#"<path d="M3 8.4 6.4 11.8 13 4.6"/>"#),
        "close" => (OPEN, r#"<path d="M4 4l8 8M12 4l-8 8"/>"#),
        "chevron-right" => (OPEN, r#"<path d="M6 3.5 10.5 8 6 12.5"/>"#),
        "chevron-down" => (OPEN, r#"<path d="M3.5 6 8 10.5 12.5 6"/>"#),
        "search" => (OPEN, r#"<circle cx="7" cy="7" r="4.6"/><path d="m10.4 10.4 3.6 3.6"/>"#),
        "settings" => (OPEN, r#"<path d="M2.2 4h6M11.8 4h2M2.2 8h2M7.8 8h6M2.2 12h7M12.6 12h1.2"/><circle cx="10" cy="4" r="1.6"/><circle cx="6" cy="8" r="1.6"/><circle cx="11" cy="12" r="1.6"/>"#),
        "history" => (OPEN, r#"<circle cx="8" cy="8" r="6"/><path d="M8 4.6V8l2.4 1.6"/>"#),
        "working-copy" => (OPEN, r#"<rect x="1.8" y="2.4" width="12.4" height="8.6" rx="1"/><path d="M5.4 14h5.2M8 11v3"/>"#),
        "commit" => (OPEN, r#"<circle cx="8" cy="8" r="2.6"/><path d="M1.6 8h3.8M10.6 8h3.8"/>"#),
        "warning" => (OPEN, r#"<path d="M7.1 2.6a1 1 0 0 1 1.8 0l5.4 9.6a1 1 0 0 1-.9 1.5H2.6a1 1 0 0 1-.9-1.5z"/><path d="M8 6.2v3.2M8 11.6v.1"/>"#),
        "info" => (OPEN, r#"<circle cx="8" cy="8" r="6"/><path d="M8 7.4v3.8M8 4.9v.1"/>"#),
        "trash" => (OPEN, r#"<path d="M2.4 4h11.2M6 4V2.4h4V4M3.8 4l.7 9.4a1 1 0 0 0 1 .9h5a1 1 0 0 0 1-.9l.7-9.4"/>"#),
        "copy" => (OPEN, r#"<rect x="5.2" y="5.2" width="8.6" height="8.6" rx="1.2"/><path d="M10.8 5.2V3.4a1 1 0 0 0-1-1H3.4a1 1 0 0 0-1 1v6.4a1 1 0 0 0 1 1h1.8"/>"#),
        "terminal" => (OPEN, r#"<rect x="1.6" y="2.4" width="12.8" height="11.2" rx="1.2"/><path d="M4.6 6 6.6 8l-2 2M8.4 10.4h3"/>"#),
        "reveal" => (OPEN, r#"<path d="M9.2 2.2h4.6v4.6M13.8 2.2 7.6 8.4M12 9.6v3.2a1 1 0 0 1-1 1H3.2a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h3.2"/>"#),
        "undo" => (OPEN, r#"<path d="M5.4 2.8 2.4 5.8l3 3"/><path d="M2.4 5.8h7.4a3.6 3.6 0 0 1 0 7.2H7"/>"#),
        "list" => (OPEN, r#"<path d="M5.6 4h8M5.6 8h8M5.6 12h8M2.4 4h.1M2.4 8h.1M2.4 12h.1"/>"#),
        "package" => (OPEN, r#"<path d="M8 1.6 14 4.6v6.8L8 14.4 2 11.4V4.6z"/><path d="M2 4.6 8 7.6l6-3M8 7.6v6.8"/>"#),
        "win-minimize" => (OPEN_THIN, r#"<path d="M3 8.5h10"/>"#),
        "win-maximize" => (OPEN_THIN, r#"<rect x="3.5" y="3.5" width="9" height="9"/>"#),
        "win-restore" => (OPEN_THIN, r#"<rect x="3.5" y="5.5" width="7" height="7"/><path d="M5.5 5.5v-2h7v7h-2"/>"#),
        "win-close" => (OPEN_THIN, r#"<path d="M3.5 3.5l9 9M12.5 3.5l-9 9"/>"#),
        "sun" => (OPEN, r#"<circle cx="8" cy="8" r="2.8"/><path d="M8 1.2v1.4M8 13.4v1.4M1.2 8h1.4M13.4 8h1.4M3.2 3.2l1 1M11.8 11.8l1 1M3.2 12.8l1-1M11.8 4.2l1-1"/>"#),
        "moon" => (OPEN, r#"<path d="M13.6 9.6A6 6 0 1 1 6.4 2.4a4.6 4.6 0 0 0 7.2 7.2z"/>"#),
        "repo" => (OPEN, r#"<path d="M3 13V3.4a1 1 0 0 1 1-1h8.6v9.4H4.2A1.2 1.2 0 0 0 3 13a1.2 1.2 0 0 0 1.2 1.2h8.4"/><path d="M6 5.4h3.6"/>"#),
        "user" => (OPEN, r#"<circle cx="8" cy="5.4" r="2.8"/><path d="M2.6 14a5.4 5.4 0 0 1 10.8 0"/>"#),
        "download" => (OPEN, r#"<path d="M8 1.8v8M4.6 6.6 8 10l3.4-3.4M2.4 10.8v2a1 1 0 0 0 1 1h9.2a1 1 0 0 0 1-1v-2"/>"#),
        "arrow-up" => (OPEN, r#"<path d="M8 13V3M4 7l4-4 4 4"/>"#),
        "arrow-down" => (OPEN, r#"<path d="M8 3v10M4 9l4 4 4-4"/>"#),
        "eye" => (OPEN, r#"<path d="M1.4 8S3.8 3.2 8 3.2 14.6 8 14.6 8 12.2 12.8 8 12.8 1.4 8 1.4 8z"/><circle cx="8" cy="8" r="2"/>"#),
        "cherry" => (OPEN, r#"<circle cx="4.8" cy="11.4" r="2.4"/><circle cx="11.2" cy="11.4" r="2.4"/><path d="M4.8 9c0-3 2.2-6 6.6-7.4M11.2 9c0-2.2-.8-4.4-2.4-5.8"/>"#),
        "pencil" => (OPEN, r#"<path d="M10.8 2.4a1.4 1.4 0 0 1 2 0l.8.8a1.4 1.4 0 0 1 0 2L6 12.8 2.4 13.6l.8-3.6z"/><path d="M9.6 3.6l2.8 2.8"/>"#),
        "dot" => (OPEN, r##"<circle cx="8" cy="8" r="3" fill="#000"/>"##),
        "globe" => (OPEN, r#"<circle cx="8" cy="8" r="6"/><path d="M2 8h12M8 2c1.6 1.8 2.4 3.8 2.4 6S9.6 12.2 8 14c-1.6-1.8-2.4-3.8-2.4-6S6.4 3.8 8 2z"/>"#),
        "diff-inline" => (OPEN, r#"<rect x="2" y="2.4" width="12" height="11.2" rx="1.2"/><path d="M4.6 6h6.8M4.6 8h6.8M4.6 10h4.4"/>"#),
        "diff-split" => (OPEN, r#"<rect x="2" y="2.4" width="12" height="11.2" rx="1.2"/><path d="M8 2.4v11.2M4.2 6h2M4.2 8.4h2M9.8 6h2M9.8 8.4h2"/>"#),
        "image" => (OPEN, r#"<rect x="2" y="2.4" width="12" height="11.2" rx="1.2"/><circle cx="5.6" cy="6" r="1.1"/><path d="m2.4 12.2 3.6-3.6 2.4 2.4 1.8-1.8 3.4 3.4"/>"#),
        "pull-request" => (OPEN, r#"<circle cx="4.5" cy="3.5" r="1.6"/><circle cx="4.5" cy="12.5" r="1.6"/><circle cx="11.5" cy="12.5" r="1.6"/><path d="M4.5 5.1v5.8M11.5 10.9V6.2a1.6 1.6 0 0 0-1.6-1.6H7.2"/><path d="M8.8 3 7.2 4.6l1.6 1.6"/>"#),
        _ => return None,
    })
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let Some(name) = path.strip_prefix("icons/").and_then(|p| p.strip_suffix(".svg")) else {
            return Ok(None);
        };
        Ok(body(name).map(|(open, inner)| Cow::Owned(format!("{open}{inner}</svg>").into_bytes())))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}
