use gpui::prelude::*;
use gpui::SharedString;
use gpui::{div, px, ClickEvent, Context, EventEmitter, Render, Window};
use rgitui_settings::SettingsState;
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Badge, Button, Icon, IconButton, IconName, IconSize, Indicator, Label, LabelSize, Tooltip,
    VerticalDivider,
};

type TooltipFactory = Box<dyn Fn(&mut gpui::Window, &mut gpui::App) -> gpui::AnyView>;

struct ToolbarButtonState {
    disabled: bool,
    loading: bool,
    tooltip_text: &'static str,
    shortcut: Option<&'static str>,
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
    CreatePr,
    Refresh,
    Settings,
    Search,
    OpenFileExplorer,
    OpenTerminal,
    OpenEditor,
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
    has_github_token: bool,
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
            has_github_token: false,
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
        has_github_token: bool,
        cx: &mut Context<Self>,
    ) {
        if self.can_push == can_push
            && self.can_pull == can_pull
            && self.has_stashes == has_stashes
            && self.has_changes == has_changes
            && self.has_github_token == has_github_token
        {
            return;
        }
        self.can_push = can_push;
        self.can_pull = can_pull;
        self.has_stashes = has_stashes;
        self.has_changes = has_changes;
        self.has_github_token = has_github_token;
        cx.notify();
    }

    pub fn set_ahead_behind(&mut self, ahead: usize, behind: usize, cx: &mut Context<Self>) {
        if self.ahead == ahead && self.behind == behind {
            return;
        }
        self.ahead = ahead;
        self.behind = behind;
        cx.notify();
    }

    pub fn set_fetching(&mut self, fetching: bool, cx: &mut Context<Self>) {
        if self.is_fetching == fetching {
            return;
        }
        self.is_fetching = fetching;
        cx.notify();
    }

    pub fn set_pulling(&mut self, pulling: bool, cx: &mut Context<Self>) {
        if self.is_pulling == pulling {
            return;
        }
        self.is_pulling = pulling;
        cx.notify();
    }

    pub fn set_pushing(&mut self, pushing: bool, cx: &mut Context<Self>) {
        if self.is_pushing == pushing {
            return;
        }
        self.is_pushing = pushing;
        cx.notify();
    }

    fn build_tooltip(tooltip_text: &'static str, shortcut: Option<&'static str>) -> TooltipFactory {
        if let Some(sc) = shortcut {
            Box::new(Tooltip::with_shortcut(tooltip_text, sc))
        } else {
            Box::new(Tooltip::text(tooltip_text))
        }
    }

    // TODO(audit) QUAL-11: the network-operation buttons (fetch/pull/push) stay
    // hand-rolled because the shared `Button` cannot express their behavior: it has
    // no child API for the inline ahead/behind `Badge`, and it forces a disabled
    // icon color, so the non-interactive `Color::Accent` "loading" affordance is
    // not reproducible. Consolidating them needs loading + child support on the
    // shared `Button`, which lives in `rgitui_ui::button`.
    fn icon_button(
        &self,
        id: &'static str,
        icon: IconName,
        label: &'static str,
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

        let tooltip_fn = Self::build_tooltip(state.tooltip_text, state.shortcut);

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
            .tooltip(move |window, cx| tooltip_fn(window, cx))
            .child(Icon::new(icon).size(IconSize::Small).color(icon_color))
            .child(Label::new(label).size(LabelSize::XSmall).color(text_color))
    }

    fn render_left_group(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let fetch_label = if self.is_fetching {
            "Fetching..."
        } else {
            "Fetch"
        };
        let pull_label = if self.is_pulling {
            "Pulling..."
        } else {
            "Pull"
        };
        let push_label = if self.is_pushing {
            "Pushing..."
        } else {
            "Push"
        };

        div()
            .h_flex()
            .items_center()
            .gap(px(2.))
            // Network operations group
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap(px(2.))
                    .child(
                        self.icon_button(
                            "tb-fetch",
                            IconName::Refresh,
                            fetch_label,
                            ToolbarButtonState {
                                disabled: self.is_fetching,
                                loading: self.is_fetching,
                                tooltip_text: "Fetch from remote",
                                shortcut: None,
                            },
                            cx,
                        )
                        .on_click(
                            cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(ToolbarEvent::Fetch)),
                        ),
                    )
                    .child({
                        let mut btn = self
                            .icon_button(
                                "tb-pull",
                                IconName::ArrowDown,
                                pull_label,
                                ToolbarButtonState {
                                    disabled: !self.can_pull,
                                    loading: self.is_pulling,
                                    tooltip_text: "Pull from remote",
                                    shortcut: None,
                                },
                                cx,
                            )
                            .on_click(
                                cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(ToolbarEvent::Pull)),
                            );
                        if self.behind > 0 && !self.is_pulling {
                            btn = btn.child(
                                Badge::new(SharedString::from(self.behind.to_string()))
                                    .color(Color::Info),
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
                                ToolbarButtonState {
                                    disabled: !self.can_push,
                                    loading: self.is_pushing,
                                    tooltip_text: "Push to remote",
                                    shortcut: None,
                                },
                                cx,
                            )
                            .on_click(
                                cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(ToolbarEvent::Push)),
                            );
                        if self.ahead > 0 && !self.is_pushing {
                            btn = btn.child(
                                Badge::new(SharedString::from(self.ahead.to_string()))
                                    .color(Color::Success),
                            );
                        }
                        btn
                    })
                    .when(self.ahead > 0 && self.behind == 0, |el| {
                        el.child(Indicator::dot(Color::Success))
                    })
                    .when(self.behind > 0 && self.ahead == 0, |el| {
                        el.child(Indicator::dot(Color::Warning))
                    })
                    .when(self.ahead > 0 && self.behind > 0, |el| {
                        el.child(Indicator::dot(Color::Info))
                    }),
            )
            .child(VerticalDivider::new())
            // Branch operations group
            .child(
                Button::new("tb-branch", "Branch")
                    .icon(IconName::GitBranch)
                    .tooltip_fn(Tooltip::with_shortcut("Create new branch", "Ctrl+B"))
                    .on_click(
                        cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(ToolbarEvent::Branch)),
                    ),
            )
            .child(VerticalDivider::new())
            // Stash operations group
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap(px(2.))
                    .child(
                        Button::new("tb-stash", "Stash")
                            .icon(IconName::Stash)
                            .disabled(!self.has_changes)
                            .tooltip_fn(Tooltip::with_shortcut("Stash working changes", "Ctrl+Z"))
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                cx.emit(ToolbarEvent::StashSave)
                            })),
                    )
                    .child(
                        Button::new("tb-pop", "Pop")
                            .icon(IconName::Undo)
                            .disabled(!self.has_stashes)
                            .tooltip_fn(Tooltip::with_shortcut(
                                "Pop top stash entry",
                                "Ctrl+Shift+Z",
                            ))
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                cx.emit(ToolbarEvent::StashPop)
                            })),
                    ),
            )
            .child(VerticalDivider::new())
            // PR creation group
            .child(
                Button::new("tb-pr", "Create PR")
                    .icon(IconName::GitPullRequest)
                    .disabled(!self.has_github_token)
                    .tooltip_fn(Tooltip::with_shortcut(
                        "Create GitHub pull request",
                        "Ctrl+G",
                    ))
                    .on_click(
                        cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(ToolbarEvent::CreatePr)),
                    ),
            )
    }

    fn render_right_group(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .h_flex()
            .items_center()
            .gap(px(2.))
            // External tools group
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap(px(2.))
                    .child(
                        IconButton::new("tb-explorer", IconName::Folder)
                            .color(Color::Muted)
                            .tooltip("Open in file explorer")
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                cx.emit(ToolbarEvent::OpenFileExplorer)
                            })),
                    )
                    .child(
                        IconButton::new("tb-terminal", IconName::Terminal)
                            .color(Color::Muted)
                            .tooltip("Open terminal")
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                cx.emit(ToolbarEvent::OpenTerminal)
                            })),
                    )
                    .child(
                        IconButton::new("tb-editor", IconName::ExternalLink)
                            .color(Color::Muted)
                            .tooltip("Open in editor")
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                cx.emit(ToolbarEvent::OpenEditor)
                            })),
                    ),
            )
            .child(VerticalDivider::new())
            // Utility actions group
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap(px(2.))
                    .child(
                        IconButton::new("tb-search", IconName::Search)
                            .color(Color::Muted)
                            .tooltip_fn(Tooltip::with_shortcut("Search commits", "Ctrl+F"))
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                cx.emit(ToolbarEvent::Search)
                            })),
                    )
                    .child(
                        IconButton::new("tb-refresh", IconName::Refresh)
                            .color(Color::Muted)
                            .tooltip_fn(Tooltip::with_shortcut("Refresh", "F5"))
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                cx.emit(ToolbarEvent::Refresh)
                            })),
                    )
                    .child(
                        IconButton::new("tb-settings", IconName::Settings)
                            .color(Color::Muted)
                            .tooltip_fn(Tooltip::with_shortcut("Settings", "Ctrl+,"))
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                cx.emit(ToolbarEvent::Settings)
                            })),
                    ),
            )
    }
}

impl Render for Toolbar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compactness = cx.global::<SettingsState>().settings().compactness;
        let toolbar_h = compactness.spacing(36.0);
        let toolbar_bg = cx.colors().toolbar_background;
        let border_color = cx.colors().border_variant;

        let left = self.render_left_group(cx);
        let right = self.render_right_group(cx);

        div()
            .h_flex()
            .w_full()
            .h(px(toolbar_h))
            .px(px(8.))
            .items_center()
            .justify_between()
            .bg(toolbar_bg)
            .border_b_1()
            .border_color(border_color)
            .child(left)
            .child(right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_toolbar_event_debug() {
        let event = ToolbarEvent::Fetch;
        assert_eq!(format!("{:?}", event), "Fetch");

        let event = ToolbarEvent::CreatePr;
        assert_eq!(format!("{:?}", event), "CreatePr");
    }

    #[test]
    fn test_toolbar_event_match() {
        let event = ToolbarEvent::Push;
        if let ToolbarEvent::Push = event {
            // expected
        } else {
            panic!("Expected Push");
        }
    }
}
