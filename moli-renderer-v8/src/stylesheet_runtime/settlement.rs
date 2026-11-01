//! Immediate settlement of exact stylesheet clients.
//!
//! A client that adopts an already-terminal physical fetch is completed in the
//! same owner operation that admitted it. Only the resulting element event and
//! genuinely asynchronous dependent resources cross a Page task boundary.

use super::*;
use crate::css_resource_urls::{
    StylesheetLoadBlockingResource, StylesheetLoadBlockingResourceKind,
    StylesheetSubresourceRequest, StylesheetSubresourceTarget,
};

impl DocumentRuntime {
    pub(crate) fn current_stylesheet_resources(
        host: &JsContextHost,
        root: DomHandle,
    ) -> Option<crate::style_engine::StylesheetResourceSnapshot> {
        let mut reads = crate::native_bridge::element::StyleObservation::new(host);
        reads.read(root).stylesheet_resource_snapshot()
    }

    pub(crate) fn reconcile_document_web_fonts_in_current_scope(
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
    ) {
        let host = unsafe { &mut *host_ptr };
        host.apply_pending_stylesheet_source_css_projections(scope, host_ptr);
        // The shared resource mask is checked before allocating a document
        // list. Default resource-free pages keep their existing cheap path.
        if host
            .current_main_document_resource_loader()
            .is_some_and(|loader| {
                loader
                    .request_client()
                    .optional_resource_fetch_enabled(crate::types::SubresourceResourceType::Font)
            })
        {
            for document in host.active_layout_document_handles() {
                Self::reconcile_web_fonts_for_document_in_current_scope(scope, host_ptr, document);
            }
        }
        crate::context_bootstrap::discover_font_requests(scope);
    }

    pub(crate) fn reconcile_web_fonts_for_document_in_current_scope(
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document: crate::document_runtime::DomHandle,
    ) {
        let host = unsafe { &mut *host_ptr };
        let Some(dispatch_scope) = host.owner_dispatch_scope_for_node(document) else {
            return;
        };
        if !host
            .document_resource_loader_for_dispatch_scope(dispatch_scope)
            .is_some_and(|loader| {
                loader
                    .request_client()
                    .optional_resource_fetch_enabled(crate::types::SubresourceResourceType::Font)
            })
        {
            return;
        }
        let Some(root) = host
            .dom_host()
            .dom()
            .document_element_handle_for_document(document)
        else {
            return;
        };
        if host.document_web_font_sidecar_is_pristine(document)
            && !host.document_has_style_state(document)
            && !host.document_has_active_author_stylesheet_sources(document)
        {
            return;
        }
        let Some(resources) =
            crate::document_runtime::DocumentRuntime::current_stylesheet_resources(host, root)
        else {
            return;
        };
        tracing::trace!(
            ?document,
            stylesheet_import_dependency_count = resources.imports().len(),
            stylesheet_web_font_dependency_count = resources.web_fonts().len(),
            "observed typed stylesheet resource manifest"
        );
        let generation = resources.generation();
        if host.document_web_font_resources_are_current(document, generation) {
            return;
        }
        let resources = resources.web_fonts().to_vec();
        host.retain_document_web_font_slots(document, resources.iter());
        if resources.is_empty() {
            host.publish_document_web_font_resource_generation(document, generation);
            return;
        }
        let resource_count = resources.len();
        let bound = resources
            .into_iter()
            .filter_map(|resource| {
                host.accept_current_stylesheet_subresource_load_delay_for_document(document)
                    .map(|binding| (binding, resource))
            })
            .collect::<Vec<_>>();
        if bound.len() == resource_count {
            host.publish_document_web_font_resource_generation(document, generation);
        }
        Self::start_stylesheet_subresource_fetches_in_current_scope(scope, host_ptr, bound);
    }

    pub(crate) fn settle_stylesheet_link_clients_in_current_scope(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        clients: Vec<StylesheetLinkClientTerminal>,
    ) {
        if clients.is_empty() {
            return;
        }
        debug_assert!(!host_ptr.is_null());
        let clients = clients
            .into_iter()
            .filter(|client| {
                self.dom_host.is_connected(client.load().owner())
                    && self
                        .stylesheet_lifecycle
                        .owner_states
                        .accepts_stylesheet_link_client(client.load())
            })
            .collect::<Vec<_>>();
        if clients.is_empty() {
            return;
        }
        let optional_resource_fetch_mask = self.current_document_resource_loader().map_or(
            crate::protocol_types::OptionalResourceFetchMask::NONE,
            |loader| loader.request_client().optional_resource_fetch_mask(),
        );
        let mut css_subresources = Vec::new();
        self.apply_stylesheet_link_client_terminals(
            scope,
            host_ptr,
            clients,
            optional_resource_fetch_mask,
            &mut css_subresources,
        );
        self.apply_pending_stylesheet_source_css_projections(scope, host_ptr);
        Self::start_stylesheet_subresource_fetches_in_current_scope(
            scope,
            host_ptr,
            css_subresources,
        );
    }

    fn apply_stylesheet_link_client_terminals(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        client_terminals: Vec<StylesheetLinkClientTerminal>,
        optional_resource_fetch_mask: crate::protocol_types::OptionalResourceFetchMask,
        css_subresources: &mut Vec<(
            crate::frame_owner_model::StylesheetSubresourceLoadDelayBinding,
            crate::css_resource_urls::StylesheetLoadBlockingResource,
        )>,
    ) {
        let mut linked_stylesheet_import_urls = Vec::new();
        let mut linked_stylesheet_import_loads = Vec::new();
        let host = unsafe { &mut *host_ptr };
        for client in client_terminals {
            let load = client.load();
            if !load.installs_stylesheet() {
                continue;
            }
            let terminal = client.terminal();
            // Ordinary resource failures create an empty owner sheet. Integrity
            // rejection stops before CSSStyleSheet creation; its link error and
            // load-delay settlement still follow the normal terminal path.
            let ready_response = terminal.ready_response();
            let stylesheet_text = ready_response
                .map(|response| response.body_text().to_owned())
                .unwrap_or_default();
            let stylesheet_base_url = match terminal.physical() {
                crate::stylesheet_blocking::StylesheetPhysicalOutcome::Response(response) => {
                    response.final_url.clone()
                }
                crate::stylesheet_blocking::StylesheetPhysicalOutcome::NetworkError(_) => {
                    load.request_url().clone()
                }
            };
            let request_url = load.request_url().clone();
            let prepared = if terminal.failed_integrity() {
                None
            } else {
                host.prepare_linked_stylesheet_resource(
                    load.owner(),
                    &stylesheet_text,
                    stylesheet_base_url.clone(),
                    request_url.clone(),
                    terminal.origin_clean().unwrap_or(false),
                )
            };
            if let Some(prepared) = prepared.as_ref() {
                host.install_linked_stylesheet(InstallLinkedStylesheet::from_prepared(
                    load.owner(),
                    request_url.clone(),
                    prepared.clone(),
                ));
            }
            let import_root = prepared.as_ref().and_then(|_| {
                host.linked_live_stylesheet(load.owner()).map(|stylesheet| {
                    ConnectedStyleImportRoot::new(load.owner(), &stylesheet, true)
                })
            });
            match self.admit_linked_stylesheet_import_graph(load.fetch(), import_root.clone()) {
                LinkedStylesheetImportGraphAdmission::Start => {}
                LinkedStylesheetImportGraphAdmission::InFlight => continue,
                LinkedStylesheetImportGraphAdmission::Completed(graph) => {
                    if let Some(root) = import_root {
                        let responses = live_stylesheet_import_responses(&graph);
                        if host
                            .install_live_stylesheet_import_graph(root.clone(), &responses)
                            .is_some()
                        {
                            let _ = host.refresh_live_stylesheet_after_import_graph(
                                root.owner,
                                root.stylesheet_id,
                            );
                        }
                    }
                    continue;
                }
            }
            let import_urls = ready_response
                .and(prepared.as_ref())
                .map(|prepared| prepared.import_urls().to_vec())
                .unwrap_or_default();
            linked_stylesheet_import_loads.push((Arc::clone(load), import_urls.clone()));
            if ready_response.is_none() {
                // Only a usable response may contribute CSS text, imports, or
                // other dependent resources. The empty owner sheet is CSSOM-only.
                continue;
            }
            if request_url.scheme() != "data" {
                linked_stylesheet_import_urls.extend(import_urls);
            }
            for resource in crate::css_resource_urls::stylesheet_load_blocking_font_resources(
                &stylesheet_text,
                &stylesheet_base_url,
                optional_resource_fetch_mask,
            ) {
                let Some(binding) = host.accept_current_main_stylesheet_subresource_load_delay()
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
        for (load, urls) in linked_stylesheet_import_loads {
            self.prime_network_stylesheet_import_loads(load, urls, host_ptr);
        }
        self.queue_linked_stylesheet_import_csp_violations_in_current_scope(
            scope,
            host_ptr,
            linked_stylesheet_import_urls,
        );
    }

    fn queue_linked_stylesheet_import_csp_violations_in_current_scope(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        import_urls: impl IntoIterator<Item = url::Url>,
    ) {
        let violations = import_urls
            .into_iter()
            .flat_map(|url| {
                let (report_only, enforced) = self
                    .style_element_request_csp_check(
                        &url,
                        crate::content_security_policy::ContentSecurityPolicyStyleElementRequest {
                            nonce: None,
                        },
                    )
                    .into_violations();
                [report_only, enforced].into_iter().flatten()
            })
            .collect::<Vec<_>>();
        for violation in violations {
            self.queue_content_security_policy_violation_event_best_effort(
                scope, host_ptr, &violation,
            );
        }
    }

    pub(crate) fn start_stylesheet_subresource_fetches_in_current_scope(
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        resources: Vec<(
            crate::frame_owner_model::StylesheetSubresourceLoadDelayBinding,
            crate::css_resource_urls::StylesheetLoadBlockingResource,
        )>,
    ) {
        if resources.is_empty() {
            return;
        }
        debug_assert!(!host_ptr.is_null());
        let started = std::time::Instant::now();
        let resource_count = resources.len();
        let host = unsafe { &mut *host_ptr };
        let retain_css_images = host.layout_policy().uses_real_layout();
        let mut admitted = Vec::with_capacity(resources.len());
        for (binding, resource) in resources {
            match resource {
                resource @ StylesheetLoadBlockingResource::Font { .. } => {
                    let Some(document) = host.stylesheet_web_font_document(binding) else {
                        continue;
                    };
                    match host.admit_document_web_font(document, resource) {
                        Some(request) => admitted.push((binding, request)),
                        None => {
                            host.settle_stylesheet_subresource_load_delay(binding);
                        }
                    }
                }
                StylesheetLoadBlockingResource::Image(url) => {
                    let identity = if retain_css_images {
                        match host.admit_stylesheet_css_image(binding, url.as_str().to_owned()) {
                            crate::native_bridge::CssImageResourceAdmission::Fetch(identity) => {
                                Some(identity)
                            }
                            crate::native_bridge::CssImageResourceAdmission::Reused => {
                                host.settle_stylesheet_subresource_load_delay(binding);
                                continue;
                            }
                            crate::native_bridge::CssImageResourceAdmission::Untracked => None,
                        }
                    } else {
                        None
                    };
                    admitted.push((binding, StylesheetSubresourceRequest::image(url, identity)));
                }
            }
        }
        Self::start_stylesheet_subresource_requests_in_current_scope(scope, host_ptr, admitted);
        tracing::debug!(
            resource_count,
            elapsed_ms = started.elapsed().as_millis(),
            "started owner-bound stylesheet subresource requests"
        );
    }

    pub(crate) fn start_stylesheet_subresource_requests_in_current_scope(
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        requests: Vec<(
            crate::frame_owner_model::StylesheetSubresourceLoadDelayBinding,
            StylesheetSubresourceRequest,
        )>,
    ) {
        debug_assert!(!host_ptr.is_null());
        let host = unsafe { &mut *host_ptr };
        let mut admitted = std::collections::VecDeque::from(requests);
        while let Some((binding, request)) = admitted.pop_front() {
            let request_url = request.url().clone();
            let failed_target = request.target().clone();
            let kind = match &failed_target {
                StylesheetSubresourceTarget::Image(_) => StylesheetLoadBlockingResourceKind::Image,
                StylesheetSubresourceTarget::Font(_) => StylesheetLoadBlockingResourceKind::Font,
            };
            match crate::network_host::start_stylesheet_subresource_fetch(
                scope, host, binding, request,
            ) {
                Ok(crate::network_host::StylesheetSubresourceFetchStart::WebFontSettled(
                    web_font,
                )) => {
                    if let Some(document) = host.stylesheet_web_font_document(binding)
                        && let Some(resource) =
                            Self::complete_document_web_font(host, document, web_font)
                        && let Some(binding) = host
                            .accept_current_stylesheet_subresource_load_delay_for_document(document)
                    {
                        admitted.push_back((binding, resource));
                    }
                }
                Ok(
                    crate::network_host::StylesheetSubresourceFetchStart::Pending
                    | crate::network_host::StylesheetSubresourceFetchStart::Settled,
                ) => {}
                Err(error) => {
                    if let StylesheetSubresourceTarget::Image(Some(identity)) = &failed_target {
                        let _ = host.fail_stylesheet_css_image(identity);
                    }
                    let settlement = host.settle_stylesheet_subresource_load_delay(binding);
                    if let StylesheetSubresourceTarget::Font(font) = failed_target
                        && let Some(document) = host.stylesheet_web_font_document(binding)
                        && let Some(resource) = Self::complete_document_web_font(
                            host,
                            document,
                            crate::css_resource_urls::CompletedStylesheetWebFont::failure(font),
                        )
                        && let Some(binding) = host
                            .accept_current_stylesheet_subresource_load_delay_for_document(document)
                    {
                        admitted.push_back((binding, resource));
                    }
                    tracing::warn!(
                        url = ?request_url,
                        ?kind,
                        owner = ?binding.owner(),
                        settled = settlement.settled(),
                        error = %format_args!("{error:#}"),
                        "stylesheet subresource failed before network scheduling"
                    );
                }
            }
        }
        crate::context_bootstrap::publish_font_face_load_changes(scope);
    }

    pub(crate) fn complete_document_web_font(
        host: &JsContextHost,
        document: DomHandle,
        terminal: crate::css_resource_urls::CompletedStylesheetWebFont,
    ) -> Option<StylesheetSubresourceRequest> {
        match host.complete_document_web_font(document, terminal) {
            crate::script_vm::web_fonts::DocumentWebFontCompletion::Registered(outcome) => {
                tracing::debug!(
                    ?outcome,
                    "registered current document web font for the next fresh layout refresh"
                )
            }
            crate::script_vm::web_fonts::DocumentWebFontCompletion::Invalid(error) => {
                tracing::warn!(
                    %error,
                    "discarded invalid current document web font response"
                )
            }
            crate::script_vm::web_fonts::DocumentWebFontCompletion::NetworkFailed => {
                tracing::debug!("current document web font request reached a failed terminal")
            }
            crate::script_vm::web_fonts::DocumentWebFontCompletion::Stale => {
                tracing::debug!("discarded superseded document web font response")
            }
            crate::script_vm::web_fonts::DocumentWebFontCompletion::Retry(resource) => {
                return Some(*resource);
            }
        }
        None
    }
}
