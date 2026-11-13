//! Pass-local visual truncation, following Blink's LineTruncator. Original
//! shaped text, source offsets, intrinsic widths and CSSOM fragments stay intact.

use std::{fmt::Debug, hash::Hash, ops::Range, sync::Arc};

use parley::{Layout, PositionedLayoutItem};

use crate::{
    DocumentLayoutServices, LayoutBoxId, LayoutBoxKind, LayoutWorld,
    inline::{InlineFormattingContext, InlineObjectRole, glyph_cluster_ranges},
    style::{InlineDirection, LayoutOverflowMode},
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

pub(crate) fn prepare_text_overflow<N>(
    world: &mut LayoutWorld<N>,
    services: &mut DocumentLayoutServices,
) where
    N: Copy + Debug + Eq + Hash,
{
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
        let rtl = style.direction() == InlineDirection::Rtl;
        if !text.lines().any(|line| line_overflows(&line, width, rtl)) {
            continue;
        }
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
                    && !has_positioned_descendant(world, id)
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
                    let source = context.source_map.iter().find(|source| {
                        if at_start {
                            source.output_range.start <= edge && edge < source.output_range.end
                        } else {
                            source.output_range.start < edge && edge <= source.output_range.end
                        }
                    });
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
    advance > end - start + tolerance
        || if rtl {
            metrics.offset + metrics.hanging_advance < -tolerance
        } else {
            metrics.offset + advance > end - start + tolerance
        }
}

fn has_positioned_descendant<N: Copy + Debug + Eq + Hash>(
    world: &LayoutWorld<N>,
    id: LayoutBoxId,
) -> bool {
    world.boxes[id.index()].children.iter().any(|child| {
        world.boxes[child.index()].style.is_absolute_positioned()
            || world.boxes[child.index()].style.is_fixed_positioned()
            || has_positioned_descendant(world, *child)
    })
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
