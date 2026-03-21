use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, EventEmitter, FocusHandle, FontWeight, KeyDownEvent, Render,
    SharedString, Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Icon, IconName, IconSize, Label, LabelSize};

#[derive(Debug, Clone)]
pub enum ShortcutsHelpEvent {
    Dismissed,
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

    fn shortcut_categories() -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
        vec![
            (
                "Navigation",
                vec![
                    ("j / k", "Up / Down in focused panel"),
                    ("g / G", "First / Last item"),
                    ("/", "Search in graph"),
                    ("d", "Toggle diff mode (unified/split)"),
                    ("Tab", "Next panel"),
                    ("Shift+Tab", "Previous panel"),
                    ("Alt+1", "Focus sidebar"),
                    ("Alt+2", "Focus graph"),
                    ("Alt+3", "Focus detail panel"),
                    ("Alt+4", "Focus diff viewer"),
                    ("Enter / Space", "Activate sidebar item"),
                    ("s", "Stage / Unstage file (sidebar)"),
                    ("x / Delete", "Delete selected item (sidebar)"),
                    ("Ctrl+Tab", "Next tab"),
                    ("Ctrl+Shift+Tab", "Previous tab"),
                ],
            ),
            (
                "Git Operations",
                vec![
                    ("Ctrl+S", "Stage all"),
                    ("Ctrl+Shift+S / Ctrl+U", "Unstage All"),
                    ("Ctrl+Enter", "Commit"),
                    ("Ctrl+B", "Create Branch"),
                    ("Ctrl+Shift+B", "Switch Branch"),
                    ("Ctrl+Z", "Stash Changes"),
                    ("Ctrl+Shift+Z", "Pop Stash"),
                ],
            ),
            (
                "Panel Management",
                vec![
                    ("Ctrl+O", "Open Repository"),
                    ("Ctrl+F", "Search"),
                    ("Ctrl+,", "Settings"),
                    ("Ctrl+Shift+P", "Command Palette"),
                ],
            ),
            (
                "General",
                vec![
                    ("Ctrl+W", "Close tab"),
                    ("Ctrl+H", "Workspace Home"),
                    ("F5", "Refresh"),
                    ("?", "This help"),
                ],
            ),
        ]
    }

    fn render_category(
        &self,
        title: &str,
        shortcuts: &[(&str, &str)],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = cx.colors();
        let hint_bg = colors.hint_background;
        let border_variant = colors.border_variant;

        let mut col = div()
            .v_flex()
            .w_full()
            .gap(px(2.))
            .child(
                div()
                    .pb(px(6.))
                    .mb(px(4.))
                    .border_b_1()
                    .border_color(border_variant)
                    .child(
                        Label::new(SharedString::from(title.to_string()))
                            .size(LabelSize::Small)
                            .weight(FontWeight::SEMIBOLD)
                            .color(Color::Accent),
                    ),
            );

        for (key, desc) in shortcuts {
            let hover_bg = colors.ghost_element_hover;
            col = col.child(
                div()
                    .h_flex()
                    .w_full()
                    .py(px(4.))
                    .px(px(4.))
                    .rounded(px(4.))
                    .items_center()
                    .hover(move |s| s.bg(hover_bg))
                    .child(
                        div().flex_1().child(
                            Label::new(SharedString::from(desc.to_string()))
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
                                Label::new(SharedString::from(key.to_string()))
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

        let left_categories = &categories[..2];
        let right_categories = &categories[2..];

        let mut left_col = div().v_flex().flex_1().gap(px(16.));
        for (title, shortcuts) in left_categories {
            left_col = left_col.child(self.render_category(title, shortcuts, cx));
        }

        let mut right_col = div().v_flex().flex_1().gap(px(16.));
        for (title, shortcuts) in right_categories {
            right_col = right_col.child(self.render_category(title, shortcuts, cx));
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
            .w(px(640.))
            .max_h(px(560.))
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
                    .h(px(48.))
                    .px(px(16.))
                    .items_center()
                    .border_b_1()
                    .border_color(colors.border_variant)
                    .justify_between()
                    .child(
                        div()
                            .h_flex()
                            .gap(px(8.))
                            .items_center()
                            .child(
                                Icon::new(IconName::Star)
                                    .size(IconSize::Small)
                                    .color(Color::Muted),
                            )
                            .child(
                                Label::new("Keyboard Shortcuts")
                                    .size(LabelSize::Large)
                                    .weight(FontWeight::SEMIBOLD),
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
                    .h(px(32.))
                    .px(px(16.))
                    .items_center()
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .bg(colors.surface_background)
                    .child(
                        Label::new("Press Esc or click outside to close")
                            .size(LabelSize::XSmall)
                            .color(Color::Placeholder),
                    ),
            );

        backdrop.child(modal).into_any_element()
    }
}
