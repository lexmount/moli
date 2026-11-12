// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec::Vec;
use parlance::BidiLevel;

use super::utils::fonts::{FONT_FAMILY_LIST, create_font_context};
use crate::layout::data::LayoutItemKind;
use crate::{
    BaseDirection, FontFamily, InlineBox, InlineBoxKind, LayoutContext, PositionedLayoutItem,
    TextStyle, VerticalAlign,
};

fn inline_box(id: u64, index: usize, kind: InlineBoxKind) -> InlineBox {
    InlineBox {
        id,
        kind,
        index,
        width: 10.0,
        height: 10.0,
        baseline: None,
        vertical_align: VerticalAlign::BASELINE,
    }
}

#[test]
fn all_box_kinds_resolve_outside_closed_bidi_contexts() {
    let mut fonts = create_font_context();
    let mut context = LayoutContext::<()>::new();
    for kind in [
        InlineBoxKind::InFlow,
        InlineBoxKind::OutOfFlow,
        InlineBoxKind::CustomOutOfFlow,
    ] {
        for text in [
            "\u{202e}x\u{202c}",
            "\u{202b}x\u{202c}",
            "\u{2067}x\u{2069}",
            "😀\u{202e}אב\u{202c}",
        ] {
            let mut builder = context.ranged_builder(&mut fonts, text, 1.0, false);
            builder.push_default(FontFamily::from(FONT_FAMILY_LIST));
            builder.set_base_direction(BaseDirection::Ltr);
            builder.push_inline_box(inline_box(42, text.len(), kind));
            let layout = builder.build(text);
            let item = layout
                .data
                .items
                .iter()
                .find(|item| item.kind == LayoutItemKind::InlineBox)
                .expect("the object must remain in the layout");
            assert_eq!(item.bidi_level, BidiLevel::new(0), "{kind:?}/{text:?}");
            assert_eq!(layout.text_len(), text.len());
        }
    }
}

#[test]
fn objects_after_closed_rtl_contexts_keep_their_visual_position_across_lines() {
    let first_line = "\u{202e}ab\u{202c}x";
    let text = "\u{202e}ab\u{202c}x\n\u{202e}cd\u{202c}x";
    let inside_first = "\u{202e}a".len();
    let outside_first = "\u{202e}ab\u{202c}".len();
    let mut fonts = create_font_context();
    let mut context = LayoutContext::<()>::new();
    let mut builder = context.ranged_builder(&mut fonts, text, 1.0, false);
    builder.push_default(FontFamily::from(FONT_FAMILY_LIST));
    builder.set_base_direction(BaseDirection::Ltr);
    for (id, index) in [
        (1, inside_first),
        (2, outside_first),
        (3, first_line.len() + 1 + inside_first),
        (4, first_line.len() + 1 + outside_first),
    ] {
        builder.push_inline_box(inline_box(id, index, InlineBoxKind::InFlow));
    }
    let mut layout = builder.build(text);
    layout.break_all_lines(None);

    let lines = layout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    for (line, expected) in lines.iter().zip([[1, 2], [3, 4]]) {
        let boxes = line
            .items()
            .filter_map(|item| match item {
                PositionedLayoutItem::InlineBox(item) => Some(item),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            boxes.iter().map(|item| item.id).collect::<Vec<_>>(),
            expected
        );
        assert!(
            boxes[0].x < boxes[1].x,
            "the object after PDF must stay to the right of the RTL content"
        );
    }
}

#[test]
fn empty_rtl_text_reorders_boxes_without_text_runs() {
    let mut fonts = create_font_context();
    let mut context = LayoutContext::<()>::new();
    let mut builder = context.ranged_builder(&mut fonts, "", 1.0, false);
    builder.set_base_direction(BaseDirection::Rtl);
    for id in [1, 2] {
        builder.push_inline_box(inline_box(id, 0, InlineBoxKind::InFlow));
    }
    let mut layout = builder.build("");
    layout.break_all_lines(None);
    let ids = layout
        .lines()
        .flat_map(|line| line.items())
        .filter_map(|item| match item {
            PositionedLayoutItem::InlineBox(item) => Some(item.id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(ids, [2, 1]);
    assert_eq!(layout.text_len(), 0);
}

#[test]
fn style_run_and_tree_builders_resolve_object_levels() {
    let text = "\u{202e}x\u{202c}";
    let mut fonts = create_font_context();
    let mut context = LayoutContext::<()>::new();
    for kind in [
        InlineBoxKind::InFlow,
        InlineBoxKind::OutOfFlow,
        InlineBoxKind::CustomOutOfFlow,
    ] {
        let style = TextStyle {
            font_family: FontFamily::from(FONT_FAMILY_LIST),
            ..TextStyle::default()
        };
        let mut builder = context.style_run_builder(&mut fonts, text, 1.0, false);
        let style_index = builder.push_style(style.clone());
        builder.push_style_run(style_index, ..);
        builder.set_base_direction(BaseDirection::Ltr);
        builder.push_inline_box(inline_box(42, text.len(), kind));
        let layout = builder.build(text);
        let item = layout
            .data
            .items
            .iter()
            .find(|item| item.kind == LayoutItemKind::InlineBox)
            .unwrap();
        assert_eq!(item.bidi_level, BidiLevel::new(0), "style-run/{kind:?}");

        let mut builder = context.tree_builder(&mut fonts, 1.0, false, &style);
        builder.set_base_direction(BaseDirection::Ltr);
        builder.push_text(text);
        builder.push_inline_box(inline_box(42, usize::MAX, kind));
        let (layout, source) = builder.build();
        assert_eq!(source, text);
        assert_eq!(layout.data.inline_boxes[0].inline_box.index, text.len());
        let item = layout
            .data
            .items
            .iter()
            .find(|item| item.kind == LayoutItemKind::InlineBox)
            .unwrap();
        assert_eq!(item.bidi_level, BidiLevel::new(0), "tree/{kind:?}");
    }
}

#[test]
fn sorting_boxes_preserves_insertion_order_at_shared_anchors() {
    let mut fonts = create_font_context();
    let mut context = LayoutContext::<()>::new();
    for direction in [BaseDirection::Ltr, BaseDirection::Rtl] {
        let text = "x";
        let mut builder = context.ranged_builder(&mut fonts, text, 1.0, false);
        builder.set_base_direction(direction);
        for (id, index) in [(4, 1), (2, 0), (3, 0), (1, 0)] {
            builder.push_inline_box(inline_box(id, index, InlineBoxKind::InFlow));
        }
        let mut layout = builder.build(text);
        let logical_ids = layout
            .data
            .inline_boxes
            .iter()
            .map(|input| input.inline_box.id)
            .collect::<Vec<_>>();
        assert_eq!(logical_ids, [2, 3, 1, 4]);
        for _ in 0..2 {
            layout.break_all_lines(None);
            let visual_ids = layout
                .lines()
                .flat_map(|line| line.items())
                .filter_map(|item| match item {
                    PositionedLayoutItem::InlineBox(item) => Some(item.id),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let expected = if direction == BaseDirection::Ltr {
                [2, 3, 1, 4]
            } else {
                [4, 1, 3, 2]
            };
            assert_eq!(visual_ids, expected);
        }
    }
}

#[test]
#[should_panic(expected = "object index must be a character boundary within text")]
fn builder_rejects_anchor_inside_utf8_character() {
    let mut fonts = create_font_context();
    let mut context = LayoutContext::<()>::new();
    let mut builder = context.ranged_builder(&mut fonts, "😀", 1.0, false);
    builder.push_inline_box(inline_box(42, 2, InlineBoxKind::InFlow));
    builder.build("😀");
}

#[test]
#[should_panic(expected = "object index must be a character boundary within text")]
fn builder_rejects_anchor_past_empty_text() {
    let mut fonts = create_font_context();
    let mut context = LayoutContext::<()>::new();
    let mut builder = context.ranged_builder(&mut fonts, "", 1.0, false);
    builder.push_inline_box(inline_box(42, 1, InlineBoxKind::InFlow));
    builder.build("");
}
