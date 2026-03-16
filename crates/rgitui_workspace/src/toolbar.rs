use gpui::prelude::*;
use gpui::SharedString;
use gpui::{div, px, ClickEvent, Context, EventEmitter, Render, Window};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Badge, Icon, IconName, IconSize, Label, LabelSize, Tooltip, VerticalDivider};

struct ToolbarButtonState {
    disabled: bool,
    loading: bool,
}

/// Events emitted by the toolbar.
#[derive(Debug, Clone)]
pub enum ToolbarEvent {
    Fetch,
    Pull,
    Push,
    Branch,
    StashSave,
    StashPop,
    Undo,
    Redo,
    Refresh,
    Settings,
    Search,
}

/// The main toolbar with quick action buttons.
pub struct Toolbar {
    can_push: bool,
    can_pull: bool,
    has_stashes: bool,
    has_changes: bool,
    is_fetching: bool,
    is_pulling: bool,
    is_pushing: bool,
    ahead: usize,
    behind: usize,
}

impl EventEmitter<ToolbarEvent> for Toolbar {}

impl Default for Toolbar {
    fn default() -> Self {
        Self::new()
    }
}

impl Toolbar {
    pub fn new() -> Self {
        Self {
            can_push: true,
            can_pull: true,
            has_stashes: false,
            has_changes: false,
            is_fetching: false,
            is_pulling: false,
            is_pushing: false,
            ahead: 0,
            behind: 0,
        }
    }

    pub fn set_state(
        &mut self,
        can_push: bool,
        can_pull: bool,
        has_stashes: bool,
        has_changes: bool,
        cx: &mut Context<Self>,
    ) {
        self.can_push = can_push;
        self.can_pull = can_pull;
        self.has_stashes = has_stashes;
        self.has_changes = has_changes;
        cx.notify();
    }

    pub fn set_ahead_behind(&mut self, ahead: usize, behind: usize, cx: &mut Context<Self>) {
        self.ahead = ahead;
        self.behind = behind;
        cx.notify();
    }

    pub fn set_fetching(&mut self, fetching: bool, cx: &mut Context<Self>) {
        self.is_fetching = fetching;
        cx.notify();
    }

    pub fn set_pulling(&mut self, pulling: bool, cx: &mut Context<Self>) {
        self.is_pulling = pulling;
        cx.notify();
    }

    pub fn set_pushing(&mut self, pushing: bool, cx: &mut Context<Self>) {
        self.is_pushing = pushing;
        cx.notify();
    }

    /// Build a styled toolbar button with icon only (compact style).
    fn icon_button(
        &self,
        id: &'static str,
        icon: IconName,
        label: &'static str,
        tooltip_text: &'static str,
        state: ToolbarButtonState,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let disabled = state.disabled;
        let loading = state.loading;
        let colors = cx.colors();
        let hover_bg = colors.ghost_element_hover;
        let active_bg = colors.ghost_element_active;

        let is_inactive = disabled || loading;

        let icon_color = if loading {
            Color::Accent
        } else if disabled {
            Color::Disabled
        } else {
            Color::Default
        };
        let text_color = if loading {
            Color::Muted
        } else if disabled {
            Color::Disabled
        } else {
            Color::Default
        };

        

        div()
            .id(id)
            .h_flex()
            .h(px(26.))
            .px(px(6.))
            .gap(px(4.))
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .when(disabled && !loading, |el| el.opacity(0.5))
            .when(!is_inactive, move |el| {
                el.hover(move |s| s.bg(hover_bg))
                    .active(move |s| s.bg(active_bg))
                    .cursor_pointer()
            })
            .tooltip(Tooltip::text(tooltip_text))
            .child(Icon::new(icon).size(IconSize::Small).color(icon_color))
            .child(
                Label::new(label)
                    .size(LabelSize::XSmall)
                    .color(text_color),
            )
    }

    /// Small icon-only button for the right side.
    fn icon_only_button(
        &self,
        id: &'static str,
        icon: IconName,
        tooltip_text: &'static str,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let colors = cx.colors();
        let hover_bg = colors.ghost_element_hover;
        let active_bg = colors.ghost_element_active;

        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .w(px(26.))
            .h(px(26.))
            .rounded(px(4.))
            .hover(move |s| s.bg(hover_bg))
            .active(move |s| s.bg(active_bg))
            .cursor_pointer()
            .tooltip(Tooltip::text(tooltip_text))
            .child(Icon::new(icon).size(IconSize::Small).color(Color::Muted))
    }
}

impl Render for Toolbar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        let fetch_label = if self.is_fetching {
            "Fetching..."
        } else {
            "Fetch"
        };
        let pull_label: &str = if self.is_pulling {
            "Pulling..."
        } else if self.behind > 0 {
            // Will be shown with badge
            "Pull"
        } else {
            "Pull"
        };
        let push_label: &str = if self.is_pushing {
            "Pushing..."
        } else {
            "Push"
        };

        div()
            .h_flex()
            .w_full()
            .h(px(36.))
            .px(px(8.))
            .gap(px(2.))
            .items_center()
            .bg(colors.toolbar_background)
            .border_b_1()
            .border_color(colors.border_variant)
            // Network operations group
            .child(
                self.icon_button(
                    "tb-fetch",
                    IconName::Refresh,
                    fetch_label,
                    "Fetch from remote (Ctrl+Shift+F)",
                    ToolbarButtonState { disabled: self.is_fetching, loading: self.is_fetching },
                    cx,
                )
                .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(ToolbarEvent::Fetch))),
            )
            .child({
                let mut btn = self
                    .icon_button(
                        "tb-pull",
                        IconName::ArrowDown,
                        pull_label,
                        "Pull from remote",
                        ToolbarButtonState { disabled: !self.can_pull, loading: self.is_pulling },
                        cx,
                    )
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(ToolbarEvent::Pull)));
                if self.behind > 0 && !self.is_pulling {
                    btn = btn.child(
                        Badge::new(SharedString::from(self.behind.to_string())).color(Color::Info),
                    );
                }
                btn
            })
            .child({
                let mut btn = self
                    .icon_button(
                        "tb-push",
                        IconName::ArrowUp,
                        push_label,
                        "Push to remote",
                        ToolbarButtonState { disabled: !self.can_push, loading: self.is_pushing },
                        cx,
                    )
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(ToolbarEvent::Push)));
                if self.ahead > 0 && !self.is_pushing {
                    btn = btn.child(
                        Badge::new(SharedString::from(self.ahead.to_string()))
                            .color(Color::Success),
                    );
                }
                btn
            })
            .child(VerticalDivider::new())
            // Branch operations
            .child(
                self.icon_button(
                    "tb-branch",
                    IconName::GitBranch,
                    "Branch",
                    "Create new branch (Ctrl+B)",
                    ToolbarButtonState { disabled: false, loading: false },
                    cx,
                )
                .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(ToolbarEvent::Branch))),
            )
            .child(VerticalDivider::new())
            // Stash operations
            .child(
                self.icon_button(
                    "tb-stash",
                    IconName::Stash,
                    "Stash",
                    "Stash working changes",
                    ToolbarButtonState { disabled: !self.has_changes, loading: false },
                    cx,
                )
                .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(ToolbarEvent::StashSave))),
            )
            .child(
                self.icon_button(
                    "tb-pop",
                    IconName::Undo,
                    "Pop",
                    "Pop top stash entry",
                    ToolbarButtonState { disabled: !self.has_stashes, loading: false },
                    cx,
                )
                .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(ToolbarEvent::StashPop))),
            )
            // Spacer
            .child(div().flex_1())
            // Right-aligned icon buttons
            .child(
                self.icon_only_button("tb-search", IconName::Search, "Search commits (Ctrl+F)", cx)
                    .on_click(
                        cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(ToolbarEvent::Search)),
                    ),
            )
            .child(
                self.icon_only_button("tb-refresh", IconName::Refresh, "Refresh (F5)", cx)
                    .on_click(
                        cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(ToolbarEvent::Refresh)),
                    ),
            )
            .child(
                self.icon_only_button("tb-settings", IconName::Settings, "Settings (Ctrl+,)", cx)
                    .on_click(
                        cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(ToolbarEvent::Settings)),
                    ),
            )
    }
}
