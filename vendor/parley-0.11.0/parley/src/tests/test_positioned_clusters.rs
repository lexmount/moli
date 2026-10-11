// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::utils::fonts::{FONT_FAMILY_LIST, create_font_context};
use crate::{
    Alignment, AlignmentOptions, BaseDirection, FontFamily, IndentOptions, InlineBox,
    InlineBoxKind, Layout, LayoutContext, TextStyle, VerticalAlign,
};

fn inline_box(id: u64, index: usize, width: f32) -> InlineBox {
    InlineBox {
        id,
        index,
        kind: InlineBoxKind::InFlow,
        width,
        height: 10.0,
        baseline: None,
        vertical_align: VerticalAlign::BASELINE,
    }
}

fn assert_positions_match(layout: &Layout<()>) {
    for line in layout.lines() {
        let mut positioned = line.positioned_visual_clusters();
        for expected in line.runs().flat_map(|run| run.visual_clusters()) {
            let (cluster, offset) = positioned.next().expect("every cluster must be positioned");
            assert_eq!(cluster.path(), expected.path(), "visual order must match");
            assert_eq!(
                offset.to_bits(),
                expected.visual_offset().unwrap().to_bits(),
                "offset must match bit for bit at {:?}",
                cluster.path(),
            );
        }
        assert!(positioned.next().is_none(), "no extra clusters");
        assert!(positioned.next().is_none(), "iteration stays exhausted");
    }
}

fn position_key((cluster, offset): (crate::Cluster<'_, ()>, f32)) -> (crate::ClusterPath, u32) {
    (cluster.path(), offset.to_bits())
}

#[test]
fn positions_match_after_rebreaking_and_realigning_styled_bidi_text() {
    let text = "office a\u{301} العربية AV alpha beta gamma delta\nאבג end ";
    let split = text.find('ا').unwrap();
    let mut fonts = create_font_context();
    let mut context = LayoutContext::<()>::new();
    for direction in [BaseDirection::Ltr, BaseDirection::Rtl] {
        let mut builder = context.style_run_builder(&mut fonts, text, 1.0, false);
        builder.set_base_direction(direction);
        let first = builder.push_style(TextStyle {
            font_family: FontFamily::from(FONT_FAMILY_LIST),
            letter_spacing: 0.75,
            word_spacing: 2.25,
            ..TextStyle::default()
        });
        let second = builder.push_style(TextStyle {
            font_family: FontFamily::from(FONT_FAMILY_LIST),
            font_size: 20.0,
            letter_spacing: -0.25,
            ..TextStyle::default()
        });
        builder.push_style_run(first, ..split);
        builder.push_style_run(second, split..);
        for (id, (index, width)) in [
            (0, 8.125),
            (split, 11.75),
            (split, 0.0),
            (split, 5.5),
            (text.len(), 6.625),
        ]
        .into_iter()
        .enumerate()
        {
            builder.push_inline_box(inline_box(id as u64, index, width));
        }
        let mut layout = builder.build(text);
        layout.set_text_indent(9.25, IndentOptions::default());
        for width in [Some(120.0), None, Some(83.0), Some(120.0)] {
            layout.break_all_lines(width);
            for alignment in [
                Alignment::Justify,
                Alignment::End,
                Alignment::Center,
                Alignment::Left,
                Alignment::Right,
                Alignment::Start,
            ] {
                layout.align(alignment, AlignmentOptions::default());
                assert_positions_match(&layout);
            }
        }
    }
}

#[test]
fn positions_follow_visual_item_reordering_and_iterator_clones_are_independent() {
    let text = "office العربية end";
    let mut fonts = create_font_context();
    let mut context = LayoutContext::<()>::new();
    let mut builder = context.ranged_builder(&mut fonts, text, 1.0, false);
    builder.push_default(FontFamily::from(FONT_FAMILY_LIST));
    builder.push_inline_box(inline_box(0, 0, 7.125));
    builder.push_inline_box(inline_box(1, 7, 9.375));
    let mut layout = builder.build(text);
    let mut breaker = layout.break_lines();
    breaker.state_mut().set_line_x(13.25);
    breaker.break_remaining(f32::MAX);
    let line = layout.get(0).unwrap();
    assert_eq!(
        line.metrics().inline_min_coord,
        13.25,
        "exercise a nonzero line origin"
    );
    let boxes = line
        .inline_box_indices()
        .map(|(index, _)| index)
        .collect::<alloc::vec::Vec<_>>();
    // Hosts may move decoration boxes, but text runs must keep their relative order.
    let order = (0..line.len())
        .filter(|index| !boxes.contains(index))
        .chain(boxes.iter().rev().copied())
        .collect::<alloc::vec::Vec<_>>();
    layout.reorder_line_items(0, &order);
    assert_positions_match(&layout);

    let line = layout.get(0).unwrap();
    let expected = line
        .runs()
        .flat_map(|run| run.visual_clusters())
        .map(|cluster| position_key((cluster, cluster.visual_offset().unwrap())))
        .collect::<alloc::vec::Vec<_>>();
    let positions = line.positioned_visual_clusters();
    for index in 0..expected.len() {
        let mut skipped = positions.clone();
        assert_eq!(
            skipped.nth(index).map(position_key),
            Some(expected[index]),
            "nth must accumulate skipped advances at {index}"
        );
        assert_eq!(
            skipped.map(position_key).collect::<alloc::vec::Vec<_>>(),
            expected[index + 1..],
            "iteration after nth at {index}"
        );
        assert_eq!(
            positions
                .clone()
                .skip(index)
                .map(position_key)
                .collect::<alloc::vec::Vec<_>>(),
            expected[index..],
            "skip at {index}"
        );
    }
    assert_eq!(
        positions.last().map(position_key),
        expected.last().copied(),
        "last must accumulate preceding advances"
    );

    // The iterator borrows the layout, rather than the temporary Line handle.
    let mut positioned = layout.get(0).unwrap().positioned_visual_clusters();
    let first = positioned.next().unwrap();
    let mut cloned = positioned.clone();
    let second = positioned.next().unwrap();
    assert_ne!(first.0.path(), second.0.path(), "the iterator must advance");
    let cloned_second = cloned.next().unwrap();
    assert_eq!(
        second.0.path(),
        cloned_second.0.path(),
        "clones share a starting position"
    );
    assert_eq!(
        second.1.to_bits(),
        cloned_second.1.to_bits(),
        "clones preserve the offset"
    );
    assert_eq!(
        positioned
            .map(|(cluster, offset)| (cluster.path(), offset.to_bits()))
            .collect::<alloc::vec::Vec<_>>(),
        cloned
            .map(|(cluster, offset)| (cluster.path(), offset.to_bits()))
            .collect::<alloc::vec::Vec<_>>(),
        "clones preserve run transitions"
    );
}

#[test]
fn empty_text_and_box_only_lines_have_no_positioned_clusters() {
    let mut fonts = create_font_context();
    let mut context = LayoutContext::<()>::new();
    for direction in [BaseDirection::Ltr, BaseDirection::Rtl] {
        for box_count in [0, 3] {
            let mut builder = context.ranged_builder(&mut fonts, "", 1.0, false);
            builder.set_base_direction(direction);
            for id in 0..box_count {
                builder.push_inline_box(inline_box(id, 0, 5.5));
            }
            let mut layout = builder.build("");
            layout.break_all_lines(Some(200.0));
            assert!(!layout.is_empty(), "there must be a line to visit");
            assert_eq!(
                layout
                    .lines()
                    .flat_map(|line| line.positioned_visual_clusters())
                    .count(),
                0
            );
            assert_positions_match(&layout);
        }
    }
}

#[test]
fn long_single_run_positions_preserve_fractional_advances() {
    let text = "A".repeat(8192);
    let mut fonts = create_font_context();
    let mut context = LayoutContext::<()>::new();
    let mut builder = context.ranged_builder(&mut fonts, &text, 1.0, false);
    builder.push_default(FontFamily::from(FONT_FAMILY_LIST));
    builder.push_default(crate::StyleProperty::LetterSpacing(0.37));
    let mut layout = builder.build(&text);
    layout.break_all_lines(None);
    assert_eq!(layout.len(), 1, "the long text must remain on one line");
    let line = layout.get(0).unwrap();
    assert_eq!(line.runs().count(), 1, "exercise a long individual run");
    let mut count = 0;
    for (index, (cluster, offset)) in line.positioned_visual_clusters().enumerate() {
        assert_eq!(
            cluster.text_range(),
            index..index + 1,
            "visit every cluster once"
        );
        // Sample the quadratic reference without making the regression test quadratic itself.
        if [0, 1, 127, 1024, 8191].contains(&index) {
            assert_eq!(
                offset.to_bits(),
                cluster.visual_offset().unwrap().to_bits(),
                "long-run rounding at {index}"
            );
        }
        count += 1;
    }
    assert_eq!(count, text.len(), "all long-run clusters must be returned");
}
