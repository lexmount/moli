use std::cell::RefCell;

use super::{
    StyleSourceId,
    cause::{PendingStyleInvalidationMergeClass, PendingStyleInvalidationWorkKind},
    native_invalidation::NativeSnapshotBatch,
    target_queries::{
        PendingStyleInvalidationTargetQueries, PendingStyleInvalidationTargetSourceIdSink,
        PendingTargetQueryIndices,
    },
};
use indexmap::IndexSet;

#[derive(Default)]
pub(super) struct PendingStyleInvalidations {
    queue: RefCell<PendingStyleInvalidationQueue>,
}

#[derive(Default)]
struct PendingStyleInvalidationQueue {
    work_items: Vec<PendingStyleInvalidationWork>,
    // Only the last work item can absorb more targets.
    tail_target_indices: Option<PendingTargetQueryIndices>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PendingStyleInvalidationWork {
    pub(super) kind: PendingStyleInvalidationWorkKind,
    pub(super) target_queries: Vec<PendingStyleInvalidationTargetQueries>,
    pub(super) native_snapshots: Option<NativeSnapshotBatch>,
    merge_class: PendingStyleInvalidationMergeClass,
}

impl PendingStyleInvalidationWork {
    pub(super) fn new(
        kind: PendingStyleInvalidationWorkKind,
        target_queries: Vec<PendingStyleInvalidationTargetQueries>,
        merge_class: PendingStyleInvalidationMergeClass,
    ) -> Self {
        Self {
            kind,
            target_queries,
            native_snapshots: None,
            merge_class,
        }
    }

    pub(super) fn with_native_snapshots(
        kind: PendingStyleInvalidationWorkKind,
        snapshots: NativeSnapshotBatch,
        merge_class: PendingStyleInvalidationMergeClass,
    ) -> Self {
        Self {
            kind,
            target_queries: Vec::new(),
            native_snapshots: Some(snapshots),
            merge_class,
        }
    }
}

#[cfg(test)]
impl PendingStyleInvalidationWork {
    pub(super) fn for_test(
        kind: PendingStyleInvalidationWorkKind,
        target_queries: Vec<PendingStyleInvalidationTargetQueries>,
    ) -> Self {
        Self::new(
            kind,
            target_queries,
            PendingStyleInvalidationMergeClass::None,
        )
    }
}

pub(super) struct PendingStyleInvalidationBatch {
    pub(super) work_items: Vec<PendingStyleInvalidationWork>,
}

#[derive(Default)]
struct PendingStyleInvalidationBatchSourceIds {
    source_ids: IndexSet<StyleSourceId>,
}

impl PendingStyleInvalidations {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn extend_work(&self, work: PendingStyleInvalidationWork) {
        if work.target_queries.is_empty() && work.native_snapshots.is_none() {
            return;
        }
        let mut queue = self.queue.borrow_mut();
        let PendingStyleInvalidationQueue {
            work_items,
            tail_target_indices,
        } = &mut *queue;
        if let Some(tail) = work_items.last_mut()
            && tail.merge_class != PendingStyleInvalidationMergeClass::None
            && tail.merge_class == work.merge_class
            && tail.native_snapshots.is_some() == work.native_snapshots.is_some()
        {
            if let (Some(tail), Some(incoming)) =
                (&mut tail.native_snapshots, work.native_snapshots)
            {
                tail.merge_from(incoming);
            } else {
                tail_target_indices
                    .get_or_insert_with(|| PendingTargetQueryIndices::new(&tail.target_queries))
                    .merge_into(&mut tail.target_queries, work.target_queries);
            }
        } else {
            *tail_target_indices = None;
            work_items.push(work);
        }
    }

    pub(super) fn take(&self) -> PendingStyleInvalidationBatch {
        PendingStyleInvalidationBatch {
            work_items: self.queue.take().work_items,
        }
    }

    #[cfg(test)]
    pub(super) fn work_item_count_for_test(&self) -> usize {
        self.queue.borrow().work_items.len()
    }

    #[cfg(test)]
    pub(super) fn has_native_snapshots_for_test(&self) -> bool {
        self.queue
            .borrow()
            .work_items
            .iter()
            .any(|work| work.native_snapshots.is_some())
    }

    #[cfg(test)]
    pub(super) fn work_kind_names_for_test(&self) -> Vec<&'static str> {
        self.queue
            .borrow()
            .work_items
            .iter()
            .map(|work_item| work_item.kind.name_for_test())
            .collect()
    }

    pub(super) fn clear(&self) {
        let mut queue = self.queue.borrow_mut();
        queue.work_items.clear();
        queue.tail_target_indices = None;
    }
}

impl PendingStyleInvalidationBatch {
    pub(super) fn has_work(&self) -> bool {
        !self.work_items.is_empty()
    }

    pub(super) fn stylesheet_source_ids(&self) -> IndexSet<StyleSourceId> {
        let mut source_ids = PendingStyleInvalidationBatchSourceIds::default();
        for work_item in &self.work_items {
            for target_query in &work_item.target_queries {
                target_query.record_stylesheet_source_id_into(&mut source_ids);
            }
        }
        source_ids.source_ids
    }
}

impl PendingStyleInvalidationTargetSourceIdSink for PendingStyleInvalidationBatchSourceIds {
    fn record_pending_target_stylesheet_source_id(&mut self, source_id: &StyleSourceId) {
        self.source_ids.insert(source_id.clone());
    }
}

#[cfg(test)]
mod tests {
    use crate::document_runtime::DomHandle;
    use crate::style_engine::{StyleInvalidationSourceTarget, StyleScopeId};

    use super::*;

    #[test]
    fn pending_target_index_follows_the_tail_across_merge_boundaries() {
        let queue = PendingStyleInvalidations::new();
        let document = DomHandle::new(1);
        let [first, second, third] = [2, 3, 4].map(DomHandle::new);
        let target = |root| StyleInvalidationSourceTarget::FallbackRoot {
            scope_id: StyleScopeId::Document(document),
            root,
        };
        let enqueue = |merge_class, roots: &[DomHandle]| {
            queue.extend_work(PendingStyleInvalidationWork::new(
                PendingStyleInvalidationWorkKind::Mutation,
                roots
                    .iter()
                    .map(|&root| {
                        PendingStyleInvalidationTargetQueries::fallback_roots_for_test(
                            target(root),
                            [root],
                        )
                    })
                    .collect(),
                merge_class,
            ));
        };
        let take_targets = || {
            queue
                .take()
                .work_items
                .into_iter()
                .map(|work| {
                    work.target_queries
                        .into_iter()
                        .map(|query| query.target().clone())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };

        let attributes = PendingStyleInvalidationMergeClass::AttributeMutation;
        let structural = PendingStyleInvalidationMergeClass::ChildListStructuralMutation;
        enqueue(attributes, &[first]);
        enqueue(attributes, &[second]);
        enqueue(attributes, &[second, third, first]);
        enqueue(structural, &[third]);
        enqueue(structural, &[second, third]);
        assert_eq!(
            take_targets(),
            [
                vec![target(first), target(second), target(third)],
                vec![target(third), target(second)],
            ]
        );

        enqueue(attributes, &[second]);
        enqueue(attributes, &[first]);
        queue.clear();
        enqueue(attributes, &[first]);
        enqueue(attributes, &[first, third]);
        assert_eq!(take_targets(), [vec![target(first), target(third)]]);
    }
}
