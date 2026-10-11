// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for the `vertical-align` style property.

use crate::test_name;
use crate::util::TestEnv;
use parley::layout::Alignment;
use parley::style::StyleProperty;
use parley::{AlignmentOptions, InlineBox, InlineBoxKind, VerticalAlign};

/// Every keyword, applied to a span in the middle of a line of text.
#[test]
fn vertical_align_keywords() {
    let mut env = TestEnv::new(test_name!(), None);

    for (align, name) in [
        (VerticalAlign::BASELINE, "baseline"),
        (VerticalAlign::SUB, "sub"),
        (VerticalAlign::SUPER, "super"),
        (VerticalAlign::TEXT_TOP, "text_top"),
        (VerticalAlign::TEXT_BOTTOM, "text_bottom"),
        (VerticalAlign::MIDDLE, "middle"),
        (VerticalAlign::TOP, "top"),
        (VerticalAlign::BOTTOM, "bottom"),
        (VerticalAlign::length(6.), "length_pos"),
        (VerticalAlign::length(-6.), "length_neg"),
    ] {
        let mut builder = env.tree_builder();
        builder.push_style_modification_span(&[StyleProperty::FontSize(24.)]);
        builder.push_text("Hx");
        builder.push_style_modification_span(&[
            StyleProperty::FontSize(12.),
            StyleProperty::VerticalAlign(align),
        ]);
        builder.push_text("Hx");
        builder.pop_style_span();
        builder.push_text("Hx");
        builder.pop_style_span();
        let (mut layout, _) = builder.build();
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());

        env.with_name(name).check_layout_snapshot(&layout);
    }
}

/// Nested parent-relative shifts compound: `super` inside `super`.
#[test]
fn vertical_align_nested_super() {
    let mut env = TestEnv::new(test_name!(), None);

    let mut builder = env.tree_builder();
    builder.push_style_modification_span(&[StyleProperty::FontSize(24.)]);
    builder.push_text("x");
    builder.push_style_modification_span(&[
        StyleProperty::FontSize(16.),
        StyleProperty::VerticalAlign(VerticalAlign::SUPER),
    ]);
    builder.push_text("2");
    builder.push_style_modification_span(&[
        StyleProperty::FontSize(10.),
        StyleProperty::VerticalAlign(VerticalAlign::SUPER),
    ]);
    builder.push_text("n");
    builder.pop_style_span();
    builder.pop_style_span();
    builder.pop_style_span();
    let (mut layout, _) = builder.build();
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());

    env.check_layout_snapshot(&layout);
}

/// A large-font ancestor span with no text of its own on the line still contributes its own
/// inline box (CSS 2.2 §10.8.1), so the line is as tall as its line-height.
#[test]
fn vertical_align_ancestor_without_text() {
    let mut env = TestEnv::new(test_name!(), None);

    let mut builder = env.tree_builder();
    builder.push_style_modification_span(&[StyleProperty::FontSize(40.)]);
    builder.push_style_modification_span(&[StyleProperty::FontSize(12.)]);
    builder.push_text("Small text in a big span");
    builder.pop_style_span();
    builder.pop_style_span();
    let (mut layout, _) = builder.build();
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());

    env.check_layout_snapshot(&layout);
}

/// Inline boxes take part in alignment too, relative to the span containing them.
#[test]
fn vertical_align_inline_boxes() {
    let mut env = TestEnv::new(test_name!(), None);

    for (align, name) in [
        (VerticalAlign::BASELINE, "baseline"),
        (VerticalAlign::MIDDLE, "middle"),
        (VerticalAlign::TEXT_TOP, "text_top"),
        (VerticalAlign::TOP, "top"),
        (VerticalAlign::BOTTOM, "bottom"),
    ] {
        let mut builder = env.tree_builder();
        builder.push_style_modification_span(&[StyleProperty::FontSize(24.)]);
        builder.push_text("Hx");
        builder.push_inline_box(InlineBox {
            id: 0,
            kind: InlineBoxKind::InFlow,
            index: 0,
            width: 12.,
            height: 12.,
            baseline: None,
            vertical_align: align,
        });
        builder.push_text("Hx");
        builder.pop_style_span();
        let (mut layout, _) = builder.build();
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());

        env.with_name(name).check_layout_snapshot(&layout);
    }
}

/// `top`/`bottom` subtrees taller than the root subtree grow the line box without moving the
/// root baseline relative to the root content.
#[test]
fn vertical_align_tall_top_bottom() {
    let mut env = TestEnv::new(test_name!(), None);

    for (align, name) in [
        (VerticalAlign::TOP, "top"),
        (VerticalAlign::BOTTOM, "bottom"),
    ] {
        let mut builder = env.tree_builder();
        builder.push_style_modification_span(&[StyleProperty::FontSize(12.)]);
        builder.push_text("Hx");
        builder.push_style_modification_span(&[
            StyleProperty::FontSize(36.),
            StyleProperty::VerticalAlign(align),
        ]);
        builder.push_text("Hx");
        builder.pop_style_span();
        builder.push_text("Hx");
        builder.pop_style_span();
        let (mut layout, _) = builder.build();
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());

        env.with_name(name).check_layout_snapshot(&layout);
    }
}
