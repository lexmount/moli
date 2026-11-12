use std::{fmt::Debug, hash::Hash, ops::Range};

use parley::PositionedLayoutItem;

use crate::{
    LayoutBoxId, LayoutPosition, LayoutWorld,
    inline::{InlineFormattingContext, InlineObjectRole, InlineSelection, glyph_cluster_ranges},
};

/// One order shared by pixels and hit geometry. Inline events select individual
/// fragments/runs so an atomic child can be interleaved with its IFC's contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaintOrderEvent {
    BoxOutsetShadow(LayoutBoxId),
    PushStackingContext(LayoutBoxId),
    BoxBackground(LayoutBoxId),
    TableCollapsedBorders(LayoutBoxId),
    BoxContents(LayoutBoxId),
    InlineContents(LayoutBoxId),
    /// Bulk text for an IFC without atomic items. Mixed IFCs instead emit
    /// individual InlineText events at their place in each line.
    BoxText(LayoutBoxId),
    /// A mixed IFC's caret paints after its contents, including glyphless lines.
    BoxCaret(LayoutBoxId),
    /// Selected text not covered by a drawable glyph run, placed in source order.
    InlineSelection {
        owner: LayoutBoxId,
        line: usize,
        start: usize,
        end: usize,
    },
    InlineBox {
        owner: LayoutBoxId,
        fragment: usize,
    },
    InlineText {
        owner: LayoutBoxId,
        line: usize,
        item: usize,
    },
    BoxOutline(LayoutBoxId),
    PopStackingContext(LayoutBoxId),
}

#[derive(Clone, Copy, Debug)]
struct ChildContext {
    id: LayoutBoxId,
    z_index: i32,
    sequence: usize,
}

#[derive(Debug)]
enum PaintEntryKind {
    Event(PaintOrderEvent),
    Group {
        root: LayoutBoxId,
        flow: Box<FlowCollection>,
    },
    Context(LayoutBoxId),
}

#[derive(Debug)]
struct PaintEntry {
    // IFCs paint line by line, in logical tree order within each line. Other
    // entries use their own tree position; a pseudo-context stays one entry.
    order: (usize, usize, usize, usize),
    kind: PaintEntryKind,
}

impl PaintEntry {
    fn event(event: PaintOrderEvent, sequence: usize) -> Self {
        Self {
            order: (sequence, 0, sequence, 0),
            kind: PaintEntryKind::Event(event),
        }
    }
}

#[derive(Debug, Default)]
struct FlowCollection {
    block_backgrounds: Vec<PaintEntry>,
    table_collapsed_borders: Vec<PaintEntry>,
    floats: Vec<PaintEntry>,
    inline_contents: Vec<PaintEntry>,
    outlines: Vec<PaintEntry>,
}

#[derive(Default)]
struct StackingDescendants {
    negative: Vec<ChildContext>,
    positioned: Vec<PaintEntry>,
    positive: Vec<ChildContext>,
}

/// Only IFCs containing atomic objects need fragment-level interleaving. Keep
/// the existing single-pass text projection for all other paragraphs.
struct InlineOrder {
    split: Vec<bool>,
    entries: Vec<Vec<PaintOrderEvent>>,
    atomic_lines: Vec<Option<(LayoutBoxId, usize)>>,
}

impl InlineOrder {
    fn new<N>(world: &LayoutWorld<N>) -> Self
    where
        N: Copy + Debug + Eq + Hash,
    {
        let mut result = Self {
            split: vec![false; world.boxes.len()],
            entries: vec![Vec::new(); world.boxes.len()],
            atomic_lines: vec![None; world.boxes.len()],
        };
        for (owner_index, layout_box) in world.boxes.iter().enumerate() {
            let Some(context) = &layout_box.inline_layout else {
                continue;
            };
            if !context
                .objects
                .iter()
                .any(|object| object.role == InlineObjectRole::Atomic)
            {
                continue;
            }
            let owner = LayoutBoxId::from_index(owner_index);
            result.split[owner_index] = true;
            for (fragment, inline) in context.fragments.boxes.iter().enumerate() {
                result.entries[inline.box_id.index()]
                    .push(PaintOrderEvent::InlineBox { owner, fragment });
            }
            let Some(layout) = &context.laid_out else {
                continue;
            };
            for (line_index, line) in layout.lines().enumerate() {
                let selected = match &context.selection {
                    Some(InlineSelection::Range(range)) => {
                        let line_range = line.text_range();
                        let selected =
                            range.start.max(line_range.start)..range.end.min(line_range.end);
                        (!selected.is_empty()).then_some(selected)
                    }
                    _ => None,
                };
                let mut covered = Vec::new();
                let mut mapped_run = None;
                let mut cluster_ranges = Vec::new().into_iter();
                for (_, inline) in line.inline_box_indices() {
                    if let Some(object) = context.object(inline.id)
                        && object.role == InlineObjectRole::Atomic
                    {
                        result.atomic_lines[object.box_id.index()] = Some((owner, line_index));
                    }
                }
                for (item, positioned) in line.items().enumerate() {
                    let PositionedLayoutItem::GlyphRun(glyphs) = positioned else {
                        continue;
                    };
                    if selected.is_some() {
                        if mapped_run != Some(glyphs.run().index()) {
                            cluster_ranges = glyph_cluster_ranges(glyphs.run()).into_iter();
                            mapped_run = Some(glyphs.run().index());
                        }
                        if let Some(range) = cluster_ranges
                            .by_ref()
                            .take(glyphs.glyphs().count())
                            .reduce(|range, cluster| {
                                range.start.min(cluster.start)..range.end.max(cluster.end)
                            })
                        {
                            covered.push(range);
                        }
                    }
                    let range = glyphs.run().text_range();
                    let source_index = context
                        .source_map
                        .partition_point(|source| source.output_range.end <= range.start);
                    let target = context
                        .source_map
                        .get(source_index)
                        .filter(|source| source.output_range.start < range.end)
                        .map_or(owner, |source| source.box_id);
                    result.entries[target.index()].push(PaintOrderEvent::InlineText {
                        owner,
                        line: line_index,
                        item,
                    });
                }
                if let Some(selected) = selected {
                    covered.sort_unstable_by_key(|range| (range.start, range.end));
                    let mut offset = selected.start;
                    for range in covered {
                        let end = range.start.min(selected.end);
                        if offset < end {
                            result.add_selection_gap(
                                world,
                                context,
                                owner,
                                line_index,
                                offset..end,
                            );
                        }
                        offset = offset.max(range.end).min(selected.end);
                        if offset == selected.end {
                            break;
                        }
                    }
                    if offset < selected.end {
                        result.add_selection_gap(
                            world,
                            context,
                            owner,
                            line_index,
                            offset..selected.end,
                        );
                    }
                }
            }
        }
        result
    }

    fn add_selection_gap<N>(
        &mut self,
        world: &LayoutWorld<N>,
        context: &InlineFormattingContext,
        owner: LayoutBoxId,
        line: usize,
        range: Range<usize>,
    ) where
        N: Copy + Debug + Eq + Hash,
    {
        // Split at source boundaries so a later inline or atomic sibling can
        // cover the highlight, just as it covers the source text's glyphs.
        let first = context
            .source_map
            .partition_point(|source| source.output_range.end <= range.start);
        let mut offset = range.start;
        for source in context.source_map[first..]
            .iter()
            .take_while(|source| source.output_range.start < range.end)
        {
            let start = source.output_range.start.min(range.end);
            if offset < start {
                self.push_selection_range(world, owner, owner, line, offset..start);
                offset = start;
            }
            let end = source.output_range.end.min(range.end);
            if offset < end {
                self.push_selection_range(world, owner, source.box_id, line, offset..end);
                offset = end;
            }
        }
        if offset < range.end {
            self.push_selection_range(world, owner, owner, line, offset..range.end);
        }
    }

    fn push_selection_range<N>(
        &mut self,
        world: &LayoutWorld<N>,
        owner: LayoutBoxId,
        target: LayoutBoxId,
        line: usize,
        range: Range<usize>,
    ) where
        N: Copy + Debug + Eq + Hash,
    {
        if !world.boxes[target.index()].style.is_visible() {
            return;
        }
        let entries = &mut self.entries[target.index()];
        if let Some(PaintOrderEvent::InlineSelection {
            owner: previous_owner,
            line: previous_line,
            end,
            ..
        }) = entries.last_mut()
            && *previous_owner == owner
            && *previous_line == line
            && *end == range.start
        {
            *end = range.end;
            return;
        }
        entries.push(PaintOrderEvent::InlineSelection {
            owner,
            line,
            start: range.start,
            end: range.end,
        });
    }

    fn order(&self, id: LayoutBoxId, sequence: &[usize]) -> (usize, usize, usize, usize) {
        self.atomic_lines[id.index()].map_or(
            (sequence[id.index()], 0, sequence[id.index()], 0),
            |(owner, line)| (sequence[owner.index()], line, sequence[id.index()], 0),
        )
    }
}

/// CSS atomic pseudo-contexts have their own block/float/inline stages, while
/// positioned descendants and real stacking contexts escape to the enclosing
/// real stacking context. They must not inherit one flattened paint level.
pub(crate) fn build_paint_order<N>(world: &LayoutWorld<N>) -> Vec<PaintOrderEvent>
where
    N: Copy + Debug + Eq + Hash,
{
    let mut sequence = vec![0; world.boxes.len()];
    let mut next = 0;
    assign_tree_order(world, world.root, &mut sequence, &mut next);
    let inline = InlineOrder::new(world);
    let mut events = Vec::with_capacity(world.boxes.len() * 4);
    emit_context(world, world.root, &sequence, &inline, &mut events);
    events
}

fn assign_tree_order<N>(
    world: &LayoutWorld<N>,
    id: LayoutBoxId,
    sequence: &mut [usize],
    next: &mut usize,
) where
    N: Copy + Debug + Eq + Hash,
{
    sequence[id.index()] = *next;
    *next += 1;
    for child in ordered_children(world, id) {
        assign_tree_order(world, child, sequence, next);
    }
}

fn emit_context<N>(
    world: &LayoutWorld<N>,
    root: LayoutBoxId,
    sequence: &[usize],
    inline: &InlineOrder,
    events: &mut Vec<PaintOrderEvent>,
) where
    N: Copy + Debug + Eq + Hash,
{
    events.push(PaintOrderEvent::PushStackingContext(root));
    push_background(root, events);
    let mut flow = FlowCollection::default();
    let mut stacking = StackingDescendants::default();
    collect_contents(world, root, sequence, inline, &mut flow);
    for child in ordered_children(world, root) {
        collect_subtree(world, child, sequence, inline, &mut flow, &mut stacking);
    }
    stacking
        .negative
        .sort_by_key(|context| (context.z_index, context.sequence));
    stacking
        .positive
        .sort_by_key(|context| (context.z_index, context.sequence));
    for context in stacking.negative {
        emit_context(world, context.id, sequence, inline, events);
    }
    let outlines = std::mem::take(&mut flow.outlines);
    emit_flow(world, flow, sequence, inline, events);
    emit_entries(world, stacking.positioned, sequence, inline, events);
    for context in stacking.positive {
        emit_context(world, context.id, sequence, inline, events);
    }
    emit_entries(world, outlines, sequence, inline, events);
    events.push(PaintOrderEvent::BoxOutline(root));
    events.push(PaintOrderEvent::PopStackingContext(root));
}

fn collect_subtree<N>(
    world: &LayoutWorld<N>,
    id: LayoutBoxId,
    sequence: &[usize],
    inline: &InlineOrder,
    flow: &mut FlowCollection,
    stacking: &mut StackingDescendants,
) where
    N: Copy + Debug + Eq + Hash,
{
    let layout_box = &world.boxes[id.index()];
    let flex_or_grid_item = is_flex_or_grid_item(world, id);
    if layout_box.creates_stacking_context(false, flex_or_grid_item) {
        let context = ChildContext {
            id,
            z_index: layout_box.style.explicit_z_index().unwrap_or(0),
            sequence: sequence[id.index()],
        };
        match context.z_index.cmp(&0) {
            std::cmp::Ordering::Less => stacking.negative.push(context),
            std::cmp::Ordering::Equal => stacking.positioned.push(PaintEntry {
                order: (context.sequence, 0, context.sequence, 0),
                kind: PaintEntryKind::Context(id),
            }),
            std::cmp::Ordering::Greater => stacking.positive.push(context),
        }
        return;
    }
    let positioned = layout_box.style.position() != LayoutPosition::Static;
    let floated = layout_box.style.is_floated() && !flex_or_grid_item;
    let atomic = flex_or_grid_item
        || (layout_box.inline_context_owner.is_some() && !layout_box.inline_flattened);
    if positioned || floated || atomic {
        let mut local = FlowCollection::default();
        collect_contents(world, id, sequence, inline, &mut local);
        for child in ordered_children(world, id) {
            collect_subtree(world, child, sequence, inline, &mut local, stacking);
        }
        let entry = PaintEntry {
            order: if positioned || floated {
                (sequence[id.index()], 0, sequence[id.index()], 0)
            } else {
                inline.order(id, sequence)
            },
            kind: PaintEntryKind::Group {
                root: id,
                flow: Box::new(local),
            },
        };
        if positioned {
            stacking.positioned.push(entry);
        } else if floated {
            flow.floats.push(entry);
        } else {
            flow.inline_contents.push(entry);
        }
        return;
    }
    if !layout_box.inline_flattened {
        flow.block_backgrounds.push(PaintEntry::event(
            PaintOrderEvent::BoxOutsetShadow(id),
            sequence[id.index()],
        ));
        flow.block_backgrounds.push(PaintEntry::event(
            PaintOrderEvent::BoxBackground(id),
            sequence[id.index()],
        ));
    }
    collect_contents(world, id, sequence, inline, flow);
    for child in ordered_children(world, id) {
        collect_subtree(world, child, sequence, inline, flow, stacking);
    }
    flow.outlines.push(PaintEntry::event(
        PaintOrderEvent::BoxOutline(id),
        sequence[id.index()],
    ));
}

fn collect_contents<N>(
    world: &LayoutWorld<N>,
    id: LayoutBoxId,
    sequence: &[usize],
    inline: &InlineOrder,
    flow: &mut FlowCollection,
) where
    N: Copy + Debug + Eq + Hash,
{
    let layout_box = &world.boxes[id.index()];
    let tree_order = sequence[id.index()];
    if !layout_box.inline_flattened {
        flow.inline_contents.push(PaintEntry::event(
            PaintOrderEvent::BoxContents(id),
            tree_order,
        ));
    }
    if layout_box.inline_layout.is_some() && !inline.split[id.index()] {
        let mut entry = PaintEntry::event(PaintOrderEvent::InlineContents(id), tree_order);
        entry.order.3 = 1;
        flow.inline_contents.push(entry);
        let mut text = PaintEntry::event(PaintOrderEvent::BoxText(id), tree_order);
        text.order.3 = 2;
        flow.inline_contents.push(text);
    }
    if inline.split[id.index()]
        && layout_box
            .inline_layout
            .as_ref()
            .is_some_and(|context| matches!(context.selection, Some(InlineSelection::Caret { .. })))
    {
        // Like Blink's bar caret foreground pass, paint once after the IFC's
        // in-flow contents. A caret does not require a drawable glyph run.
        flow.inline_contents.push(PaintEntry {
            order: (tree_order, usize::MAX, usize::MAX, 0),
            kind: PaintEntryKind::Event(PaintOrderEvent::BoxCaret(id)),
        });
    }
    for event in &inline.entries[id.index()] {
        let (owner, line, item) = match *event {
            PaintOrderEvent::InlineBox { owner, fragment } => (
                owner,
                world.boxes[owner.index()]
                    .inline_layout
                    .as_ref()
                    .unwrap()
                    .fragments
                    .boxes[fragment]
                    .line_index,
                0,
            ),
            PaintOrderEvent::InlineText { owner, line, item } => (owner, line, item + 1),
            // Backgrounds precede the source's glyphs; sibling tree order is
            // still shared with inline fragments and atomic groups.
            PaintOrderEvent::InlineSelection { owner, line, .. } => (owner, line, 0),
            _ => unreachable!(),
        };
        flow.inline_contents.push(PaintEntry {
            order: (sequence[owner.index()], line, tree_order, item),
            kind: PaintEntryKind::Event(*event),
        });
    }
    if world.collapsed_table_borders.contains_key(&id) {
        flow.table_collapsed_borders.push(PaintEntry::event(
            PaintOrderEvent::TableCollapsedBorders(id),
            tree_order,
        ));
    }
}

fn push_background(id: LayoutBoxId, events: &mut Vec<PaintOrderEvent>) {
    events.push(PaintOrderEvent::BoxOutsetShadow(id));
    events.push(PaintOrderEvent::BoxBackground(id));
}

fn emit_flow<N>(
    world: &LayoutWorld<N>,
    flow: FlowCollection,
    sequence: &[usize],
    inline: &InlineOrder,
    events: &mut Vec<PaintOrderEvent>,
) where
    N: Copy + Debug + Eq + Hash,
{
    for entries in [
        flow.block_backgrounds,
        flow.table_collapsed_borders,
        flow.floats,
        flow.inline_contents,
        flow.outlines,
    ] {
        emit_entries(world, entries, sequence, inline, events);
    }
}

fn emit_entries<N>(
    world: &LayoutWorld<N>,
    mut entries: Vec<PaintEntry>,
    sequence: &[usize],
    inline: &InlineOrder,
    events: &mut Vec<PaintOrderEvent>,
) where
    N: Copy + Debug + Eq + Hash,
{
    entries.sort_by_key(|entry| entry.order);
    for entry in entries {
        match entry.kind {
            PaintEntryKind::Event(event) => events.push(event),
            PaintEntryKind::Group { root, flow } => {
                push_background(root, events);
                emit_flow(world, *flow, sequence, inline, events);
                events.push(PaintOrderEvent::BoxOutline(root));
            }
            PaintEntryKind::Context(id) => emit_context(world, id, sequence, inline, events),
        }
    }
}

fn is_flex_or_grid_item<N>(world: &LayoutWorld<N>, id: LayoutBoxId) -> bool
where
    N: Copy + Debug + Eq + Hash,
{
    world.boxes[id.index()].parent.is_some_and(|parent| {
        let display = world.boxes[parent.index()].style.display();
        display.is_flex_container() || display.is_grid_container()
    })
}

fn ordered_children<N>(world: &LayoutWorld<N>, id: LayoutBoxId) -> Vec<LayoutBoxId>
where
    N: Copy + Debug + Eq + Hash,
{
    let layout_box = &world.boxes[id.index()];
    let display = layout_box.style.display();
    if !display.is_flex_container() && !display.is_grid_container() {
        return layout_box.children.clone();
    }
    let mut children = layout_box
        .children
        .iter()
        .copied()
        .enumerate()
        .collect::<Vec<_>>();
    children.sort_by_key(|(document_order, child)| {
        let child = &world.boxes[child.index()];
        let order = if matches!(
            child.style.position(),
            LayoutPosition::Absolute | LayoutPosition::Fixed
        ) {
            0
        } else {
            child.style.order()
        };
        (order, *document_order)
    });
    children.into_iter().map(|(_, child)| child).collect()
}
