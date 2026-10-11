// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use core::ops::Range;

use crate::{Glyph, shape::Whitespace};

use super::data::{Character, ShapedCluster};

/// A slice of shaped text.
///
/// Note that character and cluster indices (including those given out by [`Atom`] and [`Grapheme`])
/// are absolute into the underlying storage.
///
// NOTE: The motivation for this type, is that once `parley_engine` supports reshaping, this can
// hold reshaped slices that don't depend on `ShapedText`.
#[derive(Copy, Clone, Debug)]
pub struct ShapedSlice<'a> {
    /// Characters of shaped text.
    ///
    /// Most character indices (like [`ShapedCluster::chars_range`]) index into this array.
    pub(crate) characters: &'a [Character],

    pub(crate) shaped_clusters: &'a [ShapedCluster],
    pub(crate) glyphs: &'a [Glyph],

    /// The range of [`Self::shaped_clusters`] this slice covers.
    pub(crate) clusters: (u32, u32),
}

impl<'a> ShapedSlice<'a> {
    /// The character range of this slice.
    ///
    /// This indexes into [`Self::characters`].
    #[inline(always)]
    pub fn char_range(&self) -> Range<u32> {
        if self.clusters.0 == self.clusters.1 {
            0..0
        } else {
            self.shaped_clusters[self.clusters.0 as usize]
                .chars_range()
                .start
                ..self.shaped_clusters[self.clusters.1 as usize - 1]
                    .chars_range()
                    .end
        }
    }

    /// The [`Character`]s spanned by this shaped slice.
    #[inline(always)]
    pub fn characters(&self) -> &'a [Character] {
        self.characters_in(self.char_range())
    }

    /// The [`Character`]s within this slice spanned by `chars_range`.
    ///
    /// Note that `chars_range` is an absolute range over the source text.
    #[inline(always)]
    pub fn characters_in(&self, chars_range: Range<u32>) -> &'a [Character] {
        debug_assert!(
            chars_range.is_empty()
                || (self.char_range().start <= chars_range.start
                    && chars_range.end <= self.char_range().end),
            "character range out of this slice's range"
        );
        &self.characters[chars_range.start as usize..chars_range.end as usize]
    }

    /// The cluster range of this slice.
    ///
    /// This indexes into [`Self::shaped_clusters`].
    #[inline(always)]
    pub fn shaped_clusters_range(&self) -> Range<u32> {
        self.clusters.0..self.clusters.1
    }

    /// The [`ShapedCluster`]s spanned by this shaped slice.
    #[inline(always)]
    pub fn shaped_clusters(&self) -> &'a [ShapedCluster] {
        self.shaped_clusters_in(self.shaped_clusters_range())
    }

    /// The [`ShapedCluster`]s spanned by `clusters_range`.
    #[inline(always)]
    pub fn shaped_clusters_in(&self, clusters_range: Range<u32>) -> &'a [ShapedCluster] {
        debug_assert!(
            clusters_range.is_empty()
                || (self.clusters.0 <= clusters_range.start
                    && clusters_range.end <= self.clusters.1),
            "cluster range out of this slice's range"
        );
        &self.shaped_clusters[clusters_range.start as usize..clusters_range.end as usize]
    }

    /// The byte range in the source text of the given range of [`Self::characters`].
    ///
    /// If `chars` is empty, this is the empty range at the byte position of `chars.start`; see
    /// [`Self::text_byte_at`].
    pub fn text_byte_range(&self, chars: Range<u32>) -> Range<usize> {
        if chars.is_empty() {
            let pos = self.text_byte_at(chars.start);
            return pos..pos;
        }
        let first = &self.characters[chars.start as usize];
        let last = &self.characters[chars.end as usize - 1];
        first.text_byte_start as usize..last.text_byte_range().end
    }

    /// The byte position in the source text of the character boundary at `char_idx`.
    ///
    /// `char_idx` must be within this slice's [`Self::char_range`].
    #[inline]
    pub fn text_byte_at(&self, char_idx: u32) -> usize {
        let char_range = self.char_range();
        debug_assert!(
            !char_range.is_empty(),
            "called `text_byte_at` on an empty slice"
        );
        debug_assert!(
            char_range.start <= char_idx && char_idx <= char_range.end,
            "called `text_byte_at` with a character index out of the slice's range"
        );
        if char_idx == char_range.end {
            // The character after this boundary is outside this slice. Note that `Self::characters`
            // is the full slice of characters that were shaped, so the character after
            // `char_range.end` is not necessarily the next character in the source text. We use the
            // boundary behind the last character of this slice instead.
            self.characters[char_idx as usize - 1].text_byte_range().end
        } else {
            self.characters[char_idx as usize].text_byte_start as usize
        }
    }

    /// Get an iterator over glyphs in visual left-to-right order of the shaped cluster with the
    /// given index.
    #[inline]
    pub fn shaped_cluster_glyphs(&self, cluster_idx: u32) -> ShapedClusterGlyphs<'a> {
        debug_assert!(
            self.clusters.0 <= cluster_idx && cluster_idx < self.clusters.1,
            "cluster index out of this slice's range"
        );

        let shaped_cluster = self.shaped_clusters[cluster_idx as usize];

        if shaped_cluster.has_inline_glyph() {
            ShapedClusterGlyphs {
                inline: Some(Glyph {
                    id: shaped_cluster.glyph_offset,
                    x: 0.,
                    y: 0.,
                    advance: shaped_cluster.advance,
                }),
                stored: [].iter(),
            }
        } else {
            let start = shaped_cluster.glyph_offset as usize;
            ShapedClusterGlyphs {
                inline: None,
                stored: self.glyphs[start..start + usize::from(shaped_cluster.glyph_len())].iter(),
            }
        }
    }

    /// Get a cursor to walk atoms from the logical start of this slice.
    ///
    /// The cursor cannot go outside this slice, so the first step can only be taken forwards.
    /// Immediately going backwards will return `None`.
    #[inline(always)]
    pub fn atoms_start(&self) -> Atoms<'a> {
        self.atoms_from(self.clusters.0)
    }

    /// Get a cursor to walk atoms from the logical end of this slice.
    ///
    /// The cursor cannot go outside this slice, so the first step can only be taken backwards.
    /// Immediately going forwards will return `None`.
    #[inline(always)]
    pub fn atoms_end(&self) -> Atoms<'a> {
        self.atoms_from(self.clusters.1)
    }

    /// Get a cursor to walk atoms of this slice, starting at the atom boundary at `cluster`.
    ///
    /// `cluster` indexes into [`Self::shaped_clusters`], must be either a cluster of this slice or
    /// the end of the slice, and must be an atom boundary (i.e.,
    /// [`ShapedCluster::is_grapheme_start`] must be `true`).
    ///
    /// To find the atom containing an arbitrary position, use [`Self::atom_at_char`] or
    /// [`Self::atom_at_text_byte`] instead.
    #[inline(always)]
    pub fn atoms_from(&self, cluster: u32) -> Atoms<'a> {
        debug_assert!(
            self.clusters.0 <= cluster && cluster <= self.clusters.1,
            "cluster out of this slice's range"
        );
        debug_assert!(
            cluster == self.clusters.0
                || cluster == self.clusters.1
                || self.shaped_clusters[cluster as usize].is_grapheme_start(),
            "cluster is not an atom boundary"
        );
        Atoms {
            slice: *self,
            cluster_idx: cluster,
        }
    }

    /// Get the atom containing the character at `char_index`.
    ///
    /// `char_index` is a character index into the source text (in the Unicode scalar value sense).
    ///
    /// To start walking from the returned atom, call [`Atom::cursor_before`] or
    /// [`Atom::cursor_after`].
    #[inline]
    #[expect(clippy::cast_possible_truncation, reason = "deferred")]
    pub fn atom_at_char(&self, char_index: u32) -> Option<Atom<'a>> {
        let shaped_clusters =
            &self.shaped_clusters[self.clusters.0 as usize..self.clusters.1 as usize];

        let idx = shaped_clusters
            .partition_point(|cluster| cluster.chars_range().start <= char_index)
            .checked_sub(1)?;
        if shaped_clusters[idx].chars_range().end <= char_index {
            return None;
        }

        let mut idx = idx as u32;
        idx += self.clusters.0;
        while idx > self.clusters.0 && !self.shaped_clusters[idx as usize].is_grapheme_start() {
            idx -= 1;
        }

        Some(self.atoms_from(idx).next().unwrap())
    }

    /// Get the atom containing the character at `text_byte`.
    ///
    /// `text_byte` is a byte into the source text.
    ///
    /// To start walking from the returned atom, call [`Atom::cursor_before`] or
    /// [`Atom::cursor_after`].
    #[inline]
    #[expect(clippy::cast_possible_truncation, reason = "deferred")]
    pub fn atom_at_text_byte(&self, text_byte: u32) -> Option<Atom<'a>> {
        let shaped_clusters =
            &self.shaped_clusters[self.clusters.0 as usize..self.clusters.1 as usize];

        let idx = shaped_clusters
            .partition_point(|cluster| {
                self.characters[cluster.chars_range().start as usize].text_byte_start <= text_byte
            })
            .checked_sub(1)?;
        let last_character = self.characters[shaped_clusters[idx].chars_range().end as usize - 1];
        if last_character.text_byte_range().end <= text_byte as usize {
            return None;
        }

        let mut idx = idx as u32;
        idx += self.clusters.0;
        while idx > self.clusters.0 && !self.shaped_clusters[idx as usize].is_grapheme_start() {
            idx -= 1;
        }

        Some(self.atoms_from(idx).next().unwrap())
    }

    /// Narrow the shaped slice to the given range of clusters.
    ///
    /// The range indexes into [`Self::shaped_clusters`], must be a subrange of
    /// [`Self::shaped_clusters_range`], and both bounds must be an atom boundary (i.e.,
    /// [`ShapedCluster::is_grapheme_start`] must be `true`).
    #[inline(always)]
    pub fn narrow(&self, clusters: Range<u32>) -> Self {
        debug_assert!(
            clusters.start <= clusters.end,
            "narrowed cluster range is inverted"
        );
        debug_assert!(
            self.clusters.0 <= clusters.start && clusters.end <= self.clusters.1,
            "narrowed cluster range out of this slice's range"
        );
        debug_assert!(
            clusters.is_empty()
                || ((clusters.start == self.clusters.0
                    || self.shaped_clusters[clusters.start as usize].is_grapheme_start())
                    && (clusters.end == self.clusters.1
                        || self.shaped_clusters[clusters.end as usize].is_grapheme_start())),
            "narrowed cluster range is not atom-aligned"
        );
        Self {
            clusters: (clusters.start, clusters.end),
            ..*self
        }
    }

    /// Get a cursor to walk graphemes from the logical start of this slice.
    #[inline]
    pub fn graphemes_start(&self) -> Graphemes<'a> {
        let char_range = self.char_range();
        let cluster_idx = self.clusters.0;
        Graphemes {
            slice: *self,
            char_idx: char_range.start,
            cluster_idx,
            partial_advance: self.partial_advance_at(cluster_idx),
        }
    }

    /// Get a cursor to walk graphemes from the logical end of this slice.
    #[inline]
    pub fn graphemes_end(&self) -> Graphemes<'a> {
        let char_range = self.char_range();
        let cluster_idx = self.clusters.1.max(self.clusters.0 + 1) - 1;
        Graphemes {
            slice: *self,
            // char_range,
            char_idx: char_range.end,
            cluster_idx,
            partial_advance: self.partial_advance_at(cluster_idx),
        }
    }

    #[inline(always)]
    fn partial_advance_at(&self, cluster_idx: u32) -> f32 {
        if self.clusters.0 == self.clusters.1 {
            return 0.0;
        }
        let cluster = &self.shaped_clusters[cluster_idx as usize];
        cluster.advance / cluster.graphemes_overlapped(self.characters) as f32
    }
}

/// An iterator over the glyphs of a single [`ShapedCluster`], in visual left-to-right order.
///
/// See [`ShapedSlice::shaped_cluster_glyphs`]. The default value yields no glyphs.
#[derive(Clone, Debug)]
pub struct ShapedClusterGlyphs<'a> {
    /// The cluster's glyph if it is stored inline.
    inline: Option<Glyph>,
    /// The cluster's glyphs in the glyph array.
    stored: core::slice::Iter<'a, Glyph>,
}

impl ShapedClusterGlyphs<'static> {
    /// An empty iterator that does not return any glyphs.
    ///
    /// This can be used to store a value of this type before actually having access to any shaped
    /// clusters.
    pub fn empty() -> Self {
        Self {
            inline: None,
            stored: [].iter(),
        }
    }
}

impl Iterator for ShapedClusterGlyphs<'_> {
    type Item = Glyph;

    #[inline]
    fn next(&mut self) -> Option<Glyph> {
        if let Some(glyph) = self.inline.take() {
            return Some(glyph);
        }
        self.stored.next().copied()
    }

    #[inline]
    fn nth(&mut self, n: usize) -> Option<Glyph> {
        match self.inline.take() {
            Some(glyph) if n == 0 => Some(glyph),
            Some(_) => self.stored.nth(n - 1).copied(),
            None => self.stored.nth(n).copied(),
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = usize::from(self.inline.is_some()) + self.stored.len();
        (len, Some(len))
    }
}

impl ExactSizeIterator for ShapedClusterGlyphs<'_> {}

/// An [`Atom`] cursor.
///
/// This cursor can walk forwards and backwards over atoms.
#[derive(Copy, Clone, Debug)]
pub struct Atoms<'a> {
    slice: ShapedSlice<'a>,

    /// The gap we're pointing at.
    cluster_idx: u32,
}

impl<'a> Atoms<'a> {
    /// Get the atom logically preceding the cursor.
    ///
    /// See also [`Self::next`].
    #[inline]
    pub fn prev(&mut self) -> Option<Atom<'a>> {
        let end = self.cluster_idx;
        let bound = self.slice.clusters.0;
        if end == bound {
            return None;
        }

        let mut idx = end - 1;
        let mut advance = self.slice.shaped_clusters[idx as usize].advance;
        while idx > bound && !self.slice.shaped_clusters[idx as usize].is_grapheme_start() {
            idx -= 1;
            advance += self.slice.shaped_clusters[idx as usize].advance;
        }
        self.cluster_idx = idx;
        Some(Atom {
            slice: self.slice,
            clusters: (idx, end),
            chars: (
                self.slice.shaped_clusters[idx as usize].chars_range().start,
                self.slice.shaped_clusters[end as usize - 1]
                    .chars_range()
                    .end,
            ),
            advance,
        })
    }

    /// Walk the cursor backwards as an iterator.
    #[inline(always)]
    pub fn rev(mut self) -> impl Iterator<Item = Atom<'a>> + Clone + use<'a> {
        core::iter::from_fn(move || self.prev())
    }
}

impl<'a> Iterator for Atoms<'a> {
    type Item = Atom<'a>;

    /// Get the atom logically following the cursor.
    ///
    /// See also [`Self::prev`].
    #[inline]
    fn next(&mut self) -> Option<Atom<'a>> {
        let start = self.cluster_idx;
        let bound = self.slice.clusters.1;
        if start == bound {
            return None;
        }

        let mut advance = self.slice.shaped_clusters[start as usize].advance;
        let mut idx = start + 1;
        while idx < bound && !self.slice.shaped_clusters[idx as usize].is_grapheme_start() {
            advance += self.slice.shaped_clusters[idx as usize].advance;
            idx += 1;
        }
        self.cluster_idx = idx;
        Some(Atom {
            slice: self.slice,
            clusters: (start, idx),
            chars: (
                self.slice.shaped_clusters[start as usize]
                    .chars_range()
                    .start,
                self.slice.shaped_clusters[idx as usize - 1]
                    .chars_range()
                    .end,
            ),
            advance,
        })
    }
}

/// An atom of shaped text.
///
/// This is smallest span of text where both edges are a [`Grapheme`] boundary and [`ShapedCluster`]
/// boundary.
///
/// An atom cannot be broken without reshaping. Check [`Self::is_safe_to_break_before`] to see if
/// you can break the text before this atom without requiring reshaping.
#[derive(Copy, Clone, Debug)]
pub struct Atom<'a> {
    slice: ShapedSlice<'a>,

    /// The clusters this atom spans inside [`ShapedSlice::shaped_clusters`].
    clusters: (u32, u32),

    /// The characters this atom spans inside [`ShapedSlice::characters`].
    chars: (u32, u32),

    advance: f32,
}
impl<'a> Atom<'a> {
    /// The range of characters into the underlying [`ShapedSlice`] this atom spans.
    ///
    /// You can use [`ShapedSlice::text_byte_range`] to turn this into a byte range of the source
    /// text.
    pub fn char_range(&self) -> Range<u32> {
        self.chars.0..self.chars.1
    }

    /// The [`Character`]s from the underlying [`ShapedSlice`] this atom spans.
    #[inline(always)]
    pub fn characters(&self) -> &'a [Character] {
        self.slice.characters_in(self.char_range())
    }

    /// The range of [`ShapedCluster`] into the underlying [`ShapedSlice`] this atom spans.
    #[inline(always)]
    pub fn shaped_clusters_range(&self) -> Range<u32> {
        self.clusters.0..self.clusters.1
    }

    /// The [`ShapedCluster`]s from the underlying [`ShapedSlice`] this atom spans.
    #[inline(always)]
    pub fn shaped_clusters(&self) -> &'a [ShapedCluster] {
        self.slice.shaped_clusters_in(self.shaped_clusters_range())
    }

    /// This atom as a [`ShapedSlice`].
    #[inline(always)]
    pub fn slice(&self) -> ShapedSlice<'a> {
        ShapedSlice {
            clusters: self.clusters,
            ..self.slice
        }
    }

    /// Get a cursor to walk graphemes from the logical start of this atom.
    #[inline(always)]
    pub fn graphemes_start(&self) -> Graphemes<'a> {
        self.slice().graphemes_start()
    }

    /// Get a cursor to walk graphemes from the logical end of this atom.
    #[inline(always)]
    pub fn graphemes_end(&self) -> Graphemes<'a> {
        self.slice().graphemes_end()
    }

    /// Whether there is a soft wrap opportunity logically before this atom.
    #[inline(always)]
    pub fn is_soft_wrap_opportunity_before(&self) -> bool {
        self.slice.characters[self.chars.0 as usize]
            .flags
            .is_soft_wrap_opportunity()
    }

    /// Whether the atom can be broken
    #[inline(always)]
    pub fn is_safe_to_break_before(&self) -> bool {
        self.slice.shaped_clusters[self.clusters.0 as usize].is_safe_to_break_before()
    }

    /// The atom's total advance.
    ///
    /// This is the sum of the atom's clusters' advances.
    #[inline(always)]
    pub fn advance(&self) -> f32 {
        self.advance
    }

    /// Get a cursor to walk atoms, starting logically before this atom.
    pub fn cursor_before(&self) -> Atoms<'a> {
        Atoms {
            slice: self.slice,
            cluster_idx: self.clusters.0,
        }
    }

    /// Get a cursor to walk atoms, starting logically after this atom.
    pub fn cursor_after(&self) -> Atoms<'a> {
        Atoms {
            slice: self.slice,
            cluster_idx: self.clusters.1,
        }
    }
}

/// A [`Grapheme`] cursor.
///
/// This cursor can walk forwards and backwards over graphemes.
#[derive(Copy, Clone, Debug)]
pub struct Graphemes<'a> {
    slice: ShapedSlice<'a>,

    /// The gap we're pointing at into [`ShapedSlice::characters`].
    char_idx: u32,

    /// The gap we're pointing at into [`ShapedSlice::shaped_clusters`].
    cluster_idx: u32,

    /// The partial advance of `cluster_idx` shared among the graphemes it overlaps.
    partial_advance: f32,
}

impl<'a> Graphemes<'a> {
    /// Get the grapheme logically preceding the cursor.
    ///
    /// See also [`Self::next`].
    #[inline]
    pub fn prev(&mut self) -> Option<Grapheme> {
        let char_start = self.slice.char_range().start;
        if self.char_idx == char_start {
            return None;
        }
        let grapheme_end = self.char_idx;
        let mut idx = grapheme_end - 1;
        if idx
            < self.slice.shaped_clusters[self.cluster_idx as usize]
                .chars_range()
                .start
        {
            self.cluster_idx -= 1;
            self.partial_advance = self.slice.partial_advance_at(self.cluster_idx);
        }
        let is_atom_end = grapheme_end
            == self.slice.shaped_clusters[self.cluster_idx as usize]
                .chars_range()
                .end;
        let mut advance = self.partial_advance;
        while idx > char_start && !self.slice.characters[idx as usize].grapheme_start {
            idx -= 1;
            if idx
                < self.slice.shaped_clusters[self.cluster_idx as usize]
                    .chars_range()
                    .start
            {
                // The grapheme continues into the preceding cluster.
                self.cluster_idx -= 1;
                self.partial_advance = self.slice.partial_advance_at(self.cluster_idx);
                advance += self.partial_advance;
            }
        }
        let is_atom_start = idx == char_start
            || idx
                == self.slice.shaped_clusters[self.cluster_idx as usize]
                    .chars_range()
                    .start;
        let first_char = self.slice.characters[idx as usize];
        self.char_idx = idx;
        Some(Grapheme {
            chars: (idx, grapheme_end),
            advance,
            flags: GraphemeFlags::new(
                first_char.flags.is_soft_wrap_opportunity(),
                first_char.whitespace,
                is_atom_start,
                is_atom_end,
            ),
        })
    }
}

impl<'a> Iterator for Graphemes<'a> {
    type Item = Grapheme;

    /// Get the grapheme logically following the cursor.
    ///
    /// See also [`Self::prev`].
    fn next(&mut self) -> Option<Grapheme> {
        let char_end = self.slice.char_range().end;
        if self.char_idx == char_end {
            return None;
        }
        let grapheme_start = self.char_idx;
        let is_atom_start = grapheme_start
            == self.slice.shaped_clusters[self.cluster_idx as usize]
                .chars_range()
                .start;

        let mut advance = self.partial_advance;
        let mut idx = grapheme_start + 1;
        while idx < char_end && !self.slice.characters[idx as usize].grapheme_start {
            if idx
                == self.slice.shaped_clusters[self.cluster_idx as usize]
                    .chars_range()
                    .end
            {
                // The grapheme continues into the next cluster.
                self.cluster_idx += 1;
                self.partial_advance = self.slice.partial_advance_at(self.cluster_idx);
                advance += self.partial_advance;
            }
            idx += 1;
        }
        // If we stopped exactly on a cluster edge, the next cluster's share
        // belongs to the *next* grapheme: advance the lockstep without taking it.
        let is_atom_end = idx == char_end
            || idx
                == self.slice.shaped_clusters[self.cluster_idx as usize]
                    .chars_range()
                    .end;
        if idx < char_end
            && idx
                == self.slice.shaped_clusters[self.cluster_idx as usize]
                    .chars_range()
                    .end
        {
            self.cluster_idx += 1;
            self.partial_advance = self.slice.partial_advance_at(self.cluster_idx);
        }
        let first_char = self.slice.characters[grapheme_start as usize];
        self.char_idx = idx;
        Some(Grapheme {
            chars: (grapheme_start, idx),
            advance,
            flags: GraphemeFlags::new(
                first_char.flags.is_soft_wrap_opportunity(),
                first_char.whitespace,
                is_atom_start,
                is_atom_end,
            ),
        })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (
            0,
            Some(self.slice.characters.len() - self.char_idx as usize),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct GraphemeFlags(u16);

impl GraphemeFlags {
    const SOFT_WRAP_OPPORTUNITY: u16 = 1 << 0;
    const WHITESPACE_SHIFT: u16 = 1;
    const WHITESPACE_MASK: u16 = 0b111 << Self::WHITESPACE_SHIFT;
    const ATOM_START: u16 = 1 << 4;
    const ATOM_END: u16 = 1 << 5;

    // TODO: do we want to expose safe to break?
    // const SAFE_TO_BREAK_BEFORE: u16 = 1 << 6;
}

impl GraphemeFlags {
    #[inline(always)]
    fn new(
        is_soft_wrap_opportunity: bool,
        whitespace: Whitespace,
        atom_start: bool,
        atom_end: bool,
    ) -> Self {
        Self(
            if is_soft_wrap_opportunity {
                Self::SOFT_WRAP_OPPORTUNITY
            } else {
                0
            } | ((whitespace as u16) << Self::WHITESPACE_SHIFT)
                | if atom_start { Self::ATOM_START } else { 0 }
                | if atom_end { Self::ATOM_END } else { 0 },
        )
    }

    #[inline(always)]
    fn is_soft_wrap_opportunity_before(self) -> bool {
        self.0 & Self::SOFT_WRAP_OPPORTUNITY != 0
    }

    #[inline(always)]
    fn whitespace(self) -> Whitespace {
        match (self.0 & Self::WHITESPACE_MASK) >> Self::WHITESPACE_SHIFT {
            0 => Whitespace::None,
            1 => Whitespace::Space,
            2 => Whitespace::NoBreakSpace,
            3 => Whitespace::IdeographicSpace,
            4 => Whitespace::OtherSpaceSeparator,
            5 => Whitespace::Tab,
            6 => Whitespace::Newline,
            7 => Whitespace::ControlWhitespace,
            _ => unreachable!("0..8 are the only valid values"),
        }
    }

    #[inline(always)]
    fn is_atom_start(self) -> bool {
        self.0 & Self::ATOM_START != 0
    }

    #[inline(always)]
    fn is_atom_end(self) -> bool {
        self.0 & Self::ATOM_END != 0
    }
}

/// A grapheme of shaped text.
///
/// This encodes extended grapheme clusters as in [UAX #29 § 3][uax-grapheme].
///
/// Graphemes usually are the units of caret movement, selection, and hit testing. A grapheme's
/// edges are not necessarily [`ShapedCluster`] edges:
///
/// - a cluster can span multiple graphemes (e.g. a ligature)
///   - where [`ShapedCluster::advance`] is split evenly over the graphemes it overlaps.
/// - a grapheme can span multiple shaped clusters, due to shaping with `harfrust`'s
///   [monotone characters cluster level][monotone-characters]
///   - where [`Self::advance`] is the sum of shaped cluster advances.
///
/// A combination of these is also possible, e.g., a grapheme containing multiple shaped clusters
/// and crossing a shaped cluster boundary on one or both sides.
///
/// [uax-grapheme]: https://www.unicode.org/reports/tr29/#Grapheme_Cluster_Boundaries
/// [monotone-characters]: https://harfbuzz.github.io/working-with-harfbuzz-clusters.html
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grapheme {
    /// The characters this grapheme spans inside [`ShapedSlice::characters`].
    chars: (u32, u32),

    advance: f32,
    flags: GraphemeFlags,
}

impl Grapheme {
    /// The range of characters into the underlying [`ShapedSlice`] this grapheme spans.
    pub fn char_range(&self) -> Range<u32> {
        self.chars.0..self.chars.1
    }

    /// The advance of the grapheme.
    ///
    /// This is the sum of the grapheme's clusters advances. If clusters cross the boundaries of
    /// this grapheme, this includes partial cluster advances. A cluster's advance is split evenly
    /// over the graphemes it overlaps.
    #[inline(always)]
    pub fn advance(&self) -> f32 {
        self.advance
    }

    /// Whether there is a soft wrap opportunity logically before this grapheme.
    #[inline(always)]
    pub fn is_soft_wrap_opportunity_before(&self) -> bool {
        self.flags.is_soft_wrap_opportunity_before()
    }

    /// The whitespace class of this grapheme's first logical character.
    #[inline(always)]
    pub fn whitespace(&self) -> Whitespace {
        self.flags.whitespace()
    }

    /// Whether this grapheme's logical start is also the logical start of an [`Atom`].
    #[inline(always)]
    pub fn is_atom_start(&self) -> bool {
        self.flags.is_atom_start()
    }

    /// Whether this grapheme's logical end is also the logical end of an [`Atom`].
    #[inline(always)]
    pub fn is_atom_end(&self) -> bool {
        self.flags.is_atom_end()
    }
}

#[cfg(test)]
mod tests {
    use alloc::{sync::Arc, vec};

    use fontique::Synthesis;
    use linebender_resource_handle::{Blob, FontData};

    use crate::{
        Analysis, AnalysisOptions, Analyzer, FontInstance, FontInstanceRef, FontSelector,
        ShapeOptions, ShapedText, Shaper,
        itemize::{Item, Segment},
        shape::CharCluster,
    };

    const ROBOTO: &[u8] =
        include_bytes!("../../../parley_dev/assets/fonts/roboto_fonts/Roboto-Regular.ttf");

    /// A [`FontSelector`] shaping everything with a single font.
    struct SingleFont(FontInstance);

    impl FontSelector for SingleFont {
        fn select_font(
            &mut self,
            _segment: &Segment,
            _options: &ShapeOptions<'_>,
            _cluster: &mut CharCluster,
        ) -> Option<FontInstanceRef<'_>> {
            Some(self.0.as_ref())
        }
    }

    fn analyze(text: &str) -> Analysis {
        let mut analysis = Analysis::new();
        Analyzer::new().analyze(
            text,
            &AnalysisOptions {
                line_break: &[],
                line_break_override: None,
                ..AnalysisOptions::default()
            },
            &mut analysis,
        );
        analysis
    }

    fn font_instance(font_data: &'static [u8]) -> FontInstance {
        FontInstance {
            font: FontData::new(Blob::new(Arc::new(font_data)), 0),
            synthesis: Synthesis::default(),
        }
    }

    fn shape_with_font(text: &str, font_data: &'static [u8]) -> ShapedText {
        let analysis = analyze(text);
        let font = font_instance(font_data);
        let mut shaper = Shaper::default();
        let mut shaped = ShapedText::new();

        let char_style_indices = vec![0; text.chars().count()];
        let items = [Item {
            char_end: text.chars().count().try_into().unwrap(),
            options: ShapeOptions {
                font_size: 32.0,
                language: None,
                features: &[],
                variations: &[],
            },
        }];
        shaper.shape_text(
            text,
            &analysis,
            &char_style_indices,
            items,
            SingleFont(font),
            &mut shaped,
        );
        shaped
    }

    #[test]
    fn select_atom_at() {
        let shaped = shape_with_font("ffi éa\u{0301}", ROBOTO);
        // byte offsets:              0123467     ..9
        // chars:                     0123456     ..7
        // clusters:                  0001233     ..4

        let slice = shaped.run_slice(0);
        assert_eq!(slice.atom_at_char(0).unwrap().shaped_clusters_range(), 0..1);
        assert_eq!(slice.atom_at_char(1).unwrap().shaped_clusters_range(), 0..1);
        assert_eq!(slice.atom_at_char(2).unwrap().shaped_clusters_range(), 0..1);
        assert_eq!(slice.atom_at_char(3).unwrap().shaped_clusters_range(), 1..2);
        assert!(slice.atom_at_char(8).is_none());

        assert_eq!(
            slice.atom_at_text_byte(0).unwrap().shaped_clusters_range(),
            0..1
        );
        assert_eq!(
            slice.atom_at_text_byte(1).unwrap().shaped_clusters_range(),
            0..1
        );
        assert_eq!(
            slice.atom_at_text_byte(2).unwrap().shaped_clusters_range(),
            0..1
        );
        assert_eq!(
            slice.atom_at_text_byte(3).unwrap().shaped_clusters_range(),
            1..2
        );
        assert_eq!(
            slice.atom_at_text_byte(4).unwrap().shaped_clusters_range(),
            2..3
        );
        assert_eq!(
            slice.atom_at_text_byte(5).unwrap().shaped_clusters_range(),
            2..3
        );
        assert_eq!(
            slice.atom_at_text_byte(6).unwrap().shaped_clusters_range(),
            3..4
        );
        assert_eq!(
            slice.atom_at_text_byte(7).unwrap().shaped_clusters_range(),
            3..4
        );
        assert_eq!(
            slice.atom_at_text_byte(8).unwrap().shaped_clusters_range(),
            3..4
        );
        assert!(slice.atom_at_text_byte(9).is_none());
    }

    #[test]
    fn text_byte_positions() {
        let shaped = shape_with_font("ffi éa\u{0301}", ROBOTO);
        // byte offsets:              0123467     ..9
        // chars:                     0123456     ..7

        let slice = shaped.run_slice(0);
        assert_eq!(slice.text_byte_range(0..3), 0..3);
        assert_eq!(slice.text_byte_range(4..5), 4..6);
        assert_eq!(slice.text_byte_range(0..7), 0..9);

        assert_eq!(slice.text_byte_range(3..3), 3..3);
        assert_eq!(slice.text_byte_range(7..7), 9..9);

        assert_eq!(slice.text_byte_at(0), 0);
        assert_eq!(slice.text_byte_at(4), 4);
        assert_eq!(slice.text_byte_at(6), 7);
        assert_eq!(slice.text_byte_at(7), 9);
    }
}
