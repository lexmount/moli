//! Immediate settlement of exact stylesheet clients.
//!
//! A client that adopts an already-terminal physical fetch is completed in the
//! same owner operation that admitted it. Only the resulting element event and
//! genuinely asynchronous dependent resources cross a Page task boundary.

use super::*;

impl DocumentRuntime {
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
            // Blink creates the owner's CSSStyleSheet even when the resource
            // failed or was cancelled. An unusable terminal therefore installs
            // an empty source; its existing link task still reports the error.
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
            let prepared = host.prepare_linked_stylesheet_resource(
                load.owner(),
                &stylesheet_text,
                stylesheet_base_url.clone(),
                request_url.clone(),
                terminal.origin_clean().unwrap_or(false),
            );
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
            for resource in crate::css_resource_urls::stylesheet_load_blocking_resources(
                &stylesheet_text,
                &stylesheet_base_url,
                optional_resource_fetch_mask,
            ) {
                let Some(binding) = host.accept_current_main_stylesheet_subresource_load_delay()
                else {
                    tracing::debug!(
                        url = %resource.request_url(),
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
            match resource.kind() {
                crate::css_resource_urls::StylesheetLoadBlockingResourceKind::Font
                    if binding.child_handle().is_none() =>
                {
                    match host.admit_document_web_font(resource) {
                        Some(resource) => admitted.push((binding, resource, None)),
                        None => {
                            host.settle_stylesheet_subresource_load_delay(binding);
                        }
                    }
                }
                crate::css_resource_urls::StylesheetLoadBlockingResourceKind::Image
                    if retain_css_images =>
                {
                    let resolved_url = resource.request_url().as_str().to_owned();
                    match host.admit_stylesheet_css_image(binding, resolved_url) {
                        crate::native_bridge::CssImageResourceAdmission::Fetch(identity) => {
                            admitted.push((binding, resource, Some(identity)));
                        }
                        crate::native_bridge::CssImageResourceAdmission::Reused => {
                            host.settle_stylesheet_subresource_load_delay(binding);
                        }
                        crate::native_bridge::CssImageResourceAdmission::Untracked => {
                            admitted.push((binding, resource, None));
                        }
                    }
                }
                _ => admitted.push((binding, resource, None)),
            }
        }
        for (binding, resource, css_image) in admitted {
            let request_url = resource.request_url().clone();
            let kind = resource.kind();
            let failed_css_image = css_image.clone();
            let failed_web_font = (binding.child_handle().is_none()
                && kind == crate::css_resource_urls::StylesheetLoadBlockingResourceKind::Font)
                .then(|| {
                    resource
                        .web_font()
                        .cloned()
                        .map(crate::css_resource_urls::CompletedStylesheetWebFont::failure)
                })
                .flatten();
            match crate::network_host::start_stylesheet_subresource_fetch(
                scope, host, binding, resource, css_image,
            ) {
                Ok(crate::network_host::StylesheetSubresourceFetchStart::WebFontSettled(
                    web_font,
                )) => {
                    Self::complete_document_web_font(host, web_font);
                }
                Ok(
                    crate::network_host::StylesheetSubresourceFetchStart::Pending
                    | crate::network_host::StylesheetSubresourceFetchStart::Settled,
                ) => {}
                Err(error) => {
                    if let Some(identity) = failed_css_image.as_ref() {
                        let _ = host.fail_stylesheet_css_image(identity);
                    }
                    let settlement = host.settle_stylesheet_subresource_load_delay(binding);
                    if let Some(web_font) = failed_web_font {
                        Self::complete_document_web_font(host, web_font);
                    }
                    tracing::warn!(
                        url = %request_url,
                        ?kind,
                        owner = ?binding.owner(),
                        settled = settlement.settled(),
                        %error,
                        "stylesheet subresource failed before network scheduling"
                    );
                }
            }
        }
        tracing::debug!(
            resource_count,
            elapsed_ms = started.elapsed().as_millis(),
            "started owner-bound stylesheet subresource requests"
        );
    }

    fn complete_document_web_font(
        host: &JsContextHost,
        terminal: crate::css_resource_urls::CompletedStylesheetWebFont,
    ) {
        match host.complete_document_web_font(terminal) {
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
        }
    }
}
