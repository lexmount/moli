use super::JsContextHost;
use crate::{
    document_runtime::DomHandle,
    document_script_scheduler::FrameDocumentStylesheetAdmission,
    frame_owner_model::{DocumentLoadDelayTokenId, FrameDocumentTaskOwner},
    stylesheet_blocking::{
        DocumentBlockingStylesheetSignature, DocumentOwnedStylesheetDiscoveryInput,
        RendererStylesheetFetcher, ServiceWorkerStylesheetFetchContext, StylesheetFetcher,
    },
    types::{
        ChildBlockingStylesheetLoadCompletion, ChildBlockingStylesheetNetworkResult,
        SubresourceResourceType,
    },
};

impl JsContextHost {
    pub(in crate::native_bridge::context_host) fn accept_child_parser_stylesheet_inputs(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        inputs: Vec<DocumentOwnedStylesheetDiscoveryInput>,
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
            });
        let mut accepted = 0;
        for input in inputs {
            let signature = input.signature().clone();
            let import_root = if matches!(
                signature,
                DocumentBlockingStylesheetSignature::ParserCreatedStyleImport { .. }
            ) {
                let style_owner = DomHandle::new(input.node_id().index());
                self.install_owner_live_stylesheet(style_owner);
                self.owner_live_stylesheet(style_owner).map(|stylesheet| {
                    crate::document_runtime::ConnectedStyleImportRoot::new(
                        style_owner,
                        &stylesheet,
                        None,
                    )
                })
            } else {
                None
            };
            let stylesheet_store = &mut self.frame_document_stylesheets;
            let frame_owner_store = &mut self.frame_owner_store;
            let load_delay_token =
                match stylesheet_store.discover(owner.document_owner(), &input, import_root, || {
                    frame_owner_store
                        .acquire_current_child_blocking_stylesheet_load_delay(child_handle, owner)
                }) {
                    FrameDocumentStylesheetAdmission::Start(token) => token,
                    FrameDocumentStylesheetAdmission::Joined => continue,
                };
            accepted += 1;
            let Some(fetcher) = fetcher.clone().and_then(|fetcher| {
                self.stylesheet_fetch_csp_context_for_owner(
                    crate::native_bridge::OwnerDispatchScope::Child(child_handle),
                )
                .map(|context| fetcher.with_csp_context(context))
            }) else {
                self.frame_document_stylesheets
                    .take_completion(owner.document_owner(), &signature, load_delay_token)
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
            self.spawn_child_parser_stylesheet_fetch(
                child_handle,
                owner,
                document_url.clone(),
                signature,
                load_delay_token,
                fetcher,
            );
        }
        accepted
    }

    fn spawn_child_parser_stylesheet_fetch(
        &self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        document_url: url::Url,
        signature: DocumentBlockingStylesheetSignature,
        load_delay_token: DocumentLoadDelayTokenId,
        fetcher: RendererStylesheetFetcher,
    ) {
        let completion_tx = self.resource_completion_tx.clone();
        let frame_id = self
            .child_browsing_contexts
            .get(&child_handle)
            .map(|entry| entry.frame_id().to_owned());
        let resource_loader = self
            .document_resource_loader_for_owner(owner)
            .expect("accepted child stylesheet requires its Document authority");
        resource_loader.spawn_resource_task(async move {
            let csp_context = fetcher.csp_context().cloned();
            let linked = matches!(&signature, DocumentBlockingStylesheetSignature::Link { .. });
            let graph = match &signature {
                DocumentBlockingStylesheetSignature::Link { url, options } => {
                    let started = moli_time::unix_epoch_millis();
                    let terminal = fetcher
                        .fetch_stylesheet_resource(
                            document_url.clone(),
                            url.clone(),
                            options.clone(),
                        )
                        .await;
                    crate::document_runtime::fetch_complete_stylesheet_import_graph_from_root(
                        fetcher,
                        document_url.clone(),
                        crate::stylesheet_blocking::StylesheetImportNetworkResult::new(
                            url.clone(),
                            started,
                            terminal,
                        ),
                    )
                    .await
                }
                DocumentBlockingStylesheetSignature::ParserCreatedStyleImport { urls } => {
                    crate::document_runtime::fetch_complete_stylesheet_import_graph(
                        fetcher,
                        document_url.clone(),
                        urls.clone(),
                    )
                    .await
                }
            };
            let network_results = graph
                .network_results()
                .iter()
                .enumerate()
                .map(|(index, result)| ChildBlockingStylesheetNetworkResult {
                    frame_id: frame_id.clone(),
                    document_url: document_url.clone(),
                    request_url: result.request_url().clone(),
                    initiator_type: if linked && index == 0 {
                        crate::types::SubresourceRequestInitiatorType::Parser
                    } else {
                        crate::types::SubresourceRequestInitiatorType::Css
                    },
                    terminal: result.terminal().clone(),
                })
                .collect();
            let _ = completion_tx.send_child_blocking_stylesheet(
                ChildBlockingStylesheetLoadCompletion {
                    child_handle,
                    owner,
                    signature,
                    load_delay_token,
                    network_results,
                    import_graph: Some(std::sync::Arc::new(graph)),
                    csp_violations: csp_context
                        .map_or_else(Vec::new, |context| context.take_violations()),
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
        let successful = completion.successful();
        let Some(subscribers) = self.frame_document_stylesheets.take_completion(
            completion.owner.document_owner(),
            &completion.signature,
            completion.load_delay_token,
        ) else {
            tracing::debug!(
                child_handle = ?completion.child_handle,
                owner = ?completion.owner,
                signature = ?completion.signature,
                "dropping duplicate or unowned child parser stylesheet terminal"
            );
            return;
        };
        let load_delay_token = subscribers.load_delay_token;
        if let Some((root_owner, target)) = self.current_main_document_task_owner().zip(
            self.current_window_document_task_target_for_dispatch_scope(
                crate::native_bridge::OwnerDispatchScope::Child(completion.child_handle),
            ),
        ) {
            for violation in &completion.csp_violations {
                // Use the existing Page task path: no JavaScript may run while
                // this completion is still installing CSSOM and releasing blockers.
                unsafe { &mut *self.runtime }.enqueue_content_security_policy_violation_event_task(
                    crate::page_task_queue::ContentSecurityPolicyViolationEventTask::for_window(
                        root_owner,
                        target,
                        violation.clone(),
                    ),
                );
            }
        }
        let installed_stylesheet_count = self.install_child_parser_stylesheet_result(
            &completion,
            &subscribers.node_ids,
            &subscribers.import_roots,
        );
        let mut stylesheet_subresources = Vec::new();
        let optional_resource_fetch_mask = self
            .document_resource_loader_for_owner(completion.owner)
            .map_or(
                crate::protocol_types::OptionalResourceFetchMask::NONE,
                |loader| loader.request_client().optional_resource_fetch_mask(),
            );
        for network_result in &completion.network_results {
            let terminal = &network_result.terminal;
            if terminal.failed_integrity() {
                // Settle the failed load below without creating an owner sheet.
                continue;
            }
            // Ordinary resource failures still create an empty owner sheet.
            let ready_response = terminal.ready_response();
            let Some(response) = ready_response else {
                // A failed terminal exposes no response body to CSS parsing and
                // cannot admit stylesheet-dependent resource requests.
                continue;
            };
            for resource in crate::css_resource_urls::stylesheet_load_blocking_font_resources(
                response.body_text(),
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
                stylesheet_subresources.push((binding, resource));
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
        if self
            .child_document_parsers
            .is_suspended_on_parser_created_stylesheet(document_owner)
            && !self.frame_document_stylesheets.has_pending(document_owner)
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

    // Install only for this in-flight request's remaining subscribers.
    fn install_child_parser_stylesheet_result(
        &mut self,
        completion: &ChildBlockingStylesheetLoadCompletion,
        node_ids: &[crate::dom::NodeId],
        roots: &[crate::document_runtime::ConnectedStyleImportRoot],
    ) -> usize {
        let mut import_roots = roots.to_vec();
        let mut installed = 0;
        let linked =
            if let DocumentBlockingStylesheetSignature::Link { url, .. } = &completion.signature {
                let Some(result) = completion
                    .network_results
                    .iter()
                    .find(|result| &result.request_url == url)
                else {
                    return 0;
                };
                let terminal = &result.terminal;
                if terminal.failed_integrity() {
                    return 0;
                }
                let text = terminal
                    .ready_response()
                    .map_or("", |response| response.body_text());
                let base_url = match terminal.physical() {
                    crate::stylesheet_blocking::StylesheetPhysicalOutcome::Response(response) => {
                        response.final_url.clone()
                    }
                    crate::stylesheet_blocking::StylesheetPhysicalOutcome::NetworkError(_) => {
                        result.request_url.clone()
                    }
                };
                for node in node_ids {
                    let owner = DomHandle::new(node.index());
                    let Some(prepared) = self.prepare_linked_stylesheet_resource(
                        owner,
                        text,
                        base_url.clone(),
                        result.request_url.clone(),
                        terminal.origin_clean().unwrap_or(false),
                    ) else {
                        continue;
                    };
                    self.install_linked_stylesheet(
                        crate::document_runtime::InstallLinkedStylesheet::from_prepared(
                            owner,
                            result.request_url.clone(),
                            prepared,
                        ),
                    );
                    if let Some(sheet) = self.linked_live_stylesheet(owner) {
                        import_roots.push(crate::document_runtime::ConnectedStyleImportRoot::new(
                            owner,
                            &sheet,
                            Some(result.request_url.clone()),
                        ));
                    }
                    installed += 1;
                }
                true
            } else {
                false
            };
        if let Some(graph) = &completion.import_graph {
            let responses = crate::document_runtime::live_stylesheet_import_responses(graph);
            let refreshed = import_roots
                .iter()
                .filter(|root| {
                    self.install_live_stylesheet_import_graph((*root).clone(), &responses)
                        .is_some_and(|_| {
                            self.refresh_live_stylesheet_after_import_graph(
                                root.owner,
                                root.stylesheet_id,
                            )
                        })
                })
                .count();
            if !linked {
                installed = refreshed;
            }
        }
        installed
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
