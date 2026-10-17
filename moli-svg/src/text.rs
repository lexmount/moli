//! Immutable text geometry. Shaping and source-tree ownership live in adapters;
//! these queries never consult DOM, fonts, CSS, or paint resources.

use std::ops::Range;

use crate::{SvgGeometryBox, SvgGeometryPoint, SvgMatrixComponents};

/// One typographic character (possibly several UTF-16 units) from real shaping.
/// A ligature can supply several fragments; fallback glyphs can supply several
/// fragments for the same grapheme. Queries count every physical advance once.
#[derive(Clone, Debug, PartialEq)]
pub struct SvgTextFragment {
    pub code_units: Range<usize>,
    pub transform: SvgMatrixComponents,
    pub advance: f64,
    pub ascent: f64,
    pub descent: f64,
    pub right_to_left: bool,
    pub visible: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SvgTextLayout {
    /// Logical order, independently of each fragment's visual placement.
    pub fragments: Vec<SvgTextFragment>,
}

pub struct SvgTextQuery<'a> {
    layout: &'a SvgTextLayout,
    /// Sorted, disjoint ranges of addressable text belonging to this element.
    ranges: &'a [Range<usize>],
}

pub struct SvgTextCharacter {
    pub start: SvgGeometryPoint,
    pub end: SvgGeometryPoint,
    pub extent: SvgGeometryBox,
    pub rotation: f64,
}

impl SvgTextLayout {
    pub fn query<'a>(&'a self, ranges: &'a [Range<usize>]) -> SvgTextQuery<'a> {
        SvgTextQuery {
            layout: self,
            ranges,
        }
    }

    pub fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>().saturating_add(
            self.fragments
                .capacity()
                .saturating_mul(std::mem::size_of::<SvgTextFragment>()),
        )
    }
}

impl SvgTextQuery<'_> {
    pub fn number_of_chars(&self) -> usize {
        self.ranges
            .iter()
            .map(|range| range.end - range.start)
            .sum()
    }

    pub fn computed_text_length(&self) -> f64 {
        self.fragments_in(0..self.number_of_chars())
            .map(SvgTextFragment::used_advance)
            .sum()
    }

    /// `None` is an out-of-range starting index, including `(0, 0)` on empty
    /// text. An excessive count is clamped rather than wrapping the end index.
    pub fn substring_length(&self, start: usize, count: usize) -> Option<f64> {
        let total = self.number_of_chars();
        (start < total).then(|| {
            self.fragments_in(start..start.saturating_add(count).min(total))
                .map(SvgTextFragment::used_advance)
                .sum()
        })
    }

    pub fn bounding_box(&self) -> Option<SvgGeometryBox> {
        self.transformed_bounding_box(SvgMatrixComponents::identity())
    }

    pub fn transformed_bounding_box(
        &self,
        transform: SvgMatrixComponents,
    ) -> Option<SvgGeometryBox> {
        self.fragments_in(0..self.number_of_chars())
            .filter(|fragment| fragment.visible)
            .map(|fragment| fragment.extent_with_transform(transform))
            .reduce(SvgGeometryBox::union)
    }

    pub fn character(&self, index: usize) -> Option<SvgTextCharacter> {
        if index >= self.number_of_chars() {
            return None;
        }
        let mut fragments = self.fragments_in(index..index + 1);
        let first = fragments.next()?;
        let mut character = first.character();
        for fragment in fragments {
            let next = fragment.character();
            character.end = next.end;
            character.extent = character.extent.union(next.extent);
        }
        Some(character)
    }

    pub fn character_at_position(&self, point: SvgGeometryPoint) -> Option<usize> {
        if !point.x.is_finite() || !point.y.is_finite() {
            return None;
        }
        for fragment in &self.layout.fragments {
            if !fragment.visible || !fragment.contains(point) {
                continue;
            }
            let mut offset = 0;
            for range in self.ranges {
                if overlaps(range, &fragment.code_units) {
                    return Some(offset + fragment.code_units.start.max(range.start) - range.start);
                }
                offset += range.end - range.start;
            }
        }
        None
    }

    fn fragments_in(&self, selected: Range<usize>) -> impl Iterator<Item = &SvgTextFragment> {
        self.layout.fragments.iter().filter(move |fragment| {
            let mut offset = 0;
            self.ranges.iter().any(|range| {
                let local = offset..offset + range.end - range.start;
                offset = local.end;
                if !overlaps(&local, &selected) {
                    return false;
                }
                let intersection = range.start + selected.start.max(local.start) - local.start
                    ..range.start + selected.end.min(local.end) - local.start;
                overlaps(&intersection, &fragment.code_units)
            })
        })
    }
}

impl SvgTextFragment {
    fn used_advance(&self) -> f64 {
        self.advance * self.transform.a.hypot(self.transform.b)
    }

    fn character(&self) -> SvgTextCharacter {
        if !self.visible {
            return SvgTextCharacter {
                start: SvgGeometryPoint::new(0.0, 0.0),
                end: SvgGeometryPoint::new(0.0, 0.0),
                extent: SvgGeometryBox {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                },
                rotation: 0.0,
            };
        }
        let (start, end) = if self.right_to_left {
            (self.advance, 0.0)
        } else {
            (0.0, self.advance)
        };
        SvgTextCharacter {
            start: map_point(self.transform, start, 0.0),
            end: map_point(self.transform, end, 0.0),
            extent: self.extent(),
            rotation: self.transform.b.atan2(self.transform.a).to_degrees(),
        }
    }

    fn extent(&self) -> SvgGeometryBox {
        self.extent_with_transform(SvgMatrixComponents::identity())
    }

    fn extent_with_transform(&self, transform: SvgMatrixComponents) -> SvgGeometryBox {
        let transform = transform.multiply(self.transform);
        [
            (0.0, -self.ascent),
            (self.advance, -self.ascent),
            (self.advance, -self.descent),
            (0.0, -self.descent),
        ]
        .into_iter()
        .map(|(x, y)| {
            let point = map_point(transform, x, y);
            SvgGeometryBox {
                x: point.x,
                y: point.y,
                width: 0.0,
                height: 0.0,
            }
        })
        .reduce(SvgGeometryBox::union)
        .expect("a glyph cell has four corners")
    }

    fn contains(&self, point: SvgGeometryPoint) -> bool {
        if !self.transform.is_invertible() {
            return false;
        }
        let local = map_point(self.transform.inverse(), point.x, point.y);
        local.x >= 0.0
            && local.x <= self.advance
            && local.y >= -self.ascent
            && local.y <= -self.descent
    }
}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < a.end && b.start < b.end && a.start < b.end && b.start < a.end
}

fn map_point(matrix: SvgMatrixComponents, x: f64, y: f64) -> SvgGeometryPoint {
    SvgGeometryPoint::new(
        matrix.a * x + matrix.c * y + matrix.e,
        matrix.b * x + matrix.d * y + matrix.f,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fragment(range: Range<usize>, x: f64, advance: f64) -> SvgTextFragment {
        SvgTextFragment {
            code_units: range,
            transform: SvgMatrixComponents::translate(x, 20.0),
            advance,
            ascent: 15.0,
            descent: -5.0,
            right_to_left: false,
            visible: true,
        }
    }

    #[test]
    fn ranges_use_utf16_and_count_a_grapheme_advance_once() {
        let layout = SvgTextLayout {
            fragments: vec![fragment(0..2, 10.0, 20.0), fragment(2..3, 30.0, 8.0)],
        };
        let range = 0..3;
        let query = layout.query(std::slice::from_ref(&range));
        assert_eq!(query.number_of_chars(), 3);
        assert_eq!(query.computed_text_length(), 28.0);
        assert_eq!(query.substring_length(0, 1), Some(20.0));
        assert_eq!(query.substring_length(1, 1), Some(20.0));
        assert_eq!(query.substring_length(0, 2), Some(20.0));
        assert_eq!(query.substring_length(2, usize::MAX), Some(8.0));
        assert_eq!(query.substring_length(0, 0), Some(0.0));
        assert_eq!(query.substring_length(3, 0), None);
        assert_eq!(
            query.character(0).unwrap().start.x,
            query.character(1).unwrap().start.x
        );
    }

    #[test]
    fn subtree_ranges_exclude_other_spans_without_counting_position_gaps() {
        let layout = SvgTextLayout {
            fragments: vec![
                fragment(0..1, 0.0, 10.0),
                fragment(1..2, 50.0, 20.0),
                fragment(2..3, 70.0, 5.0),
            ],
        };
        let query = layout.query(&[0..1, 2..3]);
        assert_eq!(query.number_of_chars(), 2);
        assert_eq!(query.computed_text_length(), 15.0);
        assert_eq!(query.character(1).unwrap().start.x, 70.0);
        assert_eq!(query.bounding_box().unwrap().width, 75.0);
        assert_eq!(
            query.character_at_position(SvgGeometryPoint::new(72.0, 20.0)),
            Some(1)
        );
        assert_eq!(
            query.character_at_position(SvgGeometryPoint::new(60.0, 20.0)),
            None
        );
        assert_eq!(
            query.character_at_position(SvgGeometryPoint::new(f64::NAN, 20.0)),
            None
        );
    }

    #[test]
    fn glyph_scaling_and_rotation_use_the_same_transform_for_geometry_and_length() {
        let mut glyph = fragment(0..1, 0.0, 10.0);
        glyph.transform = SvgMatrixComponents::translate(30.0, 40.0)
            .multiply(SvgMatrixComponents::rotate(90.0))
            .multiply(SvgMatrixComponents::scale(2.0, 1.0));
        glyph.right_to_left = true;
        let layout = SvgTextLayout {
            fragments: vec![glyph],
        };
        let range = 0..1;
        let query = layout.query(std::slice::from_ref(&range));
        assert_eq!(query.computed_text_length(), 20.0);
        let character = query.character(0).unwrap();
        assert!((character.start.x - 30.0).abs() < 1e-8);
        assert!((character.start.y - 60.0).abs() < 1e-8);
        assert_eq!(character.end.y, 40.0);
        assert_eq!(character.rotation, 90.0);
    }
}
