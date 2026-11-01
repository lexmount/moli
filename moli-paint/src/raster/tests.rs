// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Narrowly adapted from DioxusLabs/blitz commit
// d788124ab881f9bb537cb452ec1d837604a374a8:
// - examples/screenshot.rs
// - packages/blitz-paint/src/render.rs
// - crates/anyrender_svg (parsed SVG tree execution)
// The source project is licensed MIT OR Apache-2.0.

//! Raster output and resource-budget regressions.

use crate::MAX_PAINT_LAYER_DEPTH;
use moli_layout::{
    PaintBlendMode, PaintBorderColors, PaintBrush, PaintCaptureSurface, PaintColor,
    PaintCompositeMode, PaintCornerRadii, PaintCornerRadius, PaintEdgeSizes, PaintFilter,
    PaintFragment, PaintImage, PaintImageSampling, PaintRect, PaintShape, PaintSnapshot,
    PaintSvgImage, PaintTransform2D, PaintViewport,
};
use peniko::Blob;
use std::sync::Arc;

use super::*;

fn snapshot(width: u32, height: u32, scale: f32) -> PaintSnapshot {
    PaintSnapshot::new(PaintViewport::new(width, height, scale), PaintColor::WHITE)
}

#[test]
fn maximum_corner_radii_leave_pill_corners_unpainted() {
    let mut snapshot = snapshot(120, 40, 1.0);
    snapshot.push_fragment(PaintFragment::Fill {
        fill_rule: moli_layout::PaintFillRule::NonZero,
        shape: PaintShape::RoundedRect {
            rect: PaintRect::new(0.0, 0.0, 120.0, 40.0),
            radii: PaintCornerRadii::all(PaintCornerRadius::new(f32::MAX, f32::MAX)),
        },
        brush: PaintBrush::Solid(PaintColor::new(0.0, 0.0, 1.0, 1.0)),
        transform: PaintTransform2D::IDENTITY,
    });

    let image = raster_snapshot(&snapshot).expect("pill fill should rasterize");
    for (x, y) in [(0, 0), (119, 0), (0, 39), (119, 39)] {
        assert_eq!(pixel(&image, x, y), [255, 255, 255, 255]);
    }
    assert_eq!(pixel(&image, 60, 0), [0, 0, 255, 255]);
    assert_eq!(pixel(&image, 60, 20), [0, 0, 255, 255]);
}

#[test]
fn rejects_zero_dimensions_before_backend_creation() {
    assert_eq!(
        raster_snapshot(&snapshot(0, 2, 1.0)),
        Err(PaintError::InvalidCaptureSurfaceDimensions {
            width: 0.0,
            height: 2.0,
        })
    );
    assert_eq!(
        raster_snapshot(&snapshot(2, 0, 1.0)),
        Err(PaintError::InvalidCaptureSurfaceDimensions {
            width: 2.0,
            height: 0.0,
        })
    );
}

#[test]
fn rejects_dimensions_above_vello_cpu_limit() {
    assert!(matches!(
        raster_snapshot(&snapshot(u32::from(u16::MAX) + 1, 1, 1.0)),
        Err(PaintError::BackendDimensionLimit { .. })
    ));
    assert!(matches!(
        raster_snapshot(&snapshot(1, u32::from(u16::MAX) + 1, 1.0)),
        Err(PaintError::BackendDimensionLimit { .. })
    ));
    assert!(matches!(
        raster_snapshot(&snapshot(u32::from(u16::MAX), 1, 1.01)),
        Err(PaintError::BackendDimensionLimit { .. })
    ));
}

#[test]
fn rejects_invalid_capture_device_scale() {
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(matches!(
            raster_snapshot(&snapshot(1, 1, scale)),
            Err(PaintError::InvalidCaptureDeviceScale { .. })
        ));
    }
}

#[test]
fn resource_budget_is_byte_based_instead_of_using_a_pixel_count_limit() {
    let dimensions = validate_dimensions(PaintCaptureSurface::new(4_097.0, 4_096.0, 1.0))
        .expect("dimensions supported by the backend");
    assert!(
        u64::from(dimensions.width) * u64::from(dimensions.height) > 16 * 1024 * 1024,
        "test surface must exceed the removed 16M-pixel limit"
    );
    assert_eq!(
        validate_resource_budget(dimensions, PaintStreamMetrics::default()),
        Ok(())
    );
}

#[test]
fn rejects_nested_filters_over_transient_surface_budget() {
    let mut snapshot = snapshot(4_096, 4_096, 1.0);
    for _ in 0..4 {
        snapshot.push_fragment(PaintFragment::PushLayer {
            opacity: 1.0,
            blend_mode: PaintBlendMode::Normal,
            composite: PaintCompositeMode::SrcOver,
            clip: PaintShape::Rect(PaintRect::new(0.0, 0.0, 4_096.0, 4_096.0)),
            transform: PaintTransform2D::IDENTITY,
            filter: Some(PaintFilter::Brightness(1.0)),
        });
    }
    for _ in 0..4 {
        snapshot.push_fragment(PaintFragment::PopLayer);
    }

    assert!(matches!(
        raster_snapshot(&snapshot),
        Err(PaintError::TransientRasterBudgetExceeded {
            required_bytes: 335_544_320,
            max_bytes: MAX_TRANSIENT_RASTER_BYTES,
        })
    ));
}

#[test]
fn rejects_excessive_layer_depth_before_backend_creation() {
    let mut snapshot = snapshot(1, 1, 1.0);
    for _ in 0..=MAX_PAINT_LAYER_DEPTH {
        snapshot.push_fragment(PaintFragment::push_clip(PaintRect::new(0.0, 0.0, 1.0, 1.0)));
    }
    assert_eq!(
        raster_snapshot(&snapshot),
        Err(PaintError::PaintLayerDepthBudgetExceeded {
            depth: MAX_PAINT_LAYER_DEPTH + 1,
            max_depth: MAX_PAINT_LAYER_DEPTH,
        })
    );
}

#[test]
fn device_pixel_ratio_scales_output_and_fragments() {
    let mut snapshot = snapshot(2, 1, 2.0);
    snapshot.push_fragment(PaintFragment::solid_rect(
        PaintRect::new(1.0, 0.0, 1.0, 1.0),
        PaintColor::new(1.0, 0.0, 0.0, 1.0),
    ));

    let image = raster_snapshot(&snapshot).expect("rasterization should succeed");
    assert_eq!((image.width, image.height), (4, 2));
    assert_eq!(pixel(&image, 0, 0), [255, 255, 255, 255]);
    assert_eq!(pixel(&image, 3, 1), [255, 0, 0, 255]);
}

#[test]
fn synthetic_oblique_angles_preserve_css_direction_after_the_glyph_y_flip() {
    let right_leaning = raster_synthetic_oblique_box(14.0);
    let right_top = ink_centroid(&right_leaning, 15);
    let right_bottom = ink_centroid(&right_leaning, 45);
    assert!(
        right_top > right_bottom + 5.0,
        "positive oblique angle must move the glyph top right: top={right_top}, bottom={right_bottom}"
    );

    let left_leaning = raster_synthetic_oblique_box(-14.0);
    let left_top = ink_centroid(&left_leaning, 15);
    let left_bottom = ink_centroid(&left_leaning, 45);
    assert!(
        left_top + 5.0 < left_bottom,
        "negative oblique angle must move the glyph top left: top={left_top}, bottom={left_bottom}"
    );
}

#[test]
fn snapshot_image_resource_rasterizes_without_copying_the_owned_pixels() {
    let pixels = std::sync::Arc::new(
        moli_image::RgbaImage::try_new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap(),
    );
    let mut snapshot = snapshot(4, 2, 1.0);
    let image = snapshot.add_image(pixels.clone());
    snapshot.push_fragment(PaintFragment::Image(PaintImage {
        image,
        destination: PaintRect::new(0.0, 0.0, 4.0, 2.0),
        repeat_bounds: None,
        sampling: PaintImageSampling::Nearest,
        transform: PaintTransform2D::IDENTITY,
    }));

    assert_eq!(std::sync::Arc::strong_count(&pixels), 2);
    let rendered = raster_snapshot(&snapshot).expect("image fragment should rasterize");
    assert_eq!(pixel(&rendered, 0, 0), [255, 0, 0, 255]);
    assert_eq!(pixel(&rendered, 1, 1), [255, 0, 0, 255]);
    assert_eq!(pixel(&rendered, 2, 0), [0, 255, 0, 255]);
    assert_eq!(pixel(&rendered, 3, 1), [0, 255, 0, 255]);
    drop(snapshot);
    assert_eq!(
        std::sync::Arc::strong_count(&pixels),
        1,
        "one-shot snapshot drop must release its shared image reference"
    );
}

#[test]
fn repeated_snapshot_image_preserves_fractional_period_and_finite_bounds() {
    let mut snapshot = snapshot(24, 8, 1.0);
    let image = snapshot.add_image(Arc::new(
        moli_image::RgbaImage::try_new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap(),
    ));
    snapshot.push_fragment(PaintFragment::Image(PaintImage {
        image,
        destination: PaintRect::new(-0.5, 0.0, 3.5, 2.0),
        repeat_bounds: Some(PaintRect::new(0.0, 0.0, 21.0, 6.0)),
        sampling: PaintImageSampling::Nearest,
        transform: PaintTransform2D::IDENTITY,
    }));
    let rendered = raster_snapshot(&snapshot).expect("fractional repeated image");
    for x in [0, 3, 7, 10, 14, 17] {
        assert_eq!(pixel(&rendered, x, 1), [255, 0, 0, 255], "red at {x}");
    }
    for x in [1, 2, 5, 8, 9, 12, 15, 16, 19] {
        assert_eq!(pixel(&rendered, x, 1), [0, 255, 0, 255], "green at {x}");
    }
    assert_eq!(pixel(&rendered, 21, 1), [255; 4]);
    assert_eq!(pixel(&rendered, 1, 6), [255; 4]);
    assert_eq!(snapshot.fragments.len(), 1);
}

#[test]
fn snapshot_svg_resource_rasterizes_at_its_resolved_destination_size() {
    let svg = Arc::new(
        moli_image::decode_svg_image(
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1" viewBox="0 0 2 1"><rect width="1" height="1" fill="#ff0000"/><rect x="1" width="1" height="1" fill="#00ff00"/></svg>"##,
        )
        .expect("fixture SVG should parse"),
    );
    let mut snapshot = snapshot(20, 10, 1.0);
    let image = snapshot.add_svg_image(svg.clone());
    snapshot.push_fragment(PaintFragment::SvgImage(PaintSvgImage {
        image,
        destination: PaintRect::new(0.0, 0.0, 20.0, 10.0),
        transform: PaintTransform2D::IDENTITY,
    }));

    assert_eq!(Arc::strong_count(&svg), 2);
    let rendered = raster_snapshot(&snapshot).expect("SVG fragment should rasterize");
    assert_eq!(pixel(&rendered, 2, 5), [255, 0, 0, 255]);
    assert_eq!(pixel(&rendered, 17, 5), [0, 255, 0, 255]);
    drop(snapshot);
    assert_eq!(Arc::strong_count(&svg), 1);
}

#[test]
fn fractional_svg_image_rows_share_pixel_snapped_boundaries() {
    let svg = Arc::new(
        moli_image::decode_svg_image(
            br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 12"><rect width="96" height="12" fill="#008000"/></svg>"##,
        )
        .expect("fixture SVG should parse"),
    );
    let mut snapshot = PaintSnapshot::new(
        PaintViewport::new(100, 101, 1.0),
        PaintColor::new(1.0, 0.0, 0.0, 1.0),
    );
    let image = snapshot.add_svg_image(svg);
    for row in 0..8 {
        snapshot.push_fragment(PaintFragment::SvgImage(PaintSvgImage {
            image,
            destination: PaintRect::new(0.0, row as f32 * 12.5, 100.0, 12.5),
            transform: PaintTransform2D::IDENTITY,
        }));
    }

    let rendered = raster_snapshot(&snapshot).expect("SVG rows should rasterize");
    for y in 0..100 {
        for x in 0..100 {
            assert_eq!(
                pixel(&rendered, x, y),
                [0, 128, 0, 255],
                "unexpected image coverage at ({x}, {y})"
            );
        }
    }
    for x in 0..100 {
        assert_eq!(
            pixel(&rendered, x, 100),
            [255, 0, 0, 255],
            "SVG content escaped its snapped destination at ({x}, 100)"
        );
    }
}

#[test]
fn missing_snapshot_image_resource_is_rejected_before_backend_creation() {
    let mut snapshot = snapshot(1, 1, 1.0);
    snapshot.push_fragment(PaintFragment::Image(PaintImage {
        image: moli_layout::PaintImageId::from_index(0),
        destination: PaintRect::new(0.0, 0.0, 1.0, 1.0),
        repeat_bounds: None,
        sampling: PaintImageSampling::Linear,
        transform: PaintTransform2D::IDENTITY,
    }));

    assert_eq!(
        raster_snapshot(&snapshot),
        Err(PaintError::MissingImageResource { index: 0 })
    );
}

#[test]
fn missing_snapshot_svg_resource_is_rejected_before_backend_creation() {
    let mut snapshot = snapshot(1, 1, 1.0);
    snapshot.push_fragment(PaintFragment::SvgImage(PaintSvgImage {
        image: moli_layout::PaintSvgImageId::from_index(0),
        destination: PaintRect::new(0.0, 0.0, 1.0, 1.0),
        transform: PaintTransform2D::IDENTITY,
    }));

    assert_eq!(
        raster_snapshot(&snapshot),
        Err(PaintError::MissingSvgImageResource { index: 0 })
    );
}

#[test]
fn border_edges_are_rasterized_inside_the_border_box() {
    let red = PaintColor::new(1.0, 0.0, 0.0, 1.0);
    let mut snapshot = snapshot(8, 8, 1.0);
    snapshot.push_fragment(PaintFragment::border(
        PaintRect::new(1.0, 1.0, 6.0, 6.0),
        PaintEdgeSizes::new(1.0, 1.0, 1.0, 1.0),
        PaintBorderColors::all(red),
    ));

    let image = raster_snapshot(&snapshot).unwrap();
    assert_eq!(pixel(&image, 3, 1), [255, 0, 0, 255]);
    assert_eq!(pixel(&image, 1, 3), [255, 0, 0, 255]);
    assert_eq!(pixel(&image, 3, 3), [255, 255, 255, 255]);
}

#[test]
fn fractional_box_borders_snap_before_rasterization() {
    let green = PaintColor::new(0.0, 1.0, 0.0, 1.0);
    let gray = PaintColor::new(128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0);
    let rect = PaintRect::new(1.09375, 1.0, 6.0, 6.0);
    let mut snapshot = snapshot(8, 8, 1.0);
    snapshot.push_fragment(PaintFragment::solid_rect(rect, green));
    snapshot.push_fragment(PaintFragment::border(
        rect,
        PaintEdgeSizes::new(1.0, 1.0, 1.0, 1.0),
        PaintBorderColors::all(gray),
    ));

    let image = raster_snapshot(&snapshot).unwrap();
    for coordinate in 1..7 {
        assert_eq!(pixel(&image, 1, coordinate), [128, 128, 128, 255]);
        assert_eq!(pixel(&image, 6, coordinate), [128, 128, 128, 255]);
    }
}

#[test]
fn rectangular_clip_scopes_bound_following_fragments() {
    let mut snapshot = snapshot(4, 2, 1.0);
    snapshot.push_fragment(PaintFragment::push_clip(PaintRect::new(0.0, 0.0, 2.0, 2.0)));
    snapshot.push_fragment(PaintFragment::solid_rect(
        PaintRect::new(0.0, 0.0, 4.0, 2.0),
        PaintColor::new(1.0, 0.0, 0.0, 1.0),
    ));
    snapshot.push_fragment(PaintFragment::PopLayer);

    let image = raster_snapshot(&snapshot).unwrap();
    assert_eq!(pixel(&image, 1, 1), [255, 0, 0, 255]);
    assert_eq!(pixel(&image, 3, 1), [255, 255, 255, 255]);
}

#[test]
fn color_filter_layers_are_executed_by_the_one_shot_rgba_fallback() {
    let mut snapshot = snapshot(2, 2, 1.0);
    snapshot.push_fragment(PaintFragment::PushLayer {
        opacity: 1.0,
        blend_mode: PaintBlendMode::Normal,
        composite: PaintCompositeMode::SrcOver,
        clip: PaintShape::Rect(PaintRect::new(0.0, 0.0, 2.0, 2.0)),
        transform: PaintTransform2D::IDENTITY,
        filter: Some(PaintFilter::Brightness(0.0)),
    });
    snapshot.push_fragment(PaintFragment::solid_rect(
        PaintRect::new(0.0, 0.0, 2.0, 2.0),
        PaintColor::new(1.0, 0.0, 0.0, 1.0),
    ));
    snapshot.push_fragment(PaintFragment::PopLayer);

    let image = raster_snapshot(&snapshot).unwrap();
    let pixel = pixel(&image, 1, 1);
    assert!(pixel[0] <= 2 && pixel[1] <= 2 && pixel[2] <= 2);
    assert_eq!(pixel[3], 255);
}

#[test]
fn transparent_raster_and_filter_readbacks_preserve_straight_alpha() {
    for filter in [None, Some(PaintFilter::Invert(1.0))] {
        let mut snapshot = snapshot(2, 2, 1.0);
        snapshot.canvas_color = PaintColor::TRANSPARENT;
        snapshot.push_fragment(PaintFragment::PushLayer {
            opacity: 1.0,
            blend_mode: PaintBlendMode::Normal,
            composite: PaintCompositeMode::SrcOver,
            clip: PaintShape::Rect(PaintRect::new(0.0, 0.0, 2.0, 2.0)),
            transform: PaintTransform2D::IDENTITY,
            filter,
        });
        snapshot.push_fragment(PaintFragment::solid_rect(
            PaintRect::new(0.0, 0.0, 2.0, 2.0),
            PaintColor::new(1.0, 0.0, 0.0, 0.5),
        ));
        snapshot.push_fragment(PaintFragment::PopLayer);
        let image = raster_snapshot(&snapshot).unwrap();
        assert_eq!(
            pixel(&image, 0, 0),
            if filter.is_some() {
                [0, 255, 255, 128]
            } else {
                [255, 0, 0, 128]
            }
        );
    }
}

#[test]
fn backend_filter_handles_inner_clip_layers_without_panicking() {
    let mut snapshot = snapshot(100, 100, 1.0);
    snapshot.push_fragment(PaintFragment::PushLayer {
        opacity: 1.0,
        blend_mode: PaintBlendMode::Normal,
        composite: PaintCompositeMode::SrcOver,
        clip: PaintShape::Rect(PaintRect::new(25.0, 25.0, 50.0, 50.0)),
        transform: PaintTransform2D::IDENTITY,
        filter: Some(PaintFilter::Blur(10.0)),
    });
    snapshot.push_fragment(PaintFragment::push_clip(PaintRect::new(
        30.0, 30.0, 40.0, 40.0,
    )));
    snapshot.push_fragment(PaintFragment::solid_rect(
        PaintRect::new(0.0, 0.0, 100.0, 100.0),
        PaintColor::new(1.0, 0.0, 0.0, 1.0),
    ));
    snapshot.push_fragment(PaintFragment::PopLayer);
    snapshot.push_fragment(PaintFragment::PopLayer);

    let image = raster_snapshot(&snapshot).expect("nested clipped filter must not panic");
    let center = pixel(&image, 50, 50);
    let blurred_edge = pixel(&image, 27, 50);
    assert_eq!(center[0], 255);
    assert!(center[1] < 128 && center[1] == center[2]);
    assert_eq!(center[3], 255);
    assert_eq!(blurred_edge[0], 255);
    assert!(blurred_edge[1] > 0 && blurred_edge[1] < 255);
    assert_eq!(blurred_edge[1], blurred_edge[2]);
    assert!(center[1] < blurred_edge[1]);
    assert_eq!(pixel(&image, 10, 10), [255, 255, 255, 255]);
}

#[test]
fn malformed_clip_and_font_references_are_structured_errors() {
    let mut pop = snapshot(2, 2, 1.0);
    pop.push_fragment(PaintFragment::PopLayer);
    assert_eq!(
        raster_snapshot(&pop),
        Err(PaintError::LayerStackUnderflow { fragment_index: 0 })
    );

    let mut push = snapshot(2, 2, 1.0);
    push.push_fragment(PaintFragment::push_clip(PaintRect::new(0.0, 0.0, 1.0, 1.0)));
    assert_eq!(
        raster_snapshot(&push),
        Err(PaintError::UnclosedLayerStack { depth: 1 })
    );

    let mut glyph = snapshot(2, 2, 1.0);
    glyph.push_fragment(PaintFragment::GlyphRun(moli_layout::PaintGlyphRun {
        text: None,
        font: moli_layout::PaintFontId::from_index(0),
        font_size: 12.0,
        normalized_coords: Vec::new(),
        color: PaintColor::BLACK,
        glyph_skew_radians: None,
        glyph_embolden: moli_layout::PaintPoint::ZERO,
        glyphs: vec![moli_layout::PaintGlyph {
            id: 1,
            x: 0.0,
            y: 1.0,
        }],
        transform: moli_layout::PaintTransform2D::IDENTITY,
        stroke: None,
    }));
    assert_eq!(
        raster_snapshot(&glyph),
        Err(PaintError::MissingFontResource { index: 0 })
    );
}

fn raster_synthetic_oblique_box(skew_degrees: f32) -> RasterImage {
    let mut snapshot = snapshot(80, 60, 1.0);
    let font = peniko::FontData::new(
        Blob::from(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../moli-layout/tests/fixtures/moli-ahem.ttf"
            ))
            .to_vec(),
        ),
        0,
    );
    let font = snapshot.intern_font(&font);
    snapshot.push_fragment(PaintFragment::GlyphRun(moli_layout::PaintGlyphRun {
        text: None,
        font,
        font_size: 50.0,
        normalized_coords: Vec::new(),
        color: PaintColor::BLACK,
        glyph_skew_radians: Some(skew_degrees.to_radians()),
        glyph_embolden: moli_layout::PaintPoint::ZERO,
        // The fixture's .notdef glyph is a deterministic 600 x 800 box.
        glyphs: vec![moli_layout::PaintGlyph {
            id: 0,
            x: 20.0,
            y: 50.0,
        }],
        transform: moli_layout::PaintTransform2D::IDENTITY,
        stroke: None,
    }));
    raster_snapshot(&snapshot).expect("synthetic oblique fixture should rasterize")
}

fn ink_centroid(image: &RasterImage, y: u32) -> f32 {
    let (weighted_x, coverage) = (0..image.width).fold((0.0, 0.0), |acc, x| {
        let ink = f32::from(255 - pixel(image, x, y)[0]);
        (acc.0 + x as f32 * ink, acc.1 + ink)
    });
    assert!(coverage > 0.0, "fixture row {y} must contain glyph ink");
    weighted_x / coverage
}

fn pixel(image: &RasterImage, x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * image.width + x) * 4) as usize;
    image.rgba[offset..offset + 4]
        .try_into()
        .expect("one RGBA pixel")
}

#[test]
fn nested_paths_preserve_nonzero_and_evenodd_fill_rules() {
    use moli_layout::{PaintFillRule, PaintPath, PaintPathElement as E, PaintPoint as P};
    let path = PaintPath {
        bounds: PaintRect::new(0., 0., 16., 16.),
        elements: vec![
            E::MoveTo(P::new(0., 0.)),
            E::LineTo(P::new(16., 0.)),
            E::LineTo(P::new(16., 16.)),
            E::LineTo(P::new(0., 16.)),
            E::Close,
            E::MoveTo(P::new(4., 4.)),
            E::LineTo(P::new(12., 4.)),
            E::LineTo(P::new(12., 12.)),
            E::LineTo(P::new(4., 12.)),
            E::Close,
        ],
    };
    for (rule, center) in [
        (PaintFillRule::NonZero, [0, 0, 0, 255]),
        (PaintFillRule::EvenOdd, [255, 255, 255, 255]),
    ] {
        let mut snapshot = snapshot(16, 16, 1.);
        snapshot.push_fragment(PaintFragment::Fill {
            shape: PaintShape::Path(path.clone()),
            brush: PaintBrush::Solid(PaintColor::BLACK),
            transform: PaintTransform2D::IDENTITY,
            fill_rule: rule,
        });
        let image = raster_snapshot(&snapshot).unwrap();
        assert_eq!(pixel(&image, 1, 1), [0, 0, 0, 255]);
        assert_eq!(pixel(&image, 8, 8), center);
    }
}

fn ahem_run(
    snapshot: &mut PaintSnapshot,
    stroke: Option<moli_layout::PaintTextStroke>,
    embolden: f32,
) -> moli_layout::PaintGlyphRun {
    let font = peniko::FontData::new(
        Blob::from(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../moli-layout/tests/fixtures/moli-ahem.ttf"
            ))
            .to_vec(),
        ),
        0,
    );
    moli_layout::PaintGlyphRun {
        text: None,
        font: snapshot.intern_font(&font),
        font_size: 50.0,
        normalized_coords: Vec::new(),
        color: PaintColor::BLACK,
        glyph_skew_radians: None,
        glyph_embolden: moli_layout::PaintPoint::new(embolden, embolden),
        // The fixture's .notdef glyph is a deterministic 600 x 800 box:
        // x 20..50, y 10..50 at 50px.
        glyphs: vec![moli_layout::PaintGlyph {
            id: 0,
            x: 20.0,
            y: 50.0,
        }],
        transform: moli_layout::PaintTransform2D::IDENTITY,
        stroke,
    }
}

fn raster_stroked_ahem_box(
    stroke: Option<moli_layout::PaintTextStroke>,
    embolden: f32,
) -> RasterImage {
    let mut snapshot = snapshot(80, 60, 1.0);
    let run = ahem_run(&mut snapshot, stroke, embolden);
    snapshot.push_fragment(PaintFragment::GlyphRun(run));
    raster_snapshot(&snapshot).expect("stroked glyph fixture should rasterize")
}

const RED: PaintColor = PaintColor::new(1.0, 0.0, 0.0, 1.0);

fn is_red(px: [u8; 4]) -> bool {
    px[0] > 200 && px[1] < 60 && px[2] < 60
}

fn is_black(px: [u8; 4]) -> bool {
    px[0] < 60 && px[1] < 60 && px[2] < 60
}

#[test]
fn text_stroke_is_centered_on_the_outline_and_painted_over_the_fill() {
    let plain = raster_stroked_ahem_box(None, 0.0);
    assert_eq!(pixel(&plain, 17, 30), [255, 255, 255, 255]);
    assert!(is_black(pixel(&plain, 21, 30)));

    let image = raster_stroked_ahem_box(
        Some(moli_layout::PaintTextStroke {
            width: 8.0,
            color: RED,
            paint_first: false,
        }),
        0.0,
    );
    // Outer half (4px) outside the outline.
    assert!(is_red(pixel(&image, 17, 30)), "{:?}", pixel(&image, 17, 30));
    assert_eq!(pixel(&image, 14, 30), [255, 255, 255, 255]);
    // Inner half is painted over the fill with the default paint-order.
    assert!(is_red(pixel(&image, 21, 30)), "{:?}", pixel(&image, 21, 30));
    assert!(is_black(pixel(&image, 35, 30)));
}

#[test]
fn paint_order_stroke_fill_paints_the_stroke_beneath_the_fill() {
    let image = raster_stroked_ahem_box(
        Some(moli_layout::PaintTextStroke {
            width: 8.0,
            color: RED,
            paint_first: true,
        }),
        0.0,
    );
    assert!(is_red(pixel(&image, 17, 30)), "{:?}", pixel(&image, 17, 30));
    // The fill covers the inner half of the stroke.
    assert!(
        is_black(pixel(&image, 21, 30)),
        "{:?}",
        pixel(&image, 21, 30)
    );
    assert!(is_black(pixel(&image, 35, 30)));
}

#[test]
fn text_stroke_uses_miter_joins_and_bevels_above_limit_four() {
    let mut snapshot = snapshot(240, 180, 1.0);
    let font = peniko::FontData::new(
        Blob::from(include_bytes!("../../tests/fixtures/moli-text-stroke.ttf").to_vec()),
        0,
    );
    let mut run = ahem_run(&mut snapshot, None, 0.0);
    run.font = snapshot.intern_font(&font);
    run.font_size = 100.0;
    run.color = PaintColor::TRANSPARENT;
    run.stroke = Some(moli_layout::PaintTextStroke {
        width: 10.0,
        color: RED,
        paint_first: false,
    });
    run.glyphs = vec![
        moli_layout::PaintGlyph {
            id: 1,
            x: 50.0,
            y: 130.0,
        },
        moli_layout::PaintGlyph {
            id: 2,
            x: 150.0,
            y: 130.0,
        },
    ];
    snapshot.push_fragment(PaintFragment::GlyphRun(run));
    let image = raster_snapshot(&snapshot).unwrap();
    // A's miter ratio is 2.85: the pointed join reaches above a round join.
    assert!(is_red(pixel(&image, 80, 38)), "{:?}", pixel(&image, 80, 38));
    // B's ratio is 8.06: limit 4 bevels the tip instead of rounding it or
    // allowing the long spike produced by PDF's default limit of 10.
    assert_eq!(pixel(&image, 180, 43), [255, 255, 255, 255]);
    assert!(is_red(pixel(&image, 180, 52)));
}

#[test]
fn text_stroke_follows_synthetic_bold_outline_expansion() {
    let stroke = Some(moli_layout::PaintTextStroke {
        width: 2.0,
        color: RED,
        paint_first: true,
    });
    // Embolden is expressed in local CSS pixels, independent of the font's
    // UPEM: 2px moves the fill edge to x=18 and the stroke to 17..19.
    let regular = raster_stroked_ahem_box(stroke, 0.0);
    let bold = raster_stroked_ahem_box(stroke, 2.0);
    let bold_fill = raster_stroked_ahem_box(None, 2.0);
    assert_eq!(pixel(&regular, 17, 30), [255, 255, 255, 255]);
    assert!(
        is_black(pixel(&bold_fill, 18, 30)),
        "{:?}",
        pixel(&bold_fill, 18, 30)
    );
    assert_eq!(pixel(&bold_fill, 17, 30), [255, 255, 255, 255]);
    assert!(is_red(pixel(&bold, 17, 30)), "{:?}", pixel(&bold, 17, 30));
    assert!(is_black(pixel(&bold, 18, 30)), "{:?}", pixel(&bold, 18, 30));
    assert_eq!(pixel(&bold, 16, 30), [255, 255, 255, 255]);
}

#[test]
fn synthetic_bold_fill_and_stroke_share_css_pixel_geometry_across_upem_and_sizes() {
    use moli_layout::{PaintFillRule, PaintPath, PaintPathElement as E, PaintPoint, PaintStroke};
    use skrifa::MetadataProvider;
    for bytes in [
        include_bytes!("../../tests/fixtures/moli-text-stroke.ttf").as_slice(),
        include_bytes!("../../tests/fixtures/moli-text-stroke-250.ttf").as_slice(),
    ] {
        let font = peniko::FontData::new(Blob::from(bytes.to_vec()), 0);
        let face = skrifa::FontRef::from_index(bytes, 0).unwrap();
        let id = face.charmap().map('R').unwrap().to_u32();
        for font_size in [20.0, 50.0, 80.0] {
            for order in [None, Some(false), Some(true)] {
                let mut actual = snapshot(140, 140, 1.0);
                let mut run = ahem_run(&mut actual, None, 0.3);
                run.font = actual.intern_font(&font);
                run.font_size = font_size;
                run.glyphs[0] = moli_layout::PaintGlyph {
                    id,
                    x: 40.0,
                    y: 100.0,
                };
                run.stroke = order.map(|paint_first| moli_layout::PaintTextStroke {
                    width: 2.0,
                    color: RED,
                    paint_first,
                });
                actual.push_fragment(PaintFragment::GlyphRun(run));

                // Ground truth is an ordinary rectangle expanded by 0.3px,
                // with a 2px stroke around that new contour. In particular the
                // inner stroke edge must not move another 0.6px into the fill.
                let (left, top, right, bottom) = (
                    39.7,
                    100.0 - font_size * 0.8 - 0.3,
                    40.0 + font_size * 0.6 + 0.3,
                    100.3,
                );
                let rect = PaintRect::new(left, top, right - left, bottom - top);
                let fill = PaintFragment::Fill {
                    fill_rule: PaintFillRule::NonZero,
                    shape: PaintShape::Rect(rect),
                    brush: PaintBrush::Solid(PaintColor::BLACK),
                    transform: PaintTransform2D::IDENTITY,
                };
                let stroke = PaintFragment::Stroke(PaintStroke {
                    path: PaintPath {
                        bounds: rect,
                        elements: vec![
                            E::MoveTo(PaintPoint::new(left, top)),
                            E::LineTo(PaintPoint::new(right, top)),
                            E::LineTo(PaintPoint::new(right, bottom)),
                            E::LineTo(PaintPoint::new(left, bottom)),
                            E::Close,
                        ],
                    },
                    width: 2.0,
                    color: RED,
                    join: moli_layout::PaintLineJoin::Miter,
                    start_cap: moli_layout::PaintLineCap::Butt,
                    end_cap: moli_layout::PaintLineCap::Butt,
                    miter_limit: 4.0,
                    dash_pattern: Vec::new(),
                    dash_offset: 0.0,
                    transform: PaintTransform2D::IDENTITY,
                });
                let mut reference = snapshot(140, 140, 1.0);
                if order == Some(true) {
                    reference.push_fragment(stroke.clone());
                }
                reference.push_fragment(fill);
                if order == Some(false) {
                    reference.push_fragment(stroke);
                }
                let actual = raster_snapshot(&actual).unwrap();
                let reference = raster_snapshot(&reference).unwrap();
                let difference = actual
                    .rgba
                    .iter()
                    .zip(&reference.rgba)
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap();
                assert!(
                    difference <= 1,
                    "size={font_size}, order={order:?}, difference={difference}"
                );
            }
        }
    }
}

#[test]
fn transparent_or_zero_width_text_stroke_paints_nothing() {
    let plain = raster_stroked_ahem_box(None, 0.0);
    for stroke in [
        moli_layout::PaintTextStroke {
            width: 0.0,
            color: RED,
            paint_first: false,
        },
        moli_layout::PaintTextStroke {
            width: 8.0,
            color: PaintColor::TRANSPARENT,
            paint_first: false,
        },
        moli_layout::PaintTextStroke {
            width: f32::NAN,
            color: RED,
            paint_first: false,
        },
    ] {
        assert_eq!(raster_stroked_ahem_box(Some(stroke), 0.0), plain);
    }
}

#[test]
fn html_text_shadow_uses_fill_ink_in_both_paint_orders() {
    // Chromium's GraphicsContext::DrawTextPasses clears the shadow looper on
    // the stroke pass when fill mode is present, in either paint order:
    // https://github.com/chromium/chromium/blob/main/third_party/blink/renderer/platform/graphics/graphics_context.cc
    // This contract is for ordinary HTML text; SVG has its own paint path.
    let mut reference = None;
    for stroke in [None, Some(false), Some(true)] {
        let mut snapshot = snapshot(140, 60, 1.0);
        let run = ahem_run(
            &mut snapshot,
            stroke.map(|paint_first| moli_layout::PaintTextStroke {
                width: 8.0,
                color: RED,
                paint_first,
            }),
            0.0,
        );
        snapshot.push_fragment(PaintFragment::TextShadow(moli_layout::PaintTextShadow {
            run: run.clone(),
            color: PaintColor::new(0.0, 1.0, 0.0, 1.0),
            offset: moli_layout::PaintPoint::new(80.0, 0.0),
            blur_radius: 0.0,
        }));
        snapshot.push_fragment(PaintFragment::GlyphRun(run));
        let image = raster_snapshot(&snapshot).expect("shadowed glyph should rasterize");
        assert_eq!(pixel(&image, 115, 30), [0, 255, 0, 255]);
        assert_eq!(pixel(&image, 98, 30), [255, 255, 255, 255]);
        let shadow_pixels: Vec<_> = (0..60)
            .flat_map(|y| (80..140).map(move |x| (x, y)))
            .map(|(x, y)| pixel(&image, x, y))
            .collect();
        if let Some(reference) = &reference {
            assert_eq!(
                &shadow_pixels, reference,
                "shadow must match unstroked text, stroke_first={stroke:?}"
            );
        } else {
            reference = Some(shadow_pixels);
        }
    }
}

// ---- backdrop-filter -------------------------------------------------------

fn backdrop(rect: PaintRect, filters: Vec<PaintFilter>) -> PaintFragment {
    PaintFragment::BackdropFilter {
        shape: PaintShape::Rect(rect),
        transform: PaintTransform2D::IDENTITY,
        filters,
        own_scopes: 0,
    }
}

fn effect_layer(opacity: f32, blend_mode: PaintBlendMode) -> PaintFragment {
    PaintFragment::PushLayer {
        opacity,
        blend_mode,
        composite: PaintCompositeMode::SrcOver,
        clip: PaintShape::Rect(PaintRect::new(0.0, 0.0, 40.0, 40.0)),
        transform: PaintTransform2D::IDENTITY,
        filter: None,
    }
}

fn assert_near(actual: [u8; 4], expected: [u8; 4], tolerance: u8) {
    assert!(
        actual
            .iter()
            .zip(expected)
            .all(|(a, e)| a.abs_diff(e) <= tolerance),
        "expected {expected:?} +/- {tolerance}, got {actual:?}"
    );
}

#[test]
fn backdrop_inside_ancestor_opacity_applies_the_opacity_once() {
    // Filter Effects 2: the ancestor opacity layer is the Backdrop Root, so the
    // backdrop is the layer's own content (opaque blue) and the layer opacity
    // is applied exactly once when the root composites over the white canvas.
    let rect = PaintRect::new(0.0, 0.0, 40.0, 40.0);
    let mut snapshot = snapshot(40, 40, 1.0);
    snapshot.push_fragment(effect_layer(0.5, PaintBlendMode::Normal));
    snapshot.push_fragment(PaintFragment::solid_rect(
        rect,
        PaintColor::new(0.0, 0.0, 1.0, 1.0),
    ));
    snapshot.push_fragment(backdrop(rect, vec![PaintFilter::Brightness(1.0)]));
    snapshot.push_fragment(PaintFragment::PopLayer);

    let image = raster_snapshot(&snapshot).expect("backdrop should rasterize");
    assert_near(pixel(&image, 20, 20), [128, 128, 255, 255], 2);
}

#[test]
fn backdrop_inside_ancestor_blend_applies_the_blend_once() {
    let rect = PaintRect::new(0.0, 0.0, 40.0, 40.0);
    let grey = PaintColor::new(128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0);
    let mut snapshot = snapshot(40, 40, 1.0);
    snapshot.push_fragment(PaintFragment::solid_rect(rect, grey));
    snapshot.push_fragment(effect_layer(1.0, PaintBlendMode::Multiply));
    snapshot.push_fragment(PaintFragment::solid_rect(rect, grey));
    snapshot.push_fragment(backdrop(rect, vec![PaintFilter::Brightness(1.0)]));
    snapshot.push_fragment(PaintFragment::PopLayer);

    let image = raster_snapshot(&snapshot).expect("backdrop should rasterize");
    // grey multiply grey = 64; a second multiply would give 32.
    assert_near(pixel(&image, 20, 20), [64, 64, 64, 255], 2);
}

fn rounded_backdrop(rect: PaintRect, radius: f32, filters: Vec<PaintFilter>) -> PaintFragment {
    PaintFragment::BackdropFilter {
        shape: PaintShape::RoundedRect {
            rect,
            radii: PaintCornerRadii::all(PaintCornerRadius::new(radius, radius)),
        },
        transform: PaintTransform2D::IDENTITY,
        filters,
        own_scopes: 0,
    }
}

/// White canvas with 2px black stripes at even 2px columns: x % 4 in {0, 1}.
fn striped(width: u32, height: u32) -> PaintSnapshot {
    let mut snapshot = snapshot(width, height, 1.0);
    for x in (0..width).step_by(4) {
        snapshot.push_fragment(PaintFragment::solid_rect(
            PaintRect::new(x as f32, 0.0, 2.0, height as f32),
            PaintColor::BLACK,
        ));
    }
    snapshot
}

fn stripe_value(x: u32) -> [u8; 4] {
    if x % 4 < 2 {
        [0, 0, 0, 255]
    } else {
        [255, 255, 255, 255]
    }
}

#[test]
fn backdrop_blur_stays_inside_the_rounded_border_box_with_mirrored_edges() {
    let mut snapshot = striped(60, 60);
    snapshot.push_fragment(rounded_backdrop(
        PaintRect::new(10.0, 10.0, 40.0, 40.0),
        10.0,
        vec![PaintFilter::Blur(4.0)],
    ));
    let image = raster_snapshot(&snapshot).expect("backdrop blur should rasterize");

    // Inside: stripes average to mid grey.
    for (x, y) in [(29, 30), (30, 30), (31, 30), (32, 30)] {
        assert_near(pixel(&image, x, y), [128, 128, 128, 255], 24);
    }
    // Outside the border box: untouched.
    for (x, y) in [
        (5, 30),
        (6, 30),
        (55, 30),
        (30, 5),
        (31, 55),
        (30, 9),
        (50, 30),
    ] {
        assert_eq!(pixel(&image, x, y), stripe_value(x), "({x}, {y})");
    }
    // Inside the bounding box but outside the 10px corner radius: untouched.
    for (x, y) in [(10, 10), (11, 11), (49, 10), (48, 48), (10, 49)] {
        assert_eq!(pixel(&image, x, y), stripe_value(x), "corner ({x}, {y})");
    }
    // Edge rows/columns sample mirrored backdrop, not transparent black:
    // they stay mid grey over both black and white stripes.
    for (x, y) in [
        (30, 10),
        (31, 10),
        (32, 10),
        (33, 10),
        (10, 30),
        (49, 30),
        (30, 49),
    ] {
        assert_near(pixel(&image, x, y), [128, 128, 128, 255], 40);
    }
}

#[test]
fn backdrop_root_skips_the_elements_own_layers() {
    // Layout emits the element's own opacity layer before its backdrop; that
    // layer is not the Backdrop Root, so the canvas-rooted backdrop (blue) is
    // inverted to yellow and faded once by the element's opacity.
    let rect = PaintRect::new(0.0, 0.0, 40.0, 40.0);
    let mut snapshot = snapshot(40, 40, 1.0);
    snapshot.push_fragment(PaintFragment::solid_rect(
        rect,
        PaintColor::new(0.0, 0.0, 1.0, 1.0),
    ));
    snapshot.push_fragment(effect_layer(0.5, PaintBlendMode::Normal));
    snapshot.push_fragment(PaintFragment::BackdropFilter {
        shape: PaintShape::Rect(rect),
        transform: PaintTransform2D::IDENTITY,
        filters: vec![PaintFilter::Invert(1.0)],
        own_scopes: 1,
    });
    snapshot.push_fragment(PaintFragment::PopLayer);

    let image = raster_snapshot(&snapshot).expect("backdrop should rasterize");
    assert_near(pixel(&image, 20, 20), [128, 128, 128, 255], 2);
}

#[test]
fn backdrop_inside_a_software_color_filter_layer_reads_only_that_layer() {
    let mut snapshot = snapshot(40, 40, 1.0);
    snapshot.push_fragment(PaintFragment::solid_rect(
        PaintRect::new(0.0, 0.0, 40.0, 40.0),
        PaintColor::new(1.0, 0.0, 0.0, 1.0),
    ));
    snapshot.push_fragment(PaintFragment::PushLayer {
        opacity: 1.0,
        blend_mode: PaintBlendMode::Normal,
        composite: PaintCompositeMode::SrcOver,
        clip: PaintShape::Rect(PaintRect::new(0.0, 0.0, 40.0, 40.0)),
        transform: PaintTransform2D::IDENTITY,
        filter: Some(PaintFilter::Grayscale(1.0)),
    });
    snapshot.push_fragment(PaintFragment::solid_rect(
        PaintRect::new(0.0, 0.0, 20.0, 40.0),
        PaintColor::new(0.0, 0.0, 1.0, 1.0),
    ));
    snapshot.push_fragment(backdrop(
        PaintRect::new(0.0, 0.0, 40.0, 40.0),
        vec![PaintFilter::Invert(1.0)],
    ));
    snapshot.push_fragment(PaintFragment::PopLayer);

    let image = raster_snapshot(&snapshot).expect("backdrop should rasterize");
    // The red canvas fill is outside the Backdrop Root and stays visible.
    assert_eq!(pixel(&image, 30, 20), [255, 0, 0, 255]);
    // Blue inverted to yellow, then the root's grayscale applies once.
    let [r, g, b, a] = pixel(&image, 10, 20);
    assert!(
        r == g && g == b && r > 200 && a == 255,
        "{:?}",
        [r, g, b, a]
    );
}

#[test]
fn backdrop_root_marker_makes_a_clip_scope_the_backdrop_root() {
    // A clip-path scope followed by BackdropRoot is a Backdrop Root, like
    // Chromium: the child backdrop reads only what the scope painted, so the
    // red canvas outside it is not inverted into the child's region. Without
    // the marker the clip is an ordinary overflow clip and the canvas is read.
    let rect = PaintRect::new(0.0, 0.0, 40.0, 40.0);
    let render = |marked: bool| {
        let mut snapshot = snapshot(40, 40, 1.0);
        snapshot.push_fragment(PaintFragment::solid_rect(
            rect,
            PaintColor::new(1.0, 0.0, 0.0, 1.0),
        ));
        snapshot.push_fragment(PaintFragment::PushClip {
            shape: PaintShape::Rect(rect),
            transform: PaintTransform2D::IDENTITY,
        });
        if marked {
            snapshot.push_fragment(PaintFragment::BackdropRoot);
        }
        snapshot.push_fragment(backdrop(rect, vec![PaintFilter::Invert(1.0)]));
        snapshot.push_fragment(PaintFragment::PopLayer);
        pixel(
            &raster_snapshot(&snapshot).expect("backdrop should rasterize"),
            20,
            20,
        )
    };
    // Red inverted to cyan when the canvas is the root; untouched when the clip is.
    assert_near(render(false), [0, 255, 255, 255], 2);
    assert_eq!(render(true), [255, 0, 0, 255]);
}

#[test]
fn backdrop_root_marker_makes_an_isolation_layer_the_backdrop_root() {
    // An opacity-1 layer is not a root by itself; the marker makes it one, which
    // is how a backdrop-filter ancestor scopes its descendants' backdrops.
    let rect = PaintRect::new(0.0, 0.0, 40.0, 40.0);
    let mut snapshot = snapshot(40, 40, 1.0);
    snapshot.push_fragment(PaintFragment::solid_rect(
        rect,
        PaintColor::new(0.0, 0.0, 1.0, 1.0),
    ));
    snapshot.push_fragment(effect_layer(1.0, PaintBlendMode::Normal));
    snapshot.push_fragment(PaintFragment::BackdropRoot);
    snapshot.push_fragment(backdrop(rect, vec![PaintFilter::Invert(1.0)]));
    snapshot.push_fragment(PaintFragment::PopLayer);
    let image = raster_snapshot(&snapshot).expect("backdrop should rasterize");
    // The canvas blue is outside the root, so nothing is inverted over it.
    assert_eq!(pixel(&image, 20, 20), [0, 0, 255, 255]);
}

fn entry_at(index: usize, root_start: usize) -> super::backdrop::TestEntry {
    super::backdrop::TestEntry { index, root_start }
}

#[test]
fn last_use_matches_the_naive_prefix_scan_for_nested_and_independent_roots() {
    // (index, root_start) in paint order; nested, sibling and canvas roots.
    let entries = [
        (2, 0),
        (5, 0),
        (7, 6),
        (9, 6),
        (12, 0),
        (14, 13),
        (15, 13),
        (20, 0),
        (22, 21),
        (25, 0),
    ]
    .map(|(index, root_start)| entry_at(index, root_start));
    let mut expected: Vec<usize> = entries.iter().map(|entry| entry.index).collect();
    for (position, entry) in entries.iter().enumerate() {
        for earlier in (0..position).rev() {
            if entries[earlier].index < entry.root_start {
                break;
            }
            expected[earlier] = entry.index;
        }
    }
    assert_eq!(super::backdrop::last_use_for_test(&entries), expected);
}

#[test]
fn backdrop_bookkeeping_is_not_quadratic_when_every_effect_is_skipped() {
    // 40,000 canvas-rooted backdrops used to cost ~800 million prefix updates in
    // planning alone; a zero allowance skips them all, so only bookkeeping runs.
    let entries: Vec<_> = (0..40_000).map(|i| entry_at(i + 1, 0)).collect();
    let started = std::time::Instant::now();
    let last_use = super::backdrop::last_use_for_test(&entries);
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(last_use[0], 40_000);
    assert_eq!(last_use[39_999], 40_000);
}

#[test]
fn pdf_effect_group_fallback_does_not_render_backdrop_filter() {
    // A backdrop inside a software color-filter group is rasterized by the PDF
    // painter; PDF export promises to omit backdrop-filter, so the fallback
    // must not draw it (the raster path would invert the blue to yellow).
    let rect = PaintRect::new(0.0, 0.0, 40.0, 40.0);
    let mut snapshot = snapshot(40, 40, 1.0);
    snapshot.push_fragment(PaintFragment::PushLayer {
        opacity: 1.0,
        blend_mode: PaintBlendMode::Normal,
        composite: PaintCompositeMode::SrcOver,
        clip: PaintShape::Rect(rect),
        transform: PaintTransform2D::IDENTITY,
        filter: Some(PaintFilter::Grayscale(1.0)),
    });
    snapshot.push_fragment(PaintFragment::solid_rect(
        rect,
        PaintColor::new(0.0, 0.0, 1.0, 1.0),
    ));
    snapshot.push_fragment(backdrop(rect, vec![PaintFilter::Invert(1.0)]));
    snapshot.push_fragment(PaintFragment::PopLayer);
    let with_backdrop = crate::pdf::pdf_snapshot(&snapshot, &crate::pdf::PdfOptions::default())
        .expect("PDF export skips backdrop-filter");
    snapshot
        .fragments
        .retain(|fragment| !matches!(fragment, PaintFragment::BackdropFilter { .. }));
    let without_backdrop = crate::pdf::pdf_snapshot(&snapshot, &crate::pdf::PdfOptions::default())
        .expect("PDF export of the same page");
    assert_eq!(with_backdrop, without_backdrop);
}

fn half_invert_snapshot() -> PaintSnapshot {
    let mut snapshot = snapshot(40, 40, 1.0);
    snapshot.push_fragment(PaintFragment::solid_rect(
        PaintRect::new(0.0, 0.0, 20.0, 40.0),
        PaintColor::BLACK,
    ));
    snapshot.push_fragment(backdrop(
        PaintRect::new(0.0, 0.0, 20.0, 40.0),
        vec![PaintFilter::Invert(1.0)],
    ));
    snapshot.push_fragment(backdrop(
        PaintRect::new(20.0, 0.0, 20.0, 40.0),
        vec![PaintFilter::Invert(1.0)],
    ));
    snapshot
}

#[test]
fn backdrops_within_budget_render_without_diagnostics() {
    let output = raster_snapshot_with_diagnostics(&half_invert_snapshot())
        .expect("backdrops should rasterize");
    assert!(output.diagnostics.is_empty());
    assert_eq!(pixel(&output.image, 10, 20), [255, 255, 255, 255]);
    assert_eq!(pixel(&output.image, 30, 20), [0, 0, 0, 255]);
}

#[test]
fn backdrops_over_transient_budget_degrade_with_a_diagnostic() {
    // 40x40 surface: 6_400 bytes; each region 20x40: 3_200 bytes. The output,
    // one capture surface and two working buffers take 19_200 bytes.
    // Each cached region also reserves its final/capture premultiplied copies,
    // so 9_600 more bytes admit exactly one region.
    let output = raster_with_budget(&half_invert_snapshot(), 28_800)
        .expect("over-budget backdrops degrade instead of failing");
    assert_eq!(pixel(&output.image, 10, 20), [255, 255, 255, 255]);
    // The second backdrop is skipped: its region keeps the unfiltered white.
    assert_eq!(pixel(&output.image, 30, 20), [255, 255, 255, 255]);
    assert_eq!(output.diagnostics.len(), 1);
    assert_eq!(
        output.diagnostics[0].code,
        "backdrop-filter-budget-exceeded"
    );
    assert!(
        output.diagnostics[0].message.starts_with("skipped 1 of 2"),
        "{}",
        output.diagnostics[0].message
    );
    assert_eq!(
        output.diagnostics[0].severity,
        moli_layout::PaintDiagnosticSeverity::Warning
    );
}

#[test]
fn backdrop_bytes_are_charged_against_the_transient_raster_budget() {
    let dimensions = validate_dimensions(PaintCaptureSurface::new(4_096.0, 4_096.0, 1.0))
        .expect("dimensions supported by the backend");
    let mut snapshot = snapshot(4_096, 4_096, 1.0);
    for _ in 0..3 {
        snapshot.push_fragment(backdrop(
            PaintRect::new(0.0, 0.0, 4_096.0, 4_096.0),
            vec![PaintFilter::Blur(1.0)],
        ));
    }
    let stream = validate_fragment_stream(&snapshot).expect("valid stream");
    assert_eq!(stream.backdrop_count, 3);
    assert_eq!(stream.backdrop_bytes, 3 * 67_108_864);
    assert_eq!(stream.max_backdrop_bytes, 67_108_864);
    // Output + capture + two blur buffers already fill the 256 MiB budget,
    // so every region degrades rather than failing the capture.
    assert_eq!(validate_resource_budget(dimensions, stream), Ok(()));
    assert_eq!(
        backdrop_allowance(dimensions, stream, MAX_TRANSIENT_RASTER_BYTES),
        0
    );
}

#[test]
fn pdf_export_reports_backdrop_filter_as_unsupported() {
    let snapshot = half_invert_snapshot();
    crate::pdf::pdf_snapshot(&snapshot, &crate::pdf::PdfOptions::default())
        .expect("PDF export skips backdrop-filter");
    let diagnostics = crate::pdf::pdf_snapshot_diagnostics(&snapshot);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "backdrop-filter-unsupported");
}

#[test]
fn huge_backdrop_blur_is_clamped_to_the_region() {
    let started = std::time::Instant::now();
    let mut snapshot = striped(40, 40);
    snapshot.push_fragment(backdrop(
        PaintRect::new(0.0, 0.0, 40.0, 40.0),
        vec![PaintFilter::Blur(1.0e9)],
    ));
    let image = raster_snapshot(&snapshot).expect("huge blur should rasterize");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "clamped blur took {:?}",
        started.elapsed()
    );
    // A region-sized mirrored blur flattens the stripes.
    for (x, y) in [(0, 0), (1, 20), (20, 20), (39, 39)] {
        assert_near(pixel(&image, x, y), [128, 128, 128, 255], 24);
    }
}

#[test]
fn non_finite_backdrop_blur_is_ignored() {
    for radius in [f32::NAN, f32::INFINITY, -4.0] {
        let mut snapshot = striped(20, 20);
        snapshot.push_fragment(backdrop(
            PaintRect::new(0.0, 0.0, 20.0, 20.0),
            vec![PaintFilter::Blur(radius)],
        ));
        let image = raster_snapshot(&snapshot).expect("invalid blur should rasterize");
        for x in 0..20 {
            assert_eq!(pixel(&image, x, 10), stripe_value(x), "radius {radius}");
        }
    }
}

#[test]
fn limitation_element_filter_does_not_apply_to_its_backdrop() {
    // Pins current behavior: layout pushes the element's own `filter` layers
    // after its BackdropFilter, so `filter: invert(1)` affects the element's
    // content but not the filtered backdrop (Chromium applies both).
    let rect = PaintRect::new(0.0, 0.0, 40.0, 40.0);
    let mut snapshot = snapshot(40, 40, 1.0);
    snapshot.push_fragment(PaintFragment::solid_rect(rect, PaintColor::BLACK));
    snapshot.push_fragment(backdrop(rect, vec![PaintFilter::Brightness(1.0)]));
    snapshot.push_fragment(PaintFragment::PushLayer {
        opacity: 1.0,
        blend_mode: PaintBlendMode::Normal,
        composite: PaintCompositeMode::SrcOver,
        clip: PaintShape::Rect(rect),
        transform: PaintTransform2D::IDENTITY,
        filter: Some(PaintFilter::Invert(1.0)),
    });
    snapshot.push_fragment(PaintFragment::PopLayer);

    let image = raster_snapshot(&snapshot).expect("backdrop should rasterize");
    assert_eq!(pixel(&image, 20, 20), [0, 0, 0, 255]);
}

#[test]
fn limitation_drop_shadow_in_raster_backdrop_list_is_ignored() {
    // Layout strips drop-shadow() and reports it; raster tolerates it.
    let mut snapshot = striped(20, 20);
    snapshot.push_fragment(backdrop(
        PaintRect::new(0.0, 0.0, 20.0, 20.0),
        vec![PaintFilter::DropShadow {
            offset: moli_layout::PaintPoint::new(4.0, 4.0),
            blur_radius: 2.0,
            color: PaintColor::new(1.0, 0.0, 0.0, 1.0),
        }],
    ));
    let output = raster_snapshot_with_diagnostics(&snapshot).expect("backdrop should rasterize");
    assert_eq!(output.diagnostics.len(), 1);
    assert_eq!(
        output.diagnostics[0].code,
        "backdrop-filter-drop-shadow-unsupported"
    );
    for x in 0..20 {
        assert_eq!(pixel(&output.image, x, 10), stripe_value(x));
    }
}

#[test]
fn backdrop_blur_has_a_fixed_device_pixel_cap() {
    let render = |radius| {
        let mut snapshot = snapshot(320, 32, 1.0);
        snapshot.push_fragment(PaintFragment::solid_rect(
            PaintRect::new(0.0, 0.0, 160.0, 32.0),
            PaintColor::BLACK,
        ));
        snapshot.push_fragment(backdrop(
            PaintRect::new(0.0, 0.0, 320.0, 32.0),
            vec![PaintFilter::Blur(radius)],
        ));
        raster_snapshot(&snapshot).unwrap()
    };
    assert_eq!(render(1.0e9), render(128.0));
}

#[test]
fn backdrop_blur_expanded_backend_surfaces_are_budgeted() {
    let plain = striped(40, 40);
    let mut blurred = plain.clone();
    blurred.push_fragment(backdrop(
        PaintRect::new(0.0, 0.0, 40.0, 40.0),
        vec![PaintFilter::Blur(4.0)],
    ));
    let stream = validate_fragment_stream(&blurred).unwrap();
    // Mirror sampling and Vello's filter padding require more than two
    // unexpanded region buffers. A budget that fits those must still degrade.
    assert!(stream.max_backdrop_workspace_bytes > 2 * stream.max_backdrop_bytes);
    let output = raster_with_budget(&blurred, 5 * 40 * 40 * 4).unwrap();
    assert_eq!(output.image, raster_snapshot(&plain).unwrap());
    assert_eq!(
        output.diagnostics[0].code,
        "backdrop-filter-budget-exceeded"
    );
}
