use super::{
    inspector::{
        DocumentInspectorBinding, RendererInspectorIsolateBackend,
        RendererInspectorIsolateBackendHandle,
    },
    runtime_bindings::{
        PromiseRejectDispatchSlot, failed_access_check_callback, promise_reject_callback,
        promise_trace_hook,
    },
};
use crate::{
    browsing_context_model::{
        BrowsingContextGroupId, BrowsingContextId, ScriptAgentId, TopLevelWindowProxyEndpointId,
    },
    context_bootstrap::{ContextBootstrapAssets, WINDOW_OPENER_SLOT},
    document_runtime::DocumentRuntime,
    exception_reporting::v8_message_listener,
    module_runtime::{
        dynamic_import_callback, dynamic_import_with_phase_callback,
        initialize_import_meta_object_callback,
    },
    native_bridge::bindings::NativeBridgeBindings,
    native_bridge::{
        JsContextHost, JsContextHostBridgeRef, RuntimeObservableContextToken,
        SharedPrebootstrappedChildDefaultContexts,
    },
    page_task_queue::{
        PageRuntimeTaskSource, PageRuntimeWakeSender, PageTaskSender,
        RendererPageV8ForegroundTaskSender,
    },
    resource_owner::ResourceOwnerId,
    runtime::{
        RendererAuxiliaryPageReservationAllocator, RendererPageContextCancelSender,
        RendererStagedAuxiliaryWindowProxy,
    },
    util::{get_private_value, set_private_value},
    v8_platform::{
        RendererScriptAgentPageMembership, RendererScriptAgentV8ForegroundTaskRouter,
        V8ForegroundTaskWake, V8PlatformIsolateRegistration,
    },
};
use anyhow::{Result, anyhow};
use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

static DOCUMENT_ISOLATE_CREATED_COUNT: AtomicU64 = AtomicU64::new(0);
static DOCUMENT_ISOLATE_DESTROYED_COUNT: AtomicU64 = AtomicU64::new(0);
static DOCUMENT_ISOLATE_LIVE_COUNT: AtomicU64 = AtomicU64::new(0);
static DOCUMENT_ISOLATE_RESERVED_COUNT: AtomicU64 = AtomicU64::new(0);
static NEXT_SCRIPT_AGENT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Default)]
pub(crate) struct RendererDeferredContextHostReleaseQueue {
    inner: Rc<RendererDeferredContextHostReleaseQueueInner>,
}

struct RendererDeferredContextHostRelease {
    _host: Rc<RefCell<JsContextHost>>,
    retained_v8_handle_state: Vec<Box<dyn std::any::Any>>,
}

impl std::fmt::Debug for RendererDeferredContextHostRelease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RendererDeferredContextHostRelease")
            .field(
                "retained_v8_handle_state_count",
                &self.retained_v8_handle_state.len(),
            )
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
struct RendererDeferredContextHostReleaseQueueInner {
    pending: RefCell<Vec<RendererDeferredContextHostRelease>>,
    isolate_shutting_down: Cell<bool>,
}

impl RendererDeferredContextHostReleaseQueue {
    pub(crate) fn defer(
        &self,
        host: Rc<RefCell<JsContextHost>>,
        retained_v8_handle_state: Vec<Box<dyn std::any::Any>>,
    ) {
        let release = RendererDeferredContextHostRelease {
            _host: host,
            retained_v8_handle_state,
        };
        if self.inner.isolate_shutting_down.get() {
            drop(release);
            return;
        }
        self.inner.pending.borrow_mut().push(release);
    }

    fn drain_on_entered_isolate(&self) {
        loop {
            let pending = std::mem::take(&mut *self.inner.pending.borrow_mut());
            if pending.is_empty() {
                return;
            }
            drop(pending);
        }
    }

    fn begin_isolate_shutdown(&self) {
        self.drain_on_entered_isolate();
        self.inner.isolate_shutting_down.set(true);
    }
}

fn allocate_script_agent_id() -> ScriptAgentId {
    let value = NEXT_SCRIPT_AGENT_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .expect("script-agent id allocator overflow");
    ScriptAgentId::new(value)
}

pub(crate) fn renderer_document_isolate_accounting_diagnostics()
-> crate::runtime::RendererDocumentIsolateAccountingDiagnostics {
    crate::runtime::RendererDocumentIsolateAccountingDiagnostics {
        created: DOCUMENT_ISOLATE_CREATED_COUNT.load(Ordering::Relaxed),
        destroyed: DOCUMENT_ISOLATE_DESTROYED_COUNT.load(Ordering::Relaxed),
        live: DOCUMENT_ISOLATE_LIVE_COUNT.load(Ordering::Relaxed),
        reserved: DOCUMENT_ISOLATE_RESERVED_COUNT.load(Ordering::Relaxed),
    }
}

#[derive(Debug)]
pub(crate) struct RendererDocumentIsolateReservationAccounting;

impl RendererDocumentIsolateReservationAccounting {
    pub(crate) fn new() -> Self {
        DOCUMENT_ISOLATE_RESERVED_COUNT.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

impl Drop for RendererDocumentIsolateReservationAccounting {
    fn drop(&mut self) {
        let previous = DOCUMENT_ISOLATE_RESERVED_COUNT.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(previous > 0, "document isolate reservation count underflow");
    }
}

struct RendererDocumentIsolateAccountingGuard;

impl RendererDocumentIsolateAccountingGuard {
    fn new() -> Self {
        DOCUMENT_ISOLATE_CREATED_COUNT.fetch_add(1, Ordering::Relaxed);
        DOCUMENT_ISOLATE_LIVE_COUNT.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

impl Drop for RendererDocumentIsolateAccountingGuard {
    fn drop(&mut self) {
        DOCUMENT_ISOLATE_DESTROYED_COUNT.fetch_add(1, Ordering::Relaxed);
        let previous = DOCUMENT_ISOLATE_LIVE_COUNT.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(previous > 0, "document isolate live count underflow");
    }
}

pub(super) struct ScriptVmPageRealmBootstrap {
    pub(super) resource_owner_id: ResourceOwnerId,
    pub(super) promise_reject_dispatch: PromiseRejectDispatchSlot,
    pub(super) page_inspector: DocumentInspectorBinding,
    pub(super) renderer_document_isolate: RendererDocumentIsolateHandle,
    pub(super) renderer_document_isolate_teardown: RendererDocumentIsolateTeardown,
    pub(super) document_runtime: Box<DocumentRuntime>,
    pub(super) root_frame_id: Option<String>,
    pub(super) context_host: Rc<RefCell<JsContextHost>>,
    pub(super) prebootstrapped_child_default_contexts: SharedPrebootstrappedChildDefaultContexts,
    pub(super) page_context_cancel_tx: RendererPageContextCancelSender,
    pub(super) post_domcontentloaded_page_task_tx: PageTaskSender,
    pub(super) page_runtime_wake_tx: PageRuntimeWakeSender,
    pub(super) storage_bucket_store: crate::context_bootstrap::SharedStorageBucketStore,
    pub(super) renderer_page_script_environment: Option<RendererPageScriptEnvironment>,
    pub(super) script_agent_page_membership: Option<RendererScriptAgentPageMembership>,
    pub(super) reuse_main_window_proxy: bool,
}

pub(super) struct ScriptVmContextBootstrap {
    pub(super) context: v8::Global<v8::Context>,
    pub(super) runtime_observable_context_token: RuntimeObservableContextToken,
    pub(super) bridge_ref: JsContextHostBridgeRef,
}

pub(crate) struct RendererDocumentIsolateBootstrap {
    pub(super) renderer_document_isolate: RendererDocumentIsolateHandle,
    pub(super) bridge_bindings: NativeBridgeBindings,
    pub(super) renderer_document_isolate_teardown: RendererDocumentIsolateTeardown,
    pub(super) inspector_isolate_backend: RendererInspectorIsolateBackendHandle,
    pub(super) page_inspector: DocumentInspectorBinding,
    pub(super) script_agent_page_membership: Option<RendererScriptAgentPageMembership>,
    pub(super) renderer_page_script_environment: Option<RendererPageScriptEnvironment>,
    pub(super) reuse_main_window_proxy: bool,
}

impl RendererDocumentIsolateBootstrap {
    pub(crate) fn renderer_devtools_agent_token(
        &self,
    ) -> crate::runtime::RendererDevToolsAgentToken {
        self.page_inspector.agent_token()
    }

    pub(crate) fn clone_renderer_document_isolate_handle_for_owner_retention(
        &self,
    ) -> RendererDocumentIsolateHandle {
        self.renderer_document_isolate.clone()
    }

    pub(crate) fn renderer_page_script_environment(&self) -> Option<RendererPageScriptEnvironment> {
        self.renderer_page_script_environment.clone()
    }

    pub(crate) fn script_agent_page_membership(&self) -> Option<RendererScriptAgentPageMembership> {
        self.script_agent_page_membership.clone()
    }

    pub(crate) fn inspector_isolate_backend_handle(&self) -> RendererInspectorIsolateBackendHandle {
        self.inspector_isolate_backend.clone()
    }

    pub(crate) fn with_renderer_page_script_environment(
        mut self,
        environment: RendererPageScriptEnvironment,
    ) -> Self {
        self.renderer_page_script_environment = Some(environment);
        self
    }

    pub(crate) fn with_page_inspector(mut self, page_inspector: DocumentInspectorBinding) -> Self {
        self.page_inspector = page_inspector;
        self
    }

    pub(crate) fn with_reused_main_window_proxy(mut self) -> Self {
        self.reuse_main_window_proxy = true;
        self
    }
}

#[derive(Clone)]
struct RendererRelatedPageGroup {
    id: BrowsingContextGroupId,
    named_targets: Rc<RefCell<HashMap<String, Vec<Weak<RendererRelatedPageTopLevelTargetState>>>>>,
    /// Related Page order is part of named-frame lookup. Chromium walks every
    /// live related Page's complete frame tree before consulting the next Page,
    /// so a name-indexed top-level map cannot represent this authority alone.
    top_level_targets: Rc<RefCell<Vec<Weak<RendererRelatedPageTopLevelTargetState>>>>,
    /// WindowProxy routing identity belongs to the browsing-context group,
    /// not to a replaceable LocalWindow or protocol Page projection.
    next_window_proxy_endpoint_generation: Rc<Cell<u64>>,
    window_proxy_endpoints: Rc<RefCell<HashMap<u64, Weak<RendererRelatedPageTopLevelTargetState>>>>,
}

impl Default for RendererRelatedPageGroup {
    fn default() -> Self {
        Self {
            id: BrowsingContextGroupId::allocate(),
            named_targets: Rc::new(RefCell::new(HashMap::new())),
            top_level_targets: Rc::new(RefCell::new(Vec::new())),
            next_window_proxy_endpoint_generation: Rc::new(Cell::new(1)),
            window_proxy_endpoints: Rc::new(RefCell::new(HashMap::new())),
        }
    }
}

impl RendererRelatedPageGroup {
    fn allocate_window_proxy_endpoint(&self) -> TopLevelWindowProxyEndpointId {
        let generation = self.next_window_proxy_endpoint_generation.get();
        self.next_window_proxy_endpoint_generation.set(
            generation
                .checked_add(1)
                .expect("top-level WindowProxy endpoint generation overflow"),
        );
        TopLevelWindowProxyEndpointId::new(self.id, generation)
    }

    fn register_target(&self, target: &Rc<RendererRelatedPageTopLevelTargetState>) {
        let previous = self.window_proxy_endpoints.borrow_mut().insert(
            target.window_proxy_endpoint.generation(),
            Rc::downgrade(target),
        );
        assert!(
            previous.is_none(),
            "top-level WindowProxy endpoint generation must be unique within its group"
        );
        self.top_level_targets
            .borrow_mut()
            .push(Rc::downgrade(target));
    }

    fn target_for_window_proxy_endpoint(
        &self,
        endpoint: TopLevelWindowProxyEndpointId,
    ) -> Option<Rc<RendererRelatedPageTopLevelTargetState>> {
        if endpoint.browsing_context_group_id() != self.id {
            return None;
        }
        let mut endpoints = self.window_proxy_endpoints.borrow_mut();
        let Some(target) = endpoints
            .get(&endpoint.generation())
            .and_then(Weak::upgrade)
        else {
            endpoints.remove(&endpoint.generation());
            return None;
        };
        (target.window_proxy_endpoint == endpoint).then_some(target)
    }

    fn live_targets_in_page_order(&self) -> Vec<Rc<RendererRelatedPageTopLevelTargetState>> {
        let mut live = Vec::new();
        self.top_level_targets.borrow_mut().retain(|candidate| {
            let Some(candidate) = candidate.upgrade() else {
                return false;
            };
            if candidate.lifecycle.get() != RendererTopLevelBrowsingContextLifecycle::Active {
                return false;
            }
            if candidate.is_live() {
                live.push(candidate);
            }
            true
        });
        live
    }

    fn set_target_name(
        &self,
        target: &Rc<RendererRelatedPageTopLevelTargetState>,
        next_name: String,
    ) {
        let previous_name = target.name.replace(next_name.clone());
        if previous_name == next_name {
            return;
        }
        self.unregister_target_name(target, &previous_name);
        if reusable_top_level_browsing_context_name(&next_name)
            && target.lifecycle.get() == RendererTopLevelBrowsingContextLifecycle::Active
        {
            self.named_targets
                .borrow_mut()
                .entry(next_name)
                .or_default()
                .push(Rc::downgrade(target));
        }
    }

    fn unregister_target(&self, target: &Rc<RendererRelatedPageTopLevelTargetState>) {
        let name = target.name.borrow().clone();
        self.unregister_target_name(target, &name);
    }

    fn unregister_target_name(
        &self,
        target: &Rc<RendererRelatedPageTopLevelTargetState>,
        name: &str,
    ) {
        if !reusable_top_level_browsing_context_name(name) {
            return;
        }
        let mut named_targets = self.named_targets.borrow_mut();
        let remove_entry = named_targets.get_mut(name).is_some_and(|targets| {
            targets.retain(|candidate| {
                candidate
                    .upgrade()
                    .is_some_and(|candidate| !Rc::ptr_eq(&candidate, target))
            });
            targets.is_empty()
        });
        if remove_entry {
            named_targets.remove(name);
        }
    }

    fn find_named_target(
        &self,
        source: &Rc<RendererRelatedPageTopLevelTargetState>,
        name: &str,
    ) -> Option<Rc<RendererRelatedPageTopLevelTargetState>> {
        if !reusable_top_level_browsing_context_name(name) {
            return None;
        }
        if source.name.borrow().as_str() == name && source.is_live() {
            return Some(source.clone());
        }

        let mut named_targets = self.named_targets.borrow_mut();
        let mut found = None;
        let remove_entry = named_targets.get_mut(name).is_some_and(|targets| {
            targets.retain(|candidate| {
                let Some(candidate) = candidate.upgrade() else {
                    return false;
                };
                if !candidate.is_live() {
                    return false;
                }
                if found.is_none() {
                    found = Some(candidate);
                }
                true
            });
            targets.is_empty()
        });
        if remove_entry {
            named_targets.remove(name);
        }
        found
    }
}

struct RendererRelatedPageTopLevelTargetState {
    residence: crate::RendererResolvedPopupTarget,
    window_proxy_endpoint: TopLevelWindowProxyEndpointId,
    /// Current renderer execution binding for this stable logical endpoint.
    /// It rotates only when a committed Page transition replaces the script
    /// agent/channel, never for an ordinary same-agent Document replacement.
    remote_window_proxy_channel: Cell<crate::runtime::RendererRemoteWindowProxyChannel>,
    opened_by_dom: bool,
    /// One logical browsing context can have a LocalWindow projection in one
    /// script agent and RemoteWindowProxy projections in its related peers.
    /// V8 handles never cross isolates: every entry is keyed by the exact
    /// agent whose isolate owns those handles.
    projections: RefCell<HashMap<ScriptAgentId, Weak<RendererRelatedPageTopLevelTargetProjection>>>,
    /// A committed LocalWindow -> RemoteWindowProxy transition transfers the
    /// old agent projection here. The group keeps it strongly only while the
    /// logical target is live, preserving `window.open(name) === savedProxy`
    /// without letting canceled provisional agents pin an isolate.
    parked_remote_projections:
        RefCell<HashMap<ScriptAgentId, Rc<RendererRelatedPageTopLevelTargetProjection>>>,
    /// Replicated opener relationship. The concrete WindowProxy value lives
    /// in each agent projection; this endpoint is the cross-agent authority.
    opener_endpoint: RefCell<Option<TopLevelWindowProxyEndpointId>>,
    lifecycle: Cell<RendererTopLevelBrowsingContextLifecycle>,
    active: Cell<bool>,
    focused: Cell<bool>,
    name: RefCell<String>,
    current_url: RefCell<String>,
    current_serialized_origin: RefCell<String>,
    current_opaque_origin_nonce: Cell<Option<moli_storage_key::OpaqueOriginNonce>>,
    current_document_domain: RefCell<Option<String>>,
    current_cross_origin_opener_policy:
        RefCell<Option<crate::cross_origin_isolation::TopLevelDocumentCrossOriginOpenerPolicy>>,
    /// Agent-neutral projection of the current root Document's nested frame
    /// tree. Local owner handles and V8 values must never enter this carrier:
    /// observers address frames through a root-Document-qualified token and
    /// materialize their own WindowProxy facade.
    remote_frame_tree: RefCell<Vec<RendererRemoteFrameWireSnapshot>>,
    remote_frame_tree_revision: Cell<u64>,
}

struct RendererRelatedPageTopLevelTargetProjection {
    global_proxy: OnceCell<v8::Global<v8::Object>>,
    current_default_context: RefCell<Option<v8::Global<v8::Context>>>,
    // Agent-local projection of the Page-scoped opener edge. A remote agent
    // materializes this value from `opener_endpoint` on demand.
    opener_edge: RefCell<Option<v8::Global<v8::Value>>>,
    facade_context: RefCell<Option<v8::Global<v8::Context>>>,
}

impl Default for RendererRelatedPageTopLevelTargetProjection {
    fn default() -> Self {
        Self {
            global_proxy: OnceCell::new(),
            current_default_context: RefCell::new(None),
            opener_edge: RefCell::new(None),
            facade_context: RefCell::new(None),
        }
    }
}

impl RendererRelatedPageTopLevelTargetState {
    fn is_live(&self) -> bool {
        self.lifecycle.get() == RendererTopLevelBrowsingContextLifecycle::Active
    }

    fn projection(
        &self,
        script_agent_id: ScriptAgentId,
    ) -> Option<Rc<RendererRelatedPageTopLevelTargetProjection>> {
        self.projections
            .borrow()
            .get(&script_agent_id)
            .and_then(Weak::upgrade)
            .or_else(|| {
                self.parked_remote_projections
                    .borrow()
                    .get(&script_agent_id)
                    .cloned()
            })
    }

    fn register_projection(
        &self,
        script_agent_id: ScriptAgentId,
        projection: &Rc<RendererRelatedPageTopLevelTargetProjection>,
    ) {
        let previous = self
            .projections
            .borrow_mut()
            .insert(script_agent_id, Rc::downgrade(projection));
        assert!(
            previous
                .and_then(|projection| projection.upgrade())
                .is_none()
                && !self
                    .parked_remote_projections
                    .borrow()
                    .contains_key(&script_agent_id),
            "one top-level target cannot have two live projections in one script agent"
        );
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RendererRemoteTopLevelWindowProxyTarget {
    pub(crate) residence: crate::RendererResolvedPopupTarget,
    pub(crate) endpoint: TopLevelWindowProxyEndpointId,
    pub(crate) channel: crate::runtime::RendererRemoteWindowProxyChannel,
    pub(crate) opened_by_dom: bool,
    pub(crate) active: bool,
    pub(crate) focused: bool,
    pub(crate) current_url: String,
    pub(crate) current_serialized_origin: String,
    pub(crate) current_opaque_origin_nonce: Option<moli_storage_key::OpaqueOriginNonce>,
    pub(crate) current_document_domain: Option<String>,
    pub(crate) opener_endpoint: Option<TopLevelWindowProxyEndpointId>,
}

/// Stable remote-frame route within one committed top-level Document.
///
/// Nested browsing-context ids are allocated by a Document host today and can
/// be reused after a root navigation. Qualifying the id with the exact root
/// lifecycle prevents an in-flight command or retained facade from addressing
/// a same-numbered frame in the replacement Document.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererRemoteFrameToken {
    pub(crate) endpoint: TopLevelWindowProxyEndpointId,
    pub(crate) root_document: crate::runtime::RendererDocumentLifecycleIdentity,
    pub(crate) browsing_context_id: BrowsingContextId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RendererRemoteFrameSnapshot {
    /// Monotonic browser-context-state revision assigned when this complete
    /// frame tree is published. Builders use zero before publication.
    pub(crate) revision: u64,
    pub(crate) token: RendererRemoteFrameToken,
    pub(crate) parent_browsing_context_id: Option<BrowsingContextId>,
    pub(crate) name: String,
    pub(crate) current_url: String,
    pub(crate) serialized_origin: String,
    pub(crate) opaque_origin_nonce: Option<moli_storage_key::OpaqueOriginNonce>,
    pub(crate) document_domain: Option<String>,
    pub(crate) policy_container: crate::document_runtime::DocumentPolicyContainer,
}

const REMOTE_FRAME_SNAPSHOT_WIRE_VERSION: u16 = 2;
const MAX_REMOTE_FRAME_SNAPSHOT_WIRE_BYTES: usize = 4 * 1024 * 1024;
const MAX_REMOTE_FRAME_TREE_WIRE_BYTES: usize = 64 * 1024 * 1024;
const MAX_REMOTE_FRAME_TREE_SNAPSHOTS: usize = 4_096;
const MAX_REMOTE_FRAME_SNAPSHOT_STRING_BYTES: usize = 16 * 1024;
const MAX_REMOTE_FRAME_SNAPSHOT_URL_BYTES: usize = 2 * 1024 * 1024;
const MAX_REMOTE_FRAME_SNAPSHOT_POLICIES: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
struct RendererRemoteFrameWireSnapshot {
    bytes: Arc<[u8]>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RendererRemoteFrameSnapshotWire {
    version: u16,
    revision: u64,
    endpoint_group_id: u64,
    endpoint_generation: u64,
    root_frame_page_id: u64,
    root_document_page_id: u64,
    root_document_generation: u64,
    root_document_epoch: u64,
    browsing_context_id: u64,
    parent_browsing_context_id: Option<u64>,
    name: String,
    current_url: String,
    serialized_origin: String,
    opaque_origin_nonce: Option<u64>,
    document_domain: Option<String>,
    policy_container: RendererRemoteDocumentPolicyWire,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RendererRemoteDocumentPolicyWire {
    document_referrer: String,
    referrer_policy: Option<String>,
    cross_origin_embedder_policy: crate::cross_origin_isolation::CrossOriginEmbedderPolicy,
    document_isolation_policy: crate::cross_origin_isolation::DocumentIsolationPolicy,
    cross_origin_isolated: bool,
    document_content_security_policies: Vec<String>,
    response_content_security_policies: Vec<String>,
    response_content_security_report_only_policies: Vec<String>,
    content_security_reporting_endpoints:
        crate::content_security_policy::ContentSecurityPolicyReportingEndpoints,
    credentialless: bool,
    credentialless_storage_nonce: Option<u64>,
    top_navigation_without_user_gesture_is_restricted: bool,
    sandbox: crate::document_runtime::DocumentSandboxPolicy,
}

impl From<crate::document_runtime::DocumentPolicyContainer> for RendererRemoteDocumentPolicyWire {
    fn from(policy: crate::document_runtime::DocumentPolicyContainer) -> Self {
        Self {
            document_referrer: policy.document_referrer,
            referrer_policy: policy.referrer_policy,
            cross_origin_embedder_policy: policy.cross_origin_embedder_policy,
            document_isolation_policy: policy.document_isolation_policy,
            cross_origin_isolated: policy.cross_origin_isolated,
            document_content_security_policies: policy.document_content_security_policies,
            response_content_security_policies: policy.response_content_security_policies,
            response_content_security_report_only_policies: policy
                .response_content_security_report_only_policies,
            content_security_reporting_endpoints: policy.content_security_reporting_endpoints,
            credentialless: policy.credentialless,
            credentialless_storage_nonce: policy
                .credentialless_storage_nonce
                .map(moli_storage_key::OpaqueOriginNonce::get),
            top_navigation_without_user_gesture_is_restricted: policy
                .top_navigation_without_user_gesture_is_restricted,
            sandbox: policy.sandbox,
        }
    }
}

impl TryFrom<RendererRemoteDocumentPolicyWire>
    for crate::document_runtime::DocumentPolicyContainer
{
    type Error = anyhow::Error;

    fn try_from(policy: RendererRemoteDocumentPolicyWire) -> Result<Self> {
        validate_remote_frame_string(&policy.document_referrer, "document referrer")?;
        if let Some(referrer_policy) = policy.referrer_policy.as_deref() {
            validate_remote_frame_string(referrer_policy, "referrer policy")?;
        }
        for (label, policies) in [
            ("document CSP", &policy.document_content_security_policies),
            ("response CSP", &policy.response_content_security_policies),
            (
                "report-only CSP",
                &policy.response_content_security_report_only_policies,
            ),
        ] {
            anyhow::ensure!(
                policies.len() <= MAX_REMOTE_FRAME_SNAPSHOT_POLICIES,
                "remote frame {label} list exceeds the wire limit"
            );
            for value in policies {
                validate_remote_frame_string(value, label)?;
            }
        }
        Ok(Self {
            document_referrer: policy.document_referrer,
            referrer_policy: policy.referrer_policy,
            cross_origin_embedder_policy: policy.cross_origin_embedder_policy,
            document_isolation_policy: policy.document_isolation_policy,
            cross_origin_isolated: policy.cross_origin_isolated,
            document_content_security_policies: policy.document_content_security_policies,
            response_content_security_policies: policy.response_content_security_policies,
            response_content_security_report_only_policies: policy
                .response_content_security_report_only_policies,
            content_security_reporting_endpoints: policy.content_security_reporting_endpoints,
            credentialless: policy.credentialless,
            credentialless_storage_nonce: policy
                .credentialless_storage_nonce
                .map(|nonce| {
                    anyhow::ensure!(
                        nonce != 0,
                        "remote frame credentialless storage nonce is zero"
                    );
                    Ok(moli_storage_key::OpaqueOriginNonce::new(nonce))
                })
                .transpose()?,
            top_navigation_without_user_gesture_is_restricted: policy
                .top_navigation_without_user_gesture_is_restricted,
            sandbox: policy.sandbox,
        })
    }
}

impl RendererRemoteFrameWireSnapshot {
    fn encode(snapshot: RendererRemoteFrameSnapshot) -> Result<Self> {
        anyhow::ensure!(
            snapshot.revision != 0,
            "remote frame snapshot revision must be assigned before publication"
        );
        let wire = RendererRemoteFrameSnapshotWire {
            version: REMOTE_FRAME_SNAPSHOT_WIRE_VERSION,
            revision: snapshot.revision,
            endpoint_group_id: snapshot.token.endpoint.browsing_context_group_id().value(),
            endpoint_generation: snapshot.token.endpoint.generation(),
            root_frame_page_id: snapshot.token.root_document.frame.page_id.as_u64(),
            root_document_page_id: snapshot.token.root_document.document.page_id.as_u64(),
            root_document_generation: snapshot
                .token
                .root_document
                .document
                .lifecycle_document_id_for_wire(),
            root_document_epoch: snapshot.token.root_document.epoch.0,
            browsing_context_id: snapshot.token.browsing_context_id.value(),
            parent_browsing_context_id: snapshot
                .parent_browsing_context_id
                .map(crate::browsing_context_model::BrowsingContextId::value),
            name: snapshot.name,
            current_url: snapshot.current_url,
            serialized_origin: snapshot.serialized_origin,
            opaque_origin_nonce: snapshot.opaque_origin_nonce.map(|nonce| nonce.get()),
            document_domain: snapshot.document_domain,
            policy_container: snapshot.policy_container.into(),
        };
        // Decode the source-built value once before it enters replicated
        // group state. This makes source and future IPC ingress share exactly
        // the same validation contract.
        let decoded = wire.clone().into_snapshot()?;
        debug_assert_eq!(decoded.revision, wire.revision);
        let bytes = serde_json::to_vec(&wire)
            .map_err(|error| anyhow!("failed to encode remote frame snapshot: {error}"))?;
        anyhow::ensure!(
            bytes.len() <= MAX_REMOTE_FRAME_SNAPSHOT_WIRE_BYTES,
            "remote frame snapshot exceeds the wire byte limit"
        );
        Ok(Self {
            bytes: Arc::from(bytes),
        })
    }

    fn decode(&self) -> Result<RendererRemoteFrameSnapshot> {
        anyhow::ensure!(
            self.bytes.len() <= MAX_REMOTE_FRAME_SNAPSHOT_WIRE_BYTES,
            "remote frame snapshot exceeds the wire byte limit"
        );
        serde_json::from_slice::<RendererRemoteFrameSnapshotWire>(&self.bytes)
            .map_err(|error| anyhow!("invalid remote frame snapshot wire schema: {error}"))?
            .into_snapshot()
    }
}

impl RendererRemoteFrameSnapshotWire {
    fn into_snapshot(self) -> Result<RendererRemoteFrameSnapshot> {
        anyhow::ensure!(
            self.version == REMOTE_FRAME_SNAPSHOT_WIRE_VERSION,
            "unsupported remote frame snapshot wire version {}",
            self.version
        );
        anyhow::ensure!(self.revision != 0, "remote frame snapshot revision is zero");
        let endpoint = TopLevelWindowProxyEndpointId::from_wire_parts(
            self.endpoint_group_id,
            self.endpoint_generation,
        )
        .ok_or_else(|| anyhow!("remote frame snapshot endpoint is invalid"))?;
        let frame_page_id = crate::runtime::PageId::from_wire(self.root_frame_page_id)
            .ok_or_else(|| anyhow!("remote frame root Page id is zero"))?;
        let document_page_id = crate::runtime::PageId::from_wire(self.root_document_page_id)
            .ok_or_else(|| anyhow!("remote frame Document Page id is zero"))?;
        anyhow::ensure!(
            frame_page_id == document_page_id,
            "remote frame lifecycle crosses Page identities"
        );
        anyhow::ensure!(
            self.root_document_generation != 0
                && self.root_document_epoch != 0
                && self.browsing_context_id != 0,
            "remote frame snapshot contains a zero generation"
        );
        if let Some(parent) = self.parent_browsing_context_id {
            anyhow::ensure!(
                parent != 0 && parent != self.browsing_context_id,
                "remote frame snapshot contains an invalid parent identity"
            );
        }
        validate_remote_frame_string(&self.name, "name")?;
        validate_remote_frame_url(&self.current_url)?;
        validate_remote_frame_origin(&self.serialized_origin)?;
        let opaque_origin_nonce = self
            .opaque_origin_nonce
            .map(|nonce| {
                anyhow::ensure!(nonce != 0, "remote frame opaque-origin nonce is zero");
                Ok(moli_storage_key::OpaqueOriginNonce::new(nonce))
            })
            .transpose()?;
        anyhow::ensure!(
            (self.serialized_origin == "null") == opaque_origin_nonce.is_some(),
            "remote frame opaque-origin identity disagrees with its serialized origin"
        );
        anyhow::ensure!(
            self.serialized_origin != "null" || self.document_domain.is_none(),
            "remote opaque frame cannot carry document.domain"
        );
        if let Some(domain) = self.document_domain.as_deref() {
            validate_remote_frame_string(domain, "document.domain")?;
        }
        Ok(RendererRemoteFrameSnapshot {
            revision: self.revision,
            token: RendererRemoteFrameToken {
                endpoint,
                root_document: crate::runtime::RendererDocumentLifecycleIdentity {
                    frame: crate::runtime::RendererFrameToken {
                        page_id: frame_page_id,
                    },
                    document: crate::runtime::RendererDocumentToken::from_wire_parts(
                        document_page_id,
                        self.root_document_generation,
                    )
                    .ok_or_else(|| anyhow!("remote frame Document identity is zero"))?,
                    epoch: crate::runtime::RendererLifecycleEpoch(self.root_document_epoch),
                },
                browsing_context_id: BrowsingContextId::nested(self.browsing_context_id),
            },
            parent_browsing_context_id: self
                .parent_browsing_context_id
                .map(BrowsingContextId::nested),
            name: self.name,
            current_url: self.current_url,
            serialized_origin: self.serialized_origin,
            opaque_origin_nonce,
            document_domain: self.document_domain,
            policy_container: self.policy_container.try_into()?,
        })
    }
}

fn validate_remote_frame_string(value: &str, label: &str) -> Result<()> {
    anyhow::ensure!(
        value.len() <= MAX_REMOTE_FRAME_SNAPSHOT_STRING_BYTES && !value.contains('\0'),
        "remote frame snapshot {label} is invalid"
    );
    Ok(())
}

fn validate_remote_frame_url(value: &str) -> Result<()> {
    anyhow::ensure!(
        value.len() <= MAX_REMOTE_FRAME_SNAPSHOT_URL_BYTES && !value.contains('\0'),
        "remote frame snapshot URL exceeds the wire limit"
    );
    url::Url::parse(value)
        .map(|_| ())
        .map_err(|error| anyhow!("remote frame snapshot URL is invalid: {error}"))
}

fn validate_remote_frame_origin(value: &str) -> Result<()> {
    validate_remote_frame_string(value, "serialized origin")?;
    if value == "null" {
        return Ok(());
    }
    let url = url::Url::parse(value)
        .map_err(|error| anyhow!("remote frame serialized origin is invalid: {error}"))?;
    anyhow::ensure!(
        moli_url::origin_ascii_serialization(&url) == value,
        "remote frame serialized origin is not canonical"
    );
    Ok(())
}

fn encode_remote_frame_tree_for_publication(
    mut tree: Vec<RendererRemoteFrameSnapshot>,
    endpoint: TopLevelWindowProxyEndpointId,
    page_id: crate::runtime::PageId,
    revision: u64,
) -> Result<Vec<RendererRemoteFrameWireSnapshot>> {
    anyhow::ensure!(revision != 0, "remote frame tree revision is zero");
    anyhow::ensure!(
        tree.len() <= MAX_REMOTE_FRAME_TREE_SNAPSHOTS,
        "remote frame tree exceeds the snapshot count limit"
    );
    for snapshot in &mut tree {
        snapshot.revision = revision;
    }
    validate_remote_frame_tree(&tree, endpoint, page_id, revision)?;
    let encoded = tree
        .into_iter()
        .map(RendererRemoteFrameWireSnapshot::encode)
        .collect::<Result<Vec<_>>>()?;
    let encoded_bytes = encoded.iter().try_fold(0usize, |total, snapshot| {
        total.checked_add(snapshot.bytes.len())
    });
    anyhow::ensure!(
        encoded_bytes.is_some_and(|bytes| bytes <= MAX_REMOTE_FRAME_TREE_WIRE_BYTES),
        "remote frame tree exceeds the aggregate wire byte limit"
    );
    Ok(encoded)
}

fn validate_remote_frame_tree(
    tree: &[RendererRemoteFrameSnapshot],
    endpoint: TopLevelWindowProxyEndpointId,
    page_id: crate::runtime::PageId,
    revision: u64,
) -> Result<()> {
    anyhow::ensure!(
        tree.len() <= MAX_REMOTE_FRAME_TREE_SNAPSHOTS,
        "remote frame tree exceeds the snapshot count limit"
    );
    if tree.is_empty() {
        return Ok(());
    }
    anyhow::ensure!(revision != 0, "non-empty remote frame tree has no revision");
    let root_document = tree[0].token.root_document;
    let mut parents = HashMap::with_capacity(tree.len());
    for snapshot in tree {
        anyhow::ensure!(
            snapshot.revision == revision
                && snapshot.token.endpoint == endpoint
                && snapshot.token.root_document == root_document
                && snapshot.token.root_document.frame.page_id == page_id
                && snapshot.token.root_document.document.page_id == page_id,
            "remote frame tree mixes revisions, endpoints, or root Documents"
        );
        anyhow::ensure!(
            parents
                .insert(
                    snapshot.token.browsing_context_id,
                    snapshot.parent_browsing_context_id,
                )
                .is_none(),
            "remote frame tree repeats a browsing-context identity"
        );
    }
    for parent in parents.values().flatten() {
        anyhow::ensure!(
            parents.contains_key(parent),
            "remote frame tree references a missing parent"
        );
    }
    for start in parents.keys().copied() {
        let mut current = Some(start);
        for _ in 0..tree.len() {
            current = current.and_then(|id| parents.get(&id).copied().flatten());
            if current.is_none() {
                break;
            }
        }
        anyhow::ensure!(
            current.is_none(),
            "remote frame tree contains a parent cycle"
        );
    }
    Ok(())
}

struct RendererRemoteFrameWindowProxyProjection {
    token: RendererRemoteFrameToken,
    global_proxy: v8::Global<v8::Object>,
    _facade_context: v8::Global<v8::Context>,
}

pub(crate) enum RendererRelatedTopLevelWindowProxyResolution<'s> {
    Local {
        window_proxy: v8::Local<'s, v8::Object>,
        context: v8::Local<'s, v8::Context>,
    },
    Remote(RendererRemoteTopLevelWindowProxyTarget),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RendererRelatedPageTopLevelNavigationTarget {
    pub(crate) endpoint: TopLevelWindowProxyEndpointId,
    pub(crate) residence: crate::RendererResolvedPopupTarget,
    pub(crate) name: String,
    pub(crate) is_source: bool,
}

fn reusable_top_level_browsing_context_name(name: &str) -> bool {
    !name.is_empty()
        && !name.eq_ignore_ascii_case("_self")
        && !name.eq_ignore_ascii_case("_parent")
        && !name.eq_ignore_ascii_case("_top")
        && !name.eq_ignore_ascii_case("_blank")
}

#[derive(Clone)]
pub(crate) struct RendererPageScriptEnvironment {
    page_id: u64,
    /// Immutable identity copied at admission so WindowProxy callbacks never
    /// re-borrow the already-entered isolate holder merely to select an
    /// agent-local projection.
    script_agent_id: ScriptAgentId,
    auxiliary_page_reservation_allocator: RendererAuxiliaryPageReservationAllocator,
    renderer_document_isolate: RendererDocumentIsolateHandle,
    inspector_isolate_backend: RendererInspectorIsolateBackendHandle,
    script_agent_page_membership: RendererScriptAgentPageMembership,
    page_runtime_task_source: PageRuntimeTaskSource,
    output_journal: crate::runtime::RendererTurnOutputJournal,
    related_page_group: RendererRelatedPageGroup,
    top_level_target: Rc<RendererRelatedPageTopLevelTargetState>,
    /// Strong owner for this Page's LocalWindow projection. The group registry
    /// keeps only weak entries so a canceled provisional agent cannot pin V8
    /// handles or its isolate for the lifetime of the logical target.
    top_level_projection: Rc<RendererRelatedPageTopLevelTargetProjection>,
    /// Remote facades materialized in this Page's script agent. They are
    /// observer-agent projections and disappear with the observing Page.
    remote_top_level_projections: Rc<
        RefCell<
            HashMap<TopLevelWindowProxyEndpointId, Rc<RendererRelatedPageTopLevelTargetProjection>>,
        >,
    >,
    /// Stable, observer-agent-local projections of remote nested contexts.
    /// Projection ids are private V8 markers resolved only through this map;
    /// the target-side route remains the group/document/frame token above.
    remote_frame_projections: Rc<RefCell<HashMap<u64, RendererRemoteFrameWindowProxyProjection>>>,
    remote_frame_projection_ids: Rc<RefCell<HashMap<RendererRemoteFrameToken, u64>>>,
    next_remote_frame_projection_id: Rc<Cell<u64>>,
    initial_global_proxy_facade_context: Rc<RefCell<Option<v8::Global<v8::Context>>>>,
    initial_global_proxy_security_token: Rc<RefCell<Option<v8::Global<v8::Value>>>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RendererTopLevelBrowsingContextLifecycle {
    Active,
    Closing,
    Closed,
    /// A COOP commit replaced this group-visible browsing context with a new
    /// group endpoint. Old-group WindowProxy references stay safely callable
    /// but expose closed/disconnected behavior.
    Disconnected,
}

impl std::fmt::Debug for RendererPageScriptEnvironment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererPageScriptEnvironment")
            .field("page_id", &self.page_id)
            .field(
                "isolate_identity_key",
                &self.renderer_document_isolate.identity_key(),
            )
            .field("script_agent_id", &self.script_agent_id())
            .field(
                "browsing_context_group_id",
                &self.browsing_context_group_id(),
            )
            .field(
                "runtime_task_source_identity_key",
                &self.page_runtime_task_source.identity_key(),
            )
            .field("output_stream", &self.output_journal.stream())
            .field(
                "has_global_proxy",
                &self
                    .current_agent_top_level_projection()
                    .global_proxy
                    .get()
                    .is_some(),
            )
            .field(
                "has_top_level_opener_edge",
                &self
                    .current_agent_top_level_projection()
                    .opener_edge
                    .borrow()
                    .is_some(),
            )
            .field(
                "top_level_browsing_context_lifecycle",
                &self.top_level_target.lifecycle.get(),
            )
            .field("top_level_page_active", &self.top_level_target.active.get())
            .field(
                "top_level_page_focused",
                &self.top_level_target.focused.get(),
            )
            .field(
                "top_level_browsing_context_name",
                &self.top_level_target.name,
            )
            .finish()
    }
}

impl RendererPageScriptEnvironment {
    pub(crate) fn new(
        page_id: u64,
        opened_by_dom: bool,
        initially_active: bool,
        initially_focused: bool,
        auxiliary_page_reservation_allocator: RendererAuxiliaryPageReservationAllocator,
        renderer_document_isolate: RendererDocumentIsolateHandle,
        inspector_isolate_backend: RendererInspectorIsolateBackendHandle,
        script_agent_page_membership: RendererScriptAgentPageMembership,
        page_runtime_task_source: PageRuntimeTaskSource,
        output_journal: crate::runtime::RendererTurnOutputJournal,
    ) -> Result<Self> {
        Self::new_in_related_page_group(
            page_id,
            opened_by_dom,
            initially_active,
            initially_focused,
            auxiliary_page_reservation_allocator,
            renderer_document_isolate,
            inspector_isolate_backend,
            script_agent_page_membership,
            page_runtime_task_source,
            output_journal,
            RendererRelatedPageGroup::default(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_related(
        page_id: u64,
        opened_by_dom: bool,
        initially_active: bool,
        initially_focused: bool,
        auxiliary_page_reservation_allocator: RendererAuxiliaryPageReservationAllocator,
        renderer_document_isolate: RendererDocumentIsolateHandle,
        inspector_isolate_backend: RendererInspectorIsolateBackendHandle,
        script_agent_page_membership: RendererScriptAgentPageMembership,
        page_runtime_task_source: PageRuntimeTaskSource,
        output_journal: crate::runtime::RendererTurnOutputJournal,
        source_environment: &Self,
    ) -> Result<Self> {
        let environment = Self::new_in_related_page_group(
            page_id,
            opened_by_dom,
            initially_active,
            initially_focused,
            auxiliary_page_reservation_allocator,
            renderer_document_isolate,
            inspector_isolate_backend,
            script_agent_page_membership,
            page_runtime_task_source,
            output_journal,
            source_environment.related_page_group.clone(),
        )?;
        *environment.top_level_target.opener_endpoint.borrow_mut() =
            Some(source_environment.top_level_window_proxy_endpoint_id());
        Ok(environment)
    }

    #[allow(clippy::too_many_arguments)]
    fn new_in_related_page_group(
        page_id: u64,
        opened_by_dom: bool,
        initially_active: bool,
        initially_focused: bool,
        auxiliary_page_reservation_allocator: RendererAuxiliaryPageReservationAllocator,
        renderer_document_isolate: RendererDocumentIsolateHandle,
        inspector_isolate_backend: RendererInspectorIsolateBackendHandle,
        script_agent_page_membership: RendererScriptAgentPageMembership,
        page_runtime_task_source: PageRuntimeTaskSource,
        output_journal: crate::runtime::RendererTurnOutputJournal,
        related_page_group: RendererRelatedPageGroup,
    ) -> Result<Self> {
        anyhow::ensure!(
            script_agent_page_membership.page_id().as_u64() == page_id,
            "Page script environment membership belongs to a different Page"
        );
        let residence =
            crate::RendererResolvedPopupTarget::from_residence(output_journal.stream().residence())
                .ok_or_else(|| {
                    anyhow!("Page script environment has a non-Page output residence")
                })?;
        let script_agent_id = script_agent_page_membership.script_agent_id();
        let window_proxy_endpoint = related_page_group.allocate_window_proxy_endpoint();
        let projection = Rc::new(RendererRelatedPageTopLevelTargetProjection::default());
        let mut projections = HashMap::new();
        projections.insert(script_agent_id, Rc::downgrade(&projection));
        let top_level_target = Rc::new(RendererRelatedPageTopLevelTargetState {
            residence,
            window_proxy_endpoint,
            remote_window_proxy_channel: Cell::new(
                crate::runtime::RendererRemoteWindowProxyChannel::allocate(residence),
            ),
            opened_by_dom,
            projections: RefCell::new(projections),
            parked_remote_projections: RefCell::new(HashMap::new()),
            opener_endpoint: RefCell::new(None),
            lifecycle: Cell::new(RendererTopLevelBrowsingContextLifecycle::Active),
            active: Cell::new(initially_active),
            focused: Cell::new(initially_focused),
            name: RefCell::new(String::new()),
            current_url: RefCell::new(String::new()),
            current_serialized_origin: RefCell::new(String::new()),
            current_opaque_origin_nonce: Cell::new(None),
            current_document_domain: RefCell::new(None),
            current_cross_origin_opener_policy: RefCell::new(None),
            remote_frame_tree: RefCell::new(Vec::new()),
            remote_frame_tree_revision: Cell::new(0),
        });
        related_page_group.register_target(&top_level_target);
        Ok(Self {
            page_id,
            script_agent_id,
            auxiliary_page_reservation_allocator,
            renderer_document_isolate,
            inspector_isolate_backend,
            script_agent_page_membership,
            page_runtime_task_source,
            output_journal,
            related_page_group,
            top_level_target,
            top_level_projection: projection,
            remote_top_level_projections: Rc::new(RefCell::new(HashMap::new())),
            remote_frame_projections: Rc::new(RefCell::new(HashMap::new())),
            remote_frame_projection_ids: Rc::new(RefCell::new(HashMap::new())),
            next_remote_frame_projection_id: Rc::new(Cell::new(1)),
            initial_global_proxy_facade_context: Rc::new(RefCell::new(None)),
            initial_global_proxy_security_token: Rc::new(RefCell::new(None)),
        })
    }

    /// Creates the LocalWindow projection for a same-group script-agent
    /// replacement. The logical target, endpoint generation, name, opener,
    /// lifecycle and replicated policy remain owned by the existing browsing
    /// context group; only the V8 projection and inspector agent are fresh.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_same_group_remote_agent_replacement(
        auxiliary_page_reservation_allocator: RendererAuxiliaryPageReservationAllocator,
        renderer_document_isolate: RendererDocumentIsolateHandle,
        inspector_isolate_backend: RendererInspectorIsolateBackendHandle,
        script_agent_page_membership: RendererScriptAgentPageMembership,
        page_runtime_task_source: PageRuntimeTaskSource,
        output_journal: crate::runtime::RendererTurnOutputJournal,
        previous: &Self,
    ) -> Result<Self> {
        let script_agent_id = script_agent_page_membership.script_agent_id();
        anyhow::ensure!(
            script_agent_id != previous.script_agent_id(),
            "remote-agent replacement must allocate a fresh script agent"
        );
        anyhow::ensure!(
            script_agent_page_membership.page_id().as_u64() == previous.page_id,
            "remote-agent replacement membership belongs to a different Page"
        );
        anyhow::ensure!(
            crate::RendererResolvedPopupTarget::from_residence(output_journal.stream().residence())
                == Some(previous.top_level_target.residence),
            "remote-agent replacement output stream belongs to a different Page"
        );
        let projection = Rc::new(RendererRelatedPageTopLevelTargetProjection::default());
        previous
            .top_level_target
            .register_projection(script_agent_id, &projection);
        Ok(Self {
            page_id: previous.page_id,
            script_agent_id,
            auxiliary_page_reservation_allocator,
            renderer_document_isolate,
            inspector_isolate_backend,
            script_agent_page_membership,
            page_runtime_task_source,
            output_journal,
            related_page_group: previous.related_page_group.clone(),
            top_level_target: previous.top_level_target.clone(),
            top_level_projection: projection,
            remote_top_level_projections: Rc::new(RefCell::new(HashMap::new())),
            remote_frame_projections: Rc::new(RefCell::new(HashMap::new())),
            remote_frame_projection_ids: Rc::new(RefCell::new(HashMap::new())),
            next_remote_frame_projection_id: Rc::new(Cell::new(1)),
            initial_global_proxy_facade_context: Rc::new(RefCell::new(None)),
            initial_global_proxy_security_token: Rc::new(RefCell::new(None)),
        })
    }

    pub(crate) fn page_id(&self) -> u64 {
        self.page_id
    }

    pub(crate) fn opened_by_dom(&self) -> bool {
        self.top_level_target.opened_by_dom
    }

    pub(crate) fn browsing_context_group_id(&self) -> BrowsingContextGroupId {
        self.related_page_group.id
    }

    pub(crate) fn top_level_window_proxy_endpoint_id(&self) -> TopLevelWindowProxyEndpointId {
        self.top_level_target.window_proxy_endpoint
    }

    pub(crate) fn remote_window_proxy_channel(
        &self,
    ) -> crate::runtime::RendererRemoteWindowProxyChannel {
        self.top_level_target.remote_window_proxy_channel.get()
    }

    pub(crate) fn rotate_remote_window_proxy_channel_for_agent_transition(&self) {
        self.top_level_target.remote_window_proxy_channel.set(
            crate::runtime::RendererRemoteWindowProxyChannel::allocate(
                self.top_level_target.residence,
            ),
        );
    }

    fn current_agent_top_level_projection(
        &self,
    ) -> Rc<RendererRelatedPageTopLevelTargetProjection> {
        self.top_level_projection.clone()
    }

    fn current_agent_projection_for_target(
        &self,
        target: &RendererRelatedPageTopLevelTargetState,
    ) -> Option<Rc<RendererRelatedPageTopLevelTargetProjection>> {
        if std::ptr::eq(target, self.top_level_target.as_ref()) {
            return Some(self.top_level_projection.clone());
        }
        target.projection(self.script_agent_id()).or_else(|| {
            self.remote_top_level_projections
                .borrow()
                .get(&target.window_proxy_endpoint)
                .cloned()
        })
    }

    pub(crate) fn has_other_live_top_level_target(&self) -> bool {
        self.related_page_group.live_targets_in_page_order().len() > 1
    }

    pub(crate) fn current_top_level_cross_origin_opener_policy(
        &self,
    ) -> Option<crate::cross_origin_isolation::TopLevelDocumentCrossOriginOpenerPolicy> {
        self.top_level_target
            .current_cross_origin_opener_policy
            .borrow()
            .clone()
    }

    pub(crate) fn commit_top_level_cross_origin_opener_policy(
        &self,
        state: crate::cross_origin_isolation::TopLevelDocumentCrossOriginOpenerPolicy,
    ) {
        *self
            .top_level_target
            .current_cross_origin_opener_policy
            .borrow_mut() = Some(state);
    }

    pub(crate) fn top_level_page_is_focused(&self) -> bool {
        self.top_level_target.focused.get()
    }

    pub(crate) fn top_level_page_is_active(&self) -> bool {
        self.top_level_target.active.get()
    }

    pub(crate) fn top_level_page_residence(&self) -> crate::RendererResolvedPopupTarget {
        self.top_level_target.residence
    }

    pub(crate) fn set_top_level_page_activation(
        &self,
        active: bool,
        focused: bool,
    ) -> (bool, bool) {
        (
            self.top_level_target.active.replace(active) != active,
            self.top_level_target.focused.replace(focused) != focused,
        )
    }

    pub(crate) fn auxiliary_page_reservation_allocator(
        &self,
    ) -> RendererAuxiliaryPageReservationAllocator {
        self.auxiliary_page_reservation_allocator.clone()
    }

    pub(crate) fn page_runtime_task_source(&self) -> PageRuntimeTaskSource {
        self.page_runtime_task_source.clone()
    }

    pub(crate) fn output_journal(&self) -> crate::runtime::RendererTurnOutputJournal {
        self.output_journal.clone()
    }

    /// Begins the script-visible close transaction exactly once.
    ///
    /// Like Blink's `window_is_closing_`, `Closing` is observable immediately,
    /// before the browser owner has retired the target. The Page-owned output
    /// record produced by the caller is what later performs that retirement.
    pub(crate) fn begin_top_level_browsing_context_close(&self) -> bool {
        if self.top_level_target.lifecycle.get() != RendererTopLevelBrowsingContextLifecycle::Active
        {
            return false;
        }
        self.related_page_group
            .unregister_target(&self.top_level_target);
        self.top_level_target
            .lifecycle
            .set(RendererTopLevelBrowsingContextLifecycle::Closing);
        true
    }

    pub(crate) fn mark_top_level_browsing_context_closed(&self) {
        self.related_page_group
            .unregister_target(&self.top_level_target);
        self.top_level_target
            .lifecycle
            .set(RendererTopLevelBrowsingContextLifecycle::Closed);
    }

    pub(crate) fn disconnect_top_level_browsing_context_for_group_switch(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> bool {
        if self.top_level_target.lifecycle.get() != RendererTopLevelBrowsingContextLifecycle::Active
        {
            return false;
        }
        self.related_page_group
            .unregister_target(&self.top_level_target);
        self.top_level_target
            .lifecycle
            .set(RendererTopLevelBrowsingContextLifecycle::Disconnected);
        self.sever_top_level_opener_edge(scope);
        true
    }

    pub(crate) fn top_level_browsing_context_is_closed(&self) -> bool {
        self.top_level_target.lifecycle.get() != RendererTopLevelBrowsingContextLifecycle::Active
    }

    pub(crate) fn signal_top_level_close_output_handoff(&self) {
        self.page_runtime_task_source
            .signal_top_level_close_output_handoff();
    }

    pub(crate) fn stage_related_initial_empty_page_in_scope(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        source_bridge_bindings: &NativeBridgeBindings,
        pending: crate::runtime::RendererPendingAuxiliaryPage,
        init: crate::runtime::RendererRelatedInitialEmptyPageRealmInit,
    ) -> Result<()> {
        self.auxiliary_page_reservation_allocator
            .stage_related_initial_empty_page_in_scope(
                scope,
                pending,
                self,
                source_bridge_bindings,
                init,
            )
    }

    pub(crate) fn clear_page_runtime_tasks(&self) {
        self.page_runtime_task_source.clear();
    }

    pub(crate) fn retire_output_stream(&self) {
        self.output_journal
            .retire(crate::runtime::RendererOutputStreamCloseReason::ResidenceRetired);
    }

    pub(crate) fn retire_script_agent_page_membership(&self) {
        self.script_agent_page_membership.retire();
    }

    pub(crate) fn isolate_identity_key(&self) -> usize {
        self.renderer_document_isolate.identity_key()
    }

    pub(crate) fn script_agent_id(&self) -> ScriptAgentId {
        self.script_agent_id
    }

    pub(crate) fn bootstrap_replacement_document_isolate(
        &self,
    ) -> Result<RendererDocumentIsolateBootstrap> {
        let bridge_bindings = self.renderer_document_isolate.build_bridge_bindings()?;
        Ok(RendererDocumentIsolateBootstrap {
            renderer_document_isolate: self.renderer_document_isolate.clone(),
            bridge_bindings,
            renderer_document_isolate_teardown:
                RendererDocumentIsolateTeardown::owner_reserved_page(),
            inspector_isolate_backend: self.inspector_isolate_backend.clone(),
            page_inspector: DocumentInspectorBinding::new(self.inspector_isolate_backend.clone())
                .with_output_journal(self.output_journal()),
            script_agent_page_membership: None,
            renderer_page_script_environment: Some(self.clone()),
            reuse_main_window_proxy: true,
        })
    }

    pub(crate) fn bootstrap_related_page_document_isolate(
        &self,
        v8_foreground_task_sender: RendererPageV8ForegroundTaskSender,
    ) -> Result<RendererDocumentIsolateBootstrap> {
        let script_agent_page_membership = self
            .script_agent_page_membership
            .admit_related_page(v8_foreground_task_sender)?;
        let bridge_bindings = match self.renderer_document_isolate.build_bridge_bindings() {
            Ok(bindings) => bindings,
            Err(error) => {
                script_agent_page_membership.retire();
                return Err(error);
            }
        };
        Ok(self
            .related_page_document_isolate_bootstrap(bridge_bindings, script_agent_page_membership))
    }

    /// Prepares an explicitly related Page isolate bootstrap without
    /// re-entering or re-borrowing the document-isolate holder.
    ///
    /// This is the admission half of synchronous auxiliary realm creation.
    /// The caller owns an already-entered opener scope, so the source Page's
    /// retained membership and bridge templates are the only authorities this
    /// operation may use.
    pub(crate) fn bootstrap_related_page_document_isolate_in_scope(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        source_bridge_bindings: &NativeBridgeBindings,
        v8_foreground_task_sender: RendererPageV8ForegroundTaskSender,
    ) -> Result<RendererDocumentIsolateBootstrap> {
        let script_agent_page_membership = self
            .script_agent_page_membership
            .admit_related_page(v8_foreground_task_sender)?;
        let bridge_bindings = source_bridge_bindings.build_peer_in_scope(scope);
        Ok(self
            .related_page_document_isolate_bootstrap(bridge_bindings, script_agent_page_membership))
    }

    fn related_page_document_isolate_bootstrap(
        &self,
        bridge_bindings: NativeBridgeBindings,
        script_agent_page_membership: RendererScriptAgentPageMembership,
    ) -> RendererDocumentIsolateBootstrap {
        RendererDocumentIsolateBootstrap {
            renderer_document_isolate: self.renderer_document_isolate.clone(),
            bridge_bindings,
            renderer_document_isolate_teardown:
                RendererDocumentIsolateTeardown::owner_reserved_page(),
            inspector_isolate_backend: self.inspector_isolate_backend.clone(),
            page_inspector: DocumentInspectorBinding::new(self.inspector_isolate_backend.clone()),
            script_agent_page_membership: Some(script_agent_page_membership),
            renderer_page_script_environment: None,
            reuse_main_window_proxy: false,
        }
    }

    pub(super) fn install_initial_main_window_proxy(
        &self,
        global_proxy: v8::Global<v8::Object>,
    ) -> Result<()> {
        self.current_agent_top_level_projection()
            .global_proxy
            .set(global_proxy)
            .map_err(|_| anyhow!("page script environment already retains its main WindowProxy"))
    }

    pub(crate) fn install_staged_initial_main_window_proxy(
        &self,
        staged: RendererStagedAuxiliaryWindowProxy,
    ) -> Result<()> {
        anyhow::ensure!(
            self.initial_global_proxy_facade_context.borrow().is_none(),
            "page script environment already retains a WindowProxy facade context"
        );
        let (window_proxy, facade_context, security_token) = staged.into_parts();
        self.install_initial_main_window_proxy(window_proxy)?;
        *self.initial_global_proxy_facade_context.borrow_mut() = Some(facade_context);
        *self.initial_global_proxy_security_token.borrow_mut() = security_token;
        Ok(())
    }

    pub(super) fn take_main_window_proxy_for_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> Result<v8::Local<'s, v8::Object>> {
        let window_proxy =
            self.with_main_window_proxy(|window_proxy| v8::Local::new(scope, window_proxy))?;
        if let Some(facade_context) = self.initial_global_proxy_facade_context.borrow_mut().take() {
            v8::Local::new(scope, &facade_context).detach_global();
        }
        Ok(window_proxy)
    }

    pub(super) fn take_initial_main_window_security_token<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        self.initial_global_proxy_security_token
            .borrow_mut()
            .take()
            .map(|token| v8::Local::new(scope, &token))
    }

    pub(super) fn with_main_window_proxy<T>(
        &self,
        op: impl FnOnce(&v8::Global<v8::Object>) -> T,
    ) -> Result<T> {
        let projection = self.current_agent_top_level_projection();
        let global_proxy = projection.global_proxy.get().ok_or_else(|| {
            anyhow!("replacement context is missing its page-owned main WindowProxy")
        })?;
        Ok(op(global_proxy))
    }

    pub(crate) fn set_top_level_browsing_context_name(&self, name: String) {
        self.related_page_group
            .set_target_name(&self.top_level_target, name);
    }

    pub(crate) fn related_page_named_target_for_navigation<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        name: &str,
        replacement_opener: Option<v8::Local<'s, v8::Object>>,
    ) -> Option<(
        v8::Local<'s, v8::Object>,
        v8::Local<'s, v8::Context>,
        crate::RendererResolvedPopupTarget,
    )> {
        let target = self
            .related_page_group
            .find_named_target(&self.top_level_target, name)?;
        let projection = self.current_agent_projection_for_target(&target)?;
        if let Some(opener) = replacement_opener {
            let opener: v8::Local<'s, v8::Value> = opener.into();
            *target.opener_endpoint.borrow_mut() = Some(self.top_level_window_proxy_endpoint_id());
            *projection.opener_edge.borrow_mut() = Some(v8::Global::new(scope, opener));
        }
        let window_proxy = v8::Local::new(scope, projection.global_proxy.get()?);
        let context = v8::Local::new(scope, projection.current_default_context.borrow().as_ref()?);
        Some((window_proxy, context, target.residence))
    }

    pub(crate) fn related_page_top_level_targets_for_navigation(
        &self,
    ) -> Vec<RendererRelatedPageTopLevelNavigationTarget> {
        self.related_page_group
            .live_targets_in_page_order()
            .into_iter()
            .map(|target| {
                let name = target.name.borrow().clone();
                RendererRelatedPageTopLevelNavigationTarget {
                    endpoint: target.window_proxy_endpoint,
                    residence: target.residence,
                    name,
                    is_source: Rc::ptr_eq(&target, &self.top_level_target),
                }
            })
            .collect()
    }

    pub(crate) fn related_page_current_context_for_residence<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        residence: crate::RendererResolvedPopupTarget,
    ) -> Option<v8::Local<'s, v8::Context>> {
        let target = self
            .related_page_group
            .live_targets_in_page_order()
            .into_iter()
            .find(|target| target.residence == residence)?;
        let projection = self.current_agent_projection_for_target(&target)?;
        Some(v8::Local::new(
            scope,
            projection.current_default_context.borrow().as_ref()?,
        ))
    }

    /// Resolves a group-qualified WindowProxy endpoint only while its exact
    /// target state remains the active owner. Normal Document replacement
    /// updates the state in place; close and COOP disconnection make every
    /// previously projected endpoint stale before any replacement can route.
    pub(crate) fn related_page_target_for_window_proxy_endpoint<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        endpoint: TopLevelWindowProxyEndpointId,
    ) -> Option<RendererRelatedTopLevelWindowProxyResolution<'s>> {
        let target = self
            .related_page_group
            .target_for_window_proxy_endpoint(endpoint)?;
        if !target.is_live() {
            return None;
        }
        if let Some(projection) = self.current_agent_projection_for_target(&target)
            && let Some(context) = projection.current_default_context.borrow().as_ref()
        {
            let window_proxy = v8::Local::new(scope, projection.global_proxy.get()?);
            let context = v8::Local::new(scope, context);
            return Some(RendererRelatedTopLevelWindowProxyResolution::Local {
                window_proxy,
                context,
            });
        }
        Some(RendererRelatedTopLevelWindowProxyResolution::Remote(
            RendererRemoteTopLevelWindowProxyTarget {
                residence: target.residence,
                endpoint: target.window_proxy_endpoint,
                channel: target.remote_window_proxy_channel.get(),
                opened_by_dom: target.opened_by_dom,
                active: target.active.get(),
                focused: target.focused.get(),
                current_url: target.current_url.borrow().clone(),
                current_serialized_origin: target.current_serialized_origin.borrow().clone(),
                current_opaque_origin_nonce: target.current_opaque_origin_nonce.get(),
                current_document_domain: target.current_document_domain.borrow().clone(),
                opener_endpoint: *target.opener_endpoint.borrow(),
            },
        ))
    }

    pub(crate) fn bind_current_main_default_context(&self, context: v8::Global<v8::Context>) {
        let projection = self.current_agent_top_level_projection();
        *projection.current_default_context.borrow_mut() = Some(context);
    }

    pub(crate) fn replicate_current_top_level_document(
        &self,
        url: &url::Url,
        serialized_origin: &str,
        opaque_origin_nonce: Option<moli_storage_key::OpaqueOriginNonce>,
        document_domain: Option<String>,
    ) {
        assert_eq!(
            serialized_origin == "null",
            opaque_origin_nonce.is_some(),
            "top-level opaque-origin replication requires one exact nonce"
        );
        assert!(
            opaque_origin_nonce.is_none_or(|nonce| nonce.get() != 0),
            "top-level opaque-origin replication rejects a zero nonce"
        );
        assert!(
            serialized_origin != "null" || document_domain.is_none(),
            "an opaque top-level origin cannot replicate document.domain"
        );
        *self.top_level_target.current_url.borrow_mut() = url.to_string();
        *self.top_level_target.current_serialized_origin.borrow_mut() =
            serialized_origin.to_owned();
        self.top_level_target
            .current_opaque_origin_nonce
            .set(opaque_origin_nonce);
        *self.top_level_target.current_document_domain.borrow_mut() = document_domain;
    }

    pub(crate) fn replicate_current_remote_frame_tree(
        &self,
        snapshots: Vec<RendererRemoteFrameSnapshot>,
    ) {
        let Some(revision) = self
            .top_level_target
            .remote_frame_tree_revision
            .get()
            .checked_add(1)
        else {
            tracing::warn!(
                "remote frame tree revision overflowed; disconnecting the replicated tree"
            );
            self.top_level_target.remote_frame_tree.borrow_mut().clear();
            return;
        };
        let snapshots = match encode_remote_frame_tree_for_publication(
            snapshots,
            self.top_level_window_proxy_endpoint_id(),
            self.top_level_target.residence.page_id(),
            revision,
        ) {
            Ok(snapshots) => snapshots,
            Err(error) => {
                // Names, URLs, policies and frame counts are web-controlled.
                // A value that cannot cross the remote boundary must retire
                // the previous revision instead of crashing the renderer or
                // leaving a stale tree routable.
                tracing::warn!(%error, revision, "rejected remote frame tree publication");
                self.top_level_target
                    .remote_frame_tree_revision
                    .set(revision);
                self.top_level_target.remote_frame_tree.borrow_mut().clear();
                return;
            }
        };
        self.top_level_target
            .remote_frame_tree_revision
            .set(revision);
        *self.top_level_target.remote_frame_tree.borrow_mut() = snapshots;
    }

    pub(crate) fn clear_current_remote_frame_tree(&self) {
        let Some(revision) = self
            .top_level_target
            .remote_frame_tree_revision
            .get()
            .checked_add(1)
        else {
            tracing::warn!(
                "remote frame tree revision overflowed while disconnecting the replicated tree"
            );
            self.top_level_target.remote_frame_tree.borrow_mut().clear();
            return;
        };
        self.top_level_target
            .remote_frame_tree_revision
            .set(revision);
        self.top_level_target.remote_frame_tree.borrow_mut().clear();
    }

    pub(crate) fn remote_frame_tree_snapshot(
        &self,
        endpoint: TopLevelWindowProxyEndpointId,
    ) -> Option<Vec<RendererRemoteFrameSnapshot>> {
        let target = self
            .related_page_group
            .target_for_window_proxy_endpoint(endpoint)?;
        if !target.is_live() {
            return None;
        }
        let revision = target.remote_frame_tree_revision.get();
        let wire_tree = target.remote_frame_tree.borrow();
        if wire_tree.len() > MAX_REMOTE_FRAME_TREE_SNAPSHOTS
            || wire_tree
                .iter()
                .try_fold(0usize, |total, snapshot| {
                    total.checked_add(snapshot.bytes.len())
                })
                .is_none_or(|bytes| bytes > MAX_REMOTE_FRAME_TREE_WIRE_BYTES)
        {
            return None;
        }
        let tree = wire_tree
            .iter()
            .map(RendererRemoteFrameWireSnapshot::decode)
            .collect::<Result<Vec<_>>>()
            .ok()?;
        validate_remote_frame_tree(&tree, endpoint, target.residence.page_id(), revision).ok()?;
        Some(tree)
    }

    pub(crate) fn remote_frame_snapshot(
        &self,
        token: RendererRemoteFrameToken,
    ) -> Option<RendererRemoteFrameSnapshot> {
        self.remote_frame_tree_snapshot(token.endpoint)?
            .into_iter()
            .find(|snapshot| snapshot.token == token)
    }

    pub(crate) fn remote_frame_direct_children(
        &self,
        endpoint: TopLevelWindowProxyEndpointId,
        parent: Option<RendererRemoteFrameToken>,
    ) -> Option<Vec<RendererRemoteFrameSnapshot>> {
        if parent.is_some_and(|parent| parent.endpoint != endpoint) {
            return None;
        }
        let parent_id = parent.map(|parent| parent.browsing_context_id);
        let tree = self.remote_frame_tree_snapshot(endpoint)?;
        if let Some(parent) = parent
            && !tree.iter().any(|snapshot| snapshot.token == parent)
        {
            return None;
        }
        Some(
            tree.into_iter()
                .filter(|snapshot| snapshot.parent_browsing_context_id == parent_id)
                .collect(),
        )
    }

    pub(crate) fn allocate_remote_frame_projection_id(&self) -> u64 {
        let id = self.next_remote_frame_projection_id.get();
        self.next_remote_frame_projection_id.set(
            id.checked_add(1)
                .expect("remote-frame WindowProxy projection id overflow"),
        );
        id
    }

    pub(crate) fn install_remote_frame_window_proxy_projection(
        &self,
        id: u64,
        token: RendererRemoteFrameToken,
        global_proxy: v8::Global<v8::Object>,
        facade_context: v8::Global<v8::Context>,
    ) -> Result<()> {
        anyhow::ensure!(
            self.remote_frame_snapshot(token).is_some(),
            "remote-frame WindowProxy target is no longer current"
        );
        anyhow::ensure!(
            !self
                .remote_frame_projection_ids
                .borrow()
                .contains_key(&token),
            "remote-frame WindowProxy projection is already installed"
        );
        let previous = self.remote_frame_projections.borrow_mut().insert(
            id,
            RendererRemoteFrameWindowProxyProjection {
                token,
                global_proxy,
                _facade_context: facade_context,
            },
        );
        anyhow::ensure!(
            previous.is_none(),
            "remote-frame WindowProxy projection id is already installed"
        );
        self.remote_frame_projection_ids
            .borrow_mut()
            .insert(token, id);
        Ok(())
    }

    pub(crate) fn projected_remote_frame_window_proxy<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        token: RendererRemoteFrameToken,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let id = *self.remote_frame_projection_ids.borrow().get(&token)?;
        let projections = self.remote_frame_projections.borrow();
        let projection = projections.get(&id)?;
        Some(v8::Local::new(scope, &projection.global_proxy))
    }

    pub(crate) fn remote_frame_token_for_projection_id(
        &self,
        id: u64,
    ) -> Option<RendererRemoteFrameToken> {
        self.remote_frame_projections
            .borrow()
            .get(&id)
            .map(|projection| projection.token)
    }

    pub(crate) fn top_level_opener_endpoint(&self) -> Option<TopLevelWindowProxyEndpointId> {
        *self.top_level_target.opener_endpoint.borrow()
    }

    pub(crate) fn remote_top_level_target_snapshot(
        &self,
        endpoint: TopLevelWindowProxyEndpointId,
    ) -> Option<RendererRemoteTopLevelWindowProxyTarget> {
        let target = self
            .related_page_group
            .target_for_window_proxy_endpoint(endpoint)?;
        target
            .is_live()
            .then(|| RendererRemoteTopLevelWindowProxyTarget {
                residence: target.residence,
                endpoint: target.window_proxy_endpoint,
                channel: target.remote_window_proxy_channel.get(),
                opened_by_dom: target.opened_by_dom,
                active: target.active.get(),
                focused: target.focused.get(),
                current_url: target.current_url.borrow().clone(),
                current_serialized_origin: target.current_serialized_origin.borrow().clone(),
                current_opaque_origin_nonce: target.current_opaque_origin_nonce.get(),
                current_document_domain: target.current_document_domain.borrow().clone(),
                opener_endpoint: *target.opener_endpoint.borrow(),
            })
    }

    pub(crate) fn install_remote_top_level_window_proxy_projection(
        &self,
        endpoint: TopLevelWindowProxyEndpointId,
        window_proxy: v8::Global<v8::Object>,
        facade_context: v8::Global<v8::Context>,
    ) -> Result<()> {
        let target = self
            .related_page_group
            .target_for_window_proxy_endpoint(endpoint)
            .ok_or_else(|| anyhow!("remote WindowProxy endpoint is outside this Page group"))?;
        anyhow::ensure!(
            target.is_live(),
            "remote WindowProxy endpoint is no longer active"
        );
        anyhow::ensure!(
            self.current_agent_projection_for_target(&target).is_none(),
            "remote WindowProxy projection is already installed"
        );
        let projection = Rc::new(RendererRelatedPageTopLevelTargetProjection::default());
        projection
            .global_proxy
            .set(window_proxy)
            .map_err(|_| anyhow!("remote WindowProxy projection is already installed"))?;
        *projection.facade_context.borrow_mut() = Some(facade_context);
        target.register_projection(self.script_agent_id(), &projection);
        let previous = self
            .remote_top_level_projections
            .borrow_mut()
            .insert(endpoint, projection);
        debug_assert!(previous.is_none());
        Ok(())
    }

    pub(crate) fn related_page_projected_window_proxy<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        endpoint: TopLevelWindowProxyEndpointId,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let target = self
            .related_page_group
            .target_for_window_proxy_endpoint(endpoint)?;
        let projection = self.current_agent_projection_for_target(&target)?;
        Some(v8::Local::new(scope, projection.global_proxy.get()?))
    }

    /// Converts the current agent's LocalWindow projection into a live remote
    /// facade while preserving the logical target and group endpoint.
    pub(crate) fn mark_current_agent_top_level_projection_remote(&self) -> bool {
        if self.top_level_target.lifecycle.get() != RendererTopLevelBrowsingContextLifecycle::Active
        {
            return false;
        }
        let projection = self.current_agent_top_level_projection();
        projection.current_default_context.borrow_mut().take();
        let previous = self
            .top_level_target
            .parked_remote_projections
            .borrow_mut()
            .insert(self.script_agent_id, projection);
        debug_assert!(previous.is_none());
        true
    }

    pub(crate) fn retain_current_agent_top_level_facade_context(
        &self,
        context: v8::Global<v8::Context>,
    ) {
        *self
            .current_agent_top_level_projection()
            .facade_context
            .borrow_mut() = Some(context);
    }

    pub(crate) fn has_other_live_top_level_target_in_current_agent(&self) -> bool {
        self.related_page_group
            .live_targets_in_page_order()
            .into_iter()
            .filter(|target| !Rc::ptr_eq(target, &self.top_level_target))
            .any(|target| {
                target
                    .projection(self.script_agent_id())
                    .is_some_and(|projection| projection.current_default_context.borrow().is_some())
            })
    }

    pub(crate) fn should_switch_script_agent_for_navigation(&self, final_url: &url::Url) -> bool {
        if !self.has_other_live_top_level_target_in_current_agent()
            || !matches!(final_url.scheme(), "http" | "https")
        {
            return false;
        }
        let current_origin = self.top_level_target.current_serialized_origin.borrow();
        !current_origin.is_empty()
            && current_origin.as_str() != moli_url::origin_ascii_serialization(final_url)
    }

    pub(crate) fn replace_related_page_top_level_opener<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        residence: crate::RendererResolvedPopupTarget,
        opener: v8::Local<'s, v8::Object>,
    ) -> bool {
        let Some(target) = self
            .related_page_group
            .live_targets_in_page_order()
            .into_iter()
            .find(|target| target.residence == residence)
        else {
            return false;
        };
        let Some(projection) = self.current_agent_projection_for_target(&target) else {
            return false;
        };
        *target.opener_endpoint.borrow_mut() = Some(self.top_level_window_proxy_endpoint_id());
        let opener: v8::Local<'s, v8::Value> = opener.into();
        *projection.opener_edge.borrow_mut() = Some(v8::Global::new(scope, opener));
        true
    }

    pub(super) fn restore_main_window_name_after_navigation(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        window_proxy: v8::Local<'_, v8::Object>,
    ) {
        let name = self.top_level_target.name.borrow();
        let Some(name_value) = crate::util::v8_string(scope, name.as_str()) else {
            return;
        };
        let _ = window_proxy.define_own_property(
            scope,
            crate::util::v8_string(scope, crate::context_bootstrap::WINDOW_NAME_SLOT)
                .expect("static Window name slot should fit V8")
                .into(),
            name_value.into(),
            v8::PropertyAttribute::DONT_ENUM,
        );
    }

    pub(super) fn capture_main_window_opener_for_navigation<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        window_proxy: v8::Local<'s, v8::Object>,
    ) {
        // Once bound, the Page edge is authoritative. In particular, an
        // explicit `window.opener = null` must not be reconnected from a stale
        // realm-private slot during the next Document replacement.
        let projection = self.current_agent_top_level_projection();
        if projection.opener_edge.borrow().is_some() {
            return;
        }
        *projection.opener_edge.borrow_mut() =
            get_private_value(scope, window_proxy, WINDOW_OPENER_SLOT)
                .map(|opener| v8::Global::new(scope, opener));
    }

    pub(crate) fn set_top_level_opener_edge<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        opener: v8::Local<'s, v8::Value>,
    ) {
        if opener.is_null() {
            *self.top_level_target.opener_endpoint.borrow_mut() = None;
        }
        let projection = self.current_agent_top_level_projection();
        *projection.opener_edge.borrow_mut() = Some(v8::Global::new(scope, opener));
    }

    pub(crate) fn sever_top_level_opener_edge(&self, scope: &mut v8::PinScope<'_, '_>) {
        let opener: v8::Local<'_, v8::Value> = v8::null(scope).into();
        self.set_top_level_opener_edge(scope, opener);
    }

    pub(crate) fn top_level_opener_value<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        let projection = self.current_agent_top_level_projection();
        let opener = {
            let edge = projection.opener_edge.borrow();
            edge.as_ref().map(|opener| v8::Local::new(scope, opener))?
        };
        if let Ok(opener_window) = v8::Local::<v8::Object>::try_from(opener)
            && crate::native_bridge::top_level_window_proxy_is_finally_closed(scope, opener_window)
        {
            // Blink clears the opener edge when the opener browsing context is
            // discarded. Lazily collapsing the edge here also handles a Page
            // that outlives its opener without retaining the opener host.
            self.sever_top_level_opener_edge(scope);
            return Some(v8::null(scope).into());
        }
        Some(opener)
    }

    pub(super) fn restore_main_window_opener_after_navigation<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        window_proxy: v8::Local<'s, v8::Object>,
    ) {
        let Some(opener) = self.top_level_opener_value(scope) else {
            return;
        };
        set_private_value(scope, window_proxy, WINDOW_OPENER_SLOT, opener);
    }
}

pub(crate) struct ScriptVmDefaultWorldBootstrap {
    pub(super) resource_owner_id: ResourceOwnerId,
    pub(super) promise_reject_dispatch: PromiseRejectDispatchSlot,
    pub(super) page_inspector: DocumentInspectorBinding,
    pub(super) renderer_document_isolate: RendererDocumentIsolateHandle,
    pub(super) renderer_document_isolate_teardown: RendererDocumentIsolateTeardown,
    pub(super) renderer_page_script_environment: Option<RendererPageScriptEnvironment>,
    pub(super) script_agent_page_membership: Option<RendererScriptAgentPageMembership>,
    pub(super) page_default_context: v8::Global<v8::Context>,
    pub(super) bridge_ref: JsContextHostBridgeRef,
    pub(super) runtime_observable_context_token: RuntimeObservableContextToken,
    pub(super) baseline_globals: super::ScriptGlobalsBaseline,
    pub(super) root_frame_id: Option<String>,
    pub(super) prebootstrapped_child_default_contexts: SharedPrebootstrappedChildDefaultContexts,
    // `JsContextHost` stores a non-owning pointer into `document_runtime`.
    // Keep every realm/bridge owner before the host and the host before the
    // runtime so cancellation of a staged preinspector bootstrap is safe.
    pub(super) context_host: Rc<RefCell<JsContextHost>>,
    pub(super) document_runtime: Box<DocumentRuntime>,
    pub(super) page_context_cancel_tx: RendererPageContextCancelSender,
    pub(super) post_domcontentloaded_page_task_tx: PageTaskSender,
    pub(super) page_runtime_wake_tx: PageRuntimeWakeSender,
    pub(super) storage_bucket_store: crate::context_bootstrap::SharedStorageBucketStore,
}

/// A fully bootstrapped main Page realm whose Inspector default-context
/// registration has deliberately not happened yet.
///
/// The V8 Context, stable WindowProxy, native bridge, and Document host are
/// already live at this boundary. Keeping Inspector attachment as a distinct
/// materialization step mirrors child-frame prebootstrap and is what makes it
/// possible to create an auxiliary realm synchronously from an opener callback
/// without re-entering the shared document isolate.
pub(crate) struct ScriptVmPreinspectorDefaultWorldBootstrap {
    pub(super) inner: ScriptVmDefaultWorldBootstrap,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct RendererDocumentIsolateTeardown {
    unregister_platform_on_context_teardown: bool,
    #[cfg(test)]
    requires_deferred_lifo_drop: bool,
}

impl RendererDocumentIsolateTeardown {
    fn owner_reserved_page() -> Self {
        #[cfg(test)]
        {
            Self {
                unregister_platform_on_context_teardown: false,
                requires_deferred_lifo_drop: false,
            }
        }
        #[cfg(not(test))]
        {
            Self {
                unregister_platform_on_context_teardown: false,
            }
        }
    }

    #[cfg(test)]
    fn standalone_test() -> Self {
        Self {
            unregister_platform_on_context_teardown: true,
            requires_deferred_lifo_drop: true,
        }
    }

    pub(super) fn unregister_platform_on_context_teardown(
        self,
        renderer_document_isolate: &RendererDocumentIsolateHandle,
    ) {
        if self.unregister_platform_on_context_teardown {
            renderer_document_isolate.unregister_renderer_document_isolate_platform();
        }
    }

    pub(super) fn requires_deferred_lifo_script_vm_drop(self) -> bool {
        #[cfg(test)]
        {
            self.requires_deferred_lifo_drop
        }
        #[cfg(not(test))]
        {
            false
        }
    }
}

#[derive(Clone)]
pub(crate) struct RendererDocumentIsolateHandle {
    inner: Rc<RefCell<RendererDocumentIsolateHolder>>,
    // Keep the release queue on the re-entrant-safe handle as well as in the
    // holder. Related Page construction can bootstrap a new realm while the
    // source isolate holder is already mutably borrowed and entered by the
    // window.open() callback. Looking the queue up through `inner` in that
    // path would recursively borrow the holder and abort inside V8.
    deferred_context_host_releases: RendererDeferredContextHostReleaseQueue,
}

impl std::fmt::Debug for RendererDocumentIsolateHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererDocumentIsolateHandle")
            .finish_non_exhaustive()
    }
}

impl RendererDocumentIsolateHandle {
    pub(crate) fn deferred_context_host_release_queue(
        &self,
    ) -> RendererDeferredContextHostReleaseQueue {
        self.deferred_context_host_releases.clone()
    }

    #[cfg(test)]
    pub(crate) fn new_standalone_without_owner_reservation_for_test(
        v8_foreground_task_sender: RendererPageV8ForegroundTaskSender,
    ) -> Result<RendererDocumentIsolateBootstrap> {
        Self::new_with_page_route(
            v8_foreground_task_sender,
            RendererDocumentIsolateTeardown::standalone_test(),
        )
    }

    pub(crate) fn new_owner_reserved_page(
        v8_foreground_task_sender: RendererPageV8ForegroundTaskSender,
    ) -> Result<RendererDocumentIsolateBootstrap> {
        Self::new_with_page_route(
            v8_foreground_task_sender,
            RendererDocumentIsolateTeardown::owner_reserved_page(),
        )
    }

    fn new_with_page_route(
        v8_foreground_task_sender: RendererPageV8ForegroundTaskSender,
        renderer_document_isolate_teardown: RendererDocumentIsolateTeardown,
    ) -> Result<RendererDocumentIsolateBootstrap> {
        let (renderer_document_isolate, bridge_bindings, script_agent_page_membership) =
            RendererDocumentIsolateHolder::new_holder(v8_foreground_task_sender)?;
        let deferred_context_host_releases = renderer_document_isolate
            .deferred_context_host_releases
            .clone();
        let renderer_document_isolate = Self {
            inner: Rc::new(RefCell::new(renderer_document_isolate)),
            deferred_context_host_releases,
        };
        let isolate_backend = renderer_document_isolate.inspector_isolate_backend_handle();
        Ok(RendererDocumentIsolateBootstrap {
            renderer_document_isolate,
            bridge_bindings,
            renderer_document_isolate_teardown,
            inspector_isolate_backend: isolate_backend.clone(),
            page_inspector: DocumentInspectorBinding::new(isolate_backend),
            script_agent_page_membership: Some(script_agent_page_membership),
            renderer_page_script_environment: None,
            reuse_main_window_proxy: false,
        })
    }

    pub(crate) fn identity_key(&self) -> usize {
        Rc::as_ptr(&self.inner) as usize
    }

    pub(crate) fn script_agent_id(&self) -> ScriptAgentId {
        self.inner.borrow().script_agent_id
    }

    pub(crate) fn script_agent_scope(&self) -> crate::browsing_context_model::ScriptAgentScope {
        self.inner.borrow().script_agent_foreground_router.scope()
    }

    pub(crate) fn script_agent_page_count(&self) -> usize {
        self.inner
            .borrow()
            .script_agent_foreground_router
            .page_count()
    }

    pub(crate) fn inspector_isolate_backend_handle(&self) -> RendererInspectorIsolateBackendHandle {
        self.inner
            .borrow()
            .inspector_backend
            .as_ref()
            .expect("document isolate Inspector backend missing before ScriptVm drop")
            .handle()
    }

    fn build_bridge_bindings(&self) -> Result<NativeBridgeBindings> {
        let mut holder = self.inner.borrow_mut();
        let RendererDocumentIsolateHolder {
            isolate, bootstrap, ..
        } = &mut *holder;
        let isolate_ptr = unsafe { isolate.as_raw_isolate_ptr() };
        with_entered_owned_isolate(isolate, |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let global_template = bootstrap.global_template(scope);
            let cross_origin_window_global_template =
                bootstrap.cross_origin_window_global_template(scope);
            Ok(NativeBridgeBindings::build(
                scope,
                isolate_ptr,
                global_template,
                cross_origin_window_global_template,
            ))
        })
    }

    pub(super) fn with_renderer_document_isolate_and_inspector_mut<T>(
        &self,
        op: impl FnOnce(&mut v8::OwnedIsolate, &mut RendererInspectorIsolateBackend) -> T,
    ) -> T {
        let mut holder = self.inner.borrow_mut();
        let deferred_releases = holder.deferred_context_host_releases.clone();
        let RendererDocumentIsolateHolder {
            isolate,
            inspector_backend,
            ..
        } = &mut *holder;
        let inspector_backend = inspector_backend
            .as_mut()
            .expect("document isolate Inspector backend missing before ScriptVm drop");
        with_entered_owned_isolate_value(isolate, |isolate| {
            let result = op(isolate, inspector_backend);
            deferred_releases.drain_on_entered_isolate();
            result
        })
    }

    pub(super) fn with_entered_renderer_document_isolate_and_inspector_mut<T>(
        &self,
        op: impl FnOnce(&mut v8::OwnedIsolate, &mut RendererInspectorIsolateBackend) -> Result<T>,
    ) -> Result<T> {
        let mut holder = self.inner.borrow_mut();
        let deferred_releases = holder.deferred_context_host_releases.clone();
        let RendererDocumentIsolateHolder {
            isolate,
            inspector_backend,
            ..
        } = &mut *holder;
        let inspector_backend = inspector_backend
            .as_mut()
            .ok_or_else(|| anyhow!("document isolate Inspector backend unavailable"))?;
        with_entered_owned_isolate(isolate, |isolate| {
            let result = op(isolate, inspector_backend);
            deferred_releases.drain_on_entered_isolate();
            result
        })
    }

    pub(super) fn with_renderer_document_isolate_mut<T>(
        &self,
        op: impl FnOnce(&mut v8::OwnedIsolate) -> T,
    ) -> T {
        let mut holder = self.inner.borrow_mut();
        let deferred_releases = holder.deferred_context_host_releases.clone();
        with_entered_owned_isolate_value(&mut holder.isolate, |isolate| {
            let result = op(isolate);
            deferred_releases.drain_on_entered_isolate();
            result
        })
    }

    pub(super) fn with_entered_renderer_document_isolate<T>(
        &self,
        op: impl FnOnce(&mut v8::OwnedIsolate) -> Result<T>,
    ) -> Result<T> {
        let mut holder = self.inner.borrow_mut();
        let deferred_releases = holder.deferred_context_host_releases.clone();
        with_entered_owned_isolate(&mut holder.isolate, |isolate| {
            let result = op(isolate);
            deferred_releases.drain_on_entered_isolate();
            result
        })
    }

    pub(super) fn with_entered_renderer_document_isolate_and_bootstrap<T>(
        &self,
        op: impl FnOnce(&mut v8::OwnedIsolate, &IsolateBootstrapCache) -> Result<T>,
    ) -> Result<T> {
        let mut holder = self.inner.borrow_mut();
        let deferred_releases = holder.deferred_context_host_releases.clone();
        let RendererDocumentIsolateHolder {
            isolate, bootstrap, ..
        } = &mut *holder;
        with_entered_owned_isolate(isolate, |isolate| {
            let result = op(isolate, &*bootstrap);
            deferred_releases.drain_on_entered_isolate();
            result
        })
    }

    pub(super) fn with_renderer_document_isolate_and_bootstrap_mut<T>(
        &self,
        op: impl FnOnce(&mut v8::OwnedIsolate, &IsolateBootstrapCache) -> T,
    ) -> T {
        let mut holder = self.inner.borrow_mut();
        let deferred_releases = holder.deferred_context_host_releases.clone();
        let RendererDocumentIsolateHolder {
            isolate, bootstrap, ..
        } = &mut *holder;
        with_entered_owned_isolate_value(isolate, |isolate| {
            let result = op(isolate, &*bootstrap);
            deferred_releases.drain_on_entered_isolate();
            result
        })
    }

    pub(super) fn unregister_renderer_document_isolate_platform(&self) {
        self.inner.borrow_mut()._platform_registration.unregister();
    }

    pub(super) fn renderer_document_isolate_inspector_default_context_registry_count(
        &self,
    ) -> usize {
        self.inner.borrow().inspector_backend.as_ref().map_or(
            0,
            RendererInspectorIsolateBackend::default_context_registry_count,
        )
    }
}

pub(super) struct RendererDocumentIsolateHolder {
    // Inspector backend/session teardown touches V8 objects, so it must drop before the
    // isolate. `ScriptVm::drop` normally performs explicit context destruction;
    // this field order is the final safety net for partial construction paths.
    inspector_backend: Option<RendererInspectorIsolateBackend>,
    script_agent_id: ScriptAgentId,
    script_agent_foreground_router: RendererScriptAgentV8ForegroundTaskRouter,
    bootstrap: IsolateBootstrapCache,
    _platform_registration: V8PlatformIsolateRegistration,
    isolate: v8::OwnedIsolate,
    deferred_context_host_releases: RendererDeferredContextHostReleaseQueue,
    // Declared after the isolate so destroyed/live accounting changes only
    // after `OwnedIsolate::drop` has completed disposal.
    _accounting: RendererDocumentIsolateAccountingGuard,
}

impl RendererDocumentIsolateHolder {
    fn new_holder(
        v8_foreground_task_sender: RendererPageV8ForegroundTaskSender,
    ) -> Result<(
        Self,
        NativeBridgeBindings,
        RendererScriptAgentPageMembership,
    )> {
        let timing_enabled = moli_trace::cdp_nav_timing_enabled();
        let total_start = timing_enabled.then(std::time::Instant::now);
        let script_agent_id = allocate_script_agent_id();
        let (script_agent_foreground_router, script_agent_page_membership) =
            RendererScriptAgentV8ForegroundTaskRouter::new(
                script_agent_id,
                v8_foreground_task_sender,
            );
        let foreground_wake =
            V8ForegroundTaskWake::script_agent(script_agent_foreground_router.clone());

        let isolate_new_start = timing_enabled.then(std::time::Instant::now);
        // Window agents must not block their event loop with Atomics.wait().
        // Blink configures its main-thread isolates the same way; dedicated
        // workers keep V8's default and may use the blocking operation.
        let mut isolate = v8::Isolate::new(v8::CreateParams::default().allow_atomics_wait(false));
        if timing_enabled {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "v8_isolate_new",
                elapsed_ms = isolate_new_start.unwrap().elapsed().as_secs_f64() * 1000.0,
                "v8::Isolate::new (cold, no snapshot)"
            );
        }

        // kExplicit: the owner loop manually checkpoints microtasks at
        // observable page/command boundaries.
        crate::context_bootstrap::install_agent_microtask_checkpoint_tasks(&mut isolate);
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        isolate.set_capture_stack_trace_for_uncaught_exceptions(true, 32);
        // V8 publishes ERROR messages for access-check exceptions before JavaScript gets a
        // chance to catch them. The script, callback, and promise owners already report values
        // that remain uncaught, so treating every ERROR-level listener message as uncaught
        // produces false process diagnostics for ordinary caught Web API exceptions.
        let non_exception_message_levels = v8::MessageErrorLevel::LOG
            | v8::MessageErrorLevel::DEBUG
            | v8::MessageErrorLevel::INFO
            | v8::MessageErrorLevel::WARNING;
        isolate.add_message_listener_with_error_level(
            v8_message_listener,
            non_exception_message_levels,
        );
        isolate.set_host_initialize_import_meta_object_callback(
            initialize_import_meta_object_callback,
        );
        isolate.set_host_import_module_dynamically_callback(dynamic_import_callback);
        isolate.set_host_import_module_with_phase_dynamically_callback(
            dynamic_import_with_phase_callback,
        );
        isolate.set_allow_wasm_code_generation_callback(
            super::security_policy::wasm_code_generation_check_callback,
        );
        isolate.set_modify_code_generation_from_strings_callback(
            super::security_policy::string_code_generation_check_callback,
        );
        if moli_trace::dom_binding_timing_enabled() {
            isolate.set_promise_hook(promise_trace_hook);
        }
        isolate.set_promise_reject_callback(promise_reject_callback);
        isolate.set_failed_access_check_callback_function(failed_access_check_callback);

        let platform_registration = V8PlatformIsolateRegistration::register(
            &mut isolate,
            foreground_wake.into_platform_wake(),
        );
        let isolate_ptr = unsafe { isolate.as_raw_isolate_ptr() };
        let isolate_bootstrap;
        let bridge_bindings;
        {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();

            let bootstrap_start = timing_enabled.then(std::time::Instant::now);
            isolate_bootstrap = IsolateBootstrapCache::build(scope)?;
            if timing_enabled {
                tracing::info!(
                    target: "moli_cdp_nav_timing",
                    stage = "isolate_bootstrap_cache_build",
                    elapsed_ms = bootstrap_start.unwrap().elapsed().as_secs_f64() * 1000.0,
                    "IsolateBootstrapCache::build (246 constructor specs + global template)"
                );
            }

            let bridge_start = timing_enabled.then(std::time::Instant::now);
            let global_template = isolate_bootstrap.global_template(scope);
            let cross_origin_window_global_template =
                isolate_bootstrap.cross_origin_window_global_template(scope);
            bridge_bindings = NativeBridgeBindings::build(
                scope,
                isolate_ptr,
                global_template,
                cross_origin_window_global_template,
            );
            if timing_enabled {
                tracing::info!(
                    target: "moli_cdp_nav_timing",
                    stage = "native_bridge_bindings_build",
                    elapsed_ms = bridge_start.unwrap().elapsed().as_secs_f64() * 1000.0,
                    "NativeBridgeBindings::build"
                );
            }
        }

        let inspector_start = timing_enabled.then(std::time::Instant::now);
        let inspector_backend = RendererInspectorIsolateBackend::new(&mut isolate);
        if timing_enabled {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "inspector_backend_new",
                elapsed_ms = inspector_start.unwrap().elapsed().as_secs_f64() * 1000.0,
                "RendererInspectorIsolateBackend::new"
            );
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "v8_isolate_init_total",
                elapsed_ms = total_start.unwrap().elapsed().as_secs_f64() * 1000.0,
                "V8 isolate initialization total (cold, no snapshot)"
            );
        }

        // `v8::Isolate::new` enters the isolate. Document isolates are owned
        // independently by PageVms and may be destroyed in any page order, so
        // no isolate may remain on V8's thread-local enter stack between
        // operations.
        unsafe {
            isolate.exit();
        }

        Ok((
            Self::new(
                script_agent_id,
                script_agent_foreground_router,
                inspector_backend,
                isolate_bootstrap,
                platform_registration,
                isolate,
            ),
            bridge_bindings,
            script_agent_page_membership,
        ))
    }

    pub(super) fn new(
        script_agent_id: ScriptAgentId,
        script_agent_foreground_router: RendererScriptAgentV8ForegroundTaskRouter,
        inspector_backend: RendererInspectorIsolateBackend,
        bootstrap: IsolateBootstrapCache,
        platform_registration: V8PlatformIsolateRegistration,
        isolate: v8::OwnedIsolate,
    ) -> Self {
        Self {
            inspector_backend: Some(inspector_backend),
            script_agent_id,
            script_agent_foreground_router,
            bootstrap,
            _platform_registration: platform_registration,
            isolate,
            deferred_context_host_releases: RendererDeferredContextHostReleaseQueue::default(),
            _accounting: RendererDocumentIsolateAccountingGuard::new(),
        }
    }
}

impl Drop for RendererDocumentIsolateHolder {
    fn drop(&mut self) {
        // Fields drop in declaration order after this method. Enter now so the
        // inspector and bootstrap globals are released in their owning
        // isolate, then the platform registration is canceled, and finally
        // `OwnedIsolate::drop` observes itself as current and disposes it.
        unsafe {
            self.isolate.enter();
        }
        self.deferred_context_host_releases.begin_isolate_shutdown();
    }
}

struct EnteredIsolateGuard(*mut v8::OwnedIsolate);

impl Drop for EnteredIsolateGuard {
    fn drop(&mut self) {
        unsafe {
            (*self.0).exit();
        }
    }
}

fn with_entered_owned_isolate<T>(
    isolate: &mut v8::OwnedIsolate,
    op: impl FnOnce(&mut v8::OwnedIsolate) -> Result<T>,
) -> Result<T> {
    unsafe {
        isolate.enter();
    }
    let _guard = EnteredIsolateGuard(isolate);
    op(isolate)
}

fn with_entered_owned_isolate_value<T>(
    isolate: &mut v8::OwnedIsolate,
    op: impl FnOnce(&mut v8::OwnedIsolate) -> T,
) -> T {
    unsafe {
        isolate.enter();
    }
    let _guard = EnteredIsolateGuard(isolate);
    op(isolate)
}

pub(super) struct IsolateBootstrapCache {
    pub(super) context_assets: ContextBootstrapAssets,
}

impl IsolateBootstrapCache {
    pub(super) fn build(scope: &mut v8::PinScope<'_, '_, ()>) -> Result<Self> {
        Ok(Self {
            context_assets: ContextBootstrapAssets::build(scope)?,
        })
    }

    pub(super) fn global_template<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> v8::Local<'s, v8::ObjectTemplate> {
        self.context_assets.global_template(scope)
    }

    pub(super) fn cross_origin_window_global_template<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> v8::Local<'s, v8::ObjectTemplate> {
        self.context_assets
            .cross_origin_window_global_template(scope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    struct ContextSlotDropCounter(Rc<Cell<usize>>);

    impl Drop for ContextSlotDropCounter {
        fn drop(&mut self) {
            self.0.set(self.0.get().saturating_add(1));
        }
    }

    #[test]
    fn context_annex_weak_handles_are_safe_during_isolate_teardown() {
        crate::ensure_v8_for_test();

        const ISOLATE_COUNT: usize = 4;
        const CONTEXTS_PER_ISOLATE: usize = 32;
        let dropped_slots = Rc::new(Cell::new(0));

        for _ in 0..ISOLATE_COUNT {
            let mut isolate = v8::Isolate::new(Default::default());
            let mut contexts = Vec::with_capacity(CONTEXTS_PER_ISOLATE);
            {
                let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
                let scope = &mut scope.init();
                for _ in 0..CONTEXTS_PER_ISOLATE {
                    let context = v8::Context::new(scope, Default::default());
                    let replaced = context
                        .set_slot(Rc::new(ContextSlotDropCounter(Rc::clone(&dropped_slots))));
                    assert!(replaced.is_none());
                    contexts.push(v8::Global::new(scope, context));
                }
            }

            // Leave ContextAnnex finalizers pending until OwnedIsolate teardown.
            drop(contexts);
            drop(isolate);
        }

        assert_eq!(dropped_slots.get(), ISOLATE_COUNT * CONTEXTS_PER_ISOLATE);
    }

    #[test]
    fn snapshot_creator_cleans_up_context_annex_before_creating_blob() {
        crate::ensure_v8_for_test();

        let dropped_slots = Rc::new(Cell::new(0));
        let mut snapshot_creator = v8::Isolate::snapshot_creator(None, None);
        {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut snapshot_creator));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            let replaced =
                context.set_slot(Rc::new(ContextSlotDropCounter(Rc::clone(&dropped_slots))));
            assert!(replaced.is_none());
            scope.set_default_context(context);
        }

        let startup_data = snapshot_creator
            .create_blob(v8::FunctionCodeHandling::Clear)
            .expect("snapshot creator should produce a blob");
        assert!(!startup_data.is_empty());
        assert_eq!(dropped_slots.get(), 1);
    }

    #[test]
    fn remote_frame_replication_snapshot_uses_strict_versioned_wire() {
        let page_id = crate::runtime::PageId::new_for_testing(31);
        let snapshot = RendererRemoteFrameSnapshot {
            revision: 7,
            token: RendererRemoteFrameToken {
                endpoint: TopLevelWindowProxyEndpointId::from_wire_parts(37, 41)
                    .expect("test endpoint"),
                root_document: crate::runtime::RendererDocumentLifecycleIdentity {
                    frame: crate::runtime::RendererFrameToken { page_id },
                    document: crate::runtime::RendererDocumentToken::new_for_testing(page_id, 43),
                    epoch: crate::runtime::RendererLifecycleEpoch(47),
                },
                browsing_context_id: BrowsingContextId::nested(53),
            },
            parent_browsing_context_id: None,
            name: "wire-child".to_owned(),
            current_url: "https://frame.test/current".to_owned(),
            serialized_origin: "https://frame.test".to_owned(),
            opaque_origin_nonce: None,
            document_domain: None,
            policy_container: crate::document_runtime::DocumentPolicyContainer::default(),
        };
        let encoded = RendererRemoteFrameWireSnapshot::encode(snapshot.clone())
            .expect("valid snapshot should encode");
        assert_eq!(
            encoded.decode().expect("valid snapshot should decode"),
            snapshot
        );

        let mut opaque = snapshot.clone();
        opaque.serialized_origin = "null".to_owned();
        opaque.opaque_origin_nonce = Some(moli_storage_key::OpaqueOriginNonce::new(61));
        let encoded_opaque = RendererRemoteFrameWireSnapshot::encode(opaque.clone())
            .expect("opaque snapshot with an exact nonce should encode");
        assert_eq!(
            encoded_opaque
                .decode()
                .expect("opaque snapshot should decode"),
            opaque
        );

        let mut missing_opaque_nonce: serde_json::Value =
            serde_json::from_slice(&encoded.bytes).expect("snapshot wire JSON");
        missing_opaque_nonce["serializedOrigin"] = serde_json::json!("null");
        let missing_opaque_nonce = RendererRemoteFrameWireSnapshot {
            bytes: Arc::from(
                serde_json::to_vec(&missing_opaque_nonce).expect("mutated opaque wire JSON"),
            ),
        };
        assert!(
            missing_opaque_nonce.decode().is_err(),
            "opaque remote frame ingress must require its exact nonce"
        );

        let mut tuple_with_opaque_nonce: serde_json::Value =
            serde_json::from_slice(&encoded.bytes).expect("snapshot wire JSON");
        tuple_with_opaque_nonce["opaqueOriginNonce"] = serde_json::json!(67);
        let tuple_with_opaque_nonce = RendererRemoteFrameWireSnapshot {
            bytes: Arc::from(
                serde_json::to_vec(&tuple_with_opaque_nonce)
                    .expect("mutated tuple-origin wire JSON"),
            ),
        };
        assert!(
            tuple_with_opaque_nonce.decode().is_err(),
            "tuple-origin remote frame ingress must reject an opaque nonce"
        );

        let mut zero_opaque_nonce: serde_json::Value =
            serde_json::from_slice(&encoded_opaque.bytes).expect("opaque snapshot wire JSON");
        zero_opaque_nonce["opaqueOriginNonce"] = serde_json::json!(0);
        let zero_opaque_nonce = RendererRemoteFrameWireSnapshot {
            bytes: Arc::from(
                serde_json::to_vec(&zero_opaque_nonce).expect("mutated opaque wire JSON"),
            ),
        };
        assert!(
            zero_opaque_nonce.decode().is_err(),
            "opaque remote frame ingress must reject a zero nonce"
        );

        let mut opaque_with_document_domain: serde_json::Value =
            serde_json::from_slice(&encoded_opaque.bytes).expect("opaque snapshot wire JSON");
        opaque_with_document_domain["documentDomain"] = serde_json::json!("frame.test");
        let opaque_with_document_domain = RendererRemoteFrameWireSnapshot {
            bytes: Arc::from(
                serde_json::to_vec(&opaque_with_document_domain)
                    .expect("mutated opaque document.domain wire JSON"),
            ),
        };
        assert!(
            opaque_with_document_domain.decode().is_err(),
            "opaque remote frame ingress must reject document.domain"
        );

        let mut value: serde_json::Value =
            serde_json::from_slice(&encoded.bytes).expect("snapshot wire JSON");
        value["version"] = serde_json::json!(REMOTE_FRAME_SNAPSHOT_WIRE_VERSION + 1);
        let unsupported = RendererRemoteFrameWireSnapshot {
            bytes: Arc::from(serde_json::to_vec(&value).expect("mutated wire JSON")),
        };
        assert!(unsupported.decode().is_err());

        value["version"] = serde_json::json!(REMOTE_FRAME_SNAPSHOT_WIRE_VERSION);
        value["rendererCapability"] = serde_json::json!("must-not-cross");
        let unknown = RendererRemoteFrameWireSnapshot {
            bytes: Arc::from(serde_json::to_vec(&value).expect("mutated wire JSON")),
        };
        assert!(unknown.decode().is_err());

        let mut first = snapshot.clone();
        let mut second = snapshot.clone();
        second.token.browsing_context_id = BrowsingContextId::nested(59);
        first.parent_browsing_context_id = Some(second.token.browsing_context_id);
        second.parent_browsing_context_id = Some(first.token.browsing_context_id);
        assert!(
            validate_remote_frame_tree(
                &[first, second],
                snapshot.token.endpoint,
                page_id,
                snapshot.revision,
            )
            .is_err(),
            "strict remote frame ingress must reject a parent cycle"
        );

        let mut oversized = snapshot.clone();
        oversized.name = "x".repeat(MAX_REMOTE_FRAME_SNAPSHOT_STRING_BYTES + 1);
        assert!(
            encode_remote_frame_tree_for_publication(
                vec![oversized],
                snapshot.token.endpoint,
                page_id,
                snapshot.revision + 1,
            )
            .is_err(),
            "web-controlled snapshot values that exceed the wire contract must fail closed"
        );
    }
}
