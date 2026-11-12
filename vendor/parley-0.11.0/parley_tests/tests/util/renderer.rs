// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A simple renderer that lays out some text using Parley, extracts outlines using Skrifa and
//! then paints those outlines using Tiny-Skia.
//!
//! Note: Emoji rendering is not currently implemented in this example. See the swash example
//! if you need emoji rendering.

use std::collections::HashMap;

use parley::{BoundingBox, GlyphRun, InlineBoxKind, Layout, PositionedLayoutItem};
use peniko::{
    Color,
    kurbo::{self, Affine, BezPath, Rect, Stroke},
};
use vello_cpu::{Glyph, GlyphRunBuilder, Pixmap, RenderContext, Resources};

use crate::util::env::{CLUSTER_INFO_COLOR, INLINE_BOX_BASELINE_COLOR};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ColorBrush {
    pub(crate) color: Color,
}

impl ColorBrush {
    pub(crate) fn new(color: Color) -> Self {
        Self { color }
    }
}

impl Default for ColorBrush {
    fn default() -> Self {
        Self {
            color: Color::BLACK,
        }
    }
}

pub(crate) struct RenderingConfig {
    pub background_color: Color,
    pub padding_color: Color,
    pub inline_box_color: Color,
    pub cursor_color: Color,
    /// The selection color is chosen based on line index.
    pub selection_colors: [Color; 2],

    /// The width of the pixmap in pixels, excluding padding.
    pub size: Option<kurbo::Size>,

    /// Global render scale (1.0 = 1x, 2.0 = 2x, etc.).
    pub scale: f64,

    /// Whether font hinting is enabled.
    pub hint: bool,

    /// Optional per-glyph transform (e.g., skew for fake italics).
    pub glyph_transform: Option<Affine>,
}

/// A [`RenderContext`] paired with the [`Resources`] (glyph and image caches) it draws with.
pub(crate) struct Renderer {
    pub(crate) ctx: RenderContext,
    resources: Resources,
}

fn draw_rect(renderer: &mut RenderContext, x: f64, y: f64, width: f64, height: f64, color: Color) {
    renderer.set_paint(color);
    renderer.fill_rect(&Rect::new(x, y, x + width, y + height));
}

fn draw_line<T: Into<f64>>(renderer: &mut RenderContext, x1: T, y1: T, x2: T, y2: T) {
    let mut path = BezPath::new();
    path.move_to((x1.into(), y1.into()));
    path.line_to((x2.into(), y2.into()));
    renderer.set_stroke(Stroke::new(1.0));
    renderer.stroke_path(&path);
}

/// Render the layout to a [`Pixmap`].
///
/// If given [`RenderingConfig::size`] is not specified, [`Layout::width`] and [`Layout::height`]
/// are used.
pub(crate) fn draw_layout(
    config: &RenderingConfig,
    layout: &Layout<ColorBrush>,
    cursor_rect: Option<BoundingBox>,
    selection_rects: &[(BoundingBox, usize)],
) -> Renderer {
    let scale = config.scale;
    let padding = 20;
    let base_width = config
        .size
        .map(|size| size.width as f32)
        .unwrap_or_else(|| {
            if layout.layout_max_advance() >= f32::MAX {
                layout.full_width()
            } else {
                layout.layout_max_advance()
            }
        })
        .ceil() as u16;
    let base_height = config
        .size
        .map(|size| size.height as f32)
        .unwrap_or(layout.height())
        .ceil() as u16;

    // Scale dimensions
    let width = (((base_width as f64) * scale).ceil() as u16).max(layout.width().ceil() as u16);
    let height = ((base_height as f64) * scale).ceil() as u16;
    let scaled_padding = ((padding as f64) * scale).ceil() as u16;
    let padded_width = width + scaled_padding * 2;
    let padded_height = height + scaled_padding * 2;
    let fpadding = scaled_padding as f64;

    let mut renderer = RenderContext::new(padded_width, padded_height);
    let mut resources = Resources::new();

    // Draw background rects in pixel space (before applying content transform)
    draw_rect(
        &mut renderer,
        0.0,
        0.0,
        padded_width as f64,
        padded_height as f64,
        config.padding_color,
    );
    draw_rect(
        &mut renderer,
        fpadding,
        fpadding,
        width as f64,
        height as f64,
        config.background_color,
    );

    // Set up transform: translate for padding, then scale for content
    renderer.set_transform(Affine::translate((fpadding, fpadding)) * Affine::scale(scale));

    // Selection and cursor rects are in layout coordinates
    for (rect, lidx) in selection_rects {
        draw_rect(
            &mut renderer,
            rect.x0,
            rect.y0,
            rect.width(),
            rect.height(),
            config.selection_colors[lidx % 2],
        );
    }

    if let Some(rect) = cursor_rect {
        draw_rect(
            &mut renderer,
            rect.x0,
            rect.y0,
            rect.width(),
            rect.height(),
            config.cursor_color,
        );
    }

    // Render each glyph run
    for line in layout.lines() {
        for item in line.items() {
            match item {
                PositionedLayoutItem::GlyphRun(glyph_run) => {
                    render_glyph_run(&glyph_run, &mut renderer, &mut resources, config);
                }
                PositionedLayoutItem::InlineBox(inline_box) => {
                    if inline_box.kind == InlineBoxKind::InFlow {
                        draw_rect(
                            &mut renderer,
                            inline_box.x as f64,
                            inline_box.y as f64,
                            inline_box.width as f64,
                            inline_box.height as f64,
                            config.inline_box_color,
                        );
                        // Render the (first) baseline of the inline box, if it specifies one.
                        if let Some(baseline) = inline_box.baseline {
                            let baseline_y = (inline_box.y + baseline) as f64;
                            renderer.set_paint(INLINE_BOX_BASELINE_COLOR);
                            draw_line(
                                &mut renderer,
                                inline_box.x as f64,
                                baseline_y,
                                (inline_box.x + inline_box.width) as f64,
                                baseline_y,
                            );
                        }
                    }
                }
            };
        }
    }

    Renderer {
        ctx: renderer,
        resources,
    }
}

/// Rasterize everything drawn into the [`Renderer`] to a [`Pixmap`] of the renderer's size.
pub(crate) fn render_to_pixmap(renderer: Renderer) -> Pixmap {
    let Renderer {
        mut ctx,
        mut resources,
    } = renderer;
    let mut img = Pixmap::new(ctx.width(), ctx.height());
    ctx.flush();
    ctx.render(&mut img, &mut resources);
    img
}

/// Render the layout with cluster information including measurement lines and source graphemes.
pub(crate) fn draw_layout_with_clusters(
    config: &RenderingConfig,
    layout: &Layout<ColorBrush>,
    text: &str,
    grapheme_layouts: &HashMap<String, Layout<ColorBrush>>,
) -> Renderer {
    let padding = 20;
    let line_extra_spacing = 60.0; // Extra space between lines for cluster info
    let measurement_line_height = 5.0; // Height below baseline for measurement line
    let grapheme_display_offset = 18.0; // Offset below measurement line for grapheme display

    // Calculate dimensions with extra spacing
    let width = config
        .size
        .map(|size| size.width as f32)
        .unwrap_or(layout.layout_max_advance())
        .ceil() as u16;
    let base_height = layout.height();
    let num_lines = layout.len();
    let height = (base_height + (line_extra_spacing * num_lines as f32)).ceil() as u16;
    let img_width = width.max(layout.width().ceil() as u16);
    let padded_width = img_width + padding * 2;
    let padded_height = height + padding * 2;
    let fpadding = padding as f64;

    let mut renderer = RenderContext::new(padded_width, padded_height);
    let mut resources = Resources::new();
    draw_rect(
        &mut renderer,
        0.0,
        0.0,
        padded_width as f64,
        padded_height as f64,
        config.padding_color,
    );
    draw_rect(
        &mut renderer,
        fpadding,
        fpadding,
        width as f64,
        height as f64,
        config.background_color,
    );

    // Render each line with clusters
    let mut y_offset = 0.0;
    for line in layout.lines() {
        let line_y = line.metrics().baseline + y_offset;

        // Render the normal text first
        for item in line.items() {
            match item {
                PositionedLayoutItem::GlyphRun(glyph_run) => {
                    render_glyph_run_with_offset(
                        &glyph_run,
                        &mut renderer,
                        &mut resources,
                        padding,
                        (0.0, y_offset),
                        config,
                    );
                }
                PositionedLayoutItem::InlineBox(_) => {
                    panic!("Inline boxes are not supported in cluster rendering");
                }
            }
        }

        // Now render cluster information below each line.
        for item in line.items() {
            if let PositionedLayoutItem::GlyphRun(glyph_run) = item {
                let run = glyph_run.run();
                let mut x_offset = glyph_run.offset();

                for cluster in run.visual_clusters() {
                    let cluster_width = cluster.advance();

                    // A cluster spans a grapheme, which can be multiple characters of source
                    // text (e.g. a base letter plus combining marks).
                    let grapheme = &text[cluster.text_range()];

                    // Draw measurement line
                    let measure_y = line_y as f64 + measurement_line_height + fpadding;
                    let measure_x_start = x_offset as f64 + fpadding;
                    let measure_x_end = x_offset as f64 + cluster_width as f64 + fpadding;

                    // Draw horizontal measurement line
                    renderer.set_paint(CLUSTER_INFO_COLOR);
                    draw_line(
                        &mut renderer,
                        measure_x_start,
                        measure_y,
                        measure_x_end,
                        measure_y,
                    );

                    // Draw small vertical ticks at start and end
                    const TICK_HEIGHT: f64 = 3.0;
                    draw_line(
                        &mut renderer,
                        measure_x_start,
                        measure_y - TICK_HEIGHT,
                        measure_x_start,
                        measure_y + TICK_HEIGHT,
                    );
                    draw_line(
                        &mut renderer,
                        measure_x_end,
                        measure_y - TICK_HEIGHT,
                        measure_x_end,
                        measure_y + TICK_HEIGHT,
                    );

                    // Render the grapheme, skipping whitespace.
                    if !matches!(grapheme, " " | "\n" | "\t") {
                        // Draw the cluster's grapheme glyphs under the measurement line (these
                        // should appear to be the same as the grapheme in the source text).
                        let grapheme_layout = grapheme_layouts.get(grapheme).unwrap();
                        let line = grapheme_layout.lines().next().unwrap();
                        let item = line.items().next().unwrap();
                        let glyph_run = match item {
                            PositionedLayoutItem::GlyphRun(glyph_run) => glyph_run,
                            PositionedLayoutItem::InlineBox(_) => {
                                panic!("Inline boxes are not supported in cluster rendering");
                            }
                        };

                        // Center each "reference" grapheme within the tick marks
                        let grapheme_x_offset =
                            x_offset + ((cluster_width - line.metrics().advance) / 2.0);
                        let grapheme_y_offset = line_y + measurement_line_height as f32
                            - line.metrics().baseline
                            + grapheme_display_offset;
                        render_glyph_run_with_offset(
                            &glyph_run,
                            &mut renderer,
                            &mut resources,
                            padding,
                            (grapheme_x_offset, grapheme_y_offset),
                            config,
                        );
                    }
                    x_offset += cluster_width;
                }
            }
        }

        y_offset += line_extra_spacing;
    }

    Renderer {
        ctx: renderer,
        resources,
    }
}

fn render_glyph_run(
    glyph_run: &GlyphRun<'_, ColorBrush>,
    renderer: &mut RenderContext,
    resources: &mut Resources,
    config: &RenderingConfig,
) {
    render_glyph_run_impl(glyph_run, renderer, resources, (0.0, 0.0), config);
}

fn render_glyph_run_with_offset(
    glyph_run: &GlyphRun<'_, ColorBrush>,
    renderer: &mut RenderContext,
    resources: &mut Resources,
    padding: u16,
    offset: (f32, f32),
    config: &RenderingConfig,
) {
    let padding = padding as f32;
    let (x_offset, y_offset) = offset;
    render_glyph_run_impl(
        glyph_run,
        renderer,
        resources,
        (padding + x_offset, padding + y_offset),
        config,
    );
}

fn render_glyph_run_impl(
    glyph_run: &GlyphRun<'_, ColorBrush>,
    renderer: &mut RenderContext,
    resources: &mut Resources,
    offset: (f32, f32),
    config: &RenderingConfig,
) {
    let (x_offset, y_offset) = offset;
    renderer.set_paint(glyph_run.style().brush.color);
    let run = glyph_run.run();

    let normalized_coords = bytemuck::cast_slice::<_, i16>(run.normalized_coords());
    let glyphs = glyph_run.positioned_glyphs().map(|glyph| Glyph {
        id: glyph.id,
        x: glyph.x + x_offset,
        y: glyph.y + y_offset,
    });
    glyph_run_builder(renderer, resources, glyph_run, normalized_coords, config)
        .fill_glyphs(glyphs.clone());

    let style = glyph_run.style();
    if let Some(decoration) = &style.underline {
        let underline_offset = decoration
            .offset
            .unwrap_or(run.font_metrics().underline_offset);
        let size = decoration.size.unwrap_or(run.font_metrics().underline_size);

        renderer.set_paint(decoration.brush.color);
        let x = glyph_run.offset() + x_offset;
        let x1 = x + glyph_run.advance();
        let baseline = glyph_run.baseline() + y_offset;

        // Use ink-skipping underline rendering
        glyph_run_builder(renderer, resources, glyph_run, normalized_coords, config)
            .render_decoration(
                glyphs,
                x..=x1,
                baseline,
                underline_offset,
                size,
                size, // buffer around exclusions; let's match the underline thickness
            );
    }
    if let Some(decoration) = &style.strikethrough {
        let strikethrough_offset = decoration
            .offset
            .unwrap_or(run.font_metrics().strikethrough_offset);
        let size = decoration
            .size
            .unwrap_or(run.font_metrics().strikethrough_size);

        // Strikethrough uses simple rect (doesn't skip ink)
        let y = glyph_run.baseline() as f64 - strikethrough_offset as f64 + y_offset as f64;
        let x = glyph_run.offset() as f64 + x_offset as f64;
        draw_rect(
            renderer,
            x,
            y,
            glyph_run.advance() as f64,
            size as f64,
            decoration.brush.color,
        );
    }
}

fn glyph_run_builder<'a>(
    renderer: &'a mut RenderContext,
    resources: &'a mut Resources,
    glyph_run: &GlyphRun<'_, ColorBrush>,
    normalized_coords: &'a [i16],
    config: &RenderingConfig,
) -> GlyphRunBuilder<'a> {
    let run = glyph_run.run();
    let mut builder = renderer
        .glyph_run(resources, &run.font().font)
        .font_size(run.font_size())
        .hint(config.hint)
        .normalized_coords(normalized_coords);
    if let Some(glyph_transform) = config.glyph_transform {
        builder = builder.glyph_transform(glyph_transform);
    }
    builder
}
