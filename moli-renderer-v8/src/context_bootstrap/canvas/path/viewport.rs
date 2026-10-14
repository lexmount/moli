//! Intersect native fill geometry with the bitmap before narrowing to f32.
//!
//! Paths already inside the bitmap keep the existing curve backend. Outside
//! paths use exact rational subdivision and polygon clipping: subtracting two
//! enormous endpoints must not erase a visible, pixel-sized intersection.

use moli_paint::{MAX_PAINT_OPERATIONS, PaintError};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{ToPrimitive, Zero};

use super::coordinates::{Point, Scalar};
use super::{
    Canvas2dPathState, LayoutPoint, PaintPath, PaintPathElement, PathElement, path_bounds,
};

const MAX_COORDINATE_BITS: u64 = 16_384;

#[derive(Clone, PartialEq)]
struct ExactPoint {
    x: BigRational,
    y: BigRational,
}

struct Budget {
    operations: usize,
}

impl Budget {
    fn charge(&mut self, units: usize) -> Result<(), PaintError> {
        self.operations = self.operations.saturating_add(units);
        if self.operations > MAX_PAINT_OPERATIONS {
            return Err(PaintError::PaintOperationBudgetExceeded {
                operations: self.operations,
                max_operations: MAX_PAINT_OPERATIONS,
            });
        }
        Ok(())
    }

    fn point(&mut self, point: &ExactPoint) -> Result<(), PaintError> {
        let sizes = [
            point.x.numer().bits(),
            point.x.denom().bits(),
            point.y.numer().bits(),
            point.y.denom().bits(),
        ];
        let bits = *sizes.iter().max().unwrap();
        check_precision(bits)?;
        // Charge for arithmetic precision as well as the number of vertices.
        self.charge(1 + (sizes.iter().sum::<u64>() / 256) as usize)
    }
}

fn check_precision(bits: u64) -> Result<(), PaintError> {
    if bits > MAX_COORDINATE_BITS {
        return Err(PaintError::CanvasClipCoordinateBudgetExceeded {
            bits,
            max_bits: MAX_COORDINATE_BITS,
        });
    }
    Ok(())
}

fn exact(value: Scalar) -> Result<BigRational, PaintError> {
    let (significand, exponent) = value.binary_parts();
    if significand == 0.0 {
        return Ok(BigRational::zero());
    }
    check_precision(exponent.unsigned_abs().saturating_add(53))?;
    let value = BigRational::from_float(significand).expect("native coordinates are finite");
    let scale = BigRational::from_integer(BigInt::from(1) << exponent.unsigned_abs() as usize);
    Ok(if exponent >= 0 {
        value * scale
    } else {
        value / scale
    })
}

impl ExactPoint {
    fn from_native(point: Point, budget: &mut Budget) -> Result<Self, PaintError> {
        let point = Self {
            x: exact(point.x)?,
            y: exact(point.y)?,
        };
        budget.point(&point)?;
        Ok(point)
    }

    fn midpoint(&self, other: &Self, budget: &mut Budget) -> Result<Self, PaintError> {
        let point = Self {
            x: (&self.x + &other.x) / BigInt::from(2),
            y: (&self.y + &other.y) / BigInt::from(2),
        };
        budget.point(&point)?;
        Ok(point)
    }

    fn paint(&self) -> LayoutPoint {
        LayoutPoint::new(
            self.x.to_f32().expect("clipped x is finite"),
            self.y.to_f32().expect("clipped y is finite"),
        )
    }
}

#[derive(Clone, Copy)]
enum Axis {
    X,
    Y,
}

struct Boundary {
    axis: Axis,
    value: BigRational,
    minimum: bool,
}

impl Boundary {
    fn coordinate<'a>(&self, point: &'a ExactPoint) -> &'a BigRational {
        match self.axis {
            Axis::X => &point.x,
            Axis::Y => &point.y,
        }
    }

    fn inside(&self, point: &ExactPoint) -> bool {
        if self.minimum {
            self.coordinate(point) >= &self.value
        } else {
            self.coordinate(point) <= &self.value
        }
    }

    fn intersection(
        &self,
        a: &ExactPoint,
        b: &ExactPoint,
        budget: &mut Budget,
    ) -> Result<ExactPoint, PaintError> {
        let (ax, ay, bx, by) = match self.axis {
            Axis::X => (&a.x, &a.y, &b.x, &b.y),
            Axis::Y => (&a.y, &a.x, &b.y, &b.x),
        };
        let other = (ay * bx - by * ax + &self.value * (by - ay)) / (bx - ax);
        let point = match self.axis {
            Axis::X => ExactPoint {
                x: self.value.clone(),
                y: other,
            },
            Axis::Y => ExactPoint {
                x: other,
                y: self.value.clone(),
            },
        };
        budget.point(&point)?;
        Ok(point)
    }
}

fn square(value: &BigRational) -> BigRational {
    value * value
}

fn flat(points: &[ExactPoint]) -> bool {
    let a = &points[0];
    let b = points.last().unwrap();
    let dx = &b.x - &a.x;
    let dy = &b.y - &a.y;
    let length_squared = square(&dx) + square(&dy);
    let tolerance_squared = BigRational::new(1.into(), (1024 * 1024).into());
    points[1..points.len() - 1].iter().all(|point| {
        let x = &point.x - &a.x;
        let y = &point.y - &a.y;
        if length_squared.is_zero() {
            square(&x) + square(&y) <= tolerance_squared
        } else {
            square(&(&dx * y - &dy * x)) <= &tolerance_squared * &length_squared
        }
    })
}

fn flatten(
    points: Vec<ExactPoint>,
    boundaries: &[Boundary; 4],
    contour: &mut Vec<ExactPoint>,
    budget: &mut Budget,
) -> Result<(), PaintError> {
    let mut stack = vec![points];
    while let Some(points) = stack.pop() {
        budget.charge(1)?;
        // A curve and its chord are homotopic outside the bitmap whenever the
        // entire control hull is outside one clipping half-plane.
        if boundaries
            .iter()
            .any(|edge| points.iter().all(|point| !edge.inside(point)))
            || flat(&points)
        {
            contour.push(points.last().unwrap().clone());
            continue;
        }
        let count = points.len();
        let mut triangle = points;
        let mut left = vec![triangle[0].clone()];
        let mut right = vec![triangle[count - 1].clone()];
        for level in 1..count {
            for index in 0..count - level {
                triangle[index] = triangle[index].midpoint(&triangle[index + 1], budget)?;
            }
            left.push(triangle[0].clone());
            right.push(triangle[count - 1 - level].clone());
        }
        right.reverse();
        stack.push(right);
        stack.push(left);
    }
    Ok(())
}

fn finish(
    contour: &mut Vec<ExactPoint>,
    boundaries: &[Boundary; 4],
    elements: &mut Vec<PaintPathElement>,
    budget: &mut Budget,
) -> Result<(), PaintError> {
    let mut polygon = std::mem::take(contour);
    for edge in boundaries {
        let mut clipped = Vec::new();
        if let Some(mut previous) = polygon.last() {
            for point in &polygon {
                budget.charge(1)?;
                let inside = edge.inside(point);
                if inside != edge.inside(previous) {
                    clipped.push(edge.intersection(previous, point, budget)?);
                }
                if inside {
                    clipped.push(point.clone());
                }
                previous = point;
            }
        }
        polygon = clipped;
    }
    if let Some(first) = polygon.first() {
        budget.charge(polygon.len() + 1)?;
        elements.push(PaintPathElement::MoveTo(first.paint()));
        elements.extend(
            polygon[1..]
                .iter()
                .map(|point| PaintPathElement::LineTo(point.paint())),
        );
        elements.push(PaintPathElement::Close);
    }
    Ok(())
}

pub(super) fn paint_path(
    path: &Canvas2dPathState,
    width: u32,
    height: u32,
) -> Result<PaintPath, PaintError> {
    let mut budget = Budget { operations: 0 };
    budget.charge(path.elements.len())?;
    let bounds = [-1.0, f64::from(width) + 1.0, -1.0, f64::from(height) + 1.0];
    if path
        .elements
        .iter()
        .flat_map(PathElement::points)
        .all(|point| {
            point.x >= Scalar::new(bounds[0])
                && point.x <= Scalar::new(bounds[1])
                && point.y >= Scalar::new(bounds[2])
                && point.y <= Scalar::new(bounds[3])
        })
    {
        return Ok(path.paint_path());
    }
    let boundaries = [
        Boundary {
            axis: Axis::X,
            value: BigRational::from_integer((-1).into()),
            minimum: true,
        },
        Boundary {
            axis: Axis::X,
            value: BigRational::from_integer((u64::from(width) + 1).into()),
            minimum: false,
        },
        Boundary {
            axis: Axis::Y,
            value: BigRational::from_integer((-1).into()),
            minimum: true,
        },
        Boundary {
            axis: Axis::Y,
            value: BigRational::from_integer((u64::from(height) + 1).into()),
            minimum: false,
        },
    ];
    let mut contour = Vec::new();
    let mut elements = Vec::new();
    for element in &path.elements {
        match *element {
            PathElement::MoveTo(point) => {
                finish(&mut contour, &boundaries, &mut elements, &mut budget)?;
                contour.push(ExactPoint::from_native(point, &mut budget)?);
            }
            PathElement::LineTo(point) => {
                contour.push(ExactPoint::from_native(point, &mut budget)?)
            }
            PathElement::QuadTo(control, end) => {
                let points = vec![
                    contour.last().expect("subpath starts with MoveTo").clone(),
                    ExactPoint::from_native(control, &mut budget)?,
                    ExactPoint::from_native(end, &mut budget)?,
                ];
                flatten(points, &boundaries, &mut contour, &mut budget)?;
            }
            PathElement::CubicTo(a, b, end) => {
                let points = vec![
                    contour.last().expect("subpath starts with MoveTo").clone(),
                    ExactPoint::from_native(a, &mut budget)?,
                    ExactPoint::from_native(b, &mut budget)?,
                    ExactPoint::from_native(end, &mut budget)?,
                ];
                flatten(points, &boundaries, &mut contour, &mut budget)?;
            }
            PathElement::Close => finish(&mut contour, &boundaries, &mut elements, &mut budget)?,
        }
    }
    finish(&mut contour, &boundaries, &mut elements, &mut budget)?;
    Ok(PaintPath {
        bounds: path_bounds(&elements),
        elements,
    })
}
