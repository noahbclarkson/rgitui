use anyhow::Result;
use chrono::{DateTime, Utc};
use gpui::{App, Global};
use keyring::Entry;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, sync_channel, Sender, SyncSender};
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

/// Controls whether light or dark themes are shown in the theme picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AppearanceMode {
    #[default]
    Auto,
    Light,
    Dark,
}

impl fmt::Display for AppearanceMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppearanceMode::Auto => write!(f, "Auto"),
            AppearanceMode::Light => write!(f, "Light"),
            AppearanceMode::Dark => write!(f, "Dark"),
        }
    }
}

impl FromStr for AppearanceMode {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "auto" => Ok(AppearanceMode::Auto),
            "light" => Ok(AppearanceMode::Light),
            "dark" => Ok(AppearanceMode::Dark),
            _ => Err(format!("Unknown appearance mode: {}", s)),
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

impl AutoFetchInterval {
    /// Every interval, in the order the settings UI lists them.
    pub const ALL: &'static [AutoFetchInterval] = &[
        AutoFetchInterval::Disabled,
        AutoFetchInterval::OneMinute,
        AutoFetchInterval::FiveMinutes,
        AutoFetchInterval::FifteenMinutes,
        AutoFetchInterval::ThirtyMinutes,
    ];
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

/// Interface language for localizable UI text.
///
/// The stable on-disk id is [`Language::id`] (`"en"` / `"zh-CN"`); the
/// dropdown always shows [`Language::native_name`] so each option is
/// recognizable regardless of the active language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    English,
    SimplifiedChinese,
}

impl Language {
    /// Every language, in the order the settings UI lists them.
    pub const ALL: &'static [Language] = &[Language::English, Language::SimplifiedChinese];

    /// Stable identifier used on disk and as the settings dropdown id.
    pub fn id(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::SimplifiedChinese => "zh-CN",
        }
    }

    /// Native display name. Deliberately not translated: "English" must read
    /// "English" even while the UI is Chinese, and vice versa.
    pub fn native_name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::SimplifiedChinese => "简体中文",
        }
    }

    /// Localize `key`. Missing Chinese entries fall back to English by
    /// construction: every arm pairs `(self, key)` explicitly.
    pub fn tr(self, key: TrKey) -> &'static str {
        match (self, key) {
            (_, TrKey::Preferences) => match self {
                Language::English => "Preferences",
                Language::SimplifiedChinese => "偏好设置",
            },
            (_, TrKey::SectionAppearance) => match self {
                Language::English => "Appearance",
                Language::SimplifiedChinese => "外观",
            },
            (_, TrKey::SectionAi) => "AI",
            (_, TrKey::SectionAuth) => match self {
                Language::English => "Auth",
                Language::SimplifiedChinese => "认证",
            },
            (_, TrKey::SectionGeneral) => match self {
                Language::English => "General",
                Language::SimplifiedChinese => "通用",
            },
            (_, TrKey::GeneralTitle) => match self {
                Language::English => "General",
                Language::SimplifiedChinese => "通用",
            },
            (_, TrKey::GeneralDesc) => match self {
                Language::English => "Application preferences and behavior.",
                Language::SimplifiedChinese => "应用偏好与行为。",
            },
            (_, TrKey::LanguageTitle) => match self {
                Language::English => "Language",
                Language::SimplifiedChinese => "语言",
            },
            (_, TrKey::LanguageDesc) => match self {
                Language::English => "Switch the interface language.",
                Language::SimplifiedChinese => "切换界面语言。",
            },
            (_, TrKey::MaxRecentTitle) => match self {
                Language::English => "Max Recent Repositories",
                Language::SimplifiedChinese => "最大最近仓库数",
            },
            (_, TrKey::MaxRecentDesc) => match self {
                Language::English => "Number of repos shown in the recent list.",
                Language::SimplifiedChinese => "最近列表中显示的仓库数量。",
            },
            (_, TrKey::UiDensityTitle) => match self {
                Language::English => "UI Density",
                Language::SimplifiedChinese => "界面密度",
            },
            (_, TrKey::UiDensityDesc) => match self {
                Language::English => "Adjust the spacing and sizing of UI elements.",
                Language::SimplifiedChinese => "调整界面元素的间距与尺寸。",
            },
            (_, TrKey::DensityCompact) => match self {
                Language::English => "Compact",
                Language::SimplifiedChinese => "紧凑",
            },
            (_, TrKey::DensityDefault) => match self {
                Language::English => "Default",
                Language::SimplifiedChinese => "默认",
            },
            (_, TrKey::DensityComfortable) => match self {
                Language::English => "Comfortable",
                Language::SimplifiedChinese => "舒适",
            },
            (_, TrKey::FontSizeTitle) => match self {
                Language::English => "Font Size",
                Language::SimplifiedChinese => "字号",
            },
            (_, TrKey::FontSizeDesc) => match self {
                Language::English => "Base font size for the user interface (8 - 24).",
                Language::SimplifiedChinese => "界面基础字号（8 - 24）。",
            },
            (_, TrKey::GeneralPageDesc) => match self {
                Language::English => "General application behavior and workspace defaults.",
                Language::SimplifiedChinese => "通用应用行为与工作区默认设置。",
            },
            (_, TrKey::ThemeSectionDesc) => match self {
                Language::English => "Customize the look and feel of the application.",
                Language::SimplifiedChinese => "自定义应用的外观与体验。",
            },
            (_, TrKey::ThemePageDesc) => match self {
                Language::English => "Theme, layout, and visual defaults for the application.",
                Language::SimplifiedChinese => "应用的主题、布局与视觉默认设置。",
            },
            (_, TrKey::AppearanceModeTitle) => match self {
                Language::English => "Appearance Mode",
                Language::SimplifiedChinese => "外观模式",
            },
            (_, TrKey::AppearanceModeDesc) => match self {
                Language::English => "Choose whether to show light or dark themes, or auto-detect.",
                Language::SimplifiedChinese => "选择显示浅色或深色主题，或跟随系统自动切换。",
            },
            (_, TrKey::ModeAuto) => match self {
                Language::English => "Auto",
                Language::SimplifiedChinese => "自动",
            },
            (_, TrKey::ModeLight) => match self {
                Language::English => "Light",
                Language::SimplifiedChinese => "浅色",
            },
            (_, TrKey::ModeDark) => match self {
                Language::English => "Dark",
                Language::SimplifiedChinese => "深色",
            },
            (_, TrKey::ColorThemeTitle) => match self {
                Language::English => "Color Theme",
                Language::SimplifiedChinese => "配色主题",
            },
            (_, TrKey::ColorThemeDesc) => match self {
                Language::English => "Select a theme to change all interface colors.",
                Language::SimplifiedChinese => "选择主题以更改全部界面颜色。",
            },
            (_, TrKey::ActiveBadge) => match self {
                Language::English => "Active",
                Language::SimplifiedChinese => "使用中",
            },
            (_, TrKey::CustomThemeTitle) => match self {
                Language::English => "Custom Theme Editor",
                Language::SimplifiedChinese => "自定义主题编辑器",
            },
            (_, TrKey::CustomThemeDesc) => match self {
                Language::English => "Edit colors, create custom themes, and export as JSON.",
                Language::SimplifiedChinese => "编辑颜色、创建自定义主题并导出为 JSON。",
            },
            (_, TrKey::EditThemeBtn) => match self {
                Language::English => "Edit Theme",
                Language::SimplifiedChinese => "编辑主题",
            },
            (_, TrKey::AccountsTitle) => match self {
                Language::English => "Accounts",
                Language::SimplifiedChinese => "账户",
            },
            (_, TrKey::AccountsDesc) => match self {
                Language::English => "Account profiles, HTTPS tokens, and SSH configuration.",
                Language::SimplifiedChinese => "账户档案、HTTPS 令牌与 SSH 配置。",
            },
            (_, TrKey::AccountsHeader) => match self {
                Language::English => "Accounts & Credentials",
                Language::SimplifiedChinese => "账户与凭证",
            },
            (_, TrKey::AccountsSectionDesc) => match self {
                Language::English => "Use browser sign-in for GitHub or GitLab, manage all account profiles in one place, and keep SSH separate from HTTPS tokens.",
                Language::SimplifiedChinese => "使用浏览器登录 GitHub 或 GitLab，在一处管理所有账户档案，并将 SSH 与 HTTPS 令牌分开管理。",
            },
            (_, TrKey::QuickSetupTitle) => match self {
                Language::English => "Quick Setup",
                Language::SimplifiedChinese => "快速设置",
            },
            (_, TrKey::QuickSetupDesc) => match self {
                Language::English => "Start by adding the provider account you want to use. Browser sign-in is the primary path. Manual setup is available for self-hosted or custom HTTPS remotes.",
                Language::SimplifiedChinese => "先添加要使用的服务商账户。推荐使用浏览器登录；自建或自定义 HTTPS 远程可使用手动设置。",
            },
            (_, TrKey::SignInGitHub) => match self {
                Language::English => "Sign in with GitHub",
                Language::SimplifiedChinese => "使用 GitHub 登录",
            },
            (_, TrKey::AddGitLab) => match self {
                Language::English => "Add GitLab Account",
                Language::SimplifiedChinese => "添加 GitLab 账户",
            },
            (_, TrKey::ManualCustom) => match self {
                Language::English => "Manual / Custom Host",
                Language::SimplifiedChinese => "手动 / 自定义主机",
            },
            (_, TrKey::AiPageDesc) => match self {
                Language::English => "Providers, models, and how commit messages get written.",
                Language::SimplifiedChinese => "服务商、模型，以及提交信息的生成方式。",
            },
            (_, TrKey::AiSectionConnection) => match self {
                Language::English => "CONNECTION",
                Language::SimplifiedChinese => "连接",
            },
            (_, TrKey::AiSectionBehaviour) => match self {
                Language::English => "BEHAVIOUR",
                Language::SimplifiedChinese => "行为",
            },
            (_, TrKey::AiStatusTitle) => match self {
                Language::English => "AI Commit Messages",
                Language::SimplifiedChinese => "AI 提交信息",
            },
            (_, TrKey::AiEnabledToggle) => match self {
                Language::English => "Enabled",
                Language::SimplifiedChinese => "启用",
            },
            (_, TrKey::AiStatusOff) => match self {
                Language::English => "AI is turned off. Nothing will be sent to any provider.",
                Language::SimplifiedChinese => "AI 已关闭，不会向任何服务商发送内容。",
            },
            (_, TrKey::AiStatusTestingFmt) => match self {
                Language::English => "Testing {}…",
                Language::SimplifiedChinese => "正在测试 {}…",
            },
            (_, TrKey::AiStatusRejectedFmt) => match self {
                Language::English => "{} rejected this key.",
                Language::SimplifiedChinese => "{} 拒绝了此密钥。",
            },
            (_, TrKey::AiStatusAddKeyFmt) => match self {
                Language::English => "Add a {} API key to get started.",
                Language::SimplifiedChinese => "添加 {} API 密钥以开始使用。",
            },
            (_, TrKey::AiConnectTitle) => match self {
                Language::English => "Connect an AI provider",
                Language::SimplifiedChinese => "连接 AI 服务商",
            },
            (_, TrKey::AiConnectDesc) => match self {
                Language::English => "rgitui writes commit messages from your staged diff.",
                Language::SimplifiedChinese => "rgitui 会根据已暂存的 diff 编写提交信息。",
            },
            (_, TrKey::AiApiKeyLabel) => match self {
                Language::English => "API key",
                Language::SimplifiedChinese => "API 密钥",
            },
            (_, TrKey::AiGetKeyBtn) => match self {
                Language::English => "Get a key",
                Language::SimplifiedChinese => "获取密钥",
            },
            (_, TrKey::AiKeyVerifiedFmt) => match self {
                Language::English => "Verified {} · stored in {}",
                Language::SimplifiedChinese => "已验证（{}）·存储于 {}",
            },
            (_, TrKey::AiKeyStoredHintFmt) => match self {
                Language::English => "Key stored in {}. Test it to confirm it works.",
                Language::SimplifiedChinese => "密钥存储于 {}，点击测试确认其可用。",
            },
            (_, TrKey::AiNoKeyHintFmt) => match self {
                Language::English => "No API key needed — requests go to {}.",
                Language::SimplifiedChinese => "无需 API 密钥——请求将发送至 {}。",
            },
            (_, TrKey::AiKeysStoredNoteFmt) => match self {
                Language::English => {
                    "Keys are stored in {}, never in settings.json, and are only read when a request is sent."
                }
                Language::SimplifiedChinese => {
                    "密钥存储于 {}，从不写入 settings.json，仅在发送请求时读取。"
                }
            },
            (_, TrKey::AiModelLabel) => match self {
                Language::English => "Model",
                Language::SimplifiedChinese => "模型",
            },
            (_, TrKey::AiModelCountFmt) => match self {
                Language::English => "{} models · {}",
                Language::SimplifiedChinese => "{} 个模型 · {}",
            },
            (_, TrKey::AiRefreshBtn) => match self {
                Language::English => "Refresh",
                Language::SimplifiedChinese => "刷新",
            },
            (_, TrKey::AiRefreshingBtn) => match self {
                Language::English => "Refreshing…",
                Language::SimplifiedChinese => "刷新中…",
            },
            (_, TrKey::AiModelMissingFmt) => match self {
                Language::English => {
                    "`{}` is not in {}'s current model list. It may have been retired."
                }
                Language::SimplifiedChinese => "`{}` 不在 {} 当前的模型列表中，可能已被下线。",
            },
            (_, TrKey::AiUseSuggestionFmt) => match self {
                Language::English => "Use {}",
                Language::SimplifiedChinese => "使用 {}",
            },
            (_, TrKey::AiConnectBtn) => match self {
                Language::English => "Connect",
                Language::SimplifiedChinese => "连接",
            },
            (_, TrKey::AiUseProviderBtn) => match self {
                Language::English => "Use this provider",
                Language::SimplifiedChinese => "使用此服务商",
            },
            (_, TrKey::AiTestBtn) => match self {
                Language::English => "Test",
                Language::SimplifiedChinese => "测试",
            },
            (_, TrKey::AiTestingBtn) => match self {
                Language::English => "Testing…",
                Language::SimplifiedChinese => "测试中…",
            },
            (_, TrKey::AiRemoveKeyBtn) => match self {
                Language::English => "Remove key",
                Language::SimplifiedChinese => "移除密钥",
            },
            (_, TrKey::AiAdvancedLabel) => match self {
                Language::English => "Advanced",
                Language::SimplifiedChinese => "高级",
            },
            (_, TrKey::AiBaseUrlLabel) => "Base URL",
            (_, TrKey::AiBaseUrlOverriddenFmt) => match self {
                Language::English => {
                    "Requests go to {} instead of {}. Your API key is sent to that host."
                }
                Language::SimplifiedChinese => {
                    "请求将发送至 {} 而非 {}，您的 API 密钥也会被发送到该主机。"
                }
            },
            (_, TrKey::AiBaseUrlEmptyFmt) => match self {
                Language::English => {
                    "Empty means use {}. Only OpenAI-compatible providers honour an override."
                }
                Language::SimplifiedChinese => {
                    "留空表示使用 {}，仅 OpenAI 兼容服务商支持覆盖此项。"
                }
            },
            (_, TrKey::AiAttributionTitle) => match self {
                Language::English => "Send attribution headers",
                Language::SimplifiedChinese => "发送归因请求头",
            },
            (_, TrKey::AiAttributionDesc) => match self {
                Language::English => {
                    "Adds HTTP-Referer and X-Title so rgitui appears on OpenRouter's public leaderboard. Never functional."
                }
                Language::SimplifiedChinese => {
                    "附加 HTTP-Referer 与 X-Title 请求头，使 rgitui 出现在 OpenRouter 公开排行榜上，不影响任何功能。"
                }
            },
            (_, TrKey::AiCommitStyleTitle) => match self {
                Language::English => "Commit style",
                Language::SimplifiedChinese => "提交信息风格",
            },
            (_, TrKey::AiCommitStyleDesc) => match self {
                Language::English => "How the AI should format commit messages.",
                Language::SimplifiedChinese => "AI 组织提交信息内容的方式。",
            },
            (_, TrKey::AiCtxTitle) => match self {
                Language::English => "Include project context",
                Language::SimplifiedChinese => "包含项目上下文",
            },
            (_, TrKey::AiCtxDesc) => match self {
                Language::English => {
                    "Adds README.md, CLAUDE.md and AGENTS.md to the prompt. ~4k extra tokens per request."
                }
                Language::SimplifiedChinese => {
                    "将 README.md、CLAUDE.md 与 AGENTS.md 加入提示词，每次请求约多消耗 4k token。"
                }
            },
            (_, TrKey::AiToolsTitle) => match self {
                Language::English => "Let the model read files",
                Language::SimplifiedChinese => "允许模型读取文件",
            },
            (_, TrKey::AiToolsDesc) => match self {
                Language::English => {
                    "The model may request file contents and commit history. Slower and more expensive; usually a better message."
                }
                Language::SimplifiedChinese => {
                    "模型可按需读取文件内容与提交历史。速度更慢、费用更高，但提交信息通常更好。"
                }
            },
            (_, TrKey::AiKeyPlaceholderFmt) => match self {
                Language::English => "Paste your {} API key",
                Language::SimplifiedChinese => "粘贴您的 {} API 密钥",
            },
            (_, TrKey::ShowLineNumbersTitle) => match self {
                Language::English => "Show Line Numbers in Diff",
                Language::SimplifiedChinese => "在 Diff 中显示行号",
            },
            (_, TrKey::ShowLineNumbersDesc) => match self {
                Language::English => "Display line numbers alongside diff content.",
                Language::SimplifiedChinese => "在 diff 内容旁显示行号。",
            },
            (_, TrKey::WrapLinesTitle) => match self {
                Language::English => "Wrap Long Lines in Diff",
                Language::SimplifiedChinese => "在 Diff 中自动换行",
            },
            (_, TrKey::WrapLinesDesc) => match self {
                Language::English => "Wrap overflowing lines instead of scrolling horizontally.",
                Language::SimplifiedChinese => "超长行自动换行，而非横向滚动。",
            },
            (_, TrKey::DiffModeTitle) => match self {
                Language::English => "Default Diff View Mode",
                Language::SimplifiedChinese => "默认 Diff 视图",
            },
            (_, TrKey::DiffModeDesc) => match self {
                Language::English => "Choose how diffs are displayed.",
                Language::SimplifiedChinese => "选择 diff 的显示方式。",
            },
            (_, TrKey::DiffUnified) => match self {
                Language::English => "Unified",
                Language::SimplifiedChinese => "统一视图",
            },
            (_, TrKey::DiffSideBySide) => match self {
                Language::English => "Side-by-Side",
                Language::SimplifiedChinese => "并排视图",
            },
            (_, TrKey::GraphTitle) => match self {
                Language::English => "Graph Style",
                Language::SimplifiedChinese => "提交图样式",
            },
            (_, TrKey::GraphDesc) => match self {
                Language::English => "Visual style for the commit graph rendering.",
                Language::SimplifiedChinese => "提交图渲染的视觉样式。",
            },
            (_, TrKey::GraphRails) => match self {
                Language::English => "Rails",
                Language::SimplifiedChinese => "轨道",
            },
            (_, TrKey::GraphCurved) => match self {
                Language::English => "Curved",
                Language::SimplifiedChinese => "曲线",
            },
            (_, TrKey::GraphAngular) => match self {
                Language::English => "Angular",
                Language::SimplifiedChinese => "折线",
            },
            (_, TrKey::ShowSubjectTitle) => match self {
                Language::English => "Show Subject Column",
                Language::SimplifiedChinese => "显示主题列",
            },
            (_, TrKey::ShowSubjectDesc) => match self {
                Language::English => "Display commit subject in the graph.",
                Language::SimplifiedChinese => "在提交图中显示提交主题。",
            },
            (_, TrKey::AutoFetchTitle) => match self {
                Language::English => "Auto-Fetch Interval",
                Language::SimplifiedChinese => "自动获取间隔",
            },
            (_, TrKey::AutoFetchDesc) => match self {
                Language::English => "How often to automatically fetch from remotes in the background.",
                Language::SimplifiedChinese => "后台自动从远程获取的时间间隔。",
            },
            (_, TrKey::ConfirmDestructiveTitle) => match self {
                Language::English => "Confirm Before Destructive Operations",
                Language::SimplifiedChinese => "危险操作前确认",
            },
            (_, TrKey::ConfirmDestructiveDesc) => match self {
                Language::English => "Show a confirmation dialog before force push, branch delete, discard changes, and similar actions.",
                Language::SimplifiedChinese => "在强制推送、删除分支、丢弃更改等操作前弹出确认框。",
            },
            (_, TrKey::CheckUpdatesTitle) => match self {
                Language::English => "Check for Updates on Startup",
                Language::SimplifiedChinese => "启动时检查更新",
            },
            (_, TrKey::CheckUpdatesDesc) => match self {
                Language::English => "Contact api.github.com once per day to see if a newer release is available. Turn off to keep rgitui offline.",
                Language::SimplifiedChinese => "每天联系一次 api.github.com 检查新版本。关闭可保持 rgitui 离线。",
            },
            (_, TrKey::WatchWorktreesTitle) => match self {
                Language::English => "Watch All Worktrees",
                Language::SimplifiedChinese => "监视所有工作树",
            },
            (_, TrKey::WatchWorktreesDesc) => match self {
                Language::English => "Refresh the graph whenever files change in any linked worktree, not just the current one. Useful when you have multiple worktrees open and work is happening in them in parallel.",
                Language::SimplifiedChinese => "任何关联工作树文件变化时都刷新提交图，而不仅是当前工作树。并行使用多个工作树时很有用。",
            },
            (_, TrKey::ExternalToolsTitle) => match self {
                Language::English => "External Tools",
                Language::SimplifiedChinese => "外部工具",
            },
            (_, TrKey::ExternalToolsDesc) => match self {
                Language::English => "Select a detected application or enter a custom command.",
                Language::SimplifiedChinese => "选择检测到的应用或输入自定义命令。",
            },
            (_, TrKey::TerminalLabel) => match self {
                Language::English => "Terminal",
                Language::SimplifiedChinese => "终端",
            },
            (_, TrKey::EditorLabel) => match self {
                Language::English => "Editor",
                Language::SimplifiedChinese => "编辑器",
            },
            (_, TrKey::CustomCommandLabel) => match self {
                Language::English => "Custom command (overrides selection)",
                Language::SimplifiedChinese => "自定义命令（覆盖所选项）",
            },
            (_, TrKey::ShortcutsTitle) => match self {
                Language::English => "Keyboard Shortcuts",
                Language::SimplifiedChinese => "键盘快捷键",
            },
            (_, TrKey::ShortcutsDesc) => match self {
                Language::English => "The bindings in force for a few common actions. Open the full reference from the workspace for all of them.",
                Language::SimplifiedChinese => "几个常用操作的当前绑定。从工作区可打开完整快捷键参考。",
            },
            (_, TrKey::ConfigTitle) => match self {
                Language::English => "Config File",
                Language::SimplifiedChinese => "配置文件",
            },
            (_, TrKey::ConfigDesc) => match self {
                Language::English => "Location of the settings file on disk.",
                Language::SimplifiedChinese => "磁盘上设置文件的位置。",
            },
            (_, TrKey::RevealBtn) => match self {
                Language::English => "Reveal",
                Language::SimplifiedChinese => "打开所在目录",
            },
            (_, TrKey::EditKeymapBtn) => match self {
                Language::English => "Edit keymap.json",
                Language::SimplifiedChinese => "编辑 keymap.json",
            },
            (_, TrKey::SavedFeedback) => match self {
                Language::English => "Saved",
                Language::SimplifiedChinese => "已保存",
            },
            (_, TrKey::TbFetch) => match self {
                Language::English => "Fetch",
                Language::SimplifiedChinese => "获取",
            },
            (_, TrKey::TbFetching) => match self {
                Language::English => "Fetching...",
                Language::SimplifiedChinese => "获取中…",
            },
            (_, TrKey::TbPull) => match self {
                Language::English => "Pull",
                Language::SimplifiedChinese => "拉取",
            },
            (_, TrKey::TbPulling) => match self {
                Language::English => "Pulling...",
                Language::SimplifiedChinese => "拉取中…",
            },
            (_, TrKey::TbPush) => match self {
                Language::English => "Push",
                Language::SimplifiedChinese => "推送",
            },
            (_, TrKey::TbPushing) => match self {
                Language::English => "Pushing...",
                Language::SimplifiedChinese => "推送中…",
            },
            (_, TrKey::TbBranch) => match self {
                Language::English => "Branch",
                Language::SimplifiedChinese => "分支",
            },
            (_, TrKey::TbStash) => match self {
                Language::English => "Stash",
                Language::SimplifiedChinese => "储藏",
            },
            (_, TrKey::TbPop) => match self {
                Language::English => "Pop",
                Language::SimplifiedChinese => "弹出",
            },
            (_, TrKey::TbCreatePr) => match self {
                Language::English => "Create PR",
                Language::SimplifiedChinese => "创建 PR",
            },
            (_, TrKey::StatusClean) => match self {
                Language::English => "Clean",
                Language::SimplifiedChinese => "干净",
            },
            (_, TrKey::StatusDetached) => match self {
                Language::English => "DETACHED",
                Language::SimplifiedChinese => "已分离",
            },
            (_, TrKey::SideBranches) => match self {
                Language::English => "Branches",
                Language::SimplifiedChinese => "分支",
            },
            (_, TrKey::SideRemotes) => match self {
                Language::English => "Remotes",
                Language::SimplifiedChinese => "远程",
            },
            (_, TrKey::SideRemoteBranches) => match self {
                Language::English => "Remote Branches",
                Language::SimplifiedChinese => "远程分支",
            },
            (_, TrKey::SideTags) => match self {
                Language::English => "Tags",
                Language::SimplifiedChinese => "标签",
            },
            (_, TrKey::SideStashes) => match self {
                Language::English => "Stashes",
                Language::SimplifiedChinese => "储藏",
            },
            (_, TrKey::SideWorktrees) => match self {
                Language::English => "Worktrees",
                Language::SimplifiedChinese => "工作树",
            },
            (_, TrKey::SideStaged) => match self {
                Language::English => "Staged",
                Language::SimplifiedChinese => "已暂存",
            },
            (_, TrKey::SideUnstaged) => match self {
                Language::English => "Unstaged",
                Language::SimplifiedChinese => "未暂存",
            },
            (_, TrKey::SideStageAll) => match self {
                Language::English => "Stage All",
                Language::SimplifiedChinese => "全部暂存",
            },
            (_, TrKey::SideUnstageAll) => match self {
                Language::English => "Unstage All",
                Language::SimplifiedChinese => "全部取消暂存",
            },
            (_, TrKey::SideNewWorktree) => match self {
                Language::English => "New Worktree",
                Language::SimplifiedChinese => "新建工作树",
            },
            (_, TrKey::SideWorkingClean) => match self {
                Language::English => "Working tree clean",
                Language::SimplifiedChinese => "工作树干净",
            },
            (_, TrKey::SideNoStaged) => match self {
                Language::English => "No staged changes",
                Language::SimplifiedChinese => "暂无已暂存的更改",
            },
            (_, TrKey::CommitBtn) => match self {
                Language::English => "Commit",
                Language::SimplifiedChinese => "提交",
            },
            (_, TrKey::CommitNoStaged) => match self {
                Language::English => "No Staged Changes",
                Language::SimplifiedChinese => "暂无已暂存的更改",
            },
            (_, TrKey::CommitNoMessage) => match self {
                Language::English => "No Message",
                Language::SimplifiedChinese => "请输入提交信息",
            },
            (_, TrKey::CommitAmendBtn) => match self {
                Language::English => "Amend Commit",
                Language::SimplifiedChinese => "修正提交",
            },
            (_, TrKey::AmendBtn) => match self {
                Language::English => "Amend",
                Language::SimplifiedChinese => "修正",
            },
            (_, TrKey::ClearBtn) => match self {
                Language::English => "Clear",
                Language::SimplifiedChinese => "清空",
            },
            (_, TrKey::SummaryLabel) => match self {
                Language::English => "Summary",
                Language::SimplifiedChinese => "摘要",
            },
            (_, TrKey::DescLabel) => match self {
                Language::English => "Description",
                Language::SimplifiedChinese => "详细描述",
            },
            (_, TrKey::CoAuthorsLabel) => match self {
                Language::English => "Co-Authors",
                Language::SimplifiedChinese => "共同作者",
            },
            (_, TrKey::NoStagedHint) => match self {
                Language::English => "No staged changes",
                Language::SimplifiedChinese => "暂无已暂存的更改",
            },
            (_, TrKey::CancelBtn) => match self {
                Language::English => "Cancel",
                Language::SimplifiedChinese => "取消",
            },
            (_, TrKey::ConfirmHint) => match self {
                Language::English => "Enter to confirm | Esc to cancel",
                Language::SimplifiedChinese => "回车确认 | Esc 取消",
            },
            (_, TrKey::CfConfirm) => match self {
                Language::English => "Confirm",
                Language::SimplifiedChinese => "确认",
            },
            (_, TrKey::CfDiscard) => match self {
                Language::English => "Discard",
                Language::SimplifiedChinese => "丢弃",
            },
            (_, TrKey::CfClean) => match self {
                Language::English => "Clean",
                Language::SimplifiedChinese => "清理",
            },
            (_, TrKey::CfDeleteBranch) => match self {
                Language::English => "Delete Branch",
                Language::SimplifiedChinese => "删除分支",
            },
            (_, TrKey::CfDeleteTag) => match self {
                Language::English => "Delete Tag",
                Language::SimplifiedChinese => "删除标签",
            },
            (_, TrKey::CfRemove) => match self {
                Language::English => "Remove",
                Language::SimplifiedChinese => "移除",
            },
            (_, TrKey::CfDropStash) => match self {
                Language::English => "Drop Stash",
                Language::SimplifiedChinese => "丢弃储藏",
            },
            (_, TrKey::CfReset) => match self {
                Language::English => "Reset",
                Language::SimplifiedChinese => "重置",
            },
            (_, TrKey::CfAbort) => match self {
                Language::English => "Abort",
                Language::SimplifiedChinese => "中止",
            },
            (_, TrKey::CfForcePush) => match self {
                Language::English => "Force Push",
                Language::SimplifiedChinese => "强制推送",
            },
            (_, TrKey::CfRemoveWorktree) => match self {
                Language::English => "Remove Worktree",
                Language::SimplifiedChinese => "移除工作树",
            },
            (_, TrKey::DiffFileBtn) => match self {
                Language::English => "File",
                Language::SimplifiedChinese => "文件",
            },
            (_, TrKey::DiffEmptyTitle) => match self {
                Language::English => "Select a file to view changes",
                Language::SimplifiedChinese => "选择文件以查看更改",
            },
            (_, TrKey::DiffEmptyDesc) => match self {
                Language::English => "Click a file in the sidebar or detail panel",
                Language::SimplifiedChinese => "点击侧边栏或详情面板中的文件",
            },
            (_, TrKey::DiffOpenFile) => match self {
                Language::English => "Open File",
                Language::SimplifiedChinese => "打开文件",
            },
            (_, TrKey::DetailTitle) => match self {
                Language::English => "Details",
                Language::SimplifiedChinese => "详情",
            },
            (_, TrKey::DetailEmptyTitle) => match self {
                Language::English => "No commit selected",
                Language::SimplifiedChinese => "未选择提交",
            },
            (_, TrKey::DetailEmptyDesc) => match self {
                Language::English => "Select a commit from the graph to view details",
                Language::SimplifiedChinese => "从提交图中选择一个提交以查看详情",
            },
            (_, TrKey::HomeSlogan) => match self {
                Language::English => "A workspace-oriented desktop Git client",
                Language::SimplifiedChinese => "面向工作区的桌面 Git 客户端",
            },
            (_, TrKey::OpenRepoTitle) => match self {
                Language::English => "Open Repository",
                Language::SimplifiedChinese => "打开仓库",
            },
            (_, TrKey::HomeNewWorkspace) => match self {
                Language::English => "New Workspace",
                Language::SimplifiedChinese => "新建工作区",
            },
            (_, TrKey::HomeRestoreLast) => match self {
                Language::English => "Restore Last",
                Language::SimplifiedChinese => "恢复上次",
            },
            (_, TrKey::HomeRecentWorkspaces) => match self {
                Language::English => "Recent Workspaces",
                Language::SimplifiedChinese => "最近的工作区",
            },
            (_, TrKey::HomeRecentRepos) => match self {
                Language::English => "Recent Repositories",
                Language::SimplifiedChinese => "最近的仓库",
            },
            (_, TrKey::WsSummaryFmt) => match self {
                Language::English => "{} repositories | updated {}",
                Language::SimplifiedChinese => "{} 个仓库 | 更新于 {}",
            },
            (_, TrKey::LoadingDots) => match self {
                Language::English => "Loading...",
                Language::SimplifiedChinese => "加载中…",
            },
            (_, TrKey::OpRetry) => match self {
                Language::English => "Retry",
                Language::SimplifiedChinese => "重试",
            },
            (_, TrKey::ConflictOneFmt) => match self {
                Language::English => "{} file with conflicts -- resolve before continuing",
                Language::SimplifiedChinese => "有 {} 个文件存在冲突——继续前请先解决",
            },
            (_, TrKey::ConflictManyFmt) => match self {
                Language::English => "{} files with conflicts -- resolve before continuing",
                Language::SimplifiedChinese => "有 {} 个文件存在冲突——继续前请先解决",
            },
            (_, TrKey::ConflictResolved) => match self {
                Language::English => "All conflicts resolved -- ready to continue",
                Language::SimplifiedChinese => "所有冲突已解决——可以继续",
            },
            (_, TrKey::ConflictContinue) => match self {
                Language::English => "Continue",
                Language::SimplifiedChinese => "继续",
            },
            (_, TrKey::WorktreeExit) => match self {
                Language::English => "Exit Worktree",
                Language::SimplifiedChinese => "退出工作树",
            },
            (_, TrKey::InspectingPre) => match self {
                Language::English => "Inspecting worktree:",
                Language::SimplifiedChinese => "正在查看工作树：",
            },
            (_, TrKey::DetachedHeadWord) => match self {
                Language::English => "detached HEAD",
                Language::SimplifiedChinese => "已分离的 HEAD",
            },
            (_, TrKey::BottomDiff) => match self {
                Language::English => "Diff",
                Language::SimplifiedChinese => "差异",
            },
            (_, TrKey::BottomDiffTip) => match self {
                Language::English => "Show diff (d)",
                Language::SimplifiedChinese => "显示差异 (d)",
            },
            (_, TrKey::BottomHistory) => match self {
                Language::English => "History",
                Language::SimplifiedChinese => "历史",
            },
            (_, TrKey::HistLoading) => match self {
                Language::English => "Preparing file history...",
                Language::SimplifiedChinese => "正在准备文件历史…",
            },
            (_, TrKey::HistShow) => match self {
                Language::English => "Show file history (h)",
                Language::SimplifiedChinese => "显示文件历史 (h)",
            },
            (_, TrKey::HistUnavailable) => match self {
                Language::English => "No committed history is available for this file",
                Language::SimplifiedChinese => "该文件暂无已提交历史",
            },
            (_, TrKey::BottomBlame) => match self {
                Language::English => "Blame",
                Language::SimplifiedChinese => "追溯",
            },
            (_, TrKey::BlameLoading) => match self {
                Language::English => "Preparing blame...",
                Language::SimplifiedChinese => "正在准备追溯…",
            },
            (_, TrKey::BlameShow) => match self {
                Language::English => "Show blame (b)",
                Language::SimplifiedChinese => "显示追溯 (b)",
            },
            (_, TrKey::BlameUnavailable) => match self {
                Language::English => "Blame is unavailable for this file at the selected commit",
                Language::SimplifiedChinese => "所选提交中的该文件无法追溯",
            },
            (_, TrKey::GraphShowTip) => match self {
                Language::English => "Show commit graph",
                Language::SimplifiedChinese => "显示提交图",
            },
            (_, TrKey::GraphHideTip) => match self {
                Language::English => "Hide commit graph",
                Language::SimplifiedChinese => "隐藏提交图",
            },
            (_, TrKey::DetachedTitlePre) => match self {
                Language::English => "HEAD is detached at",
                Language::SimplifiedChinese => "HEAD 已分离，位于",
            },
            (_, TrKey::DetachedConsequence) => match self {
                Language::English => "Commits made here belong to no branch",
                Language::SimplifiedChinese => "此处所做的提交不属于任何分支",
            },
            (_, TrKey::DetachedReturnPre) => match self {
                Language::English => "Return to",
                Language::SimplifiedChinese => "返回",
            },
            (_, TrKey::DetachedReturnTipPre) => match self {
                Language::English => "Check out '",
                Language::SimplifiedChinese => "检出 '",
            },
            (_, TrKey::DetachedReturnTipPost) => match self {
                Language::English => "', the branch HEAD was on before",
                Language::SimplifiedChinese => "'，HEAD 之前所在的分支",
            },
            (_, TrKey::DetachedSwitch) => match self {
                Language::English => "Switch Branch…",
                Language::SimplifiedChinese => "切换分支…",
            },
            (_, TrKey::DetachedShowGraph) => match self {
                Language::English => "Show in Graph",
                Language::SimplifiedChinese => "在提交图中显示",
            },
            (_, TrKey::UpdateMsgFmt) => match self {
                Language::English => "rgitui {} is available (you have {})",
                Language::SimplifiedChinese => "rgitui {} 可用（当前版本 {}）",
            },
            (_, TrKey::UpdateDownload) => match self {
                Language::English => "Download",
                Language::SimplifiedChinese => "下载",
            },
            (_, TrKey::MenuCherryPick) => match self {
                Language::English => "Cherry-pick commit",
                Language::SimplifiedChinese => "拣选提交",
            },
            (_, TrKey::MenuRevert) => match self {
                Language::English => "Revert commit",
                Language::SimplifiedChinese => "还原提交",
            },
            (_, TrKey::MenuCheckout) => match self {
                Language::English => "Checkout commit",
                Language::SimplifiedChinese => "检出提交",
            },
            (_, TrKey::MenuCreateBranch) => match self {
                Language::English => "Create branch here",
                Language::SimplifiedChinese => "在此创建分支",
            },
            (_, TrKey::MenuCreateTag) => match self {
                Language::English => "Create tag here",
                Language::SimplifiedChinese => "在此创建标签",
            },
            (_, TrKey::MenuBisectGood) => match self {
                Language::English => "Mark as good (bisect)",
                Language::SimplifiedChinese => "标记为正常（bisect）",
            },
            (_, TrKey::MenuBisectBad) => match self {
                Language::English => "Mark as bad (bisect)",
                Language::SimplifiedChinese => "标记为有问题（bisect）",
            },
            (_, TrKey::MenuReset) => match self {
                Language::English => "Reset to here",
                Language::SimplifiedChinese => "重置到此处",
            },
            (_, TrKey::RebaseTitle) => match self {
                Language::English => "Interactive Rebase",
                Language::SimplifiedChinese => "交互式变基",
            },
            (_, TrKey::MenuSquash) => match self {
                Language::English => "Squash selected commits",
                Language::SimplifiedChinese => "压缩所选提交",
            },
            (_, TrKey::MenuCopySha) => match self {
                Language::English => "Copy SHA",
                Language::SimplifiedChinese => "复制 SHA",
            },
            (_, TrKey::MenuCopyMsg) => match self {
                Language::English => "Copy commit message",
                Language::SimplifiedChinese => "复制提交信息",
            },
            (_, TrKey::MenuCopyAuthor) => match self {
                Language::English => "Copy author name",
                Language::SimplifiedChinese => "复制作者姓名",
            },
            (_, TrKey::MenuCopyDate) => match self {
                Language::English => "Copy date",
                Language::SimplifiedChinese => "复制日期",
            },
            (_, TrKey::MenuViewGithub) => match self {
                Language::English => "View on GitHub",
                Language::SimplifiedChinese => "在 GitHub 上查看",
            },
            (_, TrKey::GraphPanelTitle) => match self {
                Language::English => "Graph",
                Language::SimplifiedChinese => "提交图",
            },
            (_, TrKey::GraphEmpty) => match self {
                Language::English => "No commits to display",
                Language::SimplifiedChinese => "暂无提交可显示",
            },
            (_, TrKey::GraphColHash) => match self {
                Language::English => "Hash",
                Language::SimplifiedChinese => "哈希",
            },
            (_, TrKey::GraphColMessage) => match self {
                Language::English => "Message",
                Language::SimplifiedChinese => "提交信息",
            },
            (_, TrKey::GraphColAuthor) => match self {
                Language::English => "Author",
                Language::SimplifiedChinese => "作者",
            },
            (_, TrKey::GraphColDate) => match self {
                Language::English => "Date",
                Language::SimplifiedChinese => "日期",
            },
            (_, TrKey::MyCommitsOn) => match self {
                Language::English => "Showing only your commits. Click to show all commits.",
                Language::SimplifiedChinese => "仅显示你的提交，点击显示全部提交。",
            },
            (_, TrKey::MyCommitsOff) => match self {
                Language::English => "Show only your commits. Click to filter by current user.",
                Language::SimplifiedChinese => "仅显示你的提交，点击按当前用户过滤。",
            },
            (_, TrKey::GripRebase) => match self {
                Language::English => "Drag to rebase",
                Language::SimplifiedChinese => "拖拽以变基",
            },
            (_, TrKey::NoMatches) => match self {
                Language::English => "No matches",
                Language::SimplifiedChinese => "无匹配项",
            },
            (_, TrKey::LoadMore) => match self {
                Language::English => "Load more commits",
                Language::SimplifiedChinese => "加载更多提交",
            },
            (_, TrKey::ShaShort) => match self {
                Language::English => "Short (7)",
                Language::SimplifiedChinese => "短 (7)",
            },
            (_, TrKey::ShaFull) => match self {
                Language::English => "Full (40)",
                Language::SimplifiedChinese => "完整 (40)",
            },
            (_, TrKey::ShaCharsFmt) => match self {
                Language::English => "{} chars",
                Language::SimplifiedChinese => "{} 字符",
            },
            (_, TrKey::DisplaySettings) => match self {
                Language::English => "Display Settings",
                Language::SimplifiedChinese => "显示设置",
            },
            (_, TrKey::ShaLengthLabel) => match self {
                Language::English => "SHA length:",
                Language::SimplifiedChinese => "SHA 长度：",
            },
            (_, TrKey::ShowSubjectCol) => match self {
                Language::English => "Show subject column",
                Language::SimplifiedChinese => "显示主题列",
            },
            (_, TrKey::ShowAuthorCol) => match self {
                Language::English => "Show author column",
                Language::SimplifiedChinese => "显示作者列",
            },
            (_, TrKey::ShowAuthorEmail) => match self {
                Language::English => "Show author email",
                Language::SimplifiedChinese => "显示作者邮箱",
            },
            (_, TrKey::ShowDateCol) => match self {
                Language::English => "Show date column",
                Language::SimplifiedChinese => "显示日期列",
            },
            (_, TrKey::AbsoluteDates) => match self {
                Language::English => "Absolute dates",
                Language::SimplifiedChinese => "绝对时间",
            },
            (_, TrKey::ShowAvatars) => match self {
                Language::English => "Show avatars",
                Language::SimplifiedChinese => "显示头像",
            },
            (_, TrKey::ShowLanes) => match self {
                Language::English => "Show graph lanes",
                Language::SimplifiedChinese => "显示提交图轨道",
            },
            (_, TrKey::ShowBadges) => match self {
                Language::English => "Show branch/tag badges",
                Language::SimplifiedChinese => "显示分支/标签徽章",
            },
            (_, TrKey::HeadDetachedBadge) => match self {
                Language::English => "HEAD (detached)",
                Language::SimplifiedChinese => "HEAD（已分离）",
            },
            (_, TrKey::PendingOnPre) => match self {
                Language::English => "Pending changes on",
                Language::SimplifiedChinese => "待提交更改于",
            },
            (_, TrKey::PendingChanges) => match self {
                Language::English => "Pending changes",
                Language::SimplifiedChinese => "待提交的更改",
            },
            (_, TrKey::NoCommitsYet) => match self {
                Language::English => "No commits yet",
                Language::SimplifiedChinese => "暂无提交",
            },
            (_, TrKey::NoCommitsSuffix) => match self {
                Language::English => "(no commits)",
                Language::SimplifiedChinese => "（暂无提交）",
            },
            (_, TrKey::NewBranchBadge) => match self {
                Language::English => "New branch",
                Language::SimplifiedChinese => "新分支",
            },
            (_, TrKey::SearchCommitsPh) => match self {
                Language::English => "Search commits...",
                Language::SimplifiedChinese => "搜索提交…",
            },
            (_, TrKey::PrsTitle) => match self {
                Language::English => "Pull Requests",
                Language::SimplifiedChinese => "拉取请求",
            },
            (_, TrKey::PrsTab) => match self {
                Language::English => "PRs",
                Language::SimplifiedChinese => "PR",
            },
            (_, TrKey::PrDetail) => match self {
                Language::English => "Pull Request Detail",
                Language::SimplifiedChinese => "拉取请求详情",
            },
            (_, TrKey::IssuesTitle) => match self {
                Language::English => "Issues",
                Language::SimplifiedChinese => "议题",
            },
            (_, TrKey::IssueDetail) => match self {
                Language::English => "Issue Detail",
                Language::SimplifiedChinese => "议题详情",
            },
            (_, TrKey::FilterOpen) => match self {
                Language::English => "Open",
                Language::SimplifiedChinese => "开启",
            },
            (_, TrKey::FilterClosed) => match self {
                Language::English => "Closed",
                Language::SimplifiedChinese => "已关闭",
            },
            (_, TrKey::FilterAll) => match self {
                Language::English => "All",
                Language::SimplifiedChinese => "全部",
            },
            (_, TrKey::PrNew) => match self {
                Language::English => "New pull request",
                Language::SimplifiedChinese => "新建拉取请求",
            },
            (_, TrKey::PrStateMerged) => match self {
                Language::English => "Merged",
                Language::SimplifiedChinese => "已合并",
            },
            (_, TrKey::DraftBadge) => match self {
                Language::English => "Draft",
                Language::SimplifiedChinese => "草稿",
            },
            (_, TrKey::LoadingComments) => match self {
                Language::English => "Loading comments...",
                Language::SimplifiedChinese => "正在加载评论…",
            },
            (_, TrKey::CommentsFailedPre) => match self {
                Language::English => "Failed to load comments",
                Language::SimplifiedChinese => "加载评论失败",
            },
            (_, TrKey::OneComment) => match self {
                Language::English => "1 comment",
                Language::SimplifiedChinese => "1 条评论",
            },
            (_, TrKey::ManyCommentsFmt) => match self {
                Language::English => "{} comments",
                Language::SimplifiedChinese => "{} 条评论",
            },
            (_, TrKey::GhSignInPrs) => match self {
                Language::English => "Sign in to view pull requests",
                Language::SimplifiedChinese => "登录后查看拉取请求",
            },
            (_, TrKey::GhSignInIssues) => match self {
                Language::English => "Sign in to view issues",
                Language::SimplifiedChinese => "登录后查看议题",
            },
            (_, TrKey::GhAuthDesc) => match self {
                Language::English => "This repository is private or rate-limited. Add a GitHub token in Settings — for organization repos you may need a fine-grained token approved by an org owner.",
                Language::SimplifiedChinese => "该仓库为私有或已触发限流。请在设置中添加 GitHub 令牌——组织仓库可能需要经组织所有者批准的细粒度令牌。",
            },
            (_, TrKey::TryFilterHint) => match self {
                Language::English => "Try a different filter or check back later",
                Language::SimplifiedChinese => "换个过滤条件或稍后再看",
            },
            (_, TrKey::PrEmptyFmt) => match self {
                Language::English => "No {} pull requests found",
                Language::SimplifiedChinese => "没有找到{}的拉取请求",
            },
            (_, TrKey::IssueEmptyFmt) => match self {
                Language::English => "No {} issues found",
                Language::SimplifiedChinese => "没有找到{}的议题",
            },
            (_, TrKey::ReviewActions) => match self {
                Language::English => "Review Actions",
                Language::SimplifiedChinese => "评审操作",
            },
            (_, TrKey::ReviewApprove) => match self {
                Language::English => "Approve",
                Language::SimplifiedChinese => "批准",
            },
            (_, TrKey::ReviewRequest) => match self {
                Language::English => "Request Changes",
                Language::SimplifiedChinese => "请求修改",
            },
            (_, TrKey::ReviewCommentBtn) => match self {
                Language::English => "Comment",
                Language::SimplifiedChinese => "评论",
            },
            (_, TrKey::ReviewSubmitting) => match self {
                Language::English => "Submitting...",
                Language::SimplifiedChinese => "提交中…",
            },
            (_, TrKey::ReviewCommentPh) => match self {
                Language::English => "Leave a review comment (optional)",
                Language::SimplifiedChinese => "留下评审意见（可选）",
            },
            (_, TrKey::SearchIssuesTip) => match self {
                Language::English => "Search issues",
                Language::SimplifiedChinese => "搜索议题",
            },
            (_, TrKey::SearchIssuesPh) => match self {
                Language::English => "Search issues...",
                Language::SimplifiedChinese => "搜索议题…",
            },
            (_, TrKey::RepoPathLabel) => match self {
                Language::English => "Repository path",
                Language::SimplifiedChinese => "仓库路径",
            },
            (_, TrKey::BrowseBtn) => match self {
                Language::English => "Browse",
                Language::SimplifiedChinese => "浏览",
            },
            (_, TrKey::CloneBtn) => match self {
                Language::English => "Clone",
                Language::SimplifiedChinese => "克隆",
            },
            (_, TrKey::NoMatchRepos) => match self {
                Language::English => "No matching repositories",
                Language::SimplifiedChinese => "没有匹配的仓库",
            },
            (_, TrKey::NoRecentRepos) => match self {
                Language::English => "No recent repositories",
                Language::SimplifiedChinese => "暂无最近仓库",
            },
            (_, TrKey::OpenBtn) => match self {
                Language::English => "Open",
                Language::SimplifiedChinese => "打开",
            },
            (_, TrKey::EnterRepoPathPh) => match self {
                Language::English => "Enter repository path...",
                Language::SimplifiedChinese => "输入仓库路径…",
            },
            (_, TrKey::CloneTitle) => match self {
                Language::English => "Clone Repository",
                Language::SimplifiedChinese => "克隆仓库",
            },
            (_, TrKey::CloneUrlLabel) => match self {
                Language::English => "URL",
                Language::SimplifiedChinese => "链接",
            },
            (_, TrKey::ClonePathLabel) => match self {
                Language::English => "Path",
                Language::SimplifiedChinese => "路径",
            },
            (_, TrKey::CloneUrlPh) => match self {
                Language::English => "Repository URL (e.g. https://github.com/user/repo.git)",
                Language::SimplifiedChinese => "仓库链接（例如 https://github.com/user/repo.git）",
            },
            (_, TrKey::ClonePathPh) => match self {
                Language::English => "Destination Path",
                Language::SimplifiedChinese => "目标路径",
            },
            (_, TrKey::CloneHint) => match self {
                Language::English => "Enter a repository URL and a destination path to clone.",
                Language::SimplifiedChinese => "输入仓库链接与目标路径以克隆。",
            },
            (_, TrKey::CloningBtn) => match self {
                Language::English => "Cloning…",
                Language::SimplifiedChinese => "克隆中…",
            },
            (_, TrKey::NameLabel) => match self {
                Language::English => "Name",
                Language::SimplifiedChinese => "名称",
            },
            (_, TrKey::WtPathLabel) => match self {
                Language::English => "Directory Path",
                Language::SimplifiedChinese => "目录路径",
            },
            (_, TrKey::WtBranchLabel) => match self {
                Language::English => "Branch (optional)",
                Language::SimplifiedChinese => "分支（可选）",
            },
            (_, TrKey::CreateBtn) => match self {
                Language::English => "Create",
                Language::SimplifiedChinese => "创建",
            },
            (_, TrKey::WtNamePh) => match self {
                Language::English => "Worktree name (e.g. feature-x)...",
                Language::SimplifiedChinese => "工作树名称（例如 feature-x）…",
            },
            (_, TrKey::WtPathPh) => match self {
                Language::English => "Directory path (e.g. /home/user/projects/myrepo-feature-x)...",
                Language::SimplifiedChinese => "目录路径（例如 /home/user/projects/myrepo-feature-x）…",
            },
            (_, TrKey::WtBranchPh) => match self {
                Language::English => "Branch (optional, defaults to current branch)...",
                Language::SimplifiedChinese => "分支（可选，默认为当前分支）…",
            },
            (_, TrKey::TagTitle) => match self {
                Language::English => "Create Tag",
                Language::SimplifiedChinese => "创建标签",
            },
            (_, TrKey::TagNameLabel) => match self {
                Language::English => "Tag name",
                Language::SimplifiedChinese => "标签名",
            },
            (_, TrKey::TagAtCommit) => match self {
                Language::English => "At commit",
                Language::SimplifiedChinese => "位于提交",
            },
            (_, TrKey::TagCreateHint) => match self {
                Language::English => "Enter to create | Esc to cancel",
                Language::SimplifiedChinese => "回车创建 | Esc 取消",
            },
            (_, TrKey::StashTitle) => match self {
                Language::English => "Create Stash",
                Language::SimplifiedChinese => "创建储藏",
            },
            (_, TrKey::StashMsgLabel) => match self {
                Language::English => "Message (optional)",
                Language::SimplifiedChinese => "留言（可选）",
            },
            (_, TrKey::StashMsgHint) => match self {
                Language::English => "Leave empty for default: \"WIP on <branch>\"",
                Language::SimplifiedChinese => "留空使用默认留言：“WIP on <branch>”",
            },
            (_, TrKey::RenameTitle) => match self {
                Language::English => "Rename Branch",
                Language::SimplifiedChinese => "重命名分支",
            },
            (_, TrKey::RenameCurrent) => match self {
                Language::English => "Current name",
                Language::SimplifiedChinese => "当前名称",
            },
            (_, TrKey::RenameNew) => match self {
                Language::English => "New name",
                Language::SimplifiedChinese => "新名称",
            },
            (_, TrKey::RenameHint) => match self {
                Language::English => "Enter to rename | Esc to cancel",
                Language::SimplifiedChinese => "回车重命名 | Esc 取消",
            },
            (_, TrKey::RenameBtn) => match self {
                Language::English => "Rename",
                Language::SimplifiedChinese => "重命名",
            },
            (_, TrKey::BranchTitle) => match self {
                Language::English => "Create Branch",
                Language::SimplifiedChinese => "创建分支",
            },
            (_, TrKey::BranchNameLabel) => match self {
                Language::English => "Branch name",
                Language::SimplifiedChinese => "分支名",
            },
            (_, TrKey::BranchBasedOn) => match self {
                Language::English => "Based on",
                Language::SimplifiedChinese => "基于",
            },
            (_, TrKey::BranchNamePh) => match self {
                Language::English => "Enter branch name...",
                Language::SimplifiedChinese => "输入分支名…",
            },
            (_, TrKey::StashBranchTitle) => match self {
                Language::English => "Create Branch from Stash",
                Language::SimplifiedChinese => "从储藏创建分支",
            },
            (_, TrKey::StashFrom) => match self {
                Language::English => "From",
                Language::SimplifiedChinese => "来自",
            },
            (_, TrKey::CreatePrTitle) => match self {
                Language::English => "New Pull Request",
                Language::SimplifiedChinese => "新建拉取请求",
            },
            (_, TrKey::PrInto) => match self {
                Language::English => "into",
                Language::SimplifiedChinese => "到",
            },
            (_, TrKey::PrTitleLabel) => match self {
                Language::English => "Title",
                Language::SimplifiedChinese => "标题",
            },
            (_, TrKey::PrTitlePh) => match self {
                Language::English => "Pull request title",
                Language::SimplifiedChinese => "拉取请求标题",
            },
            (_, TrKey::PrDescPh) => match self {
                Language::English => "Add a description (optional)",
                Language::SimplifiedChinese => "添加描述（可选）",
            },
            (_, TrKey::PrDraftToggle) => match self {
                Language::English => "Create as draft pull request",
                Language::SimplifiedChinese => "创建为草稿拉取请求",
            },
            (_, TrKey::PrHint) => match self {
                Language::English => "Enter for a new line | Shift+Enter to create | Esc to cancel",
                Language::SimplifiedChinese => "回车换行 | Shift+回车创建 | Esc 取消",
            },
            (_, TrKey::PrSubmit) => match self {
                Language::English => "Create pull request",
                Language::SimplifiedChinese => "创建拉取请求",
            },
            (_, TrKey::PrCreating) => match self {
                Language::English => "Creating pull request...",
                Language::SimplifiedChinese => "正在创建拉取请求…",
            },
            (_, TrKey::RebasePick) => match self {
                Language::English => "Pick",
                Language::SimplifiedChinese => "拣选",
            },
            (_, TrKey::RebaseReword) => match self {
                Language::English => "Reword",
                Language::SimplifiedChinese => "改写信息",
            },
            (_, TrKey::RebaseSquash) => match self {
                Language::English => "Squash",
                Language::SimplifiedChinese => "压缩",
            },
            (_, TrKey::RebaseFixup) => match self {
                Language::English => "Fixup",
                Language::SimplifiedChinese => "并入",
            },
            (_, TrKey::RebaseDrop) => match self {
                Language::English => "Drop",
                Language::SimplifiedChinese => "丢弃",
            },
            (_, TrKey::RebaseHeaderFmt) => match self {
                Language::English => "Rebasing {} commits onto {}",
                Language::SimplifiedChinese => "正在将 {} 个提交变基到 {}",
            },
            (_, TrKey::RebasePh) => match self {
                Language::English => "Enter new commit message...",
                Language::SimplifiedChinese => "输入新的提交信息…",
            },
            (_, TrKey::RebaseDragTip) => match self {
                Language::English => "Drag to reorder",
                Language::SimplifiedChinese => "拖拽以排序",
            },
            (_, TrKey::RebaseHintNav) => match self {
                Language::English => "j/k Navigate",
                Language::SimplifiedChinese => "j/k 导航",
            },
            (_, TrKey::RebaseHintReorder) => match self {
                Language::English => "Ctrl+Up/Down Reorder",
                Language::SimplifiedChinese => "Ctrl+Up/Down 排序",
            },
            (_, TrKey::RebaseHintAction) => match self {
                Language::English => "p/r/s/f/d Set action",
                Language::SimplifiedChinese => "p/r/s/f/d 设置动作",
            },
            (_, TrKey::RebaseHintStart) => match self {
                Language::English => "Enter Start",
                Language::SimplifiedChinese => "回车开始",
            },
            (_, TrKey::RebaseHintCancel) => match self {
                Language::English => "Esc Cancel",
                Language::SimplifiedChinese => "Esc 取消",
            },
            (_, TrKey::RebaseStart) => match self {
                Language::English => "Start Rebase",
                Language::SimplifiedChinese => "开始变基",
            },
            (_, TrKey::BisectTitle) => match self {
                Language::English => "Bisect",
                Language::SimplifiedChinese => "二分查找",
            },
            (_, TrKey::BisectStarted) => match self {
                Language::English => "started",
                Language::SimplifiedChinese => "已开始",
            },
            (_, TrKey::BisectBadFmt) => match self {
                Language::English => "{} bad",
                Language::SimplifiedChinese => "不良 {}",
            },
            (_, TrKey::BisectGoodFmt) => match self {
                Language::English => "{} good",
                Language::SimplifiedChinese => "良好 {}",
            },
            (_, TrKey::BisectSkipFmt) => match self {
                Language::English => "{} skip",
                Language::SimplifiedChinese => "跳过 {}",
            },
            (_, TrKey::BisectLeftFmt) => match self {
                Language::English => "~{} steps left",
                Language::SimplifiedChinese => "还剩约 {} 步",
            },
            (_, TrKey::BisectEntriesFmt) => match self {
                Language::English => "{} entries",
                Language::SimplifiedChinese => "{} 条记录",
            },
            (_, TrKey::BisectEmpty) => match self {
                Language::English => "No bisect in progress",
                Language::SimplifiedChinese => "暂无进行中的二分查找",
            },
            (_, TrKey::BisectEmptyHint) => match self {
                Language::English => "Use 'Git: Bisect Start' to begin",
                Language::SimplifiedChinese => "使用 'Git: Bisect Start' 开始",
            },
            (_, TrKey::SplashSubtitle) => match self {
                Language::English => "GPU-accelerated Git",
                Language::SimplifiedChinese => "GPU 加速的 Git",
            },
            (_, TrKey::SplashLoading) => match self {
                Language::English => "Loading workspace…",
                Language::SimplifiedChinese => "正在加载工作区…",
            },
            (_, TrKey::ShortcutsCloseHint) => match self {
                Language::English => "Press Esc or click outside to close",
                Language::SimplifiedChinese => "按 Esc 或点击外部关闭",
            },
            (_, TrKey::MoreActionsPre) => match self {
                Language::English => "More actions",
                Language::SimplifiedChinese => "更多操作",
            },
            (_, TrKey::ThemeTitle) => match self {
                Language::English => "Theme Editor",
                Language::SimplifiedChinese => "主题编辑器",
            },
            (_, TrKey::ThemeColors) => match self {
                Language::English => "Colors",
                Language::SimplifiedChinese => "颜色",
            },
            (_, TrKey::ThemeStatusColors) => match self {
                Language::English => "Status Colors",
                Language::SimplifiedChinese => "状态颜色",
            },
            (_, TrKey::ThemeDark) => match self {
                Language::English => "dark",
                Language::SimplifiedChinese => "深色",
            },
            (_, TrKey::ThemeLight) => match self {
                Language::English => "light",
                Language::SimplifiedChinese => "浅色",
            },
            (_, TrKey::ThemeHint) => match self {
                Language::English => "Tab to move · Enter to save · Esc to close",
                Language::SimplifiedChinese => "Tab 移动 · 回车保存 · Esc 关闭",
            },
            (_, TrKey::SaveBtn) => match self {
                Language::English => "Save",
                Language::SimplifiedChinese => "保存",
            },
            (_, TrKey::ThemeBackground) => match self {
                Language::English => "Background",
                Language::SimplifiedChinese => "背景",
            },
            (_, TrKey::ThemeSurface) => match self {
                Language::English => "Surface",
                Language::SimplifiedChinese => "表面",
            },
            (_, TrKey::ThemeElevated) => match self {
                Language::English => "Elevated Surface",
                Language::SimplifiedChinese => "浮层",
            },
            (_, TrKey::ThemeBorder) => match self {
                Language::English => "Border",
                Language::SimplifiedChinese => "边框",
            },
            (_, TrKey::ThemeText) => match self {
                Language::English => "Text",
                Language::SimplifiedChinese => "文本",
            },
            (_, TrKey::ThemeTextMuted) => match self {
                Language::English => "Text Muted",
                Language::SimplifiedChinese => "次要文本",
            },
            (_, TrKey::ThemeTextAccent) => match self {
                Language::English => "Text Accent",
                Language::SimplifiedChinese => "强调文本",
            },
            (_, TrKey::ThemePlaceholder) => match self {
                Language::English => "Text Placeholder",
                Language::SimplifiedChinese => "占位文本",
            },
            (_, TrKey::ThemeIcon) => match self {
                Language::English => "Icon",
                Language::SimplifiedChinese => "图标",
            },
            (_, TrKey::ThemeFocusRing) => match self {
                Language::English => "Focus Ring",
                Language::SimplifiedChinese => "聚焦环",
            },
            (_, TrKey::ThemeSelectedBorder) => match self {
                Language::English => "Selected Border",
                Language::SimplifiedChinese => "选中边框",
            },
            (_, TrKey::ThemeAdded) => match self {
                Language::English => "Added",
                Language::SimplifiedChinese => "新增",
            },
            (_, TrKey::ThemeModified) => match self {
                Language::English => "Modified",
                Language::SimplifiedChinese => "修改",
            },
            (_, TrKey::ThemeDeleted) => match self {
                Language::English => "Deleted",
                Language::SimplifiedChinese => "删除",
            },
            (_, TrKey::ThemeUntracked) => match self {
                Language::English => "Untracked",
                Language::SimplifiedChinese => "未跟踪",
            },
            (_, TrKey::ThemeConflict) => match self {
                Language::English => "Conflict",
                Language::SimplifiedChinese => "冲突",
            },
            (_, TrKey::ThemeRenamed) => match self {
                Language::English => "Renamed",
                Language::SimplifiedChinese => "重命名",
            },
            (_, TrKey::ThemeError) => match self {
                Language::English => "Error",
                Language::SimplifiedChinese => "错误",
            },
            (_, TrKey::ThemeWarning) => match self {
                Language::English => "Warning",
                Language::SimplifiedChinese => "警告",
            },
            (_, TrKey::ThemeSuccess) => match self {
                Language::English => "Success",
                Language::SimplifiedChinese => "成功",
            },
            (_, TrKey::ThemeInfo) => match self {
                Language::English => "Info",
                Language::SimplifiedChinese => "信息",
            },
            (_, TrKey::TipOpenRepo) => match self {
                Language::English => "Open repository",
                Language::SimplifiedChinese => "打开仓库",
            },
            (_, TrKey::TipMerged) => match self {
                Language::English => "Merged into current branch",
                Language::SimplifiedChinese => "已合并到当前分支",
            },
            (_, TrKey::TipCopyBranch) => match self {
                Language::English => "Copy branch name",
                Language::SimplifiedChinese => "复制分支名",
            },
            (_, TrKey::TipCheckoutBranch) => match self {
                Language::English => "Checkout branch",
                Language::SimplifiedChinese => "检出分支",
            },
            (_, TrKey::TipMergeBranch) => match self {
                Language::English => "Merge into current branch",
                Language::SimplifiedChinese => "合并到当前分支",
            },
            (_, TrKey::TipRenameBranch) => match self {
                Language::English => "Rename branch",
                Language::SimplifiedChinese => "重命名分支",
            },
            (_, TrKey::TipDeleteBranch) => match self {
                Language::English => "Delete branch",
                Language::SimplifiedChinese => "删除分支",
            },
            (_, TrKey::TipFetchRemote) => match self {
                Language::English => "Fetch from remote",
                Language::SimplifiedChinese => "从远程获取",
            },
            (_, TrKey::TipPullRemote) => match self {
                Language::English => "Pull from remote",
                Language::SimplifiedChinese => "从远程拉取",
            },
            (_, TrKey::TipPushRemote) => match self {
                Language::English => "Push to remote",
                Language::SimplifiedChinese => "推送到远程",
            },
            (_, TrKey::TipRemoveRemote) => match self {
                Language::English => "Remove remote",
                Language::SimplifiedChinese => "移除远程",
            },
            (_, TrKey::TipCheckoutTag) => match self {
                Language::English => "Checkout tag",
                Language::SimplifiedChinese => "检出标签",
            },
            (_, TrKey::TipDeleteTag) => match self {
                Language::English => "Delete tag",
                Language::SimplifiedChinese => "删除标签",
            },
            (_, TrKey::ApplyStashLbl) => match self {
                Language::English => "Apply stash",
                Language::SimplifiedChinese => "应用储藏",
            },
            (_, TrKey::PopStashLbl) => match self {
                Language::English => "Pop stash",
                Language::SimplifiedChinese => "弹出储藏",
            },
            (_, TrKey::TipStashBranch) => match self {
                Language::English => "Create branch from stash",
                Language::SimplifiedChinese => "从储藏创建分支",
            },
            (_, TrKey::TipDropStash) => match self {
                Language::English => "Drop stash",
                Language::SimplifiedChinese => "丢弃储藏",
            },
            (_, TrKey::TipRemoveWorktree) => match self {
                Language::English => "Remove worktree",
                Language::SimplifiedChinese => "移除工作树",
            },
            (_, TrKey::TipUnstageFile) => match self {
                Language::English => "Unstage file",
                Language::SimplifiedChinese => "取消暂存文件",
            },
            (_, TrKey::TipStageFile) => match self {
                Language::English => "Stage file",
                Language::SimplifiedChinese => "暂存文件",
            },
            (_, TrKey::TipConflictResolver) => match self {
                Language::English => "Open conflict resolver",
                Language::SimplifiedChinese => "打开冲突解决器",
            },
            (_, TrKey::TipDiscardChanges) => match self {
                Language::English => "Discard changes",
                Language::SimplifiedChinese => "丢弃更改",
            },
            (_, TrKey::TipFilterLocal) => match self {
                Language::English => "Filter branches by name or last used",
                Language::SimplifiedChinese => "按名称或上次使用过滤分支",
            },
            (_, TrKey::TipFilterRemote) => match self {
                Language::English => "Filter remote branches by name or last used",
                Language::SimplifiedChinese => "按名称或上次使用过滤远程分支",
            },
            (_, TrKey::TipStagedOpts) => match self {
                Language::English => "Staged list options",
                Language::SimplifiedChinese => "已暂存列表选项",
            },
            (_, TrKey::TipUnstagedOpts) => match self {
                Language::English => "Unstaged list options",
                Language::SimplifiedChinese => "未暂存列表选项",
            },
            (_, TrKey::TipCopySha) => match self {
                Language::English => "Copy commit SHA",
                Language::SimplifiedChinese => "复制提交 SHA",
            },
            (_, TrKey::TipGpg) => match self {
                Language::English => "GPG-signed commit",
                Language::SimplifiedChinese => "GPG 签名提交",
            },
            (_, TrKey::TipCherryPick) => match self {
                Language::English => "Cherry-pick this commit",
                Language::SimplifiedChinese => "拣选此提交",
            },
            (_, TrKey::TipClearSearch) => match self {
                Language::English => "Clear search (Esc)",
                Language::SimplifiedChinese => "清除搜索（Esc）",
            },
            (_, TrKey::FlatViewTip) => match self {
                Language::English => "Switch to Tree view (v)",
                Language::SimplifiedChinese => "切换到树视图 (v)",
            },
            (_, TrKey::TreeViewTip) => match self {
                Language::English => "Switch to Flat view (v)",
                Language::SimplifiedChinese => "切换到平铺视图 (v)",
            },
            (_, TrKey::NoFilesTip) => match self {
                Language::English => "No changed files to display",
                Language::SimplifiedChinese => "暂无更改的文件",
            },
            (_, TrKey::ClearSearchTip) => match self {
                Language::English => "Clear file search to switch views",
                Language::SimplifiedChinese => "清除文件搜索以切换视图",
            },
            (_, TrKey::CommitSummaryPh) => match self {
                Language::English => "Commit summary...",
                Language::SimplifiedChinese => "提交摘要…",
            },
            (_, TrKey::CommitDescPh) => match self {
                Language::English => "Optional extended description...",
                Language::SimplifiedChinese => "可选的详细描述…",
            },
            (_, TrKey::CoAuthorNamePh) => match self {
                Language::English => "Co-author name...",
                Language::SimplifiedChinese => "共同作者姓名…",
            },
            (_, TrKey::CoAuthorEmailPh) => match self {
                Language::English => "Co-author email...",
                Language::SimplifiedChinese => "共同作者邮箱…",
            },
            (_, TrKey::TipExpandCommit) => match self {
                Language::English => "Expand commit panel",
                Language::SimplifiedChinese => "展开提交面板",
            },
            (_, TrKey::TipCollapseCommit) => match self {
                Language::English => "Collapse commit panel",
                Language::SimplifiedChinese => "折叠提交面板",
            },
            (_, TrKey::TipExplorer) => match self {
                Language::English => "Open in file explorer",
                Language::SimplifiedChinese => "在文件管理器中打开",
            },
            (_, TrKey::TipTerminal) => match self {
                Language::English => "Open terminal",
                Language::SimplifiedChinese => "打开终端",
            },
            (_, TrKey::TipEditor) => match self {
                Language::English => "Open in editor",
                Language::SimplifiedChinese => "在编辑器中打开",
            },
            (_, TrKey::TitleDetachedTag) => match self {
                Language::English => "(detached)",
                Language::SimplifiedChinese => "（已分离）",
            },
            (_, TrKey::TipApplyKeep) => match self {
                Language::English => "Apply stash (keep)",
                Language::SimplifiedChinese => "应用储藏（保留）",
            },
            (_, TrKey::TipPopRemove) => match self {
                Language::English => "Pop stash (remove)",
                Language::SimplifiedChinese => "弹出储藏（移除）",
            },
            (_, TrKey::StashesEmpty) => match self {
                Language::English => "No stashes",
                Language::SimplifiedChinese => "暂无储藏",
            },
            (_, TrKey::StashesEmptyHint) => match self {
                Language::English => "Use the toolbar to stash changes",
                Language::SimplifiedChinese => "使用工具栏储藏更改",
            },
            (_, TrKey::SearchPh) => match self {
                Language::English => "Search across all files... (press Enter)",
                Language::SimplifiedChinese => "搜索全部文件…（回车）",
            },
            (_, TrKey::SearchEmpty) => match self {
                Language::English => "Type to search across all files",
                Language::SimplifiedChinese => "输入以搜索全部文件",
            },
            (_, TrKey::SearchHint) => match self {
                Language::English => "Press Enter to search | j/k to navigate",
                Language::SimplifiedChinese => "回车搜索 | j/k 导航",
            },
            (_, TrKey::FilterFilesPh) => match self {
                Language::English => "Filter files...",
                Language::SimplifiedChinese => "过滤文件…",
            },
            (_, TrKey::FilterBranchesPh) => match self {
                Language::English => "Filter branches...",
                Language::SimplifiedChinese => "过滤分支…",
            },
            (_, TrKey::FilterRemotePh) => match self {
                Language::English => "Filter remote branches...",
                Language::SimplifiedChinese => "过滤远程分支…",
            },
            (_, TrKey::SetHttpsTokenPh) => match self {
                Language::English => "Paste a generic HTTPS token only if you need a fallback...",
                Language::SimplifiedChinese => "仅在需要备用时粘贴通用 HTTPS 令牌…",
            },
            (_, TrKey::SetGpgPh) => match self {
                Language::English => "No explicit GPG key configured",
                Language::SimplifiedChinese => "未配置 GPG 密钥",
            },
            (_, TrKey::SetUsuallyEmpty) => match self {
                Language::English => "Usually left empty",
                Language::SimplifiedChinese => "通常留空",
            },
            (_, TrKey::SetAccountTokenPh) => match self {
                Language::English => "Paste the access token for this account...",
                Language::SimplifiedChinese => "粘贴此账户的访问令牌…",
            },
            (_, TrKey::SetCustomCmdPh) => match self {
                Language::English => "Custom command override...",
                Language::SimplifiedChinese => "自定义命令覆盖…",
            },
            (_, TrKey::TipPaste) => match self {
                Language::English => "Paste from clipboard",
                Language::SimplifiedChinese => "从剪贴板粘贴",
            },
            (_, TrKey::TipHide) => match self {
                Language::English => "Hide",
                Language::SimplifiedChinese => "隐藏",
            },
            (_, TrKey::TipShow) => match self {
                Language::English => "Show",
                Language::SimplifiedChinese => "显示",
            },
            (_, TrKey::CopiedFb) => match self {
                Language::English => "Copied!",
                Language::SimplifiedChinese => "已复制！",
            },
            (_, TrKey::FindingBranches) => match self {
                Language::English => "Finding branches…",
                Language::SimplifiedChinese => "正在查找分支…",
            },
            (_, TrKey::ContainedIn) => match self {
                Language::English => "CONTAINED IN",
                Language::SimplifiedChinese => "包含于",
            },
            (_, TrKey::SearchFilesHint) => match self {
                Language::English => "/ to search files",
                Language::SimplifiedChinese => "/ 搜索文件",
            },
            (_, TrKey::SortLastUsed) => match self {
                Language::English => "Last used",
                Language::SimplifiedChinese => "上次使用",
            },
            (_, TrKey::OnlyMine) => match self {
                Language::English => "Only my branches",
                Language::SimplifiedChinese => "仅我的分支",
            },
            (_, TrKey::PopBranchFilters) => match self {
                Language::English => "Branch filters",
                Language::SimplifiedChinese => "分支过滤器",
            },
            (_, TrKey::PopRemoteFilters) => match self {
                Language::English => "Remote branch filters",
                Language::SimplifiedChinese => "远程分支过滤器",
            },
            (_, TrKey::PopStagedFiles) => match self {
                Language::English => "Staged files",
                Language::SimplifiedChinese => "已暂存文件",
            },
            (_, TrKey::PopUnstagedFiles) => match self {
                Language::English => "Unstaged files",
                Language::SimplifiedChinese => "未暂存文件",
            },
            (_, TrKey::ShowLineCounts) => match self {
                Language::English => "Show line counts",
                Language::SimplifiedChinese => "显示行数统计",
            },
            (_, TrKey::ChangeListHint) => match self {
                Language::English => "Applies to staged and unstaged files.",
                Language::SimplifiedChinese => "适用于已暂存与未暂存文件。",
            },
            (_, TrKey::NoRemotes) => match self {
                Language::English => "No remotes configured",
                Language::SimplifiedChinese => "未配置远程",
            },
            (_, TrKey::NoTags) => match self {
                Language::English => "No tags",
                Language::SimplifiedChinese => "暂无标签",
            },
            (_, TrKey::NoWorktrees) => match self {
                Language::English => "No worktrees",
                Language::SimplifiedChinese => "暂无工作树",
            },
            (_, TrKey::CurrentTag) => match self {
                Language::English => "(current)",
                Language::SimplifiedChinese => "（当前）",
            },
            (_, TrKey::StashMenuCreateBranch) => match self {
                Language::English => "Create branch",
                Language::SimplifiedChinese => "创建分支",
            },
            (_, TrKey::BlameEmpty) => match self {
                Language::English => "Select a file to view blame",
                Language::SimplifiedChinese => "选择文件以查看追溯",
            },
            (_, TrKey::FileHistoryTitle) => match self {
                Language::English => "File History",
                Language::SimplifiedChinese => "文件历史",
            },
            (_, TrKey::HistoryEmpty) => match self {
                Language::English => "Select a file to view history",
                Language::SimplifiedChinese => "选择文件以查看历史",
            },
            (_, TrKey::ReflogTitle) => match self {
                Language::English => "Reflog",
                Language::SimplifiedChinese => "引用日志",
            },
            (_, TrKey::ReflogEmpty) => match self {
                Language::English => "No reflog entries",
                Language::SimplifiedChinese => "暂无引用日志",
            },
            (_, TrKey::ReflogHint) => match self {
                Language::English => "Reflog tracks HEAD changes",
                Language::SimplifiedChinese => "引用日志记录 HEAD 的变化",
            },
            (_, TrKey::SubmodulesTitle) => match self {
                Language::English => "Submodules",
                Language::SimplifiedChinese => "子模块",
            },
            (_, TrKey::SubmodulesEmpty) => match self {
                Language::English => "No submodules found",
                Language::SimplifiedChinese => "未找到子模块",
            },
            (_, TrKey::SubmodulesHint) => match self {
                Language::English => "Add submodules to your repository",
                Language::SimplifiedChinese => "向仓库添加子模块",
            },
            (_, TrKey::CommitsCountFmt) => match self {
                Language::English => "{} commits",
                Language::SimplifiedChinese => "{} 个提交",
            },
            (_, TrKey::BhTab) => match self {
                Language::English => "Branch Health",
                Language::SimplifiedChinese => "分支健康",
            },
            (_, TrKey::BhUnmerged) => match self {
                Language::English => "Unmerged",
                Language::SimplifiedChinese => "未合并",
            },
            (_, TrKey::BhStale) => match self {
                Language::English => "Stale",
                Language::SimplifiedChinese => "长期未动",
            },
            (_, TrKey::BhDiverged) => match self {
                Language::English => "Diverged",
                Language::SimplifiedChinese => "已分叉",
            },
            (_, TrKey::BhTotal) => match self {
                Language::English => "Total",
                Language::SimplifiedChinese => "总数",
            },
            (_, TrKey::BhMerged) => match self {
                Language::English => "Merged",
                Language::SimplifiedChinese => "已合并",
            },
            (_, TrKey::AgeAny) => match self {
                Language::English => "Any time",
                Language::SimplifiedChinese => "不限时间",
            },
            (_, TrKey::AgeSevenDays) => match self {
                Language::English => "Last 7 days",
                Language::SimplifiedChinese => "近 7 天",
            },
            (_, TrKey::AgeThirtyDays) => match self {
                Language::English => "Last 30 days",
                Language::SimplifiedChinese => "近 30 天",
            },
            (_, TrKey::AgeNinetyDays) => match self {
                Language::English => "Last 90 days",
                Language::SimplifiedChinese => "近 90 天",
            },
            (_, TrKey::AgeOneYear) => match self {
                Language::English => "Last year",
                Language::SimplifiedChinese => "近一年",
            },
            (_, TrKey::TipCreatePrFull) => match self {
                Language::English => "Create GitHub pull request",
                Language::SimplifiedChinese => "创建 GitHub 拉取请求",
            },
            (_, TrKey::TipSearchCommits) => match self {
                Language::English => "Search commits",
                Language::SimplifiedChinese => "搜索提交",
            },
            (_, TrKey::TipRefresh) => match self {
                Language::English => "Refresh",
                Language::SimplifiedChinese => "刷新",
            },
            (_, TrKey::TipSettings) => match self {
                Language::English => "Settings",
                Language::SimplifiedChinese => "设置",
            },
            (_, TrKey::BlameEmptyHint) => match self {
                Language::English => "Press 'b' on a file to see line-by-line attribution",
                Language::SimplifiedChinese => "在文件上按 'b' 查看逐行归属",
            },
            (_, TrKey::HistoryEmptyHint) => match self {
                Language::English => "Press 'h' on a file to see commits",
                Language::SimplifiedChinese => "在文件上按 'h' 查看提交",
            },
            (_, TrKey::SearchResultsFmt) => match self {
                Language::English => "{} results",
                Language::SimplifiedChinese => "{} 个结果",
            },
            (_, TrKey::StagedOneFmt) => match self {
                Language::English => "{} file staged",
                Language::SimplifiedChinese => "已暂存 {} 个文件",
            },
            (_, TrKey::StagedManyFmt) => match self {
                Language::English => "{} files staged",
                Language::SimplifiedChinese => "已暂存 {} 个文件",
            },
            (_, TrKey::StagedNone) => match self {
                Language::English => "No files staged",
                Language::SimplifiedChinese => "没有暂存的文件",
            },
            (_, TrKey::DescCount11) => match self {
                Language::English => "{} char, {} line",
                Language::SimplifiedChinese => "{} 字符，{} 行",
            },
            (_, TrKey::DescCount1N) => match self {
                Language::English => "{} char, {} lines",
                Language::SimplifiedChinese => "{} 字符，{} 行",
            },
            (_, TrKey::DescCountN1) => match self {
                Language::English => "{} chars, {} line",
                Language::SimplifiedChinese => "{} 字符，{} 行",
            },
            (_, TrKey::DescCountNN) => match self {
                Language::English => "{} chars, {} lines",
                Language::SimplifiedChinese => "{} 字符，{} 行",
            },
            (_, TrKey::StatusStagedFmt) => match self {
                Language::English => "{} staged",
                Language::SimplifiedChinese => "已暂存 {}",
            },
            (_, TrKey::StatusChangedFmt) => match self {
                Language::English => "{} changed",
                Language::SimplifiedChinese => "已更改 {}",
            },
            (_, TrKey::SignedBadge) => match self {
                Language::English => "✓ Signed",
                Language::SimplifiedChinese => "✓ 已签名",
            },
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.native_name())
    }
}

impl FromStr for Language {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "en" | "english" => Ok(Language::English),
            "zh_cn" | "zh" | "chinese" | "simplified_chinese" | "简体中文" => {
                Ok(Language::SimplifiedChinese)
            }
            _ => Err(format!("Unknown language: {}", s)),
        }
    }
}

/// Keys for localizable UI text. One variant per localizable string;
/// rendering code passes these to [`Language::tr`] instead of hardcoding
/// English.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrKey {
    Preferences,
    SectionAppearance,
    SectionAi,
    SectionAuth,
    SectionGeneral,
    GeneralTitle,
    GeneralDesc,
    LanguageTitle,
    LanguageDesc,
    MaxRecentTitle,
    MaxRecentDesc,
    UiDensityTitle,
    UiDensityDesc,
    DensityCompact,
    DensityDefault,
    DensityComfortable,
    FontSizeTitle,
    FontSizeDesc,
    GeneralPageDesc,
    ThemeSectionDesc,
    ThemePageDesc,
    AppearanceModeTitle,
    AppearanceModeDesc,
    ModeAuto,
    ModeLight,
    ModeDark,
    ColorThemeTitle,
    ColorThemeDesc,
    ActiveBadge,
    CustomThemeTitle,
    CustomThemeDesc,
    EditThemeBtn,
    AccountsTitle,
    AccountsDesc,
    AccountsHeader,
    AccountsSectionDesc,
    QuickSetupTitle,
    QuickSetupDesc,
    SignInGitHub,
    AddGitLab,
    ManualCustom,
    AiPageDesc,
    AiSectionConnection,
    AiSectionBehaviour,
    AiStatusTitle,
    AiEnabledToggle,
    AiStatusOff,
    AiStatusTestingFmt,
    AiStatusRejectedFmt,
    AiStatusAddKeyFmt,
    AiConnectTitle,
    AiConnectDesc,
    AiApiKeyLabel,
    AiGetKeyBtn,
    AiKeyVerifiedFmt,
    AiKeyStoredHintFmt,
    AiNoKeyHintFmt,
    AiKeysStoredNoteFmt,
    AiModelLabel,
    AiModelCountFmt,
    AiRefreshBtn,
    AiRefreshingBtn,
    AiModelMissingFmt,
    AiUseSuggestionFmt,
    AiConnectBtn,
    AiUseProviderBtn,
    AiTestBtn,
    AiTestingBtn,
    AiRemoveKeyBtn,
    AiAdvancedLabel,
    AiBaseUrlLabel,
    AiBaseUrlOverriddenFmt,
    AiBaseUrlEmptyFmt,
    AiAttributionTitle,
    AiAttributionDesc,
    AiCommitStyleTitle,
    AiCommitStyleDesc,
    AiCtxTitle,
    AiCtxDesc,
    AiToolsTitle,
    AiToolsDesc,
    AiKeyPlaceholderFmt,
    ShowLineNumbersTitle,
    ShowLineNumbersDesc,
    WrapLinesTitle,
    WrapLinesDesc,
    DiffModeTitle,
    DiffModeDesc,
    DiffUnified,
    DiffSideBySide,
    GraphTitle,
    GraphDesc,
    GraphRails,
    GraphCurved,
    GraphAngular,
    ShowSubjectTitle,
    ShowSubjectDesc,
    AutoFetchTitle,
    AutoFetchDesc,
    ConfirmDestructiveTitle,
    ConfirmDestructiveDesc,
    CheckUpdatesTitle,
    CheckUpdatesDesc,
    WatchWorktreesTitle,
    WatchWorktreesDesc,
    ExternalToolsTitle,
    ExternalToolsDesc,
    TerminalLabel,
    EditorLabel,
    CustomCommandLabel,
    ShortcutsTitle,
    ShortcutsDesc,
    ConfigTitle,
    ConfigDesc,
    RevealBtn,
    EditKeymapBtn,
    SavedFeedback,
    TbFetch,
    TbFetching,
    TbPull,
    TbPulling,
    TbPush,
    TbPushing,
    TbBranch,
    TbStash,
    TbPop,
    TbCreatePr,
    StatusClean,
    StatusDetached,
    SideBranches,
    SideRemotes,
    SideRemoteBranches,
    SideTags,
    SideStashes,
    SideWorktrees,
    SideStaged,
    SideUnstaged,
    SideStageAll,
    SideUnstageAll,
    SideNewWorktree,
    SideWorkingClean,
    SideNoStaged,
    CommitBtn,
    CommitNoStaged,
    CommitNoMessage,
    CommitAmendBtn,
    AmendBtn,
    ClearBtn,
    SummaryLabel,
    DescLabel,
    CoAuthorsLabel,
    NoStagedHint,
    CancelBtn,
    ConfirmHint,
    CfConfirm,
    CfDiscard,
    CfClean,
    CfDeleteBranch,
    CfDeleteTag,
    CfRemove,
    CfDropStash,
    CfReset,
    CfAbort,
    CfForcePush,
    CfRemoveWorktree,
    DiffFileBtn,
    DiffEmptyTitle,
    DiffEmptyDesc,
    DiffOpenFile,
    DetailTitle,
    DetailEmptyTitle,
    DetailEmptyDesc,
    HomeSlogan,
    OpenRepoTitle,
    HomeNewWorkspace,
    HomeRestoreLast,
    HomeRecentWorkspaces,
    HomeRecentRepos,
    WsSummaryFmt,
    LoadingDots,
    OpRetry,
    ConflictOneFmt,
    ConflictManyFmt,
    ConflictResolved,
    ConflictContinue,
    WorktreeExit,
    InspectingPre,
    DetachedHeadWord,
    BottomDiff,
    BottomDiffTip,
    BottomHistory,
    HistLoading,
    HistShow,
    HistUnavailable,
    BottomBlame,
    BlameLoading,
    BlameShow,
    BlameUnavailable,
    GraphShowTip,
    GraphHideTip,
    DetachedTitlePre,
    DetachedConsequence,
    DetachedReturnPre,
    DetachedReturnTipPre,
    DetachedReturnTipPost,
    DetachedSwitch,
    DetachedShowGraph,
    UpdateMsgFmt,
    UpdateDownload,
    MenuCherryPick,
    MenuRevert,
    MenuCheckout,
    MenuCreateBranch,
    MenuCreateTag,
    MenuBisectGood,
    MenuBisectBad,
    MenuReset,
    RebaseTitle,
    MenuSquash,
    MenuCopySha,
    MenuCopyMsg,
    MenuCopyAuthor,
    MenuCopyDate,
    MenuViewGithub,
    GraphPanelTitle,
    GraphEmpty,
    GraphColHash,
    GraphColMessage,
    GraphColAuthor,
    GraphColDate,
    MyCommitsOn,
    MyCommitsOff,
    GripRebase,
    NoMatches,
    LoadMore,
    ShaShort,
    ShaFull,
    ShaCharsFmt,
    DisplaySettings,
    ShaLengthLabel,
    ShowSubjectCol,
    ShowAuthorCol,
    ShowAuthorEmail,
    ShowDateCol,
    AbsoluteDates,
    ShowAvatars,
    ShowLanes,
    ShowBadges,
    HeadDetachedBadge,
    PendingOnPre,
    PendingChanges,
    NoCommitsYet,
    NoCommitsSuffix,
    NewBranchBadge,
    SearchCommitsPh,
    PrsTitle,
    PrsTab,
    PrDetail,
    IssuesTitle,
    IssueDetail,
    FilterOpen,
    FilterClosed,
    FilterAll,
    PrNew,
    PrStateMerged,
    DraftBadge,
    LoadingComments,
    CommentsFailedPre,
    OneComment,
    ManyCommentsFmt,
    GhSignInPrs,
    GhSignInIssues,
    GhAuthDesc,
    TryFilterHint,
    PrEmptyFmt,
    IssueEmptyFmt,
    ReviewActions,
    ReviewApprove,
    ReviewRequest,
    ReviewCommentBtn,
    ReviewSubmitting,
    ReviewCommentPh,
    SearchIssuesTip,
    SearchIssuesPh,
    RepoPathLabel,
    BrowseBtn,
    CloneBtn,
    NoMatchRepos,
    NoRecentRepos,
    OpenBtn,
    EnterRepoPathPh,
    CloneTitle,
    CloneUrlLabel,
    ClonePathLabel,
    CloneUrlPh,
    ClonePathPh,
    CloneHint,
    CloningBtn,
    NameLabel,
    WtPathLabel,
    WtBranchLabel,
    CreateBtn,
    WtNamePh,
    WtPathPh,
    WtBranchPh,
    TagTitle,
    TagNameLabel,
    TagAtCommit,
    TagCreateHint,
    StashTitle,
    StashMsgLabel,
    StashMsgHint,
    RenameTitle,
    RenameCurrent,
    RenameNew,
    RenameHint,
    RenameBtn,
    BranchTitle,
    BranchNameLabel,
    BranchBasedOn,
    BranchNamePh,
    StashBranchTitle,
    StashFrom,
    CreatePrTitle,
    PrInto,
    PrTitleLabel,
    PrTitlePh,
    PrDescPh,
    PrDraftToggle,
    PrHint,
    PrSubmit,
    PrCreating,
    RebasePick,
    RebaseReword,
    RebaseSquash,
    RebaseFixup,
    RebaseDrop,
    RebaseHeaderFmt,
    RebasePh,
    RebaseDragTip,
    RebaseHintNav,
    RebaseHintReorder,
    RebaseHintAction,
    RebaseHintStart,
    RebaseHintCancel,
    RebaseStart,
    BisectTitle,
    BisectStarted,
    BisectBadFmt,
    BisectGoodFmt,
    BisectSkipFmt,
    BisectLeftFmt,
    BisectEntriesFmt,
    BisectEmpty,
    BisectEmptyHint,
    SplashSubtitle,
    SplashLoading,
    ShortcutsCloseHint,
    MoreActionsPre,
    ThemeTitle,
    ThemeColors,
    ThemeStatusColors,
    ThemeDark,
    ThemeLight,
    ThemeHint,
    SaveBtn,
    ThemeBackground,
    ThemeSurface,
    ThemeElevated,
    ThemeBorder,
    ThemeText,
    ThemeTextMuted,
    ThemeTextAccent,
    ThemePlaceholder,
    ThemeIcon,
    ThemeFocusRing,
    ThemeSelectedBorder,
    ThemeAdded,
    ThemeModified,
    ThemeDeleted,
    ThemeUntracked,
    ThemeConflict,
    ThemeRenamed,
    ThemeError,
    ThemeWarning,
    ThemeSuccess,
    ThemeInfo,
    TipOpenRepo,
    TipMerged,
    TipCopyBranch,
    TipCheckoutBranch,
    TipMergeBranch,
    TipRenameBranch,
    TipDeleteBranch,
    TipFetchRemote,
    TipPullRemote,
    TipPushRemote,
    TipRemoveRemote,
    TipCheckoutTag,
    TipDeleteTag,
    ApplyStashLbl,
    PopStashLbl,
    TipStashBranch,
    TipDropStash,
    TipRemoveWorktree,
    TipUnstageFile,
    TipStageFile,
    TipConflictResolver,
    TipDiscardChanges,
    TipFilterLocal,
    TipFilterRemote,
    TipStagedOpts,
    TipUnstagedOpts,
    TipCopySha,
    TipGpg,
    TipCherryPick,
    TipClearSearch,
    FlatViewTip,
    TreeViewTip,
    NoFilesTip,
    ClearSearchTip,
    CommitSummaryPh,
    CommitDescPh,
    CoAuthorNamePh,
    CoAuthorEmailPh,
    TipExpandCommit,
    TipCollapseCommit,
    TipExplorer,
    TipTerminal,
    TipEditor,
    TitleDetachedTag,
    TipApplyKeep,
    TipPopRemove,
    StashesEmpty,
    StashesEmptyHint,
    SearchPh,
    SearchEmpty,
    SearchHint,
    FilterFilesPh,
    FilterBranchesPh,
    FilterRemotePh,
    SetHttpsTokenPh,
    SetGpgPh,
    SetUsuallyEmpty,
    SetAccountTokenPh,
    SetCustomCmdPh,
    TipPaste,
    TipHide,
    TipShow,
    CopiedFb,
    FindingBranches,
    ContainedIn,
    SearchFilesHint,
    SortLastUsed,
    OnlyMine,
    PopBranchFilters,
    PopRemoteFilters,
    PopStagedFiles,
    PopUnstagedFiles,
    ShowLineCounts,
    ChangeListHint,
    NoRemotes,
    NoTags,
    NoWorktrees,
    CurrentTag,
    StashMenuCreateBranch,
    BlameEmpty,
    FileHistoryTitle,
    HistoryEmpty,
    ReflogTitle,
    ReflogEmpty,
    ReflogHint,
    SubmodulesTitle,
    SubmodulesEmpty,
    SubmodulesHint,
    CommitsCountFmt,
    BhTab,
    BhUnmerged,
    BhStale,
    BhDiverged,
    BhTotal,
    BhMerged,
    AgeAny,
    AgeSevenDays,
    AgeThirtyDays,
    AgeNinetyDays,
    AgeOneYear,
    TipCreatePrFull,
    TipSearchCommits,
    TipRefresh,
    TipSettings,
    BlameEmptyHint,
    HistoryEmptyHint,
    SearchResultsFmt,
    StagedOneFmt,
    StagedManyFmt,
    StagedNone,
    DescCount11,
    DescCount1N,
    DescCountN1,
    DescCountNN,
    StatusStagedFmt,
    StatusChangedFmt,
    SignedBadge,
}

/// Application settings persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default = "default_settings_version")]
    pub version: u32,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub ui_font: String,
    #[serde(default)]
    pub ai: AiSettings,
    #[serde(default)]
    pub git: GitSettings,
    #[serde(default)]
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
    #[serde(default = "default_appearance_mode")]
    pub appearance_mode: AppearanceMode,
    #[serde(default)]
    pub language: Language,
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
    pub diff_wrap_lines: bool,
    #[serde(default)]
    pub graph_style: GraphStyle,
    #[serde(default = "default_show_subject_column")]
    pub show_subject_column: bool,
    #[serde(default = "default_author_column_width")]
    pub author_column_width: f32,
    #[serde(default = "default_date_column_width")]
    pub date_column_width: f32,
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
    #[serde(default = "default_commit_limit")]
    pub commit_limit: usize,
    #[serde(default)]
    pub watch_all_worktrees: bool,
    /// Show each staged and unstaged file's added and removed line counts in
    /// the sidebar. Off by default: counting them diffs every changed file on
    /// each status refresh.
    #[serde(default)]
    pub show_change_line_stats: bool,
    /// Last known position/size of the standalone Settings window. Restored
    /// on next open if the saved origin still falls within a connected display.
    #[serde(default)]
    pub settings_window_bounds: Option<SavedWindowBounds>,
}

/// Serializable rectangle in screen coordinates. Used to persist window
/// geometry across sessions independently of GPUI's `Bounds<Pixels>`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SavedWindowBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Current settings version. Increment when making breaking changes.
const CURRENT_SETTINGS_VERSION: u32 = 4;

fn default_settings_version() -> u32 {
    CURRENT_SETTINGS_VERSION
}

fn default_theme() -> String {
    "Catppuccin Mocha".into()
}

fn default_appearance_mode() -> AppearanceMode {
    AppearanceMode::Auto
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

fn default_author_column_width() -> f32 {
    140.0
}

fn default_date_column_width() -> f32 {
    100.0
}

fn default_confirm_destructive() -> bool {
    true
}

fn default_auto_check_updates() -> bool {
    true
}

fn default_commit_limit() -> usize {
    1000
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
    /// Whether the commit graph is hidden, handing its space to the bottom
    /// panel. Files written before the option existed load with it off.
    #[serde(default)]
    pub graph_hidden: bool,
}

fn default_sidebar_width() -> f32 {
    276.0
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
            graph_hidden: false,
        }
    }
}

fn default_max_recent() -> usize {
    20
}

/// The AI providers rgitui can talk to.
///
/// Persisted by its lowercase id, and the single source of truth for endpoint
/// shape, auth style and default model. Dispatching on a bare string is what
/// let a hand-edited `settings.json` reach the network layer before failing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum AiProvider {
    #[default]
    Gemini,
    #[serde(rename = "openai")]
    OpenAi,
    Anthropic,
    #[serde(rename = "deepseek")]
    DeepSeek,
    #[serde(rename = "openrouter")]
    OpenRouter,
}

impl AiProvider {
    /// Every provider, in the order the settings UI lists them.
    pub const ALL: &'static [AiProvider] = &[
        AiProvider::Gemini,
        AiProvider::OpenAi,
        AiProvider::Anthropic,
        AiProvider::DeepSeek,
        AiProvider::OpenRouter,
    ];

    /// The stable id used in `settings.json`, keychain accounts and catalogue
    /// cache filenames. Changing one needs a migration.
    pub fn id(self) -> &'static str {
        match self {
            AiProvider::Gemini => "gemini",
            AiProvider::OpenAi => "openai",
            AiProvider::Anthropic => "anthropic",
            AiProvider::DeepSeek => "deepseek",
            AiProvider::OpenRouter => "openrouter",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            AiProvider::Gemini => "Google Gemini",
            AiProvider::OpenAi => "OpenAI",
            AiProvider::Anthropic => "Anthropic",
            AiProvider::DeepSeek => "DeepSeek",
            AiProvider::OpenRouter => "OpenRouter",
        }
    }

    /// Parse a persisted id. Case- and whitespace-insensitive, so a hand-edited
    /// `"Anthropic"` resolves instead of silently falling back.
    pub fn from_id(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        Self::ALL
            .iter()
            .copied()
            .find(|provider| provider.id() == normalized)
    }

    /// The GA model a fresh install (or a provider switch with no remembered
    /// choice) uses: cheap, fast and tool-capable, because commit-message
    /// generation does not need a frontier model.
    pub fn default_model(self) -> &'static str {
        match self {
            AiProvider::Gemini => "gemini-3.1-flash-lite",
            AiProvider::OpenAi => "gpt-5.6-luna",
            AiProvider::Anthropic => "claude-haiku-4-5",
            AiProvider::DeepSeek => "deepseek-v4-flash",
            AiProvider::OpenRouter => "google/gemini-3.1-flash-lite",
        }
    }

    /// Where the user creates an API key for this provider.
    pub fn key_url(self) -> &'static str {
        match self {
            AiProvider::Gemini => "https://aistudio.google.com/apikey",
            AiProvider::OpenAi => "https://platform.openai.com/api-keys",
            AiProvider::Anthropic => "https://console.anthropic.com/settings/keys",
            AiProvider::DeepSeek => "https://platform.deepseek.com/api_keys",
            AiProvider::OpenRouter => "https://openrouter.ai/keys",
        }
    }

    /// The host requests reach by default. Shown when warning about a
    /// `base_url_override` so the user sees what they are replacing.
    pub fn default_host(self) -> &'static str {
        match self {
            AiProvider::Gemini => "generativelanguage.googleapis.com",
            AiProvider::OpenAi => "api.openai.com",
            AiProvider::Anthropic => "api.anthropic.com",
            AiProvider::DeepSeek => "api.deepseek.com",
            AiProvider::OpenRouter => "openrouter.ai",
        }
    }

    /// Whether this provider speaks the OpenAI `/chat/completions` shape.
    /// Only these honour `base_url_override`.
    pub fn is_openai_compatible(self) -> bool {
        matches!(
            self,
            AiProvider::OpenAi | AiProvider::DeepSeek | AiProvider::OpenRouter
        )
    }
}

impl fmt::Display for AiProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

/// Deserialize a provider id leniently: an unknown value falls back to the
/// default rather than failing the whole settings file. [`init`] surfaces the
/// unknown value through `load_warnings` so the fallback is never silent.
fn deserialize_ai_provider<'de, D>(deserializer: D) -> std::result::Result<AiProvider, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    Ok(AiProvider::from_id(&raw).unwrap_or_default())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiSettings {
    #[serde(
        default = "default_ai_provider",
        deserialize_with = "deserialize_ai_provider"
    )]
    pub provider: AiProvider,
    #[serde(rename = "api_key", default, skip_serializing)]
    pub legacy_api_key: Option<String>,
    /// Whether the *active* provider holds a key. Derived from
    /// `has_api_key_for`; retained so existing readers keep working.
    #[serde(default)]
    pub has_api_key: bool,
    /// Which providers hold a key in the OS keychain. Flags only — never the
    /// secret itself.
    #[serde(default)]
    pub has_api_key_for: BTreeMap<String, bool>,
    #[serde(default = "default_ai_model")]
    pub model: String,
    /// Per-provider model pin, keyed by provider id. Preserves the user's
    /// choice per provider instead of resetting it on every provider switch.
    #[serde(default)]
    pub models_by_provider: BTreeMap<String, String>,
    /// Override endpoint for OpenAI-compatible providers (LiteLLM, Ollama's
    /// `/v1`, self-hosted gateways). Empty means use the built-in URL — the
    /// default is deliberately not stored here so it cannot freeze at whatever
    /// shipped.
    #[serde(default)]
    pub base_url_override: String,
    /// Send `HTTP-Referer`/`X-Title` to OpenRouter for leaderboard attribution.
    #[serde(default = "default_openrouter_attribution")]
    pub openrouter_attribution: bool,
    #[serde(default = "default_commit_style")]
    pub commit_style: String,
    #[serde(default = "default_ai_enabled")]
    pub enabled: bool,
    #[serde(default = "default_inject_project_context")]
    pub inject_project_context: bool,
    #[serde(default = "default_use_tools")]
    pub use_tools: bool,
}

impl AiSettings {
    /// The model pinned for `provider`, falling back to that provider's GA
    /// default. Never returns another provider's model.
    pub fn model_for(&self, provider: AiProvider) -> String {
        if provider == self.provider && !self.model.trim().is_empty() {
            return self.model.clone();
        }
        self.models_by_provider
            .get(provider.id())
            .filter(|model| !model.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| provider.default_model().to_string())
    }

    /// Whether `provider` has a key in the keychain, according to the persisted
    /// flags. Does not touch the keychain.
    pub fn has_key_for(&self, provider: AiProvider) -> bool {
        self.has_api_key_for
            .get(provider.id())
            .copied()
            .unwrap_or(false)
    }

    /// Record whether `provider` holds a key, keeping the active-provider
    /// mirror `has_api_key` in step.
    pub fn set_has_key_for(&mut self, provider: AiProvider, has_key: bool) {
        self.has_api_key_for
            .insert(provider.id().to_string(), has_key);
        if provider == self.provider {
            self.has_api_key = has_key;
        }
    }

    /// Point the active provider at `provider`, remembering the model the
    /// previous provider used so switching back restores it.
    pub fn set_active_provider(&mut self, provider: AiProvider) {
        let previous = self.provider;
        if !self.model.trim().is_empty() {
            self.models_by_provider
                .insert(previous.id().to_string(), self.model.clone());
        }
        // Resolve the incoming model *before* reassigning `provider`, so
        // `model_for`'s active-provider shortcut cannot hand back the model the
        // previous provider was using.
        let next_model = self
            .models_by_provider
            .get(provider.id())
            .filter(|model| !model.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| provider.default_model().to_string());
        self.provider = provider;
        self.model = next_model;
        self.has_api_key = self.has_key_for(provider);
    }

    /// Pin `model` for the active provider.
    pub fn set_active_model(&mut self, model: impl Into<String>) {
        let model = model.into();
        self.models_by_provider
            .insert(self.provider.id().to_string(), model.clone());
        self.model = model;
    }
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

fn default_openrouter_attribution() -> bool {
    true
}

fn default_ai_provider() -> AiProvider {
    AiProvider::Gemini
}

fn default_ai_model() -> String {
    default_ai_provider().default_model().into()
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
    /// Resolved AI keys, keyed by provider id. One slot per provider so a
    /// provider switch cannot transmit the previous provider's credential.
    pub ai_api_keys: BTreeMap<String, String>,
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
            has_api_key_for: BTreeMap::new(),
            model: default_ai_model(),
            models_by_provider: BTreeMap::new(),
            base_url_override: String::new(),
            openrouter_attribution: default_openrouter_attribution(),
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
            theme: default_theme(),
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
            appearance_mode: default_appearance_mode(),
            language: Language::default(),
            terminal_command: String::new(),
            editor_command: String::new(),
            font_size: default_font_size(),
            show_line_numbers_in_diff: default_show_line_numbers_in_diff(),
            diff_view_mode: DiffViewMode::default(),
            diff_wrap_lines: false,
            graph_style: GraphStyle::default(),
            show_subject_column: default_show_subject_column(),
            author_column_width: default_author_column_width(),
            date_column_width: default_date_column_width(),
            auto_fetch_interval: AutoFetchInterval::default(),
            confirm_destructive_operations: default_confirm_destructive(),
            auto_check_updates: default_auto_check_updates(),
            last_update_check_at: None,
            commit_limit: default_commit_limit(),
            watch_all_worktrees: false,
            show_change_line_stats: false,
            settings_window_bounds: None,
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

    /// The most recently persisted bounds for the standalone Settings window,
    /// or `None` if the user has never moved/resized it.
    pub fn settings_window_bounds(&self) -> Option<SavedWindowBounds> {
        self.settings.settings_window_bounds
    }

    /// Update the persisted Settings window bounds. Caller is responsible for
    /// invoking [`SettingsState::save`] afterwards if write-through is desired.
    pub fn set_settings_window_bounds(&mut self, bounds: Option<SavedWindowBounds>) {
        self.settings.settings_window_bounds = bounds;
    }

    /// Persist the current settings to disk.
    ///
    /// The JSON is serialized synchronously on the calling thread so the write
    /// reflects the state at the moment of the call, then handed to a single
    /// dedicated writer thread. Because that thread drains its queue in
    /// FIFO order, the on-disk file always reflects the most recent `save`,
    /// eliminating the race where two concurrent writers could land in the
    /// wrong order.
    pub fn save(&self) -> Result<()> {
        sync_auth_runtime(&self.settings);
        sync_change_line_stats(&self.settings);
        let json = serde_json::to_string_pretty(&self.settings)?;
        enqueue_write(WriteRequest {
            config_path: self.config_path.clone(),
            json,
            ack: None,
        });
        Ok(())
    }

    /// Persist the current settings to disk and block until the write has
    /// completed (or failed). Use this on shutdown paths so the final state is
    /// guaranteed to reach disk before the process exits, rather than racing a
    /// detached writer thread that may be killed mid-write.
    pub fn save_blocking(&self) -> Result<()> {
        sync_auth_runtime(&self.settings);
        let json = serde_json::to_string_pretty(&self.settings)?;
        let (ack_tx, ack_rx) = sync_channel::<()>(1);
        enqueue_write(WriteRequest {
            config_path: self.config_path.clone(),
            json,
            ack: Some(ack_tx),
        });
        // The writer always sends the ack, even on write failure, so this only
        // blocks until the queued write (and every write enqueued before it)
        // has drained. A dropped sender (writer thread gone) also unblocks us.
        let _ = ack_rx.recv();
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
        let _ = self.save_blocking();
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

        // Migration 1 -> 2: Add graph column width settings
        if self.settings.version == 1 {
            self.settings.author_column_width = default_author_column_width();
            self.settings.date_column_width = default_date_column_width();
            self.settings.version = 2;
            migrated = true;
            log::info!("Migrated settings from version 1 to 2");
        }

        // Migration 2 -> 3: AI keys became per-provider, and the shipped
        // default model was a retired id absent from the picker. Remap the
        // known-dead ids to their successors rather than leaving a pin that
        // renders as no selection at all and 404s when used.
        if self.settings.version == 2 {
            if let Some(successor) = retired_model_successor(&self.settings.ai.model) {
                log::info!(
                    "Remapping retired AI model '{}' to '{}'",
                    self.settings.ai.model,
                    successor
                );
                self.settings.ai.model = successor.to_string();
            }
            self.settings.version = 3;
            migrated = true;
            log::info!("Migrated settings from version 2 to 3");
        }

        // Migration 3 -> 4: interface language added. Existing installs keep
        // English; serde defaults already cover the missing field.
        if self.settings.version == 3 {
            self.settings.language = Language::default();
            self.settings.version = 4;
            migrated = true;
            log::info!("Migrated settings from version 3 to 4");
        }

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

    /// The API key for the active AI provider.
    pub fn ai_api_key(&self) -> Option<String> {
        self.ai_api_key_for(self.settings.ai.provider)
    }

    /// The API key stored for a specific provider, materialized only for the
    /// caller that asked for it.
    pub fn ai_api_key_for(&self, provider: AiProvider) -> Option<String> {
        with_auth_runtime(|runtime| runtime.ai_api_keys.get(provider.id()).cloned())
    }

    /// Whether the active provider has a key, without cloning any secret.
    ///
    /// Render paths must use this rather than `ai_api_key().is_some()`, which
    /// deep-clones every credential the app holds on every frame.
    pub fn has_ai_api_key(&self) -> bool {
        self.settings.ai.has_key_for(self.settings.ai.provider)
    }

    /// Whether `provider` has a key, without cloning any secret.
    pub fn has_ai_api_key_for(&self, provider: AiProvider) -> bool {
        self.settings.ai.has_key_for(provider)
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

    /// Store (or clear) the API key for the active provider.
    pub fn set_ai_api_key(&mut self, value: Option<&str>) -> Result<()> {
        self.set_ai_api_key_for(self.settings.ai.provider, value)
    }

    /// Store (or clear) the API key for a specific provider.
    ///
    /// The keychain write happens *before* any settings mutation, so a failed
    /// write leaves the recorded flags and the resolved runtime key agreeing
    /// with what is actually in the keychain.
    pub fn set_ai_api_key_for(&mut self, provider: AiProvider, value: Option<&str>) -> Result<()> {
        let has_key = write_secret(&ai_provider_account(provider.id()), value)?;
        self.settings.ai.legacy_api_key = None;
        self.settings.ai.set_has_key_for(provider, has_key);
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
        // Write the secret unconditionally, keyed by provider id. A provider that
        // was just added in the settings window is not yet present in
        // `self.settings.git.providers` (it only arrives via
        // `replace_git_providers`), so guarding the keyring write on its presence
        // here would silently drop the token on first save.
        let has_token = write_secret(&git_provider_account(provider_id), value)?;
        if let Some(provider) = self
            .settings
            .git
            .providers
            .iter_mut()
            .find(|provider| provider.id == provider_id)
        {
            provider.legacy_token = None;
            provider.has_token = has_token;
        }
        sync_auth_runtime(&self.settings);
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

        // Promote the single `ai/default` secret into the slot for whichever
        // provider was active when it was written. `ai/default` is left in
        // place so a downgrade still finds its key; a later save of that
        // provider overwrites the new slot. Idempotent: the promotion is
        // skipped once the per-provider slot exists.
        if self.settings.ai.has_api_key {
            let active = self.settings.ai.provider;
            let account = ai_provider_account(active.id());
            if read_secret(&account).is_none() {
                if let Some(key) = read_secret(AI_SECRET_ACCOUNT) {
                    if write_secret(&account, Some(&key))? {
                        self.settings.ai.set_has_key_for(active, true);
                        migrated = true;
                        log::info!(
                            "Promoted the shared AI key into the '{}' provider slot",
                            active.id()
                        );
                    }
                }
            }
        }

        // Re-derive the per-provider flags from what the keychain actually
        // holds. A flag that says "connected" for a provider with no key is
        // exactly the false-connected state the per-provider split fixes.
        for provider in AiProvider::ALL {
            let present = read_secret(&ai_provider_account(provider.id())).is_some();
            if self.settings.ai.has_key_for(*provider) != present {
                self.settings.ai.set_has_key_for(*provider, present);
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

/// Where settings, the keymap, themes and cached avatars live.
pub fn config_dir() -> PathBuf {
    state_root().config.clone()
}

/// Where regenerable caches live — history, and anything else that is an
/// optimisation rather than user data.
pub fn cache_dir() -> PathBuf {
    state_root().cache.clone()
}

/// The two directories rgitui persists to, resolved once per process.
struct StateRoot {
    config: PathBuf,
    cache: PathBuf,
}

static STATE_ROOT: OnceLock<StateRoot> = OnceLock::new();

fn state_root() -> &'static StateRoot {
    STATE_ROOT.get_or_init(|| StateRoot {
        config: dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rgitui"),
        cache: dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rgitui"),
    })
}

/// Points config and cache at `root` instead of the user's real directories.
///
/// This exists for measurement. A benchmark that reads and writes the same
/// settings file and history cache as the installed app is measuring whatever
/// the last run happened to leave behind, and leaves its own corpus in the
/// user's recent-repository list on the way out. Redirecting both to a scratch
/// directory is what makes a run start from a known state and end without a
/// trace of itself.
///
/// Call before anything reads either path — first read wins, and this returns
/// `false` if it lost that race rather than pretending to have taken effect.
#[must_use]
pub fn redirect_state_to(root: &Path) -> bool {
    STATE_ROOT
        .set(StateRoot {
            config: root.join("config"),
            cache: root.join("cache"),
        })
        .is_ok()
}

/// File name of the settings file inside [`config_dir`].
const SETTINGS_FILE_NAME: &str = "settings.json";

/// File name of the user keymap inside [`config_dir`].
const KEYMAP_FILE_NAME: &str = "keymap.json";

/// Path of the settings file.
pub fn settings_path() -> PathBuf {
    config_dir().join(SETTINGS_FILE_NAME)
}

/// Path of the user keymap, alongside the settings file.
pub fn keymap_path() -> PathBuf {
    config_dir().join(KEYMAP_FILE_NAME)
}

/// A single queued settings write. `json` is pre-serialized on the caller's
/// thread so the snapshot reflects state at call time; `ack`, when present,
/// is fired after the write attempt completes so a blocking caller can wait.
struct WriteRequest {
    config_path: PathBuf,
    json: String,
    ack: Option<SyncSender<()>>,
}

/// Sender for the dedicated settings writer thread. Wrapped in a `Mutex`
/// because the `Sync`-ness of `Sender` is not guaranteed across toolchains and
/// the sender is shared through a process-wide `OnceLock`.
fn write_sender() -> &'static Mutex<Sender<WriteRequest>> {
    static WRITE_SENDER: OnceLock<Mutex<Sender<WriteRequest>>> = OnceLock::new();
    WRITE_SENDER.get_or_init(|| {
        let (tx, rx) = channel::<WriteRequest>();
        std::thread::Builder::new()
            .name("rgitui-settings-writer".to_string())
            .spawn(move || {
                // A single writer drains the queue in FIFO order, so on-disk
                // state always matches the order saves were requested.
                while let Ok(request) = rx.recv() {
                    write_settings_file(&request.config_path, &request.json);
                    // Fire the ack on every path (including write failure) so a
                    // `save_blocking` caller never deadlocks on a failed write.
                    if let Some(ack) = request.ack {
                        let _ = ack.send(());
                    }
                }
            })
            .expect("failed to spawn settings writer thread");
        Mutex::new(tx)
    })
}

fn enqueue_write(request: WriteRequest) {
    let sender = write_sender()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if sender.send(request).is_err() {
        log::error!("Settings writer thread is no longer running; write dropped");
    }
}

/// Perform the actual atomic write of serialized settings JSON: write to a
/// temp file then rename over the target, falling back to a direct write if the
/// rename fails.
fn write_settings_file(config_path: &Path, json: &str) {
    if let Some(parent) = config_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log::error!("Failed to create settings directory: {}", e);
            return;
        }
    }
    let tmp_path = config_path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp_path, json) {
        log::error!("Failed to write settings temp file: {}", e);
        return;
    }
    if let Err(e) = std::fs::rename(&tmp_path, config_path) {
        log::error!("Failed to rename settings file: {}", e);
        if let Err(e2) = std::fs::write(config_path, json) {
            log::error!("Fallback write also failed: {}", e2);
        }
        let _ = std::fs::remove_file(&tmp_path);
    }
}

/// Initialize settings. Must be called during app init.
pub fn init(cx: &mut App) {
    let config_path = settings_path();
    let mut load_warnings = Vec::new();
    let settings = if config_path.exists() {
        match std::fs::read_to_string(&config_path) {
            Ok(json) => match serde_json::from_str::<AppSettings>(&json) {
                Ok(settings) => {
                    // The provider deserializer falls back rather than failing
                    // the whole file, so re-read the raw value to tell the user
                    // which id was not understood. Without this the settings UI
                    // would simply show a different provider selected than the
                    // one in their file.
                    if let Some(raw) = raw_ai_provider(&json) {
                        if AiProvider::from_id(&raw).is_none() {
                            let msg = format!(
                                "Unknown AI provider \"{}\" in settings.json; using {}. Valid values: {}.",
                                raw,
                                settings.ai.provider.id(),
                                AiProvider::ALL
                                    .iter()
                                    .map(|provider| provider.id())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            );
                            log::warn!("{}", msg);
                            load_warnings.push(msg);
                        }
                    }
                    settings
                }
                Err(e) => {
                    // Preserve the unparseable file so the user can recover any
                    // hand-edited content instead of silently overwriting it with
                    // defaults. The timestamp suffix avoids `:` because that is an
                    // illegal filename character on Windows.
                    let stamp = Utc::now().format("%Y%m%d%H%M%S");
                    let backup_name = format!(
                        "{}.corrupt-{}",
                        config_path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "settings.json".to_string()),
                        stamp
                    );
                    let backup_path = config_path.with_file_name(backup_name);
                    let msg = match std::fs::rename(&config_path, &backup_path) {
                        Ok(()) => format!(
                            "Settings file could not be parsed and was preserved at {}. Defaults are in use: {}",
                            backup_path.display(),
                            e
                        ),
                        Err(rename_err) => format!(
                            "Settings file could not be parsed (using defaults). Failed to back up the original ({}): {}",
                            rename_err, e
                        ),
                    };
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
    state.migrate_settings();

    // Run legacy data migrations
    state.migrate_legacy_workspace_data();
    if let Err(error) = state.migrate_legacy_secrets() {
        log::warn!("Failed to migrate secrets into keychain: {}", error);
    }

    // Resolve secrets from the keyring into the auth runtime on every startup —
    // not only when migration failed. (`save()` below also syncs, but doing it
    // here keeps the runtime correct even if the save is skipped or fails.)
    sync_auth_runtime(state.settings());
    sync_change_line_stats(state.settings());

    if let Err(error) = state.save() {
        log::warn!("Failed to persist migrated settings: {}", error);
    }
    cx.set_global(state);
}

/// The raw `ai.provider` string as it appears on disk, before the lenient
/// deserializer has had a chance to substitute a fallback.
fn raw_ai_provider(json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()?
        .get("ai")?
        .get("provider")?
        .as_str()
        .map(str::to_string)
}

/// Install default settings for a test app.
///
/// Any view that reads `cx.global::<SettingsState>()` in `render` needs this
/// before a headless test window draws it. Unlike [`init`], nothing is read
/// from or written to the user's config directory and the OS keychain is left
/// alone, so tests cannot disturb (or be disturbed by) real user state. The
/// config path points into the temp directory purely so a stray `save()` in a
/// view under test cannot land on the real file.
pub fn init_test(cx: &mut App) {
    cx.set_global(SettingsState {
        settings: AppSettings::default(),
        config_path: std::env::temp_dir().join("rgitui-test-settings.json"),
        load_warnings: Vec::new(),
    });
}

/// Borrow the resolved credentials under the lock and return only what the
/// caller needs.
///
/// Prefer this over [`current_auth_runtime`], which deep-clones every secret
/// the app holds — including ones the caller has no use for — into fresh heap
/// allocations that are dropped without zeroization.
pub fn with_auth_runtime<R>(f: impl FnOnce(&AuthRuntimeState) -> R) -> R {
    let guard = auth_runtime().read().expect(
        "git auth runtime RwLock poisoned - a previous thread panicked while holding the lock",
    );
    f(&guard)
}

pub fn current_auth_runtime() -> AuthRuntimeState {
    with_auth_runtime(|runtime| runtime.clone())
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
/// The pre-v3 single AI key slot. Retained so the v2 -> v3 migration can read
/// it and so a downgrade still finds a key; new writes never target it.
const AI_SECRET_ACCOUNT: &str = "ai/default";
const GIT_DEFAULT_HTTPS_ACCOUNT: &str = "git/default-https";

fn git_provider_account(provider_id: &str) -> String {
    format!("git/provider/{}", provider_id)
}

fn ai_provider_account(provider_id: &str) -> String {
    format!("ai/provider/{}", provider_id)
}

/// Model ids that are retired or fabricated, mapped to the successor a user
/// pinned to them should land on. Applied once, by the v2 -> v3 migration.
fn retired_model_successor(model: &str) -> Option<&'static str> {
    match model.trim() {
        // Retired 2026-06-01, and the shipped default that appeared in no
        // picker, so a fresh install rendered the Model row with nothing
        // selected.
        "gemini-2.0-flash" | "gemini-1.5-flash" | "gemini-1.5-pro" => Some("gemini-3.1-flash-lite"),
        // `20241022` is the Claude 3.5 snapshot date on a 4.5 name: a
        // guaranteed 404 that was offered in the picker.
        "claude-sonnet-4-5-20241022" => Some("claude-haiku-4-5"),
        // o-series rejects `max_tokens` and a non-default `temperature`, and
        // both are being retired.
        "o3" | "o4-mini" | "o1" | "o1-mini" => Some("gpt-5.6-luna"),
        _ => None,
    }
}

/// Mirror of [`AppSettings::show_change_line_stats`] for background threads,
/// which gather working-tree status without access to the `SettingsState`
/// global.
static CHANGE_LINE_STATS: AtomicBool = AtomicBool::new(false);

/// Whether working-tree refreshes should count added and removed lines per
/// file. Updated whenever settings are loaded or saved.
pub fn change_line_stats_enabled() -> bool {
    CHANGE_LINE_STATS.load(Ordering::Relaxed)
}

fn sync_change_line_stats(settings: &AppSettings) {
    CHANGE_LINE_STATS.store(settings.show_change_line_stats, Ordering::Relaxed);
}

fn auth_runtime() -> &'static RwLock<AuthRuntimeState> {
    static AUTH_RUNTIME: OnceLock<RwLock<AuthRuntimeState>> = OnceLock::new();
    AUTH_RUNTIME.get_or_init(|| RwLock::new(AuthRuntimeState::default()))
}

fn sync_auth_runtime(settings: &AppSettings) {
    let runtime = AuthRuntimeState {
        ai_api_keys: resolve_ai_api_keys(&settings.ai),
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

/// Resolve every provider's key from the keychain in one pass.
///
/// Mirrors the git-provider loop: one read per provider that claims a key, and
/// none for the rest, so adding a provider does not multiply the cost of an
/// unrelated save.
fn resolve_ai_api_keys(settings: &AiSettings) -> BTreeMap<String, String> {
    let mut keys = BTreeMap::new();
    for provider in AiProvider::ALL {
        if !settings.has_key_for(*provider) {
            continue;
        }
        if let Some(secret) = read_secret(&ai_provider_account(provider.id())) {
            keys.insert(provider.id().to_string(), secret);
        }
    }

    // Pre-v3 files, and any install whose migration could not write to the
    // keychain, still resolve through the shared slot for the active provider.
    if !keys.contains_key(settings.provider.id()) {
        let legacy = if settings.has_api_key {
            read_secret(AI_SECRET_ACCOUNT).or_else(|| settings.legacy_api_key.clone())
        } else {
            settings.legacy_api_key.clone()
        };
        if let Some(secret) = legacy {
            keys.insert(settings.provider.id().to_string(), secret);
        }
    }

    keys
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

    // ── Layout ────────────────────────────────────────────────────

    /// A layout saved before the graph could be hidden must keep showing it.
    #[test]
    fn layout_without_graph_hidden_shows_the_graph() {
        let layout: LayoutSettings = serde_json::from_str(
            r#"{"sidebar_width":300.0,"detail_panel_width":400.0,"diff_viewer_height":500.0,"commit_input_height":350.0}"#,
        )
        .expect("legacy layout parses");
        assert!(!layout.graph_hidden);
        assert_eq!(layout.diff_viewer_height, 500.0);
    }

    #[test]
    fn graph_hidden_round_trips() {
        let layout = LayoutSettings {
            graph_hidden: true,
            ..LayoutSettings::default()
        };
        let json = serde_json::to_string(&layout).expect("layout serializes");
        let restored: LayoutSettings = serde_json::from_str(&json).expect("layout parses");
        assert_eq!(restored, layout);
    }

    // ── AI provider catalogue coherence ───────────────────────────

    /// The bug this guards is not hypothetical: the shipped default was
    /// `gemini-2.0-flash`, which appeared in no picker, so a fresh install
    /// rendered the Model row with nothing selected at all.
    #[test]
    fn default_model_is_the_default_provider_model() {
        assert_eq!(default_ai_model(), default_ai_provider().default_model());
    }

    #[test]
    fn every_provider_has_a_distinct_id_and_a_default_model() {
        let mut ids: Vec<&str> = AiProvider::ALL.iter().map(|p| p.id()).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "provider ids must be unique");

        for provider in AiProvider::ALL {
            assert!(!provider.default_model().is_empty());
            assert!(provider.key_url().starts_with("https://"));
            assert!(!provider.display_name().is_empty());
            assert_eq!(AiProvider::from_id(provider.id()), Some(*provider));
        }
    }

    #[test]
    fn no_default_model_is_a_retired_id() {
        for provider in AiProvider::ALL {
            assert_eq!(
                retired_model_successor(provider.default_model()),
                None,
                "{} ships a retired default",
                provider.id()
            );
        }
    }

    #[test]
    fn provider_id_parsing_is_lenient_about_case_and_whitespace() {
        assert_eq!(
            AiProvider::from_id("Anthropic"),
            Some(AiProvider::Anthropic)
        );
        assert_eq!(AiProvider::from_id("  OpenAI "), Some(AiProvider::OpenAi));
        assert_eq!(
            AiProvider::from_id("openrouter"),
            Some(AiProvider::OpenRouter)
        );
        assert_eq!(AiProvider::from_id("bard"), None);
    }

    #[test]
    fn only_openai_compatible_providers_accept_a_base_url_override() {
        assert!(AiProvider::OpenAi.is_openai_compatible());
        assert!(AiProvider::DeepSeek.is_openai_compatible());
        assert!(AiProvider::OpenRouter.is_openai_compatible());
        assert!(!AiProvider::Gemini.is_openai_compatible());
        assert!(!AiProvider::Anthropic.is_openai_compatible());
    }

    // ── AiSettings model memory ───────────────────────────────────

    #[test]
    fn switching_provider_preserves_each_providers_model_choice() {
        let mut ai = AiSettings::default();
        ai.set_active_model("gemini-3.1-pro-preview");

        ai.set_active_provider(AiProvider::OpenAi);
        assert_eq!(ai.model, AiProvider::OpenAi.default_model());
        ai.set_active_model("gpt-5.4");

        ai.set_active_provider(AiProvider::Gemini);
        assert_eq!(ai.model, "gemini-3.1-pro-preview");

        ai.set_active_provider(AiProvider::OpenAi);
        assert_eq!(ai.model, "gpt-5.4");
    }

    #[test]
    fn model_for_never_returns_another_providers_model() {
        let mut ai = AiSettings::default();
        ai.set_active_model("gemini-3.1-pro-preview");
        assert_eq!(
            ai.model_for(AiProvider::Anthropic),
            AiProvider::Anthropic.default_model()
        );
    }

    #[test]
    fn key_flags_are_tracked_per_provider() {
        let mut ai = AiSettings::default();
        ai.set_has_key_for(AiProvider::Gemini, true);
        assert!(ai.has_key_for(AiProvider::Gemini));
        assert!(ai.has_api_key, "the active provider mirror must follow");
        assert!(!ai.has_key_for(AiProvider::Anthropic));

        // Switching to a provider with no key must not keep asserting
        // "connected" — that false state is what enabled the AI button for a
        // provider the app had no credential for.
        ai.set_active_provider(AiProvider::Anthropic);
        assert!(!ai.has_api_key);
    }

    // ── settings file compatibility ───────────────────────────────

    #[test]
    fn v2_settings_load_with_every_new_field_defaulted() {
        let json = r#"{
            "version": 2,
            "ai": { "provider": "openai", "model": "gpt-5.4", "has_api_key": true }
        }"#;
        let settings: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.ai.provider, AiProvider::OpenAi);
        assert_eq!(settings.ai.model, "gpt-5.4");
        assert!(settings.ai.has_api_key_for.is_empty());
        assert!(settings.ai.models_by_provider.is_empty());
        assert!(settings.ai.base_url_override.is_empty());
        assert!(settings.ai.openrouter_attribution);
    }

    #[test]
    fn an_unknown_provider_falls_back_instead_of_failing_the_file() {
        let json = r#"{ "version": 3, "ai": { "provider": "bard" } }"#;
        let settings: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.ai.provider, AiProvider::default());
        // And the raw value is still recoverable, which is what lets `init`
        // tell the user which id it did not understand.
        assert_eq!(raw_ai_provider(json).as_deref(), Some("bard"));
    }

    #[test]
    fn a_miscased_provider_resolves_rather_than_falling_back() {
        let json = r#"{ "ai": { "provider": "Anthropic" } }"#;
        let settings: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.ai.provider, AiProvider::Anthropic);
    }

    #[test]
    fn the_api_key_never_reaches_the_settings_file() {
        let mut settings = AppSettings::default();
        settings.ai.legacy_api_key = Some("sk-should-never-be-written".into());
        let json = serde_json::to_string(&settings).unwrap();
        assert!(!json.contains("sk-should-never-be-written"));
    }

    #[test]
    fn v2_to_v3_remaps_retired_model_ids() {
        let mut state = test_settings_state();
        state.settings.version = 2;
        state.settings.ai.model = "gemini-2.0-flash".into();

        assert!(state.migrate_settings());

        assert_eq!(state.settings.version, CURRENT_SETTINGS_VERSION);
        assert_eq!(state.settings.ai.model, "gemini-3.1-flash-lite");
    }

    #[test]
    fn v2_to_v3_leaves_a_live_model_alone() {
        let mut state = test_settings_state();
        state.settings.version = 2;
        state.settings.ai.model = "gemini-2.5-pro".into();

        state.migrate_settings();

        assert_eq!(state.settings.ai.model, "gemini-2.5-pro");
    }

    // ── interface language ──────────────────────────────────────

    #[test]
    fn language_ids_are_stable_and_unique() {
        assert_eq!(Language::English.id(), "en");
        assert_eq!(Language::SimplifiedChinese.id(), "zh-CN");
        assert_eq!(Language::ALL.len(), 2);
        assert_eq!("en".parse::<Language>().unwrap(), Language::English);
        assert_eq!(
            "zh-CN".parse::<Language>().unwrap(),
            Language::SimplifiedChinese
        );
        assert_eq!(
            "zh_cn".parse::<Language>().unwrap(),
            Language::SimplifiedChinese
        );
        assert_eq!(
            "简体中文".parse::<Language>().unwrap(),
            Language::SimplifiedChinese
        );
        assert!("fr".parse::<Language>().is_err());
    }

    #[test]
    fn language_dropdown_labels_stay_native() {
        assert_eq!(Language::English.native_name(), "English");
        assert_eq!(Language::SimplifiedChinese.native_name(), "简体中文");
    }

    #[test]
    fn every_translated_key_is_non_empty_in_both_languages() {
        let keys = [
            TrKey::Preferences,
            TrKey::SectionAppearance,
            TrKey::SectionAi,
            TrKey::SectionAuth,
            TrKey::SectionGeneral,
            TrKey::GeneralTitle,
            TrKey::GeneralDesc,
            TrKey::LanguageTitle,
            TrKey::LanguageDesc,
            TrKey::MaxRecentTitle,
            TrKey::MaxRecentDesc,
            TrKey::UiDensityTitle,
            TrKey::UiDensityDesc,
            TrKey::DensityCompact,
            TrKey::DensityDefault,
            TrKey::DensityComfortable,
            TrKey::FontSizeTitle,
            TrKey::FontSizeDesc,
            TrKey::GeneralPageDesc,
            TrKey::ThemeSectionDesc,
            TrKey::ThemePageDesc,
            TrKey::AppearanceModeTitle,
            TrKey::AppearanceModeDesc,
            TrKey::ModeAuto,
            TrKey::ModeLight,
            TrKey::ModeDark,
            TrKey::ColorThemeTitle,
            TrKey::ColorThemeDesc,
            TrKey::ActiveBadge,
            TrKey::CustomThemeTitle,
            TrKey::CustomThemeDesc,
            TrKey::EditThemeBtn,
            TrKey::AccountsTitle,
            TrKey::AccountsDesc,
            TrKey::AccountsHeader,
            TrKey::AccountsSectionDesc,
            TrKey::QuickSetupTitle,
            TrKey::QuickSetupDesc,
            TrKey::SignInGitHub,
            TrKey::AddGitLab,
            TrKey::ManualCustom,
            TrKey::AiPageDesc,
            TrKey::AiSectionConnection,
            TrKey::AiSectionBehaviour,
            TrKey::AiStatusTitle,
            TrKey::AiEnabledToggle,
            TrKey::AiStatusOff,
            TrKey::AiStatusTestingFmt,
            TrKey::AiStatusRejectedFmt,
            TrKey::AiStatusAddKeyFmt,
            TrKey::AiConnectTitle,
            TrKey::AiConnectDesc,
            TrKey::AiApiKeyLabel,
            TrKey::AiGetKeyBtn,
            TrKey::AiKeyVerifiedFmt,
            TrKey::AiKeyStoredHintFmt,
            TrKey::AiNoKeyHintFmt,
            TrKey::AiKeysStoredNoteFmt,
            TrKey::AiModelLabel,
            TrKey::AiModelCountFmt,
            TrKey::AiRefreshBtn,
            TrKey::AiRefreshingBtn,
            TrKey::AiModelMissingFmt,
            TrKey::AiUseSuggestionFmt,
            TrKey::AiConnectBtn,
            TrKey::AiUseProviderBtn,
            TrKey::AiTestBtn,
            TrKey::AiTestingBtn,
            TrKey::AiRemoveKeyBtn,
            TrKey::AiAdvancedLabel,
            TrKey::AiBaseUrlLabel,
            TrKey::AiBaseUrlOverriddenFmt,
            TrKey::AiBaseUrlEmptyFmt,
            TrKey::AiAttributionTitle,
            TrKey::AiAttributionDesc,
            TrKey::AiCommitStyleTitle,
            TrKey::AiCommitStyleDesc,
            TrKey::AiCtxTitle,
            TrKey::AiCtxDesc,
            TrKey::AiToolsTitle,
            TrKey::AiToolsDesc,
            TrKey::AiKeyPlaceholderFmt,
            TrKey::ShowLineNumbersTitle,
            TrKey::ShowLineNumbersDesc,
            TrKey::WrapLinesTitle,
            TrKey::WrapLinesDesc,
            TrKey::DiffModeTitle,
            TrKey::DiffModeDesc,
            TrKey::DiffUnified,
            TrKey::DiffSideBySide,
            TrKey::GraphTitle,
            TrKey::GraphDesc,
            TrKey::GraphRails,
            TrKey::GraphCurved,
            TrKey::GraphAngular,
            TrKey::ShowSubjectTitle,
            TrKey::ShowSubjectDesc,
            TrKey::AutoFetchTitle,
            TrKey::AutoFetchDesc,
            TrKey::ConfirmDestructiveTitle,
            TrKey::ConfirmDestructiveDesc,
            TrKey::CheckUpdatesTitle,
            TrKey::CheckUpdatesDesc,
            TrKey::WatchWorktreesTitle,
            TrKey::WatchWorktreesDesc,
            TrKey::ExternalToolsTitle,
            TrKey::ExternalToolsDesc,
            TrKey::TerminalLabel,
            TrKey::EditorLabel,
            TrKey::CustomCommandLabel,
            TrKey::ShortcutsTitle,
            TrKey::ShortcutsDesc,
            TrKey::ConfigTitle,
            TrKey::ConfigDesc,
            TrKey::RevealBtn,
            TrKey::EditKeymapBtn,
            TrKey::SavedFeedback,
            TrKey::TbFetch,
            TrKey::TbFetching,
            TrKey::TbPull,
            TrKey::TbPulling,
            TrKey::TbPush,
            TrKey::TbPushing,
            TrKey::TbBranch,
            TrKey::TbStash,
            TrKey::TbPop,
            TrKey::TbCreatePr,
            TrKey::StatusClean,
            TrKey::StatusDetached,
            TrKey::SideBranches,
            TrKey::SideRemotes,
            TrKey::SideRemoteBranches,
            TrKey::SideTags,
            TrKey::SideStashes,
            TrKey::SideWorktrees,
            TrKey::SideStaged,
            TrKey::SideUnstaged,
            TrKey::SideStageAll,
            TrKey::SideUnstageAll,
            TrKey::SideNewWorktree,
            TrKey::SideWorkingClean,
            TrKey::SideNoStaged,
            TrKey::CommitBtn,
            TrKey::CommitNoStaged,
            TrKey::CommitNoMessage,
            TrKey::CommitAmendBtn,
            TrKey::AmendBtn,
            TrKey::ClearBtn,
            TrKey::SummaryLabel,
            TrKey::DescLabel,
            TrKey::CoAuthorsLabel,
            TrKey::NoStagedHint,
            TrKey::CancelBtn,
            TrKey::ConfirmHint,
            TrKey::CfConfirm,
            TrKey::CfDiscard,
            TrKey::CfClean,
            TrKey::CfDeleteBranch,
            TrKey::CfDeleteTag,
            TrKey::CfRemove,
            TrKey::CfDropStash,
            TrKey::CfReset,
            TrKey::CfAbort,
            TrKey::CfForcePush,
            TrKey::CfRemoveWorktree,
            TrKey::DiffFileBtn,
            TrKey::DiffEmptyTitle,
            TrKey::DiffEmptyDesc,
            TrKey::DiffOpenFile,
            TrKey::DetailTitle,
            TrKey::DetailEmptyTitle,
            TrKey::DetailEmptyDesc,
            TrKey::HomeSlogan,
            TrKey::OpenRepoTitle,
            TrKey::HomeNewWorkspace,
            TrKey::HomeRestoreLast,
            TrKey::HomeRecentWorkspaces,
            TrKey::HomeRecentRepos,
            TrKey::WsSummaryFmt,
            TrKey::LoadingDots,
            TrKey::OpRetry,
            TrKey::ConflictOneFmt,
            TrKey::ConflictManyFmt,
            TrKey::ConflictResolved,
            TrKey::ConflictContinue,
            TrKey::WorktreeExit,
            TrKey::InspectingPre,
            TrKey::DetachedHeadWord,
            TrKey::BottomDiff,
            TrKey::BottomDiffTip,
            TrKey::BottomHistory,
            TrKey::HistLoading,
            TrKey::HistShow,
            TrKey::HistUnavailable,
            TrKey::BottomBlame,
            TrKey::BlameLoading,
            TrKey::BlameShow,
            TrKey::BlameUnavailable,
            TrKey::GraphShowTip,
            TrKey::GraphHideTip,
            TrKey::DetachedTitlePre,
            TrKey::DetachedConsequence,
            TrKey::DetachedReturnPre,
            TrKey::DetachedReturnTipPre,
            TrKey::DetachedReturnTipPost,
            TrKey::DetachedSwitch,
            TrKey::DetachedShowGraph,
            TrKey::UpdateMsgFmt,
            TrKey::UpdateDownload,
            TrKey::MenuCherryPick,
            TrKey::MenuRevert,
            TrKey::MenuCheckout,
            TrKey::MenuCreateBranch,
            TrKey::MenuCreateTag,
            TrKey::MenuBisectGood,
            TrKey::MenuBisectBad,
            TrKey::MenuReset,
            TrKey::RebaseTitle,
            TrKey::MenuSquash,
            TrKey::MenuCopySha,
            TrKey::MenuCopyMsg,
            TrKey::MenuCopyAuthor,
            TrKey::MenuCopyDate,
            TrKey::MenuViewGithub,
            TrKey::GraphPanelTitle,
            TrKey::GraphEmpty,
            TrKey::GraphColHash,
            TrKey::GraphColMessage,
            TrKey::GraphColAuthor,
            TrKey::GraphColDate,
            TrKey::MyCommitsOn,
            TrKey::MyCommitsOff,
            TrKey::GripRebase,
            TrKey::NoMatches,
            TrKey::LoadMore,
            TrKey::ShaShort,
            TrKey::ShaFull,
            TrKey::ShaCharsFmt,
            TrKey::DisplaySettings,
            TrKey::ShaLengthLabel,
            TrKey::ShowSubjectCol,
            TrKey::ShowAuthorCol,
            TrKey::ShowAuthorEmail,
            TrKey::ShowDateCol,
            TrKey::AbsoluteDates,
            TrKey::ShowAvatars,
            TrKey::ShowLanes,
            TrKey::ShowBadges,
            TrKey::HeadDetachedBadge,
            TrKey::PendingOnPre,
            TrKey::PendingChanges,
            TrKey::NoCommitsYet,
            TrKey::NoCommitsSuffix,
            TrKey::NewBranchBadge,
            TrKey::SearchCommitsPh,
            TrKey::PrsTitle,
            TrKey::PrsTab,
            TrKey::PrDetail,
            TrKey::IssuesTitle,
            TrKey::IssueDetail,
            TrKey::FilterOpen,
            TrKey::FilterClosed,
            TrKey::FilterAll,
            TrKey::PrNew,
            TrKey::PrStateMerged,
            TrKey::DraftBadge,
            TrKey::LoadingComments,
            TrKey::CommentsFailedPre,
            TrKey::OneComment,
            TrKey::ManyCommentsFmt,
            TrKey::GhSignInPrs,
            TrKey::GhSignInIssues,
            TrKey::GhAuthDesc,
            TrKey::TryFilterHint,
            TrKey::PrEmptyFmt,
            TrKey::IssueEmptyFmt,
            TrKey::ReviewActions,
            TrKey::ReviewApprove,
            TrKey::ReviewRequest,
            TrKey::ReviewCommentBtn,
            TrKey::ReviewSubmitting,
            TrKey::ReviewCommentPh,
            TrKey::SearchIssuesTip,
            TrKey::SearchIssuesPh,
            TrKey::RepoPathLabel,
            TrKey::BrowseBtn,
            TrKey::CloneBtn,
            TrKey::NoMatchRepos,
            TrKey::NoRecentRepos,
            TrKey::OpenBtn,
            TrKey::EnterRepoPathPh,
            TrKey::CloneTitle,
            TrKey::CloneUrlLabel,
            TrKey::ClonePathLabel,
            TrKey::CloneUrlPh,
            TrKey::ClonePathPh,
            TrKey::CloneHint,
            TrKey::CloningBtn,
            TrKey::NameLabel,
            TrKey::WtPathLabel,
            TrKey::WtBranchLabel,
            TrKey::CreateBtn,
            TrKey::WtNamePh,
            TrKey::WtPathPh,
            TrKey::WtBranchPh,
            TrKey::TagTitle,
            TrKey::TagNameLabel,
            TrKey::TagAtCommit,
            TrKey::TagCreateHint,
            TrKey::StashTitle,
            TrKey::StashMsgLabel,
            TrKey::StashMsgHint,
            TrKey::RenameTitle,
            TrKey::RenameCurrent,
            TrKey::RenameNew,
            TrKey::RenameHint,
            TrKey::RenameBtn,
            TrKey::BranchTitle,
            TrKey::BranchNameLabel,
            TrKey::BranchBasedOn,
            TrKey::BranchNamePh,
            TrKey::StashBranchTitle,
            TrKey::StashFrom,
            TrKey::CreatePrTitle,
            TrKey::PrInto,
            TrKey::PrTitleLabel,
            TrKey::PrTitlePh,
            TrKey::PrDescPh,
            TrKey::PrDraftToggle,
            TrKey::PrHint,
            TrKey::PrSubmit,
            TrKey::PrCreating,
            TrKey::RebasePick,
            TrKey::RebaseReword,
            TrKey::RebaseSquash,
            TrKey::RebaseFixup,
            TrKey::RebaseDrop,
            TrKey::RebaseHeaderFmt,
            TrKey::RebasePh,
            TrKey::RebaseDragTip,
            TrKey::RebaseHintNav,
            TrKey::RebaseHintReorder,
            TrKey::RebaseHintAction,
            TrKey::RebaseHintStart,
            TrKey::RebaseHintCancel,
            TrKey::RebaseStart,
            TrKey::BisectTitle,
            TrKey::BisectStarted,
            TrKey::BisectBadFmt,
            TrKey::BisectGoodFmt,
            TrKey::BisectSkipFmt,
            TrKey::BisectLeftFmt,
            TrKey::BisectEntriesFmt,
            TrKey::BisectEmpty,
            TrKey::BisectEmptyHint,
            TrKey::SplashSubtitle,
            TrKey::SplashLoading,
            TrKey::ShortcutsCloseHint,
            TrKey::MoreActionsPre,
            TrKey::ThemeTitle,
            TrKey::ThemeColors,
            TrKey::ThemeStatusColors,
            TrKey::ThemeDark,
            TrKey::ThemeLight,
            TrKey::ThemeHint,
            TrKey::SaveBtn,
            TrKey::ThemeBackground,
            TrKey::ThemeSurface,
            TrKey::ThemeElevated,
            TrKey::ThemeBorder,
            TrKey::ThemeText,
            TrKey::ThemeTextMuted,
            TrKey::ThemeTextAccent,
            TrKey::ThemePlaceholder,
            TrKey::ThemeIcon,
            TrKey::ThemeFocusRing,
            TrKey::ThemeSelectedBorder,
            TrKey::ThemeAdded,
            TrKey::ThemeModified,
            TrKey::ThemeDeleted,
            TrKey::ThemeUntracked,
            TrKey::ThemeConflict,
            TrKey::ThemeRenamed,
            TrKey::ThemeError,
            TrKey::ThemeWarning,
            TrKey::ThemeSuccess,
            TrKey::ThemeInfo,
            TrKey::TipOpenRepo,
            TrKey::TipMerged,
            TrKey::TipCopyBranch,
            TrKey::TipCheckoutBranch,
            TrKey::TipMergeBranch,
            TrKey::TipRenameBranch,
            TrKey::TipDeleteBranch,
            TrKey::TipFetchRemote,
            TrKey::TipPullRemote,
            TrKey::TipPushRemote,
            TrKey::TipRemoveRemote,
            TrKey::TipCheckoutTag,
            TrKey::TipDeleteTag,
            TrKey::ApplyStashLbl,
            TrKey::PopStashLbl,
            TrKey::TipStashBranch,
            TrKey::TipDropStash,
            TrKey::TipRemoveWorktree,
            TrKey::TipUnstageFile,
            TrKey::TipStageFile,
            TrKey::TipConflictResolver,
            TrKey::TipDiscardChanges,
            TrKey::TipFilterLocal,
            TrKey::TipFilterRemote,
            TrKey::TipStagedOpts,
            TrKey::TipUnstagedOpts,
            TrKey::TipCopySha,
            TrKey::TipGpg,
            TrKey::TipCherryPick,
            TrKey::TipClearSearch,
            TrKey::FlatViewTip,
            TrKey::TreeViewTip,
            TrKey::NoFilesTip,
            TrKey::ClearSearchTip,
            TrKey::CommitSummaryPh,
            TrKey::CommitDescPh,
            TrKey::CoAuthorNamePh,
            TrKey::CoAuthorEmailPh,
            TrKey::TipExpandCommit,
            TrKey::TipCollapseCommit,
            TrKey::TipExplorer,
            TrKey::TipTerminal,
            TrKey::TipEditor,
            TrKey::TitleDetachedTag,
            TrKey::TipApplyKeep,
            TrKey::TipPopRemove,
            TrKey::StashesEmpty,
            TrKey::StashesEmptyHint,
            TrKey::SearchPh,
            TrKey::SearchEmpty,
            TrKey::SearchHint,
            TrKey::FilterFilesPh,
            TrKey::FilterBranchesPh,
            TrKey::FilterRemotePh,
            TrKey::SetHttpsTokenPh,
            TrKey::SetGpgPh,
            TrKey::SetUsuallyEmpty,
            TrKey::SetAccountTokenPh,
            TrKey::SetCustomCmdPh,
            TrKey::TipPaste,
            TrKey::TipHide,
            TrKey::TipShow,
            TrKey::CopiedFb,
            TrKey::FindingBranches,
            TrKey::ContainedIn,
            TrKey::SearchFilesHint,
            TrKey::SortLastUsed,
            TrKey::OnlyMine,
            TrKey::PopBranchFilters,
            TrKey::PopRemoteFilters,
            TrKey::PopStagedFiles,
            TrKey::PopUnstagedFiles,
            TrKey::ShowLineCounts,
            TrKey::ChangeListHint,
            TrKey::NoRemotes,
            TrKey::NoTags,
            TrKey::NoWorktrees,
            TrKey::CurrentTag,
            TrKey::StashMenuCreateBranch,
            TrKey::BlameEmpty,
            TrKey::FileHistoryTitle,
            TrKey::HistoryEmpty,
            TrKey::ReflogTitle,
            TrKey::ReflogEmpty,
            TrKey::ReflogHint,
            TrKey::SubmodulesTitle,
            TrKey::SubmodulesEmpty,
            TrKey::SubmodulesHint,
            TrKey::CommitsCountFmt,
            TrKey::BhTab,
            TrKey::BhUnmerged,
            TrKey::BhStale,
            TrKey::BhDiverged,
            TrKey::BhTotal,
            TrKey::BhMerged,
            TrKey::AgeAny,
            TrKey::AgeSevenDays,
            TrKey::AgeThirtyDays,
            TrKey::AgeNinetyDays,
            TrKey::AgeOneYear,
            TrKey::TipCreatePrFull,
            TrKey::TipSearchCommits,
            TrKey::TipRefresh,
            TrKey::TipSettings,
            TrKey::BlameEmptyHint,
            TrKey::HistoryEmptyHint,
            TrKey::SearchResultsFmt,
            TrKey::StagedOneFmt,
            TrKey::StagedManyFmt,
            TrKey::StagedNone,
            TrKey::DescCount11,
            TrKey::DescCount1N,
            TrKey::DescCountN1,
            TrKey::DescCountNN,
            TrKey::StatusStagedFmt,
            TrKey::StatusChangedFmt,
            TrKey::SignedBadge,
        ];
        for key in keys {
            assert!(!Language::English.tr(key).is_empty());
            assert!(!Language::SimplifiedChinese.tr(key).is_empty());
        }
        assert_eq!(Language::SimplifiedChinese.tr(TrKey::SectionAi), "AI");
    }

    #[test]
    fn english_branches_keep_the_exact_source_literals() {
        // Rendering code previously hardcoded these strings; the English
        // branch must preserve them byte for byte.
        assert_eq!(Language::English.tr(TrKey::DiffUnified), "Unified");
        assert_eq!(Language::English.tr(TrKey::DiffSideBySide), "Side-by-Side");
        assert_eq!(Language::English.tr(TrKey::GraphRails), "Rails");
        assert_eq!(Language::English.tr(TrKey::GraphCurved), "Curved");
        assert_eq!(Language::English.tr(TrKey::GraphAngular), "Angular");
        assert_eq!(Language::English.tr(TrKey::ModeAuto), "Auto");
        assert_eq!(Language::English.tr(TrKey::ModeLight), "Light");
        assert_eq!(Language::English.tr(TrKey::ModeDark), "Dark");
        assert_eq!(Language::English.tr(TrKey::StatusDetached), "DETACHED");
        assert_eq!(
            Language::English.tr(TrKey::ConfirmHint),
            "Enter to confirm | Esc to cancel"
        );
        assert_eq!(Language::English.tr(TrKey::CfForcePush), "Force Push");
        assert_eq!(Language::English.tr(TrKey::RevealBtn), "Reveal");
        assert_eq!(
            Language::English.tr(TrKey::EditKeymapBtn),
            "Edit keymap.json"
        );
        assert_eq!(
            Language::English.tr(TrKey::HomeSlogan),
            "A workspace-oriented desktop Git client"
        );
        assert_eq!(
            Language::English.tr(TrKey::ConflictResolved),
            "All conflicts resolved -- ready to continue"
        );
        assert_eq!(
            Language::English.tr(TrKey::DetachedConsequence),
            "Commits made here belong to no branch"
        );
        assert_eq!(
            Language::English.tr(TrKey::MenuCherryPick),
            "Cherry-pick commit"
        );
        assert_eq!(
            Language::English.tr(TrKey::MyCommitsOff),
            "Show only your commits. Click to filter by current user."
        );
        assert_eq!(
            Language::English.tr(TrKey::DisplaySettings),
            "Display Settings"
        );
        assert_eq!(Language::English.tr(TrKey::LoadMore), "Load more commits");
        assert_eq!(Language::English.tr(TrKey::PrDetail), "Pull Request Detail");
        assert_eq!(Language::English.tr(TrKey::GhAuthDesc), "This repository is private or rate-limited. Add a GitHub token in Settings — for organization repos you may need a fine-grained token approved by an org owner.");
        assert_eq!(
            Language::English.tr(TrKey::ReviewRequest),
            "Request Changes"
        );
        assert_eq!(
            Language::English.tr(TrKey::TagCreateHint),
            "Enter to create | Esc to cancel"
        );
        assert_eq!(
            Language::English.tr(TrKey::PrHint),
            "Enter for a new line | Shift+Enter to create | Esc to cancel"
        );
        assert_eq!(
            Language::English.tr(TrKey::RebaseHintReorder),
            "Ctrl+Up/Down Reorder"
        );
        assert_eq!(
            Language::English.tr(TrKey::BisectEmptyHint),
            "Use 'Git: Bisect Start' to begin"
        );
        assert_eq!(
            Language::English.tr(TrKey::ThemeHint),
            "Tab to move · Enter to save · Esc to close"
        );
        assert_eq!(
            Language::English.tr(TrKey::SearchHint),
            "Press Enter to search | j/k to navigate"
        );
        assert_eq!(
            Language::English.tr(TrKey::CloneUrlPh),
            "Repository URL (e.g. https://github.com/user/repo.git)"
        );
        assert_eq!(
            Language::English.tr(TrKey::AiStatusTitle),
            "AI Commit Messages"
        );
        assert_eq!(Language::English.tr(TrKey::AiEnabledToggle), "Enabled");
        assert_eq!(
            Language::English.tr(TrKey::AiStatusOff),
            "AI is turned off. Nothing will be sent to any provider."
        );
        assert_eq!(
            Language::English.tr(TrKey::AiStatusTestingFmt),
            "Testing {}…"
        );
        assert_eq!(
            Language::English.tr(TrKey::AiStatusRejectedFmt),
            "{} rejected this key."
        );
        assert_eq!(
            Language::English.tr(TrKey::AiStatusAddKeyFmt),
            "Add a {} API key to get started."
        );
        assert_eq!(
            Language::English.tr(TrKey::AiConnectTitle),
            "Connect an AI provider"
        );
        assert_eq!(
            Language::English.tr(TrKey::AiConnectDesc),
            "rgitui writes commit messages from your staged diff."
        );
        assert_eq!(Language::English.tr(TrKey::AiApiKeyLabel), "API key");
        assert_eq!(Language::English.tr(TrKey::AiGetKeyBtn), "Get a key");
        assert_eq!(
            Language::English.tr(TrKey::AiKeyVerifiedFmt),
            "Verified {} · stored in {}"
        );
        assert_eq!(
            Language::English.tr(TrKey::AiKeyStoredHintFmt),
            "Key stored in {}. Test it to confirm it works."
        );
        assert_eq!(
            Language::English.tr(TrKey::AiNoKeyHintFmt),
            "No API key needed — requests go to {}."
        );
        assert_eq!(
            Language::English.tr(TrKey::AiKeysStoredNoteFmt),
            "Keys are stored in {}, never in settings.json, and are only read when a request is sent."
        );
        assert_eq!(Language::English.tr(TrKey::AiModelLabel), "Model");
        assert_eq!(
            Language::English.tr(TrKey::AiModelCountFmt),
            "{} models · {}"
        );
        assert_eq!(Language::English.tr(TrKey::AiRefreshBtn), "Refresh");
        assert_eq!(Language::English.tr(TrKey::AiRefreshingBtn), "Refreshing…");
        assert_eq!(
            Language::English.tr(TrKey::AiModelMissingFmt),
            "`{}` is not in {}'s current model list. It may have been retired."
        );
        assert_eq!(Language::English.tr(TrKey::AiUseSuggestionFmt), "Use {}");
        assert_eq!(Language::English.tr(TrKey::AiConnectBtn), "Connect");
        assert_eq!(
            Language::English.tr(TrKey::AiUseProviderBtn),
            "Use this provider"
        );
        assert_eq!(Language::English.tr(TrKey::AiTestBtn), "Test");
        assert_eq!(Language::English.tr(TrKey::AiTestingBtn), "Testing…");
        assert_eq!(Language::English.tr(TrKey::AiRemoveKeyBtn), "Remove key");
        assert_eq!(Language::English.tr(TrKey::AiAdvancedLabel), "Advanced");
        assert_eq!(Language::English.tr(TrKey::AiBaseUrlLabel), "Base URL");
        assert_eq!(
            Language::English.tr(TrKey::AiBaseUrlOverriddenFmt),
            "Requests go to {} instead of {}. Your API key is sent to that host."
        );
        assert_eq!(
            Language::English.tr(TrKey::AiBaseUrlEmptyFmt),
            "Empty means use {}. Only OpenAI-compatible providers honour an override."
        );
        assert_eq!(
            Language::English.tr(TrKey::AiAttributionTitle),
            "Send attribution headers"
        );
        assert_eq!(
            Language::English.tr(TrKey::AiAttributionDesc),
            "Adds HTTP-Referer and X-Title so rgitui appears on OpenRouter's public leaderboard. Never functional."
        );
        assert_eq!(
            Language::English.tr(TrKey::AiCommitStyleTitle),
            "Commit style"
        );
        assert_eq!(
            Language::English.tr(TrKey::AiCommitStyleDesc),
            "How the AI should format commit messages."
        );
        assert_eq!(
            Language::English.tr(TrKey::AiCtxTitle),
            "Include project context"
        );
        assert_eq!(
            Language::English.tr(TrKey::AiCtxDesc),
            "Adds README.md, CLAUDE.md and AGENTS.md to the prompt. ~4k extra tokens per request."
        );
        assert_eq!(
            Language::English.tr(TrKey::AiToolsTitle),
            "Let the model read files"
        );
        assert_eq!(
            Language::English.tr(TrKey::AiToolsDesc),
            "The model may request file contents and commit history. Slower and more expensive; usually a better message."
        );
        assert_eq!(
            Language::English.tr(TrKey::AiKeyPlaceholderFmt),
            "Paste your {} API key"
        );
    }

    #[test]
    fn v3_settings_default_to_english_and_migrate_to_v4() {
        let settings: AppSettings =
            serde_json::from_str(r#"{ "version": 3 }"#).expect("v3 file parses");
        assert_eq!(settings.language, Language::English);

        let mut state = test_settings_state();
        state.settings.version = 3;
        assert!(state.migrate_settings());
        assert_eq!(state.settings.version, CURRENT_SETTINGS_VERSION);
        assert_eq!(state.settings.language, Language::English);
    }

    #[test]
    fn retired_ids_map_to_a_successor_of_the_same_provider_family() {
        assert_eq!(
            retired_model_successor("claude-sonnet-4-5-20241022"),
            Some("claude-haiku-4-5")
        );
        assert_eq!(retired_model_successor("o4-mini"), Some("gpt-5.6-luna"));
        assert_eq!(retired_model_successor("deepseek-v4-flash"), None);
    }

    #[test]
    fn ai_provider_accounts_are_distinct_and_namespaced() {
        let accounts: Vec<String> = AiProvider::ALL
            .iter()
            .map(|p| ai_provider_account(p.id()))
            .collect();
        assert_eq!(accounts[0], "ai/provider/gemini");
        for account in &accounts {
            assert!(account.starts_with("ai/provider/"));
            assert_ne!(account, AI_SECRET_ACCOUNT);
            assert!(!account.starts_with("git/"));
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
