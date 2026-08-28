use std::collections::HashSet;

use crate::document_runtime::{DevToolsDomChildListMutationFact, DevToolsDomMutationFact};
use crate::dom::native::NodeType;
use crate::runtime::{
    RendererDocumentLifecycleIdentity, RendererDomMutationEvent, RendererDomMutationEventBatch,
};
use moli_page_types::{DevToolsSessionKey, DocumentNodeSnapshot};

use super::PageVm;

impl PageVm {
    pub(super) fn sync_devtools_dom_mutation_recording_interest(&mut self) {
        let enabled = self.dom_agent_state.has_frontend_bindings();
        self.vm()
            .document_runtime
            .dom_host()
            .set_devtools_mutation_records_enabled(enabled);
    }

    pub(crate) fn mark_document_snapshot_children_requested(
        &mut self,
        inspector_session_id: Option<&str>,
        snapshot: &DocumentNodeSnapshot,
        depth: i32,
    ) {
        let document_id = self.current_dom_agent_document_id();
        let mut stack = vec![(snapshot, depth)];
        while let Some((snapshot, remaining_depth)) = stack.pop() {
            let forced_single_text_child = remaining_depth == 0
                && matches!(snapshot.children.as_slice(), [child] if child.node_type == NodeType::Text as u8);
            if let Some(backend_node_id) = snapshot.backend_node_id {
                self.dom_agent_state.cache_child_count(
                    inspector_session_id,
                    document_id,
                    backend_node_id,
                    snapshot.child_count,
                );
                if remaining_depth != 0 || forced_single_text_child {
                    self.dom_agent_state.mark_children_requested(
                        inspector_session_id,
                        document_id,
                        backend_node_id,
                        snapshot.child_count,
                    );
                }
            }
            if remaining_depth == 0 {
                if forced_single_text_child {
                    stack.extend(snapshot.children.iter().rev().map(|child| (child, 0)));
                }
                continue;
            }
            let next_depth = if remaining_depth > 0 {
                remaining_depth - 1
            } else {
                remaining_depth
            };
            stack.extend(
                snapshot
                    .shadow_roots
                    .iter()
                    .rev()
                    .map(|child| (child, next_depth)),
            );
            stack.extend(
                snapshot
                    .children
                    .iter()
                    .rev()
                    .map(|child| (child, next_depth)),
            );
        }
    }

    pub(crate) fn mark_document_node_children_requested(
        &mut self,
        inspector_session_id: Option<&str>,
        backend_node_id: u32,
        child_count: usize,
    ) {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state.mark_children_requested(
            inspector_session_id,
            document_id,
            backend_node_id,
            child_count,
        );
    }

    #[cfg(test)]
    pub(super) fn take_pending_dom_mutation_event_batches(
        &mut self,
    ) -> Vec<RendererDomMutationEventBatch> {
        self.flush_pending_dom_mutation_event_batches();
        std::mem::take(&mut self.pending_dom_mutation_event_batches)
    }

    pub(crate) fn flush_pending_dom_mutation_event_batches(&mut self) {
        let facts = self
            .vm_mut()
            .document_runtime
            .take_pending_devtools_dom_mutations();
        if facts.is_empty() {
            return;
        }

        let agent_token = self.devtools_agent_token();
        let sessions = self.dom_agent_state.session_keys();
        let batches = sessions
            .into_iter()
            .filter_map(|session_id| {
                let events =
                    self.project_dom_mutation_facts_for_session(session_id.as_deref(), &facts);
                (!events.is_empty()).then(|| {
                    RendererDomMutationEventBatch::new(
                        agent_token,
                        DevToolsSessionKey::from_wire_session_id(session_id.as_deref()),
                        events,
                    )
                })
            })
            .collect::<Vec<_>>();
        self.pending_dom_mutation_event_batches.extend(batches);
    }

    /// Freezes every DOM mutation produced before the current owner boundary
    /// into the Page's ordered renderer-output journal.
    ///
    /// Mutation facts remain renderer-local until `PageVm` can project them
    /// through each Inspector session's frontend bindings. Once projected,
    /// they must join the same journal as lifecycle and Runtime observations;
    /// leaving them in this side queue across a turn would let later output
    /// overtake an earlier DOM change.
    pub(super) fn absorb_pending_dom_mutations_into_output_journal(&mut self) {
        self.flush_pending_dom_mutation_event_batches();
        let batches = std::mem::take(&mut self.pending_dom_mutation_event_batches);
        if batches.is_empty() {
            return;
        }
        self.append_renderer_output_records(
            batches
                .into_iter()
                .map(|batch| {
                    super::PendingRendererOutputRecord::observation(
                        None,
                        super::RendererProtocolObservation::DomMutations(batch),
                    )
                })
                .collect(),
        );
    }

    /// Implements the root Inspector DOM binding barrier that Chromium places
    /// after page `DOMContentLoaded` handlers and before its
    /// `DOM.documentUpdated` notification.
    pub(super) fn prepare_dom_agent_for_main_document_dom_content_loaded(
        &mut self,
        dispatched_document: RendererDocumentLifecycleIdentity,
    ) -> bool {
        let current_document = self.document_lifecycle.identity();
        if current_document != dispatched_document {
            tracing::debug!(
                ?dispatched_document,
                ?current_document,
                "ignored stale DCL DOM-agent binding barrier"
            );
            return false;
        }
        // DCL listeners and their task-end checkpoint may have changed the
        // tree. Preserve those notifications before invalidating the ids they
        // reference.
        self.absorb_pending_dom_mutations_into_output_journal();
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state
            .discard_all_frontend_bindings(document_id);
        self.sync_devtools_dom_mutation_recording_interest();
        true
    }

    fn project_dom_mutation_facts_for_session(
        &mut self,
        inspector_session_id: Option<&str>,
        facts: &[DevToolsDomMutationFact],
    ) -> Vec<RendererDomMutationEvent> {
        let document_id = self.current_dom_agent_document_id();
        let include_whitespace = self
            .dom_agent_state
            .includes_whitespace(inspector_session_id, document_id);
        let mut events = Vec::new();
        let mut resynchronized_parents = HashSet::new();
        for fact in facts {
            match fact {
                DevToolsDomMutationFact::Attribute {
                    target,
                    name,
                    value,
                } => {
                    let Some(node_id) = self.existing_frontend_node_id_for_handle(
                        inspector_session_id,
                        document_id,
                        *target,
                    ) else {
                        continue;
                    };
                    match value {
                        Some(value) => events.push(RendererDomMutationEvent::AttributeModified {
                            node_id,
                            name: name.clone(),
                            value: value.clone(),
                        }),
                        None => events.push(RendererDomMutationEvent::AttributeRemoved {
                            node_id,
                            name: name.clone(),
                        }),
                    }
                }
                DevToolsDomMutationFact::CharacterData {
                    target,
                    is_text,
                    old_value,
                    value,
                    parent,
                    previous_sibling_without_whitespace,
                    final_parent_child_count_without_whitespace,
                    snapshot,
                } => {
                    self.project_character_data_mutation_for_session(
                        inspector_session_id,
                        document_id,
                        include_whitespace,
                        *target,
                        *is_text,
                        old_value,
                        value,
                        *parent,
                        *previous_sibling_without_whitespace,
                        *final_parent_child_count_without_whitespace,
                        snapshot,
                        &mut events,
                    );
                }
                DevToolsDomMutationFact::ChildList(mutation) => {
                    if resynchronized_parents.contains(&mutation.target) {
                        self.remove_finally_detached_bindings_for_resynchronized_parent(
                            inspector_session_id,
                            document_id,
                            mutation,
                        );
                        continue;
                    }
                    if self.project_child_list_mutation_for_session(
                        inspector_session_id,
                        document_id,
                        include_whitespace,
                        mutation,
                        &mut events,
                    ) {
                        resynchronized_parents.insert(mutation.target);
                    }
                }
            }
        }
        events
    }

    #[allow(clippy::too_many_arguments)]
    fn project_character_data_mutation_for_session(
        &mut self,
        inspector_session_id: Option<&str>,
        document_id: Option<crate::frame_owner_model::DocumentId>,
        include_whitespace: bool,
        target: crate::dom::native::NativeNodeId,
        is_text: bool,
        old_value: &str,
        value: &str,
        parent: Option<crate::dom::native::NativeNodeId>,
        previous_sibling_without_whitespace: Option<crate::dom::native::NativeNodeId>,
        final_parent_child_count_without_whitespace: Option<usize>,
        snapshot: &DocumentNodeSnapshot,
        events: &mut Vec<RendererDomMutationEvent>,
    ) {
        let old_visible = include_whitespace
            || !is_text
            || !crate::runtime::page_dom::inspector_whitespace_text_value(old_value);
        let new_visible = include_whitespace
            || !is_text
            || !crate::runtime::page_dom::inspector_whitespace_text_value(value);
        if old_visible == new_visible {
            if new_visible
                && let Some(node_id) = self.existing_frontend_node_id_for_handle(
                    inspector_session_id,
                    document_id,
                    target,
                )
            {
                events.push(RendererDomMutationEvent::CharacterDataModified {
                    node_id,
                    character_data: value.to_owned(),
                });
            }
            return;
        }

        let target_backend_node_id = self.renderer_backend_node_id_for_live_handle(target);
        let Some(parent) = parent else {
            if !new_visible && let Some(target_backend_node_id) = target_backend_node_id {
                self.dom_agent_state
                    .remove_frontend_bindings_for_backend_node_ids(
                        inspector_session_id,
                        document_id,
                        [target_backend_node_id],
                    );
            }
            return;
        };
        let Some(parent_backend_node_id) = self.renderer_backend_node_id_for_live_handle(parent)
        else {
            return;
        };
        let Some(parent_node_id) = self
            .dom_agent_state
            .frontend_node_id_for_existing_backend_node_id(
                inspector_session_id,
                document_id,
                parent_backend_node_id,
            )
        else {
            if !new_visible && let Some(target_backend_node_id) = target_backend_node_id {
                self.dom_agent_state
                    .remove_frontend_bindings_for_backend_node_ids(
                        inspector_session_id,
                        document_id,
                        [target_backend_node_id],
                    );
            }
            return;
        };
        let children_requested = self.dom_agent_state.children_requested(
            inspector_session_id,
            document_id,
            parent_backend_node_id,
        );

        if !new_visible {
            let mut child_count = self
                .dom_agent_state
                .cached_child_count(inspector_session_id, document_id, parent_backend_node_id)
                .unwrap_or_else(|| {
                    final_parent_child_count_without_whitespace
                        .unwrap_or_default()
                        .saturating_add(1)
                });
            child_count = child_count.saturating_sub(1);
            self.dom_agent_state.cache_child_count(
                inspector_session_id,
                document_id,
                parent_backend_node_id,
                child_count,
            );
            if children_requested {
                if let Some(node_id) = self.existing_frontend_node_id_for_handle(
                    inspector_session_id,
                    document_id,
                    target,
                ) {
                    events.push(RendererDomMutationEvent::ChildNodeRemoved {
                        parent_node_id,
                        node_id,
                    });
                } else {
                    self.resynchronize_child_nodes_for_session(
                        inspector_session_id,
                        parent,
                        parent_node_id,
                        include_whitespace,
                        events,
                        "visible text node has no frontend binding",
                    );
                }
            } else {
                events.push(RendererDomMutationEvent::ChildNodeCountUpdated {
                    node_id: parent_node_id,
                    child_node_count: child_count,
                });
            }
            if let Some(target_backend_node_id) = target_backend_node_id {
                self.dom_agent_state
                    .remove_frontend_bindings_for_backend_node_ids(
                        inspector_session_id,
                        document_id,
                        [target_backend_node_id],
                    );
            }
            return;
        }

        let mut child_count = self
            .dom_agent_state
            .cached_child_count(inspector_session_id, document_id, parent_backend_node_id)
            .unwrap_or_else(|| {
                final_parent_child_count_without_whitespace
                    .unwrap_or_default()
                    .saturating_sub(1)
            });
        child_count = child_count.saturating_add(1);
        self.dom_agent_state.cache_child_count(
            inspector_session_id,
            document_id,
            parent_backend_node_id,
            child_count,
        );
        if !children_requested {
            events.push(RendererDomMutationEvent::ChildNodeCountUpdated {
                node_id: parent_node_id,
                child_node_count: child_count,
            });
            return;
        }

        let previous_node_id = match previous_sibling_without_whitespace {
            None => 0,
            Some(previous_sibling) => {
                let Some(previous_node_id) = self.existing_frontend_node_id_for_handle(
                    inspector_session_id,
                    document_id,
                    previous_sibling,
                ) else {
                    self.resynchronize_child_nodes_for_session(
                        inspector_session_id,
                        parent,
                        parent_node_id,
                        include_whitespace,
                        events,
                        "previous visible sibling has no frontend binding",
                    );
                    return;
                };
                previous_node_id
            }
        };
        let mut snapshot = snapshot.clone();
        self.assign_renderer_dom_agent_ids_to_snapshot(
            inspector_session_id,
            &mut snapshot,
            include_whitespace,
        );
        if snapshot.frontend_node_id.is_none() {
            self.resynchronize_child_nodes_for_session(
                inspector_session_id,
                parent,
                parent_node_id,
                include_whitespace,
                events,
                "visible text node has no frontend binding after insertion",
            );
            return;
        }
        events.push(RendererDomMutationEvent::ChildNodeInserted {
            parent_node_id,
            previous_node_id,
            node: Box::new(snapshot),
        });
    }

    fn project_child_list_mutation_for_session(
        &mut self,
        inspector_session_id: Option<&str>,
        document_id: Option<crate::frame_owner_model::DocumentId>,
        include_whitespace: bool,
        mutation: &DevToolsDomChildListMutationFact,
        events: &mut Vec<RendererDomMutationEvent>,
    ) -> bool {
        let Some(parent_backend_node_id) =
            self.renderer_backend_node_id_for_live_handle(mutation.target)
        else {
            return false;
        };
        let Some(parent_node_id) = self
            .dom_agent_state
            .frontend_node_id_for_existing_backend_node_id(
                inspector_session_id,
                document_id,
                parent_backend_node_id,
            )
        else {
            return false;
        };
        let children_requested = self.dom_agent_state.children_requested(
            inspector_session_id,
            document_id,
            parent_backend_node_id,
        );

        let final_child_count = if include_whitespace {
            mutation.final_child_count
        } else {
            mutation.final_child_count_without_whitespace
        };
        let removed_node_count = mutation
            .removed_subtrees
            .iter()
            .filter(|subtree| include_whitespace || !subtree.root_is_whitespace_text)
            .count();
        let added_node_count = mutation
            .added_nodes
            .iter()
            .filter(|added| {
                include_whitespace
                    || !crate::runtime::page_dom::inspector_whitespace_text_snapshot(
                        &added.snapshot,
                    )
            })
            .count();

        let mut child_count = self
            .dom_agent_state
            .cached_child_count(inspector_session_id, document_id, parent_backend_node_id)
            .unwrap_or_else(|| {
                final_child_count
                    .saturating_add(removed_node_count)
                    .saturating_sub(added_node_count)
            });
        for removed_subtree in &mutation.removed_subtrees {
            let removed_root = removed_subtree.handles.first().copied();
            if !include_whitespace && removed_subtree.root_is_whitespace_text {
                let backend_node_ids = removed_subtree
                    .handles
                    .iter()
                    .filter_map(|handle| self.renderer_backend_node_id_for_live_handle(*handle))
                    .collect::<Vec<_>>();
                self.dom_agent_state
                    .remove_frontend_bindings_for_backend_node_ids(
                        inspector_session_id,
                        document_id,
                        backend_node_ids,
                    );
                continue;
            }
            if removed_root.is_some_and(|removed_root| {
                mutation.removal_was_prepublished_for_session(inspector_session_id, removed_root)
            }) {
                continue;
            }
            child_count = child_count.saturating_sub(1);
            self.dom_agent_state.cache_child_count(
                inspector_session_id,
                document_id,
                parent_backend_node_id,
                child_count,
            );
            if children_requested
                && let Some(node_id) = removed_root.and_then(|handle| {
                    self.existing_frontend_node_id_for_handle(
                        inspector_session_id,
                        document_id,
                        handle,
                    )
                })
            {
                events.push(RendererDomMutationEvent::ChildNodeRemoved {
                    parent_node_id,
                    node_id,
                });
            } else if !children_requested {
                events.push(RendererDomMutationEvent::ChildNodeCountUpdated {
                    node_id: parent_node_id,
                    child_node_count: child_count,
                });
            }
            let backend_node_ids = removed_subtree
                .handles
                .iter()
                .filter_map(|handle| self.renderer_backend_node_id_for_live_handle(*handle))
                .collect::<Vec<_>>();
            self.dom_agent_state
                .remove_frontend_bindings_for_backend_node_ids(
                    inspector_session_id,
                    document_id,
                    backend_node_ids,
                );
        }

        let mut previous_node_id = if children_requested {
            let previous_sibling = if include_whitespace {
                mutation.previous_sibling
            } else {
                mutation.previous_sibling_without_whitespace
            };
            match previous_sibling {
                None => 0,
                Some(previous_sibling) => {
                    let Some(previous_node_id) = self.existing_frontend_node_id_for_handle(
                        inspector_session_id,
                        document_id,
                        previous_sibling,
                    ) else {
                        return self.resynchronize_child_nodes_for_session(
                            inspector_session_id,
                            mutation.target,
                            parent_node_id,
                            include_whitespace,
                            events,
                            "previous sibling has no frontend binding",
                        );
                    };
                    previous_node_id
                }
            }
        } else {
            0
        };
        for added in &mutation.added_nodes {
            if !include_whitespace
                && crate::runtime::page_dom::inspector_whitespace_text_snapshot(&added.snapshot)
            {
                continue;
            }
            child_count = child_count.saturating_add(1);
            self.dom_agent_state.cache_child_count(
                inspector_session_id,
                document_id,
                parent_backend_node_id,
                child_count,
            );
            if children_requested {
                let mut snapshot = added.snapshot.clone();
                self.assign_renderer_dom_agent_ids_to_snapshot(
                    inspector_session_id,
                    &mut snapshot,
                    include_whitespace,
                );
                let Some(inserted_node_id) = snapshot.frontend_node_id else {
                    return self.resynchronize_child_nodes_for_session(
                        inspector_session_id,
                        mutation.target,
                        parent_node_id,
                        include_whitespace,
                        events,
                        "inserted node has no frontend binding",
                    );
                };
                events.push(RendererDomMutationEvent::ChildNodeInserted {
                    parent_node_id,
                    previous_node_id,
                    node: Box::new(snapshot),
                });
                previous_node_id = inserted_node_id;
            } else {
                events.push(RendererDomMutationEvent::ChildNodeCountUpdated {
                    node_id: parent_node_id,
                    child_node_count: child_count,
                });
            }
        }
        false
    }

    fn remove_finally_detached_bindings_for_resynchronized_parent(
        &mut self,
        inspector_session_id: Option<&str>,
        document_id: Option<crate::frame_owner_model::DocumentId>,
        mutation: &DevToolsDomChildListMutationFact,
    ) {
        for removed_subtree in &mutation.removed_subtrees {
            let Some(removed_root) = removed_subtree.handles.first().copied() else {
                continue;
            };
            if self
                .vm()
                .document_runtime
                .dom_host()
                .parent_node(removed_root)
                == Some(mutation.target)
            {
                continue;
            }
            let backend_node_ids = removed_subtree
                .handles
                .iter()
                .filter_map(|handle| self.renderer_backend_node_id_for_live_handle(*handle))
                .collect::<Vec<_>>();
            self.dom_agent_state
                .remove_frontend_bindings_for_backend_node_ids(
                    inspector_session_id,
                    document_id,
                    backend_node_ids,
                );
        }
    }

    fn resynchronize_child_nodes_for_session(
        &mut self,
        inspector_session_id: Option<&str>,
        parent: crate::dom::native::NativeNodeId,
        parent_node_id: u32,
        include_whitespace: bool,
        events: &mut Vec<RendererDomMutationEvent>,
        reason: &'static str,
    ) -> bool {
        let child_handles = {
            let dom_host = self.vm().document_runtime.dom_host();
            if dom_host.node(parent).is_none() {
                tracing::warn!(
                    ?parent,
                    reason,
                    "could not resynchronize DOM child bindings"
                );
                return false;
            }
            dom_host
                .child_handles(parent)
                .filter(|&handle| {
                    include_whitespace
                        || !crate::runtime::page_dom::inspector_whitespace_text_node(
                            dom_host, handle,
                        )
                })
                .collect::<Vec<_>>()
        };
        let mut nodes = Vec::with_capacity(child_handles.len());
        for child in child_handles {
            let Some(mut snapshot) =
                crate::runtime::page_dom::live_inspector_document_node_snapshot(
                    self.vm().document_runtime.dom_host(),
                    child,
                    0,
                    Some(parent),
                    false,
                    include_whitespace,
                )
            else {
                tracing::warn!(
                    ?parent,
                    ?child,
                    reason,
                    "could not snapshot DOM child for resynchronization"
                );
                return false;
            };
            self.assign_renderer_dom_agent_ids_to_snapshot(
                inspector_session_id,
                &mut snapshot,
                include_whitespace,
            );
            if snapshot.frontend_node_id.is_none() {
                tracing::warn!(
                    ?parent,
                    ?child,
                    reason,
                    "could not bind DOM child for resynchronization"
                );
                return false;
            }
            nodes.push(snapshot);
        }
        let document_id = self.current_dom_agent_document_id();
        let Some(parent_backend_node_id) = self.renderer_backend_node_id_for_live_handle(parent)
        else {
            tracing::warn!(
                ?parent,
                reason,
                "could not bind DOM parent for resynchronization"
            );
            return false;
        };
        self.dom_agent_state.cache_child_count(
            inspector_session_id,
            document_id,
            parent_backend_node_id,
            nodes.len(),
        );
        events.push(RendererDomMutationEvent::SetChildNodes {
            parent_node_id,
            nodes,
        });
        true
    }

    fn existing_frontend_node_id_for_handle(
        &mut self,
        inspector_session_id: Option<&str>,
        document_id: Option<crate::frame_owner_model::DocumentId>,
        handle: crate::dom::native::NativeNodeId,
    ) -> Option<u32> {
        let backend_node_id = self.renderer_backend_node_id_for_live_handle(handle)?;
        self.dom_agent_state
            .frontend_node_id_for_existing_backend_node_id(
                inspector_session_id,
                document_id,
                backend_node_id,
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_previous_sibling_binding_resynchronizes_children_without_zero_ids() {
        let mut page_vm = super::super::tests::test_page_vm();
        page_vm
            .vm_mut()
            .eval(
                "document.body.innerHTML = '<div id=parent><i id=previous></i><i id=anchor></i><i id=doomed></i></div>'",
            )
            .expect("fixture DOM should evaluate");

        page_vm
            .document_node_snapshot_for_document(Some("session"), -1, false)
            .expect("document snapshot should bind the complete tree");
        let parent = page_vm
            .vm()
            .element_handle_by_id_for_test("parent")
            .expect("parent handle");
        let previous = page_vm
            .vm()
            .element_handle_by_id_for_test("previous")
            .expect("previous sibling handle");
        let doomed = page_vm
            .vm()
            .element_handle_by_id_for_test("doomed")
            .expect("later removed sibling handle");
        let document_id = page_vm.current_dom_agent_document_id();
        let parent_backend_node_id = page_vm
            .renderer_backend_node_id_for_live_handle(parent)
            .expect("parent backend node id");
        let parent_node_id = page_vm
            .dom_agent_state
            .frontend_node_id_for_existing_backend_node_id(
                Some("session"),
                document_id,
                parent_backend_node_id,
            )
            .expect("parent frontend node id");
        let previous_backend_node_id = page_vm
            .renderer_backend_node_id_for_live_handle(previous)
            .expect("previous sibling backend node id");
        let doomed_backend_node_id = page_vm
            .renderer_backend_node_id_for_live_handle(doomed)
            .expect("later removed sibling backend node id");
        page_vm
            .dom_agent_state
            .remove_frontend_bindings_for_backend_node_ids(
                Some("session"),
                document_id,
                [previous_backend_node_id],
            );

        page_vm
            .vm_mut()
            .eval(
                "document.querySelector('#anchor').before(Object.assign(document.createElement('span'), { id: 'inserted' })); document.querySelector('#doomed').remove()",
            )
            .expect("insertion should evaluate");
        let batches = page_vm.take_pending_dom_mutation_event_batches();
        let batch = batches
            .iter()
            .find(|batch| batch.session.wire_session_id() == Some("session"))
            .expect("session mutation batch");
        assert!(
            !batch.events.iter().any(|event| matches!(
                event,
                RendererDomMutationEvent::ChildNodeInserted {
                    previous_node_id: 0,
                    ..
                }
            )),
            "an unbound non-first sibling must never be projected as previousNodeId 0"
        );
        let (resynchronized_parent_id, nodes) = batch
            .events
            .iter()
            .find_map(|event| match event {
                RendererDomMutationEvent::SetChildNodes {
                    parent_node_id,
                    nodes,
                } => Some((*parent_node_id, nodes)),
                _ => None,
            })
            .expect("binding mismatch should produce an authoritative child snapshot");
        assert_eq!(resynchronized_parent_id, parent_node_id);
        assert_eq!(
            nodes
                .iter()
                .map(|node| node.local_name.as_str())
                .collect::<Vec<_>>(),
            ["i", "span", "i"]
        );
        assert!(nodes.iter().all(|node| {
            node.frontend_node_id.is_some_and(|node_id| node_id != 0)
                && node.backend_node_id.is_some_and(|node_id| node_id != 0)
        }));
        assert_eq!(
            page_vm
                .dom_agent_state
                .frontend_node_id_for_existing_backend_node_id(
                    Some("session"),
                    document_id,
                    doomed_backend_node_id,
                ),
            None,
            "later skipped facts must still discard bindings for finally detached nodes"
        );
    }
}
