use std::sync::Arc;

use moli_fetch::{FetchCancelHandle, FetchResponseFilter, Request, RequestRedirectCheck};
use parking_lot::Mutex;
use url::Url;

use crate::{
    context_bootstrap::{FetchResourceTiming, ResourcePerformanceEntry},
    document_runtime::{
        DocumentContentSecurityPolicyViolation, DocumentRequestPolicySnapshot,
        document_content_security_policy_error_message,
    },
    dynamic_script_owner::DynamicScriptServiceWorkerContext,
    native_bridge::{JsContextHost, OwnerDispatchScope},
    network::loads::ResourceLoadLease,
    network_host::{FetchCspRedirectState, WindowCspReportRequestContext},
    protocol_types::NavigationResponse,
    service_worker_runtime::ServiceWorkerRequestDestination,
    types::SubresourceResourceType,
};

use super::{ModuleFetchMetadata, ModuleKind};

/// Native settings of the Document that starts one module fetch. Neither
/// caller realms nor element wrappers are retained by asynchronous transport.
pub(crate) struct NativeModuleFetchContext {
    policy: DocumentRequestPolicySnapshot,
    report_context: WindowCspReportRequestContext,
    redirects: FetchCspRedirectState,
    violations: Arc<Mutex<Vec<DocumentContentSecurityPolicyViolation>>>,
    service_worker: DynamicScriptServiceWorkerContext,
    request_origin: moli_url::WebOrigin,
    destination: ServiceWorkerRequestDestination,
    resource_type: SubresourceResourceType,
    timing: Mutex<FetchResourceTiming>,
    entry: Mutex<Option<ResourcePerformanceEntry>>,
}

impl std::fmt::Debug for NativeModuleFetchContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeModuleFetchContext")
            .field("identity", &self.report_context.identity())
            .finish_non_exhaustive()
    }
}

impl NativeModuleFetchContext {
    pub(crate) fn capture(
        scope: &mut v8::PinScope<'_, '_>,
        host: &mut JsContextHost,
        owner: OwnerDispatchScope,
        kind: ModuleKind,
        metadata: &ModuleFetchMetadata,
    ) -> Option<Arc<Self>> {
        let loader = host.document_resource_loader_for_dispatch_scope(owner)?;
        let mut policy = host.document_connect_policy_snapshot_for_owner(owner)?;
        policy = if kind == ModuleKind::Css {
            policy.for_style_preload(metadata.nonce().map(str::to_owned))
        } else {
            policy.for_script_preload(crate::planning::ScriptFetchMetadata {
                nonce: metadata.nonce().map(str::to_owned),
                integrity: metadata.integrity().map(str::to_owned),
                ..Default::default()
            })
        };
        let report_context =
            crate::network_host::capture_window_csp_report_request_context(scope, host, owner)?;
        let redirects = FetchCspRedirectState::new(&policy);
        Some(Arc::new(Self {
            policy,
            report_context,
            redirects,
            violations: Arc::default(),
            service_worker: DynamicScriptServiceWorkerContext {
                browser_context_runtime: host.browser_context_runtime(),
                client_id: host.service_worker_client_id_for_subresource_owner(owner),
                document_url: loader.fetch_context().document_url().clone(),
            },
            request_origin: loader.fetch_context().request_origin(),
            destination: if kind == ModuleKind::Css {
                ServiceWorkerRequestDestination::Style
            } else {
                ServiceWorkerRequestDestination::Script
            },
            resource_type: if kind == ModuleKind::Css {
                SubresourceResourceType::Stylesheet
            } else {
                SubresourceResourceType::Script
            },
            // Modulepreload uses the single-module script fetch algorithm,
            // including its script initiator, regardless of its DOM source.
            timing: Mutex::new(FetchResourceTiming::for_initiator_type("script")),
            entry: Mutex::default(),
        }))
    }

    pub(crate) fn check_initial(&self, url: &Url) -> Result<(), String> {
        self.check_url(
            url,
            crate::content_security_policy::ContentSecurityPolicyRedirectStatus::NoRedirect,
        )
    }

    fn check_url(
        &self,
        url: &Url,
        redirect: crate::content_security_policy::ContentSecurityPolicyRedirectStatus,
    ) -> Result<(), String> {
        let (reports, enforced) = self
            .policy
            .check_url(&self.service_worker.document_url, url, redirect)
            .into_violations();
        let error = enforced
            .first()
            .map(|v| document_content_security_policy_error_message(v, "modulepreload"));
        self.violations
            .lock()
            .extend(reports.into_iter().chain(enforced));
        error.map_or(Ok(()), Err)
    }

    pub(crate) fn redirect_check(
        self: &Arc<Self>,
        request_url: &Url,
    ) -> Option<RequestRedirectCheck> {
        // A Service Worker can retain a redirect guard after its client
        // cancels. Keep only report data, not the Document/browser runtime.
        let violations = self.violations.clone();
        self.redirects.redirect_check(
            self.policy.clone(),
            self.service_worker.document_url.clone(),
            request_url.clone(),
            move |violation| violations.lock().push(violation),
        )
    }

    pub(crate) fn check_response_url(&self, response: &NavigationResponse) -> Result<(), String> {
        if response.redirected && !self.redirects.was_checked(&response.final_url) {
            self.check_url(&response.final_url, crate::content_security_policy::ContentSecurityPolicyRedirectStatus::FollowedRedirect)
        } else {
            Ok(())
        }
    }

    pub(crate) fn record_response(
        &self,
        request: &Request,
        response: &NavigationResponse,
        filter: Option<&FetchResponseFilter>,
        native: Option<moli_fetch::FetchResponseTiming>,
    ) {
        let mut timing = self.timing.lock();
        if let Some(native) = native {
            timing.observe_native_timing(native);
        }
        let filter = filter.cloned().unwrap_or_else(|| {
            crate::network_host::FetchResponseRequest {
                method: "GET",
                mode: request.request_mode,
                redirect_mode: request.redirect_mode(),
            }
            .network_response_filter(
                &self.request_origin,
                &response.head(),
                request.credentials_mode,
            )
        });
        *self.entry.lock() = Some(timing.response(
            &request.url,
            &self.request_origin,
            &response.head(),
            &filter,
            response.body_bytes().len(),
        ));
    }

    pub(crate) fn record_failure(&self, url: &Url) {
        let entry = self.timing.lock().failure(url);
        *self.entry.lock() = Some(entry);
    }

    pub(crate) fn take_performance_entry(&self) -> Option<ResourcePerformanceEntry> {
        self.entry.lock().take()
    }

    pub(crate) fn report_violations(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host: &mut JsContextHost,
        dispatch_dom_events: bool,
    ) {
        let violations = std::mem::take(&mut *self.violations.lock());
        let host_ptr = host as *mut JsContextHost;
        for violation in violations {
            crate::network_host::send_content_security_policy_violation_report_from_window_context(
                host,
                &self.report_context,
                &violation,
            );
            if dispatch_dom_events {
                host.dispatch_document_connect_csp_violation_event_for_exact_owner_without_report_best_effort(scope, host_ptr, self.report_context.identity(), &violation);
            }
        }
    }

    pub(crate) fn fetch(
        self: Arc<Self>,
        load: ResourceLoadLease,
        request: Request,
        callback: impl FnOnce(
            anyhow::Result<(NavigationResponse, Option<FetchResponseFilter>, bool)>,
            Option<moli_fetch::FetchResponseTiming>,
        ) + Send
        + 'static,
    ) {
        let runner = load.task_runner();
        let request_client = load.request_client();
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let cancel = FetchCancelHandle::new();
        load.attach_cancel_handle(cancel.clone());
        load.attach_consumer_cancel(move || {
            let _ = cancel_tx.send(());
        });
        runner.clone().spawn(Box::pin(async move {
            let result = tokio::select! {
                result = async {
                    match self.service_worker.browser_context_runtime.fetch_service_worker_subresource_for_client_with_metadata_and_cancellation(self.service_worker.client_id, self.service_worker.document_url.clone(), &request, &request_client, runner, self.destination, self.resource_type, cancel.clone()).await? {
                        Some(response) => Ok((*response.response, response.response_filter, !response.from_network_fallback)),
                        None => {
                            // The SDK owns its cache/preload consumer hook.
                            // Hand off the same lease after the SW stage ends.
                            load.release_consumer_cancel();
                            let (response_tx, response_rx) = tokio::sync::oneshot::channel();
                            request_client.fetch_cacheable_script_text_callback_with_load(request, load.clone(), move |response| { let _ = response_tx.send(response); })?;
                            let response = response_rx.await.map_err(|_| anyhow::anyhow!("module fetch response channel closed"))??;
                            Ok((NavigationResponse::from(response), None, false))
                        }
                    }
                } => result,
                Ok(()) = cancel_rx => { cancel.cancel(); Err(anyhow::anyhow!("modulepreload fetch cancelled when its Document retired")) }
            };
            let native = load.response_timing();
            load.finish();
            callback(result, native);
        }));
    }
}
