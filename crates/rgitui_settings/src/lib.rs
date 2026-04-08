use anyhow::Result;
use chrono::{DateTime, Utc};
use gpui::{App, Global};
use keyring::Entry;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Mutex, OnceLock, RwLock};
use uuid::Uuid;

/// Controls the compactness of the UI layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Compactness {
    Compact,
    #[default]
    Default,
    Comfortable,
}

/// How diffs are displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DiffViewMode {
    #[default]
    Unified,
    SideBySide,
}

impl fmt::Display for DiffViewMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiffViewMode::Unified => write!(f, "Unified"),
            DiffViewMode::SideBySide => write!(f, "Side-by-Side"),
        }
    }
}

impl FromStr for DiffViewMode {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "unified" => Ok(DiffViewMode::Unified),
            "side_by_side" | "side by side" => Ok(DiffViewMode::SideBySide),
            _ => Err(format!("Unknown diff view mode: {}", s)),
        }
    }
}

/// The visual style used for the commit graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GraphStyle {
    Rails,
    #[default]
    Curved,
    Angular,
}

impl fmt::Display for GraphStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphStyle::Rails => write!(f, "Rails"),
            GraphStyle::Curved => write!(f, "Curved"),
            GraphStyle::Angular => write!(f, "Angular"),
        }
    }
}

impl FromStr for GraphStyle {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "rails" => Ok(GraphStyle::Rails),
            "curved" => Ok(GraphStyle::Curved),
            "angular" => Ok(GraphStyle::Angular),
            _ => Err(format!("Unknown graph style: {}", s)),
        }
    }
}

/// How often the application should automatically fetch from remotes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoFetchInterval {
    #[default]
    Disabled,
    OneMinute,
    FiveMinutes,
    FifteenMinutes,
    ThirtyMinutes,
}

impl fmt::Display for AutoFetchInterval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AutoFetchInterval::Disabled => write!(f, "Disabled"),
            AutoFetchInterval::OneMinute => write!(f, "1 min"),
            AutoFetchInterval::FiveMinutes => write!(f, "5 min"),
            AutoFetchInterval::FifteenMinutes => write!(f, "15 min"),
            AutoFetchInterval::ThirtyMinutes => write!(f, "30 min"),
        }
    }
}

impl FromStr for AutoFetchInterval {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "disabled" => Ok(AutoFetchInterval::Disabled),
            "1 min" | "one_minute" => Ok(AutoFetchInterval::OneMinute),
            "5 min" | "five_minutes" => Ok(AutoFetchInterval::FiveMinutes),
            "15 min" | "fifteen_minutes" => Ok(AutoFetchInterval::FifteenMinutes),
            "30 min" | "thirty_minutes" => Ok(AutoFetchInterval::ThirtyMinutes),
            _ => Err(format!("Unknown auto-fetch interval: {}", s)),
        }
    }
}

impl Compactness {
    pub fn multiplier(&self) -> f32 {
        match self {
            Compactness::Compact => 0.75,
            Compactness::Default => 1.0,
            Compactness::Comfortable => 1.25,
        }
    }

    pub fn spacing(&self, base: f32) -> f32 {
        base * self.multiplier()
    }
}

impl fmt::Display for Compactness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Compactness::Compact => write!(f, "Compact"),
            Compactness::Default => write!(f, "Default"),
            Compactness::Comfortable => write!(f, "Comfortable"),
        }
    }
}

impl FromStr for Compactness {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "compact" => Ok(Compactness::Compact),
            "default" => Ok(Compactness::Default),
            "comfortable" => Ok(Compactness::Comfortable),
            _ => Err(format!("Unknown compactness value: {}", s)),
        }
    }
}

/// Application settings persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default = "default_settings_version")]
    pub version: u32,
    pub theme: String,
    #[serde(default)]
    pub ui_font: String,
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
    #[serde(default)]
    pub workspaces: Vec<StoredWorkspace>,
    #[serde(default)]
    pub active_workspace_id: Option<String>,
    #[serde(default)]
    pub clean_exit: bool,
    #[serde(default)]
    pub compactness: Compactness,
    #[serde(default)]
    pub terminal_command: String,
    #[serde(default)]
    pub editor_command: String,
    #[serde(default = "default_font_size")]
    pub font_size: u32,
    #[serde(default = "default_show_line_numbers_in_diff")]
    pub show_line_numbers_in_diff: bool,
    #[serde(default)]
    pub diff_view_mode: DiffViewMode,
    #[serde(default)]
    pub graph_style: GraphStyle,
    #[serde(default = "default_show_subject_column")]
    pub show_subject_column: bool,
    #[serde(default)]
    pub auto_fetch_interval: AutoFetchInterval,
    #[serde(default = "default_confirm_destructive")]
    pub confirm_destructive_operations: bool,
    #[serde(default = "default_auto_check_updates")]
    pub auto_check_updates: bool,
    /// Timestamp of the most recent successful GitHub release check. Used to
    /// throttle background update polling so restarts don't spam the API.
    #[serde(default)]
    pub last_update_check_at: Option<DateTime<Utc>>,
}

/// Current settings version. Increment when making breaking changes.
const CURRENT_SETTINGS_VERSION: u32 = 1;

fn default_settings_version() -> u32 {
    CURRENT_SETTINGS_VERSION
}

fn default_font_size() -> u32 {
    14
}

fn default_show_line_numbers_in_diff() -> bool {
    true
}

fn default_show_subject_column() -> bool {
    true
}

fn default_confirm_destructive() -> bool {
    true
}

fn default_auto_check_updates() -> bool {
    true
}

/// A persisted local workspace snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoredWorkspace {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub repos: Vec<PathBuf>,
    #[serde(default)]
    pub active_repo_index: usize,
    #[serde(default)]
    pub layout: LayoutSettings,
    #[serde(default = "utc_now")]
    pub created_at: DateTime<Utc>,
    #[serde(default = "utc_now")]
    pub last_opened_at: DateTime<Utc>,
}

/// Persisted layout dimensions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

fn default_sidebar_width() -> f32 {
    240.0
}
fn default_detail_panel_width() -> f32 {
    352.0
}
fn default_diff_viewer_height() -> f32 {
    345.0
}
fn default_commit_input_height() -> f32 {
    385.0
}

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
    #[serde(rename = "api_key", default, skip_serializing)]
    pub legacy_api_key: Option<String>,
    #[serde(default)]
    pub has_api_key: bool,
    #[serde(default = "default_ai_model")]
    pub model: String,
    #[serde(default = "default_commit_style")]
    pub commit_style: String,
    #[serde(default = "default_ai_enabled")]
    pub enabled: bool,
    #[serde(default = "default_inject_project_context")]
    pub inject_project_context: bool,
    #[serde(default = "default_use_tools")]
    pub use_tools: bool,
}

fn default_use_tools() -> bool {
    true
}

fn default_ai_enabled() -> bool {
    true
}

fn default_inject_project_context() -> bool {
    true
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
    #[serde(rename = "https_token", default, skip_serializing)]
    pub legacy_https_token: Option<String>,
    #[serde(default)]
    pub has_https_token: bool,
    #[serde(default)]
    pub ssh_key_path: Option<String>,
    #[serde(default)]
    pub gpg_key_id: Option<String>,
    #[serde(default)]
    pub sign_commits: bool,
    #[serde(default = "default_git_providers")]
    pub providers: Vec<GitProviderSettings>,
}

/// A provider-specific HTTPS auth entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitProviderSettings {
    pub id: String,
    #[serde(default = "default_git_provider_kind")]
    pub kind: String,
    pub display_name: String,
    pub host: String,
    #[serde(default)]
    pub username: String,
    #[serde(rename = "token", default, skip_serializing)]
    pub legacy_token: Option<String>,
    #[serde(default)]
    pub has_token: bool,
    #[serde(default)]
    pub use_for_https: bool,
}

#[derive(Debug, Clone, Default)]
pub struct AuthRuntimeState {
    pub ai_api_key: Option<String>,
    pub git: GitAuthRuntime,
}

#[derive(Debug, Clone, Default)]
pub struct GitAuthRuntime {
    pub default_https_token: Option<String>,
    pub ssh_key_path: Option<PathBuf>,
    pub gpg_key_id: Option<String>,
    pub sign_commits: bool,
    pub providers: Vec<GitProviderRuntime>,
}

#[derive(Debug, Clone, Default)]
pub struct GitProviderRuntime {
    pub id: String,
    pub kind: String,
    pub display_name: String,
    pub host: String,
    pub username: String,
    pub token: Option<String>,
    pub use_for_https: bool,
}

impl Default for GitSettings {
    fn default() -> Self {
        Self {
            legacy_https_token: None,
            has_https_token: false,
            ssh_key_path: None,
            gpg_key_id: None,
            sign_commits: false,
            providers: default_git_providers(),
        }
    }
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            provider: default_ai_provider(),
            legacy_api_key: None,
            has_api_key: false,
            model: default_ai_model(),
            commit_style: default_commit_style(),
            enabled: true,
            inject_project_context: default_inject_project_context(),
            use_tools: default_use_tools(),
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            version: default_settings_version(),
            theme: "Catppuccin Mocha".into(),
            ui_font: String::new(),
            ai: AiSettings::default(),
            git: GitSettings::default(),
            recent_repos: Vec::new(),
            max_recent_repos: default_max_recent(),
            last_workspace: Vec::new(),
            layout: LayoutSettings::default(),
            workspaces: Vec::new(),
            active_workspace_id: None,
            clean_exit: true, // First run is considered clean
            compactness: Compactness::default(),
            terminal_command: String::new(),
            editor_command: String::new(),
            font_size: default_font_size(),
            show_line_numbers_in_diff: default_show_line_numbers_in_diff(),
            diff_view_mode: DiffViewMode::default(),
            graph_style: GraphStyle::default(),
            show_subject_column: default_show_subject_column(),
            auto_fetch_interval: AutoFetchInterval::default(),
            confirm_destructive_operations: default_confirm_destructive(),
            auto_check_updates: default_auto_check_updates(),
            last_update_check_at: None,
        }
    }
}

/// Global settings state.
pub struct SettingsState {
    pub settings: AppSettings,
    config_path: PathBuf,
    load_warnings: Vec<String>,
}

impl Global for SettingsState {}

impl SettingsState {
    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    pub fn settings_mut(&mut self) -> &mut AppSettings {
        &mut self.settings
    }

    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.load_warnings)
    }

    pub fn save(&self) -> Result<()> {
        sync_auth_runtime(&self.settings);
        let json = serde_json::to_string_pretty(&self.settings)?;
        let config_path = self.config_path.clone();
        std::thread::spawn(move || {
            // Serialize concurrent writes to prevent file corruption
            static WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
            let _guard = WRITE_LOCK
                .get_or_init(|| Mutex::new(()))
                .lock()
                .unwrap_or_else(|e| e.into_inner());

            if let Some(parent) = config_path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    log::error!("Failed to create settings directory: {}", e);
                    return;
                }
            }
            // Atomic write: write to a temp file then rename
            let tmp_path = config_path.with_extension("json.tmp");
            if let Err(e) = std::fs::write(&tmp_path, &json) {
                log::error!("Failed to write settings temp file: {}", e);
                return;
            }
            if let Err(e) = std::fs::rename(&tmp_path, &config_path) {
                log::error!("Failed to rename settings file: {}", e);
                // Fallback: try direct write
                if let Err(e2) = std::fs::write(&config_path, &json) {
                    log::error!("Fallback write also failed: {}", e2);
                }
                let _ = std::fs::remove_file(&tmp_path);
            }
        });
        Ok(())
    }

    pub fn set_last_workspace(&mut self, repos: Vec<PathBuf>) {
        self.settings.last_workspace = dedup_paths(repos);
    }

    /// Record that an update check just completed.
    pub fn mark_update_check_completed(&mut self) {
        self.settings.last_update_check_at = Some(Utc::now());
        if let Err(error) = self.save() {
            log::warn!("Failed to persist update-check timestamp: {}", error);
        }
    }

    pub fn add_recent_repo(&mut self, path: PathBuf) {
        self.settings.recent_repos.retain(|p| p != &path);
        self.settings.recent_repos.insert(0, path);
        self.settings
            .recent_repos
            .truncate(self.settings.max_recent_repos);
    }

    pub fn recent_workspaces(&self, limit: usize) -> Vec<StoredWorkspace> {
        let mut workspaces = self.settings.workspaces.clone();
        workspaces.sort_by_key(|w| std::cmp::Reverse(w.last_opened_at));
        if limit > 0 {
            workspaces.truncate(limit);
        }
        workspaces
    }

    pub fn active_workspace(&self) -> Option<&StoredWorkspace> {
        let active_id = self.settings.active_workspace_id.as_ref()?;
        self.settings
            .workspaces
            .iter()
            .find(|ws| &ws.id == active_id)
    }

    pub fn workspace(&self, id: &str) -> Option<&StoredWorkspace> {
        self.settings.workspaces.iter().find(|ws| ws.id == id)
    }

    pub fn clear_active_workspace(&mut self) {
        self.settings.active_workspace_id = None;
        self.settings.last_workspace.clear();
    }

    /// Mark that the app is exiting cleanly (user-initiated close).
    pub fn mark_clean_exit(&mut self) {
        self.settings.clean_exit = true;
        let _ = self.save();
    }

    /// Mark that the app is starting up (clear the clean exit flag).
    /// Returns whether the previous session ended cleanly.
    pub fn mark_startup(&mut self) -> bool {
        let was_clean = self.settings.clean_exit;
        self.settings.clean_exit = false;
        let _ = self.save();
        was_clean
    }

    /// Check if the last session ended cleanly without modifying state.
    pub fn was_clean_exit(&self) -> bool {
        self.settings.clean_exit
    }

    pub fn save_workspace_snapshot(
        &mut self,
        workspace_id: Option<&str>,
        repos: Vec<PathBuf>,
        active_repo_index: usize,
        layout: LayoutSettings,
    ) -> Option<String> {
        let repos = dedup_paths(repos);
        if repos.is_empty() {
            self.clear_active_workspace();
            return None;
        }

        let now = Utc::now();
        let existing_index = workspace_id
            .and_then(|id| self.settings.workspaces.iter().position(|ws| ws.id == id))
            .or_else(|| {
                self.settings
                    .workspaces
                    .iter()
                    .position(|ws| ws.repos == repos)
            });

        let active_repo_index = active_repo_index.min(repos.len().saturating_sub(1));
        let id = if let Some(index) = existing_index {
            let workspace = &mut self.settings.workspaces[index];
            workspace.repos = repos.clone();
            workspace.active_repo_index = active_repo_index;
            workspace.layout = layout.clone();
            workspace.last_opened_at = now;
            workspace.id.clone()
        } else {
            let workspace = StoredWorkspace {
                id: Uuid::new_v4().to_string(),
                name: workspace_name_from_repos(&repos),
                repos: repos.clone(),
                active_repo_index,
                layout,
                created_at: now,
                last_opened_at: now,
            };
            let id = workspace.id.clone();
            self.settings.workspaces.push(workspace);
            id
        };

        self.settings.active_workspace_id = Some(id.clone());
        self.settings.last_workspace = repos;
        Some(id)
    }

    pub fn activate_workspace(&mut self, workspace_id: &str) -> bool {
        if let Some(workspace) = self
            .settings
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == workspace_id)
        {
            workspace.last_opened_at = Utc::now();
            self.settings.active_workspace_id = Some(workspace.id.clone());
            self.settings.last_workspace = workspace.repos.clone();
            true
        } else {
            false
        }
    }

    pub fn migrate_legacy_workspace_data(&mut self) {
        self.settings.recent_repos = dedup_paths(self.settings.recent_repos.clone());
        self.settings.last_workspace = dedup_paths(self.settings.last_workspace.clone());

        if self.settings.workspaces.is_empty() && !self.settings.last_workspace.is_empty() {
            let now = Utc::now();
            let workspace = StoredWorkspace {
                id: Uuid::new_v4().to_string(),
                name: workspace_name_from_repos(&self.settings.last_workspace),
                repos: self.settings.last_workspace.clone(),
                active_repo_index: 0,
                layout: self.settings.layout.clone(),
                created_at: now,
                last_opened_at: now,
            };
            self.settings.active_workspace_id = Some(workspace.id.clone());
            self.settings.workspaces.push(workspace);
        }

        if let Some(active_id) = self.settings.active_workspace_id.clone() {
            if !self
                .settings
                .workspaces
                .iter()
                .any(|workspace| workspace.id == active_id)
            {
                self.settings.active_workspace_id = self
                    .settings
                    .workspaces
                    .iter()
                    .max_by_key(|workspace| workspace.last_opened_at)
                    .map(|workspace| workspace.id.clone());
            }
        }
    }

    /// Run version-based migrations. Returns true if any migration was applied.
    pub fn migrate_settings(&mut self) -> bool {
        let mut migrated = false;
        let original_version = self.settings.version;

        // Migration 0 -> 1: Settings without version field (serde default)
        // No action needed - serde defaults handle this
        if self.settings.version == 0 {
            self.settings.version = 1;
            migrated = true;
            log::info!("Migrated settings from version 0 to 1");
        }

        // Future migrations go here:
        // Migration 1 -> 2: Example
        // if self.settings.version == 1 {
        //     // Apply migration
        //     self.settings.version = 2;
        //     migrated = true;
        //     log::info!("Migrated settings from version 1 to 2");
        // }

        // Ensure version is current
        if self.settings.version < CURRENT_SETTINGS_VERSION {
            self.settings.version = CURRENT_SETTINGS_VERSION;
            migrated = true;
            log::info!(
                "Updated settings version from {} to {}",
                original_version,
                CURRENT_SETTINGS_VERSION
            );
        }

        migrated
    }

    pub fn ai_api_key(&self) -> Option<String> {
        current_auth_runtime().ai_api_key
    }

    pub fn git_https_token(&self) -> Option<String> {
        current_auth_runtime().git.default_https_token
    }

    pub fn provider_token(&self, provider_id: &str) -> Option<String> {
        current_auth_runtime()
            .git
            .providers
            .into_iter()
            .find(|provider| provider.id == provider_id)
            .and_then(|provider| provider.token)
    }

    pub fn set_ai_api_key(&mut self, value: Option<&str>) -> Result<()> {
        self.settings.ai.legacy_api_key = None;
        self.settings.ai.has_api_key = write_secret(AI_SECRET_ACCOUNT, value)?;
        sync_auth_runtime(&self.settings);
        Ok(())
    }

    pub fn set_git_https_token(&mut self, value: Option<&str>) -> Result<()> {
        self.settings.git.legacy_https_token = None;
        self.settings.git.has_https_token = write_secret(GIT_DEFAULT_HTTPS_ACCOUNT, value)?;
        sync_auth_runtime(&self.settings);
        Ok(())
    }

    pub fn replace_git_providers(&mut self, providers: Vec<GitProviderSettings>) {
        let removed_ids: Vec<String> = self
            .settings
            .git
            .providers
            .iter()
            .filter(|existing| !providers.iter().any(|provider| provider.id == existing.id))
            .map(|provider| provider.id.clone())
            .collect();

        self.settings.git.providers = providers;
        for provider_id in removed_ids {
            let _ = delete_secret(&git_provider_account(&provider_id));
        }
        sync_auth_runtime(&self.settings);
    }

    pub fn set_git_provider_token(&mut self, provider_id: &str, value: Option<&str>) -> Result<()> {
        if let Some(provider) = self
            .settings
            .git
            .providers
            .iter_mut()
            .find(|provider| provider.id == provider_id)
        {
            provider.legacy_token = None;
            provider.has_token = write_secret(&git_provider_account(provider_id), value)?;
            sync_auth_runtime(&self.settings);
        }
        Ok(())
    }

    pub fn migrate_legacy_secrets(&mut self) -> Result<bool> {
        let mut migrated = false;

        if let Some(api_key) = self.settings.ai.legacy_api_key.clone() {
            if !self.settings.ai.has_api_key && write_secret(AI_SECRET_ACCOUNT, Some(&api_key))? {
                self.settings.ai.has_api_key = true;
                self.settings.ai.legacy_api_key = None;
                migrated = true;
            }
        }

        if let Some(token) = self.settings.git.legacy_https_token.clone() {
            if !self.settings.git.has_https_token
                && write_secret(GIT_DEFAULT_HTTPS_ACCOUNT, Some(&token))?
            {
                self.settings.git.has_https_token = true;
                self.settings.git.legacy_https_token = None;
                migrated = true;
            }
        }

        for provider in &mut self.settings.git.providers {
            if let Some(token) = provider.legacy_token.clone() {
                if !provider.has_token
                    && write_secret(&git_provider_account(&provider.id), Some(&token))?
                {
                    provider.has_token = true;
                    provider.legacy_token = None;
                    migrated = true;
                }
            }
        }

        sync_auth_runtime(&self.settings);
        Ok(migrated)
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
    let mut load_warnings = Vec::new();
    let settings = if config_path.exists() {
        match std::fs::read_to_string(&config_path) {
            Ok(json) => match serde_json::from_str::<AppSettings>(&json) {
                Ok(settings) => settings,
                Err(e) => {
                    let msg = format!("Settings parse error (using defaults): {}", e);
                    log::warn!("{}", msg);
                    load_warnings.push(msg);
                    AppSettings::default()
                }
            },
            Err(e) => {
                let msg = format!("Failed to read settings file (using defaults): {}", e);
                log::warn!("{}", msg);
                load_warnings.push(msg);
                AppSettings::default()
            }
        }
    } else {
        AppSettings::default()
    };

    let mut state = SettingsState {
        settings,
        config_path,
        load_warnings,
    };

    // Run version-based migrations first
    let version_migrated = state.migrate_settings();

    // Run legacy data migrations
    state.migrate_legacy_workspace_data();
    if let Err(error) = state.migrate_legacy_secrets() {
        log::warn!("Failed to migrate secrets into keychain: {}", error);
        sync_auth_runtime(state.settings());
    }

    // Save if any migration occurred
    if version_migrated {
        if let Err(error) = state.save() {
            log::warn!("Failed to persist migrated settings: {}", error);
        }
    } else if let Err(error) = state.save() {
        log::warn!("Failed to persist migrated settings: {}", error);
    }
    cx.set_global(state);
}

pub fn current_auth_runtime() -> AuthRuntimeState {
    auth_runtime()
        .read()
        .expect(
            "git auth runtime RwLock poisoned - a previous thread panicked while holding the lock",
        )
        .clone()
}

pub fn current_git_auth_runtime() -> GitAuthRuntime {
    current_auth_runtime().git
}

fn utc_now() -> DateTime<Utc> {
    Utc::now()
}

fn dedup_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for path in paths {
        if seen.insert(path.clone()) {
            deduped.push(path);
        }
    }
    deduped
}

fn workspace_name_from_repos(repos: &[PathBuf]) -> String {
    match repos {
        [] => "Workspace".to_string(),
        [repo] => repo
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| repo.display().to_string()),
        _ => {
            let names = repos
                .iter()
                .take(2)
                .filter_map(|repo| repo.file_name())
                .map(|name| name.to_string_lossy().to_string())
                .collect::<Vec<_>>();
            if names.is_empty() {
                format!("Workspace ({})", repos.len())
            } else if repos.len() == 2 {
                names.join(" + ")
            } else {
                format!("{} +{}", names.join(", "), repos.len() - names.len())
            }
        }
    }
}

fn default_git_provider_kind() -> String {
    "generic".to_string()
}

fn default_git_providers() -> Vec<GitProviderSettings> {
    vec![]
}

const KEYRING_SERVICE: &str = "rgitui";
const AI_SECRET_ACCOUNT: &str = "ai/default";
const GIT_DEFAULT_HTTPS_ACCOUNT: &str = "git/default-https";

fn git_provider_account(provider_id: &str) -> String {
    format!("git/provider/{}", provider_id)
}

fn auth_runtime() -> &'static RwLock<AuthRuntimeState> {
    static AUTH_RUNTIME: OnceLock<RwLock<AuthRuntimeState>> = OnceLock::new();
    AUTH_RUNTIME.get_or_init(|| RwLock::new(AuthRuntimeState::default()))
}

fn sync_auth_runtime(settings: &AppSettings) {
    let runtime = AuthRuntimeState {
        ai_api_key: resolve_ai_api_key(&settings.ai),
        git: GitAuthRuntime {
            default_https_token: resolve_git_https_token(&settings.git),
            ssh_key_path: settings.git.ssh_key_path.as_ref().map(PathBuf::from),
            gpg_key_id: settings.git.gpg_key_id.clone(),
            sign_commits: settings.git.sign_commits,
            providers: settings
                .git
                .providers
                .iter()
                .map(|provider| GitProviderRuntime {
                    id: provider.id.clone(),
                    kind: provider.kind.clone(),
                    display_name: provider.display_name.clone(),
                    host: provider.host.clone(),
                    username: provider.username.clone(),
                    token: resolve_provider_token(provider),
                    use_for_https: provider.use_for_https,
                })
                .collect(),
        },
    };

    *auth_runtime().write().expect(
        "git auth runtime RwLock poisoned - a previous thread panicked while holding the lock",
    ) = runtime;
}

fn resolve_ai_api_key(settings: &AiSettings) -> Option<String> {
    if settings.has_api_key {
        read_secret(AI_SECRET_ACCOUNT).or_else(|| settings.legacy_api_key.clone())
    } else {
        settings.legacy_api_key.clone()
    }
}

fn resolve_git_https_token(settings: &GitSettings) -> Option<String> {
    if settings.has_https_token {
        read_secret(GIT_DEFAULT_HTTPS_ACCOUNT).or_else(|| settings.legacy_https_token.clone())
    } else {
        settings.legacy_https_token.clone()
    }
}

fn resolve_provider_token(provider: &GitProviderSettings) -> Option<String> {
    if provider.has_token {
        read_secret(&git_provider_account(&provider.id)).or_else(|| provider.legacy_token.clone())
    } else {
        provider.legacy_token.clone()
    }
}

fn secret_entry(account: &str) -> Result<Entry> {
    Ok(Entry::new(KEYRING_SERVICE, account)?)
}

fn write_secret(account: &str, value: Option<&str>) -> Result<bool> {
    let entry = secret_entry(account)?;
    match value {
        Some(secret) if !secret.trim().is_empty() => {
            entry.set_password(secret)?;
            Ok(true)
        }
        _ => {
            let _ = entry.delete_credential();
            Ok(false)
        }
    }
}

fn read_secret(account: &str) -> Option<String> {
    let entry = match secret_entry(account) {
        Ok(e) => e,
        Err(e) => {
            log::warn!("Failed to create keyring entry for '{}': {}", account, e);
            return None;
        }
    };
    match entry.get_password() {
        Ok(password) => Some(password),
        Err(keyring::Error::NoEntry) => None,
        Err(e) => {
            log::warn!("Failed to read secret for '{}': {}", account, e);
            None
        }
    }
}

fn delete_secret(account: &str) -> Result<()> {
    let entry = secret_entry(account)?;
    let _ = entry.delete_credential();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_settings_state() -> SettingsState {
        SettingsState {
            settings: AppSettings::default(),
            config_path: PathBuf::from("/tmp/rgitui-test-settings.json"),
            load_warnings: Vec::new(),
        }
    }

    #[test]
    fn migrates_legacy_last_workspace_into_workspace_snapshot() {
        let mut state = test_settings_state();
        state.settings.last_workspace =
            vec![PathBuf::from("/tmp/repo-a"), PathBuf::from("/tmp/repo-b")];

        state.migrate_legacy_workspace_data();

        assert_eq!(state.settings.workspaces.len(), 1);
        assert_eq!(state.settings.workspaces[0].repos.len(), 2);
        assert_eq!(
            state.settings.active_workspace_id,
            Some(state.settings.workspaces[0].id.clone())
        );
    }

    #[test]
    fn save_workspace_snapshot_updates_existing_workspace() {
        let mut state = test_settings_state();
        let first_id = state
            .save_workspace_snapshot(
                None,
                vec![PathBuf::from("/tmp/repo-a")],
                0,
                LayoutSettings::default(),
            )
            .expect("save_workspace_snapshot should return an id for non-empty repo lists");

        let second_id = state
            .save_workspace_snapshot(
                Some(&first_id),
                vec![PathBuf::from("/tmp/repo-a"), PathBuf::from("/tmp/repo-b")],
                1,
                LayoutSettings::default(),
            )
            .expect("save_workspace_snapshot should return an id for non-empty repo lists");

        assert_eq!(first_id, second_id);
        assert_eq!(state.settings.workspaces.len(), 1);
        assert_eq!(state.settings.workspaces[0].repos.len(), 2);
        assert_eq!(state.settings.workspaces[0].active_repo_index, 1);
    }

    #[test]
    fn recent_workspaces_are_sorted_newest_first() {
        let mut state = test_settings_state();
        let now = Utc::now();
        state.settings.workspaces = vec![
            StoredWorkspace {
                id: "older".into(),
                name: "Older".into(),
                repos: vec![PathBuf::from("/tmp/older")],
                active_repo_index: 0,
                layout: LayoutSettings::default(),
                created_at: now,
                last_opened_at: now,
            },
            StoredWorkspace {
                id: "newer".into(),
                name: "Newer".into(),
                repos: vec![PathBuf::from("/tmp/newer")],
                active_repo_index: 0,
                layout: LayoutSettings::default(),
                created_at: now,
                last_opened_at: now + chrono::TimeDelta::seconds(10),
            },
        ];

        let recent = state.recent_workspaces(10);

        assert_eq!(recent[0].id, "newer");
        assert_eq!(recent[1].id, "older");
    }

    #[test]
    fn migrate_settings_updates_version() {
        let mut state = test_settings_state();
        // Simulate old settings without version (defaults to 1 via serde)
        state.settings.version = 0;

        let migrated = state.migrate_settings();

        assert!(migrated);
        assert_eq!(state.settings.version, CURRENT_SETTINGS_VERSION);
    }

    #[test]
    fn migrate_settings_no_migration_needed_for_current_version() {
        let mut state = test_settings_state();
        state.settings.version = CURRENT_SETTINGS_VERSION;

        let migrated = state.migrate_settings();

        assert!(!migrated);
        assert_eq!(state.settings.version, CURRENT_SETTINGS_VERSION);
    }

    // --- dedup_paths ---

    #[test]
    fn dedup_paths_removes_duplicates() {
        let paths = vec![
            PathBuf::from("/a"),
            PathBuf::from("/b"),
            PathBuf::from("/a"),
        ];
        let result = dedup_paths(paths);
        assert_eq!(result, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
    }

    #[test]
    fn dedup_paths_preserves_order() {
        let paths = vec![
            PathBuf::from("/c"),
            PathBuf::from("/a"),
            PathBuf::from("/b"),
            PathBuf::from("/a"),
        ];
        let result = dedup_paths(paths);
        assert_eq!(
            result,
            vec![
                PathBuf::from("/c"),
                PathBuf::from("/a"),
                PathBuf::from("/b")
            ]
        );
    }

    #[test]
    fn dedup_paths_empty_input() {
        let result = dedup_paths(vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn dedup_paths_no_duplicates() {
        let paths = vec![PathBuf::from("/x"), PathBuf::from("/y")];
        let result = dedup_paths(paths.clone());
        assert_eq!(result, paths);
    }

    // --- workspace_name_from_repos ---

    #[test]
    fn workspace_name_empty_repos_is_workspace() {
        assert_eq!(workspace_name_from_repos(&[]), "Workspace");
    }

    #[test]
    fn workspace_name_single_repo_uses_dir_name() {
        let repos = vec![PathBuf::from("/home/user/my-project")];
        assert_eq!(workspace_name_from_repos(&repos), "my-project");
    }

    #[test]
    fn workspace_name_two_repos_joined_with_plus() {
        let repos = vec![PathBuf::from("/repos/alpha"), PathBuf::from("/repos/beta")];
        assert_eq!(workspace_name_from_repos(&repos), "alpha + beta");
    }

    #[test]
    fn workspace_name_three_repos_shows_overflow() {
        let repos = vec![
            PathBuf::from("/repos/alpha"),
            PathBuf::from("/repos/beta"),
            PathBuf::from("/repos/gamma"),
        ];
        // takes first 2, then "+N" for the rest
        let name = workspace_name_from_repos(&repos);
        assert!(name.contains("alpha"));
        assert!(name.contains("beta"));
        assert!(name.contains("+1"));
    }
}
