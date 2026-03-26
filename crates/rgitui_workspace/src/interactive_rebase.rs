use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, FontWeight, KeyDownEvent,
    Render, SharedString, Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Button, ButtonSize, ButtonStyle, Label, LabelSize};

/// The action to perform on a commit during interactive rebase.
#[derive(Debug, Clone, PartialEq)]
pub enum RebaseAction {
    Pick,
    Reword(String),
    Squash,
    Fixup,
    Drop,
}

impl RebaseAction {
    /// Short display label for the action.
    fn label(&self) -> &'static str {
        match self {
            RebaseAction::Pick => "Pick",
            RebaseAction::Reword(_) => "Reword",
            RebaseAction::Squash => "Squash",
            RebaseAction::Fixup => "Fixup",
            RebaseAction::Drop => "Drop",
        }
    }

    /// Cycle to the next action.
    fn next(&self) -> RebaseAction {
        match self {
            RebaseAction::Pick => RebaseAction::Reword(String::new()),
            RebaseAction::Reword(_) => RebaseAction::Squash,
            RebaseAction::Squash => RebaseAction::Fixup,
            RebaseAction::Fixup => RebaseAction::Drop,
            RebaseAction::Drop => RebaseAction::Pick,
        }
    }

    /// Color for this action label.
    fn color(&self) -> Color {
        match self {
            RebaseAction::Pick => Color::Success,
            RebaseAction::Reword(_) => Color::Accent,
            RebaseAction::Squash => Color::Warning,
            RebaseAction::Fixup => Color::Info,
            RebaseAction::Drop => Color::Deleted,
        }
    }
}

/// A single commit entry in the interactive rebase list.
#[derive(Debug, Clone)]
pub struct RebaseEntry {
    pub oid: String,
    pub original_message: String,
    pub author: String,
    pub action: RebaseAction,
}

/// Events emitted by the interactive rebase dialog.
#[derive(Debug, Clone)]
pub enum InteractiveRebaseEvent {
    Execute(Vec<RebaseEntry>),
    Cancel,
}

/// Interactive rebase modal dialog.
pub struct InteractiveRebase {
    visible: bool,
    entries: Vec<RebaseEntry>,
    target_ref: String,
    selected_index: usize,
    /// When editing a reword message, this holds (entry index, message, cursor position).
    editing_reword: Option<(usize, String, usize)>,
    focus_handle: FocusHandle,
}

impl EventEmitter<InteractiveRebaseEvent> for InteractiveRebase {}

impl InteractiveRebase {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            visible: false,
            entries: Vec::new(),
            target_ref: String::new(),
            selected_index: 0,
            editing_reword: None,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Show the dialog with a list of commits to rebase.
    pub fn show(
        &mut self,
        entries: Vec<RebaseEntry>,
        target_ref: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.entries = entries;
        self.target_ref = target_ref.into();
        self.selected_index = 0;
        self.editing_reword = None;
        self.visible = true;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// Show without focusing (for contexts where Window is unavailable).
    pub fn show_visible(
        &mut self,
        entries: Vec<RebaseEntry>,
        target_ref: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        self.entries = entries;
        self.target_ref = target_ref.into();
        self.selected_index = 0;
        self.editing_reword = None;
        self.visible = true;
        cx.notify();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        self.entries.clear();
        self.editing_reword = None;
        cx.emit(InteractiveRebaseEvent::Cancel);
        cx.notify();
    }

    fn execute(&mut self, cx: &mut Context<Self>) {
        // Finalize any in-progress reword editing
        if let Some((idx, ref msg, _)) = self.editing_reword {
            if idx < self.entries.len() {
                self.entries[idx].action = RebaseAction::Reword(msg.clone());
            }
        }
        self.editing_reword = None;

        let entries = self.entries.clone();
        self.visible = false;
        self.entries.clear();
        cx.emit(InteractiveRebaseEvent::Execute(entries));
        cx.notify();
    }

    fn set_action_on_selected(&mut self, action: RebaseAction, cx: &mut Context<Self>) {
        if let Some(entry) = self.entries.get_mut(self.selected_index) {
            // If switching to reword, start editing
            if matches!(&action, RebaseAction::Reword(_)) {
                let msg = entry.original_message.clone();
                let len = msg.len();
                self.editing_reword = Some((self.selected_index, msg, len));
            } else {
                // If we were editing this entry's reword, cancel editing
                if let Some((edit_idx, _, _)) = &self.editing_reword {
                    if *edit_idx == self.selected_index {
                        self.editing_reword = None;
                    }
                }
            }
            entry.action = action;
            cx.notify();
        }
    }

    fn cycle_action(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(entry) = self.entries.get_mut(index) {
            let next = entry.action.next();
            // If cycling to reword, start editing
            if matches!(&next, RebaseAction::Reword(_)) {
                let msg = entry.original_message.clone();
                let len = msg.len();
                self.editing_reword = Some((index, msg, len));
            } else {
                // If we were editing this entry's reword, cancel editing
                if let Some((edit_idx, _, _)) = &self.editing_reword {
                    if *edit_idx == index {
                        self.editing_reword = None;
                    }
                }
            }
            entry.action = next;
            cx.notify();
        }
    }

    fn move_entry_up(&mut self, cx: &mut Context<Self>) {
        if self.selected_index > 0 && self.selected_index < self.entries.len() {
            self.entries
                .swap(self.selected_index, self.selected_index - 1);
            // Update editing index if needed
            if let Some((ref mut edit_idx, _, _)) = self.editing_reword {
                if *edit_idx == self.selected_index {
                    *edit_idx -= 1;
                } else if *edit_idx == self.selected_index - 1 {
                    *edit_idx += 1;
                }
            }
            self.selected_index -= 1;
            cx.notify();
        }
    }

    fn move_entry_down(&mut self, cx: &mut Context<Self>) {
        if self.selected_index + 1 < self.entries.len() {
            self.entries
                .swap(self.selected_index, self.selected_index + 1);
            // Update editing index if needed
            if let Some((ref mut edit_idx, _, _)) = self.editing_reword {
                if *edit_idx == self.selected_index {
                    *edit_idx += 1;
                } else if *edit_idx == self.selected_index + 1 {
                    *edit_idx -= 1;
                }
            }
            self.selected_index += 1;
            cx.notify();
        }
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        let key = keystroke.key.as_str();
        let modifiers = &keystroke.modifiers;

        // If editing a reword message, handle text input
        if let Some((_, ref mut msg, ref mut cursor)) = self.editing_reword {
            match key {
                "escape" => {
                    let Some((idx, _, _)) = self.editing_reword.take() else {
                        return;
                    };
                    if let Some(entry) = self.entries.get_mut(idx) {
                        entry.action = RebaseAction::Pick;
                    }
                    cx.notify();
                    return;
                }
                "enter" => {
                    let Some((idx, msg, _)) = self.editing_reword.take() else {
                        return;
                    };
                    if let Some(entry) = self.entries.get_mut(idx) {
                        entry.action = RebaseAction::Reword(msg);
                    }
                    cx.notify();
                    return;
                }
                "backspace" => {
                    if *cursor > 0 {
                        *cursor -= 1;
                        msg.remove(*cursor);
                    }
                    cx.notify();
                    return;
                }
                "delete" => {
                    if *cursor < msg.len() {
                        msg.remove(*cursor);
                    }
                    cx.notify();
                    return;
                }
                "left" => {
                    if *cursor > 0 {
                        *cursor -= 1;
                    }
                    cx.notify();
                    return;
                }
                "right" => {
                    if *cursor < msg.len() {
                        *cursor += 1;
                    }
                    cx.notify();
                    return;
                }
                "home" => {
                    *cursor = 0;
                    cx.notify();
                    return;
                }
                "end" => {
                    *cursor = msg.len();
                    cx.notify();
                    return;
                }
                _ => {
                    if let Some(key_char) = &keystroke.key_char {
                        msg.insert_str(*cursor, key_char);
                        *cursor += key_char.len();
                        cx.notify();
                        return;
                    } else if key.len() == 1 && !modifiers.control && !modifiers.platform {
                        let Some(ch) = key.chars().next() else {
                            return;
                        };
                        if ch.is_ascii_graphic() || ch == ' ' {
                            msg.insert(*cursor, ch);
                            *cursor += 1;
                            cx.notify();
                            return;
                        }
                    }
                    return;
                }
            }
        }

        // Normal mode key handling
        match key {
            "escape" => {
                self.dismiss(cx);
            }
            "enter" => {
                self.execute(cx);
            }
            "up" | "k" if !modifiers.control => {
                if self.selected_index > 0 {
                    self.selected_index -= 1;
                    cx.notify();
                }
            }
            "down" | "j" if !modifiers.control => {
                if self.selected_index + 1 < self.entries.len() {
                    self.selected_index += 1;
                    cx.notify();
                }
            }
            "up" if modifiers.control => {
                self.move_entry_up(cx);
            }
            "down" if modifiers.control => {
                self.move_entry_down(cx);
            }
            _ => {
                // Action shortcuts
                if !modifiers.control && !modifiers.platform {
                    if let Some(key_char) = keystroke.key_char.as_deref().or(Some(key)) {
                        match key_char {
                            "p" => self.set_action_on_selected(RebaseAction::Pick, cx),
                            "r" => {
                                self.set_action_on_selected(RebaseAction::Reword(String::new()), cx)
                            }
                            "s" => self.set_action_on_selected(RebaseAction::Squash, cx),
                            "f" => self.set_action_on_selected(RebaseAction::Fixup, cx),
                            "d" => self.set_action_on_selected(RebaseAction::Drop, cx),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

impl Render for InteractiveRebase {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        if !self.visible {
            return div().id("interactive-rebase").into_any_element();
        }

        let entry_count = self.entries.len();
        let header_text: SharedString =
            format!("Rebasing {} commits onto {}", entry_count, self.target_ref).into();

        // Build commit rows
        let mut rows = div()
            .id("rebase-entries")
            .v_flex()
            .w_full()
            .overflow_y_scroll()
            .max_h(px(400.));

        for (idx, entry) in self.entries.iter().enumerate() {
            let is_selected = idx == self.selected_index;
            let oid_short: SharedString =
                SharedString::from(entry.oid[..7.min(entry.oid.len())].to_string());

            let action_label: SharedString = entry.action.label().into();
            let action_color = entry.action.color();
            let is_dropped = matches!(entry.action, RebaseAction::Drop);

            // Determine what message to display
            let is_editing = self
                .editing_reword
                .as_ref()
                .is_some_and(|(edit_idx, _, _)| *edit_idx == idx);

            let display_msg: SharedString = if let Some((_, ref msg, _)) =
                self.editing_reword.as_ref().filter(|_| is_editing)
            {
                if msg.is_empty() {
                    "Enter new commit message...".into()
                } else {
                    SharedString::from(msg.clone())
                }
            } else if let RebaseAction::Reword(ref msg) = entry.action {
                if msg.is_empty() {
                    SharedString::from(entry.original_message.clone())
                } else {
                    SharedString::from(msg.clone())
                }
            } else {
                SharedString::from(entry.original_message.clone())
            };

            let msg_color = if is_dropped {
                Color::Disabled
            } else if is_editing {
                if self
                    .editing_reword
                    .as_ref()
                    .is_some_and(|(_, msg, _)| msg.is_empty())
                {
                    Color::Placeholder
                } else {
                    Color::Default
                }
            } else {
                Color::Default
            };

            let row_bg = if is_selected {
                colors.ghost_element_selected
            } else {
                gpui::Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 0.0,
                    a: 0.0,
                }
            };

            let action_bg = colors.element_background;
            let action_border = colors.border_variant;

            let idx_for_click = idx;

            let mut row = div()
                .id(ElementId::NamedInteger("rebase-entry".into(), idx as u64))
                .h_flex()
                .w_full()
                .h(px(36.))
                .px_3()
                .gap_2()
                .items_center()
                .cursor_pointer()
                .bg(row_bg)
                .hover(|s| s.bg(colors.ghost_element_hover))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.selected_index = idx_for_click;
                    cx.notify();
                }));

            // Action badge (clickable to cycle)
            let idx_for_action = idx;
            row = row.child(
                div()
                    .id(ElementId::NamedInteger("rebase-action".into(), idx as u64))
                    .h_flex()
                    .items_center()
                    .justify_center()
                    .w(px(64.))
                    .h(px(22.))
                    .px(px(6.))
                    .bg(action_bg)
                    .border_1()
                    .border_color(action_border)
                    .rounded(px(4.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.cycle_action(idx_for_action, cx);
                    }))
                    .child(
                        Label::new(action_label)
                            .size(LabelSize::XSmall)
                            .weight(FontWeight::BOLD)
                            .color(action_color),
                    ),
            );

            // OID
            row = row.child(
                Label::new(oid_short)
                    .size(LabelSize::XSmall)
                    .weight(FontWeight::MEDIUM)
                    .color(Color::Accent),
            );

            // Message (or reword editor)
            if is_editing {
                let Some((_, ref msg, cursor)) = self.editing_reword.as_ref() else {
                    continue;
                };
                let cursor = *cursor;
                let text_color = colors.text;
                let editor_bg = colors.editor_background;
                let border_focused = colors.border_focused;

                let mut input_row = div().h_flex().items_center().flex_1();

                if msg.is_empty() {
                    input_row = input_row
                        .child(div().w(px(2.)).h(px(14.)).bg(text_color))
                        .child(
                            Label::new("Enter new commit message...")
                                .size(LabelSize::Small)
                                .color(Color::Placeholder),
                        );
                } else {
                    let before = &msg[..cursor];
                    let cursor_char = if cursor < msg.len() {
                        &msg[cursor..cursor + 1]
                    } else {
                        ""
                    };
                    let after = if cursor + 1 < msg.len() {
                        &msg[cursor + 1..]
                    } else {
                        ""
                    };

                    if !before.is_empty() {
                        input_row = input_row.child(
                            Label::new(SharedString::from(before.to_string()))
                                .size(LabelSize::Small),
                        );
                    }
                    if !cursor_char.is_empty() {
                        input_row = input_row.child(
                            div().bg(text_color).child(
                                Label::new(SharedString::from(cursor_char.to_string()))
                                    .size(LabelSize::Small)
                                    .color(Color::Custom(gpui::Hsla {
                                        h: 0.0,
                                        s: 0.0,
                                        l: 0.0,
                                        a: 1.0,
                                    })),
                            ),
                        );
                    } else {
                        input_row = input_row.child(div().w(px(2.)).h(px(14.)).bg(text_color));
                    }
                    if !after.is_empty() {
                        input_row = input_row.child(
                            Label::new(SharedString::from(after.to_string()))
                                .size(LabelSize::Small),
                        );
                    }
                }

                row = row.child(
                    div()
                        .flex_1()
                        .h(px(24.))
                        .px_1()
                        .bg(editor_bg)
                        .border_1()
                        .border_color(border_focused)
                        .rounded(px(4.))
                        .h_flex()
                        .items_center()
                        .child(input_row),
                );
            } else {
                // Strikethrough for dropped commits
                if is_dropped {
                    row = row.child(
                        div().flex_1().child(
                            Label::new(display_msg)
                                .size(LabelSize::Small)
                                .color(msg_color)
                                .strikethrough(),
                        ),
                    );
                } else {
                    row = row.child(
                        div().flex_1().child(
                            Label::new(display_msg)
                                .size(LabelSize::Small)
                                .color(msg_color),
                        ),
                    );
                }
            }

            // Author
            let author: SharedString = entry.author.clone().into();
            row = row.child(
                Label::new(author)
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            );

            rows = rows.child(row);
        }

        // Build the modal
        let modal = div()
            .id("interactive-rebase-modal")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_mouse_down(
                gpui::MouseButton::Left,
                |_: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                },
            )
            .v_flex()
            .w(px(700.))
            .max_h(px(560.))
            .elevation_3(cx)
            .p_4()
            .gap_3()
            // Title
            .child(
                Label::new("Interactive Rebase")
                    .size(LabelSize::Large)
                    .weight(FontWeight::BOLD)
                    .color(Color::Default),
            )
            // Subtitle
            .child(
                Label::new(header_text)
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            // Commit list
            .child(rows)
            // Keyboard hints
            .child(
                div()
                    .h_flex()
                    .gap_3()
                    .pt_1()
                    .child(
                        Label::new("j/k Navigate")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("Ctrl+Up/Down Reorder")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("p/r/s/f/d Set action")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
            // Buttons
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .justify_end()
                    .gap_2()
                    .pt_2()
                    .child(
                        Button::new("rebase-cancel", "Cancel")
                            .size(ButtonSize::Default)
                            .style(ButtonStyle::Subtle)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.dismiss(cx);
                            })),
                    )
                    .child(
                        Button::new("rebase-start", "Start Rebase")
                            .size(ButtonSize::Default)
                            .style(ButtonStyle::Filled)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.execute(cx);
                            })),
                    ),
            );

        // Backdrop
        div()
            .id("interactive-rebase-backdrop")
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
                a: 0.5,
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.dismiss(cx);
            }))
            .child(modal)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── RebaseAction::label ────────────────────────────────────────

    #[test]
    fn rebase_action_label_pick() {
        assert_eq!(RebaseAction::Pick.label(), "Pick");
    }

    #[test]
    fn rebase_action_label_reword() {
        assert_eq!(RebaseAction::Reword("fix bug".into()).label(), "Reword");
    }

    #[test]
    fn rebase_action_label_squash() {
        assert_eq!(RebaseAction::Squash.label(), "Squash");
    }

    #[test]
    fn rebase_action_label_fixup() {
        assert_eq!(RebaseAction::Fixup.label(), "Fixup");
    }

    #[test]
    fn rebase_action_label_drop() {
        assert_eq!(RebaseAction::Drop.label(), "Drop");
    }

    // ── RebaseAction::next ─────────────────────────────────────────

    #[test]
    fn rebase_action_next_pick() {
        assert!(matches!(RebaseAction::Pick.next(), RebaseAction::Reword(_)));
    }

    #[test]
    fn rebase_action_next_reword() {
        assert!(matches!(
            RebaseAction::Reword("msg".into()).next(),
            RebaseAction::Squash
        ));
    }

    #[test]
    fn rebase_action_next_squash() {
        assert!(matches!(RebaseAction::Squash.next(), RebaseAction::Fixup));
    }

    #[test]
    fn rebase_action_next_fixup() {
        assert!(matches!(RebaseAction::Fixup.next(), RebaseAction::Drop));
    }

    #[test]
    fn rebase_action_next_drop() {
        assert!(matches!(RebaseAction::Drop.next(), RebaseAction::Pick));
    }

    #[test]
    fn rebase_action_next_cycles_all() {
        // Full cycle: Pick → Reword → Squash → Fixup → Drop → Pick
        let mut action = RebaseAction::Pick;
        action = action.next(); // Reword
        action = action.next(); // Squash
        action = action.next(); // Fixup
        action = action.next(); // Drop
        action = action.next(); // Pick
        assert!(matches!(action, RebaseAction::Pick));
    }

    // ── RebaseAction::color ────────────────────────────────────────

    #[test]
    fn rebase_action_color_pick() {
        assert!(matches!(RebaseAction::Pick.color(), Color::Success));
    }

    #[test]
    fn rebase_action_color_reword() {
        assert!(matches!(
            RebaseAction::Reword("msg".into()).color(),
            Color::Accent
        ));
    }

    #[test]
    fn rebase_action_color_squash() {
        assert!(matches!(RebaseAction::Squash.color(), Color::Warning));
    }

    #[test]
    fn rebase_action_color_fixup() {
        assert!(matches!(RebaseAction::Fixup.color(), Color::Info));
    }

    #[test]
    fn rebase_action_color_drop() {
        assert!(matches!(RebaseAction::Drop.color(), Color::Deleted));
    }

    // ── RebaseEntry ────────────────────────────────────────────────

    #[test]
    fn rebase_entry_clone() {
        let entry = RebaseEntry {
            oid: "abc123".into(),
            original_message: "fix: bug".into(),
            author: "Noah".into(),
            action: RebaseAction::Pick,
        };
        let cloned = entry.clone();
        assert_eq!(cloned.oid, entry.oid);
        assert_eq!(cloned.original_message, entry.original_message);
        assert_eq!(cloned.author, entry.author);
        assert_eq!(cloned.action, entry.action);
    }

    #[test]
    fn rebase_entry_action_is_pick() {
        let entry = RebaseEntry {
            oid: "abc123".into(),
            original_message: "fix: bug".into(),
            author: "Noah".into(),
            action: RebaseAction::Pick,
        };
        assert!(matches!(entry.action, RebaseAction::Pick));
    }

    #[test]
    fn rebase_entry_action_reword_carries_message() {
        let msg = "updated message".to_string();
        let entry = RebaseEntry {
            oid: "abc123".into(),
            original_message: "old message".into(),
            author: "Noah".into(),
            action: RebaseAction::Reword(msg.clone()),
        };
        if let RebaseAction::Reword(m) = &entry.action {
            assert_eq!(m, &msg);
        } else {
            panic!("expected Reword");
        }
    }
}
