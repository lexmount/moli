// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A simple example that lays out some text using Parley and paints the glyphs using
//! Vello CPU's built-in glyph rendering.

use std::path::Path;
use std::time::Instant;

use parley::{GlyphRun, Layout, PositionedLayoutItem};
use parley_examples_common::{
    ColorBrush, ExampleConfig, FrameKind, FrameStats, frame_sequence, output_dir, prepare_layouts,
};
use peniko::Color;
use vello_cpu::{
    Glyph, Pixmap, RenderContext, Resources,
    kurbo::{Affine, Rect, Vec2},
};

fn main() {
    let start_time = Instant::now();

    let (
        (simple_layout, simple_width, simple_height, simple_config),
        (rich_layout, rich_width, rich_height, rich_config),
    ) = prepare_layouts();
    let width = simple_width.max(rich_width);
    let height = simple_height.max(rich_height);

    let mut stats = FrameStats::new();

    stats.start("prepare_rendering");
    let mut renderer = RenderContext::new(width, height);
    let mut resources = Resources::new();
    stats.end("prepare_rendering");
    let output_path = output_dir(env!("CARGO_MANIFEST_DIR")).join("vello_cpu_render.png");

    for frame in &frame_sequence() {
        println!("\n=== {} ===", frame.label);
        let (layout, config) = match frame.kind {
            FrameKind::Simple => (&simple_layout, &simple_config),
            FrameKind::Rich => (&rich_layout, &rich_config),
        };
        render_frame(
            layout,
            config,
            &mut renderer,
            &mut resources,
            width,
            height,
            &output_path,
            &mut stats,
        );
    }

    println!("\n=== CPU Render Performance ===");
    stats.print_summary();
    println!("\nTotal elapsed: {:?}", start_time.elapsed());
}

/// Run one full frame: reset → fill glyphs → render → print stats.
fn render_frame(
    layout: &Layout<ColorBrush>,
    config: &ExampleConfig,
    renderer: &mut RenderContext,
    resources: &mut Resources,
    width: u16,
    height: u16,
    output_path: &Path,
    stats: &mut FrameStats,
) {
    stats.start("total_frame");

    stats.start("reset_renderer");
    reset_renderer(
        renderer,
        width,
        height,
        config.padding,
        config.background_color,
    );
    stats.end("reset_renderer");

    stats.start_accumulating("fill_glyphs");
    stats.start_accumulating("render_underline");
    stats.start_accumulating("render_strikethrough");

    for line in layout.lines() {
        for item in line.items() {
            match item {
                PositionedLayoutItem::GlyphRun(glyph_run) => {
                    let run = glyph_run.run();

                    stats.start("fill_glyphs");
                    let normalized_coords = bytemuck::cast_slice::<_, i16>(run.normalized_coords());
                    renderer.set_paint(glyph_run.style().brush.color);
                    renderer
                        .glyph_run(resources, &run.font().font)
                        .font_size(run.font_size())
                        .hint(config.hint)
                        .normalized_coords(normalized_coords)
                        .fill_glyphs(glyph_run.positioned_glyphs().map(|glyph| Glyph {
                            id: glyph.id,
                            x: glyph.x,
                            y: glyph.y,
                        }));
                    stats.end("fill_glyphs");

                    let style = glyph_run.style();
                    if let Some(decoration) = &style.underline {
                        let offset = decoration
                            .offset
                            .unwrap_or(run.font_metrics().underline_offset);
                        let size = decoration.size.unwrap_or(run.font_metrics().underline_size);

                        stats.start("render_underline");
                        render_decoration(renderer, &decoration.brush, &glyph_run, offset, size);
                        stats.end("render_underline");
                    }
                    if let Some(decoration) = &style.strikethrough {
                        let offset = decoration
                            .offset
                            .unwrap_or(run.font_metrics().strikethrough_offset);
                        let size = decoration
                            .size
                            .unwrap_or(run.font_metrics().strikethrough_size);

                        stats.start("render_strikethrough");
                        render_decoration(renderer, &decoration.brush, &glyph_run, offset, size);
                        stats.end("render_strikethrough");
                    }
                }

                PositionedLayoutItem::InlineBox(inline_box) => {
                    renderer.set_paint(config.foreground_color);
                    let (x0, y0) = (inline_box.x as f64, inline_box.y as f64);
                    let (x1, y1) = (x0 + inline_box.width as f64, y0 + inline_box.height as f64);
                    renderer.fill_rect(&Rect::new(x0, y0, x1, y1));
                }
            }
        }
    }

    stats.finish_accumulating("fill_glyphs");
    stats.finish_accumulating("render_underline");
    stats.finish_accumulating("render_strikethrough");

    stats.start("render");
    let mut pixmap = Pixmap::new(width, height);
    renderer.flush();
    renderer.render(&mut pixmap, resources);
    stats.end("render");

    save_output(pixmap, output_path);

    stats.end("total_frame");
}

/// Reset the render context, clear the background, and set the transform for the frame.
fn reset_renderer(
    renderer: &mut RenderContext,
    width: u16,
    height: u16,
    padding: u32,
    background_color: Color,
) {
    renderer.reset();
    renderer.set_paint(background_color);
    renderer.fill_rect(&Rect::new(0.0, 0.0, width as f64, height as f64));
    renderer.set_transform(Affine::translate(Vec2::new(padding as f64, padding as f64)));
}

/// Draw a text decoration (underline or strikethrough) as a simple filled rectangle.
fn render_decoration(
    renderer: &mut RenderContext,
    brush: &ColorBrush,
    glyph_run: &GlyphRun<'_, ColorBrush>,
    offset: f32,
    size: f32,
) {
    renderer.set_paint(brush.color);
    let y = glyph_run.baseline() - offset;
    let x = glyph_run.offset();
    let x1 = x + glyph_run.advance();
    let y1 = y + size;
    renderer.fill_rect(&Rect::new(x as f64, y as f64, x1 as f64, y1 as f64));
}

/// Save the rendered pixmap as a PNG file.
fn save_output(pixmap: Pixmap, output_path: &Path) {
    let png = pixmap.into_png().unwrap();
    std::fs::write(output_path, &png).unwrap();
}
