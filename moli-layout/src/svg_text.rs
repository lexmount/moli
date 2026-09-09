use std::ops::Range;

use moli_svg::{SvgTextLayout, SvgTextQuery};

/// Source provenance for one SVG text-content element in a frozen pass.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutSvgTextElement<N> {
    pub source: N,
    pub layout_index: usize,
    /// Sorted, disjoint addressable UTF-16 ranges, sampled while the source
    /// tree is still available. No live ancestry traversal is needed by a query.
    pub ranges: Vec<Range<usize>>,
}

/// Canonical numeric text data for one atomic inline-SVG box. The transient
/// usvg paint tree, fonts and source XML do not cross this retention boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutSvgText<N> {
    pub elements: Vec<LayoutSvgTextElement<N>>,
    pub layouts: Vec<SvgTextLayout>,
}

impl<N> Default for LayoutSvgText<N> {
    fn default() -> Self {
        Self {
            elements: Vec::new(),
            layouts: Vec::new(),
        }
    }
}

impl<N: Eq> LayoutSvgText<N> {
    pub fn query(&self, source: N) -> Option<SvgTextQuery<'_>> {
        let element = self
            .elements
            .iter()
            .find(|element| element.source == source)?;
        Some(
            self.layouts
                .get(element.layout_index)?
                .query(&element.ranges),
        )
    }
}

impl<N> LayoutSvgText<N> {
    pub fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(2 * std::mem::size_of::<usize>()) // retained Arc counters
            .saturating_add(
                self.elements
                    .capacity()
                    .saturating_mul(std::mem::size_of::<LayoutSvgTextElement<N>>()),
            )
            .saturating_add(
                self.layouts
                    .capacity()
                    .saturating_mul(std::mem::size_of::<SvgTextLayout>()),
            )
            .saturating_add(self.elements.iter().fold(0usize, |bytes, element| {
                bytes.saturating_add(
                    element
                        .ranges
                        .capacity()
                        .saturating_mul(std::mem::size_of::<Range<usize>>()),
                )
            }))
            .saturating_add(self.layouts.iter().fold(0usize, |bytes, layout| {
                bytes.saturating_add(
                    layout
                        .estimated_bytes()
                        .saturating_sub(std::mem::size_of::<SvgTextLayout>()),
                )
            }))
    }
}
