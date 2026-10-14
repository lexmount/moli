use super::super::path::coordinates::{Point, Scalar};

#[derive(Clone, Copy)]
pub(super) enum Geometry {
    Linear(Point, Point),
    Radial {
        start: Point,
        end: Point,
        r0: Scalar,
        r1: Scalar,
    },
    Conic {
        center: Point,
        angle: f64,
    },
}

impl Default for Geometry {
    fn default() -> Self {
        Self::Linear(Point::ZERO, Point::ZERO)
    }
}

fn dot(a: Point, b: Point) -> Scalar {
    a.x * b.x + a.y * b.y
}

impl Geometry {
    pub(super) fn linear(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self::Linear(Point::new(x0, y0), Point::new(x1, y1))
    }
    pub(super) fn radial(x0: f64, y0: f64, r0: f64, x1: f64, y1: f64, r1: f64) -> Self {
        Self::Radial {
            start: Point::new(x0, y0),
            end: Point::new(x1, y1),
            r0: Scalar::new(r0),
            r1: Scalar::new(r1),
        }
    }
    pub(super) fn conic(angle: f64, x: f64, y: f64) -> Self {
        Self::Conic {
            center: Point::new(x, y),
            angle: angle.rem_euclid(std::f64::consts::TAU),
        }
    }

    pub(super) fn parameter(self, point: Point) -> Option<f64> {
        match self {
            Self::Linear(start, end) => {
                let vector = end.relative_to(start);
                let length = dot(vector, vector);
                if length.is_zero() {
                    None
                } else {
                    Some((dot(point.relative_to(start), vector) / length).lossy())
                }
            }
            Self::Radial { start, end, r0, r1 } => radial(point, start, end, r0, r1),
            Self::Conic { center, angle } => {
                let p = point.relative_to(center);
                let scale = dot(p, p).sqrt()?;
                let theta = if scale.is_zero() {
                    0.0
                } else {
                    (p.y / scale).lossy().atan2((p.x / scale).lossy())
                };
                Some((theta - angle).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU)
            }
        }
    }
}

fn radial(point: Point, start: Point, end: Point, r0: Scalar, r1: Scalar) -> Option<f64> {
    if start == end && r0 == r1 {
        return None;
    }
    let dc = end.relative_to(start);
    let p = point.relative_to(start);
    let dr = r1 - r0;
    let two = Scalar::new(2.0);
    let a = dot(dc, dc) - dr * dr;
    let b = -two * (dot(p, dc) + r0 * dr);
    let c = dot(p, p) - r0 * r0;
    if a.is_zero() {
        if b.is_zero() {
            if !c.is_zero() {
                return None;
            }
            return Some(if dr > Scalar::ZERO {
                f64::INFINITY
            } else {
                (-r0 / dr).lossy()
            });
        }
        let t = -c / b;
        return (r0 + dr * t >= Scalar::ZERO).then(|| t.lossy());
    }
    let discriminant = b * b - Scalar::new(4.0) * a * c;
    let root = discriminant.sqrt()?;
    let q = -Scalar::new(0.5) * (b + if b < Scalar::ZERO { -root } else { root });
    let candidates = if q.is_zero() {
        [Scalar::ZERO; 2]
    } else {
        [q / a, c / q]
    };
    candidates
        .into_iter()
        .filter(|t| r0 + dr * *t >= Scalar::ZERO)
        .max_by(|a, b| a.partial_cmp(b).unwrap())
        .map(Scalar::lossy)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gradients_keep_large_and_subnormal_geometry_and_radial_cone_rules() {
        for scale in [f64::MAX, 1e-308] {
            let g = Geometry::linear(-scale, 0.0, scale, 0.0);
            assert_eq!(g.parameter(Point::ZERO), Some(0.5));
        }
        assert_eq!(
            Geometry::linear(1.0, 1.0, 1.0, 1.0).parameter(Point::ZERO),
            None
        );
        for (r0, r1) in [(0.0, 10.0), (10.0, 0.0)] {
            assert_eq!(
                Geometry::radial(0.0, 0.0, r0, 0.0, 0.0, r1).parameter(Point::new(5.0, 0.0)),
                Some(0.5)
            );
        }
        let g = Geometry::radial(0.0, 0.0, 5.0, 10.0, 0.0, 5.0);
        assert_eq!(g.parameter(Point::new(5.0, 0.0)), Some(1.0));
        assert_eq!(g.parameter(Point::new(5.0, 10.0)), None);
        let g = Geometry::conic(0.0, 0.0, 0.0);
        assert_eq!(g.parameter(Point::new(1.0, 0.0)), Some(0.0));
        assert_eq!(g.parameter(Point::new(0.0, 1.0)), Some(0.25));
        assert_eq!(g.parameter(Point::new(0.0, -1.0)), Some(0.75));
    }
}
