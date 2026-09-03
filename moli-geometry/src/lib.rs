//! Renderer-neutral geometry primitives shared by Web-facing bindings.

mod css_parse;
mod matrix;

pub use css_parse::{
    dom_matrix_components_from_values, parse_dom_matrix_value,
    parse_dom_matrix_value_with_dimension,
};
pub use matrix::{DOM_MATRIX_COMPONENT_COUNT, DomMatrixComponents};

#[cfg(test)]
mod tests {
    use super::{
        DomMatrixComponents, parse_dom_matrix_value, parse_dom_matrix_value_with_dimension,
    };

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {actual} to be close to {expected}"
        );
    }

    #[test]
    fn parses_css_transform_list_into_product_matrix() {
        let matrix = parse_dom_matrix_value("translateX(10px) scale(2) rotate(90deg)").unwrap();

        assert_close(matrix.m11, 0.0);
        assert_close(matrix.m12, 2.0);
        assert_close(matrix.m21, -2.0);
        assert_close(matrix.m22, 0.0);
        assert_close(matrix.m41, 10.0);
    }

    #[test]
    fn parses_whitespace_separated_css_transform_arguments() {
        let matrix = parse_dom_matrix_value("translate(10px 20px) matrix(1 0 0 1 5 6)").unwrap();

        assert_close(matrix.m11, 1.0);
        assert_close(matrix.m22, 1.0);
        assert_close(matrix.m41, 15.0);
        assert_close(matrix.m42, 26.0);
    }

    #[test]
    fn serializes_z_axis_rotate_as_2d_matrix_without_axis_angle_drift() {
        let matrix = parse_dom_matrix_value("rotate(90rad)").unwrap();

        assert!(matrix.is_2d());
        assert!(matrix.css_text().unwrap().starts_with("matrix("));
    }

    #[test]
    fn serializes_tiny_matrix_numbers_like_stylo_transform_matrices() {
        assert_eq!(
            parse_dom_matrix_value("rotate(90deg)")
                .unwrap()
                .css_text()
                .unwrap(),
            "matrix(0.0000000000000000612323, 1, -1, 0.0000000000000000612323, 0, 0)"
        );
    }

    #[test]
    fn dom_matrix_text_uses_ecmascript_number_serialization() {
        for (value, expected) in [
            (1.0 / 300_000_000.0, "3.3333333333333334e-9"),
            (f64::MAX, "1.7976931348623157e+308"),
            (f64::MIN_POSITIVE * f64::EPSILON, "5e-324"),
        ] {
            let matrix = DomMatrixComponents {
                m42: value,
                ..DomMatrixComponents::identity()
            };

            assert_eq!(
                matrix.dom_matrix_text_with_dimension(true).unwrap(),
                format!("matrix(1, 0, 0, 1, 0, {expected})")
            );
        }
    }

    #[test]
    fn inverse_handles_invertible_3d_matrix() {
        let matrix = DomMatrixComponents::identity()
            .translated(4.0, 5.0, 6.0)
            .scaled_3d(2.0, 3.0, 4.0);
        let product = matrix.multiply(matrix.inverse());

        assert_close(product.m11, 1.0);
        assert_close(product.m22, 1.0);
        assert_close(product.m33, 1.0);
        assert_close(product.m44, 1.0);
        assert_close(product.m41, 0.0);
        assert_close(product.m42, 0.0);
        assert_close(product.m43, 0.0);
    }

    #[test]
    fn inverse_product_preserves_exact_identity_for_affine_3d_matrix() {
        let matrix = DomMatrixComponents {
            m11: 1.0,
            m12: -0.5,
            m13: 0.5,
            m14: 0.0,
            m21: 0.5,
            m22: 2.0,
            m23: -0.5,
            m24: 0.0,
            m31: 0.0,
            m32: 0.0,
            m33: 1.0,
            m34: 0.0,
            m41: 10.0,
            m42: 20.0,
            m43: 10.0,
            m44: 1.0,
        };
        let product = matrix.multiply(matrix.inverse());

        assert!(product.is_identity(), "unexpected product: {product:?}");
    }

    #[test]
    fn inverse_preserves_exact_2d_structure() {
        let inverse = DomMatrixComponents::identity()
            .scaled_2d(0.1, 0.1)
            .inverse();

        assert!(inverse.is_2d());
        assert_eq!(inverse.m33, 1.0);
        assert_eq!(inverse.m44, 1.0);
    }

    #[test]
    fn combined_rotation_applies_z_then_y_then_x() {
        let combined = DomMatrixComponents::identity().rotated(90.0, 90.0, 90.0);
        let sequential = DomMatrixComponents::identity()
            .rotated(0.0, 0.0, 90.0)
            .rotated(0.0, 90.0, 0.0)
            .rotated(90.0, 0.0, 0.0);

        for (actual, expected) in [
            (combined.m11, sequential.m11),
            (combined.m12, sequential.m12),
            (combined.m13, sequential.m13),
            (combined.m21, sequential.m21),
            (combined.m22, sequential.m22),
            (combined.m23, sequential.m23),
            (combined.m31, sequential.m31),
            (combined.m32, sequential.m32),
            (combined.m33, sequential.m33),
        ] {
            assert_close(actual, expected);
        }
    }

    #[test]
    fn css_text_rejects_non_finite_components() {
        assert_eq!(
            DomMatrixComponents::identity()
                .translated(10.0, 20.0, 0.0)
                .css_text()
                .unwrap(),
            "matrix(1, 0, 0, 1, 10, 20)"
        );
        assert!(DomMatrixComponents::nan().css_text().is_none());
    }

    #[test]
    fn parsing_tracks_transform_syntax_dimension_independently_of_components() {
        let (matrix_2d, is_2d) = parse_dom_matrix_value_with_dimension("matrix(1,0,0,1,0,0)")
            .expect("2D identity matrix should parse");
        assert!(matrix_2d.is_identity());
        assert!(is_2d);

        let (matrix_3d, is_2d) =
            parse_dom_matrix_value_with_dimension("matrix3d(1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1)")
                .expect("3D identity matrix should parse");
        assert!(matrix_3d.is_identity());
        assert!(!is_2d);
    }

    #[test]
    fn parsing_accepts_an_empty_string_but_rejects_only_whitespace() {
        assert!(parse_dom_matrix_value("").is_some());
        assert!(parse_dom_matrix_value(" ").is_none());
    }
}
