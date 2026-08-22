//! Renderer-facing adapter for the canonical stylesheet blocking state.
//!
//! The state machine itself lives in `moli-stylesheet-blocking`; this
//! module coordinates it with renderer network observations and connected
//! load/error delivery without owning a second blocking state machine.

use std::sync::Arc;

use super::super::{ConnectedLoadNetworkResult, DocumentRuntime, DomHandle};
use crate::dom::NodeId;
use crate::stylesheet_blocking::{
    DocumentBlockingStylesheetSignature, DocumentOwnedBlockingStylesheetDiscoveryInput,
    StylesheetBlockingReadView, StylesheetFetch, StylesheetFetchOptions,
    collect_document_owned_blocking_stylesheets,
};
use crate::types::SubresourceResourceType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnerlessStylesheetAdmissionError {
    ContentSecurityPolicy,
}

impl DocumentRuntime {
    #[cfg(test)]
    pub(crate) fn preload_stylesheet(
        &mut self,
        request_url: url::Url,
        options: StylesheetFetchOptions,
    ) -> Option<StylesheetFetch> {
        self.preload_stylesheet_with_request_metadata(
            request_url,
            options,
            moli_fetch::RequestResourceType::CssStyleSheet,
            false,
        )
        .ok()
    }

    pub(crate) fn preload_stylesheet_with_request_metadata(
        &mut self,
        request_url: url::Url,
        options: StylesheetFetchOptions,
        request_resource_type: moli_fetch::RequestResourceType,
        link_preload: bool,
    ) -> Result<StylesheetFetch, OwnerlessStylesheetAdmissionError> {
        let (_, enforced_violation) = self
            .response_style_element_request_csp_check(
                &request_url,
                crate::content_security_policy::ContentSecurityPolicyStyleElementRequest {
                    nonce: options.nonce(),
                },
            )
            .into_violations();
        if enforced_violation.is_some() {
            // The eventual DOM client owns violation reporting and its
            // load/error event. Speculation only decides whether a physical
            // resource may start.
            return Err(OwnerlessStylesheetAdmissionError::ContentSecurityPolicy);
        }
        let document_url = self.document_url().clone();
        let fetcher = self.speculative_stylesheet_fetcher(request_resource_type, link_preload);
        Ok(self.stylesheet_lifecycle.fetches.preload_stylesheet(
            &fetcher,
            document_url,
            request_url,
            options,
        ))
    }

    #[cfg(test)]
    pub(crate) fn has_blocking_stylesheet_fetch_for_test(
        &self,
        owner: DomHandle,
        signature: &DocumentBlockingStylesheetSignature,
    ) -> bool {
        self.stylesheet_lifecycle
            .fetches
            .owner_link_fetch(NodeId::new(owner.index()), signature)
            .is_some()
    }

    pub(crate) fn note_discovered_live_blocking_stylesheets(&mut self) {
        let blockers = collect_document_owned_blocking_stylesheets(&self.dom_host);
        let document_url = self.document_url().clone();
        let fetcher = self.stylesheet_fetcher();
        self.discover_unblocked_stylesheet_inputs(
            &fetcher,
            &document_url,
            blockers
                .iter()
                .map(DocumentOwnedBlockingStylesheetDiscoveryInput::from),
        );
    }

    pub(crate) fn note_discovered_blocking_stylesheets(
        &mut self,
        document: &(impl StylesheetBlockingReadView + ?Sized),
    ) {
        let fetcher = self.stylesheet_fetcher();
        let blockers = collect_document_owned_blocking_stylesheets(document);
        let document_url = document
            .final_url_clone()
            .expect("parsed native dom must retain a document url");
        self.discover_unblocked_stylesheet_inputs(
            &fetcher,
            &document_url,
            blockers
                .iter()
                .map(DocumentOwnedBlockingStylesheetDiscoveryInput::from),
        );
    }

    pub(crate) fn note_discovered_document_owned_blocking_stylesheet_inputs<'a>(
        &mut self,
        inputs: impl IntoIterator<Item = &'a DocumentOwnedBlockingStylesheetDiscoveryInput>,
    ) -> Vec<super::StylesheetLinkClientTerminal> {
        let document_url = self.document_url().clone();
        self.note_discovered_document_owned_blocking_stylesheet_inputs_for_document_url(
            &document_url,
            inputs,
        )
    }

    pub(crate) fn note_discovered_document_owned_blocking_stylesheet_inputs_for_document_url<'a>(
        &mut self,
        document_url: &url::Url,
        inputs: impl IntoIterator<Item = &'a DocumentOwnedBlockingStylesheetDiscoveryInput>,
    ) -> Vec<super::StylesheetLinkClientTerminal> {
        let inputs = inputs.into_iter().cloned().collect::<Vec<_>>();
        let fetcher = self.stylesheet_fetcher();
        let inputs = self.discover_unblocked_stylesheet_inputs(&fetcher, document_url, inputs);
        self.bind_discovered_link_owner_operations(
            inputs
                .iter()
                .map(|input| (input.node_id(), input.signature())),
        )
    }

    fn discover_unblocked_stylesheet_inputs(
        &mut self,
        fetcher: &crate::stylesheet_blocking::RendererStylesheetFetcher,
        document_url: &url::Url,
        inputs: impl IntoIterator<Item = DocumentOwnedBlockingStylesheetDiscoveryInput>,
    ) -> Vec<DocumentOwnedBlockingStylesheetDiscoveryInput> {
        let inputs = inputs
            .into_iter()
            .filter(|input| {
                !self.stylesheet_owner_is_csp_blocked(DomHandle::new(input.node_id().index()))
            })
            .collect::<Vec<_>>();
        self.stylesheet_lifecycle.fetches.discover_from_inputs(
            fetcher,
            document_url,
            inputs.iter(),
        );
        inputs
    }

    pub(crate) async fn wait_for_script_blockers_before(
        &mut self,
        target_node_id: NodeId,
    ) -> Vec<super::StylesheetLinkClientTerminal> {
        let blockers = collect_document_owned_blocking_stylesheets(&self.dom_host);
        let inputs = blockers
            .iter()
            .map(DocumentOwnedBlockingStylesheetDiscoveryInput::from)
            .collect::<Vec<_>>();
        let completed_clients =
            self.note_discovered_document_owned_blocking_stylesheet_inputs(inputs.iter());
        loop {
            self.drain_blocking_stylesheet_completions();
            if !self
                .stylesheet_lifecycle
                .fetches
                .blocks_script(&self.dom_host, target_node_id)
            {
                return completed_clients;
            }
            if !self
                .stylesheet_lifecycle
                .fetches
                .wait_for_completion_arrival_without_timeout()
                .await
            {
                return completed_clients;
            }
        }
    }

    fn bind_discovered_link_owner_operations<'a>(
        &mut self,
        blockers: impl IntoIterator<Item = (moli_dom::NodeId, &'a DocumentBlockingStylesheetSignature)>,
    ) -> Vec<super::StylesheetLinkClientTerminal> {
        let mut completed_clients = Vec::new();
        for (node_id, signature) in blockers {
            let DocumentBlockingStylesheetSignature::Link { url, .. } = signature else {
                continue;
            };
            let Some(fetch) = self
                .stylesheet_lifecycle
                .fetches
                .owner_link_fetch(node_id, signature)
            else {
                continue;
            };
            let owner = DomHandle::new(node_id.index());
            let already_bound = self
                .stylesheet_lifecycle
                .owner_states
                .link_state(owner)
                .is_some_and(|state| state.active_load().fetch().ptr_eq(&fetch));
            if already_bound {
                continue;
            }
            let import_completion = self.initial_stylesheet_import_completion(url, &fetch);
            let load = super::StylesheetLinkClient::new(owner, url.clone(), fetch);
            self.install_stylesheet_link_state(
                owner,
                super::LinkStyleState::new(Arc::clone(&load), import_completion),
            );
            // A speculative resource may already be terminal when the parser
            // creates its first real owner. No future terminal will revisit it.
            if let Some(client) = self.promote_stylesheet_link_client_if_ready(load) {
                completed_clients.push(client);
            }
        }
        completed_clients
    }

    pub(crate) async fn wait_for_document_owned_blocking_stylesheet_signatures<'a>(
        &mut self,
        signatures: impl IntoIterator<Item = &'a DocumentBlockingStylesheetSignature>,
    ) {
        let signatures = signatures.into_iter().cloned().collect::<Vec<_>>();
        if signatures.is_empty() {
            return;
        }
        self.drain_blocking_stylesheet_completions();
        while self
            .stylesheet_lifecycle
            .fetches
            .blocks_on_signatures(signatures.iter())
        {
            let arrived = self
                .stylesheet_lifecycle
                .fetches
                .wait_for_completion_arrival_without_timeout()
                .await;
            if !arrived {
                return;
            }
            self.drain_blocking_stylesheet_completions();
        }
    }

    pub(crate) fn is_document_script_blocked_by_stylesheets(
        &mut self,
        document: &(impl StylesheetBlockingReadView + ?Sized),
        node_id: NodeId,
    ) -> bool {
        self.note_discovered_blocking_stylesheets(document);
        self.drain_blocking_stylesheet_completions();
        self.stylesheet_lifecycle
            .fetches
            .blocks_script(document, node_id)
    }

    #[cfg(test)]
    pub(crate) fn has_pending_document_owned_blocking_stylesheet_signatures<'a>(
        &mut self,
        signatures: impl IntoIterator<Item = &'a DocumentBlockingStylesheetSignature>,
    ) -> bool {
        let signatures = signatures.into_iter().cloned().collect::<Vec<_>>();
        self.drain_blocking_stylesheet_completions();
        self.stylesheet_lifecycle
            .fetches
            .blocks_on_signatures(signatures.iter())
    }

    /// Whether a parser-owned script must still wait for one of the blocking
    /// stylesheet operations captured before it.
    ///
    /// Physical stylesheet settlement and the connected `<link>` load/error
    /// task are separate browser-observable boundaries. A speculative fetch
    /// may already be terminal when the parser creates the real owner, so the
    /// posted event must remain part of this script gate even though the
    /// physical blocking state has settled.
    pub(crate) fn has_pending_parser_script_blocking_stylesheet_signatures<'a>(
        &mut self,
        signatures: impl IntoIterator<Item = &'a DocumentBlockingStylesheetSignature>,
    ) -> bool {
        let signatures = signatures
            .into_iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        if signatures.is_empty() {
            return false;
        }
        self.drain_blocking_stylesheet_completions();
        self.stylesheet_lifecycle
            .fetches
            .blocks_on_signatures(signatures.iter())
            || self.has_unsettled_parser_script_blocking_link_for_signature_set(&signatures)
    }

    fn has_unsettled_parser_script_blocking_link_for_signature_set(
        &self,
        signatures: &std::collections::HashSet<DocumentBlockingStylesheetSignature>,
    ) -> bool {
        !signatures.is_empty()
            && self
                .stylesheet_lifecycle
                .owner_states
                .link_states()
                .any(|(owner, state)| {
                    (state.is_pending() || state.posted_event_load().is_some())
                        && self
                            .stylesheet_lifecycle
                            .fetches
                            .blocking_link_signature_for_fetch(
                                NodeId::new(owner.index()),
                                state.active_load().fetch(),
                            )
                            .is_some_and(|signature| signatures.contains(signature))
                })
    }

    pub(crate) fn drain_blocking_stylesheet_completions(&mut self) {
        #[cfg(test)]
        self.apply_ready_stylesheet_networking_tasks_for_test();
        let completed_fetches = self.stylesheet_lifecycle.fetches.drain_ready_completions();
        for fetch in completed_fetches {
            self.promote_stylesheet_link_clients_for_fetch(&fetch);
        }
        self.reconcile_connected_style_imports_with_blocking_stylesheets();
    }

    pub(crate) fn apply_blocking_stylesheet_completion(
        &mut self,
        completion: crate::stylesheet_blocking::StylesheetCompletion,
    ) {
        let had_pending_blocker = self.stylesheet_lifecycle.fetches.has_any_pending_entries();
        if let Some(fetch) = self
            .stylesheet_lifecycle
            .fetches
            .apply_completion(completion)
        {
            self.promote_stylesheet_link_clients_for_fetch(&fetch);
        }
        self.reconcile_connected_style_imports_with_blocking_stylesheets();
        if had_pending_blocker && !self.stylesheet_lifecycle.fetches.has_any_pending_entries() {
            // Match Blink's task publication boundary: stylesheet settlement
            // schedules parser reevaluation even when a blocking <link> event
            // was posted by the same completion. Phase-one yields to that
            // stable event source before executing the script.
            self.request_main_parser_continuation_if_active();
        }
    }

    fn parser_blocking_stylesheet_release_is_ready(&self) -> bool {
        !self.stylesheet_lifecycle.fetches.has_any_pending_entries()
            && !self.has_unsettled_parser_blocking_link()
    }

    fn has_unsettled_parser_blocking_link(&self) -> bool {
        self.stylesheet_lifecycle
            .owner_states
            .link_states()
            .any(|(owner, state)| {
                (state.is_pending() || state.posted_event_load().is_some())
                    && self.stylesheet_lifecycle.fetches.owns_blocking_link_fetch(
                        NodeId::new(owner.index()),
                        state.active_load().fetch(),
                    )
            })
    }

    pub(crate) fn ready_connected_style_load_is_parser_blocking_link_event(
        &self,
        ready: &super::ReadyConnectedStyleLoad,
    ) -> bool {
        let super::ReadyConnectedStyleLoadOperation::StylesheetLink(load) = ready.operation()
        else {
            return false;
        };
        self.stylesheet_lifecycle
            .fetches
            .owns_blocking_link_fetch(NodeId::new(load.owner().index()), load.fetch())
    }

    pub(crate) fn release_main_parser_after_parser_blocking_link_event_if_ready(&self) {
        if self.parser_blocking_stylesheet_release_is_ready() {
            self.request_main_parser_continuation_if_active();
        }
    }

    pub(crate) fn take_ready_stylesheet_network_results(
        &mut self,
    ) -> Vec<ConnectedLoadNetworkResult> {
        self.drain_blocking_stylesheet_completions();
        let mut results = self
            .stylesheet_lifecycle
            .fetches
            .take_ready_network_results()
            .into_iter()
            .map(|result| {
                let origin_clean = result.terminal.origin_clean().unwrap_or(false);
                let physical_result = result.terminal.physical().as_result();
                ConnectedLoadNetworkResult {
                    stylesheet_fetch: result.fetch,
                    blocking_operation: result.blocking_operation,
                    source_operation: None,
                    import_roots: Vec::new(),
                    document_url: result.document_url,
                    request_url: result.request_url,
                    source_owners: result
                        .owner_node_ids
                        .into_iter()
                        .map(|node_id| DomHandle::new(node_id.index()))
                        .collect(),
                    resource_type: SubresourceResourceType::Stylesheet,
                    start_unix_millis: Some(result.start_unix_millis),
                    origin_clean,
                    result: physical_result,
                }
            })
            .collect::<Vec<_>>();
        results.extend(
            self.stylesheet_lifecycle
                .ready_connected_load_network_results
                .drain(..),
        );
        results
            .into_iter()
            .map(|result| self.apply_network_result_install_authority(result))
            .collect()
    }

    pub(crate) fn take_ready_stylesheet_link_client_terminals(
        &mut self,
    ) -> Vec<super::StylesheetLinkClientTerminal> {
        self.drain_blocking_stylesheet_completions();
        self.stylesheet_lifecycle
            .ready_stylesheet_link_client_terminals
            .drain(..)
            .filter(|client| {
                self.dom_host.is_connected(client.load().owner())
                    && self
                        .stylesheet_lifecycle
                        .owner_states
                        .accepts_stylesheet_link_client(client.load())
            })
            .collect()
    }
}
