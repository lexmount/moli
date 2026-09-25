use super::{DocumentRuntime, DomHandle, Node};
use std::collections::VecDeque;

#[derive(Debug, Default)]
pub(super) struct DocumentAutofocusState {
    processed: bool,
    candidates: VecDeque<DomHandle>,
}

impl DocumentRuntime {
    /// Only active top-level Documents own candidate lists. Inert Documents
    /// and their synthetic Windows must never acquire an autofocus residence.
    pub(crate) fn register_autofocus_document(&mut self, document: DomHandle) {
        self.document_autofocus.entry(document).or_default();
    }

    pub(crate) fn retire_autofocus_document(&mut self, document: DomHandle) {
        self.document_autofocus.remove(&document);
    }

    pub(crate) fn autofocus_processed(&self, document: DomHandle) -> bool {
        self.document_autofocus
            .get(&document)
            .is_none_or(|state| state.processed)
    }

    pub(crate) fn next_autofocus_candidate(&self, document: DomHandle) -> Option<DomHandle> {
        self.document_autofocus
            .get(&document)
            .and_then(|state| state.candidates.front().copied())
    }

    pub(crate) fn pop_autofocus_candidate(&mut self, document: DomHandle) -> Option<DomHandle> {
        self.document_autofocus
            .get_mut(&document)
            .and_then(|state| state.candidates.pop_front())
    }

    pub(crate) fn clear_autofocus_candidates(&mut self, document: DomHandle) {
        if let Some(state) = self.document_autofocus.get_mut(&document) {
            state.candidates.clear();
        }
    }

    /// The caller resolves the active top-level Document and the inserted
    /// subtree's policy. All its descendant Documents share connection order
    /// and a one-time decision, even when a candidate moves between them.
    pub(crate) fn queue_autofocus_candidates_in_subtree(
        &mut self,
        top_document: DomHandle,
        root: DomHandle,
    ) -> bool {
        let Some(state) = self
            .document_autofocus
            .get_mut(&top_document)
            .filter(|state| !state.processed)
        else {
            return false;
        };
        let mut pending = vec![root];
        let mut changed = false;
        while let Some(handle) = pending.pop() {
            if self.dom_host.is_connected(handle)
                && self
                    .dom_host
                    .node(handle)
                    .and_then(Node::as_element)
                    .is_some_and(|element| {
                        matches!(
                            element.namespace(),
                            crate::native_bridge::document::XHTML_NS
                                | crate::native_bridge::document::SVG_NS
                                | crate::native_bridge::document::MATHML_NS
                        ) && element.has_attribute("autofocus")
                    })
            {
                state.candidates.retain(|candidate| *candidate != handle);
                state.candidates.push_back(handle);
                changed = true;
            }
            pending.extend(self.dom_host.child_handles_reversed(handle));
        }
        changed
    }

    pub(crate) fn mark_autofocus_processed(&mut self, document: DomHandle) {
        if let Some(state) = self.document_autofocus.get_mut(&document) {
            state.processed = true;
            state.candidates.clear();
        }
    }
}
