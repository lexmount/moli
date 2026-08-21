use super::{bridge::NativeDomBridge, history_queue::HistoryQueueState};
use crate::{
    network::context::DocumentResourceLoaderRegistry,
    {
        broadcast_channel_runtime::SharedBroadcastChannelRegistry,
        context_bootstrap::{
            SharedStorageBucketStore, SharedWebStorageStore, WeakIndexedDbManager,
            new_shared_storage_bucket_store, new_shared_web_storage_store,
        },
        css_style::CssInlineStyleDeclarationState,
        custom_elements::{
            CustomElementReactionCoordinator, CustomElementRegistryAssociation,
            CustomElementRegistryKey, CustomElementStore, RegistryAssociationRetarget,
        },
        document_runtime::{DocumentRuntime, DomHandle, EventTargetHandle},
        document_script_scheduler::{
            FrameDocumentBlockingStylesheetStore, FrameDocumentScriptSchedulerStore,
            FrameParserClassicScriptRunnerStore, FrameParserDeferredScriptOrderStore,
        },
        frame_owner_model::{
            FrameDocumentImageLoadEventBinding, FrameDocumentMediaLoadDelayBinding,
            FrameDocumentTaskOwner, FrameOwnerStore, MainDocumentImageLoadDelayBinding,
            MainDocumentMediaLoadDelayBinding,
        },
        message_port_runtime::SharedMessagePortRegistry,
        native_bridge::{bindings::NativeBridgeBindings, element::ClientRect},
        observer_runtime::{ObserverStore, ObserverStoreAccessToken},
        page_task_queue::RendererResourceCompletionSender,
        reflector::ReflectorId,
        renderer_resource_scheduler::RendererResourceScheduler,
        runtime::{
            RendererBrowserContextRuntime, RendererDocumentLifecycleJournalHandle,
            RendererPageContextCancelReceiver, SharedRendererBackendNodeRegistry,
        },
        script_provenance::CompiledStringProvenance,
        shared_worker_runtime::SharedWorkerClientEndpointOwner,
        style_engine::MoliStyleEngine,
        text_codec::TextCodecStore,
        types::{
            BroadcastChannelId, DedicatedWorkerId, ImageRequestKey,
            InFlightWorkerSubresourceFetchState, MessagePortId, NetworkBodySourceId,
            PendingSubresourceAuthState, PendingSubresourceFetchState,
            PendingSubresourceResponseState, PendingWebSocketResponseState,
            RunningSubresourceFetchState, ScriptErrorConstructorKind, ScriptNetworkOutputItem,
            StreamingSubresourceFetchState, SubresourceResourceType,
        },
        util::{
            string_from_utf16_units_lossy, utf16_units, utf16_units_contain_unpaired_surrogate,
        },
    },
};
#[cfg(test)]
use crate::{
    runtime::{
        RendererPendingDownloadActivation, RendererPendingFileChooserActivation,
        RendererPendingPopupActivation,
    },
    types::{PendingSubresourceContinueEvent, PendingSubresourceFetchInfo},
};
use indexmap::IndexMap;
use moli_shared_worker::SharedWorkerClientOwnerId;
use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    fmt,
    ops::{Deref, DerefMut},
    rc::{Rc, Weak},
};
use url::Url;
use widestring::U16String;

use self::resource_timing::SharedResourceTimingBufferRegistry;

mod activity;
mod bridge_install;
mod broadcast_channels;
mod canvas_resources;
mod child_documents;
mod child_dynamic_scripts;
mod child_events;
mod child_frame_navigation;
mod child_frame_runtime;
pub(crate) use child_frame_runtime::install_child_window_proxy_access_check_handlers;
mod child_frame_snapshots;
mod child_frames;
mod core;
mod dialogs;
mod directory_reader_callbacks;
mod document_domain;
mod document_script_ready_inputs;
mod dom_debugger;
mod element_toggle_events;
mod event_callbacks;
mod file_chooser;
mod file_entry_file_callbacks;
mod focus;
mod frame_document_ready_routes;
mod hash_changes;
mod host_environment;
mod host_loads;
mod image_decodes;
mod image_loads;
mod image_resources;
pub(crate) use image_resources::{
    CssImageResourceAdmission, CssImageResourceRequestIdentity, ImageResponseDescriptor,
    ScannedImagePreloadAdmission,
};
mod indexed_db_tasks;
mod interaction_batch;
pub(crate) use interaction_batch::PendingScrollObservableEffects;
mod internal_node_refs;
mod layout;
mod layout_snapshot;
mod layout_state;
mod live_ranges;
mod main_document_lifecycle;
mod media_element_events;
mod media_loads;
mod misc_platform_api_tasks;
pub(crate) use media_loads::PendingMediaLoadTerminalFollowup;
mod text_track_loads;
pub(crate) use text_track_loads::{
    PendingMediaCanPlayFollowup, PendingMediaTextTrackGateRegistration,
    PendingTextTrackLoadTerminal, PendingTextTrackLoadTerminalFollowup,
};
mod message_ports;
mod messages;
mod module_owner_tasks;
mod navigation;
mod opfs_tasks;
mod permissions;
mod pointer_capture;
mod popups;
mod range_records;
mod resource_loading;
mod resource_timing;
pub(crate) use resource_timing::ResourceTimingBufferId;
mod rendering_updates;
pub(crate) use rendering_updates::PostParseAutofocusAdmission;
mod runtime_bindings;
mod runtime_observable;
mod security_policy;

mod selection_records;
mod service_workers;
pub(crate) use service_workers::ServiceWorkerWindowOwner;
mod shared_workers;
mod signal_bridge;
mod storage_events;
mod stylesheet_subresource_loads;
mod text_track_default_modes;
mod traversal_state;
mod user_interaction_tasks;
mod view_transition_updates;
mod webcrypto_tasks;
mod websockets;
mod window_document_tasks;
mod window_execution_context;
mod window_security_tokens;
mod workers;
pub(crate) use window_security_tokens::set_window_security_token;

#[cfg(test)]
pub(crate) use crate::window_document_identity::LightweightPopupDocumentId;
pub(crate) use crate::window_document_identity::{
    LightweightPopupDocumentOwner, LightweightPopupLocalWindowId, WindowDocumentOwner,
};
use crate::{
    frame_owner_model::FrameDocumentLoadDeliveryTask, runtime::ServiceWorkerControlState,
    service_worker_runtime::ServiceWorkerClientId,
};
pub(crate) use bridge_install::JsContextHostBridgeRef;
pub(crate) use child_documents::{ChildDocumentLoadApplication, ChildDocumentLoadBodyActivity};
use child_documents::{ChildDocumentParserStore, PendingChildDocumentNavigation};
use child_events::ChildWindowEventListenerEntry;
use child_frame_runtime::ChildWindowProxyRecords;
pub(crate) use child_frame_runtime::{
    cross_origin_lightweight_popup_id, is_cross_origin_location_proxy,
    is_cross_origin_top_window_proxy, throw_cross_origin_location_security_error,
    throw_cross_origin_type_error,
};
pub(crate) use child_frame_snapshots::{
    ChildBrowsingContextDocumentSnapshot, ChildBrowsingContextFrameSnapshot,
    ChildBrowsingContextSnapshot, DetachedChildBrowsingContextDocumentSnapshot,
};
use child_frames::ChildBrowsingContextEntry;
pub(in crate::native_bridge::context_host) use child_frames::ChildParserClassicScriptCandidate;
pub(crate) use event_callbacks::{EventCallbackId, PreparedEventCallback};
pub(crate) use host_loads::ChildFrameAttachmentSnapshot;
#[cfg(test)]
use host_loads::ChildFrameNavigationSnapshot;
use indexed_db_tasks::IndexedDbContextState;
use messages::QueuedWindowMessage;
pub(crate) use messages::{
    PendingWindowMessage, PendingWindowMessageEndpoint, PendingWindowMessageSource,
};
pub(crate) use moli_page_types::{
    NavigationActivationSeed, NavigationHistoryDocumentId, NavigationHistoryEntryId,
    NavigationHistoryEntryKey, NavigationHistoryEntrySeed, NavigationHistorySerializedEntry,
};
pub(crate) use navigation::{PendingLocationNavigation, PendingTopLevelNavigation};
pub(crate) use popups::{
    LightweightPopupClassicScriptFetchTarget, LightweightPopupDocumentFetchTarget,
    LightweightPopupNavigationTaskToken, PopupClassicScriptLoadApplication,
    PopupDocumentLoadApplication, PopupDocumentLoadBodyActivity, active_lightweight_popup_id,
    defer_active_lightweight_popup_restore, enter_active_lightweight_popup_scope,
    enter_top_level_lightweight_popup_scope, javascript_url_csp_source,
    lightweight_popup_id_from_window, restore_active_lightweight_popup_scope,
    restore_deferred_active_lightweight_popup_scope_if_present,
};
pub(crate) use range_records::{RangeBoundarySide, RangeRecordHandle};
pub(crate) use runtime_observable::{
    PendingRuntimeObservableConsoleSourceEvent, RuntimeObservableContextToken,
    current_runtime_observable_context_token, install_runtime_observable_context_token_for_context,
};
pub(crate) use selection_records::{
    SelectionBoundaryRole, SelectionBoundarySnapshot, SelectionRecordHandle,
};
use websockets::WebSocketConnectionState;
pub(crate) use window_execution_context::{
    DetachedWindowFetchContext, WindowExecutionContextAccessPolicy, WindowExecutionContextBinding,
    WindowExecutionContextIdentity, WindowExecutionContextOwner, WindowFetchContext,
    WindowOperationReceiver, WindowOperationReceiverCaptureError, WindowTaskTarget,
};
use window_execution_context::{
    WindowExecutionContextRealmRecords, WindowExecutionContextRealmRegistration,
    WindowExecutionContextScopedRealmRegistration,
};
use workers::WorkerConnectionState;
pub(crate) use workers::WorkerOwnerScope;

pub(crate) struct PrebootstrappedChildDefaultContext {
    pub(crate) local_window_id: crate::frame_owner_model::LocalWindowId,
    pub(crate) context: v8::Global<v8::Context>,
    pub(crate) bridge_ref: JsContextHostBridgeRef,
    pub(crate) runtime_observable_context_token: RuntimeObservableContextToken,
}

pub(crate) type SharedPrebootstrappedChildDefaultContexts =
    Rc<RefCell<HashMap<DomHandle, PrebootstrappedChildDefaultContext>>>;
pub(crate) type WeakPrebootstrappedChildDefaultContexts =
    Weak<RefCell<HashMap<DomHandle, PrebootstrappedChildDefaultContext>>>;

#[derive(Clone)]
struct ChildDefaultContextBootstrapConfig {
    host: Weak<RefCell<JsContextHost>>,
    pending_contexts: WeakPrebootstrappedChildDefaultContexts,
    resource_owner_id: crate::resource_owner::ResourceOwnerId,
    promise_reject_dispatch: crate::script_vm::PromiseRejectDispatchSlot,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RuntimeBindingExecutionContext {
    local_window_id: crate::frame_owner_model::LocalWindowId,
    context_token: RuntimeObservableContextToken,
}

impl RuntimeBindingExecutionContext {
    pub(crate) fn new(
        local_window_id: crate::frame_owner_model::LocalWindowId,
        context_token: RuntimeObservableContextToken,
    ) -> Self {
        Self {
            local_window_id,
            context_token,
        }
    }

    pub(crate) fn local_window_id(self) -> crate::frame_owner_model::LocalWindowId {
        self.local_window_id
    }

    pub(crate) fn context_token(self) -> RuntimeObservableContextToken {
        self.context_token
    }

    pub(crate) fn binding_call_source_identity(
        self,
    ) -> crate::protocol_types::RuntimeBindingCallSourceIdentity {
        crate::protocol_types::RuntimeBindingCallSourceIdentity::new(
            self.local_window_id.0,
            self.context_token.as_u64(),
        )
    }
}

pub(crate) struct BroadcastChannelWrapperEntry {
    pub(crate) identity: WindowExecutionContextIdentity,
    pub(crate) context: v8::Global<v8::Context>,
    pub(crate) wrapper: v8::Global<v8::Object>,
}

pub(crate) struct MessagePortWrapperEntry {
    pub(crate) identity: WindowExecutionContextIdentity,
    pub(crate) context: v8::Global<v8::Context>,
    pub(crate) wrapper: v8::Global<v8::Object>,
    pub(crate) listeners: crate::context_bootstrap::WindowMessagePortEventListenerRegistry,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum OwnerDispatchScope {
    Top,
    Child(DomHandle),
    LightweightPopup(u64),
}

pub(crate) struct OwnerDispatchRestore<'s> {
    previous_child: v8::Local<'s, v8::Value>,
    previous_popup: v8::Local<'s, v8::Value>,
}

/// Exact Document plus the Window dispatch address used by a queued task.
///
/// A `WindowDocumentOwner` alone is not enough for child frames and
/// lightweight popups: the dispatch scope is the stable address needed to
/// resolve the target realm and DOM handle after scheduler admission.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct WindowDocumentTaskTarget {
    owner: WindowDocumentOwner,
    dispatch_scope: OwnerDispatchScope,
}

impl WindowDocumentTaskTarget {
    pub(crate) const fn new(
        owner: WindowDocumentOwner,
        dispatch_scope: OwnerDispatchScope,
    ) -> Self {
        Self {
            owner,
            dispatch_scope,
        }
    }

    pub(crate) const fn owner(self) -> WindowDocumentOwner {
        self.owner
    }

    pub(crate) const fn dispatch_scope(self) -> OwnerDispatchScope {
        self.dispatch_scope
    }
}

/// Network requests use the same exact Document address as queued Window
/// tasks. Keep the domain name as an alias at network call sites; authority is
/// supplied by the enclosing request type, not by duplicating identity state.
pub(crate) type WindowDocumentNetworkRequestIdentity = WindowDocumentTaskTarget;

impl From<WorkerOwnerScope> for OwnerDispatchScope {
    fn from(owner: WorkerOwnerScope) -> Self {
        match owner {
            WorkerOwnerScope::Top => Self::Top,
            WorkerOwnerScope::Child(handle) => Self::Child(handle),
            WorkerOwnerScope::LightweightPopup(popup_id) => Self::LightweightPopup(popup_id),
        }
    }
}

impl OwnerDispatchScope {
    pub(crate) fn child_window(self) -> Option<DomHandle> {
        match self {
            Self::Child(handle) => Some(handle),
            Self::Top | Self::LightweightPopup(_) => None,
        }
    }

    pub(crate) fn enter<'s>(self, scope: &mut v8::PinScope<'s, '_>) -> OwnerDispatchRestore<'s> {
        let previous_child = match self {
            Self::Top | Self::LightweightPopup(_) => {
                crate::native_bridge::enter_active_child_window_scope(scope, None)
            }
            Self::Child(handle) => {
                crate::native_bridge::enter_active_child_window_scope(scope, Some(handle))
            }
        };
        let previous_popup = match self {
            Self::Top | Self::Child(_) => {
                crate::native_bridge::enter_top_level_lightweight_popup_scope(scope)
            }
            Self::LightweightPopup(popup_id) => {
                crate::native_bridge::enter_active_lightweight_popup_scope(scope, popup_id)
            }
        };
        OwnerDispatchRestore {
            previous_child,
            previous_popup,
        }
    }

    pub(crate) fn defer_restore<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        previous: OwnerDispatchRestore<'s>,
    ) {
        match self {
            Self::Top | Self::Child(_) => {
                crate::native_bridge::defer_active_lightweight_popup_restore(
                    scope,
                    previous.previous_popup,
                );
                crate::native_bridge::defer_active_child_window_restore(
                    scope,
                    previous.previous_child,
                );
            }
            Self::LightweightPopup(_) => {
                crate::native_bridge::defer_active_child_window_restore(
                    scope,
                    previous.previous_child,
                );
                crate::native_bridge::defer_active_lightweight_popup_restore(
                    scope,
                    previous.previous_popup,
                );
            }
        }
    }

    pub(crate) fn restore<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        previous: OwnerDispatchRestore<'s>,
    ) {
        match self {
            Self::Top | Self::Child(_) => {
                crate::native_bridge::restore_active_lightweight_popup_scope(
                    scope,
                    previous.previous_popup,
                );
                crate::native_bridge::restore_active_child_window_scope(
                    scope,
                    previous.previous_child,
                );
            }
            Self::LightweightPopup(_) => {
                crate::native_bridge::restore_active_child_window_scope(
                    scope,
                    previous.previous_child,
                );
                crate::native_bridge::restore_active_lightweight_popup_scope(
                    scope,
                    previous.previous_popup,
                );
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ImageLoadEventId(u64);

impl ImageLoadEventId {
    pub(crate) fn new(value: u64) -> Self {
        Self(value)
    }

    pub(crate) fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ImageDecodeRequestId(u64);

impl ImageDecodeRequestId {
    pub(crate) fn new(value: u64) -> Self {
        Self(value)
    }

    pub(crate) fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingImageDecodeRequestState {
    PendingMicrotask,
    PendingLoad,
}

struct PendingImageDecodeRequest {
    id: ImageDecodeRequestId,
    element: DomHandle,
    owner_document_handle: DomHandle,
    element_owner: FrameDocumentTaskOwner,
    relevant_context: WindowExecutionContextBinding,
    resolver: v8::Global<v8::PromiseResolver>,
    request_key: ImageRequestKey,
    state: PendingImageDecodeRequestState,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ImageDecodeRetirementOutcome {
    rejected_count: usize,
    dropped_context_count: usize,
}

impl ImageDecodeRetirementOutcome {
    pub(crate) fn rejected_count(self) -> usize {
        self.rejected_count
    }

    pub(crate) fn dropped_context_count(self) -> usize {
        self.dropped_context_count
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MediaLoadSequenceId(u64);

impl MediaLoadSequenceId {
    pub(crate) fn new(value: u64) -> Self {
        Self(value)
    }

    pub(crate) fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) struct TextTrackLoadSequenceId(u64);

impl TextTrackLoadSequenceId {
    pub(crate) fn new(value: u64) -> Self {
        Self(value)
    }

    pub(crate) fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum PendingMediaLoadOwner {
    Main {
        owner: FrameDocumentTaskOwner,
        load_delay: Option<MainDocumentMediaLoadDelayBinding>,
    },
    Child {
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        load_delay: Option<FrameDocumentMediaLoadDelayBinding>,
    },
    LoadNeutral,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingMediaLoadSequence {
    id: MediaLoadSequenceId,
    owner_document_handle: DomHandle,
    owner: PendingMediaLoadOwner,
    network_state: PendingMediaLoadNetworkState,
    loadstart_dispatched: bool,
    terminal_followup_queued: bool,
    pending_text_track_count: usize,
    canplay_waiting_for_text_tracks: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PendingMediaLoadNetworkState {
    Unbound,
    Pending(u64),
    Ready,
    Failed,
}

impl PendingMediaLoadSequence {
    pub(crate) fn new(
        id: MediaLoadSequenceId,
        owner_document_handle: DomHandle,
        owner: PendingMediaLoadOwner,
    ) -> Self {
        Self {
            id,
            owner_document_handle,
            owner,
            network_state: PendingMediaLoadNetworkState::Unbound,
            loadstart_dispatched: false,
            terminal_followup_queued: false,
            pending_text_track_count: 0,
            canplay_waiting_for_text_tracks: false,
        }
    }

    pub(crate) fn id(self) -> MediaLoadSequenceId {
        self.id
    }

    pub(crate) fn owner_document_handle(self) -> DomHandle {
        self.owner_document_handle
    }

    pub(crate) fn owner(self) -> PendingMediaLoadOwner {
        self.owner
    }

    pub(crate) fn network_request_id(self) -> Option<u64> {
        match self.network_state {
            PendingMediaLoadNetworkState::Pending(internal_id) => Some(internal_id),
            PendingMediaLoadNetworkState::Unbound
            | PendingMediaLoadNetworkState::Ready
            | PendingMediaLoadNetworkState::Failed => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PendingTextTrackLoadNetworkState {
    Unbound,
    Pending(u64),
    Ready(String),
    FetchFailed,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingTextTrackLoadSequence {
    id: TextTrackLoadSequenceId,
    owner_document_handle: DomHandle,
    media_handle: DomHandle,
    target: WindowDocumentTaskTarget,
    source: String,
    network_state: PendingTextTrackLoadNetworkState,
    terminal_followup_queued: bool,
}

impl PendingTextTrackLoadSequence {
    pub(crate) fn new(
        id: TextTrackLoadSequenceId,
        owner_document_handle: DomHandle,
        media_handle: DomHandle,
        target: WindowDocumentTaskTarget,
        source: String,
    ) -> Self {
        Self {
            id,
            owner_document_handle,
            media_handle,
            target,
            source,
            network_state: PendingTextTrackLoadNetworkState::Unbound,
            terminal_followup_queued: false,
        }
    }

    pub(crate) fn id(&self) -> TextTrackLoadSequenceId {
        self.id
    }

    pub(crate) fn owner_document_handle(&self) -> DomHandle {
        self.owner_document_handle
    }

    pub(crate) fn media_handle(&self) -> DomHandle {
        self.media_handle
    }

    pub(crate) fn target(&self) -> WindowDocumentTaskTarget {
        self.target
    }

    pub(crate) fn source(&self) -> &str {
        &self.source
    }

    pub(crate) fn network_request_id(&self) -> Option<u64> {
        match self.network_state {
            PendingTextTrackLoadNetworkState::Pending(internal_id) => Some(internal_id),
            PendingTextTrackLoadNetworkState::Unbound
            | PendingTextTrackLoadNetworkState::Ready(_)
            | PendingTextTrackLoadNetworkState::FetchFailed => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct PendingMediaTextTrackGate {
    media_handle: DomHandle,
    media_sequence: MediaLoadSequenceId,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum PendingImageLoadEventOwner {
    Main(MainDocumentImageLoadDelayBinding),
    Child(FrameDocumentImageLoadEventBinding),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingImageLoadEvent {
    id: ImageLoadEventId,
    owner_document_handle: DomHandle,
    target: WindowDocumentTaskTarget,
    owner: PendingImageLoadEventOwner,
    request_initiator_type: crate::types::SubresourceRequestInitiatorType,
    network_state: PendingImageLoadNetworkState,
    terminal_followup_queued: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PendingImageLoadNetworkState {
    Unbound,
    Pending(u64),
    DecodeQueued(PendingImageLoadTerminalSource),
    Ready(PendingImageLoadTerminalSource),
    Failed(PendingImageLoadTerminalSource),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PendingImageLoadTerminalSource {
    Local,
    Network,
}

impl PendingImageLoadEvent {
    pub(crate) fn new(
        id: ImageLoadEventId,
        owner_document_handle: DomHandle,
        target: WindowDocumentTaskTarget,
        owner: PendingImageLoadEventOwner,
        request_initiator_type: crate::types::SubresourceRequestInitiatorType,
    ) -> Self {
        Self {
            id,
            owner_document_handle,
            target,
            owner,
            request_initiator_type,
            network_state: PendingImageLoadNetworkState::Unbound,
            terminal_followup_queued: false,
        }
    }

    pub(crate) fn id(self) -> ImageLoadEventId {
        self.id
    }

    pub(crate) fn owner_document_handle(self) -> DomHandle {
        self.owner_document_handle
    }

    pub(crate) fn target(self) -> WindowDocumentTaskTarget {
        self.target
    }

    pub(crate) fn owner(self) -> PendingImageLoadEventOwner {
        self.owner
    }

    pub(crate) fn request_initiator_type(self) -> crate::types::SubresourceRequestInitiatorType {
        self.request_initiator_type
    }

    pub(crate) fn network_request_id(self) -> Option<u64> {
        match self.network_state {
            PendingImageLoadNetworkState::Pending(internal_id) => Some(internal_id),
            PendingImageLoadNetworkState::Unbound
            | PendingImageLoadNetworkState::DecodeQueued(_)
            | PendingImageLoadNetworkState::Ready(_)
            | PendingImageLoadNetworkState::Failed(_) => None,
        }
    }

    pub(crate) fn terminal_source(self) -> Option<PendingImageLoadTerminalSource> {
        match self.network_state {
            PendingImageLoadNetworkState::Ready(source)
            | PendingImageLoadNetworkState::Failed(source) => Some(source),
            PendingImageLoadNetworkState::Unbound
            | PendingImageLoadNetworkState::Pending(_)
            | PendingImageLoadNetworkState::DecodeQueued(_) => None,
        }
    }

    pub(crate) fn terminal_followup(
        self,
    ) -> Option<crate::page_task_queue::RendererPageImageLoadEventKind> {
        if !self.terminal_followup_queued {
            return None;
        }
        match self.network_state {
            PendingImageLoadNetworkState::Ready(_) => {
                Some(crate::page_task_queue::RendererPageImageLoadEventKind::Load)
            }
            PendingImageLoadNetworkState::Failed(_) => {
                Some(crate::page_task_queue::RendererPageImageLoadEventKind::Error)
            }
            PendingImageLoadNetworkState::Unbound
            | PendingImageLoadNetworkState::Pending(_)
            | PendingImageLoadNetworkState::DecodeQueued(_) => None,
        }
    }
}

/// The complete PageVm-stamped route set is created at the stable Page source
/// boundary and installed atomically before a live Window can use Web APIs.
pub(crate) type JsContextHostPageTaskCapabilities =
    crate::page_task_queue::RendererPageJsContextTaskSenders;

pub(crate) struct JsContextHost {
    runtime: *mut DocumentRuntime,
    layout_policy: moli_page_types::LayoutPolicy,
    document_layout_state: RefCell<layout_state::DocumentLayoutState>,
    layout_pass_active: Cell<bool>,
    completed_layout_pass_count: Cell<u64>,
    completed_layout_pass_time: Cell<std::time::Duration>,
    last_layout_pass_metrics: Cell<Option<moli_layout::LayoutPassMetrics>>,
    layout_snapshot_cache_hits: Cell<u64>,
    layout_snapshot_cache_misses: Cell<u64>,
    layout_snapshot_cache_publishes: Cell<u64>,
    #[cfg(test)]
    force_fresh_layout_reads_for_test: bool,
    root_document_lifecycle: Option<RendererDocumentLifecycleJournalHandle>,
    output_journal: Option<crate::runtime::RendererTurnOutputJournal>,
    page_context_resources_closed: bool,
    page_default_context: Option<v8::Weak<v8::Context>>,
    pub(crate) v8_finalizers: crate::v8_finalizer::V8FinalizerRegistry,
    pub(super) bridge: NativeDomBridge,
    backend_node_registry: SharedRendererBackendNodeRegistry,
    dom_agent_state: crate::runtime::RendererDomAgentState,
    dom_debugger_state: dom_debugger::DomDebuggerState,
    #[cfg(test)]
    bridge_ref_count: Rc<Cell<usize>>,
    range_record_registry: range_records::RangeRecordRegistry,
    selection_record_registry: selection_records::SelectionRecordRegistry,
    custom_elements: CustomElementStore,
    custom_element_reactions: CustomElementReactionCoordinator,
    child_custom_elements: HashMap<DomHandle, CustomElementStore>,
    scoped_custom_elements: HashMap<u64, CustomElementStore>,
    parser_defined_autonomous_custom_elements: VecDeque<String>,
    // Parser-created direct construction leaves html5ever's open-stack
    // placeholder alive but detached. The runtime-visible element is the
    // constructor result, and this map moves later parser-appended children to
    // that result before the next script observes the DOM.
    parser_custom_element_handoff_replacements: HashMap<DomHandle, DomHandle>,
    scoped_custom_element_registry_wrappers: HashMap<u64, v8::Weak<v8::Object>>,
    custom_element_registry_associations: IndexMap<DomHandle, CustomElementRegistryAssociation>,
    next_scoped_custom_elements_registry_id: u64,
    observers: ObserverStore,
    text_codecs: TextCodecStore,
    // Key order is browsing-context insertion order and therefore frame-tree sibling order.
    child_browsing_contexts: IndexMap<DomHandle, ChildBrowsingContextEntry>,
    frame_owner_store: FrameOwnerStore,
    frame_parser_classic_scripts: FrameParserClassicScriptRunnerStore,
    frame_parser_deferred_script_order: FrameParserDeferredScriptOrderStore,
    frame_document_blocking_stylesheets: FrameDocumentBlockingStylesheetStore,
    child_document_script_schedulers: FrameDocumentScriptSchedulerStore,
    child_document_parsers: ChildDocumentParserStore,
    child_window_proxy_records: ChildWindowProxyRecords,
    child_default_context_bootstrap: Option<ChildDefaultContextBootstrapConfig>,
    #[cfg(test)]
    force_child_default_context_preflight_failure: bool,
    child_browsing_context_document_handles: HashMap<DomHandle, DomHandle>,
    document_domain_override: Option<String>,
    next_child_browsing_context_id: u64,
    next_child_document_load_id: u64,
    next_child_classic_script_load_id: u64,
    pending_child_document_navigations: HashMap<u64, PendingChildDocumentNavigation>,
    document_resource_loaders: DocumentResourceLoaderRegistry,
    web_storage_store: SharedWebStorageStore,
    session_storage_store: SharedWebStorageStore,
    indexed_db_manager: Option<WeakIndexedDbManager>,
    storage_bucket_store: SharedStorageBucketStore,
    stored_document_start_scripts: Vec<crate::DocumentStartScript>,
    stored_runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
    app_manifest_link_change_epoch: u64,
    extra_http_headers: Vec<(String, String)>,
    permission_overrides: Vec<crate::protocol_types::PermissionOverrideRegistration>,
    locale_override: Option<String>,
    timezone_override: Option<String>,
    idle_override: Option<crate::protocol_types::EmulatedIdleOverride>,
    protocol_user_gesture_activation_depth: usize,
    webdriver_bidi_file_prompt_handler_stack: Vec<String>,
    emulated_media: crate::protocol_types::EmulatedMediaOverrides,
    viewport_surface: Option<crate::protocol_types::ViewportSurface>,
    wpt_extensions_enabled: bool,
    network_offline: bool,
    blocked_url_patterns: Vec<String>,
    service_worker_client_id: ServiceWorkerClientId,
    service_worker_control: Option<ServiceWorkerControlState>,
    next_service_worker_request_id: u64,
    pending_service_worker_registers: HashMap<u64, service_workers::PendingServiceWorkerRegister>,
    pending_service_worker_unregisters:
        HashMap<u64, service_workers::PendingServiceWorkerUnregister>,
    pending_service_worker_ready: HashMap<u64, service_workers::PendingServiceWorkerReady>,
    service_worker_registration_watchers: Vec<service_workers::ServiceWorkerRegistrationWatcher>,
    service_worker_lifecycle_watched_scopes: HashSet<(Url, String, WindowDocumentOwner)>,
    service_worker_popup_clients: HashMap<u64, ServiceWorkerClientId>,
    pending_service_worker_clients_open_window_popups:
        HashMap<u64, service_workers::PendingServiceWorkerClientsOpenWindowPopup>,
    pending_window_messages: VecDeque<QueuedWindowMessage>,
    next_window_message_task_id: crate::page_task_queue::RendererPageWindowMessageTaskId,
    indexed_db_context_tasks: IndexedDbContextState,
    window_execution_contexts: HashMap<WindowExecutionContextOwner, WindowExecutionContextBinding>,
    current_window_message_source: Option<PendingWindowMessageEndpoint>,
    pending_active_child_window_restore: Option<Option<DomHandle>>,
    pending_active_lightweight_popup_restore: Option<Option<u64>>,
    /// One child async-subresource body can keep its request attribution
    /// scope active until the enclosing selected task runs its checkpoint.
    /// The task dispatcher consumes this bit immediately after that
    /// checkpoint; it must never survive into another selected task.
    pending_child_subresource_request_scope_pop: bool,
    pending_text_control_change_commit: Option<PendingTextControlChangeCommit>,
    directory_reader_callbacks: directory_reader_callbacks::DirectoryReaderCallbackState,
    misc_platform_api_tasks: misc_platform_api_tasks::MiscPlatformApiTaskState,
    file_entry_file_callbacks: file_entry_file_callbacks::FileEntryFileCallbackState,
    user_interaction_tasks: user_interaction_tasks::UserInteractionTaskState,
    pending_image_load_events: HashMap<DomHandle, PendingImageLoadEvent>,
    next_image_load_event_id: u64,
    pending_media_load_sequences: HashMap<DomHandle, PendingMediaLoadSequence>,
    next_media_load_sequence_id: u64,
    pending_text_track_load_sequences: HashMap<DomHandle, PendingTextTrackLoadSequence>,
    next_text_track_load_sequence_id: u64,
    pending_media_text_track_gates: HashMap<DomHandle, PendingMediaTextTrackGate>,
    active_pointer_capture_ids: HashSet<i32>,
    pending_pointer_capture_targets: HashMap<i32, DomHandle>,
    pointer_capture_targets: HashMap<i32, DomHandle>,
    lazy_media_load_candidates: HashSet<DomHandle>,
    canvas_resources: canvas_resources::CanvasResourceStore,
    image_resources: image_resources::ImageResourceStore,
    next_image_decode_id: u64,
    pending_image_decode_requests: HashMap<ImageDecodeRequestId, PendingImageDecodeRequest>,
    resource_timing_buffers: SharedResourceTimingBufferRegistry,
    next_webcrypto_task_id: crate::page_task_queue::RendererPageWebCryptoTaskId,
    pending_webcrypto_tasks: HashMap<
        crate::page_task_queue::RendererPageWebCryptoTaskId,
        webcrypto_tasks::PendingWebCryptoTask,
    >,
    opfs_owner_state: Option<opfs_tasks::WindowOpfsOwnerState>,
    pub(super) history_queue: HistoryQueueState,
    rendering_updates: rendering_updates::RenderingUpdateState,
    scroll_observable_effect_batch: interaction_batch::ScrollObservableEffectBatchState,
    view_transition_updates: view_transition_updates::ViewTransitionUpdateState,
    media_element_events: media_element_events::MediaElementEventState,
    element_toggle_events: element_toggle_events::ElementToggleEventState,
    text_track_default_modes: text_track_default_modes::TextTrackDefaultModeState,
    child_document_script_ready_tasks:
        document_script_ready_inputs::ChildDocumentScriptReadyTaskLedger,
    pending_child_external_classic_document_scripts:
        HashMap<u64, child_frames::PendingChildExternalClassicDocumentScriptLoad>,
    pending_child_modulepreload_work_awaiting_realm:
        VecDeque<crate::frame_owner_model::FrameDocumentModulepreloadWorkAwaitingRealm>,
    active_child_browsing_context_host_loads: Vec<DomHandle>,
    character_data_utf16_overrides: HashMap<DomHandle, U16String>,
    child_meta_refresh_navigations: HashMap<DomHandle, host_loads::ChildMetaRefreshNavigationTask>,
    disconnected_shadow_roots: HashSet<DomHandle>,
    live_stylesheets: crate::live_stylesheet::LiveStylesheetRegistry,
    style_engine: MoliStyleEngine,
    inline_style_declarations: HashMap<DomHandle, CssInlineStyleDeclarationState>,
    css_module_texts_by_url: HashMap<String, String>,
    css_module_failed_urls: HashSet<String>,
    popover_focus_restore_targets: HashMap<DomHandle, Option<DomHandle>>,
    pending_slotchange_slots: Vec<DomHandle>,
    deferred_slotchange_slots: Vec<DomHandle>,
    slotchange_flush_scheduled: bool,
    mutation_observer_delivery_depth: usize,
    #[cfg(test)]
    pending_child_frame_tree_events: Vec<crate::protocol_types::ChildFrameTreeEventSnapshot>,
    internal_node_references: HashMap<u64, DomHandle>,
    internal_inspector_value_references: HashMap<u64, v8::Global<v8::Value>>,
    #[cfg(test)]
    completed_child_browsing_context_loads: Vec<ChildFrameNavigationSnapshot>,
    #[cfg(test)]
    completed_child_document_networks:
        Vec<crate::protocol_types::ChildFrameDocumentNetworkActivitySnapshot>,
    active_child_subresource_request_scopes: Vec<DomHandle>,
    child_window_event_listeners:
        HashMap<DomHandle, IndexMap<String, Vec<ChildWindowEventListenerEntry>>>,
    next_child_window_event_registration_id: u64,
    event_callbacks: event_callbacks::EventCallbackRegistry,
    browser_context_runtime: crate::runtime::RendererBrowserContextRuntime,
    top_level_navigation_handoff_tx:
        crate::page_task_queue::RendererTopLevelNavigationHandoffSender,
    service_worker_task_tx: crate::page_task_queue::RendererPageServiceWorkerTaskSender,
    message_port_registry: SharedMessagePortRegistry,
    message_port_wrappers: HashMap<MessagePortId, MessagePortWrapperEntry>,
    broadcast_channel_registry: SharedBroadcastChannelRegistry,
    shared_worker_client_owner_id: SharedWorkerClientOwnerId,
    child_shared_worker_client_owner_ids: HashMap<DomHandle, SharedWorkerClientOwnerId>,
    shared_worker_clients: SharedWorkerClientEndpointOwner,
    top_level_storage_key: Option<moli_storage_key::MoliStorageKey>,
    web_storage_opaque_context_nonce: Option<moli_storage_key::OpaqueOriginNonce>,
    child_web_storage_opaque_context_nonces:
        HashMap<DomHandle, moli_storage_key::OpaqueOriginNonce>,
    broadcast_channel_wrappers: HashMap<BroadcastChannelId, BroadcastChannelWrapperEntry>,
    form_past_named_items: HashMap<(DomHandle, String), DomHandle>,
    button_element_targets: HashMap<(DomHandle, String), DomHandle>,
    constructing_form_data_forms: Vec<DomHandle>,
    active_form_submission_forms: Vec<DomHandle>,
    pending_form_submission_child_targets: HashMap<DomHandle, Vec<DomHandle>>,
    active_image_submitter_coordinate: Option<(DomHandle, u32, u32)>,
    current_inline_script_stack: Vec<DomHandle>,
    compiled_string_provenance: Vec<CompiledStringProvenance>,
    /// Dynamically scoped identity of the Runtime Inspector command currently
    /// entering V8. Only effects created synchronously inside that dispatch
    /// may copy this value; it is never inferred from later Page work.
    active_runtime_command_cause: Option<crate::runtime::RendererRuntimeCommandCausalIdentity>,
    /// True only while V8 Inspector is synchronously dispatching a protocol
    /// command. DebugEvaluate can expose StackFrame objects whose location
    /// accessors are invalid, so callbacks use this scope to avoid probing them.
    active_inspector_dispatch: bool,
    pending_top_level_navigation: Option<PendingTopLevelNavigation>,
    ordinary_page_turn_navigation_handoff_active: bool,
    pub(super) next_navigation_attempt_id: u64,
    pub(super) active_navigation_attempts: HashMap<u64, &'static str>,
    pub(super) navigation_lifecycle_trace: VecDeque<(u64, &'static str, &'static str)>,
    command_turn_output: Option<crate::runtime::RendererCommandTurnOutputRecorder>,
    runtime_binding_execution_context_owners:
        HashMap<RuntimeBindingExecutionContext, FrameDocumentTaskOwner>,
    window_execution_context_realms: WindowExecutionContextRealmRecords,
    #[cfg(test)]
    pending_runtime_binding_calls: Vec<PendingRuntimeBindingCall>,
    next_runtime_observable_context_token: RuntimeObservableContextToken,
    pending_runtime_observable_console_source_events:
        Vec<PendingRuntimeObservableConsoleSourceEvent>,
    #[cfg(test)]
    pending_file_chooser_activations: Vec<RendererPendingFileChooserActivation>,
    #[cfg(test)]
    pending_download_activations: Vec<RendererPendingDownloadActivation>,
    #[cfg(test)]
    pending_popup_activations: Vec<RendererPendingPopupActivation>,
    next_lightweight_popup_id: u64,
    next_lightweight_popup_local_window_id: u64,
    next_lightweight_popup_document_id: u64,
    next_lightweight_popup_document_load_id: u64,
    next_lightweight_popup_classic_script_load_id: u64,
    lightweight_popup_browsing_contexts:
        HashMap<u64, popups::LightweightPopupBrowsingContextRecord>,
    lightweight_popup_window_names: HashMap<String, u64>,
    lightweight_popup_document_handles: HashMap<DomHandle, u64>,
    pending_lightweight_popup_document_loads:
        HashMap<u64, popups::PendingLightweightPopupDocumentLoad>,
    pending_lightweight_popup_classic_script_loads:
        HashMap<u64, popups::PendingLightweightPopupClassicScriptLoad>,
    /// Standalone-only compatibility storage for locally materialized popup
    /// documents whose test/runtime adapter has no stable Page source.
    #[cfg(test)]
    pending_javascript_dialogs: Vec<dialogs::PendingJavaScriptDialogRecord>,
    javascript_dialog_runtime: crate::runtime::RendererJavaScriptDialogRuntime,
    next_javascript_dialog_id: u64,
    javascript_dialog_handler_enabled: bool,
    pending_network_output: Vec<ScriptNetworkOutputItem>,
    focus_change_epoch: u64,
    next_subresource_network_request_handle: u64,
    subresource_activity_epoch: u64,
    subresource_last_activity_at: std::time::Instant,
    fetch_subresource_interception_enabled: bool,
    fetch_subresource_interception_resource_type: Option<SubresourceResourceType>,
    active_subresource_requests: usize,
    next_pending_subresource_fetch_id: u64,
    pending_subresource_fetches: HashMap<u64, PendingSubresourceFetchState>,
    pending_subresource_auths: HashMap<u64, PendingSubresourceAuthState>,
    pending_subresource_responses: HashMap<u64, PendingSubresourceResponseState>,
    pending_websocket_responses: HashMap<u64, PendingWebSocketResponseState>,
    #[cfg(test)]
    pending_subresource_fetch_infos: Vec<PendingSubresourceFetchInfo>,
    running_subresource_fetches: HashMap<u64, RunningSubresourceFetchState>,
    streaming_subresource_fetches: HashMap<u64, StreamingSubresourceFetchState>,
    in_flight_worker_subresource_fetches: HashMap<u64, InFlightWorkerSubresourceFetchState>,
    #[cfg(test)]
    pending_subresource_continue_events: Vec<PendingSubresourceContinueEvent>,
    pub(crate) pending_network_body_sources:
        HashMap<NetworkBodySourceId, crate::network_host::PendingNetworkBodySourceState>,
    pub(crate) pending_network_body_clones: HashMap<NetworkBodySourceId, Vec<NetworkBodySourceId>>,
    page_task_capabilities: OnceCell<JsContextHostPageTaskCapabilities>,
    resource_completion_tx: RendererResourceCompletionSender,
    resource_scheduler: RendererResourceScheduler,
    next_worker_id: u64,
    workers: HashMap<DedicatedWorkerId, WorkerConnectionState>,
    next_websocket_id: u64,
    websockets: HashMap<u64, WebSocketConnectionState>,
    synchronous_xhr_request_counts: HashMap<String, u32>,
    page_context_cancel_rx: RendererPageContextCancelReceiver,
    layout_metric_trace: RefCell<LayoutMetricTrace>,
    layout_rect_cache: RefCell<HashMap<DomHandle, (u64, ClientRect)>>,
    layout_flow_top_cache: RefCell<HashMap<DomHandle, (u64, f64)>>,
    layout_mock_rendered_element_cache: RefCell<HashMap<DomHandle, (u64, bool)>>,
    layout_preceding_flow_count_cache: RefCell<HashMap<DomHandle, (u64, usize)>>,
    layout_flow_prefix_cursor_cache: RefCell<HashMap<DomHandle, (u64, Option<DomHandle>, usize)>>,
    #[cfg(test)]
    layout_flow_subtree_node_visits: Cell<u64>,
    #[cfg(test)]
    stylo_computed_style_input_builds: Cell<u64>,
    #[cfg(test)]
    stylo_style_system_key_builds: Cell<u64>,
    #[cfg(test)]
    stylo_computed_style_property_reads: Cell<u64>,
}

struct PendingTextControlChangeCommit {
    handle: DomHandle,
    committed_value: String,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct LayoutMetricTrace {
    pub(crate) client_rect_count: u64,
    pub(crate) client_rect_ns: u128,
    pub(crate) offset_parent_count: u64,
    pub(crate) offset_parent_ns: u128,
    pub(crate) offset_position_count: u64,
    pub(crate) offset_position_ns: u128,
}

impl LayoutMetricTrace {
    pub(crate) fn saturating_delta(self, before: Self) -> Self {
        Self {
            client_rect_count: self
                .client_rect_count
                .saturating_sub(before.client_rect_count),
            client_rect_ns: self.client_rect_ns.saturating_sub(before.client_rect_ns),
            offset_parent_count: self
                .offset_parent_count
                .saturating_sub(before.offset_parent_count),
            offset_parent_ns: self
                .offset_parent_ns
                .saturating_sub(before.offset_parent_ns),
            offset_position_count: self
                .offset_position_count
                .saturating_sub(before.offset_position_count),
            offset_position_ns: self
                .offset_position_ns
                .saturating_sub(before.offset_position_ns),
        }
    }

    pub(crate) fn client_rect_ms(self) -> u128 {
        self.client_rect_ns / 1_000_000
    }

    pub(crate) fn offset_parent_ms(self) -> u128 {
        self.offset_parent_ns / 1_000_000
    }

    pub(crate) fn offset_position_ms(self) -> u128 {
        self.offset_position_ns / 1_000_000
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChildBrowsingContextNavigationRequest {
    pub(crate) url: Url,
    pub(crate) method: String,
    pub(crate) body: Option<Vec<u8>>,
    pub(crate) request_headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChildBrowsingContextBootstrap {
    AboutBlank,
    Url(Url),
    Request(ChildBrowsingContextNavigationRequest),
    Srcdoc { base_url: Url, markup: String },
}

impl ChildBrowsingContextBootstrap {
    pub(in crate::native_bridge::context_host) fn security_origin_inherited(&self) -> bool {
        match self {
            Self::AboutBlank | Self::Srcdoc { .. } => true,
            Self::Url(url) => child_browsing_context_url_inherits_security_origin(url),
            Self::Request(request) => {
                child_browsing_context_url_inherits_security_origin(&request.url)
            }
        }
    }
}

fn child_browsing_context_url_inherits_security_origin(url: &url::Url) -> bool {
    (url.scheme() == "about" && url.path() == "blank") || url.scheme() == "javascript"
}

pub use crate::protocol_types::PendingRuntimeBindingCall;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PointerCaptureDispatchEvent {
    pub(crate) event_name: &'static str,
    pub(crate) target: DomHandle,
}

impl fmt::Debug for JsContextHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JsContextHost")
            .field("runtime", &self.runtime)
            .field("bridge", &self.bridge)
            .finish()
    }
}

#[cfg(test)]
impl JsContextHost {
    pub(crate) fn child_browsing_context_pending_live_navigation_for_test(
        &self,
        handle: crate::document_runtime::DomHandle,
    ) -> Option<ChildBrowsingContextBootstrap> {
        self.child_browsing_contexts
            .get(&handle)
            .and_then(|entry| entry.pending_live_navigation())
    }

    pub(crate) fn set_child_browsing_context_document_handle_for_test(
        &mut self,
        handle: crate::document_runtime::DomHandle,
        document_handle: crate::document_runtime::DomHandle,
    ) {
        self.child_browsing_context_document_handles
            .insert(handle, document_handle);
    }
}

impl Deref for JsContextHost {
    type Target = DocumentRuntime;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.runtime }
    }
}

impl DerefMut for JsContextHost {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.runtime }
    }
}

impl JsContextHost {
    pub(crate) fn advance_app_manifest_link_change_epoch(&mut self) {
        self.app_manifest_link_change_epoch = self.app_manifest_link_change_epoch.wrapping_add(1);
    }

    pub(crate) fn app_manifest_link_change_epoch(&self) -> u64 {
        self.app_manifest_link_change_epoch
    }

    pub(crate) fn close_page_context_resources_for_teardown(&mut self) {
        if self.page_context_resources_closed {
            return;
        }
        self.page_context_resources_closed = true;
        self.retire_all_document_resource_loaders();
        self.page_default_context = None;
        self.v8_finalizers.clear_for_context_teardown();
        self.clear_pending_top_level_navigation();
        self.unregister_all_service_worker_child_clients();
        self.unregister_all_service_worker_popup_clients();
        self.browser_context_runtime
            .unregister_service_worker_client(self.service_worker_client_id);
        self.close_shared_worker_clients();
        self.close_owned_broadcast_channels();
        self.close_owned_message_ports();
        self.shutdown_workers();
    }
}

impl Drop for JsContextHost {
    fn drop(&mut self) {
        self.close_page_context_resources_for_teardown();
    }
}
