// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Text indent tests.

use crate::test_name;
use crate::util::{ColorBrush, TestEnv};
use parley::{Affinity, Alignment, AlignmentOptions, Cluster, Cursor, IndentOptions, Selection};

fn build_indented_layout(
    env: &mut TestEnv,
    text: &str,
    indent_amount: f32,
    indent_options: IndentOptions,
    wrap_width: f32,
    alignment: Alignment,
) -> parley::Layout<ColorBrush> {
    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);
    layout.set_text_indent(indent_amount, indent_options);
    layout.break_all_lines(Some(wrap_width));
    layout.align(alignment, AlignmentOptions::default());
    layout
}

#[test]
fn text_indent_basic() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "The quick brown fox jumps over the lazy dog and keeps on running far away.";
    let layout = build_indented_layout(
        &mut env,
        text,
        50.0,
        IndentOptions::default(),
        200.0,
        Alignment::Start,
    );

    assert!(layout.len() > 1, "Expected multiple lines");
    let first_line = layout.get(0).unwrap();
    assert_eq!(first_line.metrics().offset, 50.0);
    for i in 1..layout.len() {
        let line = layout.get(i).unwrap();
        assert_eq!(line.metrics().offset, 0.0, "Line {i} should have no indent");
    }

    env.check_layout_snapshot(&layout);
}

#[test]
fn text_indent_no_wrap() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "Short text.";
    let layout = build_indented_layout(
        &mut env,
        text,
        30.0,
        IndentOptions::default(),
        f32::INFINITY,
        Alignment::Start,
    );

    assert_eq!(layout.len(), 1);
    let line = layout.get(0).unwrap();
    assert_eq!(line.metrics().offset, 30.0);

    env.check_layout_snapshot(&layout);
}

#[test]
fn text_indent_each_line() {
    let mut env = TestEnv::new(test_name!(), None);
    let text =
        "First paragraph line one.\nSecond paragraph also indented.\nThird paragraph here too.";
    let layout = build_indented_layout(
        &mut env,
        text,
        40.0,
        IndentOptions {
            each_line: true,
            hanging: false,
        },
        250.0,
        Alignment::Start,
    );

    for i in 0..layout.len() {
        let line = layout.get(i).unwrap();
        let is_first_or_after_hard_break = i == 0 || {
            let prev_text = &text[layout.get(i - 1).unwrap().text_range()];
            prev_text.ends_with('\n')
        };
        if is_first_or_after_hard_break {
            assert_eq!(
                line.metrics().offset,
                40.0,
                "Line {i} should be indented after hard break"
            );
        }
    }

    env.check_layout_snapshot(&layout);
}

#[test]
fn text_indent_hanging() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "The quick brown fox jumps over the lazy dog and keeps on running far away.";
    let layout = build_indented_layout(
        &mut env,
        text,
        50.0,
        IndentOptions {
            each_line: false,
            hanging: true,
        },
        200.0,
        Alignment::Start,
    );

    assert!(layout.len() > 1, "Expected multiple lines");
    let first_line = layout.get(0).unwrap();
    assert_eq!(
        first_line.metrics().offset,
        0.0,
        "First line should NOT be indented with hanging"
    );
    for i in 1..layout.len() {
        let line = layout.get(i).unwrap();
        assert_eq!(
            line.metrics().offset,
            50.0,
            "Line {i} should be indented with hanging"
        );
    }

    env.check_layout_snapshot(&layout);
}

#[test]
fn text_indent_hanging_each_line() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "First paragraph wraps here.\nSecond paragraph also wraps around here too.";
    let layout = build_indented_layout(
        &mut env,
        text,
        40.0,
        IndentOptions {
            each_line: true,
            hanging: true,
        },
        200.0,
        Alignment::Start,
    );

    for i in 0..layout.len() {
        let line = layout.get(i).unwrap();
        let is_scope_line = i == 0 || {
            let prev_text = &text[layout.get(i - 1).unwrap().text_range()];
            prev_text.ends_with('\n')
        };
        if is_scope_line {
            assert_eq!(
                line.metrics().offset,
                0.0,
                "Line {i} (scope line) should NOT be indented with hanging"
            );
        } else {
            assert_eq!(
                line.metrics().offset,
                40.0,
                "Line {i} (continuation) should be indented with hanging"
            );
        }
    }

    env.check_layout_snapshot(&layout);
}

#[test]
fn text_indent_negative() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "The quick brown fox jumps over the lazy dog and keeps on running.";
    let layout = build_indented_layout(
        &mut env,
        text,
        -20.0,
        IndentOptions::default(),
        200.0,
        Alignment::Start,
    );

    assert!(layout.len() > 1, "Expected multiple lines");
    let first_line = layout.get(0).unwrap();
    assert_eq!(
        first_line.metrics().offset,
        -20.0,
        "First line should have negative offset"
    );

    env.check_layout_snapshot(&layout);
}

#[test]
fn text_indent_negative_hanging() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "The quick brown fox jumps over the lazy dog and keeps on running.";
    let layout = build_indented_layout(
        &mut env,
        text,
        -20.0,
        IndentOptions {
            each_line: false,
            hanging: true,
        },
        200.0,
        Alignment::Start,
    );

    assert!(layout.len() > 1, "Expected multiple lines");
    let first_line = layout.get(0).unwrap();
    assert_eq!(
        first_line.metrics().offset,
        0.0,
        "First line should have no offset"
    );
    for i in 1..layout.len() {
        let line = layout.get(i).unwrap();
        assert_eq!(
            line.metrics().offset,
            -20.0,
            "Line {i} should have negative offset with hanging"
        );
    }

    env.check_layout_snapshot(&layout);
}

#[test]
fn text_indent_center_alignment() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "The quick brown fox jumps over the lazy dog.";
    let layout = build_indented_layout(
        &mut env,
        text,
        50.0,
        IndentOptions::default(),
        300.0,
        Alignment::Center,
    );

    env.check_layout_snapshot(&layout);
}

#[test]
fn text_indent_right_alignment() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "The quick brown fox jumps over the lazy dog.";
    let layout = build_indented_layout(
        &mut env,
        text,
        50.0,
        IndentOptions::default(),
        300.0,
        Alignment::Right,
    );

    env.check_layout_snapshot(&layout);
}

#[test]
fn text_indent_justify() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "The quick brown fox jumps over the lazy dog and keeps on running far away.";
    let layout = build_indented_layout(
        &mut env,
        text,
        50.0,
        IndentOptions::default(),
        200.0,
        Alignment::Justify,
    );

    env.check_layout_snapshot(&layout);
}

#[test]
fn text_indent_zero() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "The quick brown fox jumps over the lazy dog.";
    let layout = build_indented_layout(
        &mut env,
        text,
        0.0,
        IndentOptions::default(),
        200.0,
        Alignment::Start,
    );

    for i in 0..layout.len() {
        let line = layout.get(i).unwrap();
        assert_eq!(
            line.metrics().offset,
            0.0,
            "Line {i} should have zero offset"
        );
    }

    env.check_layout_snapshot(&layout);
}

#[test]
fn text_indent_line_breaking() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "The quick brown fox jumps over the lazy dog.";

    let builder_no_indent = env.ranged_builder(text);
    let mut layout_no_indent = builder_no_indent.build(text);
    layout_no_indent.break_all_lines(Some(200.0));
    let lines_no_indent = layout_no_indent.len();

    let layout_with_indent = build_indented_layout(
        &mut env,
        text,
        80.0,
        IndentOptions::default(),
        200.0,
        Alignment::Start,
    );
    let lines_with_indent = layout_with_indent.len();

    assert!(
        lines_with_indent >= lines_no_indent,
        "Indent should cause same or more line breaks: without={lines_no_indent}, with={lines_with_indent}"
    );

    env.check_layout_snapshot(&layout_with_indent);
}

#[test]
fn text_indent_rtl_basic() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "عند برمجة أجهزة الكمبيوتر، قد تجد نفسك فجأة في مواقف غريبة، مثل الكتابة بلغة لا تتحدثها فعليًا.";
    let layout = build_indented_layout(
        &mut env,
        text,
        50.0,
        IndentOptions::default(),
        200.0,
        Alignment::Start,
    );

    assert!(layout.len() > 1, "Expected multiple lines");

    env.check_layout_snapshot(&layout);
}

#[test]
fn text_indent_rtl_hanging() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "عند برمجة أجهزة الكمبيوتر، قد تجد نفسك فجأة في مواقف غريبة، مثل الكتابة بلغة لا تتحدثها فعليًا.";
    let layout = build_indented_layout(
        &mut env,
        text,
        50.0,
        IndentOptions {
            each_line: false,
            hanging: true,
        },
        200.0,
        Alignment::Start,
    );

    assert!(layout.len() > 1, "Expected multiple lines");

    env.check_layout_snapshot(&layout);
}

#[test]
fn text_indent_rtl_each_line() {
    let mut env = TestEnv::new(test_name!(), None);
    let text =
        "عند برمجة أجهزة الكمبيوتر.\nقد تجد نفسك فجأة في مواقف غريبة.\nمثل الكتابة بلغة لا تتحدثها.";
    let layout = build_indented_layout(
        &mut env,
        text,
        40.0,
        IndentOptions {
            each_line: true,
            hanging: false,
        },
        250.0,
        Alignment::Start,
    );

    env.check_layout_snapshot(&layout);
}

#[test]
fn text_indent_rtl_negative() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "عند برمجة أجهزة الكمبيوتر، قد تجد نفسك فجأة في مواقف غريبة، مثل الكتابة بلغة لا تتحدثها فعليًا.";
    let layout = build_indented_layout(
        &mut env,
        text,
        -20.0,
        IndentOptions::default(),
        200.0,
        Alignment::Start,
    );

    assert!(layout.len() > 1, "Expected multiple lines");

    env.check_layout_snapshot(&layout);
}

#[test]
fn text_indent_rtl_center_alignment() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "عند برمجة أجهزة الكمبيوتر، قد تجد نفسك فجأة في مواقف غريبة، مثل الكتابة بلغة لا تتحدثها فعليًا.";
    let layout = build_indented_layout(
        &mut env,
        text,
        50.0,
        IndentOptions::default(),
        200.0,
        Alignment::Center,
    );

    env.check_layout_snapshot(&layout);
}

#[test]
fn text_indent_rtl_justify() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "عند برمجة أجهزة الكمبيوتر، قد تجد نفسك فجأة في مواقف غريبة، مثل الكتابة بلغة لا تتحدثها فعليًا.";
    let layout = build_indented_layout(
        &mut env,
        text,
        50.0,
        IndentOptions::default(),
        200.0,
        Alignment::Justify,
    );

    env.check_layout_snapshot(&layout);
}

#[test]
fn text_indent_cursor_geometry() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "The quick brown fox jumps over the lazy dog and keeps on running far away.";
    let indent = 50.0;
    let layout = build_indented_layout(
        &mut env,
        text,
        indent,
        IndentOptions::default(),
        200.0,
        Alignment::Start,
    );

    assert!(layout.len() > 1, "Expected multiple lines");

    let cursor_start = Cursor::from_byte_index(&layout, 0, Affinity::Downstream);
    let geom = cursor_start.geometry(&layout, 2.0);
    assert!(
        geom.x0 >= indent as f64 - 1.0,
        "Cursor at start of indented line should be at indent ({indent}), got {}",
        geom.x0
    );

    let line1_start = layout.get(1).unwrap().text_range().start;
    let cursor_line2 = Cursor::from_byte_index(&layout, line1_start, Affinity::Downstream);
    let geom2 = cursor_line2.geometry(&layout, 2.0);
    assert!(
        geom2.x0 < 1.0,
        "Cursor at start of non-indented line should be near 0, got {}",
        geom2.x0
    );
}

#[test]
fn text_indent_hit_testing() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "The quick brown fox jumps over the lazy dog and keeps on running far away.";
    let indent = 50.0;
    let layout = build_indented_layout(
        &mut env,
        text,
        indent,
        IndentOptions::default(),
        200.0,
        Alignment::Start,
    );

    let line0 = layout.get(0).unwrap();
    let line0_y = (line0.metrics().block_min_coord + line0.metrics().block_max_coord) / 2.0;

    let cursor_in_indent = Cursor::from_point(&layout, 5.0, line0_y);
    assert_eq!(
        cursor_in_indent.index(),
        0,
        "Clicking in the indent area should resolve to the start of the line"
    );

    let cursor_after_indent = Cursor::from_point(&layout, indent + 10.0, line0_y);
    assert!(
        cursor_after_indent.index() > 0,
        "Clicking after indent should resolve to a character past the start"
    );

    let line1 = layout.get(1).unwrap();
    let line1_y = (line1.metrics().block_min_coord + line1.metrics().block_max_coord) / 2.0;
    let line1_text_start = line1.text_range().start;

    let cursor_line2_start = Cursor::from_point(&layout, 5.0, line1_y);
    assert!(
        (cursor_line2_start.index() as isize - line1_text_start as isize).unsigned_abs() <= 1,
        "Clicking at x=5 on non-indented line should resolve near line start ({}), got {}",
        line1_text_start,
        cursor_line2_start.index()
    );
}

#[test]
fn text_indent_negative_hit_testing() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "The quick brown fox jumps over the lazy dog and keeps on running.";
    let layout = build_indented_layout(
        &mut env,
        text,
        -20.0,
        IndentOptions::default(),
        200.0,
        Alignment::Start,
    );

    let cursor_start = Cursor::from_byte_index(&layout, 0, Affinity::Downstream);
    let geom = cursor_start.geometry(&layout, 2.0);
    assert!(
        geom.x0 < 0.0,
        "Cursor at start of negatively indented line should be at negative x, got {}",
        geom.x0
    );

    let line0 = layout.get(0).unwrap();
    let line0_y = (line0.metrics().block_min_coord + line0.metrics().block_max_coord) / 2.0;
    let cluster_in_overflow = Cluster::from_point(&layout, -10.0, line0_y);
    assert!(
        cluster_in_overflow.is_some(),
        "Should be able to hit-test in overflow area with negative indent"
    );
}

#[test]
fn text_indent_selection_geometry() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "The quick brown fox jumps over the lazy dog and keeps on running far away.";
    let indent = 50.0;
    let layout = build_indented_layout(
        &mut env,
        text,
        indent,
        IndentOptions::default(),
        200.0,
        Alignment::Start,
    );

    let selection = Selection::new(
        Cursor::from_byte_index(&layout, 0, Affinity::Downstream),
        Cursor::from_byte_index(&layout, 5, Affinity::Downstream),
    );
    let rects = selection.geometry(&layout);
    assert!(!rects.is_empty(), "Selection should produce geometry");

    let first_rect = &rects[0].0;
    assert!(
        first_rect.x0 >= indent as f64 - 1.0,
        "Selection on indented line should start at indent ({indent}), got {}",
        first_rect.x0
    );
}
