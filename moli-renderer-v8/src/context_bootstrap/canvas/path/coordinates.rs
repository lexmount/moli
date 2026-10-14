//! Finite native coordinates with f64 precision and a separate binary exponent.
//! Affine products may exceed f64's exponent range even when every JS input is
//! finite. Geometry queries retain those products; narrowing belongs to paint.

use std::cmp::Ordering;
use std::ops::{Add, Div, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Scalar {
    significand: f64,
    exponent: i64,
}

impl Scalar {
    pub(super) const ZERO: Self = Self {
        significand: 0.0,
        exponent: 0,
    };

    pub(super) fn new(value: f64) -> Self {
        debug_assert!(value.is_finite());
        if value == 0.0 {
            return Self::ZERO;
        }
        let bits = value.to_bits();
        let exponent = ((bits >> 52) & 0x7ff) as i64;
        if exponent == 0 {
            let normal = Self::new(value * power_of_two(1022));
            return Self {
                exponent: normal.exponent - 1022,
                ..normal
            };
        }
        let significand = f64::from_bits((bits & 0x800f_ffff_ffff_ffff) | (1023_u64 << 52));
        Self {
            significand,
            exponent: exponent - 1023,
        }
    }

    pub(super) fn is_zero(self) -> bool {
        self.significand == 0.0
    }

    #[cfg(test)]
    pub(super) fn scaled(self, exponent: i64) -> Self {
        if self.is_zero() {
            self
        } else {
            Self {
                exponent: self.exponent + exponent,
                ..self
            }
        }
    }

    pub(super) fn is_small_for_f64_geometry(self) -> bool {
        self.is_zero() || (-120..=120).contains(&self.exponent)
    }

    pub(super) fn finite(self) -> Option<f64> {
        if self.is_zero() {
            return Some(0.0);
        }
        if !(-1074..=1023).contains(&self.exponent) {
            return None;
        }
        let value = if self.exponent >= -1022 {
            self.significand * power_of_two(self.exponent)
        } else {
            (self.significand * power_of_two(-1022)) * power_of_two(self.exponent + 1022)
        };
        value.is_finite().then_some(value)
    }

    pub(super) fn lossy(self) -> f64 {
        self.finite().unwrap_or_else(|| {
            if self.exponent < -1074 {
                0.0
            } else {
                self.significand.signum() * f64::INFINITY
            }
        })
    }
}

fn power_of_two(exponent: i64) -> f64 {
    debug_assert!((-1074..=1023).contains(&exponent));
    if exponent >= -1022 {
        f64::from_bits(((exponent + 1023) as u64) << 52)
    } else {
        f64::from_bits(1_u64 << (exponent + 1074))
    }
}

impl PartialOrd for Scalar {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self.significand.signum() != other.significand.signum()
            || self.is_zero()
            || other.is_zero()
        {
            return self.significand.partial_cmp(&other.significand);
        }
        let magnitude = self
            .exponent
            .cmp(&other.exponent)
            .then_with(|| self.significand.abs().total_cmp(&other.significand.abs()));
        Some(if self.significand < 0.0 {
            magnitude.reverse()
        } else {
            magnitude
        })
    }
}

impl Add for Scalar {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        if self.is_zero() {
            return other;
        }
        if other.is_zero() {
            return self;
        }
        let (large, small) = if self.exponent >= other.exponent {
            (self, other)
        } else {
            (other, self)
        };
        let difference = small.exponent - large.exponent;
        if difference < -1074 {
            return large;
        }
        let normalized =
            Self::new(large.significand + small.significand * power_of_two(difference));
        if normalized.is_zero() {
            normalized
        } else {
            Self {
                exponent: normalized.exponent + large.exponent,
                ..normalized
            }
        }
    }
}

impl Neg for Scalar {
    type Output = Self;
    fn neg(self) -> Self {
        Self {
            significand: -self.significand,
            ..self
        }
    }
}

impl Sub for Scalar {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        self + -other
    }
}

impl Mul for Scalar {
    type Output = Self;
    fn mul(self, other: Self) -> Self {
        if self.is_zero() || other.is_zero() {
            return Self::ZERO;
        }
        let normalized = Self::new(self.significand * other.significand);
        Self {
            exponent: normalized.exponent + self.exponent + other.exponent,
            ..normalized
        }
    }
}

impl Div for Scalar {
    type Output = Self;
    fn div(self, other: Self) -> Self {
        debug_assert!(!other.is_zero());
        if self.is_zero() {
            return Self::ZERO;
        }
        let normalized = Self::new(self.significand / other.significand);
        Self {
            exponent: normalized.exponent + self.exponent - other.exponent,
            ..normalized
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Point {
    pub(super) x: Scalar,
    pub(super) y: Scalar,
}

impl Point {
    pub(super) const ZERO: Self = Self {
        x: Scalar::ZERO,
        y: Scalar::ZERO,
    };

    pub(super) fn new(x: f64, y: f64) -> Self {
        Self {
            x: Scalar::new(x),
            y: Scalar::new(y),
        }
    }

    pub(super) fn transform(self, matrix: kurbo::Affine) -> Self {
        if matrix == kurbo::Affine::IDENTITY {
            return self;
        }
        let [a, b, c, d, e, f] = matrix.as_coeffs().map(Scalar::new);
        Self {
            x: a * self.x + c * self.y + e,
            y: b * self.x + d * self.y + f,
        }
    }

    pub(super) fn finite(self) -> Option<kurbo::Point> {
        Some(kurbo::Point::new(self.x.finite()?, self.y.finite()?))
    }

    pub(super) fn midpoint(self, other: Self) -> Self {
        let half = Scalar::new(0.5);
        Self {
            x: (self.x + other.x) * half,
            y: (self.y + other.y) * half,
        }
    }

    pub(super) fn relative_to(self, origin: Self) -> Self {
        Self {
            x: self.x - origin.x,
            y: self.y - origin.y,
        }
    }
}

pub(super) fn inverse(matrix: kurbo::Affine) -> Option<kurbo::Affine> {
    if !matrix.is_finite() {
        return None;
    }
    let [a, b, c, d, e, f] = matrix.as_coeffs().map(Scalar::new);
    let determinant = a * d - b * c;
    if determinant.is_zero() {
        return None;
    }
    Some(kurbo::Affine::new([
        (d / determinant).finite()?,
        (-b / determinant).finite()?,
        (-c / determinant).finite()?,
        (a / determinant).finite()?,
        ((c * f - d * e) / determinant).finite()?,
        ((b * e - a * f) / determinant).finite()?,
    ]))
}

pub(super) fn is_invertible(matrix: kurbo::Affine) -> bool {
    if !matrix.is_finite() {
        return false;
    }
    let [a, b, c, d, _, _] = matrix.as_coeffs().map(Scalar::new);
    !(a * d - b * c).is_zero()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_coordinates_preserve_overflow_products_and_underflow_signs() {
        let huge = Scalar::new(f64::MAX) * Scalar::new(8.0);
        assert!(huge > Scalar::new(f64::MAX));
        assert_eq!(huge / Scalar::new(8.0), Scalar::new(f64::MAX));
        assert!((huge + -huge).is_zero());
        let tiny = Scalar::new(f64::MIN_POSITIVE) * Scalar::new(f64::MIN_POSITIVE);
        assert!(tiny > Scalar::ZERO);
        assert!(-tiny < Scalar::ZERO);
        assert!(tiny.finite().is_none());
    }

    #[test]
    fn native_inverse_accepts_finite_large_and_small_scales() {
        for scale in [f64::MAX, 1e200, 1e-200, 1e-308] {
            let matrix = kurbo::Affine::scale(scale);
            assert!(is_invertible(matrix));
            let coefficients = inverse(matrix).unwrap().as_coeffs();
            assert!((coefficients[0] * scale - 1.0).abs() < 1e-14);
            assert!((coefficients[3] * scale - 1.0).abs() < 1e-14);
        }
        assert!(!is_invertible(kurbo::Affine::scale_non_uniform(0.0, 1.0)));
        assert!(inverse(kurbo::Affine::new([1.0, 1.0, 1.0, 1.0, 0.0, 0.0])).is_none());
    }
}
