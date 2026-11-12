//! PDF annotations do not inherit content-stream clips or transforms. Project
//! their hit regions explicitly before encoding them, including page slicing.
use std::collections::BTreeMap;

use krilla::{
    action::{Action, LinkAction},
    annotation::{LinkAnnotation, Target},
    destination::{Destination, XyzDestination},
    geom::{Point as PdfPoint, Quadrilateral},
    page::Page,
};
use moli_layout::{PaintFilter, PaintFragment, PaintLinkTarget, PaintShape, PaintSnapshot};
use peniko::kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape};

use super::{PdfError, PdfOptions};

type Polygon = Vec<Point>;
pub(super) struct LinkRegion {
    polygon: Polygon,
    target: PaintLinkTarget,
}

enum Clip {
    Rectangle(Polygon),
    Path(Vec<(Point, Point)>, Rect),
}

struct Scope {
    clip: Clip,
    hidden: bool,
}

fn spend(budget: &mut usize, count: usize) -> Result<(), PdfError> {
    *budget = budget.checked_sub(count).ok_or(PdfError::ResourceLimit)?;
    Ok(())
}

pub(super) fn collect(snapshot: &PaintSnapshot) -> Result<Vec<LinkRegion>, PdfError> {
    if snapshot.links.is_empty() {
        return Ok(Vec::new());
    }
    let mut links = snapshot.links.iter().collect::<Vec<_>>();
    links.sort_by_key(|link| link.before_fragment);
    let mut links = links.into_iter().peekable();
    let mut scopes: Vec<Scope> = Vec::new();
    let mut regions: Vec<LinkRegion> = Vec::new();
    let mut budget = 20_000_000;
    let capture = Rect::new(
        0.0,
        0.0,
        snapshot.surface.css_width.into(),
        snapshot.surface.css_height.into(),
    );
    for index in 0..=snapshot.fragments.len() {
        while let Some(link) = links.next_if(|link| link.before_fragment == index) {
            if scopes.iter().any(|scope| scope.hidden) {
                continue;
            }
            let matrix = Affine::new(link.transform.coefficients);
            let polygon = rectangle(matrix, super::rect(link.rect));
            if polygon
                .iter()
                .any(|p| !(p.x as f32).is_finite() || !(p.y as f32).is_finite())
            {
                return Err(PdfError::Encoding(
                    "invalid transformed link geometry".into(),
                ));
            }
            let mut polygons = vec![clip_rect(polygon, capture)];
            for scope in &scopes {
                let mut clipped = Vec::new();
                for polygon in polygons {
                    spend(&mut budget, polygon.len())?;
                    match &scope.clip {
                        Clip::Rectangle(clip) => clipped.push(intersect(polygon, clip)),
                        Clip::Path(edges, bounds) => {
                            curved_clip(polygon, edges, *bounds, &mut clipped, &mut budget)?;
                        }
                    }
                }
                polygons = clipped;
            }
            for polygon in polygons.into_iter().filter(|p| area(p).abs() > 1e-6) {
                spend(&mut budget, 100)?;
                regions.push(LinkRegion {
                    polygon,
                    target: link.target.clone(),
                });
            }
        }
        let Some(fragment) = snapshot.fragments.get(index) else {
            break;
        };
        let (shape, matrix, hidden) = match fragment {
            PaintFragment::PushClip { shape, transform } => (shape, transform, false),
            PaintFragment::PushLayer {
                clip,
                transform,
                opacity,
                filter,
                ..
            } => (
                clip,
                transform,
                *opacity <= 0.0 || matches!(filter, Some(PaintFilter::Opacity(v)) if *v <= 0.0),
            ),
            PaintFragment::PopLayer => {
                scopes.pop();
                continue;
            }
            _ => continue,
        };
        let matrix = Affine::new(matrix.coefficients);
        let clip = match shape {
            PaintShape::Rect(rect) => Clip::Rectangle(rectangle(matrix, super::rect(*rect))),
            _ => {
                let path = matrix * crate::geometry::to_backend_shape(shape);
                let edges = edges(&path);
                spend(&mut budget, edges.len())?;
                Clip::Path(edges, path.bounding_box())
            }
        };
        scopes.push(Scope { clip, hidden });
    }
    deduplicate(regions, &mut budget)
}

fn deduplicate(regions: Vec<LinkRegion>, budget: &mut usize) -> Result<Vec<LinkRegion>, PdfError> {
    // Paint phases can emit text children before their enclosing inline anchor.
    // A top-to-bottom sweep, with larger polygons first at the same top, visits
    // each container before its descendants regardless of their paint order.
    // Compare the already-clipped polygons, without joining separate lines or
    // filling gaps, and retain the original order of surviving annotations.
    let mut order = regions
        .iter()
        .enumerate()
        .map(|(index, region)| {
            let (top, bottom) = region.polygon.iter().fold(
                (f64::INFINITY, f64::NEG_INFINITY),
                |(top, bottom), point| (top.min(point.y), bottom.max(point.y)),
            );
            (index, top, bottom, area(&region.polygon).abs())
        })
        .collect::<Vec<_>>();
    order.sort_by(|(_, a_top, _, a_area), (_, b_top, _, b_area)| {
        a_top
            .total_cmp(b_top)
            .then_with(|| b_area.total_cmp(a_area))
    });
    let mut active: Vec<(usize, f64)> = Vec::new();
    let mut keep = vec![false; regions.len()];
    for (index, top, bottom, _) in order {
        spend(budget, active.len().saturating_add(1))?;
        active.retain(|&(_, bottom)| bottom + 1e-6 >= top);
        let region = &regions[index];
        let mut duplicate = false;
        for &(other, _) in &active {
            let other = &regions[other];
            if other.target != region.target {
                continue;
            }
            spend(
                budget,
                region.polygon.len().saturating_mul(other.polygon.len()),
            )?;
            if region.polygon.iter().all(|p| contains(&other.polygon, *p)) {
                duplicate = true;
                break;
            }
        }
        if !duplicate {
            keep[index] = true;
            active.push((index, bottom));
        }
    }
    Ok(regions
        .into_iter()
        .zip(keep)
        .filter_map(|(region, keep)| keep.then_some(region))
        .collect())
}

pub(super) struct PageLinks<'a> {
    pub regions: &'a [LinkRegion],
    pub selected: &'a BTreeMap<usize, usize>,
    pub page_height: f64,
    pub scale: f64,
    pub options: &'a PdfOptions,
}

impl PageLinks<'_> {
    pub fn append(&self, page: &mut Page<'_>, source: Rect) -> Result<(), PdfError> {
        let matrix = Affine::translate((
            self.options.margin_left,
            self.options.margin_top - source.y0 * self.scale,
        )) * Affine::scale(self.scale);
        for region in self.regions {
            let polygon = clip_rect(region.polygon.clone(), source);
            if area(&polygon).abs() <= 1e-6 {
                continue;
            }
            let polygon = polygon.into_iter().map(|p| matrix * p).collect::<Vec<_>>();
            let mut quads = Vec::new();
            if polygon.len() == 4 {
                quads.push(quad([polygon[3], polygon[2], polygon[1], polygon[0]]));
            } else {
                for i in 1..polygon.len() - 1 {
                    let a = polygon[0];
                    let b = polygon[i];
                    let c = polygon[i + 1];
                    // Split one triangle edge to provide four distinct points.
                    quads.push(quad([c, b, a.midpoint(b), a]));
                }
            }
            let destination = region.target.destination.and_then(|p| {
                let original = (f64::from(p.y).max(0.0) / self.page_height).floor() as usize;
                let output = *self.selected.get(&original)?;
                let point = PdfPoint::from_xy(
                    (self.options.margin_left + f64::from(p.x) * self.scale) as f32,
                    (self.options.margin_top
                        + (f64::from(p.y).max(0.0) - original as f64 * self.page_height)
                            * self.scale) as f32,
                );
                Some(Target::Destination(Destination::Xyz(XyzDestination::new(
                    output, point,
                ))))
            });
            let target = destination.unwrap_or_else(|| {
                Target::Action(Action::Link(LinkAction::new(region.target.url.to_string())))
            });
            page.add_annotation(LinkAnnotation::new_with_quad_points(quads, target).into());
        }
        Ok(())
    }
}

fn quad(points: [Point; 4]) -> Quadrilateral {
    Quadrilateral(points.map(|p| PdfPoint::from_xy(p.x as f32, p.y as f32)))
}

fn rectangle(matrix: Affine, rect: Rect) -> Polygon {
    let mut points = vec![
        matrix * Point::new(rect.x0, rect.y0),
        matrix * Point::new(rect.x1, rect.y0),
        matrix * Point::new(rect.x1, rect.y1),
        matrix * Point::new(rect.x0, rect.y1),
    ];
    if area(&points) < 0.0 {
        points.reverse();
    }
    points
}

fn cross(a: Point, b: Point, p: Point) -> f64 {
    (b - a).cross(p - a)
}
fn area(points: &[Point]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a.x * b.y - a.y * b.x)
        .sum::<f64>()
        * 0.5
}
fn contains(polygon: &[Point], point: Point) -> bool {
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
        .all(|(a, b)| cross(*a, *b, point) >= -1e-6)
}
fn clip_rect(polygon: Polygon, rect: Rect) -> Polygon {
    intersect(polygon, &rectangle(Affine::IDENTITY, rect))
}

// Sutherland–Hodgman clipping: both the link rectangle and transformed
// rectangular clips are convex. Preserve their actual polygons, not their AABBs.
fn intersect(mut polygon: Polygon, clip: &[Point]) -> Polygon {
    if area(clip).abs() <= 1e-6 {
        return Vec::new();
    }
    for (a, b) in clip
        .iter()
        .zip(clip.iter().cycle().skip(1))
        .take(clip.len())
    {
        let Some(mut previous) = polygon.last().copied() else {
            break;
        };
        let mut out = Vec::new();
        let mut prev_side = cross(*a, *b, previous);
        for point in polygon {
            let side = cross(*a, *b, point);
            if (side >= 0.0) != (prev_side >= 0.0) {
                let t = prev_side / (prev_side - side);
                out.push(previous.lerp(point, t));
            }
            if side >= 0.0 {
                out.push(point);
            }
            previous = point;
            prev_side = side;
        }
        polygon = out;
    }
    polygon
}

fn edges(path: &BezPath) -> Vec<(Point, Point)> {
    let mut edges = Vec::new();
    let mut start = Point::ZERO;
    let mut previous = Point::ZERO;
    peniko::kurbo::flatten(path.iter(), 0.1, |element| match element {
        PathEl::MoveTo(p) => {
            if previous != start {
                edges.push((previous, start));
            }
            start = p;
            previous = p;
        }
        PathEl::LineTo(p) => {
            edges.push((previous, p));
            previous = p;
        }
        PathEl::ClosePath => {
            edges.push((previous, start));
            previous = start;
        }
        _ => unreachable!(),
    });
    if previous != start {
        edges.push((previous, start));
    }
    edges
}

// PDF links only accept quadrilaterals. Approximate curved/non-convex clips
// with half-CSS-pixel scanline strips using the same nonzero winding rule as
// paint. This retains holes and avoids clickable bounding-box corners.
fn curved_clip(
    polygon: Polygon,
    edges: &[(Point, Point)],
    bounds: Rect,
    out: &mut Vec<Polygon>,
    budget: &mut usize,
) -> Result<(), PdfError> {
    let polygon = clip_rect(polygon, bounds);
    if area(&polygon).abs() <= 1e-6 {
        return Ok(());
    }
    let y0 = polygon.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    let y1 = polygon
        .iter()
        .map(|p| p.y)
        .fold(f64::NEG_INFINITY, f64::max);
    let rows = ((y1 - y0) * 2.0).ceil() as usize;
    spend(budget, rows.saturating_mul(edges.len().max(1)))?;
    for row in 0..rows {
        let top = y0 + row as f64 * 0.5;
        let bottom = (top + 0.5).min(y1);
        let y = (top + bottom) * 0.5;
        let mut crossings = edges
            .iter()
            .filter_map(|(a, b)| {
                if (a.y <= y && b.y > y) || (b.y <= y && a.y > y) {
                    Some((
                        a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y),
                        if b.y > a.y { 1 } else { -1 },
                    ))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut winding = 0;
        let mut left = 0.0;
        for (x, delta) in crossings {
            if winding == 0 {
                left = x;
            }
            winding += delta;
            if winding == 0 && x > left {
                spend(budget, polygon.len() + 100)?;
                let clipped = clip_rect(polygon.clone(), Rect::new(left, top, x, bottom));
                if area(&clipped).abs() > 1e-6 {
                    out.push(clipped);
                }
            }
        }
    }
    Ok(())
}
