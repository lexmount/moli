use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use crate::{
    dom::NodeId,
    frame_owner_model::{DocumentLoadDelayTokenId, FrameDocumentOwner},
    stylesheet_blocking::{
        DocumentBlockingStylesheetSignature, DocumentOwnedStylesheetDiscoveryInput,
    },
    types::ChildBlockingStylesheetLoadCompletion,
};

#[derive(Debug)]
enum FrameDocumentStylesheetStatus {
    Pending,
    // Retain the physical terminal separately from per-owner installation.
    // None represents admission without an available resource loader.
    Completed(Option<Arc<ChildBlockingStylesheetLoadCompletion>>),
}

pub(crate) enum FrameDocumentStylesheetAdmission {
    Start,
    Joined,
    Completed(Arc<ChildBlockingStylesheetLoadCompletion>),
}

#[derive(Debug)]
struct FrameDocumentStylesheetEntry {
    node_ids: HashMap<NodeId, bool>,
    import_roots: Vec<crate::document_runtime::ConnectedStyleImportRoot>,
    status: FrameDocumentStylesheetStatus,
    load_delay_token: Option<DocumentLoadDelayTokenId>,
}

impl FrameDocumentStylesheetEntry {
    fn blocks_scripts(&self) -> bool {
        matches!(self.status, FrameDocumentStylesheetStatus::Pending)
            && self.node_ids.values().any(|blocking| *blocking)
    }
}

#[derive(Debug, Default)]
pub(crate) struct FrameDocumentStylesheetStore {
    documents: BTreeMap<
        FrameDocumentOwner,
        HashMap<DocumentBlockingStylesheetSignature, FrameDocumentStylesheetEntry>,
    >,
}

impl FrameDocumentStylesheetStore {
    pub(crate) fn discover(
        &mut self,
        owner: FrameDocumentOwner,
        input: &DocumentOwnedStylesheetDiscoveryInput,
        import_root: Option<crate::document_runtime::ConnectedStyleImportRoot>,
        acquire_load_delay: impl FnOnce() -> Option<DocumentLoadDelayTokenId>,
    ) -> FrameDocumentStylesheetAdmission {
        let signature = input.signature().clone();
        let entries = self.documents.entry(owner).or_default();
        if let Some(entry) = entries.get_mut(&signature) {
            let added = entry
                .node_ids
                .insert(input.node_id(), input.blocks_scripts())
                .is_none();
            if added {
                entry.import_roots.extend(import_root);
            }
            tracing::debug!(
                owner = ?owner,
                node_id = ?input.node_id(),
                signature = ?signature,
                pending = matches!(entry.status, FrameDocumentStylesheetStatus::Pending),
                "child parser stylesheet discovery joined existing document-owned readiness"
            );
            return match &entry.status {
                FrameDocumentStylesheetStatus::Completed(Some(completion)) if added => {
                    FrameDocumentStylesheetAdmission::Completed(Arc::clone(completion))
                }
                _ => FrameDocumentStylesheetAdmission::Joined,
            };
        }
        let Some(load_delay_token) = acquire_load_delay() else {
            tracing::warn!(
                owner = ?owner,
                node_id = ?input.node_id(),
                signature = ?signature,
                "rejecting child parser stylesheet without a document load-delay token"
            );
            return FrameDocumentStylesheetAdmission::Joined;
        };
        entries.insert(
            signature.clone(),
            FrameDocumentStylesheetEntry {
                node_ids: HashMap::from([(input.node_id(), input.blocks_scripts())]),
                import_roots: import_root.into_iter().collect(),
                status: FrameDocumentStylesheetStatus::Pending,
                load_delay_token: Some(load_delay_token),
            },
        );
        tracing::debug!(
            owner = ?owner,
            node_id = ?input.node_id(),
            signature = ?signature,
            ?load_delay_token,
            pending_count = entries
                .values()
                .filter(|entry| entry.blocks_scripts())
                .count(),
            "accepted child parser stylesheet for document owner"
        );
        FrameDocumentStylesheetAdmission::Start
    }

    pub(crate) fn apply_completion(
        &mut self,
        owner: FrameDocumentOwner,
        signature: &DocumentBlockingStylesheetSignature,
        completion: Option<Arc<ChildBlockingStylesheetLoadCompletion>>,
    ) -> Option<DocumentLoadDelayTokenId> {
        let entries = self.documents.get_mut(&owner)?;
        let entry = entries.get_mut(signature)?;
        if !matches!(entry.status, FrameDocumentStylesheetStatus::Pending) {
            return None;
        }
        let successful = completion
            .as_ref()
            .is_some_and(|result| result.successful());
        entry.status = FrameDocumentStylesheetStatus::Completed(completion);
        let load_delay_token = entry.load_delay_token.take()?;
        tracing::debug!(
            owner = ?owner,
            signature = ?signature,
            successful,
            ?load_delay_token,
            node_count = entry.node_ids.len(),
            "applied child parser stylesheet terminal to document-owned readiness"
        );
        Some(load_delay_token)
    }

    pub(crate) fn blocks_signatures<'a>(
        &self,
        owner: FrameDocumentOwner,
        signatures: impl IntoIterator<Item = &'a DocumentBlockingStylesheetSignature>,
    ) -> bool {
        let Some(entries) = self.documents.get(&owner) else {
            return false;
        };
        signatures.into_iter().any(|signature| {
            entries
                .get(signature)
                .is_some_and(|entry| entry.blocks_scripts())
        })
    }

    pub(crate) fn has_pending(&self, owner: FrameDocumentOwner) -> bool {
        self.documents
            .get(&owner)
            .is_some_and(|entries| entries.values().any(|entry| entry.blocks_scripts()))
    }

    pub(crate) fn import_roots_for_signature(
        &self,
        owner: FrameDocumentOwner,
        signature: &DocumentBlockingStylesheetSignature,
    ) -> Vec<crate::document_runtime::ConnectedStyleImportRoot> {
        self.documents
            .get(&owner)
            .and_then(|entries| entries.get(signature))
            .map(|entry| entry.import_roots.clone())
            .unwrap_or_default()
    }

    pub(crate) fn node_ids_for_signature(
        &self,
        owner: FrameDocumentOwner,
        signature: &DocumentBlockingStylesheetSignature,
    ) -> Vec<NodeId> {
        self.documents
            .get(&owner)
            .and_then(|entries| entries.get(signature))
            .map(|entry| entry.node_ids.keys().copied().collect())
            .unwrap_or_default()
    }

    /// Revoke this processing's install and script-blocking authority without
    /// discarding the shared resource or the token its completion must settle.
    pub(crate) fn invalidate_owner(&mut self, node_id: NodeId) {
        for entries in self.documents.values_mut() {
            for entry in entries.values_mut() {
                entry.node_ids.remove(&node_id);
                entry
                    .import_roots
                    .retain(|root| root.owner.index() != node_id.index());
            }
        }
    }

    pub(crate) fn remove_document(&mut self, owner: FrameDocumentOwner) -> usize {
        let removed = self
            .documents
            .remove(&owner)
            .map_or(0, |entries| entries.len());
        if removed != 0 {
            tracing::debug!(
                owner = ?owner,
                removed,
                "retired child parser stylesheet readiness with document owner"
            );
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_owner_model::{DocumentId, LocalWindowId};
    use moli_stylesheet_blocking::{
        DocumentOwnedBlockingStylesheet, DocumentOwnedBlockingStylesheetCandidate,
        StylesheetFetchOptions,
    };
    use url::Url;

    fn owner() -> FrameDocumentOwner {
        FrameDocumentOwner::new(LocalWindowId(1), DocumentId(2))
    }

    fn input(node_id: usize, url: &str) -> DocumentOwnedStylesheetDiscoveryInput {
        let blocker = DocumentOwnedBlockingStylesheet::from_candidate(
            &DocumentOwnedBlockingStylesheetCandidate::Link {
                node_id: NodeId::new(node_id),
                url: Url::parse(url).expect("stylesheet url"),
                options: StylesheetFetchOptions::default(),
            },
        );
        DocumentOwnedStylesheetDiscoveryInput::from(&blocker)
    }

    #[test]
    fn nonmatching_child_stylesheet_owns_a_load_token_without_blocking_scripts() {
        let stream = crate::parser::HtmlParser::SCRIPTING_ENABLED
            .start_document(Url::parse("https://styles.test/").unwrap());
        let outcome = stream.pump_parser_step(
            "<!doctype html><link rel=stylesheet href='/print.css' media=print><script>0</script>",
        );
        let input = outcome
            .discovered_stylesheet_inputs
            .into_iter()
            .next()
            .expect("print CSS must still be discovered for loading");
        assert!(!input.blocks_scripts());
        let owner = owner();
        let mut store = FrameDocumentStylesheetStore::default();
        let token = DocumentLoadDelayTokenId(7);
        assert!(matches!(
            store.discover(owner, &input, None, || Some(token)),
            FrameDocumentStylesheetAdmission::Start
        ));
        assert!(!store.blocks_signatures(owner, [input.signature()]));
        assert!(!store.has_pending(owner));
        assert_eq!(
            store.apply_completion(owner, input.signature(), None),
            Some(token)
        );
        assert_eq!(store.apply_completion(owner, input.signature(), None), None);
        assert_eq!(store.remove_document(owner), 1);
    }

    #[test]
    fn child_stylesheet_readiness_is_owner_scoped_and_failure_unblocks() {
        let owner = owner();
        let mut store = FrameDocumentStylesheetStore::default();
        let input = input(3, "https://styles.test/blocked.css");

        let load_delay_token = DocumentLoadDelayTokenId(7);
        assert!(matches!(
            store.discover(owner, &input, None, || Some(load_delay_token)),
            FrameDocumentStylesheetAdmission::Start
        ));
        assert!(
            matches!(
                store.discover(owner, &input, None, || {
                    panic!("joined stylesheet discovery must not acquire another load-delay token")
                }),
                FrameDocumentStylesheetAdmission::Joined
            ),
            "duplicate stylesheet discovery should join the original owner entry"
        );
        assert!(store.blocks_signatures(owner, [input.signature()]));
        assert_eq!(
            store.apply_completion(owner, input.signature(), None),
            Some(load_delay_token)
        );
        assert!(!store.blocks_signatures(owner, [input.signature()]));
        assert_eq!(store.apply_completion(owner, input.signature(), None), None);
        assert_eq!(store.remove_document(owner), 1);
        assert!(!store.has_pending(owner));

        let replacement_owner =
            FrameDocumentOwner::new(owner.local_window_id, DocumentId(owner.document_id.0 + 1));
        assert!(matches!(
            store.discover(replacement_owner, &input, None, || {
                Some(DocumentLoadDelayTokenId(8))
            }),
            FrameDocumentStylesheetAdmission::Start
        ));
        assert!(
            store
                .apply_completion(owner, input.signature(), None)
                .is_none(),
            "retired owner completion must not update replacement readiness"
        );
        assert!(store.blocks_signatures(replacement_owner, [input.signature()]));
    }

    #[test]
    fn invalidated_child_owner_does_not_own_the_shared_terminal_or_block_scripts() {
        let owner = owner();
        let mut store = FrameDocumentStylesheetStore::default();
        let stream = crate::parser::HtmlParser::SCRIPTING_ENABLED
            .start_document(Url::parse("https://styles.test/").unwrap());
        let outcome = stream.pump_parser_step(
            "<!doctype html><link rel=stylesheet href='/shared.css' media=print><script>0</script>",
        );
        let nonblocking = outcome
            .discovered_stylesheet_inputs
            .into_iter()
            .next()
            .unwrap()
            .with_node_id(NodeId::new(30));
        let blocking = input(31, "https://styles.test/shared.css");
        let token = DocumentLoadDelayTokenId(7);
        assert!(matches!(
            store.discover(owner, &nonblocking, None, || Some(token)),
            FrameDocumentStylesheetAdmission::Start
        ));
        assert!(matches!(
            store.discover(owner, &blocking, None, || panic!("shared fetch")),
            FrameDocumentStylesheetAdmission::Joined
        ));
        assert!(store.has_pending(owner));
        store.invalidate_owner(blocking.node_id());
        assert!(
            !store.has_pending(owner),
            "only current subscribers can block scripts"
        );
        assert_eq!(
            store.node_ids_for_signature(owner, blocking.signature()),
            vec![nonblocking.node_id()]
        );
        assert_eq!(
            store.apply_completion(owner, blocking.signature(), None),
            Some(token),
            "revoking an owner must not lose the shared request's load-delay token"
        );
    }
}
