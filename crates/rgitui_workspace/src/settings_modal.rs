use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, FontWeight, KeyDownEvent,
    Render, SharedString, Window,
};
use rgitui_settings::{AiSettings, GitProviderSettings, GitSettings, SettingsState};
use rgitui_theme::{ActiveTheme, Color, StyledExt, ThemeState};
use rgitui_ui::{
    Button, ButtonSize, ButtonStyle, CheckState, Checkbox, Icon, IconName, IconSize, Label,
    LabelSize,
};

/// Events emitted by the settings modal.
#[derive(Debug, Clone)]
pub enum SettingsModalEvent {
    Dismissed,
    ThemeChanged(String),
    SettingsChanged,
}

/// Which section of the settings is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsSection {
    Theme,
    Ai,
    Auth,
    General,
}

/// Which text field is currently focused for editing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusedField {
    None,
    AiApiKey,
    GitHttpsToken,
    GitSshKeyPath,
    GitGpgKeyId,
    ProviderDisplayName,
    ProviderHost,
    ProviderUsername,
    ProviderToken,
}

#[derive(Debug, Clone)]
struct EditableGitProvider {
    id: String,
    kind: String,
    display_name: String,
    host: String,
    username: String,
    token: String,
    has_token: bool,
    use_for_https: bool,
}

/// The settings modal.
pub struct SettingsModal {
    visible: bool,
    focus_handle: FocusHandle,
    active_section: SettingsSection,

    // Text input state
    focused_field: FocusedField,
    cursor_pos: usize,

    // Theme state
    selected_theme: String,
    available_themes: Vec<String>,

    // AI state
    ai_provider: String,
    ai_api_key: String,
    ai_model: String,
    ai_commit_style: String,
    ai_enabled: bool,

    // Git state
    git_https_token: String,
    git_ssh_key_path: String,
    git_gpg_key_id: String,
    git_sign_commits: bool,
    git_providers: Vec<EditableGitProvider>,
    selected_provider_index: usize,
    expanded_provider_kind: Option<String>,
    pending_browser_auth_provider_id: Option<String>,
    show_ai_api_key: bool,
    show_git_https_token: bool,
    show_provider_token: bool,

    // General state
    max_recent_repos: usize,
    feedback_message: Option<String>,
    feedback_is_error: bool,
}

impl EventEmitter<SettingsModalEvent> for SettingsModal {}

impl SettingsModal {
    fn provider_kind_title(kind: &str) -> &'static str {
        match kind {
            "github" => "GitHub",
            "gitlab" => "GitLab",
            "custom" => "Custom",
            _ => "Provider",
        }
    }

    fn default_provider_username(kind: &str) -> &'static str {
        match kind {
            "github" => "x-access-token",
            "gitlab" => "oauth2",
            _ => "git",
        }
    }

    fn detected_default_ssh_key_path() -> Option<String> {
        let home = std::env::var("HOME").ok()?;
        for key_name in ["id_ed25519", "id_rsa", "id_ecdsa"] {
            let candidate = std::path::PathBuf::from(&home).join(".ssh").join(key_name);
            if candidate.exists() {
                return Some(candidate.display().to_string());
            }
        }
        None
    }

    fn ssh_agent_available() -> bool {
        std::env::var_os("SSH_AUTH_SOCK").is_some()
    }

    pub fn new(cx: &mut Context<Self>) -> Self {
        let (settings, available_themes) = cx.read_global::<SettingsState, _>(|state, cx| {
            let settings = state.settings().clone();
            let themes: Vec<String> = cx
                .global::<ThemeState>()
                .available_themes()
                .iter()
                .map(|t| t.name.clone())
                .collect();
            (settings, themes)
        });

        Self {
            visible: false,
            focus_handle: cx.focus_handle(),
            active_section: SettingsSection::Theme,
            focused_field: FocusedField::None,
            cursor_pos: 0,
            selected_theme: settings.theme.clone(),
            available_themes,
            ai_provider: settings.ai.provider.clone(),
            ai_api_key: cx
                .global::<SettingsState>()
                .ai_api_key()
                .unwrap_or_default(),
            ai_model: settings.ai.model.clone(),
            ai_commit_style: settings.ai.commit_style.clone(),
            ai_enabled: settings.ai.enabled,
            git_https_token: cx
                .global::<SettingsState>()
                .git_https_token()
                .unwrap_or_default(),
            git_ssh_key_path: settings.git.ssh_key_path.clone().unwrap_or_default(),
            git_gpg_key_id: settings.git.gpg_key_id.clone().unwrap_or_default(),
            git_sign_commits: settings.git.sign_commits,
            git_providers: settings
                .git
                .providers
                .iter()
                .map(|provider| EditableGitProvider {
                    id: provider.id.clone(),
                    kind: provider.kind.clone(),
                    display_name: provider.display_name.clone(),
                    host: provider.host.clone(),
                    username: if provider.username
                        == Self::default_provider_username(&provider.kind)
                    {
                        String::new()
                    } else {
                        provider.username.clone()
                    },
                    token: cx
                        .global::<SettingsState>()
                        .provider_token(&provider.id)
                        .unwrap_or_default(),
                    has_token: provider.has_token,
                    use_for_https: provider.use_for_https,
                })
                .collect(),
            selected_provider_index: 0,
            expanded_provider_kind: None,
            pending_browser_auth_provider_id: None,
            show_ai_api_key: false,
            show_git_https_token: false,
            show_provider_token: false,
            max_recent_repos: settings.max_recent_repos,
            feedback_message: None,
            feedback_is_error: false,
        }
    }

    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.visible = !self.visible;
        if self.visible {
            self.reload_from_settings(cx);
            self.focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    pub fn toggle_visible(&mut self, cx: &mut Context<Self>) {
        self.visible = !self.visible;
        if self.visible {
            self.reload_from_settings(cx);
        }
        cx.notify();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        cx.emit(SettingsModalEvent::Dismissed);
        cx.notify();
    }

    fn reload_from_settings(&mut self, cx: &mut Context<Self>) {
        cx.read_global::<SettingsState, _>(|state, _cx| {
            let s = state.settings();
            self.selected_theme = s.theme.clone();
            self.ai_provider = s.ai.provider.clone();
            self.ai_api_key = state.ai_api_key().unwrap_or_default();
            self.ai_model = s.ai.model.clone();
            self.ai_commit_style = s.ai.commit_style.clone();
            self.ai_enabled = s.ai.enabled;
            self.git_https_token = state.git_https_token().unwrap_or_default();
            self.git_ssh_key_path = s.git.ssh_key_path.clone().unwrap_or_default();
            self.git_gpg_key_id = s.git.gpg_key_id.clone().unwrap_or_default();
            self.git_sign_commits = s.git.sign_commits;
            self.git_providers = s
                .git
                .providers
                .iter()
                .map(|provider| EditableGitProvider {
                    id: provider.id.clone(),
                    kind: provider.kind.clone(),
                    display_name: provider.display_name.clone(),
                    host: provider.host.clone(),
                    username: if provider.username
                        == Self::default_provider_username(&provider.kind)
                    {
                        String::new()
                    } else {
                        provider.username.clone()
                    },
                    token: state.provider_token(&provider.id).unwrap_or_default(),
                    has_token: provider.has_token,
                    use_for_https: provider.use_for_https,
                })
                .collect();
            self.selected_provider_index = self
                .selected_provider_index
                .min(self.git_providers.len().saturating_sub(1));
            self.expanded_provider_kind = None;
            self.pending_browser_auth_provider_id = None;
            self.show_ai_api_key = false;
            self.show_git_https_token = false;
            self.show_provider_token = false;
            self.max_recent_repos = s.max_recent_repos;
            self.feedback_message = None;
            self.feedback_is_error = false;
        });
    }

    fn apply_theme(&mut self, theme_name: String, cx: &mut Context<Self>) {
        self.selected_theme = theme_name.clone();
        cx.update_global::<ThemeState, _>(|state, _cx| {
            state.set_theme_by_name(&theme_name);
        });
        cx.update_global::<SettingsState, _>(|state, _cx| {
            state.settings_mut().theme = theme_name.clone();
            if let Err(e) = state.save() {
                log::error!("Failed to save settings: {}", e);
            }
        });
        cx.emit(SettingsModalEvent::ThemeChanged(theme_name));
        cx.notify();
    }

    fn save_settings(&mut self, cx: &mut Context<Self>) {
        let result = cx.update_global::<SettingsState, _>(|state, _cx| -> anyhow::Result<()> {
            let providers: Vec<GitProviderSettings> = self
                .git_providers
                .iter()
                .filter(|provider| !provider.host.trim().is_empty())
                .map(|provider| GitProviderSettings {
                    id: provider.id.clone(),
                    kind: provider.kind.clone(),
                    display_name: if provider.display_name.trim().is_empty() {
                        provider.host.clone()
                    } else {
                        provider.display_name.clone()
                    },
                    host: provider.host.trim().to_string(),
                    username: provider.username.trim().to_string(),
                    legacy_token: None,
                    has_token: !provider.token.trim().is_empty(),
                    use_for_https: provider.use_for_https,
                })
                .collect();

            state.settings_mut().ai = AiSettings {
                provider: self.ai_provider.clone(),
                legacy_api_key: None,
                has_api_key: !self.ai_api_key.trim().is_empty(),
                model: self.ai_model.clone(),
                commit_style: self.ai_commit_style.clone(),
                enabled: self.ai_enabled,
            };
            state.settings_mut().git = GitSettings {
                legacy_https_token: None,
                has_https_token: !self.git_https_token.trim().is_empty(),
                ssh_key_path: if self.git_ssh_key_path.trim().is_empty() {
                    None
                } else {
                    Some(self.git_ssh_key_path.trim().to_string())
                },
                gpg_key_id: if self.git_gpg_key_id.trim().is_empty() {
                    None
                } else {
                    Some(self.git_gpg_key_id.trim().to_string())
                },
                sign_commits: self.git_sign_commits,
                providers,
            };
            state.settings_mut().max_recent_repos = self.max_recent_repos;

            state.set_ai_api_key(Some(self.ai_api_key.trim()).filter(|v| !v.is_empty()))?;
            state
                .set_git_https_token(Some(self.git_https_token.trim()).filter(|v| !v.is_empty()))?;
            state.replace_git_providers(
                self.git_providers
                    .iter()
                    .filter(|provider| !provider.host.trim().is_empty())
                    .map(|provider| GitProviderSettings {
                        id: provider.id.clone(),
                        kind: provider.kind.clone(),
                        display_name: if provider.display_name.trim().is_empty() {
                            provider.host.clone()
                        } else {
                            provider.display_name.clone()
                        },
                        host: provider.host.trim().to_string(),
                        username: provider.username.trim().to_string(),
                        legacy_token: None,
                        has_token: !provider.token.trim().is_empty(),
                        use_for_https: provider.use_for_https,
                    })
                    .collect(),
            );
            for provider in &self.git_providers {
                state.set_git_provider_token(
                    &provider.id,
                    Some(provider.token.trim()).filter(|v| !v.is_empty()),
                )?;
            }
            state.save()?;
            Ok(())
        });

        match result {
            Ok(()) => {
                self.feedback_message = Some("Saved settings and synced keychain secrets.".into());
                self.feedback_is_error = false;
                cx.emit(SettingsModalEvent::SettingsChanged);
            }
            Err(error) => {
                log::error!("Failed to save settings: {}", error);
                self.feedback_message = Some(error.to_string());
                self.feedback_is_error = true;
            }
        }
        cx.notify();
    }

    /// Get a mutable reference to the currently focused text field.
    fn focus_field(&mut self, field: FocusedField, cx: &mut Context<Self>) {
        self.focused_field = field;
        // Set cursor to end of the focused field
        self.cursor_pos = match field {
            FocusedField::None => 0,
            FocusedField::AiApiKey => self.ai_api_key.len(),
            FocusedField::GitHttpsToken => self.git_https_token.len(),
            FocusedField::GitSshKeyPath => self.git_ssh_key_path.len(),
            FocusedField::GitGpgKeyId => self.git_gpg_key_id.len(),
            FocusedField::ProviderDisplayName => self
                .current_provider()
                .map(|provider| provider.display_name.len())
                .unwrap_or(0),
            FocusedField::ProviderHost => self
                .current_provider()
                .map(|provider| provider.host.len())
                .unwrap_or(0),
            FocusedField::ProviderUsername => self
                .current_provider()
                .map(|provider| provider.username.len())
                .unwrap_or(0),
            FocusedField::ProviderToken => self
                .current_provider()
                .map(|provider| provider.token.len())
                .unwrap_or(0),
        };
        cx.notify();
    }

    fn clear_focused_field(&mut self, cx: &mut Context<Self>) {
        if self.focused_field != FocusedField::None {
            self.focused_field = FocusedField::None;
            cx.notify();
        }
    }

    fn current_provider(&self) -> Option<&EditableGitProvider> {
        self.git_providers.get(self.selected_provider_index)
    }

    fn current_provider_mut(&mut self) -> Option<&mut EditableGitProvider> {
        self.git_providers.get_mut(self.selected_provider_index)
    }

    fn provider_has_token(&self, index: usize) -> bool {
        self.git_providers
            .get(index)
            .map(|provider| provider.has_token || !provider.token.trim().is_empty())
            .unwrap_or(false)
    }

    fn focus_provider_token_for_kind(&mut self, kind: &str, cx: &mut Context<Self>) {
        if self.provider_indices_for_kind(kind).is_empty() {
            self.add_provider(kind, cx);
        } else {
            self.select_provider_kind(kind, cx);
        }
        self.focus_field(FocusedField::ProviderToken, cx);
        self.feedback_message = Some(
            "Paste the token into the selected profile. It will be saved to your OS keychain."
                .into(),
        );
        self.feedback_is_error = false;
        cx.notify();
    }

    fn paste_provider_token_for_kind(&mut self, kind: &str, cx: &mut Context<Self>) {
        if self.provider_indices_for_kind(kind).is_empty() {
            self.add_provider(kind, cx);
        } else {
            self.select_provider_kind(kind, cx);
        }

        let clipboard_text = cx
            .read_from_clipboard()
            .and_then(|clipboard| clipboard.text())
            .map(|text| text.lines().next().unwrap_or("").trim().to_string())
            .filter(|text| !text.is_empty());

        if let Some(token) = clipboard_text {
            if let Some(provider) = self.current_provider_mut() {
                provider.token = token;
                provider.has_token = !provider.token.is_empty();
            }
            self.complete_browser_onboarding_for_current_provider();
            self.save_settings(cx);
        } else {
            self.focus_provider_token_for_kind(kind, cx);
        }
    }

    fn complete_browser_onboarding_for_current_provider(&mut self) {
        let current_provider_id = self.current_provider().map(|provider| provider.id.clone());
        if let Some(provider_id) = current_provider_id {
            let token_present = self
                .current_provider()
                .map(|provider| !provider.token.trim().is_empty())
                .unwrap_or(false);
            if token_present {
                if self.pending_browser_auth_provider_id.as_deref() == Some(provider_id.as_str()) {
                    self.pending_browser_auth_provider_id = None;
                    self.feedback_message =
                        Some("Saved token to keychain for the selected profile.".into());
                    self.feedback_is_error = false;
                }
                if let Some(provider) = self.current_provider_mut() {
                    provider.has_token = true;
                }
            } else if let Some(provider) = self.current_provider_mut() {
                provider.has_token = false;
            }
        }
    }

    fn provider_indices_for_kind(&self, kind: &str) -> Vec<usize> {
        self.git_providers
            .iter()
            .enumerate()
            .filter_map(|(index, provider)| (provider.kind == kind).then_some(index))
            .collect()
    }

    fn select_provider_kind(&mut self, kind: &str, cx: &mut Context<Self>) {
        self.expanded_provider_kind = Some(kind.to_string());
        if let Some(index) = self.provider_indices_for_kind(kind).first().copied() {
            self.selected_provider_index = index;
        }
        self.focused_field = FocusedField::None;
        cx.notify();
    }

    fn add_provider(&mut self, kind: &str, cx: &mut Context<Self>) {
        let next_sequence = self
            .git_providers
            .iter()
            .filter(|provider| provider.kind == kind)
            .filter_map(|provider| provider.id.rsplit('-').next()?.parse::<usize>().ok())
            .max()
            .unwrap_or(0)
            + 1;
        let (display_name, host, username) = match kind {
            "github" => (
                format!("GitHub Account {}", next_sequence),
                "github.com".to_string(),
                String::new(),
            ),
            "gitlab" => (
                format!("GitLab Account {}", next_sequence),
                "gitlab.com".to_string(),
                String::new(),
            ),
            _ => (
                format!("Custom Provider {}", next_sequence),
                String::new(),
                String::new(),
            ),
        };

        self.git_providers.push(EditableGitProvider {
            id: format!("{}-{}", kind, next_sequence),
            kind: kind.to_string(),
            display_name,
            host,
            username,
            token: String::new(),
            has_token: false,
            use_for_https: true,
        });
        self.selected_provider_index = self.git_providers.len().saturating_sub(1);
        self.expanded_provider_kind = Some(kind.to_string());
        self.feedback_message = Some("Added a new auth profile.".into());
        self.feedback_is_error = false;
        self.save_settings(cx);
    }

    fn selected_provider_index_for_kind(&self, kind: &str) -> Option<usize> {
        if self
            .current_provider()
            .map(|provider| provider.kind.as_str() == kind)
            .unwrap_or(false)
        {
            Some(self.selected_provider_index)
        } else {
            self.provider_indices_for_kind(kind).first().copied()
        }
    }

    fn remove_current_provider(&mut self, cx: &mut Context<Self>) {
        if self.git_providers.is_empty() {
            return;
        }

        let provider = self.git_providers.remove(self.selected_provider_index);
        if self.pending_browser_auth_provider_id.as_deref() == Some(provider.id.as_str()) {
            self.pending_browser_auth_provider_id = None;
        }
        self.selected_provider_index = self
            .selected_provider_index
            .min(self.git_providers.len().saturating_sub(1));
        self.feedback_message = Some(format!("Removed provider '{}'.", provider.display_name));
        self.feedback_is_error = false;
        self.save_settings(cx);
    }

    fn is_field_unmasked(&self, field: FocusedField) -> bool {
        match field {
            FocusedField::AiApiKey => self.show_ai_api_key,
            FocusedField::GitHttpsToken => self.show_git_https_token,
            FocusedField::ProviderToken => self.show_provider_token,
            _ => true,
        }
    }

    fn toggle_mask_visibility(&mut self, field: FocusedField, cx: &mut Context<Self>) {
        match field {
            FocusedField::AiApiKey => self.show_ai_api_key = !self.show_ai_api_key,
            FocusedField::GitHttpsToken => self.show_git_https_token = !self.show_git_https_token,
            FocusedField::ProviderToken => self.show_provider_token = !self.show_provider_token,
            _ => {}
        }
        cx.notify();
    }

    fn import_from_clipboard(&mut self, field: FocusedField, cx: &mut Context<Self>) {
        let Some(clipboard) = cx.read_from_clipboard() else {
            self.feedback_message = Some("Clipboard did not contain text.".into());
            self.feedback_is_error = true;
            cx.notify();
            return;
        };

        let Some(text) = clipboard.text() else {
            self.feedback_message = Some("Clipboard did not contain text.".into());
            self.feedback_is_error = true;
            cx.notify();
            return;
        };

        let imported = text.lines().next().unwrap_or("").trim().to_string();
        match field {
            FocusedField::AiApiKey => self.ai_api_key = imported,
            FocusedField::GitHttpsToken => self.git_https_token = imported,
            FocusedField::ProviderToken => {
                if let Some(provider) = self.current_provider_mut() {
                    provider.token = imported;
                    provider.has_token = !provider.token.is_empty();
                }
                self.complete_browser_onboarding_for_current_provider();
            }
            _ => {}
        }

        self.feedback_message = Some("Imported secret from clipboard.".into());
        self.feedback_is_error = false;
        self.save_settings(cx);
    }

    fn provider_connect_url(&self, kind: &str, host_override: Option<&str>) -> Option<String> {
        match kind {
            "github" => Some("https://github.com/settings/personal-access-tokens/new".to_string()),
            "gitlab" => {
                let host = host_override
                    .filter(|host| !host.trim().is_empty())
                    .unwrap_or("gitlab.com");
                Some(format!(
                    "https://{host}/-/user_settings/personal_access_tokens?name=rgitui&scopes=read_repository,write_repository"
                ))
            }
            _ => None,
        }
    }

    fn connect_provider_in_browser(
        &mut self,
        kind: &str,
        create_new_account: bool,
        cx: &mut Context<Self>,
    ) {
        let index = if create_new_account || self.provider_indices_for_kind(kind).is_empty() {
            self.add_provider(kind, cx);
            self.selected_provider_index
        } else if let Some(index) = self.selected_provider_index_for_kind(kind) {
            index
        } else {
            self.add_provider(kind, cx);
            self.selected_provider_index
        };
        self.selected_provider_index = index;
        self.expanded_provider_kind = Some(kind.to_string());
        self.pending_browser_auth_provider_id = self
            .git_providers
            .get(index)
            .map(|provider| provider.id.clone());

        let host = self
            .git_providers
            .get(index)
            .map(|provider| provider.host.as_str());
        if let Some(url) = self.provider_connect_url(kind, host) {
            cx.open_url(&url);
            self.feedback_message = Some(
                "Browser login started. Finish in the browser, then use Paste Token if the token is not captured automatically."
                    .into(),
            );
            self.feedback_is_error = false;
        } else {
            self.feedback_message = Some(
                "Custom providers do not have a standard browser onboarding flow. Add the host details in Custom Settings and paste a token there.".into(),
            );
            self.feedback_is_error = false;
        }
        cx.notify();
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        let key = keystroke.key.as_str();
        let ctrl = keystroke.modifiers.control || keystroke.modifiers.platform;

        if key == "escape" {
            if self.focused_field != FocusedField::None {
                self.focused_field = FocusedField::None;
                cx.notify();
            } else {
                self.dismiss(cx);
            }
            return;
        }

        // If a text field is focused, handle text input.
        // We use a macro to get a mutable reference to the focused field's string
        // without borrowing all of `self`, so we can still access `self.cursor_pos`.
        macro_rules! focused_text {
            ($self:ident) => {
                match $self.focused_field {
                    FocusedField::None => None,
                    FocusedField::AiApiKey => Some(&mut $self.ai_api_key),
                    FocusedField::GitHttpsToken => Some(&mut $self.git_https_token),
                    FocusedField::GitSshKeyPath => Some(&mut $self.git_ssh_key_path),
                    FocusedField::GitGpgKeyId => Some(&mut $self.git_gpg_key_id),
                    FocusedField::ProviderDisplayName => $self
                        .git_providers
                        .get_mut($self.selected_provider_index)
                        .map(|provider| &mut provider.display_name),
                    FocusedField::ProviderHost => $self
                        .git_providers
                        .get_mut($self.selected_provider_index)
                        .map(|provider| &mut provider.host),
                    FocusedField::ProviderUsername => $self
                        .git_providers
                        .get_mut($self.selected_provider_index)
                        .map(|provider| &mut provider.username),
                    FocusedField::ProviderToken => $self
                        .git_providers
                        .get_mut($self.selected_provider_index)
                        .map(|provider| &mut provider.token),
                }
            };
        }

        if self.focused_field != FocusedField::None {
            if ctrl {
                match key {
                    "a" => {
                        if let Some(text) = focused_text!(self) {
                            self.cursor_pos = text.len();
                        }
                        cx.notify();
                        return;
                    }
                    "v" => {
                        if let Some(clipboard) = cx.read_from_clipboard() {
                            if let Some(paste) = clipboard.text() {
                                let line = paste.lines().next().unwrap_or("");
                                if let Some(text) = focused_text!(self) {
                                    let pos = self.cursor_pos.min(text.len());
                                    text.insert_str(pos, line);
                                    self.cursor_pos = pos + line.len();
                                }
                                if self.focused_field == FocusedField::ProviderToken {
                                    self.complete_browser_onboarding_for_current_provider();
                                }
                                self.save_settings(cx);
                            }
                        }
                        return;
                    }
                    _ => {}
                }
                return;
            }

            match key {
                "backspace" => {
                    if self.cursor_pos > 0 {
                        if let Some(text) = focused_text!(self) {
                            let prev = prev_char_boundary(text, self.cursor_pos);
                            text.drain(prev..self.cursor_pos);
                            self.cursor_pos = prev;
                        }
                        if self.focused_field == FocusedField::ProviderToken {
                            self.complete_browser_onboarding_for_current_provider();
                        }
                        self.save_settings(cx);
                    }
                }
                "delete" => {
                    if let Some(text) = focused_text!(self) {
                        if self.cursor_pos < text.len() {
                            let next = next_char_boundary(text, self.cursor_pos);
                            text.drain(self.cursor_pos..next);
                            if self.focused_field == FocusedField::ProviderToken {
                                self.complete_browser_onboarding_for_current_provider();
                            }
                            self.save_settings(cx);
                        }
                    }
                }
                "left" => {
                    if self.cursor_pos > 0 {
                        if let Some(text) = focused_text!(self) {
                            self.cursor_pos = prev_char_boundary(text, self.cursor_pos);
                        }
                        cx.notify();
                    }
                }
                "right" => {
                    if let Some(text) = focused_text!(self) {
                        if self.cursor_pos < text.len() {
                            self.cursor_pos = next_char_boundary(text, self.cursor_pos);
                        }
                    }
                    cx.notify();
                }
                "home" => {
                    self.cursor_pos = 0;
                    cx.notify();
                }
                "end" => {
                    if let Some(text) = focused_text!(self) {
                        self.cursor_pos = text.len();
                    }
                    cx.notify();
                }
                "tab" | "enter" => {
                    self.focused_field = FocusedField::None;
                    cx.notify();
                }
                _ => {
                    if let Some(key_char) = &keystroke.key_char {
                        if let Some(text) = focused_text!(self) {
                            let pos = self.cursor_pos.min(text.len());
                            text.insert_str(pos, key_char);
                            self.cursor_pos = pos + key_char.len();
                        }
                        if self.focused_field == FocusedField::ProviderToken {
                            self.complete_browser_onboarding_for_current_provider();
                        }
                        self.save_settings(cx);
                    }
                }
            }
            return;
        }
    }

    // ── Helper: section header with icon ────────────────────────────────
    fn section_header(icon: IconName, title: &str, subtitle: &str) -> impl IntoElement {
        div()
            .v_flex()
            .w_full()
            .gap(px(4.))
            .pb(px(12.))
            .child(
                div()
                    .h_flex()
                    .gap(px(8.))
                    .items_center()
                    .child(Icon::new(icon).size(IconSize::Medium).color(Color::Accent))
                    .child(
                        Label::new(SharedString::from(title.to_string()))
                            .size(LabelSize::Large)
                            .weight(FontWeight::BOLD),
                    ),
            )
            .child(
                Label::new(SharedString::from(subtitle.to_string()))
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
    }

    // ── Helper: setting group card ──────────────────────────────────────
    fn setting_card(cx: &Context<Self>) -> gpui::Div {
        let colors = cx.colors().clone();
        div()
            .v_flex()
            .w_full()
            .p(px(14.))
            .gap(px(10.))
            .rounded(px(8.))
            .bg(colors.surface_background)
            .border_1()
            .border_color(colors.border_variant)
    }

    // ── Helper: setting row label ───────────────────────────────────────
    fn setting_label(title: &str, description: &str) -> impl IntoElement {
        div()
            .v_flex()
            .gap(px(2.))
            .child(
                Label::new(SharedString::from(title.to_string()))
                    .size(LabelSize::Small)
                    .weight(FontWeight::SEMIBOLD),
            )
            .child(
                Label::new(SharedString::from(description.to_string()))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
    }

    // ── Helper: pill toggle group ───────────────────────────────────────
    fn pill_group(
        &self,
        group_id: &str,
        options: &[&str],
        selected: &str,
        on_select: impl Fn(&mut Self, String, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let on_select = std::rc::Rc::new(on_select);
        let options_vec: Vec<String> = options.iter().map(|o| o.to_string()).collect();
        let selected_str = selected.to_string();
        let colors = cx.colors();

        let mut row = div()
            .h_flex()
            .gap(px(4.))
            .p(px(3.))
            .rounded(px(8.))
            .bg(colors.element_background);

        for (idx, option) in options_vec.into_iter().enumerate() {
            let is_selected = option == selected_str;
            let label: SharedString = option.clone().into();
            let on_select = on_select.clone();

            row = row.child(
                div()
                    .id(ElementId::NamedInteger(
                        SharedString::from(format!("pill-{}", group_id)),
                        idx as u64,
                    ))
                    .h_flex()
                    .h(px(28.))
                    .px(px(12.))
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(is_selected, |el| {
                        el.bg(colors.elevated_surface_background)
                            .border_1()
                            .border_color(colors.border)
                    })
                    .when(!is_selected, |el| {
                        el.hover(|s| s.bg(colors.ghost_element_hover))
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        on_select(this, option.clone(), cx);
                    }))
                    .child(
                        Label::new(label)
                            .size(LabelSize::XSmall)
                            .color(if is_selected {
                                Color::Default
                            } else {
                                Color::Muted
                            })
                            .weight(if is_selected {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::NORMAL
                            }),
                    ),
            );
        }

        row
    }

    // ── Helper: clickable text input field ─────────────────────────────
    fn text_input(
        &self,
        id: &'static str,
        field: FocusedField,
        value: &str,
        placeholder: &'static str,
        masked: bool,
        icon: IconName,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let colors = cx.colors();
        let is_focused = self.focused_field == field;
        let cursor_pos = self.cursor_pos;
        let show_unmasked = self.is_field_unmasked(field);
        let display_value = if masked && !show_unmasked {
            "*".repeat(value.len())
        } else {
            value.to_string()
        };
        let cursor_pos = cursor_pos.min(display_value.len());
        let (before_cursor, after_cursor) = if is_focused {
            (
                display_value[..cursor_pos].to_string(),
                display_value[cursor_pos..].to_string(),
            )
        } else {
            (display_value.clone(), String::new())
        };

        let mut input_content = div().h_flex().items_center().min_w_0();
        if value.is_empty() && !is_focused {
            input_content = input_content.child(
                Label::new(placeholder)
                    .size(LabelSize::Small)
                    .color(Color::Placeholder),
            );
        } else {
            if !before_cursor.is_empty() {
                input_content = input_content.child(
                    Label::new(SharedString::from(before_cursor))
                        .size(LabelSize::Small)
                        .color(Color::Default),
                );
            }
            if is_focused {
                input_content = input_content.child(
                    div()
                        .w(px(1.5))
                        .h(px(18.))
                        .bg(colors.border_focused)
                        .mr(px(1.)),
                );
            }
            if !after_cursor.is_empty() {
                input_content = input_content.child(
                    Label::new(SharedString::from(after_cursor))
                        .size(LabelSize::Small)
                        .color(Color::Default),
                );
            }
        }

        div()
            .id(id)
            .h_flex()
            .w_full()
            .h(px(36.))
            .px(px(12.))
            .items_center()
            .bg(colors.editor_background)
            .border_1()
            .border_color(if is_focused {
                colors.border_focused
            } else {
                colors.border
            })
            .rounded(px(6.))
            .cursor_text()
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.focus_field(field, cx);
            }))
            .child(Icon::new(icon).size(IconSize::Small).color(Color::Muted))
            .child(div().flex_1().min_w_0().pl(px(8.)).child(input_content))
            .when(masked, |el| {
                el.child(
                    Button::new(
                        ElementId::Name(SharedString::from(format!("{id}-import"))),
                        "Paste",
                    )
                    .style(ButtonStyle::Subtle)
                    .size(ButtonSize::Compact)
                    .icon(IconName::Copy)
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.import_from_clipboard(field, cx);
                        },
                    )),
                )
                .child(
                    Button::new(
                        ElementId::Name(SharedString::from(format!("{id}-toggle-mask"))),
                        if show_unmasked { "Hide" } else { "Show" },
                    )
                    .style(ButtonStyle::Subtle)
                    .size(ButtonSize::Compact)
                    .icon(if show_unmasked {
                        IconName::EyeOff
                    } else {
                        IconName::Eye
                    })
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.toggle_mask_visibility(field, cx);
                        },
                    )),
                )
            })
    }

    // ── Sidebar navigation ──────────────────────────────────────────────
    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        let sections: [(SettingsSection, IconName, &str); 4] = [
            (SettingsSection::Theme, IconName::Eye, "Appearance"),
            (SettingsSection::Ai, IconName::Sparkle, "AI"),
            (SettingsSection::Auth, IconName::GitBranch, "Auth"),
            (SettingsSection::General, IconName::Settings, "General"),
        ];

        let mut sidebar = div()
            .id("settings-sidebar")
            .v_flex()
            .w(px(220.))
            .h_full()
            .bg(colors.surface_background)
            .border_r_1()
            .border_color(colors.border_variant)
            .pt(px(12.))
            .pb(px(12.))
            .gap(px(2.))
            .px(px(8.))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.clear_focused_field(cx);
            }));

        // Header
        sidebar = sidebar.child(
            div()
                .h_flex()
                .gap(px(8.))
                .items_center()
                .px(px(8.))
                .pb(px(12.))
                .child(
                    Icon::new(IconName::Settings)
                        .size(IconSize::Small)
                        .color(Color::Muted),
                )
                .child(
                    Label::new("Preferences")
                        .size(LabelSize::Small)
                        .weight(FontWeight::BOLD)
                        .color(Color::Muted),
                ),
        );

        for (section, icon, label) in sections {
            let is_active = section == self.active_section;
            let label_str: SharedString = label.into();

            sidebar = sidebar.child(
                div()
                    .id(ElementId::Name(SharedString::from(format!(
                        "settings-nav-{}",
                        label
                    ))))
                    .h_flex()
                    .w_full()
                    .h(px(34.))
                    .px(px(10.))
                    .gap(px(8.))
                    .items_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(is_active, |el| el.bg(colors.ghost_element_selected))
                    .when(!is_active, |el| {
                        el.hover(|s| s.bg(colors.ghost_element_hover))
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.active_section = section;
                        cx.notify();
                    }))
                    .child(Icon::new(icon).size(IconSize::Small).color(if is_active {
                        Color::Accent
                    } else {
                        Color::Muted
                    }))
                    .child(
                        Label::new(label_str)
                            .size(LabelSize::Small)
                            .weight(if is_active {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .color(if is_active {
                                Color::Default
                            } else {
                                Color::Muted
                            }),
                    ),
            );
        }

        sidebar
    }

    // ── Theme section ───────────────────────────────────────────────────
    fn render_theme_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        let mut section = div().v_flex().w_full().gap(px(16.));

        section = section.child(Self::section_header(
            IconName::Eye,
            "Appearance",
            "Customize the look and feel of the application.",
        ));

        // Theme grid
        let mut card = Self::setting_card(cx);
        card = card.child(Self::setting_label(
            "Color Theme",
            "Select a theme to change all interface colors.",
        ));

        let mut theme_grid = div().v_flex().w_full().gap(px(6.));

        for (idx, theme_name) in self.available_themes.iter().enumerate() {
            let is_selected = *theme_name == self.selected_theme;
            let name: SharedString = theme_name.clone().into();
            let theme_name_owned = theme_name.clone();

            theme_grid = theme_grid.child(
                div()
                    .id(ElementId::NamedInteger("theme-option".into(), idx as u64))
                    .h_flex()
                    .w_full()
                    .h(px(40.))
                    .px(px(12.))
                    .gap(px(10.))
                    .items_center()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .when(is_selected, |el| {
                        el.bg(colors.ghost_element_selected)
                            .border_1()
                            .border_color(colors.border_focused)
                    })
                    .when(!is_selected, |el| {
                        el.border_1()
                            .border_color(colors.border_variant)
                            .hover(|s| s.bg(colors.ghost_element_hover))
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.apply_theme(theme_name_owned.clone(), cx);
                    }))
                    // Color swatch
                    .child(
                        div()
                            .w(px(20.))
                            .h(px(20.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(if is_selected {
                                colors.border_focused
                            } else {
                                colors.border_variant
                            })
                            .bg(if is_selected {
                                colors.text_accent
                            } else {
                                colors.element_background
                            }),
                    )
                    // Theme name
                    .child(
                        Label::new(name)
                            .size(LabelSize::Small)
                            .weight(if is_selected {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::NORMAL
                            }),
                    )
                    // Active badge
                    .child(div().flex_1())
                    .when(is_selected, |el| {
                        el.child(
                            div()
                                .h_flex()
                                .h(px(20.))
                                .px(px(8.))
                                .rounded(px(10.))
                                .bg(gpui::Hsla {
                                    a: 0.15,
                                    ..colors.text_accent
                                })
                                .items_center()
                                .child(
                                    Label::new("Active")
                                        .size(LabelSize::XSmall)
                                        .color(Color::Accent)
                                        .weight(FontWeight::SEMIBOLD),
                                ),
                        )
                    }),
            );
        }

        card = card.child(theme_grid);
        section = section.child(card);
        section
    }

    // ── AI section ──────────────────────────────────────────────────────
    fn render_ai_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut section = div().v_flex().w_full().gap(px(16.));

        section = section.child(Self::section_header(
            IconName::Sparkle,
            "AI Configuration",
            "Configure AI-powered commit message generation.",
        ));

        // Enable/disable card
        let ai_enabled = self.ai_enabled;
        let mut enable_card = Self::setting_card(cx);
        enable_card = enable_card.child(
            div()
                .h_flex()
                .w_full()
                .items_center()
                .child(
                    div()
                        .v_flex()
                        .flex_1()
                        .gap(px(2.))
                        .child(
                            Label::new("Enable AI")
                                .size(LabelSize::Small)
                                .weight(FontWeight::SEMIBOLD),
                        )
                        .child(
                            Label::new("Generate commit messages using AI")
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                )
                .child(
                    div()
                        .id("ai-toggle")
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.ai_enabled = !this.ai_enabled;
                            this.save_settings(cx);
                        }))
                        .child(Checkbox::new(
                            "ai-enabled-cb",
                            if ai_enabled {
                                CheckState::Checked
                            } else {
                                CheckState::Unchecked
                            },
                        )),
                ),
        );
        section = section.child(enable_card);

        // Provider + Model card
        let mut provider_card = Self::setting_card(cx);
        provider_card = provider_card
            .child(Self::setting_label("Provider", "Choose your AI provider."))
            .child(self.pill_group(
                "provider",
                &["gemini", "openai", "anthropic"],
                &self.ai_provider,
                |this, value, cx| {
                    this.ai_provider = value;
                    // Reset model when provider changes
                    this.ai_model = match this.ai_provider.as_str() {
                        "gemini" => "gemini-3-flash-preview".into(),
                        "openai" => "gpt-5-mini".into(),
                        "anthropic" => "claude-sonnet-4-6".into(),
                        _ => "gemini-3-flash-preview".into(),
                    };
                    this.save_settings(cx);
                },
                cx,
            ));

        // Model selector
        let models: Vec<&str> = match self.ai_provider.as_str() {
            "gemini" => vec![
                "gemini-3.1-pro-preview",
                "gemini-3-flash-preview",
                "gemini-2.5-flash",
                "gemini-2.5-pro",
            ],
            "openai" => vec![
                "gpt-5.4",
                "gpt-5",
                "gpt-5-mini",
                "gpt-5-nano",
                "o3",
                "o4-mini",
            ],
            "anthropic" => vec![
                "claude-opus-4-6",
                "claude-sonnet-4-6",
                "claude-sonnet-4-5-20241022",
                "claude-haiku-4-5",
            ],
            _ => vec!["gemini-2.5-flash"],
        };
        provider_card = provider_card
            .child(Self::setting_label("Model", "Select the model to use."))
            .child(self.pill_group(
                "model",
                &models,
                &self.ai_model,
                |this, value, cx| {
                    this.ai_model = value;
                    this.save_settings(cx);
                },
                cx,
            ));

        section = section.child(provider_card);

        // Commit style card
        let mut style_card = Self::setting_card(cx);
        style_card = style_card
            .child(Self::setting_label(
                "Commit Style",
                "How the AI should format commit messages.",
            ))
            .child(self.pill_group(
                "commit-style",
                &["conventional", "descriptive", "brief"],
                &self.ai_commit_style,
                |this, value, cx| {
                    this.ai_commit_style = value;
                    this.save_settings(cx);
                },
                cx,
            ));
        section = section.child(style_card);

        // API Key card
        let mut key_card = Self::setting_card(cx);
        key_card = key_card
            .child(Self::setting_label(
                "API Key",
                "Stored in your OS keychain and only materialized in memory when needed.",
            ))
            .child(self.text_input(
                "ai-api-key-input",
                FocusedField::AiApiKey,
                &self.ai_api_key,
                "Click to enter API key...",
                true,
                IconName::Eye,
                cx,
            ));
        section = section.child(key_card);

        section
    }

    // ── Git section ──────────────────────────────────────────────────────
    fn render_git_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors().clone();
        let mut section = div().v_flex().w_full().gap(px(16.));
        section = section.child(Self::section_header(
            IconName::GitBranch,
            "Accounts & Credentials",
            "Use browser sign-in for GitHub or GitLab, manage all account profiles in one place, and keep SSH separate from HTTPS tokens.",
        ));

        let mut quick_setup = Self::setting_card(cx);
        quick_setup = quick_setup
            .child(Self::setting_label(
                "Quick Setup",
                "Start by adding the provider account you want to use. Browser sign-in is the primary path. Manual setup is available for self-hosted or custom HTTPS remotes.",
            ))
            .child(
                div()
                    .h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        Button::new("add-github-account", "Add GitHub Account")
                            .style(ButtonStyle::Filled)
                            .size(ButtonSize::Compact)
                            .icon(IconName::Plus)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.add_provider("github", cx);
                            })),
                    )
                    .child(
                        Button::new("add-gitlab-account", "Add GitLab Account")
                            .style(ButtonStyle::Outlined)
                            .size(ButtonSize::Compact)
                            .icon(IconName::Plus)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.add_provider("gitlab", cx);
                            })),
                    )
                    .child(
                        Button::new("add-custom-account", "Manual / Custom Host")
                            .style(ButtonStyle::Outlined)
                            .size(ButtonSize::Compact)
                            .icon(IconName::ExternalLink)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.add_provider("custom", cx);
                            })),
                    ),
            );
        section = section.child(quick_setup);

        let mut profiles_card = Self::setting_card(cx);
        profiles_card = profiles_card.child(Self::setting_label(
            "Profiles",
            "All saved accounts live here. Select one to connect it, paste a token, or adjust host matching.",
        ));

        if self.git_providers.is_empty() {
            profiles_card = profiles_card.child(
                div()
                    .v_flex()
                    .w_full()
                    .p(px(18.))
                    .gap(px(6.))
                    .rounded(px(8.))
                    .bg(colors.element_background)
                    .border_1()
                    .border_color(colors.border_variant)
                    .child(
                        Label::new("No provider accounts configured yet.")
                            .size(LabelSize::Small)
                            .weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        Label::new(
                            "Add a GitHub account to start with the default flow, or choose Manual / Custom Host for enterprise and other HTTPS remotes.",
                        )
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                    ),
            );
        } else {
            let mut profiles_table = div()
                .v_flex()
                .w_full()
                .rounded(px(8.))
                .border_1()
                .border_color(colors.border_variant)
                .overflow_hidden();

            profiles_table = profiles_table.child(
                div()
                    .h_flex()
                    .w_full()
                    .px(px(12.))
                    .py(px(8.))
                    .gap(px(12.))
                    .bg(colors.surface_background)
                    .border_b_1()
                    .border_color(colors.border_variant)
                    .child(
                        Label::new("Profile")
                            .size(LabelSize::XSmall)
                            .weight(FontWeight::SEMIBOLD)
                            .color(Color::Muted),
                    )
                    .child(div().w(px(96.)))
                    .child(
                        Label::new("Host")
                            .size(LabelSize::XSmall)
                            .weight(FontWeight::SEMIBOLD)
                            .color(Color::Muted),
                    )
                    .child(div().flex_1())
                    .child(
                        Label::new("Status")
                            .size(LabelSize::XSmall)
                            .weight(FontWeight::SEMIBOLD)
                            .color(Color::Muted),
                    ),
            );

            for (index, provider) in self.git_providers.iter().enumerate() {
                let is_selected = index == self.selected_provider_index;
                let has_token = self.provider_has_token(index);
                let browser_login_pending =
                    self.pending_browser_auth_provider_id.as_deref() == Some(provider.id.as_str());
                let provider_kind = provider.kind.clone();
                let connect_kind = provider.kind.clone();
                let remove_index = index;
                let title = Self::provider_kind_title(&provider.kind);
                let label = if provider.display_name.trim().is_empty() {
                    format!("{} Account {}", title, index + 1)
                } else {
                    provider.display_name.clone()
                };
                let status_label = if browser_login_pending {
                    "Browser login pending"
                } else if has_token {
                    "Connected"
                } else {
                    "Needs token"
                };

                profiles_table = profiles_table.child(
                    div()
                        .id(ElementId::NamedInteger(
                            "provider-profile-row".into(),
                            index as u64,
                        ))
                        .h_flex()
                        .w_full()
                        .items_center()
                        .gap(px(12.))
                        .px(px(12.))
                        .py(px(10.))
                        .cursor_pointer()
                        .when(is_selected, |el| {
                            el.bg(colors.ghost_element_selected)
                                .border_l_2()
                                .border_color(colors.border_focused)
                        })
                        .when(!is_selected, |el| {
                            el.hover(|s| s.bg(colors.ghost_element_hover))
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.selected_provider_index = index;
                            this.focused_field = FocusedField::None;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .v_flex()
                                .w(px(210.))
                                .min_w(px(210.))
                                .gap(px(2.))
                                .child(
                                    Label::new(label)
                                        .size(LabelSize::Small)
                                        .weight(FontWeight::SEMIBOLD)
                                        .truncate(),
                                )
                                .child(
                                    Label::new(Self::provider_kind_title(&provider_kind))
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                ),
                        )
                        .child(
                            div()
                                .px(px(8.))
                                .py(px(3.))
                                .rounded(px(999.))
                                .bg(colors.surface_background)
                                .child(
                                    Label::new(title)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                ),
                        )
                        .child(
                            div().flex_1().min_w_0().child(
                                Label::new(provider.host.clone())
                                    .size(LabelSize::Small)
                                    .truncate()
                                    .color(if provider.host.trim().is_empty() {
                                        Color::Placeholder
                                    } else {
                                        Color::Default
                                    }),
                            ),
                        )
                        .child(Label::new(status_label).size(LabelSize::XSmall).color(
                            if has_token {
                                Color::Success
                            } else {
                                Color::Warning
                            },
                        ))
                        .child(
                            div()
                                .h_flex()
                                .gap(px(6.))
                                .child(
                                    Button::new(
                                        ElementId::Name(SharedString::from(format!(
                                            "profile-connect-{index}"
                                        ))),
                                        if provider.kind == "custom" {
                                            "Manual"
                                        } else if has_token {
                                            "Reconnect"
                                        } else {
                                            "Connect"
                                        },
                                    )
                                    .style(if provider.kind == "custom" {
                                        ButtonStyle::Outlined
                                    } else {
                                        ButtonStyle::Subtle
                                    })
                                    .size(ButtonSize::Compact)
                                    .icon(if provider.kind == "custom" {
                                        IconName::Edit
                                    } else {
                                        IconName::ExternalLink
                                    })
                                    .on_click(cx.listener(
                                        move |this, _: &ClickEvent, _, cx| {
                                            this.selected_provider_index = remove_index;
                                            if connect_kind == "custom" {
                                                this.focus_field(FocusedField::ProviderHost, cx);
                                            } else {
                                                this.connect_provider_in_browser(
                                                    &connect_kind,
                                                    false,
                                                    cx,
                                                );
                                            }
                                        },
                                    )),
                                )
                                .child(
                                    Button::new(
                                        ElementId::Name(SharedString::from(format!(
                                            "profile-remove-{index}"
                                        ))),
                                        "Remove",
                                    )
                                    .style(ButtonStyle::Transparent)
                                    .size(ButtonSize::Compact)
                                    .icon(IconName::Trash)
                                    .color(Color::Deleted)
                                    .on_click(cx.listener(
                                        move |this, _: &ClickEvent, _, cx| {
                                            this.selected_provider_index = remove_index;
                                            this.remove_current_provider(cx);
                                        },
                                    )),
                                ),
                        ),
                );
            }

            profiles_card = profiles_card.child(profiles_table);
        }
        section = section.child(profiles_card);

        if let Some(provider) = self.current_provider() {
            let provider = provider.clone();
            let kind = provider.kind.clone();
            let connect_kind = kind.clone();
            let paste_kind = kind.clone();
            let default_host = match kind.as_str() {
                "github" => "github.com",
                "gitlab" => "gitlab.com",
                _ => "git.example.com",
            };
            let browser_login_pending =
                self.pending_browser_auth_provider_id.as_deref() == Some(provider.id.as_str());
            let has_token = !provider.token.trim().is_empty() || provider.has_token;
            let show_manual_username = kind == "custom"
                || !provider.username.trim().is_empty()
                || provider.host.trim() != default_host;

            let mut selected_profile = Self::setting_card(cx);
            selected_profile = selected_profile
                .child(Self::setting_label(
                    &format!("Selected Profile: {}", provider.display_name),
                    if has_token {
                        "This account has credentials stored in the keychain. Reconnect in the browser or replace the token manually if needed."
                    } else if browser_login_pending {
                        "The browser step has started. If the browser does not hand back a token automatically, use Paste Token below."
                    } else if kind == "custom" {
                        "Custom hosts use manual setup. Fill in the host and token, then enable HTTPS matching if you want it used automatically."
                    } else {
                        "Connect this account in the browser first. Manual token paste is the fallback."
                    },
                ))
                .child(
                    div()
                        .h_flex()
                        .flex_wrap()
                        .gap(px(8.))
                        .child(
                            Button::new(
                                "selected-profile-connect",
                                if kind == "custom" { "Manual Setup" } else { "Connect in Browser" },
                            )
                            .style(ButtonStyle::Filled)
                            .size(ButtonSize::Compact)
                            .icon(if kind == "custom" {
                                IconName::Edit
                            } else {
                                IconName::ExternalLink
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                if connect_kind == "custom" {
                                    this.focus_field(FocusedField::ProviderHost, cx);
                                } else {
                                    this.connect_provider_in_browser(&connect_kind, false, cx);
                                }
                            })),
                        )
                        .child(
                            Button::new("selected-profile-paste", "Paste Token")
                                .style(ButtonStyle::Outlined)
                                .size(ButtonSize::Compact)
                                .icon(IconName::Copy)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.paste_provider_token_for_kind(&paste_kind, cx);
                                })),
                        ),
                )
                .child(Self::setting_label(
                    "Account Label",
                    "Use a clear label like Work, Personal, or Client so switching is obvious.",
                ))
                .child(self.text_input(
                    "provider-display-name",
                    FocusedField::ProviderDisplayName,
                    &provider.display_name,
                    "Work / Personal / Client",
                    false,
                    IconName::User,
                    cx,
                ))
                .child(Self::setting_label(
                    "Host",
                    if kind == "github" {
                        "Leave this as github.com unless you are targeting GitHub Enterprise."
                    } else if kind == "gitlab" {
                        "Leave this as gitlab.com unless you are targeting a self-hosted GitLab."
                    } else {
                        "Hostname to match against remote URLs, for example git.example.com."
                    },
                ))
                .child(self.text_input(
                    "provider-host",
                    FocusedField::ProviderHost,
                    &provider.host,
                    default_host,
                    false,
                    IconName::ExternalLink,
                    cx,
                ))
                .child(Self::setting_label(
                    "Access Token",
                    "Stored in your OS keychain. Use Paste Token if the browser flow does not return credentials automatically.",
                ))
                .child(self.text_input(
                    "provider-token",
                    FocusedField::ProviderToken,
                    &provider.token,
                    "Paste the access token for this account...",
                    true,
                    IconName::Eye,
                    cx,
                ))
                .when(show_manual_username, |el| {
                    el.child(Self::setting_label(
                        "Advanced HTTPS Username",
                        "Leave this empty unless your server explicitly requires a custom username. Built-in providers infer the correct default automatically.",
                    ))
                    .child(self.text_input(
                        "provider-username",
                        FocusedField::ProviderUsername,
                        &provider.username,
                        "Usually left empty",
                        false,
                        IconName::User,
                        cx,
                    ))
                })
                .child(
                    div()
                        .h_flex()
                        .flex_wrap()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .gap(px(12.))
                        .child(Self::setting_label(
                            "Use for HTTPS matching",
                            "If enabled, rgitui will use this account automatically when a remote host matches the host above.",
                        ))
                        .child(
                            Button::new(
                                "provider-use-for-https-toggle",
                                if provider.use_for_https { "Enabled" } else { "Disabled" },
                            )
                            .style(ButtonStyle::Outlined)
                            .size(ButtonSize::Compact)
                            .icon(if provider.use_for_https {
                                IconName::Check
                            } else {
                                IconName::Minus
                            })
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, _, cx| {
                                    if let Some(provider) = this.current_provider_mut() {
                                        provider.use_for_https = !provider.use_for_https;
                                    }
                                    this.save_settings(cx);
                                },
                            )),
                        ),
                );
            section = section.child(selected_profile);
        }

        // HTTPS Token card
        let mut https_card = Self::setting_card(cx);
        https_card = https_card
            .child(Self::setting_label(
                "Fallback HTTPS Token / PAT",
                "Used only when no profile-specific account matches the remote host. Leave this empty unless you truly need a generic fallback.",
            ))
            .child(self.text_input(
                "git-https-token-input",
                FocusedField::GitHttpsToken,
                &self.git_https_token,
                "Paste a generic HTTPS token only if you need a fallback...",
                true,
                IconName::Eye,
                cx,
            ));
        section = section.child(https_card);

        // SSH Key Path card
        let detected_ssh_key = Self::detected_default_ssh_key_path();
        let ssh_agent_available = Self::ssh_agent_available();
        let mut ssh_card = Self::setting_card(cx);
        ssh_card = ssh_card
            .child(Self::setting_label(
                "SSH",
                "SSH applies only to remotes such as git@github.com:owner/repo.git or ssh://host/path.git. SSH network operations use your system Git and SSH environment so existing keys, agent state, and host config continue to work.",
            ))
            .child(
                div()
                    .h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        div()
                            .px(px(8.))
                            .py(px(4.))
                            .rounded(px(999.))
                            .bg(colors.surface_background)
                            .child(
                                Label::new(if ssh_agent_available {
                                    "ssh-agent detected"
                                } else {
                                    "ssh-agent not detected"
                                })
                                .size(LabelSize::XSmall)
                                .color(if ssh_agent_available {
                                    Color::Success
                                } else {
                                    Color::Muted
                                }),
                            ),
                    )
                    .when_some(detected_ssh_key.clone(), |el, key_path| {
                        el.child(
                            div()
                                .px(px(8.))
                                .py(px(4.))
                                .rounded(px(999.))
                                .bg(colors.surface_background)
                                .child(
                                    Label::new(format!("Detected key: {}", key_path))
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .truncate(),
                                ),
                        )
                    }),
            )
            .child(Self::setting_label(
                "SSH Key Path",
                "Optional explicit private key path. Leave empty to use ssh-agent first and then fall back to automatically detected keys in ~/.ssh.",
            ))
            .child(self.text_input(
                "git-ssh-key-input",
                FocusedField::GitSshKeyPath,
                &self.git_ssh_key_path,
                "~/.ssh/id_ed25519 (default)",
                false,
                IconName::File,
                cx,
            ))
            .when_some(detected_ssh_key, |el, key_path| {
                let already_selected = self.git_ssh_key_path.trim() == key_path;
                el.child(
                    div()
                        .h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .gap(px(12.))
                        .p(px(10.))
                        .rounded(px(8.))
                        .bg(colors.element_background)
                        .border_1()
                        .border_color(colors.border_variant)
                        .child(
                            div()
                                .v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap(px(2.))
                                .child(
                                    Label::new(if already_selected {
                                        "Detected key is currently selected"
                                    } else {
                                        "Detected default SSH key"
                                    })
                                    .size(LabelSize::Small)
                                    .weight(FontWeight::SEMIBOLD),
                                )
                                .child(
                                    Label::new(key_path.clone())
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .truncate(),
                                ),
                        )
                        .child(
                            Button::new(
                                "use-detected-ssh-key",
                                if already_selected {
                                    "Using This Key"
                                } else {
                                    "Use Detected Key"
                                },
                            )
                            .style(if already_selected {
                                ButtonStyle::Subtle
                            } else {
                                ButtonStyle::Filled
                            })
                            .size(ButtonSize::Compact)
                            .icon(IconName::Check)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.git_ssh_key_path = key_path.clone();
                                this.feedback_message = Some(
                                    "Configured the detected SSH private key for SSH remotes."
                                        .into(),
                                );
                                this.feedback_is_error = false;
                                this.save_settings(cx);
                            })),
                        ),
                )
            });
        section = section.child(ssh_card);

        // GPG Signing card
        let sign_commits = self.git_sign_commits;
        let mut gpg_card = Self::setting_card(cx);
        gpg_card = gpg_card
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .items_center()
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .gap(px(2.))
                            .child(
                                Label::new("Sign Commits")
                                    .size(LabelSize::Small)
                                    .weight(FontWeight::SEMIBOLD),
                            )
                            .child(
                                Label::new("Sign commits with your configured GPG key.")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    )
                    .child(
                        div()
                            .id("sign-commits-toggle")
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.git_sign_commits = !this.git_sign_commits;
                                this.save_settings(cx);
                            }))
                            .child(Checkbox::new(
                                "sign-commits-cb",
                                if sign_commits {
                                    CheckState::Checked
                                } else {
                                    CheckState::Unchecked
                                },
                            )),
                    ),
            )
            .child(Self::setting_label(
                "GPG Key ID",
                "Leave empty to use your existing git config default signing key.",
            ))
            .child(self.text_input(
                "git-gpg-key-input",
                FocusedField::GitGpgKeyId,
                &self.git_gpg_key_id,
                "No explicit GPG key configured",
                false,
                IconName::Eye,
                cx,
            ));
        section = section.child(gpg_card);

        section = section.child(
            Self::setting_card(cx).child(
                div()
                    .v_flex()
                    .gap(px(6.))
                    .child(Self::setting_label(
                        "Authentication Order",
                        "When contacting a remote, rgitui tries SSH first for SSH remotes, then matched provider accounts for HTTPS remotes, then the generic HTTPS fallback, then the git credential helper.",
                    ))
                    .child(
                        Label::new(
                            "If a repo remote is HTTPS, an SSH key will not be used until you change that remote to an SSH URL.",
                        )
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                    ),
            ),
        );

        section
    }

    // ── General section ─────────────────────────────────────────────────
    fn render_general_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        let current_max = self.max_recent_repos;

        let mut section = div().v_flex().w_full().gap(px(16.));

        section = section.child(Self::section_header(
            IconName::Settings,
            "General",
            "Application preferences and behavior.",
        ));

        // Recent repos card
        let mut repos_card = Self::setting_card(cx);
        repos_card = repos_card.child(
            div()
                .h_flex()
                .w_full()
                .items_center()
                .child(
                    div()
                        .v_flex()
                        .flex_1()
                        .gap(px(2.))
                        .child(
                            Label::new("Max Recent Repositories")
                                .size(LabelSize::Small)
                                .weight(FontWeight::SEMIBOLD),
                        )
                        .child(
                            Label::new("Number of repos shown in the recent list.")
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                )
                .child(
                    div()
                        .h_flex()
                        .gap(px(4.))
                        .items_center()
                        .child(
                            Button::new("max-repos-dec", "-")
                                .style(ButtonStyle::Outlined)
                                .size(ButtonSize::Compact)
                                .disabled(current_max <= 5)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    if this.max_recent_repos > 5 {
                                        this.max_recent_repos -= 5;
                                        this.save_settings(cx);
                                    }
                                })),
                        )
                        .child(
                            div()
                                .w(px(40.))
                                .h(px(28.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(colors.editor_background)
                                .border_1()
                                .border_color(colors.border)
                                .rounded(px(6.))
                                .child(
                                    Label::new(SharedString::from(current_max.to_string()))
                                        .size(LabelSize::Small)
                                        .weight(FontWeight::SEMIBOLD),
                                ),
                        )
                        .child(
                            Button::new("max-repos-inc", "+")
                                .style(ButtonStyle::Outlined)
                                .size(ButtonSize::Compact)
                                .disabled(current_max >= 50)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    if this.max_recent_repos < 50 {
                                        this.max_recent_repos += 5;
                                        this.save_settings(cx);
                                    }
                                })),
                        ),
                ),
        );
        section = section.child(repos_card);

        // UI density card (future)
        let mut density_card = Self::setting_card(cx);
        density_card = density_card.child(
            div()
                .v_flex()
                .gap(px(8.))
                .child(Self::setting_label(
                    "UI Density",
                    "Adjust the spacing and sizing of UI elements.",
                ))
                .child(
                    div()
                        .h_flex()
                        .gap(px(4.))
                        .p(px(3.))
                        .rounded(px(8.))
                        .bg(colors.element_background)
                        .child(
                            div()
                                .h_flex()
                                .h(px(28.))
                                .px(px(12.))
                                .items_center()
                                .justify_center()
                                .rounded(px(6.))
                                .child(
                                    Label::new("Compact")
                                        .size(LabelSize::XSmall)
                                        .color(Color::Disabled),
                                ),
                        )
                        .child(
                            div()
                                .h_flex()
                                .h(px(28.))
                                .px(px(12.))
                                .items_center()
                                .justify_center()
                                .rounded(px(6.))
                                .bg(colors.elevated_surface_background)
                                .border_1()
                                .border_color(colors.border)
                                .child(
                                    Label::new("Default")
                                        .size(LabelSize::XSmall)
                                        .weight(FontWeight::SEMIBOLD),
                                ),
                        )
                        .child(
                            div()
                                .h_flex()
                                .h(px(28.))
                                .px(px(12.))
                                .items_center()
                                .justify_center()
                                .rounded(px(6.))
                                .child(
                                    Label::new("Comfortable")
                                        .size(LabelSize::XSmall)
                                        .color(Color::Disabled),
                                ),
                        ),
                )
                .child(
                    Label::new("Coming soon")
                        .size(LabelSize::XSmall)
                        .color(Color::Placeholder),
                ),
        );
        section = section.child(density_card);

        // Keyboard shortcuts info card
        let mut shortcuts_card = Self::setting_card(cx);
        shortcuts_card = shortcuts_card.child(
            div()
                .v_flex()
                .gap(px(8.))
                .child(Self::setting_label(
                    "Keyboard Shortcuts",
                    "Quick reference for common actions.",
                ))
                .child(self.render_shortcut_row("Command Palette", "Ctrl+Shift+P"))
                .child(self.render_shortcut_row("Search Commits", "Ctrl+F"))
                .child(self.render_shortcut_row("Stage All", "Ctrl+S"))
                .child(self.render_shortcut_row("Unstage All", "Ctrl+Shift+S"))
                .child(self.render_shortcut_row("Commit", "Ctrl+Enter (in message)"))
                .child(self.render_shortcut_row("Open Repository", "Ctrl+O"))
                .child(self.render_shortcut_row("Refresh", "F5"))
                .child(self.render_shortcut_row("Settings", "Ctrl+,")),
        );
        section = section.child(shortcuts_card);

        section
    }

    fn render_shortcut_row(&self, action: &str, shortcut: &str) -> impl IntoElement {
        div()
            .h_flex()
            .w_full()
            .items_center()
            .child(
                Label::new(SharedString::from(action.to_string()))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .child(div().flex_1())
            .child(
                div()
                    .h_flex()
                    .h(px(20.))
                    .px(px(8.))
                    .rounded(px(4.))
                    .bg(gpui::Hsla {
                        h: 0.0,
                        s: 0.0,
                        l: 0.5,
                        a: 0.1,
                    })
                    .items_center()
                    .child(
                        Label::new(SharedString::from(shortcut.to_string()))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .weight(FontWeight::SEMIBOLD),
                    ),
            )
    }
}

impl Render for SettingsModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().id("settings-modal").into_any_element();
        }

        let mut content_container = div().v_flex().w_full().gap(px(16.));

        if let Some(message) = &self.feedback_message {
            content_container = content_container.child(
                div()
                    .w_full()
                    .h_flex()
                    .gap(px(8.))
                    .items_center()
                    .p(px(10.))
                    .rounded(px(8.))
                    .bg(if self.feedback_is_error {
                        cx.status().error_background
                    } else {
                        cx.status().success_background
                    })
                    .border_1()
                    .border_color(if self.feedback_is_error {
                        cx.status().error
                    } else {
                        cx.status().success
                    })
                    .child(
                        Icon::new(if self.feedback_is_error {
                            IconName::X
                        } else {
                            IconName::Check
                        })
                        .size(IconSize::Small)
                        .color(if self.feedback_is_error {
                            Color::Error
                        } else {
                            Color::Success
                        }),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Label::new(message.clone())
                                .size(LabelSize::Small)
                                .color(Color::Default),
                        ),
                    ),
            );
        }

        let content = match self.active_section {
            SettingsSection::Theme => self.render_theme_section(cx).into_any_element(),
            SettingsSection::Ai => self.render_ai_section(cx).into_any_element(),
            SettingsSection::Auth => self.render_git_section(cx).into_any_element(),
            SettingsSection::General => self.render_general_section(cx).into_any_element(),
        };
        content_container = content_container.child(content);

        let sidebar = self.render_sidebar(cx);
        let colors = cx.colors();
        let (page_title, page_subtitle) = match self.active_section {
            SettingsSection::Theme => (
                "Preferences",
                "Theme, layout, and visual defaults for the application.",
            ),
            SettingsSection::Ai => (
                "Preferences",
                "Provider, model, and keychain-backed AI settings.",
            ),
            SettingsSection::Auth => (
                "Preferences",
                "Account profiles, HTTPS tokens, and SSH configuration.",
            ),
            SettingsSection::General => (
                "Preferences",
                "General application behavior and workspace defaults.",
            ),
        };

        let backdrop = div()
            .id("settings-modal-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_stretch()
            .justify_center()
            .p(px(20.))
            .bg(gpui::Hsla {
                h: 0.0,
                s: 0.0,
                l: 0.0,
                a: 0.5,
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.dismiss(cx);
            }))
            .on_scroll_wheel(|_, _, cx| {
                cx.stop_propagation();
            });

        let modal = div()
            .id("settings-modal-container")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .h_flex()
            .size_full()
            .bg(colors.elevated_surface_background)
            .border_1()
            .border_color(colors.border)
            .rounded(px(14.))
            .elevation_3(cx)
            .overflow_hidden()
            .on_click(|_: &ClickEvent, _, cx| {
                cx.stop_propagation();
            })
            // Left sidebar
            .child(sidebar)
            // Right content
            .child(
                div()
                    .id("settings-page-shell")
                    .v_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.clear_focused_field(cx);
                    }))
                    .child(
                        div()
                            .h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap(px(16.))
                            .px(px(28.))
                            .py(px(20.))
                            .bg(colors.surface_background)
                            .border_b_1()
                            .border_color(colors.border_variant)
                            .child(
                                div()
                                    .v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(2.))
                                    .child(
                                        Label::new(page_title)
                                            .size(LabelSize::Large)
                                            .weight(FontWeight::BOLD),
                                    )
                                    .child(
                                        Label::new(page_subtitle)
                                            .size(LabelSize::Small)
                                            .color(Color::Muted)
                                            .truncate(),
                                    ),
                            )
                            .child(
                                Button::new("settings-close", "Close")
                                    .style(ButtonStyle::Subtle)
                                    .size(ButtonSize::Compact)
                                    .icon(IconName::X)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.dismiss(cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id("settings-content")
                            .v_flex()
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .p(px(28.))
                            .overflow_y_scroll()
                            .overflow_x_hidden()
                            .child(content_container),
                    ),
            );

        backdrop.child(modal).into_any_element()
    }
}

fn prev_char_boundary(s: &str, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let mut p = pos - 1;
    while p > 0 && !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}

fn next_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let mut p = pos + 1;
    while p < s.len() && !s.is_char_boundary(p) {
        p += 1;
    }
    p
}
