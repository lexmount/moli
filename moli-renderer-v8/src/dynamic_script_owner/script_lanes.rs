use std::collections::VecDeque;

use crate::{planning::PreparedScript, types::ScriptKind};

use super::{DynamicScriptEntry, DynamicScriptQueueKind, DynamicScriptReadyState};

/// The authoritative residence for runtime-created script scheduling lanes.
///
/// The three ordered queues form one insertion-ordered candidate lane: only
/// the earliest queue head may participate. The async queue exposes every
/// ready entry independently. Lifecycle eligibility filters candidates in
/// place; rejected entries are never copied into a second waiting queue.
#[derive(Debug, Default)]
pub(super) struct DynamicScriptLanes {
    pub(super) in_order: VecDeque<DynamicScriptEntry>,
    pub(super) importmap_in_order: VecDeque<DynamicScriptEntry>,
    pub(super) module_in_order: VecDeque<DynamicScriptEntry>,
    pub(super) async_scripts: VecDeque<DynamicScriptEntry>,
}

impl DynamicScriptLanes {
    pub(super) fn is_empty(&self) -> bool {
        self.in_order.is_empty()
            && self.importmap_in_order.is_empty()
            && self.module_in_order.is_empty()
            && self.async_scripts.is_empty()
    }

    pub(super) fn take_next_eligible(
        &mut self,
        predicate: &mut impl FnMut(&PreparedScript) -> bool,
    ) -> Option<DynamicScriptEntry> {
        self.rotate_blocked_module_in_order_failures();

        let in_order_front = self
            .next_in_order_front_queue_kind()
            .filter(|queue_kind| {
                self.front_entry_for_queue_kind(*queue_kind)
                    .is_some_and(|entry| Self::entry_is_eligible(entry, predicate))
            })
            .and_then(|queue_kind| {
                self.front_entry_for_queue_kind(queue_kind)
                    .and_then(|entry| Self::ready_order(entry).map(|order| (queue_kind, order)))
            });
        let async_front = self
            .async_scripts
            .iter()
            .enumerate()
            .filter(|(index, entry)| {
                !Self::deferred_module_failure_is_blocked_by_later_module(
                    &self.async_scripts,
                    *index,
                    entry,
                )
            })
            .filter(|(_, entry)| Self::entry_is_eligible(entry, predicate))
            .filter_map(|(index, entry)| {
                Self::ready_order(entry).map(|order| {
                    (
                        index,
                        order,
                        entry.script.position,
                        entry.script.kind == ScriptKind::Module,
                    )
                })
            })
            .min_by_key(|(_, order, position, _)| (*order, *position));

        match (in_order_front, async_front) {
            (
                Some((queue_kind, in_order_order)),
                Some((async_index, async_order, async_position, async_is_module)),
            ) => {
                let in_order_position = self
                    .front_entry_for_queue_kind(queue_kind)
                    .map(|entry| entry.script.position)
                    .unwrap_or(usize::MAX);
                if queue_kind == DynamicScriptQueueKind::ImportMapInOrder
                    && async_is_module
                    && in_order_position < async_position
                {
                    self.pop_front_for_queue_kind(queue_kind)
                } else if (async_order, async_position) < (in_order_order, in_order_position) {
                    self.async_scripts.remove(async_index)
                } else {
                    self.pop_front_for_queue_kind(queue_kind)
                }
            }
            (Some((queue_kind, _)), None) => self.pop_front_for_queue_kind(queue_kind),
            (None, Some((async_index, _, _, _))) => self.async_scripts.remove(async_index),
            (None, None) => None,
        }
    }

    fn entry_is_eligible(
        entry: &DynamicScriptEntry,
        predicate: &mut impl FnMut(&PreparedScript) -> bool,
    ) -> bool {
        match entry.ready_state {
            DynamicScriptReadyState::Ready { .. } | DynamicScriptReadyState::Failed { .. } => {
                predicate(&entry.script)
            }
            DynamicScriptReadyState::ReadyModuleScriptGraph { .. }
            | DynamicScriptReadyState::ReadyModuleScriptEvaluation { .. } => true,
            DynamicScriptReadyState::Loading
            | DynamicScriptReadyState::SuspendedModuleScriptGraph { .. }
            | DynamicScriptReadyState::SuspendedModuleScriptEvaluation { .. } => false,
        }
    }

    fn rotate_blocked_module_in_order_failures(&mut self) {
        let len = self.module_in_order.len();
        for _ in 0..len {
            let Some(front) = self.module_in_order.front() else {
                return;
            };
            if !Self::deferred_module_failure_is_blocked_by_later_module(
                &self.module_in_order,
                0,
                front,
            ) {
                return;
            }
            let Some(entry) = self.module_in_order.pop_front() else {
                return;
            };
            self.module_in_order.push_back(entry);
        }
    }

    fn deferred_module_failure_is_blocked_by_later_module(
        queue: &VecDeque<DynamicScriptEntry>,
        index: usize,
        entry: &DynamicScriptEntry,
    ) -> bool {
        let DynamicScriptReadyState::Failed { failure, .. } = &entry.ready_state else {
            return false;
        };
        if !failure.is_deferrable_module() {
            return false;
        }
        queue.iter().skip(index + 1).any(|later| {
            later.script.kind == ScriptKind::Module && later.script.position > entry.script.position
        })
    }

    fn next_in_order_front_queue_kind(&self) -> Option<DynamicScriptQueueKind> {
        let classic = self.in_order.front();
        let importmap = self.importmap_in_order.front();
        let module = self.module_in_order.front();
        let mut front: Option<(DynamicScriptQueueKind, &DynamicScriptEntry)> = None;
        for candidate in [
            classic.map(|entry| (DynamicScriptQueueKind::InOrder, entry)),
            importmap.map(|entry| (DynamicScriptQueueKind::ImportMapInOrder, entry)),
            module.map(|entry| (DynamicScriptQueueKind::ModuleInOrder, entry)),
        ]
        .into_iter()
        .flatten()
        {
            match front {
                Some((_, current)) if current.script.position <= candidate.1.script.position => {}
                _ => front = Some(candidate),
            }
        }
        let (queue_kind, entry) = front?;
        Self::ready_order(entry).is_some().then_some(queue_kind)
    }

    fn front_entry_for_queue_kind(
        &self,
        queue_kind: DynamicScriptQueueKind,
    ) -> Option<&DynamicScriptEntry> {
        match queue_kind {
            DynamicScriptQueueKind::InOrder => self.in_order.front(),
            DynamicScriptQueueKind::ImportMapInOrder => self.importmap_in_order.front(),
            DynamicScriptQueueKind::ModuleInOrder => self.module_in_order.front(),
            DynamicScriptQueueKind::Async => None,
        }
    }

    fn pop_front_for_queue_kind(
        &mut self,
        queue_kind: DynamicScriptQueueKind,
    ) -> Option<DynamicScriptEntry> {
        match queue_kind {
            DynamicScriptQueueKind::InOrder => self.in_order.pop_front(),
            DynamicScriptQueueKind::ImportMapInOrder => self.importmap_in_order.pop_front(),
            DynamicScriptQueueKind::ModuleInOrder => self.module_in_order.pop_front(),
            DynamicScriptQueueKind::Async => None,
        }
    }

    fn ready_order(entry: &DynamicScriptEntry) -> Option<u64> {
        match entry.ready_state {
            DynamicScriptReadyState::Loading
            | DynamicScriptReadyState::SuspendedModuleScriptGraph { .. }
            | DynamicScriptReadyState::SuspendedModuleScriptEvaluation { .. } => None,
            DynamicScriptReadyState::Ready { order, .. }
            | DynamicScriptReadyState::ReadyModuleScriptGraph { order, .. }
            | DynamicScriptReadyState::ReadyModuleScriptEvaluation { order, .. }
            | DynamicScriptReadyState::Failed { order, .. } => Some(order),
        }
    }
}

#[cfg(test)]
mod tests {
    use moli_parser::ScriptSource;
    use url::Url;

    use crate::{
        dom::NodeId,
        planning::{PreparedScript, ScriptFetchMetadata},
        types::{ScriptKind, ScriptMode, ScriptSourceKind},
    };

    use super::*;
    use crate::dynamic_script_owner::DynamicScriptOwnerId;

    fn ready_entry(
        id: u64,
        position: usize,
        mode: ScriptMode,
        handle: &str,
        ready_order: u64,
    ) -> DynamicScriptEntry {
        let url = Url::parse(&format!("https://example.test/script-{id}.js"))
            .expect("test URL should parse");
        DynamicScriptEntry {
            id: DynamicScriptOwnerId::from_u64(id),
            script: PreparedScript {
                position,
                node_id: NodeId::new(position + 1),
                kind: if mode == ScriptMode::ModuleInOrder {
                    ScriptKind::Module
                } else {
                    ScriptKind::Classic
                },
                mode,
                source_kind: ScriptSourceKind::Inline,
                fetch_metadata: ScriptFetchMetadata::default(),
                source: ScriptSource::Loaded(String::new()),
                url: url.clone(),
                base_url: url,
                initiator_url: Url::parse("https://example.test/")
                    .expect("initiator URL should parse"),
                host_script_handle: Some(handle.to_owned()),
            },
            ready_state: DynamicScriptReadyState::Ready {
                order: ready_order,
                source_network_result: None,
            },
        }
    }

    #[test]
    fn gated_ordered_head_does_not_hide_an_eligible_async_lane() {
        let mut lanes = DynamicScriptLanes::default();
        lanes
            .module_in_order
            .push_back(ready_entry(1, 1, ScriptMode::ModuleInOrder, "gated", 1));
        lanes
            .async_scripts
            .push_back(ready_entry(2, 2, ScriptMode::Async, "eligible-async", 2));

        let selected = lanes
            .take_next_eligible(&mut |script| script.host_script_handle.as_deref() != Some("gated"))
            .expect("eligible async lane should remain selectable");

        assert_eq!(selected.id, DynamicScriptOwnerId::from_u64(2));
        assert_eq!(
            lanes.module_in_order.front().map(|entry| entry.id),
            Some(DynamicScriptOwnerId::from_u64(1)),
            "the gated ordered head must remain in its original lane"
        );
    }

    #[test]
    fn gated_earliest_ordered_head_blocks_later_ordered_lanes() {
        let mut lanes = DynamicScriptLanes::default();
        lanes
            .in_order
            .push_back(ready_entry(1, 1, ScriptMode::InOrder, "gated", 1));
        lanes.module_in_order.push_back(ready_entry(
            2,
            2,
            ScriptMode::ModuleInOrder,
            "later-ordered",
            2,
        ));

        assert!(
            lanes
                .take_next_eligible(&mut |script| {
                    script.host_script_handle.as_deref() != Some("gated")
                })
                .is_none(),
            "a later ordered script must not jump over the shared earliest ordered head"
        );
        assert_eq!(lanes.in_order.len(), 1);
        assert_eq!(lanes.module_in_order.len(), 1);
    }

    #[test]
    fn gated_async_entry_does_not_hide_another_ready_async_entry() {
        let mut lanes = DynamicScriptLanes::default();
        lanes
            .async_scripts
            .push_back(ready_entry(1, 1, ScriptMode::Async, "gated", 1));
        lanes
            .async_scripts
            .push_back(ready_entry(2, 2, ScriptMode::Async, "eligible-async", 2));

        let selected = lanes
            .take_next_eligible(&mut |script| script.host_script_handle.as_deref() != Some("gated"))
            .expect("second ready async entry should remain selectable");

        assert_eq!(selected.id, DynamicScriptOwnerId::from_u64(2));
        assert_eq!(lanes.async_scripts.len(), 1);
        assert_eq!(
            lanes.async_scripts.front().map(|entry| entry.id),
            Some(DynamicScriptOwnerId::from_u64(1))
        );
    }
}
