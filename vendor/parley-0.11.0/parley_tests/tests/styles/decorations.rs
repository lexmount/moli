// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for text decoration style properties.

use crate::test_name;
use crate::util::{TestEnv, samples};
use parley::AlignmentOptions;
use parley::layout::Alignment;
use parley::style::StyleProperty;

// ============================================================================
// Underline Tests
// ============================================================================

#[test]
fn style_underline() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::Underline(true));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());

    env.check_layout_snapshot(&layout);
}

#[test]
fn style_underline_offset_values() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    for offset in [-2.0, 0.0, 2.0, 4.0] {
        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::Underline(true));
        builder.push_default(StyleProperty::UnderlineOffset(Some(offset)));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());

        let name = if offset < 0.0 {
            format!("neg_{}", (-offset) as i32)
        } else {
            format!("pos_{}", offset as i32)
        };
        env.with_name(&name).check_layout_snapshot(&layout);
    }
}

#[test]
fn style_underline_size_values() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    for size in [0.5, 1.0, 2.0, 4.0] {
        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::Underline(true));
        builder.push_default(StyleProperty::UnderlineSize(Some(size)));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());

        let name = format!("size_{}", (size * 10.0) as i32);
        env.with_name(&name).check_layout_snapshot(&layout);
    }
}

#[test]
fn style_underline_across_line_break() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN_MULTILINE;

    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::Underline(true));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());

    env.check_layout_snapshot(&layout);
}

#[test]
fn style_underline_partial_text() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "Hello World Test";

    let mut builder = env.ranged_builder(text);
    // Underline only "World"
    builder.push(StyleProperty::Underline(true), 6..11);
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());

    env.check_layout_snapshot(&layout);
}

// ============================================================================
// Strikethrough Tests
// ============================================================================

#[test]
fn style_strikethrough() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::Strikethrough(true));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());

    env.check_layout_snapshot(&layout);
}

#[test]
fn style_strikethrough_offset_values() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    for offset in [-2.0, 0.0, 2.0, 4.0] {
        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::Strikethrough(true));
        builder.push_default(StyleProperty::StrikethroughOffset(Some(offset)));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());

        let name = if offset < 0.0 {
            format!("neg_{}", (-offset) as i32)
        } else {
            format!("pos_{}", offset as i32)
        };
        env.with_name(&name).check_layout_snapshot(&layout);
    }
}

#[test]
fn style_strikethrough_size_values() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    for size in [0.5, 1.0, 2.0, 4.0] {
        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::Strikethrough(true));
        builder.push_default(StyleProperty::StrikethroughSize(Some(size)));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());

        let name = format!("size_{}", (size * 10.0) as i32);
        env.with_name(&name).check_layout_snapshot(&layout);
    }
}

// ============================================================================
// Combined Decoration Tests
// ============================================================================

#[test]
fn style_underline_and_strikethrough() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::Underline(true));
    builder.push_default(StyleProperty::Strikethrough(true));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());

    env.check_layout_snapshot(&layout);
}
