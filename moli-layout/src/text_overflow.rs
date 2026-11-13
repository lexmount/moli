//! Pass-local visual truncation, following Blink's LineTruncator. Original
//! shaped text, source offsets, intrinsic widths and CSSOM fragments stay intact.

use std::{fmt::Debug, hash::Hash, ops::Range, sync::Arc};

use parley::{Layout, PositionedLayoutItem};

use crate::{
    DocumentLayoutServices, LayoutBoxId, LayoutBoxKind, LayoutWorld,
    inline::{
        InlineFormattingContext, InlineObjectRole, InlineSourceMapEntry, glyph_cluster_ranges,
    },
    style::LayoutOverflowMode,
    stylo_to_parley::TextBrush,
};

#[derive(Debug)]
pub(crate) struct TextOverflow {
    pub(crate) marker: Layout<TextBrush>,
    pub(crate) marker_text: Arc<str>,
    pub(crate) marker_width: f32,
    pub(crate) lines: Vec<Option<EllipsisLine>>,
    pub(crate) hidden_boxes: Vec<LayoutBoxId>,
}

#[derive(Debug)]
pub(crate) struct EllipsisLine {
    pub(crate) marker_offset: f32,
    pub(crate) visible_start: f32,
    pub(crate) visible_end: f32,
    pub(crate) target: LayoutBoxId,
    pub(crate) source_offset: usize,
    pub(crate) rtl: bool,
    visible_text: Vec<Range<usize>>,
}

impl EllipsisLine {
    pub(crate) fn text_is_visible(&self, range: &Range<usize>) -> bool {
        let index = self
            .visible_text
            .partition_point(|visible| visible.end <= range.start);
        self.visible_text
            .get(index)
            .is_some_and(|visible| visible.start <= range.start && range.end <= visible.end)
    }
}

#[derive(Debug)]
enum ChildKind {
    Text { range: Range<usize>, rtl: bool },
    Atomic(LayoutBoxId),
}

#[derive(Debug)]
struct VisualChild {
    start: f32,
    end: f32,
    kind: ChildKind,
}

/// Pass-local visibility shared by stacking, paint and its auxiliary phases.
/// Truncation hides normal-flow contents, while independently painted
/// positioned/stacking descendants can still participate in the outer context.
pub(crate) struct TextOverflowVisibility {
    hidden: Vec<bool>,
}

impl TextOverflowVisibility {
    pub(crate) fn new<N: Copy + Debug + Eq + Hash>(world: &LayoutWorld<N>) -> Self {
        let mut hidden = vec![false; world.boxes.len()];
        let mut has_hidden = false;
        for layout_box in &world.boxes {
            if let Some(overflow) = layout_box
                .inline_layout
                .as_ref()
                .and_then(|context| context.text_overflow.as_ref())
            {
                for id in &overflow.hidden_boxes {
                    hidden[id.index()] = true;
                    has_hidden = true;
                }
            }
        }
        if !has_hidden {
            return Self { hidden };
        }
        let mut stack = vec![(world.root, false)];
        while let Some((id, parent_hidden)) = stack.pop() {
            let layout_box = &world.boxes[id.index()];
            let flex_or_grid_item = layout_box.parent.is_some_and(|parent| {
                let display = world.boxes[parent.index()].style.display();
                display.is_flex_container() || display.is_grid_container()
            });
            let independent = layout_box.style.position() != crate::LayoutPosition::Static
                || layout_box.creates_stacking_context(false, flex_or_grid_item);
            hidden[id.index()] |= parent_hidden && !independent;
            stack.extend(
                layout_box
                    .children
                    .iter()
                    .map(|child| (*child, hidden[id.index()])),
            );
        }
        Self { hidden }
    }

    pub(crate) fn is_hidden(&self, id: LayoutBoxId) -> bool {
        self.hidden[id.index()]
    }
}

pub(crate) fn prepare_text_overflow<N>(
    world: &mut LayoutWorld<N>,
    services: &mut DocumentLayoutServices,
) where
    N: Copy + Debug + Eq + Hash,
{
    let mut propagating_oof = None;
    for index in 0..world.boxes.len() {
        let layout_box = &world.boxes[index];
        let Some(context) = layout_box.inline_layout.as_ref() else {
            continue;
        };
        let style_box = if layout_box.kind() == LayoutBoxKind::AnonymousBlock {
            layout_box.parent.unwrap_or(LayoutBoxId::from_index(index))
        } else {
            LayoutBoxId::from_index(index)
        };
        let style = &world.boxes[style_box.index()].style;
        if !style.text_overflow_ellipsis() {
            continue;
        }
        let vertical = style.writing_mode() != taffy::WritingMode::HorizontalTb;
        let modes = style.overflow_modes();
        if modes[usize::from(vertical)] == LayoutOverflowMode::Visible {
            continue;
        }
        let scroll = world.boxes[style_box.index()].scroll_offset;
        if if vertical {
            scroll.y != 0.0
        } else {
            scroll.x != 0.0
        } {
            continue;
        }
        let layout = layout_box.final_layout;
        let width = if vertical {
            layout.size.height
                - layout.border.top
                - layout.border.bottom
                - layout.padding.top
                - layout.padding.bottom
                - layout.scrollbar_size.height
        } else {
            layout.size.width
                - layout.border.left
                - layout.border.right
                - layout.padding.left
                - layout.padding.right
                - layout.scrollbar_size.width
        }
        .max(0.0);
        let Some(text) = context.laid_out.as_ref() else {
            continue;
        };
        // Use the same resolved paragraph base direction as line breaking and
        // alignment. In plaintext it can differ from the CSS direction, and
        // an individual bidi run's direction does not describe the paragraph.
        let rtl = text.is_rtl();
        if !text.lines().any(|line| line_overflows(&line, width, rtl)) {
            continue;
        }
        let propagating_oof =
            propagating_oof.get_or_insert_with(|| propagating_positioned_descendants(world));
        let parley = services.parley_mut();
        let mut marker_style = style.parley_text_style();
        // Blink shapes the generated marker directly with the line's font,
        // without text spacing or the original text's decoration fragments.
        marker_style.letter_spacing = 0.0;
        marker_style.word_spacing = 0.0;
        marker_style.brush.decoration = Default::default();
        parley.resolve_font_families(&mut marker_style, None);
        let primary = parley.primary_font_identity(&marker_style);
        let shape = |parley: &mut crate::text::ParleyDocumentServices, text: &str| {
            let mut builder =
                parley
                    .layout_context
                    .style_run_builder(&mut parley.font_context, text, 1.0, true);
            let style = builder.push_style(marker_style.clone());
            builder.push_style_run(style, ..);
            let mut layout = builder.build(text);
            layout.break_all_lines(None);
            layout
        };
        let mut marker = shape(parley, "…");
        let use_periods = marker
            .lines()
            .flat_map(|line| line.runs())
            .next()
            .is_none_or(|run| {
                primary.is_some_and(|primary| {
                    primary != (run.font().font.data.id(), run.font().font.index)
                }) || run
                    .clusters()
                    .flat_map(|cluster| cluster.glyphs())
                    .any(|glyph| glyph.id == 0)
            });
        let marker_text: Arc<str> = if use_periods {
            marker = shape(parley, "...");
            Arc::from("...")
        } else {
            Arc::from("…")
        };
        let marker_width = marker
            .lines()
            .next()
            .map_or(0.0, |line| line.metrics().advance);
        if marker_width <= 0.0 {
            continue;
        }
        let mut overflow = TextOverflow {
            marker,
            marker_text,
            marker_width,
            lines: Vec::new(),
            hidden_boxes: Vec::new(),
        };
        for line in text.lines() {
            if !line_overflows(&line, width, rtl) {
                overflow.lines.push(None);
                continue;
            }
            let (start, end) = line_bounds(&line, width);
            let children = visual_children(context, &line);
            let ordered = (0..children.len()).map(|index| {
                if rtl {
                    children.len() - index - 1
                } else {
                    index
                }
            });
            let mut last = None;
            for (position, child_index) in ordered.enumerate() {
                let child = &children[child_index];
                let fits = if rtl {
                    child.start >= start + marker_width
                } else {
                    child.end <= end - marker_width
                };
                // The first character/atomic object is clipped rather than
                // ellipsed, even when the marker cannot fit in the box.
                if fits || position == 0 {
                    last = Some(child_index);
                } else {
                    break;
                }
            }
            let Some(last) = last else {
                overflow.lines.push(None);
                continue;
            };
            let child = &children[last];
            if if rtl {
                child.end <= start
            } else {
                child.start >= end
            } {
                overflow.lines.push(None);
                continue;
            }
            let edge = if rtl { child.start } else { child.end };
            let marker_offset = if rtl { edge - marker_width } else { edge };
            let mut visible_text = Vec::new();
            for (child_index, child) in children.iter().enumerate() {
                if if rtl {
                    child_index >= last
                } else {
                    child_index <= last
                } {
                    if let ChildKind::Text { range, .. } = &child.kind {
                        visible_text.push(range.clone());
                    }
                } else if let ChildKind::Atomic(id) = child.kind
                    && !propagating_oof[id.index()]
                {
                    // Blink leaves these fragments available so their OOF
                    // descendants can still participate in the outer context.
                    overflow.hidden_boxes.push(id);
                }
            }
            visible_text.sort_unstable_by_key(|range| range.start);
            let mut merged: Vec<Range<usize>> = Vec::new();
            for range in visible_text {
                if let Some(last) = merged.last_mut()
                    && range.start <= last.end
                {
                    last.end = last.end.max(range.end);
                } else {
                    merged.push(range);
                }
            }
            let (target, source_offset) = match &children[last].kind {
                ChildKind::Atomic(id) => (*id, 0),
                ChildKind::Text {
                    range,
                    rtl: child_rtl,
                } => {
                    let at_start = rtl != *child_rtl;
                    let edge = if at_start { range.start } else { range.end };
                    let source = source_at_edge(&context.source_map, edge, at_start);
                    source.map_or((context.root_style, 0), |source| {
                        (
                            source.box_id,
                            if at_start {
                                source.source_utf16_range.start
                            } else {
                                source.source_utf16_range.end
                            },
                        )
                    })
                }
            };
            overflow.lines.push(Some(EllipsisLine {
                marker_offset,
                visible_start: if rtl { edge } else { f32::NEG_INFINITY },
                visible_end: if rtl { f32::INFINITY } else { edge },
                target,
                source_offset,
                rtl,
                visible_text: merged,
            }));
        }
        let context = world.boxes[index].inline_layout.as_mut().unwrap();
        for (index, ellipsis) in overflow.lines.iter().enumerate() {
            let Some(ellipsis) = ellipsis else {
                continue;
            };
            for line in overflow.marker.lines() {
                for run in line.runs() {
                    let metrics = run.font_metrics();
                    let rect = crate::LayoutRect::new(
                        ellipsis.marker_offset,
                        context.line_placements[index].baseline - metrics.ascent,
                        overflow.marker_width,
                        metrics.ascent + metrics.descent,
                    );
                    for style in overflow.marker.styles() {
                        context.fragments.lines[index]
                            .paint_bounds
                            .include_text_ink(rect, style, run.font_size());
                    }
                }
            }
        }
        context.text_overflow = Some(Box::new(overflow));
    }
}

fn source_at_edge(
    sources: &[InlineSourceMapEntry],
    edge: usize,
    at_start: bool,
) -> Option<&InlineSourceMapEntry> {
    // Normalization produces monotonic starts and ends, including duplicate
    // origins for collapsed text. Find the first containing origin without
    // rescanning the entire IFC for every overflowing line.
    let index = sources.partition_point(|source| {
        if at_start {
            source.output_range.end <= edge
        } else {
            source.output_range.end < edge
        }
    });
    sources.get(index).filter(|source| {
        if at_start {
            source.output_range.start <= edge && edge < source.output_range.end
        } else {
            source.output_range.start < edge && edge <= source.output_range.end
        }
    })
}

fn line_bounds(line: &parley::Line<'_, TextBrush>, width: f32) -> (f32, f32) {
    // Float exclusions can narrow a line independently of the block width.
    let metrics = line.metrics();
    (
        metrics.inline_min_coord.max(0.0),
        metrics.inline_max_coord.min(width),
    )
}

fn line_overflows(line: &parley::Line<'_, TextBrush>, width: f32, rtl: bool) -> bool {
    let metrics = line.metrics();
    let (start, end) = line_bounds(line, width);
    let advance = metrics.advance - metrics.hanging_advance;
    let tolerance = 1.0 / 64.0;
    // Offset is relative to the float-constrained line slot. Compare the
    // positioned inline-end against that same slot in layout coordinates;
    // a wide run shifted past inline-start need not overflow inline-end.
    let origin = metrics.inline_min_coord + metrics.offset;
    if rtl {
        origin + metrics.hanging_advance < start - tolerance
    } else {
        origin + advance > end + tolerance
    }
}

fn propagating_positioned_descendants<N: Copy + Debug + Eq + Hash>(
    world: &LayoutWorld<N>,
) -> Vec<bool> {
    #[derive(Clone, Copy, Default)]
    struct Pending {
        absolute: bool,
        fixed: bool,
    }
    let mut order = vec![world.root];
    let mut cursor = 0;
    while cursor < order.len() {
        let id = order[cursor];
        order.extend(&world.boxes[id.index()].children);
        cursor += 1;
    }
    let mut pending = vec![Pending::default(); world.boxes.len()];
    let mut propagating = vec![false; world.boxes.len()];
    for id in order.into_iter().rev() {
        let layout_box = &world.boxes[id.index()];
        let mut descendants = pending[id.index()];
        // Only OOF information still escaping this box prevents truncation.
        // An absolute child already placed by this box's containing block is
        // not equivalent to Blink's pending OofPositionedDescendants.
        descendants.absolute &= !world.establishes_positioned_containing_block(id);
        descendants.fixed &= !world.establishes_fixed_containing_block(id);
        propagating[id.index()] = descendants.absolute || descendants.fixed;
        if let Some(parent) = layout_box.parent {
            pending[parent.index()].absolute |=
                descendants.absolute || layout_box.style.is_absolute_positioned();
            pending[parent.index()].fixed |=
                descendants.fixed || layout_box.style.is_fixed_positioned();
        }
    }
    propagating
}

fn visual_children(
    context: &InlineFormattingContext,
    line: &parley::Line<'_, TextBrush>,
) -> Vec<VisualChild> {
    let mut children: Vec<VisualChild> = Vec::new();
    let mut mapped_run = None;
    let mut ranges = Vec::new().into_iter();
    for item in line.items() {
        match item {
            PositionedLayoutItem::InlineBox(item) => {
                if let Some(object) = context.object(item.id)
                    && object.role == InlineObjectRole::Atomic
                {
                    children.push(VisualChild {
                        start: item.x,
                        end: item.x + item.width,
                        kind: ChildKind::Atomic(object.box_id),
                    });
                }
            }
            PositionedLayoutItem::GlyphRun(glyphs) => {
                if mapped_run != Some(glyphs.run().index()) {
                    ranges = glyph_cluster_ranges(glyphs.run()).into_iter();
                    mapped_run = Some(glyphs.run().index());
                }
                let mut offset = glyphs.offset();
                for (glyph, range) in glyphs.glyphs().zip(ranges.by_ref()) {
                    let end = offset + glyph.advance;
                    if glyphs.style().brush.paint {
                        if let Some(last) = children.last_mut()
                            && matches!(&last.kind, ChildKind::Text { range: last_range, .. } if *last_range == range)
                        {
                            last.end = end;
                        } else {
                            children.push(VisualChild {
                                start: offset,
                                end,
                                kind: ChildKind::Text {
                                    range,
                                    rtl: glyphs.run().is_rtl(),
                                },
                            });
                        }
                    }
                    offset = end;
                }
            }
        }
    }
    children
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_source_edges_preserve_boundary_and_collapsed_origin_identity() {
        let sources = [0..1, 1..3, 1..3, 3..7, 9..12].map(|output_range| InlineSourceMapEntry {
            source_byte_range: output_range.clone(),
            source_utf16_range: output_range.clone(),
            output_range,
            box_id: LayoutBoxId::from_index(0),
        });
        for edge in 0..=13 {
            for at_start in [false, true] {
                let expected = sources.iter().position(|source| {
                    if at_start {
                        source.output_range.contains(&edge)
                    } else {
                        source.output_range.start < edge && edge <= source.output_range.end
                    }
                });
                let actual = source_at_edge(&sources, edge, at_start).map(|source| {
                    sources
                        .iter()
                        .position(|entry| std::ptr::eq(entry, source))
                        .unwrap()
                });
                assert_eq!(actual, expected, "edge={edge}, at_start={at_start}");
            }
        }
    }
}
