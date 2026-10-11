// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Additional advance applied on top of shaped advance.

use parley_engine::{Atom, Grapheme, ShapedSlice, shape::Whitespace};

/// Whether the whitespace is a word separator.
///
/// This mostly follows [CSS Text 3 § 7.1][css-word-separator], in that fixed-width spaces are not
/// considered word separators (and do not get stretched). Unicode's segmentation does consider such
/// fixed-width spaces to be word boundaries.
///
/// Note that the spec includes, e.g., the Ethiopic word space (U+1361) as a word separator (which
/// generally has a glyph), but we currently do not.
///
/// [css-word-separator]: https://www.w3.org/TR/css-text-3/#word-separator
pub(crate) fn is_word_separator(whitespace: Whitespace) -> bool {
    whitespace.is_space_or_nbsp()
}

/// Additional gaps to apply around a shaped advance.
///
/// This spacing is in visual order.
///
/// [CSS Text 4 § 8][css-spacing] says to apply spacing around word separators and "typographic
/// character units." Such a typographic character unit is an extended grapheme cluster, i.e.,
/// [`parley_engine::Grapheme`]. Word separators form their own graphemes. As ligatures should not
/// be broken up and we cannot tell `HarfRust` we want spacing between graphemes, you probably want
/// to apply spacing at the boundaries of an [`parley_engine::Atom`] (and in case of letter spacing,
/// disable optional ligation when shaping).
///
/// [css-spacing]: https://www.w3.org/TR/css-text-4/#spacing
#[derive(Copy, Clone, Debug)]
pub(crate) struct Gaps {
    /// For horizontal text, additional spacing visually to the left.
    ///
    /// For vertical text, this is to the top.
    ///
    /// Note: this field is wired up, but it's currently always zero as we follow Blink in assigning
    /// word and letter spacing to be visually after (see also docs on [`Spacing`]). However, e.g.,
    /// `text-justify: inter-character` would distribute spacing around graphemes, at which point
    /// `before` becomes used.
    pub(crate) before: f32,

    /// For horizontal text, additional spacing visually to the right.
    ///
    /// For vertical text, this is to the bottom.
    pub(crate) after: f32,
}

impl Gaps {
    pub(crate) const ZERO: Self = Self {
        before: 0.,
        after: 0.,
    };

    /// The total gap.
    #[inline(always)]
    pub(crate) const fn total(self) -> f32 {
        self.before + self.after
    }
}

/// Additional spacing to apply.
///
/// Word and letter spacing are applied visually after atoms. Note Blink and Gecko behave
/// differently: Blink applies spacing visually after, but Gecko applies it logically after. There
/// is [some discussion on whether the CSS specs should improve][css-reconsider-spacing].
///
/// [css-reconsider-spacing]: https://github.com/w3c/csswg-drafts/issues/10193#issue-2234554156
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Spacing {
    /// Additional spacing inserted after every word.
    ///
    /// See [CSS Text 4 § 8.1][css-word-spacing].
    ///
    /// [css-word-spacing]: https://www.w3.org/TR/css-text-4/#word-spacing-property
    pub(crate) word: f32,
    /// Additional spacing inserted after every atom.
    ///
    /// See [CSS Text 4 § 8.2][css-letter-spacing].
    ///
    /// [css-letter-spacing]: https://www.w3.org/TR/css-text-4/#letter-spacing-property
    pub(crate) letter: f32,
}

impl Spacing {
    #[inline(always)]
    pub(crate) const fn new(word: f32, letter: f32) -> Self {
        Self { word, letter }
    }

    #[inline(always)]
    pub(crate) const fn is_zero(self) -> bool {
        self.word == 0. && self.letter == 0.
    }

    /// The gaps around an atom whose first character has the given whitespace class.
    ///
    /// This is [`EffectiveSpacing::gaps`] under [`Justification::NONE`].
    #[inline(always)]
    pub(crate) fn gaps(self, whitespace: Whitespace) -> Gaps {
        if whitespace == Whitespace::Newline {
            return Gaps::ZERO;
        }

        let mut gaps = Gaps {
            before: 0.,
            after: self.letter,
        };

        if is_word_separator(whitespace) {
            gaps.after += self.word;
        }

        gaps
    }
}

/// Justification to apply to a single line.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Justification {
    /// The amount of additional spacing to apply per justification opportunity.
    pub(crate) amount_per_opportunity: f32,

    /// One past the line's logically last shaped cluster eligible for justification. Shaped
    /// clusters at or beyond this index are not justified, as they hang past the line's end, or are
    /// part of the last atom that doesn't fully hang (e.g., an NBSP doesn't hang, but if it closes
    /// the line, it shouldn't get justified).
    pub(crate) justification_end_cluster: u32,
}

impl Justification {
    /// No justification.
    pub(crate) const NONE: Self = Self {
        amount_per_opportunity: 0.,
        justification_end_cluster: u32::MAX,
    };
}

impl Default for Justification {
    fn default() -> Self {
        Self::NONE
    }
}

/// Additional spacing to apply, including justification (if any).
#[derive(Copy, Clone, Debug)]
pub(crate) struct EffectiveSpacing {
    spacing: Spacing,
    justification: Justification,
}

impl EffectiveSpacing {
    /// The combined [`Spacing`] and [`Justification`] effective for a run.
    ///
    /// To measure text for purposes like line breaking, pass [`Justification::NONE`].
    #[inline(always)]
    pub(crate) const fn new(spacing: Spacing, justification: Justification) -> Self {
        Self {
            spacing,
            justification,
        }
    }

    #[inline(always)]
    pub(crate) const fn is_zero(self) -> bool {
        self.spacing.is_zero() && self.justification.amount_per_opportunity == 0.
    }

    /// The gaps around `atom`.
    #[inline(always)]
    pub(crate) fn gaps(self, atom: &Atom<'_>) -> Gaps {
        let whitespace = atom.characters()[0].whitespace;
        let mut gaps = self.spacing.gaps(whitespace);

        if is_word_separator(whitespace)
            && atom.shaped_clusters_range().end <= self.justification.justification_end_cluster
        {
            gaps.after += self.justification.amount_per_opportunity;
        }

        gaps
    }

    /// The total advance of `atom`, i.e., the sum of the shaped advance and gaps.
    #[inline(always)]
    pub(crate) fn atom_advance(self, atom: &Atom<'_>) -> f32 {
        atom.advance() + self.gaps(atom).total()
    }

    /// The total advance of `grapheme`, i.e., the sum of the shaped advance and gaps.
    ///
    /// This only adds the gap that the grapheme owns, i.e., if that grapheme boundary is also an
    /// atom boundary.
    pub(crate) fn grapheme_advance(self, atom: &Atom<'_>, grapheme: Grapheme, is_rtl: bool) -> f32 {
        let gaps = self.gaps(atom);

        // TODO: perhaps spacing should not be applied if the grapheme `continues_before` /
        // `continues_after`. The same then should hold true in `gaps` computed for an atom.
        let (owns_before, owns_after) = if is_rtl {
            (grapheme.is_atom_end(), grapheme.is_atom_start())
        } else {
            (grapheme.is_atom_start(), grapheme.is_atom_end())
        };

        grapheme.advance()
            + if owns_before { gaps.before } else { 0. }
            + if owns_after { gaps.after } else { 0. }
    }

    /// The total advance of `slice`, i.e., the sum of shaped advances and gaps.
    #[inline]
    pub(crate) fn slice_advance(self, slice: ShapedSlice<'_>) -> f32 {
        if self.is_zero() {
            slice
                .shaped_clusters()
                .iter()
                .map(|cluster| cluster.advance)
                .sum()
        } else {
            slice
                .atoms_start()
                .map(|atom| self.atom_advance(&atom))
                .sum()
        }
    }
}
