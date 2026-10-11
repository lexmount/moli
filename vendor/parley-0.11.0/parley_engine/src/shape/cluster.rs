// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#![expect(missing_docs, reason = "Deferred")]
#![expect(missing_debug_implementations, reason = "Deferred")]

use core::cmp::Ordering;

use alloc::vec::Vec;
use icu_normalizer::properties::Decomposed;
use parley_emoji::{EmojiDFA, EmojiPresentationStyle, EmojiSegmentationCategory};

use crate::{CharInfo, analysis::AnalysisDataSources};

#[derive(Debug, Default)]
pub struct CharCluster {
    chars: Vec<Char>,
    map_len: u8,
    start: u32,
    end: u32,
    force_normalize: bool,
    comp: Form,
    decomp: Form,
    emoji_presentation_style: EmojiPresentationStyle,
}

impl CharCluster {
    #[inline]
    pub fn range(&self) -> SourceRange {
        SourceRange {
            start: self.start,
            end: self.end,
        }
    }

    #[inline(always)]
    pub fn chars(&self) -> &[Char] {
        &self.chars
    }

    #[inline(always)]
    pub fn is_emoji(&self) -> bool {
        self.emoji_presentation_style.is_emoji()
    }
}

/// Source range of a cluster in code units.
#[derive(Copy, Clone)]
pub struct SourceRange {
    pub start: u32,
    pub end: u32,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct Char {
    /// The character.
    pub ch: char,
    /// Whether the character
    pub is_control_character: bool,
    /// True if the character should be considered when mapping glyphs.
    pub contributes_to_shaping: bool,
    /// Indexes into the list of styles for the containing text run, to find the style applicable
    /// to this character.
    pub style_index: u16,
}

/// Whitespace class of a character.
///
/// All white space characters as defined
/// [by Unicode](https://www.unicode.org/Public/UCD/latest/ucd/PropList.txt) map to a variant of
/// this enum. The variants are based on the various sets of white space as in
/// [CSS Text 4][css-text-4].
///
/// A character maps to [`Whitespace::None`] if and only if that character's [`char::is_whitespace`]
/// is `false`.
///
/// [css-text-4]: https://www.w3.org/TR/css-text-4/
#[derive(Copy, Clone, PartialOrd, Ord, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Whitespace {
    /// Not white space.
    None = 0,

    /// Standard space.
    Space = 1,

    /// Non-breaking space (U+00A0).
    NoBreakSpace = 2,

    /// Ideographic space (U+3000).
    ///
    /// Note CSS Text 4 doesn't treat this space any differently from [`Self::OtherSpaceSeparator`],
    /// but some applications do.
    IdeographicSpace = 3,

    /// The Unicode "space separators" (category `Zs`), except for the standard space, non-breaking
    /// space and ideographic space.
    ///
    /// Applications may want to treat those three space separators differently from the general
    /// class of space separators.
    OtherSpaceSeparator = 4,

    /// Horizontal tab.
    Tab = 5,

    /// Newline (CR, LF, CRLF, LS, or PS).
    Newline = 6,

    /// Vertical tab, form feed and next line.
    ///
    /// These are Unicode `White_Space`, but CSS Text 4 § 4 says to treat them as visible glyphs.
    ///
    /// Note [`Self::Tab`] and some of [`Self::Newline`] are also in the Unicode `Cc` category.
    /// There are also `Cc` characters that aren't `White_Space` (those would be
    /// [`Whitespace::None`]).
    ControlWhitespace = 7,
    // NOTE: if you grow these variants more than eight, ensure you also update the encoding in
    // `GraphemeFlags`.
}

impl Whitespace {
    #[inline]
    pub(crate) const fn from_char(c: char) -> Self {
        // See https://www.unicode.org/Public/UCD/latest/ucd/PropList.txt.
        match c {
            ' ' => Self::Space,
            '\u{00A0}' => Self::NoBreakSpace,
            '\u{3000}' => Self::IdeographicSpace,
            // Ogham space mark, en quad through hair space, narrow no-break space, medium
            // mathematical space.
            '\u{1680}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' => {
                Self::OtherSpaceSeparator
            }
            '\t' => Self::Tab,
            // Newline, carriage return, line separator, paragraph separator.
            '\n' | '\r' | '\u{2028}' | '\u{2029}' => Self::Newline,
            // Vertical tab, form feed, next line. These are `White_Space` that CSS says to render
            // as control characters.
            '\u{000b}' | '\u{000c}' | '\u{0085}' => Self::ControlWhitespace,
            _ => Self::None,
        }
    }

    /// Returns true for space or no break space.
    #[inline]
    pub fn is_space_or_nbsp(self) -> bool {
        matches!(self, Self::Space | Self::NoBreakSpace)
    }
}

/// A font's coverage of a character cluster.
///
/// This is the fraction `covered / total` of characters covered by a font. Coverages are ordered by
/// the value of the fraction (with `0 / 0` considered to be completely covered). Use, e.g.,
/// [`Self::cmp`] to compare coverages.
///
/// In the unlikely event there are more characters than this in the character cluster, the counts
/// are saturated to [`u8::MAX`].
#[derive(Copy, Clone, Eq, Debug)]
pub struct Coverage {
    /// The number of characters covered by the font.
    pub covered: u8,

    /// The number of characters counting towards coverage.
    pub total: u8,
}

impl Coverage {
    /// Zero coverage.
    ///
    /// Useful, for example, as a starting "best coverage," in case you're searching for a
    /// best-covering font.
    pub const NONE: Self = Self {
        covered: 0,
        total: u8::MAX,
    };

    /// Complete coverage, meaning all characters contributing to shaping are covered.
    ///
    /// A coverage compares equal to `COMPLETE` exactly when it is complete, see also
    /// [`Self::is_complete`].
    pub const COMPLETE: Self = Self {
        covered: u8::MAX,
        total: u8::MAX,
    };

    /// Returns `true` iff all characters are covered.
    #[inline(always)]
    #[must_use]
    pub fn is_complete(self) -> bool {
        self.covered >= self.total
    }
}

impl PartialEq for Coverage {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl PartialOrd for Coverage {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Coverage {
    /// Coverages are ordered by the value of the fraction `covered / total`.
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        // We check `is_complete` explicitly, such that fractions of `x / 0`, which are considered
        // complete, are ordered identically to other complete fractions. Cases other than those
        // with a denominator of 0 would have already been correctly ordered by value through the
        // cross-multiplication in the `(false, false)` arm, but note we now order all complete
        // fractions as `Ordering::Equal`.
        match (self.is_complete(), other.is_complete()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => (u16::from(self.covered) * u16::from(other.total))
                .cmp(&(u16::from(other.covered) * u16::from(self.total))),
        }
    }
}

impl CharCluster {
    #[inline]
    pub(crate) fn clear(&mut self) {
        self.chars.clear();
        self.map_len = 0;
        self.start = 0;
        self.end = 0;
        self.force_normalize = false;
        self.comp.clear();
        self.decomp.clear();
        self.emoji_presentation_style = EmojiPresentationStyle::Default;
    }

    #[inline(always)]
    fn len(&self) -> usize {
        self.chars.len()
    }

    /// Returns the primary style index for the cluster.
    #[inline(always)]
    pub fn style_index(&self) -> u16 {
        self.chars[0].style_index
    }

    #[inline(always)]
    fn contributes_to_shaping(ch: char, analysis_data_sources: &AnalysisDataSources) -> bool {
        let props = analysis_data_sources.properties(ch);
        crate::analysis::contributes_to_shaping(props.general_category(), props.script())
    }

    fn decomposed(&mut self, analysis_data_sources: &AnalysisDataSources) -> Option<&[Char]> {
        match self.decomp.state {
            FormState::Invalid => None,
            FormState::None => {
                self.decomp.state = FormState::Invalid;

                // Only attempt pairwise normalization (1 <-> 2 characters)
                if self.chars.len() != 1 {
                    return None;
                }

                let decomposer = analysis_data_sources.decomposing_normalizer();
                let decomp = decomposer.decompose(self.chars[0].ch);
                match decomp {
                    Decomposed::Default | Decomposed::Singleton(_) => {
                        return None;
                    }
                    Decomposed::Expansion(a, b) => {
                        let mut copy = self.chars[0];
                        copy.ch = a;
                        copy.contributes_to_shaping =
                            Self::contributes_to_shaping(a, analysis_data_sources);
                        self.decomp.chars[0] = copy;

                        copy.ch = b;
                        copy.contributes_to_shaping =
                            Self::contributes_to_shaping(b, analysis_data_sources);
                        self.decomp.chars[1] = copy;

                        self.decomp.len = 2;
                    }
                }

                self.decomp.state = FormState::Valid;
                self.decomp.setup();
                Some(self.decomp.chars())
            }
            FormState::Valid => Some(self.decomp.chars()),
        }
    }

    fn composed(&mut self, analysis_data_sources: &AnalysisDataSources) -> Option<&[Char]> {
        match self.comp.state {
            FormState::Invalid => None,
            FormState::None => {
                self.comp.state = FormState::Invalid;

                // Only attempt pairwise normalization (1 <-> 2 characters)
                if self.chars.len() != 2 {
                    return None;
                }

                let composer = analysis_data_sources.composing_normalizer();
                let ch = composer.compose(self.chars[0].ch, self.chars[1].ch)?;

                let mut copy = self.chars[0];
                copy.ch = ch;
                copy.contributes_to_shaping =
                    Self::contributes_to_shaping(ch, analysis_data_sources);
                self.comp.chars[0] = copy;
                self.comp.len = 1;

                self.comp.state = FormState::Valid;
                self.comp.setup();
                Some(self.comp.chars())
            }
            FormState::Valid => Some(self.comp.chars()),
        }
    }

    /// Calculate a candidate font's coverage of this character cluster.
    ///
    /// The callback `covers` should return whether the character is covered by the font under
    /// consideration. The coverage returned is the best coverage of the character cluster's
    /// characters, or the characters of its normalized forms (see
    /// <https://www.unicode.org/reports/tr15/>).
    ///
    /// Coverages are ordered by how much of the cluster they cover, see [`Coverage`]. Note
    /// [`Coverage::total`] may be different from the count of [`Self::chars`], and may also differ
    /// between subsequent calls due to returning the best coverage of either the original
    /// characters or one of the normalized forms.
    pub fn calculate_coverage(
        &mut self,
        covers: impl Fn(char) -> bool,
        analysis_data_sources: &AnalysisDataSources,
    ) -> Coverage {
        let len = self.len();
        let mut best_coverage = Mapper {
            chars: &self.chars[..len],
            map_len: self.map_len,
        }
        .coverage(&covers);
        if best_coverage.is_complete() {
            return best_coverage;
        }
        if self.force_normalize && self.composed(analysis_data_sources).is_some() {
            let coverage = self.comp.coverage(&covers);
            if coverage > best_coverage {
                best_coverage = coverage;
                if coverage.is_complete() {
                    return best_coverage;
                }
            }
        }
        if self.decomposed(analysis_data_sources).is_some() {
            let coverage = self.decomp.coverage(&covers);
            if coverage > best_coverage {
                best_coverage = coverage;
                if coverage.is_complete() {
                    return best_coverage;
                }
            }
            if !self.force_normalize && self.composed(analysis_data_sources).is_some() {
                let coverage = self.comp.coverage(&covers);
                if coverage > best_coverage {
                    best_coverage = coverage;
                    if coverage.is_complete() {
                        return best_coverage;
                    }
                }
            }
        }

        best_coverage
    }

    /// Rebuilds `self` in-place using the existing allocation for the given grapheme.
    ///
    /// `chars` must yield the grapheme's characters with associated analysis info, and style index.
    ///
    /// `code_unit_offset_in_string` must be the byte offset of the grapheme's first character in the
    /// source string. When this method returns, its value is the byte offset just past the end of
    /// the grapheme in the source string.
    #[expect(clippy::cast_possible_truncation, reason = "Deferred")]
    #[inline]
    pub(crate) fn fill(
        &mut self,
        chars: impl Iterator<Item = (char, CharInfo, u16)>,
        code_unit_offset_in_string: &mut usize,
    ) {
        // Reset cluster but keep allocation
        self.clear();

        let mut force_normalize = false;
        let mut is_emoji = false;
        let mut map_len: u8 = 0;
        let mut emoji_dfa = EmojiDFA::new();
        let mut leading_is_emoji_presentation = false;
        let start = *code_unit_offset_in_string as u32;

        for (i, (ch, info, style_index)) in chars.enumerate() {
            *code_unit_offset_in_string += ch.len_utf8();
            force_normalize |= info.force_normalize();

            is_emoji |= info.is_emoji_or_pictograph();

            let mut is_emoji_presentation_selector = false;

            if is_emoji {
                if i == 0 {
                    leading_is_emoji_presentation = info.is_emoji_presentation();
                }

                let category = EmojiSegmentationCategory::from_codepoint(
                    ch as u32,
                    info.is_emoji(),
                    info.is_emoji_presentation(),
                    info.is_emoji_modifier(),
                    info.is_emoji_modifier_base(),
                    info.is_region_indicator(),
                );

                is_emoji_presentation_selector = category.is_presentation_selector();

                emoji_dfa.step_record(category);
            }

            // Variation selectors don't impact font coverage, and are default ignorable.
            // TODO: Is there a default ignorable flag we should be using instead?
            let contributes_to_shaping =
                info.contributes_to_shaping() && !is_emoji_presentation_selector;
            if contributes_to_shaping {
                map_len = map_len.saturating_add(1);
            }

            self.chars.push(Char {
                ch,
                contributes_to_shaping,
                style_index,
                is_control_character: info.is_control(),
            });
        }

        // Finalize cluster metadata
        let end = *code_unit_offset_in_string as u32;
        self.map_len = map_len;
        self.start = start;
        self.end = end;
        self.force_normalize = force_normalize;

        if is_emoji {
            self.emoji_presentation_style =
                emoji_dfa.presentation_style(leading_is_emoji_presentation);
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum FormState {
    None,
    Valid,
    Invalid,
}

#[derive(Clone, Debug)]
pub(crate) struct Form {
    chars: [Char; 2],
    len: u8,
    map_len: u8,
    state: FormState,
}

impl Default for Form {
    fn default() -> Self {
        Self::new()
    }
}

impl Form {
    fn new() -> Self {
        Self {
            chars: [Char::default(), Char::default()],
            len: 0,
            map_len: 0,
            state: FormState::None,
        }
    }

    fn clear(&mut self) {
        self.chars = [Char::default(), Char::default()];
        self.len = 0;
        self.map_len = 0;
        self.state = FormState::None;
    }

    #[inline(always)]
    fn chars(&self) -> &[Char] {
        &self.chars[..self.len as usize]
    }

    #[inline(always)]
    #[expect(clippy::cast_possible_truncation, reason = "Deferred")]
    fn setup(&mut self) {
        self.map_len = self
            .chars()
            .iter()
            .filter(|c| !c.is_control_character)
            .count() as u8;
    }

    #[inline(always)]
    fn coverage(&self, covers: &impl Fn(char) -> bool) -> Coverage {
        Mapper {
            chars: &self.chars[..self.len as usize],
            map_len: self.map_len,
        }
        .coverage(covers)
    }
}

struct Mapper<'a> {
    chars: &'a [Char],
    map_len: u8,
}

impl<'a> Mapper<'a> {
    /// Returns the coverage of characters contributing to shaping.
    fn coverage(&self, covers: &impl Fn(char) -> bool) -> Coverage {
        let mut mapped: u8 = 0;
        for c in self.chars.iter() {
            if c.contributes_to_shaping && covers(c.ch) {
                mapped = mapped.saturating_add(1);
            }
        }

        // If this assert is hit, some bookkeeping is wrong somewhere, but the returned coverage is
        // then simply considered to be complete.
        debug_assert!(
            mapped <= self.map_len,
            "Counted more covered characters than the number of characters we consider to be contributing."
        );

        Coverage {
            covered: mapped,
            total: self.map_len,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::shape::Whitespace;

    use super::Coverage;

    #[test]
    fn ordering() {
        assert!(Coverage::NONE < Coverage::COMPLETE);

        assert_eq!(
            Coverage {
                covered: 0,
                total: 0,
            },
            Coverage::COMPLETE
        );
        assert_eq!(
            Coverage {
                covered: 5,
                total: 5,
            },
            Coverage::COMPLETE
        );
        assert_eq!(
            Coverage {
                // `covered` is greater than `total`, which shouldn't be produced by us, but is
                // possible because `Coverage`'s fields are `pub`.
                covered: 3,
                total: 2,
            },
            Coverage::COMPLETE
        );
        assert_eq!(
            Coverage {
                covered: 255,
                total: 255,
            },
            Coverage::COMPLETE
        );

        assert!(
            Coverage {
                covered: 4,
                total: 5,
            } > Coverage::NONE
        );
        assert!(
            Coverage {
                covered: 4,
                total: 5,
            } < Coverage::COMPLETE
        );

        assert!(
            Coverage {
                covered: 254,
                total: 255,
            } > Coverage::NONE
        );
        assert!(
            Coverage {
                covered: 254,
                total: 255,
            } < Coverage::COMPLETE
        );
    }

    #[test]
    fn whitespace() {
        // Test all code points from `NUL` up to the last `White_Space` character as defined in
        // https://www.unicode.org/Public/UCD/latest/ucd/PropList.txt.
        for c in '\u{0000}'..='\u{3000}' {
            let ws = Whitespace::from_char(c);
            assert!(
                c.is_whitespace() && ws != Whitespace::None
                    || !c.is_whitespace() && ws == Whitespace::None,
                "`Whitespace` should encode exactly all of Unicode's `White_Space` characters."
            );
        }
    }
}
