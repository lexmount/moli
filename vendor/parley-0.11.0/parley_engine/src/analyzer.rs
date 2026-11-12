// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The analyzer API.

use core::ops::Range;

use parlance::{BaseDirection, BidiLevel, Language, LineBreak, WordBreak};

use crate::{bidi::BidiResolver, break_overrides::LineBreakOverrideFn};

use crate::analysis::{Analysis, analyze_text};

/// Reusable scratch for [`Analyzer::analyze`].
#[derive(Default)]
pub struct Analyzer {
    pub(crate) bidi: BidiResolver,
}

impl core::fmt::Debug for Analyzer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Analyzer").finish_non_exhaustive()
    }
}

/// How an inline object or transparent marker participates in bidi analysis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineBoxBidi {
    /// Resolve a neutral U+FFFC without inserting it into the source text.
    Neutral,
    /// Inherit the preceding source character, object or marker's assigned level.
    ///
    /// Before the first item, inherit the paragraph level. This includes the
    /// level assigned to a preceding `InheritNext` marker at the same anchor.
    InheritPrevious,
    /// Inherit the next source character or neutral object's resolved level.
    ///
    /// Other transparent markers are skipped when looking ahead. At EOF,
    /// inherit the paragraph level. The assigned level becomes the preceding
    /// level for any following `InheritPrevious` marker.
    InheritNext,
}

/// An inline object's source anchor, bidi participation and resolved level.
///
/// The replacement character is not inserted into the source text and does not
/// participate in segmentation or shaping.
#[derive(Clone, Copy, Debug)]
pub struct BidiObject {
    index: usize,
    participation: InlineBoxBidi,
    level: BidiLevel,
}

impl BidiObject {
    /// Creates a neutral object anchored at `index` in the source text.
    ///
    /// See [`Self::index`] for the placement semantics and valid offsets.
    #[inline]
    pub fn new(index: usize) -> Self {
        Self::with_participation(index, InlineBoxBidi::Neutral)
    }

    /// Creates an object or transparent marker with explicit bidi participation.
    ///
    /// The source anchor follows the same rules as [`Self::new`].
    pub fn with_participation(index: usize, participation: InlineBoxBidi) -> Self {
        Self {
            index,
            participation,
            level: BidiLevel::new(0),
        }
    }

    /// Returns how this object participates in bidi analysis.
    pub fn participation(&self) -> InlineBoxBidi {
        self.participation
    }

    /// Source byte offset at which the object participates in bidi analysis.
    ///
    /// Neutral objects insert a virtual U+FFFC; inherited-level markers add no
    /// character to the bidi input.
    ///
    /// An object at offset `i` participates immediately before the source character
    /// beginning at `i`; `text.len()` places it after the final character. The offset
    /// must be a character boundary within the source text.
    #[inline]
    pub fn index(&self) -> usize {
        self.index
    }

    /// The object's resolved bidi level.
    ///
    /// The level is initialized to zero by [`Self::new`] and overwritten by
    /// [`Analyzer::analyze_with_objects`].
    #[inline]
    pub fn level(&self) -> BidiLevel {
        self.level
    }

    #[inline]
    pub(crate) fn set_level(&mut self, level: BidiLevel) {
        self.level = level;
    }
}

impl Analyzer {
    /// Creates a new analyzer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Analyze `text`, overwriting `analysis`.
    ///
    /// This reuses the allocations of `analysis`.
    pub fn analyze(&mut self, text: &str, options: &AnalysisOptions<'_>, analysis: &mut Analysis) {
        analysis.clear();
        analyze_text(self, text, options, &mut [], analysis);
    }

    /// Analyze source text and inline objects together, overwriting their bidi levels.
    ///
    /// Neutral objects participate in the Unicode bidirectional algorithm as U+FFFC,
    /// irrespective of whether they occupy space in the layout. Inherited-level
    /// markers add no characters. Objects at the same index are assigned in slice order.
    /// Source character offsets and segmentation are retained. Virtual characters
    /// are excluded from shaping input, but source character bidi levels may change
    /// and affect later shaping. Allocations in `analysis` are reused.
    ///
    /// # Panics
    ///
    /// Panics if objects are not sorted by index or if an index is not a character
    /// boundary within `text`.
    pub fn analyze_with_objects(
        &mut self,
        text: &str,
        options: &AnalysisOptions<'_>,
        objects: &mut [BidiObject],
        analysis: &mut Analysis,
    ) {
        let mut previous_index = 0;
        for object in objects.iter_mut() {
            assert!(
                text.is_char_boundary(object.index),
                "object index must be a character boundary within text"
            );
            assert!(
                object.index >= previous_index,
                "objects must be sorted by index"
            );
            previous_index = object.index;
            object.level = BidiLevel::new(0);
        }
        analysis.clear();
        analyze_text(self, text, options, objects, analysis);
    }
}

/// Configuration of line break opportunities for a range of text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LineBreakConfig {
    /// CSS `word-break`.
    pub word_break: WordBreak,
    /// CSS `line-break`.
    pub line_break: LineBreak,
    /// The content language.
    ///
    /// Chinese and Japanese content uses tailored line breaking rules, which for example allow
    /// breaks before small kana and some punctuation under [`LineBreak::Normal`] and
    /// [`LineBreak::Loose`].
    pub language: Option<Language>,
}

/// Options controlling [`Analyzer::analyze`].
#[derive(Clone, Copy, Default)]
pub struct AnalysisOptions<'a> {
    /// The paragraph's base direction.
    ///
    /// Defaults to [`BaseDirection::Auto`], which infers the direction from the text.
    pub base_direction: BaseDirection,

    /// Line breaking configuration for ranges of the source text.
    ///
    /// Ranges must be sorted and non-overlapping, and must start and end on character boundaries of
    /// the text. Empty ranges are ignored. Gaps use [`LineBreakConfig::default`].
    pub line_break: &'a [(Range<usize>, LineBreakConfig)],

    /// Ranges of the source text in which a soft wrap opportunity follows every space, tab, and
    /// ideographic space.
    ///
    /// This implements the additional soft wrap opportunities of
    /// [CSS's white-space-collapse: break-spaces][css-break-spaces].
    ///
    /// Outside these ranges, [UAX #14 § 6][uax-14-algorithm] is followed, which defines where soft
    /// wrap opportunities exist. For example, rules LB7 and LB18 give a sequence of spaces an
    /// opportunity only at its end. The ranges specified here add an opportunity after every space,
    /// tab, and ideographic space, except directly before a mandatory break. This overrides the
    /// "non-tailorable" UAX #14 Rule LB7, and so deviates from Unicode's line breaking algorithm.
    ///
    /// Ranges must be sorted and non-overlapping, and must start and end on character boundaries
    /// of the text. Empty ranges are ignored. Gaps apply the default rules.
    ///
    /// [css-break-spaces]: https://www.w3.org/TR/css-text-4/#valdef-white-space-collapse-break-spaces
    /// [uax-14-algorithm]: https://unicode.org/reports/tr14/#Algorithm
    pub break_spaces: &'a [Range<usize>],

    /// The callback which will be called as a first provider of line breaking decisions.
    ///
    /// See [`LineBreakOverrideFn`] for more details.
    pub line_break_override: Option<&'a LineBreakOverrideFn>,
}

impl core::fmt::Debug for AnalysisOptions<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AnalysisOptions")
            .field("base_direction", &self.base_direction)
            .field("line_break", &self.line_break)
            .field("break_spaces", &self.break_spaces)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use parlance::BidiLevel;

    use super::{AnalysisOptions, Analyzer};
    use crate::{Analysis, BaseDirection};

    fn analyze(text: &str, base_direction: BaseDirection) -> Analysis {
        let mut analyzer = Analyzer::new();
        let mut analysis = Analysis::new();
        analyzer.analyze(
            text,
            &AnalysisOptions {
                base_direction,
                ..AnalysisOptions::default()
            },
            &mut analysis,
        );
        analysis
    }

    #[test]
    fn explicit_rtl_resolves_numeric_and_neutral_text() {
        let text = "123 / 456";
        let auto = analyze(text, BaseDirection::Auto);
        let rtl = analyze(text, BaseDirection::Rtl);

        assert_eq!(auto.paragraph_level(), BidiLevel::new(0));
        assert!(auto.paragraph_level().is_ltr());
        assert!(auto.bidi_levels().is_empty());

        assert_eq!(rtl.paragraph_level(), BidiLevel::new(1));
        assert!(rtl.paragraph_level().is_rtl());
        assert_eq!(rtl.bidi_levels().len(), text.chars().count());
        for (ch, level) in text.chars().zip(rtl.bidi_levels()) {
            if ch.is_ascii_digit() {
                assert!(level.is_ltr());
            }
        }
        assert!(rtl.bidi_levels().iter().any(|level| level.is_rtl()));
    }

    #[test]
    fn explicit_ltr_takes_precedence_over_first_strong_direction() {
        let text = "مرحبا hello";
        let auto = analyze(text, BaseDirection::Auto);
        let ltr = analyze(text, BaseDirection::Ltr);

        assert!(auto.paragraph_level().is_rtl());
        assert!(ltr.paragraph_level().is_ltr());
        assert_ne!(auto.bidi_levels(), ltr.bidi_levels());
    }

    #[test]
    fn explicit_rtl_preserves_ltr_run_direction() {
        let analysis = analyze("hello", BaseDirection::Rtl);

        assert!(analysis.paragraph_level().is_rtl());
        assert!(analysis.bidi_levels().iter().all(|level| level.is_ltr()));
    }

    #[test]
    fn explicit_direction_applies_to_empty_text() {
        let analysis = analyze("", BaseDirection::Rtl);

        assert!(analysis.paragraph_level().is_rtl());
        assert!(analysis.bidi_levels().is_empty());
    }
}
