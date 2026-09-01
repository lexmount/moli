use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
};

use crate::planning::{
    PreparedScript, SharedScriptSourceLoadCompleter,
    external_script_source_load_outcome_from_result,
    load_prepared_script_source_outcome_with_document_character_set,
};
use crate::shared_worker_runtime::RendererSharedWorkerRuntimeDiagnostics;
use moli_fetch::{ResponseBody, ResponseHead};
use parking_lot::Mutex;
use serde_json::{Value, json};
use url::Url;

mod dedicated_workers;
mod service_worker_runtime;
mod service_workers;
mod shared_workers;

static NEXT_RENDERER_STORAGE_PARTITION_ID: AtomicU64 = AtomicU64::new(0);
static NEXT_RENDERER_BROWSER_CONTEXT_RUNTIME_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Default)]
pub(crate) struct RendererOutputTransportSenderSlot {
    sender: Arc<Mutex<Option<super::RendererOutputTransportSender>>>,
}

impl RendererOutputTransportSenderSlot {
    pub(crate) fn set(&self, sender: super::RendererOutputTransportSender) {
        let mut slot = self.sender.lock();
        if let Some(existing) = slot.as_ref() {
            assert!(
                existing.same_channel(&sender),
                "one BrowserContext renderer output stream cannot change protocol transport"
            );
            return;
        }
        *slot = Some(sender);
    }

    pub(crate) fn sender(&self) -> Option<super::RendererOutputTransportSender> {
        self.sender.lock().clone()
    }
}

pub(crate) use crate::service_worker_runtime::ServiceWorkerControlState;
pub use service_workers::{
    RendererReservedServiceWorkerClient, RendererServiceWorkerMainResourceFetch,
};

/// Browser-context/partition scoped renderer runtime state.
///
/// This is the Moli-side equivalent of the communication/runtime state
/// Chromium hangs from `StoragePartitionImpl`: page and worker VMs borrow this
/// handle so same-context MessagePort, BroadcastChannel, and SharedWorker
/// routing share one state owner instead of following individual pages.
#[derive(Clone, Debug)]
pub struct RendererBrowserContextRuntime {
    inner: Arc<RendererBrowserContextRuntimeInner>,
}

/// Thread-affine owner returned by constructors that create a network runtime.
///
/// Clones obtained through Deref are renderer handles only. The owner set is
/// kept outside `RendererBrowserContextRuntimeInner`, so its fetch JoinHandles
/// are not reachable through the cloneable renderer context graph.
#[derive(Debug)]
pub struct RendererBrowserContextRuntimeOwner {
    runtime: Option<RendererBrowserContextRuntime>,
    producer_registry: RendererProducerRegistry,
    resource_runtime_owner_root: Option<crate::network::BrowserResourceRuntimeOwnerRoot>,
}

/// Cloneable, root-bound access for a NavigationEngine that shares this
/// renderer browser context. The weak registrar and renderer handle are minted
/// together, so replacement cannot accidentally target another context's
/// current binding.
#[derive(Clone, Debug)]
pub struct RendererBrowserContextRuntimeOwnerAccess {
    runtime: RendererBrowserContextRuntime,
    producer_registrar: RendererProducerRegistrar,
    resource_runtime_registrar: crate::network::BrowserResourceRuntimeOwnerRegistrar,
}

#[derive(Debug)]
struct RendererProducerRegistry {
    inner: Rc<RefCell<RendererProducerRegistryState>>,
}

#[derive(Clone, Debug)]
struct RendererProducerRegistrar {
    inner: Weak<RefCell<RendererProducerRegistryState>>,
}

#[derive(Debug, Default)]
struct RendererProducerRegistryState {
    terminal: bool,
    producers: HashMap<u64, super::page::RendererProducerShutdownHandle>,
}

impl RendererProducerRegistry {
    fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(RendererProducerRegistryState::default())),
        }
    }

    fn registrar(&self) -> RendererProducerRegistrar {
        RendererProducerRegistrar {
            inner: Rc::downgrade(&self.inner),
        }
    }

    fn cancel_all(&self) {
        let producers = {
            let mut state = self.inner.borrow_mut();
            state.terminal = true;
            std::mem::take(&mut state.producers)
        };
        for producer in producers.into_values() {
            producer.cancel_page_producers();
        }
    }
}

impl RendererProducerRegistrar {
    fn register(&self, runtime: &super::JsRuntime) -> Result<(), &'static str> {
        let Some(registry) = self.inner.upgrade() else {
            return Err("renderer browser context owner has been dropped");
        };
        let mut state = registry.borrow_mut();
        if state.terminal {
            return Err("renderer browser context owner is shut down");
        }
        state.producers.retain(|_, producer| producer.is_live());
        let producer = runtime.producer_shutdown_handle();
        state
            .producers
            .insert(producer.renderer_owner_id(), producer);
        Ok(())
    }
}

/// Worker-thread view of browser-context runtime state.
///
/// Workers inherit owner-scoped communication registries, but they are not
/// SharedWorker requesters: the current browser-compatible surface only exposes
/// the SharedWorker constructor on Window.
#[derive(Clone, Debug)]
pub(crate) struct RendererWorkerContextRuntime {
    message_port_registry: crate::message_port_runtime::SharedMessagePortRegistry,
    broadcast_channel_registry: crate::broadcast_channel_runtime::SharedBroadcastChannelRegistry,
    storage_partition_identity: RendererStoragePartitionIdentity,
}

/// Process-local storage partition identity shared by pages and workers in one
/// browser-context runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RendererStoragePartitionIdentity {
    browser_context_id: String,
    profile_partition_id: String,
}

#[derive(Debug)]
struct RendererBrowserContextRuntimeInner {
    id: super::RendererBrowserContextRuntimeId,
    message_port_registry: crate::message_port_runtime::SharedMessagePortRegistry,
    broadcast_channel_registry: crate::broadcast_channel_runtime::SharedBroadcastChannelRegistry,
    browser_resource_runtime: crate::network::BrowserResourceRuntimeBinding,
    shared_worker_runtime: shared_workers::LazySharedWorkerRuntime,
    service_worker_runtime: service_worker_runtime::LazyServiceWorkerRuntime,
    storage_partition_identity: RendererStoragePartitionIdentity,
    next_web_storage_opaque_context_nonce: AtomicU64,
    next_child_document_loader_id: AtomicU64,
    next_detached_parser_script_fetch_id: AtomicU64,
    next_dedicated_worker_instance_id: AtomicU64,
    dedicated_worker_devtools_targets: Mutex<HashMap<u64, DedicatedWorkerDevToolsTarget>>,
    dedicated_worker_pause_on_start_for_devtools: AtomicBool,
    javascript_dialog_handler_enabled: AtomicBool,
    renderer_output_transport_tx: RendererOutputTransportSenderSlot,
}

#[derive(Clone, Debug)]
struct DedicatedWorkerDevToolsTarget {
    handle: crate::worker::WorkerDevToolsHandle,
    output_journal: Option<super::RendererTurnOutputJournal>,
}

#[derive(Debug)]
struct DetachedParserScriptFetchContinuationInner {
    script: PreparedScript,
    request_client: crate::network::ResourceRequestClient,
    task_runner: crate::network::RendererResourceTaskRunner,
    document_character_set: Option<String>,
    completer: SharedScriptSourceLoadCompleter,
}

#[derive(Clone, Debug)]
pub struct DetachedParserScriptFetchContinuation {
    inner: Arc<Mutex<Option<DetachedParserScriptFetchContinuationInner>>>,
}

impl PartialEq for DetachedParserScriptFetchContinuation {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl DetachedParserScriptFetchContinuation {
    fn new(
        script: PreparedScript,
        request_client: crate::network::ResourceRequestClient,
        task_runner: crate::network::RendererResourceTaskRunner,
        document_character_set: Option<String>,
        completer: SharedScriptSourceLoadCompleter,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Some(
                DetachedParserScriptFetchContinuationInner {
                    script,
                    request_client,
                    task_runner,
                    document_character_set,
                    completer,
                },
            ))),
        }
    }

    fn take(&self) -> Option<DetachedParserScriptFetchContinuationInner> {
        self.inner.lock().take()
    }

    pub fn fail(&self, error_text: String) -> bool {
        let Some(inner) = self.take() else {
            return false;
        };
        inner
            .completer
            .finish(external_script_source_load_outcome_from_result(
                &inner.script,
                Err(error_text),
                inner.document_character_set.as_deref(),
            ));
        true
    }

    pub fn fulfill(
        &self,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: Vec<u8>,
    ) -> bool {
        let Some(inner) = self.take() else {
            return false;
        };
        let text = String::from_utf8_lossy(&response_body).into_owned();
        let response = crate::protocol_types::NavigationResponse::from_head_and_materialized_body(
            ResponseHead {
                final_url: inner.script.url.clone(),
                status: response_code,
                headers: response_headers,
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                negotiated_http_version: None,
            },
            ResponseBody::materialized_text(text, response_body),
        );
        inner
            .completer
            .finish(external_script_source_load_outcome_from_result(
                &inner.script,
                Ok(response),
                inner.document_character_set.as_deref(),
            ));
        true
    }

    pub fn continue_request(&self, url: Option<Url>) -> bool {
        let Some(mut inner) = self.take() else {
            return false;
        };
        if let Some(url) = url {
            inner.script.url = url;
        }
        let task_runner = inner.task_runner.clone();
        task_runner.spawn(async move {
            let outcome = load_prepared_script_source_outcome_with_document_character_set(
                &inner.script,
                &inner.request_client,
                inner.document_character_set.as_deref(),
                Some(moli_fetch::RequestResourceType::ParserBlockingScript),
            )
            .await;
            inner.completer.finish(outcome);
        });
        true
    }
}

impl Drop for RendererBrowserContextRuntimeInner {
    fn drop(&mut self) {
        terminate_browser_context_resource_producers(self);
    }
}

fn terminate_browser_context_resource_producers(inner: &RendererBrowserContextRuntimeInner) {
    let dedicated_worker_targets =
        std::mem::take(&mut *inner.dedicated_worker_devtools_targets.lock());
    for target in dedicated_worker_targets.into_values() {
        let _ = target.handle.terminate_for_devtools();
    }
    if let Some(shared_worker_runtime) = inner.shared_worker_runtime.get() {
        shared_worker_runtime.terminate_all_for_context_shutdown();
    }
    if let Some(service_worker_runtime) = inner.service_worker_runtime.get() {
        service_worker_runtime.terminate_all_for_context_shutdown();
    }
}

impl Default for RendererBrowserContextRuntimeOwner {
    fn default() -> Self {
        RendererBrowserContextRuntime::new()
    }
}

impl RendererBrowserContextRuntime {
    // A browser-context runtime is only valid while its thread-affine owner is
    // retained, so construction returns that owner rather than a bare handle.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> RendererBrowserContextRuntimeOwner {
        Self::new_with_service_worker_resource_store(
            crate::new_shared_service_worker_resource_store(),
        )
    }

    pub fn new_with_service_worker_resource_store(
        service_worker_resource_store: crate::SharedServiceWorkerResourceStore,
    ) -> RendererBrowserContextRuntimeOwner {
        let browser_resource_runtime_owner = crate::network::BrowserResourceRuntimeOwner::new(
            &moli_fetch::FetchConfig::default(),
            moli_cookie_jar::new_shared_browser_cookie_store(),
        );
        Self::new_owned_with_service_worker_resource_store_and_browser_resource_runtime(
            service_worker_resource_store,
            browser_resource_runtime_owner,
        )
    }

    pub fn new_owned_with_service_worker_resource_store_and_browser_resource_runtime(
        service_worker_resource_store: crate::SharedServiceWorkerResourceStore,
        browser_resource_runtime_owner: crate::network::BrowserResourceRuntimeOwnerRegistration,
    ) -> RendererBrowserContextRuntimeOwner {
        let (resource_runtime_owner_root, browser_resource_runtime_binding) =
            crate::network::BrowserResourceRuntimeOwnerRoot::new(browser_resource_runtime_owner);
        let runtime =
            Self::new_with_service_worker_resource_store_and_browser_resource_runtime_binding(
                service_worker_resource_store,
                browser_resource_runtime_binding,
            );
        RendererBrowserContextRuntimeOwner {
            runtime: Some(runtime),
            producer_registry: RendererProducerRegistry::new(),
            resource_runtime_owner_root: Some(resource_runtime_owner_root),
        }
    }

    fn new_with_service_worker_resource_store_and_browser_resource_runtime_binding(
        service_worker_resource_store: crate::SharedServiceWorkerResourceStore,
        browser_resource_runtime: crate::network::BrowserResourceRuntimeBinding,
    ) -> Self {
        let message_port_registry = crate::message_port_runtime::new_message_port_registry();
        let broadcast_channel_registry =
            crate::broadcast_channel_runtime::new_broadcast_channel_registry();
        Self::from_parts(
            message_port_registry,
            broadcast_channel_registry,
            None,
            service_worker_resource_store,
            browser_resource_runtime,
        )
    }

    #[cfg(test)]
    pub(crate) fn new_with_registries_for_test(
        message_port_registry: crate::message_port_runtime::SharedMessagePortRegistry,
        broadcast_channel_registry: crate::broadcast_channel_runtime::SharedBroadcastChannelRegistry,
    ) -> RendererBrowserContextRuntimeOwner {
        let browser_resource_runtime_owner = crate::network::BrowserResourceRuntimeOwner::new(
            &moli_fetch::FetchConfig::default(),
            moli_cookie_jar::new_shared_browser_cookie_store(),
        );
        let (resource_runtime_owner_root, browser_resource_runtime) =
            crate::network::BrowserResourceRuntimeOwnerRoot::new(browser_resource_runtime_owner);
        let runtime = Self::from_parts(
            message_port_registry,
            broadcast_channel_registry,
            None,
            crate::new_shared_service_worker_resource_store(),
            browser_resource_runtime,
        );
        RendererBrowserContextRuntimeOwner {
            runtime: Some(runtime),
            producer_registry: RendererProducerRegistry::new(),
            resource_runtime_owner_root: Some(resource_runtime_owner_root),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_parts_for_test(
        message_port_registry: crate::message_port_runtime::SharedMessagePortRegistry,
        broadcast_channel_registry: crate::broadcast_channel_runtime::SharedBroadcastChannelRegistry,
        shared_worker_runtime: crate::shared_worker_runtime::SharedWorkerRuntimeService,
    ) -> RendererBrowserContextRuntimeOwner {
        let browser_resource_runtime_owner = crate::network::BrowserResourceRuntimeOwner::new(
            &moli_fetch::FetchConfig::default(),
            moli_cookie_jar::new_shared_browser_cookie_store(),
        );
        let (resource_runtime_owner_root, browser_resource_runtime) =
            crate::network::BrowserResourceRuntimeOwnerRoot::new(browser_resource_runtime_owner);
        let runtime = Self::from_parts(
            message_port_registry,
            broadcast_channel_registry,
            Some(shared_worker_runtime),
            crate::new_shared_service_worker_resource_store(),
            browser_resource_runtime,
        );
        RendererBrowserContextRuntimeOwner {
            runtime: Some(runtime),
            producer_registry: RendererProducerRegistry::new(),
            resource_runtime_owner_root: Some(resource_runtime_owner_root),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_worker_context_and_service_worker_store_for_test(
        restored_worker_context_runtime: RendererWorkerContextRuntime,
        service_worker_resource_store: crate::SharedServiceWorkerResourceStore,
    ) -> RendererBrowserContextRuntimeOwner {
        let browser_resource_runtime_owner = crate::network::BrowserResourceRuntimeOwner::new(
            &moli_fetch::FetchConfig::default(),
            moli_cookie_jar::new_shared_browser_cookie_store(),
        );
        let (resource_runtime_owner_root, browser_resource_runtime) =
            crate::network::BrowserResourceRuntimeOwnerRoot::new(browser_resource_runtime_owner);
        let runtime = Self::from_parts_with_worker_context_runtime(
            restored_worker_context_runtime,
            None,
            service_worker_resource_store,
            browser_resource_runtime,
        );
        RendererBrowserContextRuntimeOwner {
            runtime: Some(runtime),
            producer_registry: RendererProducerRegistry::new(),
            resource_runtime_owner_root: Some(resource_runtime_owner_root),
        }
    }

    fn from_parts(
        message_port_registry: crate::message_port_runtime::SharedMessagePortRegistry,
        broadcast_channel_registry: crate::broadcast_channel_runtime::SharedBroadcastChannelRegistry,
        shared_worker_runtime: Option<crate::shared_worker_runtime::SharedWorkerRuntimeService>,
        service_worker_resource_store: crate::SharedServiceWorkerResourceStore,
        browser_resource_runtime: crate::network::BrowserResourceRuntimeBinding,
    ) -> Self {
        let storage_partition_identity = RendererStoragePartitionIdentity::new_process_local();
        let service_worker_context_runtime = RendererWorkerContextRuntime::with_identity(
            message_port_registry.clone(),
            broadcast_channel_registry.clone(),
            storage_partition_identity.clone(),
        );
        Self::from_parts_with_worker_context_runtime(
            service_worker_context_runtime,
            shared_worker_runtime,
            service_worker_resource_store,
            browser_resource_runtime,
        )
    }

    fn from_parts_with_worker_context_runtime(
        service_worker_context_runtime: RendererWorkerContextRuntime,
        shared_worker_runtime: Option<crate::shared_worker_runtime::SharedWorkerRuntimeService>,
        service_worker_resource_store: crate::SharedServiceWorkerResourceStore,
        browser_resource_runtime: crate::network::BrowserResourceRuntimeBinding,
    ) -> Self {
        let message_port_registry = service_worker_context_runtime.message_port_registry();
        let broadcast_channel_registry =
            service_worker_context_runtime.broadcast_channel_registry();
        let storage_partition_identity =
            service_worker_context_runtime.storage_partition_identity();
        let id = super::RendererBrowserContextRuntimeId::new(
            NEXT_RENDERER_BROWSER_CONTEXT_RUNTIME_ID.fetch_add(1, Ordering::Relaxed),
        );
        let renderer_output_transport_tx = RendererOutputTransportSenderSlot::default();
        let shared_worker_runtime = match shared_worker_runtime {
            Some(service) => shared_workers::LazySharedWorkerRuntime::from_service(
                service,
                id,
                renderer_output_transport_tx.clone(),
            ),
            None => shared_workers::LazySharedWorkerRuntime::new(
                id,
                renderer_output_transport_tx.clone(),
            ),
        };
        let service_worker_runtime = service_worker_runtime::LazyServiceWorkerRuntime::new(
            service_worker_resource_store,
            service_worker_context_runtime,
            browser_resource_runtime.clone(),
            id,
            renderer_output_transport_tx.clone(),
        );
        Self {
            inner: Arc::new(RendererBrowserContextRuntimeInner {
                id,
                message_port_registry,
                broadcast_channel_registry,
                browser_resource_runtime: browser_resource_runtime.clone(),
                shared_worker_runtime,
                service_worker_runtime,
                storage_partition_identity,
                next_web_storage_opaque_context_nonce: AtomicU64::default(),
                next_child_document_loader_id: AtomicU64::default(),
                next_detached_parser_script_fetch_id: AtomicU64::default(),
                next_dedicated_worker_instance_id: AtomicU64::default(),
                dedicated_worker_devtools_targets: Mutex::new(HashMap::new()),
                dedicated_worker_pause_on_start_for_devtools: AtomicBool::new(false),
                javascript_dialog_handler_enabled: AtomicBool::new(false),
                renderer_output_transport_tx,
            }),
        }
    }

    pub fn browser_resource_runtime(&self) -> crate::network::BrowserResourceRuntime {
        self.inner.browser_resource_runtime.current()
    }

    /// Stops browser-context producers before the external network owner root
    /// broadcasts shutdown and joins fetch threads.
    pub fn terminate_resource_producers_for_owner_shutdown(&self) {
        terminate_browser_context_resource_producers(&self.inner);
    }

    pub fn shares_state_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    pub fn id(&self) -> super::RendererBrowserContextRuntimeId {
        self.inner.id
    }

    pub(crate) fn set_renderer_output_transport_sender(
        &self,
        sender: super::RendererOutputTransportSender,
    ) {
        self.inner.renderer_output_transport_tx.set(sender.clone());
        if let Some(shared_worker_runtime) = self.inner.shared_worker_runtime.get() {
            shared_worker_runtime.bind_target_output_transport(sender.clone());
        }
        if let Some(service_worker_runtime) = self.inner.service_worker_runtime.get() {
            service_worker_runtime.bind_target_output_transport(sender);
        }
    }

    pub(crate) fn renderer_output_transport_sender(
        &self,
    ) -> Option<super::RendererOutputTransportSender> {
        self.inner.renderer_output_transport_tx.sender()
    }

    pub(crate) fn storage_partition_identity(&self) -> RendererStoragePartitionIdentity {
        self.inner.storage_partition_identity.clone()
    }

    pub fn moli_memory_diagnostics(&self) -> Value {
        let shared_worker_runtime_initialized = self.inner.shared_worker_runtime.is_initialized();
        let mut shared_worker_diagnostics = self.inner.shared_worker_runtime.get().map_or_else(
            || {
                json!({
                    "matchingEntryCount": 0,
                    "loadingInstanceCount": 0,
                    "runningInstanceCount": 0,
                    "clientCount": 0,
                    "loadingHostCount": 0,
                    "runningWorkerIsolateCount": 0,
                    "pendingServiceLaneEventCount": 0,
                })
            },
            |runtime| runtime.moli_memory_diagnostics(),
        );
        shared_worker_diagnostics["runtimeInitialized"] = shared_worker_runtime_initialized.into();
        let service_worker_runtime_initialized = self.inner.service_worker_runtime.is_initialized();
        let deferred_service_worker_window_client_count = self
            .inner
            .service_worker_runtime
            .deferred_window_client_count();
        let service_worker_diagnostics = self
            .inner
            .service_worker_runtime
            .get()
            .map_or_else(Default::default, |runtime| runtime.diagnostics_snapshot());
        json!({
            "rendererOutputTransport": self
                .inner
                .renderer_output_transport_tx
                .sender()
                .map(|sender| sender.diagnostics()),
            "sharedWorker": shared_worker_diagnostics,
            "serviceWorker": {
                "runtimeInitialized": service_worker_runtime_initialized,
                "deferredWindowClients": deferred_service_worker_window_client_count,
                "registrations": service_worker_diagnostics.registration_count,
                "runtimeRegistrations": service_worker_diagnostics.registration_count,
                "versions": service_worker_diagnostics.version_count,
                "installingVersions": service_worker_diagnostics.installing_version_count,
                "activatedVersions": service_worker_diagnostics.activated_version_count,
                "redundantVersions": service_worker_diagnostics.redundant_version_count,
                "stoppedVersions": service_worker_diagnostics.stopped_version_count,
                "startingVersions": service_worker_diagnostics.starting_version_count,
                "runningVersions": service_worker_diagnostics.running_version_count,
                "stoppingVersions": service_worker_diagnostics.stopping_version_count,
                "runningWorkers": service_worker_diagnostics.running_host_count,
                "pendingUnregistrations": service_worker_diagnostics.pending_unregistration_count,
                "inFlightEvents": service_worker_diagnostics.in_flight_event_count,
                "liveClients": service_worker_diagnostics.live_client_count,
                "controlledClients": service_worker_diagnostics.controlled_client_count,
                "pendingServiceLaneEventCount": service_worker_diagnostics.pending_service_lane_event_count,
            },
        })
    }

    pub fn shared_worker_running_worker_isolate_count_for_diagnostics(&self) -> usize {
        self.shared_worker_runtime_diagnostics_for_diagnostics()
            .running_worker_isolate_count
    }

    pub fn shared_worker_runtime_diagnostics_for_diagnostics(
        &self,
    ) -> RendererSharedWorkerRuntimeDiagnostics {
        self.inner
            .shared_worker_runtime
            .get()
            .map_or_else(RendererSharedWorkerRuntimeDiagnostics::default, |runtime| {
                runtime.diagnostics_snapshot()
            })
    }

    pub(crate) fn message_port_registry(
        &self,
    ) -> crate::message_port_runtime::SharedMessagePortRegistry {
        self.inner.message_port_registry.clone()
    }

    pub(crate) fn broadcast_channel_registry(
        &self,
    ) -> crate::broadcast_channel_runtime::SharedBroadcastChannelRegistry {
        self.inner.broadcast_channel_registry.clone()
    }

    pub(crate) fn worker_context_runtime(&self) -> RendererWorkerContextRuntime {
        RendererWorkerContextRuntime {
            message_port_registry: self.message_port_registry(),
            broadcast_channel_registry: self.broadcast_channel_registry(),
            storage_partition_identity: self.storage_partition_identity(),
        }
    }

    pub(crate) fn service_worker_runtime(
        &self,
    ) -> crate::service_worker_runtime::ServiceWorkerRuntimeService {
        self.inner.service_worker_runtime.get_or_init()
    }

    pub(crate) fn service_worker_runtime_if_initialized(
        &self,
    ) -> Option<crate::service_worker_runtime::ServiceWorkerRuntimeService> {
        self.inner.service_worker_runtime.get()
    }

    pub(crate) fn ensure_service_worker_runtime_for_navigation(&self) -> bool {
        self.inner
            .service_worker_runtime
            .get_or_init_for_navigation()
            .is_some()
    }

    pub(crate) fn next_web_storage_opaque_context_nonce(
        &self,
    ) -> moli_storage_key::OpaqueOriginNonce {
        moli_storage_key::OpaqueOriginNonce::new(
            self.inner
                .next_web_storage_opaque_context_nonce
                .fetch_add(1, Ordering::Relaxed)
                .saturating_add(1),
        )
    }

    pub(crate) fn allocate_child_document_loader_id(&self) -> String {
        let next_id = self
            .inner
            .next_child_document_loader_id
            .fetch_add(1, Ordering::Relaxed)
            .checked_add(1)
            .expect("browser-context child Document loader id space exhausted");
        format!("LID-CHILD-{next_id:010}")
    }

    pub(crate) fn prepare_detached_parser_script_fetch(
        &self,
        mut info: crate::protocol_types::PendingSubresourceFetchInfo,
        script: PreparedScript,
        request_client: crate::network::ResourceRequestClient,
        task_runner: crate::network::RendererResourceTaskRunner,
        document_character_set: Option<String>,
        completer: SharedScriptSourceLoadCompleter,
    ) -> (
        crate::protocol_types::PendingSubresourceFetchInfo,
        DetachedParserScriptFetchContinuation,
    ) {
        info.internal_id = self
            .inner
            .next_detached_parser_script_fetch_id
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        (
            info,
            DetachedParserScriptFetchContinuation::new(
                script,
                request_client,
                task_runner,
                document_character_set,
                completer,
            ),
        )
    }

    pub fn set_javascript_dialog_handler_enabled(&self, enabled: bool) {
        self.inner
            .javascript_dialog_handler_enabled
            .store(enabled, Ordering::Relaxed);
    }

    pub fn javascript_dialog_handler_enabled(&self) -> bool {
        self.inner
            .javascript_dialog_handler_enabled
            .load(Ordering::Relaxed)
    }
}

impl RendererBrowserContextRuntimeOwner {
    pub fn handle(&self) -> RendererBrowserContextRuntime {
        self.runtime
            .as_ref()
            .expect("renderer browser context owner was already split")
            .clone()
    }

    pub fn owner_access(&self) -> RendererBrowserContextRuntimeOwnerAccess {
        RendererBrowserContextRuntimeOwnerAccess {
            runtime: self.handle(),
            producer_registrar: self.producer_registry.registrar(),
            resource_runtime_registrar: self
                .resource_runtime_owner_root
                .as_ref()
                .expect("renderer browser context owner was already shut down")
                .registrar(),
        }
    }

    pub fn replace_browser_resource_runtime(
        &self,
        registration: crate::network::BrowserResourceRuntimeOwnerRegistration,
    ) -> Result<crate::network::BrowserResourceRuntime, String> {
        self.owner_access()
            .replace_owned(registration)
            .map_err(str::to_owned)
    }

    /// Close renderer-producer admission and stop context-owned workers while
    /// leaving network owner roots available for an outer lifetime boundary to
    /// join after all of its `JsRuntime` handles have been dropped.
    pub fn terminate_renderer_producers_for_owner_shutdown(&mut self) {
        self.producer_registry.cancel_all();
        if let Some(runtime) = self.runtime.take() {
            runtime.terminate_resource_producers_for_owner_shutdown();
            drop(runtime);
        }
    }

    /// Broadcast shutdown and join every active or retired network owner.
    /// Callers with separate renderer handles first use
    /// [`Self::terminate_renderer_producers_for_owner_shutdown`] and drop those
    /// handles before entering this terminal network boundary.
    pub fn shutdown_network_and_join(&mut self) {
        self.terminate_renderer_producers_for_owner_shutdown();
        if let Some(owner_root) = self.resource_runtime_owner_root.take() {
            owner_root.shutdown_and_join();
            drop(owner_root);
        }
    }

    pub fn shutdown_and_join(&mut self) {
        self.terminate_renderer_producers_for_owner_shutdown();
        self.shutdown_network_and_join();
    }

    #[cfg(test)]
    fn registered_producer_count_for_testing(&self) -> usize {
        self.producer_registry.inner.borrow().producers.len()
    }
}

impl RendererBrowserContextRuntimeOwnerAccess {
    pub fn runtime(&self) -> RendererBrowserContextRuntime {
        self.runtime.clone()
    }

    pub fn register_renderer_producer(
        &self,
        runtime: &super::JsRuntime,
    ) -> Result<(), &'static str> {
        self.producer_registrar.register(runtime)
    }

    pub fn current_browser_resource_runtime(
        &self,
    ) -> Result<crate::network::BrowserResourceRuntime, &'static str> {
        self.resource_runtime_registrar.current_registered()
    }

    pub fn replace_owned(
        &self,
        registration: crate::network::BrowserResourceRuntimeOwnerRegistration,
    ) -> Result<crate::network::BrowserResourceRuntime, &'static str> {
        self.resource_runtime_registrar.replace_owned(registration)
    }

    pub fn adopt_registered(
        &self,
        runtime: crate::network::BrowserResourceRuntime,
    ) -> Result<(), &'static str> {
        self.resource_runtime_registrar.adopt_registered(runtime)
    }

    pub fn validate_registered(
        &self,
        runtime: &crate::network::BrowserResourceRuntime,
    ) -> Result<(), &'static str> {
        self.resource_runtime_registrar.validate_registered(runtime)
    }

    pub fn reap_retired_resource_runtimes(&self) {
        self.resource_runtime_registrar.reap_retired();
    }
}

impl Drop for RendererBrowserContextRuntimeOwner {
    fn drop(&mut self) {
        self.shutdown_and_join();
    }
}

impl std::ops::Deref for RendererBrowserContextRuntimeOwner {
    type Target = RendererBrowserContextRuntime;

    fn deref(&self) -> &Self::Target {
        self.runtime
            .as_ref()
            .expect("renderer browser context owner was already split")
    }
}

impl RendererStoragePartitionIdentity {
    fn new_process_local() -> Self {
        let id = NEXT_RENDERER_STORAGE_PARTITION_ID
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        Self {
            browser_context_id: format!("renderer-browser-context:{id}"),
            profile_partition_id: format!("renderer-profile-partition:{id}"),
        }
    }

    pub(crate) fn browser_context_id(&self) -> &str {
        &self.browser_context_id
    }

    pub(crate) fn profile_partition_id(&self) -> &str {
        &self.profile_partition_id
    }
}

impl RendererWorkerContextRuntime {
    pub(crate) fn new(
        message_port_registry: crate::message_port_runtime::SharedMessagePortRegistry,
        broadcast_channel_registry: crate::broadcast_channel_runtime::SharedBroadcastChannelRegistry,
    ) -> Self {
        Self::with_identity(
            message_port_registry,
            broadcast_channel_registry,
            RendererStoragePartitionIdentity::new_process_local(),
        )
    }

    fn with_identity(
        message_port_registry: crate::message_port_runtime::SharedMessagePortRegistry,
        broadcast_channel_registry: crate::broadcast_channel_runtime::SharedBroadcastChannelRegistry,
        storage_partition_identity: RendererStoragePartitionIdentity,
    ) -> Self {
        Self {
            message_port_registry,
            broadcast_channel_registry,
            storage_partition_identity,
        }
    }

    pub(crate) fn message_port_registry(
        &self,
    ) -> crate::message_port_runtime::SharedMessagePortRegistry {
        self.message_port_registry.clone()
    }

    pub(crate) fn broadcast_channel_registry(
        &self,
    ) -> crate::broadcast_channel_runtime::SharedBroadcastChannelRegistry {
        self.broadcast_channel_registry.clone()
    }

    pub(crate) fn storage_partition_identity(&self) -> RendererStoragePartitionIdentity {
        self.storage_partition_identity.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use moli_fetch::{FetchCancelHandle, Request};

    use super::RendererBrowserContextRuntime;
    use crate::{
        network::ResourceRequestClient,
        runtime::{
            JsRuntime, RendererOutputTransportMessage, RendererPageContextCancelReason,
            RendererPageReservationToken, renderer_output_transport_channel,
        },
    };

    async fn assert_single_page_reservation_release(
        output_rx: &mut crate::runtime::RendererOutputTransportReceiver,
        token: RendererPageReservationToken,
    ) {
        let message = tokio::time::timeout(Duration::from_secs(3), output_rx.recv())
            .await
            .expect("page reservation release should arrive")
            .expect("renderer output transport should remain live");
        assert!(matches!(
            message,
            RendererOutputTransportMessage::PageReservationReleased {
                owner_local_host_id,
                page_id,
            } if owner_local_host_id == token.local_host_id() && page_id == token.page_id()
        ));
        assert!(matches!(
            output_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn cloned_runtime_shares_partition_state() {
        let runtime = RendererBrowserContextRuntime::new();
        let clone = runtime.clone();

        assert!(runtime.shares_state_with(&clone));
    }

    #[test]
    fn worker_services_stay_deferred_until_first_use() {
        let runtime = RendererBrowserContextRuntime::new();

        assert_eq!(
            runtime.moli_memory_diagnostics()["sharedWorker"]["runtimeInitialized"],
            false
        );
        assert_eq!(
            runtime.moli_memory_diagnostics()["serviceWorker"]["runtimeInitialized"],
            false
        );

        let _ = runtime.next_shared_worker_client_owner_id();
        let document_url = url::Url::parse("https://deferred-client.test/").unwrap();
        let client_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        runtime.register_service_worker_client(
            document_url.clone(),
            moli_storage_key::MoliStorageKey::first_party_from_url(&document_url, None)
                .serialized_storage_key(),
            crate::service_worker_runtime::ServiceWorkerClientFrameType::TopLevel,
            Some(crate::native_bridge::WindowDocumentOwner::for_test(1)),
            client_queue.sender(),
        );
        let (shared_wake_tx, _shared_wake_rx) =
            crate::shared_worker_runtime::shared_worker_owner_wake_channel();
        runtime.add_shared_worker_owner_wake_sender(shared_wake_tx);
        let (service_wake_tx, _service_wake_rx) =
            crate::service_worker_runtime::service_worker_owner_wake_channel();
        runtime.add_service_worker_owner_wake_sender(service_wake_tx);
        runtime.set_service_worker_pause_on_start_for_devtools(true);

        assert!(!runtime.ensure_service_worker_runtime_for_navigation());
        assert_eq!(
            runtime.moli_memory_diagnostics()["sharedWorker"]["runtimeInitialized"],
            false
        );
        assert_eq!(
            runtime.moli_memory_diagnostics()["serviceWorker"]["runtimeInitialized"],
            false
        );
        assert_eq!(
            runtime.moli_memory_diagnostics()["serviceWorker"]["deferredWindowClients"],
            1
        );

        runtime.inner.shared_worker_runtime.get_or_init();
        runtime.service_worker_runtime();
        assert_eq!(
            runtime.moli_memory_diagnostics()["sharedWorker"]["runtimeInitialized"],
            true
        );
        assert_eq!(
            runtime.moli_memory_diagnostics()["serviceWorker"]["runtimeInitialized"],
            true
        );
        assert_eq!(
            runtime.moli_memory_diagnostics()["serviceWorker"]["deferredWindowClients"],
            0
        );
        assert_eq!(
            runtime
                .service_worker_runtime()
                .diagnostics_snapshot()
                .live_client_count,
            1
        );
        assert!(runtime.service_worker_pause_on_start_for_devtools());
    }

    #[test]
    fn cloned_runtime_shares_child_document_loader_id_sequence() {
        let runtime = RendererBrowserContextRuntime::new();
        let clone = runtime.clone();

        assert_eq!(
            runtime.allocate_child_document_loader_id(),
            "LID-CHILD-0000000001"
        );
        assert_eq!(
            clone.allocate_child_document_loader_id(),
            "LID-CHILD-0000000002"
        );
    }

    #[test]
    fn fresh_runtime_gets_isolated_partition_state() {
        let left = RendererBrowserContextRuntime::new();
        let right = RendererBrowserContextRuntime::new();

        assert!(!left.shares_state_with(&right));
    }

    #[test]
    fn worker_context_runtime_can_outlive_browser_context_without_runtime_service_edge() {
        let worker_runtime = {
            let runtime = RendererBrowserContextRuntime::new();
            runtime.worker_context_runtime()
        };

        let _ = worker_runtime.message_port_registry();
        let _ = worker_runtime.broadcast_channel_registry();
    }

    #[test]
    fn worker_context_runtime_inherits_storage_partition_identity() {
        let runtime = RendererBrowserContextRuntime::new();
        let worker_runtime = runtime.worker_context_runtime();

        assert_eq!(
            worker_runtime.storage_partition_identity(),
            runtime.storage_partition_identity()
        );
    }

    #[test]
    fn fresh_runtime_gets_isolated_storage_partition_identity() {
        let left = RendererBrowserContextRuntime::new();
        let right = RendererBrowserContextRuntime::new();

        assert_ne!(
            left.storage_partition_identity(),
            right.storage_partition_identity()
        );
    }

    #[test]
    fn producer_registry_reaps_dead_weak_entries_on_registration() {
        let owner = RendererBrowserContextRuntime::new();
        let access = owner.owner_access();
        let first = JsRuntime::initialize_with_browser_context_owner_access(&access)
            .expect("first producer should register");
        assert_eq!(owner.registered_producer_count_for_testing(), 1);
        drop(first);

        let second = JsRuntime::initialize_with_browser_context_owner_access(&access)
            .expect("second producer should register");
        assert_eq!(
            owner.registered_producer_count_for_testing(),
            1,
            "dead producer history must not accumulate for a long-lived context"
        );
        drop(second);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn terminal_context_rejects_queued_command_new_page_slot_and_network_submit() {
        let mut owner = RendererBrowserContextRuntime::new();
        let access = owner.owner_access();
        let runtime = JsRuntime::initialize_with_browser_context_owner_access(&access)
            .expect("producer should register");
        let stale_client = ResourceRequestClient::from_browser_resource_runtime(
            access
                .current_browser_resource_runtime()
                .expect("resource runtime should be live before shutdown"),
        );
        let (entered_rx, release_tx) = runtime.install_owner_command_dispatch_gate_for_testing();
        let reply_rx = runtime
            .enqueue_owner_command_probe_for_testing()
            .expect("probe should enqueue before shutdown");
        entered_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("renderer command should reach deterministic dispatch barrier");

        owner.shutdown_and_join();
        assert!(
            access.current_browser_resource_runtime().is_err(),
            "terminal weak owner access must reject stale runtime lookup"
        );
        let submit_error = stale_client
            .fetch_raw_stream_with_cancel(
                Request::get("http://127.0.0.1:9/post-shutdown")
                    .expect("test request should build"),
                FetchCancelHandle::new(),
            )
            .await
            .expect_err("terminal fetch owner must reject new submission");
        assert_eq!(
            submit_error.to_string(),
            "fetch runtime is shutting down",
            "terminal BrowserContext handles must reject at fetch admission"
        );

        release_tx
            .send(())
            .expect("release renderer command dispatch barrier");
        let command_result = tokio::time::timeout(Duration::from_secs(3), reply_rx)
            .await
            .expect("terminal command reply should not hang")
            .expect("renderer reply channel should remain explicit");
        let command_error = match command_result {
            Err(error) => error,
            Ok(_) => panic!("queued pre-terminal command must fail after terminal boundary"),
        };
        assert!(command_error.to_string().contains("dropped"));

        let (attach_result, cancel_rx) = runtime.try_attach_page_slot_for_testing();
        assert!(
            attach_result.is_err(),
            "terminal PageTable must reject attach"
        );
        assert_eq!(
            cancel_rx.reason(),
            Some(RendererPageContextCancelReason::ContextDropped)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn create_html_page_reservation_releases_once_when_context_is_already_terminal() {
        let mut owner = RendererBrowserContextRuntime::new();
        let runtime =
            JsRuntime::initialize_with_browser_context_owner_access(&owner.owner_access())
                .expect("producer should register");
        let loader_owner = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
            .expect("test loader should initialize");
        let (output_tx, mut output_rx) = renderer_output_transport_channel();
        runtime.set_renderer_output_transport_sender(output_tx);
        let token = runtime.reserve_page_for_creation();

        owner.shutdown_and_join();
        assert!(
            runtime
                .start_minimal_html_page_for_reservation_testing(token, &loader_owner)
                .is_err(),
            "terminal context must reject CreateHtmlPage"
        );
        assert_single_page_reservation_release(&mut output_rx, token).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn create_html_page_reservation_releases_once_when_render_admission_send_fails() {
        let mut owner = RendererBrowserContextRuntime::new();
        let runtime =
            JsRuntime::initialize_with_browser_context_owner_access(&owner.owner_access())
                .expect("producer should register");
        let loader_owner = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
            .expect("test loader should initialize");
        let (output_tx, mut output_rx) = renderer_output_transport_channel();
        runtime.set_renderer_output_transport_sender(output_tx);
        let token = runtime.reserve_page_for_creation();

        runtime.close_owner_command_admission_for_testing();
        assert!(
            runtime
                .start_minimal_html_page_for_reservation_testing(token, &loader_owner)
                .is_err(),
            "closed render admission must return the owned CreateHtmlPage envelope"
        );
        assert_single_page_reservation_release(&mut output_rx, token).await;
        owner.shutdown_and_join();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn queued_create_html_page_reservation_releases_once_during_terminal_drain() {
        let mut owner = RendererBrowserContextRuntime::new();
        let runtime =
            JsRuntime::initialize_with_browser_context_owner_access(&owner.owner_access())
                .expect("producer should register");
        let loader_owner = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
            .expect("test loader should initialize");
        let (output_tx, mut output_rx) = renderer_output_transport_channel();
        runtime.set_renderer_output_transport_sender(output_tx);
        let token = runtime.reserve_page_for_creation();
        let (entered_rx, release_tx) = runtime.install_owner_command_dispatch_gate_for_testing();
        let pending = runtime
            .start_minimal_html_page_for_reservation_testing(token, &loader_owner)
            .expect("CreateHtmlPage should enqueue before the terminal boundary");
        entered_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("CreateHtmlPage should reach the deterministic dispatch gate");

        // PendingHtmlPage intentionally remains live across root shutdown. It
        // may retain a terminal renderer handle, but cannot submit or revive
        // work after producer admission closes; network children are joined by
        // the root independently.
        owner.shutdown_and_join();
        release_tx
            .send(())
            .expect("release CreateHtmlPage terminal drain");
        assert!(
            pending.await_ready().await.is_err(),
            "queued CreateHtmlPage must complete with an explicit terminal error"
        );
        assert_single_page_reservation_release(&mut output_rx, token).await;
        let stale_token = runtime.reserve_page_for_creation();
        assert!(
            runtime
                .start_minimal_html_page_for_reservation_testing(stale_token, &loader_owner)
                .is_err(),
            "escaped terminal renderer handle must not admit new work"
        );
        assert_single_page_reservation_release(&mut output_rx, stale_token).await;
    }
}
