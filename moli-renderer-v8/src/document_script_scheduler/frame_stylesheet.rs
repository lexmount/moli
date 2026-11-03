use std::collections::{BTreeMap, HashMap};

use crate::{
    dom::NodeId,
    frame_owner_model::{DocumentLoadDelayTokenId, FrameDocumentOwner},
    stylesheet_blocking::{
        DocumentBlockingStylesheetSignature, DocumentOwnedStylesheetDiscoveryInput,
    },
};

pub(crate) enum FrameDocumentStylesheetAdmission {
    Start(DocumentLoadDelayTokenId),
    Joined,
}

pub(crate) struct FrameDocumentStylesheetCompletion {
    pub(crate) node_ids: Vec<NodeId>,
    pub(crate) import_roots: Vec<crate::document_runtime::ConnectedStyleImportRoot>,
    pub(crate) load_delay_token: DocumentLoadDelayTokenId,
}

#[derive(Debug)]
struct FrameDocumentStylesheetEntry {
    node_ids: HashMap<NodeId, bool>,
    import_roots: Vec<crate::document_runtime::ConnectedStyleImportRoot>,
    load_delay_token: DocumentLoadDelayTokenId,
}

impl FrameDocumentStylesheetEntry {
    fn blocks_scripts(&self) -> bool {
        self.node_ids.values().any(|blocking| *blocking)
    }
}

#[derive(Debug, Default)]
pub(crate) struct FrameDocumentStylesheetStore {
    // Only in-flight requests are shared here. Later owners enter Fetch again,
    // leaving response reuse, revalidation and failure retries to its cache.
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
                "child parser stylesheet discovery joined existing document-owned readiness"
            );
            return FrameDocumentStylesheetAdmission::Joined;
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
                load_delay_token,
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
        FrameDocumentStylesheetAdmission::Start(load_delay_token)
    }

    pub(crate) fn take_completion(
        &mut self,
        owner: FrameDocumentOwner,
        signature: &DocumentBlockingStylesheetSignature,
        load_delay_token: DocumentLoadDelayTokenId,
    ) -> Option<FrameDocumentStylesheetCompletion> {
        let entries = self.documents.get_mut(&owner)?;
        // A signature can be used again after completion. The original load's
        // token prevents a delayed duplicate from consuming the next request.
        if entries.get(signature)?.load_delay_token != load_delay_token {
            return None;
        }
        let entry = entries.remove(signature)?;
        if entries.is_empty() {
            self.documents.remove(&owner);
        }
        tracing::debug!(
            owner = ?owner,
            signature = ?signature,
            ?load_delay_token,
            node_count = entry.node_ids.len(),
            "applied child parser stylesheet terminal to document-owned readiness"
        );
        Some(FrameDocumentStylesheetCompletion {
            node_ids: entry.node_ids.into_keys().collect(),
            import_roots: entry.import_roots,
            load_delay_token,
        })
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
            FrameDocumentStylesheetAdmission::Start(acquired) if acquired == token
        ));
        assert!(!store.blocks_signatures(owner, [input.signature()]));
        assert!(!store.has_pending(owner));
        assert_eq!(
            store
                .take_completion(owner, input.signature(), token)
                .map(|done| done.load_delay_token),
            Some(token)
        );
        assert!(
            store
                .take_completion(owner, input.signature(), token)
                .is_none()
        );
        assert_eq!(store.remove_document(owner), 0);
    }

    #[test]
    fn child_stylesheet_readiness_is_owner_scoped_and_failure_unblocks() {
        let owner = owner();
        let mut store = FrameDocumentStylesheetStore::default();
        let input = input(3, "https://styles.test/blocked.css");

        let load_delay_token = DocumentLoadDelayTokenId(7);
        assert!(matches!(
            store.discover(owner, &input, None, || Some(load_delay_token)),
            FrameDocumentStylesheetAdmission::Start(acquired) if acquired == load_delay_token
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
            store
                .take_completion(owner, input.signature(), load_delay_token)
                .map(|done| done.load_delay_token),
            Some(load_delay_token)
        );
        assert!(!store.blocks_signatures(owner, [input.signature()]));
        assert!(
            store
                .take_completion(owner, input.signature(), load_delay_token)
                .is_none()
        );
        assert_eq!(store.remove_document(owner), 0);
        assert!(!store.has_pending(owner));

        let replacement_owner =
            FrameDocumentOwner::new(owner.local_window_id, DocumentId(owner.document_id.0 + 1));
        assert!(matches!(
            store.discover(replacement_owner, &input, None, || {
                Some(DocumentLoadDelayTokenId(8))
            }),
            FrameDocumentStylesheetAdmission::Start(_)
        ));
        assert!(
            store
                .take_completion(owner, input.signature(), load_delay_token)
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
            FrameDocumentStylesheetAdmission::Start(acquired) if acquired == token
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
        let done = store
            .take_completion(owner, blocking.signature(), token)
            .unwrap();
        assert_eq!(done.node_ids, vec![nonblocking.node_id()]);
        assert_eq!(
            done.load_delay_token, token,
            "revoking an owner must not lose the shared request's load-delay token"
        );
    }

    #[test]
    fn completed_request_is_removed_and_cannot_settle_a_later_fetch() {
        let owner = owner();
        let mut store = FrameDocumentStylesheetStore::default();
        let first = input(3, "https://styles.test/shared.css");
        let later = input(4, "https://styles.test/shared.css");
        let first_token = DocumentLoadDelayTokenId(7);
        let later_token = DocumentLoadDelayTokenId(8);
        assert!(matches!(
            store.discover(owner, &first, None, || Some(first_token)),
            FrameDocumentStylesheetAdmission::Start(token) if token == first_token
        ));
        let done = store
            .take_completion(owner, first.signature(), first_token)
            .unwrap();
        assert_eq!(done.node_ids, vec![first.node_id()]);
        assert!(
            store.documents.is_empty(),
            "completed responses must not remain cached in the stylesheet store"
        );
        assert!(matches!(
            store.discover(owner, &later, None, || Some(later_token)),
            FrameDocumentStylesheetAdmission::Start(token) if token == later_token
        ));
        assert!(
            store
                .take_completion(owner, first.signature(), first_token)
                .is_none()
        );
        assert!(store.blocks_signatures(owner, [later.signature()]));
        let done = store
            .take_completion(owner, later.signature(), later_token)
            .unwrap();
        assert_eq!(done.load_delay_token, later_token);
        assert_eq!(done.node_ids, vec![later.node_id()]);
        assert!(!store.has_pending(owner));
    }
}
