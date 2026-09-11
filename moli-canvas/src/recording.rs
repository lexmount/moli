//! Browser-independent ordered Canvas 2D recorder.
//!
//! A [`DrawRecording`] captures every ordinary drawing operation with its
//! frozen inputs (geometry, captured paint state, and immutable source image
//! snapshots) in call order. Executing it against a persistent
//! [`CanvasSurface`] reproduces the drawn result while batching contiguous
//! scene-expressible source-over operations (path fills/strokes, rectangles)
//! into fewer rasterizations, and preserving order through segmented internal
//! steps for destructive clears, image/text blits, and direct pixel writes.
//!
//! The recorder holds no page-layout or browser state; the browser adapter owns
//! the drawing state and current path and only hands frozen inputs here.
//! Executing/flushing must not reset the drawing state or current path — those
//! stay with the caller. `clear()` discards pending work.

use std::sync::Arc;

use anyrender::PaintScene;
use kurbo::{Affine, BezPath, Rect, Shape, Stroke};
use moli_image::RgbaImage;
use peniko::{Color, Fill};

use crate::blit::{blit_draw_image_filtered_premul, blit_image_data_premul};
use crate::surface::{CanvasSurface, CanvasSurfaceError};
use crate::text::draw_text_premul;
use crate::types::{CanvasRect, DrawImageBlit, ScaleFilter};

/// Frozen stroke metrics that must survive the recording boundary unchanged.
#[derive(Clone, Debug)]
pub struct StrokeSpec {
    pub width: f64,
    pub cap: kurbo::Cap,
    pub join: kurbo::Join,
    pub miter_limit: f64,
    pub dash_pattern: Vec<f64>,
    pub dash_offset: f64,
}

/// One ordered ordinary drawing operation with captured inputs.
#[derive(Clone, Debug)]
pub enum DrawOp {
    /// An already-canvas-space path filled with a straight-alpha color.
    FillPath { path: BezPath, color: [u8; 4] },
    /// A user-space path stroked with the given transform and frozen metrics.
    StrokePath {
        path: BezPath,
        transform: Affine,
        style: StrokeSpec,
        color: [u8; 4],
    },
    /// An axis-aligned rectangle filled with a straight-alpha color.
    FillRect { rect: Rect, color: [u8; 4] },
    /// A rectangle stroked with the given transform and frozen metrics.
    StrokeRect {
        rect: Rect,
        transform: Affine,
        style: StrokeSpec,
        color: [u8; 4],
    },
    /// A destructive rectangle clear.
    ClearRect { rect: Rect },
    /// A captured source image drawn into a destination rectangle.
    DrawImage {
        dest: Rect,
        source: Arc<RgbaImage>,
        blit: DrawImageBlit,
        filter: ScaleFilter,
    },
    /// Text rendered with the monochrome (font8x8) glyph helper.
    Text {
        text: String,
        x: f64,
        y: f64,
        font: String,
        color: [u8; 4],
    },
    /// A raw pixel overwrite (does not apply ordinary paint state).
    PutImageData { source: RgbaImage, dx: i32, dy: i32 },
}

/// Statistics from one [`DrawRecording::execute`] invocation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExecutionStats {
    /// Number of backend (surface) submissions produced.
    pub flushes: u64,
    /// Number of ordered direct-steps produced (clears / blits / text / writes).
    pub direct_steps: u64,
}

/// An ordered recording of ordinary drawing operations with frozen inputs.
#[derive(Clone, Debug, Default)]
pub struct DrawRecording {
    ops: Vec<DrawOp>,
    /// Estimated retained storage bytes (for early-flush accounting).
    estimated_bytes: usize,
}

impl DrawRecording {
    /// Creates an empty recording.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the recording has no pending operations.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Number of recorded logical operations.
    pub fn op_count(&self) -> usize {
        self.ops.len()
    }

    /// Estimated storage bytes retained by pending operations.
    pub fn estimated_bytes(&self) -> usize {
        self.estimated_bytes
    }

    /// Discards pending operations (reset semantics). Never affects the
    /// caller's drawing state or current path.
    pub fn clear(&mut self) {
        self.ops.clear();
        self.estimated_bytes = 0;
    }

    fn push(&mut self, op: DrawOp, bytes: usize) {
        self.estimated_bytes = self.estimated_bytes.saturating_add(bytes);
        self.ops.push(op);
    }

    pub fn push_fill_path(&mut self, path: BezPath, color: [u8; 4]) {
        let bytes = path.elements().len() * 24 + 4;
        self.push(DrawOp::FillPath { path, color }, bytes);
    }

    pub fn push_stroke_path(
        &mut self,
        path: BezPath,
        transform: Affine,
        style: StrokeSpec,
        color: [u8; 4],
    ) {
        let bytes = path.elements().len() * 24 + 4;
        self.push(
            DrawOp::StrokePath {
                path,
                transform,
                style,
                color,
            },
            bytes,
        );
    }

    pub fn push_fill_rect(&mut self, rect: Rect, color: [u8; 4]) {
        self.push(DrawOp::FillRect { rect, color }, 16 + 4);
    }

    pub fn push_stroke_rect(
        &mut self,
        rect: Rect,
        transform: Affine,
        style: StrokeSpec,
        color: [u8; 4],
    ) {
        self.push(
            DrawOp::StrokeRect {
                rect,
                transform,
                style,
                color,
            },
            16 + 4,
        );
    }

    pub fn push_clear_rect(&mut self, rect: Rect) {
        self.push(DrawOp::ClearRect { rect }, 16);
    }

    pub fn push_draw_image(
        &mut self,
        dest: Rect,
        source: Arc<RgbaImage>,
        blit: DrawImageBlit,
        filter: ScaleFilter,
    ) {
        let bytes = source.byte_len() + 8;
        self.push(
            DrawOp::DrawImage {
                dest,
                source,
                blit,
                filter,
            },
            bytes,
        );
    }

    pub fn push_text(&mut self, text: String, x: f64, y: f64, font: String, color: [u8; 4]) {
        let bytes = text.len() + font.len() + 16 + 4;
        self.push(
            DrawOp::Text {
                text,
                x,
                y,
                font,
                color,
            },
            bytes,
        );
    }

    pub fn push_put_image_data(&mut self, source: RgbaImage, dx: i32, dy: i32) {
        let bytes = source.byte_len() + 8;
        self.push(DrawOp::PutImageData { source, dx, dy }, bytes);
    }

    /// Executes the recorded operations against `surface` in call order.
    ///
    /// Contiguous scene-expressible source-over operations (path fills/strokes
    /// and rectangles) are accumulated into one backend submission; destructive
    /// clears, image/text blits, and direct pixel writes are executed as ordered
    /// internal steps, so a mixed draw/clear/write sequence preserves ordering.
    pub fn execute(
        &self,
        surface: &mut CanvasSurface,
    ) -> Result<ExecutionStats, CanvasSurfaceError> {
        let mut stats = ExecutionStats::default();
        let mut batch: Vec<&DrawOp> = Vec::new();

        let is_scene_op = |op: &DrawOp| {
            matches!(
                op,
                DrawOp::FillPath { .. }
                    | DrawOp::StrokePath { .. }
                    | DrawOp::FillRect { .. }
                    | DrawOp::StrokeRect { .. }
            )
        };

        for op in &self.ops {
            if is_scene_op(op) {
                batch.push(op);
                continue;
            }
            if !batch.is_empty() {
                flush_scene_batch(surface, &batch)?;
                stats.flushes = stats.flushes.saturating_add(1);
                batch.clear();
            }
            execute_direct(surface, op)?;
            stats.direct_steps = stats.direct_steps.saturating_add(1);
        }
        if !batch.is_empty() {
            flush_scene_batch(surface, &batch)?;
            stats.flushes = stats.flushes.saturating_add(1);
        }
        Ok(stats)
    }
}

fn flush_scene_batch(
    surface: &mut CanvasSurface,
    ops: &[&DrawOp],
) -> Result<(), CanvasSurfaceError> {
    surface.render(|scene| {
        for op in ops {
            match op {
                DrawOp::FillPath { path, color } => {
                    scene.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        to_color(*color),
                        None,
                        path,
                    );
                }
                DrawOp::StrokePath {
                    path,
                    transform,
                    style,
                    color,
                } => {
                    let stroke = to_stroke(style);
                    scene.stroke(&stroke, *transform, to_color(*color), None, path);
                }
                DrawOp::FillRect { rect, color } => {
                    scene.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        to_color(*color),
                        None,
                        rect,
                    );
                }
                DrawOp::StrokeRect {
                    rect,
                    transform,
                    style,
                    color,
                } => {
                    let stroke = to_stroke(style);
                    let path = rect.to_path(0.001);
                    scene.stroke(&stroke, *transform, to_color(*color), None, &path);
                }
                _ => unreachable!("scene batch only contains scene-expressible ops"),
            }
        }
    })
}

fn execute_direct(surface: &mut CanvasSurface, op: &DrawOp) -> Result<(), CanvasSurfaceError> {
    if surface.is_empty() {
        return Ok(());
    }
    match op {
        DrawOp::ClearRect { rect } => {
            let width = surface.width();
            let height = surface.height();
            let pixels = surface.premutated_mut();
            clear_rect_premul(pixels, width, height, rect_to_canvas(*rect));
        }
        DrawOp::DrawImage {
            dest,
            source,
            blit,
            filter,
        } => {
            let Some(blit_rect) = DrawImageBlit::new(
                blit.source_x,
                blit.source_y,
                blit.source_width,
                blit.source_height,
                dest.x0,
                dest.y0,
                dest.x1 - dest.x0,
                dest.y1 - dest.y0,
            ) else {
                return Ok(());
            };
            let width = surface.width();
            let height = surface.height();
            let pixels = surface.premutated_mut();
            blit_draw_image_filtered_premul(
                pixels,
                width,
                height,
                &source.rgba,
                source.width,
                source.height,
                blit_rect,
                *filter,
            );
        }
        DrawOp::Text {
            text,
            x,
            y,
            font,
            color,
        } => {
            let width = surface.width();
            let height = surface.height();
            let pixels = surface.premutated_mut();
            draw_text_premul(pixels, width, height, text, *x, *y, font, *color);
        }
        DrawOp::PutImageData { source, dx, dy } => {
            let width = surface.width();
            let height = surface.height();
            let pixels = surface.premutated_mut();
            blit_image_data_premul(
                pixels,
                width,
                height,
                &source.rgba,
                source.width,
                source.height,
                *dx,
                *dy,
                0,
                0,
                source.width as i32,
                source.height as i32,
            );
        }
        DrawOp::FillPath { .. }
        | DrawOp::StrokePath { .. }
        | DrawOp::FillRect { .. }
        | DrawOp::StrokeRect { .. } => {
            unreachable!("scene-expressible ops are batched, not direct")
        }
    }
    surface.mark_dirty_and_invalidate_snapshot();
    Ok(())
}

fn to_color(rgba: [u8; 4]) -> Color {
    Color::new([
        f64::from(rgba[0]) as f32 / 255.0,
        f64::from(rgba[1]) as f32 / 255.0,
        f64::from(rgba[2]) as f32 / 255.0,
        f64::from(rgba[3]) as f32 / 255.0,
    ])
}

fn to_stroke(style: &StrokeSpec) -> Stroke {
    let mut stroke = Stroke::new(style.width);
    stroke.join = style.join;
    stroke.start_cap = style.cap;
    stroke.end_cap = style.cap;
    stroke.miter_limit = style.miter_limit;
    stroke.dash_pattern = style.dash_pattern.iter().copied().collect();
    stroke.dash_offset = style.dash_offset;
    stroke
}

fn rect_to_canvas(rect: Rect) -> CanvasRect {
    (
        rect.x0 as i32,
        rect.y0 as i32,
        rect.x1 as i32,
        rect.y1 as i32,
    )
}

/// Zeros pixels in the given rectangle of a premultiplied RGBA8 surface
/// (transparent black). This is O(rect area), not O(surface area).
fn clear_rect_premul(pixels: &mut [u8], width: u32, height: u32, rect: CanvasRect) {
    let (left, top, right, bottom) = rect;
    if left >= right || top >= bottom {
        return;
    }
    let start_x = left.max(0).min(width as i32) as u32;
    let start_y = top.max(0).min(height as i32) as u32;
    let end_x = right.max(0).min(width as i32) as u32;
    let end_y = bottom.max(0).min(height as i32) as u32;
    let row_stride = width as usize * 4;
    for y in start_y..end_y {
        let row_start = y as usize * row_stride + start_x as usize * 4;
        let row_end = row_start + (end_x - start_x) as usize * 4;
        for byte in &mut pixels[row_start..row_end] {
            *byte = 0;
        }
    }
}
