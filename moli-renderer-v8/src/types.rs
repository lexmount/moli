use url::Url;

use crate::document_runtime::DocumentPolicyContainer;
use crate::dom::native::NativeNodeId;

pub(crate) type MessagePortId = moli_message_port::MessagePortId;
pub(crate) type BroadcastChannelId = moli_broadcast_channel::BroadcastChannelId;
pub(crate) type NetworkBodySourceId = u64;
pub(crate) type SharedNavigationResponseResult =
    std::sync::Arc<std::result::Result<NavigationResponse, String>>;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct DedicatedWorkerId(u64);

impl DedicatedWorkerId {
    pub(crate) fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub(crate) fn as_u64(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for DedicatedWorkerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptErrorConstructorKind {
    Error,
    SyntaxError,
    WebAssemblyCompileError,
    WebAssemblyLinkError,
}

pub use moli_script::{
    ScriptElementClassificationInput, ScriptPreparationClassificationInput,
    classify_script_preparation,
};
#[cfg(test)]
pub use moli_script::{ScriptSchedulingInput, classify_script_mode};

#[allow(unused_imports)]
pub use crate::protocol_types::{
    ContentSecurityPolicyIssueSnapshot, ContentSecurityPolicyViolationType, DocumentStartScript,
    InspectorIssueSnapshot, InspectorSourceCodeLocationSnapshot, JsValueSnapshot,
    NavigationRedirect, NavigationResponse, PendingSubresourceAuthInfo,
    PendingSubresourceContinueEvent, PendingSubresourceContinueOutcome,
    PendingSubresourceFetchInfo, PendingSubresourceResponseInfo, QuirksModeIssueSnapshot,
    ScriptExecutionReport, ScriptGlobalsSnapshotState, ScriptKind, ScriptMode, ScriptNetworkOutput,
    ScriptNetworkOutputItem, ScriptObservableOutput, ScriptObservableOutputItem, ScriptRun,
    ScriptRunOutcome, ScriptSkipReason, ScriptSourceKind, SubresourceAuthChallenge,
    SubresourceAuthCredentials, SubresourceAuthScheme, SubresourceAuthTarget,
    SubresourceBodyFinished, SubresourceBodyFinishedResult, SubresourceDataReceived,
    SubresourceEventSourceMessageReceived, SubresourceJsonPathEquals, SubresourceJsonPathRegex,
    SubresourceNetworkOutcome, SubresourceNetworkRecord, SubresourceNetworkRequestHandle,
    SubresourceRequestInitiatorType, SubresourceRequestStarted, SubresourceResourceType,
    SubresourceResponseBody, SubresourceResponseBodyWriter, SubresourceResponseStarted,
    SubresourceResponseWaitCriteria, WebSocketFrameDirection, WebSocketFrameOpcode,
    WebSocketLifecycleEvent, WebSocketLifecycleKind, WebSocketNetworkEvent,
};

pub(super) enum PendingSubresourceContinuation {
    Beacon,
    CspReport {
        client_id: crate::service_worker_runtime::ServiceWorkerClientId,
    },
    EventSource(v8::Global<v8::Object>),
    Fetch(PendingWindowFetchContinuation),
    Image {
        image_handle: crate::document_runtime::DomHandle,
        sequence: crate::native_bridge::ImageLoadEventId,
        request_initiator_type: SubresourceRequestInitiatorType,
    },
    Media {
        media_handle: crate::document_runtime::DomHandle,
        sequence: crate::native_bridge::MediaLoadSequenceId,
    },
    TextTrack {
        track_handle: crate::document_runtime::DomHandle,
        sequence: crate::native_bridge::TextTrackLoadSequenceId,
    },
    StylesheetSubresource {
        binding: crate::frame_owner_model::StylesheetSubresourceLoadDelayBinding,
        web_font: Option<crate::css_resource_urls::StylesheetWebFont>,
        css_image: Option<crate::native_bridge::CssImageResourceRequestIdentity>,
    },
    Xhr(v8::Global<v8::Object>),
    WebSocket(PendingWebSocketConnection),
    WorkerFetch {
        worker_id: DedicatedWorkerId,
        fetch_id: u32,
    },
    WorkerXhr {
        worker_id: DedicatedWorkerId,
        xhr_id: u32,
    },
    WorkerCspReport {
        worker_id: DedicatedWorkerId,
        report_id: u32,
    },
    SharedWorkerFetch {
        instance_id: moli_shared_worker::SharedWorkerInstanceId,
        fetch_id: u32,
    },
    SharedWorkerXhr {
        instance_id: moli_shared_worker::SharedWorkerInstanceId,
        xhr_id: u32,
    },
    SharedWorkerCspReport {
        instance_id: moli_shared_worker::SharedWorkerInstanceId,
        report_id: u32,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum ImageRequestCorsMode {
    NoCors,
    Anonymous,
    UseCredentials,
}

impl ImageRequestCorsMode {
    pub(super) fn from_cross_origin_attribute(value: Option<&str>) -> Self {
        match value {
            None => Self::NoCors,
            Some(value) if value.trim().eq_ignore_ascii_case("use-credentials") => {
                Self::UseCredentials
            }
            Some(_) => Self::Anonymous,
        }
    }
}

impl PendingSubresourceContinuation {
    pub(super) fn request_initiator_type(&self) -> SubresourceRequestInitiatorType {
        match self {
            Self::Image {
                request_initiator_type,
                ..
            } => *request_initiator_type,
            Self::Media { .. } | Self::TextTrack { .. } => SubresourceRequestInitiatorType::Other,
            Self::StylesheetSubresource { .. } => SubresourceRequestInitiatorType::Css,
            _ => SubresourceRequestInitiatorType::Script,
        }
    }

    #[cfg(test)]
    pub(super) fn delays_document_load_event(&self) -> bool {
        matches!(
            self,
            Self::Image { .. }
                | Self::Media { .. }
                | Self::TextTrack { .. }
                | Self::StylesheetSubresource { .. }
        )
    }

    pub(super) fn dedicated_worker_id(&self) -> Option<DedicatedWorkerId> {
        match self {
            Self::WorkerFetch { worker_id, .. }
            | Self::WorkerXhr { worker_id, .. }
            | Self::WorkerCspReport { worker_id, .. } => Some(*worker_id),
            _ => None,
        }
    }

    pub(super) fn stylesheet_subresource_owner(
        &self,
    ) -> Option<crate::frame_owner_model::FrameDocumentTaskOwner> {
        match self {
            Self::StylesheetSubresource { binding, .. } => Some(binding.owner()),
            _ => None,
        }
    }

    pub(super) fn is_window_xhr(&self) -> bool {
        matches!(self, Self::Xhr(_))
    }

    pub(super) fn is_window_fetch(&self) -> bool {
        matches!(self, Self::Fetch(_))
    }

    pub(super) fn is_window_event_source(&self) -> bool {
        matches!(self, Self::EventSource(_))
    }

    pub(super) fn window_fetch_keepalive(&self) -> bool {
        match self {
            Self::Fetch(fetch) => fetch.keepalive(),
            _ => false,
        }
    }

    pub(super) fn is_detached_window_fetch(&self) -> bool {
        match self {
            Self::Fetch(fetch) => fetch.is_detached(),
            _ => false,
        }
    }

    pub(super) fn window_fetch(&self) -> Option<&PendingWindowFetchContinuation> {
        match self {
            Self::Fetch(fetch) => Some(fetch),
            _ => None,
        }
    }
}

pub(super) struct PendingWindowFetchContinuation {
    promise: PendingWindowFetchPromise,
    keepalive: bool,
    connect_policy: crate::document_runtime::DocumentConnectPolicySnapshot,
    csp_report_context: crate::network_host::WindowCspReportRequestContext,
}

enum PendingWindowFetchPromise {
    Active(v8::Global<v8::PromiseResolver>),
    DetachedKeepalive,
}

impl PendingWindowFetchContinuation {
    pub(super) fn new(
        resolver: v8::Global<v8::PromiseResolver>,
        keepalive: bool,
        connect_policy: crate::document_runtime::DocumentConnectPolicySnapshot,
        csp_report_context: crate::network_host::WindowCspReportRequestContext,
    ) -> Self {
        Self {
            promise: PendingWindowFetchPromise::Active(resolver),
            keepalive,
            connect_policy,
            csp_report_context,
        }
    }

    pub(super) fn keepalive(&self) -> bool {
        self.keepalive
    }

    pub(super) fn is_detached(&self) -> bool {
        matches!(self.promise, PendingWindowFetchPromise::DetachedKeepalive)
    }

    pub(super) fn detach(&mut self) -> bool {
        if !self.keepalive || self.is_detached() {
            return false;
        }
        self.promise = PendingWindowFetchPromise::DetachedKeepalive;
        true
    }

    pub(super) fn into_resolver(self) -> Option<v8::Global<v8::PromiseResolver>> {
        match self.promise {
            PendingWindowFetchPromise::Active(resolver) => Some(resolver),
            PendingWindowFetchPromise::DetachedKeepalive => None,
        }
    }

    pub(super) fn resolver(&self) -> Option<&v8::Global<v8::PromiseResolver>> {
        match &self.promise {
            PendingWindowFetchPromise::Active(resolver) => Some(resolver),
            PendingWindowFetchPromise::DetachedKeepalive => None,
        }
    }

    pub(super) fn connect_policy(&self) -> &crate::document_runtime::DocumentConnectPolicySnapshot {
        &self.connect_policy
    }

    pub(super) fn csp_report_context(&self) -> &crate::network_host::WindowCspReportRequestContext {
        &self.csp_report_context
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct ImageRequestKey {
    url: String,
    cors_mode: ImageRequestCorsMode,
    density_bits: u64,
}

impl ImageRequestKey {
    pub(super) fn with_density(url: String, cors_mode: ImageRequestCorsMode, density: f64) -> Self {
        let density = if density == 0.0 { 0.0 } else { density };
        debug_assert!(density.is_finite() && density >= 0.0);
        Self {
            url,
            cors_mode,
            density_bits: density.to_bits(),
        }
    }

    pub(super) fn url(&self) -> &str {
        &self.url
    }

    pub(super) fn density(&self) -> f64 {
        f64::from_bits(self.density_bits)
    }
}
pub(super) struct PendingWebSocketConnection {
    pub(super) socket_id: u64,
    pub(super) protocols: Vec<String>,
    pub(super) connect_options: moli_websocket::ConnectOptions,
}

pub(super) struct PendingWebSocketResponseState {
    pub(super) internal_id: u64,
    pub(super) socket_id: u64,
}

pub(super) enum PendingSubresourceExecutionContext {
    Window(crate::native_bridge::WindowExecutionContextBinding),
    WindowFetch(crate::native_bridge::WindowFetchContext),
    WindowNetworkOnly(crate::native_bridge::WindowExecutionContextIdentity),
    WindowDocumentNetworkOnly(crate::native_bridge::WindowDocumentNetworkRequestIdentity),
    DetachedWindowFetch(crate::native_bridge::DetachedWindowFetchContext),
    Adapter {
        dispatch_scope: crate::native_bridge::OwnerDispatchScope,
        context: v8::Global<v8::Context>,
    },
}

impl PendingSubresourceExecutionContext {
    pub(super) fn window(binding: crate::native_bridge::WindowExecutionContextBinding) -> Self {
        Self::Window(binding)
    }

    pub(super) fn window_fetch(context: crate::native_bridge::WindowFetchContext) -> Self {
        Self::WindowFetch(context)
    }

    pub(super) fn window_network_only(
        identity: crate::native_bridge::WindowExecutionContextIdentity,
    ) -> Self {
        Self::WindowNetworkOnly(identity)
    }

    pub(super) fn window_document_network_only(
        identity: crate::native_bridge::WindowDocumentNetworkRequestIdentity,
    ) -> Self {
        Self::WindowDocumentNetworkOnly(identity)
    }

    pub(super) fn adapter(
        dispatch_scope: crate::native_bridge::OwnerDispatchScope,
        context: v8::Global<v8::Context>,
    ) -> Self {
        Self::Adapter {
            dispatch_scope,
            context,
        }
    }

    /// Window whose request policy and host-side continuation scope apply.
    ///
    /// `WindowFetch` derives both this scope and its script realm from one
    /// authorized receiver. They remain separate fields because a keepalive
    /// request may outlive JS delivery, not because callers may pair unrelated
    /// realms and Windows.
    pub(super) fn dispatch_scope(&self) -> crate::native_bridge::OwnerDispatchScope {
        match self {
            Self::Window(binding) => binding.dispatch_scope(),
            Self::WindowFetch(context) => context.request_target().dispatch_scope(),
            Self::WindowNetworkOnly(identity) => identity.dispatch_scope(),
            Self::WindowDocumentNetworkOnly(identity) => identity.dispatch_scope(),
            Self::DetachedWindowFetch(context) => context.request_target().dispatch_scope(),
            Self::Adapter { dispatch_scope, .. } => *dispatch_scope,
        }
    }

    pub(super) fn context_global(&self) -> Option<&v8::Global<v8::Context>> {
        match self {
            Self::Window(binding) => Some(binding.context_global()),
            Self::WindowFetch(context) => Some(context.script_realm().context_global()),
            Self::WindowNetworkOnly(_)
            | Self::WindowDocumentNetworkOnly(_)
            | Self::DetachedWindowFetch(_) => None,
            Self::Adapter { context, .. } => Some(context),
        }
    }

    pub(super) fn window_realm_binding(
        &self,
    ) -> Option<&crate::native_bridge::WindowExecutionContextBinding> {
        match self {
            Self::Window(binding) => Some(binding),
            Self::WindowFetch(context) => Some(context.script_realm()),
            Self::WindowNetworkOnly(_)
            | Self::WindowDocumentNetworkOnly(_)
            | Self::DetachedWindowFetch(_)
            | Self::Adapter { .. } => None,
        }
    }

    #[cfg(test)]
    pub(super) fn active_window_fetch_context(
        &self,
    ) -> Option<&crate::native_bridge::WindowFetchContext> {
        match self {
            Self::WindowFetch(context) => Some(context),
            _ => None,
        }
    }

    pub(super) fn window_request_target(&self) -> Option<crate::native_bridge::WindowTaskTarget> {
        match self {
            Self::Window(binding) => Some(crate::native_bridge::WindowTaskTarget::new(
                binding.dispatch_scope(),
                binding.owner(),
            )),
            Self::WindowFetch(context) => Some(context.request_target()),
            Self::WindowNetworkOnly(identity) => Some(crate::native_bridge::WindowTaskTarget::new(
                identity.dispatch_scope(),
                identity.owner(),
            )),
            Self::WindowDocumentNetworkOnly(_)
            | Self::DetachedWindowFetch(_)
            | Self::Adapter { .. } => None,
        }
    }

    pub(super) fn window_realm_owner(
        &self,
    ) -> Option<crate::native_bridge::WindowExecutionContextOwner> {
        match self {
            Self::Window(binding) => Some(binding.owner()),
            Self::WindowFetch(context) => Some(context.script_realm().owner()),
            Self::WindowNetworkOnly(identity) => Some(identity.owner()),
            Self::WindowDocumentNetworkOnly(_)
            | Self::DetachedWindowFetch(_)
            | Self::Adapter { .. } => None,
        }
    }

    pub(super) fn realm_token(
        &self,
    ) -> Option<crate::native_bridge::RuntimeObservableContextToken> {
        match self {
            Self::Window(binding) => Some(binding.realm_token()),
            Self::WindowFetch(context) => Some(context.script_realm().realm_token()),
            Self::WindowNetworkOnly(identity) => Some(identity.realm_token()),
            Self::WindowDocumentNetworkOnly(_)
            | Self::DetachedWindowFetch(_)
            | Self::Adapter { .. } => None,
        }
    }

    pub(super) fn is_window_network_only(&self) -> bool {
        matches!(
            self,
            Self::WindowNetworkOnly(_) | Self::WindowDocumentNetworkOnly(_)
        )
    }

    pub(super) fn window_network_only_identity(
        &self,
    ) -> Option<crate::native_bridge::WindowExecutionContextIdentity> {
        match self {
            Self::WindowNetworkOnly(identity) => Some(*identity),
            _ => None,
        }
    }

    pub(super) fn window_document_network_only_identity(
        &self,
    ) -> Option<crate::native_bridge::WindowDocumentNetworkRequestIdentity> {
        match self {
            Self::WindowDocumentNetworkOnly(identity) => Some(*identity),
            _ => None,
        }
    }

    pub(super) fn detached_window_fetch_context(
        &self,
    ) -> Option<crate::native_bridge::DetachedWindowFetchContext> {
        match self {
            Self::DetachedWindowFetch(context) => Some(*context),
            _ => None,
        }
    }

    pub(super) fn detached_window_fetch_identity(
        &self,
    ) -> Option<(
        crate::native_bridge::WindowExecutionContextOwner,
        crate::native_bridge::RuntimeObservableContextToken,
    )> {
        let context = self.detached_window_fetch_context()?;
        Some((
            context.request_target().owner(),
            context.script_realm_token(),
        ))
    }
}

pub(super) struct PendingSubresourceFetchState {
    pub(super) info: PendingSubresourceFetchInfo,
    pub(super) load: crate::network::loads::ResourceLoadLease,
    pub(super) execution_context: PendingSubresourceExecutionContext,
    pub(super) credentials_mode: moli_fetch::RequestCredentialsMode,
    pub(super) request_mode: moli_fetch::RequestMode,
    pub(super) network_partition_key: Option<String>,
    pub(super) policy_context: SubresourcePolicyContext,
    pub(super) continuation: PendingSubresourceContinuation,
    // Window fetches that need CORS preflight emit the actual request-start
    // after the preflight record, not when the pending fetch is registered.
    pub(super) deferred_request_started: bool,
}

impl PendingSubresourceFetchState {
    pub(super) fn detach_keepalive_window_fetch(&mut self) -> bool {
        let PendingSubresourceExecutionContext::WindowFetch(context) = &self.execution_context
        else {
            return false;
        };
        if !self.continuation.window_fetch_keepalive() {
            return false;
        }
        // Detaching a keepalive request intentionally drops the V8 Global but
        // preserves both logical addresses: the request target remains useful
        // for network/CSP observation, while the promise realm locator is kept
        // only for diagnostics and must never be used for JS delivery.
        let detached_context = context.detached();
        let PendingSubresourceContinuation::Fetch(fetch) = &mut self.continuation else {
            return false;
        };
        if !fetch.detach() {
            return false;
        }
        self.execution_context =
            PendingSubresourceExecutionContext::DetachedWindowFetch(detached_context);
        true
    }
}

pub(super) struct PendingSubresourceResponseState {
    pub(super) pending: PendingSubresourceFetchState,
    pub(super) request_url: Url,
    pub(super) request_method: String,
    pub(super) request_headers: Vec<(String, String)>,
    pub(super) request_body: Option<String>,
    pub(super) response: NavigationResponse,
}

pub(super) struct PendingSubresourceAuthState {
    pub(super) pending: PendingSubresourceFetchState,
    pub(super) request_url: Url,
    pub(super) request_method: String,
    pub(super) request_headers: Vec<(String, String)>,
    pub(super) request_body: Option<String>,
    pub(super) intercept_response: bool,
    pub(super) initial_network_request_headers: Option<Vec<(String, String)>>,
    pub(super) response: NavigationResponse,
}

pub(super) struct RunningSubresourceFetchState {
    pub(super) pending: PendingSubresourceFetchState,
    pub(super) request_url: Url,
    pub(super) request_method: String,
    pub(super) request_headers: Vec<(String, String)>,
    pub(super) request_body: Option<String>,
    pub(super) intercept_response: bool,
    pub(super) handle_auth_requests: bool,
    pub(super) initial_auth_network_request_headers: Option<Vec<(String, String)>>,
}

pub(super) struct InFlightWorkerSubresourceFetchState {
    pub(super) pending: PendingSubresourceFetchState,
    pub(super) request_url: Url,
    pub(super) request_method: String,
    pub(super) request_headers: Vec<(String, String)>,
    pub(super) request_body: Option<String>,
}

#[derive(Debug)]
pub(super) struct AsyncSubresourceFetchCompletion {
    pub(super) internal_id: u64,
    pub(super) request_url: Url,
    pub(super) request_method: String,
    pub(super) request_headers: Vec<(String, String)>,
    pub(super) request_body: Option<String>,
    pub(super) response_status_text: Option<String>,
    pub(super) skip_fetch_security_validation: bool,
    pub(super) response_filter: Option<AsyncSubresourceFetchResponseFilter>,
    pub(super) network_error_text: Option<String>,
    pub(super) result: std::result::Result<NavigationResponse, String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AsyncSubresourceFetchResponseFilter {
    Opaque,
    OpaqueRedirect,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SubresourcePolicyContext {
    pub(super) cross_origin_embedder_policy:
        crate::cross_origin_isolation::CrossOriginEmbedderPolicy,
    pub(super) document_isolation_policy: crate::cross_origin_isolation::DocumentIsolationPolicy,
    pub(super) cross_origin_isolated: bool,
}

impl SubresourcePolicyContext {
    pub(super) fn from_document_policy(policy: &DocumentPolicyContainer) -> Self {
        Self {
            cross_origin_embedder_policy: policy.cross_origin_embedder_policy,
            document_isolation_policy: policy.document_isolation_policy,
            cross_origin_isolated: policy.cross_origin_isolated,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct AsyncSubresourceNetworkContext {
    pub(super) frame_id: Option<String>,
    pub(super) document_url: Url,
    pub(super) resource_type: SubresourceResourceType,
    pub(super) policy_context: SubresourcePolicyContext,
}

#[derive(Debug)]
pub(super) struct AsyncSubresourceStreamingStarted {
    pub(super) internal_id: u64,
    pub(super) request_url: Url,
    pub(super) request_method: String,
    pub(super) request_headers: Vec<(String, String)>,
    pub(super) request_body: Option<String>,
    pub(super) body_source_id: NetworkBodySourceId,
    pub(super) head: moli_fetch::ResponseHead,
    pub(super) network_request_headers: Option<Vec<(String, String)>>,
}

#[derive(Debug)]
pub(super) struct AsyncSubresourceStreamingChunk {
    pub(super) body_source_id: NetworkBodySourceId,
    pub(super) bytes: Vec<u8>,
}

#[derive(Debug)]
pub(super) struct AsyncSubresourceStreamingFinished {
    pub(super) internal_id: u64,
    pub(super) body_source_id: NetworkBodySourceId,
    pub(super) result: std::result::Result<(), String>,
}

/// PageVm-local identity needed to authorize one async-subresource networking
/// event before it can touch request or body-stream state.
///
/// The stable Page envelope adds the root renderer Document token. Keeping the
/// phase in this target prevents a completion, stream start, chunk, or finish
/// from being accepted merely because one of their numeric ids was reused by
/// another kind of resident.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AsyncSubresourceFetchEventTarget {
    Completion {
        internal_id: u64,
    },
    StreamingStart {
        internal_id: u64,
        body_source_id: NetworkBodySourceId,
    },
    StreamingChunk {
        body_source_id: NetworkBodySourceId,
    },
    StreamingFinish {
        internal_id: u64,
        body_source_id: NetworkBodySourceId,
    },
    /// A producer-captured network fact has no live JS request owner. It is
    /// still namespaced by the root Document in the Page task envelope.
    ObservedNetworkRecord,
}

#[derive(Debug)]
pub(super) enum AsyncSubresourceFetchEvent {
    Completion(Box<AsyncSubresourceFetchCompletion>),
    ObservedNetworkRecord(Box<SubresourceNetworkRecord>),
    StreamingStarted(Box<AsyncSubresourceStreamingStarted>),
    StreamingChunk(AsyncSubresourceStreamingChunk),
    StreamingFinished(AsyncSubresourceStreamingFinished),
}

impl AsyncSubresourceFetchEvent {
    pub(crate) fn target(&self) -> AsyncSubresourceFetchEventTarget {
        match self {
            Self::Completion(completion) => AsyncSubresourceFetchEventTarget::Completion {
                internal_id: completion.internal_id,
            },
            Self::ObservedNetworkRecord(_) => {
                AsyncSubresourceFetchEventTarget::ObservedNetworkRecord
            }
            Self::StreamingStarted(started) => AsyncSubresourceFetchEventTarget::StreamingStart {
                internal_id: started.internal_id,
                body_source_id: started.body_source_id,
            },
            Self::StreamingChunk(chunk) => AsyncSubresourceFetchEventTarget::StreamingChunk {
                body_source_id: chunk.body_source_id,
            },
            Self::StreamingFinished(finished) => {
                AsyncSubresourceFetchEventTarget::StreamingFinish {
                    internal_id: finished.internal_id,
                    body_source_id: finished.body_source_id,
                }
            }
        }
    }
}

#[derive(Debug)]
pub(super) struct ServiceWorkerRegisterCompletion {
    pub(super) request_id: u64,
    pub(super) document_owner: crate::window_document_identity::WindowDocumentOwner,
    pub(super) result: std::result::Result<
        crate::service_worker_runtime::ServiceWorkerRegistrationSnapshot,
        crate::service_worker_runtime::ServiceWorkerRegistrationError,
    >,
}

#[derive(Debug)]
pub(super) struct ServiceWorkerReadyCompletion {
    pub(super) request_id: u64,
    pub(super) document_owner: crate::window_document_identity::WindowDocumentOwner,
    pub(super) registration: crate::service_worker_runtime::ServiceWorkerRegistrationSnapshot,
}

#[derive(Debug)]
pub(super) struct ServiceWorkerUnregisterCompletion {
    pub(super) request_id: u64,
    pub(super) document_owner: crate::window_document_identity::WindowDocumentOwner,
    pub(super) result: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ServiceWorkerWindowClientTarget {
    pub(super) client_id: crate::service_worker_runtime::ServiceWorkerClientId,
    pub(super) document_owner: crate::window_document_identity::WindowDocumentOwner,
}

#[derive(Debug)]
pub(super) struct ServiceWorkerClientMessageCompletion {
    pub(super) target: ServiceWorkerWindowClientTarget,
    pub(super) source_version_id: crate::service_worker_runtime::ServiceWorkerVersionId,
    pub(super) source_script_url: url::Url,
    pub(super) source_state: &'static str,
    pub(super) payload: crate::structured_clone::V8StructuredClonePayload,
}

#[derive(Debug)]
pub(super) struct ServiceWorkerClientNavigateRequestCompletion {
    pub(super) target: ServiceWorkerWindowClientTarget,
    pub(super) request_id: u64,
    pub(super) source_version_id: crate::service_worker_runtime::ServiceWorkerVersionId,
    pub(super) source_run: crate::runtime::RendererServiceWorkerRunIdentity,
    pub(super) url: url::Url,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ServiceWorkerClientNavigateContinuation {
    pub(super) request_id: u64,
    pub(super) source_version_id: crate::service_worker_runtime::ServiceWorkerVersionId,
    pub(super) source_run: crate::runtime::RendererServiceWorkerRunIdentity,
}

#[derive(Debug)]
pub(super) struct ServiceWorkerClientNavigateCompletion {
    pub(super) request_id: u64,
    pub(super) source_version_id: crate::service_worker_runtime::ServiceWorkerVersionId,
    pub(super) source_run: crate::runtime::RendererServiceWorkerRunIdentity,
    pub(super) result: std::result::Result<
        Option<crate::service_worker_runtime::ServiceWorkerClientSnapshot>,
        crate::service_worker_runtime::ServiceWorkerClientNavigateError,
    >,
}

#[derive(Debug)]
pub(super) struct ServiceWorkerClientFocusRequestCompletion {
    pub(super) target: ServiceWorkerWindowClientTarget,
    pub(super) request_id: u64,
    pub(super) source_version_id: crate::service_worker_runtime::ServiceWorkerVersionId,
    pub(super) source_run: crate::runtime::RendererServiceWorkerRunIdentity,
}

#[derive(Debug)]
pub(super) struct ServiceWorkerClientFocusCompletion {
    pub(super) request_id: u64,
    pub(super) source_version_id: crate::service_worker_runtime::ServiceWorkerVersionId,
    pub(super) source_run: crate::runtime::RendererServiceWorkerRunIdentity,
    pub(super) result: std::result::Result<
        crate::service_worker_runtime::ServiceWorkerClientSnapshot,
        crate::service_worker_runtime::ServiceWorkerClientFocusError,
    >,
}

#[derive(Debug)]
pub(super) struct ServiceWorkerClientsOpenWindowRequestCompletion {
    pub(super) host: ServiceWorkerWindowClientTarget,
    pub(super) request_id: u64,
    pub(super) source_version_id: crate::service_worker_runtime::ServiceWorkerVersionId,
    pub(super) source_run: crate::runtime::RendererServiceWorkerRunIdentity,
    pub(super) url: url::Url,
}

#[derive(Debug)]
pub(super) struct ServiceWorkerClientsOpenWindowCompletion {
    pub(super) request_id: u64,
    pub(super) source_version_id: crate::service_worker_runtime::ServiceWorkerVersionId,
    pub(super) source_run: crate::runtime::RendererServiceWorkerRunIdentity,
    pub(super) result: std::result::Result<
        Option<crate::service_worker_runtime::ServiceWorkerClientSnapshot>,
        crate::service_worker_runtime::ServiceWorkerClientsOpenWindowError,
    >,
}

#[derive(Debug)]
pub(super) struct ServiceWorkerNotificationActionNavigateRequestCompletion {
    pub(super) host: ServiceWorkerWindowClientTarget,
    pub(super) url: url::Url,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum ServiceWorkerLifecycleClientEvent {
    UpdateFound,
    WorkerStateChanged {
        version_id: crate::service_worker_runtime::ServiceWorkerVersionId,
        state: &'static str,
    },
}

#[derive(Debug)]
pub(super) struct ServiceWorkerLifecycleNotification {
    pub(super) document_owner: crate::window_document_identity::WindowDocumentOwner,
    pub(super) storage_key: String,
    pub(super) registration: crate::service_worker_runtime::ServiceWorkerRegistrationSnapshot,
    pub(super) events: Vec<ServiceWorkerLifecycleClientEvent>,
}

#[derive(Debug)]
pub(super) struct ServiceWorkerControllerChangeCompletion {
    pub(super) target: ServiceWorkerWindowClientTarget,
}

pub(super) struct StreamingSubresourceFetchState {
    pub(super) pending: PendingSubresourceFetchState,
    pub(super) request_url: Url,
    pub(super) request_method: String,
    pub(super) request_headers: Vec<(String, String)>,
    pub(super) request_body: Option<String>,
    pub(super) body_source_id: NetworkBodySourceId,
    pub(super) head: moli_fetch::ResponseHead,
    pub(super) network_request_headers: Option<Vec<(String, String)>>,
    pub(super) body_writer: SubresourceResponseBodyWriter,
    pub(super) event_source_parser: Option<crate::network_host::EventSourceParser>,
    pub(super) xhr_response: Option<XhrStreamingResponseState>,
}

pub(super) struct EventSourceStreamingChunkDelivery<'s> {
    pub(super) context: v8::Local<'s, v8::Context>,
    pub(super) event_source: v8::Local<'s, v8::Object>,
    pub(super) request_handle: Option<SubresourceNetworkRequestHandle>,
    pub(super) messages: Vec<crate::network_host::EventSourceMessage>,
}

pub(super) struct XhrStreamingResponseState {
    pending_utf8_bytes: Vec<u8>,
    loaded: usize,
    total: Option<usize>,
}

impl XhrStreamingResponseState {
    pub(super) fn new(headers: &[(String, String)]) -> Self {
        let total = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.trim().parse::<usize>().ok());
        Self {
            pending_utf8_bytes: Vec::new(),
            loaded: 0,
            total,
        }
    }

    pub(super) fn append(&mut self, bytes: &[u8]) -> (String, usize, Option<usize>) {
        self.loaded = self.loaded.saturating_add(bytes.len());
        self.pending_utf8_bytes.extend_from_slice(bytes);

        let mut decoded = String::new();
        let mut consumed = 0;
        while consumed < self.pending_utf8_bytes.len() {
            let remaining = &self.pending_utf8_bytes[consumed..];
            match std::str::from_utf8(remaining) {
                Ok(text) => {
                    decoded.push_str(text);
                    consumed = self.pending_utf8_bytes.len();
                }
                Err(error) => {
                    let valid_end = consumed + error.valid_up_to();
                    decoded.push_str(
                        std::str::from_utf8(&self.pending_utf8_bytes[consumed..valid_end])
                            .expect("Utf8Error::valid_up_to must identify a valid UTF-8 prefix"),
                    );
                    consumed = valid_end;
                    let Some(invalid_len) = error.error_len() else {
                        break;
                    };
                    decoded.push('\u{fffd}');
                    consumed += invalid_len;
                }
            }
        }
        if consumed > 0 {
            self.pending_utf8_bytes.drain(..consumed);
        }
        (decoded, self.loaded, self.total)
    }
}

pub(super) struct XhrStreamingChunkDelivery<'s> {
    pub(super) context: v8::Local<'s, v8::Context>,
    pub(super) xhr: v8::Local<'s, v8::Object>,
    pub(super) dispatch_scope: crate::native_bridge::OwnerDispatchScope,
    pub(super) realm_token: Option<crate::native_bridge::RuntimeObservableContextToken>,
    pub(super) internal_id: u64,
    pub(super) request_handle: Option<SubresourceNetworkRequestHandle>,
    pub(super) decoded_text: String,
    pub(super) loaded: usize,
    pub(super) total: Option<usize>,
}

/// Exact PageVm-local identity of one parser-blocking external script created
/// by `document.write()`.
///
/// The main Document owner changes on `document.open()`, while `load_id`
/// distinguishes consecutive loads within the same Document. A stable Page
/// queue adds the root renderer Document token at its envelope boundary, so
/// this target remains exact across both same-PageVm and cross-PageVm
/// replacement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DocumentWriteExternalScriptFetchTarget {
    task_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    load_id: u64,
}

impl DocumentWriteExternalScriptFetchTarget {
    pub(crate) fn new(
        task_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        load_id: u64,
    ) -> Self {
        Self {
            task_owner,
            load_id,
        }
    }

    pub(crate) fn task_owner(self) -> crate::frame_owner_model::FrameDocumentTaskOwner {
        self.task_owner
    }

    pub(crate) fn load_id(self) -> u64 {
        self.load_id
    }
}

/// Producer-captured Network attribution for a `document.write()` script.
///
/// This describes protocol output only. It must never participate in
/// executable-owner authorization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DocumentWriteExternalScriptNetworkAttribution {
    document_url: Url,
    request_url: Url,
}

impl DocumentWriteExternalScriptNetworkAttribution {
    pub(crate) fn new(document_url: Url, request_url: Url) -> Self {
        Self {
            document_url,
            request_url,
        }
    }

    pub(crate) fn document_url(&self) -> &Url {
        &self.document_url
    }

    pub(crate) fn request_url(&self) -> &Url {
        &self.request_url
    }
}

#[derive(Debug)]
pub(super) struct DocumentWriteExternalScriptLoadCompletion {
    target: DocumentWriteExternalScriptFetchTarget,
    result: std::result::Result<String, String>,
    network_result: Option<SharedNavigationResponseResult>,
    network_attribution: DocumentWriteExternalScriptNetworkAttribution,
}

impl DocumentWriteExternalScriptLoadCompletion {
    pub(crate) fn new(
        target: DocumentWriteExternalScriptFetchTarget,
        result: std::result::Result<String, String>,
        network_result: Option<SharedNavigationResponseResult>,
        network_attribution: DocumentWriteExternalScriptNetworkAttribution,
    ) -> Self {
        Self {
            target,
            result,
            network_result,
            network_attribution,
        }
    }

    pub(crate) fn target(&self) -> DocumentWriteExternalScriptFetchTarget {
        self.target
    }

    pub(crate) fn network_result(&self) -> Option<&SharedNavigationResponseResult> {
        self.network_result.as_ref()
    }

    pub(crate) fn network_attribution(&self) -> &DocumentWriteExternalScriptNetworkAttribution {
        &self.network_attribution
    }

    pub(crate) fn into_result(self) -> std::result::Result<String, String> {
        self.result
    }

    #[cfg(test)]
    pub(crate) fn load_id(&self) -> u64 {
        self.target.load_id()
    }

    #[cfg(test)]
    pub(crate) fn for_test(load_id: u64) -> Self {
        let task_owner = crate::frame_owner_model::FrameDocumentTaskOwner::new(
            crate::frame_owner_model::FrameSchedulerLaneId(71),
            crate::frame_owner_model::LocalWindowId(73),
            crate::frame_owner_model::DocumentId(79),
        );
        Self::new(
            DocumentWriteExternalScriptFetchTarget::new(task_owner, load_id),
            Ok("window.documentWriteExternalScriptLoaded = true".to_owned()),
            None,
            DocumentWriteExternalScriptNetworkAttribution::new(
                Url::parse("https://document-write.test/document").unwrap(),
                Url::parse(&format!("https://document-write.test/script-{load_id}.js")).unwrap(),
            ),
        )
    }
}

#[derive(Debug)]
pub(super) struct ChildClassicScriptNetworkAttribution {
    pub(super) frame_id: Option<String>,
    pub(super) document_url: Url,
    pub(super) request_url: Url,
}

#[derive(Debug)]
pub(super) struct ChildClassicScriptLoadCompletion {
    pub(super) owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    pub(super) load_id: u64,
    pub(super) handle: NativeNodeId,
    pub(super) script_handle: NativeNodeId,
    pub(super) result: std::result::Result<String, String>,
    pub(super) network_result: Option<SharedNavigationResponseResult>,
    pub(super) network_attribution: ChildClassicScriptNetworkAttribution,
}

#[derive(Debug)]
pub(super) struct ChildBlockingStylesheetNetworkResult {
    pub(super) frame_id: Option<String>,
    pub(super) document_url: Url,
    pub(super) request_url: Url,
    pub(super) initiator_type: SubresourceRequestInitiatorType,
    pub(super) terminal: crate::stylesheet_blocking::StylesheetFetchTerminal,
}

#[derive(Debug)]
pub(super) struct ChildBlockingStylesheetLoadCompletion {
    pub(super) child_handle: NativeNodeId,
    pub(super) owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    pub(super) signature: crate::DocumentBlockingStylesheetSignature,
    pub(super) network_results: Vec<ChildBlockingStylesheetNetworkResult>,
}

impl ChildBlockingStylesheetLoadCompletion {
    pub(super) fn successful(&self) -> bool {
        self.network_results
            .iter()
            .all(|result| result.terminal.is_ready())
    }
}

/// Immutable protocol/network attribution captured before an async child
/// module fetch starts.
///
/// This record deliberately contains no executable child/document/realm
/// identity. Authorization belongs exclusively to
/// `ChildDocumentModuleFetchTarget`.
#[derive(Clone, Debug)]
pub(super) struct ChildModuleFetchNetworkAttribution {
    frame_id: Option<String>,
    document_url: Url,
    request_url: Url,
    initiator_type: SubresourceRequestInitiatorType,
}

impl ChildModuleFetchNetworkAttribution {
    pub(super) fn parser(frame_id: Option<String>, document_url: Url, request_url: Url) -> Self {
        Self {
            frame_id,
            document_url,
            request_url,
            initiator_type: SubresourceRequestInitiatorType::Parser,
        }
    }

    pub(super) fn dynamic_import(
        frame_id: Option<String>,
        document_url: Url,
        request_url: Url,
    ) -> Self {
        Self {
            frame_id,
            document_url,
            request_url,
            initiator_type: SubresourceRequestInitiatorType::Script,
        }
    }

    pub(super) fn frame_id(&self) -> Option<&str> {
        self.frame_id.as_deref()
    }

    pub(super) fn document_url(&self) -> &Url {
        &self.document_url
    }

    pub(super) fn request_url(&self) -> &Url {
        &self.request_url
    }

    pub(super) fn initiator_type(&self) -> SubresourceRequestInitiatorType {
        self.initiator_type
    }
}

#[derive(Debug)]
pub(super) struct ChildParserModuleRootFetchCompletion {
    target: crate::frame_owner_model::ChildDocumentModuleFetchTarget,
    request_id: crate::frame_owner_model::FrameRequestId,
    request_key: crate::module_runtime::ModuleMapKey,
    result: std::result::Result<crate::module_runtime::ModuleGraphFetchedSource, String>,
    network_result: Option<SharedNavigationResponseResult>,
    network_attribution: ChildModuleFetchNetworkAttribution,
}

impl ChildParserModuleRootFetchCompletion {
    pub(super) fn new(
        target: crate::frame_owner_model::ChildDocumentModuleFetchTarget,
        request_id: crate::frame_owner_model::FrameRequestId,
        request_key: crate::module_runtime::ModuleMapKey,
        result: std::result::Result<crate::module_runtime::ModuleGraphFetchedSource, String>,
        network_result: Option<SharedNavigationResponseResult>,
        network_attribution: ChildModuleFetchNetworkAttribution,
    ) -> Self {
        Self {
            target,
            request_id,
            request_key,
            result,
            network_result,
            network_attribution,
        }
    }

    pub(super) fn target(&self) -> crate::frame_owner_model::ChildDocumentModuleFetchTarget {
        self.target
    }

    pub(super) fn request_id(&self) -> crate::frame_owner_model::FrameRequestId {
        self.request_id
    }

    pub(super) fn network_result(&self) -> Option<&SharedNavigationResponseResult> {
        self.network_result.as_ref()
    }

    pub(super) fn network_attribution(&self) -> &ChildModuleFetchNetworkAttribution {
        &self.network_attribution
    }

    pub(super) fn into_module_terminal_parts(
        self,
    ) -> (
        crate::frame_owner_model::ChildDocumentModuleFetchTarget,
        crate::module_runtime::ModuleMapKey,
        std::result::Result<crate::module_runtime::ModuleGraphFetchedSource, String>,
    ) {
        (self.target, self.request_key, self.result)
    }
}

#[derive(Debug)]
pub(super) struct ChildModuleDependencyFetchCompletion {
    target: crate::frame_owner_model::ChildDocumentModuleFetchTarget,
    request_id: crate::frame_owner_model::FrameRequestId,
    task: crate::frame_owner_model::FrameDocumentModuleDependencyFetchTask,
    result: std::result::Result<crate::module_runtime::ModuleGraphFetchedSource, String>,
    network_result: Option<SharedNavigationResponseResult>,
    network_attribution: ChildModuleFetchNetworkAttribution,
}

impl ChildModuleDependencyFetchCompletion {
    pub(super) fn new(
        child_handle: crate::dom::native::NativeNodeId,
        request_id: crate::frame_owner_model::FrameRequestId,
        task: crate::frame_owner_model::FrameDocumentModuleDependencyFetchTask,
        result: std::result::Result<crate::module_runtime::ModuleGraphFetchedSource, String>,
        network_result: Option<SharedNavigationResponseResult>,
        network_attribution: ChildModuleFetchNetworkAttribution,
    ) -> Self {
        let target = crate::frame_owner_model::ChildDocumentModuleFetchTarget::new(
            child_handle,
            task.owner(),
            task.realm_id(),
        );
        Self {
            target,
            request_id,
            task,
            result,
            network_result,
            network_attribution,
        }
    }

    pub(super) fn target(&self) -> crate::frame_owner_model::ChildDocumentModuleFetchTarget {
        self.target
    }

    pub(super) fn request_id(&self) -> crate::frame_owner_model::FrameRequestId {
        self.request_id
    }

    pub(super) fn network_result(&self) -> Option<&SharedNavigationResponseResult> {
        self.network_result.as_ref()
    }

    pub(super) fn network_attribution(&self) -> &ChildModuleFetchNetworkAttribution {
        &self.network_attribution
    }

    pub(super) fn into_module_terminal_parts(
        self,
    ) -> (
        crate::frame_owner_model::FrameDocumentModuleDependencyFetchTask,
        std::result::Result<crate::module_runtime::ModuleGraphFetchedSource, String>,
    ) {
        (self.task, self.result)
    }
}

/// Completion of one child-document dynamic-import fetch.
///
/// The executable target and protocol attribution are captured independently
/// before the native fetch starts. The stable Page queue adds the root
/// `RendererDocumentToken`; this payload supplies the exact PageVm-local
/// child/document/realm target.
#[derive(Debug)]
pub(super) struct ChildDynamicImportFetchCompletion {
    target: crate::frame_owner_model::ChildDocumentModuleFetchTarget,
    load_id: u64,
    result: std::result::Result<crate::module_runtime::ModuleGraphFetchedSource, String>,
    network_result: Option<SharedNavigationResponseResult>,
    network_attribution: ChildModuleFetchNetworkAttribution,
}

impl ChildDynamicImportFetchCompletion {
    pub(super) fn new(
        target: crate::frame_owner_model::ChildDocumentModuleFetchTarget,
        load_id: u64,
        result: std::result::Result<crate::module_runtime::ModuleGraphFetchedSource, String>,
        network_result: Option<SharedNavigationResponseResult>,
        network_attribution: ChildModuleFetchNetworkAttribution,
    ) -> Self {
        Self {
            target,
            load_id,
            result,
            network_result,
            network_attribution,
        }
    }

    pub(super) fn target(&self) -> crate::frame_owner_model::ChildDocumentModuleFetchTarget {
        self.target
    }

    pub(super) fn network_result(&self) -> Option<&SharedNavigationResponseResult> {
        self.network_result.as_ref()
    }

    pub(super) fn network_attribution(&self) -> &ChildModuleFetchNetworkAttribution {
        &self.network_attribution
    }

    pub(super) fn into_terminal_parts(
        self,
    ) -> (
        crate::frame_owner_model::ChildDocumentModuleFetchTarget,
        u64,
        std::result::Result<crate::module_runtime::ModuleGraphFetchedSource, String>,
    ) {
        (self.target, self.load_id, self.result)
    }
}

/// Completion of one child-document `modulepreload` fetch.
///
/// The executable target and protocol attribution are captured independently
/// before the native fetch starts. The stable Page queue adds the root
/// `RendererDocumentToken`; this payload supplies the exact PageVm-local
/// child/document/realm target.
#[derive(Debug)]
pub(super) struct ChildModulepreloadFetchCompletion {
    target: crate::frame_owner_model::ChildDocumentModuleFetchTarget,
    load_id: u64,
    result: std::result::Result<crate::module_runtime::ModuleGraphFetchedSource, String>,
    network_result: Option<SharedNavigationResponseResult>,
    network_attribution: ChildModuleFetchNetworkAttribution,
}

impl ChildModulepreloadFetchCompletion {
    pub(super) fn new(
        target: crate::frame_owner_model::ChildDocumentModuleFetchTarget,
        load_id: u64,
        result: std::result::Result<crate::module_runtime::ModuleGraphFetchedSource, String>,
        network_result: Option<SharedNavigationResponseResult>,
        network_attribution: ChildModuleFetchNetworkAttribution,
    ) -> Self {
        Self {
            target,
            load_id,
            result,
            network_result,
            network_attribution,
        }
    }

    pub(super) fn target(&self) -> crate::frame_owner_model::ChildDocumentModuleFetchTarget {
        self.target
    }

    pub(super) fn network_result(&self) -> Option<&SharedNavigationResponseResult> {
        self.network_result.as_ref()
    }

    pub(super) fn network_attribution(&self) -> &ChildModuleFetchNetworkAttribution {
        &self.network_attribution
    }

    pub(super) fn into_module_terminal_parts(
        self,
    ) -> (
        crate::frame_owner_model::ChildDocumentModuleFetchTarget,
        u64,
        std::result::Result<crate::module_runtime::ModuleGraphFetchedSource, String>,
    ) {
        (self.target, self.load_id, self.result)
    }
}

#[derive(Debug)]
pub(super) struct LoadedChildDocument {
    pub(super) final_url: Url,
    pub(super) policy_container: DocumentPolicyContainer,
    pub(super) content_type: Option<String>,
    pub(super) character_set: String,
    pub(super) markup: String,
    pub(super) document_network: Option<crate::protocol_types::ChildFrameDocumentNetworkSnapshot>,
}

#[derive(Debug)]
pub(super) enum ChildDocumentLoadOutcome {
    Loaded(Box<LoadedChildDocument>),
    IgnoredNavigation,
}

/// Immutable frame/protocol attribution captured before a child-document
/// navigation fetch starts.
///
/// This record is intentionally separate from the executable navigation
/// target. It remains valid when the initiating Document is replaced, but none
/// of its fields may authorize a commit into the then-current PageVm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ChildDocumentLoadNetworkAttribution {
    frame_id: String,
    parent_frame_id: Option<String>,
    loader_id: String,
}

impl ChildDocumentLoadNetworkAttribution {
    pub(super) fn new(
        frame_id: String,
        parent_frame_id: Option<String>,
        loader_id: String,
    ) -> Self {
        Self {
            frame_id,
            parent_frame_id,
            loader_id,
        }
    }

    pub(super) fn frame_id(&self) -> &str {
        &self.frame_id
    }

    pub(super) fn parent_frame_id(&self) -> Option<&str> {
        self.parent_frame_id.as_deref()
    }

    pub(super) fn loader_id(&self) -> &str {
        &self.loader_id
    }
}

#[derive(Debug)]
pub(super) struct ChildDocumentLoadCompletion {
    target: crate::frame_owner_model::ChildDocumentNavigationFetchTarget,
    network_attribution: ChildDocumentLoadNetworkAttribution,
    result: std::result::Result<ChildDocumentLoadOutcome, String>,
}

impl ChildDocumentLoadCompletion {
    pub(super) fn new(
        target: crate::frame_owner_model::ChildDocumentNavigationFetchTarget,
        network_attribution: ChildDocumentLoadNetworkAttribution,
        result: std::result::Result<ChildDocumentLoadOutcome, String>,
    ) -> Self {
        Self {
            target,
            network_attribution,
            result,
        }
    }

    pub(super) fn target(&self) -> crate::frame_owner_model::ChildDocumentNavigationFetchTarget {
        self.target
    }

    #[cfg(test)]
    pub(super) fn load_id(&self) -> u64 {
        self.target.load_id()
    }

    pub(super) fn network_attribution(&self) -> &ChildDocumentLoadNetworkAttribution {
        &self.network_attribution
    }

    pub(super) fn document_network(
        &self,
    ) -> Option<&crate::protocol_types::ChildFrameDocumentNetworkSnapshot> {
        match &self.result {
            Ok(ChildDocumentLoadOutcome::Loaded(loaded)) => loaded.document_network.as_ref(),
            Ok(ChildDocumentLoadOutcome::IgnoredNavigation) | Err(_) => None,
        }
    }

    pub(super) fn into_application_parts(
        self,
    ) -> (
        crate::frame_owner_model::ChildDocumentNavigationFetchTarget,
        ChildDocumentLoadNetworkAttribution,
        std::result::Result<ChildDocumentLoadOutcome, String>,
    ) {
        (self.target, self.network_attribution, self.result)
    }

    #[cfg(test)]
    pub(super) fn for_test(
        load_id: u64,
        child_handle: NativeNodeId,
        result: std::result::Result<ChildDocumentLoadOutcome, String>,
    ) -> Self {
        use crate::frame_owner_model::{
            ChildDocumentNavigationFetchTarget, DocumentId, FrameDocumentTaskOwner, FrameRequestId,
            FrameSchedulerLaneId, LocalWindowId,
        };

        let target = ChildDocumentNavigationFetchTarget::for_test(
            child_handle,
            FrameDocumentTaskOwner::new(
                FrameSchedulerLaneId(load_id),
                LocalWindowId(load_id),
                DocumentId(load_id),
            ),
            load_id,
            FrameRequestId(load_id),
        );
        Self::new(
            target,
            ChildDocumentLoadNetworkAttribution::new(
                format!("TEST-CHILD-FRAME-{load_id}"),
                None,
                format!("TEST-CHILD-LOADER-{load_id}"),
            ),
            result,
        )
    }
}

#[derive(Debug)]
pub(super) struct PopupDocumentLoadCompletion {
    target: crate::native_bridge::LightweightPopupDocumentFetchTarget,
    pub(super) result: std::result::Result<PopupDocumentLoadOutcome, String>,
}

impl PopupDocumentLoadCompletion {
    pub(crate) fn new(
        target: crate::native_bridge::LightweightPopupDocumentFetchTarget,
        result: std::result::Result<PopupDocumentLoadOutcome, String>,
    ) -> Self {
        Self { target, result }
    }

    pub(crate) fn target(&self) -> crate::native_bridge::LightweightPopupDocumentFetchTarget {
        self.target
    }
}

#[derive(Debug)]
pub(super) struct PopupClassicScriptLoadCompletion {
    target: crate::native_bridge::LightweightPopupClassicScriptFetchTarget,
    pub(super) result: std::result::Result<LoadedChildScriptSource, String>,
}

impl PopupClassicScriptLoadCompletion {
    pub(crate) fn new(
        target: crate::native_bridge::LightweightPopupClassicScriptFetchTarget,
        result: std::result::Result<LoadedChildScriptSource, String>,
    ) -> Self {
        Self { target, result }
    }

    pub(crate) fn target(&self) -> crate::native_bridge::LightweightPopupClassicScriptFetchTarget {
        self.target
    }
}

#[derive(Debug)]
pub(super) enum PopupDocumentLoadOutcome {
    Loaded(Box<LoadedChildDocument>),
    IgnoredNavigation,
}

#[derive(Debug, Clone)]
pub(super) struct LoadedChildScriptSource {
    pub(super) final_url: Url,
    pub(super) redirected: bool,
    pub(super) source: String,
}

#[cfg(test)]
#[derive(Debug)]
pub(super) struct ModuleGraphFetchCompletion {
    pub(super) load_id: u64,
    pub(super) requester: ModuleGraphFetchRequester,
    pub(super) ordering: ModuleGraphFetchOrdering,
    pub(super) request_url: Url,
    pub(super) result: std::result::Result<crate::module_runtime::ModuleGraphFetchedSource, String>,
    pub(super) network_result: Option<SharedNavigationResponseResult>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ModuleGraphFetchRequester {
    ParserOwnedModuleScript,
    RuntimeOwnedModuleScript,
    DynamicImport,
    ModulePreload,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ModuleGraphFetchOrdering {
    DclCritical,
    Runtime,
    BackgroundPreload,
}

#[cfg(test)]
mod tests {
    use super::{ImageRequestCorsMode, ImageRequestKey, PendingSubresourceContinuation};

    #[test]
    fn image_request_identity_canonicalizes_cross_origin_state() {
        assert_eq!(
            ImageRequestCorsMode::from_cross_origin_attribute(None),
            ImageRequestCorsMode::NoCors
        );
        assert_eq!(
            ImageRequestCorsMode::from_cross_origin_attribute(Some("invalid")),
            ImageRequestCorsMode::Anonymous
        );
        assert_eq!(
            ImageRequestCorsMode::from_cross_origin_attribute(Some(" USE-CREDENTIALS ")),
            ImageRequestCorsMode::UseCredentials
        );

        let url = "https://example.test/image.png".to_owned();
        assert_ne!(
            ImageRequestKey::with_density(url.clone(), ImageRequestCorsMode::NoCors, 1.0),
            ImageRequestKey::with_density(url.clone(), ImageRequestCorsMode::Anonymous, 1.0),
            "cache and in-flight identity must include the canonical CORS mode"
        );
        assert_ne!(
            ImageRequestKey::with_density(url.clone(), ImageRequestCorsMode::NoCors, 1.0),
            ImageRequestKey::with_density(url, ImageRequestCorsMode::NoCors, 2.0),
            "responsive-image identity must include the selected candidate density"
        );
    }

    #[test]
    fn element_resource_continuations_identify_document_load_event_delays() {
        let image = PendingSubresourceContinuation::Image {
            image_handle: crate::document_runtime::DomHandle::new(1),
            sequence: crate::native_bridge::ImageLoadEventId::new(1),
            request_initiator_type: crate::types::SubresourceRequestInitiatorType::Parser,
        };
        let media = PendingSubresourceContinuation::Media {
            media_handle: crate::document_runtime::DomHandle::new(2),
            sequence: crate::native_bridge::MediaLoadSequenceId::new(2),
        };
        let text_track = PendingSubresourceContinuation::TextTrack {
            track_handle: crate::document_runtime::DomHandle::new(3),
            sequence: crate::native_bridge::TextTrackLoadSequenceId::new(3),
        };
        let stylesheet_subresource = PendingSubresourceContinuation::StylesheetSubresource {
            binding: crate::frame_owner_model::StylesheetSubresourceLoadDelayBinding::Main {
                owner: crate::frame_owner_model::FrameDocumentTaskOwner::new(
                    crate::frame_owner_model::FrameSchedulerLaneId(4),
                    crate::frame_owner_model::LocalWindowId(4),
                    crate::frame_owner_model::DocumentId(4),
                ),
                load_delay_token: None,
            },
            web_font: None,
            css_image: None,
        };

        assert!(image.delays_document_load_event());
        assert_eq!(
            image.request_initiator_type(),
            crate::types::SubresourceRequestInitiatorType::Parser
        );
        assert_eq!(
            media.request_initiator_type(),
            crate::types::SubresourceRequestInitiatorType::Other
        );
        assert_eq!(
            stylesheet_subresource.request_initiator_type(),
            crate::types::SubresourceRequestInitiatorType::Css
        );
        assert!(media.delays_document_load_event());
        assert!(text_track.delays_document_load_event());
        assert!(stylesheet_subresource.delays_document_load_event());
        assert!(!PendingSubresourceContinuation::Beacon.delays_document_load_event());
    }
}
