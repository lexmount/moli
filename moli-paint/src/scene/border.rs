// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Narrowly adapted from DioxusLabs/blitz commit
// d788124ab881f9bb537cb452ec1d837604a374a8:
// - examples/screenshot.rs
// - packages/blitz-paint/src/render.rs
// - crates/anyrender_svg (parsed SVG tree execution)
// The source project is licensed MIT OR Apache-2.0.

//! Border styles and edge geometry shared by paint backends.

use super::to_backend_color;
use crate::geometry::{normalize_radii, rounded_rect_path, to_backend_rect};
use anyrender::PaintScene;
use moli_layout::{
    PaintBorderColors, PaintBorderStyle, PaintBorderStyles, PaintColor, PaintCornerRadii,
    PaintCornerRadius, PaintEdgeSizes, PaintRect, pixel_snap_paint_axis,
    pixel_snap_paint_axis_allowing_zero,
};
use peniko::Fill;
use peniko::kurbo::Shape as _;
use peniko::kurbo::{
    Affine, BezPath, Cap, Circle, Ellipse, Insets, Join, PathEl, Point, Rect, RoundedRect,
    RoundedRectRadii, Stroke,
};

pub(super) fn paint_border(
    scene: &mut impl PaintScene,
    rect: PaintRect,
    widths: PaintEdgeSizes,
    colors: PaintBorderColors,
    styles: PaintBorderStyles,
    radii: PaintCornerRadii,
    transform: Affine,
    _device_scale: f64,
) {
    let Some(rect) = sanitize_border_rect(rect) else {
        return;
    };
    let widths = sanitize_border_widths(widths, rect.width() as f32, rect.height() as f32);
    let Some((rect, widths)) = pixel_snapped_border_geometry(rect, widths) else {
        return;
    };
    let radii = normalize_radii(radii, rect.width(), rect.height());
    if paint_uniform_solid_border(scene, rect, widths, colors, styles, radii, transform) {
        return;
    }
    let edges = [
        (BorderEdge::Top, widths.top, colors.top, styles.top),
        (BorderEdge::Right, widths.right, colors.right, styles.right),
        (
            BorderEdge::Bottom,
            widths.bottom,
            colors.bottom,
            styles.bottom,
        ),
        (BorderEdge::Left, widths.left, colors.left, styles.left),
    ];
    let visible_edges = edges.into_iter().filter(|&(_, width, color, style)| {
        width > 0.0
            && color.alpha > 0.0
            && !matches!(style, PaintBorderStyle::None | PaintBorderStyle::Hidden)
    });
    if !radii.is_zero()
        && let Some((_, _, color, PaintBorderStyle::Solid)) = visible_edges.clone().next()
        && visible_edges
            .clone()
            .all(|(_, _, other, style)| other == color && style == PaintBorderStyle::Solid)
    {
        // Fill matching edges together so their rounded joins are not
        // antialiased as separate, partially transparent clips.
        let mut clip = BezPath::new();
        for (edge, _, _, _) in visible_edges {
            clip.extend(border_edge_clip(rect, widths, radii, edge));
        }
        paint_border_ring_slice(
            scene, rect, widths, radii, &clip, 0.0, 1.0, color, transform,
        );
        return;
    }
    for (edge, width, color, style) in visible_edges {
        paint_border_edge(
            scene, rect, widths, radii, edge, width, color, style, transform,
        );
    }
}

/// Resolve the outer and inner border edges onto the layout-pixel grid.
///
/// Blink independently snaps `PixelSnappedContouredBorder` and
/// `PixelSnappedContouredInnerBorder`. Reconstructing the four used widths
/// from those two rectangles preserves that contract for asymmetric and
/// fractional borders instead of merely rounding the outer origin.
fn pixel_snapped_border_geometry(
    rect: Rect,
    widths: PaintEdgeSizes,
) -> Option<(Rect, PaintEdgeSizes)> {
    let (outer_x, outer_width) = pixel_snap_paint_axis(rect.x0 as f32, rect.width() as f32);
    let (outer_y, outer_height) = pixel_snap_paint_axis(rect.y0 as f32, rect.height() as f32);
    if outer_width <= 0.0 || outer_height <= 0.0 {
        return None;
    }

    let inner_x = rect.x0 as f32 + widths.left;
    let inner_y = rect.y0 as f32 + widths.top;
    let inner_width = (rect.width() as f32 - widths.left - widths.right).max(0.0);
    let inner_height = (rect.height() as f32 - widths.top - widths.bottom).max(0.0);
    let (inner_x, inner_width) = pixel_snap_paint_axis_allowing_zero(inner_x, inner_width);
    let (inner_y, inner_height) = pixel_snap_paint_axis_allowing_zero(inner_y, inner_height);

    let outer = Rect::new(
        f64::from(outer_x),
        f64::from(outer_y),
        f64::from(outer_x + outer_width),
        f64::from(outer_y + outer_height),
    );
    let widths = PaintEdgeSizes::new(
        (inner_y - outer_y).clamp(0.0, outer_height),
        (outer.x1 as f32 - (inner_x + inner_width)).clamp(0.0, outer_width),
        (outer.y1 as f32 - (inner_y + inner_height)).clamp(0.0, outer_height),
        (inner_x - outer_x).clamp(0.0, outer_width),
    );
    Some((
        outer,
        sanitize_border_widths(widths, outer_width, outer_height),
    ))
}

#[derive(Clone, Copy)]
enum BorderEdge {
    Top,
    Right,
    Bottom,
    Left,
}

#[allow(clippy::too_many_arguments)]
fn paint_border_edge(
    scene: &mut impl PaintScene,
    rect: Rect,
    widths: PaintEdgeSizes,
    radii: PaintCornerRadii,
    edge: BorderEdge,
    width: f32,
    color: PaintColor,
    style: PaintBorderStyle,
    transform: Affine,
) {
    let clip = border_edge_clip(rect, widths, radii, edge);
    let clip_pattern_to_ring =
        !radii.is_zero() && matches!(style, PaintBorderStyle::Dashed | PaintBorderStyle::Dotted);
    if clip_pattern_to_ring {
        // A stroke on the centerline can spill past the actual border where
        // adjacent edges are thinner, especially near a rounded join.
        scene.push_clip_layer(transform, &border_ring_path(rect, widths, radii, 0.0, 1.0));
    }
    match style {
        PaintBorderStyle::None | PaintBorderStyle::Hidden => {}
        PaintBorderStyle::Solid => paint_border_ring_slice(
            scene, rect, widths, radii, &clip, 0.0, 1.0, color, transform,
        ),
        PaintBorderStyle::Double if width >= 3.0 => {
            paint_border_ring_slice(
                scene,
                rect,
                widths,
                radii,
                &clip,
                0.0,
                1.0 / 3.0,
                color,
                transform,
            );
            paint_border_ring_slice(
                scene,
                rect,
                widths,
                radii,
                &clip,
                2.0 / 3.0,
                1.0,
                color,
                transform,
            );
        }
        PaintBorderStyle::Double => paint_border_ring_slice(
            scene, rect, widths, radii, &clip, 0.0, 1.0, color, transform,
        ),
        PaintBorderStyle::Groove | PaintBorderStyle::Ridge => {
            let ridge = style == PaintBorderStyle::Ridge;
            let (outer, inner) = grooved_colors(color, edge, ridge);
            paint_border_ring_slice(
                scene, rect, widths, radii, &clip, 0.0, 0.5, outer, transform,
            );
            paint_border_ring_slice(
                scene, rect, widths, radii, &clip, 0.5, 1.0, inner, transform,
            );
        }
        PaintBorderStyle::Inset | PaintBorderStyle::Outset => {
            let inset = style == PaintBorderStyle::Inset;
            paint_border_ring_slice(
                scene,
                rect,
                widths,
                radii,
                &clip,
                0.0,
                1.0,
                beveled_color(color, edge, inset),
                transform,
            );
        }
        PaintBorderStyle::Dashed => paint_dashed_border_edge(
            scene, rect, widths, radii, edge, &clip, width, color, transform,
        ),
        PaintBorderStyle::Dotted => paint_dotted_border_edge(
            scene, rect, widths, radii, edge, &clip, width, color, transform,
        ),
    }
    if clip_pattern_to_ring {
        scene.pop_layer();
    }
}

fn paint_uniform_solid_border(
    scene: &mut impl PaintScene,
    rect: Rect,
    widths: PaintEdgeSizes,
    colors: PaintBorderColors,
    styles: PaintBorderStyles,
    radii: PaintCornerRadii,
    transform: Affine,
) -> bool {
    if [styles.top, styles.right, styles.bottom, styles.left]
        .iter()
        .any(|style| *style != PaintBorderStyle::Solid)
        || [colors.right, colors.bottom, colors.left]
            .iter()
            .any(|color| *color != colors.top)
        || colors.top.alpha <= 0.0
    {
        return false;
    }
    let width = widths.top;
    if width <= 0.0
        || [widths.right, widths.bottom, widths.left]
            .iter()
            .any(|candidate| (*candidate - width).abs() > f32::EPSILON)
    {
        return false;
    }

    let width = f64::from(width);
    let stroke = Stroke::new(width).with_join(Join::Miter);
    let corners = [
        radii.top_left,
        radii.top_right,
        radii.bottom_right,
        radii.bottom_left,
    ];
    let circular = corners
        .iter()
        .all(|radius| (radius.x - radius.y).abs() < 0.01);
    if circular
        && corners
            .iter()
            .all(|radius| radius.x == 0.0 || f64::from(radius.x) >= width)
    {
        let half_width = width / 2.0;
        let centerline = rect - Insets::uniform(half_width);
        let centerline_radius =
            |radius: PaintCornerRadius| (f64::from(radius.x) - half_width).max(0.0);
        let radii = RoundedRectRadii::new(
            centerline_radius(radii.top_left),
            centerline_radius(radii.top_right),
            centerline_radius(radii.bottom_right),
            centerline_radius(radii.bottom_left),
        );
        scene.stroke(
            &stroke,
            transform,
            to_backend_color(colors.top),
            None,
            &RoundedRect::from_rect(centerline, radii),
        );
        return true;
    }

    let half_x = rect.width() / 2.0;
    let half_y = rect.height() / 2.0;
    let is_half = |radius: PaintCornerRadius| {
        (f64::from(radius.x) - half_x).abs() < 0.01 && (f64::from(radius.y) - half_y).abs() < 0.01
    };
    if corners.into_iter().all(is_half) {
        let radius_x = (rect.width() + (rect.width() - width * 2.0).max(0.0)) / 4.0;
        let radius_y = (rect.height() + (rect.height() - width * 2.0).max(0.0)) / 4.0;
        scene.stroke(
            &stroke,
            transform,
            to_backend_color(colors.top),
            None,
            &Ellipse::new(rect.center(), (radius_x, radius_y), 0.0),
        );
        return true;
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn paint_dashed_border_edge(
    scene: &mut impl PaintScene,
    rect: Rect,
    widths: PaintEdgeSizes,
    radii: PaintCornerRadii,
    edge: BorderEdge,
    clip: &BezPath,
    width: f32,
    color: PaintColor,
    transform: Affine,
) {
    let thickness = f64::from(width);
    if thickness <= 0.0 {
        return;
    }
    let (dash_ratio, gap_ratio) = dashed_ratios(thickness);
    let (centerline, dash, gap) = if !radii.is_zero() {
        let center = border_slice(rect, widths, radii, 0.5);
        let centerline = rounded_rect_path(center.0, center.1, true);
        let perimeter = centerline.perimeter(0.1);
        if perimeter <= 0.0 {
            return;
        }
        let initial_period = (dash_ratio + gap_ratio) * thickness;
        let count = (perimeter / initial_period).round().max(1.0);
        let period = perimeter / count;
        let dash = period * dash_ratio / (dash_ratio + gap_ratio);
        (centerline, dash, period - dash)
    } else {
        let half = thickness / 2.0;
        let (start, end, length) = match edge {
            BorderEdge::Top => (
                Point::new(rect.x0, rect.y0 + half),
                Point::new(rect.x1, rect.y0 + half),
                rect.width(),
            ),
            BorderEdge::Bottom => (
                Point::new(rect.x0, rect.y1 - half),
                Point::new(rect.x1, rect.y1 - half),
                rect.width(),
            ),
            BorderEdge::Left => (
                Point::new(rect.x0 + half, rect.y0),
                Point::new(rect.x0 + half, rect.y1),
                rect.height(),
            ),
            BorderEdge::Right => (
                Point::new(rect.x1 - half, rect.y0),
                Point::new(rect.x1 - half, rect.y1),
                rect.height(),
            ),
        };
        if length <= 0.0 {
            return;
        }
        let initial_dash = dash_ratio * thickness;
        let initial_gap = gap_ratio * thickness;
        let count = ((length + initial_gap) / (initial_dash + initial_gap))
            .round()
            .max(1.0);
        let ratio = dash_ratio / gap_ratio;
        let gap = length / (count * ratio + count - 1.0);
        let mut line = BezPath::new();
        line.move_to(start);
        line.line_to(end);
        (line, ratio * gap, gap)
    };

    let stroke = Stroke::new(thickness)
        .with_caps(Cap::Butt)
        .with_dashes(0.0, [dash, gap]);
    scene.push_clip_layer(transform, clip);
    scene.stroke(
        &stroke,
        transform,
        to_backend_color(color),
        None,
        &centerline,
    );
    scene.pop_layer();
}

#[allow(clippy::too_many_arguments)]
fn paint_dotted_border_edge(
    scene: &mut impl PaintScene,
    rect: Rect,
    widths: PaintEdgeSizes,
    radii: PaintCornerRadii,
    edge: BorderEdge,
    clip: &BezPath,
    width: f32,
    color: PaintColor,
    transform: Affine,
) {
    let thickness = f64::from(width);
    if thickness <= 0.0 {
        return;
    }
    let radius = thickness / 2.0;
    let mut path = BezPath::new();
    if !radii.is_zero() {
        let center = border_slice(rect, widths, radii, 0.5);
        let centerline = rounded_rect_path(center.0, center.1, true);
        let perimeter = centerline.perimeter(0.1);
        if perimeter <= 0.0 {
            return;
        }
        let count = (perimeter / (2.0 * thickness)).round().max(1.0);
        let spacing = perimeter / count;
        for center in sample_points_along_path(&centerline, spacing, count as usize) {
            path.extend(Circle::new(center, radius).path_elements(0.1));
        }
    } else {
        let length = match edge {
            BorderEdge::Top | BorderEdge::Bottom => rect.width(),
            BorderEdge::Left | BorderEdge::Right => rect.height(),
        };
        if length <= 0.0 {
            return;
        }
        let dot_center = |along: f64| match edge {
            BorderEdge::Top => Point::new(rect.x0 + along, rect.y0 + radius),
            BorderEdge::Bottom => Point::new(rect.x0 + along, rect.y1 - radius),
            BorderEdge::Left => Point::new(rect.x0 + radius, rect.y0 + along),
            BorderEdge::Right => Point::new(rect.x1 - radius, rect.y0 + along),
        };
        let span = length - thickness;
        if span <= 0.0 {
            path.extend(Circle::new(dot_center(length / 2.0), radius).path_elements(0.1));
        } else {
            let gaps = (span / (2.0 * thickness)).round().max(1.0) as usize;
            let spacing = span / gaps as f64;
            for index in 0..=gaps {
                path.extend(
                    Circle::new(dot_center(radius + index as f64 * spacing), radius)
                        .path_elements(0.1),
                );
            }
        }
    }

    scene.push_clip_layer(transform, clip);
    scene.fill(
        Fill::NonZero,
        transform,
        to_backend_color(color),
        None,
        &path,
    );
    scene.pop_layer();
}

#[allow(clippy::too_many_arguments)]
fn paint_border_ring_slice(
    scene: &mut impl PaintScene,
    rect: Rect,
    widths: PaintEdgeSizes,
    radii: PaintCornerRadii,
    clip: &BezPath,
    outer_fraction: f64,
    inner_fraction: f64,
    color: PaintColor,
    transform: Affine,
) {
    let ring = border_ring_path(rect, widths, radii, outer_fraction, inner_fraction);
    scene.push_clip_layer(transform, clip);
    scene.fill(
        Fill::NonZero,
        transform,
        to_backend_color(color),
        None,
        &ring,
    );
    scene.pop_layer();
}

fn border_ring_path(
    rect: Rect,
    widths: PaintEdgeSizes,
    radii: PaintCornerRadii,
    outer_fraction: f64,
    inner_fraction: f64,
) -> BezPath {
    let outer = border_slice(rect, widths, radii, outer_fraction);
    let inner = border_slice(rect, widths, radii, inner_fraction);
    let mut ring = rounded_rect_path(outer.0, outer.1, true);
    ring.extend(rounded_rect_path(inner.0, inner.1, false));
    ring
}

fn border_slice(
    rect: Rect,
    widths: PaintEdgeSizes,
    radii: PaintCornerRadii,
    fraction: f64,
) -> (Rect, PaintCornerRadii) {
    let top = f64::from(widths.top) * fraction;
    let right = f64::from(widths.right) * fraction;
    let bottom = f64::from(widths.bottom) * fraction;
    let left = f64::from(widths.left) * fraction;
    let inset = Rect::new(
        (rect.x0 + left).min(rect.x1),
        (rect.y0 + top).min(rect.y1),
        (rect.x1 - right).max(rect.x0),
        (rect.y1 - bottom).max(rect.y0),
    );
    let radii = PaintCornerRadii {
        top_left: inset_radius(radii.top_left, left, top),
        top_right: inset_radius(radii.top_right, right, top),
        bottom_right: inset_radius(radii.bottom_right, right, bottom),
        bottom_left: inset_radius(radii.bottom_left, left, bottom),
    };
    (inset, normalize_radii(radii, inset.width(), inset.height()))
}

fn inset_radius(radius: PaintCornerRadius, horizontal: f64, vertical: f64) -> PaintCornerRadius {
    PaintCornerRadius::new(
        (f64::from(radius.x) - horizontal).max(0.0) as f32,
        (f64::from(radius.y) - vertical).max(0.0) as f32,
    )
}

fn border_edge_clip(
    rect: Rect,
    widths: PaintEdgeSizes,
    radii: PaintCornerRadii,
    edge: BorderEdge,
) -> BezPath {
    let (inner, inner_radii) = border_slice(rect, widths, radii, 1.0);
    let corners = |rect: Rect| {
        [
            Point::new(rect.x0, rect.y0),
            Point::new(rect.x1, rect.y0),
            Point::new(rect.x1, rect.y1),
            Point::new(rect.x0, rect.y1),
        ]
    };
    let outer = corners(rect);
    let inner = corners(inner);
    let radii = [
        inner_radii.top_left,
        inner_radii.top_right,
        inner_radii.bottom_right,
        inner_radii.bottom_left,
    ];
    let (start, end) = match edge {
        BorderEdge::Top => (0, 1),
        BorderEdge::Right => (1, 2),
        BorderEdge::Bottom => (2, 3),
        BorderEdge::Left => (3, 0),
    };
    let mut path = BezPath::new();
    path.move_to(outer[start]);
    path.line_to(outer[end]);
    path.line_to(border_corner_clip_end(outer[end], inner[end], radii[end]));
    path.line_to(border_corner_clip_end(
        outer[start],
        inner[start],
        radii[start],
    ));
    path.close_path();
    path
}

fn border_corner_clip_end(outer: Point, inner: Point, radius: PaintCornerRadius) -> Point {
    let direction = inner - outer;
    if radius.x == 0.0 || radius.y == 0.0 || outer == inner {
        return inner;
    }

    // Extend the miter from the inner rectangle's corner to the first
    // radius centerline it crosses. This endpoint lies inside the rounded
    // inner border, so the clip includes its share of the whole ring.
    let x_distance = f64::from(radius.x) / direction.x.abs();
    let y_distance = f64::from(radius.y) / direction.y.abs();
    inner + direction * x_distance.min(y_distance)
}

// Border shading and dash/dot distribution are DOM-neutral ports of
// `blitz-paint/src/render/border.rs` at Blitz d788124a. Geometry remains in
// snapshot CSS pixels and is scaled once by the fragment transform.
fn darken(color: PaintColor) -> PaintColor {
    let value = color.red.max(color.green).max(color.blue);
    if value == 0.0 {
        return color;
    }
    let multiplier = ((value - 0.33) / value).max(0.0);
    PaintColor::new(
        color.red * multiplier,
        color.green * multiplier,
        color.blue * multiplier,
        color.alpha,
    )
}

fn lighten(color: PaintColor) -> PaintColor {
    let value = color.red.max(color.green).max(color.blue);
    if value == 0.0 {
        return PaintColor::new(0.33, 0.33, 0.33, color.alpha);
    }
    let multiplier = (value + 0.33).min(1.0) / value;
    PaintColor::new(
        color.red * multiplier,
        color.green * multiplier,
        color.blue * multiplier,
        color.alpha,
    )
}

fn relative_luminance(color: PaintColor) -> f32 {
    let linear = |channel: f32| {
        let channel = channel.clamp(0.0, 1.0);
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.red) + 0.7152 * linear(color.green) + 0.0722 * linear(color.blue)
}

fn contrast_ratio(first: PaintColor, second: PaintColor) -> f32 {
    let first = relative_luminance(first) + 0.05;
    let second = relative_luminance(second) + 0.05;
    if first > second {
        first / second
    } else {
        second / first
    }
}

fn beveled_color(color: PaintColor, edge: BorderEdge, inset: bool) -> PaintColor {
    let top_or_left = matches!(edge, BorderEdge::Top | BorderEdge::Left);
    let should_darken = top_or_left == inset;
    let dark = darken(color);
    if should_darken {
        return dark;
    }
    if color.red >= 150.0 / 255.0 || color.green >= 92.0 / 255.0 {
        return color;
    }
    const MIN_CONTRAST_RATIO: f32 = 1.75;
    if contrast_ratio(color, dark) < MIN_CONTRAST_RATIO {
        lighten(color)
    } else {
        color
    }
}

fn grooved_colors(color: PaintColor, edge: BorderEdge, ridge: bool) -> (PaintColor, PaintColor) {
    (
        beveled_color(color, edge, !ridge),
        beveled_color(color, edge, ridge),
    )
}

fn dashed_ratios(thickness: f64) -> (f64, f64) {
    if thickness >= 3.0 {
        (2.0, 1.0)
    } else {
        (3.0, 2.0)
    }
}

fn sample_points_along_path(path: &BezPath, spacing: f64, count: usize) -> Vec<Point> {
    let mut polyline = Vec::new();
    peniko::kurbo::flatten(path.iter(), 0.1, |element| match element {
        PathEl::MoveTo(point) | PathEl::LineTo(point) => polyline.push(point),
        PathEl::ClosePath => {
            if let Some(&first) = polyline.first() {
                polyline.push(first);
            }
        }
        _ => {}
    });
    if polyline.len() < 2 {
        return Vec::new();
    }

    let mut points = Vec::with_capacity(count);
    let mut segment = 0;
    let mut segment_start = 0.0;
    let mut segment_length = (polyline[1] - polyline[0]).hypot();
    for index in 0..count {
        let target = index as f64 * spacing;
        while segment + 2 < polyline.len() && target > segment_start + segment_length {
            segment_start += segment_length;
            segment += 1;
            segment_length = (polyline[segment + 1] - polyline[segment]).hypot();
        }
        let progress = if segment_length > 0.0 {
            ((target - segment_start) / segment_length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        points.push(polyline[segment].lerp(polyline[segment + 1], progress));
    }
    points
}

fn sanitize_border_rect(rect: PaintRect) -> Option<Rect> {
    to_backend_rect(rect)
}

fn sanitize_border_widths(
    widths: PaintEdgeSizes,
    rect_width: f32,
    rect_height: f32,
) -> PaintEdgeSizes {
    fn finite_nonnegative(value: f32) -> f32 {
        if value.is_finite() {
            value.max(0.0)
        } else {
            0.0
        }
    }

    let mut widths = PaintEdgeSizes::new(
        finite_nonnegative(widths.top),
        finite_nonnegative(widths.right),
        finite_nonnegative(widths.bottom),
        finite_nonnegative(widths.left),
    );
    let horizontal = widths.left + widths.right;
    if horizontal > rect_width && horizontal > 0.0 {
        let scale = rect_width / horizontal;
        widths.left *= scale;
        widths.right *= scale;
    }
    let vertical = widths.top + widths.bottom;
    if vertical > rect_height && vertical > 0.0 {
        let scale = rect_height / vertical;
        widths.top *= scale;
        widths.bottom *= scale;
    }
    widths
}
