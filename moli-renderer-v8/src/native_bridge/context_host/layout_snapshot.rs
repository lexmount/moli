use moli_layout::FrozenLayoutTree;

use crate::{
    document_runtime::DomHandle,
    style_engine::{StyleViewport, StyloStyleEnvironment},
};

/// Ambient browser style inputs shared by the snapshot's Document trees.
/// DOM and stylesheet mutations deliberately remain outside this identity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LayoutEnvironment {
    pub(super) viewport: StyleViewport,
    pub(super) media: StyloStyleEnvironment,
}

struct LatestFrozenLayout {
    document: DomHandle,
    tree: Box<FrozenLayoutTree<DomHandle>>,
    environment: LayoutEnvironment,
    reusable_for_demand: bool,
}

/// Single-slot storage for the latest successful frozen layout tree.
///
/// This owns exactly one recursively frozen snapshot. It has no separately
/// keyed child tree, working layout world, source index, hit-test index, Taffy
/// cache, style borrow, pass diagnostics, paint snapshot, timer, or invalidation
/// policy. The captured browser environment lets geometry demands check reuse
/// without discarding the sample used by snapshot-only CSSOM readers.
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

    pub(super) fn matches_environment(&self, environment: LayoutEnvironment) -> bool {
        self.latest.as_ref().is_some_and(|snapshot| {
            snapshot.reusable_for_demand && snapshot.environment == environment
        })
    }

    pub(super) fn publish(
        &mut self,
        document: DomHandle,
        tree: FrozenLayoutTree<DomHandle>,
        environment: LayoutEnvironment,
    ) {
        self.latest = Some(LatestFrozenLayout {
            document,
            tree: Box::new(tree),
            environment,
            reusable_for_demand: true,
        });
    }

    /// Keep the rendered world available to input while the next geometry
    /// demand rebuilds it after an interaction such as scrolling or hover.
    pub(super) fn invalidate_for_demand(&mut self) {
        if let Some(snapshot) = &mut self.latest {
            snapshot.reusable_for_demand = false;
        }
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
