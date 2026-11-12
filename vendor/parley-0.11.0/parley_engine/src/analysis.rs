// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Text analysis.
//!
//! Analysis is performed prior to shaping and is independent of fonts, turning a `&str` into
//! [`Analysis`].

use alloc::vec::Vec;
use core::ops::Range;

use icu_locale_core::{LanguageIdentifier, langid};
use icu_normalizer::properties::{
    CanonicalComposition, CanonicalCompositionBorrowed, CanonicalDecomposition,
    CanonicalDecompositionBorrowed,
};
use icu_properties::props::{BidiMirroringGlyph, GeneralCategory, GraphemeClusterBreak, Script};
use icu_properties::{
    CodePointMapData, CodePointMapDataBorrowed, PropertyNamesShort, PropertyNamesShortBorrowed,
};
use icu_segmenter::options::{
    LineBreakOptions, LineBreakStrictness, LineBreakWordOption, WordBreakInvariantOptions,
};
use icu_segmenter::{
    GraphemeClusterSegmenter, GraphemeClusterSegmenterBorrowed, LineSegmenter,
    LineSegmenterBorrowed, WordSegmenter, WordSegmenterBorrowed,
};
use parlance::{BaseDirection, BidiLevel, LineBreak, WordBreak};
use parley_data::Properties;

use crate::bidi;
use crate::break_overrides::LineBreakContext;
use crate::{AnalysisOptions, Analyzer, BidiObject, InlineBoxBidi, LineBreakConfig};

/// The result of [`Analyzer::analyze`].
#[derive(Debug, Default)]
pub struct Analysis {
    /// Info for each character.
    pub(crate) info: Vec<CharInfo>,

    /// Bidi level for each character, parallel to `info`.
    ///
    /// Empty if the text is all LTR.
    pub(crate) levels: Vec<BidiLevel>,

    /// The base bidi level of the paragraph of text.
    pub(crate) paragraph_level: BidiLevel,
}

impl Analysis {
    /// Create a reusable [`Analysis`].
    ///
    /// Pass this to [`Analyzer::analyze`].
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear the result while retaining capacity.
    pub(crate) fn clear(&mut self) {
        self.info.clear();
        self.levels.clear();
        self.paragraph_level = BidiLevel::new(0);
    }

    /// The per-character info in source order.
    #[inline(always)]
    pub fn char_info(&self) -> &[CharInfo] {
        &self.info
    }

    /// The bidi level for each character, parallel to [`Self::char_info`].
    ///
    /// Empty when the whole paragraph is left-to-right.
    #[inline(always)]
    pub fn bidi_levels(&self) -> &[BidiLevel] {
        &self.levels
    }

    /// The base bidi level of the paragraph of text.
    #[inline(always)]
    pub fn paragraph_level(&self) -> BidiLevel {
        self.paragraph_level
    }
}

// TODO: Make `pub(crate)` once `parley_engine` owns shaping.
#[doc(hidden)]
#[expect(missing_debug_implementations, reason = "Will become private")]
pub struct AnalysisDataSources;

impl AnalysisDataSources {
    #[expect(clippy::new_without_default, reason = "Will become private")]
    pub fn new() -> Self {
        Self
    }

    #[inline(always)]
    pub fn properties(&self, c: char) -> Properties {
        Properties::get(c)
    }

    #[inline(always)]
    pub fn grapheme_segmenter(&self) -> GraphemeClusterSegmenterBorrowed<'_> {
        const { GraphemeClusterSegmenter::new() }
    }

    #[inline(always)]
    fn word_segmenter(&self) -> WordSegmenterBorrowed<'static> {
        #[cfg(feature = "complex-scripts")]
        {
            WordSegmenter::new_dictionary(WordBreakInvariantOptions::default())
        }
        #[cfg(not(feature = "complex-scripts"))]
        {
            const { WordSegmenter::new_for_non_complex_scripts(WordBreakInvariantOptions::default()) }
        }
    }

    #[inline(always)]
    fn line_segmenter(&self, key: SegmenterKey) -> LineSegmenterBorrowed<'static> {
        // We use Japanese arbitrarily; see `SegmenterKey::ja_zh`.
        static JA: LanguageIdentifier = langid!("ja");

        let mut opt = LineBreakOptions::default();
        opt.word_option = Some(key.word_option);
        opt.strictness = Some(key.strictness);
        opt.content_locale = key.ja_zh.then_some(&JA);
        line_segmenter_impl(opt)
    }

    #[inline(always)]
    pub fn composing_normalizer(&self) -> CanonicalCompositionBorrowed<'_> {
        const { CanonicalComposition::new() }
    }

    #[inline(always)]
    pub fn decomposing_normalizer(&self) -> CanonicalDecompositionBorrowed<'_> {
        const { CanonicalDecomposition::new() }
    }

    #[inline(always)]
    pub fn script_short_name(&self) -> PropertyNamesShortBorrowed<'static, Script> {
        PropertyNamesShort::new()
    }

    #[inline(always)]
    fn brackets(&self) -> CodePointMapDataBorrowed<'_, BidiMirroringGlyph> {
        const { CodePointMapData::new() }
    }
}

#[cfg(feature = "complex-scripts")]
#[inline(always)]
fn line_segmenter_impl(opt: LineBreakOptions<'_>) -> LineSegmenterBorrowed<'static> {
    LineSegmenter::new_dictionary(opt)
}

#[cfg(not(feature = "complex-scripts"))]
#[inline(always)]
fn line_segmenter_impl(opt: LineBreakOptions<'_>) -> LineSegmenterBorrowed<'static> {
    LineSegmenter::new_for_non_complex_scripts(opt)
}

/// The line segmenter configuration for a [`LineBreakConfig`].
///
/// Configurations which result in the same segmentation map to the same key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SegmenterKey {
    word_option: LineBreakWordOption,
    strictness: LineBreakStrictness,
    /// See [`LineBreakConfig::language`]; the only languages which impact segmentation are
    /// Chinese and Japanese. These impact it in the same way, and only under
    /// [`LineBreak::Normal`] and [`LineBreak::Loose`], so we only track whether one of them
    /// applies.
    ja_zh: bool,
}

impl SegmenterKey {
    const DEFAULT: Self = Self {
        word_option: LineBreakWordOption::Normal,
        strictness: LineBreakStrictness::Strict,
        ja_zh: false,
    };

    fn new(config: &LineBreakConfig) -> Self {
        let strictness = match config.line_break {
            LineBreak::Strict => LineBreakStrictness::Strict,
            LineBreak::Normal => LineBreakStrictness::Normal,
            LineBreak::Loose => LineBreakStrictness::Loose,
            // We add opportunities per grapheme ourselves.
            LineBreak::Anywhere => return Self::DEFAULT,
        };
        let word_option = match config.word_break {
            WordBreak::Normal => LineBreakWordOption::Normal,
            WordBreak::BreakAll => LineBreakWordOption::BreakAll,
            WordBreak::KeepAll => LineBreakWordOption::KeepAll,
        };
        let ja_zh = match strictness {
            LineBreakStrictness::Normal | LineBreakStrictness::Loose => config
                .language
                .is_some_and(|language| matches!(language.language(), "ja" | "zh")),
            _ => false,
        };
        Self {
            word_option,
            strictness,
            ja_zh,
        }
    }
}

/// Per-character analysis info.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CharInfo {
    /// The Unicode script this character belongs to.
    pub script: Script,
    flags: u16,
}

impl CharInfo {
    const VARIATION_SELECTOR_SHIFT: u16 = 0;
    const REGION_INDICATOR_SHIFT: u16 = 1;
    const CONTROL_SHIFT: u16 = 2;
    const EMOJI_OR_PICTOGRAPH_SHIFT: u16 = 3;
    const CONTRIBUTES_TO_SHAPING_SHIFT: u16 = 4;
    const FORCE_NORMALIZE_SHIFT: u16 = 5;
    const GRAPHEME_START_SHIFT: u16 = 6;
    const WORD_BOUNDARY_SHIFT: u8 = 7;
    const SOFT_WRAP_OPPORTUNITY_SHIFT: u16 = 8;
    const EMOJI_SHIFT: u16 = 9;
    const EMOJI_PRESENTATION_SHIFT: u16 = 10;
    const EMOJI_MODIFIER_SHIFT: u16 = 11;
    const EMOJI_MODIFIER_BASE_SHIFT: u16 = 12;

    const VARIATION_SELECTOR_MASK: u16 = 1 << Self::VARIATION_SELECTOR_SHIFT;
    const REGION_INDICATOR_MASK: u16 = 1 << Self::REGION_INDICATOR_SHIFT;
    const CONTROL_MASK: u16 = 1 << Self::CONTROL_SHIFT;
    const EMOJI_OR_PICTOGRAPH_MASK: u16 = 1 << Self::EMOJI_OR_PICTOGRAPH_SHIFT;
    const CONTRIBUTES_TO_SHAPING_MASK: u16 = 1 << Self::CONTRIBUTES_TO_SHAPING_SHIFT;
    const FORCE_NORMALIZE_MASK: u16 = 1 << Self::FORCE_NORMALIZE_SHIFT;
    const GRAPHEME_START_MASK: u16 = 1 << Self::GRAPHEME_START_SHIFT;
    const WORD_BOUNDARY_MASK: u16 = 1 << Self::WORD_BOUNDARY_SHIFT;
    const SOFT_WRAP_OPPORTUNITY_MASK: u16 = 1 << Self::SOFT_WRAP_OPPORTUNITY_SHIFT;
    const EMOJI_MASK: u16 = 1 << Self::EMOJI_SHIFT;
    const EMOJI_PRESENTATION_MASK: u16 = 1 << Self::EMOJI_PRESENTATION_SHIFT;
    const EMOJI_MODIFIER_MASK: u16 = 1 << Self::EMOJI_MODIFIER_SHIFT;
    const EMOJI_MODIFIER_BASE_MASK: u16 = 1 << Self::EMOJI_MODIFIER_BASE_SHIFT;

    fn new(
        script: Script,
        is_variation_selector: bool,
        is_region_indicator: bool,
        is_control: bool,
        is_emoji_or_pictograph: bool,
        contributes_to_shaping: bool,
        force_normalize: bool,
        is_grapheme_start: bool,
        is_word_boundary: bool,
        is_soft_wrap_opportunity: bool,
        is_emoji: bool,
        is_emoji_presentation: bool,
        is_emoji_modifier: bool,
        is_emoji_modifier_base: bool,
    ) -> Self {
        Self {
            script,
            flags: (is_variation_selector as u16) << Self::VARIATION_SELECTOR_SHIFT
                | (is_region_indicator as u16) << Self::REGION_INDICATOR_SHIFT
                | (is_control as u16) << Self::CONTROL_SHIFT
                | (is_emoji_or_pictograph as u16) << Self::EMOJI_OR_PICTOGRAPH_SHIFT
                | (contributes_to_shaping as u16) << Self::CONTRIBUTES_TO_SHAPING_SHIFT
                | (force_normalize as u16) << Self::FORCE_NORMALIZE_SHIFT
                | (is_grapheme_start as u16) << Self::GRAPHEME_START_SHIFT
                | (is_word_boundary as u16) << Self::WORD_BOUNDARY_SHIFT
                | (is_soft_wrap_opportunity as u16) << Self::SOFT_WRAP_OPPORTUNITY_SHIFT
                | (is_emoji as u16) << Self::EMOJI_SHIFT
                | (is_emoji_presentation as u16) << Self::EMOJI_PRESENTATION_SHIFT
                | (is_emoji_modifier as u16) << Self::EMOJI_MODIFIER_SHIFT
                | (is_emoji_modifier_base as u16) << Self::EMOJI_MODIFIER_BASE_SHIFT,
        }
    }

    /// Whether this character is a variation selector.
    #[inline(always)]
    pub fn is_variation_selector(self) -> bool {
        self.flags & Self::VARIATION_SELECTOR_MASK != 0
    }

    /// Whether this character is a regional indicator symbol.
    #[inline(always)]
    pub fn is_region_indicator(self) -> bool {
        self.flags & Self::REGION_INDICATOR_MASK != 0
    }

    /// Whether this character is a control character.
    #[inline(always)]
    pub fn is_control(self) -> bool {
        self.flags & Self::CONTROL_MASK != 0
    }

    /// Returns whether the character has either of the `Emoji` or`Extended_Pictographic` properties.
    ///
    /// Parley Engine uses this to determine whether a character might be part of an Emoji, and so whether to do
    /// more stringent checks.
    ///
    /// [UTS #51]: https://www.unicode.org/reports/tr51/tr51-29.html#Emoji_Properties
    #[inline(always)]
    pub fn is_emoji_or_pictograph(self) -> bool {
        self.flags & Self::EMOJI_OR_PICTOGRAPH_MASK != 0
    }

    /// Whether this character has the `Emoji` property ([UTS #51][]).
    ///
    /// [UTS #51]: https://www.unicode.org/reports/tr51/tr51-29.html#Emoji_Properties
    #[inline(always)]
    pub fn is_emoji(self) -> bool {
        self.flags & Self::EMOJI_MASK != 0
    }

    /// Whether this character has the `Emoji_Presentation` property ([UTS #51][]), i.e. it is
    /// displayed as an emoji by default.
    ///
    /// [UTS #51]: https://www.unicode.org/reports/tr51/tr51-29.html#Emoji_Properties
    #[inline(always)]
    pub fn is_emoji_presentation(self) -> bool {
        self.flags & Self::EMOJI_PRESENTATION_MASK != 0
    }

    /// Whether this character has the `Emoji_Modifier` property ([UTS #51][]), i.e. it is a
    /// skin tone modifier.
    ///
    /// [UTS #51]: https://www.unicode.org/reports/tr51/tr51-29.html#Emoji_Properties
    #[inline(always)]
    pub fn is_emoji_modifier(self) -> bool {
        self.flags & Self::EMOJI_MODIFIER_MASK != 0
    }

    /// Whether this character has the `Emoji_Modifier_Base` property ([UTS #51][]), i.e. it can
    /// be followed by a skin tone modifier.
    ///
    /// [UTS #51]: https://www.unicode.org/reports/tr51/tr51-29.html#Emoji_Properties
    #[inline(always)]
    pub fn is_emoji_modifier_base(self) -> bool {
        self.flags & Self::EMOJI_MODIFIER_BASE_MASK != 0
    }

    /// Whether this character contributes glyphs to shaping (`false` for control characters and
    /// most format characters).
    #[inline(always)]
    pub fn contributes_to_shaping(self) -> bool {
        self.flags & Self::CONTRIBUTES_TO_SHAPING_MASK != 0
    }

    /// Whether this character should be normalized before glyph mapping during shaping.
    #[inline(always)]
    pub fn force_normalize(self) -> bool {
        self.flags & Self::FORCE_NORMALIZE_MASK != 0
    }

    /// Whether this character begins a grapheme cluster ([UAX #29 § 3][graphemes]).
    ///
    /// [graphemes]: https://www.unicode.org/reports/tr29/#Grapheme_Cluster_Boundaries
    #[inline(always)]
    pub fn is_grapheme_start(self) -> bool {
        self.flags & Self::GRAPHEME_START_MASK != 0
    }

    /// Whether there is a word boundary before this character ([UAX #29 § 4][words]).
    ///
    /// [words]: https://www.unicode.org/reports/tr29/#Word_Boundaries
    #[inline(always)]
    pub fn is_word_boundary(self) -> bool {
        self.flags & Self::WORD_BOUNDARY_MASK != 0
    }

    /// Whether there is a soft wrap opportunity before this character ([UAX #14][line-breaking]).
    ///
    /// Note mandatory breaks (like `\n`) are encoded as [`Whitespace`](crate::shape::Whitespace).
    ///
    /// [line-breaking]: https://www.unicode.org/reports/tr14/
    #[inline(always)]
    pub fn is_soft_wrap_opportunity(self) -> bool {
        self.flags & Self::SOFT_WRAP_OPPORTUNITY_MASK != 0
    }
}

pub(crate) fn analyze_text(
    analyzer: &mut Analyzer,
    text: &str,
    options: &AnalysisOptions<'_>,
    objects: &mut [BidiObject],
    analysis: &mut Analysis,
) {
    /// Turns the sparse, sorted, non-overlapping `options.line_break` into a contiguous sequence of
    /// `(range, segmenter key)` segments covering all of `text`.
    ///
    /// Any region not covered by an override takes the default configuration.
    struct DenseLineBreaks<'a> {
        line_break: &'a [(Range<usize>, LineBreakConfig)],
        /// Index of the next word break to emit.
        next: usize,
        /// Start of the next segment to emit.
        cursor: usize,
        text_len: usize,
    }

    impl<'a> DenseLineBreaks<'a> {
        fn new(line_break: &'a [(Range<usize>, LineBreakConfig)], text_len: usize) -> Self {
            Self {
                line_break,
                next: 0,
                cursor: 0,
                text_len,
            }
        }
    }

    impl Iterator for DenseLineBreaks<'_> {
        type Item = (Range<usize>, SegmenterKey);

        fn next(&mut self) -> Option<Self::Item> {
            if self.cursor >= self.text_len {
                return None;
            }

            // Ignore empty ranges.
            while self
                .line_break
                .get(self.next)
                .is_some_and(|(range, _)| range.is_empty())
            {
                self.next += 1;
            }

            match self.line_break.get(self.next) {
                // A gap before the next override: fill it with the default up to its start.
                Some((range, _)) if self.cursor < range.start => {
                    let segment = self.cursor..range.start;
                    self.cursor = range.start;
                    Some((segment, SegmenterKey::DEFAULT))
                }
                // At the next override: emit it.
                Some((range, config)) => {
                    self.cursor = range.end;
                    self.next += 1;
                    Some((range.start..range.end, SegmenterKey::new(config)))
                }
                // No overrides remain: fill the default to the end.
                None => {
                    let segment = self.cursor..self.text_len;
                    self.cursor = self.text_len;
                    Some((segment, SegmenterKey::DEFAULT))
                }
            }
        }
    }

    struct LineBreakSegmentIter<'a, I: Iterator> {
        text: &'a str,
        segments: I,
        char_indices: core::str::CharIndices<'a>,
        current_char: (usize, char),
        building_range_start: usize,
        previous_key: SegmenterKey,
        done: bool,
    }

    impl<'a, I> LineBreakSegmentIter<'a, I>
    where
        I: Iterator<Item = (Range<usize>, SegmenterKey)>,
    {
        fn new(text: &'a str, segments: I, first_segment: (Range<usize>, SegmenterKey)) -> Self {
            let mut char_indices = text.char_indices();
            let current_char_len = char_indices.next().unwrap();

            Self {
                text,
                segments,
                char_indices,
                current_char: current_char_len,
                building_range_start: first_segment.0.start,
                previous_key: first_segment.1,
                done: false,
            }
        }
    }

    impl<'a, I> Iterator for LineBreakSegmentIter<'a, I>
    where
        I: Iterator<Item = (Range<usize>, SegmenterKey)>,
    {
        type Item = (&'a str, SegmenterKey, bool);

        fn next(&mut self) -> Option<Self::Item> {
            if self.done {
                return None;
            }

            for (range, key) in self.segments.by_ref() {
                assert!(range.start < range.end, "Segments must not be empty");

                let style_start_index = range.start;
                let mut prev_char_index = self.current_char;

                // Find the character at the style boundary.
                while self.current_char.0 < style_start_index {
                    prev_char_index = self.current_char;
                    self.current_char = self.char_indices.next().unwrap();
                }

                let current_key = key;
                if self.previous_key == current_key {
                    continue;
                }

                // Produce one substring for each different segmenter key run
                let prev_size = prev_char_index.1.len_utf8();
                let size = self.current_char.1.len_utf8();

                let substring = &self.text[self.building_range_start..style_start_index + size];
                let result_style = self.previous_key;

                self.building_range_start = style_start_index - prev_size;
                self.previous_key = current_key;

                return Some((substring, result_style, false));
            }

            // Final segment
            self.done = true;
            let last_substring = &self.text[self.building_range_start..self.text.len()];
            Some((last_substring, self.previous_key, true))
        }
    }

    if text.is_empty() {
        resolve_bidi(
            analyzer,
            text,
            options.base_direction,
            objects,
            analysis,
            &AnalysisDataSources::new(),
        );
        return;
    }

    // Line boundaries (word break naming refers to the line boundary determination config).
    //
    // This breaks text into sequences with similar line boundary config (part of style
    // information). If this config is consistent for all text, we use a fast path through this.
    let mut segments = DenseLineBreaks::new(options.line_break, text.len());
    // `text` is non-empty (checked above), so there is always at least one segment.
    let first_segment = segments.next().unwrap();
    let contiguous_substrings = LineBreakSegmentIter::new(text, segments, first_segment);

    let mut global_offset = 0;
    let mut line_boundary_positions: Vec<usize> = Vec::new();

    let data_sources = AnalysisDataSources::new();

    for (substring_index, (substring, segmenter_key, last)) in contiguous_substrings.enumerate() {
        // Fast path for text with a single segmenter configuration.
        if substring_index == 0 && last {
            let mut lb_iter = data_sources
                .line_segmenter(segmenter_key)
                .segment_str(substring);

            let _first = lb_iter.next();
            let second = lb_iter.next();
            if second.is_none() {
                continue;
            }
            let third = lb_iter.next();
            if third.is_none() {
                continue;
            }

            let iter = [second.unwrap(), third.unwrap()].into_iter().chain(lb_iter);

            line_boundary_positions.extend(iter);
            // Remove the unnecessary boundary at the end added by ICU4X.
            line_boundary_positions.pop();
            break;
        }

        let line_boundaries_iter = data_sources
            .line_segmenter(segmenter_key)
            .segment_str(substring);

        let mut substring_chars = substring.chars();
        if substring_index != 0 {
            global_offset -= substring_chars.next().unwrap().len_utf8();
        }
        // There will always be at least two characters if we are not taking the fast path for
        // a single word break style substring.
        let last_len = substring_chars.next_back().unwrap().len_utf8();

        // Mark line boundaries (overriding word boundaries where present).
        for (index, pos) in line_boundaries_iter.enumerate() {
            // icu adds leading and trailing line boundaries, which we don't use.
            if index == 0 || pos == substring.len() {
                continue;
            }

            // For all but the last substring, we ignore line boundaries caused by the last
            // character, as this character is carried back from the next substring, and will be
            // accounted for there.
            if !last && pos == substring.len() - last_len {
                continue;
            }
            line_boundary_positions.push(pos + global_offset);
        }

        if !last {
            global_offset += substring.len() - last_len;
        }
    }

    // Collect boundary byte positions compactly
    let mut wb_iter = data_sources.word_segmenter().segment_str(text).peekable();
    let mut gb_iter = data_sources
        .grapheme_segmenter()
        .segment_str(text)
        .peekable();

    // Merge boundaries - line takes precedence over word
    let mut lb_iter = line_boundary_positions.iter().peekable();
    let mut prev_char = None;
    let mut prev_prev_char = None;
    // Whether the preceding character is a mandatory break.
    let mut prev_is_mandatory_linebreak = false;
    // Whether the preceding character is a break-space, i.e. a space, tab or ideographic space in
    // a `break_spaces` range.
    let mut prev_is_break_space = false;
    let mut break_spaces_iter = options.break_spaces.iter().peekable();
    let mut break_anywhere_iter = options
        .line_break
        .iter()
        .filter(|(_, config)| config.line_break == LineBreak::Anywhere)
        .map(|(range, _)| range)
        .peekable();
    let boundary_iter = text.char_indices().map(|(byte_pos, ch)| {
        // advance any stale word boundary positions
        while let Some(&w) = wb_iter.peek() {
            if w < byte_pos {
                _ = wb_iter.next();
            } else {
                break;
            }
        }
        // advance any stale grapheme boundary positions
        while let Some(&g) = gb_iter.peek() {
            if g < byte_pos {
                _ = gb_iter.next();
            } else {
                break;
            }
        }
        // advance any stale line boundary positions
        while let Some(&l) = lb_iter.peek() {
            if *l < byte_pos {
                _ = lb_iter.next();
            } else {
                break;
            }
        }

        let mut is_word = false;
        if let Some(&w) = wb_iter.peek()
            && w == byte_pos
        {
            is_word = true;
            _ = wb_iter.next();
        }
        let mut is_grapheme_start = false;
        if let Some(&g) = gb_iter.peek()
            && g == byte_pos
        {
            is_grapheme_start = true;
            _ = gb_iter.next();
        }
        let properties = data_sources.properties(ch);

        // For characters requiring a dictionary for deciding word boundaries, apply
        // UAX #29 Rule WB999 in case there's no dictionary (because ICU4X does not).
        if !is_word
            && is_grapheme_start
            && properties.needs_dictionary_word_break()
            && !has_word_dictionary(properties.script())
        {
            is_word = true;
        }

        let is_icu_line = if let Some(&l) = lb_iter.peek()
            && *l == byte_pos
        {
            _ = lb_iter.next();
            true
        } else {
            false
        };

        // CSS Text 4 § 4.3.1 `break-spaces`: a soft wrap opportunity exists after each preserved
        // space or tab (but never directly before a mandatory break, UAX #14 LB6).
        while break_spaces_iter
            .peek()
            .is_some_and(|range| range.end <= byte_pos)
        {
            _ = break_spaces_iter.next();
        }
        let after_break_space = prev_is_break_space;
        prev_is_break_space = break_spaces_iter
            .peek()
            .is_some_and(|range| range.contains(&byte_pos))
            && matches!(ch, ' ' | '\t' | '\u{3000}');

        while break_anywhere_iter
            .peek()
            .is_some_and(|range| range.end <= byte_pos)
        {
            _ = break_anywhere_iter.next();
        }
        let is_break_anywhere = break_anywhere_iter
            .peek()
            .is_some_and(|range| range.contains(&byte_pos));

        let is_mandatory_linebreak = properties.is_mandatory_linebreak();

        // We never have a soft wrap opportunity before a mandatory break (following UAX #14 LB6),
        // and we never have one right after a mandatory break: the mandatory break is handled
        // separately.
        let is_soft_wrap_opportunity = if is_mandatory_linebreak || prev_is_mandatory_linebreak {
            false
        } else {
            let mut opportunity = if is_break_anywhere {
                // `line-break: anywhere`: an opportunity around every grapheme.
                prev_char.is_some() && is_grapheme_start
            } else {
                is_icu_line || after_break_space
            };
            // Consumers can override soft wrap opportunities, except around a mandatory
            // break.
            if let (Some(prev), Some(lb_override)) = (prev_char, options.line_break_override) {
                let forced = lb_override(LineBreakContext {
                    before_before: prev_prev_char,
                    before: prev,
                    after: ch,
                });
                if let Some(forced) = forced {
                    opportunity = forced;
                }
            }
            opportunity
        };

        prev_prev_char = prev_char;
        prev_char = Some(ch);
        prev_is_mandatory_linebreak = is_mandatory_linebreak;

        (
            is_soft_wrap_opportunity,
            is_word,
            is_grapheme_start,
            ch,
            properties,
        )
    });

    let mut needs_bidi_resolution = false;

    analysis.info.reserve(text.len());
    for (is_soft_wrap_opportunity, is_word, is_grapheme_start, ch, properties) in boundary_iter {
        let script = properties.script();
        let grapheme_cluster_break = properties.grapheme_cluster_break();
        let bidi_class = properties.bidi_class();
        let general_category = properties.general_category();
        let is_emoji_or_pictograph = properties.is_emoji_or_pictograph();
        let is_variation_selector = properties.is_variation_selector();
        let is_region_indicator = properties.is_region_indicator();

        let force_normalize = {
            // "Extend" break chars should be normalized first, with two exceptions
            if matches!(grapheme_cluster_break, GraphemeClusterBreak::Extend) &&
                    ch as u32 != 0x200C && // Is not a Zero Width Non-Joiner &&
                    !is_variation_selector
            {
                true
            } else {
                // All spacing mark break chars should be normalized first.
                matches!(grapheme_cluster_break, GraphemeClusterBreak::SpacingMark)
            }
        };

        needs_bidi_resolution |= bidi::needs_bidi_resolution(bidi_class);

        analysis.info.push(CharInfo::new(
            script,
            is_variation_selector,
            is_region_indicator,
            general_category == GeneralCategory::Control,
            is_emoji_or_pictograph,
            contributes_to_shaping(general_category, script),
            force_normalize,
            is_grapheme_start,
            is_word,
            is_soft_wrap_opportunity,
            properties.is_emoji(),
            properties.is_emoji_presentation(),
            properties.is_emoji_modifier(),
            properties.is_emoji_modifier_base(),
        ));
    }

    if needs_bidi_resolution || options.base_direction == BaseDirection::Rtl {
        resolve_bidi(
            analyzer,
            text,
            options.base_direction,
            objects,
            analysis,
            &data_sources,
        );
    }
}

// Object anchors are UTF-8 byte offsets into the source text. Resolver levels
// follow the merged character sequence. Projection retains only levels for
// source characters in Analysis and writes object levels back to BidiObjects.
fn resolve_bidi(
    analyzer: &mut Analyzer,
    text: &str,
    direction: BaseDirection,
    objects: &mut [BidiObject],
    analysis: &mut Analysis,
    data: &AnalysisDataSources,
) {
    let properties = |ch| {
        let bidi_class = data.properties(ch).bidi_class();
        // TODO: maybe extend Properties to u64 to fit BidiMirroringGlyph
        (ch, (bidi_class, data.brackets().get(ch)))
    };
    if objects.is_empty() {
        analyzer
            .bidi
            .resolve(text.chars().map(properties), direction);
        core::mem::swap(&mut analysis.levels, &mut analyzer.bidi.levels);
        analysis.paragraph_level = analyzer.bidi.base_level();
        return;
    }

    let mut characters = text.char_indices().peekable();
    let mut anchors = objects.iter().peekable();
    // Merge objects into the bidi input without allocating a modified source string.
    let input = core::iter::from_fn(|| {
        loop {
            if anchors.peek().is_some_and(|object| {
                characters
                    .peek()
                    .is_none_or(|(byte, _)| object.index() <= *byte)
            }) {
                let object = anchors.next()?;
                if object.participation() == InlineBoxBidi::Neutral {
                    return Some('\u{fffc}');
                }
            } else {
                return characters.next().map(|(_, ch)| ch);
            }
        }
    });
    analyzer.bidi.resolve(input.map(properties), direction);
    analysis.paragraph_level = analyzer.bidi.base_level();

    // Project the merged levels back onto source characters and objects separately.
    analysis.levels.reserve(analysis.info.len());
    let mut levels = analyzer.bidi.levels().iter().copied().peekable();
    let mut objects = objects.iter_mut().peekable();
    let paragraph_level = analysis.paragraph_level;
    let mut previous = paragraph_level;
    // Transparent markers consume no resolver level. Every assigned item,
    // including a look-ahead marker, becomes the preceding assigned level.
    let assign = |object: &mut BidiObject,
                  previous: &mut BidiLevel,
                  levels: &mut core::iter::Peekable<
        core::iter::Copied<core::slice::Iter<'_, BidiLevel>>,
    >| {
        let level = match object.participation() {
            InlineBoxBidi::Neutral => levels.next().unwrap(),
            InlineBoxBidi::InheritPrevious => *previous,
            InlineBoxBidi::InheritNext => levels.peek().copied().unwrap_or(paragraph_level),
        };
        object.set_level(level);
        *previous = level;
    };
    for (byte, _) in text.char_indices() {
        while objects.peek().is_some_and(|object| object.index() == byte) {
            assign(objects.next().unwrap(), &mut previous, &mut levels);
        }
        previous = levels.next().unwrap();
        analysis.levels.push(previous);
    }
    for object in objects {
        assign(object, &mut previous, &mut levels);
    }
    debug_assert_eq!(levels.len(), 0, "all bidi input levels must be consumed");
}

/// All characters contribute to shaping except:
/// - Control characters
/// - Format characters, unless they use the "Inherited" script
#[inline(always)]
pub(crate) fn contributes_to_shaping(general_category: GeneralCategory, script: Script) -> bool {
    if matches!(
        general_category,
        GeneralCategory::Control
            | GeneralCategory::LineSeparator
            | GeneralCategory::ParagraphSeparator
    ) {
        return false;
    }

    !(general_category == GeneralCategory::Format && script != Script::Inherited)
}

/// Whether ICU4X has a word-segmentation dictionary for this script.
///
/// If it does not, we need to apply UAX #29 Rule WB999 manually, because ICU4X does not.
///
/// This is approximate, because ICU4X actually decides based on a table of code point ranges: e.g.,
/// some Han characters do not get a dictionary check. That's only a problem if there's a run of
/// repeated such characters, which is probably not too likely to be a problem in practice.
///
/// If ICU4X does implement WB999 in the future, we can stop doing it manually (and this method can
/// be removed).
#[inline(always)]
fn has_word_dictionary(script: Script) -> bool {
    cfg!(feature = "complex-scripts")
        && matches!(
            script,
            Script::Han
                | Script::Hiragana
                | Script::Thai
                | Script::Lao
                | Script::Khmer
                | Script::Myanmar
        )
}

#[cfg(test)]
mod test {
    use alloc::vec::Vec;
    use icu_segmenter::WordSegmenter;
    use icu_segmenter::options::WordBreakInvariantOptions;

    /// For some characters, determining word boundaries requires a dictionary lookup.
    ///
    /// Without dictionaries, ICU4X reports such runs as being a single word. But note
    /// UAX #29 Rule WB999 says that, without a dictionary, there should be word boundaries.
    ///
    /// We do that patchup ourselves. Once this test starts failing, it's likely ICU4X now follows
    /// that rule, and we can drop the patchup (the code calling [`super::has_word_dictionary`]).
    /// Once that's the case, [`parley_data::Properties::needs_dictionary_word_break`] can also be
    /// dropped.
    #[test]
    fn icu4x_word_segmentation_patchup_required() {
        let segmenter =
            WordSegmenter::new_for_non_complex_scripts(WordBreakInvariantOptions::default());
        let breaks: Vec<_> = segmenter.segment_str("中文文本测试").collect();
        assert_eq!(breaks, &[0, 18]);
    }
}
