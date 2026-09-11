//! Native (V8-free) tests for the Canvas surface/backend contract.
//!
//! These exercise the public [`CanvasSurface`] API through the real Vello CPU
//! backend, proving repeated rendering onto a persistent surface, immutable
//! snapshot isolation, reset, composition, pixel-format conversion, region
//! readback, and the failure/accounting contract.

use std::sync::Arc;

use anyrender::PaintScene;
use anyrender_vello_cpu::VelloCpuScenePainter;
use kurbo::{Affine, Rect};
use moli_canvas::{CanvasSurface, CanvasSurfaceError};
use peniko::{Color, Fill};

fn fill(scene: &mut VelloCpuScenePainter, x: f64, y: f64, w: f64, h: f64, color: Color) {
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        color,
        None,
        &Rect::new(x, y, x + w, y + h),
    );
}

const RED: Color = Color::new([1.0, 0.0, 0.0, 1.0]);
const GREEN: Color = Color::new([0.0, 1.0, 0.0, 1.0]);

/// Reads the straight-alpha RGBA8 pixel at (x, y) via a 1x1 region readback.
fn pixel(surface: &CanvasSurface, x: i32, y: i32) -> [u8; 4] {
    let bytes = surface.readback_region(x, y, 1, 1);
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

#[test]
fn repeated_source_over_rendering_preserves_prior_content() {
    let mut surface = CanvasSurface::new(16, 16).unwrap();
    surface
        .render(|s| fill(s, 1.0, 1.0, 5.0, 5.0, RED))
        .unwrap();
    // A second source-over batch composes over the first, not replacing it.
    surface
        .render(|s| fill(s, 3.0, 3.0, 5.0, 5.0, GREEN))
        .unwrap();

    // (2,2) is covered by both: red over which green was drawn at (3,3)+,
    // so the interior at (2,2) is still pure red.
    assert_eq!(pixel(&surface, 2, 2), [255, 0, 0, 255]);
    // (5,5) is inside the green rect (green's alpha=255 overwrites red fully).
    assert_eq!(pixel(&surface, 5, 5), [0, 255, 0, 255]);
    // Snapshots reflect the cumulative content.
    let snap = surface.snapshot().unwrap();
    assert_eq!(
        snap.rgba[((2 * 16 + 2) * 4)..((2 * 16 + 2) * 4 + 4)],
        [255, 0, 0, 255]
    );
    assert_eq!(
        snap.rgba[((5 * 16 + 5) * 4)..((5 * 16 + 5) * 4 + 4)],
        [0, 255, 0, 255]
    );
}

#[test]
fn snapshot_isolation_and_clean_repeated_reads() {
    let mut surface = CanvasSurface::new(16, 16).unwrap();
    surface
        .render(|s| fill(s, 1.0, 1.0, 5.0, 5.0, RED))
        .unwrap();
    let first = surface.snapshot().unwrap();

    // Drawing more does not mutate the already-published snapshot.
    surface
        .render(|s| fill(s, 8.0, 8.0, 5.0, 5.0, GREEN))
        .unwrap();
    assert_eq!(
        first.rgba[(2 * 16 + 2) * 4 + 3],
        255,
        "old snapshot is unchanged where it had content"
    );
    // The old snapshot has no green at the new rectangle area.
    assert_eq!(
        first.rgba[(10 * 16 + 10) * 4 + 1],
        0,
        "old snapshot predates the green fill"
    );

    // Clean repeated reads produce the same cached snapshot (no re-conversion).
    let second = surface.snapshot().unwrap();
    let third = surface.snapshot().unwrap();
    assert_eq!(second.rgba, third.rgba);
    assert!(
        Arc::ptr_eq(&second, &third),
        "clean repeated reads reuse the cached snapshot"
    );

    // The surface and the new snapshot both have the green.
    let fresh = surface.snapshot().unwrap();
    assert_eq!(fresh.rgba[(10 * 16 + 10) * 4], 0);
    assert_eq!(fresh.rgba[(10 * 16 + 10) * 4 + 1], 255);
    assert_eq!(fresh.rgba[(10 * 16 + 10) * 4 + 3], 255);
}

#[test]
fn clear_between_draws_preserves_ordering() {
    let mut surface = CanvasSurface::new(16, 16).unwrap();
    surface
        .render(|s| fill(s, 0.0, 0.0, 16.0, 16.0, RED))
        .unwrap();
    // Destructive clear removes everything.
    surface.clear();
    assert_eq!(
        pixel(&surface, 5, 5),
        [0, 0, 0, 0],
        "clear empties the surface"
    );
    surface
        .render(|s| fill(s, 2.0, 2.0, 4.0, 4.0, GREEN))
        .unwrap();
    assert_eq!(
        pixel(&surface, 3, 3),
        [0, 255, 0, 255],
        "draw after clear lands correctly"
    );
    assert_eq!(
        pixel(&surface, 10, 10),
        [0, 0, 0, 0],
        "no stale red remains"
    );
}

#[test]
fn reset_semantics_same_size_and_resize() {
    let mut surface = CanvasSurface::new(10, 10).unwrap();
    surface
        .render(|s| fill(s, 0.0, 0.0, 10.0, 10.0, RED))
        .unwrap();

    // Same-size resize still clears per reset semantics and reuses allocation.
    surface.resize(10, 10).unwrap();
    assert_eq!(
        pixel(&surface, 5, 5),
        [0, 0, 0, 0],
        "same-size resize clears content"
    );
    assert_eq!(surface.premultiplied().len(), 10 * 10 * 4);

    // Different-size resize resets dimensions and content.
    surface
        .render(|s| fill(s, 0.0, 0.0, 5.0, 5.0, GREEN))
        .unwrap();
    surface.resize(20, 4).unwrap();
    assert_eq!(surface.premultiplied().len(), 20 * 4 * 4);
    assert_eq!(
        pixel(&surface, 2, 2),
        [0, 0, 0, 0],
        "resized surface is empty"
    );

    // Zero-size is well-defined and does not panic on render/readback.
    let mut zero = CanvasSurface::new(0, 0).unwrap();
    assert!(zero.is_empty());
    zero.render(|_| {}).unwrap();
    assert_eq!(zero.readback_region(0, 0, 4, 4), vec![0; 4 * 4 * 4]);
}

#[test]
fn readback_region_clipping_and_independence() {
    let mut surface = CanvasSurface::new(8, 8).unwrap();
    surface
        .render(|s| fill(s, 1.0, 1.0, 4.0, 4.0, RED))
        .unwrap();

    // Out-of-canvas parts are filled transparent; only the visible intersection is read.
    let region = surface.readback_region(-2, 2, 6, 4);
    assert_eq!(region.len(), 6 * 4 * 4);
    // Pixel that lies within the red rect at (source 2,2) maps to (4,0) in region:
    let idx = 4 * 4;
    assert_eq!(region[idx..idx + 4], [255, 0, 0, 255]);
    // Pixels outside the canvas (region column 0 and 1, x=-2,-1) are transparent.
    assert_eq!(&region[0..3], &[0, 0, 0]);

    // Readback returns an independent buffer; writing to it does not affect the surface.
    let mut independent = surface.readback_region(0, 0, 8, 8);
    independent[0] = 99;
    assert_eq!(pixel(&surface, 0, 0), [0, 0, 0, 0]);
}

#[test]
fn pixel_format_low_alpha_and_round_trip() {
    let mut surface = CanvasSurface::new(8, 8).unwrap();
    // A translucent red (alpha ~= 51) leaves a low-alpha premultiplied channel.
    surface
        .render(|s| {
            fill(
                s,
                1.0,
                1.0,
                6.0,
                6.0,
                Color::new([1.0, 0.0, 0.0, 51.0 / 255.0]),
            )
        })
        .unwrap();
    let px = pixel(&surface, 3, 3);
    assert_eq!(px[3], 51, "alpha is preserved straight");
    // Straight red = 255 within rounding at alpha 51.
    assert!(px[0] >= 245, "low-alpha straight red stays near 255");

    // Round-trip: a snapshot and a region read share the same straight conversion.
    let snap = surface.snapshot().unwrap();
    let snap_px = &snap.rgba[((3 * 8 + 3) * 4)..((3 * 8 + 3) * 4 + 4)];
    assert_eq!(snap_px[3], 51);
    assert_eq!(snap_px, &[px[0], px[1], px[2], px[3]]);

    // Fully transparent pixels normalize to transparent black on readback.
    assert_eq!(pixel(&surface, 0, 0), [0, 0, 0, 0]);
}

#[test]
fn invalid_or_oversized_surface_fails_without_mutating_existing() {
    // Edge beyond Vello's u16 limit is rejected.
    assert!(matches!(
        CanvasSurface::new(u16::MAX as u32 + 1, 4),
        Err(CanvasSurfaceError::SurfaceTooLarge { .. })
    ));
    // Byte lengths beyond the platform budget are rejected.
    assert!(CanvasSurface::new(u32::MAX, u32::MAX).is_err());
    // An oversized resize leaves the prior surface intact (no mutation).
    let mut surface = CanvasSurface::new(8, 8).unwrap();
    surface
        .render(|s| fill(s, 0.0, 0.0, 8.0, 8.0, RED))
        .unwrap();
    assert!(surface.resize(u16::MAX as u32 + 1, 1).is_err());
    assert_eq!(surface.width(), 8);
    assert_eq!(
        pixel(&surface, 4, 4),
        [255, 0, 0, 255],
        "failed resize preserves old content"
    );
}

#[test]
fn deterministic_counters_prove_scheduling_and_clean_reads() {
    let mut surface = CanvasSurface::new(64, 64).unwrap();
    // N draws against the persistent surface each submit one backend flush, but
    // do not produce any full-image copies until an observation occurs.
    for i in 0..100u32 {
        let off = (i % 40) as f64;
        surface
            .render(move |s| fill(s, off, off, 4.0, 4.0, RED))
            .unwrap();
    }
    assert_eq!(surface.flush_count(), 100, "each draw submits one flush");
    assert_eq!(
        surface.snapshot_count(),
        0,
        "no observation yet, no full-image copy"
    );

    // One observation performs a single full-image conversion.
    let _snap = surface.snapshot().unwrap();
    assert_eq!(surface.snapshot_count(), 1);

    // A clean subsequent observation performs zero additional conversion work.
    for _ in 0..10 {
        let _again = surface.snapshot().unwrap();
    }
    assert_eq!(
        surface.snapshot_count(),
        1,
        "clean repeated reads reuse the cached snapshot"
    );

    // A new draw invalidates the snapshot; the next observation converts once.
    surface
        .render(|s| fill(s, 50.0, 50.0, 4.0, 4.0, GREEN))
        .unwrap();
    assert_eq!(
        surface.snapshot_count(),
        1,
        "unobserved new content is not converted"
    );
    let _after = surface.snapshot().unwrap();
    assert_eq!(surface.snapshot_count(), 2);
}
