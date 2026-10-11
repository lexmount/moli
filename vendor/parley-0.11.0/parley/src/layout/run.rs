// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::layout::cluster::Cluster;
use crate::layout::data::{LineItemData, RunData};
use crate::layout::layout::Layout;
use crate::layout::spacing::{EffectiveSpacing, Gaps, Justification};
use crate::style::Brush;

use core::ops::Range;
use fontique::Synthesis;
use parley_engine::{
    Atom, Atoms, FontInstance, FontMetrics, Glyph, Graphemes, NormalizedCoord, ShapedClusterGlyphs,
    ShapedRun, ShapedSlice,
};

/// Sequence of shaped clusters using a single font.
pub struct Run<'a, B: Brush> {
    pub(crate) layout: &'a Layout<B>,
    /// The index of the line this run is part of.
    pub(crate) line_index: u32,
    /// The index of the run within the line it is part of.
    pub(crate) index: u32,
    /// The index of the shaped run within [`parley_engine::ShapedText`].
    pub(crate) shaped_text_run_index: u32,
    pub(crate) shaped: &'a ShapedRun,
    pub(crate) data: &'a RunData,
    pub(crate) line_data: Option<&'a LineItemData>,
}

// `Run` is `Copy` and `Clone` regardless of `B`.
impl<B: Brush> Copy for Run<'_, B> {}
impl<B: Brush> Clone for Run<'_, B> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, B: Brush> Run<'a, B> {
    #[expect(clippy::cast_possible_truncation, reason = "deferred")]
    pub(crate) fn new(
        layout: &'a Layout<B>,
        line_index: u32,
        index: u32,
        run_index: usize,
        line_data: Option<&'a LineItemData>,
    ) -> Self {
        Self {
            layout,
            line_index,
            index,
            shaped_text_run_index: run_index as u32,
            shaped: &layout.data.shaped_text.runs()[run_index],
            data: &layout.data.runs[run_index],
            line_data,
        }
    }

    /// Borrow the shaped content of this run as a slice.
    ///
    /// Note this covers the whole shaped run even when this [`Run`] is scoped to a line. Also see
    /// [`Self::line_slice`].
    pub(crate) fn full_slice(&self) -> ShapedSlice<'a> {
        self.layout
            .data
            .shaped_text
            .run_slice(self.shaped_text_run_index)
    }

    /// Borrow the shaped content of this run as a slice, narrowed to this [`Run`]'s line.
    pub(crate) fn line_slice(&self) -> ShapedSlice<'a> {
        let slice = self.full_slice();
        match self.line_data {
            Some(line_data) => slice.narrow(line_data.shaped_cluster_range.clone()),
            None => slice,
        }
    }

    /// Returns the index of the run within the line.
    pub fn index(&self) -> usize {
        self.index as usize
    }

    /// Returns the font for the run.
    pub fn font(&self) -> &FontInstance {
        self.layout
            .data
            .shaped_text
            .fonts()
            .get(self.shaped.font_index)
            .unwrap()
    }

    /// Returns the font size for the run.
    pub fn font_size(&self) -> f32 {
        self.shaped.font_size
    }

    /// Returns the font attributes for the run.
    pub fn font_attrs(&self) -> &fontique::Attributes {
        &self.data.font_attrs
    }

    /// Returns the synthesis suggestions for the font associated with the run.
    pub fn synthesis(&self) -> Synthesis {
        self.data.synthesis
    }

    /// Returns the normalized variation coordinates for the font associated
    /// with the run.
    pub fn normalized_coords(&self) -> &[NormalizedCoord] {
        self.layout
            .data
            .shaped_text
            .normalized_coords()
            .get(self.shaped.normalized_coords_range.clone())
            .unwrap_or(&[])
    }

    /// Returns the line height resolved from the full shaped run's first character's
    /// style and its font metrics.
    ///
    /// Styles within a shaped run can have different line heights. This value uses
    /// the same first character even when this run is scoped to one line. Use
    /// [`crate::Line::metrics`] for the containing line's metrics.
    pub fn line_height(&self) -> f32 {
        self.data.line_height
    }

    /// Returns metrics for the run.
    pub fn font_metrics(&self) -> &FontMetrics {
        &self.shaped.font_metrics
    }

    #[inline]
    pub(crate) fn line_spacing(&self) -> EffectiveSpacing {
        let justification = if self.line_data.is_some() {
            self.layout.data.lines[self.line_index as usize].justification
        } else {
            Justification::NONE
        };
        EffectiveSpacing::new(self.data.spacing, justification)
    }

    /// Returns the advance for the run.
    ///
    /// This includes the additional advances inserted for word spacing, letter spacing, and, if the
    /// layout has been [aligned](Layout::align) with [`Justify`](crate::Alignment::Justify), the
    /// justification.
    pub fn advance(&self) -> f32 {
        let spacing = self.line_spacing();
        let raw = spacing.slice_advance(self.line_slice());
        if let Some(allocated) = self.line_data.and_then(|data| data.allocated_advance) {
            let unadjusted = EffectiveSpacing::new(self.data.spacing, Justification::NONE)
                .slice_advance(self.line_slice());
            allocated + (raw - unadjusted)
        } else {
            raw
        }
    }

    /// Returns the original text range for the run.
    pub fn text_range(&self) -> Range<usize> {
        self.line_data
            .map(|d| &d.text_range)
            .unwrap_or(&self.shaped.range.byte_range)
            .clone()
    }

    /// Returns `true` if the run has right-to-left directionality.
    pub fn is_rtl(&self) -> bool {
        self.shaped.bidi_level.is_rtl()
    }

    /// Returns the cluster range for the run.
    ///
    /// The indices are grapheme cluster indices, relative to the shaped run this [`Run`] belongs
    /// to: for a run scoped to a line, this is the sub-range of the shaped run's clusters that fall
    /// on that line; otherwise it covers all of the shaped run's clusters.
    ///
    /// Note this counts the shaped run's clusters, so the cost is linear in the shaped run's
    /// length.
    pub fn cluster_range(&self) -> Range<usize> {
        let start = match self.line_data {
            Some(line_data) => count_graphemes(self.full_slice().narrow(
                self.shaped.shaped_clusters_range.start..line_data.shaped_cluster_range.start,
            )),
            None => 0,
        };
        start..start + self.len()
    }

    /// Returns the number of clusters in the run.
    ///
    /// Note this counts the run's clusters, so the cost is linear in the run's length. To check
    /// whether the run has any clusters, use [`Self::is_empty`].
    pub fn len(&self) -> usize {
        count_graphemes(self.line_slice())
    }

    /// Returns `true` if the run is empty.
    pub fn is_empty(&self) -> bool {
        self.line_slice().shaped_clusters_range().is_empty()
    }

    /// Returns the cluster at the specified logical index.
    ///
    /// Note this walks the run's clusters, so the cost is `O(index)`.
    pub fn get(&self, index: usize) -> Option<Cluster<'a, B>> {
        Clusters::new(*self, false).nth(index)
    }

    /// Returns an iterator over the clusters in logical order.
    pub fn clusters(&self) -> impl Iterator<Item = Cluster<'a, B>> + Clone + use<'a, B> {
        Clusters::new(*self, false)
    }

    /// Returns an iterator over the clusters in visual order.
    pub fn visual_clusters(&self) -> impl Iterator<Item = Cluster<'a, B>> + Clone + use<'a, B> {
        Clusters::new(*self, self.is_rtl())
    }

    /// Returns an iterator over the clusters in reverse visual order.
    pub(crate) fn visual_clusters_rev(
        &self,
    ) -> impl Iterator<Item = Cluster<'a, B>> + Clone + use<'a, B> {
        Clusters::new(*self, !self.is_rtl())
    }

    /// The cluster containing the character at `char_index`.
    ///
    /// Returns `None` if `char_index` is not within this run.
    pub(crate) fn cluster_containing_char(&self, char_index: u32) -> Option<Cluster<'a, B>> {
        let atom = self.line_slice().atom_at_char(char_index)?;
        let grapheme = atom
            .graphemes_start()
            .find(|grapheme| grapheme.char_range().contains(&char_index))?;
        Some(Cluster {
            run: *self,
            atom,
            grapheme,
        })
    }

    /// The cluster containing the source text byte at `text_byte`.
    ///
    /// Returns `None` if `text_byte` is not within this run.
    pub(crate) fn cluster_at_text_byte(&self, text_byte: usize) -> Option<Cluster<'a, B>> {
        let slice = self.line_slice();
        let atom = slice.atom_at_text_byte(u32::try_from(text_byte).ok()?)?;
        let grapheme = atom
            .graphemes_start()
            .find(|grapheme| slice.text_byte_range(grapheme.char_range()).end > text_byte)?;
        Some(Cluster {
            run: *self,
            atom,
            grapheme,
        })
    }

    /// An iterator over the glyphs in `shaped_clusters` in visual left-to-right order.
    ///
    /// This includes additional spacing from [`EffectiveSpacing`].
    ///
    /// `shaped_clusters` indexes into the shaped run this [`Run`] belongs to, and must be
    /// atom-aligned.
    pub(crate) fn glyphs_in(self, shaped_clusters: Range<u32>) -> Glyphs<'a> {
        Glyphs::new(
            self.full_slice().narrow(shaped_clusters),
            self.is_rtl(),
            self.line_spacing(),
        )
    }
}

/// An iterator over the glyphs of a range of shaped clusters, in visual left-to-right order.
///
/// See [`Run::glyphs_in`].
#[derive(Clone)]
pub(crate) struct Glyphs<'a> {
    /// The slice of shaped text narrowed to the shaped clusters being iterated.
    slice: ShapedSlice<'a>,
    is_rtl: bool,
    /// The spacing to apply, if any. Without spacing, atoms are not tracked.
    spacing: Option<EffectiveSpacing>,
    /// The shaped clusters not yet visited: all of `slice`'s clusters without spacing, otherwise
    /// those of the current atom. Visited in logical order for LTR text and in reverse for RTL.
    shaped_clusters: Range<u32>,
    /// The not-yet-yielded glyphs of the current shaped cluster.
    glyphs: ShapedClusterGlyphs<'a>,
    /// The gaps around the current atom; only meaningful with `spacing`.
    gaps: Gaps,
    /// The index within the current atom of the next glyph to yield. The first one gets
    /// `gaps.before`.
    atom_glyph: usize,
    /// The number of glyphs of the current atom. The last one gets `gaps.after`.
    atom_glyph_count: usize,
}

impl<'a> Glyphs<'a> {
    fn new(slice: ShapedSlice<'a>, is_rtl: bool, spacing: EffectiveSpacing) -> Self {
        let clusters = slice.shaped_clusters_range();
        Self {
            slice,
            is_rtl,
            spacing: (!spacing.is_zero()).then_some(spacing),
            shaped_clusters: if spacing.is_zero() {
                clusters
            } else {
                // With spacing, clusters are visited atom by atom: start out on no atom, at the
                // visually leftmost edge of the slice.
                if is_rtl {
                    clusters.end..clusters.end
                } else {
                    clusters.start..clusters.start
                }
            },
            glyphs: ShapedClusterGlyphs::empty(),
            gaps: Gaps::ZERO,
            atom_glyph: 0,
            atom_glyph_count: 0,
        }
    }
}

impl Iterator for Glyphs<'_> {
    type Item = Glyph;

    #[inline]
    fn next(&mut self) -> Option<Glyph> {
        loop {
            if let Some(mut glyph) = self.glyphs.next() {
                if self.spacing.is_some() {
                    if self.atom_glyph == 0 {
                        glyph.x += self.gaps.before;
                        glyph.advance += self.gaps.before;
                    }
                    self.atom_glyph += 1;
                    if self.atom_glyph == self.atom_glyph_count {
                        glyph.advance += self.gaps.after;
                    }
                }
                return Some(glyph);
            }

            let cluster = if self.is_rtl {
                self.shaped_clusters.next_back()
            } else {
                self.shaped_clusters.next()
            };
            if let Some(cluster) = cluster {
                self.glyphs = self.slice.shaped_cluster_glyphs(cluster);
                continue;
            }

            // Without spacing, that was the last cluster. With spacing, it was the last cluster of
            // the current atom; move on to the next atom.
            let spacing = self.spacing?;
            let atom = if self.is_rtl {
                self.slice.atoms_from(self.shaped_clusters.start).prev()
            } else {
                self.slice.atoms_from(self.shaped_clusters.end).next()
            }?;
            self.shaped_clusters = atom.shaped_clusters_range();
            self.gaps = spacing.gaps(&atom);
            self.atom_glyph = 0;
            self.atom_glyph_count = atom
                .shaped_clusters()
                .iter()
                .map(|cluster| usize::from(cluster.glyph_len()))
                .sum();
        }
    }
}

/// An iterator over a [`Run`]'s clusters.
///
/// This walks the run's graphemes. Each grapheme is one [`Cluster`].
struct Clusters<'a, B: Brush> {
    run: Run<'a, B>,
    /// Cursor over the run's (line-scoped) atoms.
    atoms: Atoms<'a>,
    /// Grapheme cursor over the run's (line-scoped) slice.
    graphemes: Graphemes<'a>,
    /// The atom containing the most recently yielded grapheme; `None` before the first grapheme.
    atom: Option<Atom<'a>>,
    /// Whether iteration is in reverse logical order.
    rev: bool,
}

impl<'a, B: Brush> Clusters<'a, B> {
    fn new(run: Run<'a, B>, rev: bool) -> Self {
        let slice = run.line_slice();
        Self {
            run,
            atoms: if rev {
                slice.atoms_end()
            } else {
                slice.atoms_start()
            },
            graphemes: if rev {
                slice.graphemes_end()
            } else {
                slice.graphemes_start()
            },
            atom: None,
            rev,
        }
    }
}

impl<B: Brush> Clone for Clusters<'_, B> {
    fn clone(&self) -> Self {
        Self {
            run: self.run,
            atoms: self.atoms,
            graphemes: self.graphemes,
            atom: self.atom,
            rev: self.rev,
        }
    }
}

impl<'a, B: Brush> Iterator for Clusters<'a, B> {
    type Item = Cluster<'a, B>;

    fn next(&mut self) -> Option<Self::Item> {
        let grapheme = if self.rev {
            self.graphemes.prev()?
        } else {
            self.graphemes.next()?
        };
        let entered_new_atom = if self.rev {
            grapheme.is_atom_end()
        } else {
            grapheme.is_atom_start()
        };
        if entered_new_atom {
            self.atom = if self.rev {
                self.atoms.prev()
            } else {
                self.atoms.next()
            };
        }
        let atom = self.atom.expect(
            "The first call to `next` should always be an atom edge, so this should always be set at this point.",
        );
        if self.run.layout.data.text_item_quantization.is_some() {
            let range = self.run.full_slice().text_byte_range(grapheme.char_range());
            let item = self.run.text_range();
            if range.start >= item.end || range.end <= item.start {
                return self.next();
            }
        }
        Some(Cluster {
            run: self.run,
            atom,
            grapheme,
        })
    }
}

/// The number of graphemes in `slice`.
///
/// This is `O(n)` in the slice's characters.
fn count_graphemes(slice: ShapedSlice<'_>) -> usize {
    slice
        .characters()
        .iter()
        .filter(|character| character.grapheme_start)
        .count()
}
