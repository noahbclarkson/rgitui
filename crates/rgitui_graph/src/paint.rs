//! Graph primitives drawn as quads wherever a quad can draw them.
//!
//! gpui rasterizes every run of paths it cannot merge into a full-window,
//! multisampled intermediate texture, then resolves and composites it. One such
//! pass is cheap on a discrete GPU and expensive on an integrated one: on a
//! Ryzen laptop, nine per frame (what the graph used to produce with a handful
//! of rows on screen) was enough to back frames up in the GPU queue and push
//! hover feedback 50–150ms behind the pointer. Quads, by contrast, are instanced
//! and nearly free.
//!
//! So the rules for graph painting are:
//!
//! - Straight lines (`paint_vertical_lane`), dots and rings are quads. Only
//!   curves, and the small chevron beside a pinned node, are paths.
//! - Nothing a row paints leaves that row's bounds. gpui orders primitives by
//!   which earlier ones they overlap; a line that overshoots into the next row
//!   chains every row's draw order onto the one above, and paths at different
//!   orders separated by text cannot share a pass. Kept inside its row, every
//!   row's curves land on the same few orders and gpui draws all of them in one
//!   or two passes however many rows are visible.
//!
//! Unrounded, borderless quads take the shader's no-antialiasing fast path, so
//! line segments in adjacent rows tile without a seam.

use gpui::{point, px, size, BorderStyle, Bounds, Hsla, Pixels, Point, Window};

/// The square that circumscribes a circle of `radius` about `center`.
pub(crate) fn circle_bounds(center: Point<Pixels>, radius: Pixels) -> Bounds<Pixels> {
    Bounds {
        origin: point(center.x - radius, center.y - radius),
        size: size(radius * 2.0, radius * 2.0),
    }
}

/// The outer radius of a ring whose stroke of `stroke` width is centred on
/// `radius` — how a stroked circle path places its stroke, so a ring drawn
/// here sits where the path it replaces used to.
pub(crate) fn ring_outer_radius(radius: Pixels, stroke: Pixels) -> Pixels {
    radius + stroke / 2.0
}

/// Paints a filled circle as a fully rounded quad.
pub(crate) fn paint_disc(window: &mut Window, center: Point<Pixels>, radius: Pixels, color: Hsla) {
    window.paint_quad(gpui::quad(
        circle_bounds(center, radius),
        radius,
        color,
        px(0.0),
        gpui::transparent_black(),
        BorderStyle::Solid,
    ));
}

/// Paints a circular ring as a fully rounded, bordered quad with no fill.
/// `radius` is to the middle of the stroke, as with a stroked circle path.
pub(crate) fn paint_ring(
    window: &mut Window,
    center: Point<Pixels>,
    radius: Pixels,
    stroke: Pixels,
    color: Hsla,
    style: BorderStyle,
) {
    let outer = ring_outer_radius(radius, stroke);
    window.paint_quad(gpui::quad(
        circle_bounds(center, outer),
        outer,
        gpui::transparent_black(),
        stroke,
        color,
        style,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circle_bounds_are_the_circumscribed_square() {
        let b = circle_bounds(point(px(16.0), px(16.0)), px(4.0));
        assert_eq!(b.origin, point(px(12.0), px(12.0)));
        assert_eq!(b.size, size(px(8.0), px(8.0)));
    }

    #[test]
    fn ring_stroke_is_centred_on_radius() {
        assert_eq!(ring_outer_radius(px(9.5), px(2.5)), px(10.75));
    }

    #[test]
    fn occluding_ring_fits_inside_a_row_at_every_compactness() {
        // The occluding disc behind each node must not cross into the
        // neighbouring rows; see the module docs for why that matters.
        for mul in [0.75_f32, 1.0, 1.25] {
            let row_height = 32.0 * mul;
            let ring_r = 14.0 * mul;
            assert!(ring_r * 2.0 <= row_height, "mul {mul}");
        }
    }
}
