use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, EventEmitter, FocusHandle, FontWeight, KeyDownEvent, Render,
    SharedString, Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Label, LabelSize};

/// Events emitted by the shortcuts help overlay.
#[derive(Debug, Clone)]
pub enum ShortcutsHelpEvent {
    Dismissed,
}

/// A modal overlay that displays keyboard shortcuts grouped by category.
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
                "Git",
                vec![
                    ("Ctrl+S", "Stage all"),
                    ("Ctrl+Shift+S", "Unstage All"),
                    ("Ctrl+Enter", "Commit"),
                    ("Ctrl+B", "Create Branch"),
                    ("Ctrl+Z", "Stash Changes"),
                    ("Ctrl+Shift+Z", "Pop Stash"),
                ],
            ),
            (
                "View",
                vec![
                    ("Ctrl+O", "Open Repository"),
                    ("Ctrl+F", "Search"),
                    ("Ctrl+,", "Settings"),
                    ("Ctrl+Shift+P", "Command Palette"),
                ],
            ),
            (
                "Window",
                vec![
                    ("Ctrl+W", "Close tab"),
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

        let mut col = div().v_flex().w_full().gap_1().child(
            Label::new(SharedString::from(title.to_string()))
                .size(LabelSize::Small)
                .weight(FontWeight::SEMIBOLD)
                .color(Color::Accent),
        );

        for (key, desc) in shortcuts {
            col = col.child(
                div()
                    .h_flex()
                    .w_full()
                    .gap_3()
                    .py(px(2.))
                    .child(
                        div().w(px(140.)).flex_shrink_0().child(
                            div().h_flex().child(
                                div()
                                    .px(px(6.))
                                    .py(px(2.))
                                    .bg(colors.element_background)
                                    .border_1()
                                    .border_color(colors.border_variant)
                                    .rounded(px(4.))
                                    .child(
                                        Label::new(SharedString::from(key.to_string()))
                                            .size(LabelSize::XSmall)
                                            .weight(FontWeight::MEDIUM),
                                    ),
                            ),
                        ),
                    )
                    .child(
                        Label::new(SharedString::from(desc.to_string()))
                            .size(LabelSize::Small)
                            .color(Color::Muted),
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

        // Build the two columns before borrowing colors
        let left_categories = &categories[..2];
        let right_categories = &categories[2..];

        let mut left_col = div().v_flex().flex_1().gap_4();
        for (title, shortcuts) in left_categories {
            left_col = left_col.child(self.render_category(title, shortcuts, cx));
        }

        let mut right_col = div().v_flex().flex_1().gap_4();
        for (title, shortcuts) in right_categories {
            right_col = right_col.child(self.render_category(title, shortcuts, cx));
        }

        let colors = cx.colors();

        let backdrop = div()
            .id("shortcuts-help-backdrop")
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
            .w(px(520.))
            .bg(colors.elevated_surface_background)
            .border_1()
            .border_color(colors.border)
            .rounded_lg()
            .elevation_3(cx)
            .overflow_hidden()
            // Prevent clicks inside the modal from dismissing it
            .on_click(|_: &ClickEvent, _, cx| {
                cx.stop_propagation();
            })
            // Header
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(colors.border_variant)
                    .justify_between()
                    .child(
                        Label::new("Keyboard Shortcuts")
                            .size(LabelSize::Large)
                            .weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        Label::new("Esc to close")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
            // Body — two-column grid
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .p_4()
                    .gap_6()
                    .child(left_col)
                    .child(right_col),
            );

        backdrop.child(modal).into_any_element()
    }
}
