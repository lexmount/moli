use moli_fetch::{Request, RequestCacheMode, RequestMode};
use url::Url;

use crate::types::NetworkBodySourceId;
use crate::{
    network::ResourceRequestClient, page_task_queue::RendererResourceCompletionSender,
    runtime::RendererServiceWorkerRunIdentity, structured_clone::V8StructuredClonePayload,
    types::AsyncSubresourceNetworkContext,
};

use super::{
    clients::ServiceWorkerClientSnapshot,
    ids::{
        ServiceWorkerClientId, ServiceWorkerEventId, ServiceWorkerRegistrationId,
        ServiceWorkerVersionId,
    },
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
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) run: RendererServiceWorkerRunIdentity,
    pub(crate) kind: ServiceWorkerLifecycleEventKind,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerLifecycleCompletion {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) run: RendererServiceWorkerRunIdentity,
    pub(crate) kind: ServiceWorkerLifecycleEventKind,
    pub(crate) result: Result<(), String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerMessageEvent {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) run: RendererServiceWorkerRunIdentity,
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
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) run: RendererServiceWorkerRunIdentity,
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
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) run: RendererServiceWorkerRunIdentity,
    pub(crate) result: Result<(), String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerPushEvent {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) run: RendererServiceWorkerRunIdentity,
    pub(crate) data: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerPushCompletion {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) run: RendererServiceWorkerRunIdentity,
    pub(crate) result: Result<(), String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerSyncEvent {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) run: RendererServiceWorkerRunIdentity,
    pub(crate) tag: String,
    pub(crate) last_chance: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerSyncCompletion {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) run: RendererServiceWorkerRunIdentity,
    pub(crate) tag: String,
    pub(crate) result: Result<(), String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerPeriodicSyncEvent {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) run: RendererServiceWorkerRunIdentity,
    pub(crate) tag: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerPeriodicSyncCompletion {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) run: RendererServiceWorkerRunIdentity,
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
        Self::TypeError("The client is inactive.".to_owned())
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
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) run: RendererServiceWorkerRunIdentity,
    pub(crate) result: Result<(), String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerFetchRequest {
    pub(crate) client_id: ServiceWorkerClientId,
    pub(crate) resulting_client_id: Option<ServiceWorkerClientId>,
    pub(crate) url: Url,
    pub(crate) method: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Option<Vec<u8>>,
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
    pub(crate) cache: String,
    pub(crate) referrer: String,
    pub(crate) referrer_policy: String,
    pub(crate) integrity: String,
    pub(crate) keepalive: bool,
}

impl Default for ServiceWorkerFetchRequestMetadata {
    fn default() -> Self {
        Self {
            cache: "default".to_owned(),
            referrer: "about:client".to_owned(),
            referrer_policy: String::new(),
            integrity: String::new(),
            keepalive: false,
        }
    }
}

pub(crate) fn service_worker_fetch_request_metadata(
    request: &Request,
) -> ServiceWorkerFetchRequestMetadata {
    let subresource_metadata = request.subresource_request_metadata();
    ServiceWorkerFetchRequestMetadata {
        cache: service_worker_request_cache_label(request.cache_mode()).to_owned(),
        referrer: if request.infers_referrer_from_initiator() {
            "about:client".to_owned()
        } else {
            String::new()
        },
        referrer_policy: subresource_metadata
            .and_then(|metadata| metadata.referrer_policy.clone())
            .unwrap_or_default(),
        integrity: subresource_metadata
            .and_then(|metadata| metadata.integrity.clone())
            .unwrap_or_default(),
        keepalive: false,
    }
}

fn service_worker_request_cache_label(cache_mode: RequestCacheMode) -> &'static str {
    match cache_mode {
        RequestCacheMode::Default => "default",
        RequestCacheMode::Validate | RequestCacheMode::Bypass => "reload",
        RequestCacheMode::NoStore => "no-store",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerRequestDestination {
    Empty,
    Audio,
    Document,
    Font,
    Iframe,
    Image,
    Manifest,
    Report,
    Script,
    SharedWorker,
    Style,
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
            Self::Document => "document",
            Self::Font => "font",
            Self::Iframe => "iframe",
            Self::Image => "image",
            Self::Manifest => "manifest",
            Self::Report => "report",
            Self::Script => "script",
            Self::SharedWorker => "sharedworker",
            Self::Style => "style",
            Self::Track => "track",
            Self::Video => "video",
            Self::Worker => "worker",
            Self::Dictionary => "dictionary",
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
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) run: RendererServiceWorkerRunIdentity,
    pub(crate) request: ServiceWorkerFetchRequest,
    pub(crate) navigation_preload_sent: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerFetchResponse {
    pub(crate) final_url: Option<Url>,
    pub(crate) response_type: String,
    pub(crate) redirected: bool,
    pub(crate) status: u16,
    pub(crate) status_text: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) enum ServiceWorkerFetchResult {
    Fallback,
    Response(ServiceWorkerFetchResponse),
    Failure(String),
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerFetchStreamStarted {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) run: RendererServiceWorkerRunIdentity,
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
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) run: RendererServiceWorkerRunIdentity,
    pub(crate) request_url: Url,
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
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) run: RendererServiceWorkerRunIdentity,
    pub(crate) body_source_id: NetworkBodySourceId,
    pub(crate) result: Result<(), String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerNavigationPreloadFailure {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) run: RendererServiceWorkerRunIdentity,
    pub(crate) message: String,
}

#[derive(Clone, Debug)]
pub(crate) struct MaterializedServiceWorkerFetchResponseHead {
    pub(crate) final_url: Option<Url>,
    pub(crate) response_type: String,
    pub(crate) redirected: bool,
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
}

#[derive(Debug)]
pub(crate) enum ServiceWorkerDirectFetchResult {
    Fallback,
    Response(ServiceWorkerDirectFetchResponse),
    Failure(String),
}

#[derive(Debug)]
pub(crate) struct ServiceWorkerDirectFetchResponse {
    pub(crate) response: Box<crate::protocol_types::NavigationResponse>,
    pub(crate) response_filter: Option<crate::types::AsyncSubresourceFetchResponseFilter>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerFetchCompletion {
    pub(crate) event_id: ServiceWorkerEventId,
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) run: RendererServiceWorkerRunIdentity,
    pub(crate) result: ServiceWorkerFetchResult,
}

pub(crate) struct ServiceWorkerFetchDispatch {
    pub(crate) internal_id: u64,
    pub(crate) request: ServiceWorkerFetchRequest,
    pub(crate) request_body_text: Option<String>,
    pub(crate) cors_preflight_request_headers: Vec<(String, String)>,
    pub(crate) request_cookie_report: Option<moli_cookie_jar::StoredCookieQueryReport>,
    pub(crate) network_context: AsyncSubresourceNetworkContext,
    pub(crate) completion_tx: RendererResourceCompletionSender,
    pub(crate) request_client: ResourceRequestClient,
    pub(crate) resource_task_runner: crate::network::RendererResourceTaskRunner,
    pub(crate) cancel_handle: moli_fetch::FetchCancelHandle,
    pub(crate) direct_completion_tx:
        Option<tokio::sync::oneshot::Sender<ServiceWorkerDirectFetchResult>>,
}
