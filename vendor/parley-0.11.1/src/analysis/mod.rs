// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

pub(crate) mod cluster;

use alloc::vec::Vec;
use core::marker::PhantomData;

use crate::break_overrides::{LineBreakContext, LineBreakOverrideFn};
use crate::resolve::StyleRun;
use crate::{Brush, LayoutContext, WordBreak};

use icu_normalizer::properties::{
    CanonicalComposition, CanonicalCompositionBorrowed, CanonicalDecomposition,
    CanonicalDecompositionBorrowed,
};
use icu_properties::props::{BidiMirroringGlyph, GeneralCategory, GraphemeClusterBreak, Script};
use icu_properties::{
    CodePointMapData, CodePointMapDataBorrowed, PropertyNamesShort, PropertyNamesShortBorrowed,
};
use icu_segmenter::options::{LineBreakOptions, LineBreakWordOption, WordBreakInvariantOptions};
use icu_segmenter::{
    GraphemeClusterSegmenter, GraphemeClusterSegmenterBorrowed, LineSegmenter,
    LineSegmenterBorrowed, WordSegmenter, WordSegmenterBorrowed,
};
use parley_data::Properties;

pub(crate) struct AnalysisDataSources;

impl AnalysisDataSources {
    pub(crate) fn new() -> Self {
        Self
    }

    #[inline(always)]
    pub(crate) fn properties(&self, c: char) -> Properties {
        Properties::get(c)
    }

    #[inline(always)]
    pub(crate) fn grapheme_segmenter(&self) -> GraphemeClusterSegmenterBorrowed<'_> {
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
    fn line_segmenter(&self, word_break_strength: WordBreak) -> LineSegmenterBorrowed<'static> {
        match word_break_strength {
            WordBreak::Normal => {
                let mut opt = LineBreakOptions::default();
                opt.word_option = Some(LineBreakWordOption::Normal);
                line_segmenter_impl(opt)
            }
            WordBreak::BreakAll => {
                let mut opt = LineBreakOptions::default();
                opt.word_option = Some(LineBreakWordOption::BreakAll);
                line_segmenter_impl(opt)
            }
            WordBreak::KeepAll => {
                let mut opt = LineBreakOptions::default();
                opt.word_option = Some(LineBreakWordOption::KeepAll);
                line_segmenter_impl(opt)
            }
        }
    }

    #[inline(always)]
    fn composing_normalizer(&self) -> CanonicalCompositionBorrowed<'_> {
        const { CanonicalComposition::new() }
    }

    #[inline(always)]
    fn decomposing_normalizer(&self) -> CanonicalDecompositionBorrowed<'_> {
        const { CanonicalDecomposition::new() }
    }

    #[inline(always)]
    pub(crate) fn script_short_name(&self) -> PropertyNamesShortBorrowed<'static, Script> {
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

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct CharInfo {
    /// The line/word breaking boundary classification of this character.
    pub boundary: Boundary,
    /// The Unicode script this character belongs to.
    pub script: Script,
    /// The grapheme cluster boundary property of this character.
    pub grapheme_cluster_break: GraphemeClusterBreak,
    /// The impact this character has on directionality.
    pub bidi_class: icu_properties::props::BidiClass,
    /// Whether or not the character is a bracket, plus mirror data if so.
    pub bracket: BidiMirroringGlyph,

    flags: u8,
}

impl CharInfo {
    const VARIATION_SELECTOR_SHIFT: u8 = 0;
    const REGION_INDICATOR_SHIFT: u8 = 1;
    const CONTROL_SHIFT: u8 = 2;
    const EMOJI_OR_PICTOGRAPH_SHIFT: u8 = 3;
    const CONTRIBUTES_TO_SHAPING_SHIFT: u8 = 4;
    const FORCE_NORMALIZE_SHIFT: u8 = 5;

    #[allow(
        dead_code,
        reason = "To be used in more complete emoji checking, in select_font"
    )]
    const VARIATION_SELECTOR_MASK: u8 = 1 << Self::VARIATION_SELECTOR_SHIFT;
    #[allow(
        dead_code,
        reason = "To be used in more complete emoji checking, in select_font"
    )]
    const REGION_INDICATOR_MASK: u8 = 1 << Self::REGION_INDICATOR_SHIFT;
    const CONTROL_MASK: u8 = 1 << Self::CONTROL_SHIFT;
    const EMOJI_OR_PICTOGRAPH_MASK: u8 = 1 << Self::EMOJI_OR_PICTOGRAPH_SHIFT;
    const CONTRIBUTES_TO_SHAPING_MASK: u8 = 1 << Self::CONTRIBUTES_TO_SHAPING_SHIFT;
    const FORCE_NORMALIZE_MASK: u8 = 1 << Self::FORCE_NORMALIZE_SHIFT;

    fn new(
        boundary: Boundary,
        script: Script,
        grapheme_cluster_break: GraphemeClusterBreak,
        bidi_class: icu_properties::props::BidiClass,
        bracket: BidiMirroringGlyph,
        is_variation_selector: bool,
        is_region_indicator: bool,
        is_control: bool,
        is_emoji_or_pictograph: bool,
        contributes_to_shaping: bool,
        force_normalize: bool,
    ) -> Self {
        Self {
            boundary,
            script,
            grapheme_cluster_break,
            bidi_class,
            bracket,
            flags: (is_variation_selector as u8) << Self::VARIATION_SELECTOR_SHIFT
                | (is_region_indicator as u8) << Self::REGION_INDICATOR_SHIFT
                | (is_control as u8) << Self::CONTROL_SHIFT
                | (is_emoji_or_pictograph as u8) << Self::EMOJI_OR_PICTOGRAPH_SHIFT
                | (contributes_to_shaping as u8) << Self::CONTRIBUTES_TO_SHAPING_SHIFT
                | (force_normalize as u8) << Self::FORCE_NORMALIZE_SHIFT,
        }
    }

    #[allow(
        dead_code,
        reason = "To be used in more complete emoji checking, in select_font"
    )]
    #[inline(always)]
    pub(crate) fn is_variation_selector(self) -> bool {
        self.flags & Self::VARIATION_SELECTOR_MASK != 0
    }

    #[allow(
        dead_code,
        reason = "To be used in more complete emoji checking, in select_font"
    )]
    #[inline(always)]
    pub(crate) fn is_region_indicator(self) -> bool {
        self.flags & Self::REGION_INDICATOR_MASK != 0
    }

    #[inline(always)]
    pub(crate) fn is_control(self) -> bool {
        self.flags & Self::CONTROL_MASK != 0
    }

    #[inline(always)]
    pub(crate) fn is_emoji_or_pictograph(self) -> bool {
        self.flags & Self::EMOJI_OR_PICTOGRAPH_MASK != 0
    }

    #[inline(always)]
    pub(crate) fn contributes_to_shaping(self) -> bool {
        self.flags & Self::CONTRIBUTES_TO_SHAPING_MASK != 0
    }

    #[inline(always)]
    pub(crate) fn force_normalize(self) -> bool {
        self.flags & Self::FORCE_NORMALIZE_MASK != 0
    }
}

/// Boundary type of a character or cluster.
#[derive(Copy, Clone, PartialOrd, Ord, PartialEq, Eq, Debug)]
#[repr(u8)]
pub(crate) enum Boundary {
    /// Not a boundary.
    None = 0,
    /// Start of a word.
    Word = 1,
    /// Potential line break.
    Line = 2,
    /// Mandatory line break.
    Mandatory = 3,
}

pub(crate) fn analyze_text<B: Brush>(
    lcx: &mut LayoutContext<B>,
    mut text: &str,
    line_break_override: Option<&LineBreakOverrideFn>,
) {
    struct WordBreakSegmentIter<'a, I: Iterator, B: Brush> {
        text: &'a str,
        style_runs: I,
        lcx: &'a LayoutContext<B>,
        char_indices: core::str::CharIndices<'a>,
        current_char: (usize, char),
        building_range_start: usize,
        previous_word_break_style: WordBreak,
        done: bool,
        _phantom: PhantomData<B>,
    }

    impl<'a, I, B: Brush + 'a> WordBreakSegmentIter<'a, I, B>
    where
        I: Iterator<Item = &'a StyleRun>,
    {
        fn new(
            text: &'a str,
            style_runs: I,
            lcx: &'a LayoutContext<B>,
            first_style_run: &StyleRun,
        ) -> Self {
            let mut char_indices = text.char_indices();
            let current_char_len = char_indices.next().unwrap();
            let first_style = &lcx.style_table[first_style_run.style_index as usize];

            Self {
                text,
                style_runs,
                lcx,
                char_indices,
                current_char: current_char_len,
                building_range_start: first_style_run.range.start,
                previous_word_break_style: first_style.word_break,
                done: false,
                _phantom: PhantomData,
            }
        }
    }

    impl<'a, I, B: Brush + 'a> Iterator for WordBreakSegmentIter<'a, I, B>
    where
        I: Iterator<Item = &'a StyleRun>,
    {
        type Item = (&'a str, WordBreak, bool);

        fn next(&mut self) -> Option<Self::Item> {
            if self.done {
                return None;
            }

            for style_run in self.style_runs.by_ref() {
                // Empty style ranges are disallowed.
                assert!(style_run.range.start < style_run.range.end);

                let style_start_index = style_run.range.start;
                let mut prev_char_index = self.current_char;

                // Find the character at the style boundary
                while self.current_char.0 < style_start_index {
                    prev_char_index = self.current_char;
                    self.current_char = self.char_indices.next().unwrap();
                }

                let current_word_break_style =
                    self.lcx.style_table[style_run.style_index as usize].word_break;
                if self.previous_word_break_style == current_word_break_style {
                    continue;
                }

                // Produce one substring for each different word break style run
                let prev_size = prev_char_index.1.len_utf8();
                let size = self.current_char.1.len_utf8();

                let substring = &self.text[self.building_range_start..style_start_index + size];
                let result_style = self.previous_word_break_style;

                self.building_range_start = style_start_index - prev_size;
                self.previous_word_break_style = current_word_break_style;

                return Some((substring, result_style, false));
            }

            // Final segment
            self.done = true;
            let last_substring = &self.text[self.building_range_start..self.text.len()];
            Some((last_substring, self.previous_word_break_style, true))
        }
    }

    if text.is_empty() {
        text = " ";
    }

    // Line boundaries (word break naming refers to the line boundary determination config).
    //
    // This breaks text into sequences with similar line boundary config (part of style
    // information). If this config is consistent for all text, we use a fast path through this.
    let (first_style_run, rest_runs) = lcx
        .style_runs
        .split_first()
        .expect("analyze_text requires at least one style run");

    let contiguous_word_break_substrings =
        WordBreakSegmentIter::new(text, rest_runs.iter(), lcx, first_style_run);
    let mut global_offset = 0;
    let mut line_boundary_positions: Vec<usize> = Vec::new();
    for (substring_index, (substring, word_break_strength, last)) in
        contiguous_word_break_substrings.enumerate()
    {
        // Fast path for text with a single word-break option.
        if substring_index == 0 && last {
            let mut lb_iter = lcx
                .analysis_data_sources
                .line_segmenter(word_break_strength)
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

        let line_boundaries_iter = lcx
            .analysis_data_sources
            .line_segmenter(word_break_strength)
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
    let mut wb_iter = lcx
        .analysis_data_sources
        .word_segmenter()
        .segment_str(text)
        .peekable();

    // Merge boundaries - line takes precedence over word
    let mut lb_iter = line_boundary_positions.iter().peekable();
    let mut prev_char = None;
    let mut prev_prev_char = None;
    let boundary_iter = text.char_indices().map(|(byte_pos, ch)| {
        // advance any stale word boundary positions
        while let Some(&w) = wb_iter.peek() {
            if w < byte_pos {
                _ = wb_iter.next();
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
        if let Some(&w) = wb_iter.peek() {
            if w == byte_pos {
                is_word = true;
                _ = wb_iter.next();
            }
        }
        let mut is_line = false;
        if let Some(&l) = lb_iter.peek() {
            if *l == byte_pos {
                is_line = true;
                _ = lb_iter.next();
            }
        }

        // This leaves word boundaries intact. Consumers can only impact line boundaries.
        if let (Some(prev), Some(lb_override)) = (prev_char, line_break_override) {
            let forced = lb_override(LineBreakContext {
                before_before: prev_prev_char,
                before: prev,
                after: ch,
            });
            if let Some(forced) = forced {
                is_line = forced;
            }
        }
        prev_prev_char = prev_char;
        prev_char = Some(ch);

        let boundary = if is_line {
            Boundary::Line
        } else if is_word {
            Boundary::Word
        } else {
            Boundary::None
        };

        (boundary, ch)
    });

    let properties = |c| lcx.analysis_data_sources.properties(c);

    let mut needs_bidi_resolution = false;

    lcx.info.reserve(text.len());
    boundary_iter
        // Shift line break data forward one, as line boundaries corresponding with line-breaking
        // characters (like '\n') exist at an index position one higher than the respective
        // character's index, but we need our iterators to align, and the rest are simply
        // character-indexed.
        .fold(false, |is_mandatory_linebreak, (boundary, ch)| {
            let properties = properties(ch);
            let script = properties.script();
            let grapheme_cluster_break = properties.grapheme_cluster_break();
            let bidi_class = properties.bidi_class();
            let general_category = properties.general_category();
            let is_emoji_or_pictograph = properties.is_emoji_or_pictograph();
            let is_variation_selector = properties.is_variation_selector();
            let is_region_indicator = properties.is_region_indicator();
            let next_mandatory_linebreak = properties.is_mandatory_linebreak();

            let boundary = if is_mandatory_linebreak {
                Boundary::Mandatory
            } else {
                boundary
            };

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

            needs_bidi_resolution |= crate::bidi::needs_bidi_resolution(bidi_class);
            // TODO: maybe extend Properties to u64 to fit BidiMirroringGlyph
            let bracket = lcx.analysis_data_sources.brackets().get(ch);

            lcx.info.push((
                CharInfo::new(
                    boundary,
                    script,
                    grapheme_cluster_break,
                    bidi_class,
                    bracket,
                    is_variation_selector,
                    is_region_indicator,
                    general_category == GeneralCategory::Control,
                    is_emoji_or_pictograph,
                    contributes_to_shaping(general_category, script),
                    force_normalize,
                ),
                0, // Style index is populated later
            ));

            next_mandatory_linebreak
        });

    if needs_bidi_resolution {
        lcx.bidi.resolve(
            text.chars().zip(
                lcx.info
                    .iter()
                    .map(|info| (info.0.bidi_class, info.0.bracket)),
            ),
            None,
        );
    }
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
