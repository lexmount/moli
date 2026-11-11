use moli_fetch::{Request, RequestCacheMode, RequestMode};
use url::Url;

use crate::types::NetworkBodySourceId;
use crate::{
    network::ResourceRequestClient, page_task_queue::RendererResourceCompletionSender,
    structured_clone::V8StructuredClonePayload, types::AsyncSubresourceNetworkContext,
};

use super::{
    clients::ServiceWorkerClientSnapshot,
    ids::{
        ServiceWorkerClientId, ServiceWorkerEventId, ServiceWorkerRegistrationId,
        ServiceWorkerVersionId,
    },
    run_owner::ServiceWorkerRunOwner,
    snapshots::ServiceWorkerVersionSnapshot,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerLifecycleEventKind {
    Install,
    Activate,
}

impl ServiceWorkerLifecycleEventKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Activate => "activate",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerLifecycleEvent {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) owner: ServiceWorkerRunOwner,
    pub(crate) kind: ServiceWorkerLifecycleEventKind,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerLifecycleCompletion {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) owner: ServiceWorkerRunOwner,
    pub(crate) kind: ServiceWorkerLifecycleEventKind,
    pub(crate) result: Result<(), String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerMessageEvent {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) owner: ServiceWorkerRunOwner,
    pub(crate) source_client_id: Option<ServiceWorkerClientId>,
    pub(crate) source_client_url: Option<Url>,
    pub(crate) source_client_snapshot: Option<ServiceWorkerClientSnapshot>,
    pub(crate) source_worker: Option<ServiceWorkerVersionSnapshot>,
    pub(crate) source_origin: String,
    pub(crate) payload: V8StructuredClonePayload,
    pub(crate) window_interaction_allowed: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerNotificationEvent {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) kind: ServiceWorkerNotificationEventKind,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) owner: ServiceWorkerRunOwner,
    pub(crate) notification_id: u64,
    pub(crate) title: String,
    pub(crate) tag: String,
    pub(crate) metadata: ServiceWorkerNotificationMetadata,
    pub(crate) actions: Vec<ServiceWorkerNotificationAction>,
    pub(crate) action: String,
    pub(crate) data: V8StructuredClonePayload,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerNotificationEventKind {
    Click,
    Close,
}

impl ServiceWorkerNotificationEventKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Click => "notificationclick",
            Self::Close => "notificationclose",
        }
    }

    pub(crate) fn grants_window_interaction(self) -> bool {
        matches!(self, Self::Click)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerNotificationCompletion {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) owner: ServiceWorkerRunOwner,
    pub(crate) result: Result<(), String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerPushEvent {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) owner: ServiceWorkerRunOwner,
    pub(crate) data: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerPushCompletion {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) owner: ServiceWorkerRunOwner,
    pub(crate) result: Result<(), String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerSyncEvent {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) owner: ServiceWorkerRunOwner,
    pub(crate) tag: String,
    pub(crate) last_chance: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerSyncCompletion {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) owner: ServiceWorkerRunOwner,
    pub(crate) tag: String,
    pub(crate) result: Result<(), String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerPeriodicSyncEvent {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) owner: ServiceWorkerRunOwner,
    pub(crate) tag: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerPeriodicSyncCompletion {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) owner: ServiceWorkerRunOwner,
    pub(crate) tag: String,
    pub(crate) result: Result<(), String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerShowNotification {
    pub(crate) request_id: u64,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) title: String,
    pub(crate) tag: String,
    pub(crate) metadata: ServiceWorkerNotificationMetadata,
    pub(crate) actions: Vec<ServiceWorkerNotificationAction>,
    pub(crate) data: V8StructuredClonePayload,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerShowNotificationResult {
    pub(crate) request_id: u64,
    pub(crate) result: Result<(), String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerNotificationMetadata {
    pub(crate) dir: String,
    pub(crate) lang: String,
    pub(crate) body: String,
    pub(crate) icon: String,
    pub(crate) image: String,
    pub(crate) badge: String,
    pub(crate) vibrate: Vec<u32>,
    pub(crate) timestamp: Option<u64>,
    pub(crate) renotify: bool,
    pub(crate) silent: Option<bool>,
    pub(crate) require_interaction: bool,
}

impl Default for ServiceWorkerNotificationMetadata {
    fn default() -> Self {
        Self {
            dir: "auto".to_owned(),
            lang: String::new(),
            body: String::new(),
            icon: String::new(),
            image: String::new(),
            badge: String::new(),
            vibrate: Vec::new(),
            timestamp: None,
            renotify: false,
            silent: None,
            require_interaction: false,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerNotificationAction {
    pub(crate) action: String,
    pub(crate) title: String,
    pub(crate) icon: String,
    pub(crate) navigate: Option<Url>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerNotificationSnapshot {
    pub(crate) id: u64,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) title: String,
    pub(crate) tag: String,
    pub(crate) metadata: ServiceWorkerNotificationMetadata,
    pub(crate) actions: Vec<ServiceWorkerNotificationAction>,
    pub(crate) data: V8StructuredClonePayload,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerGetNotifications {
    pub(crate) request_id: u64,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) tag: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerGetNotificationsResult {
    pub(crate) request_id: u64,
    pub(crate) result: Result<Vec<ServiceWorkerNotificationSnapshot>, String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerSyncRegistration {
    pub(crate) request_id: u64,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) tag: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerSyncRegistrationResult {
    pub(crate) request_id: u64,
    pub(crate) result: Result<(), String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerSyncGetTags {
    pub(crate) request_id: u64,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) version_id: ServiceWorkerVersionId,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerSyncGetTagsResult {
    pub(crate) request_id: u64,
    pub(crate) result: Result<Vec<String>, String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerPeriodicSyncRegistration {
    pub(crate) request_id: u64,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) tag: String,
    pub(crate) min_interval_ms: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerPeriodicSyncRegistrationResult {
    pub(crate) request_id: u64,
    pub(crate) result: Result<(), String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerPeriodicSyncGetTags {
    pub(crate) request_id: u64,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) version_id: ServiceWorkerVersionId,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerPeriodicSyncGetTagsResult {
    pub(crate) request_id: u64,
    pub(crate) result: Result<Vec<String>, String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerPeriodicSyncUnregistration {
    pub(crate) request_id: u64,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) tag: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerPeriodicSyncUnregistrationResult {
    pub(crate) request_id: u64,
    pub(crate) result: Result<(), String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ServiceWorkerPushSubscriptionSnapshot {
    pub(crate) endpoint: String,
    pub(crate) user_visible_only: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerPushSubscribe {
    pub(crate) request_id: u64,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) user_visible_only: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerPushSubscribeResult {
    pub(crate) request_id: u64,
    pub(crate) result: Result<ServiceWorkerPushSubscriptionSnapshot, String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerPushGetSubscription {
    pub(crate) request_id: u64,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) version_id: ServiceWorkerVersionId,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerPushGetSubscriptionResult {
    pub(crate) request_id: u64,
    pub(crate) result: Result<Option<ServiceWorkerPushSubscriptionSnapshot>, String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerPushUnsubscribe {
    pub(crate) request_id: u64,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) version_id: ServiceWorkerVersionId,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerPushUnsubscribeResult {
    pub(crate) request_id: u64,
    pub(crate) result: Result<bool, String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerCloseNotification {
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) notification_id: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerClientMessage {
    pub(crate) source_version_id: ServiceWorkerVersionId,
    pub(crate) target_client_id: ServiceWorkerClientId,
    pub(crate) payload: V8StructuredClonePayload,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerWorkerMessage {
    pub(crate) source_version_id: ServiceWorkerVersionId,
    pub(crate) target_version_id: ServiceWorkerVersionId,
    pub(crate) payload: V8StructuredClonePayload,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerClientNavigate {
    pub(crate) request_id: u64,
    pub(crate) source_version_id: ServiceWorkerVersionId,
    pub(crate) target_client_id: ServiceWorkerClientId,
    pub(crate) url: Url,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerClientNavigateError {
    TypeError(String),
}

impl ServiceWorkerClientNavigateError {
    pub(crate) fn type_error(message: impl Into<String>) -> Self {
        Self::TypeError(message.into())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerClientNavigateResult {
    pub(crate) request_id: u64,
    pub(crate) result:
        Result<Option<ServiceWorkerClientSnapshot>, ServiceWorkerClientNavigateError>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerClientFocus {
    pub(crate) request_id: u64,
    pub(crate) source_version_id: ServiceWorkerVersionId,
    pub(crate) target_client_id: ServiceWorkerClientId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerClientFocusError {
    DomException { name: &'static str, message: String },
    TypeError(String),
}

impl ServiceWorkerClientFocusError {
    pub(crate) fn not_found() -> Self {
        Self::DomException {
            name: "NotFoundError",
            message: "The client was not found.".to_owned(),
        }
    }

    pub(crate) fn inactive() -> Self {
        Self::type_error("The client is inactive.")
    }

    pub(crate) fn type_error(message: impl Into<String>) -> Self {
        Self::TypeError(message.into())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerClientFocusResult {
    pub(crate) request_id: u64,
    pub(crate) result: Result<ServiceWorkerClientSnapshot, ServiceWorkerClientFocusError>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerClientsOpenWindow {
    pub(crate) request_id: u64,
    pub(crate) source_version_id: ServiceWorkerVersionId,
    pub(crate) url: Url,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerClientsOpenWindowError {
    TypeError(String),
}

impl ServiceWorkerClientsOpenWindowError {
    pub(crate) fn type_error(message: impl Into<String>) -> Self {
        Self::TypeError(message.into())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerClientsOpenWindowResult {
    pub(crate) request_id: u64,
    pub(crate) result:
        Result<Option<ServiceWorkerClientSnapshot>, ServiceWorkerClientsOpenWindowError>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerMessageCompletion {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) owner: ServiceWorkerRunOwner,
    pub(crate) result: Result<(), String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerFetchRequest {
    pub(crate) client_id: Option<ServiceWorkerClientId>,
    pub(crate) resulting_client_id: Option<ServiceWorkerClientId>,
    pub(crate) url: Url,
    pub(crate) method: String,
    /// Isomorphically decoded wire bytes, ready for the worker's Fetch Headers.
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Option<bytes::Bytes>,
    pub(crate) body_stream: Option<crate::context_bootstrap::ReadableStreamClonePayload>,
    pub(crate) destination: ServiceWorkerRequestDestination,
    pub(crate) request_mode: moli_fetch::RequestMode,
    pub(crate) credentials_mode: moli_fetch::RequestCredentialsMode,
    pub(crate) redirect_mode: moli_fetch::RequestRedirectMode,
    pub(crate) priority: Option<moli_fetch::FetchPriorityHint>,
    pub(crate) is_reload: bool,
    pub(crate) metadata: ServiceWorkerFetchRequestMetadata,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ServiceWorkerFetchRequestMetadata {
    pub(crate) mixed_content_policy: moli_fetch::RequestMixedContentPolicy,
    pub(crate) body_source_is_null: bool,
    pub(crate) cache: String,
    pub(crate) referrer: String,
    pub(crate) referrer_policy: String,
    pub(crate) integrity: String,
    pub(crate) keepalive: bool,
    pub(crate) is_history_navigation: bool,
    pub(crate) use_cors_preflight: bool,
    /// Native preload producers must not consume their own pending response.
    /// This flag is not exposed on the worker's JavaScript Request.
    pub(crate) link_preload: bool,
}

impl Default for ServiceWorkerFetchRequestMetadata {
    fn default() -> Self {
        Self {
            mixed_content_policy: Default::default(),
            body_source_is_null: false,
            cache: "default".to_owned(),
            referrer: "about:client".to_owned(),
            referrer_policy: String::new(),
            integrity: String::new(),
            keepalive: false,
            is_history_navigation: false,
            use_cors_preflight: false,
            link_preload: false,
        }
    }
}

impl ServiceWorkerFetchRequestMetadata {
    pub(crate) fn navigation_kind(
        &self,
        is_reload: bool,
    ) -> moli_fetch::BrowserNavigationRequestKind {
        if is_reload {
            moli_fetch::BrowserNavigationRequestKind::Reload
        } else if self.is_history_navigation {
            moli_fetch::BrowserNavigationRequestKind::HistoryTraversal
        } else {
            moli_fetch::BrowserNavigationRequestKind::Navigate
        }
    }
}

pub(crate) fn service_worker_fetch_request_metadata(
    request: &Request,
) -> ServiceWorkerFetchRequestMetadata {
    let subresource_metadata = request.subresource_request_metadata();
    ServiceWorkerFetchRequestMetadata {
        mixed_content_policy: request.mixed_content_policy(),
        body_source_is_null: request.body_source_is_null(),
        // Native navigation already implements a soft reload through
        // BrowserNavigationRequestKind and Cache-Control: max-age=0. Its
        // Fetch Request representation exposes that policy as no-cache.
        cache: service_worker_request_cache_label(
            if request.request_mode == RequestMode::Navigate
                && request.browser_navigation_kind()
                    == moli_fetch::BrowserNavigationRequestKind::Reload
                && request.cache_mode() == RequestCacheMode::Default
            {
                RequestCacheMode::Validate
            } else {
                request.cache_mode()
            },
        )
        .to_owned(),
        referrer: request
            .referrer_header_value(&request.url)
            .unwrap_or_default(),
        referrer_policy: request.effective_referrer_policy().to_owned(),
        integrity: subresource_metadata
            .and_then(|metadata| metadata.integrity.clone())
            .unwrap_or_default(),
        keepalive: false,
        is_history_navigation: request.browser_navigation_kind()
            == moli_fetch::BrowserNavigationRequestKind::HistoryTraversal,
        use_cors_preflight: request.use_cors_preflight(),
        link_preload: request.priority_hints.link_preload,
    }
}

fn service_worker_request_cache_label(cache_mode: RequestCacheMode) -> &'static str {
    cache_mode.into()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerRequestDestination {
    Empty,
    Audio,
    AudioWorklet,
    Document,
    Font,
    Iframe,
    Image,
    Json,
    Manifest,
    Report,
    Script,
    SharedWorker,
    Style,
    Text,
    Track,
    Video,
    Worker,
    Dictionary,
}

impl ServiceWorkerRequestDestination {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "",
            Self::Audio => "audio",
            Self::AudioWorklet => "audioworklet",
            Self::Document => "document",
            Self::Font => "font",
            Self::Iframe => "iframe",
            Self::Image => "image",
            Self::Json => "json",
            Self::Manifest => "manifest",
            Self::Report => "report",
            Self::Script => "script",
            Self::SharedWorker => "sharedworker",
            Self::Style => "style",
            Self::Text => "text",
            Self::Track => "track",
            Self::Video => "video",
            Self::Worker => "worker",
            // Dictionary remains an internal loader kind; its Fetch request
            // destination is empty, including service-worker interception.
            Self::Dictionary => "",
        }
    }

    pub(crate) fn for_subresource_resource_type(
        resource_type: crate::types::SubresourceResourceType,
    ) -> Option<Self> {
        match resource_type {
            crate::types::SubresourceResourceType::Stylesheet => Some(Self::Style),
            crate::types::SubresourceResourceType::Image => Some(Self::Image),
            crate::types::SubresourceResourceType::Font => Some(Self::Font),
            crate::types::SubresourceResourceType::Audio => Some(Self::Audio),
            crate::types::SubresourceResourceType::Video => Some(Self::Video),
            crate::types::SubresourceResourceType::Script => Some(Self::Script),
            crate::types::SubresourceResourceType::TextTrack => Some(Self::Track),
            crate::types::SubresourceResourceType::Fetch => Some(Self::Empty),
            crate::types::SubresourceResourceType::CspReport => Some(Self::Report),
            crate::types::SubresourceResourceType::Dictionary => Some(Self::Dictionary),
            crate::types::SubresourceResourceType::Manifest => Some(Self::Manifest),
            _ => None,
        }
    }
}

pub(crate) fn service_worker_opaque_response_rejection(
    request_mode: RequestMode,
    destination: ServiceWorkerRequestDestination,
) -> Option<String> {
    if request_mode != RequestMode::NoCors {
        return Some(
            "FetchEvent.respondWith rejected an opaque Response for a request whose mode is not no-cors"
                .to_owned(),
        );
    }
    if matches!(
        destination,
        ServiceWorkerRequestDestination::Document
            | ServiceWorkerRequestDestination::Iframe
            | ServiceWorkerRequestDestination::Worker
            | ServiceWorkerRequestDestination::SharedWorker
    ) {
        return Some(
            "FetchEvent.respondWith rejected an opaque Response for a client request".to_owned(),
        );
    }
    None
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerFetchEvent {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) owner: ServiceWorkerRunOwner,
    pub(crate) request: ServiceWorkerFetchRequest,
    pub(crate) navigation_preload_sent: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerFetchResponse {
    pub(crate) cors_exposed_header_names: Option<Vec<String>>,
    pub(crate) request_include_credentials: bool,
    pub(crate) final_url: Option<Url>,
    pub(crate) response_type: String,
    pub(crate) redirected: bool,
    pub(crate) status: u16,
    pub(crate) status_text: String,
    pub(crate) headers: Vec<(String, Vec<u8>)>,
    pub(crate) body_is_null: bool,
    pub(crate) body: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) enum ServiceWorkerFetchResult {
    Fallback,
    FallbackStream(moli_fetch::UploadBody),
    Response(ServiceWorkerFetchResponse),
    Failure(String),
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerFetchStreamStarted {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) owner: ServiceWorkerRunOwner,
    pub(crate) body_source_id: NetworkBodySourceId,
    pub(crate) response_head: MaterializedServiceWorkerFetchResponseHead,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerFetchStreamChunk {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) body_source_id: NetworkBodySourceId,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerNavigationPreloadResponseStarted {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) owner: ServiceWorkerRunOwner,
    pub(crate) request_url: Url,
    pub(crate) request_method: String,
    pub(crate) request_mode: moli_fetch::RequestMode,
    pub(crate) body_source_id: NetworkBodySourceId,
    pub(crate) response_head: MaterializedServiceWorkerFetchResponseHead,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerNavigationPreloadStreamChunk {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) body_source_id: NetworkBodySourceId,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerNavigationPreloadStreamFinished {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) owner: ServiceWorkerRunOwner,
    pub(crate) body_source_id: NetworkBodySourceId,
    pub(crate) result: Result<(), String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerNavigationPreloadFailure {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) owner: ServiceWorkerRunOwner,
    pub(crate) message: String,
}

#[derive(Clone, Debug)]
pub(crate) struct MaterializedServiceWorkerFetchResponseHead {
    pub(crate) cors_exposed_header_names: Option<Vec<String>>,
    pub(crate) request_include_credentials: bool,
    pub(crate) final_url: Option<Url>,
    pub(crate) response_type: String,
    pub(crate) redirected: bool,
    pub(crate) status: u16,
    pub(crate) status_text: String,
    pub(crate) headers: Vec<(String, Vec<u8>)>,
}

#[derive(Debug)]
pub(crate) struct ServiceWorkerDirectFetchCompletion {
    sender: tokio::sync::oneshot::Sender<ServiceWorkerDirectFetchResult>,
    pub(super) stream_response: bool,
    pub(super) defer_network_fallback: bool,
    pub(super) defer_redirects: bool,
}

impl ServiceWorkerDirectFetchCompletion {
    pub(crate) fn buffered(
        sender: tokio::sync::oneshot::Sender<ServiceWorkerDirectFetchResult>,
    ) -> Self {
        Self {
            sender,
            stream_response: false,
            defer_network_fallback: false,
            defer_redirects: false,
        }
    }

    pub(crate) fn streaming(
        sender: tokio::sync::oneshot::Sender<ServiceWorkerDirectFetchResult>,
    ) -> Self {
        Self {
            stream_response: true,
            ..Self::buffered(sender)
        }
    }

    /// Main worker requests re-enter SW selection after every redirect. The
    /// caller owns both redirect following and any transport fallback.
    pub(crate) fn buffered_with_caller_redirects(
        sender: tokio::sync::oneshot::Sender<ServiceWorkerDirectFetchResult>,
    ) -> Self {
        Self {
            defer_redirects: true,
            defer_network_fallback: true,
            ..Self::buffered(sender)
        }
    }

    pub(crate) fn streaming_with_network_fallback(
        sender: tokio::sync::oneshot::Sender<ServiceWorkerDirectFetchResult>,
    ) -> Self {
        Self {
            defer_network_fallback: true,
            ..Self::streaming(sender)
        }
    }

    pub(super) fn send(self, result: ServiceWorkerDirectFetchResult) -> bool {
        self.sender.send(result).is_ok()
    }
}

#[derive(Debug)]
pub(crate) enum ServiceWorkerDirectFetchResult {
    Fallback,
    FallbackStream(moli_fetch::UploadBody),
    NetworkFallback {
        request: Box<moli_fetch::Request>,
        preflight_headers: Vec<(String, String)>,
    },
    Response(ServiceWorkerDirectFetchResponse),
    Stream(Box<ServiceWorkerDirectFetchStream>),
    Failure(String),
}

#[derive(Debug)]
pub(crate) struct ServiceWorkerDirectFetchStream {
    pub(crate) body_source_id: crate::types::NetworkBodySourceId,
    pub(crate) head: moli_fetch::ResponseHead,
    pub(crate) response_filter: Option<crate::types::AsyncSubresourceFetchResponseFilter>,
    pub(crate) events: tokio::sync::mpsc::UnboundedReceiver<ServiceWorkerDirectFetchStreamEvent>,
}

#[derive(Debug)]
pub(crate) enum ServiceWorkerDirectFetchStreamEvent {
    Chunk(Vec<u8>),
    Finished(Result<(), String>),
}

#[derive(Debug)]
pub(crate) struct ServiceWorkerDirectFetchResponse {
    /// The worker response's URL before applying the request-URL fallback.
    pub(crate) response_url: Option<Url>,
    pub(crate) response: Box<crate::protocol_types::NavigationResponse>,
    pub(crate) response_filter: Option<crate::types::AsyncSubresourceFetchResponseFilter>,
    pub(crate) from_network_fallback: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerFetchCompletion {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) owner: ServiceWorkerRunOwner,
    pub(crate) result: ServiceWorkerFetchResult,
}

pub(crate) struct ServiceWorkerFetchDispatch {
    pub(crate) request_body_owner: Option<crate::context_bootstrap::RequestBodyOwner>,
    pub(crate) internal_id: u64,
    pub(crate) request: ServiceWorkerFetchRequest,
    pub(crate) redirect_check: Option<moli_fetch::RequestRedirectCheck>,
    pub(crate) cors_preflight_request_headers: Vec<(String, String)>,
    pub(crate) request_cookie_report: Option<moli_cookie_jar::StoredCookieQueryReport>,
    pub(crate) network_context: AsyncSubresourceNetworkContext,
    pub(crate) completion_tx: RendererResourceCompletionSender,
    pub(crate) request_client: ResourceRequestClient,
    pub(crate) resource_task_runner: crate::network::RendererResourceTaskRunner,
    pub(crate) cancel_handle: moli_fetch::FetchCancelHandle,
    pub(crate) direct_completion_tx: Option<ServiceWorkerDirectFetchCompletion>,
}

#[cfg(test)]
mod referrer_tests {
    use super::*;

    #[test]
    fn navigation_fetch_metadata_preserves_explicit_cache_modes_on_soft_reload() {
        use moli_fetch::BrowserNavigationRequestKind as Kind;
        for mode in [RequestMode::Navigate, RequestMode::Cors] {
            for kind in [Kind::Navigate, Kind::Reload, Kind::HistoryTraversal] {
                for (cache, label) in [
                    (RequestCacheMode::Default, "default"),
                    (RequestCacheMode::Validate, "no-cache"),
                    (RequestCacheMode::Bypass, "reload"),
                    (RequestCacheMode::NoStore, "no-store"),
                    (RequestCacheMode::ForceCache, "force-cache"),
                    (RequestCacheMode::OnlyIfCached, "only-if-cached"),
                ] {
                    let request = Request::get("https://example.test/document")
                        .unwrap()
                        .with_request_mode(mode)
                        .with_browser_navigation_kind(kind)
                        .with_cache_mode(cache);
                    let metadata = service_worker_fetch_request_metadata(&request);
                    let expected = if mode == RequestMode::Navigate
                        && kind == Kind::Reload
                        && cache == RequestCacheMode::Default
                    {
                        "no-cache"
                    } else {
                        label
                    };
                    assert_eq!(metadata.cache, expected, "{mode:?} {kind:?} {cache:?}");
                }
            }
        }
    }

    #[test]
    fn native_fetch_event_metadata_resolves_referrer_policy_and_source() {
        let source = Url::parse("https://user:password@origin.test/from?q=1#private").unwrap();
        let full = "https://origin.test/from?q=1";
        let default = "strict-origin-when-cross-origin";
        for (target, policy, document_policy, referrer, expected_policy, expected_referrer) in [
            (
                "https://origin.test/resource",
                None,
                None,
                None,
                default,
                full,
            ),
            (
                "https://cross.test/resource",
                None,
                None,
                None,
                default,
                "https://origin.test/",
            ),
            ("http://cross.test/resource", None, None, None, default, ""),
            (
                "https://origin.test/resource",
                None,
                Some("origin"),
                None,
                "origin",
                "https://origin.test/",
            ),
            (
                "https://origin.test/resource",
                Some(""),
                Some("no-referrer"),
                None,
                "no-referrer",
                "",
            ),
            (
                "https://origin.test/resource",
                Some("unsafe-url"),
                Some("no-referrer"),
                None,
                "unsafe-url",
                full,
            ),
            (
                "https://origin.test/resource",
                Some("no-referrer"),
                Some("unsafe-url"),
                None,
                "no-referrer",
                "",
            ),
            (
                "https://origin.test/resource",
                None,
                Some(""),
                None,
                default,
                full,
            ),
            (
                "https://origin.test/resource",
                Some("unsafe-url"),
                None,
                Some(""),
                "unsafe-url",
                "",
            ),
            (
                "https://origin.test/resource",
                Some("unsafe-url"),
                None,
                Some("https://origin.test/selected#private"),
                "unsafe-url",
                "https://origin.test/selected",
            ),
        ] {
            let mut request = Request::get(target)
                .unwrap()
                .with_initiator_url(&source)
                .with_cache_mode(RequestCacheMode::Validate)
                .with_use_cors_preflight(true)
                .with_subresource_request_metadata(moli_fetch::SubresourceRequestMetadata {
                    referrer_policy: policy.map(str::to_owned),
                    document_referrer_policy: document_policy.map(str::to_owned),
                    integrity: Some("sha384-test".to_owned()),
                });
            if let Some(referrer) = referrer {
                request = request.with_fetch_referrer(referrer).unwrap();
            }
            let metadata = service_worker_fetch_request_metadata(&request);
            assert_eq!(
                metadata.referrer, expected_referrer,
                "{target} {policy:?} {document_policy:?}"
            );
            assert_eq!(metadata.referrer_policy, expected_policy);
            assert_eq!(metadata.cache, "no-cache");
            assert_eq!(metadata.integrity, "sha384-test");
            assert!(metadata.use_cors_preflight);
        }
    }
}
