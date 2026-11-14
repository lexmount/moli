use std::{fmt::Debug, hash::Hash};

use crate::{
    LayoutBoxId, LayoutPosition, LayoutWorld,
    inline::{
        InlineObjectRole,
        display::{InlineDisplayItems, InlineItemContent, TextOrigin},
    },
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
    /// individual InlineItem events at their place in each line.
    BoxText(LayoutBoxId),
    /// A mixed IFC's caret paints after its contents, including glyphless lines.
    BoxCaret(LayoutBoxId),
    InlineBox {
        owner: LayoutBoxId,
        fragment: usize,
    },
    InlineItem {
        owner: LayoutBoxId,
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

/// Fragment events interleave atomics and let independently painted inlines
/// retain their share of a hidden host IFC. Other paragraphs use bulk text.
struct InlineOrder<'a> {
    split: Vec<bool>,
    display: &'a InlineDisplayItems<'a>,
    entries: Vec<Vec<PaintOrderEvent>>,
    atomic_lines: Vec<Option<(LayoutBoxId, usize)>>,
}

impl<'a> InlineOrder<'a> {
    fn new<N>(world: &LayoutWorld<N>, display: &'a InlineDisplayItems<'a>) -> Self
    where
        N: Copy + Debug + Eq + Hash,
    {
        let mut result = Self {
            split: vec![false; world.boxes.len()],
            display,
            entries: vec![Vec::new(); world.boxes.len()],
            atomic_lines: vec![None; world.boxes.len()],
        };
        for (index, layout_box) in world.boxes.iter().enumerate() {
            let Some(context) = &layout_box.inline_layout else {
                continue;
            };
            let owner = LayoutBoxId::from_index(index);
            let Some(items) = display.context(owner) else {
                continue;
            };
            let has_atomic = context
                .objects
                .iter()
                .any(|object| object.role == InlineObjectRole::Atomic);
            let has_independent_inline = !display.paints_self(owner)
                && context
                    .structural_boxes
                    .iter()
                    .any(|inline| display.paints_self(inline.box_id));
            if !has_atomic && !has_independent_inline {
                continue;
            }
            result.split[index] = true;
            for (item, entry) in items.items.iter().enumerate() {
                let event = match entry.content {
                    InlineItemContent::Atomic(id) => {
                        result.atomic_lines[id.index()] = Some((owner, entry.line));
                        continue;
                    }
                    InlineItemContent::InlineBox { fragment } => {
                        PaintOrderEvent::InlineBox { owner, fragment }
                    }
                    _ => PaintOrderEvent::InlineItem { owner, item },
                };
                result.entries[entry.paint_owner.index()].push(event);
            }
        }
        result
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
pub(crate) fn build_paint_order<N>(
    world: &LayoutWorld<N>,
    display: &InlineDisplayItems<'_>,
) -> Vec<PaintOrderEvent>
where
    N: Copy + Debug + Eq + Hash,
{
    let mut sequence = vec![0; world.boxes.len()];
    let mut next = 0;
    assign_tree_order(world, world.root, &mut sequence, &mut next);
    let inline = InlineOrder::new(world, display);
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
    let hidden = !inline.display.paints_self(root);
    if !hidden {
        push_background(root, events);
    }
    let mut flow = FlowCollection::default();
    let mut stacking = StackingDescendants::default();
    if !hidden {
        collect_contents(world, root, sequence, inline, &mut flow);
    }
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
    if !hidden {
        events.push(PaintOrderEvent::BoxOutline(root));
    }
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
    let creates_context = layout_box.creates_stacking_context(false, flex_or_grid_item);
    if !inline.display.paints_self(id) && !creates_context {
        // Do not emit this box's normal-flow paint/hit events, but still find
        // independent descendants collected by the enclosing real context.
        for child in ordered_children(world, id) {
            collect_subtree(world, child, sequence, inline, flow, stacking);
        }
        return;
    }
    // A hidden stacking root still contains independently painted descendants;
    // retain its effects and z-order, omitting only its own paint in emit_context.
    if creates_context {
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
        && inline
            .display
            .context(id)
            .is_some_and(|context| context.caret.is_some())
    {
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
            PaintOrderEvent::InlineItem { owner, item } => {
                let entry = &inline.display.context(owner).unwrap().items[item];
                (owner, entry.line, entry.order)
            }
            _ => unreachable!(),
        };
        let source_order = match *event {
            PaintOrderEvent::InlineItem { owner, item } => {
                let entry = &inline.display.context(owner).unwrap().items[item];
                if matches!(&entry.content, InlineItemContent::Text(view) if matches!(view.origin, TextOrigin::Generated { .. }))
                {
                    usize::MAX
                } else {
                    tree_order
                }
            }
            _ => tree_order,
        };
        flow.inline_contents.push(PaintEntry {
            order: (sequence[owner.index()], line, source_order, item),
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
