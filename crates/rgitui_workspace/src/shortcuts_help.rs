//! The keyboard shortcut reference, rendered from the keymap in force.
//!
//! Nothing here is a literal list of shortcuts. Every row comes from
//! [`crate::keymap::summary`]: the description is the command's doc comment, the
//! keystroke is what the user's `keymap.json` actually produced, a binding the
//! user supplied is badged as such, and a binding that was dropped — because two
//! of theirs collided, or because a keystroke now belongs to another command —
//! is shown as a warning on the row it affects. Opening this panel is therefore
//! how a user finds out *which* of their bindings was ignored and why.
//!
//! Rows also carry the informational notes: a panel binding winning a keystroke
//! from a global one is legitimate scoping, so it is styled as an aside and is
//! deliberately absent from the "with a problem" count in the header.

use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, EventEmitter, FocusHandle, FontWeight, Render, SharedString,
    Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Badge, Button, ButtonSize, ButtonStyle, Icon, IconName, IconSize, Label, LabelSize,
};

use crate::keymap::{self, CommandBindings, CommandGroup, KeymapSummary, NoteSeverity};
use crate::CommandId;

#[derive(Debug, Clone)]
pub enum ShortcutsHelpEvent {
    Dismissed,
    /// The user asked to edit `keymap.json`; the workspace opens it.
    OpenKeymapFile,
}

/// Badge text marking a binding that came from the user's `keymap.json`.
const USER_BINDING_BADGE: &str = "keymap.json";

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

    /// Renders one command's row: what it does, what it is bound to, where that
    /// binding came from and anything wrong with it.
    fn render_command(&self, entry: &CommandBindings, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors().clone();
        let hover_bg = colors.ghost_element_hover;
        let is_user_defined = entry.is_user_defined();
        let keystrokes = entry.display();

        let mut row = div()
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

        if is_user_defined {
            headline = headline.child(
                div()
                    .flex_shrink_0()
                    .child(Badge::new(USER_BINDING_BADGE).color(Color::Info)),
            );
        }

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
                    .border_color(if is_user_defined {
                        Color::Info.color(cx)
                    } else {
                        colors.border
                    })
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

        row = row.child(headline);

        for note in &entry.notes {
            // Info notes are the deliberate deeper-wins scoping the defaults rely
            // on, so they read as an aside rather than as something to fix.
            let (icon, color) = match note.severity {
                NoteSeverity::Warning => (IconName::AlertTriangle, Color::Warning),
                NoteSeverity::Info => (IconName::Info, Color::Muted),
            };
            row = row.child(
                div()
                    .h_flex()
                    .w_full()
                    .gap(px(6.))
                    .items_start()
                    .child(Icon::new(icon).size(IconSize::XSmall).color(color))
                    .child(
                        div().flex_1().min_w_0().child(
                            Label::new(SharedString::from(note.message.clone()))
                                .size(LabelSize::XSmall)
                                .color(color),
                        ),
                    ),
            );
        }

        row
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
        let mut parts = vec![format!(
            "{} of {} commands are bound",
            summary.bound_command_count(),
            summary.commands().len()
        )];
        let user_bindings = summary.user_binding_count();
        if user_bindings > 0 {
            parts.push(format!("{user_bindings} from your keymap.json",));
        }
        let warnings = summary.warning_count();
        if warnings > 0 {
            parts.push(format!(
                "{warnings} with a problem — see the warnings below",
            ));
        }
        parts.join(", ")
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
        let keymap_file = keymap::keymap_path().display().to_string();

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
                        Button::new("shortcuts-open-keymap", "Edit keymap.json")
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Compact)
                            .icon(IconName::Settings)
                            .tooltip(SharedString::from(keymap_file))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                cx.emit(ShortcutsHelpEvent::OpenKeymapFile);
                                this.dismiss(cx);
                            })),
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
                                "Every shortcut below is rebindable, and each is shown as your \
                                     keymap.json leaves it. Plain-letter shortcuts only act on the \
                                     panel that has focus — that is what each group's key context \
                                     means. Commands marked {} have no keystroke and are reached \
                                     from the command palette{}.",
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
    use crate::keymap::conflict::BindingSpec;
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
            ShortcutsHelpEvent::OpenKeymapFile => unreachable!(),
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
        // Nothing to warn about and nothing user-defined in the defaults.
        assert!(!subtitle.contains("keymap.json"), "{subtitle}");
        assert!(!subtitle.contains("problem"), "{subtitle}");
    }

    /// A user binding and a conflict both have to be visible in the panel: the
    /// badge comes from `is_user_defined`, the warning row from `warnings`.
    #[test]
    fn the_subtitle_and_rows_surface_user_bindings_and_conflicts() {
        let mut specs = crate::keymap::loader::default_specs();
        specs.push(BindingSpec::user_binding(
            "ctrl-alt-9",
            Some("Workspace"),
            "rgitui::Pull",
        ));
        specs.push(BindingSpec::user_binding(
            "ctrl-alt-9",
            Some("Workspace"),
            "rgitui::Push",
        ));
        let report = crate::keymap::conflict::detect_conflicts(&specs);
        let applied: Vec<usize> = (0..specs.len()).filter(|i| report.is_kept(*i)).collect();
        let summary = KeymapSummary::build(&specs, &applied, &report, KeystrokeStyle::Words);

        let subtitle = ShortcutsHelp::subtitle(&summary);
        assert!(subtitle.contains("keymap.json"), "{subtitle}");
        assert!(subtitle.contains("problem"), "{subtitle}");

        // Push won, so it carries the badge; Pull lost, so it carries the warning.
        assert!(summary.is_user_defined(CommandId::Push));
        assert_eq!(summary.warnings(CommandId::Pull).len(), 1);
        assert!(summary.warnings(CommandId::Pull)[0].contains("ignored"));
    }
}
