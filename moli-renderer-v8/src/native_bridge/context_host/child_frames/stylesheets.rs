use super::JsContextHost;
use crate::{
    document_runtime::DomHandle,
    frame_owner_model::FrameDocumentTaskOwner,
    stylesheet_blocking::{
        DocumentBlockingStylesheetSignature, DocumentOwnedBlockingStylesheetDiscoveryInput,
        RendererStylesheetFetcher, ServiceWorkerStylesheetFetchContext, StylesheetFetchOptions,
        StylesheetFetcher,
    },
    types::{ChildBlockingStylesheetLoadCompletion, ChildStylesheetResponse},
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
        let (fetcher, requests) = match &signature {
            DocumentBlockingStylesheetSignature::Link { url, options } => {
                (fetcher, vec![(url.clone(), options.clone())])
            }
            DocumentBlockingStylesheetSignature::ParserCreatedStyleImport { urls } => (
                fetcher.for_css_imports(),
                urls.iter()
                    .cloned()
                    .map(|url| (url, StylesheetFetchOptions::default()))
                    .collect(),
            ),
        };
        let completion_tx = self.resource_completion_tx.clone();
        let resource_loader = self
            .document_resource_loader_for_owner(owner)
            .expect("accepted child stylesheet requires its Document authority");
        resource_loader.spawn_resource_task(async move {
            let mut network_results = Vec::with_capacity(requests.len());
            for (request_url, options) in requests {
                let terminal = fetcher
                    .fetch_stylesheet_resource(document_url.clone(), request_url.clone(), options)
                    .await;
                network_results.push(ChildStylesheetResponse {
                    request_url,
                    terminal,
                });
            }
            let _ = completion_tx.send_child_blocking_stylesheet(
                ChildBlockingStylesheetLoadCompletion {
                    child_handle,
                    owner,
                    signature,
                    network_results,
                },
            );
        });
    }

    pub(crate) fn apply_child_blocking_stylesheet_load_completion(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        completion: ChildBlockingStylesheetLoadCompletion,
    ) {
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
            let stylesheet_text = ready_response
                .map(|response| response.body_text().to_owned())
                .unwrap_or_default();
            let stylesheet_base_url = match terminal.physical() {
                crate::stylesheet_blocking::StylesheetPhysicalOutcome::Response(response) => {
                    response.final_url.clone()
                }
                crate::stylesheet_blocking::StylesheetPhysicalOutcome::NetworkError(_) => {
                    network_result.request_url.clone()
                }
            };
            for owner in linked_stylesheet_owners.iter().copied() {
                let Some(prepared) = self.prepare_linked_stylesheet_resource(
                    owner,
                    &stylesheet_text,
                    stylesheet_base_url.clone(),
                    network_result.request_url.clone(),
                    terminal.origin_clean().unwrap_or(false),
                ) else {
                    continue;
                };
                self.install_linked_stylesheet(
                    crate::document_runtime::InstallLinkedStylesheet::from_prepared(
                        owner,
                        network_result.request_url.clone(),
                        prepared,
                    ),
                );
                installed_stylesheet_count += 1;
            }
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
                        url = %resource.request_url(),
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
        for (binding, resource) in stylesheet_subresources {
            let request_url = resource.request_url().clone();
            let kind = resource.kind();
            let css_image = if kind
                == crate::css_resource_urls::StylesheetLoadBlockingResourceKind::Image
                && self.layout_policy().uses_real_layout()
            {
                match self.admit_stylesheet_css_image(binding, request_url.as_str().to_owned()) {
                    crate::native_bridge::CssImageResourceAdmission::Fetch(identity) => {
                        Some(identity)
                    }
                    crate::native_bridge::CssImageResourceAdmission::Reused => {
                        self.settle_stylesheet_subresource_load_delay(binding);
                        continue;
                    }
                    crate::native_bridge::CssImageResourceAdmission::Untracked => None,
                }
            } else {
                None
            };
            let failed_css_image = css_image.clone();
            if let Err(error) = crate::network_host::start_stylesheet_subresource_fetch(
                scope, self, binding, resource, css_image,
            ) {
                if let Some(identity) = failed_css_image.as_ref() {
                    let _ = self.fail_stylesheet_css_image(identity);
                }
                let settlement = self.settle_stylesheet_subresource_load_delay(binding);
                tracing::warn!(
                    child_handle = ?completion.child_handle,
                    owner = ?completion.owner,
                    url = %request_url,
                    ?kind,
                    settled = settlement.settled(),
                    %error,
                    "child stylesheet subresource failed before network scheduling"
                );
            }
        }
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
}
