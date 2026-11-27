//! Publication of completed stylesheet work on the owning Document.

use super::{
    DocumentRuntime, PageScriptHost, StylesheetLinkClientTerminal,
    blocking_import_graph::ReadyBlockingStyleImportGraph,
};
use crate::{
    context_bootstrap::{ResourcePerformanceEntry, record_resource_performance_entry},
    document_runtime::{ConnectedLoadNetworkResult, ConnectedStyleImportRoot},
    live_stylesheet::LiveStylesheetImportResponse,
    stylesheet_blocking::DocumentBlockingStylesheetSignature,
    types::{SubresourceRequestInitiatorType, SubresourceResourceType},
};
use std::collections::HashSet;

pub(crate) struct ReadyStylesheetPublication {
    network_results: Vec<ConnectedLoadNetworkResult>,
    client_terminals: Vec<StylesheetLinkClientTerminal>,
    blocking_import_graphs: Vec<ReadyBlockingStyleImportGraph>,
}

impl DocumentRuntime {
    /// Advance the stylesheet completion source and collect results already
    /// admitted to this Document. Page Networking tasks keep their own
    /// selection boundary.
    pub(crate) fn take_ready_stylesheet_publication(
        &mut self,
    ) -> Option<ReadyStylesheetPublication> {
        self.drain_blocking_stylesheet_completions();
        let network_results = self.take_ready_stylesheet_network_results();
        let client_terminals = self.take_ready_stylesheet_link_client_terminals();
        let blocking_import_graphs = self.take_ready_blocking_style_import_graphs();
        if network_results.is_empty()
            && client_terminals.is_empty()
            && blocking_import_graphs.is_empty()
        {
            return None;
        }
        Some(ReadyStylesheetPublication {
            network_results,
            client_terminals,
            blocking_import_graphs,
        })
    }

    pub(crate) fn publish_ready_stylesheet_results_in_current_scope(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
    ) {
        if let Some(publication) = self.take_ready_stylesheet_publication() {
            self.publish_stylesheet_results_in_current_scope(scope, host_ptr, publication);
        }
    }

    pub(crate) fn prepare_parser_stylesheet_gate_in_current_scope(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        signatures: &HashSet<DocumentBlockingStylesheetSignature>,
    ) -> bool {
        self.publish_ready_stylesheet_results_in_current_scope(scope, host_ptr);
        self.has_pending_parser_script_blocking_stylesheet_signatures(signatures)
    }

    /// Install the live CSSOM before a script can observe the completed
    /// stylesheet. Element load/error still belongs to a separate Page task.
    pub(crate) fn publish_stylesheet_results_in_current_scope(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        publication: ReadyStylesheetPublication,
    ) {
        let ReadyStylesheetPublication {
            network_results: results,
            client_terminals,
            blocking_import_graphs,
        } = publication;
        let optional_resource_fetch_mask = self.current_document_resource_loader().map_or(
            crate::protocol_types::OptionalResourceFetchMask::NONE,
            |loader| loader.request_client().optional_resource_fetch_mask(),
        );
        let mut performance_entries = Vec::new();
        let mut css_subresources = Vec::new();
        let mut import_graph_results: Vec<(
            ConnectedStyleImportRoot,
            Vec<LiveStylesheetImportResponse>,
        )> = Vec::new();
        let mut parsed_responses = Vec::new();
        let host = unsafe { &mut *host_ptr };
        for network_result in results {
            let ConnectedLoadNetworkResult {
                network_observed,
                resource_timing_initiator_type,
                stylesheet_fetch,
                blocking_operation: _,
                source_operation: _,
                import_roots,
                parsed_stylesheet,
                document_url,
                request_url,
                source_owners,
                resource_type,
                start_unix_millis,
                origin_clean,
                result,
            } = network_result;
            parsed_responses.extend(parsed_stylesheet.iter().cloned());
            let is_import_graph_result = !import_roots.is_empty();
            let import_roots = import_roots
                .into_iter()
                .filter(|root| {
                    host.live_stylesheet(root.stylesheet_id)
                        .is_some_and(|stylesheet| root.matches_stylesheet(&stylesheet))
                })
                .collect::<Vec<_>>();
            let has_stylesheet_install_authority = if is_import_graph_result {
                !import_roots.is_empty()
            } else {
                !source_owners.is_empty()
            };
            performance_entries.push(ResourcePerformanceEntry::from_network_result(
                request_url.as_str(),
                resource_timing_initiator_type,
                start_unix_millis,
                &result,
            ));
            if resource_type == SubresourceResourceType::Stylesheet && stylesheet_fetch.is_none() {
                let validated_response = result
                    .as_ref()
                    .ok()
                    .filter(|response| (200..=299).contains(&response.status))
                    .and_then(|response| {
                        crate::stylesheet_blocking::validate_stylesheet_response(
                            &request_url,
                            response.clone(),
                        )
                        .ok()
                    });
                let (stylesheet_text, stylesheet_base_url) = validated_response
                    .as_ref()
                    .map(|response| (response.body_text().to_owned(), response.final_url.clone()))
                    .unwrap_or_else(|| {
                        let final_url = result
                            .as_ref()
                            .ok()
                            .map(|response| response.final_url.clone())
                            .unwrap_or_else(|| request_url.clone());
                        (String::new(), final_url)
                    });
                if is_import_graph_result {
                    let response = LiveStylesheetImportResponse {
                        request_url: request_url.clone(),
                        response_url: stylesheet_base_url.clone(),
                        css_text: stylesheet_text.clone(),
                        parsed_stylesheet: parsed_stylesheet.clone(),
                        successful: validated_response.is_some(),
                        origin_clean,
                    };
                    for root in import_roots {
                        if let Some((_, responses)) =
                            import_graph_results.iter_mut().find(|(candidate, _)| {
                                candidate.owner == root.owner
                                    && candidate.stylesheet_id == root.stylesheet_id
                                    && candidate.contents_revision == root.contents_revision
                                    && candidate.import_generation == root.import_generation
                            })
                        {
                            responses.push(response.clone());
                        } else {
                            import_graph_results.push((root, vec![response.clone()]));
                        }
                    }
                }
                if !is_import_graph_result {
                    for owner in source_owners.iter().copied() {
                        let Some(prepared) = host.prepare_linked_stylesheet_resource(
                            owner,
                            &stylesheet_text,
                            stylesheet_base_url.clone(),
                            request_url.clone(),
                            origin_clean,
                            parsed_stylesheet.as_ref(),
                        ) else {
                            continue;
                        };
                        host.install_linked_stylesheet(
                            crate::document_runtime::InstallLinkedStylesheet::from_prepared(
                                owner,
                                request_url.clone(),
                                prepared,
                            ),
                        );
                    }
                }
                // A stale completion remains observable as Network and
                // Performance data, but cannot derive new requests in the
                // current document after its exact owner/root authority was
                // revoked.
                if has_stylesheet_install_authority && validated_response.is_some() {
                    for resource in
                        crate::css_resource_urls::stylesheet_load_blocking_font_resources(
                            &stylesheet_text,
                            &stylesheet_base_url,
                            optional_resource_fetch_mask,
                        )
                    {
                        let Some(binding) =
                            host.accept_current_main_stylesheet_subresource_load_delay()
                        else {
                            tracing::debug!(
                                url = ?resource.request_url(),
                                kind = ?resource.kind(),
                                "skipping stylesheet subresource for stale main document owner"
                            );
                            continue;
                        };
                        css_subresources.push((binding, resource));
                    }
                }
            }
            if !network_observed {
                host.record_get_subresource_network_result_with_initiator(
                    None,
                    document_url,
                    request_url,
                    resource_type,
                    SubresourceRequestInitiatorType::Parser,
                    &result,
                );
            }
        }
        for (root, responses) in import_graph_results {
            let _ = host.install_and_refresh_live_stylesheet_import_graph(root, &responses);
        }
        // Roots filtered by revision or generation still leave a completed
        // network record. Only this installation batch needs its native parse.
        for parsed in parsed_responses {
            parsed.discard();
        }
        for blocking_import_graph in blocking_import_graphs {
            let (source, roots, graph) = blocking_import_graph.into_parts();
            let mut successful = graph.successful();
            let expected_root_count = roots.len();
            let roots = roots
                .into_iter()
                .filter(|root| {
                    host.live_stylesheet(root.stylesheet_id)
                        .is_some_and(|stylesheet| root.matches_stylesheet(&stylesheet))
                })
                .collect::<Vec<_>>();
            successful &= !roots.is_empty() && roots.len() == expected_root_count;
            let responses =
                crate::document_runtime::live_stylesheet_import_responses(graph.as_ref());
            if !roots.is_empty() {
                for response in &responses {
                    if !response.successful {
                        continue;
                    }
                    for resource in
                        crate::css_resource_urls::stylesheet_load_blocking_font_resources(
                            &response.css_text,
                            &response.response_url,
                            optional_resource_fetch_mask,
                        )
                    {
                        let Some(binding) =
                            host.accept_current_main_stylesheet_subresource_load_delay()
                        else {
                            continue;
                        };
                        css_subresources.push((binding, resource));
                    }
                }
            }
            for root in roots {
                match host.install_and_refresh_live_stylesheet_import_graph(root, &responses) {
                    Some(graph_successful) => {
                        successful &= graph_successful;
                    }
                    None => successful = false,
                }
            }
            self.complete_ready_blocking_style_import_graph(source, successful, graph);
        }
        self.settle_stylesheet_link_clients_in_current_scope(scope, host_ptr, client_terminals);
        self.apply_pending_stylesheet_source_css_projections(scope, host_ptr);
        Self::start_stylesheet_subresource_fetches_in_current_scope(
            scope,
            host_ptr,
            css_subresources,
        );
        if !performance_entries.is_empty() {
            // A borrowed document.write method can enter from another realm.
            // Resource Timing belongs to the Window that owns these fetches.
            let context = unsafe { &*host_ptr }
                .page_default_context(scope)
                .unwrap_or_else(|| scope.get_current_context());
            let scope = &mut v8::ContextScope::new(scope, context);
            for entry in performance_entries {
                record_resource_performance_entry(scope, entry);
            }
        }
    }
}
