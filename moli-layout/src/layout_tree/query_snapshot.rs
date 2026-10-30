//! Snapshot-owned, bounded provenance indexes for repeated CSSOM reads.

use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    hash::Hash,
    sync::OnceLock,
};

use super::{
    FrozenLayoutTree, LayoutAnswers, LayoutElementMetrics, LayoutNodeOutput, LayoutPassMetrics,
    LayoutQuery, LayoutQueryAnswer, LayoutQueryBatch, LayoutTreeRetentionMetrics,
    MAX_RETAINED_LAYOUT_TREE_BYTES,
};

/// Owns an immutable published tree and its lazily derived source indexes.
/// Indexes cannot outlive, or be reused with a different version of, the tree.
pub struct LayoutQuerySnapshot<N>
where
    N: Copy + Debug + Eq + Hash,
{
    tree: Box<FrozenLayoutTree<N>>,
    metrics: LayoutTreeRetentionMetrics,
    indexes: OnceLock<Option<SourceIndexes<N>>>,
}

struct SourceIndexes<N> {
    root: SourceIndexTree<N>,
    estimated_bytes: usize,
}

struct SourceIndexTree<N> {
    root: N,
    outputs: HashMap<N, Option<LayoutNodeOutput>>,
    children: Vec<Self>,
}

impl<N: Copy + Debug + Eq + Hash> SourceIndexTree<N> {
    // A conservative upper bound, checked before allocating any indexes.
    // Hash tables reserve at most a power-of-two bucket count above twice
    // their requested entries. Vec growth uses at most four slots per ID.
    fn estimated_bytes(tree: &FrozenLayoutTree<N>) -> Option<usize> {
        let entries = tree
            .boxes
            .len()
            .checked_mul(2)?
            .checked_add(tree.scroll_proxy_links.len())?;
        let buckets = entries.checked_mul(2)?.checked_next_power_of_two()?.max(4);
        let mut bytes = std::mem::size_of::<Self>()
            .checked_add(buckets.checked_mul(
                std::mem::size_of::<(N, Option<LayoutNodeOutput>)>().checked_add(1)?,
            )?)?
            .checked_add(16)?;
        for layout_box in &tree.boxes {
            bytes = bytes.checked_add(
                layout_box
                    .fragments
                    .len()
                    .checked_mul(4)?
                    .checked_mul(std::mem::size_of::<super::LayoutFragmentId>())?,
            )?;
        }
        bytes = bytes.checked_add(
            tree.scroll_proxy_links
                .len()
                .checked_mul(4)?
                .checked_mul(std::mem::size_of::<super::LayoutOutputBoxId>())?,
        )?;
        bytes = bytes.checked_add(
            tree.embedded_frames()
                .len()
                .checked_mul(4)?
                .checked_mul(std::mem::size_of::<Self>())?,
        )?;
        for child in tree.embedded_frames() {
            bytes = bytes.checked_add(Self::estimated_bytes(&child.tree)?)?;
        }
        Some(bytes)
    }

    fn build(tree: &FrozenLayoutTree<N>) -> Self {
        Self {
            root: tree.source_root(),
            outputs: tree.all_source_outputs(),
            children: tree
                .embedded_frames()
                .map(|frame| Self::build(&frame.tree))
                .collect(),
        }
    }

    fn for_root(&self, root: N) -> Option<&HashMap<N, Option<LayoutNodeOutput>>> {
        if self.root == root {
            return Some(&self.outputs);
        }
        self.children.iter().find_map(|child| child.for_root(root))
    }
}

impl<N: Copy + Debug + Eq + Hash> LayoutQuerySnapshot<N> {
    pub fn new(tree: FrozenLayoutTree<N>) -> Self {
        let metrics = tree.retention_metrics();
        Self {
            tree: Box::new(tree),
            metrics,
            indexes: OnceLock::new(),
        }
    }

    pub fn tree(&self) -> &FrozenLayoutTree<N> {
        &self.tree
    }

    pub fn queries_for_root(&self, root: N) -> Option<LayoutQueryView<'_, N>> {
        self.tree.tree_for_root(root).map(|tree| LayoutQueryView {
            snapshot: self,
            tree,
        })
    }

    pub fn retention_metrics(&self) -> LayoutTreeRetentionMetrics {
        let mut metrics = self.metrics;
        if let Some(Some(indexes)) = self.indexes.get() {
            metrics.estimated_geometry_bytes = metrics
                .estimated_geometry_bytes
                .saturating_add(indexes.estimated_bytes);
        }
        metrics
    }

    fn outputs(&self, root: N) -> Option<&HashMap<N, Option<LayoutNodeOutput>>> {
        let indexes = self
            .indexes
            .get_or_init(|| {
                let estimated_bytes = SourceIndexTree::estimated_bytes(&self.tree)?
                    .checked_add(std::mem::size_of::<Self>())?;
                if self
                    .metrics
                    .estimated_geometry_bytes
                    .checked_add(estimated_bytes)?
                    > MAX_RETAINED_LAYOUT_TREE_BYTES
                {
                    return None;
                }
                Some(SourceIndexes {
                    root: SourceIndexTree::build(&self.tree),
                    estimated_bytes,
                })
            })
            .as_ref()?;
        indexes.root.for_root(root)
    }
}

/// A query view tied to one exact member tree of its owning snapshot.
pub struct LayoutQueryView<'a, N>
where
    N: Copy + Debug + Eq + Hash,
{
    snapshot: &'a LayoutQuerySnapshot<N>,
    tree: &'a FrozenLayoutTree<N>,
}

impl<N: Copy + Debug + Eq + Hash> LayoutQueryView<'_, N> {
    pub fn tree(&self) -> &FrozenLayoutTree<N> {
        self.tree
    }

    pub fn answer_query(&self, query: &LayoutQuery<N>) -> LayoutQueryAnswer<N> {
        if let Some(source) = query.fragment_source()
            && let Some(outputs) = self.snapshot.outputs(self.tree.source_root())
        {
            return self
                .tree
                .answer_fragment_query(query, outputs.get(&source).and_then(Option::as_ref));
        }
        if let LayoutQuery::ElementMetrics { source } = query {
            return LayoutQueryAnswer::ElementMetrics(
                self.element_metrics_with_offset_parent_filter(*source, |_| true),
            );
        }
        self.tree.answer_query(query)
    }

    pub fn answer_queries_with(
        &self,
        batch: &LayoutQueryBatch<N>,
        metrics: LayoutPassMetrics,
        mut specialize: impl FnMut(&LayoutQuery<N>) -> Option<LayoutQueryAnswer<N>>,
    ) -> LayoutAnswers<N> {
        // Preserve the existing operation-local batch optimization if the
        // retained index does not fit alongside the canonical snapshot.
        if !batch.queries.iter().any(|query| {
            query.fragment_source().is_some() || matches!(query, LayoutQuery::ElementMetrics { .. })
        }) || self.snapshot.outputs(self.tree.source_root()).is_none()
        {
            return self.tree.answer_queries_with(batch, metrics, specialize);
        }
        let answers = batch
            .queries
            .iter()
            .map(|query| specialize(query).unwrap_or_else(|| self.answer_query(query)))
            .collect();
        LayoutAnswers { answers, metrics }
    }

    /// Rendered-text membership without constructing or projecting quads.
    pub fn sources_with_content_fragments(&self, sources: &[N]) -> HashSet<N> {
        if sources.is_empty() {
            return HashSet::new();
        }
        let mut temporary;
        let outputs = if let Some(outputs) = self.snapshot.outputs(self.tree.source_root()) {
            outputs
        } else {
            // Budget fallback still batches membership into one provenance pass.
            temporary = sources
                .iter()
                .copied()
                .map(|source| (source, None))
                .collect();
            self.tree.source_outputs(&mut temporary);
            &temporary
        };
        sources
            .iter()
            .copied()
            .filter(|source| {
                outputs
                    .get(source)
                    .and_then(Option::as_ref)
                    .is_some_and(|output| self.tree.has_content_quads_for_output(output))
            })
            .collect()
    }

    pub fn element_metrics_with_offset_parent_filter(
        &self,
        source: N,
        filter: impl FnMut(N) -> bool,
    ) -> Option<LayoutElementMetrics<N>> {
        if let Some(outputs) = self.snapshot.outputs(self.tree.source_root()) {
            let output = outputs.get(&source)?.as_ref()?;
            return self
                .tree
                .element_metrics_for_output_with_offset_parent_filter(output, filter);
        }
        self.tree
            .element_metrics_for_source_with_offset_parent_filter(source, filter)
    }
}

#[cfg(test)]
mod tests {
    use super::super::source_query::tests::inline_fragment_tree;
    use super::super::{FrozenCoordinateSpace, FrozenEmbeddedFrame, LayoutTransform2D};
    use super::*;

    #[test]
    fn budget_fallback_preserves_answers_and_does_not_retry_allocation() {
        let tree = inline_fragment_tree();
        let query = LayoutQuery::ClientRects { source: 1 };
        let expected = tree.answer_query(&query);
        let mut snapshot = LayoutQuerySnapshot::new(tree);
        // Model a canonical tree occupying the entire retention budget without
        // allocating a giant fixture merely to test the admission decision.
        snapshot.metrics.estimated_geometry_bytes = MAX_RETAINED_LAYOUT_TREE_BYTES;
        let view = snapshot.queries_for_root(0).unwrap();
        assert_eq!(view.answer_query(&query), expected);
        assert_eq!(view.answer_query(&query), expected);
        assert!(matches!(snapshot.indexes.get(), Some(None)));
        let sources = [0, 1, 99];
        let expected = sources
            .into_iter()
            .filter(|source| !snapshot.tree().content_quads_for_source(*source).is_empty())
            .collect::<HashSet<_>>();
        assert_eq!(view.sources_with_content_fragments(&sources), expected);
        assert_eq!(
            snapshot.retention_metrics().estimated_geometry_bytes,
            MAX_RETAINED_LAYOUT_TREE_BYTES
        );
    }

    #[test]
    fn member_documents_with_overlapping_sources_use_their_own_provenance() {
        let parent = inline_fragment_tree();
        let mut child = inline_fragment_tree();
        child.fragments[0].rect.x = 200.0;
        // Build a distinct member root with the same source ID as the parent.
        let child = FrozenLayoutTree::new(
            2,
            child.viewport,
            child.viewport_scroll,
            child.content_size,
            child.root_box,
            child.boxes,
            child.fragments,
            child.scroll_proxy_links,
            FrozenCoordinateSpace {
                owner: None,
                local_to_viewport: LayoutTransform2D::IDENTITY,
                local_to_viewport_ignoring_css_transforms: LayoutTransform2D::IDENTITY,
            },
            child.clip_chain,
            Vec::new(),
        );
        let query = LayoutQuery::ClientRects { source: 1 };
        let parent_expected = parent.answer_query(&query);
        let child_expected = child.answer_query(&query);
        assert_ne!(parent_expected, child_expected);
        let parent = FrozenLayoutTree::new(
            0,
            parent.viewport,
            parent.viewport_scroll,
            parent.content_size,
            parent.root_box,
            parent.boxes,
            parent.fragments,
            parent.scroll_proxy_links,
            FrozenCoordinateSpace {
                owner: None,
                local_to_viewport: LayoutTransform2D::IDENTITY,
                local_to_viewport_ignoring_css_transforms: LayoutTransform2D::IDENTITY,
            },
            parent.clip_chain,
            vec![FrozenEmbeddedFrame {
                source: 10,
                tree: Box::new(child),
            }],
        );
        let snapshot = LayoutQuerySnapshot::new(parent);
        assert_eq!(
            snapshot.queries_for_root(0).unwrap().answer_query(&query),
            parent_expected
        );
        assert_eq!(
            snapshot.queries_for_root(2).unwrap().answer_query(&query),
            child_expected
        );
        assert!(snapshot.queries_for_root(99).is_none());
    }

    #[test]
    fn replacement_snapshot_cannot_reuse_previous_geometry() {
        let first = LayoutQuerySnapshot::new(inline_fragment_tree());
        let query = LayoutQuery::ClientRects { source: 1 };
        let before = first.queries_for_root(0).unwrap().answer_query(&query);
        let mut next_tree = inline_fragment_tree();
        next_tree.fragments[0].rect.x = 100.0;
        let next = LayoutQuerySnapshot::new(next_tree);
        assert!(next.indexes.get().is_none());
        let after = next.queries_for_root(0).unwrap().answer_query(&query);
        assert_ne!(before, after);
        assert_eq!(
            first.queries_for_root(0).unwrap().answer_query(&query),
            before
        );
    }
}
