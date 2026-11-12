// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{
    FontContext, Language, LayoutContext, LineBreak, RangedBuilder, StyleProperty,
    WhiteSpaceCollapse, WordBreak,
};
use alloc::{vec, vec::Vec};
use fontique::FontWeight;
use icu_properties::props::Script;
use icu_segmenter::{LineSegmenter, options::LineBreakOptions};

/// Whether we have complex script dictionaries compiled-in, which ICU4X then uses for segmentation.
const HAS_DICTIONARIES: bool = cfg!(feature = "complex-scripts");

#[derive(Default)]
struct TestContext {
    pub layout_context: LayoutContext,
    pub font_context: FontContext,
}

impl TestContext {
    fn expect_soft_wrap_opportunity_list(self, expected: Vec<bool>) -> Self {
        let actual = self.soft_wrap_opportunity_list();
        assert_eq!(actual, expected, "Soft wrap opportunity list mismatch");
        self
    }

    fn expect_bidi_embed_level_list(self, expected: &[u8]) -> Self {
        let actual = self.layout_context.analysis.bidi_levels();
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(actual),
            expected,
            "Bidi embed level list mismatch"
        );
        self
    }

    fn expect_script_list(self, expected: Vec<Script>) -> Self {
        let actual: Vec<_> = self
            .layout_context
            .analysis
            .char_info()
            .iter()
            .map(|info| info.script)
            .collect();
        assert_eq!(actual, expected, "Script list mismatch");
        self
    }

    fn expect_grapheme_start_list(self, expected: Vec<bool>) -> Self {
        let actual: Vec<_> = self
            .layout_context
            .analysis
            .char_info()
            .iter()
            .map(|info| info.is_grapheme_start())
            .collect();
        assert_eq!(actual, expected, "Grapheme start list mismatch");
        self
    }

    fn expect_word_boundary_list(self, expected: Vec<bool>) -> Self {
        let actual: Vec<_> = self
            .layout_context
            .analysis
            .char_info()
            .iter()
            .map(|info| info.is_word_boundary())
            .collect();
        assert_eq!(actual, expected, "Word boundary list mismatch");
        self
    }

    fn expect_is_control_list(self, expected: Vec<bool>) -> Self {
        let actual: Vec<_> = self
            .layout_context
            .analysis
            .char_info()
            .iter()
            .map(|info| info.is_control())
            .collect();
        assert_eq!(actual, expected, "Is control list mismatch");
        self
    }

    fn expect_contributes_to_shaping_list(self, expected: Vec<bool>) -> Self {
        let actual: Vec<_> = self
            .layout_context
            .analysis
            .char_info()
            .iter()
            .map(|info| info.contributes_to_shaping())
            .collect();
        assert_eq!(actual, expected, "Contributes to shaping list mismatch");
        self
    }

    fn expect_force_normalize_list(self, expected: Vec<bool>) -> Self {
        let actual: Vec<_> = self
            .layout_context
            .analysis
            .char_info()
            .iter()
            .map(|info| info.force_normalize())
            .collect();
        assert_eq!(actual, expected, "Force normalize list mismatch");
        self
    }

    fn expect_is_emoji_or_pictograph_list(self, expected: Vec<bool>) -> Self {
        let actual: Vec<_> = self
            .layout_context
            .analysis
            .char_info()
            .iter()
            .map(|info| info.is_emoji_or_pictograph())
            .collect();
        assert_eq!(actual, expected, "Is emoji or pictograph list mismatch");
        self
    }

    fn soft_wrap_opportunity_list(&self) -> Vec<bool> {
        self.layout_context
            .analysis
            .char_info()
            .iter()
            .map(|info| info.is_soft_wrap_opportunity())
            .collect()
    }
}

fn verify_analysis(
    text: &str,
    configure_builder: impl for<'a> FnOnce(&mut RangedBuilder<'a, [u8; 4]>),
) -> TestContext {
    let mut test_context = TestContext::default();

    {
        let mut builder = test_context.layout_context.ranged_builder(
            &mut test_context.font_context,
            text,
            1.,
            true,
        );

        // Apply test-specific configuration
        configure_builder(&mut builder);

        _ = builder.build(text);
    }

    test_context
}

fn verify_analysis_with_override(
    text: &str,
    line_break_override: &crate::LineBreakOverrideFn,
) -> TestContext {
    let mut test_context = TestContext::default();

    {
        let mut builder = test_context.layout_context.ranged_builder(
            &mut test_context.font_context,
            text,
            1.,
            true,
        );
        builder.set_line_break_override(Some(line_break_override));
        _ = builder.build(text);
    }

    test_context
}

#[test]
fn test_line_break_override_none_matches_default() {
    let text = "ab/cd ef";
    let default = verify_analysis(text, |_| {}).soft_wrap_opportunity_list();
    let overridden = verify_analysis_with_override(text, &|_| None).soft_wrap_opportunity_list();
    assert_eq!(default, overridden);
}

#[test]
fn test_line_break_override_suppresses_break_after_slash() {
    let text = "ab/cd";
    let default = verify_analysis(text, |_| {}).soft_wrap_opportunity_list();
    assert!(default.contains(&true),);

    let overridden = verify_analysis_with_override(text, &|cx| {
        if cx.before == '/' { Some(false) } else { None }
    })
    .soft_wrap_opportunity_list();

    assert!(!overridden.contains(&true),);
}

#[test]
fn test_line_break_override_doesnt_add_soft_wrap_opportunity_around_mandatory_break() {
    let overridden =
        verify_analysis_with_override("a\nb", &|_| Some(true)).soft_wrap_opportunity_list();

    assert_eq!(overridden, vec![false, false, false]);
}

#[test]
fn test_latin_mixed_keep_all_last() {
    verify_analysis("AB", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 0..1);
        builder.push(StyleProperty::WordBreak(WordBreak::KeepAll), 1..2);
    })
    .expect_soft_wrap_opportunity_list(vec![false, false])
    .expect_word_boundary_list(vec![true, false])
    .expect_bidi_embed_level_list(&[])
    .expect_script_list(vec![Script::Latin, Script::Latin])
    .expect_is_control_list(vec![false, false])
    .expect_contributes_to_shaping_list(vec![true, true])
    .expect_force_normalize_list(vec![false, false]);
}

#[test]
fn test_mandatory_break_in_text() {
    verify_analysis("ABC DEF\nG", |_| {})
        .expect_soft_wrap_opportunity_list(vec![
            false, false, false, false, true, false, false, false, false,
        ])
        .expect_word_boundary_list(vec![
            true, false, false, true, true, false, false, true, true,
        ])
        .expect_bidi_embed_level_list(&[])
        .expect_script_list(vec![
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Common,
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Common,
            Script::Latin,
        ])
        .expect_is_control_list(vec![
            false, false, false, false, false, false, false, true, false,
        ])
        .expect_contributes_to_shaping_list(vec![
            true, true, true, true, true, true, true, false, true,
        ])
        .expect_force_normalize_list(vec![
            false, false, false, false, false, false, false, false, false,
        ]);
}

#[test]
fn test_line_separator_is_hard_break() {
    verify_analysis("A\u{2028}B", |_| {})
        .expect_soft_wrap_opportunity_list(vec![false, false, false])
        .expect_word_boundary_list(vec![true, true, true])
        .expect_contributes_to_shaping_list(vec![true, false, true]);
}

#[test]
fn test_paragraph_separator_is_hard_break() {
    verify_analysis("A\u{2029}B", |_| {})
        .expect_soft_wrap_opportunity_list(vec![false, false, false])
        .expect_word_boundary_list(vec![true, true, true])
        .expect_contributes_to_shaping_list(vec![true, false, true]);
}

/// ICU4X emits a soft break opportunity at the end of a complex-script run even
/// when it is directly followed by a mandatory break character (violating UAX #14
/// LB6). That opportunity must be dropped so the newline is not treated as a
/// regular wrap opportunity. See <https://github.com/linebender/parley/issues/768>.
#[test]
fn test_mandatory_break_after_complex_script_run() {
    // Check that the ICU4X bug still reproduces: a break opportunity is reported at byte 6, between
    // the second Thai character and the `\n`. Once this assertion fails, ICU4X has been fixed and
    // this test as well as `builders_newline_inside_complex_script_run_is_hard_break` can be
    // dropped.
    let icu_breaks: Vec<usize> = LineSegmenter::new_dictionary(LineBreakOptions::default())
        .segment_str("กก\nกก")
        .collect();
    assert_eq!(icu_breaks, vec![0, 6, 7, 13]);

    // Orthogonal to this test, but without a dictionary, UAX #29 WB999 gives a word boundary at
    // every grapheme.
    let inner = !HAS_DICTIONARIES;

    // Thai
    verify_analysis("กก\nกก", |_| {})
        .expect_soft_wrap_opportunity_list(vec![false, false, false, false, false])
        .expect_word_boundary_list(vec![true, inner, true, true, inner]);
    // Khmer
    verify_analysis("ក្ម\nក្ម", |_| {})
        .expect_soft_wrap_opportunity_list(vec![false, false, false, false, false, false, false])
        .expect_word_boundary_list(vec![true, false, false, true, true, false, false]);
    // Lao
    verify_analysis("ກກ\nກກ", |_| {})
        .expect_soft_wrap_opportunity_list(vec![false, false, false, false, false])
        .expect_word_boundary_list(vec![true, inner, true, true, inner]);
}

#[test]
fn test_word_boundaries_complex_scripts() {
    // 中文|文本|测试 with a dictionary, 中|文|文|本|测|试 without.
    verify_analysis("中文文本测试", |_| {}).expect_word_boundary_list(if HAS_DICTIONARIES {
        vec![true, false, true, false, true, false]
    } else {
        vec![true, true, true, true, true, true]
    });
    // สวัสดี|ครับ with a dictionary, one word per grapheme cluster without.
    verify_analysis("สวัสดีครับ", |_| {})
        .expect_grapheme_start_list(vec![
            true, true, false, true, true, false, true, true, false, true,
        ])
        .expect_word_boundary_list(if HAS_DICTIONARIES {
            vec![
                true, false, false, false, false, false, true, false, false, false,
            ]
        } else {
            vec![
                true, true, false, true, true, false, true, true, false, true,
            ]
        });
    verify_analysis("カタカナ", |_| {}).expect_word_boundary_list(vec![true, false, false, false]);
}

/// Scripts that need a dictionary, but for which we don't have one even under `complex-scripts` get
/// UAX #29 WB999 boundaries.
#[test]
fn test_word_boundaries_complex_script_without_dictionary() {
    // Ahom letters (U+11700, U+11701) and a combining vowel sign (U+1171D).
    verify_analysis("\u{11700}\u{1171d}\u{11701} \u{11700}", |_| {})
        .expect_grapheme_start_list(vec![true, false, true, true, true])
        .expect_word_boundary_list(vec![true, false, true, true, true]);
}

#[test]
fn test_blank() {
    verify_analysis("", |_| {})
        .expect_soft_wrap_opportunity_list(vec![false])
        .expect_word_boundary_list(vec![true])
        .expect_bidi_embed_level_list(&[])
        .expect_script_list(vec![Script::Common])
        .expect_is_control_list(vec![false])
        .expect_contributes_to_shaping_list(vec![true])
        .expect_force_normalize_list(vec![false]);
}

#[test]
fn test_latin_mixed_keep_all_first() {
    verify_analysis("AB", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::KeepAll), 0..1);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 1..2);
    })
    .expect_soft_wrap_opportunity_list(vec![false, false])
    .expect_word_boundary_list(vec![true, false]);
}

#[test]
fn test_mixed_break_four_segments() {
    verify_analysis("ABCD 123", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 0..1);
        builder.push(StyleProperty::WordBreak(WordBreak::KeepAll), 1..2);
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 2..4);
        builder.push(StyleProperty::WordBreak(WordBreak::KeepAll), 4..8);
    })
    .expect_soft_wrap_opportunity_list(vec![false, false, true, true, false, true, false, false])
    .expect_word_boundary_list(vec![true, false, false, false, true, true, false, false]);
}

#[test]
fn test_alternate_twice_within_word_normal_break_normal() {
    verify_analysis("ABC", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 0..1);
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 1..2);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 2..3);
    })
    .expect_soft_wrap_opportunity_list(vec![false, true, false])
    .expect_word_boundary_list(vec![true, false, false]);
}

#[test]
fn test_alternate_twice_within_word_break_normal_break() {
    verify_analysis("ABC", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 0..1);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 1..2);
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 2..3);
    })
    .expect_soft_wrap_opportunity_list(vec![false, false, true])
    .expect_word_boundary_list(vec![true, false, false]);
}

#[test]
fn test_latin_trailing_space_mixed() {
    verify_analysis("AB ", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 0..1);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 1..3);
    })
    .expect_soft_wrap_opportunity_list(vec![false, false, false])
    .expect_word_boundary_list(vec![true, false, true])
    .expect_bidi_embed_level_list(&[])
    .expect_script_list(vec![Script::Latin, Script::Latin, Script::Common]);
}

#[test]
fn test_latin_leading_space_mixed() {
    verify_analysis(" AB", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 0..1);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 1..3);
    })
    .expect_soft_wrap_opportunity_list(vec![false, true, false])
    .expect_word_boundary_list(vec![true, true, false])
    .expect_bidi_embed_level_list(&[])
    .expect_script_list(vec![Script::Common, Script::Latin, Script::Latin]);
}

#[test]
fn test_latin_mixed_break_all_last() {
    verify_analysis("AB", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 0..1);
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 1..2);
    })
    .expect_soft_wrap_opportunity_list(vec![false, true])
    .expect_word_boundary_list(vec![true, false]);
}

#[test]
fn test_latin_mixed_break_all_first() {
    verify_analysis("AB", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 0..1);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 1..2);
    })
    .expect_soft_wrap_opportunity_list(vec![false, false])
    .expect_word_boundary_list(vec![true, false]);
}

#[test]
fn test_all_whitespace() {
    verify_analysis("   ", |_| {})
        .expect_soft_wrap_opportunity_list(vec![false, false, false])
        .expect_word_boundary_list(vec![true, false, false])
        .expect_bidi_embed_level_list(&[])
        .expect_script_list(vec![Script::Common, Script::Common, Script::Common]);
}

#[test]
fn test_multi_char_grapheme() {
    verify_analysis("A e\u{301} B", |_| {})
        .expect_soft_wrap_opportunity_list(vec![false, false, true, false, false, true])
        .expect_word_boundary_list(vec![true, true, true, false, true, true])
        .expect_script_list(vec![
            Script::Latin,
            Script::Common,
            Script::Latin,
            Script::Inherited,
            Script::Common,
            Script::Latin,
        ])
        .expect_is_control_list(vec![false, false, false, false, false, false])
        .expect_contributes_to_shaping_list(vec![true, true, true, true, true, true])
        .expect_force_normalize_list(vec![false, false, false, true, false, false])
        .expect_grapheme_start_list(vec![true, true, true, false, true, true]);
}

#[test]
fn test_grapheme_start_multi_char_graphemes() {
    // CRLF (GB3), emoji ZWJ sequence 👨‍👩‍👧 (GB9/GB11), and regional indicator
    // pair 🇦🇺 (GB12) are each a single extended grapheme cluster.
    verify_analysis(
        "a\r\n\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{1F1E6}\u{1F1FA}b",
        |_| {},
    )
    .expect_grapheme_start_list(vec![
        true,  // 'a'
        true,  // '\r'
        false, // '\n' (CR x LF)
        true,  // 👨
        false, // ZWJ
        false, // 👩
        false, // ZWJ
        false, // 👧
        true,  // 🇦 (regional indicator A)
        false, // 🇺 (regional indicator U)
        true,  // 'b'
    ]);
}

#[test]
fn test_mixed_break_frequent_alternation() {
    verify_analysis("ABCD 123", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 0..1);
        builder.push(StyleProperty::WordBreak(WordBreak::KeepAll), 1..2);
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 2..3);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 3..4);
        builder.push(StyleProperty::WordBreak(WordBreak::KeepAll), 4..5);
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 5..6);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 6..7);
        builder.push(StyleProperty::WordBreak(WordBreak::KeepAll), 7..8);
    })
    .expect_soft_wrap_opportunity_list(vec![false, false, true, false, false, true, false, false])
    .expect_word_boundary_list(vec![true, false, false, false, true, true, false, false])
    .expect_script_list(vec![
        Script::Latin,
        Script::Latin,
        Script::Latin,
        Script::Latin,
        Script::Common,
        Script::Common,
        Script::Common,
        Script::Common,
    ]);
}

#[test]
fn test_mixed_style() {
    verify_analysis("A  B  C D", |builder| {
        builder.push(StyleProperty::FontWeight(FontWeight::new(400.0)), 0..3);
        builder.push(StyleProperty::FontWeight(FontWeight::new(700.0)), 3..9);
    })
    .expect_soft_wrap_opportunity_list(vec![
        false, false, false, true, false, false, true, false, true,
    ])
    .expect_word_boundary_list(vec![true, true, false, true, true, false, true, true, true])
    .expect_script_list(vec![
        Script::Latin,
        Script::Common,
        Script::Common,
        Script::Latin,
        Script::Common,
        Script::Common,
        Script::Latin,
        Script::Common,
        Script::Latin,
    ]);
}

#[test]
fn test_mixed_ltr_rtl() {
    verify_analysis("Hello مرحبا", |_| {})
        .expect_soft_wrap_opportunity_list(vec![
            false, false, false, false, false, false, true, false, false, false, false,
        ])
        .expect_word_boundary_list(vec![
            true, false, false, false, false, true, true, false, false, false, false,
        ])
        .expect_bidi_embed_level_list(&[0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1])
        .expect_script_list(vec![
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Common,
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
        ]);
}

#[test]
fn test_bidi_bracket_pairs() {
    // In the following RTL paragraph, the `]!)` is sandwiched by LTR text, but pairs with the
    // brackets in the strong RTL part (and note the neutral `!` therefore also becomes RTL).
    verify_analysis("אב(גד[&e(b)f]!)gh", |_| {})
        .expect_bidi_embed_level_list(&[1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 1, 1, 1, 2, 2]);
}

#[test]
fn test_multi_byte_chars_alternating_break_all() {
    verify_analysis("€你€你AA", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 0..3);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 3..6);
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 6..9);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 9..12);
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 12..13);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 13..14);
    })
    .expect_soft_wrap_opportunity_list(vec![false, false, true, false, true, false])
    .expect_word_boundary_list(vec![true, true, true, true, true, false])
    .expect_script_list(vec![
        Script::Common,
        Script::Han,
        Script::Common,
        Script::Han,
        Script::Latin,
        Script::Latin,
    ]);
}

#[test]
fn test_multi_byte_chars_varying_utf8_lengths_whitespace_separated() {
    verify_analysis("ß € 𝓗 你 ą", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 0..3);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 3..7);
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 7..12);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 12..16);
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 16..19);
    })
    .expect_soft_wrap_opportunity_list(vec![
        false, false, true, false, true, false, true, false, true,
    ])
    .expect_word_boundary_list(vec![true, true, true, true, true, true, true, true, true])
    .expect_script_list(vec![
        Script::Latin,
        Script::Common,
        Script::Common,
        Script::Common,
        Script::Common,
        Script::Common,
        Script::Han,
        Script::Common,
        Script::Latin,
    ]);
}

#[test]
fn test_multi_byte_chars_varying_utf8_lengths() {
    verify_analysis("ß€𝓗你ą", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 0..2);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 2..5);
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 5..9);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 9..12);
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 12..14);
    })
    .expect_soft_wrap_opportunity_list(vec![false, false, false, true, true])
    .expect_word_boundary_list(vec![true, true, true, true, true])
    .expect_script_list(vec![
        Script::Latin,
        Script::Common,
        Script::Common,
        Script::Han,
        Script::Latin,
    ]);
}

#[test]
fn test_mixed_ltr_rtl_nested_embedding() {
    verify_analysis("In Hebrew: שנת 2024 היא...", |_| {})
        .expect_soft_wrap_opportunity_list(vec![
            false, false, false, true, false, false, false, false, false, false, false, true,
            false, false, false, true, false, false, false, false, true, false, false, false,
            false, false,
        ])
        .expect_word_boundary_list(vec![
            true, false, true, true, false, false, false, false, false, true, true, true, false,
            false, true, true, false, false, false, true, true, false, false, true, true, true,
        ])
        .expect_bidi_embed_level_list(&[
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 1, 1, 1, 1, 0, 0, 0,
        ])
        .expect_script_list(vec![
            Script::Latin,
            Script::Latin,
            Script::Common,
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Common,
            Script::Common,
            Script::Hebrew,
            Script::Hebrew,
            Script::Hebrew,
            Script::Common,
            Script::Common,
            Script::Common,
            Script::Common,
            Script::Common,
            Script::Common,
            Script::Hebrew,
            Script::Hebrew,
            Script::Hebrew,
            Script::Common,
            Script::Common,
            Script::Common,
        ]);
}

#[test]
fn test_mixed_break_simple() {
    verify_analysis("ABCD 123", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 0..1);
        builder.push(StyleProperty::WordBreak(WordBreak::KeepAll), 1..8);
    })
    .expect_soft_wrap_opportunity_list(vec![false, false, false, false, false, true, false, false])
    .expect_word_boundary_list(vec![true, false, false, false, true, true, false, false])
    .expect_script_list(vec![
        Script::Latin,
        Script::Latin,
        Script::Latin,
        Script::Latin,
        Script::Common,
        Script::Common,
        Script::Common,
        Script::Common,
    ]);
}

#[test]
fn test_multi_char_grapheme_mixed_break_all() {
    verify_analysis("A e\u{301} B", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 0..1);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 1..2);
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 2..5);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 5..6);
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 6..7);
    })
    .expect_soft_wrap_opportunity_list(vec![false, false, true, false, false, true])
    .expect_word_boundary_list(vec![true, true, true, false, true, true])
    .expect_bidi_embed_level_list(&[])
    .expect_script_list(vec![
        Script::Latin,
        Script::Common,
        Script::Latin,
        Script::Inherited,
        Script::Common,
        Script::Latin,
    ])
    .expect_is_control_list(vec![false, false, false, false, false, false])
    .expect_contributes_to_shaping_list(vec![true, true, true, true, true, true])
    .expect_force_normalize_list(vec![false, false, false, true, false, false]);
}

#[test]
fn test_multi_byte_chars_alternating_keep_all() {
    verify_analysis("€你€你AA", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::KeepAll), 0..3);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 3..6);
        builder.push(StyleProperty::WordBreak(WordBreak::KeepAll), 6..9);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 9..12);
        builder.push(StyleProperty::WordBreak(WordBreak::KeepAll), 12..13);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 13..14);
    })
    .expect_soft_wrap_opportunity_list(vec![false, false, true, false, false, false])
    .expect_word_boundary_list(vec![true, true, true, true, true, false])
    .expect_script_list(vec![
        Script::Common,
        Script::Han,
        Script::Common,
        Script::Han,
        Script::Latin,
        Script::Latin,
    ]);
}

#[test]
fn test_mixed_ltr_rtl_multiple_segments() {
    verify_analysis("Hello مرحبا World عالم Test اختبار", |_| {})
        .expect_soft_wrap_opportunity_list(vec![
            false, false, false, false, false, false, true, false, false, false, false, false,
            true, false, false, false, false, false, true, false, false, false, false, true, false,
            false, false, false, true, false, false, false, false, false,
        ])
        .expect_word_boundary_list(vec![
            true, false, false, false, false, true, true, false, false, false, false, true, true,
            false, false, false, false, true, true, false, false, false, true, true, false, false,
            false, true, true, false, false, false, false, false,
        ])
        .expect_bidi_embed_level_list(&[
            0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 1,
            1, 1, 1, 1, 1,
        ])
        .expect_script_list(vec![
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Common,
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
            Script::Common,
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Common,
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
            Script::Common,
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Common,
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
        ]);
}

#[test]
fn test_multi_char_grapheme_mixed_break_and_keep_all() {
    verify_analysis("A e\u{301} B", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::KeepAll), 0..1);
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 1..2);
        builder.push(StyleProperty::WordBreak(WordBreak::KeepAll), 2..5);
        builder.push(StyleProperty::WordBreak(WordBreak::BreakAll), 5..6);
        builder.push(StyleProperty::WordBreak(WordBreak::KeepAll), 6..7);
    })
    .expect_soft_wrap_opportunity_list(vec![false, false, true, false, false, true])
    .expect_word_boundary_list(vec![true, true, true, false, true, true])
    .expect_bidi_embed_level_list(&[])
    .expect_script_list(vec![
        Script::Latin,
        Script::Common,
        Script::Latin,
        Script::Inherited,
        Script::Common,
        Script::Latin,
    ])
    .expect_is_control_list(vec![false, false, false, false, false, false])
    .expect_contributes_to_shaping_list(vec![true, true, true, true, true, true])
    .expect_force_normalize_list(vec![false, false, false, true, false, false]);
}

#[test]
fn test_multi_char_grapheme_mixed_keep_all() {
    verify_analysis("A e\u{301} B", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::KeepAll), 0..1);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 1..2);
        builder.push(StyleProperty::WordBreak(WordBreak::KeepAll), 2..5);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 5..6);
        builder.push(StyleProperty::WordBreak(WordBreak::KeepAll), 6..7);
    })
    .expect_soft_wrap_opportunity_list(vec![false, false, true, false, false, true])
    .expect_word_boundary_list(vec![true, true, true, false, true, true])
    .expect_bidi_embed_level_list(&[])
    .expect_script_list(vec![
        Script::Latin,
        Script::Common,
        Script::Latin,
        Script::Inherited,
        Script::Common,
        Script::Latin,
    ])
    .expect_is_control_list(vec![false, false, false, false, false, false])
    .expect_contributes_to_shaping_list(vec![true, true, true, true, true, true])
    .expect_force_normalize_list(vec![false, false, false, true, false, false]);
}

#[test]
fn test_multi_paragraph_bidi() {
    verify_analysis("Hello مرحبا \nTest اختبار", |_| {})
        .expect_soft_wrap_opportunity_list(vec![
            false, false, false, false, false, false, true, false, false, false, false, false,
            false, false, false, false, false, false, true, false, false, false, false, false,
        ])
        .expect_word_boundary_list(vec![
            true, false, false, false, false, true, true, false, false, false, false, true, true,
            true, false, false, false, true, true, false, false, false, false, false,
        ])
        .expect_bidi_embed_level_list(&[
            0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1,
        ])
        .expect_script_list(vec![
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Common,
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
            Script::Common,
            Script::Common,
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Latin,
            Script::Common,
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
        ])
        .expect_is_control_list(vec![
            false, false, false, false, false, false, false, false, false, false, false, false,
            true, false, false, false, false, false, false, false, false, false, false, false,
        ])
        .expect_contributes_to_shaping_list(vec![
            true, true, true, true, true, true, true, true, true, true, true, true, false, true,
            true, true, true, true, true, true, true, true, true, true,
        ]);
}

#[test]
fn test_single_char() {
    verify_analysis("A", |_| {})
        .expect_soft_wrap_opportunity_list(vec![false])
        .expect_word_boundary_list(vec![true]);
}

#[test]
fn test_rtl_paragraph_with_non_authoritative_logical_first_char_two_paragraphs() {
    verify_analysis("حدا\u{64b} \nحدا\u{64b} ", |_| {})
        .expect_soft_wrap_opportunity_list(vec![
            false, false, false, false, false, false, false, false, false, false, false,
        ])
        .expect_word_boundary_list(vec![
            true, false, false, false, true, true, true, false, false, false, true,
        ])
        .expect_bidi_embed_level_list(&[1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1])
        .expect_script_list(vec![
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
            Script::Inherited,
            Script::Common,
            Script::Common,
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
            Script::Inherited,
            Script::Common,
        ])
        .expect_is_control_list(vec![
            false, false, false, false, false, true, false, false, false, false, false,
        ])
        .expect_contributes_to_shaping_list(vec![
            true, true, true, true, true, false, true, true, true, true, true,
        ])
        .expect_force_normalize_list(vec![
            false, false, false, true, false, false, false, false, false, true, false,
        ]);
}

#[test]
fn test_three_chars() {
    verify_analysis("ABC", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 0..3);
    })
    .expect_soft_wrap_opportunity_list(vec![false, false, false])
    .expect_word_boundary_list(vec![true, false, false]);
}

#[test]
fn test_single_char_multi_byte() {
    verify_analysis("€", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::KeepAll), 0..3);
    })
    .expect_soft_wrap_opportunity_list(vec![false])
    .expect_word_boundary_list(vec![true])
    .expect_bidi_embed_level_list(&[])
    .expect_script_list(vec![Script::Common]);
}

#[test]
fn test_rtl_paragraph_with_non_authoritative_logical_first_character() {
    verify_analysis("حدا\u{64b} ", |_| {})
        .expect_soft_wrap_opportunity_list(vec![false, false, false, false, false])
        .expect_word_boundary_list(vec![true, false, false, false, true])
        .expect_bidi_embed_level_list(&[1, 1, 1, 1, 1])
        .expect_script_list(vec![
            Script::Arabic,
            Script::Arabic,
            Script::Arabic,
            Script::Inherited,
            Script::Common,
        ])
        .expect_is_control_list(vec![false, false, false, false, false])
        .expect_contributes_to_shaping_list(vec![true, true, true, true, true])
        .expect_force_normalize_list(vec![false, false, false, true, false]);
}

#[test]
fn test_two_newlines() {
    verify_analysis("\n\n", |_| {})
        .expect_soft_wrap_opportunity_list(vec![false, false])
        .expect_word_boundary_list(vec![true, true])
        .expect_bidi_embed_level_list(&[])
        .expect_script_list(vec![Script::Common, Script::Common]);
}

#[test]
fn test_newline() {
    verify_analysis("\n", |_| {})
        .expect_soft_wrap_opportunity_list(vec![false])
        .expect_word_boundary_list(vec![true])
        .expect_bidi_embed_level_list(&[])
        .expect_script_list(vec![Script::Common]);
}

#[test]
fn test_two_chars_keep_all() {
    verify_analysis("AB", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::KeepAll), 0..2);
    })
    .expect_soft_wrap_opportunity_list(vec![false, false])
    .expect_word_boundary_list(vec![true, false])
    .expect_bidi_embed_level_list(&[])
    .expect_script_list(vec![Script::Latin, Script::Latin]);
}

#[test]
fn test_whitespace_contiguous_interspersed_in_latin() {
    verify_analysis("A  B  C D", |_| {})
        .expect_soft_wrap_opportunity_list(vec![
            false, false, false, true, false, false, true, false, true,
        ])
        .expect_word_boundary_list(vec![true, true, false, true, true, false, true, true, true])
        .expect_script_list(vec![
            Script::Latin,
            Script::Common,
            Script::Common,
            Script::Latin,
            Script::Common,
            Script::Common,
            Script::Latin,
            Script::Common,
            Script::Latin,
        ]);
}

#[test]
fn test_whitespace_contiguous_interspersed_in_latin_mixed() {
    verify_analysis("A  B  C D", |builder| {
        builder.push(StyleProperty::WordBreak(WordBreak::KeepAll), 0..3);
        builder.push(StyleProperty::WordBreak(WordBreak::Normal), 3..9);
    })
    .expect_soft_wrap_opportunity_list(vec![
        false, false, false, true, false, false, true, false, true,
    ])
    .expect_word_boundary_list(vec![true, true, false, true, true, false, true, true, true])
    .expect_script_list(vec![
        Script::Latin,
        Script::Common,
        Script::Common,
        Script::Latin,
        Script::Common,
        Script::Common,
        Script::Latin,
        Script::Common,
        Script::Latin,
    ]);
}

#[test]
fn test_color_emoji_with_presentation() {
    verify_analysis("\u{270c}\u{fe0f}", |_| {})
        .expect_is_emoji_or_pictograph_list(vec![true, false])
        .expect_force_normalize_list(vec![false, false]);
}

#[test]
fn test_break_spaces_adds_opportunity_between_spaces() {
    verify_analysis("A  B", |builder| {
        builder.push(
            StyleProperty::WhiteSpaceCollapse(WhiteSpaceCollapse::BreakSpaces),
            0..4,
        );
    })
    .expect_soft_wrap_opportunity_list(vec![false, false, true, true])
    .expect_word_boundary_list(vec![true, true, false, true]);
}

#[test]
fn test_break_spaces_not_applied_under_preserve() {
    verify_analysis("A  B", |builder| {
        builder.push(
            StyleProperty::WhiteSpaceCollapse(WhiteSpaceCollapse::Preserve),
            0..4,
        );
    })
    .expect_soft_wrap_opportunity_list(vec![false, false, false, true])
    .expect_word_boundary_list(vec![true, true, false, true]);
}

#[test]
fn test_break_spaces_tab_and_ideographic_space() {
    verify_analysis("A \t\u{3000}B", |builder| {
        builder.push(
            StyleProperty::WhiteSpaceCollapse(WhiteSpaceCollapse::BreakSpaces),
            0..6,
        );
    })
    .expect_soft_wrap_opportunity_list(vec![false, false, true, true, true])
    .expect_word_boundary_list(vec![true, true, true, true, true]);
}

#[test]
fn test_break_spaces_mandatory_break_takes_precedence() {
    verify_analysis("A \n B", |builder| {
        builder.push(
            StyleProperty::WhiteSpaceCollapse(WhiteSpaceCollapse::BreakSpaces),
            0..5,
        );
    })
    .expect_soft_wrap_opportunity_list(vec![false, false, false, false, true])
    .expect_word_boundary_list(vec![true, true, true, true, true]);
}

#[test]
fn test_break_spaces_across_style_boundary() {
    // The opportunity after a preserved space applies even when the following character is in
    // another style run.
    verify_analysis("A B", |builder| {
        builder.push(
            StyleProperty::WhiteSpaceCollapse(WhiteSpaceCollapse::BreakSpaces),
            0..2,
        );
        builder.push(
            StyleProperty::WhiteSpaceCollapse(WhiteSpaceCollapse::Preserve),
            2..3,
        );
    })
    .expect_soft_wrap_opportunity_list(vec![false, false, true])
    .expect_word_boundary_list(vec![true, true, true]);

    verify_analysis("A B", |builder| {
        builder.push(
            StyleProperty::WhiteSpaceCollapse(WhiteSpaceCollapse::Preserve),
            0..1,
        );
        builder.push(
            StyleProperty::WhiteSpaceCollapse(WhiteSpaceCollapse::BreakSpaces),
            1..3,
        );
    })
    .expect_soft_wrap_opportunity_list(vec![false, false, true])
    .expect_word_boundary_list(vec![true, true, true]);
}

fn soft_wrap_opportunities(text: &str, line_break: LineBreak, language: Option<&str>) -> Vec<bool> {
    verify_analysis(text, |builder| {
        builder.push_default(StyleProperty::LineBreak(line_break));
        builder.push_default(StyleProperty::Locale(
            language.map(|language| Language::parse(language).unwrap()),
        ));
    })
    .soft_wrap_opportunity_list()
}

#[test]
fn test_line_break_strictness() {
    // U+3041 HIRAGANA LETTER SMALL A is a conditional Japanese starter, which may only start a line
    // under `normal` and `loose`. A break before U+2010 HYPHEN after an ideograph is only allowed
    // under `loose`.
    for (line_break, small_kana, hyphen) in [
        (LineBreak::Strict, false, false),
        (LineBreak::Normal, true, false),
        (LineBreak::Loose, true, true),
    ] {
        assert_eq!(
            soft_wrap_opportunities("文ぁ文", line_break, None)[1],
            small_kana,
            "{line_break:?}"
        );
        assert_eq!(
            soft_wrap_opportunities("文‐文", line_break, None)[1],
            hyphen,
            "{line_break:?}"
        );
    }
}

#[test]
fn test_line_break_chinese_japanese_tailoring() {
    // A break before U+301C WAVE DASH under `normal` is only allowed for Chinese or Japanese.
    let text = "文〜文";
    assert!(!soft_wrap_opportunities(text, LineBreak::Normal, None)[1]);
    assert!(!soft_wrap_opportunities(text, LineBreak::Normal, Some("en"))[1]);
    assert!(soft_wrap_opportunities(text, LineBreak::Normal, Some("ja"))[1]);
    assert!(soft_wrap_opportunities(text, LineBreak::Normal, Some("zh-Hant"))[1]);

    // A break before U+FF04 FULLWIDTH DOLLAR SIGN (PR with East Asian Width F) under `loose`
    // is only allowed for Chinese or Japanese.
    let text = "文＄文";
    assert!(!soft_wrap_opportunities(text, LineBreak::Loose, Some("en"))[2]);
    assert!(soft_wrap_opportunities(text, LineBreak::Loose, Some("ja"))[2]);
}

#[test]
fn test_line_break_anywhere() {
    // Opportunities around every character, including those with the GL (U+00A0 NO-BREAK SPACE)
    // and WJ (U+2060 WORD JOINER) classes, and punctuation.
    assert_eq!(
        soft_wrap_opportunities("ab\u{a0}c\u{2060}d/e", LineBreak::Anywhere, None),
        vec![false, true, true, true, true, true, true, true]
    );
    // But not before a combining mark, nor before a mandatory break.
    assert_eq!(
        soft_wrap_opportunities("ae\u{301}\nb", LineBreak::Anywhere, None),
        vec![false, true, false, false, false]
    );
}

#[test]
fn test_line_break_anywhere_overrides_keep_all() {
    verify_analysis("abc", |builder| {
        builder.push_default(StyleProperty::WordBreak(WordBreak::KeepAll));
        builder.push_default(StyleProperty::LineBreak(LineBreak::Anywhere));
    })
    .expect_soft_wrap_opportunity_list(vec![false, true, true]);
}

#[test]
fn test_line_break_override_applies_under_anywhere() {
    let mut test_context = TestContext::default();
    {
        let text = "a b/c";
        let mut builder = test_context.layout_context.ranged_builder(
            &mut test_context.font_context,
            text,
            1.,
            true,
        );
        builder.push_default(StyleProperty::LineBreak(LineBreak::Anywhere));
        builder.set_line_break_override(Some(&|_| Some(false)));
        _ = builder.build(text);
    }
    assert_eq!(test_context.soft_wrap_opportunity_list(), vec![false; 5]);
}

#[test]
fn test_line_break_anywhere_graphemes() {
    // Opportunities are around grapheme clusters, not code points: a Hangul syllable of conjoining
    // jamo, a pair of regional indicators, an emoji with a modifier, and a ZWJ are not split.
    for (text, expected) in [
        ("\u{1100}\u{1161}\u{11A8}x", vec![false, false, false, true]),
        ("🇯🇵🇺🇸", vec![false, false, true, false]),
        ("👍🏽x", vec![false, false, true]),
        ("a\u{200D}b", vec![false, false, true]),
    ] {
        assert_eq!(
            soft_wrap_opportunities(text, LineBreak::Anywhere, None),
            expected,
            "{text:?}"
        );
    }
}

#[test]
fn test_line_break_language_change_under_strict() {
    // The language does not affect `strict` line breaking, so a language change must not create an
    // opportunity: UAX #14 LB14 prohibits a break after an opening bracket followed by spaces.
    let text = "x(  y";
    verify_analysis(text, |builder| {
        builder.push(
            StyleProperty::Locale(Some(Language::parse("en").unwrap())),
            0..3,
        );
        builder.push(
            StyleProperty::Locale(Some(Language::parse("ja").unwrap())),
            3..5,
        );
    })
    .expect_soft_wrap_opportunity_list(vec![false; 5]);
}

#[test]
fn test_line_break_mixed_runs() {
    // The opportunity before a character is decided by that character's configuration.
    verify_analysis("abcdef", |builder| {
        builder.push(StyleProperty::LineBreak(LineBreak::Anywhere), 0..2);
        builder.push(StyleProperty::LineBreak(LineBreak::Anywhere), 4..6);
    })
    .expect_soft_wrap_opportunity_list(vec![false, true, false, false, true, true]);
}
