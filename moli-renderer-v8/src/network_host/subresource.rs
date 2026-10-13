use super::*;
use crate::{
    native_bridge::{OwnerDispatchScope, PageScriptHost},
    network::context::{DocumentResourceLoader, SubresourceRequestEnvironment},
    service_worker_runtime::{
        ServiceWorkerClientId, ServiceWorkerFetchDispatch, ServiceWorkerRequestDestination,
        service_worker_fetch_request_metadata,
    },
    types::{
        AsyncSubresourceFetchCompletion, AsyncSubresourceNetworkContext,
        PendingSubresourceFetchInfo, SubresourceFetchRegistration, SubresourceResourceType,
    },
};
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

    pub(super) const AUDIO: Self = Self {
        resource_type: SubresourceResourceType::Audio,
        request_resource_type: RequestResourceType::Media,
        metadata: BrowserRequestMetadata::Audio,
        destination: ServiceWorkerRequestDestination::Audio,
        ..Self::CSS_IMAGE
    };

    pub(super) const VIDEO: Self = Self {
        resource_type: SubresourceResourceType::Video,
        metadata: BrowserRequestMetadata::Video,
        destination: ServiceWorkerRequestDestination::Video,
        ..Self::AUDIO
    };

    pub(super) const TEXT_TRACK: Self = Self {
        resource_type: SubresourceResourceType::TextTrack,
        request_resource_type: RequestResourceType::TextTrack,
        metadata: BrowserRequestMetadata::TextTrack,
        destination: ServiceWorkerRequestDestination::Track,
        ..Self::CSS_IMAGE
    };

    pub(super) fn with_request_modes(
        self,
        request_mode: RequestMode,
        credentials_mode: RequestCredentialsMode,
    ) -> Self {
        Self {
            request_mode,
            credentials_mode,
            ..self
        }
    }
}

/// Captures one resource request without starting it. The caller first admits
/// it through CSP/local-resource policy and binds its own pending continuation,
/// then dispatches using the assigned request identity.
pub(super) struct PreparedSubresourceFetch {
    pub(super) registration: SubresourceFetchRegistration,
    pub(super) network_partition_key: Option<String>,
    pub(super) network_context: AsyncSubresourceNetworkContext,
    pub(super) pending_info: PendingSubresourceFetchInfo,
    pub(super) dispatch: SubresourceRequestDispatch,
    resource_loader: DocumentResourceLoader,
}

impl PreparedSubresourceFetch {
    pub(super) fn prepare(
        scope: &mut v8::PinScope<'_, '_>,
        host: &PageScriptHost,
        owner: OwnerDispatchScope,
        resource_loader: &DocumentResourceLoader,
        environment: SubresourceRequestEnvironment,
        request_url: url::Url,
        options: SubresourceFetchOptions,
    ) -> Self {
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
        let request = Request::new_browser(
            "GET",
            request_url.clone(),
            None,
            Vec::new(),
            request_origin.clone(),
        )
        .with_initiator_url(&document_url)
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
        Self {
            registration: if host.should_intercept_subresource(options.resource_type) {
                SubresourceFetchRegistration::Intercepted
            } else {
                SubresourceFetchRegistration::Direct(FetchCancelHandle::new())
            },
            network_partition_key,
            network_context: AsyncSubresourceNetworkContext {
                request_origin,
                frame_id,
                document_url,
                resource_type: options.resource_type,
                policy_context,
            },
            pending_info,
            dispatch: SubresourceRequestDispatch {
                request,
                client_id: host.service_worker_client_id_for_subresource_owner(owner),
                destination: options.destination,
            },
            resource_loader: resource_loader.clone(),
        }
    }

    pub(super) fn with_service_worker_client(mut self, client_id: ServiceWorkerClientId) -> Self {
        self.dispatch.client_id = client_id;
        self
    }

    pub(super) fn dispatch(self, host: &PageScriptHost, internal_id: u64) {
        let SubresourceFetchRegistration::Direct(cancel_handle) = self.registration else {
            return;
        };
        self.dispatch.dispatch(
            host,
            internal_id,
            self.resource_loader.request_client().clone(),
            self.resource_loader.task_runner(),
            self.network_context,
            self.pending_info.request_cookie_report,
            cancel_handle,
        );
    }
}

/// Execution data captured before exposing a pause. Both direct dispatch and
/// Fetch.continueRequest use this path without re-entering interception.
#[derive(Clone)]
pub(crate) struct SubresourceRequestDispatch {
    pub(crate) request: Request,
    client_id: ServiceWorkerClientId,
    destination: ServiceWorkerRequestDestination,
}

#[derive(Debug, thiserror::Error)]
#[error("service worker subresource dispatch failed")]
struct ServiceWorkerSubresourceDispatchError;

impl SubresourceRequestDispatch {
    pub(crate) fn with_request(&self, request: Request) -> Self {
        Self {
            request,
            client_id: self.client_id,
            destination: self.destination,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dispatch(
        self,
        host: &PageScriptHost,
        internal_id: u64,
        loader: ResourceRequestClient,
        task_runner: crate::network::RendererResourceTaskRunner,
        network_context: AsyncSubresourceNetworkContext,
        request_cookie_report: Option<moli_cookie_jar::StoredCookieQueryReport>,
        cancel_handle: FetchCancelHandle,
    ) {
        let request = self.request;
        let request_url = request.url.clone();
        let request_method = request.method.clone();
        let request_headers = request.request_headers.clone();
        let request_body = request
            .body
            .as_ref()
            .map(|body| String::from_utf8_lossy(body).into_owned());
        // Use the initiating client's identity and frozen loader, including its
        // bypass setting, rather than looking up the currently installed Page.
        let browser_context_runtime = host.browser_context_runtime();
        if matches!(request_url.scheme(), "http" | "https")
            && !loader.bypass_service_worker()
            && browser_context_runtime
                .service_worker_controller_for_fetch(self.client_id, &request_url)
                .is_some()
        {
            let dispatch = ServiceWorkerFetchDispatch {
                internal_id,
                request: host.service_worker_fetch_request(
                    self.client_id,
                    request_url.clone(),
                    request_method.clone(),
                    request_headers.clone(),
                    request.body.clone(),
                    self.destination,
                    request.request_mode,
                    request.credentials_mode,
                    request.redirect_mode(),
                    request.priority_hints.fetch_priority,
                    service_worker_fetch_request_metadata(&request),
                ),
                cors_preflight_request_headers: Vec::new(),
                request_cookie_report,
                network_context,
                completion_tx: host.resource_completion_sender(),
                request_client: loader,
                resource_task_runner: task_runner,
                cancel_handle,
                direct_completion_tx: None,
            };
            if !browser_context_runtime.dispatch_service_worker_fetch_request(dispatch, request) {
                let _ = host.resource_completion_sender().send_async_subresource(
                    AsyncSubresourceFetchCompletion {
                        internal_id,
                        request_url,
                        request_method,
                        request_headers,
                        request_body,
                        response_status_text: None,
                        skip_fetch_security_validation: false,
                        response_filter: None,
                        network_error_text: None,
                        result: Err(ServiceWorkerSubresourceDispatchError.to_string()).into(),
                    },
                );
            }
            return;
        }
        spawn_async_subresource_fetch(
            task_runner,
            host.resource_completion_sender(),
            loader,
            request,
            Some(cancel_handle),
            Vec::new(),
            internal_id,
            network_context,
            request_url,
            request_method,
            request_headers,
            request_body,
        );
    }
}
