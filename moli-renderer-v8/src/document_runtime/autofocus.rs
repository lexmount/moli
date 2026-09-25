use super::{DocumentRuntime, DomHandle, Node};

#[derive(Debug, Default)]
pub(super) struct DocumentAutofocusState {
    processed: bool,
    candidates: Vec<DomHandle>,
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

    pub(crate) fn autofocus_candidates(&self, document: DomHandle) -> &[DomHandle] {
        self.document_autofocus
            .get(&document)
            .map_or(&[], |state| state.candidates.as_slice())
    }

    pub(crate) fn take_autofocus_candidates(&mut self, document: DomHandle) -> Vec<DomHandle> {
        self.document_autofocus
            .get_mut(&document)
            .map(|state| std::mem::take(&mut state.candidates))
            .unwrap_or_default()
    }

    /// Return the Documents whose insertion-ordered candidate lists changed.
    /// Registration is explicit, so connectedness alone cannot admit an inert
    /// Document or reset an existing Document's one-time decision.
    pub(crate) fn queue_autofocus_candidates_in_subtrees(
        &mut self,
        roots: &[DomHandle],
    ) -> Vec<DomHandle> {
        if roots
            .first()
            .and_then(|root| self.dom_host.owner_document_handle(*root))
            .is_none_or(|document| self.autofocus_processed(document))
        {
            return Vec::new();
        }
        let mut pending = roots.iter().rev().copied().collect::<Vec<_>>();
        let mut documents = Vec::new();
        while let Some(handle) = pending.pop() {
            if self.dom_host.is_connected(handle)
                && let Some(document) = self.dom_host.owner_document_handle(handle)
                && let Some(state) = self.document_autofocus.get_mut(&document)
                && !state.processed
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
                state.candidates.push(handle);
                if !documents.contains(&document) {
                    documents.push(document);
                }
            }
            pending.extend(self.dom_host.child_handles_reversed(handle));
        }
        documents
    }

    pub(crate) fn mark_autofocus_processed(&mut self, document: DomHandle) {
        if let Some(state) = self.document_autofocus.get_mut(&document) {
            state.processed = true;
            state.candidates.clear();
        }
    }
}
