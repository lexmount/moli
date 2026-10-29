// SPDX-License-Identifier: MIT OR Apache-2.0

//! CSS character processing shared by text rendering and resource discovery.
//! This module owns no layout boxes, geometry, fonts, or source nodes.

use std::{
    char::{ToLowercase, ToUppercase},
    iter::Once,
};
use style::{
    computed_values::white_space_collapse::T as StyloWhiteSpace,
    values::specified::text::TextTransformCase,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextWhiteSpaceCollapse {
    #[default]
    Collapse,
    Preserve,
    PreserveBreaks,
    BreakSpaces,
}

impl From<StyloWhiteSpace> for TextWhiteSpaceCollapse {
    fn from(value: StyloWhiteSpace) -> Self {
        match value {
            StyloWhiteSpace::Collapse => Self::Collapse,
            StyloWhiteSpace::Preserve => Self::Preserve,
            StyloWhiteSpace::PreserveBreaks => Self::PreserveBreaks,
            StyloWhiteSpace::BreakSpaces => Self::BreakSpaces,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextTransform {
    #[default]
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

impl From<TextTransformCase> for TextTransform {
    fn from(value: TextTransformCase) -> Self {
        match value {
            TextTransformCase::None | TextTransformCase::MathAuto => Self::None,
            TextTransformCase::Uppercase => Self::Uppercase,
            TextTransformCase::Lowercase => Self::Lowercase,
            TextTransformCase::Capitalize => Self::Capitalize,
        }
    }
}

pub enum TextCaseMapping {
    Original(Once<char>),
    Uppercase(ToUppercase),
    Lowercase(ToLowercase),
}

impl Iterator for TextCaseMapping {
    type Item = char;
    fn next(&mut self) -> Option<char> {
        match self {
            Self::Original(value) => value.next(),
            Self::Uppercase(value) => value.next(),
            Self::Lowercase(value) => value.next(),
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Original(value) => value.size_hint(),
            Self::Uppercase(value) => value.size_hint(),
            Self::Lowercase(value) => value.size_hint(),
        }
    }
}

pub struct TextTransformState {
    word_start: bool,
}

impl Default for TextTransformState {
    fn default() -> Self {
        Self { word_start: true }
    }
}

impl TextTransformState {
    pub fn reset(&mut self) {
        self.word_start = true;
    }

    /// Observe source characters before whitespace folding or control filtering.
    /// The state continues across adjacent text runs and inline style changes.
    pub fn transform(&mut self, character: char, transform: TextTransform) -> TextCaseMapping {
        let result = match transform {
            TextTransform::Uppercase => TextCaseMapping::Uppercase(character.to_uppercase()),
            TextTransform::Lowercase => TextCaseMapping::Lowercase(character.to_lowercase()),
            TextTransform::Capitalize if self.word_start && character.is_alphabetic() => {
                TextCaseMapping::Uppercase(character.to_uppercase())
            }
            _ => TextCaseMapping::Original(std::iter::once(character)),
        };
        if character.is_alphanumeric() {
            self.word_start = false;
        } else if !matches!(character as u32, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F)
        {
            self.word_start = true;
        }
        result
    }
}

pub enum TextCharacter {
    Collapsible { segment_break: bool },
    ForcedBreak { preserve_pending: bool },
    Preserved { character: char, break_after: bool },
}

pub fn classify_text_character(character: char, mode: TextWhiteSpaceCollapse) -> TextCharacter {
    let segment_break = matches!(character, '\n' | '\r' | '\u{000C}');
    let collapsible = character == ' ' || character == '\t' || segment_break;
    match mode {
        TextWhiteSpaceCollapse::Collapse if collapsible => {
            TextCharacter::Collapsible { segment_break }
        }
        TextWhiteSpaceCollapse::PreserveBreaks if segment_break => TextCharacter::ForcedBreak {
            preserve_pending: false,
        },
        TextWhiteSpaceCollapse::PreserveBreaks if collapsible => TextCharacter::Collapsible {
            segment_break: false,
        },
        TextWhiteSpaceCollapse::Preserve | TextWhiteSpaceCollapse::BreakSpaces if segment_break => {
            TextCharacter::ForcedBreak {
                preserve_pending: true,
            }
        }
        _ => TextCharacter::Preserved {
            character,
            break_after: mode == TextWhiteSpaceCollapse::BreakSpaces && character == ' ',
        },
    }
}

/// Selects both the generated wrapper and its whitespace-suppression rule.
/// Mixed block/inline flow follows `white-space-collapse`; Flex and Grid omit
/// an all-CSS-whitespace text sequence before generating an anonymous item,
/// even when the inherited whitespace mode preserves those characters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AnonymousInlineRunRole {
    #[default]
    MixedFlow,
    FlexItem,
    GridItem,
}

fn whitespace_text_is_ignorable(text: &str, mode: TextWhiteSpaceCollapse) -> bool {
    if text.is_empty() {
        return true;
    }
    if !text_contains_only_css_whitespace(text) {
        return false;
    }
    match mode {
        TextWhiteSpaceCollapse::Collapse => true,
        TextWhiteSpaceCollapse::PreserveBreaks => !text
            .chars()
            .any(|character| matches!(character, '\n' | '\r' | '\u{000C}')),
        TextWhiteSpaceCollapse::Preserve | TextWhiteSpaceCollapse::BreakSpaces => false,
    }
}

fn text_contains_only_css_whitespace(text: &str) -> bool {
    text.chars()
        .all(|character| matches!(character, ' ' | '\t' | '\n' | '\r' | '\u{000C}'))
}

pub fn text_is_ignorable_in_anonymous_run(
    text: &str,
    mode: TextWhiteSpaceCollapse,
    run_role: AnonymousInlineRunRole,
) -> bool {
    match run_role {
        AnonymousInlineRunRole::MixedFlow => whitespace_text_is_ignorable(text, mode),
        AnonymousInlineRunRole::FlexItem | AnonymousInlineRunRole::GridItem => {
            text_contains_only_css_whitespace(text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AnonymousInlineRunRole, TextWhiteSpaceCollapse as InlineWhiteSpaceCollapse,
        text_contains_only_css_whitespace, text_is_ignorable_in_anonymous_run,
        whitespace_text_is_ignorable,
    };

    #[test]
    fn only_collapsible_whitespace_is_ignorable_during_box_construction() {
        assert!(whitespace_text_is_ignorable(
            " \t\n",
            InlineWhiteSpaceCollapse::Collapse,
        ));
        assert!(whitespace_text_is_ignorable(
            " \t",
            InlineWhiteSpaceCollapse::PreserveBreaks,
        ));
        assert!(!whitespace_text_is_ignorable(
            "\n",
            InlineWhiteSpaceCollapse::PreserveBreaks,
        ));
        assert!(!whitespace_text_is_ignorable(
            "\n",
            InlineWhiteSpaceCollapse::Preserve,
        ));
        assert!(!whitespace_text_is_ignorable(
            " ",
            InlineWhiteSpaceCollapse::BreakSpaces,
        ));
        assert!(!whitespace_text_is_ignorable(
            "\u{00a0}",
            InlineWhiteSpaceCollapse::Collapse,
        ));
    }

    #[test]
    fn flex_and_grid_item_runs_ignore_css_whitespace_independently_of_collapse_mode() {
        assert!(text_contains_only_css_whitespace(" \t\n\r\u{000C}"));
        assert!(!text_contains_only_css_whitespace("\u{00a0}"));

        for role in [
            AnonymousInlineRunRole::FlexItem,
            AnonymousInlineRunRole::GridItem,
        ] {
            assert!(text_is_ignorable_in_anonymous_run(
                " \t\n\r\u{000C}",
                InlineWhiteSpaceCollapse::Preserve,
                role,
            ));
            assert!(!text_is_ignorable_in_anonymous_run(
                "\u{00a0}",
                InlineWhiteSpaceCollapse::Collapse,
                role,
            ));
            assert!(!text_is_ignorable_in_anonymous_run(
                " text ",
                InlineWhiteSpaceCollapse::Preserve,
                role,
            ));
        }

        assert!(!text_is_ignorable_in_anonymous_run(
            " \n",
            InlineWhiteSpaceCollapse::Preserve,
            AnonymousInlineRunRole::MixedFlow,
        ));
    }
}
