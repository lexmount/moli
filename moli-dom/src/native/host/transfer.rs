use super::*;

/// Native identities to rebind after moving an ownership graph between hosts.
/// V8 reflectors, callbacks, observers, resources and registry state belong to
/// the renderer and must migrate separately before exposing the moved graph.
#[derive(Debug, PartialEq, Eq)]
pub struct DomSubtreeTransfer {
    root: DomHandle,
    handles: HashMap<DomHandle, DomHandle>,
}

impl DomSubtreeTransfer {
    pub fn root(&self) -> DomHandle {
        self.root
    }

    pub fn handles(&self) -> &HashMap<DomHandle, DomHandle> {
        &self.handles
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomSubtreeTransferError {
    InvalidTargetDocument(DomHandle),
    InvalidSource(DomHandle),
    AttachedSource(DomHandle),
    InvalidTree(DomHandle),
    OpenReference {
        node: DomHandle,
        referenced: DomHandle,
    },
    HandleCapacity,
}

#[derive(Clone, Copy)]
enum TransferDocument {
    Target,
    TemplateContents,
}

struct TransferPlan {
    nodes: Vec<(DomHandle, TransferDocument)>,
    membership: HashSet<DomHandle>,
}

impl DomHost {
    /// Move a detached, closed native ownership graph without DOM clone steps.
    ///
    /// Includes all shadow roots (also closed/non-clonable), template contents,
    /// manual slot assignments and native element state. Old source slots are
    /// retired, never reused; snapshots keep their immutable source view.
    ///
    /// This is the native stage of adoption, not a Web API entrypoint. The
    /// caller must first perform removal and its lifecycle/observer work, and
    /// then rebind renderer identities using the returned map. References that
    /// cross the graph boundary currently require a wider ownership service;
    /// they are rejected before either host changes instead of being dropped
    /// or accidentally rebound to a colliding host-local handle.
    pub fn transfer_detached_subtree_from(
        &mut self,
        document: DomHandle,
        source: &mut Self,
        root: DomHandle,
    ) -> Result<DomSubtreeTransfer, DomSubtreeTransferError> {
        if !self.node(document).is_some_and(Node::is_document) {
            return Err(DomSubtreeTransferError::InvalidTargetDocument(document));
        }
        let plan = source.preflight_subtree_transfer(root)?;
        let needs_template_document = plan
            .nodes
            .iter()
            .any(|(_, owner)| matches!(owner, TransferDocument::TemplateContents));
        let capacity = self
            .dom
            .len()
            .checked_add(plan.nodes.len())
            .and_then(|count| {
                count.checked_add(usize::from(
                    needs_template_document
                        && !self.dom.inert_template_documents.contains_key(&document),
                ))
            });
        if capacity.is_none_or(|count| count > u32::MAX as usize) {
            return Err(DomSubtreeTransferError::HandleCapacity);
        }
        // Every fallible check precedes destination allocation or source removal.
        let template_document = if needs_template_document {
            self.dom
                .appropriate_template_contents_owner_document(document)
        } else {
            document
        };
        let first_slot = self.dom.len();
        let handles: HashMap<_, _> = plan
            .nodes
            .iter()
            .enumerate()
            .map(|(offset, (old, _))| (*old, DomHandle::new(first_slot + offset)))
            .collect();
        for (old, owner) in &plan.nodes {
            let mut node = source
                .dom
                .nodes
                .take(old.index())
                .expect("preflight retained every source node");
            let document = match owner {
                TransferDocument::Target => document,
                TransferDocument::TemplateContents => template_document,
            };
            node.remap_for_transfer(handles[old], document, &handles);
            self.dom.nodes.push(node);
        }
        self.transfer_shadow_bindings_from(source, &handles);
        self.dom
            .transfer_stylesheet_candidate_scopes_from(&mut source.dom, &handles);
        for old in &plan.membership {
            if let Some(assigned) = source.manual_slot_assignments.borrow_mut().remove(old) {
                let assigned = assigned.into_iter().map(|node| handles[&node]).collect();
                self.manual_slot_assignments
                    .borrow_mut()
                    .insert(handles[old], assigned);
            }
        }
        source
            .child_browsing_context_host_candidates
            .borrow_mut()
            .retain(|handle| !plan.membership.contains(handle));
        for (old, _) in &plan.nodes {
            let new = handles[old];
            if self
                .node(new)
                .and_then(Node::as_element)
                .is_some_and(|element| {
                    is_html_frame_owner_candidate(element.local_name(), element.namespace())
                })
            {
                self.child_browsing_context_host_candidates
                    .borrow_mut()
                    .push(new);
            }
        }
        if source
            .active_element
            .get()
            .is_some_and(|node| plan.membership.contains(&node))
        {
            source.active_element.set(None);
        }
        if source
            .focus_transition_common_ancestor
            .get()
            .is_some_and(|node| plan.membership.contains(&node))
        {
            source.focus_transition_common_ancestor.set(None);
        }
        source
            .hovered_elements
            .borrow_mut()
            .retain(|node| !plan.membership.contains(node));
        source.invalidate_indexes_after_transfer();
        self.invalidate_indexes_after_transfer();
        Ok(DomSubtreeTransfer {
            root: handles[&root],
            handles,
        })
    }

    fn preflight_subtree_transfer(
        &self,
        root: DomHandle,
    ) -> Result<TransferPlan, DomSubtreeTransferError> {
        let node = self
            .node(root)
            .ok_or(DomSubtreeTransferError::InvalidSource(root))?;
        if node.is_document() || self.is_shadow_root(root) {
            return Err(DomSubtreeTransferError::InvalidSource(root));
        }
        if node.parent_node().is_some() || node.is_connected() {
            return Err(DomSubtreeTransferError::AttachedSource(root));
        }
        let mut plan = TransferPlan {
            nodes: Vec::new(),
            membership: HashSet::new(),
        };
        let mut stack = vec![(root, TransferDocument::Target)];
        while let Some((handle, owner)) = stack.pop() {
            let node = self
                .node(handle)
                .ok_or(DomSubtreeTransferError::InvalidTree(handle))?;
            if !plan.membership.insert(handle) || node.is_document() || node.is_connected() {
                return Err(DomSubtreeTransferError::InvalidTree(handle));
            }
            plan.nodes.push((handle, owner));
            let mut child = node.first_child();
            let mut siblings = HashSet::new();
            while let Some(handle) = child {
                if !siblings.insert(handle) || self.parent_node(handle) != Some(node.id()) {
                    return Err(DomSubtreeTransferError::InvalidTree(handle));
                }
                stack.push((handle, owner));
                child = self
                    .node(handle)
                    .ok_or(DomSubtreeTransferError::InvalidTree(handle))?
                    .next_sibling();
            }
            if let Some(shadow) = self.shadow_root_handle(handle) {
                stack.push((shadow, owner));
            }
            if let Some(contents) = node.as_element().and_then(Element::template_contents) {
                stack.push((contents, TransferDocument::TemplateContents));
            }
        }
        // Both outgoing and incoming host-local references must be closed.
        for node in self.dom.nodes() {
            for referenced in node.native_node_references() {
                if plan.membership.contains(&node.id()) != plan.membership.contains(&referenced) {
                    return Err(DomSubtreeTransferError::OpenReference {
                        node: node.id(),
                        referenced,
                    });
                }
            }
        }
        for (slot, assigned) in self.manual_slot_assignments.borrow().iter() {
            for &referenced in assigned {
                if plan.membership.contains(slot) != plan.membership.contains(&referenced) {
                    return Err(DomSubtreeTransferError::OpenReference {
                        node: *slot,
                        referenced,
                    });
                }
            }
        }
        for &(node, _) in &plan.nodes {
            for referenced in self
                .dom
                .stylesheet_candidate_handles_for_tree_scope(node)
                .iter()
            {
                if !plan.membership.contains(referenced) {
                    return Err(DomSubtreeTransferError::OpenReference {
                        node,
                        referenced: *referenced,
                    });
                }
            }
        }
        Ok(plan)
    }

    fn transfer_shadow_bindings_from(
        &mut self,
        source: &mut Self,
        handles: &HashMap<DomHandle, DomHandle>,
    ) {
        let mut changed = false;
        for (old, new) in handles {
            if let Some(mut state) = source.shadow_roots_by_host.borrow_mut().remove(old) {
                let old_root = state.handle;
                state.handle = handles[&old_root];
                source.shadow_hosts_by_root.borrow_mut().remove(&old_root);
                self.shadow_hosts_by_root
                    .borrow_mut()
                    .insert(state.handle, *new);
                self.shadow_roots_by_host.borrow_mut().insert(*new, state);
                changed = true;
            }
        }
        if changed {
            source.record_shadow_root_binding_mutation();
            self.record_shadow_root_binding_mutation();
        }
    }

    fn invalidate_indexes_after_transfer(&self) {
        self.id_index.borrow_mut().take();
        self.name_index.borrow_mut().take();
        *self.element_query_index.borrow_mut() = ElementQueryIndex::default();
        self.shadow_slot_name_indexes.borrow_mut().clear();
        self.record_mutation(MutationScope::QueryState);
    }
}

#[cfg(test)]
mod tests;
