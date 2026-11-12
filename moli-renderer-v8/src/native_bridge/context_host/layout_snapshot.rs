use moli_layout::{FrozenLayoutTree, LayoutQuerySnapshot, LayoutQueryView};

use crate::document_runtime::DomHandle;

struct LatestFrozenLayout {
    document: DomHandle,
    tree: LayoutQuerySnapshot<DomHandle>,
}

/// Single-slot storage for the latest successful frozen layout tree.
///
/// This owns exactly one recursively frozen snapshot. It has no separately
/// keyed child tree, working layout world, hit-test index, Taffy
/// cache, style borrow, pass diagnostics, paint snapshot, timer, or invalidation
/// policy. The first geometry demand initializes it. Screenshots, screencast
/// frames and explicit layout refreshes replace
/// this published layout; print projections never enter this cache.
/// Bounded source indexes are derived lazily by the immutable snapshot owner
/// and are dropped with it. Live DOM/style generations never validate them.
#[derive(Default)]
pub(super) struct LatestLayoutTreeCache {
    latest: Option<LatestFrozenLayout>,
}

impl LatestLayoutTreeCache {
    pub(super) fn get(&self, document: DomHandle) -> Option<&FrozenLayoutTree<DomHandle>> {
        self.latest
            .as_ref()
            .filter(|snapshot| snapshot.document == document)
            .map(|snapshot| snapshot.tree.tree())
    }

    pub(super) fn get_for_root(&self, root: DomHandle) -> Option<&FrozenLayoutTree<DomHandle>> {
        self.latest.as_ref()?.tree.tree().tree_for_root(root)
    }

    pub(super) fn queries_for_root(
        &self,
        root: DomHandle,
    ) -> Option<LayoutQueryView<'_, DomHandle>> {
        self.latest.as_ref()?.tree.queries_for_root(root)
    }

    pub(super) fn publish(&mut self, document: DomHandle, tree: FrozenLayoutTree<DomHandle>) {
        self.latest = Some(LatestFrozenLayout {
            document,
            tree: LayoutQuerySnapshot::new(tree),
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
