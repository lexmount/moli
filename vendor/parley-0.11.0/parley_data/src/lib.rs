// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `parley_data` packages the Unicode data that Parley's text analysis and shaping pipeline needs at runtime.
//! It exposes a locale-invariant `CompositeProps` data backed by compact `PackTab` lookup tables, allowing the engine to obtain all required character properties with a single lookup.

#![no_std]

use icu_collections::codepointtrie::TrieValue;
use icu_properties::props::{BidiClass, GeneralCategory, GraphemeClusterBreak, Script};

/// Baked data (`PackTab` tables).
#[cfg(feature = "baked")]
pub mod generated;

/// Unicode character properties relevant for text analysis.
#[derive(Copy, Clone, Debug)]
pub struct Properties(u32);

impl Properties {
    const SCRIPT_BITS: u32 = 8;
    const GC_BITS: u32 = 5;
    const GCB_BITS: u32 = 5;
    const BIDI_BITS: u32 = 5;
    const IS_EMOJI_OR_PICTOGRAPH_BITS: u32 = 1;
    const IS_VARIATION_SELECTOR_BITS: u32 = 1;
    const IS_REGION_INDICATOR_BITS: u32 = 1;
    const IS_MANDATORY_LINE_BREAK_BITS: u32 = 1;
    const NEEDS_DICTIONARY_WORD_BREAK_BITS: u32 = 1;
    const IS_EMOJI_BITS: u32 = 1;
    const IS_EMOJI_PRESENTATION_BITS: u32 = 1;
    const IS_EMOJI_MODIFIER_BITS: u32 = 1;
    const IS_EMOJI_MODIFIER_BASE_BITS: u32 = 1;

    const SCRIPT_SHIFT: u32 = 0;
    const GC_SHIFT: u32 = Self::SCRIPT_SHIFT + Self::SCRIPT_BITS;
    const GCB_SHIFT: u32 = Self::GC_SHIFT + Self::GC_BITS;
    const BIDI_SHIFT: u32 = Self::GCB_SHIFT + Self::GCB_BITS;
    const IS_EMOJI_OR_PICTOGRAPH_SHIFT: u32 = Self::BIDI_SHIFT + Self::BIDI_BITS;
    const IS_VARIATION_SELECTOR_SHIFT: u32 =
        Self::IS_EMOJI_OR_PICTOGRAPH_SHIFT + Self::IS_EMOJI_OR_PICTOGRAPH_BITS;
    const IS_REGION_INDICATOR_SHIFT: u32 =
        Self::IS_VARIATION_SELECTOR_SHIFT + Self::IS_VARIATION_SELECTOR_BITS;
    const IS_MANDATORY_LINE_BREAK_SHIFT: u32 =
        Self::IS_REGION_INDICATOR_SHIFT + Self::IS_REGION_INDICATOR_BITS;
    const NEEDS_DICTIONARY_WORD_BREAK_SHIFT: u32 =
        Self::IS_MANDATORY_LINE_BREAK_SHIFT + Self::IS_MANDATORY_LINE_BREAK_BITS;
    const IS_EMOJI_SHIFT: u32 =
        Self::NEEDS_DICTIONARY_WORD_BREAK_SHIFT + Self::NEEDS_DICTIONARY_WORD_BREAK_BITS;
    const IS_EMOJI_PRESENTATION_SHIFT: u32 = Self::IS_EMOJI_SHIFT + Self::IS_EMOJI_BITS;
    const IS_EMOJI_MODIFIER_SHIFT: u32 =
        Self::IS_EMOJI_PRESENTATION_SHIFT + Self::IS_EMOJI_PRESENTATION_BITS;
    const IS_EMOJI_MODIFIER_BASE_SHIFT: u32 =
        Self::IS_EMOJI_MODIFIER_SHIFT + Self::IS_EMOJI_MODIFIER_BITS;

    #[cfg(feature = "baked")]
    #[inline(always)]
    /// Returns the properties for a given character.
    pub fn get(ch: char) -> Self {
        Self(generated::composite_get(ch as u32))
    }

    /// Creates a new [`Properties`] from the given properties
    pub fn new(
        script: Script,
        gc: GeneralCategory,
        gcb: GraphemeClusterBreak,
        bidi: BidiClass,
        is_emoji_or_pictographic: bool,
        is_variation_selector: bool,
        is_region_indicator: bool,
        is_mandatory_linebreak: bool,
        needs_dictionary_word_break: bool,
        is_emoji: bool,
        is_emoji_presentation: bool,
        is_emoji_modifier: bool,
        is_emoji_modifier_base: bool,
    ) -> Self {
        // The TrieValue implementation is guaranteed to be stable (strongly enough for our purposes, at least).
        // See https://github.com/unicode-org/icu4x/issues/6067#issuecomment-5361908792 and
        // https://github.com/linebender/parley/pull/845#discussion_r4102175970
        let s = script.to_u32();
        let gc = gc as u32;
        let gcb = gcb.to_u32();
        let bidi = bidi.to_u32();

        Self(
            (s << Self::SCRIPT_SHIFT)
                | (gc << Self::GC_SHIFT)
                | (gcb << Self::GCB_SHIFT)
                | (bidi << Self::BIDI_SHIFT)
                | ((is_emoji_or_pictographic as u32) << Self::IS_EMOJI_OR_PICTOGRAPH_SHIFT)
                | ((is_variation_selector as u32) << Self::IS_VARIATION_SELECTOR_SHIFT)
                | ((is_region_indicator as u32) << Self::IS_REGION_INDICATOR_SHIFT)
                | ((is_mandatory_linebreak as u32) << Self::IS_MANDATORY_LINE_BREAK_SHIFT)
                | ((needs_dictionary_word_break as u32) << Self::NEEDS_DICTIONARY_WORD_BREAK_SHIFT)
                | ((is_emoji as u32) << Self::IS_EMOJI_SHIFT)
                | ((is_emoji_presentation as u32) << Self::IS_EMOJI_PRESENTATION_SHIFT)
                | ((is_emoji_modifier as u32) << Self::IS_EMOJI_MODIFIER_SHIFT)
                | ((is_emoji_modifier_base as u32) << Self::IS_EMOJI_MODIFIER_BASE_SHIFT),
        )
    }

    #[inline(always)]
    fn bits(&self, shift: u32, bits: u32) -> u32 {
        (self.0 >> shift) & ((1 << bits) - 1)
    }

    /// Returns the script for the character.
    #[inline(always)]
    pub fn script(&self) -> Script {
        Script::try_from_u32(self.bits(Self::SCRIPT_SHIFT, Self::SCRIPT_BITS)).unwrap_or_default()
    }

    /// Returns the general category for the character.
    #[inline(always)]
    pub fn general_category(&self) -> GeneralCategory {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "general category data only occupies GC_BITS bits."
        )]
        GeneralCategory::try_from(self.bits(Self::GC_SHIFT, Self::GC_BITS) as u8).unwrap()
    }

    /// Returns the grapheme cluster break for the character.
    #[inline(always)]
    pub fn grapheme_cluster_break(&self) -> GraphemeClusterBreak {
        GraphemeClusterBreak::try_from_u32(self.bits(Self::GCB_SHIFT, Self::GCB_BITS))
            .unwrap_or_default()
    }

    /// Returns the bidirectional class for the character.
    #[inline(always)]
    pub fn bidi_class(&self) -> BidiClass {
        BidiClass::try_from_u32(self.bits(Self::BIDI_SHIFT, Self::BIDI_BITS)).unwrap_or_default()
    }

    /// Returns whether the character has either of the `Emoji` or`Extended_Pictographic` properties ([UTS #51][]).
    ///
    /// [UTS #51]: https://www.unicode.org/reports/tr51/tr51-29.html#Emoji_Properties
    #[inline(always)]
    pub fn is_emoji_or_pictograph(&self) -> bool {
        self.bits(
            Self::IS_EMOJI_OR_PICTOGRAPH_SHIFT,
            Self::IS_EMOJI_OR_PICTOGRAPH_BITS,
        ) != 0
    }

    /// Returns whether the character is a variation selector.
    #[inline(always)]
    pub fn is_variation_selector(&self) -> bool {
        self.bits(
            Self::IS_VARIATION_SELECTOR_SHIFT,
            Self::IS_VARIATION_SELECTOR_BITS,
        ) != 0
    }

    /// Returns whether the character is a region indicator.
    #[inline(always)]
    pub fn is_region_indicator(&self) -> bool {
        self.bits(
            Self::IS_REGION_INDICATOR_SHIFT,
            Self::IS_REGION_INDICATOR_BITS,
        ) != 0
    }

    /// Returns whether the character is a mandatory linebreak.
    #[inline(always)]
    pub fn is_mandatory_linebreak(&self) -> bool {
        self.bits(
            Self::IS_MANDATORY_LINE_BREAK_SHIFT,
            Self::IS_MANDATORY_LINE_BREAK_BITS,
        ) != 0
    }

    /// Returns whether the ICU4X word segmenter defers this character to a dictionary to determine
    /// the word break boundaries.
    ///
    /// Without the dictionary, ICU4X reports a run of such characters as a single word.
    #[inline(always)]
    pub fn needs_dictionary_word_break(&self) -> bool {
        self.bits(
            Self::NEEDS_DICTIONARY_WORD_BREAK_SHIFT,
            Self::NEEDS_DICTIONARY_WORD_BREAK_BITS,
        ) != 0
    }

    /// Returns whether the character has the `Emoji` property ([UTS #51][]).
    ///
    /// [UTS #51]: https://www.unicode.org/reports/tr51/tr51-29.html#Emoji_Properties
    #[inline(always)]
    pub fn is_emoji(&self) -> bool {
        self.bits(Self::IS_EMOJI_SHIFT, Self::IS_EMOJI_BITS) != 0
    }

    /// Returns whether the character has the `Emoji_Presentation` property ([UTS #51][]), i.e.
    /// it is displayed as an emoji by default.
    ///
    /// [UTS #51]: https://www.unicode.org/reports/tr51/tr51-29.html#Emoji_Properties
    #[inline(always)]
    pub fn is_emoji_presentation(&self) -> bool {
        self.bits(
            Self::IS_EMOJI_PRESENTATION_SHIFT,
            Self::IS_EMOJI_PRESENTATION_BITS,
        ) != 0
    }

    /// Returns whether the character has the `Emoji_Modifier` property ([UTS #51][]), i.e. it is
    /// a skin tone modifier.
    ///
    /// [UTS #51]: https://www.unicode.org/reports/tr51/tr51-29.html#Emoji_Properties
    #[inline(always)]
    pub fn is_emoji_modifier(&self) -> bool {
        self.bits(Self::IS_EMOJI_MODIFIER_SHIFT, Self::IS_EMOJI_MODIFIER_BITS) != 0
    }

    /// Returns whether the character has the `Emoji_Modifier_Base` property ([UTS #51][]), i.e.
    /// it can be followed by a skin tone modifier.
    ///
    /// [UTS #51]: https://www.unicode.org/reports/tr51/tr51-29.html#Emoji_Properties
    #[inline(always)]
    pub fn is_emoji_modifier_base(&self) -> bool {
        self.bits(
            Self::IS_EMOJI_MODIFIER_BASE_SHIFT,
            Self::IS_EMOJI_MODIFIER_BASE_BITS,
        ) != 0
    }
}

impl From<Properties> for u32 {
    fn from(value: Properties) -> Self {
        value.0
    }
}

#[cfg(all(test, feature = "baked"))]
mod tests {
    use super::Properties;
    use icu_properties::props::{
        BidiClass, Emoji, EmojiModifier, EmojiModifierBase, EmojiPresentation,
        ExtendedPictographic, GeneralCategory, GraphemeClusterBreak, LineBreak, RegionalIndicator,
        Script, VariationSelector,
    };
    use icu_properties::{CodePointMapData, CodePointSetData};

    #[test]
    fn properties_match_icu4x() {
        // Asserts that every character's properties match ICU4X's canonical data.
        for cp in 0_u32..=0x10FFFF {
            let Some(ch) = char::from_u32(cp) else {
                continue;
            };
            let actual = Properties::get(ch);
            let expected = UnclampedProperties::from_icu4x(cp);
            assert_eq!(
                UnclampedProperties::unpack(actual),
                expected,
                "roundtrip mismatch at U+{cp:04X}"
            );
            let packed = expected.pack();
            assert_eq!(
                u32::from(actual),
                u32::from(packed),
                "packed mismatch at U+{cp:04X}: actual={actual:?}, expected={packed:?}"
            );
        }
    }

    #[derive(Debug, PartialEq)]
    /// [`Properties`] but not bitpacked for tests.
    struct UnclampedProperties {
        script: Script,
        general_category: GeneralCategory,
        grapheme_cluster_break: GraphemeClusterBreak,
        bidi_class: BidiClass,
        is_emoji_or_pictograph: bool,
        is_variation_selector: bool,
        is_region_indicator: bool,
        is_mandatory_linebreak: bool,
        needs_dictionary_word_break: bool,
        is_emoji: bool,
        is_emoji_presentation: bool,
        is_emoji_modifier: bool,
        is_emoji_modifier_base: bool,
    }

    impl UnclampedProperties {
        fn from_icu4x(cp: u32) -> Self {
            Self {
                script: CodePointMapData::<Script>::new().get32(cp),
                general_category: CodePointMapData::<GeneralCategory>::new().get32(cp),
                grapheme_cluster_break: CodePointMapData::<GraphemeClusterBreak>::new().get32(cp),
                bidi_class: CodePointMapData::<BidiClass>::new().get32(cp),
                is_emoji_or_pictograph: CodePointSetData::new::<Emoji>().contains32(cp)
                    || CodePointSetData::new::<ExtendedPictographic>().contains32(cp),
                is_variation_selector: CodePointSetData::new::<VariationSelector>().contains32(cp),
                is_region_indicator: CodePointSetData::new::<RegionalIndicator>().contains32(cp),
                is_mandatory_linebreak: matches!(
                    CodePointMapData::<LineBreak>::new().get32(cp),
                    LineBreak::MandatoryBreak
                        | LineBreak::CarriageReturn
                        | LineBreak::LineFeed
                        | LineBreak::NextLine
                ),
                needs_dictionary_word_break: CodePointMapData::<LineBreak>::new().get32(cp)
                    == LineBreak::ComplexContext
                    || matches!(
                        CodePointMapData::<Script>::new().get32(cp),
                        Script::Han | Script::Hiragana
                    ),
                is_emoji: CodePointSetData::new::<Emoji>().contains32(cp),
                is_emoji_presentation: CodePointSetData::new::<EmojiPresentation>().contains32(cp),
                is_emoji_modifier: CodePointSetData::new::<EmojiModifier>().contains32(cp),
                is_emoji_modifier_base: CodePointSetData::new::<EmojiModifierBase>().contains32(cp),
            }
        }

        /// Unpacks `properties` using its accessors.
        fn unpack(properties: Properties) -> Self {
            Self {
                script: properties.script(),
                general_category: properties.general_category(),
                grapheme_cluster_break: properties.grapheme_cluster_break(),
                bidi_class: properties.bidi_class(),
                is_emoji_or_pictograph: properties.is_emoji_or_pictograph(),
                is_variation_selector: properties.is_variation_selector(),
                is_region_indicator: properties.is_region_indicator(),
                is_mandatory_linebreak: properties.is_mandatory_linebreak(),
                needs_dictionary_word_break: properties.needs_dictionary_word_break(),
                is_emoji: properties.is_emoji(),
                is_emoji_presentation: properties.is_emoji_presentation(),
                is_emoji_modifier: properties.is_emoji_modifier(),
                is_emoji_modifier_base: properties.is_emoji_modifier_base(),
            }
        }

        /// Packs these properties using [`Properties::new`].
        fn pack(&self) -> Properties {
            Properties::new(
                self.script,
                self.general_category,
                self.grapheme_cluster_break,
                self.bidi_class,
                self.is_emoji_or_pictograph,
                self.is_variation_selector,
                self.is_region_indicator,
                self.is_mandatory_linebreak,
                self.needs_dictionary_word_break,
                self.is_emoji,
                self.is_emoji_presentation,
                self.is_emoji_modifier,
                self.is_emoji_modifier_base,
            )
        }
    }
}
