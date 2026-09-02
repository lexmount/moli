use std::collections::{HashMap, hash_map::Entry};

use parking_lot::Mutex;
use url::Url;

use crate::{
    page_task_queue::RendererPageServiceWorkerTaskSender,
    runtime::{RendererRuntimeInspectorMessage, RendererRuntimeInspectorResponseSender},
    service_worker_runtime::{
        ServiceWorkerClientFrameType, ServiceWorkerClientId, ServiceWorkerRegistrationId,
        ServiceWorkerRuntimeOwnerWakeSender, ServiceWorkerVersionId,
    },
    window_document_identity::WindowDocumentOwner,
};

use super::RendererBrowserContextRuntime;

/// Keeps the full Service Worker registry dormant while retaining the small
/// amount of browser-context state that must exist before first use.
pub(super) struct LazyServiceWorkerRuntime {
    state: Mutex<LazyServiceWorkerRuntimeState>,
    resource_store: crate::SharedServiceWorkerResourceStore,
    restored_worker_context_runtime: super::RendererWorkerContextRuntime,
    browser_resource_runtime: crate::network::BrowserResourceRuntimeBinding,
    client_id_allocator: crate::service_worker_runtime::ServiceWorkerClientIdAllocator,
    browser_context_runtime_id: crate::runtime::RendererBrowserContextRuntimeId,
    output_transport: crate::runtime::RendererOutputTransportSenderSlot,
}

enum LazyServiceWorkerRuntimeState {
    Deferred {
        owner_wake_senders: Vec<ServiceWorkerRuntimeOwnerWakeSender>,
        window_clients: HashMap<ServiceWorkerClientId, DeferredServiceWorkerWindowClient>,
        force_update_on_page_load: bool,
        pause_new_workers_on_start: bool,
        related_pause_on_start_policies: Vec<(u64, u64, String, String)>,
    },
    Live(crate::service_worker_runtime::ServiceWorkerRuntimeService),
}

struct DeferredServiceWorkerWindowClient {
    document_url: Url,
    storage_key: String,
    frame_type: ServiceWorkerClientFrameType,
    document_owner: Option<WindowDocumentOwner>,
    completion_tx: RendererPageServiceWorkerTaskSender,
}

impl std::fmt::Debug for LazyServiceWorkerRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LazyServiceWorkerRuntime")
            .field("initialized", &self.is_initialized())
            .finish()
    }
}

impl LazyServiceWorkerRuntime {
    pub(super) fn new(
        resource_store: crate::SharedServiceWorkerResourceStore,
        restored_worker_context_runtime: super::RendererWorkerContextRuntime,
        browser_resource_runtime: crate::network::BrowserResourceRuntimeBinding,
        browser_context_runtime_id: crate::runtime::RendererBrowserContextRuntimeId,
        output_transport: crate::runtime::RendererOutputTransportSenderSlot,
    ) -> Self {
        Self {
            state: Mutex::new(LazyServiceWorkerRuntimeState::Deferred {
                owner_wake_senders: Vec::new(),
                window_clients: HashMap::new(),
                force_update_on_page_load: false,
                pause_new_workers_on_start: false,
                related_pause_on_start_policies: Vec::new(),
            }),
            resource_store,
            restored_worker_context_runtime,
            browser_resource_runtime,
            client_id_allocator: Default::default(),
            browser_context_runtime_id,
            output_transport,
        }
    }

    pub(super) fn get_or_init(&self) -> crate::service_worker_runtime::ServiceWorkerRuntimeService {
        let mut state = self.state.lock();
        if let LazyServiceWorkerRuntimeState::Live(service) = &*state {
            return service.clone();
        }
        let LazyServiceWorkerRuntimeState::Deferred {
            owner_wake_senders,
            window_clients,
            force_update_on_page_load,
            pause_new_workers_on_start,
            related_pause_on_start_policies,
        } = &mut *state
        else {
            unreachable!();
        };
        let owner_wake_senders = std::mem::take(owner_wake_senders);
        let window_clients = std::mem::take(window_clients);
        let force_update_on_page_load = *force_update_on_page_load;
        let pause_new_workers_on_start = *pause_new_workers_on_start;
        let related_pause_on_start_policies = std::mem::take(related_pause_on_start_policies);
        let service = crate::service_worker_runtime::
            new_service_worker_runtime_service_with_resource_store_and_browser_resource_runtime_binding(
                self.resource_store.clone(),
                self.restored_worker_context_runtime.clone(),
                self.browser_resource_runtime.clone(),
                self.client_id_allocator.clone(),
                self.browser_context_runtime_id,
                self.output_transport.clone(),
            );
        for sender in owner_wake_senders {
            service.add_owner_wake_sender(sender);
        }
        service.set_force_update_on_page_load_for_devtools(force_update_on_page_load);
        service.set_pause_new_workers_on_start_for_devtools(pause_new_workers_on_start);
        service.set_related_pause_on_start_policies_for_devtools(related_pause_on_start_policies);
        for (client_id, client) in window_clients {
            let inserted = service.register_allocated_client_with_storage_key(
                client_id,
                client.document_url,
                client.storage_key,
                client.frame_type,
                client.document_owner,
                client.completion_tx,
            );
            debug_assert!(inserted, "deferred Service Worker client id must be unique");
        }
        *state = LazyServiceWorkerRuntimeState::Live(service.clone());
        service
    }

    pub(super) fn get(&self) -> Option<crate::service_worker_runtime::ServiceWorkerRuntimeService> {
        let state = self.state.lock();
        let LazyServiceWorkerRuntimeState::Live(service) = &*state else {
            return None;
        };
        Some(service.clone())
    }

    /// Returns a live runtime when one already exists or persisted
    /// registrations may control the navigation. An empty store stays lazy.
    pub(super) fn get_or_init_for_navigation(
        &self,
    ) -> Option<crate::service_worker_runtime::ServiceWorkerRuntimeService> {
        if let Some(service) = self.get() {
            return Some(service);
        }
        if self.resource_store.lock().is_empty() {
            return None;
        }
        Some(self.get_or_init())
    }

    pub(super) fn is_initialized(&self) -> bool {
        matches!(*self.state.lock(), LazyServiceWorkerRuntimeState::Live(_))
    }

    pub(super) fn allocate_client_id(
        &self,
    ) -> crate::service_worker_runtime::ServiceWorkerClientId {
        self.client_id_allocator.allocate()
    }

    pub(super) fn register_window_client(
        &self,
        client_id: ServiceWorkerClientId,
        document_url: Url,
        storage_key: String,
        frame_type: ServiceWorkerClientFrameType,
        document_owner: Option<WindowDocumentOwner>,
        completion_tx: RendererPageServiceWorkerTaskSender,
    ) -> bool {
        let service = {
            let mut state = self.state.lock();
            match &mut *state {
                LazyServiceWorkerRuntimeState::Deferred { window_clients, .. } => {
                    let client = DeferredServiceWorkerWindowClient {
                        document_url,
                        storage_key,
                        frame_type,
                        document_owner,
                        completion_tx,
                    };
                    return match window_clients.entry(client_id) {
                        Entry::Vacant(entry) => {
                            entry.insert(client);
                            true
                        }
                        Entry::Occupied(_) => false,
                    };
                }
                LazyServiceWorkerRuntimeState::Live(service) => service.clone(),
            }
        };
        service.register_allocated_client_with_storage_key(
            client_id,
            document_url,
            storage_key,
            frame_type,
            document_owner,
            completion_tx,
        )
    }

    pub(super) fn update_window_client(
        &self,
        client_id: ServiceWorkerClientId,
        document_url: Url,
        storage_key: String,
        frame_type: ServiceWorkerClientFrameType,
        document_owner: Option<WindowDocumentOwner>,
        completion_tx: Option<RendererPageServiceWorkerTaskSender>,
    ) -> bool {
        let service = {
            let mut state = self.state.lock();
            match &mut *state {
                LazyServiceWorkerRuntimeState::Deferred { window_clients, .. } => {
                    let Some(client) = window_clients.get_mut(&client_id) else {
                        return false;
                    };
                    client.document_url = document_url;
                    client.storage_key = storage_key;
                    client.frame_type = frame_type;
                    client.document_owner = document_owner;
                    if let Some(completion_tx) = completion_tx {
                        client.completion_tx = completion_tx;
                    }
                    return true;
                }
                LazyServiceWorkerRuntimeState::Live(service) => service.clone(),
            }
        };
        if let Some(completion_tx) = completion_tx {
            service.update_client_document_with_storage_key_and_completion_sender(
                client_id,
                document_url,
                storage_key,
                frame_type,
                document_owner,
                completion_tx,
            )
        } else {
            service.update_client_document_with_storage_key(
                client_id,
                document_url,
                storage_key,
                frame_type,
                document_owner,
            )
        }
    }

    pub(super) fn unregister_client(&self, client_id: ServiceWorkerClientId) {
        let service = {
            let mut state = self.state.lock();
            match &mut *state {
                LazyServiceWorkerRuntimeState::Deferred { window_clients, .. } => {
                    window_clients.remove(&client_id);
                    return;
                }
                LazyServiceWorkerRuntimeState::Live(service) => service.clone(),
            }
        };
        service.unregister_client(client_id);
    }

    pub(super) fn deferred_window_client_count(&self) -> usize {
        let state = self.state.lock();
        match &*state {
            LazyServiceWorkerRuntimeState::Deferred { window_clients, .. } => window_clients.len(),
            LazyServiceWorkerRuntimeState::Live(_) => 0,
        }
    }

    pub(super) fn add_owner_wake_sender(&self, sender: ServiceWorkerRuntimeOwnerWakeSender) {
        let mut state = self.state.lock();
        match &mut *state {
            LazyServiceWorkerRuntimeState::Deferred {
                owner_wake_senders, ..
            } => owner_wake_senders.push(sender),
            LazyServiceWorkerRuntimeState::Live(service) => service.add_owner_wake_sender(sender),
        }
    }

    pub(super) fn set_force_update_on_page_load(&self, force_update: bool) {
        let mut state = self.state.lock();
        match &mut *state {
            LazyServiceWorkerRuntimeState::Deferred {
                force_update_on_page_load,
                ..
            } => *force_update_on_page_load = force_update,
            LazyServiceWorkerRuntimeState::Live(service) => {
                service.set_force_update_on_page_load_for_devtools(force_update)
            }
        }
    }

    pub(super) fn force_update_on_page_load(&self) -> bool {
        let state = self.state.lock();
        match &*state {
            LazyServiceWorkerRuntimeState::Deferred {
                force_update_on_page_load,
                ..
            } => *force_update_on_page_load,
            LazyServiceWorkerRuntimeState::Live(service) => {
                service.force_update_on_page_load_for_devtools()
            }
        }
    }

    pub(super) fn set_pause_new_workers_on_start(&self, pause: bool) {
        let mut state = self.state.lock();
        match &mut *state {
            LazyServiceWorkerRuntimeState::Deferred {
                pause_new_workers_on_start,
                ..
            } => *pause_new_workers_on_start = pause,
            LazyServiceWorkerRuntimeState::Live(service) => {
                service.set_pause_new_workers_on_start_for_devtools(pause)
            }
        }
    }

    pub(super) fn pause_new_workers_on_start(&self) -> bool {
        let state = self.state.lock();
        match &*state {
            LazyServiceWorkerRuntimeState::Deferred {
                pause_new_workers_on_start,
                ..
            } => *pause_new_workers_on_start,
            LazyServiceWorkerRuntimeState::Live(service) => {
                service.pause_new_workers_on_start_for_devtools()
            }
        }
    }

    pub(super) fn set_related_pause_on_start_policies(
        &self,
        policies: Vec<(u64, u64, String, String)>,
    ) {
        let mut state = self.state.lock();
        match &mut *state {
            LazyServiceWorkerRuntimeState::Deferred {
                related_pause_on_start_policies,
                ..
            } => *related_pause_on_start_policies = policies,
            LazyServiceWorkerRuntimeState::Live(service) => {
                service.set_related_pause_on_start_policies_for_devtools(policies)
            }
        }
    }
}

impl RendererBrowserContextRuntime {
    fn service_worker_runtime_for_existing_registration(
        &self,
    ) -> Option<crate::service_worker_runtime::ServiceWorkerRuntimeService> {
        self.inner
            .service_worker_runtime
            .get_or_init_for_navigation()
    }

    pub(crate) fn add_service_worker_owner_wake_sender(
        &self,
        sender: ServiceWorkerRuntimeOwnerWakeSender,
    ) {
        self.inner
            .service_worker_runtime
            .add_owner_wake_sender(sender);
    }

    pub(crate) fn drain_service_worker_service_lane(&self) -> usize {
        self.inner
            .service_worker_runtime
            .get()
            .map_or(0, |runtime| runtime.drain_service_lane())
    }

    pub async fn dispatch_service_worker_runtime_protocol_message(
        &self,
        version_id: u64,
        inspector_session_id: Option<String>,
        raw_json: String,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        let Some(runtime) = self.service_worker_runtime_for_existing_registration() else {
            return Err("ServiceWorkerRuntimeUnavailable".to_owned());
        };
        runtime
            .dispatch_runtime_protocol_message(
                ServiceWorkerVersionId::from_u64_for_binding(version_id),
                inspector_session_id,
                raw_json,
            )
            .await
    }

    pub async fn dispatch_service_worker_runtime_protocol_message_with_deferred_response(
        &self,
        version_id: u64,
        inspector_session_id: Option<String>,
        raw_json: String,
        deferred_response: RendererRuntimeInspectorResponseSender,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        let Some(runtime) = self.service_worker_runtime_for_existing_registration() else {
            return Err("ServiceWorkerRuntimeUnavailable".to_owned());
        };
        runtime
            .dispatch_runtime_protocol_message_with_deferred_response(
                ServiceWorkerVersionId::from_u64_for_binding(version_id),
                inspector_session_id,
                raw_json,
                deferred_response,
            )
            .await
    }

    pub async fn dispatch_service_worker_runtime_protocol_message_with_devtools_session_response(
        &self,
        version_id: u64,
        inspector_session_id: String,
        raw_json: String,
        response: RendererRuntimeInspectorResponseSender,
    ) -> Result<crate::runtime::CompletedWorkerRuntimeInspectorCommandDispatch, String> {
        let Some(runtime) = self.service_worker_runtime_for_existing_registration() else {
            return Err("ServiceWorkerRuntimeUnavailable".to_owned());
        };
        runtime
            .dispatch_runtime_protocol_message_with_devtools_session_response(
                ServiceWorkerVersionId::from_u64_for_binding(version_id),
                inspector_session_id,
                raw_json,
                response,
            )
            .await
    }

    pub fn detach_service_worker_runtime_inspector_session(
        &self,
        version_id: u64,
        inspector_session_id: Option<String>,
    ) -> bool {
        self.service_worker_runtime_for_existing_registration()
            .is_some_and(|runtime| {
                runtime.detach_runtime_inspector_session(
                    ServiceWorkerVersionId::from_u64_for_binding(version_id),
                    inspector_session_id,
                )
            })
    }

    pub fn unregister_service_worker_scope_for_devtools(
        &self,
        scope_url: &url::Url,
    ) -> Result<bool, String> {
        self.service_worker_runtime_for_existing_registration()
            .map_or(Ok(false), |runtime| {
                runtime.devtools_unregister_scope(scope_url)
            })
    }

    pub fn start_service_worker_for_devtools(&self, scope_url: &url::Url) -> Result<bool, String> {
        self.service_worker_runtime_for_existing_registration()
            .map_or(Ok(false), |runtime| {
                runtime.devtools_start_worker_for_scope(scope_url)
            })
    }

    pub fn stop_service_worker_for_devtools(&self, version_id: u64) -> Result<bool, String> {
        self.service_worker_runtime_for_existing_registration()
            .map_or(Ok(false), |runtime| {
                runtime.devtools_stop_worker_version(ServiceWorkerVersionId::from_u64_for_binding(
                    version_id,
                ))
            })
    }

    pub fn stop_all_service_workers_for_devtools(&self) -> Result<usize, String> {
        self.service_worker_runtime_for_existing_registration()
            .map_or(Ok(0), |runtime| runtime.devtools_stop_all_workers())
    }

    pub fn skip_waiting_service_worker_for_devtools(
        &self,
        scope_url: &url::Url,
    ) -> Result<bool, String> {
        self.service_worker_runtime_for_existing_registration()
            .map_or(Ok(false), |runtime| {
                runtime.devtools_skip_waiting_for_scope(scope_url)
            })
    }

    pub fn update_service_worker_registration_for_devtools(
        &self,
        scope_url: &url::Url,
    ) -> Result<bool, String> {
        self.service_worker_runtime_for_existing_registration()
            .map_or(Ok(false), |runtime| {
                runtime.devtools_update_registration_for_scope(scope_url, self.clone())
            })
    }

    pub fn set_service_worker_force_update_on_page_load_for_devtools(&self, force_update: bool) {
        self.inner
            .service_worker_runtime
            .set_force_update_on_page_load(force_update);
    }

    pub fn service_worker_force_update_on_page_load_for_devtools(&self) -> bool {
        self.inner
            .service_worker_runtime
            .force_update_on_page_load()
    }

    pub fn controlled_service_worker_window_client_urls_for_devtools(
        &self,
        registration_id: u64,
        version_id: u64,
    ) -> Vec<String> {
        self.service_worker_runtime_for_existing_registration()
            .map_or_else(Vec::new, |runtime| {
                runtime.controlled_window_client_urls_for_version_for_devtools(
                    ServiceWorkerRegistrationId::from_u64_for_binding(registration_id),
                    ServiceWorkerVersionId::from_u64_for_binding(version_id),
                )
            })
    }

    pub fn controlled_service_worker_window_client_ids_for_devtools(
        &self,
        registration_id: u64,
        version_id: u64,
    ) -> Vec<u64> {
        self.service_worker_runtime_for_existing_registration()
            .map_or_else(Vec::new, |runtime| {
                runtime.controlled_window_client_ids_for_version_for_devtools(
                    ServiceWorkerRegistrationId::from_u64_for_binding(registration_id),
                    ServiceWorkerVersionId::from_u64_for_binding(version_id),
                )
            })
    }

    pub fn set_service_worker_pause_on_start_for_devtools(&self, pause: bool) {
        self.inner
            .service_worker_runtime
            .set_pause_new_workers_on_start(pause);
    }

    pub fn set_service_worker_related_pause_on_start_policies_for_devtools(
        &self,
        policies: Vec<(u64, u64, String, String)>,
    ) {
        self.inner
            .service_worker_runtime
            .set_related_pause_on_start_policies(policies);
    }

    pub fn set_service_worker_pause_on_start_for_version_for_devtools(
        &self,
        version_id: u64,
        pause: bool,
    ) -> bool {
        self.service_worker_runtime_for_existing_registration()
            .is_some_and(|runtime| {
                runtime.set_pause_on_start_for_version_for_devtools(
                    ServiceWorkerVersionId::from_u64_for_binding(version_id),
                    pause,
                )
            })
    }

    pub fn service_worker_pause_on_start_for_devtools(&self) -> bool {
        self.inner
            .service_worker_runtime
            .pause_new_workers_on_start()
    }

    pub fn set_service_worker_devtools_attached(&self, version_id: u64, attached: bool) {
        if let Some(runtime) = self.service_worker_runtime_for_existing_registration() {
            runtime.set_devtools_attached_for_version(
                ServiceWorkerVersionId::from_u64_for_binding(version_id),
                attached,
            );
        }
    }

    pub fn run_service_worker_if_waiting_for_debugger_for_devtools(&self, version_id: u64) -> bool {
        self.service_worker_runtime_for_existing_registration()
            .is_some_and(|runtime| {
                runtime.devtools_run_if_waiting_for_debugger(
                    ServiceWorkerVersionId::from_u64_for_binding(version_id),
                )
            })
    }

    pub fn release_all_service_workers_waiting_for_debugger_for_devtools(&self) -> usize {
        self.service_worker_runtime_for_existing_registration()
            .map_or(0, |runtime| {
                runtime.devtools_release_all_workers_waiting_for_debugger()
            })
    }

    pub fn deliver_push_message_for_devtools(
        &self,
        origin: &url::Url,
        registration_id: u64,
        data: Option<Vec<u8>>,
    ) -> Result<bool, String> {
        self.service_worker_runtime_for_existing_registration()
            .map_or(Ok(false), |runtime| {
                runtime.devtools_deliver_push_message(
                    origin,
                    ServiceWorkerRegistrationId::from_u64_for_binding(registration_id),
                    data,
                )
            })
    }

    pub fn dispatch_sync_event_for_devtools(
        &self,
        origin: &url::Url,
        registration_id: u64,
        tag: String,
        last_chance: bool,
    ) -> Result<bool, String> {
        self.service_worker_runtime_for_existing_registration()
            .map_or(Ok(false), |runtime| {
                runtime.devtools_dispatch_sync_event(
                    origin,
                    ServiceWorkerRegistrationId::from_u64_for_binding(registration_id),
                    tag,
                    last_chance,
                )
            })
    }

    pub fn dispatch_periodic_sync_event_for_devtools(
        &self,
        origin: &url::Url,
        registration_id: u64,
        tag: String,
    ) -> Result<bool, String> {
        self.service_worker_runtime_for_existing_registration()
            .map_or(Ok(false), |runtime| {
                runtime.devtools_dispatch_periodic_sync_event(
                    origin,
                    ServiceWorkerRegistrationId::from_u64_for_binding(registration_id),
                    tag,
                )
            })
    }
}
