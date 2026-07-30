//! The keyboard shortcut reference, rendered from the keymap in force.
//!
//! Nothing here is a literal list of shortcuts. Every row comes from
//! [`crate::keymap::summary`]: the group is a `commands!` view block, the
//! description is the command's doc comment, and the keystroke is the one the
//! user's `keymap.json` actually produced rather than the registry default.

use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, EventEmitter, FocusHandle, FontWeight, Render, SharedString,
    Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Icon, IconName, IconSize, Label, LabelSize};

use crate::keymap::{self, CommandBindings, CommandGroup, KeymapSummary};
use crate::CommandId;

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

    /// Runs a keyboard command scoped to `ShortcutsHelp`.
    ///
    /// Enter is not handled here: it is propagated so the focused field's own
    /// submission fires exactly once.
    fn dispatch_command(&mut self, cmd: CommandId, _window: &mut Window, cx: &mut Context<Self>) {
        match cmd {
            CommandId::Cancel => self.dismiss(cx),
            _ => cx.propagate(),
        }
    }

    /// Renders one command's row: what it does and what it is bound to.
    fn render_command(&self, entry: &CommandBindings, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors().clone();
        let hover_bg = colors.ghost_element_hover;
        let keystrokes = entry.display();

        let row = div()
            .v_flex()
            .w_full()
            .py(px(5.))
            .px(px(4.))
            .rounded(px(4.))
            .gap(px(3.))
            .hover(move |s| s.bg(hover_bg));

        let mut headline = div()
            .flex()
            .flex_row()
            .w_full()
            .items_center()
            .gap(px(10.))
            .child(
                div().flex_1().min_w_0().child(
                    Label::new(SharedString::from(entry.command.description()))
                        .size(LabelSize::Small)
                        .color(if keystrokes.is_some() {
                            Color::Default
                        } else {
                            Color::Muted
                        }),
                ),
            );

        headline = match keystrokes {
            Some(keystrokes) => headline.child(
                div()
                    .h_flex()
                    .flex_shrink_0()
                    .h(px(24.))
                    .px(px(10.))
                    .gap_1()
                    .rounded(px(5.))
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.hint_background)
                    .items_center()
                    .child(
                        Label::new(SharedString::from(keystrokes))
                            .size(LabelSize::Small)
                            .weight(FontWeight::BOLD)
                            .color(Color::Default),
                    ),
            ),
            None => headline.child(
                div().flex_shrink_0().child(
                    Label::new(keymap::display::UNBOUND)
                        .size(LabelSize::XSmall)
                        .color(Color::Placeholder),
                ),
            ),
        };

        row.child(headline)
    }

    fn render_group(&self, group: &CommandGroup, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors().clone();
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
                            Label::new(group.view)
                                .size(LabelSize::Small)
                                .weight(FontWeight::SEMIBOLD)
                                .color(Color::Accent),
                        )
                        .child(
                            Label::new(SharedString::from(group.description()))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                ),
        );

        for entry in &group.commands {
            col = col.child(self.render_command(entry, cx));
        }

        col
    }

    /// The header summary line, counted from the keymap rather than written out.
    fn subtitle(summary: &KeymapSummary) -> String {
        format!(
            "{} of {} commands are bound",
            summary.bound_command_count(),
            summary.commands().len()
        )
    }
}

impl Render for ShortcutsHelp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().id("shortcuts-help").into_any_element();
        }

        let summary = keymap::summary(cx);
        let groups = summary.groups();
        let subtitle = Self::subtitle(&summary);
        let palette_hint = keymap::shortcut(CommandId::CommandPalette, cx);

        let colors = cx.colors().clone();
        let viewport = window.viewport_size();
        let viewport_width = f32::from(viewport.width);
        let viewport_height = f32::from(viewport.height);
        let modal_width = px((viewport_width - 32.0).clamp(300.0, 960.0));
        let modal_height = px((viewport_height - 32.0).clamp(280.0, 720.0));
        let use_two_columns = viewport_width >= 960.0;

        // Split the groups into two balanced columns by row count, so a long
        // block like `Workspace` does not leave the second column empty.
        let body = if use_two_columns {
            let total: usize = groups.iter().map(|group| group.commands.len()).sum();
            let mut running = 0usize;
            let split = groups
                .iter()
                .position(|group| {
                    running += group.commands.len();
                    running * 2 >= total
                })
                .map_or(groups.len(), |index| index + 1)
                .min(groups.len());

            let mut left_col = div().v_flex().flex_1().min_w_0().gap(px(16.));
            for group in &groups[..split] {
                left_col = left_col.child(self.render_group(group, cx));
            }

            let mut right_col = div().v_flex().flex_1().min_w_0().gap(px(16.));
            for group in &groups[split..] {
                right_col = right_col.child(self.render_group(group, cx));
            }

            div()
                .id("shortcuts-body")
                .flex()
                .flex_row()
                .flex_1()
                .min_h_0()
                .w_full()
                .p(px(16.))
                .gap(px(24.))
                .items_start()
                .overflow_y_scroll()
                .child(left_col)
                .child(right_col)
                .into_any_element()
        } else {
            let mut column = div().v_flex().w_full().gap(px(16.));
            for group in &groups {
                column = column.child(self.render_group(group, cx));
            }

            div()
                .id("shortcuts-body")
                .v_flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .p(px(16.))
                .overflow_y_scroll()
                .child(column)
                .into_any_element()
        };

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
            .map(|el| {
                keymap::bind_actions(el, "ShortcutsHelp", &["Menu"], cx, Self::dispatch_command)
            })
            .v_flex()
            .w(modal_width)
            .h(modal_height)
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
                    .gap(px(10.))
                    .child(
                        div()
                            .h_flex()
                            .flex_1()
                            .min_w_0()
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
                                    .min_w_0()
                                    .gap(px(2.))
                                    .child(
                                        Label::new("Keyboard Shortcuts")
                                            .size(LabelSize::Large)
                                            .weight(FontWeight::SEMIBOLD),
                                    )
                                    .child(
                                        Label::new(SharedString::from(subtitle))
                                            .size(LabelSize::XSmall)
                                            .truncate()
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
                div().w_full().px(px(16.)).pt(px(12.)).child(
                    div()
                        .w_full()
                        .rounded(px(8.))
                        .bg(colors.surface_background)
                        .border_1()
                        .border_color(colors.border_variant)
                        .px(px(12.))
                        .py(px(10.))
                        .child(
                            Label::new(SharedString::from(format!(
                                "Every shortcut below is shown as your keymap.json leaves \n                                     it. Plain-letter shortcuts only act on the panel that has \n                                     focus — that is what each group's key context means. \n                                     Commands marked {} have no keystroke and are reached from \n                                     the command palette{}.",
                                keymap::display::UNBOUND,
                                palette_hint
                                    .as_deref()
                                    .map(|hint| format!(" ({hint})"))
                                    .unwrap_or_default(),
                            )))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                        ),
                ),
            )
            .child(body)
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
                    .when_some(
                        palette_hint.filter(|_| viewport_width >= 520.0),
                        |footer, hint| {
                            footer.child(
                                Label::new(SharedString::from(format!("More actions: {hint}")))
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                        },
                    ),
            );

        backdrop.child(modal).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::display::KeystrokeStyle;

    fn defaults() -> KeymapSummary {
        KeymapSummary::defaults(KeystrokeStyle::Words)
    }

    #[test]
    fn test_shortcuts_help_event_debug() {
        let event = ShortcutsHelpEvent::Dismissed;
        assert_eq!(format!("{:?}", event), "Dismissed");
    }

    #[test]
    fn test_shortcuts_help_event_match() {
        match ShortcutsHelpEvent::Dismissed {
            ShortcutsHelpEvent::Dismissed => {}
        }
    }

    /// Every group the view renders has a heading, a derived key-context blurb
    /// and at least one row, so no group can render as an empty box.
    #[test]
    fn every_group_has_a_heading_and_rows() {
        let summary = defaults();
        let groups = summary.groups();
        assert!(!groups.is_empty());
        for group in &groups {
            assert!(!group.view.is_empty());
            assert!(!group.description().is_empty());
            assert!(!group.commands.is_empty(), "{} is empty", group.view);
            for entry in &group.commands {
                assert!(
                    !entry.command.description().is_empty(),
                    "{} has no description to render",
                    entry.command
                );
            }
        }
    }

    /// The old hand-written table advertised `Ctrl+Shift+F` for Fetch while the
    /// keymap bound `Ctrl+Shift+R`. The row now comes from the keymap, so the
    /// two cannot differ.
    #[test]
    fn a_row_shows_the_keystroke_the_keymap_binds() {
        let summary = defaults();
        assert_eq!(
            summary.display(CommandId::Fetch),
            crate::keymap::humanize_sequence(
                CommandId::Fetch.default_bindings()[0].0,
                KeystrokeStyle::Words
            )
        );
    }

    #[test]
    fn the_subtitle_counts_what_the_keymap_holds() {
        let summary = defaults();
        let subtitle = ShortcutsHelp::subtitle(&summary);
        assert!(
            subtitle.contains(&summary.bound_command_count().to_string()),
            "{subtitle}"
        );
    }
}
