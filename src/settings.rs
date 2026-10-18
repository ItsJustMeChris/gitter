//! Persistent application settings, stored as JSON in the platform config dir.

use crate::theme::ThemeMode;
use gpui::Global;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RecentRepo {
    pub path: PathBuf,
    pub name: String,
    pub last_opened: i64,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeMode,
    pub recent_repos: Vec<RecentRepo>,
    pub open_tabs: Vec<PathBuf>,
    pub active_tab: Option<usize>,
    pub sidebar_width: f32,
    pub history_split: f32,
    pub details_split: f32,
    pub working_copy_split: f32,
    pub pull_rebase: bool,
    pub git_path: Option<String>,
    pub diff_context: u32,
    /// Show diffs as old/new columns instead of a single unified column.
    pub diff_side_by_side: bool,
    pub auto_fetch_minutes: u32,
    pub clone_directory: Option<PathBuf>,
    pub show_all_branches: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeMode::Dark,
            recent_repos: Vec::new(),
            open_tabs: Vec::new(),
            active_tab: None,
            sidebar_width: 250.,
            history_split: 0.5,
            details_split: 0.36,
            working_copy_split: 0.38,
            pull_rebase: false,
            git_path: None,
            diff_context: 3,
            diff_side_by_side: false,
            auto_fetch_minutes: 10,
            clone_directory: None,
            show_all_branches: true,
        }
    }
}

impl Global for Settings {}

impl Settings {
    fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("gitter").join("settings.json"))
    }

    pub fn load() -> Settings {
        let Some(path) = Self::path() else { return Settings::default() };
        std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }

    pub fn touch_recent(&mut self, path: &Path, name: &str) {
        let now = chrono::Utc::now().timestamp();
        if let Some(r) = self.recent_repos.iter_mut().find(|r| r.path == path) {
            r.last_opened = now;
            r.name = name.to_string();
        } else {
            self.recent_repos.push(RecentRepo { path: path.to_path_buf(), name: name.to_string(), last_opened: now, pinned: false });
        }
        self.recent_repos.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.last_opened.cmp(&a.last_opened)));
        self.recent_repos.truncate(60);
    }

    pub fn remove_recent(&mut self, path: &Path) {
        self.recent_repos.retain(|r| r.path != path);
    }

    pub fn default_clone_dir(&self) -> PathBuf {
        self.clone_directory
            .clone()
            .or_else(|| dirs::home_dir().map(|h| h.join("Documents").join("GitHub")).filter(|p| p.exists()))
            .or_else(|| dirs::home_dir().map(|h| h.join("Projects")))
            .unwrap_or_else(|| PathBuf::from("."))
    }
}
