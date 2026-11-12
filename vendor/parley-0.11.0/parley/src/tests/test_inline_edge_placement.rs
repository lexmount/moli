// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::utils::fonts::{FONT_FAMILY_LIST, create_font_context};
use crate::FontFamily;
use crate::layout::data::LayoutItemKind;
use crate::{
    BaseDirection, InlineBox, InlineBoxBidi, InlineBoxKind, LayoutContext, PositionedLayoutItem,
    VerticalAlign,
};
use alloc::vec::Vec;

#[test]
fn visual_box_repositioning_keeps_text_ranges_and_advances() {
    let text = "aאבb";
    let mut fonts = create_font_context();
    let mut context = LayoutContext::<()>::new();
    let mut builder = context.ranged_builder(&mut fonts, text, 1.0, false);
    builder.push_default(FontFamily::from(FONT_FAMILY_LIST));
    builder.set_base_direction(BaseDirection::Ltr);
    for (id, index, width, bidi) in [
        (0, 1, 20.0, InlineBoxBidi::InheritNext),
        (1, 5, 0.0, InlineBoxBidi::InheritPrevious),
    ] {
        builder.push_inline_box_with_bidi(
            InlineBox {
                id,
                index,
                width,
                kind: InlineBoxKind::InFlow,
                height: 0.0,
                baseline: None,
                vertical_align: VerticalAlign::default(),
            },
            bidi,
        );
    }
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    let before = layout.lines().next().unwrap();
    let width = before.metrics().advance;
    let ranges = before
        .runs()
        .map(|run| run.text_range())
        .collect::<Vec<_>>();
    let old_left = before
        .runs()
        .find(|run| run.text_range().start == 1)
        .unwrap()
        .visual_clusters()
        .next()
        .unwrap()
        .visual_offset()
        .unwrap();
    let items = &layout.data.line_items[layout.data.lines[0].item_range.clone()];
    let start = items
        .iter()
        .position(|item| item.kind == LayoutItemKind::InlineBox && item.index == 0)
        .unwrap();
    let end = items
        .iter()
        .position(|item| item.kind == LayoutItemKind::InlineBox && item.index == 1)
        .unwrap();
    let text_slot = items
        .iter()
        .position(|item| item.kind == LayoutItemKind::TextRun && item.text_range.start == 1)
        .unwrap();
    let mut order = (0..items.len())
        .filter(|index| *index != start && *index != end)
        .collect::<Vec<_>>();
    let position = order.iter().position(|index| *index == text_slot).unwrap();
    order.insert(position + 1, end);
    order.insert(position, start);
    layout.reorder_line_items(0, &order);
    let after = layout.lines().next().unwrap();
    assert_eq!(after.metrics().advance, width);
    assert_eq!(
        after.runs().map(|run| run.text_range()).collect::<Vec<_>>(),
        ranges
    );
    let new_left = after
        .runs()
        .find(|run| run.text_range().start == 1)
        .unwrap()
        .visual_clusters()
        .next()
        .unwrap()
        .visual_offset()
        .unwrap();
    assert!((new_left - old_left - 20.0).abs() < 0.001);
    let positions = after
        .items()
        .filter_map(|item| match item {
            PositionedLayoutItem::InlineBox(item) => Some(item),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(positions[0].id, 0);
    assert_eq!(positions[1].id, 1);
    assert_eq!(layout.text_len(), text.len());
    // A new break pass rebuilds the visual order from the unchanged logical items.
    layout.break_all_lines(None);
    let reset = layout
        .lines()
        .next()
        .unwrap()
        .runs()
        .find(|run| run.text_range().start == 1)
        .unwrap()
        .visual_clusters()
        .next()
        .unwrap()
        .visual_offset()
        .unwrap();
    assert_eq!(reset, old_left);
}
