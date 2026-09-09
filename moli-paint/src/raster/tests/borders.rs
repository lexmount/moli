use super::*;

#[test]
fn solid_border_ring_has_continuous_coverage_without_repeated_alpha() {
    // Compare against non-overlapping, pixel-aligned rectangles. They have no
    // diagonal joins and composite each border pixel exactly once.
    for widths in [
        PaintEdgeSizes::new(0.0, 0.0, 0.0, 0.0),
        PaintEdgeSizes::new(4.0, 4.0, 4.0, 4.0),
        PaintEdgeSizes::new(2.0, 10.0, 2.0, 10.0),
        PaintEdgeSizes::new(10.0, 2.0, 10.0, 2.0),
        PaintEdgeSizes::new(3.0, 7.0, 5.0, 9.0),
        PaintEdgeSizes::new(0.0, 7.0, 5.0, 9.0),
        PaintEdgeSizes::new(3.0, 0.0, 5.0, 9.0),
        PaintEdgeSizes::new(3.0, 7.0, 0.0, 9.0),
        PaintEdgeSizes::new(3.0, 7.0, 5.0, 0.0),
        PaintEdgeSizes::new(16.0, 20.0, 16.0, 20.0),
        PaintEdgeSizes::new(32.0, 0.0, 0.0, 0.0),
    ] {
        for alpha in [1.0, 0.5] {
            for scale in [1.0, 2.0] {
                let color = PaintColor::new(0.0, 0.0, 1.0, alpha);
                let background = PaintColor::new(1.0, 0.0, 0.0, 1.0);
                let viewport = PaintViewport::new(56, 48, scale);
                let mut actual = PaintSnapshot::new(viewport, background);
                actual.push_fragment(PaintFragment::border(
                    // The outer and inner edges snap before device scaling.
                    PaintRect::new(8.25, 8.25, 40.0, 32.0),
                    widths,
                    PaintBorderColors::all(color),
                ));
                let mut reference = PaintSnapshot::new(viewport, background);
                for rect in [
                    PaintRect::new(8.0, 8.0, 40.0, widths.top),
                    PaintRect::new(8.0, 40.0 - widths.bottom, 40.0, widths.bottom),
                    PaintRect::new(
                        8.0,
                        8.0 + widths.top,
                        widths.left,
                        32.0 - widths.top - widths.bottom,
                    ),
                    PaintRect::new(
                        48.0 - widths.right,
                        8.0 + widths.top,
                        widths.right,
                        32.0 - widths.top - widths.bottom,
                    ),
                ] {
                    reference.push_fragment(PaintFragment::solid_rect(rect, color));
                }
                let actual = raster_snapshot(&actual).expect("solid border should rasterize");
                let reference = raster_snapshot(&reference).expect("reference should rasterize");
                for y in 0..actual.height {
                    for x in 0..actual.width {
                        assert_eq!(
                            pixel(&actual, x, y),
                            pixel(&reference, x, y),
                            "border coverage at ({x}, {y}), widths={widths:?}, alpha={alpha}, scale={scale}",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn solid_elliptical_border_joins_preserve_coverage_and_the_inner_hole() {
    for radii in [
        PaintCornerRadii::all(PaintCornerRadius::new(8.0, 6.0)),
        PaintCornerRadii {
            top_left: PaintCornerRadius::new(4.0, 9.0),
            top_right: PaintCornerRadius::new(10.0, 4.0),
            bottom_right: PaintCornerRadius::new(9.0, 5.0),
            bottom_left: PaintCornerRadius::new(6.0, 8.0),
        },
    ] {
        for (widths, joins) in [
            (
                PaintEdgeSizes::new(4.0, 4.0, 4.0, 4.0),
                [(3, 3), (76, 3), (76, 60), (3, 60)],
            ),
            (
                PaintEdgeSizes::new(4.0, 12.0, 6.0, 10.0),
                [(5, 2), (73, 2), (73, 61), (4, 61)],
            ),
        ] {
            for alpha in [1.0, 0.5] {
                for scale in [1, 2] {
                    let background = PaintColor::new(1.0, 0.0, 0.0, 1.0);
                    let color = PaintColor::new(0.0, 0.0, 1.0, alpha);
                    let viewport = PaintViewport::new(96, 80, scale as f32);
                    let mut snapshot = PaintSnapshot::new(viewport, background);
                    snapshot.push_fragment(PaintFragment::Border {
                        rect: PaintRect::new(8.0, 8.0, 80.0, 64.0),
                        widths,
                        colors: PaintBorderColors::all(color),
                        styles: PaintBorderStyles::all(PaintBorderStyle::Solid),
                        radii,
                        transform: PaintTransform2D::IDENTITY,
                    });
                    let image = raster_snapshot(&snapshot).expect("elliptical border");
                    // Use a single fill to obtain the backend's exact alpha
                    // quantization, independently of all border geometry.
                    let mut reference = PaintSnapshot::new(viewport, background);
                    reference.push_fragment(PaintFragment::solid_rect(
                        PaintRect::new(0.0, 0.0, 4.0, 4.0),
                        color,
                    ));
                    let reference = raster_snapshot(&reference).expect("single color fill");
                    let expected = pixel(&reference, 0, 0);
                    for (x, y) in joins {
                        for dy in 0..scale {
                            for dx in 0..scale {
                                assert_eq!(
                                    pixel(&image, (8 + x) * scale + dx, (8 + y) * scale + dy),
                                    expected,
                                    "join ({x}, {y}), radii={radii:?}, widths={widths:?}, alpha={alpha}, scale={scale}",
                                );
                            }
                        }
                    }
                    for (x, y) in [(8, 8), (87, 8), (87, 71), (8, 71), (48, 40)] {
                        assert_eq!(
                            pixel(&image, x * scale, y * scale),
                            [255, 0, 0, 255],
                            "outside the ring at ({x}, {y}), radii={radii:?}",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn elliptical_border_uses_inset_radii_instead_of_a_normal_offset_stroke() {
    for alpha in [1.0, 0.5] {
        let mut snapshot = PaintSnapshot::new(
            PaintViewport::new(300, 120, 1.0),
            PaintColor::new(1.0, 0.0, 0.0, 1.0),
        );
        snapshot.push_fragment(PaintFragment::Border {
            rect: PaintRect::new(20.0, 20.0, 240.0, 80.0),
            widths: PaintEdgeSizes::new(16.0, 16.0, 16.0, 16.0),
            colors: PaintBorderColors::all(PaintColor::new(0.0, 0.0, 1.0, alpha)),
            styles: PaintBorderStyles::all(PaintBorderStyle::Solid),
            radii: PaintCornerRadii::all(PaintCornerRadius::new(120.0, 40.0)),
            transform: PaintTransform2D::IDENTITY,
        });
        let image = raster_snapshot(&snapshot).expect("elliptical border");
        // Chromium's inner ellipse has radii 104/24. A 16px stroke around
        // the 112/32 centerline ellipse wrongly paints these interior pixels.
        for (x, y) in [(56, 46), (223, 46), (56, 73), (223, 73)] {
            assert_eq!(
                pixel(&image, x, y),
                [255, 0, 0, 255],
                "inner ellipse ({x}, {y})"
            );
        }
    }
}

#[test]
fn mixed_border_edges_preserve_color_style_and_visibility() {
    let blue = PaintColor::new(0.0, 0.0, 1.0, 1.0);
    let green = PaintColor::new(0.0, 1.0, 0.0, 1.0);
    for (color, style, expected) in [
        (green, PaintBorderStyle::Solid, [0, 255, 0, 255]),
        (blue, PaintBorderStyle::None, [255, 0, 0, 255]),
        (blue, PaintBorderStyle::Hidden, [255, 0, 0, 255]),
        (
            PaintColor::TRANSPARENT,
            PaintBorderStyle::Solid,
            [255, 0, 0, 255],
        ),
        (blue, PaintBorderStyle::Double, [255, 0, 0, 255]),
    ] {
        let mut snapshot = PaintSnapshot::new(
            PaintViewport::new(96, 80, 1.0),
            PaintColor::new(1.0, 0.0, 0.0, 1.0),
        );
        snapshot.push_fragment(PaintFragment::Border {
            rect: PaintRect::new(8.0, 8.0, 80.0, 64.0),
            widths: PaintEdgeSizes::new(6.0, 12.0, 6.0, 10.0),
            colors: PaintBorderColors {
                top: color,
                ..PaintBorderColors::all(blue)
            },
            styles: PaintBorderStyles {
                top: style,
                ..PaintBorderStyles::all(PaintBorderStyle::Solid)
            },
            radii: PaintCornerRadii::ZERO,
            transform: PaintTransform2D::IDENTITY,
        });
        let image = raster_snapshot(&snapshot).expect("mixed border");
        assert_eq!(
            pixel(&image, 48, 11),
            expected,
            "top style={style:?}, color={color:?}"
        );
        for (x, y) in [(10, 40), (84, 40), (48, 69)] {
            assert_eq!(pixel(&image, x, y), [0, 0, 255, 255]);
        }
        if style == PaintBorderStyle::Double {
            assert_eq!(pixel(&image, 48, 9), [0, 0, 255, 255]);
            assert_eq!(pixel(&image, 48, 13), [0, 0, 255, 255]);
        }
    }
}
