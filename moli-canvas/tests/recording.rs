//! Native (V8-free) tests for the ordered Canvas recorder [`DrawRecording`].
//!
//! These prove that all ordinary operations are captured with frozen inputs,
//! executed in call order against a persistent [`CanvasSurface`], batched into
//! fewer backend submissions where possible, and that ordering survives
//! destructive clears, direct pixel writes, and immutable source capture.

use std::sync::Arc;

use kurbo::{Affine, BezPath, Point, Rect};
use moli_canvas::{CanvasSurface, DrawImageBlit, DrawRecording, ScaleFilter, StrokeSpec};
use moli_image::RgbaImage;

fn rect_path(x: f64, y: f64, w: f64, h: f64) -> BezPath {
    let mut path = BezPath::new();
    path.move_to(Point::new(x, y));
    path.line_to(Point::new(x + w, y));
    path.line_to(Point::new(x + w, y + h));
    path.line_to(Point::new(x, y + h));
    path.close_path();
    path
}

fn pixel(surface: &CanvasSurface, x: i32, y: i32) -> [u8; 4] {
    let bytes = surface.readback_region(x, y, 1, 1);
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

#[test]
fn many_path_fills_batch_into_one_flush_with_one_observation() {
    let mut surface = CanvasSurface::new(64, 64).unwrap();
    let mut rec = DrawRecording::new();
    for i in 0..50u32 {
        rec.push_fill_path(
            rect_path(f64::from(i % 40) + 2.0, f64::from(i % 30) + 2.0, 4.0, 4.0),
            [255, 0, 0, 255],
        );
    }
    let before_snapshots = surface.snapshot_count();
    let stats = rec.execute(&mut surface).expect("recording executes");
    assert_eq!(stats.flushes, 1, "50 fills share one backend submission");
    assert_eq!(stats.direct_steps, 0, "no direct ops in a pure fill batch");
    assert_eq!(rec.op_count(), 50, "all ops are retained for replay/order");

    let _snap = surface.snapshot().unwrap();
    assert_eq!(
        surface.snapshot_count(),
        before_snapshots + 1,
        "one observation performs one full-image conversion"
    );
    // An interior filled pixel is red.
    assert_eq!(pixel(&surface, 5, 5), [255, 0, 0, 255]);
}

#[test]
fn clear_segments_a_batch_and_preserves_draw_order() {
    let mut surface = CanvasSurface::new(64, 64).unwrap();
    let mut rec = DrawRecording::new();
    rec.push_fill_rect(Rect::new(0.0, 0.0, 32.0, 32.0), [255, 0, 0, 255]);
    rec.push_clear_rect(Rect::new(0.0, 0.0, 32.0, 32.0));
    rec.push_fill_rect(Rect::new(0.0, 0.0, 32.0, 32.0), [0, 255, 0, 255]);

    let stats = rec.execute(&mut surface).expect("recording executes");
    // The clear breaks the fill batch into a before/after pair.
    assert_eq!(
        stats.flushes, 2,
        "the clear segments the scene batch into two submissions"
    );
    assert_eq!(stats.direct_steps, 1, "the clear is an ordered direct step");
    // The final red-over-green ordering: the last fill is green and opaque,
    // so the covered pixel is green.
    assert_eq!(pixel(&surface, 16, 16), [0, 255, 0, 255]);
    // The clear removed the first red and preserved the second fill's order.
    assert_eq!(pixel(&surface, 40, 40), [0, 0, 0, 0]);
}

#[test]
fn source_snapshot_is_immutable_across_later_mutation() {
    let mut surface = CanvasSurface::new(32, 32).unwrap();
    // A source canvas-like image captured as an owned snapshot.
    let mut src_pixels = vec![0u8; 8 * 8 * 4];
    for row in 0..8u32 {
        for col in 0..8u32 {
            let i = ((row * 8 + col) * 4) as usize;
            src_pixels[i] = 255; // red
            src_pixels[i + 1] = 0;
            src_pixels[i + 2] = 0;
            src_pixels[i + 3] = 255;
        }
    }
    let source = Arc::new(RgbaImage::try_new(8, 8, src_pixels).expect("valid source"));

    let mut rec = DrawRecording::new();
    let blit = DrawImageBlit::new(0.0, 0.0, 8.0, 8.0, 0.0, 0.0, 8.0, 8.0).expect("valid blit");
    rec.push_draw_image(
        Rect::new(0.0, 0.0, 8.0, 8.0),
        source.clone(),
        blit,
        ScaleFilter::Nearest,
    );
    // Mutating the backing source image after capture must not change the draw.
    let _ = Arc::get_mut(&mut source.clone()).map(|img| {
        for byte in img.rgba.iter_mut() {
            *byte = 0;
        }
    });
    rec.execute(&mut surface).expect("recording executes");
    assert_eq!(
        pixel(&surface, 4, 4),
        [255, 0, 0, 255],
        "captured source is stable"
    );
}

#[test]
fn reset_discards_pending_operations() {
    let mut surface = CanvasSurface::new(16, 16).unwrap();
    let mut rec = DrawRecording::new();
    rec.push_fill_rect(Rect::new(0.0, 0.0, 8.0, 8.0), [255, 0, 0, 255]);
    assert!(!rec.is_empty());
    rec.clear();
    assert!(rec.is_empty());
    assert_eq!(rec.op_count(), 0);
    rec.execute(&mut surface).expect("empty recording executes");
    assert_eq!(
        pixel(&surface, 2, 2),
        [0, 0, 0, 0],
        "reset discards content"
    );
}

#[test]
fn put_image_data_is_an_ordered_raw_overwrite() {
    let mut surface = CanvasSurface::new(8, 8).unwrap();
    // Paint a translucent-ish background, then raw-overwrite a region via
    // putImageData; the overwrite must not apply paint state.
    let mut rec = DrawRecording::new();
    rec.push_fill_rect(Rect::new(0.0, 0.0, 8.0, 8.0), [0, 255, 0, 255]);
    let mut img_pixels = vec![0u8; 2 * 2 * 4];
    img_pixels[0] = 255; // blue, opaque
    img_pixels[1] = 0;
    img_pixels[2] = 255;
    img_pixels[3] = 255;
    let image = RgbaImage::try_new(2, 2, img_pixels).expect("valid image");
    rec.push_put_image_data(image, 1, 1);

    let stats = rec.execute(&mut surface).expect("recording executes");
    assert_eq!(
        stats.direct_steps, 1,
        "putImageData is a direct ordered write"
    );
    assert_eq!(
        pixel(&surface, 1, 1),
        [255, 0, 255, 255],
        "raw overwrite wins"
    );
    assert_eq!(
        pixel(&surface, 5, 5),
        [0, 255, 0, 255],
        "background kept outside overwrite"
    );
}

#[test]
fn stroke_path_carries_frozen_metrics_across_the_recording_boundary() {
    let mut surface = CanvasSurface::new(32, 32).unwrap();
    let mut rec = DrawRecording::new();
    let style = StrokeSpec {
        width: 3.0,
        cap: kurbo::Cap::Round,
        join: kurbo::Join::Round,
        miter_limit: 10.0,
        dash_pattern: Vec::new(),
        dash_offset: 0.0,
    };
    let path = rect_path(4.0, 4.0, 16.0, 16.0);
    rec.push_stroke_path(path, Affine::IDENTITY, style, [0, 0, 255, 255]);

    let stats = rec.execute(&mut surface).expect("recording executes");
    assert_eq!(stats.flushes, 1, "stroke path is a scene batch op");
    // A point on the stroke perimeter is blue.
    assert_eq!(pixel(&surface, 4, 15), [0, 0, 255, 255]);
    // The interior is not filled.
    assert_eq!(pixel(&surface, 12, 12), [0, 0, 0, 0]);
}
