// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Content width tests.

use crate::util::TestEnv;
use crate::{test_name, util::ColorBrush};
use parley::{
    Alignment, AlignmentOptions, ContentWidths, InlineBox, InlineBoxKind, Layout, LineBreak,
    StyleProperty, TextWrapMode, VerticalAlign, WhiteSpaceCollapse,
};

/// Checks that calculated content widths agree with actual line breaking.
///
/// Leaves the layout broken at the min-content width.
fn assert_content_widths_match_layout(layout: &mut Layout<ColorBrush>) -> ContentWidths {
    let widths = layout.calculate_content_widths();

    // Note: if the line breaker and content width calculation performed the same floating point
    // operations in the same order, we could in theory guarantee exact equality.
    layout.break_all_lines(None);
    assert!(
        (layout.width() - widths.max).abs() < 1e-3,
        "Unconstrained layout width {} should equal the max content width {}",
        layout.width(),
        widths.max
    );

    layout.break_all_lines(Some(widths.min));
    assert!(
        (layout.width() - widths.min).abs() < 1e-3,
        "Layout width {} at the min content width should equal it ({})",
        layout.width(),
        widths.min
    );

    widths
}

/// The width of `text` laid out on a single line.
fn single_line_width(env: &mut TestEnv, text: &str) -> f32 {
    let mut layout = env.ranged_builder(text).build(text);
    layout.break_all_lines(None);
    layout.width()
}

#[test]
fn content_widths_multiple_trailing_spaces() {
    let mut env = TestEnv::new(test_name!(), None);

    // The widest word is followed by three spaces. When the line breaks after them, they all hang,
    // so none of them should count towards the min-content width. The middle space has a
    // different font size, so the three spaces are spread over three runs.
    let text = "AA BB CCC   DD EE";
    let mut builder = env.ranged_builder(text);
    builder.push(StyleProperty::FontSize(15.9), 10..11);
    let mut layout = builder.build(text);

    let widths = assert_content_widths_match_layout(&mut layout);
    assert_eq!(
        layout.len(),
        5,
        "one word per line at the min content width"
    );

    let word_width = single_line_width(&mut env, "CCC");
    assert!(
        (widths.min - word_width).abs() < 1e-3,
        "Min content width {} should equal the widest word's width {}",
        widths.min,
        word_width
    );
}

#[test]
fn content_widths_partially_hanging_atom() {
    let mut env = TestEnv::new(test_name!(), None);

    // A prepend character followed by a space forms a single grapheme, and so a single atom, of
    // two shaped clusters. Only the space's cluster hangs.
    for (text, word) in [
        // LTR
        ("a\u{0D4E} b c", "a\u{0D4E}"),
        // RTL
        ("\u{0710}\u{070F} \u{0712} \u{0713}", "\u{0710}\u{070F}"),
    ] {
        let mut layout = env.ranged_builder(text).build(text);
        let widths = assert_content_widths_match_layout(&mut layout);
        assert_eq!(
            layout.len(),
            3,
            "one word per line at the min content width"
        );

        let word_width = single_line_width(&mut env, word);
        assert!(
            (widths.min - word_width).abs() < 1e-3,
            "Min content width {} should equal the widest word's width {} (excluding the space)",
            widths.min,
            word_width
        );

        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::LetterSpacing(2.));
        builder.push_default(StyleProperty::WordSpacing(3.));
        let mut layout = builder.build(text);
        assert_content_widths_match_layout(&mut layout);
        assert_eq!(
            layout.len(),
            3,
            "one word per line at the min content width"
        );
    }
}

#[test]
fn content_widths_partially_hanging_atom_mixed_collapse_modes() {
    let mut env = TestEnv::new(test_name!(), None);

    // The "\u{0D4E} " in the following is a grapheme (and thus an atom). Whether the space hangs
    // conditionally follows the space's style.
    let text = "a\u{0D4E} ";
    let word_width = single_line_width(&mut env, "a\u{0D4E}");
    let word_with_space_width = single_line_width(&mut env, text);
    for (prepend_mode, space_mode, expected_max) in [
        (
            WhiteSpaceCollapse::Preserve,
            WhiteSpaceCollapse::Collapse,
            word_width,
        ),
        (
            WhiteSpaceCollapse::Collapse,
            WhiteSpaceCollapse::Preserve,
            word_with_space_width,
        ),
    ] {
        let mut builder = env.ranged_builder(text);
        builder.push(StyleProperty::WhiteSpaceCollapse(prepend_mode), 0..4);
        builder.push(StyleProperty::WhiteSpaceCollapse(space_mode), 4..5);
        let mut layout = builder.build(text);

        let widths = assert_content_widths_match_layout(&mut layout);
        assert!(
            (widths.max - expected_max).abs() < 1e-3,
            "Max content width {} should be {expected_max} for {prepend_mode:?} and {space_mode:?}",
            widths.max,
        );
        assert!(
            (widths.min - word_width).abs() < 1e-3,
            "Min content width {} should be {word_width} for {prepend_mode:?} and {space_mode:?}",
            widths.min,
        );

        // The same holds with spacing applied.
        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::LetterSpacing(2.));
        builder.push(StyleProperty::WhiteSpaceCollapse(prepend_mode), 0..4);
        builder.push(StyleProperty::WhiteSpaceCollapse(space_mode), 4..5);
        let mut layout = builder.build(text);
        assert_content_widths_match_layout(&mut layout);
    }
}

#[test]
fn content_widths_line_break_anywhere() {
    let mut env = TestEnv::new(test_name!(), None);

    // With `line-break: anywhere` there is a soft wrap opportunity between every character, so
    // the min-content width is the width of the widest character.
    let text = "WWW WW";
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::LineBreak(LineBreak::Anywhere));
    let mut layout = builder.build(text);

    let widths = assert_content_widths_match_layout(&mut layout);
    assert_eq!(layout.len(), 5, "one character per line (the space hangs)");

    let char_width = single_line_width(&mut env, "W");
    assert!(
        (widths.min - char_width).abs() < 1e-3,
        "Min content width {} should equal a single character's width {}",
        widths.min,
        char_width
    );
}

#[test]
fn content_widths_mixed_direction() {
    let mut env = TestEnv::new(test_name!(), None);

    // Whitespace only hangs if it ends up at the line's end edge after bidi reordering. The
    // content widths should agree with the line breaker on which whitespace hangs.
    for text in [
        "abc ااا ببب def",
        "ااا abc def ببب",
        "abc ااا\u{a0}ببب   def",
    ] {
        let mut layout = env.ranged_builder(text).build(text);
        assert_content_widths_match_layout(&mut layout);
    }
}

#[test]
fn inbox_content_width() {
    let mut env = TestEnv::new(test_name!(), None);

    {
        let text = "Hello world!";
        let mut builder = env.ranged_builder(text);
        builder.push_inline_box(InlineBox {
            id: 0,
            kind: InlineBoxKind::InFlow,
            index: 3,
            width: 100.0,
            height: 10.0,
            baseline: None,
            vertical_align: VerticalAlign::BASELINE,
        });
        let mut layout = builder.build(text);
        let ContentWidths {
            min: min_content_width,
            ..
        } = layout.calculate_content_widths();
        layout.break_all_lines(Some(min_content_width));
        layout.align(Alignment::Start, AlignmentOptions::default());

        env.with_name("full_width").check_layout_snapshot(&layout);
    }

    {
        let text = "A ";
        let mut builder = env.ranged_builder(text);
        builder.push_inline_box(InlineBox {
            id: 0,
            kind: InlineBoxKind::InFlow,
            index: 2,
            width: 10.0,
            height: 10.0,
            baseline: None,
            vertical_align: VerticalAlign::BASELINE,
        });
        let mut layout = builder.build(text);
        let ContentWidths {
            max: max_content_width,
            ..
        } = layout.calculate_content_widths();
        layout.break_all_lines(Some(max_content_width));
        layout.align(Alignment::Start, AlignmentOptions::default());

        assert!(
            layout.width() <= max_content_width,
            "Layout should never be wider than the max content width"
        );

        env.with_name("trailing_whitespace")
            .check_layout_snapshot(&layout);
    }
}

#[test]
fn content_widths_max_floored_by_min() {
    let mut env = TestEnv::new(test_name!(), None);

    // A negative-width inline box (e.g. an inline-block with a negative margin) reduces the
    // max-content width, but the max-content width can never be smaller than the min-content
    // width.
    let text = "";
    let mut builder = env.ranged_builder(text);
    for (id, width) in [(0, 100.0), (1, -50.0)] {
        builder.push_inline_box(InlineBox {
            id,
            kind: InlineBoxKind::InFlow,
            index: 0,
            width,
            height: 10.0,
            baseline: None,
            vertical_align: VerticalAlign::BASELINE,
        });
    }
    let layout = builder.build(text);

    let widths = layout.calculate_content_widths();
    assert_eq!(widths.min, 100.0);
    assert_eq!(widths.max, 100.0);
}

#[test]
fn content_widths_trailing_whitespace_by_collapse_mode() {
    let mut env = TestEnv::new(test_name!(), None);

    // Following CSS Text 4 § 4.3.2, trailing whitespace before a forced line break hangs
    // conditionally when whitespace is preserved: it counts towards the max-content width. When
    // whitespace is collapsed, it hangs unconditionally, so it never counts.
    //
    // Ideographic spaces are used as they are not collapsible: whitespace processing would have
    // removed other spaces before the forced break.
    let text = "AA\u{3000}\u{3000}\u{3000}\nB\u{3000}\u{3000}";
    let word_width = single_line_width(&mut env, "AA");
    let word_with_spaces_width = single_line_width(&mut env, "AA\u{3000}\u{3000}\u{3000}");
    for (mode, wrap_mode, expected_max) in [
        (
            WhiteSpaceCollapse::Preserve,
            TextWrapMode::Wrap,
            word_with_spaces_width,
        ),
        (WhiteSpaceCollapse::Collapse, TextWrapMode::Wrap, word_width),
        (
            WhiteSpaceCollapse::PreserveBreaks,
            TextWrapMode::Wrap,
            word_width,
        ),
        (
            WhiteSpaceCollapse::Collapse,
            TextWrapMode::NoWrap,
            word_width,
        ),
    ] {
        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::WhiteSpaceCollapse(mode));
        builder.push_default(StyleProperty::TextWrapMode(wrap_mode));
        let mut layout = builder.build(text);

        let widths = assert_content_widths_match_layout(&mut layout);
        assert!(
            (widths.max - expected_max).abs() < 1e-3,
            "Max content width {} should be {expected_max} for {mode:?} and {wrap_mode:?}",
            widths.max,
        );
        assert!(
            (widths.min - word_width).abs() < 1e-3,
            "Min content width {} should be {word_width} for {mode:?} and {wrap_mode:?}",
            widths.min,
        );
    }

    // The ideographic spaces survive the tree builder's whitespace collapsing.
    let mut builder = env.tree_builder();
    builder.push_style_modification_span(&[StyleProperty::WhiteSpaceCollapse(
        WhiteSpaceCollapse::PreserveBreaks,
    )]);
    builder.push_text(text);
    let (mut layout, _) = builder.build();
    let widths = assert_content_widths_match_layout(&mut layout);
    assert!(
        (widths.max - word_width).abs() < 1e-3,
        "Max content width {} should be the widest word's width {word_width}",
        widths.max,
    );
}
