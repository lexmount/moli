//! Final, pass-owned inline items. Source layout stays intact; all visual
//! consumers receive views whose shape, placement and participation are final.

use std::{fmt::Debug, hash::Hash, ops::Range, sync::Arc};

use parley::{
    Affinity, GlyphRun, Layout, PositionedLayoutItem,
    editing::{Cursor, Selection},
};

use super::{
    InlineFormattingContext, InlineObjectRole, InlinePaintBounds, InlineSelection, truncation,
};
use crate::{
    DocumentLayoutServices, LayoutBoxId, LayoutPoint, LayoutRect, LayoutWorld, PaintColor,
    stylo_to_parley::TextBrush,
};

pub(crate) struct GeneratedText {
    layout: Layout<TextBrush>,
    text: Arc<str>,
    width: f32,
}

pub(crate) struct GeneratedTextPool(Vec<Option<Box<GeneratedText>>>);

/// A construction plan is consumed by finish(). It never reaches paint or the
/// frozen tree. Only the independent generated-shape pool is borrowed by items.
pub(crate) struct DisplayBuilder {
    paragraphs: Vec<Option<Vec<Option<truncation::PreparedLine>>>>,
    paints_self: Vec<bool>,
}

pub(crate) fn prepare<N: Copy + Debug + Eq + Hash>(
    world: &LayoutWorld<N>,
    services: &mut DocumentLayoutServices,
) -> (GeneratedTextPool, DisplayBuilder) {
    let prepared = truncation::prepare(world, services);
    if prepared.is_empty() {
        return (
            GeneratedTextPool(Vec::new()),
            DisplayBuilder {
                paragraphs: Vec::new(),
                paints_self: Vec::new(),
            },
        );
    }
    let mut pool = Vec::with_capacity(prepared.len());
    let mut paragraphs = Vec::with_capacity(prepared.len());
    let mut suppressed = vec![false; world.boxes.len()];
    for paragraph in prepared {
        if let Some(paragraph) = paragraph {
            for id in paragraph.hidden_boxes {
                suppressed[id.index()] = true;
            }
            pool.push(Some(Box::new(GeneratedText {
                layout: paragraph.marker,
                text: paragraph.marker_text,
                width: paragraph.marker_width,
            })));
            paragraphs.push(Some(paragraph.lines));
        } else {
            pool.push(None);
            paragraphs.push(None);
        }
    }
    if suppressed.iter().any(|suppressed| *suppressed) {
        let mut stack = vec![(world.root, false)];
        while let Some((id, parent_suppressed)) = stack.pop() {
            let layout_box = &world.boxes[id.index()];
            let flex_or_grid_item = layout_box.parent.is_some_and(|parent| {
                let display = world.boxes[parent.index()].style.display();
                display.is_flex_container() || display.is_grid_container()
            });
            let independent = layout_box.style.position() != crate::LayoutPosition::Static
                || layout_box.creates_stacking_context(false, flex_or_grid_item);
            suppressed[id.index()] |= parent_suppressed && !independent;
            stack.extend(
                layout_box
                    .children
                    .iter()
                    .map(|child| (*child, suppressed[id.index()])),
            );
        }
    }
    (
        GeneratedTextPool(pool),
        DisplayBuilder {
            paragraphs,
            paints_self: suppressed
                .into_iter()
                .map(|suppressed| !suppressed)
                .collect(),
        },
    )
}

#[derive(Clone, Copy)]
pub(crate) struct TextAnchor {
    pub(crate) target: LayoutBoxId,
    pub(crate) utf16_offset: usize,
    pub(crate) rtl: bool,
}

pub(crate) enum TextOrigin<'a> {
    Source,
    /// Generated text has a pointer/caret anchor, not invented source
    /// characters. Baseline selection policy leaves this text unselected.
    Generated {
        text: &'a Arc<str>,
        anchor: TextAnchor,
    },
}

pub(crate) struct TextView<'a> {
    pub(crate) glyphs: GlyphRun<'a, TextBrush>,
    /// Final placement in the IFC owner's local coordinate space. The shape
    /// retains its original inline offset and baseline, including RTL suffixes.
    pub(crate) offset: LayoutPoint,
    pub(crate) origin: TextOrigin<'a>,
}

pub(crate) enum InlineItemContent<'a> {
    Text(TextView<'a>),
    InlineBox {
        fragment: usize,
    },
    Atomic(LayoutBoxId),
    /// Glyphless source content (newlines, empty lines and object gaps).
    Control(Range<usize>),
}

pub(crate) struct InlineDisplayItem<'a> {
    pub(crate) line: usize,
    pub(crate) paint_owner: LayoutBoxId,
    /// Original visual item order. Generated line-end content follows the line.
    pub(crate) order: usize,
    pub(crate) content: InlineItemContent<'a>,
    /// Selection paint is finalized along with text, independent of glyphs.
    pub(crate) selection: Vec<LayoutRect>,
    pub(crate) paint_bounds: InlinePaintBounds,
}

pub(crate) struct DisplayLine {
    pub(crate) items: Range<usize>,
    pub(crate) paint_bounds: InlinePaintBounds,
    pub(crate) selection: Vec<LayoutRect>,
}

pub(crate) struct InlineDisplayContext<'a> {
    pub(crate) items: Vec<InlineDisplayItem<'a>>,
    pub(crate) lines: Vec<DisplayLine>,
    pub(crate) caret: Option<(LayoutRect, PaintColor)>,
    /// The final source hit geometry can reuse its original batch unchanged.
    /// Partial views instead project membership from the item stream.
    pub(crate) complete_source_geometry: bool,
}

#[derive(Default)]
pub(crate) struct InlineDisplayItems<'a> {
    contexts: Vec<Option<Box<InlineDisplayContext<'a>>>>,
    /// Final box participation, used to schedule backgrounds/effects and to
    /// stop ancestor text-mask traversal at suppressed boxes. Text participation
    /// itself is already resolved in items, including independent descendants.
    /// Empty when all boxes participate, avoiding per-box state for clean text.
    paints_self: Vec<bool>,
}

impl<'a> InlineDisplayItems<'a> {
    pub(crate) fn context(&self, owner: LayoutBoxId) -> Option<&InlineDisplayContext<'a>> {
        self.contexts.get(owner.index())?.as_deref()
    }

    pub(crate) fn paints_self(&self, id: LayoutBoxId) -> bool {
        paints_self(&self.paints_self, id)
    }
}

fn paints_self(boxes: &[bool], id: LayoutBoxId) -> bool {
    boxes.get(id.index()).copied().unwrap_or(true)
}

impl DisplayBuilder {
    pub(crate) fn finish<'a, N: Copy + Debug + Eq + Hash>(
        self,
        world: &'a LayoutWorld<N>,
        pool: &'a GeneratedTextPool,
        content_origin: impl Fn(LayoutBoxId) -> LayoutPoint,
    ) -> InlineDisplayItems<'a> {
        let contexts = world
            .boxes
            .iter()
            .enumerate()
            .map(|(index, layout_box)| {
                let context = layout_box.inline_layout.as_ref()?;
                let layout = context.laid_out.as_ref()?;
                let owner = LayoutBoxId::from_index(index);
                let origin = content_origin(owner);
                let plans = self.paragraphs.get(index).and_then(Option::as_deref);
                let mut result = InlineDisplayContext {
                    items: Vec::with_capacity(
                        layout.len() + context.fragments.boxes.len() + context.objects.len(),
                    ),
                    lines: Vec::with_capacity(layout.len()),
                    caret: caret(context).filter(|_| {
                        paints_self(&self.paints_self, owner) && layout_box.style.is_visible()
                    }),
                    complete_source_geometry: plans.is_none()
                        && paints_self(&self.paints_self, owner)
                        && context.source_map.iter().all(|source| {
                            paints_self(&self.paints_self, source.box_id)
                                && world.boxes[source.box_id.index()].style.is_visible()
                        }),
                };
                let mut boxes = if context.fragments.boxes.is_empty() {
                    Vec::new()
                } else {
                    vec![Vec::new(); layout.len()]
                };
                for (fragment, inline) in context.fragments.boxes.iter().enumerate() {
                    boxes[inline.line_index].push((fragment, inline.box_id));
                }
                let mut covered = Vec::new();
                let has_selection = matches!(context.selection, Some(InlineSelection::Range(_)));
                for (line_index, line) in layout.lines().enumerate() {
                    let start = result.items.len();
                    let plan = plans.and_then(|lines| lines[line_index].as_ref());
                    for &(fragment, target) in boxes.get(line_index).into_iter().flatten() {
                        if paints_self(&self.paints_self, target) {
                            result.items.push(InlineDisplayItem {
                                line: line_index,
                                paint_owner: target,
                                order: 0,
                                content: InlineItemContent::InlineBox { fragment },
                                selection: Vec::new(),
                                paint_bounds: InlinePaintBounds::Empty,
                            });
                        }
                    }
                    covered.clear();
                    for (item_index, positioned) in line.items().enumerate() {
                        match positioned {
                            PositionedLayoutItem::InlineBox(inline) => {
                                if let Some(object) = context.object(inline.id)
                                    && object.role == InlineObjectRole::Atomic
                                {
                                    result.items.push(InlineDisplayItem {
                                        line: line_index,
                                        paint_owner: object.box_id,
                                        order: item_index + 1,
                                        content: InlineItemContent::Atomic(object.box_id),
                                        selection: Vec::new(),
                                        paint_bounds: InlinePaintBounds::Empty,
                                    });
                                }
                            }
                            PositionedLayoutItem::GlyphRun(glyphs) => {
                                let range = glyphs.text_range();
                                covered.push(range.clone());
                                let target = source_owner(context, owner, &range);
                                if !glyphs.style().brush.paint
                                    || !paints_self(&self.paints_self, target)
                                    || !world.boxes[target.index()].style.is_visible()
                                {
                                    result.complete_source_geometry = false;
                                    continue;
                                }
                                let mut push_view = |view: GlyphRun<'a, TextBrush>| {
                                    let offset = context
                                        .line_placements
                                        .get(line_index)
                                        .map_or(0.0, |line| line.item_offset(item_index));
                                    let selection =
                                        selected_rects(context, line_index, view.text_range());
                                    result.items.push(InlineDisplayItem {
                                        line: line_index,
                                        paint_owner: target,
                                        order: item_index + 1,
                                        content: InlineItemContent::Text(TextView {
                                            glyphs: view,
                                            offset: LayoutPoint::new(0.0, offset),
                                            origin: TextOrigin::Source,
                                        }),
                                        selection,
                                        paint_bounds: InlinePaintBounds::Empty,
                                    });
                                };
                                if let Some(plan) = plan {
                                    for range in plan.visible_ranges(range) {
                                        push_view(
                                            glyphs.slice_text(range).expect(
                                                "truncation retains complete shaping atoms",
                                            ),
                                        );
                                    }
                                } else {
                                    push_view(glyphs);
                                }
                            }
                        }
                    }
                    // A removed glyph is not a glyphless control. Find controls
                    // from the original streams, then apply line participation.
                    if plan.is_none() {
                        covered.sort_unstable_by_key(|range| (range.start, range.end));
                        let line_range = line.text_range();
                        let mut offset = line_range.start;
                        for range in &covered {
                            if offset < range.start {
                                add_controls(
                                    &mut result.items,
                                    world,
                                    &self.paints_self,
                                    context,
                                    owner,
                                    line_index,
                                    offset..range.start,
                                );
                            }
                            offset = offset.max(range.end);
                        }
                        if offset < line_range.end {
                            add_controls(
                                &mut result.items,
                                world,
                                &self.paints_self,
                                context,
                                owner,
                                line_index,
                                offset..line_range.end,
                            );
                        }
                    }
                    if let Some(plan) = plan
                        && paints_self(&self.paints_self, owner)
                        && let Some(marker) = &pool.0[index]
                    {
                        for marker_line in marker.layout.lines() {
                            for positioned in marker_line.items() {
                                let PositionedLayoutItem::GlyphRun(glyphs) = positioned else {
                                    continue;
                                };
                                if !glyphs.style().brush.paint {
                                    continue;
                                }
                                result.items.push(InlineDisplayItem {
                                    line: line_index,
                                    paint_owner: owner,
                                    order: usize::MAX,
                                    content: InlineItemContent::Text(TextView {
                                        glyphs,
                                        offset: LayoutPoint::new(
                                            plan.marker_offset(marker.width),
                                            context.line_placements[line_index].baseline
                                                - marker_line.metrics().baseline,
                                        ),
                                        origin: TextOrigin::Generated {
                                            text: &marker.text,
                                            anchor: TextAnchor {
                                                target: plan.target,
                                                utf16_offset: plan.source_offset,
                                                rtl: plan.rtl,
                                            },
                                        },
                                    }),
                                    selection: Vec::new(),
                                    paint_bounds: InlinePaintBounds::Empty,
                                });
                            }
                        }
                    }
                    let end = result.items.len();
                    let mut bounds = InlinePaintBounds::Empty;
                    let mut selected = Vec::new();
                    for item in &mut result.items[start..end] {
                        if let InlineItemContent::Text(view) = &mut item.content {
                            view.offset.x += origin.x;
                            view.offset.y += origin.y;
                        }
                        for rect in &mut item.selection {
                            *rect = crate::overflow::offset_rect(*rect, origin);
                        }
                        let mut item_bounds = InlinePaintBounds::Empty;
                        match &item.content {
                            InlineItemContent::Text(view) => {
                                let metrics = view.glyphs.run().font_metrics();
                                item_bounds.include_text_ink(
                                    LayoutRect::new(
                                        view.offset.x + view.glyphs.offset(),
                                        view.offset.y + view.glyphs.baseline() - metrics.ascent,
                                        view.glyphs.advance().max(0.0),
                                        metrics.ascent + metrics.descent,
                                    ),
                                    view.glyphs.style(),
                                    view.glyphs.run().font_size(),
                                );
                                if has_selection && matches!(view.origin, TextOrigin::Source) {
                                    selected.push(view.glyphs.text_range());
                                }
                            }
                            InlineItemContent::Control(range) if has_selection => {
                                selected.push(range.clone())
                            }
                            _ => {}
                        }
                        for rect in &item.selection {
                            item_bounds.include(*rect);
                        }
                        item.paint_bounds = item_bounds;
                        match item_bounds {
                            InlinePaintBounds::Empty => {}
                            InlinePaintBounds::Bounded(rect) => bounds.include(rect),
                            InlinePaintBounds::Unbounded => bounds = InlinePaintBounds::Unbounded,
                        }
                    }
                    // Bulk selection retains merged geometry, while fragment events
                    // use each item's geometry at its own place in paint order.
                    merge_ranges(&mut selected);
                    let selection = selected
                        .into_iter()
                        .flat_map(|range| selected_rects(context, line_index, range))
                        .map(|rect| crate::overflow::offset_rect(rect, origin))
                        .collect::<Vec<_>>();
                    for rect in &selection {
                        bounds.include(*rect);
                    }
                    result.lines.push(DisplayLine {
                        items: start..end,
                        paint_bounds: bounds,
                        selection,
                    });
                }
                if let Some((rect, _)) = &mut result.caret {
                    *rect = crate::overflow::offset_rect(*rect, origin);
                }
                Some(Box::new(result))
            })
            .collect();
        InlineDisplayItems {
            contexts,
            paints_self: self.paints_self,
        }
    }
}

fn source_owner(
    context: &InlineFormattingContext,
    fallback: LayoutBoxId,
    range: &Range<usize>,
) -> LayoutBoxId {
    let first = context
        .source_map
        .partition_point(|source| source.output_range.end <= range.start);
    context
        .source_map
        .get(first)
        .filter(|source| source.output_range.start < range.end)
        .map_or(fallback, |source| source.box_id)
}

fn add_controls<N: Copy + Debug + Eq + Hash>(
    items: &mut Vec<InlineDisplayItem<'_>>,
    world: &LayoutWorld<N>,
    box_participation: &[bool],
    context: &InlineFormattingContext,
    owner: LayoutBoxId,
    line: usize,
    range: Range<usize>,
) {
    let first = context
        .source_map
        .partition_point(|source| source.output_range.end <= range.start);
    let mut offset = range.start;
    let mut push = |target: LayoutBoxId, range: Range<usize>| {
        if paints_self(box_participation, target) && world.boxes[target.index()].style.is_visible()
        {
            let selection = selected_rects(context, line, range.clone());
            items.push(InlineDisplayItem {
                line,
                paint_owner: target,
                order: 0,
                content: InlineItemContent::Control(range),
                selection,
                paint_bounds: InlinePaintBounds::Empty,
            });
        }
    };
    for source in context.source_map[first..]
        .iter()
        .take_while(|source| source.output_range.start < range.end)
    {
        let start = source.output_range.start.min(range.end);
        if offset < start {
            push(owner, offset..start);
            offset = start;
        }
        let end = source.output_range.end.min(range.end);
        if offset < end {
            push(source.box_id, offset..end);
            offset = end;
        }
    }
    if offset < range.end {
        push(owner, offset..range.end);
    }
}

fn merge_ranges(ranges: &mut Vec<Range<usize>>) {
    ranges.sort_unstable_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges.drain(..) {
        if let Some(last) = merged.last_mut()
            && range.start <= last.end
        {
            last.end = last.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    *ranges = merged;
}

// Selection geometry and the 1.5 CSS px caret below retain the Blitz d788124a
// editing projection. Finalize them with line placement before paint consumes
// the display items, including controls with no drawable glyphs.
fn selected_rects(
    context: &InlineFormattingContext,
    line: usize,
    range: Range<usize>,
) -> Vec<LayoutRect> {
    let Some(InlineSelection::Range(selected)) = &context.selection else {
        return Vec::new();
    };
    let range = range.start.max(selected.start)..range.end.min(selected.end);
    if range.is_empty() {
        return Vec::new();
    }
    let layout = context.laid_out.as_ref().unwrap();
    let anchor = Cursor::from_byte_index(layout, range.start, Affinity::Downstream);
    let focus = Cursor::from_byte_index(layout, range.end, Affinity::Downstream);
    let mut rects = Vec::new();
    Selection::new(anchor, focus).geometry_with(layout, |rect, index| {
        if index != line {
            return;
        }
        let rect = selection_rect(
            context,
            index,
            rect.x0 as f32,
            rect.y0 as f32,
            (rect.x1 - rect.x0).max(0.0) as f32,
            (rect.y1 - rect.y0).max(0.0) as f32,
        );
        if rect.width > 0.0 && rect.height > 0.0 {
            rects.push(rect);
        }
    });
    rects
}

fn caret(context: &InlineFormattingContext) -> Option<(LayoutRect, PaintColor)> {
    let layout = context.laid_out.as_ref()?;
    let InlineSelection::Caret { offset, color } = context.selection.as_ref()? else {
        return None;
    };
    if color.alpha <= 0.0 {
        return None;
    }
    let cursor = Cursor::from_byte_index(layout, *offset, Affinity::Downstream);
    let rect = cursor.geometry(layout, 1.5);
    let line = layout
        .lines()
        .enumerate()
        .find(|(_, line)| line.text_range().contains(offset))
        .map(|(index, _)| index)
        .unwrap_or_else(|| layout.len().saturating_sub(1));
    let rect = selection_rect(
        context,
        line,
        rect.x0 as f32,
        rect.y0 as f32,
        (rect.x1 - rect.x0).max(1.5) as f32,
        (rect.y1 - rect.y0).max(0.0) as f32,
    );
    (rect.height > 0.0).then_some((rect, *color))
}

fn selection_rect(
    context: &InlineFormattingContext,
    line: usize,
    x: f32,
    fallback_y: f32,
    width: f32,
    fallback_height: f32,
) -> LayoutRect {
    let (y, height) = context
        .line_placements
        .get(line)
        .map_or((fallback_y, fallback_height), |line| {
            (line.rect.y, line.rect.height)
        });
    LayoutRect::new(x, y, width, height)
}
