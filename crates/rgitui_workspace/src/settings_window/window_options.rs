use gpui::{
    point, px, App, Bounds, Pixels, Point, Size, TitlebarOptions, WindowBounds, WindowDecorations,
    WindowOptions,
};
use rgitui_settings::{SavedWindowBounds, SettingsState};

const DEFAULT_WIDTH: f32 = 880.0;
const DEFAULT_HEIGHT: f32 = 640.0;

/// Default `WindowOptions` for the standalone settings window.
///
/// Restores the last persisted bounds when they fall within a connected
/// display; otherwise centers a fixed-size, server-decorated window. A unique
/// `app_id` lets platform window managers distinguish it from the main rgitui
/// window.
pub fn settings_window_options(cx: &mut App) -> WindowOptions {
    let bounds = restore_or_center_bounds(cx);

    WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("rgitui — Settings".into()),
            appears_transparent: false,
            ..Default::default()
        }),
        window_min_size: Some(Size {
            width: px(720.0),
            height: px(560.0),
        }),
        window_decorations: Some(WindowDecorations::Server),
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        focus: true,
        show: true,
        app_id: Some("rgitui-settings".into()),
        ..Default::default()
    }
}

fn restore_or_center_bounds(cx: &mut App) -> Bounds<Pixels> {
    let saved = cx
        .try_global::<SettingsState>()
        .and_then(|state| state.settings_window_bounds());

    if let Some(saved) = saved {
        if saved.width > 0.0 && saved.height > 0.0 && origin_on_any_display(cx, saved) {
            return Bounds {
                origin: point(px(saved.x), px(saved.y)),
                size: Size {
                    width: px(saved.width),
                    height: px(saved.height),
                },
            };
        }
    }

    Bounds::centered(
        None,
        Size {
            width: px(DEFAULT_WIDTH),
            height: px(DEFAULT_HEIGHT),
        },
        cx,
    )
}

fn origin_on_any_display(cx: &App, saved: SavedWindowBounds) -> bool {
    let origin = Point {
        x: px(saved.x),
        y: px(saved.y),
    };
    cx.displays()
        .iter()
        .any(|display| display.bounds().contains(&origin))
}
