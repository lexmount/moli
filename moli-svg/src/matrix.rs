use kurbo::{Affine, Point, Vec2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SvgMatrixComponents {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl SvgMatrixComponents {
    pub fn identity() -> Self {
        Self::from_affine(Affine::IDENTITY)
    }

    pub fn translate(x: f64, y: f64) -> Self {
        Self::from_affine(Affine::translate(Vec2::new(x, y)))
    }

    pub fn scale(x: f64, y: f64) -> Self {
        Self::from_affine(Affine::scale_non_uniform(x, y))
    }

    pub fn rotate(angle: f64) -> Self {
        Self::from_affine(Affine::rotate(angle.to_radians()))
    }

    pub fn rotate_around(angle: f64, cx: f64, cy: f64) -> Self {
        Self::from_affine(Affine::rotate_about(angle.to_radians(), Point::new(cx, cy)))
    }

    pub fn skew_x(angle: f64) -> Self {
        Self::from_affine(Affine::skew(angle.to_radians().tan(), 0.0))
    }

    pub fn skew_y(angle: f64) -> Self {
        Self::from_affine(Affine::skew(0.0, angle.to_radians().tan()))
    }

    pub fn multiply(self, other: Self) -> Self {
        Self::from_affine(self.to_affine() * other.to_affine())
    }

    pub fn then_translate(self, x: f64, y: f64) -> Self {
        Self::from_affine(self.to_affine().pre_translate(Vec2::new(x, y)))
    }

    pub fn then_scale(self, factor: f64) -> Self {
        Self::from_affine(self.to_affine().pre_scale(factor))
    }

    pub fn then_scale_non_uniform(self, x: f64, y: f64) -> Self {
        Self::from_affine(self.to_affine().pre_scale_non_uniform(x, y))
    }

    pub fn then_rotate(self, angle: f64) -> Self {
        Self::from_affine(self.to_affine().pre_rotate(angle.to_radians()))
    }

    pub fn then_rotate_from_vector(self, x: f64, y: f64) -> Option<Self> {
        if x == 0.0 || y == 0.0 {
            return None;
        }
        Some(self.then_rotate(y.atan2(x).to_degrees()))
    }

    pub fn then_flip_x(self) -> Self {
        Self::from_affine(self.to_affine().pre_scale_non_uniform(-1.0, 1.0))
    }

    pub fn then_flip_y(self) -> Self {
        Self::from_affine(self.to_affine().pre_scale_non_uniform(1.0, -1.0))
    }

    pub fn then_skew_x(self, angle: f64) -> Self {
        Self::from_affine(self.to_affine().pre_skew(angle.to_radians().tan(), 0.0))
    }

    pub fn then_skew_y(self, angle: f64) -> Self {
        Self::from_affine(self.to_affine().pre_skew(0.0, angle.to_radians().tan()))
    }

    pub fn determinant(self) -> f64 {
        self.to_affine().determinant()
    }

    pub fn has_finite_components(self) -> bool {
        self.to_affine().is_finite()
    }

    pub fn is_invertible(self) -> bool {
        let determinant = self.determinant();
        self.has_finite_components() && determinant.is_finite() && determinant != 0.0
    }

    pub fn inverse(self) -> Self {
        if !self.is_invertible() {
            return Self::from_affine(Affine::new([f64::NAN; 6]));
        }
        Self::from_affine(self.to_affine().inverse())
    }

    pub fn serialize_transform_matrix(self) -> String {
        format!(
            "matrix({} {} {} {} {} {})",
            serialize_number(self.a),
            serialize_number(self.b),
            serialize_number(self.c),
            serialize_number(self.d),
            serialize_number(self.e),
            serialize_number(self.f)
        )
    }

    pub(crate) fn to_affine(self) -> Affine {
        Affine::new([self.a, self.b, self.c, self.d, self.e, self.f])
    }

    fn from_affine(affine: Affine) -> Self {
        let [a, b, c, d, e, f] = affine.as_coeffs();
        Self { a, b, c, d, e, f }
    }
}

pub fn serialize_number(value: f64) -> String {
    value.to_string()
}

#[cfg(test)]
mod serialization_tests {
    use super::serialize_number;
    #[test]
    fn serialized_svg_numbers_round_trip_large_finite_values_and_signed_zero() {
        for value in [
            1e20_f64,
            -1e20,
            f64::MAX,
            f64::MIN_POSITIVE,
            f64::from(f32::MAX),
            f64::from(f32::from_bits(1)),
            -0.0,
        ] {
            let serialized = serialize_number(value);
            assert_eq!(
                serialized.parse::<f64>().unwrap().to_bits(),
                value.to_bits(),
                "{serialized} must preserve {value}"
            );
        }
    }
}
