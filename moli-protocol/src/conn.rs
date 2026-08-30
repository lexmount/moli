use std::{
    cell::OnceCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use moli_cookie_jar::{StoredCookie, StoredCookieQueryReport};
use moli_fetch::FetchConfig;
use parking_lot::Mutex;
use serde_json::json;

use crate::devtools_runtime::{
    DevToolsCommandContext, DevToolsTargetFilterEntry, DevToolsTargetInfo, DevToolsTargetKind,
};
use crate::domains::command_output::{BackgroundProtocolEventBuffer, CommandOutputBuffer};

use moli_core::{
    LayoutPolicy, OptionalResourceFetchMask, RendererOutputPublicationOrdering,
    RendererOutputTransportMessage,
    network::{SharedWebStorageStore, new_shared_web_storage_store},
    page::{NavigationResponse, Page, SubresourceAuthCredentials},
    runtime::{
        NavigationEngine, NavigationRuntimeConfig, storage_partition::StoragePartitionState,
    },
};

pub const DEFAULT_CDP_PAGE_TARGET_ID: &str = "moli-default";
pub const DEFAULT_CDP_TAB_TARGET_ID: &str = "moli-default-tab";

mod activity_source;
mod bidi_channel_work;
mod body_spool;
mod browser_context;
mod command_owner_scope;
mod command_view;
mod cookie_manager_surface;
mod cookie_owner;
mod cookie_policy_surface;
#[cfg(test)]
mod cookie_store_boundary;
mod devtools_command;
mod dispatch;
mod downloads;
mod fetch_support;
mod inspector_route;
mod output;
mod page_state;
mod popup_activation_work;
mod popup_navigation_work;
mod protocol_output;
mod renderer_command_turn;
mod resource_runtime_support;
mod runtime_eval;
mod runtime_load;
mod scheduler_hooks;
mod scheduler_state;
mod settings;
#[cfg(test)]
mod site_data_manager_surface;
mod state;
mod target;
mod top_level_navigation_work;

pub use crate::domains::network::IoStreamState;
#[cfg(test)]
pub(crate) use bidi_channel_work::BidiChannelOwnerActionKind;
pub(crate) use bidi_channel_work::{
    BidiChannelListenerResidence, BidiChannelOwnerAction, BidiChannelOwnerActionBody,
    BidiChannelPageOwner,
};
pub(crate) use body_spool::{CapturedBody, CapturedBodyWriter};
pub(crate) use browser_context::{
    PageLifecycleEventsEnableResult, SessionOwnerInspectorEnableResult,
    SessionOwnerRuntimeFrontendEnableResult, TargetEmulationSessionStateMut,
    TargetNavigationLoadInputs,
};
pub(crate) use command_owner_scope::CommandOwnerScope;
pub use command_view::Cmd;
pub(crate) use cookie_manager_surface::BrowserContextCookieManagerSurfaceSnapshot;
#[cfg(test)]
pub(crate) use cookie_manager_surface::{
    BrowserContextCookieBackendConnectionState, BrowserContextDefaultCookieWriteUrlSource,
    BrowserContextDocumentCookieCacheLookupResult, BrowserContextFirstCookieRequest,
    BrowserContextStructuredCookieCommandVerdict, BrowserContextStructuredCookieWriteBackendStatus,
    BrowserContextStructuredCookieWriteReadinessStatus,
};
#[cfg(test)]
pub(crate) use cookie_owner::{
    BrowserContextCookieGetFreshnessStatus, BrowserContextCookieSetReadinessStatus,
};
pub use devtools_command::DevToolsCommandDispatchOutcome;
pub(crate) use devtools_command::DevToolsCommandExecutionOutput;
pub use dispatch::{CdpCommandTaskStep, CompletedCdpCommandDispatch, PendingCdpCommandDispatch};
pub(crate) use downloads::SharedDownloadRegistry;
pub(crate) use fetch_support::PendingStreamingDocumentResponseNavigation;
pub(crate) use fetch_support::{
    ClaimedSubresourceContinueRequest, CompletedFetchResponseBodyStreamReadDispatch,
    PendingFetchResponseBodyStreamRead, PendingFetchResponseBodyStreamReadDispatch,
    PendingFetchResponseBodyStreamReadStart, PendingSubresourceFetchResidence,
};
pub use fetch_support::{
    DocumentBodySource, FetchAuthChallenge, FetchInterceptionPattern, FetchRequestStage,
    FetchResourceTypeFilter, InFlightSubresourceFetchRequest, PausedDocumentTransfer,
    PausedDocumentTransfers, PendingFetchAuthNavigation, PendingFetchNavigation,
    PendingFetchResponseOpenedBodyStream, PendingSubresourceFetchAuthRequest,
    PendingSubresourceFetchAuthStage, PendingSubresourceFetchAuthStageChain,
    PendingSubresourceFetchOwnerKind, PendingSubresourceFetchRequest,
    PendingSubresourceFetchRequestStage, PendingSubresourceFetchRequestStageChain,
    PendingSubresourceFetchResponseRequest, PendingSubresourceFetchResponseStage,
    PendingSubresourceFetchResponseStageChain, ResponseStageUrlMatchPolicy,
    fetch_subresource_interception_config, fetch_subresource_interception_config_for_patterns,
};
pub use moli_protocol_cdp::{
    CdpRendererCommandAccess, CdpRendererCommandPolicy, CdpRendererCommandReplacement,
    CdpRendererCommandReplayDispatch, CdpRequest, ParsedCdpCommand,
};
use target::DEFAULT_BROWSER_CONTEXT_ID;
pub(crate) use target::{
    CdpSessionRoute, DefaultTargetLifecycle, TargetActivationTransition, TargetHandlerAccessMode,
};

#[derive(Clone, Debug)]
pub enum CdpTargetHostLifecycleDelta {
    Created(DevToolsTargetInfo),
    InfoChanged(DevToolsTargetInfo),
    Activated { target_id: String },
    Destroyed { target_id: String },
}

#[derive(Clone)]
pub struct CdpTargetHostLifecycleObserver {
    callback: Arc<dyn Fn(CdpTargetHostLifecycleDelta) + Send + Sync>,
}

impl CdpTargetHostLifecycleObserver {
    pub fn new(callback: impl Fn(CdpTargetHostLifecycleDelta) + Send + Sync + 'static) -> Self {
        Self {
            callback: Arc::new(callback),
        }
    }

    fn notify(&self, delta: CdpTargetHostLifecycleDelta) {
        (self.callback)(delta);
    }
}

/// The unique authority to publish that one command response has entered the
/// protocol output sequence.
///
/// This value is deliberately not `Clone`: observers may be cloned freely,
/// but only the command dispatcher may release (or drop/cancel) the waiters
/// associated with this exact command.
#[must_use = "dropping the permit cancels observers waiting for this command response"]
pub struct CommandResponseFlushPermit {
    sender: tokio::sync::watch::Sender<bool>,
    deferred_releases: Arc<Mutex<CommandResponseFlushDeferredReleases>>,
}

struct CommandResponseFlushRelease {
    release: Option<Box<dyn FnOnce() + Send + 'static>>,
}

impl CommandResponseFlushRelease {
    fn new(release: impl FnOnce() + Send + 'static) -> Self {
        Self {
            release: Some(Box::new(release)),
        }
    }

    fn run(mut self) {
        self.run_inner();
    }

    fn run_inner(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

impl Drop for CommandResponseFlushRelease {
    fn drop(&mut self) {
        self.run_inner();
    }
}

#[derive(Default)]
struct CommandResponseFlushDeferredReleases {
    finished: bool,
    releases: Vec<CommandResponseFlushRelease>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DevToolsDocumentLifecycleWaitKey {
    registration_id: state::RendererDocumentLifecycleWaiterId,
    renderer_document: moli_core::page::RendererDocumentToken,
    renderer_epoch: moli_core::page::RendererLifecycleEpoch,
    milestone: moli_core::page::RendererDocumentLifecycleMilestone,
    frame_id: String,
    loader_id: String,
}

impl DevToolsDocumentLifecycleWaitKey {
    pub fn frame_id(&self) -> &str {
        self.frame_id.as_str()
    }

    pub fn milestone(&self) -> moli_core::page::RendererDocumentLifecycleMilestone {
        self.milestone
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DevToolsDocumentLifecycleWaitState {
    Pending,
    Reached,
    Interrupted,
    Superseded,
    Unavailable,
}

/// Current top-level Document readiness for one exact DevTools target route.
///
/// This deliberately distinguishes a live target that has not committed its
/// next Document from a target that no longer exists. WebDriver uses the
/// distinction to wait at the browsing-context boundary instead of probing a
/// renderer command until it happens to stop returning `NoDocumentLoaded`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DevToolsDocumentNavigationState {
    Unavailable,
    PendingNavigation,
    AwaitingCommit,
    Committed { loader_id: String },
}

fn devtools_document_lifecycle_wait_state_for_slot(
    slot: &TargetRuntimeSlot,
    key: &DevToolsDocumentLifecycleWaitKey,
) -> DevToolsDocumentLifecycleWaitState {
    let page_slot = slot.page_slot();
    let Some(outcome) = page_slot.renderer_document_lifecycle_waiter_outcome(
        key.registration_id,
        key.renderer_document,
        key.renderer_epoch,
        &key.frame_id,
        &key.loader_id,
    ) else {
        return DevToolsDocumentLifecycleWaitState::Superseded;
    };
    match outcome {
        moli_core::page::RendererDocumentLifecycleWaitOutcome::Reached(_) => {
            DevToolsDocumentLifecycleWaitState::Reached
        }
        moli_core::page::RendererDocumentLifecycleWaitOutcome::Interrupted(_) => {
            DevToolsDocumentLifecycleWaitState::Interrupted
        }
        moli_core::page::RendererDocumentLifecycleWaitOutcome::Pending => {
            match page_slot.renderer_document_lifecycle_binding() {
                None => DevToolsDocumentLifecycleWaitState::Unavailable,
                Some(binding)
                    if binding.renderer_document == key.renderer_document
                        && binding.renderer_epoch == key.renderer_epoch
                        && binding.frame_id == key.frame_id
                        && binding.loader_id == key.loader_id =>
                {
                    DevToolsDocumentLifecycleWaitState::Pending
                }
                Some(_) => DevToolsDocumentLifecycleWaitState::Superseded,
            }
        }
    }
}

impl CommandResponseFlushPermit {
    fn finish_deferred_releases(&self) -> Vec<CommandResponseFlushRelease> {
        let mut deferred = self.deferred_releases.lock();
        if deferred.finished {
            Vec::new()
        } else {
            deferred.finished = true;
            std::mem::take(&mut deferred.releases)
        }
    }

    pub fn finish(self) {
        let releases = self.finish_deferred_releases();
        let _ = self.sender.send(true);
        for release in releases {
            release.run();
        }
    }
}

impl Drop for CommandResponseFlushPermit {
    fn drop(&mut self) {
        for release in self.finish_deferred_releases() {
            release.run();
        }
    }
}

/// Cloneable, read-only observation of one command response flush.
///
/// Cloning this context creates another observer of the same command. It never
/// creates another authority capable of releasing that command's waiters.
#[derive(Clone, Default)]
pub struct CommandResponseFlushContext {
    receiver: Option<tokio::sync::watch::Receiver<bool>>,
    deferred_releases: Option<Arc<Mutex<CommandResponseFlushDeferredReleases>>>,
}

impl CommandResponseFlushContext {
    fn new(
        receiver: tokio::sync::watch::Receiver<bool>,
        deferred_releases: Arc<Mutex<CommandResponseFlushDeferredReleases>>,
    ) -> Self {
        Self {
            receiver: Some(receiver),
            deferred_releases: Some(deferred_releases),
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        self.receiver.is_some()
    }

    pub(crate) fn receiver(&self) -> Option<tokio::sync::watch::Receiver<bool>> {
        self.receiver.clone()
    }

    pub(crate) fn defer_until_response_flush(&self, release: impl FnOnce() + Send + 'static) {
        let release = CommandResponseFlushRelease::new(release);
        let immediate = match &self.deferred_releases {
            Some(deferred_releases) => {
                let mut deferred = deferred_releases.lock();
                if deferred.finished {
                    Some(release)
                } else {
                    deferred.releases.push(release);
                    None
                }
            }
            None => Some(release),
        };
        if let Some(release) = immediate {
            release.run();
        }
    }
}

#[derive(Clone, Default)]
pub struct CommandDispatchContext {
    response_flush: CommandResponseFlushContext,
    terminal_response_delivery_override: Option<moli_page_types::RendererInspectorResponseDelivery>,
    protocol_events: Vec<BackgroundProtocolEvent>,
    post_renderer_output_events: Vec<BackgroundProtocolEvent>,
    renderer_output_boundary: Option<moli_core::RendererOutputFence>,
    post_response_events: Vec<BackgroundProtocolEvent>,
    renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
}

impl CommandDispatchContext {
    pub fn new(response_flush: CommandResponseFlushContext) -> Self {
        Self {
            response_flush,
            terminal_response_delivery_override: None,
            protocol_events: Vec::new(),
            post_renderer_output_events: Vec::new(),
            renderer_output_boundary: None,
            post_response_events: Vec::new(),
            renderer_output_predecessor: None,
        }
    }

    pub(crate) fn response_flush(&self) -> &CommandResponseFlushContext {
        &self.response_flush
    }

    pub(crate) fn set_terminal_response_delivery_override(
        &mut self,
        response_delivery: moli_page_types::RendererInspectorResponseDelivery,
    ) {
        self.terminal_response_delivery_override = Some(response_delivery);
    }

    pub(crate) const fn terminal_response_delivery_override(
        &self,
    ) -> Option<moli_page_types::RendererInspectorResponseDelivery> {
        self.terminal_response_delivery_override
    }

    pub(crate) fn push_protocol_event(&mut self, event: BackgroundProtocolEvent) {
        self.protocol_events_mut().push(event);
    }

    pub(crate) fn protocol_events_mut(&mut self) -> &mut Vec<BackgroundProtocolEvent> {
        if self.renderer_output_boundary.is_some() {
            &mut self.post_renderer_output_events
        } else {
            &mut self.protocol_events
        }
    }

    pub(crate) fn protocol_events_len(&self) -> usize {
        self.protocol_events.len() + self.post_renderer_output_events.len()
    }

    pub(crate) fn take_protocol_events(&mut self) -> Vec<BackgroundProtocolEvent> {
        assert!(
            self.renderer_output_boundary.is_none(),
            "an exact renderer boundary must be consumed with both protocol-event segments"
        );
        std::mem::take(&mut self.protocol_events)
    }

    pub(crate) fn append_renderer_fenced_protocol_events(
        &mut self,
        before_boundary: Vec<BackgroundProtocolEvent>,
        boundary: Option<moli_core::RendererOutputFence>,
        after_boundary: Vec<BackgroundProtocolEvent>,
    ) {
        self.protocol_events_mut().extend(before_boundary);
        let Some(boundary) = boundary else {
            assert!(
                after_boundary.is_empty(),
                "post-renderer events require an exact renderer boundary"
            );
            return;
        };
        assert!(
            self.renderer_output_boundary.is_none(),
            "one command turn cannot contain multiple renderer insertion boundaries"
        );
        self.renderer_output_boundary = Some(boundary);
        self.post_renderer_output_events.extend(after_boundary);
    }

    pub(crate) fn take_renderer_fenced_protocol_events(
        &mut self,
    ) -> (
        Vec<BackgroundProtocolEvent>,
        Option<moli_core::RendererOutputFence>,
        Vec<BackgroundProtocolEvent>,
    ) {
        (
            std::mem::take(&mut self.protocol_events),
            self.renderer_output_boundary.take(),
            std::mem::take(&mut self.post_renderer_output_events),
        )
    }

    pub(crate) fn extend_post_response_events(
        &mut self,
        events: impl IntoIterator<Item = BackgroundProtocolEvent>,
    ) {
        self.post_response_events.extend(events);
    }

    pub(crate) fn take_protocol_events_before_events(
        &mut self,
        events: Vec<BackgroundProtocolEvent>,
    ) -> Vec<BackgroundProtocolEvent> {
        assert!(
            self.renderer_output_boundary.is_none(),
            "an exact renderer boundary cannot be flattened into protocol events"
        );
        let mut protocol_events = self.take_protocol_events();
        protocol_events.extend(events);
        protocol_events
    }

    pub(crate) fn take_post_response_events(&mut self) -> Vec<BackgroundProtocolEvent> {
        std::mem::take(&mut self.post_response_events)
    }

    /// Adds one exact concrete renderer cursor that must cross protocol
    /// ingress before this command's response is exposed.
    ///
    /// A cursor is source-stream scoped. Deduplication is exact and never
    /// widens the fence into a Page- or process-wide watermark.
    pub(crate) fn set_renderer_output_predecessor(
        &mut self,
        predecessor: moli_core::RendererOutputFence,
    ) {
        predecessor.merge_into_same_stream_tail(&mut self.renderer_output_predecessor);
    }

    #[doc(hidden)]
    pub fn take_renderer_output_predecessor(&mut self) -> Option<moli_core::RendererOutputFence> {
        self.renderer_output_predecessor.take()
    }
}

pub(crate) use moli_protocol_cdp::{DEFAULT_LOADER_ID, monotonic_timestamp_seconds};
pub(crate) use output::NavigationBackgroundEvent;
pub use output::{
    BackgroundCommandResponsePayload, BackgroundEventSender, BackgroundProtocolEvent,
    PageScreencastFrameMetadata, RuntimeInspectorAsyncCompletionReceiver,
    RuntimeInspectorResponseReady, RuntimeInspectorResponseReadySender, build_event,
};
pub(crate) use output::{
    BackgroundCommandResponsePayloadRef, BackgroundServiceWorkerErrorMessage,
    BackgroundServiceWorkerRegistration, BackgroundServiceWorkerVersion,
    build_command_success_response,
};
pub(crate) use page_state::{LoadedNavigationPageCommit, LoadedNavigationRendererAttachmentCommit};
pub(crate) use popup_activation_work::PopupTargetActivationAction;
pub(crate) use popup_navigation_work::{
    PopupTargetNavigationKind, PopupTargetNavigationOwnerAction,
};
pub(crate) use runtime_eval::{
    ClaimedPendingInspectorAwait, ClaimedPendingInspectorAwaitOwner, RuntimeBindingCallEvent,
    RuntimeEnableReplayEvent, renderer_command_turn_frontend_protocol_response,
    runtime_remote_object_ids_in_map,
};
pub use runtime_eval::{
    CompletedMoliDiagnosticsDispatch, CompletedRuntimeBindingPageCommandDispatch,
    CompletedRuntimeChildDefaultContextLookupDispatch, CompletedRuntimeEnableEventsDispatch,
    CompletedRuntimeProtocolMessageDispatch, CompletedServiceWorkerRuntimeProtocolMessageDispatch,
    CompletedSharedWorkerRuntimeProtocolMessageDispatch, PendingMoliDiagnosticsDispatch,
    PendingRuntimeBindingPageCommandDispatch, PendingRuntimeChildDefaultContextLookupDispatch,
    PendingRuntimeEnableEventsDispatch, PendingRuntimeProtocolMessageDispatch,
    PendingServiceWorkerRuntimeProtocolMessageDispatch,
    PendingSharedWorkerRuntimeProtocolMessageDispatch,
};
pub(crate) use runtime_load::decode_data_url_response;
pub(crate) use runtime_load::{
    BackgroundNavigationBodyCompletionSink, BackgroundNavigationEarlyResult,
    BackgroundNavigationLoadJob, CompletedInitialDocumentPageBuild, FailedInitialDocumentPageBuild,
    InitialDocumentPageInstallResult, InitialDocumentPageOwner, PausedResponsePreparedDocument,
    PendingInitialDocumentPageBuild, ResponseCommitReady,
};
use scheduler_hooks::CdpSchedulerHooks;
use scheduler_state::CdpConnectionSchedulerState;
pub use scheduler_state::{CdpRendererOwnerTurnOutcome, CdpSchedulerEvent, CdpTurnOutcome};
#[cfg(test)]
pub(crate) use site_data_manager_surface::{
    BrowserContextReservedSiteDataOwnerState, BrowserContextSiteDataManagerOwnerState,
};
pub use state::{
    BackgroundTarget, BrowserContext, BrowserWindowBounds, DevToolsPageResidenceIdentity,
    DocumentStartScript, DownloadNavigation, EmulatedDeviceMetrics, EmulatedGeolocationOverride,
    EmulatedGeolocationOverrideState, EmulatedMediaOverrides, IsolatedWorldDefinition,
    LoadedNavigation, NavigationDispatchState, NavigationLoadOutcome, NavigationRequestLoadPolicy,
    PageNavigationHistoryEntry, ParkedFetchState, ParkedNetworkArtifacts, ParkedPageSessionState,
    PendingNavigationHistoryUpdate, RuntimeBindingDefinition, TargetInfo, URL_BASE,
};
pub(crate) use state::{
    BrowserContextPageStorageHandles, BrowserContextResourceStorageHandles,
    BrowserContextStoragePartitionHandles, CommittedRendererAgentAttachment,
    CommittedRendererDocumentBinding, CompletedDownloadBody, CompletedDownloadBodyArtifact,
    DedicatedWorkerMainScriptOutcome, DedicatedWorkerMainScriptSnapshot,
    DedicatedWorkerTargetState, DevToolsConsoleOutputSessionState, DevToolsLogViolationThreshold,
    DocumentNavigationToken, DuplicatePendingRendererCommand, EmulatedNetworkConditions,
    EmulatedViewportSurface, InspectorCommandDispatch, NETWORK_ERROR_PAGE_URL,
    NavigationResultProjection, NavigationSourceDocumentSecurityContext,
    NetworkErrorPageNavigation, PageScreencastConfig, PageScreencastFormat, ParkedTargetAuxState,
    ParkedTargetOwnerState, PendingBidiChannelListener, PendingInspectorAwait,
    PendingRendererCommandKey, PerformanceTimeDomain, PreparedRendererCallDispatch, ProfilerAction,
    ProfilerInspectorCommand, RendererCommandCorrelation, RendererCommandDescriptor,
    RendererCommandReplay, RendererDocumentLifecycleObservation, RendererDocumentLifecycleObserver,
    RendererMainDocumentCommitSeed, RendererPageResidenceIdentity,
    ServiceWorkerRuntimeExceptionSnapshot, ServiceWorkerTargetState, SharedWorkerTargetState,
    SiteDataClearOptions, TargetIdentityState, TargetInitialEmptyDocumentCreator, TargetOwnerState,
    TargetPageAttachmentId, TargetPageProtocolAttachmentIdentity, TargetPageResidenceIdentity,
    TargetPageResidenceObservation, TargetPageResidenceToken, TargetPageSessionState,
    TargetPreparedJavaScriptDialog, TargetPreparedJavaScriptDialogRoute,
    TargetRootDocumentProtocolAttachmentIdentity, TargetRuntimeSlot,
    TargetServiceWorkerProtocolAttachmentIdentity, TargetServiceWorkerProtocolAttachmentRetirement,
    TargetServiceWorkerRunIdentity, TargetServiceWorkerRunRetirement,
    TargetServiceWorkerRuntimeAttachmentIdentity, TargetServiceWorkerVersionIdentity,
    TargetServiceWorkerVersionRetirement, TargetSharedWorkerProtocolAttachmentIdentity,
    TargetSharedWorkerProtocolAttachmentRetirement, TargetSlotState, TargetWindowSurfaceState,
    viewport_surface_install_script,
};
#[cfg(test)]
pub(crate) use state::{
    DevToolsSessionState, TargetJavaScriptDialog, TargetJavaScriptDialogScopeObserver,
    TargetPageSlot, TargetRuntimeSessionState,
};
pub(crate) use target::{
    PreparedTargetAttach, PreparedTargetHostClosure, PreparedTargetHostDelta,
    TargetAttachSessionCommit, TargetBindingCleanupAction, TargetClosureCleanupPlan,
    TargetSessionDetachCleanupPlan,
};
use target::{
    TargetClosurePlan, TargetControlPlane, TargetEventPlan, TargetHostDelta,
    target_destroyed_automation_events,
};
pub(crate) use top_level_navigation_work::TopLevelLocationNavigationOwnerAction;

pub struct PendingDeferredMainDocumentLoadCompletion {
    inner: crate::domains::activity::PendingDeferredMainDocumentLoadCompletionActivity,
}

pub struct CompletedDeferredMainDocumentLoadCompletion {
    inner: crate::domains::activity::CompletedDeferredMainDocumentLoadCompletionActivity,
}

/// Stable identity of one exact deferred-load lifecycle observation.
///
/// The protocol owner allocates this identity before an adapter starts an
/// asynchronous wait. CDP, BiDi, and Classic carry it through the typed
/// completion instead of manufacturing adapter-local observation generations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeferredMainDocumentLoadObservationId(u64);

impl DeferredMainDocumentLoadObservationId {
    #[cfg(feature = "test-support")]
    pub(crate) fn from_test_value(value: u64) -> Self {
        assert_ne!(value, 0, "load observation identity starts at one");
        Self(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferredMainDocumentLoadCompletionOutputInterest {
    renderer_page: Option<RendererPageResidenceIdentity>,
    renderer_document: Option<moli_core::RendererDocumentLifecycleIdentity>,
}

/// Exact scope of concrete renderer output that may still acquire a
/// main-document load predecessor from the command turn currently completing.
///
/// This value is derived while consuming a one-shot renderer publication. It
/// retains only the Page/Document identity needed for a later load action to
/// prove causality; it carries neither a renderer source capability nor
/// permission to rescan Page state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeferredMainDocumentLoadPredecessorCandidate {
    renderer_page: RendererPageResidenceIdentity,
    renderer_document: moli_core::RendererDocumentLifecycleIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferredMainDocumentLoadCompletionOutputAction {
    ProcessNow,
    Queue,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ConnectionNetworkRequestIdAllocator {
    next_sequence: u64,
}

impl ConnectionNetworkRequestIdAllocator {
    pub(crate) fn allocate_sequence(&mut self) -> u64 {
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .expect("connection network request id sequence exhausted");
        self.next_sequence
    }

    pub(crate) fn allocate_request_id(&mut self) -> String {
        format!("REQ-{}", self.allocate_sequence())
    }

    #[cfg(test)]
    pub(crate) fn next_sequence_for_test(&self) -> u64 {
        self.next_sequence
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IdleNavigationEngineReleaseResult {
    pub(crate) reset: bool,
    pub(crate) reason: &'static str,
    pub(crate) loaded_browser_context_count: usize,
    pub(crate) live_target_browser_context_count: usize,
    pub(crate) retained_background_navigation_engine_count: usize,
}

impl IdleNavigationEngineReleaseResult {
    pub(crate) fn to_protocol_json(self) -> serde_json::Value {
        json!({
            "reset": self.reset,
            "reason": self.reason,
            "loadedBrowserContextCount": self.loaded_browser_context_count,
            "liveTargetBrowserContextCount": self.live_target_browser_context_count,
            "retainedBackgroundNavigationEngineCount": self.retained_background_navigation_engine_count,
        })
    }
}

impl PendingDeferredMainDocumentLoadCompletion {
    pub(crate) fn new(
        inner: crate::domains::activity::PendingDeferredMainDocumentLoadCompletionActivity,
    ) -> Self {
        Self { inner }
    }

    pub fn session_id(&self) -> Option<&str> {
        self.inner.session_id()
    }

    pub fn output_interest(&self) -> DeferredMainDocumentLoadCompletionOutputInterest {
        DeferredMainDocumentLoadCompletionOutputInterest::new(
            self.inner.renderer_page_residence_identity(),
            self.inner.renderer_document_identity(),
        )
    }

    pub fn observation_id(&self) -> DeferredMainDocumentLoadObservationId {
        self.inner.observation_id()
    }

    pub async fn wait(self) -> CompletedDeferredMainDocumentLoadCompletion {
        CompletedDeferredMainDocumentLoadCompletion {
            inner: self.inner.wait().await,
        }
    }
}

impl CompletedDeferredMainDocumentLoadCompletion {
    pub(crate) fn new(
        inner: crate::domains::activity::CompletedDeferredMainDocumentLoadCompletionActivity,
    ) -> Self {
        Self { inner }
    }

    pub fn session_id(&self) -> Option<&str> {
        self.inner.session_id()
    }

    pub fn observation_id(&self) -> DeferredMainDocumentLoadObservationId {
        self.inner.observation_id()
    }
}

impl DeferredMainDocumentLoadCompletionOutputInterest {
    pub(crate) fn new(
        renderer_page: Option<RendererPageResidenceIdentity>,
        renderer_document: Option<moli_core::RendererDocumentLifecycleIdentity>,
    ) -> Self {
        Self {
            renderer_page,
            renderer_document,
        }
    }

    #[cfg(feature = "test-support")]
    pub(crate) fn from_test_residence(
        renderer_page: RendererPageResidenceIdentity,
        renderer_document: Option<moli_core::RendererDocumentLifecycleIdentity>,
    ) -> Self {
        Self::new(Some(renderer_page), renderer_document)
    }

    pub fn route_output_while_waiting(
        &self,
        message: &RendererOutputTransportMessage,
    ) -> DeferredMainDocumentLoadCompletionOutputAction {
        let RendererOutputTransportMessage::Publication(publication) = message else {
            return DeferredMainDocumentLoadCompletionOutputAction::ProcessNow;
        };
        let residence = publication.cursor().stream().residence();
        if !self
            .renderer_page
            .is_some_and(|renderer_page| renderer_page.matches_residence(residence))
        {
            return DeferredMainDocumentLoadCompletionOutputAction::ProcessNow;
        }
        match publication.ordering() {
            RendererOutputPublicationOrdering::AfterPendingPageLoad { source_document }
                if self.renderer_document == Some(source_document) =>
            {
                DeferredMainDocumentLoadCompletionOutputAction::Queue
            }
            RendererOutputPublicationOrdering::Unconstrained
            | RendererOutputPublicationOrdering::AfterPendingPageLoad { .. } => {
                DeferredMainDocumentLoadCompletionOutputAction::ProcessNow
            }
        }
    }

    pub fn observes_predecessor_candidate(
        &self,
        candidate: DeferredMainDocumentLoadPredecessorCandidate,
    ) -> bool {
        self.renderer_page == Some(candidate.renderer_page)
            && self.renderer_document == Some(candidate.renderer_document)
    }
}

impl DeferredMainDocumentLoadPredecessorCandidate {
    /// Selects only work whose browser-visible effects are ordered after the
    /// exact Page's load boundary.
    ///
    /// Parser, module, child-frame and ordinary lifecycle output are load
    /// prerequisites and therefore return `None`. A timer is Page-scoped;
    /// lifecycle action output additionally carries its exact source
    /// Document.
    pub fn from_renderer_publication(publication: &RendererOutputTransportMessage) -> Option<Self> {
        let RendererOutputTransportMessage::Publication(publication) = publication else {
            return None;
        };
        let RendererOutputPublicationOrdering::AfterPendingPageLoad { source_document } =
            publication.ordering()
        else {
            return None;
        };
        Some(Self {
            renderer_page: RendererPageResidenceIdentity::from_residence(
                publication.cursor().stream().residence(),
            )
            .expect("post-load publication ordering is only valid for a Page stream"),
            renderer_document: source_document,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeAwaitJob {
    command_id: u64,
    session_id: Option<String>,
    owner_route: Option<CdpSessionRoute>,
    object_group: Option<String>,
    action: &'static str,
}

impl RuntimeAwaitJob {
    pub(crate) fn new(
        command_id: u64,
        session_id: Option<&str>,
        owner_route: Option<CdpSessionRoute>,
        object_group: Option<&str>,
        action: &'static str,
    ) -> Self {
        Self {
            command_id,
            session_id: session_id.map(str::to_owned),
            owner_route,
            object_group: object_group.map(str::to_owned),
            action,
        }
    }

    pub(crate) fn trace_fields(&self) -> serde_json::Value {
        json!({
            "commandId": self.command_id,
            "sessionId": self.session_id,
            "ownerRoute": self.owner_route.as_ref().map(|route| format!("{route:?}")),
            "objectGroup": self.object_group,
            "action": self.action,
        })
    }

    pub(crate) fn session_id(&self) -> Option<String> {
        self.session_id.clone()
    }
}

#[derive(Clone)]
pub struct CdpInitialStoragePartition {
    handles: BrowserContextStoragePartitionHandles,
    fallback_session_storage_store: SharedWebStorageStore,
}

impl CdpInitialStoragePartition {
    pub fn memory() -> Self {
        Self::new(BrowserContextStoragePartitionHandles::memory())
    }

    pub fn with_cookies(cookies: Vec<StoredCookie>) -> Self {
        Self::new(BrowserContextStoragePartitionHandles::with_initial_cookies(
            cookies,
        ))
    }

    fn new(handles: BrowserContextStoragePartitionHandles) -> Self {
        Self {
            handles,
            fallback_session_storage_store: new_shared_web_storage_store(),
        }
    }

    pub fn from_storage_partition(
        cookies: Vec<StoredCookie>,
        storage_partition: &StoragePartitionState,
    ) -> Self {
        Self::new(
            BrowserContextStoragePartitionHandles::from_storage_partition(
                cookies,
                storage_partition,
            ),
        )
    }

    fn into_parts(self) -> (BrowserContextStoragePartitionHandles, SharedWebStorageStore) {
        (self.handles, self.fallback_session_storage_store)
    }
}

struct CdpInitialStoragePartitionOwner {
    handles: BrowserContextStoragePartitionHandles,
    fallback_session_storage_store: SharedWebStorageStore,
}

impl CdpInitialStoragePartitionOwner {
    fn new(
        handles: BrowserContextStoragePartitionHandles,
        fallback_session_storage_store: SharedWebStorageStore,
    ) -> Self {
        Self {
            handles,
            fallback_session_storage_store,
        }
    }

    fn from_initial_storage_partition(
        initial_storage_partition: CdpInitialStoragePartition,
    ) -> Self {
        let (handles, fallback_session_storage_store) = initial_storage_partition.into_parts();
        Self::new(handles, fallback_session_storage_store)
    }

    fn new_default_browser_context(
        &self,
        id: String,
        http_cache_root: Option<PathBuf>,
        http_cache_max_bytes: Option<u64>,
    ) -> BrowserContext {
        BrowserContext::new_with_storage_partition_handles_and_http_cache(
            id,
            self.handles.clone(),
            http_cache_root,
            http_cache_max_bytes,
        )
    }

    fn resource_storage_handles(&self) -> BrowserContextResourceStorageHandles {
        self.handles
            .resource_storage_handles(self.fallback_session_storage_store.clone())
    }

    fn page_storage_handles(&self) -> BrowserContextPageStorageHandles {
        self.handles
            .page_storage_handles(self.fallback_session_storage_store.clone())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AutoAttachOwnerPolicy {
    wait_for_debugger_on_start: bool,
    target_filter: CdpTargetFilter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CdpTargetFilterEntry {
    pub(crate) exclude: bool,
    pub(crate) target_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CdpTargetFilter {
    entries: Vec<CdpTargetFilterEntry>,
}

impl CdpTargetFilter {
    pub(crate) fn from_entries(entries: Vec<CdpTargetFilterEntry>) -> Self {
        Self { entries }
    }

    pub(crate) fn from_devtools_entries(entries: Vec<DevToolsTargetFilterEntry>) -> Self {
        Self {
            entries: entries
                .into_iter()
                .map(|entry| CdpTargetFilterEntry {
                    exclude: entry.exclude,
                    target_type: entry.target_type,
                })
                .collect(),
        }
    }

    pub(crate) fn to_devtools_entries(&self) -> Vec<DevToolsTargetFilterEntry> {
        self.entries
            .iter()
            .map(|entry| DevToolsTargetFilterEntry {
                exclude: entry.exclude,
                target_type: entry.target_type.clone(),
            })
            .collect()
    }

    pub(crate) fn default_target_discovery() -> Self {
        Self::default_auto_attach()
    }

    pub(crate) fn default_auto_attach() -> Self {
        Self {
            entries: vec![
                CdpTargetFilterEntry {
                    exclude: true,
                    target_type: Some("browser".to_owned()),
                },
                CdpTargetFilterEntry {
                    exclude: true,
                    target_type: Some("tab".to_owned()),
                },
                CdpTargetFilterEntry {
                    exclude: false,
                    target_type: None,
                },
            ],
        }
    }

    pub(crate) fn matches(&self, target_type: &str) -> bool {
        for entry in &self.entries {
            if entry
                .target_type
                .as_deref()
                .is_none_or(|entry_type| entry_type == target_type)
            {
                return !entry.exclude;
            }
        }
        false
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ServiceWorkerAutoAttachRelatedOwner {
    owner_session_id: Option<String>,
    browser_context_id: String,
    registration_id: u64,
    base_version_id: u64,
    script_url: String,
    scope_url: String,
    allow_service_worker_targets: bool,
    wait_for_debugger_on_start: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ServiceWorkerAutoAttachRelatedOwnerSession {
    pub(crate) owner_session_id: Option<String>,
    pub(crate) wait_for_debugger_on_start: bool,
}

/// Drop-addressable slot for the connection's active renderer owner.
///
/// `CdpConnection::drop` must release this engine before joining its extracted
/// BrowserContext network roots. A plain field would only be dropped after the
/// Drop implementation returned, which reverses that order.
struct NavigationEngineOwnerSlot {
    engine: OnceCell<NavigationEngine>,
    runtime_config: NavigationRuntimeConfig,
    renderer_publication_sender: Option<moli_core::RendererOutputTransportSender>,
}

impl NavigationEngineOwnerSlot {
    fn materialized(engine: NavigationEngine) -> Self {
        let runtime_config = engine.runtime_config();
        let slot = Self {
            engine: OnceCell::new(),
            runtime_config,
            renderer_publication_sender: None,
        };
        slot.engine
            .set(engine)
            .expect("fresh navigation engine slot must be empty");
        slot
    }

    fn deferred(runtime_config: NavigationRuntimeConfig) -> Self {
        Self {
            engine: OnceCell::new(),
            runtime_config,
            renderer_publication_sender: None,
        }
    }

    #[cfg(test)]
    fn is_materialized(&self) -> bool {
        self.engine.get().is_some()
    }

    fn runtime_config(&self) -> NavigationRuntimeConfig {
        self.engine
            .get()
            .map(NavigationEngine::runtime_config)
            .unwrap_or_else(|| self.runtime_config.clone())
    }

    fn fetch_config(&self) -> &moli_fetch::FetchConfig {
        self.engine
            .get()
            .map(NavigationEngine::fetch_config)
            .unwrap_or_else(|| self.runtime_config.fetch_config())
    }

    fn layout_policy(&self) -> LayoutPolicy {
        self.engine
            .get()
            .map(NavigationEngine::layout_policy)
            .unwrap_or_else(|| self.runtime_config.layout_policy())
    }

    fn set_renderer_output_transport_sender(
        &mut self,
        sender: moli_core::RendererOutputTransportSender,
    ) {
        if let Some(engine) = self.engine.get() {
            engine.set_renderer_output_transport_sender(sender.clone());
        }
        self.renderer_publication_sender = Some(sender);
    }

    fn replace(&mut self, engine: NavigationEngine) -> Option<NavigationEngine> {
        self.runtime_config = engine.runtime_config();
        if let Some(sender) = self.renderer_publication_sender.as_ref() {
            engine.set_renderer_output_transport_sender(sender.clone());
        }
        let previous = self.engine.take();
        self.engine
            .set(engine)
            .expect("navigation engine slot must be empty after take");
        previous
    }

    fn take(&mut self) -> Option<NavigationEngine> {
        self.engine.take()
    }

    fn ensure(&self) -> &NavigationEngine {
        self.engine.get_or_init(|| {
            let engine = NavigationEngine::new_with_runtime_config(self.runtime_config.clone());
            if let Some(sender) = self.renderer_publication_sender.as_ref() {
                engine.set_renderer_output_transport_sender(sender.clone());
            }
            engine
        })
    }

    fn ensure_mut(&mut self) -> &mut NavigationEngine {
        self.ensure();
        self.engine
            .get_mut()
            .expect("navigation engine was just materialized")
    }
}

/// Persistent per-connection state.
pub struct CdpConnection {
    // Browser/session routing state.
    pub browser_context: Option<BrowserContext>,
    pub inactive_browser_contexts: Vec<BrowserContext>,
    browser_session_ids: HashSet<String>,
    /// When true, every newly-created target is immediately auto-attached.
    pub auto_attach: bool,
    /// Root/browser owner Target discovery mirror for diagnostics and
    /// cross-crate schedulers. Event routing uses `target_handlers`.
    pub(crate) target_discovery_enabled: bool,
    /// Whether URL/title changes should be surfaced through
    /// Target.targetInfoChanged for the root/browser owner. Event routing uses
    /// `target_handlers`.
    pub(crate) target_info_change_events_enabled: bool,
    pub(crate) target_discovery_filter: Option<Vec<DevToolsTargetFilterEntry>>,
    /// Chromium/Playwright auto-attach can ask new targets to wait until
    /// Runtime.runIfWaitingForDebugger before their initial document proceeds.
    pub auto_attach_wait_for_debugger_on_start: bool,
    auto_attach_owner_sessions: HashMap<Option<String>, AutoAttachOwnerPolicy>,
    target_control: TargetControlPlane,
    default_target_lifecycle: DefaultTargetLifecycle,
    service_worker_auto_attach_related_owners: Vec<ServiceWorkerAutoAttachRelatedOwner>,
    service_worker_pause_on_start_owner_sessions: HashSet<Option<String>>,
    dedicated_worker_pause_on_start_owner_sessions: HashSet<Option<String>>,
    install_default_target_on_auto_attach: bool,
    next_bc_id: u32,
    next_target_id: u32,
    shared_target_id_allocator: Option<Arc<AtomicU64>>,
    next_tab_target_id: u32,
    shared_tab_target_id_allocator: Option<Arc<AtomicU64>>,
    next_session_id: u32,
    next_page_domain_subscription_generation: u64,
    next_internal_runtime_command_id: u64,
    network_request_id_allocator: ConnectionNetworkRequestIdAllocator,
    none_session_owner_route_override: Option<CdpSessionRoute>,
    pending_runtime_await_jobs: HashMap<PendingRendererCommandKey, RuntimeAwaitJob>,
    claimed_pending_inspector_await_owners:
        HashMap<PendingRendererCommandKey, ClaimedPendingInspectorAwaitOwner>,

    // Browser profile, permissions, download and global IO state.
    pub window_bounds: BrowserWindowBounds,
    pub download_behavior: BrowserDownloadBehavior,
    pub permission_overrides: Vec<PermissionOverride>,
    next_global_io_stream_id: u64,
    base_browser_identity: moli_browser_profile::BrowserIdentityProfile,
    global_extra_headers: Vec<(String, String)>,
    global_browser_identity_override: Option<moli_browser_profile::BrowserIdentityProfile>,
    global_network_conditions: Option<EmulatedNetworkConditions>,
    global_geolocation_override: Option<EmulatedGeolocationOverrideState>,
    global_cache_disabled: bool,
    pub(crate) network_data_collectors: crate::domains::network::NetworkDataCollectorStore,
    base_http_proxy: Option<String>,
    base_http_no_proxy: Option<String>,
    base_tls_verify_host: bool,
    initial_storage_partition: CdpInitialStoragePartitionOwner,
    pub(crate) global_io_streams: HashMap<String, IoStreamState>,
    pub(crate) tracing_state: crate::domains::tracing::TracingState,
    download_registry: SharedDownloadRegistry,

    // Transport/scheduler integration hooks. These are channels out of the
    // renderer/browser owner into the outer CDP scheduler; they should not grow
    // into protocol routing state.
    scheduler_hooks: CdpSchedulerHooks,
    target_host_lifecycle_observer: Option<CdpTargetHostLifecycleObserver>,

    // Scheduler-visible queues that are still stored on the connection while
    // source-specific queue ownership is being migrated outward.
    scheduler_state: CdpConnectionSchedulerState,

    // Renderer/page owner state. This must remain single-owner and current
    // thread affine while V8/DOM/Page state still lives inside NavigationEngine.
    engine: NavigationEngineOwnerSlot,
    // Background or inactive navigations run on fresh NavigationEngine
    // instances so they do not block the active page owner. A loaded Page only
    // keeps a weak renderer handle; the engine is the strong owner of that
    // renderer runtime. Keep engines keyed by browser-context/target while the
    // page is parked in the background or in an inactive browser context, and
    // swap them back into `engine` when that owner becomes active.
    retained_background_navigation_engines: HashMap<(String, String), NavigationEngine>,
}

impl Default for CdpConnection {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for CdpConnection {
    fn drop(&mut self) {
        // Separate per-context roots from target/page state so teardown can
        // follow the observable producer order even though NavigationEngines
        // and detached local tasks retain only weak owner access.
        let mut contexts = Vec::new();
        if let Some(context) = self.browser_context.take() {
            contexts.push(context);
        }
        contexts.append(&mut self.inactive_browser_contexts);
        let roots = contexts
            .iter_mut()
            .filter_map(BrowserContext::take_renderer_runtime_owner_for_teardown)
            .collect::<Vec<_>>();

        let mut roots = roots;
        for root in &mut roots {
            root.terminate_renderer_producers_for_owner_shutdown();
        }

        // Dropping contexts releases active/background target and Page state.
        drop(contexts);

        // RenderRuntimeOwner joins happen when the last JsRuntime-backed
        // NavigationEngine handle is released. Do that explicitly here rather
        // than relying on field drop after this method returns.
        self.retained_background_navigation_engines.clear();
        drop(self.engine.take());

        // Only after active and retained renderer owners are gone may the
        // context roots close fetch admission and join semantic/curl owners.
        for root in &mut roots {
            root.shutdown_network_and_join();
        }
        drop(roots);
    }
}

impl CdpConnection {
    pub fn new() -> Self {
        Self::new_with_initial_storage_partition(CdpInitialStoragePartition::memory())
    }

    pub(crate) fn layout_policy(&self) -> LayoutPolicy {
        self.engine.layout_policy()
    }

    pub fn has_pending_javascript_dialog(&self) -> bool {
        self.browser_context
            .iter()
            .chain(self.inactive_browser_contexts.iter())
            .any(BrowserContext::has_pending_javascript_dialog)
    }

    pub fn set_automation_javascript_dialog_handler_enabled(&mut self, enabled: bool) -> bool {
        let Some(browser_context) = self.browser_context.as_ref() else {
            return false;
        };
        browser_context
            .renderer_runtime()
            .set_javascript_dialog_handler_enabled(enabled);
        true
    }

    pub fn new_with_initial_cookies(initial_cookies: Vec<StoredCookie>) -> Self {
        Self::new_with_initial_storage_partition(CdpInitialStoragePartition::with_cookies(
            initial_cookies,
        ))
    }

    pub fn new_with_fetch_config(fetch_config: FetchConfig) -> Self {
        Self::new_with_initial_storage_partition_and_fetch_config(
            CdpInitialStoragePartition::memory(),
            fetch_config,
        )
    }

    pub fn enable_webdriver_bidi_download_events(&mut self) -> bool {
        self.download_behavior.enable_webdriver_bidi_events()
    }

    pub fn disable_webdriver_bidi_download_events(&mut self) -> bool {
        self.download_behavior.disable_webdriver_bidi_events()
    }

    pub fn new_with_initial_storage_partition(
        initial_storage_partition: CdpInitialStoragePartition,
    ) -> Self {
        Self::new_with_initial_storage_partition_and_fetch_config(
            initial_storage_partition,
            FetchConfig::default(),
        )
    }

    pub fn new_with_initial_storage_partition_and_fetch_config(
        initial_storage_partition: CdpInitialStoragePartition,
        fetch_config: FetchConfig,
    ) -> Self {
        Self::new_with_initial_storage_partition_fetch_config_and_resource_loading(
            initial_storage_partition,
            fetch_config,
            OptionalResourceFetchMask::NONE,
            true,
        )
    }

    pub fn new_with_initial_storage_partition_fetch_config_and_image_fetch_enabled(
        initial_storage_partition: CdpInitialStoragePartition,
        fetch_config: FetchConfig,
        image_fetch_enabled: bool,
    ) -> Self {
        let optional_resource_fetch_mask = if image_fetch_enabled {
            OptionalResourceFetchMask::IMAGE
        } else {
            OptionalResourceFetchMask::NONE
        };
        Self::new_with_initial_storage_partition_fetch_config_and_resource_loading(
            initial_storage_partition,
            fetch_config,
            optional_resource_fetch_mask,
            true,
        )
    }

    pub fn new_with_initial_storage_partition_fetch_config_and_resource_loading(
        initial_storage_partition: CdpInitialStoragePartition,
        fetch_config: FetchConfig,
        optional_resource_fetch_mask: OptionalResourceFetchMask,
        subframe_loading_enabled: bool,
    ) -> Self {
        Self::new_with_initial_storage_partition_and_runtime_config(
            initial_storage_partition,
            NavigationRuntimeConfig::new(
                fetch_config,
                optional_resource_fetch_mask,
                subframe_loading_enabled,
                LayoutPolicy::default(),
            ),
        )
    }

    pub fn new_with_initial_storage_partition_and_runtime_config(
        initial_storage_partition: CdpInitialStoragePartition,
        navigation_runtime_config: NavigationRuntimeConfig,
    ) -> Self {
        Self::new_with_initial_storage_partition_owner_and_runtime_config(
            CdpInitialStoragePartitionOwner::from_initial_storage_partition(
                initial_storage_partition,
            ),
            navigation_runtime_config,
        )
    }

    /// Creates protocol state without starting the renderer runtime.
    ///
    /// The runtime is materialized on first engine-backed operation. This is
    /// intended for browser-level CDP owners that publish a target before any
    /// frontend attaches to its page.
    pub fn new_with_deferred_navigation_runtime(
        initial_storage_partition: CdpInitialStoragePartition,
        navigation_runtime_config: NavigationRuntimeConfig,
    ) -> Self {
        let initial_storage_partition =
            CdpInitialStoragePartitionOwner::from_initial_storage_partition(
                initial_storage_partition,
            );
        Self::new_with_initial_storage_partition_owner_and_engine(
            initial_storage_partition,
            NavigationEngineOwnerSlot::deferred(navigation_runtime_config),
        )
    }

    fn new_with_initial_storage_partition_owner_and_runtime_config(
        initial_storage_partition: CdpInitialStoragePartitionOwner,
        navigation_runtime_config: NavigationRuntimeConfig,
    ) -> Self {
        Self::new_with_initial_storage_partition_owner_and_engine(
            initial_storage_partition,
            NavigationEngineOwnerSlot::materialized(NavigationEngine::new_with_runtime_config(
                navigation_runtime_config,
            )),
        )
    }

    fn new_with_initial_storage_partition_owner_and_engine(
        initial_storage_partition: CdpInitialStoragePartitionOwner,
        engine: NavigationEngineOwnerSlot,
    ) -> Self {
        let fetch_config = engine.fetch_config();
        let base_browser_identity = fetch_config.browser_identity().clone();
        let base_http_proxy = fetch_config.http_proxy().map(str::to_owned);
        let base_http_no_proxy = fetch_config.http_no_proxy().map(str::to_owned);
        let base_tls_verify_host = fetch_config.tls_verify_host();
        Self {
            browser_context: None,
            inactive_browser_contexts: Vec::new(),
            browser_session_ids: HashSet::new(),
            auto_attach: false,
            target_discovery_enabled: false,
            target_info_change_events_enabled: false,
            target_discovery_filter: None,
            auto_attach_wait_for_debugger_on_start: false,
            auto_attach_owner_sessions: HashMap::new(),
            target_control: TargetControlPlane::default(),
            default_target_lifecycle: DefaultTargetLifecycle::default(),
            service_worker_auto_attach_related_owners: Vec::new(),
            service_worker_pause_on_start_owner_sessions: HashSet::new(),
            dedicated_worker_pause_on_start_owner_sessions: HashSet::new(),
            install_default_target_on_auto_attach: false,
            window_bounds: BrowserWindowBounds::default(),
            download_behavior: BrowserDownloadBehavior::default(),
            permission_overrides: Vec::new(),
            next_bc_id: 0,
            next_global_io_stream_id: 0,
            next_target_id: 0,
            shared_target_id_allocator: None,
            next_tab_target_id: 0,
            shared_tab_target_id_allocator: None,
            next_session_id: 0,
            next_page_domain_subscription_generation: 0,
            next_internal_runtime_command_id: 902_000_000,
            network_request_id_allocator: ConnectionNetworkRequestIdAllocator::default(),
            pending_runtime_await_jobs: HashMap::new(),
            claimed_pending_inspector_await_owners: HashMap::new(),
            base_browser_identity,
            global_extra_headers: Vec::new(),
            global_browser_identity_override: None,
            global_network_conditions: None,
            global_geolocation_override: None,
            global_cache_disabled: false,
            network_data_collectors: crate::domains::network::NetworkDataCollectorStore::default(),
            base_http_proxy,
            base_http_no_proxy,
            base_tls_verify_host,
            initial_storage_partition,
            global_io_streams: HashMap::new(),
            tracing_state: crate::domains::tracing::TracingState::default(),
            download_registry: SharedDownloadRegistry::default(),
            scheduler_hooks: CdpSchedulerHooks::default(),
            target_host_lifecycle_observer: None,
            scheduler_state: CdpConnectionSchedulerState::default(),
            none_session_owner_route_override: None,
            engine,
            retained_background_navigation_engines: HashMap::new(),
        }
    }

    pub fn set_background_event_sender(&mut self, sender: BackgroundEventSender) {
        self.scheduler_hooks.set_background_event_sender(sender);
    }

    pub fn set_shared_target_id_allocator(&mut self, allocator: Arc<AtomicU64>) {
        self.shared_target_id_allocator = Some(allocator);
    }

    pub fn set_shared_tab_target_id_allocator(&mut self, allocator: Arc<AtomicU64>) {
        self.shared_tab_target_id_allocator = Some(allocator);
    }

    pub fn set_target_host_lifecycle_observer(&mut self, observer: CdpTargetHostLifecycleObserver) {
        self.target_host_lifecycle_observer = Some(observer);
    }

    pub fn set_runtime_inspector_response_ready_sender(
        &mut self,
        sender: RuntimeInspectorResponseReadySender,
    ) {
        self.scheduler_hooks
            .set_runtime_inspector_response_ready_sender(sender);
    }

    pub fn set_background_navigation_completion_sender(
        &mut self,
        sender: tokio::sync::mpsc::UnboundedSender<
            crate::domains::page::BackgroundNavigationCompletion,
        >,
    ) {
        self.scheduler_hooks
            .set_background_navigation_completion_sender(sender);
    }

    pub fn set_renderer_publication_sender(
        &mut self,
        sender: moli_core::RendererOutputTransportSender,
    ) {
        self.scheduler_hooks
            .set_renderer_publication_sender(sender.clone());
        self.engine
            .set_renderer_output_transport_sender(sender.clone());
        for engine in self.retained_background_navigation_engines.values() {
            engine.set_renderer_output_transport_sender(sender.clone());
        }
    }

    pub(crate) fn background_event_sender(&self) -> Option<BackgroundEventSender> {
        self.scheduler_hooks.background_event_sender()
    }

    pub(crate) fn runtime_inspector_response_ready_sender(
        &self,
    ) -> Option<RuntimeInspectorResponseReadySender> {
        self.scheduler_hooks
            .runtime_inspector_response_ready_sender()
    }

    pub(crate) fn document_navigation_cancellation_handle(
        &self,
        token: &DocumentNavigationToken,
    ) -> Option<moli_fetch::FetchCancelHandle> {
        let browser_context_id = self.browser_context_id_for_target(&token.target_id)?;
        self.browser_context_by_id(browser_context_id)?
            .document_navigation_cancellation_handle(token)
    }

    pub(crate) fn arm_background_navigation_completion(
        &mut self,
        token: &DocumentNavigationToken,
        additional_cancellation: Option<moli_fetch::FetchCancelHandle>,
    ) -> bool {
        let browser_context_id = self
            .browser_context_id_for_target(&token.target_id)
            .map(str::to_owned);
        let Some(browser_context_id) = browser_context_id else {
            if let Some(cancellation) = additional_cancellation {
                cancellation.cancel();
            }
            return false;
        };
        self.browser_context_by_id_mut(&browser_context_id)
            .is_some_and(|browser_context| {
                browser_context.arm_background_navigation_completion(token, additional_cancellation)
            })
    }

    pub(crate) fn settle_background_navigation_completion(
        &mut self,
        token: &DocumentNavigationToken,
    ) -> bool {
        let browser_context_id = self
            .browser_context_id_for_target(&token.target_id)
            .map(str::to_owned);
        let Some(browser_context_id) = browser_context_id else {
            return false;
        };
        self.browser_context_by_id_mut(&browser_context_id)
            .is_some_and(|browser_context| {
                browser_context.settle_background_navigation_completion(token)
            })
    }

    pub fn has_inflight_background_navigation(&self) -> bool {
        self.browser_contexts()
            .any(BrowserContext::has_inflight_background_navigation)
    }

    pub fn has_inflight_background_navigation_for_target(&self, target_id: &str) -> bool {
        let Some(browser_context_id) = self.browser_context_id_for_target(target_id) else {
            return false;
        };
        self.browser_context_by_id(browser_context_id)
            .is_some_and(|browser_context| {
                browser_context.has_inflight_background_navigation_for_target(target_id)
            })
    }

    fn target_id_for_session_owner(&self, session_id: Option<&str>) -> Option<String> {
        let (browser_context_id, target_id) = self.target_owner_identity_for_session(session_id)?;
        target_id.or_else(|| {
            self.browser_context_by_id(&browser_context_id)
                .and_then(BrowserContext::active_target_id)
                .map(str::to_owned)
        })
    }

    pub(crate) fn session_owner_target_has_waiting_for_debugger_session(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        self.target_id_for_session_owner(session_id)
            .is_some_and(|target_id| self.target_has_waiting_for_debugger_session(&target_id))
    }

    pub fn background_navigation_target_id_for_event(
        &self,
        event: &BackgroundProtocolEvent,
    ) -> Option<String> {
        event
            .navigation_gate_target_id()
            .filter(|target_id| self.browser_context_id_for_target(target_id).is_some())
            .map(str::to_owned)
            .or_else(|| self.target_id_for_session_owner(event.protocol_session_id()))
    }

    pub fn has_inflight_background_navigation_for_devtools_context(
        &self,
        context: &crate::devtools_runtime::DevToolsCommandContext,
    ) -> bool {
        context
            .target_id
            .as_ref()
            .map(crate::devtools_runtime::DevToolsTargetId::as_str)
            .map(str::to_owned)
            .or_else(|| {
                self.target_id_for_session_owner(
                    context
                        .session_id
                        .as_ref()
                        .map(crate::devtools_runtime::DevToolsSessionId::as_str),
                )
            })
            .is_some_and(|target_id| self.has_inflight_background_navigation_for_target(&target_id))
    }

    pub fn has_pending_document_navigation_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        let Some((browser_context_id, target_id)) =
            self.target_owner_identity_for_session(session_id)
        else {
            return false;
        };
        self.browser_context_by_id(&browser_context_id)
            .is_some_and(|browser_context| {
                browser_context.has_pending_document_navigation_for_target(target_id.as_deref())
            })
    }

    fn document_navigation_state_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> DevToolsDocumentNavigationState {
        if self.target_owner_identity_for_session(session_id).is_none() {
            return DevToolsDocumentNavigationState::Unavailable;
        }
        if self.has_pending_document_navigation_for_session_owner(session_id) {
            return DevToolsDocumentNavigationState::PendingNavigation;
        }
        // The initial empty Document has a real loader identity before any
        // cross-document navigation token exists.  Use the same committed
        // frame-tree authority as Page/Runtime routing instead of consulting
        // only `pending_document_navigation` / `committed_document_navigation`.
        // Otherwise a materialized `about:blank` is misclassified as
        // `AwaitingCommit`, and ChromeDriver-style pre-command navigation
        // waits can never complete.
        match self.target_session_owner_frame_tree_loader_id(session_id) {
            Some(loader_id) => DevToolsDocumentNavigationState::Committed { loader_id },
            None => DevToolsDocumentNavigationState::AwaitingCommit,
        }
    }

    /// Resolves Document readiness through the exact target captured by a
    /// protocol-neutral command context. A target-id context intentionally
    /// uses the owner-route override rather than whichever target is currently
    /// active in the browser context.
    pub fn devtools_context_document_navigation_state(
        &mut self,
        context: &DevToolsCommandContext,
    ) -> DevToolsDocumentNavigationState {
        if let Some(target_id) = context.target_id.as_ref() {
            let Some(route) = self.target_session_route_for_target_id(target_id.as_str()) else {
                return DevToolsDocumentNavigationState::Unavailable;
            };
            let mut route_scope = self.scoped_none_session_owner_route_override(route);
            return route_scope
                .conn_mut()
                .document_navigation_state_for_session_owner(None);
        }
        self.document_navigation_state_for_session_owner(
            context
                .session_id
                .as_ref()
                .map(|session_id| session_id.as_str()),
        )
    }

    pub fn renderer_document_navigation_is_suspended_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        self.runtime_session_owner_slot(session_id)
            .is_ok_and(TargetRuntimeSlot::renderer_document_navigation_is_suspended)
    }

    pub(crate) fn accepts_pending_document_navigation_for_session_owner(
        &self,
        session_id: Option<&str>,
        token: &DocumentNavigationToken,
    ) -> bool {
        let Some((browser_context_id, target_id)) =
            self.target_owner_identity_for_session(session_id)
        else {
            return false;
        };
        if target_id.as_deref() != Some(token.target_id.as_str()) {
            return false;
        }
        self.browser_context_by_id(&browser_context_id)
            .is_some_and(|browser_context| {
                browser_context.accepts_pending_document_navigation_event(token)
            })
    }

    pub(crate) fn ensure_document_accessible_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Result<(), String> {
        if self.has_pending_document_navigation_for_session_owner(session_id) {
            return Err("Navigation is changing the document".to_owned());
        }
        Ok(())
    }

    pub(crate) fn current_document_loader_id_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Option<String> {
        self.runtime_session_owner_slot(session_id)
            .ok()
            .and_then(|slot| slot.current_document_loader_id().map(str::to_owned))
    }

    #[cfg(test)]
    pub(crate) fn ingest_renderer_network_output_item_and_prepare_live_delivery_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        source_document: moli_core::RendererDocumentLifecycleIdentity,
        item: &moli_core::page::ScriptNetworkOutputItem,
    ) -> Option<crate::domains::network::TargetNetworkBacklogPreparedDelivery> {
        self.ingest_renderer_page_network_output_item_and_prepare_live_delivery_for_session_owner(
            session_id,
            None,
            source_document,
            item,
        )
    }

    pub(crate) fn ingest_renderer_page_network_output_item_and_prepare_live_delivery_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        source_renderer_page: Option<RendererPageResidenceIdentity>,
        source_document: moli_core::RendererDocumentLifecycleIdentity,
        item: &moli_core::page::ScriptNetworkOutputItem,
    ) -> Option<crate::domains::network::TargetNetworkBacklogPreparedDelivery> {
        let primary_session_id = self.runtime_session_owner_primary_session_id(session_id);
        let mut request_id_allocator = std::mem::take(&mut self.network_request_id_allocator);
        let delivery = self
            .runtime_session_owner_slot_mut(session_id)
            .ok()
            .and_then(|slot| {
                slot.ingest_renderer_network_output_item_and_prepare_live_delivery(
                    source_renderer_page,
                    source_document,
                    item,
                    session_id,
                    primary_session_id.as_deref(),
                    None,
                    &mut request_id_allocator,
                )
            });
        self.network_request_id_allocator = request_id_allocator;
        delivery
    }

    pub(crate) fn loaded_page_mut_for_protocol_access(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<&mut Page, String> {
        self.ensure_document_accessible_for_session_owner(session_id)?;
        self.loaded_page_mut_for_interruptible_protocol_access(session_id)
    }

    /// Returns the exact Page that remains attached while a cross-Document
    /// navigation is suspended.
    ///
    /// Only commands classified as renderer-interruptible at the parsed CDP
    /// boundary may use this access path. Ordinary protocol commands must use
    /// [`Self::loaded_page_mut_for_protocol_access`] so they wait for the
    /// replacement attachment instead of entering the old renderer.
    pub(crate) fn loaded_page_mut_for_interruptible_protocol_access(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<&mut Page, String> {
        self.runtime_session_owner_slot_mut(session_id)?
            .loaded_page_mut()
            .ok_or_else(|| "NoDocumentLoaded".to_owned())
    }

    pub(crate) fn start_document_navigation_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        loader_id: String,
    ) -> Option<DocumentNavigationToken> {
        let (browser_context_id, target_id) = self.target_owner_identity_for_session(session_id)?;
        let target_id = target_id?;
        self.browser_context_by_id_mut(&browser_context_id)?
            .start_document_navigation_for_target(&target_id, loader_id)
    }

    pub(crate) fn commit_document_navigation_for_session_owner_if_matches(
        &mut self,
        session_id: Option<&str>,
        token: &DocumentNavigationToken,
    ) {
        let Some((browser_context_id, target_id)) =
            self.target_owner_identity_for_session(session_id)
        else {
            return;
        };
        if target_id.as_deref() != Some(token.target_id.as_str()) {
            return;
        }
        if let Some(browser_context) = self.browser_context_by_id_mut(&browser_context_id) {
            browser_context.commit_document_navigation_if_matches(token);
        }
    }

    pub(crate) fn bind_renderer_document_lifecycle_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        artifacts: moli_core::page::RendererPageCreationArtifacts,
        navigation: Option<DocumentNavigationToken>,
        frame_id: String,
        loader_id: String,
    ) -> (
        Option<CommittedRendererDocumentBinding>,
        Vec<moli_core::page::RendererDocumentLifecycleEvent>,
    ) {
        let (binding, events, document_scope_changed) = {
            let Ok(slot) = self.runtime_session_owner_slot_mut(session_id) else {
                return (None, Vec::new());
            };
            let previous_document_scope = slot
                .page_slot()
                .renderer_document_lifecycle_binding()
                .map(CommittedRendererDocumentBinding::renderer_document_identity);
            let events = slot
                .page_slot_mut()
                .bind_renderer_document_lifecycle(artifacts, navigation, frame_id, loader_id);
            let binding = slot
                .page_slot()
                .renderer_document_lifecycle_binding()
                .cloned();
            let current_document_scope = binding
                .as_ref()
                .map(CommittedRendererDocumentBinding::renderer_document_identity);
            (
                binding,
                events,
                current_document_scope != previous_document_scope,
            )
        };
        if document_scope_changed {
            self.retire_javascript_dialogs_for_session_owner(session_id);
        }
        (binding, events)
    }

    pub(crate) fn ingest_renderer_document_lifecycle_events_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        events: Vec<moli_core::page::RendererDocumentLifecycleEvent>,
    ) -> (
        Option<CommittedRendererDocumentBinding>,
        Vec<moli_core::page::RendererDocumentLifecycleEvent>,
    ) {
        let (binding, events, document_scope_changed, document_input_stream_opened) = {
            let Ok(slot) = self.runtime_session_owner_slot_mut(session_id) else {
                return (None, Vec::new());
            };
            let previous_document_scope = slot
                .page_slot()
                .renderer_document_lifecycle_binding()
                .map(CommittedRendererDocumentBinding::renderer_document_identity);
            let events = slot
                .page_slot_mut()
                .ingest_renderer_document_lifecycle_events(events);
            let binding = slot
                .page_slot()
                .renderer_document_lifecycle_binding()
                .cloned();
            let current_document_scope = binding
                .as_ref()
                .map(CommittedRendererDocumentBinding::renderer_document_identity);
            let document_input_stream_opened = binding.as_ref().is_some_and(|binding| {
                binding.document_open_replacement_epoch == Some(binding.renderer_epoch)
            });
            (
                binding,
                events,
                current_document_scope != previous_document_scope,
                document_input_stream_opened,
            )
        };
        if document_scope_changed {
            self.retire_javascript_dialogs_for_session_owner(session_id);
        }
        if document_input_stream_opened {
            // The concrete lifecycle record is the authoritative notification
            // that `document.open()` replaced the initial empty Document. Do
            // not defer this state transition to a later diagnostics snapshot:
            // that would make a later owner turn rediscover and settle output
            // produced by this renderer turn.
            self.with_target_owner_state_for_session_mut(session_id, |owner_state| {
                owner_state.mark_initial_empty_document_exited();
            });
        }
        (binding, events)
    }

    fn retire_javascript_dialogs_for_session_owner(&mut self, session_id: Option<&str>) {
        let event_session_ids = self.page_event_session_ids_for_session_owner(session_id);
        if let Ok(slot) = self.runtime_session_owner_slot_mut(session_id) {
            slot.retire_javascript_dialog_scope();
        }
        for event_session_id in event_session_ids {
            let _ = self.with_target_devtools_session_state_for_session_mut(
                event_session_id.as_deref(),
                |state| state.page_session_state.javascript_dialog_state.clear(),
            );
        }
    }

    pub(crate) fn begin_renderer_document_load_visibility_barrier_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        loader_id: &str,
    ) -> bool {
        self.runtime_session_owner_slot_mut(session_id)
            .is_ok_and(|slot| {
                slot.page_slot_mut()
                    .begin_renderer_document_load_visibility_barrier(loader_id)
            })
    }

    pub(crate) fn release_renderer_document_load_visibility_barrier_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        loader_id: &str,
    ) -> Option<Vec<moli_core::page::RendererDocumentLifecycleEvent>> {
        self.runtime_session_owner_slot_mut(session_id)
            .ok()?
            .page_slot_mut()
            .release_renderer_document_load_visibility_barrier(loader_id)
    }

    pub(crate) fn cancel_renderer_document_load_visibility_barrier_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        loader_id: &str,
    ) -> bool {
        self.runtime_session_owner_slot_mut(session_id)
            .is_ok_and(|slot| {
                slot.page_slot_mut()
                    .cancel_renderer_document_load_visibility_barrier(loader_id)
            })
    }

    #[cfg(test)]
    pub(crate) fn renderer_document_lifecycle_authoritative_state_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Option<(
        CommittedRendererDocumentBinding,
        moli_core::page::RendererDocumentLifecycleSnapshot,
    )> {
        let page_slot = self
            .runtime_session_owner_slot(session_id)
            .ok()?
            .page_slot();
        Some((
            page_slot.renderer_document_lifecycle_binding()?.clone(),
            page_slot.renderer_document_lifecycle_authoritative_snapshot()?,
        ))
    }

    pub(crate) fn register_exact_renderer_document_lifecycle_observer_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        expected_binding: Option<&CommittedRendererDocumentBinding>,
        milestone: moli_core::page::RendererDocumentLifecycleMilestone,
    ) -> RendererDocumentLifecycleObserver {
        let Some(expected_binding) = expected_binding else {
            return RendererDocumentLifecycleObserver::resolved(
                RendererDocumentLifecycleObservation::Unavailable,
            );
        };
        let Ok(slot) = self.runtime_session_owner_slot_mut(session_id) else {
            return RendererDocumentLifecycleObserver::resolved(
                RendererDocumentLifecycleObservation::Unavailable,
            );
        };
        slot.page_slot_mut()
            .register_exact_renderer_document_lifecycle_observer(expected_binding, milestone)
    }

    pub(crate) fn renderer_document_lifecycle_visible_state_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Option<(
        CommittedRendererDocumentBinding,
        moli_core::page::RendererDocumentLifecycleSnapshot,
    )> {
        let page_slot = self
            .runtime_session_owner_slot(session_id)
            .ok()?
            .page_slot();
        Some((
            page_slot.renderer_document_lifecycle_binding()?.clone(),
            page_slot.renderer_document_lifecycle_visible_snapshot()?,
        ))
    }

    pub(crate) fn arm_root_post_load_observation_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        loader_id: &str,
    ) -> bool {
        self.runtime_session_owner_slot_mut(session_id)
            .is_ok_and(|slot| {
                slot.page_slot_mut()
                    .arm_root_post_load_observation(loader_id)
            })
    }

    /// Consumes the exact stopped-loading fact owned by an armed root post-load
    /// observation and, when `Page` has subscribers, publishes its frozen
    /// protocol output.
    ///
    /// Having no `Page` subscriber is a normal terminal outcome. The binding
    /// must still be consumed so enabling `Page` later cannot replay a
    /// historical event from an earlier navigation.
    pub(crate) fn settle_root_frame_stopped_loading_observation(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<
        crate::domains::activity::RootFrameStoppedLoadingSettlement,
        crate::domains::activity::RootFrameStoppedLoadingSettlementError,
    > {
        use crate::domains::activity::{
            RootFrameStoppedLoadingSettlement as Settlement,
            RootFrameStoppedLoadingSettlementError as SettlementError,
        };

        let binding = self
            .runtime_session_owner_slot_mut(session_id)
            .ok()
            .and_then(|slot| {
                slot.page_slot_mut()
                    .take_root_frame_stopped_loading_binding()
            });
        let Some(binding) = binding else {
            return Err(SettlementError::MissingArmedObservation);
        };
        if self
            .subscribed_page_event_session_ids_for_session_owner(session_id)
            .is_empty()
        {
            return Ok(Settlement::Unobserved);
        }
        let attachments = self
            .page_event_protocol_attachments_for_session_owner(session_id)
            .ok_or(SettlementError::SubscribedAttachmentUnavailable)?;
        let publish_sequence = self
            .scheduler_state
            .allocate_protocol_work_publish_sequence();
        let output = crate::domains::activity::ProtocolOutputWork::root_frame_stopped_loading(
            attachments,
            binding.frame_id,
            binding.loader_id,
        );
        let work = crate::domains::activity::ProtocolSchedulerWork::protocol_observation(
            publish_sequence,
            output,
        );
        self.scheduler_state
            .push_scheduler_event(CdpSchedulerEvent::ProtocolWorkPublished { work });
        Ok(Settlement::Published)
    }

    pub(crate) fn emit_root_network_idle_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        out: &mut Vec<BackgroundProtocolEvent>,
    ) -> bool {
        if !self
            .runtime_session_owner_slot(session_id)
            .is_ok_and(|slot| slot.renderer_subresources_are_idle())
        {
            return false;
        }
        let binding = self
            .runtime_session_owner_slot_mut(session_id)
            .ok()
            .and_then(|slot| slot.page_slot_mut().take_root_network_idle_binding());
        let Some(binding) = binding else {
            return false;
        };
        let timestamp = monotonic_timestamp_seconds();
        for event_session_id in self.page_event_session_ids_for_session_owner(session_id) {
            let lifecycle_enabled = self
                .target_page_session_state_for_session(event_session_id.as_deref())
                .is_some_and(|state| state.page_lifecycle_events);
            crate::domains::page::emit_navigation_network_idle_background_events(
                out,
                event_session_id.as_deref(),
                lifecycle_enabled,
                &binding.frame_id,
                &binding.loader_id,
                timestamp,
            );
        }
        true
    }

    pub fn devtools_context_routes_to_top_level_target(
        &self,
        context: &DevToolsCommandContext,
    ) -> bool {
        context.target_id.as_ref().is_some_and(|target_id| {
            self.target_session_route_for_target_id(target_id.as_str())
                .is_some()
        })
    }

    pub(crate) fn command_owner_scope_for_devtools_context(
        &self,
        context: &DevToolsCommandContext,
    ) -> Option<CommandOwnerScope> {
        if let Some(target_id) = context.target_id.as_ref() {
            let route = self
                .target_session_route_for_target_id(target_id.as_str())
                .or_else(|| self.target_session_route_for_child_frame_id(target_id.as_str()))?;
            return Some(CommandOwnerScope::from_session_and_owner_route(
                None,
                Some(route),
            ));
        }
        Some(CommandOwnerScope::from_session_and_owner_route(
            context
                .session_id
                .as_ref()
                .map(|session_id| session_id.as_str()),
            None,
        ))
    }

    /// Captures the exact target Page currently addressed by a protocol-neutral
    /// command context. The identity remains stable across Document replacement
    /// within that Page and changes when the Page itself is replaced.
    pub fn page_residence_identity_for_devtools_context(
        &mut self,
        context: &DevToolsCommandContext,
    ) -> Option<DevToolsPageResidenceIdentity> {
        let owner_scope = self.command_owner_scope_for_devtools_context(context)?;
        let session_id = owner_scope.session_id().map(str::to_owned);
        let mut route_scope = owner_scope.enter(self);
        route_scope
            .conn_mut()
            .target_page_residence_identity_for_session(session_id.as_deref())
    }

    pub fn capture_devtools_document_lifecycle_wait_key(
        &mut self,
        context: &DevToolsCommandContext,
        expected_loader_id: &str,
        milestone: moli_core::page::RendererDocumentLifecycleMilestone,
    ) -> Option<DevToolsDocumentLifecycleWaitKey> {
        let registration = if let Some(target_id) = context.target_id.as_ref() {
            let route = self.target_session_route_for_target_id(target_id.as_str())?;
            let mut route_scope = self.scoped_none_session_owner_route_override(route);
            route_scope
                .conn_mut()
                .runtime_session_owner_slot_mut(None)
                .ok()?
                .page_slot_mut()
                .register_renderer_document_lifecycle_waiter(milestone, expected_loader_id)
        } else {
            self.runtime_session_owner_slot_mut(
                context
                    .session_id
                    .as_ref()
                    .map(|session_id| session_id.as_str()),
            )
            .ok()?
            .page_slot_mut()
            .register_renderer_document_lifecycle_waiter(milestone, expected_loader_id)
        };
        let (registration_id, binding) = registration?;
        Some(DevToolsDocumentLifecycleWaitKey {
            registration_id,
            renderer_document: binding.renderer_document,
            renderer_epoch: binding.renderer_epoch,
            milestone,
            frame_id: binding.frame_id,
            loader_id: binding.loader_id,
        })
    }

    pub fn devtools_document_lifecycle_wait_state(
        &mut self,
        context: &DevToolsCommandContext,
        key: &DevToolsDocumentLifecycleWaitKey,
    ) -> DevToolsDocumentLifecycleWaitState {
        if let Some(target_id) = context.target_id.as_ref() {
            let Some(route) = self.target_session_route_for_target_id(target_id.as_str()) else {
                return DevToolsDocumentLifecycleWaitState::Unavailable;
            };
            let mut route_scope = self.scoped_none_session_owner_route_override(route);
            return route_scope
                .conn_mut()
                .runtime_session_owner_slot(None)
                .ok()
                .map_or(DevToolsDocumentLifecycleWaitState::Unavailable, |slot| {
                    devtools_document_lifecycle_wait_state_for_slot(slot, key)
                });
        }
        self.runtime_session_owner_slot(
            context
                .session_id
                .as_ref()
                .map(|session_id| session_id.as_str()),
        )
        .ok()
        .map_or(DevToolsDocumentLifecycleWaitState::Unavailable, |slot| {
            devtools_document_lifecycle_wait_state_for_slot(slot, key)
        })
    }

    pub fn release_devtools_document_lifecycle_wait_key(
        &mut self,
        context: &DevToolsCommandContext,
        key: &DevToolsDocumentLifecycleWaitKey,
    ) -> bool {
        if let Some(target_id) = context.target_id.as_ref() {
            let Some(route) = self.target_session_route_for_target_id(target_id.as_str()) else {
                return false;
            };
            let mut route_scope = self.scoped_none_session_owner_route_override(route);
            return route_scope
                .conn_mut()
                .runtime_session_owner_slot_mut(None)
                .is_ok_and(|slot| {
                    slot.page_slot_mut()
                        .release_renderer_document_lifecycle_waiter(
                            key.registration_id,
                            key.renderer_document,
                            key.renderer_epoch,
                            &key.frame_id,
                            &key.loader_id,
                        )
                });
        }
        self.runtime_session_owner_slot_mut(
            context
                .session_id
                .as_ref()
                .map(|session_id| session_id.as_str()),
        )
        .is_ok_and(|slot| {
            slot.page_slot_mut()
                .release_renderer_document_lifecycle_waiter(
                    key.registration_id,
                    key.renderer_document,
                    key.renderer_epoch,
                    &key.frame_id,
                    &key.loader_id,
                )
        })
    }

    pub(crate) fn accepts_document_body_completion_for_session_owner(
        &self,
        session_id: Option<&str>,
        token: &DocumentNavigationToken,
    ) -> bool {
        let Some((browser_context_id, target_id)) =
            self.target_owner_identity_for_session(session_id)
        else {
            return false;
        };
        if target_id.as_deref() != Some(token.target_id.as_str()) {
            return false;
        }
        self.browser_context_by_id(&browser_context_id)
            .is_some_and(|browser_context| {
                browser_context.accepts_document_body_completion_event(token)
            })
    }

    pub(crate) fn clear_pending_document_navigation_for_session_owner_if_loader_matches(
        &mut self,
        session_id: Option<&str>,
        loader_id: &str,
    ) {
        let Some((browser_context_id, target_id)) =
            self.target_owner_identity_for_session(session_id)
        else {
            return;
        };
        if let Some(browser_context) = self.browser_context_by_id_mut(&browser_context_id) {
            browser_context.clear_pending_document_navigation_for_target_if_loader_matches(
                target_id.as_deref(),
                loader_id,
            );
        }
        self.discard_uncommitted_main_document_resource_for_session_owner(session_id, loader_id);
    }

    pub fn take_scheduler_events(&mut self) -> Vec<CdpSchedulerEvent> {
        self.scheduler_state.take_scheduler_events()
    }

    pub(crate) fn push_scheduler_event(&mut self, event: CdpSchedulerEvent) {
        self.scheduler_state.push_scheduler_event(event);
    }

    pub fn begin_command_response_flush_permit(
        &mut self,
    ) -> (CommandResponseFlushPermit, CommandResponseFlushContext) {
        let (sender, receiver) = tokio::sync::watch::channel(false);
        let deferred_releases: Arc<Mutex<CommandResponseFlushDeferredReleases>> = Arc::default();
        (
            CommandResponseFlushPermit {
                sender,
                deferred_releases: deferred_releases.clone(),
            },
            CommandResponseFlushContext::new(receiver, deferred_releases),
        )
    }

    pub(crate) fn extend_scheduler_events(&mut self, events: Vec<CdpSchedulerEvent>) {
        self.scheduler_state.extend_scheduler_events(events);
    }

    pub(crate) fn record_scheduler_activity_trace(&mut self, event: serde_json::Value) {
        self.scheduler_state.push_activity_trace(event);
    }

    pub(crate) fn scheduler_activity_trace_enabled(&self) -> bool {
        moli_trace::cdp_nav_timing_enabled()
    }

    pub(crate) fn runtime_await_trace_enabled(&self) -> bool {
        moli_trace::cdp_runtime_trace_enabled() || self.scheduler_activity_trace_enabled()
    }

    pub(crate) fn record_runtime_await_trace(
        &mut self,
        event: &'static str,
        command_id: Option<u64>,
        session_id: Option<&str>,
        fields: serde_json::Value,
    ) {
        if !self.runtime_await_trace_enabled() {
            return;
        }
        self.record_scheduler_activity_trace(json!({
            "kind": event,
            "commandId": command_id,
            "sessionId": session_id,
            "fields": fields,
            "pendingRuntimeAwaitJobCount": self.pending_runtime_await_jobs.len(),
        }));
    }

    pub(crate) fn background_navigation_completion_sender_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Option<
        tokio::sync::mpsc::UnboundedSender<crate::domains::page::BackgroundNavigationCompletion>,
    > {
        if !self.can_run_background_navigation_for_session_owner(session_id) {
            return None;
        }
        self.scheduler_hooks
            .background_navigation_completion_sender()
    }

    fn can_run_background_navigation_for_session_owner(&self, session_id: Option<&str>) -> bool {
        if !self
            .scheduler_hooks
            .has_background_navigation_completion_sender()
        {
            return false;
        }
        self.target_owner_identity_for_session(session_id)
            .is_some_and(|(_, target_id)| target_id.is_some())
    }

    fn can_run_background_navigation_for_active_session(&self) -> bool {
        if !self
            .scheduler_hooks
            .has_background_navigation_completion_sender()
            || !self.inactive_browser_contexts.is_empty()
        {
            return false;
        }
        self.browser_context
            .as_ref()
            .is_some_and(|browser_context| browser_context.background_targets.is_empty())
    }

    pub(crate) fn can_defer_initial_document_page_build(&self) -> bool {
        self.can_run_background_navigation_for_active_session()
    }

    pub async fn drain_background_navigation_completion_turn_async(
        &mut self,
        completion: crate::domains::page::BackgroundNavigationCompletion,
    ) -> CdpRendererOwnerTurnOutcome {
        let mut command_context = CommandDispatchContext::default();
        let protocol_events = self
            .drain_background_navigation_completion_events_with_context(
                completion,
                &mut command_context,
            )
            .await;
        command_context
            .protocol_events_mut()
            .extend(protocol_events);
        let (protocol_events, renderer_output_boundary, post_renderer_output_events) =
            command_context.take_renderer_fenced_protocol_events();
        CdpTurnOutcome::new_with_protocol_and_post_response_events(
            protocol_events,
            command_context.take_post_response_events(),
            self.take_scheduler_events(),
        )
        .with_renderer_output_boundary(renderer_output_boundary, post_renderer_output_events)
        .with_renderer_output_predecessor(command_context.take_renderer_output_predecessor())
    }

    async fn drain_background_navigation_completion_events_with_context(
        &mut self,
        completion: crate::domains::page::BackgroundNavigationCompletion,
        command_context: &mut CommandDispatchContext,
    ) -> Vec<BackgroundProtocolEvent> {
        let completion = match completion {
            crate::domains::page::BackgroundNavigationCompletion::Lifecycle(completion) => {
                if !self.settle_background_navigation_completion(completion.navigation_token()) {
                    tracing::debug!(
                        token = ?completion.navigation_token(),
                        "background navigation completion did not match the target-owned request"
                    );
                }
                completion
            }
            crate::domains::page::BackgroundNavigationCompletion::MainDocumentBody(completion) => {
                let previous_none_session_owner_route =
                    completion.navigate_session_id().is_none().then(|| {
                        self.replace_none_session_owner_route_override(
                            completion.none_session_owner_route(),
                        )
                    });
                completion.record_if_current(self);
                if let Some(previous) = previous_none_session_owner_route {
                    self.replace_none_session_owner_route_override(previous);
                }
                return command_context.take_protocol_events();
            }
        };
        let previous_none_session_owner_route =
            completion.navigate_session_id().is_none().then(|| {
                self.replace_none_session_owner_route_override(
                    completion.none_session_owner_route(),
                )
            });
        let timing_started = moli_trace::cdp_nav_timing_enabled().then(std::time::Instant::now);
        if timing_started.is_some() {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url = %completion.requested_url(),
                stage = "background_completion_enqueue_start",
                ready_to_enqueue_ms = completion.ready_elapsed_ms(),
            );
        }
        // Always complete the materialized navigation so the client receives
        // a terminal Page.navigate response (success or abort-error) for the
        // outstanding command id. The completion helper is responsible for the
        // navigate_id-gated abort emission; the only thing we must avoid here
        // is letting the stale background engine displace the live one.
        let is_current = completion.is_current_for_connection(self);
        let completion = completion.materialize_with_engine_retention(self, is_current);
        if let Some(started) = timing_started {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "background_completion_materialized",
                phase_ms = started.elapsed().as_millis(),
            );
        }
        let protocol_events = self
            .drain_materialized_navigation_completion_background_events(completion, command_context)
            .await;
        if let Some(previous) = previous_none_session_owner_route {
            self.replace_none_session_owner_route_override(previous);
        }
        protocol_events
    }

    pub(crate) fn replace_navigation_engine(&mut self, engine: NavigationEngine) {
        let engine = engine;
        self.apply_scheduler_senders_to_navigation_engine(&engine);
        drop(self.engine.replace(engine));
    }

    pub(crate) fn apply_scheduler_senders_to_navigation_engine(&self, engine: &NavigationEngine) {
        if let Some(sender) = self.scheduler_hooks.renderer_publication_sender() {
            engine.set_renderer_output_transport_sender(sender);
        }
    }

    fn retain_background_navigation_engine(
        &mut self,
        browser_context_id: String,
        target_id: String,
        engine: NavigationEngine,
    ) -> Result<(), String> {
        let owner_access = self
            .browser_context_by_id(&browser_context_id)
            .ok_or_else(|| {
                format!(
                    "cannot retain navigation engine for missing BrowserContext `{browser_context_id}`"
                )
            })?
            .renderer_runtime_owner_access();
        if !engine
            .browser_context_runtime()
            .shares_state_with(&owner_access.runtime())
        {
            return Err(format!(
                "navigation engine renderer context does not match BrowserContext `{browser_context_id}`"
            ));
        }
        self.apply_scheduler_senders_to_navigation_engine(&engine);
        let replaced = self
            .retained_background_navigation_engines
            .insert((browser_context_id, target_id), engine);
        // The replaced wrapper may carry the last ambient (non-Document)
        // lease for a retired fetch runtime. Drop it before asking the exact
        // BrowserContext root to reap; doing this at the one retained-engine
        // transition avoids route-specific finish hooks.
        drop(replaced);
        owner_access.reap_retired_resource_runtimes();
        Ok(())
    }

    pub(crate) fn forget_retained_navigation_engine_for_target(&mut self, target_id: &str) {
        self.retained_background_navigation_engines
            .retain(|(_, retained_target_id), _| retained_target_id != target_id);
    }

    pub(crate) fn adopt_loaded_navigation_engine_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        engine: NavigationEngine,
    ) {
        let route = match session_id {
            Some(_) => self.session_route(session_id),
            None => self.none_session_owner_route_override(),
        };
        let Some(route) = route else {
            self.replace_navigation_engine(engine);
            return;
        };
        match route {
            CdpSessionRoute::BackgroundTarget {
                browser_context_id,
                target_id,
            } => self
                .retain_background_navigation_engine(browser_context_id, target_id, engine)
                .expect("background route must retain its exact BrowserContext engine"),
            CdpSessionRoute::ActiveTarget {
                browser_context_id,
                target_id: Some(target_id),
            } if self
                .browser_context
                .as_ref()
                .is_none_or(|browser_context| browser_context.id != browser_context_id) =>
            {
                self.retain_background_navigation_engine(browser_context_id, target_id, engine)
                    .expect(
                        "inactive active-target route must retain its exact BrowserContext engine",
                    )
            }
            CdpSessionRoute::ActiveTarget {
                browser_context_id,
                target_id: None,
            } if self
                .browser_context
                .as_ref()
                .is_none_or(|browser_context| browser_context.id != browser_context_id) =>
            {
                if let Some(target_id) = self
                    .browser_context_by_id(&browser_context_id)
                    .and_then(|browser_context| browser_context.active_target_id())
                    .map(str::to_owned)
                {
                    self.retain_background_navigation_engine(browser_context_id, target_id, engine)
                        .expect("inactive active-target route must retain its exact BrowserContext engine");
                } else {
                    self.replace_navigation_engine(engine);
                }
            }
            CdpSessionRoute::AuxiliaryTarget {
                browser_context_id,
                target_id,
            } if self
                .browser_context_by_id(&browser_context_id)
                .and_then(|browser_context| browser_context.background_target(&target_id))
                .is_some()
                || self
                    .browser_context
                    .as_ref()
                    .is_none_or(|browser_context| browser_context.id != browser_context_id) =>
            {
                self.retain_background_navigation_engine(browser_context_id, target_id, engine)
                    .expect("auxiliary route must retain its exact BrowserContext engine")
            }
            CdpSessionRoute::Browser
            | CdpSessionRoute::TabTarget { .. }
            | CdpSessionRoute::ActiveTarget { .. }
            | CdpSessionRoute::AuxiliaryTarget { .. }
            | CdpSessionRoute::SharedWorkerTarget { .. }
            | CdpSessionRoute::DedicatedWorkerTarget { .. }
            | CdpSessionRoute::ServiceWorkerTarget { .. } => self.replace_navigation_engine(engine),
        }
    }

    pub(crate) fn adopt_loaded_navigation_engine_for_page_owner(
        &mut self,
        owner: &InitialDocumentPageOwner,
        engine: NavigationEngine,
    ) {
        let owner_is_current_active_target =
            self.browser_context
                .as_ref()
                .is_some_and(|browser_context| {
                    browser_context.id == owner.browser_context_id
                        && browser_context.active_target_id() == Some(owner.target_id.as_str())
                });
        if owner_is_current_active_target {
            self.replace_navigation_engine(engine);
            return;
        }
        self.retain_background_navigation_engine(
            owner.browser_context_id.clone(),
            owner.target_id.clone(),
            engine,
        )
        .expect("page owner must retain its exact BrowserContext engine");
    }

    pub(crate) fn enqueue_deferred_main_document_load_completion(
        &mut self,
        admission: crate::domains::activity::DeferredMainDocumentLoadCompletionAdmission,
    ) {
        if !admission.is_still_current_for_scheduler(self) {
            tracing::debug!(
                session_id = admission.session_id(),
                "dropping obsolete deferred main-document load completion before enqueue"
            );
            return;
        }
        let owner_scope = admission.owner_scope().clone();
        let observation_id = self
            .scheduler_state
            .allocate_deferred_main_document_load_observation_id();
        let completion = {
            let mut route_scope = owner_scope.enter(self);
            admission.bind_lifecycle_observer(route_scope.conn_mut(), observation_id)
        };
        let publish_sequence = self
            .scheduler_state
            .allocate_protocol_work_publish_sequence();
        let work = crate::domains::activity::ProtocolSchedulerWork::main_document_load_owner_action(
            publish_sequence,
            completion,
        );
        self.scheduler_state
            .push_scheduler_event(CdpSchedulerEvent::ProtocolWorkPublished { work });
    }

    /// Publishes a top-level navigation already moved into prepared output.
    ///
    /// The prepared value retains its exact Page residence. The route is
    /// captured here, while the output drain is still running under the
    /// producer's owner scope. This boundary deliberately performs no
    /// navigation: the returned scheduler work is the sole execution
    /// authority.
    pub(crate) fn publish_prepared_top_level_location_navigation_owner_action(
        &mut self,
        session_id: Option<&str>,
        page_owner: TargetPageResidenceIdentity,
        navigation: moli_core::page::RendererDocumentSourcedTopLevelLocationNavigation,
    ) {
        let action = TopLevelLocationNavigationOwnerAction::from_prepared(
            self, session_id, page_owner, navigation,
        );
        self.publish_top_level_location_navigation_owner_action(action);
    }

    fn publish_top_level_location_navigation_owner_action(
        &mut self,
        action: TopLevelLocationNavigationOwnerAction,
    ) {
        let publish_sequence = self
            .scheduler_state
            .allocate_protocol_work_publish_sequence();
        let work =
            crate::domains::activity::ProtocolSchedulerWork::top_level_location_navigation_owner_action(
                publish_sequence,
                action,
            );
        self.scheduler_state
            .push_scheduler_event(CdpSchedulerEvent::ProtocolWorkPublished { work });
    }

    pub(crate) fn publish_popup_target_navigation_owner_action(
        &mut self,
        action: PopupTargetNavigationOwnerAction,
    ) {
        let publish_sequence = self
            .scheduler_state
            .allocate_protocol_work_publish_sequence();
        let work =
            crate::domains::activity::ProtocolSchedulerWork::popup_target_navigation_owner_action(
                publish_sequence,
                action,
            );
        self.scheduler_state
            .push_scheduler_event(CdpSchedulerEvent::ProtocolWorkPublished { work });
    }

    pub(crate) fn publish_popup_target_activation_action(
        &mut self,
        action: PopupTargetActivationAction,
    ) {
        let publish_sequence = self
            .scheduler_state
            .allocate_protocol_work_publish_sequence();
        let work = crate::domains::activity::ProtocolSchedulerWork::popup_target_activation_action(
            publish_sequence,
            action,
        );
        self.scheduler_state
            .push_scheduler_event(CdpSchedulerEvent::ProtocolWorkPublished { work });
    }

    pub(crate) fn publish_page_target_termination_owner_action(
        &mut self,
        action: crate::domains::page::PageTargetTerminationOwnerAction,
    ) {
        let publish_sequence = self
            .scheduler_state
            .allocate_protocol_work_publish_sequence();
        let work =
            crate::domains::activity::ProtocolSchedulerWork::page_target_termination_owner_action(
                publish_sequence,
                action,
            );
        self.scheduler_state
            .push_scheduler_event(CdpSchedulerEvent::ProtocolWorkPublished { work });
    }

    pub async fn complete_deferred_main_document_load_completion_for_scheduler(
        &mut self,
        completion: CompletedDeferredMainDocumentLoadCompletion,
    ) -> CdpTurnOutcome {
        let owner_scope = completion.inner.owner_scope().clone();
        let mut output = BackgroundProtocolEventBuffer::default();
        {
            let mut route_scope = owner_scope.enter(self);
            completion
                .inner
                .emit_async(route_scope.conn_mut(), &mut output)
                .await;
        }
        CdpTurnOutcome::new_with_protocol_and_post_response_events(
            output.into_events(),
            Vec::new(),
            self.take_scheduler_events(),
        )
    }

    pub(crate) fn enqueue_navigation_background_event(&mut self, event: NavigationBackgroundEvent) {
        self.scheduler_state.push_navigation_background_event(event);
    }

    pub(crate) fn enqueue_navigation_background_protocol_event(
        &mut self,
        token: DocumentNavigationToken,
        event: BackgroundProtocolEvent,
    ) {
        self.enqueue_navigation_background_event(NavigationBackgroundEvent::background_event(
            token, event,
        ));
    }

    pub(crate) fn send_navigation_background_protocol_event(
        &mut self,
        token: DocumentNavigationToken,
        event: BackgroundProtocolEvent,
    ) {
        self.enqueue_navigation_background_protocol_event(token, event);
        self.flush_navigation_background_events_to_sender();
    }

    fn drain_navigation_background_protocol_events(&mut self) -> Vec<BackgroundProtocolEvent> {
        let events = self.scheduler_state.take_navigation_background_events();
        events
            .into_iter()
            .filter_map(|event| {
                event.into_background_protocol_event_if_current(self.browser_contexts())
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn drain_navigation_background_events(&mut self) -> Vec<serde_json::Value> {
        self.drain_navigation_background_protocol_events()
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect()
    }

    pub(crate) fn flush_navigation_background_events_to_sender(&mut self) {
        let Some(sender) = self.scheduler_hooks.background_event_sender() else {
            return;
        };
        for event in self.drain_navigation_background_protocol_events() {
            let _ = sender.send(event);
        }
    }

    pub(crate) async fn drain_materialized_navigation_completion_background_events(
        &mut self,
        completion: crate::domains::page::MaterializedNavigationCompletion,
        command_context: &mut CommandDispatchContext,
    ) -> Vec<BackgroundProtocolEvent> {
        let command_id = completion.navigate_id();
        let command_session_id = completion.navigate_session_id().map(str::to_owned);
        let mut output = CommandOutputBuffer::default();
        self.drain_materialized_navigation_completion_into_buffer(
            &mut output,
            completion,
            command_context,
        )
        .await;
        let (
            before_renderer_output,
            renderer_output_boundary,
            after_renderer_output,
            post_response_events,
        ) = output
            .into_plan()
            .into_renderer_fenced_background_and_post_response_events(
                command_id,
                command_session_id.as_deref(),
            );
        command_context.append_renderer_fenced_protocol_events(
            before_renderer_output,
            renderer_output_boundary,
            after_renderer_output,
        );
        command_context.extend_post_response_events(post_response_events);
        Vec::new()
    }

    #[cfg(test)]
    pub(crate) async fn drain_materialized_navigation_completion_into(
        &mut self,
        out: &mut Vec<serde_json::Value>,
        completion: crate::domains::page::MaterializedNavigationCompletion,
        command_context: &mut CommandDispatchContext,
    ) {
        let mut events = self
            .drain_materialized_navigation_completion_background_events(completion, command_context)
            .await;
        let (before_renderer_output, renderer_output_boundary, after_renderer_output) =
            command_context.take_renderer_fenced_protocol_events();
        assert!(
            renderer_output_boundary.is_none(),
            "message-only navigation helper cannot flatten a renderer output boundary"
        );
        events.extend(before_renderer_output);
        events.extend(after_renderer_output);
        events.extend(command_context.take_post_response_events());
        out.extend(
            events
                .into_iter()
                .map(BackgroundProtocolEvent::into_protocol_message),
        );
    }

    pub(crate) async fn drain_materialized_navigation_completion_into_buffer(
        &mut self,
        out: &mut CommandOutputBuffer,
        completion: crate::domains::page::MaterializedNavigationCompletion,
        command_context: &mut CommandDispatchContext,
    ) {
        let timing_started = moli_trace::cdp_nav_timing_enabled().then(std::time::Instant::now);
        if timing_started.is_some() {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url = %completion.requested_url(),
                stage = "materialized_completion_drain_start",
            );
        }
        let is_current = completion.is_current_for_connection(self);
        let (token, state, navigation, engine) = completion.into_parts();
        if !is_current {
            crate::domains::page::push_superseded_navigation_result(out, &state);
            return;
        }
        let navigation_session_id = state.navigate_session_id.clone();
        crate::domains::page::complete_materialized_navigation_into_buffer_async(
            self,
            out,
            token.clone(),
            state,
            navigation,
            command_context,
        )
        .await;
        if let Some(engine) = engine {
            self.adopt_loaded_navigation_engine_for_session_owner(
                navigation_session_id.as_deref(),
                engine,
            );
        }
        if let Some(started) = timing_started {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "materialized_completion_drain_end",
                phase_ms = started.elapsed().as_millis(),
            );
        }
    }

    pub(crate) fn none_session_owner_route_override(&self) -> Option<CdpSessionRoute> {
        self.none_session_owner_route_override.clone()
    }

    pub(crate) fn replace_none_session_owner_route_override(
        &mut self,
        route: Option<CdpSessionRoute>,
    ) -> Option<CdpSessionRoute> {
        std::mem::replace(&mut self.none_session_owner_route_override, route)
    }

    pub(crate) fn scoped_none_session_owner_route_override(
        &mut self,
        route: CdpSessionRoute,
    ) -> NoneSessionOwnerRouteOverrideScope<'_> {
        NoneSessionOwnerRouteOverrideScope::enter(self, Some(route))
    }

    pub(crate) fn scoped_optional_none_session_owner_route_override(
        &mut self,
        route: Option<CdpSessionRoute>,
    ) -> NoneSessionOwnerRouteOverrideScope<'_> {
        NoneSessionOwnerRouteOverrideScope::enter(self, route)
    }

    pub(crate) fn response_body_materialize_limit(&self) -> usize {
        self.engine
            .fetch_config()
            .http_max_response_size()
            .unwrap_or(body_spool::DEFAULT_BODY_MATERIALIZE_LIMIT)
    }

    pub(crate) fn moli_memory_diagnostics(&self) -> serde_json::Value {
        let active_browser_context = self
            .browser_context
            .as_ref()
            .map(BrowserContext::moli_memory_diagnostics);
        let inactive_browser_contexts = self
            .inactive_browser_contexts
            .iter()
            .map(BrowserContext::moli_memory_diagnostics)
            .collect::<Vec<_>>();
        let retained_engine_keys = self
            .retained_background_navigation_engines
            .keys()
            .map(|(browser_context_id, target_id)| {
                json!({
                    "browserContextId": browser_context_id,
                    "targetId": target_id,
                })
            })
            .collect::<Vec<_>>();
        let browser_context_count = self.browser_contexts().count();
        let loaded_document_page_count = self
            .browser_contexts()
            .map(BrowserContext::loaded_document_page_count)
            .sum::<usize>();
        let pending_document_page_build_count = self
            .browser_contexts()
            .map(BrowserContext::pending_document_page_build_count)
            .sum::<usize>();
        let mut loaded_document_renderer_owner_ids = HashSet::new();
        let mut document_renderer_owner_ids = HashSet::new();
        for browser_context in self.browser_contexts() {
            loaded_document_renderer_owner_ids
                .extend(browser_context.loaded_document_renderer_owner_ids_for_diagnostics());
            document_renderer_owner_ids
                .extend(browser_context.document_renderer_owner_ids_for_diagnostics());
        }
        let loaded_document_renderer_owner_count = loaded_document_renderer_owner_ids.len();
        let shared_worker_target_count = self
            .browser_contexts()
            .map(|context| context.shared_worker_targets.len())
            .sum::<usize>();
        let service_worker_target_count = self
            .browser_contexts()
            .map(|context| context.service_worker_targets.len())
            .sum::<usize>();
        let page_target_pending_inspector_await_count = self
            .browser_contexts()
            .map(BrowserContext::page_target_pending_inspector_await_count_for_diagnostics)
            .sum::<usize>();
        let page_target_with_pending_inspector_await_count = self
            .browser_contexts()
            .map(BrowserContext::page_target_with_pending_inspector_await_count_for_diagnostics)
            .sum::<usize>();
        let shared_worker_target_pending_inspector_await_count = self
            .browser_contexts()
            .map(BrowserContext::shared_worker_target_pending_inspector_await_count_for_diagnostics)
            .sum::<usize>();
        let shared_worker_target_with_pending_inspector_await_count = self
            .browser_contexts()
            .map(
                BrowserContext::shared_worker_target_with_pending_inspector_await_count_for_diagnostics,
            )
            .sum::<usize>();
        let service_worker_target_pending_inspector_await_count = self
            .browser_contexts()
            .map(
                BrowserContext::service_worker_target_pending_inspector_await_count_for_diagnostics,
            )
            .sum::<usize>();
        let service_worker_target_with_pending_inspector_await_count = self
            .browser_contexts()
            .map(
                BrowserContext::service_worker_target_with_pending_inspector_await_count_for_diagnostics,
            )
            .sum::<usize>();
        let pending_inspector_await_count = page_target_pending_inspector_await_count
            + shared_worker_target_pending_inspector_await_count
            + service_worker_target_pending_inspector_await_count;
        let dedicated_worker_running_worker_isolate_count = self
            .browser_contexts()
            .map(BrowserContext::dedicated_worker_running_worker_isolate_count_for_diagnostics)
            .sum::<usize>();
        let mut shared_worker_matching_entry_count = 0;
        let mut shared_worker_loading_instance_count = 0;
        let mut shared_worker_running_instance_count = 0;
        let mut shared_worker_client_count = 0;
        let mut shared_worker_loading_host_count = 0;
        let mut shared_worker_running_worker_isolate_count = 0;
        let mut shared_worker_pending_service_lane_event_count = 0;
        for shared_worker_diagnostics in self
            .browser_contexts()
            .map(BrowserContext::shared_worker_runtime_diagnostics_for_diagnostics)
        {
            shared_worker_matching_entry_count += shared_worker_diagnostics.matching_entry_count;
            shared_worker_loading_instance_count +=
                shared_worker_diagnostics.loading_instance_count;
            shared_worker_running_instance_count +=
                shared_worker_diagnostics.running_instance_count;
            shared_worker_client_count += shared_worker_diagnostics.client_count;
            shared_worker_loading_host_count += shared_worker_diagnostics.loading_host_count;
            shared_worker_running_worker_isolate_count +=
                shared_worker_diagnostics.running_worker_isolate_count;
            shared_worker_pending_service_lane_event_count +=
                shared_worker_diagnostics.pending_service_lane_event_count;
        }
        let retained_background_navigation_engine_count =
            self.retained_background_navigation_engines.len();
        let active_engine = self.engine.ensure();
        let active_renderer_owner_id = active_engine.renderer_owner_id_for_diagnostics();
        let mut retained_background_navigation_engine_renderer_owner_ids = HashSet::new();
        let mut estimated_renderer_owner_ids = HashSet::new();
        estimated_renderer_owner_ids.insert(active_renderer_owner_id);
        estimated_renderer_owner_ids.extend(document_renderer_owner_ids.iter().copied());
        for engine in self.retained_background_navigation_engines.values() {
            let renderer_owner_id = engine.renderer_owner_id_for_diagnostics();
            if renderer_owner_id != active_renderer_owner_id {
                retained_background_navigation_engine_renderer_owner_ids.insert(renderer_owner_id);
            }
            estimated_renderer_owner_ids.insert(renderer_owner_id);
        }
        let retained_background_navigation_engine_renderer_owner_count =
            retained_background_navigation_engine_renderer_owner_ids.len();
        let estimated_renderer_owner_count = estimated_renderer_owner_ids.len();
        let document_isolate_model = active_engine.document_isolate_model_for_diagnostics();
        let estimated_document_isolate_count =
            loaded_document_page_count + pending_document_page_build_count;
        let document_isolate_accounting =
            active_engine.document_isolate_accounting_for_diagnostics();
        let document_isolate_accounting = json!({
            "scope": "renderer-process",
            "created": document_isolate_accounting.created,
            "destroyed": document_isolate_accounting.destroyed,
            "live": document_isolate_accounting.live,
            "reserved": document_isolate_accounting.reserved,
        });
        let estimated_worker_isolate_count = dedicated_worker_running_worker_isolate_count
            + shared_worker_running_worker_isolate_count;
        let estimated_live_v8_isolate_count =
            estimated_document_isolate_count + estimated_worker_isolate_count;
        let active_navigation_engine_resource_runtime = active_engine
            .resource_request_client()
            .map(|client| client.resource_runtime_diagnostics());
        let active_navigation_engine_resource_runtime_id =
            active_navigation_engine_resource_runtime
                .as_ref()
                .map(|diagnostics| diagnostics.runtime_id);
        let active_navigation_engine_memory_cache =
            active_navigation_engine_resource_runtime.map(|diagnostics| diagnostics.memory_cache);
        json!({
            "connection": {
                "hasActiveBrowserContext": self.browser_context.is_some(),
                "inactiveBrowserContextCount": self.inactive_browser_contexts.len(),
                "browserSessionIdCount": self.browser_session_ids.len(),
                "globalIoStreamCount": self.global_io_streams.len(),
                "tracing": self.tracing_state.diagnostics(),
                "permissionOverrideCount": self.permission_overrides.len(),
                "retainedBackgroundNavigationEngineCount": retained_background_navigation_engine_count,
                "retainedBackgroundNavigationEngineKeys": retained_engine_keys,
                "autoAttach": self.auto_attach,
                "targetDiscoveryEnabled": self.target_discovery_enabled,
                "targetInfoChangeEventsEnabled": self.target_info_change_events_enabled,
                "activeNavigationEngine": {
                    "imageFetchEnabled": active_engine.image_fetch_enabled(),
                    "optionalResourceFetchMask": active_engine.optional_resource_fetch_mask().bits(),
                    "subframeLoadingEnabled": active_engine.subframe_loading_enabled(),
                    "resourceRuntimeId": active_navigation_engine_resource_runtime_id,
                    "networkMemoryCache": active_navigation_engine_memory_cache,
                    "browserContextRuntime": active_engine
                        .browser_context_runtime()
                        .moli_memory_diagnostics(),
                },
            },
            "isolateScope": {
                "documentIsolateModel": document_isolate_model,
                "workerIsolateModel": "per-worker-thread",
                "activeNavigationEngineRendererOwnerCount": 1,
                "retainedBackgroundNavigationEngineRendererOwnerCount": retained_background_navigation_engine_renderer_owner_count,
                "estimatedRendererOwnerCount": estimated_renderer_owner_count,
                "browserContextCount": browser_context_count,
                "loadedDocumentPageCount": loaded_document_page_count,
                "loadedDocumentRendererOwnerCount": loaded_document_renderer_owner_count,
                "pendingDocumentPageBuildCount": pending_document_page_build_count,
                "estimatedDocumentIsolateCount": estimated_document_isolate_count,
                "documentIsolateAccounting": document_isolate_accounting,
                "estimatedWorkerIsolateCount": estimated_worker_isolate_count,
                "estimatedLiveV8IsolateCount": estimated_live_v8_isolate_count,
                "runtimeGetHeapUsageV8HeapScope": "page-vm-document-isolate",
                "runtimeGetHeapUsageV8HeapIsTargetLocal": true,
                "runtimeGetHeapUsageMoliCountersScope": "target-document",
                "runtimeCollectGarbageScope": "page-vm-document-isolate",
                "v8ForegroundTaskWakeScope": "page-vm-document-isolate",
                "v8ForegroundTaskWakeContextGroupIdAvailable": false,
                "v8ForegroundTaskWakeInternalPolicy": "page-runtime-queue-and-owner-page-tick",
                "v8ForegroundTaskWakeExternalPolicy": "page-owner-runtime-wake",
                "pendingInspectorAwaitCount": pending_inspector_await_count,
                "pageTargetPendingInspectorAwaitCount": page_target_pending_inspector_await_count,
                "pageTargetWithPendingInspectorAwaitCount": page_target_with_pending_inspector_await_count,
                "sharedWorkerTargetPendingInspectorAwaitCount": shared_worker_target_pending_inspector_await_count,
                "sharedWorkerTargetWithPendingInspectorAwaitCount": shared_worker_target_with_pending_inspector_await_count,
                "serviceWorkerTargetPendingInspectorAwaitCount": service_worker_target_pending_inspector_await_count,
                "serviceWorkerTargetWithPendingInspectorAwaitCount": service_worker_target_with_pending_inspector_await_count,
                "sharedWorkerTargetCount": shared_worker_target_count,
                "serviceWorkerTargetCount": service_worker_target_count,
                "sharedWorkerMatchingEntryCount": shared_worker_matching_entry_count,
                "sharedWorkerLoadingInstanceCount": shared_worker_loading_instance_count,
                "sharedWorkerRunningInstanceCount": shared_worker_running_instance_count,
                "sharedWorkerClientCount": shared_worker_client_count,
                "sharedWorkerLoadingHostCount": shared_worker_loading_host_count,
                "sharedWorkerRunningWorkerIsolateCount": shared_worker_running_worker_isolate_count,
                "sharedWorkerPendingServiceLaneEventCount": shared_worker_pending_service_lane_event_count,
                "sharedWorkerProtocolDispatchRequiresLiveOwnerPageCommand": false,
            },
            "scheduler": self.scheduler_state.moli_memory_diagnostics(),
            "activeBrowserContext": active_browser_context,
            "inactiveBrowserContexts": inactive_browser_contexts,
        })
    }

    fn idle_navigation_engine_release_counts(&self) -> (usize, usize, usize) {
        let loaded_browser_context_count = self
            .browser_contexts()
            .filter(|browser_context| {
                browser_context.has_loaded_page()
                    || browser_context
                        .background_targets
                        .iter()
                        .any(BackgroundTarget::has_loaded_page)
            })
            .count();

        let live_target_browser_context_count = self
            .browser_contexts()
            .filter(|browser_context| {
                browser_context.has_active_target()
                    || !browser_context.background_targets.is_empty()
            })
            .count();

        (
            loaded_browser_context_count,
            live_target_browser_context_count,
            self.retained_background_navigation_engines.len(),
        )
    }

    pub(crate) fn release_idle_navigation_engine_memory_if_idle(
        &mut self,
    ) -> IdleNavigationEngineReleaseResult {
        let (
            loaded_browser_context_count,
            live_target_browser_context_count,
            retained_background_navigation_engine_count,
        ) = self.idle_navigation_engine_release_counts();
        let eligible = loaded_browser_context_count == 0
            && live_target_browser_context_count == 0
            && retained_background_navigation_engine_count == 0;
        if !eligible {
            return IdleNavigationEngineReleaseResult {
                reset: false,
                reason: "not-idle",
                loaded_browser_context_count,
                live_target_browser_context_count,
                retained_background_navigation_engine_count,
            };
        }

        let replacement = NavigationEngine::new_with_runtime_config(self.engine.runtime_config());
        self.replace_navigation_engine(replacement);
        IdleNavigationEngineReleaseResult {
            reset: true,
            reason: "idle-engine-replaced",
            loaded_browser_context_count,
            live_target_browser_context_count,
            retained_background_navigation_engine_count,
        }
    }

    pub(crate) fn release_idle_navigation_engine_memory_after_target_close(&mut self) {
        let result = self.release_idle_navigation_engine_memory_if_idle();
        if result.reset {
            tracing::debug!(
                target: "moli_cdp_memory",
                reason = result.reason,
                "released idle navigation engine after final target close"
            );
        }
    }

    pub(crate) fn moli_reset_idle_navigation_engine_for_diagnostics(
        &mut self,
    ) -> serde_json::Value {
        self.release_idle_navigation_engine_memory_if_idle()
            .to_protocol_json()
    }

    pub(crate) fn new_browser_context(&self, id: String) -> BrowserContext {
        let mut browser_context = self.initial_storage_partition.new_default_browser_context(
            id,
            self.fetch_config().http_cache_dir().map(PathBuf::from),
            self.fetch_config().http_cache_max_bytes(),
        );
        self.apply_global_browser_context_state(&mut browser_context);
        browser_context
    }

    pub(crate) fn ensure_browser_context_for_implicit_target_creation(&mut self) {
        if self.browser_context.is_some() || !self.inactive_browser_contexts.is_empty() {
            return;
        }
        let browser_context_id = if self.default_target_lifecycle.is_placeholder() {
            self.default_browser_context_id().to_owned()
        } else {
            self.gen_bc_id()
        };
        self.insert_browser_context(self.new_browser_context(browser_context_id));
    }

    fn apply_global_browser_context_state(&self, browser_context: &mut BrowserContext) {
        browser_context
            .network_policy
            .set_cache_disabled(self.global_cache_disabled);
        browser_context.global_extra_headers = self.global_extra_headers.clone();
        browser_context.global_network_conditions = self.global_network_conditions;
        browser_context.global_geolocation_override = self.global_geolocation_override.clone();
    }

    pub(crate) fn new_ephemeral_browser_context(&self, id: String) -> BrowserContext {
        let mut browser_context = BrowserContext::new_ephemeral_with_http_cache(
            id,
            self.fetch_config().http_cache_dir().map(PathBuf::from),
            self.fetch_config().http_cache_max_bytes(),
        );
        self.apply_global_browser_context_state(&mut browser_context);
        browser_context
    }

    pub fn snapshot_cookies(&mut self) -> Vec<StoredCookie> {
        self.browser_context
            .iter()
            .chain(self.inactive_browser_contexts.iter())
            .flat_map(BrowserContext::snapshot_cookies)
            .collect()
    }

    pub fn snapshot_profile_backed_cookies(&mut self) -> Option<Vec<StoredCookie>> {
        let mut saw_profile_backed_context = false;
        let cookies = self
            .browser_context
            .iter()
            .chain(self.inactive_browser_contexts.iter())
            .filter(|context| {
                let is_profile_backed = context.is_profile_backed_storage_partition();
                saw_profile_backed_context |= is_profile_backed;
                is_profile_backed
            })
            .flat_map(BrowserContext::snapshot_cookies)
            .collect();
        saw_profile_backed_context.then_some(cookies)
    }

    // ── ID generators ────────────────────────────────────────────────────────

    pub fn gen_bc_id(&mut self) -> String {
        self.next_bc_id = self
            .next_bc_id
            .checked_add(1)
            .expect("browser context id space exhausted");
        format!("BID-{}", self.next_bc_id)
    }

    pub fn gen_user_browser_context_id(&mut self) -> String {
        loop {
            self.next_bc_id = self
                .next_bc_id
                .checked_add(1)
                .expect("browser context id space exhausted");
            let id = format!("user-context-{}", self.next_bc_id);
            if !self.has_browser_context_id(&id) {
                return id;
            }
        }
    }

    pub fn default_browser_context_id(&self) -> &'static str {
        DEFAULT_BROWSER_CONTEXT_ID
    }

    pub fn default_target_id(&self) -> &'static str {
        DEFAULT_CDP_PAGE_TARGET_ID
    }

    pub fn default_tab_target_id(&self) -> &'static str {
        DEFAULT_CDP_TAB_TARGET_ID
    }

    pub(crate) fn devtools_target_info(&self, target_id: &str) -> Option<DevToolsTargetInfo> {
        if let Some(page_target_id) = self.primary_page_target_id_for_tab_target_id(target_id) {
            let page_target_info = self.devtools_page_or_worker_target_info(page_target_id)?;
            return self
                .target_control
                .tab_target_info_for_page_target_info(page_target_info);
        }
        self.devtools_page_or_worker_target_info(target_id)
    }

    fn devtools_page_or_worker_target_info(&self, target_id: &str) -> Option<DevToolsTargetInfo> {
        self.browser_contexts()
            .find_map(|browser_context| browser_context.devtools_target_info(target_id))
            .or_else(|| {
                (target_id == DEFAULT_CDP_PAGE_TARGET_ID)
                    .then(|| self.default_target_lifecycle.placeholder_page_info())
                    .flatten()
            })
    }

    pub(crate) fn devtools_target_infos(&self) -> Vec<DevToolsTargetInfo> {
        let mut target_infos = Vec::new();
        if let Some(page_target_info) = self.default_target_lifecycle.placeholder_page_info() {
            if let Some(tab_target_info) = self
                .target_control
                .tab_target_info_for_page_target_info(page_target_info.clone())
            {
                target_infos.push(tab_target_info);
            }
            target_infos.push(page_target_info);
        }
        for browser_context in self.browser_contexts() {
            for mut page_or_worker_target_info in browser_context.devtools_target_infos() {
                if let Some(target_id) = page_or_worker_target_info.target_id.as_ref() {
                    page_or_worker_target_info.moli_popup_id =
                        browser_context.target_popup_id(target_id.as_str());
                }
                if let Some(tab_target_info) = self
                    .target_control
                    .tab_target_info_for_page_target_info(page_or_worker_target_info.clone())
                {
                    target_infos.push(tab_target_info);
                }
                target_infos.push(page_or_worker_target_info);
            }
        }
        target_infos
    }

    pub fn publish_default_browser_target(&mut self) {
        if !self.default_target_lifecycle.publish() {
            return;
        }
        let default_target_id = self.default_target_id().to_owned();
        self.register_top_level_page_target(&default_target_id);
        self.notify_target_host_activated(&default_target_id);
    }

    /// Crosses the default target's placeholder-to-live boundary when the
    /// requested operation genuinely needs a page owner.
    pub(crate) fn ensure_default_target_live(&mut self, target_id: &str) {
        if self
            .default_target_lifecycle
            .is_placeholder_target(target_id)
        {
            self.install_default_browser_target();
        }
    }

    pub(crate) fn default_placeholder_is_logically_active(&self, target_id: &str) -> bool {
        self.default_target_lifecycle
            .is_placeholder_target(target_id)
            && self.browser_contexts().all(|browser_context| {
                !browser_context.has_active_target()
                    && browser_context.background_targets.is_empty()
            })
    }

    pub(crate) fn close_default_target_placeholder(
        &mut self,
        target_id: &str,
    ) -> Option<TargetEventPlan> {
        if !self
            .default_target_lifecycle
            .is_placeholder_target(target_id)
        {
            return None;
        }

        let target_host_closure = self.prepare_target_host_closure(DEFAULT_CDP_PAGE_TARGET_ID);
        let (detached_info_deltas, destroyed_deltas) = target_host_closure.into_parts();
        let mut plan = self.prepared_target_host_deltas_event_plan(detached_info_deltas);
        plan.extend(self.detach_closed_top_level_target_sessions_event_plan(
            DEFAULT_CDP_PAGE_TARGET_ID,
            Some("Render process gone."),
        ));
        let closed = self.default_target_lifecycle.close_placeholder(target_id);
        debug_assert!(closed, "validated default placeholder must close");
        plan.extend(self.prepared_target_host_deltas_event_plan(destroyed_deltas));
        Some(plan)
    }

    pub(crate) fn mark_default_browser_target_closed(&mut self) {
        self.default_target_lifecycle.mark_closed();
    }

    pub(crate) fn register_top_level_page_target(&mut self, page_target_id: &str) -> String {
        let tab_target_id = if page_target_id == self.default_target_id() {
            self.default_tab_target_id().to_owned()
        } else {
            self.gen_tab_target_id()
        };
        self.target_control
            .register_tab(tab_target_id.clone(), page_target_id.to_owned());
        for target_id in [&tab_target_id, page_target_id] {
            if let Some(target_info) = self.target_info_for_host_delta(target_id) {
                self.notify_target_host_lifecycle(CdpTargetHostLifecycleDelta::Created(
                    target_info,
                ));
            }
        }
        tab_target_id
    }

    pub(crate) fn register_worker_target_host(
        &mut self,
        target_id: &str,
        kind: DevToolsTargetKind,
    ) {
        self.target_control
            .register_worker(target_id.to_owned(), kind);
    }

    pub(crate) fn remove_worker_target_host(&mut self, target_id: &str) -> bool {
        self.target_control.remove_worker(target_id)
    }

    #[doc(hidden)]
    pub fn tab_target_id_for_page_target_id(&self, page_target_id: &str) -> Option<&str> {
        self.target_control
            .tab_target_id_for_page_target_id(page_target_id)
    }

    pub(crate) fn primary_page_target_id_for_tab_target_id(
        &self,
        tab_target_id: &str,
    ) -> Option<&str> {
        self.target_control
            .primary_page_target_id_for_tab_target_id(tab_target_id)
    }

    pub(crate) fn primary_session_id_for_tab_target_id(&self, tab_target_id: &str) -> Option<&str> {
        self.target_control
            .primary_session_id_for_tab_target_id(tab_target_id)
    }

    pub(crate) fn assign_session_to_tab_target(
        &mut self,
        tab_target_id: &str,
        session_id: String,
        auxiliary: bool,
    ) -> bool {
        self.target_control
            .assign_session_to_tab_target(tab_target_id, session_id, auxiliary)
    }

    pub(crate) fn remove_tab_session(&mut self, session_id: &str) -> Option<String> {
        self.target_control.remove_tab_session(session_id)
    }

    pub(crate) fn remove_tab_for_page_target(
        &mut self,
        page_target_id: &str,
    ) -> Option<TargetClosurePlan> {
        let closure_plan = self
            .target_control
            .remove_tab_by_page_target_id(page_target_id)?;
        for target_id in closure_plan.destroyed_target_ids() {
            self.notify_target_host_lifecycle(CdpTargetHostLifecycleDelta::Destroyed {
                target_id: target_id.to_owned(),
            });
        }
        Some(closure_plan)
    }

    pub(crate) fn detach_closed_top_level_target_sessions_event_plan(
        &mut self,
        page_target_id: &str,
        reason: Option<&str>,
    ) -> TargetEventPlan {
        let Some(closure_plan) = self.remove_tab_for_page_target(page_target_id) else {
            return TargetEventPlan::default();
        };
        debug_assert!(
            closure_plan
                .destroyed_target_ids()
                .any(|target_id| target_id == page_target_id)
        );
        let target = closure_plan.tab_target();
        let tab_target_id = target.id().to_owned();
        let tab_session_ids = target.session_ids();
        self.detach_target_closure_cleanup_event_plan(
            TargetClosureCleanupPlan::new(tab_target_id, reason, tab_session_ids),
            None,
        )
    }

    pub(crate) fn rollback_top_level_target_tab_sessions_without_event(
        &mut self,
        page_target_id: &str,
    ) {
        let Some(closure_plan) = self.remove_tab_for_page_target(page_target_id) else {
            return;
        };
        for session_id in closure_plan.tab_target().session_ids() {
            self.clear_auto_attach_owner(Some(&session_id));
            self.rollback_attached_session_without_event(&session_id);
        }
    }

    pub(crate) fn tab_target_id_for_session_id(&self, session_id: &str) -> Option<&str> {
        self.target_control.tab_target_id_for_session_id(session_id)
    }

    pub(crate) fn browser_context_id_for_tab_target_id(
        &self,
        tab_target_id: &str,
    ) -> Option<String> {
        let page_target_id = self.primary_page_target_id_for_tab_target_id(tab_target_id)?;
        self.browser_contexts()
            .find(|browser_context| {
                browser_context
                    .devtools_target_info(page_target_id)
                    .is_some()
            })
            .map(|browser_context| browser_context.id.clone())
    }

    pub(crate) fn tab_target_info_for_page_target_info(
        &self,
        page_target_info: &DevToolsTargetInfo,
    ) -> Option<DevToolsTargetInfo> {
        if page_target_info.kind != DevToolsTargetKind::Page {
            return None;
        }
        self.target_control
            .tab_target_info_for_page_target_info(page_target_info.clone())
    }

    pub(crate) fn tab_target_info(&self, tab_target_id: &str) -> Option<DevToolsTargetInfo> {
        let target_info = self.devtools_target_info(tab_target_id)?;
        (target_info.kind == DevToolsTargetKind::Tab).then_some(target_info)
    }

    pub(crate) fn set_target_discovery_for_owner(
        &mut self,
        owner_session_id: Option<&str>,
        filter: CdpTargetFilter,
    ) {
        let root_filter = owner_session_id
            .is_none()
            .then(|| filter.to_devtools_entries());
        self.target_control
            .set_discover_targets(owner_session_id, filter);
        if let Some(root_filter) = root_filter {
            self.target_discovery_enabled = true;
            self.target_info_change_events_enabled = true;
            self.target_discovery_filter = Some(root_filter);
        }
    }

    pub(crate) fn set_target_discovery_for_owner_from_devtools_filter(
        &mut self,
        owner_session_id: Option<&str>,
        filter: Option<Vec<DevToolsTargetFilterEntry>>,
    ) {
        let handler_filter = filter
            .clone()
            .map(CdpTargetFilter::from_devtools_entries)
            .unwrap_or_else(CdpTargetFilter::default_target_discovery);
        self.set_target_discovery_for_owner(owner_session_id, handler_filter);
        if owner_session_id.is_none() {
            self.target_discovery_filter = filter;
        }
    }

    pub(crate) fn clear_target_discovery_for_owner(&mut self, owner_session_id: Option<&str>) {
        self.target_control.clear_discover_targets(owner_session_id);
        if owner_session_id.is_none() {
            self.target_discovery_enabled = false;
            self.target_info_change_events_enabled = false;
            self.target_discovery_filter = None;
        }
    }

    pub fn root_target_discovery_enabled(&self) -> bool {
        self.target_discovery_enabled
    }

    pub fn replace_root_target_discovery_enabled(&mut self, enabled: bool) -> bool {
        let previous = self.target_discovery_enabled;
        if previous != enabled {
            self.set_root_target_discovery_enabled(enabled);
        }
        previous
    }

    pub fn set_root_target_discovery_enabled(&mut self, enabled: bool) {
        if enabled {
            self.set_target_discovery_for_owner(None, CdpTargetFilter::default_target_discovery());
        } else {
            self.clear_target_discovery_for_owner(None);
        }
    }

    pub(crate) fn target_discovery_filter_for_owner(
        &self,
        owner_session_id: Option<&str>,
    ) -> Option<Vec<DevToolsTargetFilterEntry>> {
        self.target_control
            .discover_filter_entries(owner_session_id)
    }

    pub(crate) fn initial_target_created_events_for_discovery_owner(
        &mut self,
        owner_session_id: Option<&str>,
        target_infos: Vec<DevToolsTargetInfo>,
    ) -> Vec<BackgroundProtocolEvent> {
        self.target_control
            .initial_target_created_events_for_owner(owner_session_id, target_infos)
    }

    pub(crate) fn has_any_target_discovery(&self) -> bool {
        self.target_control.has_any_discovery()
    }

    pub(crate) fn has_any_target_info_observer(&self) -> bool {
        self.target_control.has_any_target_info_observer()
    }

    fn exact_target_created_events_for_all_discovery_owners(
        &mut self,
        target_info: DevToolsTargetInfo,
    ) -> Vec<BackgroundProtocolEvent> {
        self.target_control
            .target_created_events_for_all_discovery_owners(target_info)
    }

    pub(crate) fn target_created_event_plan(&mut self, target_id: &str) -> TargetEventPlan {
        self.target_created_event_plan_for_target_delta(target_id)
    }

    fn target_created_event_plan_for_target_delta(&mut self, target_id: &str) -> TargetEventPlan {
        let deltas = self.target_control.target_created_deltas(target_id);
        self.target_host_delta_events(deltas)
    }

    pub(crate) fn target_info_changed_event_plan_for_observable_target(
        &mut self,
        browser_context_id: &str,
        target_id: &str,
    ) -> TargetEventPlan {
        let Some(target_info) = self
            .browser_context_by_id(browser_context_id)
            .and_then(|browser_context| browser_context.devtools_target_info(target_id))
        else {
            return TargetEventPlan::default();
        };
        let tab_target_info = self.tab_target_info_for_page_target_info(&target_info);
        self.notify_target_host_lifecycle(CdpTargetHostLifecycleDelta::InfoChanged(target_info));
        if let Some(tab_target_info) = tab_target_info {
            self.notify_target_host_lifecycle(CdpTargetHostLifecycleDelta::InfoChanged(
                tab_target_info,
            ));
        }
        if !self.has_any_target_info_observer() {
            return TargetEventPlan::default();
        }
        self.exact_target_info_changed_event_plan_for_target_delta(target_id)
    }

    pub(crate) fn target_info_changed_event_plan_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> TargetEventPlan {
        let Some((browser_context_id, Some(target_id))) =
            self.target_owner_identity_for_session(session_id)
        else {
            return TargetEventPlan::default();
        };
        self.target_info_changed_event_plan_for_observable_target(&browser_context_id, &target_id)
    }

    pub(crate) fn exact_target_info_changed_event_plan_for_target_delta(
        &mut self,
        target_id: &str,
    ) -> TargetEventPlan {
        self.target_host_delta_events([TargetHostDelta::info_changed(target_id.to_owned())])
    }

    fn target_host_delta_events(
        &mut self,
        deltas: impl IntoIterator<Item = TargetHostDelta>,
    ) -> TargetEventPlan {
        self.prepared_target_host_delta_events(
            deltas
                .into_iter()
                .map(PreparedTargetHostDelta::without_snapshot),
        )
    }

    pub(crate) fn prepared_target_host_delta_event_plan(
        &mut self,
        prepared_delta: PreparedTargetHostDelta,
    ) -> TargetEventPlan {
        self.prepared_target_host_deltas_event_plan([prepared_delta])
    }

    pub(crate) fn prepared_target_info_changed_event_plan_for_discovery_owners(
        &self,
        prepared_delta: PreparedTargetHostDelta,
    ) -> TargetEventPlan {
        let (delta, prepared_snapshot) = prepared_delta.into_parts();
        let TargetHostDelta::InfoChanged { target_id } = delta else {
            debug_assert!(false, "expected a prepared targetInfoChanged delta");
            return TargetEventPlan::default();
        };
        let Some(target_info) =
            prepared_snapshot.or_else(|| self.target_info_for_host_delta(&target_id))
        else {
            return TargetEventPlan::default();
        };
        TargetEventPlan::from_background_events(
            self.target_control
                .target_info_changed_events_for_all_discovery_owners(target_info),
        )
    }

    pub(crate) fn prepared_target_host_deltas_event_plan(
        &mut self,
        prepared_deltas: impl IntoIterator<Item = PreparedTargetHostDelta>,
    ) -> TargetEventPlan {
        self.prepared_target_host_delta_events(prepared_deltas)
    }

    pub(crate) fn prepare_destroyed_target_host_delta(
        &self,
        target_id: &str,
    ) -> Option<PreparedTargetHostDelta> {
        self.target_info_for_host_delta(target_id)
            .map(|target_info| {
                PreparedTargetHostDelta::destroyed(target_id.to_owned(), Some(target_info))
            })
    }

    pub(crate) fn prepare_target_host_closure(&self, target_id: &str) -> PreparedTargetHostClosure {
        let mut detached_info_deltas = Vec::new();
        let mut destroyed_deltas = Vec::new();
        for delta in self.target_control.target_destroyed_deltas(target_id) {
            let target_id = delta.target_id().to_owned();
            let Some(target_info) = self.target_info_for_host_delta(&target_id) else {
                continue;
            };
            if target_info.attached {
                let mut detached_target_info = target_info.clone();
                detached_target_info.attached = false;
                detached_info_deltas.push(PreparedTargetHostDelta::info_changed(
                    target_id.clone(),
                    Some(detached_target_info),
                ));
            }
            destroyed_deltas.push(PreparedTargetHostDelta::destroyed(
                target_id,
                Some(target_info),
            ));
        }
        PreparedTargetHostClosure::new(detached_info_deltas, destroyed_deltas)
    }

    fn prepared_target_host_delta_events(
        &mut self,
        deltas: impl IntoIterator<Item = PreparedTargetHostDelta>,
    ) -> TargetEventPlan {
        TargetEventPlan::from_background_events(
            deltas
                .into_iter()
                .flat_map(|delta| self.single_prepared_target_host_delta_events(delta))
                .collect(),
        )
    }

    fn single_prepared_target_host_delta_events(
        &mut self,
        prepared_delta: PreparedTargetHostDelta,
    ) -> Vec<BackgroundProtocolEvent> {
        let (delta, prepared_snapshot) = prepared_delta.into_parts();
        match delta {
            TargetHostDelta::Created { target_id } => {
                let Some(target_info) =
                    prepared_snapshot.or_else(|| self.target_info_for_host_delta(&target_id))
                else {
                    return Vec::new();
                };
                self.exact_target_created_events_for_all_discovery_owners(target_info)
            }
            TargetHostDelta::InfoChanged { target_id } => {
                let Some(target_info) =
                    prepared_snapshot.or_else(|| self.target_info_for_host_delta(&target_id))
                else {
                    return Vec::new();
                };
                self.exact_target_info_changed_events_for_all_observer_owners(target_info)
            }
            TargetHostDelta::Destroyed { target_id } => {
                let Some(target_info) =
                    prepared_snapshot.or_else(|| self.target_info_for_host_delta(&target_id))
                else {
                    return Vec::new();
                };
                self.exact_target_destroyed_events_for_all_discovery_owners(target_info)
            }
        }
    }

    fn target_info_for_host_delta(&self, target_id: &str) -> Option<DevToolsTargetInfo> {
        self.devtools_target_info(target_id)
    }

    fn exact_target_info_changed_events_for_all_observer_owners(
        &self,
        target_info: DevToolsTargetInfo,
    ) -> Vec<BackgroundProtocolEvent> {
        self.target_control
            .target_info_changed_events_for_all_observer_owners(target_info)
    }

    fn exact_target_destroyed_events_for_all_discovery_owners(
        &mut self,
        target_info: DevToolsTargetInfo,
    ) -> Vec<BackgroundProtocolEvent> {
        self.target_control
            .target_destroyed_events_for_all_discovery_owners(target_info)
    }

    pub(crate) fn target_crashed_events_for_all_discovery_owners(
        &self,
        target_id: &str,
        status: &str,
        error_code: i32,
    ) -> Vec<BackgroundProtocolEvent> {
        self.target_control
            .target_crashed_events_for_all_discovery_owners(target_id, status, error_code)
    }

    pub(crate) fn target_destroyed_automation_events(
        &self,
        target_info: DevToolsTargetInfo,
    ) -> Vec<BackgroundProtocolEvent> {
        target_destroyed_automation_events(
            self.project_page_tab_target_infos_for_destruction(target_info),
        )
    }

    fn project_page_tab_target_infos_for_destruction(
        &self,
        target_info: DevToolsTargetInfo,
    ) -> Vec<DevToolsTargetInfo> {
        self.target_control
            .project_page_tab_target_infos_for_destruction(target_info)
    }

    #[cfg(test)]
    pub(crate) fn tab_target_count(&self) -> usize {
        self.target_control.len()
    }

    #[cfg(test)]
    pub(crate) fn target_registry_host_kind(&self, target_id: &str) -> Option<DevToolsTargetKind> {
        self.target_control.host_kind(target_id)
    }

    pub fn install_default_browser_target(&mut self) {
        if self.default_target_lifecycle.is_live() || self.default_target_lifecycle.is_closed() {
            return;
        }

        let was_placeholder = self.default_target_lifecycle.is_placeholder();
        let default_browser_context_id = self.default_browser_context_id().to_owned();
        let default_target_id = self.default_target_id().to_owned();
        if !self.has_browser_context_id(&default_browser_context_id) {
            let mut browser_context = self.new_browser_context(default_browser_context_id.clone());
            browser_context.set_active_target_id(default_target_id.clone());
            browser_context.set_target_url("about:blank".to_owned());
            browser_context.begin_active_target_initial_empty_document("about:blank".to_owned());
            self.insert_browser_context(browser_context);
        } else {
            let browser_context = self
                .browser_context_by_id_mut(&default_browser_context_id)
                .expect("known default BrowserContext must remain addressable");
            if !browser_context.is_active_target(&default_target_id)
                && browser_context
                    .background_target(&default_target_id)
                    .is_none()
            {
                if browser_context.has_active_target() {
                    browser_context.stage_background_target(
                        default_target_id.clone(),
                        None,
                        "about:blank".to_owned(),
                        Some("about:blank".to_owned()),
                        None,
                    );
                } else {
                    browser_context.set_active_target_id(default_target_id.clone());
                    browser_context.set_target_url("about:blank".to_owned());
                    browser_context
                        .begin_active_target_initial_empty_document("about:blank".to_owned());
                }
            }
        }
        if self.target_control.host_kind(&default_target_id).is_none() {
            self.register_top_level_page_target(&default_target_id);
        }
        self.default_target_lifecycle.mark_live();
        if !was_placeholder {
            self.notify_target_host_activated(&default_target_id);
        }
    }

    pub fn enable_default_target_on_auto_attach(&mut self) {
        self.install_default_target_on_auto_attach = true;
    }

    pub(crate) fn install_default_browser_target_for_auto_attach_if_enabled(&mut self) {
        if self.install_default_target_on_auto_attach {
            self.install_default_browser_target();
        }
    }

    pub fn gen_target_id(&mut self) -> String {
        loop {
            let id = if let Some(allocator) = self.shared_target_id_allocator.as_ref() {
                allocator
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                        current.checked_add(1)
                    })
                    .expect("shared target id space exhausted")
                    + 1
            } else {
                self.next_target_id = self
                    .next_target_id
                    .checked_add(1)
                    .expect("target id space exhausted");
                u64::from(self.next_target_id)
            };
            let target_id = format!("TID-{id}");
            // Target ids supplied while restoring or embedding an existing
            // target share the same CDP namespace as ids allocated here.
            // Never let a later worker/page allocation alias such a target:
            // looking it up would otherwise return the pre-existing target's
            // kind and state even though the renderer record names a worker.
            let target_id_is_live = self.target_control.host_kind(&target_id).is_some()
                || self
                    .browser_context
                    .iter()
                    .chain(self.inactive_browser_contexts.iter())
                    .any(|context| context.devtools_target_info(&target_id).is_some());
            if !target_id_is_live {
                return target_id;
            }
        }
    }

    fn gen_tab_target_id(&mut self) -> String {
        loop {
            let id = if let Some(allocator) = self.shared_tab_target_id_allocator.as_ref() {
                allocator
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                        current.checked_add(1)
                    })
                    .expect("shared tab target id space exhausted")
                    + 1
            } else {
                self.next_tab_target_id = self
                    .next_tab_target_id
                    .checked_add(1)
                    .expect("tab target id space exhausted");
                u64::from(self.next_tab_target_id)
            };
            let target_id = format!("TAB-{id}");
            if self.target_control.host_kind(&target_id).is_none() {
                return target_id;
            }
        }
    }

    pub fn gen_session_id(&mut self) -> String {
        loop {
            self.next_session_id = self
                .next_session_id
                .checked_add(1)
                .expect("DevTools session id space exhausted");
            let session_id = format!("SID-{}", self.next_session_id);
            // Embedded callers and test/protocol bootstrap paths may install a
            // caller-supplied session id without advancing this allocator.
            // A generated id must therefore be unique in the live CDP
            // namespace, not merely unique among earlier generated ids.
            //
            // Chromium sidesteps this collision class by assigning each
            // attached DevTools session a fresh UnguessableToken. Moli
            // keeps readable ids, so it must explicitly skip occupied ones.
            if self.session_route(Some(&session_id)).is_none() {
                return session_id;
            }
        }
    }

    pub(crate) fn open_global_io_stream(&mut self, bytes: Vec<u8>) -> String {
        self.next_global_io_stream_id = self
            .next_global_io_stream_id
            .checked_add(1)
            .expect("global IO stream id space exhausted");
        let handle = format!("BROWSER-STREAM-{}", self.next_global_io_stream_id);
        self.global_io_streams
            .insert(handle.clone(), IoStreamState::from_bytes(bytes, 0));
        handle
    }

    fn notify_target_host_lifecycle(&self, delta: CdpTargetHostLifecycleDelta) {
        if let Some(observer) = self.target_host_lifecycle_observer.as_ref() {
            observer.notify(delta);
        }
    }

    pub(crate) fn notify_target_host_activated(&self, target_id: &str) {
        self.notify_target_host_lifecycle(CdpTargetHostLifecycleDelta::Activated {
            target_id: target_id.to_owned(),
        });
        if let Some(tab_target_id) = self.tab_target_id_for_page_target_id(target_id) {
            self.notify_target_host_lifecycle(CdpTargetHostLifecycleDelta::Activated {
                target_id: tab_target_id.to_owned(),
            });
        }
    }
}

pub(crate) struct NoneSessionOwnerRouteOverrideScope<'a> {
    conn: &'a mut CdpConnection,
    previous_route: Option<Option<CdpSessionRoute>>,
}

impl<'a> NoneSessionOwnerRouteOverrideScope<'a> {
    fn enter(conn: &'a mut CdpConnection, target_route: Option<CdpSessionRoute>) -> Self {
        let previous_route = conn.replace_none_session_owner_route_override(target_route);
        Self {
            conn,
            previous_route: Some(previous_route),
        }
    }

    pub(crate) fn conn_mut(&mut self) -> &mut CdpConnection {
        self.conn
    }

    pub(crate) fn restore(&mut self) {
        if let Some(previous_route) = self.previous_route.take() {
            self.conn
                .replace_none_session_owner_route_override(previous_route);
        }
    }
}

impl Drop for NoneSessionOwnerRouteOverrideScope<'_> {
    fn drop(&mut self) {
        self.restore();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserDownloadBehavior {
    pub behavior: String,
    pub download_path: Option<String>,
    pub automation_events_enabled: bool,
    pub webdriver_bidi_events_enabled: bool,
    pub browser_context_id: Option<String>,
    pub browser_context_overrides: HashMap<String, BrowserDownloadBehaviorSettings>,
    browser_event_subscription_generations: HashMap<Option<String>, u64>,
    next_browser_event_subscription_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserDownloadBehaviorSettings {
    pub behavior: String,
    pub download_path: Option<String>,
    pub automation_events_enabled: bool,
}

impl Default for BrowserDownloadBehaviorSettings {
    fn default() -> Self {
        Self {
            behavior: "default".to_owned(),
            download_path: None,
            automation_events_enabled: false,
        }
    }
}

impl Default for BrowserDownloadBehavior {
    fn default() -> Self {
        let default_settings = BrowserDownloadBehaviorSettings::default();
        Self {
            behavior: default_settings.behavior,
            download_path: default_settings.download_path,
            automation_events_enabled: default_settings.automation_events_enabled,
            webdriver_bidi_events_enabled: false,
            browser_context_id: None,
            browser_context_overrides: HashMap::new(),
            browser_event_subscription_generations: HashMap::new(),
            next_browser_event_subscription_generation: 0,
        }
    }
}

impl BrowserDownloadBehavior {
    pub(crate) fn set_global(
        &mut self,
        behavior: String,
        download_path: Option<String>,
        automation_events_enabled: bool,
    ) {
        self.behavior = behavior;
        self.download_path = download_path;
        self.automation_events_enabled = automation_events_enabled;
        self.browser_context_id = None;
    }

    pub(crate) fn set_global_policy(&mut self, behavior: String, download_path: Option<String>) {
        self.behavior = behavior;
        self.download_path = download_path;
        self.browser_context_id = None;
    }

    pub(crate) fn set_browser_context(
        &mut self,
        browser_context_id: String,
        behavior: String,
        download_path: Option<String>,
        automation_events_enabled: bool,
    ) {
        self.browser_context_overrides.insert(
            browser_context_id.clone(),
            BrowserDownloadBehaviorSettings {
                behavior: behavior.clone(),
                download_path: download_path.clone(),
                automation_events_enabled,
            },
        );
        self.browser_context_id = Some(browser_context_id);
    }

    pub(crate) fn set_browser_context_policy(
        &mut self,
        browser_context_id: String,
        behavior: String,
        download_path: Option<String>,
    ) {
        let automation_events_enabled = self
            .browser_context_overrides
            .get(&browser_context_id)
            .is_some_and(|settings| settings.automation_events_enabled);
        self.browser_context_overrides.insert(
            browser_context_id.clone(),
            BrowserDownloadBehaviorSettings {
                behavior,
                download_path,
                automation_events_enabled,
            },
        );
        self.browser_context_id = Some(browser_context_id);
    }

    pub(crate) fn reset_global(&mut self) {
        let default = BrowserDownloadBehaviorSettings::default();
        self.behavior = default.behavior;
        self.download_path = default.download_path;
        self.automation_events_enabled = default.automation_events_enabled;
        self.browser_context_id = None;
    }

    pub(crate) fn reset_browser_context(&mut self, browser_context_id: &str) {
        self.browser_context_overrides.remove(browser_context_id);
        if self.browser_context_id.as_deref() == Some(browser_context_id) {
            self.browser_context_id = None;
        }
    }

    pub(crate) fn enable_webdriver_bidi_events(&mut self) -> bool {
        let changed = !self.webdriver_bidi_events_enabled;
        self.webdriver_bidi_events_enabled = true;
        changed
    }

    pub(crate) fn disable_webdriver_bidi_events(&mut self) -> bool {
        let changed = self.webdriver_bidi_events_enabled;
        self.webdriver_bidi_events_enabled = false;
        changed
    }

    pub(crate) fn clear_browser_context(&mut self, browser_context_id: &str) {
        self.reset_browser_context(browser_context_id);
    }

    pub(crate) fn set_browser_events_enabled_for_session(
        &mut self,
        session_id: Option<&str>,
        enabled: bool,
    ) {
        self.next_browser_event_subscription_generation = self
            .next_browser_event_subscription_generation
            .wrapping_add(1);
        let session_id = session_id.map(str::to_owned);
        if enabled {
            self.browser_event_subscription_generations
                .insert(session_id, self.next_browser_event_subscription_generation);
        } else {
            self.browser_event_subscription_generations
                .remove(&session_id);
        }
    }

    pub(crate) fn browser_event_observers(&self) -> Vec<(Option<String>, u64)> {
        let mut observers = self
            .browser_event_subscription_generations
            .iter()
            .map(|(session_id, generation)| (session_id.clone(), *generation))
            .collect::<Vec<_>>();
        observers.sort_by(|left, right| left.0.cmp(&right.0));
        observers
    }

    #[cfg(test)]
    pub(crate) fn browser_event_session_ids(&self) -> Vec<Option<String>> {
        self.browser_event_observers()
            .into_iter()
            .map(|(session_id, _)| session_id)
            .collect()
    }

    pub(crate) fn browser_event_subscription_is_current(
        &self,
        session_id: Option<&str>,
        generation: u64,
    ) -> bool {
        self.browser_event_subscription_generations
            .get(&session_id.map(str::to_owned))
            .is_some_and(|current| *current == generation)
    }

    pub(crate) fn effective_for_browser_context(
        &self,
        browser_context_id: Option<&str>,
    ) -> BrowserDownloadBehaviorSettings {
        if let Some(browser_context_id) = browser_context_id
            && let Some(settings) = self.browser_context_overrides.get(browser_context_id)
        {
            return settings.clone();
        }
        BrowserDownloadBehaviorSettings {
            behavior: self.behavior.clone(),
            download_path: self.download_path.clone(),
            automation_events_enabled: self.automation_events_enabled,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PermissionOverride {
    pub permission: serde_json::Value,
    pub setting: String,
    pub origin: Option<String>,
    pub embedded_origin: Option<String>,
    pub browser_context_id: Option<String>,
}

#[cfg(test)]
mod tests;
