use moli_layout::FrozenLayoutTree;

use crate::document_runtime::DomHandle;

struct LatestFrozenLayout {
    document: DomHandle,
    tree: Box<FrozenLayoutTree<DomHandle>>,
    metrics: moli_layout::LayoutPassMetrics,
}

/// Single-slot storage for one top-level Document's latest frozen layout tree.
///
/// This owns exactly one recursively frozen snapshot. It has no separately
/// keyed child tree, working layout world, source index, hit-test index, Taffy
/// cache, style borrow, diagnostic buffers, paint snapshot, timer, or invalidation
/// policy. The first geometry demand initializes it. Rendering updates,
/// screenshots and screencast frames replace this
/// published layout; print projections never enter this cache. The main
/// Document and each live popup own separate instances; embedded Documents
/// share the member trees in their top-level Document's snapshot.
#[derive(Default)]
pub(super) struct LatestLayoutTreeCache {
    latest: Option<LatestFrozenLayout>,
}

impl LatestLayoutTreeCache {
    pub(super) fn get(&self, document: DomHandle) -> Option<&FrozenLayoutTree<DomHandle>> {
        self.latest
            .as_ref()
            .filter(|snapshot| snapshot.document == document)
            .map(|snapshot| snapshot.tree.as_ref())
    }

    pub(super) fn get_for_root(&self, root: DomHandle) -> Option<&FrozenLayoutTree<DomHandle>> {
        self.latest.as_ref()?.tree.tree_for_root(root)
    }

    pub(super) fn pass_metrics(&self) -> Option<moli_layout::LayoutPassMetrics> {
        self.latest.as_ref().map(|snapshot| snapshot.metrics)
    }

    pub(super) fn publish(
        &mut self,
        document: DomHandle,
        tree: FrozenLayoutTree<DomHandle>,
        metrics: moli_layout::LayoutPassMetrics,
    ) {
        self.latest = Some(LatestFrozenLayout {
            document,
            tree: Box::new(tree),
            metrics,
        });
    }

    pub(super) fn clear(&mut self) {
        self.latest = None;
    }

    #[cfg(test)]
    pub(super) fn observability(
        &self,
    ) -> Option<(DomHandle, moli_layout::LayoutTreeRetentionMetrics)> {
        self.latest
            .as_ref()
            .map(|snapshot| (snapshot.document, snapshot.tree.retention_metrics()))
    }
}
