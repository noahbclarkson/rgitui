use anyhow::Result;
use gpui::{App, Global};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Application settings persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub theme: String,
    pub ai: AiSettings,
    #[serde(default)]
    pub git: GitSettings,
    pub recent_repos: Vec<PathBuf>,
    #[serde(default = "default_max_recent")]
    pub max_recent_repos: usize,
    #[serde(default)]
    pub last_workspace: Vec<PathBuf>,
    #[serde(default)]
    pub layout: LayoutSettings,
}

/// Persisted layout dimensions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutSettings {
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f32,
    #[serde(default = "default_detail_panel_width")]
    pub detail_panel_width: f32,
    #[serde(default = "default_diff_viewer_height")]
    pub diff_viewer_height: f32,
    #[serde(default = "default_commit_input_height")]
    pub commit_input_height: f32,
}

fn default_sidebar_width() -> f32 { 240.0 }
fn default_detail_panel_width() -> f32 { 320.0 }
fn default_diff_viewer_height() -> f32 { 300.0 }
fn default_commit_input_height() -> f32 { 200.0 }

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            sidebar_width: default_sidebar_width(),
            detail_panel_width: default_detail_panel_width(),
            diff_viewer_height: default_diff_viewer_height(),
            commit_input_height: default_commit_input_height(),
        }
    }
}

fn default_max_recent() -> usize {
    20
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiSettings {
    #[serde(default = "default_ai_provider")]
    pub provider: String,
    pub api_key: Option<String>,
    #[serde(default = "default_ai_model")]
    pub model: String,
    #[serde(default = "default_commit_style")]
    pub commit_style: String,
    #[serde(default)]
    pub enabled: bool,
}

fn default_ai_provider() -> String {
    "gemini".into()
}

fn default_ai_model() -> String {
    "gemini-2.0-flash".into()
}

fn default_commit_style() -> String {
    "conventional".into()
}

/// Git authentication and signing settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitSettings {
    #[serde(default)]
    pub https_token: Option<String>,
    #[serde(default)]
    pub ssh_key_path: Option<String>,
    #[serde(default)]
    pub gpg_key_id: Option<String>,
    #[serde(default)]
    pub sign_commits: bool,
}

impl Default for GitSettings {
    fn default() -> Self {
        Self {
            https_token: None,
            ssh_key_path: None,
            gpg_key_id: None,
            sign_commits: false,
        }
    }
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            provider: default_ai_provider(),
            api_key: None,
            model: default_ai_model(),
            commit_style: default_commit_style(),
            enabled: false,
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: "Catppuccin Mocha".into(),
            ai: AiSettings::default(),
            git: GitSettings::default(),
            recent_repos: Vec::new(),
            max_recent_repos: default_max_recent(),
            last_workspace: Vec::new(),
            layout: LayoutSettings::default(),
        }
    }
}

/// Global settings state.
pub struct SettingsState {
    pub settings: AppSettings,
    config_path: PathBuf,
}

impl Global for SettingsState {}

impl SettingsState {
    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    pub fn settings_mut(&mut self) -> &mut AppSettings {
        &mut self.settings
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&self.settings)?;
        std::fs::write(&self.config_path, json)?;
        Ok(())
    }

    pub fn set_last_workspace(&mut self, repos: Vec<PathBuf>) {
        self.settings.last_workspace = repos;
    }

    pub fn add_recent_repo(&mut self, path: PathBuf) {
        self.settings.recent_repos.retain(|p| p != &path);
        self.settings.recent_repos.insert(0, path);
        self.settings
            .recent_repos
            .truncate(self.settings.max_recent_repos);
    }
}

/// Get the config directory path.
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rgitui")
}

/// Initialize settings. Must be called during app init.
pub fn init(cx: &mut App) {
    let config_path = config_dir().join("settings.json");
    let settings = if config_path.exists() {
        match std::fs::read_to_string(&config_path) {
            Ok(json) => serde_json::from_str(&json).unwrap_or_default(),
            Err(_) => AppSettings::default(),
        }
    } else {
        AppSettings::default()
    };

    cx.set_global(SettingsState {
        settings,
        config_path,
    });
}
