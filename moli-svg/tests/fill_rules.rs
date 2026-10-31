use moli_svg::{
    SvgFillRule, SvgGeometryElement, SvgGeometryPoint, is_point_in_fill, is_point_in_fill_with_rule,
};

#[test]
fn fill_containment_rejects_nonfinite_coordinates() {
    let circle = SvgGeometryElement::Circle {
        cx: 0.0,
        cy: 0.0,
        r: 10.0,
    };
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for point in [
            SvgGeometryPoint::new(value, 0.0),
            SvgGeometryPoint::new(0.0, value),
        ] {
            for rule in [SvgFillRule::NonZero, SvgFillRule::EvenOdd] {
                assert!(!is_point_in_fill_with_rule(&circle, point, rule));
            }
        }
    }
}

#[test]
fn evenodd_fill_excludes_nested_interiors_but_includes_boundaries() {
    let rings = SvgGeometryElement::Path {
        d: "M0 0H10V10H0Z M2 2H8V8H2Z".to_owned(),
    };
    let center = SvgGeometryPoint::new(5.0, 5.0);
    assert!(is_point_in_fill(&rings, center));
    assert!(!is_point_in_fill_with_rule(
        &rings,
        center,
        SvgFillRule::EvenOdd
    ));
    for point in [
        SvgGeometryPoint::new(1.0, 5.0),
        SvgGeometryPoint::new(2.0, 5.0),
        SvgGeometryPoint::new(8.0, 5.0),
        SvgGeometryPoint::new(10.0, 5.0),
    ] {
        assert!(is_point_in_fill_with_rule(
            &rings,
            point,
            SvgFillRule::EvenOdd
        ));
    }
}
