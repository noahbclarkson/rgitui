use gpui::prelude::*;
use gpui::{
    canvas, div, px, App, Bounds, ClickEvent, Context, ElementId, EventEmitter, FocusHandle,
    FontWeight, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Render,
    SharedString, WeakEntity, Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Button, ButtonSize, ButtonStyle, Label, LabelSize, Tooltip};

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

/// Internal event carrying the entry index for a drag start.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct DragStartEvent(usize);

/// Interactive rebase modal dialog.
pub struct InteractiveRebase {
    visible: bool,
    entries: Vec<RebaseEntry>,
    target_ref: String,
    selected_index: usize,
    /// When editing a reword message, this holds (entry index, message, cursor position).
    editing_reword: Option<(usize, String, usize)>,
    focus_handle: FocusHandle,

    // Drag-to-reorder state
    /// Index of the entry being dragged.
    dragging_index: Option<usize>,
    /// Index where the dragged entry would be inserted.
    drag_hover_index: Option<usize>,
    /// Bounds of the entries container div (for mouse-to-slot conversion).
    /// These are the window-relative bounds of the visible viewport — they
    /// already reflect the container's scroll position, so no separate scroll
    /// offset tracking is needed for mouse-to-slot conversion.
    container_bounds: Bounds<Pixels>,
    /// Weak reference to self, for use in mouse event handlers.
    entity: WeakEntity<Self>,
}

const REBASE_ENTRY_HEIGHT: f32 = 36.0;

impl EventEmitter<InteractiveRebaseEvent> for InteractiveRebase {}
impl EventEmitter<DragStartEvent> for InteractiveRebase {}

impl InteractiveRebase {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            visible: false,
            entries: Vec::new(),
            target_ref: String::new(),
            selected_index: 0,
            editing_reword: None,
            focus_handle: cx.focus_handle(),
            dragging_index: None,
            drag_hover_index: None,
            container_bounds: Bounds::default(),
            entity: cx.weak_entity(),
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
        self.dragging_index = None;
        self.drag_hover_index = None;
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
        self.dragging_index = None;
        self.drag_hover_index = None;
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
        self.dragging_index = None;
        self.drag_hover_index = None;
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

        // Cancel any in-progress drag
        self.dragging_index = None;
        self.drag_hover_index = None;

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

    /// Start dragging an entry (called on mousedown of the drag handle).
    fn start_drag(&mut self, index: usize, cx: &mut Context<Self>) {
        self.dragging_index = Some(index);
        self.drag_hover_index = Some(index);
        cx.notify();
    }

    /// Update which slot the dragged entry is hovering over (called on mousemove).
    fn update_drag_hover(&mut self, mouse_y: Pixels, cx: &mut Context<Self>) {
        let Some(_drag_idx) = self.dragging_index else {
            return;
        };
        let bounds = self.container_bounds;

        // Convert window Y to container-relative Y.
        // bounds.origin is the window-relative top-left of the visible viewport,
        // so subtracting it gives the mouse position relative to the visible top —
        // no separate scroll offset needed.
        let rel_y = mouse_y - bounds.origin.y;
        let slot = (rel_y.as_f32() / REBASE_ENTRY_HEIGHT) as usize;
        let slot = slot.min(self.entries.len().saturating_sub(1));

        if self.drag_hover_index != Some(slot) {
            self.drag_hover_index = Some(slot);
            cx.notify();
        }
    }

    /// Apply a drag reorder: move the entry at `drag_idx` to `hover_idx`,
    /// shifting intermediate entries. `editing_index` tracks an optional
    /// active edit (e.g., reword) and is adjusted to follow the moved entry.
    pub(crate) fn apply_drag_reorder(
        entries: &mut Vec<RebaseEntry>,
        drag_idx: usize,
        hover_idx: usize,
        editing_index: &mut Option<usize>,
    ) {
        if drag_idx == hover_idx || drag_idx >= entries.len() || hover_idx >= entries.len() {
            return;
        }
        let entry = entries.remove(drag_idx);
        entries.insert(hover_idx, entry);

        if let Some(ref mut edit_idx) = *editing_index {
            if *edit_idx == drag_idx {
                *edit_idx = hover_idx;
            } else if drag_idx < hover_idx {
                // Dragged downward: entries between shift left → editing index may decrease
                if *edit_idx > drag_idx && *edit_idx <= hover_idx {
                    *edit_idx -= 1;
                }
            } else {
                // drag_idx > hover_idx: dragged upward, entries between shift right
                if *edit_idx >= hover_idx && *edit_idx < drag_idx {
                    *edit_idx += 1;
                }
            }
        }
    }

    /// Complete the drag operation (called on mouseup).
    fn end_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag_idx) = self.dragging_index else {
            return;
        };
        let Some(hover_idx) = self.drag_hover_index else {
            self.dragging_index = None;
            self.drag_hover_index = None;
            cx.notify();
            return;
        };

        if drag_idx != hover_idx {
            let mut edit_idx: Option<usize> = self.editing_reword.as_ref().map(|(i, _, _)| *i);
            Self::apply_drag_reorder(&mut self.entries, drag_idx, hover_idx, &mut edit_idx);
            if let Some(ref mut editing) = self.editing_reword {
                if let Some(idx) = edit_idx {
                    editing.0 = idx;
                }
            }
            // Keep selection on the moved entry
            self.selected_index = hover_idx;
        }

        self.dragging_index = None;
        self.drag_hover_index = None;
        cx.notify();
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
                // Cancel any in-progress drag, then dismiss
                if self.dragging_index.is_some() {
                    self.dragging_index = None;
                    self.drag_hover_index = None;
                    cx.notify();
                } else {
                    self.dismiss(cx);
                }
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

        // Bounds tracker for the entries container
        let bounds_tracker = cx.weak_entity();
        let entries_bounds_tracker = canvas(
            {
                let bounds_tracker = bounds_tracker.clone();
                move |bounds: Bounds<Pixels>, _: &mut Window, cx: &mut App| {
                    bounds_tracker
                        .update(cx, |this: &mut InteractiveRebase, _| {
                            this.container_bounds = bounds;
                        })
                        .ok();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        // Capture entity for drag handle callbacks
        let entity = self.entity.clone();

        // Build commit rows with drag-to-reorder support
        let mut rows = div()
            .id("rebase-entries")
            .v_flex()
            .w_full()
            .overflow_y_scroll()
            .max_h(px(400.))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.dragging_index.is_some() {
                    this.update_drag_hover(event.position.y, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &gpui::MouseUpEvent, _, cx| {
                    if this.dragging_index.is_some() {
                        this.end_drag(cx);
                    }
                }),
            );

        rows = rows.child(entries_bounds_tracker);

        let dragging_index = self.dragging_index;
        let drag_hover_index = self.drag_hover_index;

        for (idx, entry) in self.entries.iter().enumerate() {
            let is_selected = idx == self.selected_index;
            let is_dragging = dragging_index == Some(idx);
            let is_hover_target = drag_hover_index == Some(idx) && dragging_index != Some(idx);
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

            // Row background: drag hover gets accent tint + drop line; dragging gets dimmed; selected normal
            let row_bg = if is_hover_target {
                colors.ghost_element_selected
            } else if is_dragging {
                gpui::Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 0.0,
                    a: 0.0,
                }
            } else if is_selected {
                colors.ghost_element_selected
            } else {
                gpui::Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 0.0,
                    a: 0.0,
                }
            };

            // Drop indicator: 2px accent line at top of hovered row — shows where the
            // dragged item will land. Rendered as a top border so it "floats" visually
            // between the previous row and the hovered row.
            let drop_indicator = if is_hover_target && self.dragging_index.is_some() {
                Some(colors.text_accent)
            } else {
                None
            };

            let action_bg = colors.element_background;
            let action_border = colors.border_variant;

            let idx_for_click = idx;

            // Drag opacity: dragged item is dimmed
            let row_opacity = if is_dragging { 0.4 } else { 1.0 };

            let mut row = div()
                .id(ElementId::NamedInteger("rebase-entry".into(), idx as u64))
                .h_flex()
                .w_full()
                .h(px(36.))
                .px_3()
                .gap_2()
                .items_center()
                .opacity(row_opacity)
                .bg(row_bg)
                .border_t_2()
                .border_color(drop_indicator.unwrap_or(colors.border_transparent))
                .hover(|s| {
                    if !is_dragging {
                        s.bg(colors.ghost_element_hover)
                    } else {
                        s
                    }
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if this.dragging_index.is_none() {
                        this.selected_index = idx_for_click;
                        cx.notify();
                    }
                }));

            // Drag handle (grip icon — start drag on mousedown)
            let idx_for_drag = idx;
            let entity_for_drag = entity.clone();
            row = row.child(
                div()
                    .id(ElementId::NamedInteger(
                        "rebase-drag-handle".into(),
                        idx as u64,
                    ))
                    .w(px(16.))
                    .h_flex()
                    .items_center()
                    .justify_center()
                    .cursor_grab()
                    .tooltip(Tooltip::text("Drag to reorder"))
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                            entity_for_drag
                                .update(cx, |this, cx| {
                                    this.start_drag(idx_for_drag, cx);
                                })
                                .ok();
                        },
                    ),
            );

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

    // ── Drag-to-reorder ─────────────────────────────────────────────

    fn make_entries_abcde() -> Vec<RebaseEntry> {
        vec![
            RebaseEntry {
                oid: "aaa".into(),
                original_message: "commit A".into(),
                author: "A".into(),
                action: RebaseAction::Pick,
            },
            RebaseEntry {
                oid: "bbb".into(),
                original_message: "commit B".into(),
                author: "B".into(),
                action: RebaseAction::Pick,
            },
            RebaseEntry {
                oid: "ccc".into(),
                original_message: "commit C".into(),
                author: "C".into(),
                action: RebaseAction::Pick,
            },
            RebaseEntry {
                oid: "ddd".into(),
                original_message: "commit D".into(),
                author: "D".into(),
                action: RebaseAction::Pick,
            },
            RebaseEntry {
                oid: "eee".into(),
                original_message: "commit E".into(),
                author: "E".into(),
                action: RebaseAction::Pick,
            },
        ]
    }

    #[test]
    fn drag_reorder_moves_entry_downward() {
        // Drag entry at index 1 (B) to index 3 → [A, C, D, B, E]
        let mut entries = make_entries_abcde();
        let mut editing: Option<usize> = None;
        InteractiveRebase::apply_drag_reorder(&mut entries, 1, 3, &mut editing);
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[0].oid.as_str(), "aaa");
        assert_eq!(entries[1].oid.as_str(), "ccc"); // shifted left
        assert_eq!(entries[2].oid.as_str(), "ddd"); // shifted left
        assert_eq!(entries[3].oid.as_str(), "bbb"); // dropped here
        assert_eq!(entries[4].oid.as_str(), "eee");
    }

    #[test]
    fn drag_reorder_moves_entry_upward() {
        // Drag entry at index 3 (D) to index 1 → [A, D, B, C, E]
        let mut entries = make_entries_abcde();
        let mut editing: Option<usize> = None;
        InteractiveRebase::apply_drag_reorder(&mut entries, 3, 1, &mut editing);
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[0].oid.as_str(), "aaa");
        assert_eq!(entries[1].oid.as_str(), "ddd"); // dropped here
        assert_eq!(entries[2].oid.as_str(), "bbb"); // shifted right
        assert_eq!(entries[3].oid.as_str(), "ccc"); // shifted right
        assert_eq!(entries[4].oid.as_str(), "eee");
    }

    #[test]
    fn drag_reorder_same_position_is_noop() {
        let mut entries = make_entries_abcde();
        let mut editing: Option<usize> = None;
        InteractiveRebase::apply_drag_reorder(&mut entries, 2, 2, &mut editing);
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[2].oid.as_str(), "ccc"); // unchanged
    }

    #[test]
    fn drag_reorder_drag_out_of_bounds_is_noop() {
        let mut entries = make_entries_abcde();
        let mut editing: Option<usize> = None;
        InteractiveRebase::apply_drag_reorder(&mut entries, 99, 1, &mut editing);
        assert_eq!(entries[0].oid.as_str(), "aaa"); // all unchanged
    }

    #[test]
    fn drag_reorder_hover_out_of_bounds_is_noop() {
        let mut entries = make_entries_abcde();
        let mut editing: Option<usize> = None;
        InteractiveRebase::apply_drag_reorder(&mut entries, 1, 99, &mut editing);
        assert_eq!(entries[1].oid.as_str(), "bbb"); // all unchanged
    }

    #[test]
    fn drag_reorder_editing_index_follows_moved_entry_downward() {
        // Drag B (idx 1) to idx 3.
        // After remove(B): [A,C,D,E]. D at intermediate idx 2.
        // After insert(B@3): E (intermediate idx 3) is pushed to 4.
        // D stays at 2. The hover position (3 in original space) holds the inserted B.
        // D (original idx 3, i.e. between 1<3<=3) is decremented to 2.
        let mut entries = make_entries_abcde();
        let mut editing = Some(3); // D
        InteractiveRebase::apply_drag_reorder(&mut entries, 1, 3, &mut editing);
        assert_eq!(editing, Some(2));
    }

    #[test]
    fn drag_reorder_editing_index_follows_moved_entry_upward() {
        // Drag D (idx 3) to idx 1.
        // After remove(D): [A,B,C,E] — C is at idx 2 in intermediate.
        // After insert(D@1): [A,D,B,C,E] — C ends at idx 3.
        let mut entries = make_entries_abcde();
        let mut editing = Some(2); // C
        InteractiveRebase::apply_drag_reorder(&mut entries, 3, 1, &mut editing);
        // C (at original idx 2, which is between 1 and 3 in original array) shifts to 3
        assert_eq!(editing, Some(3));
    }

    #[test]
    fn drag_reorder_editing_index_follows_itself_when_moved() {
        // Drag B (idx 1) to idx 3; editing B itself
        let mut entries = make_entries_abcde();
        let mut editing = Some(1);
        InteractiveRebase::apply_drag_reorder(&mut entries, 1, 3, &mut editing);
        assert_eq!(editing, Some(3)); // B now at position 3
    }

    #[test]
    fn drag_reorder_editing_index_unchanged_when_not_involved() {
        // Drag B (idx 1) to idx 4; editing E (idx 4).
        // After remove(B): [A,C,D,E] — E at intermediate idx 3.
        // After insert(B@4): E at final idx 4 (pushed right by B). Entry at idx 4 (B) occupies that slot.
        // Condition (edit_idx > 1 && edit_idx <= 4): E at 4 satisfies (4>1 && 4<=4) → shift to 3.
        let mut entries = make_entries_abcde();
        let mut editing = Some(4); // E
        InteractiveRebase::apply_drag_reorder(&mut entries, 1, 4, &mut editing);
        assert_eq!(editing, Some(3)); // E shifted left by one (intermediate 3)
        assert_eq!(entries[3].oid.as_str(), "eee"); // E is now at 3, B at 4
    }

    #[test]
    fn drag_reorder_editing_index_none_is_handled() {
        let mut entries = make_entries_abcde();
        let mut editing: Option<usize> = None;
        InteractiveRebase::apply_drag_reorder(&mut entries, 0, 4, &mut editing);
        assert_eq!(editing, None); // no edit in progress
        assert_eq!(entries[4].oid.as_str(), "aaa"); // A is now last
    }
}
