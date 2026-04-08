use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{canvas, div, point, px, App, Bounds, Context, PathBuilder, Pixels, Render, Window};
use rgitui_theme::{lane_color, ActiveTheme, StyledExt};

// ─── Pre-computed trig ──────────────────────────────────────────────

fn circle_offsets() -> &'static [(f32, f32)] {
    static OFFSETS: OnceLock<Vec<(f32, f32)>> = OnceLock::new();
    OFFSETS.get_or_init(|| {
        (0..36)
            .map(|i| {
                let a = (i as f32) * std::f32::consts::TAU / 36.0;
                (a.cos(), a.sin())
            })
            .collect()
    })
}

fn build_dot(cx_x: Pixels, cy_y: Pixels, radius: f32) -> Option<gpui::Path<Pixels>> {
    let offsets = circle_offsets();
    let mut path = PathBuilder::fill();
    for (i, &(cos, sin)) in offsets.iter().enumerate() {
        let x = cx_x + px(radius * cos);
        let y = cy_y + px(radius * sin);
        if i == 0 {
            path.move_to(point(x, y));
        } else {
            path.line_to(point(x, y));
        }
    }
    path.close();
    path.build().ok()
}

// ─── Graph constants ────────────────────────────────────────────────

const CANVAS_W: f32 = 100.0;
const CANVAS_H: f32 = 220.0;
const LANE_SPACING: f32 = 22.0;
const ROW_HEIGHT: f32 = 28.0;
const DOT_RADIUS: f32 = 3.2;
const STROKE_W: f32 = 1.5;
const GLOW_W: f32 = 6.0;
const GLOW_ALPHA: f32 = 0.15;
const SEGMENTS: usize = 20;

// ─── Bezier helpers ─────────────────────────────────────────────────

fn bezier_pt(x0: f32, y0: f32, x1: f32, y1: f32, t: f32) -> (f32, f32) {
    let mid_y = y0 + (y1 - y0) * 0.5;
    let u = 1.0 - t;
    let bx = u * u * u * x0 + 3.0 * u * u * t * x0 + 3.0 * u * t * t * x1 + t * t * t * x1;
    let by = u * u * u * y0
        + 3.0 * u * u * t * mid_y
        + 3.0 * u * t * t * mid_y
        + t * t * t * y1;
    (bx, by)
}

/// Draw a path from a set of points with both glow and main stroke.
fn paint_line(window: &mut Window, pts: &[(f32, f32)], color: gpui::Hsla) {
    if pts.len() < 2 {
        return;
    }
    // Glow pass
    let glow_color = gpui::Hsla {
        a: GLOW_ALPHA,
        ..color
    };
    let mut glow = PathBuilder::stroke(px(GLOW_W));
    glow.move_to(point(px(pts[0].0), px(pts[0].1)));
    for &(x, y) in &pts[1..] {
        glow.line_to(point(px(x), px(y)));
    }
    if let Ok(built) = glow.build() {
        window.paint_path(built, glow_color);
    }
    // Main stroke
    let mut path = PathBuilder::stroke(px(STROKE_W));
    path.move_to(point(px(pts[0].0), px(pts[0].1)));
    for &(x, y) in &pts[1..] {
        path.line_to(point(px(x), px(y)));
    }
    if let Ok(built) = path.build() {
        window.paint_path(built, color);
    }
}

/// Draw an edge clipped so only the portion with Y >= min_y is visible.
/// (Bottom-to-top reveal: the reveal line sweeps upward.)
fn draw_edge_clipped(
    window: &mut Window,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    min_y: f32,
    color: gpui::Hsla,
) {
    // Edge goes from (x0,y0) at top to (x1,y1) at bottom.
    // We only draw the portion where y >= min_y.
    if y1 < min_y {
        return; // entirely above reveal line
    }

    let mut pts: Vec<(f32, f32)> = Vec::with_capacity(SEGMENTS + 1);

    if (x0 - x1).abs() < 0.1 {
        // Straight vertical
        let start_y = y0.max(min_y);
        pts.push((x0, start_y));
        pts.push((x1, y1));
    } else {
        // Collect bezier points, clipping to min_y
        let mut all: Vec<(f32, f32)> = Vec::with_capacity(SEGMENTS + 1);
        all.push((x0, y0));
        for i in 1..=SEGMENTS {
            let t = i as f32 / SEGMENTS as f32;
            all.push(bezier_pt(x0, y0, x1, y1, t));
        }

        for i in 0..all.len() {
            let (ax, ay) = all[i];
            if ay >= min_y {
                if pts.is_empty() && i > 0 {
                    // Interpolate entry point from previous segment
                    let (px_val, py) = all[i - 1];
                    if py < min_y {
                        let frac = (min_y - py) / (ay - py);
                        let clip_x = px_val + (ax - px_val) * frac;
                        pts.push((clip_x, min_y));
                    }
                }
                pts.push((ax, ay));
            }
        }
    }

    paint_line(window, &pts, color);
}

// ─── Graph layout ───────────────────────────────────────────────────

struct Row {
    lane: usize,
    color_idx: usize,
}

const ROWS: &[Row] = &[
    Row { lane: 0, color_idx: 0 },
    Row { lane: 0, color_idx: 0 },
    Row { lane: 1, color_idx: 1 },
    Row { lane: 0, color_idx: 0 },
    Row { lane: 1, color_idx: 1 },
    Row { lane: 0, color_idx: 0 },
    Row { lane: 2, color_idx: 2 },
    Row { lane: 0, color_idx: 0 },
];

struct Edge {
    from: usize,
    to: usize,
    color_idx: usize,
}

const EDGES: &[Edge] = &[
    Edge { from: 0, to: 1, color_idx: 0 },
    Edge { from: 1, to: 3, color_idx: 0 },
    Edge { from: 3, to: 5, color_idx: 0 },
    Edge { from: 5, to: 7, color_idx: 0 },
    Edge { from: 1, to: 2, color_idx: 1 },
    Edge { from: 2, to: 4, color_idx: 1 },
    Edge { from: 4, to: 5, color_idx: 1 },
    Edge { from: 5, to: 6, color_idx: 2 },
    Edge { from: 6, to: 7, color_idx: 2 },
];

fn paint_graph(bounds: &Bounds<Pixels>, window: &mut Window, progress: f32) {
    let ox = f32::from(bounds.origin.x);
    let oy = f32::from(bounds.origin.y);
    let center_x = CANVAS_W / 2.0;

    let lane_x = |lane: usize| -> f32 {
        let offset = match lane {
            0 => 0.0,
            1 => LANE_SPACING,
            2 => -LANE_SPACING,
            _ => 0.0,
        };
        ox + center_x + offset
    };
    let row_y = |row: usize| -> f32 { oy + ROW_HEIGHT * (row as f32 + 0.5) };

    // Reveal line sweeps from bottom to top.
    // At progress=0 min_y is at the very bottom (nothing visible).
    // At progress=1 min_y is at the very top (everything visible).
    let min_y = oy + CANVAS_H * (1.0 - progress);

    // Edges
    for edge in EDGES {
        let from = &ROWS[edge.from];
        let to = &ROWS[edge.to];
        draw_edge_clipped(
            window,
            lane_x(from.lane),
            row_y(edge.from),
            lane_x(to.lane),
            row_y(edge.to),
            min_y,
            lane_color(edge.color_idx),
        );
    }

    // Dots — appear with scale-in when the reveal reaches them
    for (i, row) in ROWS.iter().enumerate() {
        let dy = row_y(i);
        if dy < min_y {
            continue;
        }
        let color = lane_color(row.color_idx);
        let dx = lane_x(row.lane);

        let appear_frac = ((dy - min_y) / 14.0).min(1.0);
        let r = DOT_RADIUS * appear_frac;
        if r < 0.3 {
            continue;
        }

        // Glow
        let glow_color = gpui::Hsla {
            a: GLOW_ALPHA * appear_frac,
            ..color
        };
        if let Some(g) = build_dot(px(dx), px(dy), r + 3.0) {
            window.paint_path(g, glow_color);
        }
        // Fill
        if let Some(d) = build_dot(px(dx), px(dy), r) {
            window.paint_path(d, color);
        }
    }
}

// ─── Easing ─────────────────────────────────────────────────────────

fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

// ─── SplashScreen ───────────────────────────────────────────────────

pub struct SplashScreen {
    start: Instant,
}

const ANIM_DURATION: Duration = Duration::from_millis(1400);
const FADE_DURATION: Duration = Duration::from_millis(1000);

impl SplashScreen {
    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let done = this
                    .update(cx, |this: &mut SplashScreen, cx| {
                        let t =
                            this.start.elapsed().as_secs_f32() / ANIM_DURATION.as_secs_f32();
                        cx.notify();
                        t >= 1.0
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
        })
        .detach();

        Self {
            start: Instant::now(),
        }
    }
}

/// Build the typewriter text: returns `visible` characters in accent color
/// followed by the remaining characters in a dim muted color.
fn typewriter_text(
    full: &str,
    progress: f32,
    visible_color: gpui::Hsla,
    hidden_color: gpui::Hsla,
) -> gpui::AnyElement {
    let total = full.chars().count();
    let visible = ((total as f32) * progress).ceil() as usize;
    let shown: String = full.chars().take(visible).collect();
    let rest: String = full.chars().skip(visible).collect();

    div()
        .h_flex()
        .child(div().text_color(visible_color).child(shown))
        .child(div().text_color(hidden_color).child(rest))
        .into_any_element()
}

impl Render for SplashScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let elapsed = self.start.elapsed();
        let raw_t = (elapsed.as_secs_f32() / ANIM_DURATION.as_secs_f32()).min(1.0);
        let graph_progress = ease_out_cubic(raw_t);

        let fade_t = (elapsed.as_secs_f32() / FADE_DURATION.as_secs_f32()).min(1.0);
        let opacity = ease_out_cubic(fade_t);

        // Title typewriter: plays over the first 600ms
        let title_t = (elapsed.as_secs_f32() / 0.6).min(1.0);
        // Subtitle typewriter: starts at 300ms, plays over 500ms
        let subtitle_t =
            ((elapsed.as_secs_f32() - 0.3).max(0.0) / 0.5).min(1.0);

        let colors = cx.colors();
        let text_color = colors.text;
        let muted = colors.text_muted;
        let accent = colors.text_accent;
        let surface = colors.elevated_surface_background;
        let border = colors.border;

        // Very dim version of muted for not-yet-typed characters
        let hidden = gpui::Hsla {
            a: 0.08,
            ..muted
        };

        let title = div()
            .text_size(px(22.))
            .font_weight(gpui::FontWeight::BOLD)
            .child(typewriter_text("rgitui", title_t, text_color, hidden))
            .into_any_element();

        let subtitle = div()
            .text_size(px(11.))
            .child(typewriter_text(
                "GPU-accelerated Git",
                subtitle_t,
                muted,
                hidden,
            ))
            .into_any_element();

        let graph = div()
            .id("splash-graph")
            .w(px(CANVAS_W))
            .h(px(CANVAS_H))
            .mt(px(20.))
            .mb(px(12.))
            .child(canvas(
                |_bounds: Bounds<Pixels>, _window: &mut Window, _cx: &mut App| {},
                move |bounds: Bounds<Pixels>, _: (), window: &mut Window, _cx: &mut App| {
                    paint_graph(&bounds, window, graph_progress);
                },
            ))
            .into_any_element();

        // Loading text typewriter: starts at 500ms, plays over 400ms
        let loading_t =
            ((elapsed.as_secs_f32() - 0.5).max(0.0) / 0.4).min(1.0);

        let loading = div()
            .text_size(px(10.))
            .child(typewriter_text(
                "Loading workspace\u{2026}",
                loading_t,
                accent,
                hidden,
            ))
            .into_any_element();

        div()
            .id("splash-screen")
            .size_full()
            .bg(surface)
            .border_1()
            .border_color(border)
            .rounded(px(8.))
            .v_flex()
            .items_center()
            .justify_center()
            .gap(px(4.))
            .opacity(opacity)
            .child(title)
            .child(subtitle)
            .child(graph)
            .child(loading)
    }
}
