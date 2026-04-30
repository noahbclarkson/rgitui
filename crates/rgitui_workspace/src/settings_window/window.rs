use gpui::prelude::*;
use gpui::{
    div, px, Bounds, ClickEvent, Context, Entity, FocusHandle, KeyDownEvent, Pixels, Render,
    Subscription, Window,
};
use rgitui_settings::{SavedWindowBounds, SettingsState};
use rgitui_theme::{ActiveTheme, StyledExt, ThemeState};
use rgitui_ui::{Button, ButtonSize, ButtonStyle, IconName};

use super::events::SettingsViewEvent;
use super::view::SettingsView;

/// Top-level entity rendered into a dedicated OS window that hosts the
/// shared `SettingsView`. Keeps the inner view focused, repaints on global
/// theme/settings changes, and provides its own Close affordance and Esc
/// handling — independent of the main workspace window.
pub struct SettingsWindow {
    view: Entity<SettingsView>,
    focus: FocusHandle,
    last_saved_bounds: Option<Bounds<Pixels>>,
    _theme_observer: Subscription,
    _settings_observer: Subscription,
    _view_subscription: Subscription,
}

impl SettingsWindow {
    /// Construct a `SettingsWindow`. Must be called inside the new window's
    /// `cx.open_window` callback so the inner `SettingsView` (and its
    /// `TextInput` children) bind to the new window's focus tree.
    pub fn new(cx: &mut Context<Self>) -> Self {
        let view = cx.new(SettingsView::new);
        view.update(cx, |v, cx| v.reload_from_settings(cx));

        let theme_observer = cx.observe_global::<ThemeState>(|_, cx| cx.notify());
        let settings_observer = cx.observe_global::<SettingsState>(|_, cx| cx.notify());

        let view_subscription =
            cx.subscribe(&view, |_this, _view, _event: &SettingsViewEvent, cx| {
                cx.notify();
            });

        Self {
            view,
            focus: cx.focus_handle(),
            last_saved_bounds: None,
            _theme_observer: theme_observer,
            _settings_observer: settings_observer,
            _view_subscription: view_subscription,
        }
    }

    /// Update the in-memory bounds in `SettingsState` if they changed since the
    /// last render. The on-disk write is deferred to window close so drag-resize
    /// doesn't trigger a `state.save()` (and its synchronous keyring fetch) per
    /// frame.
    fn persist_bounds_if_changed(&mut self, window: &Window, cx: &mut Context<Self>) {
        let current = window.bounds();
        if self
            .last_saved_bounds
            .map(|prev| prev == current)
            .unwrap_or(false)
        {
            return;
        }

        let saved = SavedWindowBounds {
            x: current.origin.x.into(),
            y: current.origin.y.into(),
            width: current.size.width.into(),
            height: current.size.height.into(),
        };

        let initial = self.last_saved_bounds.is_none();
        self.last_saved_bounds = Some(current);

        if initial {
            return;
        }

        if !cx.has_global::<SettingsState>() {
            return;
        }

        cx.update_global::<SettingsState, _>(|state, _| {
            state.set_settings_window_bounds(Some(saved));
        });
    }

    /// Handle to the inner `SettingsView` entity. Useful for plumbing events
    /// across windows in later phases.
    pub fn view(&self) -> &Entity<SettingsView> {
        &self.view
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        if event.keystroke.key.as_str() == "escape" {
            window.remove_window();
        }
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.persist_bounds_if_changed(window, cx);

        let background = cx.colors().background;
        let border_variant = cx.colors().border_variant;
        let body = self
            .view
            .update(cx, |view, cx| view.render_window_body(window, cx));

        div()
            .id("settings-window-root")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::handle_key_down))
            .v_flex()
            .size_full()
            .bg(background)
            .child(div().flex_1().min_h_0().child(body))
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .justify_end()
                    .gap(px(8.))
                    .p_3()
                    .border_t_1()
                    .border_color(border_variant)
                    .child(
                        Button::new("settings-window-close", "Close")
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Default)
                            .icon(IconName::X)
                            .on_click(cx.listener(|_, _: &ClickEvent, window, _| {
                                window.remove_window();
                            })),
                    ),
            )
    }
}
