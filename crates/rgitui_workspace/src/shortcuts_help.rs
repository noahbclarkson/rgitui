use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, EventEmitter, FocusHandle, FontWeight, KeyDownEvent, Render,
    Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Icon, IconName, IconSize, Label, LabelSize};

#[derive(Debug, Clone)]
pub enum ShortcutsHelpEvent {
    Dismissed,
}

struct ShortcutCategory {
    title: &'static str,
    description: &'static str,
    shortcuts: &'static [(&'static str, &'static str)],
}

pub struct ShortcutsHelp {
    visible: bool,
    focus_handle: FocusHandle,
}

impl EventEmitter<ShortcutsHelpEvent> for ShortcutsHelp {}

impl ShortcutsHelp {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            visible: false,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.visible = !self.visible;
        if self.visible {
            self.focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    pub fn toggle_visible(&mut self, cx: &mut Context<Self>) {
        self.visible = !self.visible;
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        cx.emit(ShortcutsHelpEvent::Dismissed);
        cx.notify();
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key.as_str() == "escape" {
            self.dismiss(cx);
            cx.stop_propagation();
        }
    }

    fn shortcut_categories() -> Vec<ShortcutCategory> {
        vec![
            ShortcutCategory {
                title: "Workspace",
                description: "App-level actions available from anywhere outside active overlays.",
                shortcuts: &[
                    ("Ctrl+Shift+P", "Open command palette"),
                    ("Ctrl+O", "Open repository"),
                    ("Ctrl+H", "Go to workspace home"),
                    ("Ctrl+W", "Close current tab"),
                    ("Ctrl+,", "Open settings"),
                    ("F5", "Refresh repository state"),
                    ("?", "Open this help"),
                ],
            },
            ShortcutCategory {
                title: "Navigation",
                description: "Focus a panel first. Plain-letter shortcuts are context-sensitive.",
                shortcuts: &[
                    ("j / k", "Move up / down in the focused panel"),
                    ("g / G", "Jump to first / last item"),
                    ("Tab / Shift+Tab", "Cycle focused panel"),
                    ("Alt+1 / 2 / 3 / 4", "Focus sidebar / graph / detail / diff"),
                    ("Ctrl+Tab / Ctrl+Shift+Tab", "Next / previous tab"),
                    ("Enter / Space", "Activate selected sidebar item"),
                ],
            },
            ShortcutCategory {
                title: "Views & Search",
                description: "Fast access to panels and graph-specific tools.",
                shortcuts: &[
                    ("Ctrl+F", "Toggle commit graph search"),
                    ("/", "Start in-graph search"),
                    ("d", "Toggle diff mode (unified / split)"),
                    ("b", "Toggle blame view for selected file"),
                    ("h", "Toggle file history view for selected file"),
                    ("y", "Copy SHA of selected commit"),
                    ("Shift+C", "Copy commit message of selected commit"),
                    ("Esc", "Close the active overlay or modal"),
                ],
            },
            ShortcutCategory {
                title: "Git & AI",
                description:
                    "Common write actions. More advanced operations live in the command palette.",
                shortcuts: &[
                    ("Ctrl+Shift+F", "Fetch"),
                    ("Ctrl+S", "Stage all changes"),
                    ("Ctrl+Shift+S / Ctrl+U", "Unstage all changes"),
                    ("Ctrl+Enter", "Commit"),
                    ("Ctrl+B", "Create branch"),
                    ("Ctrl+Shift+B", "Switch branch (focus sidebar)"),
                    ("Ctrl+Z / Ctrl+Shift+Z", "Stash changes / pop stash"),
                    ("Ctrl+G", "Generate AI commit message"),
                    ("s", "Stage / unstage selected file in sidebar"),
                    (
                        "Alt+S / Alt+U",
                        "Stage / unstage current hunk in diff viewer",
                    ),
                    (
                        "s / u",
                        "Stage / unstage hunks under selection (or cursor hunk)",
                    ),
                    ("x / Delete", "Discard selected sidebar item"),
                ],
            },
        ]
    }

    fn shortcut_count(categories: &[ShortcutCategory]) -> usize {
        categories
            .iter()
            .map(|category| category.shortcuts.len())
            .sum()
    }

    fn render_category(
        &self,
        category: &ShortcutCategory,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = cx.colors();
        let hint_bg = colors.hint_background;
        let border_variant = colors.border_variant;

        let mut col = div().v_flex().w_full().gap(px(2.)).child(
            div()
                .pb(px(6.))
                .mb(px(4.))
                .border_b_1()
                .border_color(border_variant)
                .child(
                    div()
                        .v_flex()
                        .gap(px(4.))
                        .child(
                            Label::new(category.title)
                                .size(LabelSize::Small)
                                .weight(FontWeight::SEMIBOLD)
                                .color(Color::Accent),
                        )
                        .child(
                            Label::new(category.description)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                ),
        );

        for (key, desc) in category.shortcuts {
            let hover_bg = colors.ghost_element_hover;
            col = col.child(
                div()
                    .h_flex()
                    .w_full()
                    .py(px(4.))
                    .px(px(4.))
                    .rounded(px(4.))
                    .items_center()
                    .gap(px(12.))
                    .hover(move |s| s.bg(hover_bg))
                    .child(
                        div().flex_1().child(
                            Label::new(*desc)
                                .size(LabelSize::Small)
                                .color(Color::Default),
                        ),
                    )
                    .child(
                        div()
                            .h_flex()
                            .h(px(22.))
                            .px(px(8.))
                            .rounded(px(4.))
                            .bg(hint_bg)
                            .items_center()
                            .child(
                                Label::new(*key)
                                    .size(LabelSize::XSmall)
                                    .weight(FontWeight::MEDIUM)
                                    .color(Color::Muted),
                            ),
                    ),
            );
        }

        col
    }
}

impl Render for ShortcutsHelp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().id("shortcuts-help").into_any_element();
        }

        let categories = Self::shortcut_categories();
        let total_shortcuts = Self::shortcut_count(&categories);

        let left_categories = &categories[..2];
        let right_categories = &categories[2..];

        let mut left_col = div().v_flex().flex_1().gap(px(16.));
        for category in left_categories {
            left_col = left_col.child(self.render_category(category, cx));
        }

        let mut right_col = div().v_flex().flex_1().gap(px(16.));
        for category in right_categories {
            right_col = right_col.child(self.render_category(category, cx));
        }

        let colors = cx.colors();

        let backdrop = div()
            .id("shortcuts-help-backdrop")
            .occlude()
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
                a: 0.4,
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.dismiss(cx);
            }));

        let modal = div()
            .id("shortcuts-help-container")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .v_flex()
            .w(px(760.))
            .max_h(px(620.))
            .elevation_3(cx)
            .rounded(px(10.))
            .overflow_hidden()
            .on_click(|_: &ClickEvent, _, cx| {
                cx.stop_propagation();
            })
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .h(px(56.))
                    .px(px(16.))
                    .items_center()
                    .border_b_1()
                    .border_color(colors.border_variant)
                    .justify_between()
                    .child(
                        div()
                            .h_flex()
                            .gap(px(10.))
                            .items_center()
                            .child(
                                Icon::new(IconName::Star)
                                    .size(IconSize::Small)
                                    .color(Color::Muted),
                            )
                            .child(
                                div()
                                    .v_flex()
                                    .gap(px(2.))
                                    .child(
                                        Label::new("Keyboard Shortcuts")
                                            .size(LabelSize::Large)
                                            .weight(FontWeight::SEMIBOLD),
                                    )
                                    .child(
                                        Label::new(format!(
                                            "{} shortcuts across navigation, views, workspace, and git actions",
                                            total_shortcuts
                                        ))
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .id("shortcuts-close-btn")
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(28.))
                            .h(px(28.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .hover(|s| s.bg(colors.ghost_element_hover))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.dismiss(cx);
                            }))
                            .child(
                                Icon::new(IconName::X)
                                    .size(IconSize::Small)
                                    .color(Color::Muted),
                            ),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .px(px(16.))
                    .pt(px(12.))
                    .child(
                        div()
                            .w_full()
                            .rounded(px(8.))
                            .bg(colors.surface_background)
                            .border_1()
                            .border_color(colors.border_variant)
                            .px(px(12.))
                            .py(px(10.))
                            .child(
                                Label::new(
                                    "Tip: plain-letter shortcuts like d, b, h, j, and k depend on which panel is focused. Use Ctrl+Shift+P for less common actions like reflog, submodules, bisect, and stash management.",
                                )
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                            ),
                    ),
            )
            .child(
                div()
                    .id("shortcuts-body")
                    .h_flex()
                    .w_full()
                    .p(px(16.))
                    .gap(px(24.))
                    .overflow_y_scroll()
                    .child(left_col)
                    .child(right_col),
            )
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .h(px(36.))
                    .px(px(16.))
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .bg(colors.surface_background)
                    .child(
                        Label::new("Press Esc or click outside to close")
                            .size(LabelSize::XSmall)
                            .color(Color::Placeholder),
                    )
                    .child(
                        Label::new("More actions: Ctrl+Shift+P")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            );

        backdrop.child(modal).into_any_element()
    }
}
