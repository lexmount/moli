mod geometry;
mod helpers;
mod length;
mod matrix;
mod path;
mod transform;

pub use geometry::{
    SvgGeometryBox, SvgGeometryElement, SvgGeometryPoint, SvgGeometrySegment,
    bounding_box_for_element, bounding_box_for_segments, bounding_box_for_transformed_element,
    is_point_in_fill, point_at_length, segments_for_element,
};
pub use length::{
    SvgLength, SvgLengthUnit, parse_length, parse_length_list, parse_number, parse_number_list,
    parse_point_list,
};
pub use matrix::{SvgMatrixComponents, serialize_number};
pub use transform::{
    SvgTransform, SvgTransformKind, consolidate_transform_matrices, parse_transform_attribute,
    serialize_transform_list,
};

#[cfg(test)]
mod tests {
    use crate::{
        SvgGeometryElement, SvgGeometryPoint, SvgGeometrySegment, SvgLengthUnit,
        SvgMatrixComponents, SvgTransform, SvgTransformKind, bounding_box_for_element,
        bounding_box_for_segments, bounding_box_for_transformed_element,
        consolidate_transform_matrices, is_point_in_fill, parse_length, parse_length_list,
        parse_number, parse_number_list, parse_point_list, parse_transform_attribute,
        point_at_length, segments_for_element, serialize_number, serialize_transform_list,
    };

    fn path_segments(raw: &str) -> Vec<SvgGeometrySegment> {
        segments_for_element(SvgGeometryElement::Path { d: raw.to_owned() })
    }

    #[test]
    fn point_list_parser_rejects_invalid_tokens_and_truncates_incomplete_pairs() {
        assert_eq!(
            parse_point_list("0,0 100,0 100,100 0,100"),
            Some(vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)])
        );
        assert_eq!(
            parse_point_list("0,0 100,0 20"),
            Some(vec![(0.0, 0.0), (100.0, 0.0)])
        );
        assert_eq!(
            parse_point_list("0,0 100,0 20,"),
            Some(vec![(0.0, 0.0), (100.0, 0.0)])
        );
        assert_eq!(parse_point_list("0,0 100,0 INVALID"), None);
    }

    fn polyline_segments(raw: &str) -> Vec<SvgGeometrySegment> {
        segments_for_element(SvgGeometryElement::Polyline {
            points: raw.to_owned(),
        })
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {actual} to be close to {expected}"
        );
    }

    fn assert_near(actual: f64, expected: f64, epsilon: f64) {
        assert!(
            (actual - expected).abs() < epsilon,
            "expected {actual} to be within {epsilon} of {expected}"
        );
    }

    #[test]
    fn path_geometry_handles_line_commands_and_points() {
        let segments = path_segments("M 0 0 L 3 4 H 7 V 1");
        let total = segments.iter().map(|segment| segment.length()).sum::<f64>();
        assert_close(total, 12.0);

        let point = point_at_length(&segments, 7.0);
        assert_close(point.x, 5.0);
        assert_close(point.y, 4.0);

        let compact = path_segments("M10-20l30.1.5.1-20z");
        let compact_end = point_at_length(&compact, 10_000.0);
        assert_close(compact_end.x, 10.0);
        assert_close(compact_end.y, -20.0);
    }

    #[test]
    fn move_only_path_preserves_its_zero_length_point() {
        let segments = path_segments("M 1e2,-2E1");
        assert_eq!(segments.len(), 1);
        assert_close(segments[0].length(), 0.0);

        let point = point_at_length(&segments, 0.0);
        assert_close(point.x, 100.0);
        assert_close(point.y, -20.0);
    }

    #[test]
    fn path_geometry_preserves_complete_segments_before_a_parse_error() {
        let invalid_command = path_segments("M 10 10 L 30 10 X 50 10");
        assert_eq!(invalid_command.len(), 1);
        assert_close(invalid_command[0].length(), 20.0);

        let incomplete_curve = path_segments("M 10 10 L 30 10 C 40 0 50 20");
        assert_eq!(incomplete_curve.len(), 1);
        assert_close(incomplete_curve[0].length(), 20.0);
    }

    #[test]
    fn path_geometry_rejects_numbers_with_trailing_decimal_points() {
        let separate_command = path_segments("M 0 0 L 10 10 L 20.,30");
        assert_eq!(separate_command.len(), 1);
        let end = point_at_length(&separate_command, f64::INFINITY);
        assert_close(end.x, 10.0);
        assert_close(end.y, 10.0);

        let compound_command = path_segments("M 0 0 L 10 10 20.,30");
        assert_eq!(compound_command.len(), 1);
        let end = point_at_length(&compound_command, f64::INFINITY);
        assert_close(end.x, 10.0);
        assert_close(end.y, 10.0);

        let first_coordinate = path_segments("M 0 0 L 15. 20");
        assert_close(first_coordinate[0].length(), 0.0);
        let end = point_at_length(&first_coordinate, f64::INFINITY);
        assert_close(end.x, 0.0);
        assert_close(end.y, 0.0);
    }

    #[test]
    fn path_geometry_handles_relative_commands_and_close_path() {
        let segments = path_segments("m 1 1 l 3 0 v 4 h -3 z");
        let total = segments.iter().map(|segment| segment.length()).sum::<f64>();
        assert_close(total, 14.0);

        let point = point_at_length(&segments, 12.0);
        assert_close(point.x, 1.0);
        assert_close(point.y, 3.0);
    }

    #[test]
    fn path_geometry_preserves_cubic_and_quadratic_curves() {
        let cubic = path_segments("M 0 0 C 0 10 10 10 10 0");
        assert!(cubic.iter().map(|segment| segment.length()).sum::<f64>() > 19.0);
        let cubic_end = point_at_length(&cubic, 10_000.0);
        assert_close(cubic_end.x, 10.0);
        assert_close(cubic_end.y, 0.0);

        let quadratic = path_segments("M 0 0 Q 5 10 10 0 T 20 0");
        assert!(
            quadratic
                .iter()
                .map(|segment| segment.length())
                .sum::<f64>()
                > 22.0
        );
        let quadratic_end = point_at_length(&quadratic, 10_000.0);
        assert_close(quadratic_end.x, 20.0);
        assert_close(quadratic_end.y, 0.0);
    }

    #[test]
    fn path_geometry_converts_svg_arcs_to_bezier_curves() {
        let arc = path_segments("M 0 0 A 10 10 0 0 1 20 0");
        let total = arc.iter().map(|segment| segment.length()).sum::<f64>();
        assert_near(total, std::f64::consts::PI * 10.0, 0.1);
        let arc_end = point_at_length(&arc, 10_000.0);
        assert_close(arc_end.x, 20.0);
        assert_close(arc_end.y, 0.0);

        let relative_arc = path_segments("M 10 10 a 5 5 0 0 0 10 0");
        let relative_end = point_at_length(&relative_arc, 10_000.0);
        assert_close(relative_end.x, 20.0);
        assert_close(relative_end.y, 10.0);
    }

    #[test]
    fn bounding_box_covers_shape_and_path_segments() {
        let rect = segments_for_element(SvgGeometryElement::Rect {
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 40.0,
            rx: 0.0,
            ry: 0.0,
        });
        let rect_box = bounding_box_for_segments(&rect).unwrap();
        assert_close(rect_box.x, 10.0);
        assert_close(rect_box.y, 20.0);
        assert_close(rect_box.width, 30.0);
        assert_close(rect_box.height, 40.0);

        let path = path_segments("M 0 0 L 3 4 H 7 V 1");
        let path_box = bounding_box_for_segments(&path).unwrap();
        assert_close(path_box.x, 0.0);
        assert_close(path_box.y, 0.0);
        assert_close(path_box.width, 7.0);
        assert_close(path_box.height, 4.0);
    }

    #[test]
    fn element_bounding_box_preserves_move_to_only_geometry() {
        let path = SvgGeometryElement::Path {
            d: "M 40 20".to_owned(),
        };
        let bbox = bounding_box_for_element(&path).unwrap();
        assert_close(bbox.x, 40.0);
        assert_close(bbox.y, 20.0);
        assert_close(bbox.width, 0.0);
        assert_close(bbox.height, 0.0);

        let polyline = SvgGeometryElement::Polyline {
            points: "10,20".to_owned(),
        };
        let bbox = bounding_box_for_element(&polyline).unwrap();
        assert_close(bbox.x, 10.0);
        assert_close(bbox.y, 20.0);
        assert_close(bbox.width, 0.0);
        assert_close(bbox.height, 0.0);
    }

    #[test]
    fn transformed_element_bounding_box_uses_kurbo_path_geometry() {
        let path = SvgGeometryElement::Path {
            d: "M 0 0 L 10 20".to_owned(),
        };
        let bbox =
            bounding_box_for_transformed_element(&path, SvgMatrixComponents::translate(30.0, 40.0))
                .unwrap();
        assert_close(bbox.x, 30.0);
        assert_close(bbox.y, 40.0);
        assert_close(bbox.width, 10.0);
        assert_close(bbox.height, 20.0);
    }

    #[test]
    fn negative_ellipse_radius_falls_back_to_the_valid_radius() {
        let rx_invalid = bounding_box_for_element(&SvgGeometryElement::Ellipse {
            cx: 1.0,
            cy: 12.0,
            rx: -5.0,
            ry: 10.0,
        })
        .unwrap();
        assert_close(rx_invalid.x, -9.0);
        assert_close(rx_invalid.y, 2.0);
        assert_close(rx_invalid.width, 20.0);
        assert_close(rx_invalid.height, 20.0);

        let ry_invalid = bounding_box_for_element(&SvgGeometryElement::Ellipse {
            cx: 6.0,
            cy: 2.0,
            rx: 5.0,
            ry: -10.0,
        })
        .unwrap();
        assert_close(ry_invalid.x, 1.0);
        assert_close(ry_invalid.y, -3.0);
        assert_close(ry_invalid.width, 10.0);
        assert_close(ry_invalid.height, 10.0);
    }

    #[test]
    fn rounded_rect_geometry_uses_elliptical_corner_arcs() {
        let rect = segments_for_element(SvgGeometryElement::Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 300.0,
            rx: 50.0,
            ry: 50.0,
        });
        let length = rect.iter().map(SvgGeometrySegment::length).sum::<f64>();
        assert_near(length, 600.0 + std::f64::consts::PI * 100.0, 0.1);
        let bbox = bounding_box_for_segments(&rect).unwrap();
        assert_close(bbox.x, 0.0);
        assert_close(bbox.y, 0.0);
        assert_close(bbox.width, 200.0);
        assert_close(bbox.height, 300.0);

        let clamped = segments_for_element(SvgGeometryElement::Rect {
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 10.0,
            rx: 50.0,
            ry: 50.0,
        });
        let clamped_length = clamped.iter().map(SvgGeometrySegment::length).sum::<f64>();
        assert_near(clamped_length, 48.4, 0.2);
    }

    #[test]
    fn fill_containment_covers_basic_svg_geometry() {
        let rect = SvgGeometryElement::Rect {
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 40.0,
            rx: 0.0,
            ry: 0.0,
        };
        assert!(is_point_in_fill(&rect, SvgGeometryPoint::new(25.0, 30.0)));
        assert!(is_point_in_fill(&rect, SvgGeometryPoint::new(10.0, 30.0)));
        assert!(is_point_in_fill(&rect, SvgGeometryPoint::new(40.0, 30.0)));
        assert!(is_point_in_fill(&rect, SvgGeometryPoint::new(25.0, 20.0)));
        assert!(is_point_in_fill(&rect, SvgGeometryPoint::new(25.0, 60.0)));
        assert!(!is_point_in_fill(&rect, SvgGeometryPoint::new(0.0, 0.0)));

        let line = SvgGeometryElement::Line {
            x1: 0.0,
            y1: 0.0,
            x2: 10.0,
            y2: 10.0,
        };
        assert!(!is_point_in_fill(&line, SvgGeometryPoint::new(5.0, 5.0)));

        let rounded_rect = SvgGeometryElement::Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
            rx: 25.0,
            ry: 25.0,
        };
        assert!(is_point_in_fill(
            &rounded_rect,
            SvgGeometryPoint::new(50.0, 5.0)
        ));
        assert!(!is_point_in_fill(
            &rounded_rect,
            SvgGeometryPoint::new(5.0, 5.0)
        ));

        let circle = SvgGeometryElement::Circle {
            cx: 5.0,
            cy: 6.0,
            r: 10.0,
        };
        assert!(is_point_in_fill(&circle, SvgGeometryPoint::new(5.0, 6.0)));
        assert!(!is_point_in_fill(&circle, SvgGeometryPoint::new(20.0, 6.0)));

        let polygon = SvgGeometryElement::Polygon {
            points: "0 0 10 0 10 10 0 10".to_owned(),
        };
        assert!(is_point_in_fill(&polygon, SvgGeometryPoint::new(4.0, 4.0)));
        assert!(!is_point_in_fill(
            &polygon,
            SvgGeometryPoint::new(12.0, 4.0)
        ));

        let path = SvgGeometryElement::Path {
            d: "M 0 0 L 10 0 L 10 10 Z".to_owned(),
        };
        assert!(is_point_in_fill(&path, SvgGeometryPoint::new(8.0, 2.0)));
        assert!(!is_point_in_fill(&path, SvgGeometryPoint::new(2.0, 8.0)));
    }

    #[test]
    fn fill_containment_uses_nonzero_winding_and_closes_each_subpath() {
        let same_direction_rings = SvgGeometryElement::Path {
            d: "M0 0H10V10H0Z M2 2H8V8H2Z".to_owned(),
        };
        assert!(is_point_in_fill(
            &same_direction_rings,
            SvgGeometryPoint::new(5.0, 5.0)
        ));

        let open_subpaths = SvgGeometryElement::Path {
            d: "M0 0L10 0L10 10 M20 0L30 0L30 10".to_owned(),
        };
        assert!(is_point_in_fill(
            &open_subpaths,
            SvgGeometryPoint::new(8.0, 2.0)
        ));
        assert!(is_point_in_fill(
            &open_subpaths,
            SvgGeometryPoint::new(28.0, 2.0)
        ));
    }

    #[test]
    fn curve_bounding_box_uses_bezier_extrema() {
        let path = path_segments("M0 0 C100 100 100-100 200 0");
        let bounds = bounding_box_for_segments(&path).unwrap();
        let y_extent = 50.0 / 3.0 * 3.0_f64.sqrt();
        assert_near(bounds.x, 0.0, 1e-9);
        assert_near(bounds.y, -y_extent, 1e-9);
        assert_near(bounds.width, 200.0, 1e-9);
        assert_near(bounds.height, y_extent * 2.0, 1e-9);
    }

    #[test]
    fn poly_points_geometry_rejects_odd_coordinates() {
        assert!(polyline_segments("0 0 10").is_empty());
        let compact = polyline_segments("0,0 10-5 20.5.5");
        assert_eq!(compact.len(), 2);
        let compact_end = point_at_length(&compact, 10_000.0);
        assert_close(compact_end.x, 20.5);
        assert_close(compact_end.y, 0.5);
    }

    #[test]
    fn svg_scalar_and_list_parsers_use_svgtypes_boundaries() {
        let length = parse_length(" 1.5em ").unwrap();
        assert_close(length.value, 1.5);
        assert_eq!(length.unit, SvgLengthUnit::Ems);
        assert_eq!(length.serialize(), "1.5em");
        assert_eq!(parse_length("1pX").unwrap().unit, SvgLengthUnit::Px);
        assert_eq!(serialize_number(3.0), "3");
        assert_eq!(serialize_number(3.25), "3.25");
        assert!(parse_length("1 px").is_none());

        let lengths = parse_length_list("1 2px,3% 4-5").unwrap();
        assert_eq!(lengths.len(), 5);
        assert_close(lengths[3].value, 4.0);
        assert_eq!(lengths[3].unit, SvgLengthUnit::Number);
        assert_close(lengths[4].value, -5.0);
        assert!(parse_length_list("1px nope 2px").is_none());

        let modern = parse_length_list("10cap, 2rlh 3vw-4q").unwrap();
        assert_eq!(modern.len(), 4);
        assert_eq!(modern[0].unit, SvgLengthUnit::Cap);
        assert_eq!(modern[1].unit, SvgLengthUnit::Rlh);
        assert_eq!(modern[2].unit, SvgLengthUnit::Vw);
        assert_close(modern[3].value, -4.0);
        assert_eq!(modern[3].unit, SvgLengthUnit::Q);
        assert_eq!(modern[0].serialize(), "10cap");
        assert_eq!(parse_length_list(" 10cap ").unwrap().len(), 1);
        assert!(parse_length_list("10cap,").is_none());
        assert!(parse_length_list("10cap nope").is_none());

        assert_close(parse_number(" .5 ").unwrap(), 0.5);
        assert_eq!(
            parse_number_list("10 20,30-40").unwrap(),
            vec![10.0, 20.0, 30.0, -40.0]
        );
        assert!(parse_number_list("10 nope 20").is_none());
    }

    #[test]
    fn circle_and_ellipse_geometry_are_bezier_paths() {
        let circle = segments_for_element(SvgGeometryElement::Circle {
            cx: 0.0,
            cy: 0.0,
            r: 10.0,
        });
        let circle_length = circle.iter().map(|segment| segment.length()).sum::<f64>();
        assert_near(circle_length, std::f64::consts::TAU * 10.0, 0.1);
        let start = point_at_length(&circle, 0.0);
        assert_close(start.x, 10.0);
        assert_close(start.y, 0.0);

        let ellipse = segments_for_element(SvgGeometryElement::Ellipse {
            cx: 5.0,
            cy: 6.0,
            rx: 10.0,
            ry: 4.0,
        });
        assert!(ellipse.iter().map(|segment| segment.length()).sum::<f64>() > 44.0);
    }

    #[test]
    fn svg_transform_attribute_parses_affine_css_and_svg_forms() {
        let transforms =
            parse_transform_attribute("translate(10 20) scale(2, 3) rotate(90 5 5) skewX(45)")
                .unwrap();

        assert_eq!(transforms.len(), 4);
        assert_eq!(transforms[0].kind, SvgTransformKind::Translate);
        assert_close(transforms[0].matrix.e, 10.0);
        assert_close(transforms[0].matrix.f, 20.0);
        assert_eq!(transforms[1].kind, SvgTransformKind::Scale);
        assert_close(transforms[1].matrix.a, 2.0);
        assert_close(transforms[1].matrix.d, 3.0);
        assert_eq!(transforms[2].kind, SvgTransformKind::Rotate);
        assert_close(transforms[2].angle, 90.0);
        assert_eq!(transforms[3].kind, SvgTransformKind::SkewX);
    }

    #[test]
    fn svg_transform_constructors_set_kind_angle_and_matrix() {
        let matrix = SvgTransform::matrix(SvgMatrixComponents::translate(1.0, 2.0));
        assert_eq!(matrix.kind, SvgTransformKind::Matrix);
        assert_close(matrix.angle, 0.0);
        assert_eq!(matrix.matrix, SvgMatrixComponents::translate(1.0, 2.0));

        let translate = SvgTransform::translate(10.0, 20.0);
        assert_eq!(translate.kind, SvgTransformKind::Translate);
        assert_eq!(translate.matrix, SvgMatrixComponents::translate(10.0, 20.0));

        let scale = SvgTransform::scale(2.0, 3.0);
        assert_eq!(scale.kind, SvgTransformKind::Scale);
        assert_eq!(scale.matrix, SvgMatrixComponents::scale(2.0, 3.0));

        let rotate = SvgTransform::rotate(45.0, 5.0, 6.0);
        assert_eq!(rotate.kind, SvgTransformKind::Rotate);
        assert_close(rotate.angle, 45.0);
        assert_eq!(
            rotate.matrix,
            SvgMatrixComponents::rotate_around(45.0, 5.0, 6.0)
        );

        let skew_x = SvgTransform::skew_x(15.0);
        assert_eq!(skew_x.kind, SvgTransformKind::SkewX);
        assert_close(skew_x.angle, 15.0);
        assert_eq!(skew_x.matrix, SvgMatrixComponents::skew_x(15.0));

        let skew_y = SvgTransform::skew_y(20.0);
        assert_eq!(skew_y.kind, SvgTransformKind::SkewY);
        assert_close(skew_y.angle, 20.0);
        assert_eq!(skew_y.matrix, SvgMatrixComponents::skew_y(20.0));
    }

    #[test]
    fn svg_transform_attribute_rejects_non_affine_3d_tail() {
        assert!(parse_transform_attribute("translate3d(1px, 2px, 3px)").is_none());
        assert!(parse_transform_attribute("scale3d(1, 2, 3)").is_none());
        assert!(parse_transform_attribute("matrix(1 0 0 1 0 0,)").is_none());
    }

    #[test]
    fn svg_matrix_components_multiply_and_inverse_round_trip() {
        let matrix = SvgMatrixComponents::translate(10.0, 20.0)
            .multiply(SvgMatrixComponents::scale(2.0, 3.0))
            .multiply(SvgMatrixComponents::rotate(30.0));
        assert!(matrix.is_invertible());
        assert!(matrix.serialize_transform_matrix().starts_with("matrix("));

        let consolidated = consolidate_transform_matrices([
            SvgMatrixComponents::translate(10.0, 20.0),
            SvgMatrixComponents::scale(2.0, 3.0),
        ])
        .unwrap();
        assert_eq!(
            consolidated,
            SvgMatrixComponents::translate(10.0, 20.0)
                .multiply(SvgMatrixComponents::scale(2.0, 3.0))
        );
        assert!(consolidate_transform_matrices([]).is_none());
        assert_eq!(
            serialize_transform_list(&[
                SvgMatrixComponents::translate(10.0, 20.0),
                SvgMatrixComponents::scale(2.0, 3.0),
            ]),
            "matrix(1 0 0 1 10 20) matrix(2 0 0 3 0 0)"
        );

        let product = matrix.multiply(matrix.inverse());

        assert_close(product.a, 1.0);
        assert_close(product.b, 0.0);
        assert_close(product.c, 0.0);
        assert_close(product.d, 1.0);
        assert_close(product.e, 0.0);
        assert_close(product.f, 0.0);
    }

    #[test]
    fn svg_matrix_components_affine_operation_helpers_post_multiply() {
        let current = SvgMatrixComponents::translate(10.0, 20.0);
        assert_eq!(
            current.then_translate(3.0, 4.0),
            current.multiply(SvgMatrixComponents::translate(3.0, 4.0))
        );
        assert_eq!(
            current.then_scale(2.0),
            current.multiply(SvgMatrixComponents::scale(2.0, 2.0))
        );
        assert_eq!(
            current.then_scale_non_uniform(2.0, 3.0),
            current.multiply(SvgMatrixComponents::scale(2.0, 3.0))
        );
        assert_eq!(
            current.then_flip_x(),
            current.multiply(SvgMatrixComponents::scale(-1.0, 1.0))
        );
        assert_eq!(
            current.then_flip_y(),
            current.multiply(SvgMatrixComponents::scale(1.0, -1.0))
        );
        assert_eq!(
            current.then_skew_x(15.0),
            current.multiply(SvgMatrixComponents::skew_x(15.0))
        );
        assert_eq!(
            current.then_skew_y(20.0),
            current.multiply(SvgMatrixComponents::skew_y(20.0))
        );
        assert!(current.then_rotate_from_vector(0.0, 1.0).is_none());

        let rotated = current.then_rotate_from_vector(1.0, 1.0).unwrap();
        assert_eq!(rotated, current.multiply(SvgMatrixComponents::rotate(45.0)));
    }

    #[test]
    fn svg_matrix_components_treat_non_finite_values_as_non_invertible() {
        let matrix = SvgMatrixComponents {
            a: f64::NAN,
            ..SvgMatrixComponents::identity()
        };

        assert!(!matrix.has_finite_components());
        assert!(!matrix.is_invertible());
        assert!(matrix.inverse().a.is_nan());
    }
}
