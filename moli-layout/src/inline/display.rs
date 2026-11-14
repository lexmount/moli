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

#[derive(Clone)]
pub(crate) enum TextOrigin<'a> {
    Source,
    /// Generated text has a pointer/caret anchor, not invented source
    /// characters. Baseline selection policy leaves this text unselected.
    Generated {
        text: &'a Arc<str>,
        anchor: TextAnchor,
    },
}

#[derive(Clone)]
pub(crate) struct TextView<'a> {
    pub(crate) glyphs: GlyphRun<'a, TextBrush>,
    /// Final placement in the IFC owner's local coordinate space. The shape
    /// retains its original inline offset and baseline, including RTL suffixes.
    pub(crate) offset: LayoutPoint,
    pub(crate) origin: TextOrigin<'a>,
}

#[derive(Clone)]
pub(crate) enum InlineItemContent<'a> {
    Text(TextView<'a>),
    InlineBox {
        fragment: usize,
    },
    Atomic(LayoutBoxId),
    /// Glyphless source content (newlines, empty lines and object gaps).
    Control(Range<usize>),
}

/// Cloning copies only view metadata; all shape data remains borrowed.
#[derive(Clone)]
pub(crate) struct InlineDisplayItem<'a> {
    pub(crate) line: usize,
    /// Scheduling owner; source identity is retained by source_range().
    pub(crate) paint_owner: LayoutBoxId,
    /// Original visual item order. Generated line-end content follows the line.
    pub(crate) order: usize,
    pub(crate) content: InlineItemContent<'a>,
    pub(crate) paint_bounds: InlinePaintBounds,
}

impl InlineDisplayItem<'_> {
    pub(crate) fn source_range(&self) -> Option<Range<usize>> {
        match &self.content {
            InlineItemContent::Text(view) if matches!(view.origin, TextOrigin::Source) => {
                Some(view.glyphs.text_range())
            }
            InlineItemContent::Control(range) => Some(range.clone()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DisplayLineItems {
    /// Positioned text is borrowed; only glyphless source spans are stored.
    Natural(Range<usize>),
    Materialized(Range<usize>),
}

pub(crate) struct DisplayLine {
    items: DisplayLineItems,
    pub(crate) paint_bounds: InlinePaintBounds,
    pub(crate) selection: Vec<LayoutRect>,
}

pub(crate) struct InlineDisplayContext<'a> {
    items: Vec<InlineDisplayItem<'a>>,
    controls: Vec<Range<usize>>,
    /// Query index into existing source geometry, never another box tree.
    inline_boxes_by_line: Vec<usize>,
    pub(crate) lines: Vec<DisplayLine>,
    pub(crate) caret: Option<(LayoutRect, PaintColor)>,
    /// The final source hit geometry can reuse its original batch unchanged.
    /// Partial views instead project membership from the item stream.
    pub(crate) complete_source_geometry: bool,
    /// Mixed IFCs need stable item IDs for interleaved or independent paint.
    pub(crate) split_paint: bool,
    source: &'a InlineFormattingContext,
    origin: LayoutPoint,
    owner: LayoutBoxId,
}

impl<'a> InlineDisplayContext<'a> {
    /// One final-item stream for paint, selection, masks and source hit mapping.
    /// Optional IDs address retained items when fragment scheduling needs them.
    pub(crate) fn display_items(
        &self,
        line: usize,
    ) -> impl Iterator<Item = (Option<usize>, InlineDisplayItem<'a>)> + '_ {
        self.line_items(line, self.lines[line].items.clone())
    }

    pub(crate) fn item(&self, index: usize) -> InlineDisplayItem<'a> {
        self.items[index].clone()
    }

    pub(crate) fn visit_item_selection(
        &self,
        item: &InlineDisplayItem<'_>,
        mut visit: impl FnMut(LayoutRect),
    ) {
        if let Some(range) = item.source_range() {
            for_each_selected_rect(self.source, item.line, range, |rect| {
                visit(crate::overflow::offset_rect(rect, self.origin));
            });
        }
    }

    fn line_items(
        &self,
        line: usize,
        storage: DisplayLineItems,
    ) -> impl Iterator<Item = (Option<usize>, InlineDisplayItem<'a>)> + '_ {
        let (natural, range) = match storage {
            DisplayLineItems::Natural(range) => (true, range),
            DisplayLineItems::Materialized(range) => (false, range),
        };
        let source = self.source;
        let origin = self.origin;
        let owner = self.owner;
        let first = self
            .inline_boxes_by_line
            .partition_point(|fragment| source.fragments.boxes[*fragment].line_index < line);
        let boxes = self.inline_boxes_by_line[first..]
            .iter()
            .take_while(move |fragment| source.fragments.boxes[**fragment].line_index == line)
            .map(move |fragment| {
                (
                    None,
                    InlineDisplayItem {
                        line,
                        paint_owner: source.fragments.boxes[*fragment].box_id,
                        order: 0,
                        content: InlineItemContent::InlineBox {
                            fragment: *fragment,
                        },
                        paint_bounds: InlinePaintBounds::Empty,
                    },
                )
            });
        let text = natural
            .then(|| source.laid_out.as_ref().unwrap().get(line).unwrap())
            .into_iter()
            .flat_map(|line| line.items().enumerate())
            .filter_map(move |(position, positioned)| {
                let PositionedLayoutItem::GlyphRun(glyphs) = positioned else {
                    return None;
                };
                Some((
                    None,
                    resolve_item(
                        source,
                        origin,
                        InlineDisplayItem {
                            line,
                            paint_owner: owner,
                            order: position + 1,
                            content: InlineItemContent::Text(source_view(
                                source, line, position, glyphs, origin,
                            )),
                            paint_bounds: InlinePaintBounds::Empty,
                        },
                    ),
                ))
            });
        let control_range = if natural { range.clone() } else { 0..0 };
        let controls = self.controls[control_range]
            .iter()
            .map(move |control| (None, self.control_item(line, control)));
        let retained_range = if natural { 0..0 } else { range };
        let retained_start = retained_range.start;
        let retained = self.items[retained_range]
            .iter()
            .enumerate()
            .map(move |(index, item)| (Some(retained_start + index), item.clone()));
        boxes.chain(text).chain(controls).chain(retained)
    }

    fn control_item(&self, line: usize, control: &Range<usize>) -> InlineDisplayItem<'a> {
        resolve_item(
            self.source,
            self.origin,
            InlineDisplayItem {
                line,
                paint_owner: self.owner,
                order: 0,
                content: InlineItemContent::Control(control.clone()),
                paint_bounds: InlinePaintBounds::Empty,
            },
        )
    }

    fn line_summary(
        &self,
        line: usize,
        storage: DisplayLineItems,
    ) -> (InlinePaintBounds, usize, bool) {
        let mut bounds = InlinePaintBounds::Empty;
        let mut covered = 0;
        let mut all_paint = true;
        for (_, item) in self.line_items(line, storage) {
            include_bounds(&mut bounds, item.paint_bounds);
            if let InlineItemContent::Text(view) = &item.content
                && matches!(view.origin, TextOrigin::Source)
            {
                covered += view.glyphs.text_range().len();
                all_paint &= view.glyphs.style().brush.paint;
            }
        }
        (bounds, covered, all_paint)
    }

    fn line_selection(&self, line: usize, storage: DisplayLineItems) -> Vec<LayoutRect> {
        if !matches!(self.source.selection, Some(InlineSelection::Range(_))) {
            return Vec::new();
        }
        let mut ranges = self
            .line_items(line, storage)
            .filter_map(|(_, item)| item.source_range())
            .collect::<Vec<_>>();
        merge_ranges(&mut ranges);
        let mut rects = Vec::new();
        for range in ranges {
            for_each_selected_rect(self.source, line, range, |rect| {
                rects.push(crate::overflow::offset_rect(rect, self.origin));
            });
        }
        rects
    }
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
                let owner_participates = paints_self(&self.paints_self, owner);
                let split_paint = context
                    .objects
                    .iter()
                    .any(|object| object.role == InlineObjectRole::Atomic)
                    || (!owner_participates
                        && context
                            .structural_boxes
                            .iter()
                            .any(|inline| paints_self(&self.paints_self, inline.box_id)));
                let source_participates = owner_participates
                    && context.source_map.iter().all(|source| {
                        paints_self(&self.paints_self, source.box_id)
                            && world.boxes[source.box_id.index()].style.is_visible()
                    });
                let natural_allowed = !split_paint && source_participates;
                let mut inline_boxes_by_line = context
                    .fragments
                    .boxes
                    .iter()
                    .enumerate()
                    .filter(|(_, fragment)| paints_self(&self.paints_self, fragment.box_id))
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();
                inline_boxes_by_line.sort_unstable_by_key(|fragment| {
                    (context.fragments.boxes[*fragment].line_index, *fragment)
                });
                let mut result = InlineDisplayContext {
                    items: Vec::with_capacity(if natural_allowed {
                        plans.map_or(0, |lines| {
                            lines.iter().filter(|line| line.is_some()).count() * 2
                        })
                    } else {
                        layout.len() + context.objects.len()
                    }),
                    controls: Vec::new(),
                    inline_boxes_by_line,
                    lines: Vec::with_capacity(layout.len()),
                    caret: caret(context)
                        .filter(|_| owner_participates && layout_box.style.is_visible()),
                    complete_source_geometry: plans.is_none() && source_participates,
                    split_paint,
                    source: context,
                    origin,
                    owner,
                };
                let mut covered = Vec::new();
                for (line_index, line) in layout.lines().enumerate() {
                    let plan = plans.and_then(|lines| lines[line_index].as_ref());
                    if natural_allowed && plan.is_none() {
                        let start = result.controls.len();
                        let (mut bounds, covered_bytes, all_paint) = result
                            .line_summary(line_index, DisplayLineItems::Natural(start..start));
                        let line_range = line.text_range();
                        if all_paint && covered_bytes <= line_range.len() {
                            if covered_bytes < line_range.len() {
                                covered.clear();
                                covered.extend(line.items().filter_map(
                                    |positioned| match positioned {
                                        PositionedLayoutItem::GlyphRun(glyphs) => {
                                            Some(glyphs.text_range())
                                        }
                                        PositionedLayoutItem::InlineBox(_) => None,
                                    },
                                ));
                                for_each_uncovered_range(&mut covered, line_range, |range| {
                                    for_each_control_span(
                                        world,
                                        &self.paints_self,
                                        context,
                                        owner,
                                        range,
                                        |_, range| result.controls.push(range),
                                    );
                                });
                                for control in &result.controls[start..] {
                                    include_bounds(
                                        &mut bounds,
                                        result.control_item(line_index, control).paint_bounds,
                                    );
                                }
                            }
                            let storage = DisplayLineItems::Natural(start..result.controls.len());
                            let selection = result.line_selection(line_index, storage.clone());
                            for rect in &selection {
                                bounds.include(*rect);
                            }
                            result.lines.push(DisplayLine {
                                items: storage,
                                paint_bounds: bounds,
                                selection,
                            });
                            continue;
                        }
                    }
                    let start = result.items.len();
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
                                    result.items.push(resolve_item(
                                        context,
                                        origin,
                                        InlineDisplayItem {
                                            line: line_index,
                                            paint_owner: if split_paint { target } else { owner },
                                            order: item_index + 1,
                                            content: InlineItemContent::Text(source_view(
                                                context, line_index, item_index, view, origin,
                                            )),
                                            paint_bounds: InlinePaintBounds::Empty,
                                        },
                                    ));
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
                    // Controls come from gaps in the original shape, never omitted glyphs.
                    if plan.is_none() {
                        for_each_uncovered_range(&mut covered, line.text_range(), |range| {
                            for_each_control_span(
                                world,
                                &self.paints_self,
                                context,
                                owner,
                                range,
                                |target, range| {
                                    result.items.push(resolve_item(
                                        context,
                                        origin,
                                        InlineDisplayItem {
                                            line: line_index,
                                            paint_owner: if split_paint { target } else { owner },
                                            order: 0,
                                            content: InlineItemContent::Control(range),
                                            paint_bounds: InlinePaintBounds::Empty,
                                        },
                                    ))
                                },
                            );
                        });
                    }
                    if let Some(plan) = plan
                        && owner_participates
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
                                result.items.push(resolve_item(
                                    context,
                                    origin,
                                    InlineDisplayItem {
                                        line: line_index,
                                        paint_owner: owner,
                                        order: usize::MAX,
                                        content: InlineItemContent::Text(TextView {
                                            glyphs,
                                            offset: LayoutPoint::new(
                                                origin.x + plan.marker_offset(marker.width),
                                                origin.y
                                                    + (context.line_placements[line_index]
                                                        .baseline
                                                        - marker_line.metrics().baseline),
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
                                        paint_bounds: InlinePaintBounds::Empty,
                                    },
                                ));
                            }
                        }
                    }
                    let storage = DisplayLineItems::Materialized(start..result.items.len());
                    let (mut bounds, _, _) = result.line_summary(line_index, storage.clone());
                    let selection = result.line_selection(line_index, storage.clone());
                    for rect in &selection {
                        bounds.include(*rect);
                    }
                    result.lines.push(DisplayLine {
                        items: storage,
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

fn source_view<'a>(
    context: &InlineFormattingContext,
    line: usize,
    item: usize,
    glyphs: GlyphRun<'a, TextBrush>,
    origin: LayoutPoint,
) -> TextView<'a> {
    let offset = context
        .line_placements
        .get(line)
        .map_or(0.0, |line| line.item_offset(item));
    TextView {
        glyphs,
        offset: LayoutPoint::new(origin.x, origin.y + offset),
        origin: TextOrigin::Source,
    }
}

fn text_view_bounds(view: &TextView<'_>) -> InlinePaintBounds {
    let metrics = view.glyphs.run().font_metrics();
    let mut bounds = InlinePaintBounds::Empty;
    bounds.include_text_ink(
        LayoutRect::new(
            view.offset.x + view.glyphs.offset(),
            view.offset.y + view.glyphs.baseline() - metrics.ascent,
            view.glyphs.advance().max(0.0),
            metrics.ascent + metrics.descent,
        ),
        view.glyphs.style(),
        view.glyphs.run().font_size(),
    );
    bounds
}

fn include_bounds(bounds: &mut InlinePaintBounds, item: InlinePaintBounds) {
    match item {
        InlinePaintBounds::Empty => {}
        InlinePaintBounds::Bounded(rect) => bounds.include(rect),
        InlinePaintBounds::Unbounded => *bounds = InlinePaintBounds::Unbounded,
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

fn for_each_control_span<N: Copy + Debug + Eq + Hash>(
    world: &LayoutWorld<N>,
    box_participation: &[bool],
    context: &InlineFormattingContext,
    owner: LayoutBoxId,
    range: Range<usize>,
    mut visit: impl FnMut(LayoutBoxId, Range<usize>),
) {
    let first = context
        .source_map
        .partition_point(|source| source.output_range.end <= range.start);
    let mut offset = range.start;
    let mut push = |target: LayoutBoxId, range: Range<usize>| {
        if paints_self(box_participation, target) && world.boxes[target.index()].style.is_visible()
        {
            visit(target, range);
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

fn for_each_uncovered_range(
    covered: &mut [Range<usize>],
    range: Range<usize>,
    mut visit: impl FnMut(Range<usize>),
) {
    covered.sort_unstable_by_key(|range| (range.start, range.end));
    let mut offset = range.start;
    for covered in covered {
        if offset < covered.start {
            visit(offset..covered.start);
        }
        offset = offset.max(covered.end);
    }
    if offset < range.end {
        visit(offset..range.end);
    }
}

fn resolve_item<'a>(
    context: &InlineFormattingContext,
    origin: LayoutPoint,
    mut item: InlineDisplayItem<'a>,
) -> InlineDisplayItem<'a> {
    if let InlineItemContent::Text(view) = &item.content {
        item.paint_bounds = text_view_bounds(view);
    }
    if let Some(range) = item.source_range() {
        for_each_selected_rect(context, item.line, range, |rect| {
            item.paint_bounds
                .include(crate::overflow::offset_rect(rect, origin));
        });
    }
    item
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
fn for_each_selected_rect(
    context: &InlineFormattingContext,
    line: usize,
    range: Range<usize>,
    mut visit: impl FnMut(LayoutRect),
) {
    let Some(InlineSelection::Range(selected)) = &context.selection else {
        return;
    };
    let range = range.start.max(selected.start)..range.end.min(selected.end);
    if range.is_empty() {
        return;
    }
    let layout = context.laid_out.as_ref().unwrap();
    let anchor = Cursor::from_byte_index(layout, range.start, Affinity::Downstream);
    let focus = Cursor::from_byte_index(layout, range.end, Affinity::Downstream);
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
            visit(rect);
        }
    });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        LayoutBoxKind, LayoutDisplay, ResolvedLayoutStyle, SystemFontPolicy, WebFontFace,
        WebFontRegistration,
        inline::{InlineContentWidthsMemo, InlineFragments, build_inline_fragments},
    };
    use parley::style::TextStyle;

    #[test]
    fn natural_lines_borrow_styled_runs_without_allocating_items() {
        for (text, font, direction) in [
            (
                "ABCD EFGHIJKLMNOPQRST ABCDEFGH",
                include_bytes!("../../tests/fixtures/moli-ahem.ttf").as_slice(),
                parley::BaseDirection::Ltr,
            ),
            (
                "אבבאבאבאבא באאבבאאבבא",
                include_bytes!("../../tests/fixtures/moli-hebrew-emoji.ttf").as_slice(),
                parley::BaseDirection::Rtl,
            ),
            (
                "office ffi a\u{301}\n\nABCD EFGH",
                include_bytes!("../../../moli-paint/tests/fixtures/moli-pdf.ttf").as_slice(),
                parley::BaseDirection::Ltr,
            ),
        ] {
            let mut services =
                DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
            services
                .register_web_font(WebFontRegistration::new(
                    "natural",
                    WebFontFace::new("NaturalTest"),
                    font.to_vec(),
                ))
                .unwrap();
            let parley = services.parley_mut();
            let mut style = TextStyle {
                font_family: "NaturalTest".into(),
                ..TextStyle::default()
            };
            parley.resolve_font_families(&mut style, None);
            let mut larger = style.clone();
            larger.font_size = style.font_size * 1.25;
            let mut builder =
                parley
                    .layout_context
                    .style_run_builder(&mut parley.font_context, text, 1.0, true);
            builder.set_base_direction(direction);
            let first = builder.push_style(style);
            let second = builder.push_style(larger);
            let split = text.char_indices().nth(4).unwrap().0;
            builder.push_style_run(first, 0..split);
            builder.push_style_run(second, split..text.len());
            let mut layout = builder.build(text);
            layout.break_all_lines(Some(80.0));
            assert!(layout.len() > 1);
            let root = LayoutBoxId::from_index(0);
            let mut context = InlineFormattingContext {
                text: text.into(),
                root_style: root,
                measurement_layout: None,
                laid_out: Some(layout),
                content_widths: InlineContentWidthsMemo::default(),
                no_soft_wrap: false,
                text_units: Vec::new(),
                source_map: Vec::new(),
                selection: Some(InlineSelection::Range(split..text.len())),
                objects: Vec::new(),
                font_metrics: vec![None; 2],
                parent_strut: None,
                uses_quirks_line_height: false,
                root_includes_used_font_metrics: false,
                style_parents: vec![root; 2],
                structural_boxes: Vec::new(),
                line_placements: Vec::new(),
                fragments: InlineFragments::default(),
            };
            context.fragments =
                build_inline_fragments(&context, context.laid_out.as_ref().unwrap(), &[]);
            let mut root_box = LayoutWorld::<usize>::new_box(
                Some(0),
                None,
                None,
                "root".into(),
                None,
                None,
                None,
                LayoutBoxKind::PrincipalBlock,
                ResolvedLayoutStyle::synthetic(
                    LayoutDisplay::Block,
                    taffy::Style::default(),
                    PaintColor::TRANSPARENT,
                ),
                None,
            );
            root_box.inline_layout = Some(context);
            let world = LayoutWorld::new(root_box, false);
            let pool = GeneratedTextPool(Vec::new());
            let origin = LayoutPoint::new(7.0, 11.0);
            let mut display = DisplayBuilder {
                paragraphs: Vec::new(),
                paints_self: Vec::new(),
            }
            .finish(&world, &pool, |_| origin);
            let display = display.contexts[root.index()].as_deref_mut().unwrap();
            assert!(display.items.is_empty());
            assert_eq!(display.items.capacity(), 0);
            assert!(
                display
                    .lines
                    .iter()
                    .all(|line| matches!(line.items, DisplayLineItems::Natural(_)))
            );
            let layout = world.boxes[0]
                .inline_layout
                .as_ref()
                .unwrap()
                .laid_out
                .as_ref()
                .unwrap();
            for (index, line) in layout.lines().enumerate() {
                let mut expected = Vec::new();
                for item in line.items() {
                    if let PositionedLayoutItem::GlyphRun(glyphs) = item {
                        expected.extend(glyphs.positioned_glyphs().map(|glyph| {
                            (
                                glyph.id,
                                (glyph.x + origin.x).to_bits(),
                                (glyph.y + origin.y).to_bits(),
                            )
                        }));
                    }
                }
                let mut actual = Vec::new();
                for (_, item) in display.display_items(index) {
                    if let InlineItemContent::Text(view) = &item.content {
                        actual.extend(view.glyphs.positioned_glyphs().map(|glyph| {
                            (
                                glyph.id,
                                (glyph.x + view.offset.x).to_bits(),
                                (glyph.y + view.offset.y).to_bits(),
                            )
                        }));
                    }
                }
                assert_eq!(actual, expected, "{text}: line {index}");
                let mut ranges = display
                    .display_items(index)
                    .filter_map(|(_, item)| item.source_range())
                    .collect::<Vec<_>>();
                ranges.sort_unstable_by_key(|range| (range.start, range.end));
                let line_range = line.text_range();
                let mut next = line_range.start;
                for range in ranges {
                    assert_eq!(range.start, next, "source coverage: {text}: line {index}");
                    next = range.end;
                }
                assert_eq!(
                    next, line_range.end,
                    "glyphless coverage: {text}: line {index}"
                );
            }

            let signature = |context: &InlineDisplayContext<'_>| {
                (0..context.lines.len())
                    .map(|line| {
                        context
                            .display_items(line)
                            .map(|(_, item)| {
                                let mut selection = Vec::new();
                                context.visit_item_selection(&item, |rect| selection.push(rect));
                                let glyphs = match &item.content {
                                    InlineItemContent::Text(view) => view
                                        .glyphs
                                        .positioned_glyphs()
                                        .map(|glyph| {
                                            (
                                                glyph.id,
                                                (glyph.x + view.offset.x).to_bits(),
                                                (glyph.y + view.offset.y).to_bits(),
                                            )
                                        })
                                        .collect::<Vec<_>>(),
                                    _ => Vec::new(),
                                };
                                (
                                    item.paint_owner,
                                    item.order,
                                    item.source_range(),
                                    glyphs,
                                    selection,
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            };
            let borrowed = signature(display);
            let mut retained = Vec::new();
            let mut lines = Vec::new();
            for line in 0..display.lines.len() {
                let start = retained.len();
                retained.extend(
                    display
                        .display_items(line)
                        .map(|(_, item)| item)
                        .filter(|item| {
                            !matches!(item.content, InlineItemContent::InlineBox { .. })
                        }),
                );
                lines.push(DisplayLineItems::Materialized(start..retained.len()));
            }
            display.items = retained;
            display.controls.clear();
            for (line, items) in display.lines.iter_mut().zip(lines) {
                line.items = items;
            }
            assert_eq!(
                signature(display),
                borrowed,
                "storage changes must preserve glyphs, source identity and selection: {text}"
            );
        }
    }
}
