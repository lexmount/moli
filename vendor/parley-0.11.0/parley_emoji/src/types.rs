// Copyright 2026 Christian Hansen and the Parley Authors
// SPDX-License-Identifier: MIT

// Adapted from <https://github.com/chansen/c-emoji>

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum EmojiState {
    /// No valid transition
    Reject = 0,
    /// Not inside any sequence
    Start,

    /// Accepting — a complete sequence ends here (may be extended)
    Terminal,
    Emoji,
    EmojiModifierBase,
    OptionalZwj,
    KeycapVs,
    TagBase,
    RegionalIndicator,

    /// Pending — inside a valid prefix, no complete sequence yet
    TagSpec,
    TagEmpty,
    KeycapBase,
    Zwj,
}

impl EmojiState {
    pub(crate) const TERMINAL: u8 = Self::Terminal as u8;
    pub(crate) const REGIONAL_INDICATOR: u8 = Self::RegionalIndicator as u8;

    #[inline]
    pub(crate) const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Start,
            2 => Self::Terminal,
            3 => Self::Emoji,
            4 => Self::EmojiModifierBase,
            5 => Self::OptionalZwj,
            6 => Self::KeycapVs,
            7 => Self::TagBase,
            8 => Self::RegionalIndicator,
            9 => Self::TagSpec,
            10 => Self::TagEmpty,
            11 => Self::KeycapBase,
            12 => Self::Zwj,
            _ => Self::Reject,
        }
    }

    /// Returns true if the emoji state is rejected.
    #[inline]
    pub(crate) const fn is_rejected(self) -> bool {
        matches!(self, Self::Reject)
    }

    /// Returns true if the emoji state is started.
    #[allow(
        unused,
        reason = "The text has been shaped and segmented into clusters."
    )]
    #[inline]
    pub(crate) const fn is_started(self) -> bool {
        matches!(self, Self::Start)
    }

    /// Returns true if the emoji state is accepting.
    #[allow(
        unused,
        reason = "The text has been shaped and segmented into clusters."
    )]
    #[inline]
    pub(crate) const fn is_accepting(self) -> bool {
        let curr = self as u8;
        Self::TERMINAL <= curr && curr <= Self::REGIONAL_INDICATOR
    }
}

/// Represents the category of an emoji segmentation.
///
/// <https://unicode.org/reports/tr51/#Definitions>
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EmojiSegmentationCategory {
    /// Emoji property
    Emoji = 0,
    /// Emoji presentation property
    EmojiPresentation,
    /// Emoji modifier property
    EmojiModifier,
    /// Emoji modifier base property
    EmojiModifierBase,
    /// `[0-9]` `#` `*`
    KeycapBase,
    /// `0x20E3`
    KeycapEnd,
    /// `0x1F3F4`
    TagBase,
    /// `[0xE0030-0xE0039]` or `[0xE0061-0xE007A]`
    TagSpec,
    /// `0xE007F`
    TagEnd,
    /// Regional Indicator character
    RegionalIndicator,
    /// `0xFE0E`
    Vs15,
    /// `0xFE0F`
    Vs16,
    /// `0x200D`
    Zwj,
    /// Other character
    Other,
}

impl EmojiSegmentationCategory {
    /// Returns the category of the given codepoint and its [UTS #51 properties][props].
    ///
    /// The boolean arguments are the character's `Emoji`, `Emoji_Presentation`,
    /// `Emoji_Modifier`, `Emoji_Modifier_Base` and `Regional_Indicator` properties.
    ///
    /// [props]: https://www.unicode.org/reports/tr51/tr51-29.html#Emoji_Properties
    #[inline]
    pub const fn from_codepoint(
        cp: u32,
        is_emoji: bool,
        is_emoji_presentation: bool,
        is_emoji_modifier: bool,
        is_emoji_modifier_base: bool,
        is_regional_indicator: bool,
    ) -> Self {
        match cp {
            0x30..=0x39 | 0x23 | 0x2A => Self::KeycapBase,
            0x200D => Self::Zwj,
            0x20E3 => Self::KeycapEnd,
            0xFE0E => Self::Vs15,
            0xFE0F => Self::Vs16,
            0x1F3F4 => Self::TagBase,
            0xE0030..=0xE0039 | 0xE0061..=0xE007A => Self::TagSpec,
            0xE007F => Self::TagEnd,
            _ => {
                if is_regional_indicator {
                    return Self::RegionalIndicator;
                }

                if is_emoji_modifier_base {
                    return Self::EmojiModifierBase;
                }

                if is_emoji_modifier {
                    return Self::EmojiModifier;
                }

                if is_emoji_presentation {
                    return Self::EmojiPresentation;
                }

                if is_emoji {
                    return Self::Emoji;
                }

                Self::Other
            }
        }
    }

    /// Returns true if it a presentation selector, VS15 or VS16.
    ///
    /// e.g.
    ///  - `U+270C + U+FE0E`: `✌`, force text presentation
    ///  - `U+270C + U+FE0F`: `✌️`, force emoji presentation
    pub const fn is_presentation_selector(self) -> bool {
        matches!(self, Self::Vs15 | Self::Vs16)
    }
}

/// Represents the category of an emoji sequence.
///
/// <https://www.unicode.org/reports/tr51/tr51-29.html#Emoji_Sequences>
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum EmojiSequence {
    /// Emoji basic sequence
    Basic,
    /// Emoji modifier sequence
    ///
    /// <https://www.unicode.org/reports/tr51/tr51-29.html#def_emoji_modifier_sequence>
    Modifier,
    /// Emoji flag sequence
    ///
    /// <https://www.unicode.org/reports/tr51/tr51-29.html#def_emoji_flag_sequence>
    Flag,
    /// Emoji tag sequence
    ///
    /// <https://www.unicode.org/reports/tr51/tr51-29.html#def_emoji_tag_sequence>
    Tag,
    /// Emoji keycap sequence
    ///
    /// <https://www.unicode.org/reports/tr51/tr51-29.html#def_emoji_keycap_sequence>
    Keycap,
    /// Emoji ZWJ sequence
    ///
    /// <https://www.unicode.org/reports/tr51/tr51-29.html#def_emoji_zwj_sequence>
    Zwj,
}

/// Represents presentation style for displaying emojis.
///
/// <https://www.unicode.org/reports/tr51/tr51-29.html#Presentation_Style>
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub enum EmojiPresentationStyle {
    /// Represents default presentation.
    #[default]
    Default,
    /// Represents a text presentation.
    Text,
    /// Represents an emoji presentation.
    Emoji,
}

impl EmojiPresentationStyle {
    /// Returns true if it is an emoji presentation.
    #[inline]
    pub const fn is_emoji(self) -> bool {
        matches!(self, Self::Emoji)
    }
}
