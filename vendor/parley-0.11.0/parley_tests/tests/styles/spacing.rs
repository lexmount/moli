// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for spacing style properties.

use crate::test_name;
use crate::util::{ColorBrush, TestEnv, samples};
use parley::layout::Alignment;
use parley::style::{LineHeight, StyleProperty};
use parley::{AlignmentOptions, Layout};

/// Helper to build a layout with line height applied
fn build_with_line_height(
    env: &mut TestEnv,
    text: &str,
    line_height: LineHeight,
) -> Layout<ColorBrush> {
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::LineHeight(line_height));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());
    layout
}

/// Helper to build a layout with letter spacing applied
fn build_with_letter_spacing(env: &mut TestEnv, text: &str, spacing: f32) -> Layout<ColorBrush> {
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::LetterSpacing(spacing));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());
    layout
}

/// Helper to build a layout with word spacing applied
fn build_with_word_spacing(env: &mut TestEnv, text: &str, spacing: f32) -> Layout<ColorBrush> {
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::WordSpacing(spacing));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());
    layout
}

// ============================================================================
// LineHeight Tests
// ============================================================================

#[test]
fn style_line_height_absolute() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN_MULTILINE;

    for height in [16.0, 24.0, 32.0, 48.0] {
        let layout = build_with_line_height(&mut env, text, LineHeight::Absolute(height));

        env.with_name(&format!("abs_{height}"))
            .check_layout_snapshot(&layout);
    }
}

#[test]
fn style_line_height_font_relative() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN_MULTILINE;

    for factor in [1.0, 1.2, 1.5, 2.0] {
        let layout = build_with_line_height(&mut env, text, LineHeight::FontSizeRelative(factor));

        // Use underscore in name to avoid dots in filename
        let name = format!("rel_{}", (factor * 10.0) as i32);
        env.with_name(&name).check_layout_snapshot(&layout);
    }
}

#[test]
fn style_line_height_metrics_relative() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN_MULTILINE;

    for factor in [0.8, 1.0, 1.2, 1.5] {
        let layout = build_with_line_height(&mut env, text, LineHeight::MetricsRelative(factor));

        let name = format!("metrics_{}", (factor * 10.0) as i32);
        env.with_name(&name).check_layout_snapshot(&layout);
    }
}

// ============================================================================
// LetterSpacing Tests
// ============================================================================

#[test]
fn style_letter_spacing_positive() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    for spacing in [1.0, 2.0, 4.0, 8.0] {
        let layout = build_with_letter_spacing(&mut env, text, spacing);

        env.with_name(&format!("pos_{spacing}"))
            .check_layout_snapshot(&layout);
    }
}

#[test]
fn style_letter_spacing_negative() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    for spacing in [-1.0, -2.0] {
        let layout = build_with_letter_spacing(&mut env, text, spacing);

        let name = format!("neg_{}", (-spacing) as i32);
        env.with_name(&name).check_layout_snapshot(&layout);
    }
}

#[test]
fn style_letter_spacing_with_ligatures() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LIGATURES;

    // Without letter spacing - ligatures may form
    let layout_no_spacing = build_with_letter_spacing(&mut env, text, 0.0);
    env.with_name("no_spacing")
        .check_layout_snapshot(&layout_no_spacing);

    // With letter spacing - ligatures should break
    let layout_with_spacing = build_with_letter_spacing(&mut env, text, 2.0);
    env.with_name("with_spacing")
        .check_layout_snapshot(&layout_with_spacing);
}

// ============================================================================
// WordSpacing Tests
// ============================================================================

#[test]
fn style_word_spacing_positive() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::SPACED;

    for spacing in [2.0, 4.0, 8.0, 16.0] {
        let layout = build_with_word_spacing(&mut env, text, spacing);

        env.with_name(&format!("pos_{spacing}"))
            .check_layout_snapshot(&layout);
    }
}

#[test]
fn style_word_spacing_negative() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::SPACED;

    for spacing in [-2.0, -4.0] {
        let layout = build_with_word_spacing(&mut env, text, spacing);

        let name = format!("neg_{}", (-spacing) as i32);
        env.with_name(&name).check_layout_snapshot(&layout);
    }
}

// ============================================================================
// Run Splitting Tests
// ============================================================================

#[test]
fn spacing_causes_style_run_breaks() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "foo bar";
    let mut builder = env.ranged_builder(text);
    builder.push(StyleProperty::WordSpacing(2.0), 3..text.len());
    builder.push(StyleProperty::LetterSpacing(1.5), 3..text.len());

    let mut layout = builder.build(text);
    layout.break_all_lines(None);

    // Collect all runs from the layout
    let runs: Vec<_> = layout.lines().flat_map(|line| line.runs()).collect();

    assert_eq!(
        runs.len(),
        2,
        "expected two runs after spacing property changes"
    );

    // Verify text ranges are split correctly
    assert_eq!(&text[runs[0].text_range()], "foo",);
    assert_eq!(&text[runs[1].text_range()], " bar",);

    env.with_name("spacing_style_run_breaks")
        .check_layout_snapshot(&layout);
}

// ============================================================================
// Grapheme tests
// ============================================================================

#[test]
fn spacing_and_multi_shaped_cluster_graphemes() {
    let mut env = TestEnv::new(test_name!(), None);

    // The combining acute accent `U+0301` combines with the character to form a grapheme, including
    // if that previous character is a space. Our Roboto font doesn't carry a glyph for `sp+U+0301`
    // or `h+U+0301`, so both become two shaped clusters, but form one grapheme.
    //
    // Compare with the following HTML.
    //
    // ```html
    // <p style="letter-spacing: 10px; background: #eee; width: fit-content;">a &#x0301;bh&#x0301;d</p>
    // <p style="word-spacing: 10px; background: #eee; width: fit-content;">a &#x0301;bh&#x0301;d</p>
    // ```
    let text = "a \u{0301}bh\u{0301}d";

    {
        let mut builder = env.ranged_builder(text);
        builder.push(StyleProperty::LetterSpacing(10.), 0..text.len());

        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        env.with_name("letter").check_layout_snapshot(&layout);
    }

    {
        let mut builder = env.ranged_builder(text);
        builder.push(StyleProperty::WordSpacing(10.), 0..text.len());

        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        env.with_name("word").check_layout_snapshot(&layout);
    }
}
