// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Narrowly adapted from DioxusLabs/blitz commit
// d788124ab881f9bb537cb452ec1d837604a374a8:
// - examples/screenshot.rs
// - packages/blitz-paint/src/render.rs
// - crates/anyrender_svg (parsed SVG tree execution)
// The source project is licensed MIT OR Apache-2.0.

//! Raster output and resource-budget regressions.

use moli_layout::{
    PaintBlendMode, PaintBorderColors, PaintBorderStyle, PaintBorderStyles, PaintBrush,
    PaintCaptureSurface, PaintColor, PaintCompositeMode, PaintCornerRadii, PaintCornerRadius,
    PaintEdgeSizes, PaintFilter, PaintFragment, PaintImage, PaintImageSampling, PaintRect,
    PaintShape, PaintSnapshot, PaintSvgImage, PaintTransform2D, PaintViewport,
};
use peniko::Blob;
use std::sync::Arc;

use super::*;

fn snapshot(width: u32, height: u32, scale: f32) -> PaintSnapshot {
    PaintSnapshot::new(PaintViewport::new(width, height, scale), PaintColor::WHITE)
}

#[test]
fn rasterizes_into_existing_bgra_storage_with_straight_alpha() {
    let snapshot = PaintSnapshot::new(
        PaintViewport::new(2, 1, 2.),
        PaintColor::new(0.75, 0.5, 0.25, 0.5),
    );
    let layout = raster_buffer_layout(2., 1., 2.).unwrap();
    let expected = raster_snapshot(&snapshot).unwrap();
    let mut bytes = vec![255; layout.byte_len];
    let supplied_address = bytes.as_ptr();

    assert!(matches!(
        raster_snapshot_into(&snapshot, &mut bytes[..4], RasterPixelFormat::Bgra8),
        Err(PaintError::UnexpectedBufferLength { .. })
    ));
    assert_eq!(
        raster_snapshot_into(&snapshot, &mut bytes, RasterPixelFormat::Bgra8).unwrap(),
        layout
    );
    assert_eq!(supplied_address, bytes.as_ptr());
    let expected_pixels = expected.rgba.as_chunks::<4>().0;
    let actual_pixels = bytes.as_chunks::<4>().0;
    for (rgba, bgra) in expected_pixels.iter().zip(actual_pixels) {
        assert_eq!(*bgra, [rgba[2], rgba[1], rgba[0], rgba[3]]);
    }
    assert!(bytes[3] < 255);
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
fn rounded_border_keeps_top_arcs_when_the_bottom_edge_is_transparent() {
    let radius = PaintCornerRadius::new(26.0, 26.0);
    for bottom_width in [0.0, 1.0] {
        let mut snapshot = snapshot(128, 60, 2.0);
        snapshot.push_fragment(PaintFragment::Border {
            rect: PaintRect::new(4.0, 4.0, 120.0, 52.0),
            widths: PaintEdgeSizes::new(1.0, 1.0, bottom_width, 1.0),
            colors: PaintBorderColors {
                bottom: PaintColor::TRANSPARENT,
                ..PaintBorderColors::all(PaintColor::new(0.0, 0.0, 0.0, 1.0))
            },
            styles: PaintBorderStyles::default(),
            radii: PaintCornerRadii {
                top_left: radius,
                top_right: radius,
                ..PaintCornerRadii::default()
            },
            transform: PaintTransform2D::IDENTITY,
        });
        let image = raster_snapshot(&snapshot).expect("rounded border should rasterize");
        for (x, y) in [(23, 23), (232, 23)] {
            assert!(
                pixel(&image, x, y)[0] < 100,
                "rounded join at ({x}, {y}) should be painted with bottom width {bottom_width}"
            );
        }
        assert_eq!(pixel(&image, 8, 8), [255; 4]);
        assert_eq!(pixel(&image, 128, 111), [255; 4]);
    }
}

#[test]
fn rounded_border_miter_preserves_adjacent_edge_colors() {
    let mut snapshot = snapshot(128, 60, 2.0);
    snapshot.push_fragment(PaintFragment::Border {
        rect: PaintRect::new(4.0, 4.0, 120.0, 52.0),
        widths: PaintEdgeSizes::new(1.0, 1.0, 1.0, 1.0),
        colors: PaintBorderColors {
            top: PaintColor::new(1.0, 0.0, 0.0, 1.0),
            left: PaintColor::new(0.0, 0.0, 1.0, 1.0),
            ..PaintBorderColors::all(PaintColor::TRANSPARENT)
        },
        styles: PaintBorderStyles::default(),
        radii: PaintCornerRadii {
            top_left: PaintCornerRadius::new(26.0, 26.0),
            ..PaintCornerRadii::default()
        },
        transform: PaintTransform2D::IDENTITY,
    });
    let image = raster_snapshot(&snapshot).expect("two-color rounded border should rasterize");
    let top_arc = pixel(&image, 34, 15);
    let left_arc = pixel(&image, 15, 34);
    assert!(top_arc[0] == 255 && top_arc[2] < 100, "{top_arc:?}");
    assert!(left_arc[2] == 255 && left_arc[0] < 100, "{left_arc:?}");
}

#[test]
fn rounded_border_miter_stays_clear_of_the_opposite_corner() {
    let mut snapshot = snapshot(100, 100, 1.0);
    let large_radius = PaintCornerRadius::new(90.0, 90.0);
    snapshot.push_fragment(PaintFragment::Border {
        rect: PaintRect::new(0.0, 0.0, 100.0, 100.0),
        widths: PaintEdgeSizes::new(5.0, 5.0, 5.0, 5.0),
        colors: PaintBorderColors {
            top: PaintColor::new(1.0, 0.0, 0.0, 1.0),
            ..PaintBorderColors::all(PaintColor::TRANSPARENT)
        },
        styles: PaintBorderStyles::default(),
        radii: PaintCornerRadii {
            top_left: large_radius,
            bottom_right: large_radius,
            ..PaintCornerRadii::default()
        },
        transform: PaintTransform2D::IDENTITY,
    });
    let image = raster_snapshot(&snapshot).expect("opposing rounded corners should rasterize");
    assert_eq!(pixel(&image, 85, 2), [255, 0, 0, 255]);
    assert_eq!(pixel(&image, 75, 68), [255; 4]);
}

#[test]
fn asymmetric_rounded_border_stays_inside_the_outer_contour() {
    for reflected in [false, true] {
        let mut snapshot = snapshot(100, 100, 1.0);
        let transform = if reflected {
            PaintTransform2D::new([-1.0, 0.0, 0.0, 1.0, 100.0, 0.0])
        } else {
            PaintTransform2D::IDENTITY
        };
        snapshot.push_fragment(PaintFragment::Border {
            rect: PaintRect::new(0.0, 0.0, 100.0, 100.0),
            widths: PaintEdgeSizes::new(1.0, 50.0, 1.0, 1.0),
            colors: PaintBorderColors::all(PaintColor::new(1.0, 0.0, 0.0, 1.0)),
            styles: PaintBorderStyles::default(),
            radii: PaintCornerRadii {
                top_left: PaintCornerRadius::new(100.0, 100.0),
                ..PaintCornerRadii::default()
            },
            transform,
        });
        let image = raster_snapshot(&snapshot).expect("asymmetric rounded border should rasterize");
        let sample = |x, y| pixel(&image, if reflected { 99 - x } else { x }, y);
        assert_eq!(sample(80, 35), [255, 0, 0, 255]);
        assert_eq!(sample(30, 20), [255; 4]);
        assert_eq!(sample(15, 50), [255; 4]);
    }
}

#[test]
fn rounded_border_patterns_stay_inside_the_outer_contour() {
    let red = PaintColor::new(1.0, 0.0, 0.0, 1.0);
    let transparent = PaintBorderColors::all(PaintColor::TRANSPARENT);
    for (style, widths, colors, outside_pixel) in [
        (
            PaintBorderStyle::Dashed,
            PaintEdgeSizes::new(12.0, 1.0, 1.0, 1.0),
            PaintBorderColors {
                top: red,
                ..transparent
            },
            (300, 58),
        ),
        (
            PaintBorderStyle::Dotted,
            PaintEdgeSizes::new(1.0, 1.0, 1.0, 12.0),
            PaintBorderColors {
                left: red,
                ..transparent
            },
            (64, 30),
        ),
    ] {
        let mut snapshot = snapshot(168, 116, 2.0);
        snapshot.push_fragment(PaintFragment::Border {
            rect: PaintRect::new(12.0, 12.0, 144.0, 92.0),
            widths,
            colors,
            styles: PaintBorderStyles::all(style),
            radii: PaintCornerRadii::all(PaintCornerRadius::new(48.0, 48.0)),
            transform: PaintTransform2D::IDENTITY,
        });
        let image = raster_snapshot(&snapshot).expect("rounded pattern should rasterize");
        assert!(
            image
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[1] < 100)
        );
        assert_eq!(pixel(&image, outside_pixel.0, outside_pixel.1), [255; 4]);
    }
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
    assert_eq!(validate_resource_budget(dimensions, 0), Ok(()));
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
fn deep_balanced_clip_scopes_pass_snapshot_validation() {
    let mut snapshot = snapshot(1, 1, 1.0);
    for _ in 0..4_097 {
        snapshot.push_fragment(PaintFragment::push_clip(PaintRect::new(0.0, 0.0, 1.0, 1.0)));
    }
    for _ in 0..4_097 {
        snapshot.push_fragment(PaintFragment::PopLayer);
    }

    assert_eq!(validate_fragment_stream(&snapshot), Ok(0));
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
    snapshot.push_fragment(PaintFragment::Tiles(Box::new(moli_layout::PaintTiles {
        source: moli_layout::PaintTileSource::Image(image, PaintImageSampling::Nearest),
        source_rect: PaintRect::new(0.0, 0.0, 2.0, 1.0),
        grid: moli_layout::PaintTileGrid {
            x: moli_layout::PaintTileAxis::new(-0.5, 3.5, 3.5, 7),
            y: moli_layout::PaintTileAxis::new(0.0, 2.0, 2.0, 3),
            coverage: PaintRect::new(0.0, 0.0, 21.0, 6.0),
            snap_tiles: false,
        },
        transform: PaintTransform2D::IDENTITY,
    })));
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
fn source_rectangle_clamps_linear_sampling_across_a_spaced_tile_grid() {
    use moli_layout::{PaintTileAxis, PaintTileGrid, PaintTileSource, PaintTiles};

    let mut snapshot = snapshot(28, 9, 1.0);
    let image = snapshot.add_image(Arc::new(
        moli_image::RgbaImage::try_new(
            4,
            1,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 0, 0, 255,
            ],
        )
        .unwrap(),
    ));
    snapshot.push_fragment(PaintFragment::Tiles(Box::new(PaintTiles {
        source: PaintTileSource::Image(image, PaintImageSampling::Linear),
        source_rect: PaintRect::new(1.25, 0.0, 1.5, 1.0),
        grid: PaintTileGrid {
            x: PaintTileAxis::new(0.0, 8.0, 11.0, 3),
            y: PaintTileAxis::new(2.0, 4.0, 0.0, 1),
            coverage: PaintRect::new(1.0, 1.0, 24.0, 7.0),
            snap_tiles: true,
        },
        transform: PaintTransform2D::translation(1.0, 1.0),
    })));
    let result = raster_snapshot(&snapshot).unwrap();
    assert_eq!(pixel(&result, 2, 3), [0, 247, 8, 255]);
    for x in [12, 23] {
        assert_eq!(pixel(&result, x, 3), [0, 255, 0, 255]);
    }
    for x in [8, 19] {
        assert_eq!(pixel(&result, x, 6), [0, 0, 255, 255]);
    }
    for (x, y) in [(1, 3), (9, 3), (11, 3), (20, 6), (26, 3), (3, 7)] {
        assert_eq!(pixel(&result, x, y), [255; 4], "gap or clip at ({x}, {y})");
    }
    for x in 2..=8 {
        assert_eq!(pixel(&result, x, 3)[0], 0, "neighbor texel at {x}");
    }
    assert_eq!(snapshot.images.len(), 1);
}

#[test]
fn spaced_image_tiles_share_the_fractional_device_grid() {
    use moli_layout::{PaintTileAxis, PaintTileGrid, PaintTileSource, PaintTiles};

    let mut snapshot = snapshot(32, 16, 1.25);
    let image = snapshot.add_image(Arc::new(
        moli_image::RgbaImage::try_new(1, 1, vec![0, 0, 255, 255]).unwrap(),
    ));
    snapshot.push_fragment(PaintFragment::Tiles(Box::new(PaintTiles {
        source: PaintTileSource::Image(image, PaintImageSampling::Nearest),
        source_rect: PaintRect::new(0.0, 0.0, 1.0, 1.0),
        grid: PaintTileGrid {
            x: PaintTileAxis::new(0.9375, 4.0, 5.625, 3),
            y: PaintTileAxis::new(3.25, 4.0, 0.0, 1),
            coverage: PaintRect::new(0.0, 0.0, 32.0, 16.0),
            snap_tiles: true,
        },
        transform: PaintTransform2D::IDENTITY,
    })));
    let rendered = raster_snapshot(&snapshot).unwrap();
    for y in 0..rendered.height {
        for x in 0..rendered.width {
            let expected = if (4..9).contains(&y)
                && ((1..6).contains(&x) || (8..13).contains(&x) || (15..20).contains(&x))
            {
                [0, 0, 255, 255]
            } else {
                [255; 4]
            };
            assert_eq!(
                pixel(&rendered, x, y),
                expected,
                "tile or gap at ({x}, {y})"
            );
        }
    }
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
fn transformed_svg_text_matches_equivalent_authored_coordinates() {
    let render = |content: &str| {
        let source = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="60" font-family="DejaVu Sans" font-size="18">{content}</svg>"#
        );
        let svg = Arc::new(moli_image::decode_svg_image(source.as_bytes()).unwrap());
        let mut snapshot = snapshot(100, 60, 1.0);
        let image = snapshot.add_svg_image(svg);
        snapshot.push_fragment(PaintFragment::SvgImage(PaintSvgImage {
            image,
            destination: PaintRect::new(0.0, 0.0, 100.0, 60.0),
            transform: PaintTransform2D::IDENTITY,
        }));
        raster_snapshot(&snapshot).unwrap()
    };
    let grouped = render(
        r#"<g transform="translate(10 8)"><g transform="translate(10 7)"><text x="5" y="20">SVG</text></g></g>"#,
    );
    let positioned = render(r#"<text x="25" y="35">SVG</text>"#);
    assert!(
        positioned
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[0] < 128)
            .count()
            > 20,
        "the reference must contain glyph pixels"
    );
    assert_eq!(
        grouped.rgba, positioned.rgba,
        "flattened glyphs must apply their absolute ancestor transform exactly once"
    );
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

#[cfg(feature = "molio")]
#[test]
fn raster_damage_repaints_changed_image_rows_with_exact_pixels() {
    use moli_layout::{PaintTileAxis, PaintTileGrid, PaintTileSource, PaintTiles};

    let mut first = snapshot(96, 120, 1.25);
    first.surface.device_scale = 2.0;
    first.push_fragment(PaintFragment::solid_rect(
        PaintRect::new(0.0, 0.0, 96.0, 120.0),
        PaintColor::new(0.1, 0.3, 0.5, 1.0),
    ));
    let image = first.add_image(Arc::new(
        moli_image::RgbaImage::try_new(1, 1, vec![255, 0, 0, 170]).unwrap(),
    ));
    first.push_fragment(PaintFragment::Image(PaintImage {
        image,
        destination: PaintRect::new(3.0, 30.5, 80.0, 30.0),
        sampling: PaintImageSampling::Linear,
        transform: PaintTransform2D::IDENTITY,
    }));
    first.push_fragment(PaintFragment::Tiles(Box::new(PaintTiles {
        source: PaintTileSource::Image(image, PaintImageSampling::Linear),
        source_rect: PaintRect::new(0.0, 0.0, 1.0, 1.0),
        grid: PaintTileGrid {
            x: PaintTileAxis::new(5.25, 18.5, 25.0, 3),
            y: PaintTileAxis::new(81.5, 6.9375, 0.0, 1),
            coverage: PaintRect::new(0.0, 81.0, 96.0, 10.0),
            snap_tiles: true,
        },
        transform: PaintTransform2D::IDENTITY,
    })));
    first.push_fragment(PaintFragment::solid_rect(
        PaintRect::new(7.0, 45.0, 6.5, 40.0),
        PaintColor::new(1.0, 1.0, 0.0, 0.3),
    ));
    let mut second = first.clone();
    second.images[image.index()].image =
        Arc::new(moli_image::RgbaImage::try_new(1, 1, vec![0, 255, 0, 180]).unwrap());
    let layout = raster_buffer_layout(96.0, 120.0, 2.0).unwrap();
    let mut different_dpr = first.clone();
    different_dpr.viewport.device_pixel_ratio = 2.0;
    assert_eq!(
        raster_damage_rows(&first, &different_dpr, layout),
        0..layout.height
    );
    let rows = raster_damage_rows(&first, &second, layout);
    assert_eq!(rows, 58..180);
    let mut actual = vec![0; layout.byte_len];
    raster_snapshot_into(&first, &mut actual, RasterPixelFormat::Bgra8).unwrap();
    let mut expected = vec![0; layout.byte_len];
    raster_snapshot_into(&second, &mut expected, RasterPixelFormat::Bgra8).unwrap();
    assert!(
        actual != expected,
        "the replacement image must change pixels"
    );
    raster_snapshot_rows_into(&second, &mut actual, RasterPixelFormat::Bgra8, rows).unwrap();
    let first_difference = actual
        .as_chunks::<4>()
        .0
        .iter()
        .zip(expected.as_chunks::<4>().0)
        .position(|(actual, expected)| actual != expected);
    assert_eq!(first_difference, None, "first mismatched BGRA pixel");
}
