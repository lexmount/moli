// Copyright 2026 Christian Hansen and the Parley Authors
// SPDX-License-Identifier: MIT

// Adapted from <https://github.com/chansen/c-emoji>

use super::types::{EmojiPresentationStyle, EmojiSegmentationCategory, EmojiSequence, EmojiState};

const N_STATES: usize = 13;
const N_CATEGORIES: usize = 14;

/// The transition table for Emoji DFA.
///
/// <https://unicode.org/reports/tr51/#Definitions>
static DFA_TRANS: [[u8; N_CATEGORIES]; N_STATES] = {
    use EmojiSegmentationCategory as Category;
    use EmojiState as State;

    let mut t = [[State::Reject as u8; N_CATEGORIES]; N_STATES];

    /// Adds a state transition to the DFA transition table.
    macro_rules! add {
        ($state:expr, $category:expr, $next_state:expr) => {
            t[$state as usize][$category as usize] = $next_state as u8
        };
    }

    // Text and Emoji presentation sequences
    {
        add!(State::Start, Category::Emoji, State::Emoji);

        add!(State::Start, Category::EmojiPresentation, State::Emoji);

        // Text presentation sequence
        //
        // <https://unicode.org/reports/tr51/#def_text_presentation_sequence>
        add!(State::Emoji, Category::Vs15, State::Terminal);

        // Emoji presentation sequence
        //
        // <https://unicode.org/reports/tr51/#def_emoji_presentation_sequence>
        add!(State::Emoji, Category::Vs16, State::OptionalZwj);

        // ZWJ
        add!(State::Emoji, Category::Zwj, State::Zwj);
    }

    // Emoji modifier sequence
    //
    // <https://unicode.org/reports/tr51/#def_emoji_modifier_sequence>
    {
        add!(
            State::Start,
            Category::EmojiModifierBase,
            State::EmojiModifierBase
        );

        add!(State::EmojiModifierBase, Category::Vs15, State::Terminal);
        // TODO: Handle: https://www.unicode.org/reports/tr51/tr51-29.html#def_text_presentation_sequence:~:text=However%2C%20some%20older,should%20be%20ignored.
        add!(State::EmojiModifierBase, Category::Vs16, State::OptionalZwj);
        add!(State::EmojiModifierBase, Category::Zwj, State::Zwj);
        add!(
            State::EmojiModifierBase,
            Category::EmojiModifier,
            State::OptionalZwj
        );

        // other
        add!(State::Start, Category::EmojiModifier, State::Terminal);
    }

    // Emoji flag sequence -- A sequence of two Regional Indicator characters.
    //
    // <https://unicode.org/reports/tr51/#def_emoji_flag_sequence>
    {
        add!(
            State::Start,
            Category::RegionalIndicator,
            State::RegionalIndicator
        );

        add!(
            State::RegionalIndicator,
            Category::RegionalIndicator,
            State::Terminal
        );
    }

    // Emoji tag sequence (ETS).
    //
    // <https://unicode.org/reports/tr51/#def_emoji_tag_sequence>
    {
        add!(State::Start, Category::TagBase, State::TagBase);

        add!(State::TagBase, Category::Vs15, State::Terminal);
        add!(State::TagBase, Category::Vs16, State::OptionalZwj);
        add!(State::TagBase, Category::TagSpec, State::TagSpec);
        add!(State::TagBase, Category::TagEnd, State::TagEmpty); // without any `TagSpec`
        add!(State::TagBase, Category::Zwj, State::Zwj);

        // (seq)+
        add!(State::TagSpec, Category::TagSpec, State::TagSpec);
        add!(State::TagSpec, Category::TagEnd, State::Terminal);
    }

    // Emoji keycap sequence.
    //
    // <https://unicode.org/reports/tr51/#def_emoji_keycap_sequence>
    {
        add!(State::Start, Category::KeycapBase, State::KeycapBase);

        add!(State::KeycapBase, Category::KeycapEnd, State::Terminal);
        add!(State::KeycapBase, Category::Vs15, State::KeycapVs);
        add!(State::KeycapBase, Category::Vs16, State::KeycapVs);

        add!(State::KeycapVs, Category::KeycapEnd, State::Terminal);
    }

    // Emoji ZWJ sequence.
    //
    // <https://unicode.org/reports/tr51/#def_emoji_zwj_sequence>
    {
        add!(State::OptionalZwj, Category::Zwj, State::Zwj);

        // (zwj emoji_zwj_element)+
        add!(State::Zwj, Category::Emoji, State::Emoji);
        add!(State::Zwj, Category::EmojiPresentation, State::Emoji);
        add!(
            State::Zwj,
            Category::EmojiModifierBase,
            State::EmojiModifierBase
        );
    }

    t
};

/// Recognizes Unicode emoji sequence.
#[derive(Clone, Copy, Debug)]
pub struct EmojiDFA {
    /// The current emoji state.
    state: EmojiState,
    /// Stores the visited state and category.
    recorded: (u16, u16),
}

impl EmojiDFA {
    /// A default value for [`EmojiDFA`].
    const DEFAULT: Self = Self {
        state: EmojiState::Start,
        recorded: (0, 0),
    };

    /// Creates a new [`EmojiDFA`].
    #[inline]
    pub const fn new() -> Self {
        Self::DEFAULT
    }

    /// Updates the emoji state with [`EmojiSegmentationCategory`].
    #[inline]
    const fn step(state: EmojiState, category: EmojiSegmentationCategory) -> EmojiState {
        EmojiState::from_u8(DFA_TRANS[state as usize][category as usize])
    }

    /// Records the emoji state and segmentation category with [`EmojiSegmentationCategory`].
    #[inline]
    pub fn step_record(&mut self, category: EmojiSegmentationCategory) {
        let curr = self.state;
        let next = Self::step(curr, category);

        if next.is_rejected() {
            return;
        }

        self.recorded.0 |= 1 << (next as u8);
        self.recorded.1 |= 1 << (category as u8);
        self.state = next;
    }

    #[inline]
    const fn contains_state(self, state: EmojiState) -> bool {
        self.recorded.0 & (1 << (state as u8)) != 0
    }

    #[inline]
    const fn contains_category(self, category: EmojiSegmentationCategory) -> bool {
        self.recorded.1 & (1 << (category as u8)) != 0
    }

    /// Return the [`EmojiSequence`].
    ///
    /// <https://www.unicode.org/reports/tr51/tr51-29.html#Emoji_Sequences>
    #[inline]
    const fn sequence(self) -> EmojiSequence {
        if self.contains_category(EmojiSegmentationCategory::Zwj) {
            return EmojiSequence::Zwj;
        }

        if self.contains_category(EmojiSegmentationCategory::TagBase)
            && self.contains_category(EmojiSegmentationCategory::TagEnd)
        {
            return EmojiSequence::Tag;
        }

        if self.contains_state(EmojiState::RegionalIndicator)
            && self.contains_state(EmojiState::Terminal)
        {
            return EmojiSequence::Flag;
        }

        if self.contains_category(EmojiSegmentationCategory::EmojiModifierBase)
            && self.contains_category(EmojiSegmentationCategory::EmojiModifier)
        {
            return EmojiSequence::Modifier;
        }

        if self.contains_category(EmojiSegmentationCategory::KeycapEnd) {
            return EmojiSequence::Keycap;
        }

        EmojiSequence::Basic
    }

    /// Returns the emoji presentation style.
    ///
    /// <https://www.unicode.org/reports/tr51/tr51-29.html#Presentation_Style>
    #[inline]
    pub const fn presentation_style(
        self,
        leading_is_emoji_presentation: bool,
    ) -> EmojiPresentationStyle {
        // Emoji presentation with variation selectors
        if self.contains_category(EmojiSegmentationCategory::Vs15) {
            return EmojiPresentationStyle::Text;
        }
        if self.contains_category(EmojiSegmentationCategory::Vs16) {
            return EmojiPresentationStyle::Emoji;
        }

        // Resolves presentation
        match self.sequence() {
            EmojiSequence::Basic if !leading_is_emoji_presentation => EmojiPresentationStyle::Text,
            _ => EmojiPresentationStyle::Emoji,
        }
    }

    /// Returns true if the emoji has variation selector.
    ///
    /// - `emoji_character + VS15`
    /// - `emoji_character + VS16`
    ///
    /// <https://www.unicode.org/reports/tr51/tr51-29.html#def_text_presentation_selector>
    /// <https://www.unicode.org/reports/tr51/tr51-29.html#def_emoji_presentation_selector>
    #[inline]
    pub const fn has_vs(self) -> bool {
        match self.sequence() {
            EmojiSequence::Basic | EmojiSequence::Keycap => {
                self.contains_category(EmojiSegmentationCategory::Vs15)
                    || self.contains_category(EmojiSegmentationCategory::Vs16)
            }
            _ => false,
        }
    }
}

impl Default for EmojiDFA {
    fn default() -> Self {
        Self::DEFAULT
    }
}
