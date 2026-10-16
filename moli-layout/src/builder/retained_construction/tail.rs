//! Local terminal block-list edits. Dense IDs of surviving boxes do not move.
//! Shared construction handles new subtrees and rejects unsupported shapes;
//! middle insertion, reordering and nonterminal edits retain the cold fallback.

use std::collections::HashSet;

use super::*;
use crate::LayoutBox;

pub(super) struct TailEdit<N> {
    pub(super) parent: LayoutBoxId,
    pub(super) keep_boxes: usize,
    source_index: usize,
    children: Vec<N>,
    added: Vec<N>,
    removed_sources: HashSet<N>,
}

impl<N> TailEdit<N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub(super) fn removes(&self, node: N) -> bool {
        self.removed_sources.contains(&node)
    }

    pub(super) fn plan(
        previous: &RetainedWork<N>,
        inputs: &ConstructionInputs<N>,
        source_index: usize,
        children: Vec<N>,
    ) -> Option<Self> {
        let input = &inputs.sources[source_index];
        let old_children = input.children.as_ref()?;
        let shared = old_children.len().min(children.len());
        if old_children[..shared] != children[..shared] || old_children.len() == children.len() {
            return None;
        }
        let (boxes, mapping) = previous.owned_construction()?;
        let parent = *mapping.get(&input.node)?;
        if !plain_block(&boxes[parent.index()])
            || boxes[parent.index()]
                .children
                .iter()
                .any(|id| !plain_block(&boxes[id.index()]))
            || !descends_from(boxes, boxes.len() - 1, parent.index())
        {
            return None;
        }
        let added = children[shared..].to_vec();
        let removed = &old_children[shared..];
        let keep_boxes = if let Some(first) = removed.first() {
            let first = *mapping.get(first)?;
            if !boxes[parent.index()].children.contains(&first) {
                return None;
            }
            first.index()
        } else {
            boxes.len()
        };
        let removed_roots = removed
            .iter()
            .map(|node| mapping.get(node).map(|id| id.index()))
            .collect::<Option<HashSet<_>>>()?;
        // Eligible construction parents precede children. Classify every
        // removed box once, including batch removal, instead of searching
        // each removed root's ancestry for every descendant (quadratic).
        let mut removed_boxes = Vec::with_capacity(boxes.len() - keep_boxes);
        for (index, layout_box) in boxes.iter().enumerate().skip(keep_boxes) {
            let belongs = removed_roots.contains(&index)
                || layout_box.parent.is_some_and(|parent| {
                    parent.index() >= keep_boxes && removed_boxes[parent.index() - keep_boxes]
                });
            if !belongs {
                return None;
            }
            removed_boxes.push(belongs);
        }
        let mut removed_sources: HashSet<N> = boxes[keep_boxes..]
            .iter()
            .filter_map(|b| b.source)
            .collect();
        // Suppressed and ignored descendants also belong to the removed
        // source closure, although they have no construction slot.
        for entry in &inputs.sources {
            if entry
                .parent
                .is_some_and(|parent| removed_sources.contains(&parent))
            {
                removed_sources.insert(entry.node);
            }
        }
        let mut seen: HashSet<N> = inputs.sources.iter().map(|entry| entry.node).collect();
        if added.iter().any(|node| !seen.insert(*node)) {
            return None;
        }
        Some(Self {
            parent,
            keep_boxes,
            source_index,
            children,
            added,
            removed_sources,
        })
    }

    pub(super) fn apply<S, R>(
        self,
        builder: &mut BoxBuilder<'_, S, R>,
        world: &mut LayoutWorld<N>,
        mut inputs: Box<ConstructionInputs<N>>,
    ) -> Result<Option<Box<ConstructionInputs<N>>>, LayoutError>
    where
        S: LayoutSource<NodeId = N>,
        R: LayoutStyleResolver<N>,
    {
        world.truncate_construction_tail(self.keep_boxes);
        let parent_node = inputs.sources[self.source_index].node;
        inputs.sources[self.source_index].children = Some(self.children);
        inputs
            .sources
            .retain(|entry| !self.removed_sources.contains(&entry.node));
        for input in &inputs.sources {
            builder
                .seen_sources
                .insert(input.node, "retained graph".to_owned());
        }
        let mut ancestor = Some(self.parent);
        while let Some(id) = ancestor {
            builder
                .active_sources
                .insert(world.boxes[id.index()].source.expect("owned source box"));
            ancestor = world.boxes[id.index()].parent;
        }
        builder.construction_inputs = Some(inputs);
        let style = world.boxes[self.parent.index()].style.clone();
        let mut children = world.boxes[self.parent.index()]
            .children
            .iter()
            .copied()
            .filter(|id| id.index() < self.keep_boxes)
            .collect::<Vec<_>>();
        for node in self.added {
            if builder.source.flat_parent(node) != Some(parent_node) {
                return Ok(None);
            }
            builder
                .seen_sources
                .insert(node, "retained block list".to_owned());
            children.extend(builder.build_source_node(world, node, &style)?);
        }
        if world.boxes[self.keep_boxes..]
            .iter()
            .any(|b| b.kind != LayoutBoxKind::Text && !plain_block(b))
        {
            return Ok(None);
        }
        builder.attach_children(world, self.parent, parent_node, &style, children, true)?;
        // Shared flow normalization can discard whitespace boxes. Ordinary
        // construction compacts them away; do not publish an unrooted new
        // source slot or run full compaction over retained survivors here.
        if world.boxes[self.keep_boxes..]
            .iter()
            .any(|b| b.parent.is_none())
        {
            return Ok(None);
        }
        let Some(inputs) = builder.construction_inputs.take() else {
            return Ok(None);
        };
        if !inputs.supports_graph(world) {
            return Ok(None);
        }
        Ok(Some(inputs))
    }
}

fn plain_block<N>(b: &LayoutBox<N>) -> bool {
    b.source.is_some()
        && b.owner.is_none()
        && b.pseudo.is_none()
        && b.anonymous_reason.is_none()
        && matches!(
            b.kind,
            LayoutBoxKind::PrincipalBlock | LayoutBoxKind::PrincipalFlowRoot
        )
        && !b.style.is_out_of_flow()
        && !b.style.is_floated()
        && b.element_semantics.as_ref().is_some_and(|semantics| {
            semantics.namespace == LayoutNamespace::Html
                && semantics.category == LayoutElementCategory::Generic
                && semantics.replaced.is_none()
        })
}

fn descends_from<N>(boxes: &[LayoutBox<N>], mut child: usize, ancestor: usize) -> bool {
    for _ in 0..boxes.len() {
        if child == ancestor {
            return true;
        }
        let Some(parent) = boxes[child].parent else {
            return false;
        };
        child = parent.index();
    }
    false
}
