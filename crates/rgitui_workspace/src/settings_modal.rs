use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, FontWeight,
    KeyDownEvent, Render, SharedString, Window,
};
use rgitui_settings::{AiSettings, GitSettings, SettingsState};
use rgitui_theme::{ActiveTheme, Color, StyledExt, ThemeState};
use rgitui_ui::{
    Button, ButtonSize, ButtonStyle, Checkbox, CheckState, Icon, IconName, IconSize, Label,
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
    Git,
    General,
}

/// The settings modal.
pub struct SettingsModal {
    visible: bool,
    focus_handle: FocusHandle,
    active_section: SettingsSection,

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

    // General state
    max_recent_repos: usize,
}

impl EventEmitter<SettingsModalEvent> for SettingsModal {}

impl SettingsModal {
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
            selected_theme: settings.theme.clone(),
            available_themes,
            ai_provider: settings.ai.provider.clone(),
            ai_api_key: settings.ai.api_key.clone().unwrap_or_default(),
            ai_model: settings.ai.model.clone(),
            ai_commit_style: settings.ai.commit_style.clone(),
            ai_enabled: settings.ai.enabled,
            git_https_token: settings.git.https_token.clone().unwrap_or_default(),
            git_ssh_key_path: settings.git.ssh_key_path.clone().unwrap_or_default(),
            git_gpg_key_id: settings.git.gpg_key_id.clone().unwrap_or_default(),
            git_sign_commits: settings.git.sign_commits,
            max_recent_repos: settings.max_recent_repos,
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
            self.ai_api_key = s.ai.api_key.clone().unwrap_or_default();
            self.ai_model = s.ai.model.clone();
            self.ai_commit_style = s.ai.commit_style.clone();
            self.ai_enabled = s.ai.enabled;
            self.git_https_token = s.git.https_token.clone().unwrap_or_default();
            self.git_ssh_key_path = s.git.ssh_key_path.clone().unwrap_or_default();
            self.git_gpg_key_id = s.git.gpg_key_id.clone().unwrap_or_default();
            self.git_sign_commits = s.git.sign_commits;
            self.max_recent_repos = s.max_recent_repos;
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
        cx.update_global::<SettingsState, _>(|state, _cx| {
            let s = state.settings_mut();
            s.ai = AiSettings {
                provider: self.ai_provider.clone(),
                api_key: if self.ai_api_key.is_empty() {
                    None
                } else {
                    Some(self.ai_api_key.clone())
                },
                model: self.ai_model.clone(),
                commit_style: self.ai_commit_style.clone(),
                enabled: self.ai_enabled,
            };
            s.git = GitSettings {
                https_token: if self.git_https_token.is_empty() {
                    None
                } else {
                    Some(self.git_https_token.clone())
                },
                ssh_key_path: if self.git_ssh_key_path.is_empty() {
                    None
                } else {
                    Some(self.git_ssh_key_path.clone())
                },
                gpg_key_id: if self.git_gpg_key_id.is_empty() {
                    None
                } else {
                    Some(self.git_gpg_key_id.clone())
                },
                sign_commits: self.git_sign_commits,
            };
            s.max_recent_repos = self.max_recent_repos;
            if let Err(e) = state.save() {
                log::error!("Failed to save settings: {}", e);
            }
        });
        cx.emit(SettingsModalEvent::SettingsChanged);
        cx.notify();
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => self.dismiss(cx),
            "backspace" => {
                if self.active_section == SettingsSection::Ai && !self.ai_api_key.is_empty() {
                    self.ai_api_key.pop();
                    self.save_settings(cx);
                }
            }
            _ => {
                if self.active_section == SettingsSection::Ai {
                    if let Some(key_char) = &event.keystroke.key_char {
                        if !event.keystroke.modifiers.control
                            && !event.keystroke.modifiers.platform
                        {
                            self.ai_api_key.push_str(key_char);
                            self.save_settings(cx);
                        }
                    }
                }
            }
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
        let colors = cx.colors();
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

    // ── Sidebar navigation ──────────────────────────────────────────────
    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        let sections: [(SettingsSection, IconName, &str); 4] = [
            (SettingsSection::Theme, IconName::Eye, "Appearance"),
            (SettingsSection::Ai, IconName::Sparkle, "AI"),
            (SettingsSection::Git, IconName::GitBranch, "Git"),
            (SettingsSection::General, IconName::Settings, "General"),
        ];

        let mut sidebar = div()
            .v_flex()
            .w(px(180.))
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
                    Label::new("Settings")
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
                    .when(!is_active, |el| el.hover(|s| s.bg(colors.ghost_element_hover)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.active_section = section;
                        cx.notify();
                    }))
                    .child(
                        Icon::new(icon).size(IconSize::Small).color(if is_active {
                            Color::Accent
                        } else {
                            Color::Muted
                        }),
                    )
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
        let editor_bg = cx.colors().editor_background;
        let border_color = cx.colors().border;

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
                        "gemini" => "gemini-2.0-flash".into(),
                        "openai" => "gpt-4o".into(),
                        "anthropic" => "claude-sonnet-4-6".into(),
                        _ => "gemini-2.0-flash".into(),
                    };
                    this.save_settings(cx);
                },
                cx,
            ));

        // Model selector
        let models: Vec<&str> = match self.ai_provider.as_str() {
            "gemini" => vec!["gemini-2.0-flash", "gemini-2.5-pro"],
            "openai" => vec!["gpt-4o", "gpt-4o-mini"],
            "anthropic" => vec!["claude-sonnet-4-6", "claude-haiku-4-5-20251001"],
            _ => vec!["gemini-2.0-flash"],
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
        let api_key_display: SharedString = if self.ai_api_key.is_empty() {
            "Click here and type your API key...".into()
        } else {
            let masked = if self.ai_api_key.len() > 4 {
                let visible = &self.ai_api_key[self.ai_api_key.len() - 4..];
                format!("{}{}", "*".repeat(self.ai_api_key.len() - 4), visible)
            } else {
                "*".repeat(self.ai_api_key.len())
            };
            masked.into()
        };
        let api_key_color = if self.ai_api_key.is_empty() {
            Color::Placeholder
        } else {
            Color::Muted
        };

        let mut key_card = Self::setting_card(cx);
        key_card = key_card
            .child(Self::setting_label(
                "API Key",
                "Your key is stored locally in ~/.config/rgitui/settings.json",
            ))
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .h(px(36.))
                    .px(px(12.))
                    .items_center()
                    .bg(editor_bg)
                    .border_1()
                    .border_color(border_color)
                    .rounded(px(6.))
                    .child(
                        Icon::new(IconName::Eye)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        div().flex_1().pl(px(8.)).child(
                            Label::new(api_key_display)
                                .size(LabelSize::Small)
                                .color(api_key_color),
                        ),
                    ),
            );
        section = section.child(key_card);

        section
    }

    // ── Git section ──────────────────────────────────────────────────────
    fn render_git_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        let editor_bg = colors.editor_background;
        let border_color = colors.border;

        let mut section = div().v_flex().w_full().gap(px(16.));

        section = section.child(Self::section_header(
            IconName::GitBranch,
            "Git Authentication",
            "Configure credentials for remote operations and commit signing.",
        ));

        // HTTPS Token card
        let https_display: SharedString = if self.git_https_token.is_empty() {
            "No token configured".into()
        } else {
            let len = self.git_https_token.len();
            if len > 4 {
                let visible = &self.git_https_token[len - 4..];
                format!("{}{}", "*".repeat(len - 4), visible).into()
            } else {
                "*".repeat(len).into()
            }
        };
        let https_color = if self.git_https_token.is_empty() {
            Color::Placeholder
        } else {
            Color::Muted
        };

        let mut https_card = Self::setting_card(cx);
        https_card = https_card
            .child(Self::setting_label(
                "HTTPS Token / PAT",
                "Personal access token for GitHub, GitLab, etc. Used for HTTPS remotes.",
            ))
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .h(px(36.))
                    .px(px(12.))
                    .items_center()
                    .bg(editor_bg)
                    .border_1()
                    .border_color(border_color)
                    .rounded(px(6.))
                    .child(
                        Icon::new(IconName::Eye)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        div().flex_1().pl(px(8.)).child(
                            Label::new(https_display)
                                .size(LabelSize::Small)
                                .color(https_color),
                        ),
                    )
                    .when(!self.git_https_token.is_empty(), |el| {
                        el.child(
                            Button::new("clear-https-token", "Clear")
                                .size(ButtonSize::Compact)
                                .style(ButtonStyle::Outlined)
                                .color(Color::Muted)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.git_https_token.clear();
                                    this.save_settings(cx);
                                })),
                        )
                    }),
            );
        section = section.child(https_card);

        // SSH Key Path card
        let ssh_display: SharedString = if self.git_ssh_key_path.is_empty() {
            "Default (~/.ssh/id_ed25519, id_rsa)".into()
        } else {
            self.git_ssh_key_path.clone().into()
        };
        let ssh_color = if self.git_ssh_key_path.is_empty() {
            Color::Placeholder
        } else {
            Color::Default
        };

        let mut ssh_card = Self::setting_card(cx);
        ssh_card = ssh_card
            .child(Self::setting_label(
                "SSH Key Path",
                "Path to your SSH private key. Leave empty to use ssh-agent or default keys.",
            ))
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .h(px(36.))
                    .px(px(12.))
                    .items_center()
                    .bg(editor_bg)
                    .border_1()
                    .border_color(border_color)
                    .rounded(px(6.))
                    .child(
                        Icon::new(IconName::File)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        div().flex_1().pl(px(8.)).child(
                            Label::new(ssh_display)
                                .size(LabelSize::Small)
                                .color(ssh_color),
                        ),
                    )
                    .when(!self.git_ssh_key_path.is_empty(), |el| {
                        el.child(
                            Button::new("clear-ssh-path", "Clear")
                                .size(ButtonSize::Compact)
                                .style(ButtonStyle::Outlined)
                                .color(Color::Muted)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.git_ssh_key_path.clear();
                                    this.save_settings(cx);
                                })),
                        )
                    }),
            );
        section = section.child(ssh_card);

        // GPG Signing card
        let sign_commits = self.git_sign_commits;
        let gpg_display: SharedString = if self.git_gpg_key_id.is_empty() {
            "No GPG key configured".into()
        } else {
            self.git_gpg_key_id.clone().into()
        };
        let gpg_color = if self.git_gpg_key_id.is_empty() {
            Color::Placeholder
        } else {
            Color::Default
        };

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
                                Label::new("Sign commits with GPG key")
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
                "The key ID to use for signing. Leave empty to use git config default.",
            ))
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .h(px(36.))
                    .px(px(12.))
                    .items_center()
                    .bg(editor_bg)
                    .border_1()
                    .border_color(border_color)
                    .rounded(px(6.))
                    .child(
                        Icon::new(IconName::Eye)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        div().flex_1().pl(px(8.)).child(
                            Label::new(gpg_display)
                                .size(LabelSize::Small)
                                .color(gpg_color),
                        ),
                    )
                    .when(!self.git_gpg_key_id.is_empty(), |el| {
                        el.child(
                            Button::new("clear-gpg-key", "Clear")
                                .size(ButtonSize::Compact)
                                .style(ButtonStyle::Outlined)
                                .color(Color::Muted)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.git_gpg_key_id.clear();
                                    this.save_settings(cx);
                                })),
                        )
                    }),
            );
        section = section.child(gpg_card);

        // Info card about auth strategy
        let mut info_card = Self::setting_card(cx);
        info_card = info_card.child(
            div()
                .v_flex()
                .gap(px(8.))
                .child(Self::setting_label(
                    "Authentication Order",
                    "How rgitui attempts to authenticate with remotes:",
                ))
                .child(
                    div()
                        .v_flex()
                        .gap(px(4.))
                        .child(
                            Label::new("1. SSH agent (if available)")
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("2. Custom SSH key (if configured above)")
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("3. Default SSH keys (~/.ssh/id_ed25519, id_rsa)")
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("4. HTTPS token (if configured above)")
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("5. Git credential helper")
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new("6. Anonymous access (public repos)")
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                ),
        );
        section = section.child(info_card);

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
                .child(self.render_shortcut_row("Commit", "Ctrl+Enter"))
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

        let content = match self.active_section {
            SettingsSection::Theme => self.render_theme_section(cx).into_any_element(),
            SettingsSection::Ai => self.render_ai_section(cx).into_any_element(),
            SettingsSection::Git => self.render_git_section(cx).into_any_element(),
            SettingsSection::General => self.render_general_section(cx).into_any_element(),
        };

        let sidebar = self.render_sidebar(cx);
        let colors = cx.colors();

        let backdrop = div()
            .id("settings-modal-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
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
            .w(px(640.))
            .h(px(500.))
            .bg(colors.elevated_surface_background)
            .border_1()
            .border_color(colors.border)
            .rounded(px(12.))
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
                    .id("settings-content")
                    .v_flex()
                    .flex_1()
                    .h_full()
                    .p(px(20.))
                    .overflow_y_scroll()
                    .child(content),
            );

        backdrop.child(modal).into_any_element()
    }
}
