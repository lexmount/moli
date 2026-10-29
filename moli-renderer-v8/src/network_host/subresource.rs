use super::*;
use crate::{
    native_bridge::{JsContextHost, OwnerDispatchScope},
    network::context::{DocumentResourceLoader, SubresourceRequestEnvironment},
    service_worker_runtime::{
        ServiceWorkerFetchDispatch, ServiceWorkerRequestDestination,
        service_worker_fetch_request_metadata,
    },
    types::{
        AsyncSubresourceFetchCompletion, AsyncSubresourceNetworkContext,
        PendingSubresourceFetchInfo, SubresourceResourceType,
    },
};
use anyhow::{Context, Result};
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, RequestCredentialsMode, RequestMode,
    RequestRedirectMode, RequestResourceType,
};

#[derive(Clone, Copy)]
pub(super) struct SubresourceFetchOptions {
    pub(super) resource_type: SubresourceResourceType,
    pub(super) request_mode: RequestMode,
    pub(super) credentials_mode: RequestCredentialsMode,
    request_resource_type: RequestResourceType,
    metadata: BrowserRequestMetadata,
    destination: ServiceWorkerRequestDestination,
}

impl SubresourceFetchOptions {
    pub(super) const FONT: Self = Self {
        resource_type: SubresourceResourceType::Font,
        request_mode: RequestMode::Cors,
        credentials_mode: RequestCredentialsMode::SameOrigin,
        request_resource_type: RequestResourceType::Font,
        metadata: BrowserRequestMetadata::Font,
        destination: ServiceWorkerRequestDestination::Font,
    };

    pub(super) const CSS_IMAGE: Self = Self {
        resource_type: SubresourceResourceType::Image,
        request_mode: RequestMode::NoCors,
        credentials_mode: RequestCredentialsMode::Include,
        request_resource_type: RequestResourceType::Image,
        metadata: BrowserRequestMetadata::Image,
        destination: ServiceWorkerRequestDestination::Image,
    };
}

/// Captures one resource request without starting it. The caller first admits
/// it through CSP/local-resource policy and binds its own pending continuation,
/// then dispatches using the assigned request identity.
pub(super) struct PreparedSubresourceFetch {
    pub(super) cancel_handle: FetchCancelHandle,
    pub(super) network_partition_key: Option<String>,
    pub(super) network_context: AsyncSubresourceNetworkContext,
    pub(super) pending_info: PendingSubresourceFetchInfo,
    request: Request,
    resource_loader: DocumentResourceLoader,
    owner: OwnerDispatchScope,
    destination: ServiceWorkerRequestDestination,
}

impl PreparedSubresourceFetch {
    pub(super) fn prepare(
        scope: &mut v8::PinScope<'_, '_>,
        host: &JsContextHost,
        owner: OwnerDispatchScope,
        resource_loader: &DocumentResourceLoader,
        environment: SubresourceRequestEnvironment,
        request_url: url::Url,
        options: SubresourceFetchOptions,
    ) -> Result<Self> {
        let SubresourceRequestEnvironment {
            document_url,
            request_origin,
            frame_id,
            ..
        } = environment;
        let network_partition_key = active_subresource_network_partition_key(host, owner);
        let policy_context = effective_subresource_policy_context(scope, host, owner);
        let request_cookie_report = observe_subresource_request_cookie_report(
            resource_loader.request_client(),
            &document_url,
            &request_origin,
            &request_url,
            "GET",
            options.credentials_mode,
        );
        let request = Request::new("GET", request_url.as_str(), None, Vec::new())
            .context("failed to create subresource GET request")?
            .with_initiator_url(&document_url)
            .with_request_origin(request_origin.clone())
            .with_resource_type(options.request_resource_type)
            .with_page_network_policy()
            .with_request_mode(options.request_mode)
            .with_credentials_mode(options.credentials_mode)
            .with_network_partition_key(network_partition_key.clone())
            .with_redirect_mode(RequestRedirectMode::Follow)
            .with_browser_request_metadata(options.metadata)
            .with_subframe_context(frame_id.is_some());
        let pending_info = PendingSubresourceFetchInfo {
            internal_id: 0,
            network_request_handle: None,
            frame_id: frame_id.clone(),
            document_url: document_url.clone(),
            url: request_url,
            websocket_socket_id: None,
            method: "GET".to_owned(),
            request_headers: Default::default(),
            request_body: None,
            request_body_bytes: None,
            resource_type: options.resource_type,
            request_cookie_report,
        };
        Ok(Self {
            cancel_handle: FetchCancelHandle::new(),
            network_partition_key,
            network_context: AsyncSubresourceNetworkContext {
                request_origin,
                frame_id,
                document_url,
                resource_type: options.resource_type,
                policy_context,
            },
            pending_info,
            request,
            resource_loader: resource_loader.clone(),
            owner,
            destination: options.destination,
        })
    }

    pub(super) fn dispatch(
        self,
        host: &mut JsContextHost,
        internal_id: u64,
        service_worker_failure: &str,
    ) {
        let loader = self.resource_loader.request_client().clone();
        let task_runner = self.resource_loader.task_runner();
        let info = self.pending_info;
        let client_id = host.service_worker_client_id_for_subresource_owner(self.owner);
        if matches!(info.url.scheme(), "http" | "https")
            && host
                .service_worker_controller_for_fetch(client_id, &info.document_url, &info.url)
                .is_some()
        {
            let dispatch = ServiceWorkerFetchDispatch {
                internal_id,
                request: host.service_worker_fetch_request(
                    client_id,
                    info.url.clone(),
                    info.method.clone(),
                    Vec::new(),
                    None,
                    self.destination,
                    self.request.request_mode,
                    self.request.credentials_mode,
                    RequestRedirectMode::Follow,
                    self.request.priority_hints.fetch_priority,
                    service_worker_fetch_request_metadata(&self.request),
                ),
                cors_preflight_request_headers: Vec::new(),
                request_cookie_report: info.request_cookie_report,
                network_context: self.network_context,
                completion_tx: host.resource_completion_sender(),
                request_client: loader,
                resource_task_runner: task_runner,
                cancel_handle: self.cancel_handle,
                direct_completion_tx: None,
            };
            if !host.dispatch_service_worker_fetch(dispatch) {
                let _ = host.resource_completion_sender().send_async_subresource(
                    AsyncSubresourceFetchCompletion {
                        internal_id,
                        request_url: info.url,
                        request_method: info.method,
                        request_headers: info.request_headers,
                        request_body: info.request_body,
                        response_status_text: None,
                        skip_fetch_security_validation: false,
                        response_filter: None,
                        network_error_text: None,
                        result: Err(service_worker_failure.to_owned()).into(),
                    },
                );
            }
            return;
        }
        spawn_async_subresource_fetch(
            task_runner,
            host.resource_completion_sender(),
            loader,
            self.request,
            Some(self.cancel_handle),
            Vec::new(),
            internal_id,
            self.network_context,
            info.url,
            info.method,
            info.request_headers,
            info.request_body,
        );
    }
}
