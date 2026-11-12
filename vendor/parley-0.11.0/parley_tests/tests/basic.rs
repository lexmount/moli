// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Basic (and assorted) layout tests.

use crate::util::TestEnv;
use crate::{test_name, util::ColorBrush};
use parley::{
    Alignment, AlignmentOptions, BreakReason, ContentWidths, FontFamily, FontWeight, InlineBox,
    InlineBoxKind, Layout, LineHeight, PositionedLayoutItem, StyleProperty, TextStyle,
    TextWrapMode, VerticalAlign, WhiteSpaceCollapse,
};
use peniko::color::{AlphaColor, Srgb, palette};
use peniko::kurbo::Size;

#[test]
fn plain_multiline_text() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "Hello world!\nLine 2\nLine 4";
    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());

    env.check_layout_snapshot(&layout);
}

#[test]
fn unicode_separators_break_lines() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "A\u{2028}B\u{2029}C";
    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);
    layout.break_all_lines(None);

    assert_eq!(layout.len(), 3, "expected 3 lines from U+2028 and U+2029");
    assert_eq!(
        layout.get(0).unwrap().break_reason(),
        BreakReason::Explicit,
        "expected U+2028 to cause an explicit break"
    );
    assert_eq!(
        layout.get(1).unwrap().break_reason(),
        BreakReason::Explicit,
        "expected U+2029 to cause an explicit break"
    );
    assert_ne!(
        layout.get(2).unwrap().break_reason(),
        BreakReason::Explicit,
        "did not expect a trailing explicit break"
    );
}

#[test]
fn placing_inboxes() {
    let mut env = TestEnv::new(test_name!(), None);

    for (position, test_case_name) in [
        (0, "start"),
        (3, "in_word"),
        (12, "end_nl"),
        (13, "start_nl"),
    ] {
        let text = "Hello world!\nLine 2\nLine 4";
        let mut builder = env.ranged_builder(text);
        builder.push_inline_box(InlineBox {
            id: 0,
            kind: InlineBoxKind::InFlow,
            index: position,
            width: 10.0,
            height: 10.0,
            baseline: None,
            vertical_align: VerticalAlign::BASELINE,
        });
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());
        env.with_name(test_case_name).check_layout_snapshot(&layout);
    }
}

#[test]
fn only_inboxes_wrap() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "";
    let mut builder = env.ranged_builder(text);
    for id in 0..10 {
        builder.push_inline_box(InlineBox {
            id,
            kind: InlineBoxKind::InFlow,
            index: 0,
            width: 10.0,
            height: 10.0,
            baseline: None,
            vertical_align: VerticalAlign::BASELINE,
        });
    }
    let mut layout = builder.build(text);
    layout.break_all_lines(Some(40.0));
    layout.align(Alignment::Center, AlignmentOptions::default());

    env.check_layout_snapshot(&layout);
}

#[test]
fn full_width_inbox() {
    let mut env = TestEnv::new(test_name!(), None);

    for (width, test_case_name) in [(99., "smaller"), (100., "exact"), (101., "larger")] {
        let text = "ABC";
        let mut builder = env.ranged_builder(text);
        builder.push_inline_box(InlineBox {
            id: 0,
            kind: InlineBoxKind::InFlow,
            index: 1,
            width: 10.,
            height: 10.0,
            baseline: None,
            vertical_align: VerticalAlign::BASELINE,
        });
        builder.push_inline_box(InlineBox {
            id: 1,
            kind: InlineBoxKind::InFlow,
            index: 1,
            width,
            height: 10.0,
            baseline: None,
            vertical_align: VerticalAlign::BASELINE,
        });
        builder.push_inline_box(InlineBox {
            id: 2,
            kind: InlineBoxKind::InFlow,
            index: 2,
            width,
            height: 10.0,
            baseline: None,
            vertical_align: VerticalAlign::BASELINE,
        });
        let mut layout = builder.build(text);
        layout.break_all_lines(Some(100.));
        layout.align(Alignment::Start, AlignmentOptions::default());
        env.with_name(test_case_name).check_layout_snapshot(&layout);
    }
}

#[test]
fn inbox_separated_by_whitespace() {
    let mut env = TestEnv::new(test_name!(), None);

    let mut builder = env.tree_builder();
    builder.push_inline_box(InlineBox {
        id: 0,
        kind: InlineBoxKind::InFlow,
        index: 0,
        width: 10.,
        height: 10.0,
        baseline: None,
        vertical_align: VerticalAlign::BASELINE,
    });
    builder.push_text(" ");
    builder.push_inline_box(InlineBox {
        id: 1,
        kind: InlineBoxKind::InFlow,
        index: 1,
        width: 10.0,
        height: 10.0,
        baseline: None,
        vertical_align: VerticalAlign::BASELINE,
    });
    builder.push_text(" ");
    builder.push_inline_box(InlineBox {
        id: 2,
        kind: InlineBoxKind::InFlow,
        index: 2,
        width: 10.0,
        height: 10.0,
        baseline: None,
        vertical_align: VerticalAlign::BASELINE,
    });
    builder.push_text(" ");
    builder.push_inline_box(InlineBox {
        id: 3,
        kind: InlineBoxKind::InFlow,
        index: 3,
        width: 10.0,
        height: 10.0,
        baseline: None,
        vertical_align: VerticalAlign::BASELINE,
    });
    let (mut layout, _text) = builder.build();
    layout.break_all_lines(Some(100.));
    layout.align(Alignment::Start, AlignmentOptions::default());
    env.check_layout_snapshot(&layout);
}

/// The `baseline` field of an inline box should align the box's baseline with the text baseline.
///
/// The rendered snapshots draw the box's baseline (when set) as a horizontal line, which should
/// always line up with the baseline of the surrounding text.
#[test]
fn inbox_with_baseline() {
    let mut env = TestEnv::new(test_name!(), None);

    // The box is 30px tall. We vary where its baseline sits within the box:
    // - `top`: baseline at the top, so the box hangs below the text baseline.
    // - `middle`: baseline in the middle of the box.
    // - `bottom`: baseline at the bottom, equivalent to not specifying a baseline.
    for (baseline, test_case_name) in [(0.0, "top"), (15.0, "middle"), (30.0, "bottom")] {
        let text = "Hello world!";
        let mut builder = env.ranged_builder(text);
        builder.push_inline_box(InlineBox {
            id: 0,
            kind: InlineBoxKind::InFlow,
            index: 5,
            width: 20.0,
            height: 30.0,
            baseline: Some(baseline),
            vertical_align: VerticalAlign::BASELINE,
        });
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());
        env.with_name(test_case_name).check_layout_snapshot(&layout);
    }
}

/// Inline boxes with differing heights but matching baselines should all align to the same text
/// baseline.
#[test]
fn inboxes_with_matching_baselines() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "AxByC";
    let mut builder = env.ranged_builder(text);
    // A short box and a tall box, both with their baselines 10px from the top.
    builder.push_inline_box(InlineBox {
        id: 0,
        kind: InlineBoxKind::InFlow,
        index: 1,
        width: 15.0,
        height: 15.0,
        baseline: Some(10.0),
        vertical_align: VerticalAlign::BASELINE,
    });
    builder.push_inline_box(InlineBox {
        id: 1,
        kind: InlineBoxKind::InFlow,
        index: 3,
        width: 15.0,
        height: 40.0,
        baseline: Some(10.0),
        vertical_align: VerticalAlign::BASELINE,
    });
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());
    env.check_layout_snapshot(&layout);
}

/// A box that mostly sits above the baseline (large ascent) and a box that mostly sits below the
/// baseline (large descent) on the same line. The line height must grow to fit the combined
/// ascent of the first box and descent of the second box, even though neither box on its own is
/// as tall as that sum.
#[test]
fn inboxes_with_large_ascent_and_descent() {
    let mut env = TestEnv::new(test_name!(), None);

    // Surround the line containing the boxes with plain text lines, so we can see how the
    // adjacent lines are positioned relative to the (much taller) box line.
    let text = "Line above\nAxB\nLine below";
    let box_line_start = text.find("AxB").unwrap();
    let mut builder = env.ranged_builder(text);
    // Large ascent: the baseline is near the bottom of the box, so most of it is above the
    // baseline.
    builder.push_inline_box(InlineBox {
        id: 0,
        kind: InlineBoxKind::InFlow,
        index: box_line_start + 1,
        width: 15.0,
        height: 40.0,
        baseline: Some(38.0),
        vertical_align: VerticalAlign::BASELINE,
    });
    // Large descent: the baseline is near the top of the box, so most of it is below the baseline.
    builder.push_inline_box(InlineBox {
        id: 1,
        kind: InlineBoxKind::InFlow,
        index: box_line_start + 2,
        width: 15.0,
        height: 40.0,
        baseline: Some(2.0),
        vertical_align: VerticalAlign::BASELINE,
    });
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());

    // The middle line (containing the boxes) should be tall enough to contain the first box's
    // ascent (38px) plus the second box's descent (40 - 2 = 38px).
    let box_line = layout.get(1).unwrap();
    assert!(
        box_line.metrics().line_height >= 76.0,
        "expected line height of at least 76px to fit the combined ascent and descent, got {}",
        box_line.metrics().line_height
    );

    env.check_layout_snapshot(&layout);
}

#[test]
fn inbox_below_baseline_keeps_grid() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "AAAA\nBBBB\nCCCC";
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::LineHeight(LineHeight::Absolute(60.0)));
    builder.push_inline_box(InlineBox {
        id: 0,
        kind: InlineBoxKind::InFlow,
        index: 7, // between the B's on the middle line
        width: 20.0,
        height: 15.0,
        baseline: Some(0.0),
        vertical_align: VerticalAlign::BASELINE,
    });
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());

    // Every line has the same 60px line height, so consecutive baselines must be exactly 60px
    // apart. The inline box on the middle line fits within the line and must not disturb this.
    let baselines: Vec<f32> = (0..layout.len())
        .map(|i| layout.get(i).unwrap().metrics().baseline)
        .collect();
    assert_eq!(
        baselines[1] - baselines[0],
        60.0,
        "middle line baseline off-grid: {baselines:?}"
    );
    assert_eq!(
        baselines[2] - baselines[1],
        60.0,
        "bottom line baseline off-grid: {baselines:?}"
    );

    env.check_layout_snapshot(&layout);
}

#[test]
fn trailing_whitespace_ltr() {
    let mut env = TestEnv::new(test_name!(), None);

    {
        let text = "AAA BBB";
        let builder = env.ranged_builder(text);
        let mut layout = builder.build(text);
        layout.break_all_lines(Some(45.));
        layout.align(Alignment::Start, AlignmentOptions::default());

        env.with_name("soft_wrap").check_layout_snapshot(&layout);
    }

    {
        let text = "AAA \nBBB";
        let builder = env.ranged_builder(text);
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());

        env.with_name("hard_wrap").check_layout_snapshot(&layout);
    }
}

#[test]
fn trailing_whitespace_rtl() {
    let mut env = TestEnv::new(test_name!(), None);

    {
        let text = "بببب ااااا";
        let builder = env.ranged_builder(text);
        let mut layout = builder.build(text);
        layout.break_all_lines(Some(45.));
        layout.align(Alignment::Start, AlignmentOptions::default());

        env.with_name("soft_wrap").check_layout_snapshot(&layout);
    }

    {
        let text = "بببب \nااااا";
        let builder = env.ranged_builder(text);
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());

        env.with_name("hard_wrap").check_layout_snapshot(&layout);
    }
}

#[test]
fn trailing_whitespace_bidi() {
    let mut env = TestEnv::new(test_name!(), None);

    {
        for (text, test_case_name) in [
            ("AAA ااااا", "soft_wrap_ltr_rtl"),
            ("بببب BBB", "soft_wrap_rtl_ltr"),
        ] {
            let builder = env.ranged_builder(text);
            let mut layout = builder.build(text);
            layout.break_all_lines(Some(45.));
            layout.align(Alignment::Start, AlignmentOptions::default());

            env.with_name(test_case_name).check_layout_snapshot(&layout);
        }
    }

    {
        for (text, test_case_name) in [
            ("AAA \nااااا", "hard_wrap_ltr_rtl"),
            ("بببب \nBBB", "hard_wrap_rtl_ltr"),
        ] {
            let builder = env.ranged_builder(text);
            let mut layout = builder.build(text);
            layout.break_all_lines(None);
            layout.align(Alignment::Start, AlignmentOptions::default());

            env.with_name(test_case_name).check_layout_snapshot(&layout);
        }
    }
}

#[test]
fn leading_whitespace() {
    let mut env = TestEnv::new(test_name!(), None);

    for (mode, test_case_name) in [
        (WhiteSpaceCollapse::Preserve, "preserve"),
        (WhiteSpaceCollapse::Collapse, "collapse"),
    ] {
        let mut builder = env.tree_builder();
        builder.push_style_modification_span(&[StyleProperty::WhiteSpaceCollapse(mode)]);
        builder.push_text("Line 1");
        builder.push_style_modification_span(&[StyleProperty::WhiteSpaceCollapse(
            WhiteSpaceCollapse::Preserve,
        )]);
        builder.push_text("\n");
        builder.pop_style_span();
        builder.push_text("  Line 2");
        let (mut layout, _) = builder.build();
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());
        env.with_name(test_case_name).check_layout_snapshot(&layout);
    }
}

#[test]
fn collapsible_whitespace_crosses_spans_and_inline_boxes() {
    let mut env = TestEnv::new(test_name!(), None);

    let inline_box = || InlineBox {
        id: 0,
        index: 0,
        width: 10.,
        height: 10.,
        baseline: None,
        kind: InlineBoxKind::InFlow,
        vertical_align: VerticalAlign::BASELINE,
    };

    // Trailing whitespace of a span collapses with the content following the span.
    let mut builder = env.tree_builder();
    builder.push_style_modification_span(&[StyleProperty::WhiteSpaceCollapse(
        WhiteSpaceCollapse::Collapse,
    )]);
    builder.push_style_modification_span(None);
    builder.push_text("Hello ");
    builder.pop_style_span();
    builder.push_text("world");
    assert_eq!(builder.build().1, "Hello world");

    // A whitespace-only span collapses into a single space.
    let mut builder = env.tree_builder();
    builder.push_style_modification_span(&[StyleProperty::WhiteSpaceCollapse(
        WhiteSpaceCollapse::Collapse,
    )]);
    builder.push_text("Hello");
    builder.push_style_modification_span(None);
    builder.push_text("  ");
    builder.pop_style_span();
    builder.push_text("  world");
    assert_eq!(builder.build().1, "Hello world");

    // Whitespace collapses across nested span boundaries.
    let mut builder = env.tree_builder();
    builder.push_style_modification_span(&[StyleProperty::WhiteSpaceCollapse(
        WhiteSpaceCollapse::Collapse,
    )]);
    builder.push_text("a ");
    builder.push_style_modification_span(None);
    builder.push_style_modification_span(None);
    builder.push_text(" ");
    builder.pop_style_span();
    builder.pop_style_span();
    builder.push_text(" b");
    assert_eq!(builder.build().1, "a b");

    // Whitespace surrounding an inline box collapses into a single space on each side. The box is
    // not whitespace, so the two spaces do not collapse with each other.
    let mut builder = env.tree_builder();
    builder.push_style_modification_span(&[StyleProperty::WhiteSpaceCollapse(
        WhiteSpaceCollapse::Collapse,
    )]);
    builder.push_text("a  ");
    builder.push_inline_box(inline_box());
    builder.push_text("  b");
    let (layout, text) = builder.build();
    assert_eq!(text, "a  b");
    assert_eq!(layout.inline_boxes().next().unwrap().index, 2);

    // Whitespace at the start and end of the text is removed, including in enclosing spans.
    let mut builder = env.tree_builder();
    builder.push_style_modification_span(&[StyleProperty::WhiteSpaceCollapse(
        WhiteSpaceCollapse::Collapse,
    )]);
    builder.push_style_modification_span(None);
    builder.push_text("  ");
    builder.pop_style_span();
    builder.push_text("a");
    builder.push_style_modification_span(None);
    builder.push_text("  ");
    builder.pop_style_span();
    assert_eq!(builder.build().1, "a");

    // An inline box is not whitespace: whitespace before it is kept.
    let mut builder = env.tree_builder();
    builder.push_style_modification_span(&[StyleProperty::WhiteSpaceCollapse(
        WhiteSpaceCollapse::Collapse,
    )]);
    builder.push_text("a ");
    builder.push_inline_box(inline_box());
    let (layout, text) = builder.build();
    assert_eq!(text, "a ");
    assert_eq!(layout.inline_boxes().next().unwrap().index, 2);

    // Collapsible whitespace on either side of a preserved segment break (e.g. a `<br>`) is
    // removed, while whitespace following a preserved space is kept.
    let mut builder = env.tree_builder();
    builder.push_style_modification_span(&[StyleProperty::WhiteSpaceCollapse(
        WhiteSpaceCollapse::Collapse,
    )]);
    builder.push_text("a ");
    builder.push_style_modification_span(&[StyleProperty::WhiteSpaceCollapse(
        WhiteSpaceCollapse::Preserve,
    )]);
    builder.push_text("\n");
    builder.pop_style_span();
    builder.push_text(" b ");
    builder.push_style_modification_span(&[StyleProperty::WhiteSpaceCollapse(
        WhiteSpaceCollapse::Preserve,
    )]);
    builder.push_text(" ");
    builder.pop_style_span();
    builder.push_text(" c");
    assert_eq!(builder.build().1, "a\nb   c");

    // An out-of-flow box is not content: it neither splits a word nor prevents the whitespace on
    // either side of it from collapsing together.
    let out_of_flow_box = |width| InlineBox {
        kind: InlineBoxKind::OutOfFlow,
        width,
        height: width,
        ..inline_box()
    };
    for split in 0..=7 {
        let mut builder = env.tree_builder();
        builder.push_style_modification_span(&[StyleProperty::WhiteSpaceCollapse(
            WhiteSpaceCollapse::Collapse,
        )]);
        builder.push_text(&"XXXXXXX"[..split]);
        builder.push_inline_box(out_of_flow_box(100.));
        builder.push_text(&"XXXXXXX"[split..]);
        let (mut layout, text) = builder.build();
        assert_eq!(text, "XXXXXXX");
        assert_eq!(layout.inline_boxes().next().unwrap().index, split);
        layout.break_all_lines(Some(1.));
        assert_eq!(layout.len(), 1, "box at {split}");
    }
    for suffix in ["  b", "\nb"] {
        let mut builder = env.tree_builder();
        builder.push_style_modification_span(&[StyleProperty::WhiteSpaceCollapse(
            WhiteSpaceCollapse::Collapse,
        )]);
        builder.push_text("a  ");
        builder.push_inline_box(out_of_flow_box(0.));
        builder.push_text(suffix);
        assert_eq!(builder.build().1, "a b");
    }
}

#[test]
fn inline_box_index() {
    let mut env = TestEnv::new(test_name!(), None);

    // Boxes of every kind are indexed after the text pushed before them, in the current span or
    // an enclosing one.
    let mut builder = env.tree_builder();
    for (id, kind) in [
        InlineBoxKind::InFlow,
        InlineBoxKind::OutOfFlow,
        InlineBoxKind::CustomOutOfFlow,
    ]
    .into_iter()
    .enumerate()
    {
        builder.push_text("ab");
        builder.push_style_modification_span(None);
        builder.push_text("cd");
        builder.push_inline_box(InlineBox {
            id: id as u64,
            index: 0,
            width: 10.,
            height: 10.,
            baseline: None,
            kind,
            vertical_align: VerticalAlign::BASELINE,
        });
        builder.pop_style_span();
    }
    let (layout, text) = builder.build();
    assert_eq!(text, "abcdabcdabcd");
    let boxes: Vec<_> = layout.inline_boxes().map(|b| (b.id, b.index)).collect();
    assert_eq!(boxes, [(0, 4), (1, 8), (2, 12)]);
}

#[test]
fn collapsed_space_belongs_to_first_span() {
    let mut env = TestEnv::new(test_name!(), None);

    // Whitespace spanning a span boundary collapses into a single space, which belongs to the span
    // the whitespace sequence started in.
    let mut builder = env.tree_builder();
    builder.push_style_modification_span(&[StyleProperty::WhiteSpaceCollapse(
        WhiteSpaceCollapse::Collapse,
    )]);
    builder.push_text("aa ");
    builder.push_style_modification_span(&[StyleProperty::FontWeight(FontWeight::BOLD)]);
    builder.push_text(" bb");
    builder.pop_style_span();
    let (mut layout, text) = builder.build();
    layout.break_all_lines(None);
    assert_eq!(text, "aa bb");

    let style_indices: Vec<u16> = layout
        .get(0)
        .unwrap()
        .runs()
        .flat_map(|run| run.clusters().map(|cluster| cluster.style_index()))
        .collect();
    let space_style = style_indices[2];
    assert_eq!(space_style, style_indices[0], "space is not bold");
    assert_ne!(space_style, style_indices[3], "\"bb\" is not bold");
}

#[test]
fn collapsed_space_wraps_if_any_collapsed_span_wraps() {
    let mut env = TestEnv::new(test_name!(), None);

    // Number of lines the given text is broken into at a width that only fits one word, where
    // `nowrap` selects which of the two spans disables wrapping.
    let mut line_count = |first: &str, second: &str, nowrap: [bool; 2]| {
        let mut builder = env.tree_builder();
        builder.push_style_modification_span(&[StyleProperty::WhiteSpaceCollapse(
            WhiteSpaceCollapse::Collapse,
        )]);
        for (text, nowrap) in [first, second].into_iter().zip(nowrap) {
            let mode = if nowrap {
                TextWrapMode::NoWrap
            } else {
                TextWrapMode::Wrap
            };
            builder.push_style_modification_span(&[StyleProperty::TextWrapMode(mode)]);
            builder.push_text(text);
            builder.pop_style_span();
        }
        let (mut layout, _) = builder.build();
        layout.break_all_lines(Some(30.));
        layout.len()
    };

    // The collapsed space is a wrap opportunity if any of the whitespace it collapses comes from a
    // span that allows wrapping, even when it is attributed to a span that does not.
    assert_eq!(line_count("aa ", " bb", [true, false]), 2);
    assert_eq!(line_count("aa ", " bb", [false, true]), 2);
    assert_eq!(line_count("aa ", " bb", [false, false]), 2);
    assert_eq!(line_count("aa ", " bb", [true, true]), 1);

    // Whitespace from a single span keeps that span's wrapping behaviour.
    assert_eq!(line_count("aa ", "bb", [true, false]), 1);
    assert_eq!(line_count("aa", " bb", [false, true]), 1);
    assert_eq!(line_count("aa ", "bb", [false, true]), 2);
}

#[test]
fn nested_span_inheritance() {
    let ts = |c: AlphaColor<Srgb>| TextStyle {
        font_family: FontFamily::from(crate::util::env::FONT_FAMILY_LIST),
        font_size: 24.,
        line_height: LineHeight::Absolute(30.),
        brush: ColorBrush::new(c),
        ..TextStyle::default()
    };
    let sp = |c: AlphaColor<Srgb>| [StyleProperty::Brush(ColorBrush::new(c))];

    let mut env = TestEnv::new(test_name!(), None);
    let mut tb = env.tree_builder();
    tb.push_style_span(ts(palette::css::RED));
    tb.push_text("N"); // Red
    tb.push_style_span(ts(palette::css::RED));
    tb.push_text("e"); // Red
    tb.push_style_span(ts(palette::css::GREEN));
    tb.push_text("s"); // Green
    tb.push_style_modification_span(&sp(palette::css::GREEN));
    tb.push_text("t"); // Green
    tb.push_style_span(ts(palette::css::BLUE));
    tb.push_text("e"); // Blue
    tb.push_style_modification_span(None);
    tb.push_text("d"); // Blue
    tb.push_text(" ");
    tb.pop_style_span();
    tb.push_text("s"); // Blue
    tb.pop_style_span();
    tb.push_text("p"); // Green
    tb.pop_style_span();
    tb.push_text("a"); // Green
    tb.pop_style_span();
    tb.push_text("n"); // Red
    tb.pop_style_span();
    tb.push_text("s"); // Red
    tb.pop_style_span();
    tb.push_text(" w"); // Root style
    tb.push_style_span(ts(palette::css::GOLD));
    tb.push_style_span(ts(palette::css::GOLD));
    tb.push_style_span(ts(palette::css::GOLD));
    tb.push_text("o"); // Triple-nested gold
    tb.pop_style_span();
    tb.pop_style_span();
    tb.pop_style_span();
    tb.push_text("rk"); // Root style

    let (mut layout, _) = tb.build();
    layout.break_all_lines(None);
    env.check_layout_snapshot(&layout);
}

#[test]
fn base_level_alignment_ltr() {
    let mut env = TestEnv::new(test_name!(), None);

    for (alignment, test_case_name) in [
        (Alignment::Start, "start"),
        (Alignment::End, "end"),
        (Alignment::Center, "center"),
        (Alignment::Justify, "justify"),
    ] {
        let text = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.";
        let builder = env.ranged_builder(text);
        let mut layout = builder.build(text);
        layout.break_all_lines(Some(150.0));
        layout.align(alignment, AlignmentOptions::default());
        env.with_name(test_case_name).check_layout_snapshot(&layout);
    }
}

#[test]
fn base_level_alignment_rtl() {
    let mut env = TestEnv::new(test_name!(), None);

    for (alignment, test_case_name) in [
        (Alignment::Start, "start"),
        (Alignment::End, "end"),
        (Alignment::Center, "center"),
        (Alignment::Justify, "justify"),
    ] {
        let text = "عند برمجة أجهزة الكمبيوتر، قد تجد نفسك فجأة في مواقف غريبة، مثل الكتابة بلغة لا تتحدثها فعليًا.";
        let builder = env.ranged_builder(text);
        let mut layout = builder.build(text);
        layout.break_all_lines(Some(150.0));
        layout.align(alignment, AlignmentOptions::default());
        env.with_name(test_case_name).check_layout_snapshot(&layout);
    }
}

#[test]
/// The last line of the layout and lines ending in an explicit line break use the last line
/// alignment, all other lines use the main alignment.
fn last_line_alignment() {
    let mut env = TestEnv::new(test_name!(), None);

    let ltr = "Lorem ipsum dolor sit amet,\nconsectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore.";
    let rtl = "عند برمجة أجهزة الكمبيوتر، قد تجد نفسك فجأة\nفي مواقف غريبة، مثل الكتابة بلغة لا تتحدثها فعليًا.";

    for (text, text_name) in [(ltr, "ltr"), (rtl, "rtl")] {
        for (alignment, last_line_alignment, test_case_name) in [
            (Alignment::Justify, Alignment::End, "justify_end"),
            (Alignment::Justify, Alignment::Center, "justify_center"),
            (Alignment::Justify, Alignment::Justify, "justify_justify"),
            (Alignment::Center, Alignment::Start, "center_start"),
        ] {
            let builder = env.ranged_builder(text);
            let mut layout = builder.build(text);
            layout.break_all_lines(Some(150.0));
            layout.align(
                alignment,
                AlignmentOptions {
                    last_line_alignment: Some(last_line_alignment),
                    ..AlignmentOptions::default()
                },
            );
            env.with_name(&format!("{text_name}_{test_case_name}"))
                .check_layout_snapshot(&layout);
        }
    }
}

#[test]
/// On overflow without alignment-on-overflow, RTL-text should be start-aligned (i.e., aligned to
/// the right edge, overflowing on the left).
fn overflow_alignment_rtl() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "عند برمجة أجهزة الكمبيوتر، قد تجد نفسك فجأة في مواقف غريبة، مثل الكتابة بلغة لا تتحدثها فعليًا.";
    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);
    layout.break_all_lines(Some(1000.0));
    layout.align(Alignment::Center, AlignmentOptions::default());
    env.rendering_config().size = Some(Size::new(10., layout.height().into()));
    env.check_layout_snapshot(&layout);
}

#[test]
/// Justified alignment must only stretch at grapheme boundaries, and not all spaces are grapheme
/// boundaries. See <https://www.unicode.org/reports/tr29/#GB9b>.
fn dont_justify_space_inside_grapheme() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "aa\u{0D4E} bb cc dd";
    //                    ^
    //         Not a grapheme boundary

    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);
    // Breaks after "cc ", leaving the first line one space to stretch.
    layout.break_all_lines(Some(80.0));
    layout.align(Alignment::Justify, AlignmentOptions::default());

    env.check_layout_snapshot(&layout);
}

#[test]
fn justify_with_overflowing_trailing_space() {
    let mut env = TestEnv::new(test_name!(), None);

    // This test has a trailing space on the first line that hangs past the line box. This checks
    // whether that interacts correctly with justification. In previous versions, that hanging space
    // was miscounted, resulting in justification opportunities being undercounted.
    //
    // The large word spacing ensures the trailing space actually overflows, and also makes the
    // justification more clearly visible: the free space that justification redistributes is the
    // portion of the hanging space that's inside the line box.
    let text = "sit amet, consectetur adipiscing";
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::WordSpacing(48.0));
    let mut layout = builder.build(text);
    layout.break_all_lines(Some(280.0));
    layout.align(Alignment::Justify, AlignmentOptions::default());

    env.check_layout_snapshot(&layout);

    let line = layout.get(0).unwrap();
    let justified_space_advances: Vec<_> = line
        .runs()
        .flat_map(|run| run.clusters())
        .filter(|cluster| cluster.is_space_or_nbsp())
        .map(|cluster| cluster.advance())
        .collect();
    assert!(
        (justified_space_advances[0] - justified_space_advances[1]).abs() < 0.001,
        "The two spaces on the first line should be equal in size"
    );
}

#[test]
fn justify_consecutive_spaces() {
    let mut env = TestEnv::new(test_name!(), None);

    // Each space is a separate justification opportunity, including when they are consecutive
    // (without whitespace collapsing). The second paragraph here has a run of three consecutive
    // spaces.
    let text = "Foo bar baz supercalifragilistic.\n\nFoo   bar baz supercalifragilistic.";
    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);
    layout.break_all_lines(Some(150.0));
    layout.align(Alignment::Justify, AlignmentOptions::default());

    env.check_layout_snapshot(&layout);

    // Check that the second paragraph's interior spaces on the first line all stretch by the same
    // amount.
    let space_advances: Vec<_> = layout
        .get(3)
        .unwrap()
        .runs()
        .flat_map(|run| run.clusters())
        .filter(|cluster| cluster.is_space_or_nbsp())
        .map(|cluster| cluster.advance())
        .collect();
    assert_eq!(space_advances.len(), 5, "four spaces plus one trailing");
    assert!(
        space_advances[..4]
            .iter()
            .all(|advance| (advance - space_advances[0]).abs() < 0.001),
        "The interior spaces should stretch equally"
    );
}

#[test]
fn justify_with_multiple_trailing_spaces() {
    let mut env = TestEnv::new(test_name!(), None);

    // The first line breaks after "CC   ", leaving three trailing spaces. All of them should hang
    // past the line box unstretched, while justification distributes the line's free space over the
    // two inter-word spaces only.
    let text = "AA BB CC   DD EE";
    let max_advance = 95.0;
    let mut builder = env.ranged_builder(text);

    // This should also correctly hang the whitespace when the spaces are not in the same run (e.g.,
    // because of a font size change).
    builder.push(StyleProperty::FontSize(15.9), 10..11);
    let mut layout = builder.build(text);
    layout.break_all_lines(Some(max_advance));
    layout.align(Alignment::Justify, AlignmentOptions::default());
    env.check_cluster_snapshot(&layout, text, 12.0);
}

#[test]
fn trailing_nbsp_doesnt_hang_but_also_doesnt_justify_ltr() {
    let mut env = TestEnv::new(test_name!(), None);

    // The first line breaks after "CC\u{a0} " (with `\u{a0}` being a non-breaking space). That last
    // space hangs, but the non-breaking space doesn't. As it ends the line, the non-breaking space
    // should not get a justification stretch applied.
    let text = "AA BB CC\u{a0} DD EE";
    let max_advance = 95.0;
    let builder = env.ranged_builder(text);

    let mut layout = builder.build(text);
    layout.break_all_lines(Some(max_advance));
    layout.align(Alignment::Justify, AlignmentOptions::default());
    env.check_cluster_snapshot(&layout, text, 12.0);
}

#[test]
fn trailing_nbsp_doesnt_hang_but_also_doesnt_justify_rtl() {
    let mut env = TestEnv::new(test_name!(), None);

    // The first line breaks after "ججج\u{a0} " (with `\u{a0}` being a non-breaking space). The last
    // space hangs (on the left side of the line, as the paragraph is RTL), but the non-breaking
    // space doesn't. As it ends the line, the non-breaking space should not get a justification
    // stretch applied, but does get word-spacing.
    let text = "ااا ببب ججج\u{a0} ددد";
    let max_advance = 143.0;
    let mut builder = env.ranged_builder(text);

    // Add some word spacing to also test interaction with spacing.
    builder.push_default(StyleProperty::WordSpacing(12.0));

    let mut layout = builder.build(text);
    layout.break_all_lines(Some(max_advance));
    layout.align(Alignment::Justify, AlignmentOptions::default());
    env.check_cluster_snapshot(&layout, text, 12.0);
}

#[test]
fn content_widths() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "Hello world!\nLonger line with a looooooooong word.";
    let builder = env.ranged_builder(text);

    let mut layout = builder.build(text);

    let ContentWidths {
        min: min_content_width,
        max: max_content_width,
    } = layout.calculate_content_widths();

    layout.break_all_lines(Some(min_content_width));
    layout.align(Alignment::Start, AlignmentOptions::default());
    env.with_name("min").check_layout_snapshot(&layout);

    layout.break_all_lines(Some(max_content_width));
    layout.align(Alignment::Start, AlignmentOptions::default());
    env.with_name("max").check_layout_snapshot(&layout);
}

#[test]
fn content_widths_rtl() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "بببب ااااا";
    let builder = env.ranged_builder(text);

    let mut layout = builder.build(text);

    let ContentWidths {
        min: min_content_width,
        max: max_content_width,
    } = layout.calculate_content_widths();

    layout.break_all_lines(Some(min_content_width));
    layout.align(Alignment::Start, AlignmentOptions::default());
    env.with_name("min").check_layout_snapshot(&layout);

    layout.break_all_lines(Some(max_content_width));
    layout.align(Alignment::Start, AlignmentOptions::default());
    assert!(
        layout.width() <= max_content_width,
        "Layout should never be wider than the max content width"
    );
    env.with_name("max").check_layout_snapshot(&layout);
}

#[test]
fn test_cluster_info() {
    let test_name = test_name!();
    let mut env = TestEnv::new(test_name, Size::new(400.0, 200.0));

    let test_cluster_layout = |test: &str, text: &str, env: &mut TestEnv| {
        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::FontSize(24.0));
        let mut layout = builder.build(text);
        layout.break_all_lines(Some(400.0));
        env.with_name(test)
            .check_cluster_snapshot(&layout, text, 12.0);
    };

    // Latin LTR with ligature
    let text = "Hello Ligature: fi";
    test_cluster_layout("latin", text, &mut env);

    // Arabic RTL with ligature
    let text = "حداً ";
    test_cluster_layout("arabic", text, &mut env);

    // Mixed content with ligature
    let text = "Hello Ligature: fi, Arabic: حداً";
    test_cluster_layout("ltr_rtl_mixed", text, &mut env);

    // Newlines
    let text = "Hello\nLigature:\nfi";
    test_cluster_layout("newlines", text, &mut env);
}

#[test]
fn text_range_rtl() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "اللغة العربية";
    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);
    layout.break_all_lines(Some(100.0));
    layout.align(Alignment::Start, AlignmentOptions::default());

    for line in layout.lines() {
        for item in line.items() {
            if let PositionedLayoutItem::GlyphRun(glyph_run) = item {
                glyph_run.run().clusters().for_each(|c| {
                    if !c.is_space_or_nbsp() {
                        assert_eq!(c.text_range().len(), 2);
                    }
                });
            }
        }
    }
}

#[test]
/// Layouts can be re-line-broken and re-aligned.
fn realign() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.";
    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);
    layout.break_all_lines(Some(150.0));
    for idx in 0..8 {
        if [2, 3, 4].contains(&idx) {
            layout.break_all_lines(Some(150.0));
        }
        layout.align(Alignment::Justify, AlignmentOptions::default());
    }
    env.check_layout_snapshot(&layout);
}

#[test]
/// Tests that all alignment option combinations can be applied to a dirty layout.
///
/// Rendering 684 snapshots takes over 10 seconds on a 5950X, so a simpler assert is used instead.
fn realign_all() {
    let mut env = TestEnv::new(test_name!(), None);

    let latin = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.";
    let arabic = "عند برمجة أجهزة الكمبيوتر، قد تجد نفسك فجأة في مواقف غريبة، مثل الكتابة بلغة لا تتحدثها فعليًا.";

    let texts = [(latin, "latin"), (arabic, "arabic")];

    let alignments = [
        (Alignment::Start, "start"),
        (Alignment::End, "end"),
        (Alignment::Left, "left"),
        (Alignment::Center, "center"),
        (Alignment::Right, "right"),
        (Alignment::Justify, "justify"),
    ];

    let all_opts = [
        (Some(150.), AlignmentOptions::default(), "150", "default"),
        (
            None,
            AlignmentOptions {
                align_when_overflowing: true,
                ..AlignmentOptions::default()
            },
            "none",
            "awo_true",
        ),
        (
            None,
            AlignmentOptions {
                align_when_overflowing: false,
                ..AlignmentOptions::default()
            },
            "none",
            "awo_false",
        ),
    ];

    // Build a collection of base truth
    let mut layouts = Vec::new();
    for (text, _) in texts {
        for (alignment, _) in alignments {
            for (max_advance, opts, _, _) in all_opts {
                let builder = env.ranged_builder(text);
                let mut layout = builder.build(text);
                layout.break_all_lines(max_advance);
                layout.align(alignment, opts);
                layouts.push(layout);
            }
        }
    }

    // Loop over all the base truths ...
    let mut idx = 0;
    for (text_idx, (_, text_name)) in texts.iter().enumerate() {
        for (_, align_name) in alignments {
            for (max_advance, _, ma_name, opts_name) in all_opts {
                let layout = &layouts[idx];
                idx += 1;

                let base_name = format!("{text_name}_{align_name}_{opts_name}_{ma_name}");
                //env.with_name(&base_name).check_layout_snapshot(&layout);

                // ... and make sure every combination can be applied on top without issues
                let mut jdx = text_idx * (layouts.len() / texts.len());
                for (top_alignment, align_name) in alignments {
                    for (top_max_advance, top_opts, ma_name, opts_name) in all_opts {
                        let mut top_layout = layout.clone();
                        // Only break lines again if the max advance differs from base,
                        // because otherwise we already have the correct line breaks.
                        // We want to specifically test the optimization of skipping it.
                        if max_advance != top_max_advance {
                            top_layout.break_all_lines(top_max_advance);
                        }
                        top_layout.align(top_alignment, top_opts);

                        let top_name = format!("{text_name}_{align_name}_{opts_name}_{ma_name}");
                        //env.with_name(&top_name).check_layout_snapshot(&top_layout);

                        let top_layout_truth = &layouts[jdx];
                        jdx += 1;

                        crate::util::assert_eq_layout_alignments(
                            top_layout_truth,
                            &top_layout,
                            &format!("{base_name} -> {top_name}"),
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn layout_impl_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Layout<()>>();
}

#[test]
fn shaping_context_across_items() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "كتب\nكتب";
    // Bytes:   0246..
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::FontSize(16.));
    // On the first line, the change in font size creates itemization boundaries. Shaping should
    // still select correct alternates (initial/medial/final forms in this case). Noto Kufi doesn't
    // form ligatures here, so the glyphs of the first and second lines should be identical (modulo
    // font size).
    builder.push(StyleProperty::FontSize(18.0), 2..4);

    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    env.check_layout_snapshot(&layout);

    assert_eq!(
        layout.get(0).unwrap().runs().count(),
        4,
        "The first line must be four shaped runs (one for each character, and the newline), otherwise it didn't actually separate into items"
    );
    assert_eq!(
        layout.get(1).unwrap().runs().count(),
        1,
        "The second line must be a single shaped run"
    );

    let glyph_ids_by_line: Vec<Vec<u32>> = layout
        .lines()
        .map(|line| {
            line.items()
                .filter_map(|item| match item {
                    PositionedLayoutItem::GlyphRun(run) => {
                        Some(run.glyphs().map(|glyph| glyph.id).collect::<Vec<_>>())
                    }
                    PositionedLayoutItem::InlineBox(_) => None,
                })
                .flatten()
                .collect()
        })
        .collect();

    assert_eq!(
        glyph_ids_by_line[0], glyph_ids_by_line[1],
        "Glyphs must be identical"
    );
}

#[test]
fn trailing_whitespace_alignment_by_collapse_mode() {
    let mut env = TestEnv::new(test_name!(), None);

    // Following CSS Text 4 § 4.3.2, trailing whitespace before a forced line break (including the
    // end of the layout) hangs conditionally when whitespace is preserved: the part that fits
    // stays inside the line box and is aligned along with the text. In the collapsing modes the
    // whitespace hangs unconditionally, so the text is flush with the line's end edge. Note the
    // ranged builder doesn't collapse whitespace itself, so the spaces reach layout.
    let text = "AA   \nBB  ";
    let mut word_layout = env.ranged_builder("AA").build("AA");
    word_layout.break_all_lines(None);
    let word_width = word_layout.width();
    for (mode, test_case_name, hangs_unconditionally) in [
        (WhiteSpaceCollapse::Preserve, "preserve", false),
        (WhiteSpaceCollapse::Collapse, "collapse", true),
        (WhiteSpaceCollapse::PreserveBreaks, "preserve_breaks", true),
    ] {
        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::WhiteSpaceCollapse(mode));
        let mut layout = builder.build(text);
        layout.break_all_lines(Some(60.0));
        layout.align(Alignment::End, AlignmentOptions::default());
        env.with_name(test_case_name).check_layout_snapshot(&layout);

        let first_line_advance = layout.get(0).unwrap().metrics().advance;
        if hangs_unconditionally {
            assert!(
                (layout.width() - word_width).abs() < 1e-3,
                "Layout width {} should exclude the hanging spaces for {mode:?}",
                layout.width()
            );
        } else {
            assert!(
                (layout.width() - first_line_advance).abs() < 1e-3,
                "Layout width {} should include the fitting spaces for {mode:?}",
                layout.width()
            );
        }
    }
}
