use anyhow::Result;
use chrono::{DateTime, Utc};
use gpui::{App, Global};
use keyring::Entry;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{OnceLock, RwLock};
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
    pub compactness: Compactness,
    #[serde(default)]
    pub terminal_command: String,
    #[serde(default)]
    pub editor_command: String,
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
    320.0
}
fn default_diff_viewer_height() -> f32 {
    300.0
}
fn default_commit_input_height() -> f32 {
    260.0
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
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
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
            compactness: Compactness::default(),
            terminal_command: String::new(),
            editor_command: String::new(),
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
        sync_auth_runtime(&self.settings);
        let json = serde_json::to_string_pretty(&self.settings)?;
        std::fs::write(&self.config_path, json)?;
        Ok(())
    }

    pub fn set_last_workspace(&mut self, repos: Vec<PathBuf>) {
        self.settings.last_workspace = dedup_paths(repos);
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
    let settings = if config_path.exists() {
        match std::fs::read_to_string(&config_path) {
            Ok(json) => serde_json::from_str(&json).unwrap_or_default(),
            Err(_) => AppSettings::default(),
        }
    } else {
        AppSettings::default()
    };

    let mut state = SettingsState {
        settings,
        config_path,
    };
    state.migrate_legacy_workspace_data();
    if let Err(error) = state.migrate_legacy_secrets() {
        log::warn!("Failed to migrate secrets into keychain: {}", error);
        sync_auth_runtime(state.settings());
    } else if let Err(error) = state.save() {
        log::warn!("Failed to persist migrated settings: {}", error);
    }
    cx.set_global(state);
}

pub fn current_auth_runtime() -> AuthRuntimeState {
    auth_runtime()
        .read()
        .expect("git auth runtime RwLock poisoned - a previous thread panicked while holding the lock")
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
    vec![
        GitProviderSettings {
            id: "github.com".to_string(),
            kind: "github".to_string(),
            display_name: "GitHub".to_string(),
            host: "github.com".to_string(),
            username: "x-access-token".to_string(),
            legacy_token: None,
            has_token: false,
            use_for_https: true,
        },
        GitProviderSettings {
            id: "gitlab.com".to_string(),
            kind: "gitlab".to_string(),
            display_name: "GitLab".to_string(),
            host: "gitlab.com".to_string(),
            username: "oauth2".to_string(),
            legacy_token: None,
            has_token: false,
            use_for_https: true,
        },
    ]
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

    *auth_runtime().write().expect("git auth runtime RwLock poisoned - a previous thread panicked while holding the lock") = runtime;
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
    let entry = secret_entry(account).ok()?;
    entry.get_password().ok()
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
}
