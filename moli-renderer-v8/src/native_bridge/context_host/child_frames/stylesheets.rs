use super::JsContextHost;
use crate::{
    document_runtime::DomHandle,
    frame_owner_model::FrameDocumentTaskOwner,
    stylesheet_blocking::{
        DocumentBlockingStylesheetSignature, DocumentOwnedBlockingStylesheetDiscoveryInput,
        RendererStylesheetFetcher, ServiceWorkerStylesheetFetchContext, StylesheetFetchOptions,
        StylesheetFetcher,
    },
    types::{
        ChildBlockingStylesheetLoadCompletion, ChildBlockingStylesheetNetworkResult,
        SubresourceResourceType,
    },
};

impl JsContextHost {
    pub(in crate::native_bridge::context_host) fn accept_child_parser_blocking_stylesheet_inputs(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        inputs: Vec<DocumentOwnedBlockingStylesheetDiscoveryInput>,
    ) -> usize {
        if inputs.is_empty()
            || !self
                .frame_owner_store
                .child_document_task_owner_is_current(child_handle, owner)
        {
            return 0;
        }
        let document_url = self
            .frame_owner_current_child_snapshot(child_handle)
            .map(|snapshot| snapshot.document_url)
            .unwrap_or_else(|| self.document_url().clone());
        let fetcher = self
            .document_resource_loader_for_owner(owner)
            .map(|loader| {
                let client_id = self.service_worker_client_id_for_subresource_owner(
                    crate::native_bridge::context_host::OwnerDispatchScope::Child(child_handle),
                );
                RendererStylesheetFetcher::new(
                    loader,
                    Some(ServiceWorkerStylesheetFetchContext {
                        browser_context_runtime: self.browser_context_runtime.clone(),
                        client_id,
                    }),
                )
                .with_document_referrer_policy(
                    self.child_browsing_context_document_handle(child_handle)
                        .and_then(|document| {
                            crate::context_bootstrap::document_referrer_policy_for_native_document(
                                self, document,
                            )
                        }),
                )
                .with_document_character_set(
                    self.child_browsing_context_document_handle(child_handle)
                        .and_then(|document| {
                            self.dom_host().document_character_set_for_handle(document)
                        })
                        .map(str::to_owned),
                )
            });
        let mut accepted = 0;
        for input in inputs {
            let signature = input.signature().clone();
            let stylesheet_store = &mut self.frame_document_blocking_stylesheets;
            let frame_owner_store = &mut self.frame_owner_store;
            if !stylesheet_store.discover(owner.document_owner(), &input, || {
                frame_owner_store
                    .acquire_current_child_blocking_stylesheet_load_delay(child_handle, owner)
            }) {
                continue;
            }
            accepted += 1;
            let Some(fetcher) = fetcher.clone() else {
                let load_delay_token = self
                    .frame_document_blocking_stylesheets
                    .apply_completion(owner.document_owner(), &signature, false)
                    .expect("accepted child stylesheet must own a load-delay token");
                let released = self
                    .frame_owner_store
                    .release_blocking_stylesheet_load_delay(owner, load_delay_token);
                debug_assert!(
                    released,
                    "current child stylesheet failure must release its exact load-delay token"
                );
                tracing::warn!(
                    child_handle = ?child_handle,
                    owner = ?owner,
                    signature = ?signature,
                    ?load_delay_token,
                    "child parser stylesheet has no installed loader; recording typed failure"
                );
                continue;
            };
            self.spawn_child_parser_blocking_stylesheet_fetch(
                child_handle,
                owner,
                document_url.clone(),
                signature,
                fetcher,
            );
        }
        accepted
    }

    fn spawn_child_parser_blocking_stylesheet_fetch(
        &self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        document_url: url::Url,
        signature: DocumentBlockingStylesheetSignature,
        fetcher: RendererStylesheetFetcher,
    ) {
        let import_roots = if matches!(
            &signature,
            DocumentBlockingStylesheetSignature::ParserCreatedStyleImport { .. }
        ) {
            self.frame_document_blocking_stylesheets
                .node_ids_for_signature(owner.document_owner(), &signature)
                .into_iter()
                .filter_map(|node| {
                    self.owner_live_stylesheet(node).map(|sheet| {
                        crate::document_runtime::ConnectedStyleImportRoot::new(node, &sheet, false)
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        let completion_tx = self.resource_completion_tx.clone();
        let frame_id = self
            .child_browsing_contexts
            .get(&child_handle)
            .map(|entry| entry.frame_id().to_owned());
        let resource_loader = self
            .document_resource_loader_for_owner(owner)
            .expect("accepted child stylesheet requires its Document authority");
        resource_loader.spawn_resource_task(async move {
            let document_options =
                fetcher.prepare_stylesheet_fetch_options(StylesheetFetchOptions::default());
            let mut network_results = Vec::new();
            let import_graph = match &signature {
                DocumentBlockingStylesheetSignature::Link { url, options } => {
                    let options = fetcher.prepare_stylesheet_fetch_options(options.clone());
                    let terminal = fetcher
                        .fetch_stylesheet_resource(document_url.clone(), url.clone(), options)
                        .await;
                    let graph = if let Some(response) = terminal.ready_response() {
                        let urls = crate::style_engine::stylesheet_top_level_import_urls(
                            terminal
                                .stylesheet_text()
                                .expect("ready CSS has decoded text"),
                            &response.final_url,
                            false,
                        )
                        .unwrap_or_default();
                        let options = crate::document_runtime::stylesheet_import_options(
                            &terminal,
                            &document_options,
                        )
                        .expect("a usable stylesheet retains its import request policy");
                        Some(
                            crate::document_runtime::fetch_complete_stylesheet_import_graph(
                                fetcher.clone(),
                                document_url.clone(),
                                urls,
                                options,
                            )
                            .await,
                        )
                    } else {
                        None
                    };
                    network_results.push(ChildBlockingStylesheetNetworkResult {
                        is_root: true,
                        frame_id: frame_id.clone(),
                        document_url: document_url.clone(),
                        request_url: url.clone(),
                        initiator_type: crate::types::SubresourceRequestInitiatorType::Parser,
                        terminal,
                    });
                    graph
                }
                DocumentBlockingStylesheetSignature::ParserCreatedStyleImport { urls } => {
                    let options = StylesheetFetchOptions::for_import(
                        document_url.clone(),
                        document_options
                            .document_referrer_policy()
                            .map(str::to_owned),
                    )
                    .with_document_referrer_policy(
                        document_options
                            .document_referrer_policy()
                            .map(str::to_owned),
                    )
                    .with_environment_encoding(
                        document_options.environment_encoding().map(str::to_owned),
                    );
                    Some(
                        crate::document_runtime::fetch_complete_stylesheet_import_graph(
                            fetcher,
                            document_url.clone(),
                            urls.clone(),
                            options,
                        )
                        .await,
                    )
                }
            }
            .map(std::sync::Arc::new);
            if let Some(graph) = &import_graph {
                network_results.extend(graph.network_results().iter().map(|result| {
                    ChildBlockingStylesheetNetworkResult {
                        is_root: false,
                        frame_id: frame_id.clone(),
                        document_url: document_url.clone(),
                        request_url: result.request_url().clone(),
                        initiator_type: crate::types::SubresourceRequestInitiatorType::Css,
                        terminal: result.terminal().clone(),
                    }
                }));
            }
            let _ = completion_tx.send_child_blocking_stylesheet(
                ChildBlockingStylesheetLoadCompletion {
                    child_handle,
                    owner,
                    signature,
                    network_results,
                    import_graph,
                    import_roots,
                },
            );
        });
    }

    pub(crate) fn apply_child_blocking_stylesheet_load_completion(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        completion: ChildBlockingStylesheetLoadCompletion,
    ) {
        for network_result in &completion.network_results {
            let physical_result = network_result.terminal.physical().as_result();
            self.record_get_subresource_network_result_with_initiator(
                network_result.frame_id.clone(),
                network_result.document_url.clone(),
                network_result.request_url.clone(),
                SubresourceResourceType::Stylesheet,
                network_result.initiator_type,
                &physical_result,
            );
        }
        if !self
            .frame_owner_store
            .child_document_task_owner_is_current(completion.child_handle, completion.owner)
        {
            tracing::debug!(
                child_handle = ?completion.child_handle,
                owner = ?completion.owner,
                signature = ?completion.signature,
                "dropping child parser stylesheet terminal for stale document owner"
            );
            return;
        }
        let linked_stylesheet_owners = matches!(
            &completion.signature,
            DocumentBlockingStylesheetSignature::Link { .. }
        )
        .then(|| {
            self.frame_document_blocking_stylesheets
                .node_ids_for_signature(completion.owner.document_owner(), &completion.signature)
        })
        .unwrap_or_default();
        let successful = completion.successful();
        let Some(load_delay_token) = self.frame_document_blocking_stylesheets.apply_completion(
            completion.owner.document_owner(),
            &completion.signature,
            successful,
        ) else {
            tracing::debug!(
                child_handle = ?completion.child_handle,
                owner = ?completion.owner,
                signature = ?completion.signature,
                "dropping duplicate or unowned child parser stylesheet terminal"
            );
            return;
        };
        let mut import_roots = completion.import_roots.clone();
        let mut installed_stylesheet_count = 0;
        let mut stylesheet_subresources = Vec::new();
        let optional_resource_fetch_mask = self
            .document_resource_loader_for_owner(completion.owner)
            .map_or(
                crate::protocol_types::OptionalResourceFetchMask::NONE,
                |loader| loader.request_client().optional_resource_fetch_mask(),
            );
        for network_result in &completion.network_results {
            let terminal = &network_result.terminal;
            // Match main-document LinkStyle semantics: failure changes the link
            // event result, but still creates an empty owner CSSStyleSheet.
            let ready_response = terminal.ready_response();
            let stylesheet_text = terminal
                .stylesheet_text()
                .map(str::to_owned)
                .unwrap_or_default();
            let stylesheet_base_url = match terminal.physical() {
                crate::stylesheet_blocking::StylesheetPhysicalOutcome::Response(response) => {
                    response.final_url.clone()
                }
                crate::stylesheet_blocking::StylesheetPhysicalOutcome::NetworkError(_) => {
                    network_result.request_url.clone()
                }
            };
            for owner in linked_stylesheet_owners
                .iter()
                .copied()
                .filter(|_| network_result.is_root)
            {
                let Some(prepared) = self.prepare_linked_stylesheet_resource(
                    owner,
                    &stylesheet_text,
                    stylesheet_base_url.clone(),
                    network_result.request_url.clone(),
                    terminal.origin_clean().unwrap_or(false),
                ) else {
                    continue;
                };
                let has_imports = prepared.has_import_rules();
                self.install_linked_stylesheet(
                    crate::document_runtime::InstallLinkedStylesheet::from_prepared(
                        owner,
                        network_result.request_url.clone(),
                        prepared,
                    ),
                );
                self.retain_linked_stylesheet_import_options(owner, terminal);
                if has_imports && let Some(sheet) = self.linked_live_stylesheet(owner) {
                    import_roots.push(crate::document_runtime::ConnectedStyleImportRoot::new(
                        owner, &sheet, true,
                    ));
                }
                installed_stylesheet_count += 1;
            }
            let Some(response) = ready_response else {
                // A failed terminal exposes no response body to CSS parsing and
                // cannot admit stylesheet-dependent resource requests.
                continue;
            };
            for resource in crate::css_resource_urls::stylesheet_load_blocking_font_resources(
                &stylesheet_text,
                &response.final_url,
                optional_resource_fetch_mask,
            ) {
                let Some(binding) = self.accept_current_child_stylesheet_subresource_load_delay(
                    completion.child_handle,
                    completion.owner,
                ) else {
                    tracing::debug!(
                        child_handle = ?completion.child_handle,
                        owner = ?completion.owner,
                        url = ?resource.request_url(),
                        kind = ?resource.kind(),
                        "skipping stylesheet subresource for stale child document owner"
                    );
                    continue;
                };
                let resource =
                    resource.with_resource_timing_initiator(Some(response.final_url.to_string()));
                stylesheet_subresources.push((binding, resource));
            }
        }
        if let Some(graph) = &completion.import_graph {
            let responses = crate::document_runtime::live_stylesheet_import_responses(graph);
            for root in import_roots {
                if self
                    .live_stylesheet(root.stylesheet_id)
                    .is_none_or(|sheet| !root.matches_stylesheet(&sheet))
                {
                    continue;
                }
                if self
                    .install_live_stylesheet_import_graph(root.clone(), &responses)
                    .is_some()
                {
                    let _ = self
                        .refresh_live_stylesheet_after_import_graph(root.owner, root.stylesheet_id);
                }
            }
        }
        let released = self
            .frame_owner_store
            .release_blocking_stylesheet_load_delay(completion.owner, load_delay_token);
        debug_assert!(
            released,
            "current child stylesheet terminal must release its exact load-delay token"
        );
        if !released {
            tracing::error!(
                child_handle = ?completion.child_handle,
                owner = ?completion.owner,
                signature = ?completion.signature,
                ?load_delay_token,
                "child stylesheet terminal could not release its document load-delay token"
            );
            return;
        }
        crate::document_runtime::DocumentRuntime::start_stylesheet_subresource_fetches_in_current_scope(
            scope,
            self,
            stylesheet_subresources,
        );
        tracing::debug!(
            child_handle = ?completion.child_handle,
            owner = ?completion.owner,
            signature = ?completion.signature,
            successful,
            ?load_delay_token,
            installed_stylesheet_count,
            "installed child stylesheet and bound its load-blocking subresources"
        );

        let document_owner = completion.owner.document_owner();
        if let Some(document) = self.child_browsing_context_document_handle(completion.child_handle)
            && let Some(top_document) = self.top_level_document_for_document(document)
        {
            self.queue_top_document_post_parse_autofocus(top_document);
        }
        if self
            .child_document_parsers
            .is_suspended_on_parser_created_stylesheet(document_owner)
            && !self
                .frame_document_blocking_stylesheets
                .has_pending(document_owner)
        {
            let resume = self.resume_live_child_document_parser_after_blocker(
                scope,
                completion.child_handle,
                document_owner,
            );
            if resume.parser_was_resumed() {
                if let Some(work) = resume.into_scheduler_work() {
                    self.push_child_document_script_ready_input(work);
                }
                return;
            }
        }

        if self.queue_current_child_parser_blocking_script_if_ready(completion.child_handle) {
            return;
        }
        if self.queue_next_child_parser_deferred_script_if_ready(
            completion.child_handle,
            completion.owner,
        ) {
            return;
        }
        if self.queue_child_document_domcontentloaded_if_ready(
            completion.child_handle,
            completion.owner,
        ) {
            return;
        }
        if self.queue_child_document_complete_lifecycle_if_ready(completion.child_handle) {
            tracing::debug!(
                child_handle = ?completion.child_handle,
                owner = ?completion.owner,
                signature = ?completion.signature,
                "queued stable child lifecycle task after document-owned stylesheet terminal"
            );
            return;
        }
        tracing::debug!(
            child_handle = ?completion.child_handle,
            owner = ?completion.owner,
            signature = ?completion.signature,
            successful,
            "child parser stylesheet terminal applied without an immediately runnable script head"
        );
    }

    pub(crate) fn record_historical_child_blocking_stylesheet_network_results(
        &mut self,
        completion: &ChildBlockingStylesheetLoadCompletion,
    ) {
        for network_result in &completion.network_results {
            let physical_result = network_result.terminal.physical().as_result();
            self.record_historical_get_subresource_network_result_with_initiator(
                network_result.frame_id.clone(),
                network_result.document_url.clone(),
                network_result.request_url.clone(),
                SubresourceResourceType::Stylesheet,
                network_result.initiator_type,
                &physical_result,
            );
        }
    }
}
