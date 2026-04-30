//! The settings view rendered inside [`SettingsWindow`].
//!
//! Save semantics:
//! - Booleans, dropdowns, and other simple controls auto-commit to
//!   [`rgitui_settings::SettingsState`] and trigger an immediate
//!   `state.save()`.
//! - Text inputs commit ONLY on `TextInputEvent::Submit` (Enter). They
//!   update local state on `Changed(_)` for live display (e.g. masking)
//!   but never write through to settings on every keystroke. This avoids
//!   round-tripping OS-keychain secrets on every character. Closing the
//!   window with un-submitted text discards the change; the user must
//!   re-paste and Enter to persist.
//!
//! [`SettingsWindow`]: super::SettingsWindow

use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, ClickEvent, Context, ElementId, Entity, EventEmitter, FontWeight,
    SharedString, Task, Window,
};
use rgitui_settings::{
    config_dir, AiSettings, AppearanceMode, AutoFetchInterval, Compactness, DiffViewMode,
    GitProviderSettings, GraphStyle, SettingsState,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt, ThemeState};
use rgitui_ui::{
    Button, ButtonSize, ButtonStyle, CheckState, Checkbox, Icon, IconName, IconSize, Label,
    LabelSize, TextInput, TextInputEvent,
};

use super::events::SettingsViewEvent;
use super::{SettingsWindowAction, SettingsWindowActionGlobal};
use crate::github_device_flow::{self, DeviceFlowStatus};

/// Which section of the settings is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsSection {
    Theme,
    Ai,
    Auth,
    General,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MaskedField {
    AiApiKey,
    GitHttpsToken,
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

/// A detected application available on the system.
#[derive(Debug, Clone)]
struct DetectedApp {
    display_name: String,
    command: String,
}

fn is_command_available(cmd: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        is_command_available_windows(cmd)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("which")
            .arg(cmd)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

#[cfg(target_os = "windows")]
fn is_command_available_windows(cmd: &str) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let output = std::process::Command::new("cmd")
        .args(["/c", "where", cmd])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match output {
        Ok(out) => {
            let path_str = String::from_utf8_lossy(&out.stdout);
            for line in path_str.lines() {
                let line = line.trim();
                if !line.is_empty()
                    && !line.to_lowercase().ends_with(".bat")
                    && !line.to_lowercase().ends_with(".cmd")
                {
                    return true;
                }
            }
            false
        }
        Err(_) => false,
    }
}

fn detect_available_terminals() -> Vec<DetectedApp> {
    let candidates: &[(&str, &str)] = {
        #[cfg(target_os = "windows")]
        {
            &[
                ("Windows Terminal", "wt"),
                ("PowerShell", "pwsh"),
                ("PowerShell (Windows)", "powershell"),
                ("Command Prompt", "cmd"),
                ("Alacritty", "alacritty"),
                ("WezTerm", "wezterm"),
            ]
        }
        #[cfg(target_os = "macos")]
        {
            &[
                ("Alacritty", "alacritty"),
                ("WezTerm", "wezterm"),
                ("Kitty", "kitty"),
                ("iTerm", "iterm2"),
            ]
        }
        #[cfg(target_os = "linux")]
        {
            &[
                ("GNOME Terminal", "gnome-terminal"),
                ("Konsole", "konsole"),
                ("Alacritty", "alacritty"),
                ("WezTerm", "wezterm"),
                ("Kitty", "kitty"),
                ("Foot", "foot"),
                ("Xterm", "xterm"),
            ]
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            &[]
        }
    };

    let mut results = Vec::new();

    #[cfg(target_os = "macos")]
    {
        results.push(DetectedApp {
            display_name: "Terminal.app".to_string(),
            command: "Terminal.app".to_string(),
        });
    }

    for &(name, cmd) in candidates {
        if is_command_available(cmd) {
            results.push(DetectedApp {
                display_name: name.to_string(),
                command: cmd.to_string(),
            });
        }
    }
    results
}

#[allow(unused_mut)]
fn detect_available_editors() -> Vec<DetectedApp> {
    let mut candidates: Vec<(&str, &str)> = vec![
        ("VS Code", "code"),
        ("Cursor", "cursor"),
        ("Zed", "zed"),
        ("Sublime Text", "subl"),
        ("Neovim", "nvim"),
        ("Vim", "vim"),
        ("Nano", "nano"),
        ("Emacs", "emacs"),
    ];

    #[cfg(target_os = "windows")]
    {
        candidates.push(("Notepad++", "notepad++"));
        candidates.push(("Notepad", "notepad"));
    }

    let mut results = Vec::new();
    for (name, cmd) in candidates {
        if is_command_available(cmd) {
            results.push(DetectedApp {
                display_name: name.to_string(),
                command: cmd.to_string(),
            });
        }
    }

    #[cfg(target_os = "macos")]
    {
        results.push(DetectedApp {
            display_name: "TextEdit".to_string(),
            command: "open -a TextEdit".to_string(),
        });
    }

    results
}

/// The settings view.
pub struct SettingsView {
    active_section: SettingsSection,

    // Theme state
    selected_theme: String,
    available_themes: Vec<String>,

    // AI state
    ai_provider: String,
    ai_model: String,
    ai_commit_style: String,
    ai_enabled: bool,
    ai_inject_project_context: bool,
    ai_use_tools: bool,

    // AI text editors
    ai_api_key_editor: Entity<TextInput>,

    // Git state
    git_sign_commits: bool,
    git_providers: Vec<EditableGitProvider>,
    selected_provider_index: usize,
    expanded_provider_kind: Option<String>,
    pending_browser_auth_provider_id: Option<String>,
    show_ai_api_key: bool,
    show_git_https_token: bool,
    show_provider_token: bool,

    // Git text editors
    git_https_token_editor: Entity<TextInput>,
    git_ssh_key_path_editor: Entity<TextInput>,
    git_gpg_key_id_editor: Entity<TextInput>,

    // Provider text editors (slot editors for the currently selected provider)
    provider_display_name_editor: Entity<TextInput>,
    provider_host_editor: Entity<TextInput>,
    provider_username_editor: Entity<TextInput>,
    provider_token_editor: Entity<TextInput>,

    // General state
    max_recent_repos: usize,
    compactness: Compactness,
    font_size: u32,
    show_line_numbers_in_diff: bool,
    diff_view_mode: DiffViewMode,
    diff_wrap_lines: bool,
    graph_style: GraphStyle,
    show_subject_column: bool,
    auto_fetch_interval: AutoFetchInterval,
    confirm_destructive_operations: bool,
    auto_check_updates: bool,
    watch_all_worktrees: bool,
    terminal_command_editor: Entity<TextInput>,
    editor_command_editor: Entity<TextInput>,
    detected_terminals: Vec<DetectedApp>,
    detected_editors: Vec<DetectedApp>,
    detection_complete: bool,
    selected_terminal_index: Option<usize>,
    selected_editor_index: Option<usize>,
    feedback_message: Option<String>,
    feedback_is_error: bool,

    // GitHub Device Flow
    device_flow_status: DeviceFlowStatus,
    device_flow_task: Option<Task<()>>,
}

impl EventEmitter<SettingsViewEvent> for SettingsView {}

impl SettingsView {
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
        let home = dirs::home_dir()?.to_string_lossy().to_string();
        for key_name in ["id_ed25519", "id_rsa", "id_ecdsa"] {
            let candidate = std::path::PathBuf::from(&home).join(".ssh").join(key_name);
            if candidate.exists() {
                return Some(candidate.display().to_string());
            }
        }
        None
    }

    pub fn new(cx: &mut Context<Self>) -> Self {
        let (settings, available_themes) = cx.read_global::<SettingsState, _>(|state, cx| {
            let settings = state.settings().clone();
            let themes: Vec<String> = cx
                .global::<ThemeState>()
                .themes_for_appearance(settings.appearance_mode)
                .iter()
                .map(|t| t.name.clone())
                .collect();
            (settings, themes)
        });

        let ai_api_key_val = cx
            .global::<SettingsState>()
            .ai_api_key()
            .unwrap_or_default();
        let git_https_token_val = cx
            .global::<SettingsState>()
            .git_https_token()
            .unwrap_or_default();
        let git_ssh_key_path_val = settings.git.ssh_key_path.clone().unwrap_or_default();
        let git_gpg_key_id_val = settings.git.gpg_key_id.clone().unwrap_or_default();

        let git_providers: Vec<EditableGitProvider> = settings
            .git
            .providers
            .iter()
            .map(|provider| EditableGitProvider {
                id: provider.id.clone(),
                kind: provider.kind.clone(),
                display_name: provider.display_name.clone(),
                host: provider.host.clone(),
                username: if provider.username == Self::default_provider_username(&provider.kind) {
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
            .collect();

        let ai_api_key_editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("Click to enter API key...");
            ti.set_masked(true);
            if !ai_api_key_val.is_empty() {
                ti.set_text(&ai_api_key_val, cx);
            }
            ti
        });

        let git_https_token_editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("Paste a generic HTTPS token only if you need a fallback...");
            ti.set_masked(true);
            if !git_https_token_val.is_empty() {
                ti.set_text(&git_https_token_val, cx);
            }
            ti
        });

        let git_ssh_key_path_editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("~/.ssh/id_ed25519 (default)");
            if !git_ssh_key_path_val.is_empty() {
                ti.set_text(&git_ssh_key_path_val, cx);
            }
            ti
        });

        let git_gpg_key_id_editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("No explicit GPG key configured");
            if !git_gpg_key_id_val.is_empty() {
                ti.set_text(&git_gpg_key_id_val, cx);
            }
            ti
        });

        let first_provider = git_providers.first();

        let provider_display_name_editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("Work / Personal / Client");
            if let Some(p) = first_provider {
                ti.set_text(&p.display_name, cx);
            }
            ti
        });

        let provider_host_editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("github.com");
            if let Some(p) = first_provider {
                ti.set_text(&p.host, cx);
            }
            ti
        });

        let provider_username_editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("Usually left empty");
            if let Some(p) = first_provider {
                ti.set_text(&p.username, cx);
            }
            ti
        });

        let provider_token_editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("Paste the access token for this account...");
            ti.set_masked(true);
            if let Some(p) = first_provider {
                ti.set_text(&p.token, cx);
            }
            ti
        });

        let terminal_command_editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("Custom command override...");
            if !settings.terminal_command.is_empty() {
                ti.set_text(&settings.terminal_command, cx);
            }
            ti
        });

        let editor_command_editor = cx.new(|cx| {
            let mut ti = TextInput::new(cx);
            ti.set_placeholder("Custom command override...");
            if !settings.editor_command.is_empty() {
                ti.set_text(&settings.editor_command, cx);
            }
            ti
        });

        let detected_terminals = Vec::new();
        let detected_editors = Vec::new();
        let selected_terminal_index = None;
        let selected_editor_index = None;

        let terminal_cmd_for_detect = settings.terminal_command.clone();
        let editor_cmd_for_detect = settings.editor_command.clone();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let terminals = cx
                .background_executor()
                .spawn(async { detect_available_terminals() })
                .await;
            let editors = cx
                .background_executor()
                .spawn(async { detect_available_editors() })
                .await;
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    this.detected_terminals = terminals;
                    this.detected_editors = editors;
                    this.detection_complete = true;
                    this.recompute_selected_indices(
                        &terminal_cmd_for_detect,
                        &editor_cmd_for_detect,
                    );
                    cx.notify();
                })
                .ok();
            });
        })
        .detach();

        cx.subscribe(
            &ai_api_key_editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| {
                if let TextInputEvent::Submit = event {
                    this.save_settings(cx);
                }
            },
        )
        .detach();

        cx.subscribe(
            &git_https_token_editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| {
                if let TextInputEvent::Submit = event {
                    this.save_settings(cx);
                }
            },
        )
        .detach();

        cx.subscribe(
            &git_ssh_key_path_editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| {
                if let TextInputEvent::Submit = event {
                    this.save_settings(cx);
                }
            },
        )
        .detach();

        cx.subscribe(
            &git_gpg_key_id_editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| {
                if let TextInputEvent::Submit = event {
                    this.save_settings(cx);
                }
            },
        )
        .detach();

        cx.subscribe(
            &provider_display_name_editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| match event {
                TextInputEvent::Changed(text) => {
                    if let Some(provider) = this.git_providers.get_mut(this.selected_provider_index)
                    {
                        provider.display_name = text.clone();
                    }
                }
                TextInputEvent::Submit => {
                    this.save_settings(cx);
                }
            },
        )
        .detach();

        cx.subscribe(
            &provider_host_editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| match event {
                TextInputEvent::Changed(text) => {
                    if let Some(provider) = this.git_providers.get_mut(this.selected_provider_index)
                    {
                        provider.host = text.clone();
                    }
                }
                TextInputEvent::Submit => {
                    this.save_settings(cx);
                }
            },
        )
        .detach();

        cx.subscribe(
            &provider_username_editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| match event {
                TextInputEvent::Changed(text) => {
                    if let Some(provider) = this.git_providers.get_mut(this.selected_provider_index)
                    {
                        provider.username = text.clone();
                    }
                }
                TextInputEvent::Submit => {
                    this.save_settings(cx);
                }
            },
        )
        .detach();

        cx.subscribe(
            &provider_token_editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| match event {
                TextInputEvent::Changed(text) => {
                    if let Some(provider) = this.git_providers.get_mut(this.selected_provider_index)
                    {
                        provider.token = text.clone();
                        provider.has_token = !provider.token.is_empty();
                    }
                }
                TextInputEvent::Submit => {
                    this.complete_browser_onboarding_for_current_provider();
                    this.save_settings(cx);
                }
            },
        )
        .detach();

        cx.subscribe(
            &terminal_command_editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| match event {
                TextInputEvent::Changed(text) => {
                    if text.is_empty() {
                        this.selected_terminal_index = if this.detected_terminals.is_empty() {
                            None
                        } else {
                            Some(0)
                        };
                    } else {
                        this.selected_terminal_index = this
                            .detected_terminals
                            .iter()
                            .position(|app| app.command == *text);
                    }
                }
                TextInputEvent::Submit => {
                    this.save_settings(cx);
                }
            },
        )
        .detach();

        cx.subscribe(
            &editor_command_editor,
            |this: &mut Self, _, event: &TextInputEvent, cx| match event {
                TextInputEvent::Changed(text) => {
                    if text.is_empty() {
                        this.selected_editor_index = if this.detected_editors.is_empty() {
                            None
                        } else {
                            Some(0)
                        };
                    } else {
                        this.selected_editor_index = this
                            .detected_editors
                            .iter()
                            .position(|app| app.command == *text);
                    }
                }
                TextInputEvent::Submit => {
                    this.save_settings(cx);
                }
            },
        )
        .detach();

        Self {
            active_section: SettingsSection::Theme,
            selected_theme: settings.theme.clone(),
            available_themes,
            ai_provider: settings.ai.provider.clone(),
            ai_model: settings.ai.model.clone(),
            ai_commit_style: settings.ai.commit_style.clone(),
            ai_enabled: settings.ai.enabled,
            ai_inject_project_context: settings.ai.inject_project_context,
            ai_use_tools: settings.ai.use_tools,
            ai_api_key_editor,
            git_sign_commits: settings.git.sign_commits,
            git_providers,
            selected_provider_index: 0,
            expanded_provider_kind: None,
            pending_browser_auth_provider_id: None,
            show_ai_api_key: false,
            show_git_https_token: false,
            show_provider_token: false,
            git_https_token_editor,
            git_ssh_key_path_editor,
            git_gpg_key_id_editor,
            provider_display_name_editor,
            provider_host_editor,
            provider_username_editor,
            provider_token_editor,
            max_recent_repos: settings.max_recent_repos,
            compactness: settings.compactness,
            font_size: settings.font_size,
            show_line_numbers_in_diff: settings.show_line_numbers_in_diff,
            diff_view_mode: settings.diff_view_mode,
            diff_wrap_lines: settings.diff_wrap_lines,
            graph_style: settings.graph_style,
            show_subject_column: settings.show_subject_column,
            auto_fetch_interval: settings.auto_fetch_interval,
            confirm_destructive_operations: settings.confirm_destructive_operations,
            auto_check_updates: settings.auto_check_updates,
            watch_all_worktrees: settings.watch_all_worktrees,
            terminal_command_editor,
            editor_command_editor,
            detected_terminals,
            detected_editors,
            detection_complete: false,
            selected_terminal_index,
            selected_editor_index,
            feedback_message: None,
            feedback_is_error: false,
            device_flow_status: DeviceFlowStatus::Idle,
            device_flow_task: None,
        }
    }

    pub(super) fn reload_from_settings(&mut self, cx: &mut Context<Self>) {
        let (ai_api_key_val, git_https_token_val, git_ssh_key_path_val, git_gpg_key_id_val) =
            cx.read_global::<SettingsState, _>(|state, _cx| {
                let s = state.settings();
                self.selected_theme = s.theme.clone();
                self.ai_provider = s.ai.provider.clone();
                self.ai_model = s.ai.model.clone();
                self.ai_commit_style = s.ai.commit_style.clone();
                self.ai_enabled = s.ai.enabled;
                self.ai_inject_project_context = s.ai.inject_project_context;
                self.ai_use_tools = s.ai.use_tools;
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
                self.compactness = s.compactness;
                self.font_size = s.font_size;
                self.show_line_numbers_in_diff = s.show_line_numbers_in_diff;
                self.diff_view_mode = s.diff_view_mode;
                self.diff_wrap_lines = s.diff_wrap_lines;
                self.graph_style = s.graph_style;
                self.show_subject_column = s.show_subject_column;
                self.auto_fetch_interval = s.auto_fetch_interval;
                self.confirm_destructive_operations = s.confirm_destructive_operations;
                self.auto_check_updates = s.auto_check_updates;
                self.watch_all_worktrees = s.watch_all_worktrees;
                self.feedback_message = None;
                self.feedback_is_error = false;
                (
                    state.ai_api_key().unwrap_or_default(),
                    state.git_https_token().unwrap_or_default(),
                    s.git.ssh_key_path.clone().unwrap_or_default(),
                    s.git.gpg_key_id.clone().unwrap_or_default(),
                )
            });

        let terminal_cmd = cx.read_global::<SettingsState, _>(|state, _cx| {
            state.settings().terminal_command.clone()
        });
        let editor_cmd = cx
            .read_global::<SettingsState, _>(|state, _cx| state.settings().editor_command.clone());
        self.terminal_command_editor
            .update(cx, |e, cx| e.set_text(&terminal_cmd, cx));
        self.editor_command_editor
            .update(cx, |e, cx| e.set_text(&editor_cmd, cx));

        self.recompute_selected_indices(&terminal_cmd, &editor_cmd);

        self.ai_api_key_editor
            .update(cx, |e, cx| e.set_text(ai_api_key_val, cx));
        self.git_https_token_editor
            .update(cx, |e, cx| e.set_text(git_https_token_val, cx));
        self.git_ssh_key_path_editor
            .update(cx, |e, cx| e.set_text(git_ssh_key_path_val, cx));
        self.git_gpg_key_id_editor
            .update(cx, |e, cx| e.set_text(git_gpg_key_id_val, cx));

        self.sync_provider_editors(cx);
    }

    fn recompute_selected_indices(&mut self, terminal_cmd: &str, editor_cmd: &str) {
        self.selected_terminal_index = if terminal_cmd.is_empty() {
            if self.detected_terminals.is_empty() {
                None
            } else {
                Some(0)
            }
        } else {
            self.detected_terminals
                .iter()
                .position(|app| app.command == terminal_cmd)
        };

        self.selected_editor_index = if editor_cmd.is_empty() {
            if self.detected_editors.is_empty() {
                None
            } else {
                Some(0)
            }
        } else {
            self.detected_editors
                .iter()
                .position(|app| app.command == editor_cmd)
        };
    }

    fn set_appearance_mode(&mut self, mode: AppearanceMode, cx: &mut Context<Self>) {
        cx.update_global::<SettingsState, _>(|state, _cx| {
            state.settings_mut().appearance_mode = mode;
            if let Err(e) = state.save() {
                log::error!("Failed to save settings: {}", e);
            }
        });

        let available_themes = cx.global::<ThemeState>().themes_for_appearance(mode);
        self.available_themes = available_themes.iter().map(|t| t.name.clone()).collect();

        if !self.available_themes.contains(&self.selected_theme) {
            if let Some(first_theme) = available_themes.first() {
                let name = first_theme.name.clone();
                self.apply_theme(name, cx);
            }
        }
        cx.notify();
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
        cx.emit(SettingsViewEvent::ThemeChanged(theme_name.clone()));
        SettingsWindowActionGlobal::try_send(cx, SettingsWindowAction::ThemeChanged(theme_name));
        cx.notify();
    }

    fn save_settings(&mut self, cx: &mut Context<Self>) {
        let ai_api_key = self.ai_api_key_editor.read(cx).text().to_string();
        let git_https_token = self.git_https_token_editor.read(cx).text().to_string();
        let git_ssh_key_path = self.git_ssh_key_path_editor.read(cx).text().to_string();
        let git_gpg_key_id = self.git_gpg_key_id_editor.read(cx).text().to_string();
        let terminal_command = self.terminal_command_editor.read(cx).text().to_string();
        let editor_command = self.editor_command_editor.read(cx).text().to_string();

        let result = cx.update_global::<SettingsState, _>(|state, _cx| -> anyhow::Result<()> {
            state.settings_mut().ai = AiSettings {
                provider: self.ai_provider.clone(),
                legacy_api_key: None,
                has_api_key: !ai_api_key.trim().is_empty(),
                model: self.ai_model.clone(),
                commit_style: self.ai_commit_style.clone(),
                enabled: self.ai_enabled,
                inject_project_context: self.ai_inject_project_context,
                use_tools: self.ai_use_tools,
            };
            state.settings_mut().max_recent_repos = self.max_recent_repos;
            state.settings_mut().compactness = self.compactness;
            state.settings_mut().font_size = self.font_size;
            state.settings_mut().show_line_numbers_in_diff = self.show_line_numbers_in_diff;
            state.settings_mut().diff_view_mode = self.diff_view_mode;
            state.settings_mut().diff_wrap_lines = self.diff_wrap_lines;
            state.settings_mut().graph_style = self.graph_style;
            state.settings_mut().show_subject_column = self.show_subject_column;
            state.settings_mut().auto_fetch_interval = self.auto_fetch_interval;
            state.settings_mut().confirm_destructive_operations =
                self.confirm_destructive_operations;
            state.settings_mut().auto_check_updates = self.auto_check_updates;
            state.settings_mut().watch_all_worktrees = self.watch_all_worktrees;
            state.settings_mut().terminal_command = terminal_command.trim().to_string();
            state.settings_mut().editor_command = editor_command.trim().to_string();
            {
                let git = state.settings_mut();
                git.git.legacy_https_token = None;
                git.git.has_https_token = !git_https_token.trim().is_empty();
                git.git.ssh_key_path = if git_ssh_key_path.trim().is_empty() {
                    None
                } else {
                    Some(git_ssh_key_path.trim().to_string())
                };
                git.git.gpg_key_id = if git_gpg_key_id.trim().is_empty() {
                    None
                } else {
                    Some(git_gpg_key_id.trim().to_string())
                };
                git.git.sign_commits = self.git_sign_commits;
            }

            state.set_ai_api_key(Some(ai_api_key.trim()).filter(|v| !v.is_empty()))?;
            state.set_git_https_token(Some(git_https_token.trim()).filter(|v| !v.is_empty()))?;

            // Write provider tokens to keychain BEFORE replacing providers,
            // so that sync_auth_runtime inside replace_git_providers can
            // resolve the tokens from the keychain immediately.
            for provider in &self.git_providers {
                if !provider.host.trim().is_empty() {
                    state.set_git_provider_token(
                        &provider.id,
                        Some(provider.token.trim()).filter(|v| !v.is_empty()),
                    )?;
                }
            }
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
            state.save()?;
            Ok(())
        });

        match result {
            Ok(()) => {
                self.feedback_message = None;
                self.feedback_is_error = false;
                SettingsWindowActionGlobal::try_send(cx, SettingsWindowAction::SettingsChanged);
            }
            Err(error) => {
                log::error!("Failed to save settings: {}", error);
                self.feedback_message = Some(error.to_string());
                self.feedback_is_error = true;
            }
        }
        cx.notify();
    }

    fn sync_provider_editors(&self, cx: &mut Context<Self>) {
        if let Some(provider) = self.current_provider() {
            let dn = provider.display_name.clone();
            let host = provider.host.clone();
            let uname = provider.username.clone();
            let token = provider.token.clone();
            self.provider_display_name_editor
                .update(cx, |e, cx| e.set_text(dn, cx));
            self.provider_host_editor
                .update(cx, |e, cx| e.set_text(host, cx));
            self.provider_username_editor
                .update(cx, |e, cx| e.set_text(uname, cx));
            self.provider_token_editor
                .update(cx, |e, cx| e.set_text(token, cx));
        } else {
            self.provider_display_name_editor
                .update(cx, |e, cx| e.clear(cx));
            self.provider_host_editor.update(cx, |e, cx| e.clear(cx));
            self.provider_username_editor
                .update(cx, |e, cx| e.clear(cx));
            self.provider_token_editor.update(cx, |e, cx| e.clear(cx));
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

    fn focus_provider_token_for_kind(
        &mut self,
        kind: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.provider_indices_for_kind(kind).is_empty() {
            self.add_provider(kind, cx);
        } else {
            self.select_provider_kind(kind, cx);
        }
        self.sync_provider_editors(cx);
        self.provider_token_editor
            .update(cx, |e, cx| e.focus(window, cx));
        self.feedback_message = Some(
            "Paste the token into the selected profile. It will be saved to your OS keychain."
                .into(),
        );
        self.feedback_is_error = false;
        cx.notify();
    }

    fn paste_provider_token_for_kind(
        &mut self,
        kind: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
                provider.token = token.clone();
                provider.has_token = !provider.token.is_empty();
            }
            self.provider_token_editor
                .update(cx, |e, cx| e.set_text(token, cx));
            self.complete_browser_onboarding_for_current_provider();
            self.save_settings(cx);
        } else {
            self.focus_provider_token_for_kind(kind, window, cx);
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
        self.sync_provider_editors(cx);
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
        self.sync_provider_editors(cx);
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
            self.device_flow_status = DeviceFlowStatus::Idle;
            self.device_flow_task = None;
        }
        self.selected_provider_index = self
            .selected_provider_index
            .min(self.git_providers.len().saturating_sub(1));
        self.sync_provider_editors(cx);
        self.feedback_message = Some(format!("Removed provider '{}'.", provider.display_name));
        self.feedback_is_error = false;
        self.save_settings(cx);
    }

    fn is_field_unmasked(&self, field: MaskedField) -> bool {
        match field {
            MaskedField::AiApiKey => self.show_ai_api_key,
            MaskedField::GitHttpsToken => self.show_git_https_token,
            MaskedField::ProviderToken => self.show_provider_token,
        }
    }

    fn toggle_mask_visibility(&mut self, field: MaskedField, cx: &mut Context<Self>) {
        match field {
            MaskedField::AiApiKey => {
                self.show_ai_api_key = !self.show_ai_api_key;
                let masked = !self.show_ai_api_key;
                self.ai_api_key_editor
                    .update(cx, |e, _cx| e.set_masked(masked));
            }
            MaskedField::GitHttpsToken => {
                self.show_git_https_token = !self.show_git_https_token;
                let masked = !self.show_git_https_token;
                self.git_https_token_editor
                    .update(cx, |e, _cx| e.set_masked(masked));
            }
            MaskedField::ProviderToken => {
                self.show_provider_token = !self.show_provider_token;
                let masked = !self.show_provider_token;
                self.provider_token_editor
                    .update(cx, |e, _cx| e.set_masked(masked));
            }
        }
        cx.notify();
    }

    fn import_from_clipboard(&mut self, field: MaskedField, cx: &mut Context<Self>) {
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
            MaskedField::AiApiKey => {
                self.ai_api_key_editor
                    .update(cx, |e, cx| e.set_text(&imported, cx));
            }
            MaskedField::GitHttpsToken => {
                self.git_https_token_editor
                    .update(cx, |e, cx| e.set_text(&imported, cx));
            }
            MaskedField::ProviderToken => {
                if let Some(provider) = self.current_provider_mut() {
                    provider.token = imported.clone();
                    provider.has_token = !provider.token.is_empty();
                }
                self.provider_token_editor
                    .update(cx, |e, cx| e.set_text(&imported, cx));
                self.complete_browser_onboarding_for_current_provider();
            }
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

        if kind == "github" {
            self.start_github_device_flow(cx);
            return;
        }

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

    fn start_github_device_flow(&mut self, cx: &mut Context<Self>) {
        self.device_flow_status = DeviceFlowStatus::WaitingForUser {
            user_code: "Loading...".into(),
            verification_uri: String::new(),
        };
        self.feedback_message = Some("Starting GitHub login...".into());
        self.feedback_is_error = false;
        cx.notify();

        let http = cx.http_client();
        // Held on the entity so the polling future is dropped (and cancelled
        // at its next .await) when the settings window closes.
        let task = cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let device_code_result = github_device_flow::request_device_code(http.clone()).await;

            let device_resp = match device_code_result {
                Ok(resp) => resp,
                Err(e) => {
                    cx.update(|cx| {
                        this.update(cx, |modal, cx| {
                            modal.device_flow_status = DeviceFlowStatus::Error(e.clone());
                            modal.feedback_message = Some(format!("Device flow failed: {e}"));
                            modal.feedback_is_error = true;
                            cx.notify();
                        })
                        .ok();
                    });
                    return;
                }
            };

            let user_code = device_resp.user_code.clone();
            let verification_uri = device_resp.verification_uri.clone();
            let device_code = device_resp.device_code.clone();
            let mut interval = std::time::Duration::from_secs(device_resp.interval.max(5));
            let expires_at =
                std::time::Instant::now() + std::time::Duration::from_secs(device_resp.expires_in);

            cx.update(|cx| {
                this.update(cx, |modal, cx| {
                    modal.device_flow_status = DeviceFlowStatus::WaitingForUser {
                        user_code: user_code.clone(),
                        verification_uri: verification_uri.clone(),
                    };
                    modal.feedback_message =
                        Some(format!("Enter code {} at {}", user_code, verification_uri));
                    modal.feedback_is_error = false;
                    cx.open_url(&verification_uri);
                    cx.notify();
                })
                .ok();
            });

            loop {
                smol::Timer::after(interval).await;

                if std::time::Instant::now() > expires_at {
                    cx.update(|cx| {
                        this.update(cx, |modal, cx| {
                            modal.device_flow_status = DeviceFlowStatus::Error(
                                "Device code expired. Please try again.".into(),
                            );
                            modal.feedback_message =
                                Some("Device code expired. Please try again.".into());
                            modal.feedback_is_error = true;
                            modal.pending_browser_auth_provider_id = None;
                            cx.notify();
                        })
                        .ok();
                    });
                    return;
                }

                match github_device_flow::poll_for_token(http.clone(), &device_code).await {
                    Ok(github_device_flow::PollResult::Pending) => continue,
                    Ok(github_device_flow::PollResult::SlowDown) => {
                        interval += std::time::Duration::from_secs(5);
                        continue;
                    }
                    Ok(github_device_flow::PollResult::Token(token)) => {
                        cx.update(|cx| {
                            this.update(cx, |modal, cx| {
                                // Find the provider that initiated this flow by its ID,
                                // not by selected_provider_index (which may have changed).
                                let target_id = modal.pending_browser_auth_provider_id.clone();
                                let target_index = target_id.as_ref().and_then(|id| {
                                    modal.git_providers.iter().position(|p| &p.id == id)
                                });
                                if let Some(index) = target_index {
                                    modal.git_providers[index].token = token.clone();
                                    modal.git_providers[index].has_token = true;
                                    // Update editor if this provider is currently selected
                                    if modal.selected_provider_index == index {
                                        modal.provider_token_editor.update(cx, |e, cx| {
                                            e.set_text(&token, cx);
                                        });
                                    }
                                } else if let Some(provider) = modal.current_provider_mut() {
                                    // Fallback: use current provider if ID tracking lost
                                    provider.token = token.clone();
                                    provider.has_token = true;
                                    modal.provider_token_editor.update(cx, |e, cx| {
                                        e.set_text(&token, cx);
                                    });
                                }
                                modal.device_flow_status = DeviceFlowStatus::Success;
                                modal.pending_browser_auth_provider_id = None;
                                modal.save_settings(cx);
                                modal.feedback_message =
                                    Some("GitHub account connected successfully!".into());
                                modal.feedback_is_error = false;
                                cx.notify();
                            })
                            .ok();
                        });
                        return;
                    }
                    Err(e) => {
                        cx.update(|cx| {
                            this.update(cx, |modal, cx| {
                                modal.device_flow_status = DeviceFlowStatus::Error(e.clone());
                                modal.feedback_message = Some(e);
                                modal.feedback_is_error = true;
                                modal.pending_browser_auth_provider_id = None;
                                cx.notify();
                            })
                            .ok();
                        });
                        return;
                    }
                }
            }
        });
        self.device_flow_task = Some(task);
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
            .p(px(16.))
            .gap(px(12.))
            .rounded(px(8.))
            .bg(colors.surface_background)
            .border_1()
            .border_color(colors.border_variant)
    }

    fn section_divider(cx: &Context<Self>) -> gpui::Div {
        let colors = cx.colors().clone();
        div().w_full().h(px(1.)).bg(colors.border_variant)
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

    fn masked_editor_row(
        &self,
        id: &'static str,
        editor: &Entity<TextInput>,
        masked_field: MaskedField,
        icon: IconName,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let show_unmasked = self.is_field_unmasked(masked_field);
        div()
            .h_flex()
            .w_full()
            .gap(px(8.))
            .items_center()
            .child(Icon::new(icon).size(IconSize::Small).color(Color::Muted))
            .child(div().flex_1().min_w_0().child(editor.clone()))
            .child(
                Button::new(
                    ElementId::Name(SharedString::from(format!("{id}-import"))),
                    "Paste",
                )
                .style(ButtonStyle::Subtle)
                .size(ButtonSize::Compact)
                .icon(IconName::Copy)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.import_from_clipboard(masked_field, cx);
                })),
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
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.toggle_mask_visibility(masked_field, cx);
                })),
            )
    }

    fn icon_editor_row(&self, editor: &Entity<TextInput>, icon: IconName) -> gpui::Div {
        div()
            .h_flex()
            .w_full()
            .gap(px(8.))
            .items_center()
            .child(Icon::new(icon).size(IconSize::Small).color(Color::Muted))
            .child(div().flex_1().min_w_0().child(editor.clone()))
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
            .px(px(8.));

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
                    .h(px(36.))
                    .px(px(10.))
                    .gap(px(8.))
                    .items_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(is_active, |el| {
                        el.bg(colors.ghost_element_selected)
                            .border_l_2()
                            .border_color(colors.border_focused)
                    })
                    .when(!is_active, |el| {
                        el.hover(|s| s.bg(colors.ghost_element_hover))
                            .active(|s| s.bg(colors.ghost_element_active))
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

        // Appearance Mode (Auto/Light/Dark)
        let mut app_card = Self::setting_card(cx);
        app_card = app_card.child(Self::setting_label(
            "Appearance Mode",
            "Choose whether to show light or dark themes, or auto-detect.",
        ));

        let appearance_mode = cx.global::<SettingsState>().settings().appearance_mode;
        let mut app_options = div().h_flex().w_full().gap(px(8.));

        for (mode, label, icon) in [
            (AppearanceMode::Auto, "Auto", IconName::Info),
            (AppearanceMode::Light, "Light", IconName::Info), // Replace with Sun/Moon icons if available
            (AppearanceMode::Dark, "Dark", IconName::Info),
        ] {
            let is_selected = mode == appearance_mode;
            let colors_clone = colors.clone();

            let mode_str = format!("appearance-mode-{}", mode);
            app_options = app_options.child(
                div()
                    .id(mode_str)
                    .h_flex()
                    .px(px(12.))
                    .py(px(6.))
                    .gap(px(6.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(is_selected, |el| {
                        el.bg(colors_clone.ghost_element_selected)
                            .border_1()
                            .border_color(colors_clone.border_focused)
                    })
                    .when(!is_selected, |el| {
                        el.border_1()
                            .border_color(colors_clone.border_variant)
                            .hover(|s| s.bg(colors_clone.ghost_element_hover))
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.set_appearance_mode(mode, cx);
                    }))
                    .child(Icon::new(icon).size(IconSize::Small))
                    .child(Label::new(label).size(LabelSize::Small)),
            );
        }
        app_card = app_card.child(app_options);
        section = section.child(app_card);

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
                                .h(px(22.))
                                .px(px(8.))
                                .rounded(px(4.))
                                .bg(colors.hint_background)
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

        // Edit theme button
        section = section.child({
            let mut card = Self::setting_card(cx);
            card = card.child(
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
                                Label::new("Custom Theme Editor")
                                    .size(LabelSize::Small)
                                    .weight(FontWeight::SEMIBOLD),
                            )
                            .child(
                                Label::new(
                                    "Edit colors, create custom themes, and export as JSON.",
                                )
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                            ),
                    )
                    .child(
                        Button::new("open-theme-editor", "Edit Theme")
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Subtle)
                            .icon(IconName::Edit)
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                SettingsWindowActionGlobal::try_send(
                                    cx,
                                    SettingsWindowAction::OpenThemeEditor,
                                );
                            })),
                    ),
            );
            card
        });

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

        // Inject project context card
        let inject_ctx = self.ai_inject_project_context;
        let mut inject_ctx_card = Self::setting_card(cx);
        inject_ctx_card = inject_ctx_card.child(
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
                            Label::new("Inject project context")
                                .size(LabelSize::Small)
                                .weight(FontWeight::SEMIBOLD),
                        )
                        .child(
                            Label::new(
                                "Include README.md, CLAUDE.md, and AGENTS.md in the AI prompt",
                            )
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                        ),
                )
                .child(
                    div()
                        .id("ai-inject-ctx-toggle")
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.ai_inject_project_context = !this.ai_inject_project_context;
                            this.save_settings(cx);
                        }))
                        .child(Checkbox::new(
                            "ai-inject-ctx-cb",
                            if inject_ctx {
                                CheckState::Checked
                            } else {
                                CheckState::Unchecked
                            },
                        )),
                ),
        );
        section = section.child(inject_ctx_card);

        // Use tools card
        let use_tools = self.ai_use_tools;
        let mut use_tools_card = Self::setting_card(cx);
        use_tools_card = use_tools_card.child(
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
                            Label::new("Use AI tools")
                                .size(LabelSize::Small)
                                .weight(FontWeight::SEMIBOLD),
                        )
                        .child(
                            Label::new(
                                "Allow the AI to request file contents and commit history for better messages",
                            )
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                        ),
                )
                .child(
                    div()
                        .id("ai-use-tools-toggle")
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.ai_use_tools = !this.ai_use_tools;
                            this.save_settings(cx);
                        }))
                        .child(Checkbox::new(
                            "ai-use-tools-cb",
                            if use_tools {
                                CheckState::Checked
                            } else {
                                CheckState::Unchecked
                            },
                        )),
                ),
        );
        section = section.child(use_tools_card);

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
            .child(self.masked_editor_row(
                "ai-api-key-input",
                &self.ai_api_key_editor,
                MaskedField::AiApiKey,
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
                        Button::new("add-github-account", "Sign in with GitHub")
                            .style(ButtonStyle::Filled)
                            .size(ButtonSize::Compact)
                            .icon(IconName::Plus)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.connect_provider_in_browser("github", true, cx);
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

        if let DeviceFlowStatus::WaitingForUser {
            ref user_code,
            ref verification_uri,
        } = self.device_flow_status
        {
            let code = user_code.clone();
            let uri: SharedString = verification_uri.clone().into();
            let mut device_card = Self::setting_card(cx);
            device_card = device_card
                .child(Self::setting_label(
                    "GitHub Device Login",
                    "A browser window has been opened. Enter the code below to authorize rgitui.",
                ))
                .child(
                    div()
                        .v_flex()
                        .w_full()
                        .p(px(16.))
                        .gap(px(12.))
                        .rounded(px(8.))
                        .bg(colors.element_background)
                        .border_1()
                        .border_color(colors.border_focused)
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    Icon::new(IconName::ExternalLink)
                                        .size(IconSize::Small)
                                        .color(Color::Accent),
                                )
                                .child(Label::new(uri).size(LabelSize::Small).color(Color::Accent)),
                        )
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .gap(px(12.))
                                .child(
                                    Label::new("Your code:")
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                )
                                .child(
                                    div()
                                        .px(px(12.))
                                        .py(px(6.))
                                        .rounded(px(6.))
                                        .bg(colors.surface_background)
                                        .border_1()
                                        .border_color(colors.border_variant)
                                        .child(
                                            Label::new(code.clone())
                                                .size(LabelSize::Large)
                                                .weight(FontWeight::BOLD),
                                        ),
                                )
                                .child(
                                    Button::new("copy-device-code", "Copy Code")
                                        .style(ButtonStyle::Outlined)
                                        .size(ButtonSize::Compact)
                                        .icon(IconName::Copy)
                                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                                code.clone(),
                                            ));
                                        })),
                                ),
                        )
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .gap(px(6.))
                                .child(
                                    Icon::new(IconName::Refresh)
                                        .size(IconSize::XSmall)
                                        .color(Color::Warning),
                                )
                                .child(
                                    Label::new("Waiting for authorization...")
                                        .size(LabelSize::XSmall)
                                        .color(Color::Warning),
                                ),
                        ),
                );
            section = section.child(device_card);
        }

        if let DeviceFlowStatus::Error(ref err) = self.device_flow_status {
            let err_text = err.to_string();
            let mut err_card = Self::setting_card(cx);
            err_card = err_card.child(
                div()
                    .v_flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(32.))
                    .py(px(24.))
                    .child(
                        Icon::new(IconName::AlertTriangle)
                            .size(IconSize::Large)
                            .color(Color::Error),
                    )
                    .child(
                        Label::new(err_text.clone())
                            .size(LabelSize::Small)
                            .color(Color::Error),
                    )
                    .child(
                        Button::new("retry-gh-login", "Retry")
                            .icon(IconName::Refresh)
                            .size(ButtonSize::Default)
                            .style(ButtonStyle::Filled)
                            .color(Color::Accent)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.start_github_device_flow(cx);
                            })),
                    ),
            );
            section = section.child(err_card);
        }

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
                    .bg(colors.element_background)
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
                            this.sync_provider_editors(cx);
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
                        .child(
                            div()
                                .h_flex()
                                .gap(px(6.))
                                .items_center()
                                .child(div().w(px(8.)).h(px(8.)).rounded(px(4.)).bg(if has_token {
                                    cx.status().success
                                } else if browser_login_pending {
                                    cx.status().warning
                                } else {
                                    cx.status().error
                                }))
                                .child(Label::new(status_label).size(LabelSize::XSmall).color(
                                    if has_token {
                                        Color::Success
                                    } else {
                                        Color::Warning
                                    },
                                )),
                        )
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
                                        move |this, _: &ClickEvent, window, cx| {
                                            this.selected_provider_index = remove_index;
                                            this.sync_provider_editors(cx);
                                            if connect_kind == "custom" {
                                                this.provider_host_editor.update(cx, |e, cx| {
                                                    e.focus(window, cx);
                                                });
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
                                if kind == "custom" {
                                    "Manual Setup"
                                } else if kind == "github" {
                                    "Sign in with GitHub"
                                } else {
                                    "Connect in Browser"
                                },
                            )
                            .style(ButtonStyle::Filled)
                            .size(ButtonSize::Compact)
                            .icon(if kind == "custom" {
                                IconName::Edit
                            } else {
                                IconName::ExternalLink
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                if connect_kind == "custom" {
                                    this.provider_host_editor.update(cx, |e, cx| {
                                        e.focus(window, cx);
                                    });
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
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    this.paste_provider_token_for_kind(&paste_kind, window, cx);
                                })),
                        ),
                )
                .child(Self::setting_label(
                    "Account Label",
                    "Use a clear label like Work, Personal, or Client so switching is obvious.",
                ))
                .child(self.icon_editor_row(
                    &self.provider_display_name_editor,
                    IconName::User,
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
                .child(self.icon_editor_row(
                    &self.provider_host_editor,
                    IconName::ExternalLink,
                ))
                .child(Self::setting_label(
                    "Access Token",
                    "Auto-filled when you sign in via browser. Stored securely in your OS keychain. You can also paste a Personal Access Token manually.",
                ))
                .child(self.masked_editor_row(
                    "provider-token",
                    &self.provider_token_editor,
                    MaskedField::ProviderToken,
                    IconName::Eye,
                    cx,
                ))
                .when(show_manual_username, |el| {
                    el.child(Self::setting_label(
                        "Advanced HTTPS Username",
                        "Leave this empty unless your server explicitly requires a custom username. Built-in providers infer the correct default automatically.",
                    ))
                    .child(self.icon_editor_row(
                        &self.provider_username_editor,
                        IconName::User,
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
                            "When enabled, this account's token is used for fetch/pull/push to remotes matching the host above. Also enables automatic SSH-to-HTTPS rewriting.",
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
            .child(self.masked_editor_row(
                "git-https-token-input",
                &self.git_https_token_editor,
                MaskedField::GitHttpsToken,
                IconName::Eye,
                cx,
            ));
        section = section.child(https_card);

        // SSH Key Path card
        let detected_ssh_key = Self::detected_default_ssh_key_path();
        let ssh_path_val = self.git_ssh_key_path_editor.read(cx).text().to_string();
        let has_ssh_key = !ssh_path_val.trim().is_empty();
        let mut ssh_card = Self::setting_card(cx);
        ssh_card = ssh_card
            .child(Self::setting_label(
                "SSH Key Override",
                "Optional. If set, this key is tried first for SSH remotes. If SSH fails and you have a GitHub account above, rgitui automatically falls back to HTTPS.",
            ))
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .gap(px(6.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(self.icon_editor_row(
                                &self.git_ssh_key_path_editor,
                                IconName::File,
                            )),
                    )
                    .when(has_ssh_key, |el| {
                        el.child(
                            Button::new("clear-ssh-key", "Clear")
                                .style(ButtonStyle::Outlined)
                                .size(ButtonSize::Compact)
                                .icon(IconName::X)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.git_ssh_key_path_editor
                                        .update(cx, |e, cx| e.set_text("", cx));
                                    this.save_settings(cx);
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .when_some(detected_ssh_key, |el, key_path| {
                let already_selected = ssh_path_val.trim() == key_path;
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
                                    Label::new(format!("Detected: {}", key_path.clone()))
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .truncate(),
                                ),
                        )
                        .when(!already_selected, |el| {
                            el.child(
                                Button::new("use-detected-ssh-key", "Use This Key")
                                    .style(ButtonStyle::Outlined)
                                    .size(ButtonSize::Compact)
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        this.git_ssh_key_path_editor
                                            .update(cx, |e, cx| e.set_text(key_path.clone(), cx));
                                        this.save_settings(cx);
                                    })),
                            )
                        }),
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
            .child(self.icon_editor_row(&self.git_gpg_key_id_editor, IconName::Eye));
        section = section.child(gpg_card);

        section = section.child(
            Self::setting_card(cx).child(
                div()
                    .v_flex()
                    .gap(px(6.))
                    .child(Self::setting_label(
                        "How Authentication Works",
                        "rgitui detects the remote URL protocol and authenticates accordingly.",
                    ))
                    .child(
                        Label::new(
                            "HTTPS remotes: uses your GitHub account token (above), or falls back to your system git credential helper (e.g. gh auth).",
                        )
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                    )
                    .child(
                        Label::new(
                            "SSH remotes: if no SSH key is configured above, and your system has an HTTPS credential helper, rgitui automatically rewrites SSH to HTTPS so your GitHub login works.",
                        )
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                    )
                    .child(
                        Label::new(
                            "SSH remotes with a key: uses the SSH key path above (or ssh-agent) directly.",
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
        let colors = cx.colors().clone();
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

        // UI density card
        let current_compactness = self.compactness;
        let mut density_card = Self::setting_card(cx);
        density_card = density_card.child(
            div()
                .v_flex()
                .gap(px(8.))
                .child(Self::setting_label(
                    "UI Density",
                    "Adjust the spacing and sizing of UI elements.",
                ))
                .child({
                    let options: [(&str, &str, Compactness); 3] = [
                        ("compact", "Compact", Compactness::Compact),
                        ("default", "Default", Compactness::Default),
                        ("comfortable", "Comfortable", Compactness::Comfortable),
                    ];
                    let mut row = div()
                        .h_flex()
                        .gap(px(4.))
                        .p(px(3.))
                        .rounded(px(8.))
                        .bg(colors.element_background);
                    for (id, label, variant) in options {
                        let is_selected = current_compactness == variant;
                        let hover_bg = colors.ghost_element_hover;
                        let mut btn = div()
                            .id(id)
                            .h_flex()
                            .h(px(28.))
                            .px(px(12.))
                            .items_center()
                            .justify_center()
                            .rounded(px(6.))
                            .cursor_pointer();
                        if is_selected {
                            btn = btn
                                .bg(colors.elevated_surface_background)
                                .border_1()
                                .border_color(colors.border)
                                .child(
                                    Label::new(label)
                                        .size(LabelSize::XSmall)
                                        .weight(FontWeight::SEMIBOLD),
                                );
                        } else {
                            btn = btn.hover(move |s| s.bg(hover_bg)).child(
                                Label::new(label)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            );
                        }
                        btn = btn.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.compactness = variant;
                            this.save_settings(cx);
                        }));
                        row = row.child(btn);
                    }
                    row
                }),
        );
        section = section.child(density_card);

        // Font size card
        let current_font_size = self.font_size;
        let mut font_size_card = Self::setting_card(cx);
        font_size_card = font_size_card.child(
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
                            Label::new("Font Size")
                                .size(LabelSize::Small)
                                .weight(FontWeight::SEMIBOLD),
                        )
                        .child(
                            Label::new("Base font size for the user interface (8 - 24).")
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
                            Button::new("font-size-dec", "-")
                                .style(ButtonStyle::Outlined)
                                .size(ButtonSize::Compact)
                                .disabled(current_font_size <= 8)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    if this.font_size > 8 {
                                        this.font_size -= 1;
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
                                    Label::new(SharedString::from(current_font_size.to_string()))
                                        .size(LabelSize::Small)
                                        .weight(FontWeight::SEMIBOLD),
                                ),
                        )
                        .child(
                            Button::new("font-size-inc", "+")
                                .style(ButtonStyle::Outlined)
                                .size(ButtonSize::Compact)
                                .disabled(current_font_size >= 24)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    if this.font_size < 24 {
                                        this.font_size += 1;
                                        this.save_settings(cx);
                                    }
                                })),
                        ),
                ),
        );
        section = section.child(font_size_card);
        section = section.child(Self::section_divider(cx));

        // Diff settings card
        let mut diff_card = Self::setting_card(cx);
        let show_line_numbers = self.show_line_numbers_in_diff;
        let wrap_lines = self.diff_wrap_lines;
        diff_card = diff_card
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
                                Label::new("Show Line Numbers in Diff")
                                    .size(LabelSize::Small)
                                    .weight(FontWeight::SEMIBOLD),
                            )
                            .child(
                                Label::new("Display line numbers alongside diff content.")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    )
                    .child(
                        div()
                            .id("show-line-numbers-toggle")
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.show_line_numbers_in_diff = !this.show_line_numbers_in_diff;
                                this.save_settings(cx);
                            }))
                            .child(Checkbox::new(
                                "show-line-numbers-cb",
                                if show_line_numbers {
                                    CheckState::Checked
                                } else {
                                    CheckState::Unchecked
                                },
                            )),
                    ),
            )
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
                                Label::new("Wrap Long Lines in Diff")
                                    .size(LabelSize::Small)
                                    .weight(FontWeight::SEMIBOLD),
                            )
                            .child(
                                Label::new(
                                    "Wrap overflowing lines instead of scrolling horizontally.",
                                )
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                            ),
                    )
                    .child(
                        div()
                            .id("diff-wrap-lines-toggle")
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.diff_wrap_lines = !this.diff_wrap_lines;
                                this.save_settings(cx);
                            }))
                            .child(Checkbox::new(
                                "diff-wrap-lines-cb",
                                if wrap_lines {
                                    CheckState::Checked
                                } else {
                                    CheckState::Unchecked
                                },
                            )),
                    ),
            )
            .child(
                div()
                    .v_flex()
                    .gap(px(8.))
                    .child(Self::setting_label(
                        "Default Diff View Mode",
                        "Choose how diffs are displayed.",
                    ))
                    .child(self.pill_group(
                        "diff-view-mode",
                        &["Unified", "Side-by-Side"],
                        &self.diff_view_mode.to_string(),
                        |this, value, cx| {
                            this.diff_view_mode = match value.as_str() {
                                "Unified" => DiffViewMode::Unified,
                                "Side-by-Side" => DiffViewMode::SideBySide,
                                _ => DiffViewMode::Unified,
                            };
                            this.save_settings(cx);
                        },
                        cx,
                    )),
            );
        section = section.child(diff_card);

        // Graph style card
        let mut graph_card = Self::setting_card(cx);
        graph_card = graph_card.child(
            div()
                .v_flex()
                .gap(px(8.))
                .child(Self::setting_label(
                    "Graph Style",
                    "Visual style for the commit graph rendering.",
                ))
                .child(self.pill_group(
                    "graph-style",
                    &["Rails", "Curved", "Angular"],
                    &self.graph_style.to_string(),
                    |this, value, cx| {
                        this.graph_style = match value.as_str() {
                            "Rails" => GraphStyle::Rails,
                            "Curved" => GraphStyle::Curved,
                            "Angular" => GraphStyle::Angular,
                            _ => GraphStyle::Rails,
                        };
                        this.save_settings(cx);
                    },
                    cx,
                ))
                .child({
                    let show_subject = self.show_subject_column;
                    div()
                        .h_flex()
                        .w_full()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .v_flex()
                                .flex_1()
                                .gap(px(2.))
                                .child(
                                    Label::new("Show Subject Column")
                                        .size(LabelSize::Small)
                                        .weight(FontWeight::SEMIBOLD),
                                )
                                .child(
                                    Label::new("Display commit subject in the graph.")
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                ),
                        )
                        .child(
                            div()
                                .id("show-subject-column-toggle")
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.show_subject_column = !this.show_subject_column;
                                    this.save_settings(cx);
                                }))
                                .child(Checkbox::new(
                                    "show-subject-column-cb",
                                    if show_subject {
                                        CheckState::Checked
                                    } else {
                                        CheckState::Unchecked
                                    },
                                )),
                        )
                }),
        );
        section = section.child(graph_card);

        // Auto-fetch interval card
        let mut fetch_card = Self::setting_card(cx);
        fetch_card = fetch_card.child(
            div()
                .v_flex()
                .gap(px(8.))
                .child(Self::setting_label(
                    "Auto-Fetch Interval",
                    "How often to automatically fetch from remotes in the background.",
                ))
                .child(self.pill_group(
                    "auto-fetch",
                    &["Disabled", "1 min", "5 min", "15 min", "30 min"],
                    &self.auto_fetch_interval.to_string(),
                    |this, value, cx| {
                        this.auto_fetch_interval = match value.as_str() {
                            "Disabled" => AutoFetchInterval::Disabled,
                            "1 min" => AutoFetchInterval::OneMinute,
                            "5 min" => AutoFetchInterval::FiveMinutes,
                            "15 min" => AutoFetchInterval::FifteenMinutes,
                            "30 min" => AutoFetchInterval::ThirtyMinutes,
                            _ => AutoFetchInterval::Disabled,
                        };
                        this.save_settings(cx);
                    },
                    cx,
                )),
        );
        section = section.child(fetch_card);

        // Confirm destructive operations card
        let confirm_destructive = self.confirm_destructive_operations;
        let mut confirm_card = Self::setting_card(cx);
        confirm_card = confirm_card.child(
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
                            Label::new("Confirm Before Destructive Operations")
                                .size(LabelSize::Small)
                                .weight(FontWeight::SEMIBOLD),
                        )
                        .child(
                            Label::new(
                                "Show a confirmation dialog before force push, branch delete, discard changes, and similar actions.",
                            )
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                        ),
                )
                .child(
                    div()
                        .id("confirm-destructive-toggle")
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.confirm_destructive_operations =
                                !this.confirm_destructive_operations;
                            this.save_settings(cx);
                        }))
                        .child(Checkbox::new(
                            "confirm-destructive-cb",
                            if confirm_destructive {
                                CheckState::Checked
                            } else {
                                CheckState::Unchecked
                            },
                        )),
                ),
        );
        section = section.child(confirm_card);

        // Auto-check updates card
        let auto_check_updates = self.auto_check_updates;
        let mut updates_card = Self::setting_card(cx);
        updates_card = updates_card.child(
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
                            Label::new("Check for Updates on Startup")
                                .size(LabelSize::Small)
                                .weight(FontWeight::SEMIBOLD),
                        )
                        .child(
                            Label::new(
                                "Contact api.github.com once per day to see if a newer release is available. Turn off to keep rgitui offline.",
                            )
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                        ),
                )
                .child(
                    div()
                        .id("auto-check-updates-toggle")
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.auto_check_updates = !this.auto_check_updates;
                            this.save_settings(cx);
                        }))
                        .child(Checkbox::new(
                            "auto-check-updates-cb",
                            if auto_check_updates {
                                CheckState::Checked
                            } else {
                                CheckState::Unchecked
                            },
                        )),
                ),
        );
        section = section.child(updates_card);

        // Watch all worktrees card
        let watch_all_worktrees = self.watch_all_worktrees;
        let mut watch_card = Self::setting_card(cx);
        watch_card = watch_card.child(
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
                            Label::new("Watch All Worktrees")
                                .size(LabelSize::Small)
                                .weight(FontWeight::SEMIBOLD),
                        )
                        .child(
                            Label::new(
                                "Refresh the graph whenever files change in any linked worktree, not just the current one. Useful when you have multiple worktrees open and work is happening in them in parallel.",
                            )
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                        ),
                )
                .child(
                    div()
                        .id("watch-all-worktrees-toggle")
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.watch_all_worktrees = !this.watch_all_worktrees;
                            this.save_settings(cx);
                        }))
                        .child(Checkbox::new(
                            "watch-all-worktrees-cb",
                            if watch_all_worktrees {
                                CheckState::Checked
                            } else {
                                CheckState::Unchecked
                            },
                        )),
                ),
        );
        section = section.child(watch_card);
        section = section.child(Self::section_divider(cx));

        // External tools card
        let mut tools_card = Self::setting_card(cx);
        let terminal_custom_text = self.terminal_command_editor.read(cx).text().to_string();
        let editor_custom_text = self.editor_command_editor.read(cx).text().to_string();
        let terminal_has_custom = !terminal_custom_text.is_empty()
            && !self
                .detected_terminals
                .iter()
                .any(|app| app.command == terminal_custom_text);
        let editor_has_custom = !editor_custom_text.is_empty()
            && !self
                .detected_editors
                .iter()
                .any(|app| app.command == editor_custom_text);

        let mut terminal_section = div().v_flex().gap(px(6.)).child(
            Label::new("Terminal")
                .size(LabelSize::XSmall)
                .weight(FontWeight::SEMIBOLD),
        );

        if self.detected_terminals.is_empty() {
            let msg = if self.detection_complete {
                "No terminals detected on this system."
            } else {
                "Detecting available terminals..."
            };
            terminal_section =
                terminal_section.child(Label::new(msg).size(LabelSize::XSmall).color(Color::Muted));
        } else {
            let mut list = div()
                .v_flex()
                .gap(px(2.))
                .p(px(4.))
                .rounded(px(6.))
                .bg(colors.element_background);

            for (i, app) in self.detected_terminals.iter().enumerate() {
                let is_selected = self.selected_terminal_index == Some(i) && !terminal_has_custom;
                let display = SharedString::from(format!("{} ({})", app.display_name, app.command));
                let hover_bg = colors.ghost_element_hover;
                let selected_bg = colors.elevated_surface_background;
                let border_color = colors.border;
                let accent = colors.text_accent;

                let mut row = div()
                    .id(ElementId::Name(SharedString::from(format!(
                        "terminal-{}",
                        i
                    ))))
                    .h_flex()
                    .gap(px(8.))
                    .px(px(8.))
                    .py(px(4.))
                    .items_center()
                    .rounded(px(4.))
                    .cursor_pointer();

                if is_selected {
                    row = row.bg(selected_bg).border_1().border_color(border_color);
                } else {
                    row = row.hover(move |s| s.bg(hover_bg));
                }

                let dot = div().w(px(8.)).h(px(8.)).rounded(px(4.)).flex_shrink_0();
                let dot = if is_selected {
                    dot.bg(accent)
                } else {
                    dot.border_1().border_color(border_color)
                };

                let label_color = if is_selected {
                    Color::Default
                } else {
                    Color::Muted
                };

                row = row.child(dot).child(
                    Label::new(display)
                        .size(LabelSize::XSmall)
                        .color(label_color),
                );

                row = row.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.selected_terminal_index = Some(i);
                    if let Some(app) = this.detected_terminals.get(i) {
                        this.terminal_command_editor
                            .update(cx, |e, cx| e.set_text(&app.command, cx));
                    }
                    this.save_settings(cx);
                }));

                list = list.child(row);
            }
            terminal_section = terminal_section.child(list);
        }

        terminal_section = terminal_section.child(
            div()
                .v_flex()
                .gap(px(2.))
                .child(
                    Label::new("Custom command (overrides selection)")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .child(self.terminal_command_editor.clone()),
        );

        let mut editor_section = div().v_flex().gap(px(6.)).child(
            Label::new("Editor")
                .size(LabelSize::XSmall)
                .weight(FontWeight::SEMIBOLD),
        );

        if self.detected_editors.is_empty() {
            let msg = if self.detection_complete {
                "No editors detected on this system."
            } else {
                "Detecting available editors..."
            };
            editor_section =
                editor_section.child(Label::new(msg).size(LabelSize::XSmall).color(Color::Muted));
        } else {
            let mut list = div()
                .v_flex()
                .gap(px(2.))
                .p(px(4.))
                .rounded(px(6.))
                .bg(colors.element_background);

            for (i, app) in self.detected_editors.iter().enumerate() {
                let is_selected = self.selected_editor_index == Some(i) && !editor_has_custom;
                let display = SharedString::from(format!("{} ({})", app.display_name, app.command));
                let hover_bg = colors.ghost_element_hover;
                let selected_bg = colors.elevated_surface_background;
                let border_color = colors.border;
                let accent = colors.text_accent;

                let mut row = div()
                    .id(ElementId::Name(SharedString::from(format!("editor-{}", i))))
                    .h_flex()
                    .gap(px(8.))
                    .px(px(8.))
                    .py(px(4.))
                    .items_center()
                    .rounded(px(4.))
                    .cursor_pointer();

                if is_selected {
                    row = row.bg(selected_bg).border_1().border_color(border_color);
                } else {
                    row = row.hover(move |s| s.bg(hover_bg));
                }

                let dot = div().w(px(8.)).h(px(8.)).rounded(px(4.)).flex_shrink_0();
                let dot = if is_selected {
                    dot.bg(accent)
                } else {
                    dot.border_1().border_color(border_color)
                };

                let label_color = if is_selected {
                    Color::Default
                } else {
                    Color::Muted
                };

                row = row.child(dot).child(
                    Label::new(display)
                        .size(LabelSize::XSmall)
                        .color(label_color),
                );

                row = row.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.selected_editor_index = Some(i);
                    if let Some(app) = this.detected_editors.get(i) {
                        this.editor_command_editor
                            .update(cx, |e, cx| e.set_text(&app.command, cx));
                    }
                    this.save_settings(cx);
                }));

                list = list.child(row);
            }
            editor_section = editor_section.child(list);
        }

        editor_section = editor_section.child(
            div()
                .v_flex()
                .gap(px(2.))
                .child(
                    Label::new("Custom command (overrides selection)")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .child(self.editor_command_editor.clone()),
        );

        tools_card = tools_card.child(
            div()
                .v_flex()
                .gap(px(16.))
                .child(Self::setting_label(
                    "External Tools",
                    "Select a detected application or enter a custom command.",
                ))
                .child(terminal_section)
                .child(editor_section),
        );
        section = section.child(tools_card);
        section = section.child(Self::section_divider(cx));

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
                .child(self.render_shortcut_row("Command Palette", "Ctrl+Shift+P", cx))
                .child(self.render_shortcut_row("Search Commits", "Ctrl+F", cx))
                .child(self.render_shortcut_row("Stage All", "Ctrl+S", cx))
                .child(self.render_shortcut_row("Unstage All", "Ctrl+Shift+S", cx))
                .child(self.render_shortcut_row("Commit", "Ctrl+Enter (in message)", cx))
                .child(self.render_shortcut_row("Open Repository", "Ctrl+O", cx))
                .child(self.render_shortcut_row("Refresh", "F5", cx))
                .child(self.render_shortcut_row("Settings", "Ctrl+,", cx)),
        );
        section = section.child(shortcuts_card);
        section = section.child(Self::section_divider(cx));

        let config_path = config_dir().join("settings.json");
        let config_path_display = config_path.display().to_string();
        let config_dir_path = config_dir();
        let mut config_card = Self::setting_card(cx);
        config_card = config_card.child(
            div()
                .v_flex()
                .gap(px(8.))
                .child(Self::setting_label(
                    "Config File",
                    "Location of the settings file on disk.",
                ))
                .child(
                    div()
                        .h_flex()
                        .w_full()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div().flex_1().min_w_0().child(
                                Label::new(SharedString::from(config_path_display))
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                        )
                        .child(
                            Button::new("reveal-config-dir", "Reveal")
                                .style(ButtonStyle::Subtle)
                                .size(ButtonSize::Compact)
                                .icon(IconName::Folder)
                                .on_click(move |_: &ClickEvent, _, _cx| {
                                    #[cfg(target_os = "windows")]
                                    {
                                        let _ = std::process::Command::new("explorer")
                                            .arg(&config_dir_path)
                                            .spawn();
                                    }
                                    #[cfg(target_os = "macos")]
                                    {
                                        let _ = std::process::Command::new("open")
                                            .arg(&config_dir_path)
                                            .spawn();
                                    }
                                    #[cfg(target_os = "linux")]
                                    {
                                        let _ = std::process::Command::new("xdg-open")
                                            .arg(&config_dir_path)
                                            .spawn();
                                    }
                                }),
                        ),
                ),
        );
        section = section.child(config_card);

        section
    }

    fn render_shortcut_row(
        &self,
        action: &str,
        shortcut: &str,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = cx.colors();
        div()
            .h_flex()
            .w_full()
            .items_center()
            .py(px(4.))
            .child(
                Label::new(SharedString::from(action.to_string()))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .child(div().flex_1())
            .child(
                div()
                    .h_flex()
                    .h(px(22.))
                    .px(px(8.))
                    .rounded(px(4.))
                    .bg(colors.hint_background)
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

impl SettingsView {
    /// Build the inner sidebar+content layout for the standalone
    /// `SettingsWindow` host. The host provides its own close affordance.
    pub fn render_window_body(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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

        let header = div()
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
            );

        div()
            .h_flex()
            .size_full()
            .child(sidebar)
            .child(
                div()
                    .id("settings-page-shell")
                    .v_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .child(header)
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
            )
            .into_any_element()
    }
}

