//! Handing keyboard focus back when the element holding it stops being drawn.
//!
//! Every overlay and dialog focuses its own input when it opens, and closing one
//! only hides it. A hidden overlay draws nothing, so the element still holding
//! focus is missing from the frame. gpui then resolves keystrokes from the window
//! root, above the handlers the workspace attaches, and every shortcut silently
//! does nothing until the user clicks a panel. Swapping out a focused panel
//! strands focus the same way: Esc in the reflog puts the diff viewer back in its
//! place, and switching tab stops drawing the old tab altogether.
//!
//! The overlays close in many ways — the palette alone hides on Enter, on Esc, on
//! a click outside and on a click on a row, and a command it runs may open a
//! dialog of its own — and panels are swapped from as many places, so rather than
//! each of them handing focus back, the workspace watches from its render. Focus
//! is recorded on every frame with no overlay open, so the target is wherever the
//! user last was however the overlay was opened, and it is handed back only once
//! the last open overlay has closed.

use gpui::{Context, WeakFocusHandle, Window};

use super::layout::WORKSPACE_KEY_CONTEXT;
use super::{BottomPanelMode, FocusedPanel, Workspace};

/// What one frame changed about the open overlays.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OverlayTransition<T> {
    /// Nothing to hand back this frame.
    Steady,
    /// The last open overlay closed. Carries what held focus before the first of
    /// them opened, if anything did.
    Closed(Option<T>),
}

/// Remembers where focus was before an overlay opened.
///
/// Generic over the recorded target so the transitions can be tested without a
/// window.
pub(crate) struct OverlayFocus<T> {
    overlay_open: bool,
    return_to: Option<T>,
}

impl<T> Default for OverlayFocus<T> {
    fn default() -> Self {
        Self {
            overlay_open: false,
            return_to: None,
        }
    }
}

impl<T> OverlayFocus<T> {
    /// Advances by one frame.
    ///
    /// While no overlay is open, `focused` is recorded. Once one opens the
    /// recording is left as it is, so the overlay's own input never becomes the
    /// target, and a chain of overlays — a palette command that opens a dialog —
    /// returns to where the user started.
    pub(crate) fn observe(
        &mut self,
        overlay_open: bool,
        focused: impl FnOnce() -> Option<T>,
    ) -> OverlayTransition<T> {
        match (self.overlay_open, overlay_open) {
            (false, false) => {
                self.return_to = focused();
                OverlayTransition::Steady
            }
            (true, false) => {
                self.overlay_open = false;
                OverlayTransition::Closed(self.return_to.take())
            }
            (_, true) => {
                self.overlay_open = true;
                OverlayTransition::Steady
            }
        }
    }
}

/// Remembers which tab was drawn, and which view filled its bottom panel.
///
/// Generic over the tab's identity so the transitions can be tested without a
/// window.
pub(crate) struct DrawnPanels<T> {
    last: Option<(T, BottomPanelMode)>,
}

impl<T> Default for DrawnPanels<T> {
    fn default() -> Self {
        Self { last: None }
    }
}

impl<T: PartialEq> DrawnPanels<T> {
    /// Advances by one frame, given the active tab and its bottom panel.
    ///
    /// Returns the panel that took the place of whatever this frame stopped
    /// drawing: the graph of a newly active tab, or the bottom panel of the same
    /// tab when its view was swapped. `None` when nothing was swapped out.
    pub(crate) fn observe(&mut self, drawn: Option<(T, BottomPanelMode)>) -> Option<FocusedPanel> {
        let last = std::mem::replace(&mut self.last, drawn);
        let (tab, bottom_panel) = self.last.as_ref()?;
        match last {
            Some((last_tab, last_bottom_panel)) if last_tab == *tab => {
                (last_bottom_panel != *bottom_panel).then_some(FocusedPanel::DiffViewer)
            }
            _ => Some(FocusedPanel::Graph),
        }
    }
}

impl Workspace {
    /// Runs once per frame from `render`: records where focus is while no
    /// overlay is open, and hands it back after the last one closes or after the
    /// panel holding it is swapped out.
    pub(super) fn track_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let overlay_open = self.any_overlay_visible(cx);
        let transition = self.focus.overlay_focus.observe(overlay_open, || {
            window.focused(cx).map(|handle| handle.downgrade())
        });
        let drawn = self
            .tabs
            .get(self.active_tab)
            .map(|tab| (tab.project.entity_id(), tab.bottom_panel_mode));
        let replacement = self.focus.drawn_panels.observe(drawn);
        let hand_back = match transition {
            OverlayTransition::Closed(return_to) => Some((return_to, FocusedPanel::Graph)),
            // An open overlay holds focus itself; a panel swapped out underneath
            // it is dealt with once the overlay closes.
            OverlayTransition::Steady if overlay_open => None,
            OverlayTransition::Steady => replacement.map(|panel| (None, panel)),
        };
        if let Some((return_to, fallback)) = hand_back {
            // Keystrokes resolve against the last frame drawn, which until this
            // one finishes still has the overlay or the old panel in it. Whether
            // focus has been stranded can only be told once this frame is drawn.
            cx.on_next_frame(window, move |this, window, cx| {
                this.return_focus(return_to, fallback, window, cx);
            });
        }
    }

    /// Hands stranded focus back to `return_to`, or to `fallback` when that
    /// element was not drawn either.
    ///
    /// Focus that already reaches the workspace is left alone: something took it
    /// on purpose. After an overlay closes, `return_to` is where the user was
    /// before it opened and `fallback` is the commit graph, which every tab
    /// draws, because the overlay may have run a command that switched tab or
    /// swapped the panel out. After a panel is swapped out there is nothing to
    /// go back to, and focus moves to the panel that replaced it.
    fn return_focus(
        &mut self,
        return_to: Option<WeakFocusHandle>,
        fallback: FocusedPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.any_overlay_visible(cx) || focus_reaches_workspace(window) {
            return;
        }
        log::debug!(
            "workspace: returning stranded focus (was {:?})",
            window.focused(cx)
        );
        if let Some(handle) = return_to.and_then(|handle| handle.upgrade()) {
            window.focus(&handle, cx);
        }
        if !focus_reaches_workspace(window) {
            self.focus_panel(fallback, window, cx);
        }
    }
}

/// Whether keystrokes reach the workspace's handlers.
///
/// gpui looks the focused element up in the last frame drawn and dispatches from
/// the window root when it is not there, so this holds only when that frame drew
/// the focused element somewhere under the workspace root.
fn focus_reaches_workspace(window: &Window) -> bool {
    window
        .context_stack()
        .iter()
        .any(|context| context.contains(WORKSPACE_KEY_CONTEXT))
}

#[cfg(test)]
mod tests {
    use super::{BottomPanelMode, DrawnPanels, FocusedPanel, OverlayFocus, OverlayTransition};

    #[test]
    fn returns_to_what_was_focused_before_the_overlay_opened() {
        let mut focus = OverlayFocus::default();
        assert_eq!(
            focus.observe(false, || Some("graph")),
            OverlayTransition::Steady
        );
        assert_eq!(
            focus.observe(true, || Some("palette input")),
            OverlayTransition::Steady
        );
        assert_eq!(
            focus.observe(false, || Some("palette input")),
            OverlayTransition::Closed(Some("graph"))
        );
    }

    #[test]
    fn records_the_latest_focus_however_the_overlay_was_opened() {
        let mut focus = OverlayFocus::default();
        focus.observe(false, || Some("sidebar"));
        focus.observe(false, || Some("graph"));
        focus.observe(true, || None);
        assert_eq!(
            focus.observe(false, || None),
            OverlayTransition::Closed(Some("graph"))
        );
    }

    #[test]
    fn waits_until_every_chained_overlay_has_closed() {
        let mut focus = OverlayFocus::default();
        focus.observe(false, || Some("graph"));
        focus.observe(true, || Some("palette input"));
        // "Create Branch" hid the palette in the same update that opened the
        // dialog, so no frame is drawn with neither on screen.
        assert_eq!(
            focus.observe(true, || Some("branch name")),
            OverlayTransition::Steady
        );
        assert_eq!(
            focus.observe(false, || Some("branch name")),
            OverlayTransition::Closed(Some("graph"))
        );
    }

    #[test]
    fn hands_focus_back_once_per_close() {
        let mut focus = OverlayFocus::default();
        focus.observe(false, || Some("graph"));
        focus.observe(true, || None);
        focus.observe(false, || None);
        assert_eq!(
            focus.observe(false, || Some("sidebar")),
            OverlayTransition::Steady
        );
        focus.observe(true, || None);
        assert_eq!(
            focus.observe(false, || None),
            OverlayTransition::Closed(Some("sidebar"))
        );
    }

    #[test]
    fn an_overlay_open_from_the_first_frame_has_nothing_to_return_to() {
        let mut focus = OverlayFocus::<&str>::default();
        focus.observe(true, || None);
        assert_eq!(
            focus.observe(false, || None),
            OverlayTransition::Closed(None)
        );
    }

    #[test]
    fn nothing_is_swapped_out_while_the_same_panels_are_drawn() {
        let mut drawn = DrawnPanels::default();
        drawn.observe(Some(("repo", BottomPanelMode::Diff)));
        assert_eq!(drawn.observe(Some(("repo", BottomPanelMode::Diff))), None);
    }

    #[test]
    fn swapping_the_bottom_view_hands_over_to_its_replacement() {
        let mut drawn = DrawnPanels::default();
        drawn.observe(Some(("repo", BottomPanelMode::Reflog)));
        assert_eq!(
            drawn.observe(Some(("repo", BottomPanelMode::Diff))),
            Some(FocusedPanel::DiffViewer)
        );
        assert_eq!(drawn.observe(Some(("repo", BottomPanelMode::Diff))), None);
    }

    #[test]
    fn switching_tab_hands_over_to_the_new_tab_graph() {
        let mut drawn = DrawnPanels::default();
        drawn.observe(Some(("first", BottomPanelMode::Diff)));
        assert_eq!(
            drawn.observe(Some(("second", BottomPanelMode::Blame))),
            Some(FocusedPanel::Graph)
        );
    }

    #[test]
    fn a_tab_opened_from_the_welcome_screen_hands_over_to_its_graph() {
        let mut drawn = DrawnPanels::default();
        assert_eq!(drawn.observe(None), None);
        assert_eq!(
            drawn.observe(Some(("repo", BottomPanelMode::Diff))),
            Some(FocusedPanel::Graph)
        );
    }

    #[test]
    fn closing_the_last_tab_has_nothing_to_hand_over_to() {
        let mut drawn = DrawnPanels::default();
        drawn.observe(Some(("repo", BottomPanelMode::Diff)));
        assert_eq!(drawn.observe(None), None);
    }
}
