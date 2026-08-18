use anyhow::{Result, anyhow};
use moli_fetch::{BrowserNavigationRequestKind, FetchCancelHandle, Request};

use crate::network::{
    RendererResourceTaskRunner, ResourceRequestClient, navigation::NavigationResourceLoader,
};
use crate::service_worker_runtime::{
    ServiceWorkerClientFrameType, ServiceWorkerClientId, ServiceWorkerClientType,
    ServiceWorkerControlState, ServiceWorkerDirectFetchResponse, ServiceWorkerDirectFetchResult,
    ServiceWorkerFetchDispatch, ServiceWorkerFetchRequest, ServiceWorkerNavigationPreloadState,
    ServiceWorkerNavigationPreloadStateError, ServiceWorkerNotificationAction,
    ServiceWorkerNotificationMetadata, ServiceWorkerPushSubscriptionSnapshot,
    ServiceWorkerRegistrationSnapshot, ServiceWorkerRequestDestination,
    ServiceWorkerUnregisterStart, ServiceWorkerVersionId, service_worker_fetch_request_metadata,
};
use crate::structured_clone::V8StructuredClonePayload;
use crate::types::{AsyncSubresourceNetworkContext, SubresourceResourceType};
use url::Url;

use super::RendererBrowserContextRuntime;

pub struct RendererReservedServiceWorkerClient {
    browser_context_runtime: RendererBrowserContextRuntime,
    client_id: Option<ServiceWorkerClientId>,
}

impl RendererReservedServiceWorkerClient {
    fn new(
        browser_context_runtime: RendererBrowserContextRuntime,
        client_id: ServiceWorkerClientId,
    ) -> Self {
        Self {
            browser_context_runtime,
            client_id: Some(client_id),
        }
    }

    pub(crate) fn release(mut self) -> ServiceWorkerClientId {
        self.client_id
            .take()
            .expect("reserved service worker client already released")
    }
}

impl std::fmt::Debug for RendererReservedServiceWorkerClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererReservedServiceWorkerClient")
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

impl Drop for RendererReservedServiceWorkerClient {
    fn drop(&mut self) {
        if let Some(client_id) = self.client_id.take() {
            self.browser_context_runtime
                .unregister_service_worker_client(client_id);
        }
    }
}

#[derive(Debug)]
pub struct RendererServiceWorkerMainResourceFetch {
    pub reserved_client: Option<RendererReservedServiceWorkerClient>,
    pub response: Option<crate::protocol_types::NavigationResponse>,
}

impl RendererBrowserContextRuntime {
    pub async fn fetch_service_worker_main_resource_for_navigation(
        &self,
        request: &Request,
        navigation_loader: &NavigationResourceLoader,
    ) -> Result<RendererServiceWorkerMainResourceFetch> {
        let bypass_service_worker = navigation_loader.request_client().bypass_service_worker();
        if !matches!(request.url.scheme(), "http" | "https") {
            return Ok(RendererServiceWorkerMainResourceFetch {
                reserved_client: None,
                response: None,
            });
        }

        let storage_key =
            moli_storage_key::MoliStorageKey::first_party_from_url(&request.url, None)
                .serialized_storage_key();
        let completion_tx =
            crate::page_task_queue::RendererResourceCompletionSender::direct_completion_only();
        let client_id = if bypass_service_worker {
            self.register_reserved_service_worker_client_bypassing_service_worker(
                request.url.clone(),
                storage_key,
                ServiceWorkerClientFrameType::TopLevel,
                None,
            )
        } else {
            self.register_reserved_service_worker_client(
                request.url.clone(),
                storage_key,
                ServiceWorkerClientFrameType::TopLevel,
                None,
            )
        };
        let reserved_client = RendererReservedServiceWorkerClient::new(self.clone(), client_id);
        if bypass_service_worker {
            return Ok(RendererServiceWorkerMainResourceFetch {
                reserved_client: Some(reserved_client),
                response: None,
            });
        }
        let Some(controller) = self.service_worker_controller_for_client(client_id) else {
            drop(reserved_client);
            return Ok(RendererServiceWorkerMainResourceFetch {
                reserved_client: None,
                response: None,
            });
        };
        if let Some(force_update_rx) =
            self.force_update_service_worker_registration_for_page_load(controller.scope_url())
        {
            let _ = force_update_rx.await;
        }
        if self
            .service_worker_controller_for_client(client_id)
            .is_none()
        {
            drop(reserved_client);
            return Ok(RendererServiceWorkerMainResourceFetch {
                reserved_client: None,
                response: None,
            });
        }

        let response = self
            .fetch_service_worker_main_resource_for_reserved_client(
                client_id,
                request,
                navigation_loader.request_client(),
                navigation_loader.task_runner(),
                completion_tx,
                ServiceWorkerRequestDestination::Document,
            )
            .await?;
        Ok(RendererServiceWorkerMainResourceFetch {
            reserved_client: Some(reserved_client),
            response,
        })
    }

    pub(crate) async fn fetch_service_worker_child_main_resource_for_reserved_client(
        &self,
        client_id: ServiceWorkerClientId,
        request: &Request,
        request_client: &ResourceRequestClient,
        resource_task_runner: RendererResourceTaskRunner,
        completion_tx: crate::page_task_queue::RendererResourceCompletionSender,
    ) -> Result<Option<crate::protocol_types::NavigationResponse>> {
        self.fetch_service_worker_main_resource_for_reserved_client(
            client_id,
            request,
            request_client,
            resource_task_runner,
            completion_tx,
            ServiceWorkerRequestDestination::Iframe,
        )
        .await
    }

    async fn fetch_service_worker_main_resource_for_reserved_client(
        &self,
        client_id: ServiceWorkerClientId,
        request: &Request,
        request_client: &ResourceRequestClient,
        resource_task_runner: RendererResourceTaskRunner,
        completion_tx: crate::page_task_queue::RendererResourceCompletionSender,
        destination: ServiceWorkerRequestDestination,
    ) -> Result<Option<crate::protocol_types::NavigationResponse>> {
        if !matches!(request.url.scheme(), "http" | "https") {
            return Ok(None);
        }
        if self
            .service_worker_controller_for_client(client_id)
            .is_none()
        {
            return Ok(None);
        }

        let (direct_completion_tx, main_resource_completion_rx) = tokio::sync::oneshot::channel();
        let request_body_text = request
            .body
            .as_ref()
            .map(|body| String::from_utf8_lossy(body).into_owned());
        let redirect_mode =
            main_resource_fetch_event_redirect_mode(destination, request.redirect_mode);
        let dispatch = ServiceWorkerFetchDispatch {
            internal_id: 0,
            request: ServiceWorkerFetchRequest {
                client_id,
                resulting_client_id: Some(client_id),
                url: request.url.clone(),
                method: request.method.clone(),
                headers: request.request_headers.clone(),
                body: request.body.clone(),
                destination,
                request_mode: request.request_mode,
                credentials_mode: request.credentials_mode,
                redirect_mode,
                priority: request.priority_hints.fetch_priority,
                is_reload: request.browser_navigation_kind()
                    == BrowserNavigationRequestKind::Reload,
                metadata: service_worker_fetch_request_metadata(request),
            },
            request_body_text,
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report: None,
            network_context: AsyncSubresourceNetworkContext {
                frame_id: None,
                document_url: request.url.clone(),
                resource_type: SubresourceResourceType::Fetch,
                policy_context: Default::default(),
            },
            completion_tx,
            request_client: request_client.clone(),
            resource_task_runner,
            cancel_handle: FetchCancelHandle::new(),
            direct_completion_tx: Some(direct_completion_tx),
        };

        if !self.dispatch_service_worker_fetch(dispatch) {
            return Ok(None);
        }

        match main_resource_completion_rx.await {
            Ok(ServiceWorkerDirectFetchResult::Fallback) => Ok(None),
            Ok(ServiceWorkerDirectFetchResult::Response(response)) => Ok(Some(*response.response)),
            Ok(ServiceWorkerDirectFetchResult::Failure(message)) => Err(anyhow!(message)),
            Err(_) => Err(anyhow!(
                "service worker main resource fetch completion channel closed"
            )),
        }
    }

    pub(crate) async fn fetch_service_worker_subresource_for_client(
        &self,
        client_id: ServiceWorkerClientId,
        document_url: Url,
        request: &Request,
        request_client: &ResourceRequestClient,
        resource_task_runner: RendererResourceTaskRunner,
        destination: ServiceWorkerRequestDestination,
        resource_type: SubresourceResourceType,
    ) -> Result<Option<crate::protocol_types::NavigationResponse>> {
        self.fetch_service_worker_subresource_for_client_with_metadata(
            client_id,
            document_url,
            request,
            request_client,
            resource_task_runner,
            destination,
            resource_type,
        )
        .await
        .map(|response| response.map(|response| *response.response))
    }

    pub(crate) async fn fetch_service_worker_subresource_for_client_with_metadata(
        &self,
        client_id: ServiceWorkerClientId,
        document_url: Url,
        request: &Request,
        request_client: &ResourceRequestClient,
        resource_task_runner: RendererResourceTaskRunner,
        destination: ServiceWorkerRequestDestination,
        resource_type: SubresourceResourceType,
    ) -> Result<Option<ServiceWorkerDirectFetchResponse>> {
        if !matches!(request.url.scheme(), "http" | "https") {
            return Ok(None);
        }
        if self
            .service_worker_controller_for_fetch(client_id, &request.url)
            .is_none()
        {
            return Ok(None);
        }

        let completion_tx =
            crate::page_task_queue::RendererResourceCompletionSender::direct_completion_only();
        let (direct_completion_tx, direct_completion_rx) = tokio::sync::oneshot::channel();
        let request_body_text = request
            .body
            .as_ref()
            .map(|body| String::from_utf8_lossy(body).into_owned());
        let dispatch = ServiceWorkerFetchDispatch {
            internal_id: 0,
            request: ServiceWorkerFetchRequest {
                client_id,
                resulting_client_id: None,
                url: request.url.clone(),
                method: request.method.clone(),
                headers: request.request_headers.clone(),
                body: request.body.clone(),
                destination,
                request_mode: request.request_mode,
                credentials_mode: request.credentials_mode,
                redirect_mode: request.redirect_mode,
                priority: request.priority_hints.fetch_priority,
                is_reload: false,
                metadata: service_worker_fetch_request_metadata(request),
            },
            request_body_text,
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report: None,
            network_context: AsyncSubresourceNetworkContext {
                frame_id: None,
                document_url,
                resource_type,
                policy_context: Default::default(),
            },
            completion_tx,
            request_client: request_client.clone(),
            resource_task_runner,
            cancel_handle: FetchCancelHandle::new(),
            direct_completion_tx: Some(direct_completion_tx),
        };

        if !self.dispatch_service_worker_fetch(dispatch) {
            return Ok(None);
        }

        match direct_completion_rx.await {
            Ok(ServiceWorkerDirectFetchResult::Fallback) => Ok(None),
            Ok(ServiceWorkerDirectFetchResult::Response(response)) => Ok(Some(response)),
            Ok(ServiceWorkerDirectFetchResult::Failure(message)) => Err(anyhow!(message)),
            Err(_) => Err(anyhow!(
                "service worker subresource fetch completion channel closed"
            )),
        }
    }

    fn force_update_service_worker_registration_for_page_load(
        &self,
        scope_url: &Url,
    ) -> Option<tokio::sync::oneshot::Receiver<()>> {
        let (_, receiver) = self
            .inner
            .service_worker_runtime
            .devtools_force_update_registration_for_page_load(scope_url, self.clone());
        receiver
    }

    pub(crate) async fn fetch_service_worker_main_resource_for_worker(
        &self,
        client_id: ServiceWorkerClientId,
        request: &Request,
        request_client: &ResourceRequestClient,
        resource_task_runner: RendererResourceTaskRunner,
        destination: ServiceWorkerRequestDestination,
    ) -> Result<Option<crate::protocol_types::NavigationResponse>> {
        self.inner
            .service_worker_runtime
            .fetch_main_resource_for_worker_client(
                client_id,
                request,
                request_client,
                resource_task_runner,
                destination,
                FetchCancelHandle::new(),
            )
            .await
            .map_err(|message| anyhow!(message))
    }

    pub(crate) fn register_service_worker_client(
        &self,
        document_url: Url,
        storage_key: String,
        frame_type: ServiceWorkerClientFrameType,
        document_owner: Option<crate::window_document_identity::WindowDocumentOwner>,
        completion_tx: crate::page_task_queue::RendererPageServiceWorkerTaskSender,
    ) -> ServiceWorkerClientId {
        self.inner
            .service_worker_runtime
            .register_client_with_storage_key(
                document_url,
                storage_key,
                frame_type,
                document_owner,
                completion_tx,
            )
    }

    pub(crate) fn register_reserved_service_worker_client(
        &self,
        document_url: Url,
        storage_key: String,
        frame_type: ServiceWorkerClientFrameType,
        document_owner: Option<crate::window_document_identity::WindowDocumentOwner>,
    ) -> ServiceWorkerClientId {
        self.inner
            .service_worker_runtime
            .register_reserved_client_with_storage_key(
                document_url,
                storage_key,
                frame_type,
                document_owner,
            )
    }

    fn register_reserved_service_worker_client_bypassing_service_worker(
        &self,
        document_url: Url,
        storage_key: String,
        frame_type: ServiceWorkerClientFrameType,
        document_owner: Option<crate::window_document_identity::WindowDocumentOwner>,
    ) -> ServiceWorkerClientId {
        self.inner
            .service_worker_runtime
            .register_reserved_client_with_storage_key_bypassing_service_worker(
                document_url,
                storage_key,
                frame_type,
                document_owner,
            )
    }

    pub(crate) fn register_reserved_service_worker_worker_client(
        &self,
        script_url: Url,
        storage_key: String,
        client_type: ServiceWorkerClientType,
        secure_context: bool,
    ) -> ServiceWorkerClientId {
        self.inner
            .service_worker_runtime
            .register_reserved_worker_client_with_storage_key(
                script_url,
                storage_key,
                client_type,
                secure_context,
            )
    }

    pub(crate) fn register_reserved_service_worker_worker_client_inheriting_controller(
        &self,
        script_url: Url,
        storage_key: String,
        client_type: ServiceWorkerClientType,
        secure_context: bool,
        parent_client_id: ServiceWorkerClientId,
    ) -> Option<ServiceWorkerClientId> {
        self.inner
            .service_worker_runtime
            .register_reserved_worker_client_inheriting_controller_from_client(
                script_url,
                storage_key,
                client_type,
                secure_context,
                parent_client_id,
            )
    }

    pub(crate) fn unregister_service_worker_client(&self, client_id: ServiceWorkerClientId) {
        self.inner
            .service_worker_runtime
            .unregister_client(client_id);
    }

    pub(crate) fn update_service_worker_client_document(
        &self,
        client_id: ServiceWorkerClientId,
        document_url: Url,
        storage_key: String,
        frame_type: ServiceWorkerClientFrameType,
        document_owner: Option<crate::window_document_identity::WindowDocumentOwner>,
    ) -> bool {
        self.inner
            .service_worker_runtime
            .update_client_document_with_storage_key(
                client_id,
                document_url,
                storage_key,
                frame_type,
                document_owner,
            )
    }

    pub(crate) fn update_service_worker_client_document_and_page_endpoint(
        &self,
        client_id: ServiceWorkerClientId,
        document_url: Url,
        storage_key: String,
        frame_type: ServiceWorkerClientFrameType,
        document_owner: Option<crate::window_document_identity::WindowDocumentOwner>,
        completion_tx: crate::page_task_queue::RendererPageServiceWorkerTaskSender,
    ) -> bool {
        self.inner
            .service_worker_runtime
            .update_client_document_with_storage_key_and_completion_sender(
                client_id,
                document_url,
                storage_key,
                frame_type,
                document_owner,
                completion_tx,
            )
    }

    pub(crate) fn unregister_service_worker_scope(
        &self,
        scope_url: &Url,
        storage_key: String,
        request_id: u64,
        document_owner: crate::window_document_identity::WindowDocumentOwner,
        completion_tx: crate::page_task_queue::RendererPageServiceWorkerTaskSender,
    ) -> ServiceWorkerUnregisterStart {
        self.inner
            .service_worker_runtime
            .start_unregistration_with_storage_key(
                scope_url,
                storage_key,
                request_id,
                document_owner,
                completion_tx,
            )
    }

    pub(crate) fn service_worker_registration_for_client(
        &self,
        client_url: &Url,
        storage_key: &str,
    ) -> Option<ServiceWorkerRegistrationSnapshot> {
        self.inner
            .service_worker_runtime
            .matching_registration_for_client_with_storage_key(client_url, storage_key)
    }

    pub(crate) fn service_worker_registrations(
        &self,
        document_url: &Url,
        storage_key: &str,
    ) -> Vec<ServiceWorkerRegistrationSnapshot> {
        self.inner
            .service_worker_runtime
            .all_registrations_with_storage_key(document_url, storage_key)
    }

    pub(crate) fn service_worker_navigation_preload_state(
        &self,
        scope_url: &Url,
    ) -> Option<ServiceWorkerNavigationPreloadState> {
        self.inner
            .service_worker_runtime
            .navigation_preload_state_for_scope(scope_url)
    }

    pub(crate) fn set_service_worker_navigation_preload_enabled(
        &self,
        scope_url: &Url,
        enabled: bool,
    ) -> Result<(), ServiceWorkerNavigationPreloadStateError> {
        self.inner
            .service_worker_runtime
            .set_navigation_preload_enabled_for_scope(scope_url, enabled)
    }

    pub(crate) fn set_service_worker_navigation_preload_header_value(
        &self,
        scope_url: &Url,
        header_value: String,
    ) -> Result<(), ServiceWorkerNavigationPreloadStateError> {
        self.inner
            .service_worker_runtime
            .set_navigation_preload_header_value_for_scope(scope_url, header_value)
    }

    pub(crate) fn watch_service_worker_ready_registration(
        &self,
        document_url: Url,
        storage_key: String,
        request_id: u64,
        document_owner: crate::window_document_identity::WindowDocumentOwner,
        completion_tx: crate::page_task_queue::RendererPageServiceWorkerTaskSender,
    ) -> bool {
        self.inner
            .service_worker_runtime
            .watch_ready_registration_with_storage_key(
                document_url,
                storage_key,
                request_id,
                document_owner,
                completion_tx,
            )
    }

    pub(crate) fn watch_service_worker_registration_lifecycle(
        &self,
        scope_url: Url,
        storage_key: String,
        document_owner: crate::window_document_identity::WindowDocumentOwner,
        completion_tx: crate::page_task_queue::RendererPageServiceWorkerTaskSender,
    ) {
        self.inner
            .service_worker_runtime
            .watch_registration_lifecycle(scope_url, storage_key, document_owner, completion_tx)
    }

    pub(crate) fn service_worker_controller_for_client(
        &self,
        client_id: ServiceWorkerClientId,
    ) -> Option<ServiceWorkerControlState> {
        self.inner
            .service_worker_runtime
            .matching_controller_for_client(client_id)
    }

    pub(crate) fn service_worker_controller_for_fetch(
        &self,
        client_id: ServiceWorkerClientId,
        request_url: &Url,
    ) -> Option<ServiceWorkerControlState> {
        self.inner
            .service_worker_runtime
            .matching_controller_for_client_fetch(client_id, request_url)
    }

    pub(crate) fn dispatch_service_worker_fetch(
        &self,
        dispatch: ServiceWorkerFetchDispatch,
    ) -> bool {
        self.inner
            .service_worker_runtime
            .dispatch_controlled_fetch(dispatch)
    }

    pub(crate) fn abort_service_worker_fetch(&self, internal_id: u64) -> bool {
        self.inner
            .service_worker_runtime
            .abort_controlled_fetch(internal_id)
    }

    pub(crate) fn abort_service_worker_fetch_with_reason(
        &self,
        internal_id: u64,
        reason: Option<V8StructuredClonePayload>,
    ) -> bool {
        self.inner
            .service_worker_runtime
            .abort_controlled_fetch_with_reason(internal_id, reason)
    }

    pub(crate) fn dispatch_service_worker_message(
        &self,
        version_id: ServiceWorkerVersionId,
        source_client_id: ServiceWorkerClientId,
        source_origin: Option<String>,
        payload: V8StructuredClonePayload,
    ) -> bool {
        self.inner
            .service_worker_runtime
            .dispatch_message_to_version(version_id, source_client_id, source_origin, payload)
    }

    pub fn dispatch_service_worker_notification_click(
        &self,
        scope_url: &Url,
        title: impl Into<String>,
        action: impl Into<String>,
    ) -> bool {
        self.inner
            .service_worker_runtime
            .dispatch_notification_click_for_scope(scope_url, title.into(), action.into())
    }

    pub fn dispatch_service_worker_notification_close(
        &self,
        scope_url: &Url,
        title: impl Into<String>,
    ) -> bool {
        self.inner
            .service_worker_runtime
            .dispatch_notification_close_for_scope(scope_url, title.into())
    }

    pub fn dispatch_service_worker_push(&self, scope_url: &Url, data: Option<Vec<u8>>) -> bool {
        self.inner
            .service_worker_runtime
            .dispatch_push_for_scope(scope_url, data)
    }

    pub fn dispatch_service_worker_periodic_sync(&self, scope_url: &Url, tag: &str) -> bool {
        self.inner
            .service_worker_runtime
            .dispatch_periodic_sync_for_scope(scope_url, tag)
    }

    pub(crate) fn register_service_worker_sync(&self, scope_url: &Url, tag: String) -> bool {
        self.inner
            .service_worker_runtime
            .register_sync_for_scope(scope_url, tag)
    }

    pub(crate) fn service_worker_sync_tags(&self, scope_url: &Url) -> Vec<String> {
        self.inner
            .service_worker_runtime
            .sync_tags_for_scope(scope_url)
    }

    pub(crate) fn register_service_worker_periodic_sync(
        &self,
        scope_url: &Url,
        tag: String,
        min_interval_ms: u64,
    ) -> bool {
        self.inner
            .service_worker_runtime
            .register_periodic_sync_for_scope(scope_url, tag, min_interval_ms)
    }

    pub(crate) fn service_worker_periodic_sync_tags(&self, scope_url: &Url) -> Vec<String> {
        self.inner
            .service_worker_runtime
            .periodic_sync_tags_for_scope(scope_url)
    }

    pub(crate) fn unregister_service_worker_periodic_sync(
        &self,
        scope_url: &Url,
        tag: &str,
    ) -> bool {
        self.inner
            .service_worker_runtime
            .unregister_periodic_sync_for_scope(scope_url, tag)
    }

    pub(crate) fn subscribe_service_worker_push(
        &self,
        scope_url: &Url,
        user_visible_only: bool,
    ) -> Option<ServiceWorkerPushSubscriptionSnapshot> {
        self.inner
            .service_worker_runtime
            .subscribe_push_for_scope(scope_url, user_visible_only)
    }

    pub(crate) fn service_worker_push_subscription(
        &self,
        scope_url: &Url,
    ) -> Option<ServiceWorkerPushSubscriptionSnapshot> {
        self.inner
            .service_worker_runtime
            .push_subscription_for_scope(scope_url)
    }

    pub(crate) fn unsubscribe_service_worker_push(&self, scope_url: &Url) -> bool {
        self.inner
            .service_worker_runtime
            .unsubscribe_push_for_scope(scope_url)
    }

    pub(crate) fn show_service_worker_notification(
        &self,
        scope_url: &Url,
        title: impl Into<String>,
        tag: impl Into<String>,
        metadata: ServiceWorkerNotificationMetadata,
        actions: Vec<ServiceWorkerNotificationAction>,
        data: V8StructuredClonePayload,
    ) -> bool {
        self.inner
            .service_worker_runtime
            .show_notification_for_scope(
                scope_url,
                title.into(),
                tag.into(),
                metadata,
                actions,
                data,
            )
    }

    pub(crate) fn service_worker_notifications(
        &self,
        scope_url: &Url,
        tag: Option<&str>,
    ) -> Vec<crate::runtime::ServiceWorkerNotificationSnapshot> {
        self.inner
            .service_worker_runtime
            .notifications_for_scope(scope_url, tag)
    }

    pub(crate) fn close_service_worker_notification(
        &self,
        registration_id: crate::runtime::ServiceWorkerRegistrationId,
        notification_id: u64,
    ) -> bool {
        self.inner
            .service_worker_runtime
            .close_notification(registration_id, notification_id)
    }

    #[cfg(test)]
    pub(crate) fn stop_service_worker_hosts_for_test(&self) {
        self.inner
            .service_worker_runtime
            .stop_all_running_hosts_for_test();
    }
}

fn main_resource_fetch_event_redirect_mode(
    destination: ServiceWorkerRequestDestination,
    request_redirect_mode: moli_fetch::RequestRedirectMode,
) -> moli_fetch::RequestRedirectMode {
    match destination {
        ServiceWorkerRequestDestination::Document | ServiceWorkerRequestDestination::Iframe => {
            moli_fetch::RequestRedirectMode::Manual
        }
        _ => request_redirect_mode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_fetch::RequestRedirectMode;

    #[test]
    fn main_resource_fetch_event_redirect_mode_is_manual_for_navigations() {
        assert_eq!(
            main_resource_fetch_event_redirect_mode(
                ServiceWorkerRequestDestination::Document,
                RequestRedirectMode::Follow,
            ),
            RequestRedirectMode::Manual
        );
        assert_eq!(
            main_resource_fetch_event_redirect_mode(
                ServiceWorkerRequestDestination::Iframe,
                RequestRedirectMode::Error,
            ),
            RequestRedirectMode::Manual
        );
    }

    #[test]
    fn main_resource_fetch_event_redirect_mode_preserves_non_navigation_requests() {
        assert_eq!(
            main_resource_fetch_event_redirect_mode(
                ServiceWorkerRequestDestination::Empty,
                RequestRedirectMode::Follow,
            ),
            RequestRedirectMode::Follow
        );
        assert_eq!(
            main_resource_fetch_event_redirect_mode(
                ServiceWorkerRequestDestination::Worker,
                RequestRedirectMode::Error,
            ),
            RequestRedirectMode::Error
        );
    }
}
