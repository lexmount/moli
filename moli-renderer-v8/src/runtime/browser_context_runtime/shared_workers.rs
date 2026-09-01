use crate::{
    RendererSyntheticResponseBody,
    shared_worker_runtime::{SharedWorkerLaunchParams, SharedWorkerRuntimeOwnerWakeSender},
    worker::{WorkerPendingFetchContinue, WorkerPendingXhrContinue},
};
use moli_shared_worker::{
    SharedWorkerClientId, SharedWorkerClientOwnerId, SharedWorkerDescriptor, SharedWorkerInstanceId,
};
use parking_lot::Mutex;

use super::RendererBrowserContextRuntime;
use crate::runtime::{
    RendererOwnerLocalHostId, RendererRuntimeInspectorMessage,
    RendererRuntimeInspectorResponseSender,
};

/// Defers the browser-context SharedWorker registry until the first actual
/// `connect_shared_worker` call. ID allocation and owner routing do not require
/// the registry.
pub(super) struct LazySharedWorkerRuntime {
    state: Mutex<LazySharedWorkerRuntimeState>,
    client_owner_id_allocator: crate::shared_worker_runtime::SharedWorkerClientOwnerIdAllocator,
    browser_context_runtime_id: crate::runtime::RendererBrowserContextRuntimeId,
    output_transport: crate::runtime::RendererOutputTransportSenderSlot,
}

enum LazySharedWorkerRuntimeState {
    Deferred {
        owner_wake_senders: Vec<SharedWorkerRuntimeOwnerWakeSender>,
        owner_local_host_id: Option<RendererOwnerLocalHostId>,
    },
    Live(crate::shared_worker_runtime::SharedWorkerRuntimeService),
}

impl std::fmt::Debug for LazySharedWorkerRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LazySharedWorkerRuntime")
            .field("initialized", &self.is_initialized())
            .finish()
    }
}

impl LazySharedWorkerRuntime {
    pub(super) fn new(
        browser_context_runtime_id: crate::runtime::RendererBrowserContextRuntimeId,
        output_transport: crate::runtime::RendererOutputTransportSenderSlot,
    ) -> Self {
        Self {
            state: Mutex::new(LazySharedWorkerRuntimeState::Deferred {
                owner_wake_senders: Vec::new(),
                owner_local_host_id: None,
            }),
            client_owner_id_allocator: Default::default(),
            browser_context_runtime_id,
            output_transport,
        }
    }

    pub(super) fn from_service(
        service: crate::shared_worker_runtime::SharedWorkerRuntimeService,
        browser_context_runtime_id: crate::runtime::RendererBrowserContextRuntimeId,
        output_transport: crate::runtime::RendererOutputTransportSenderSlot,
    ) -> Self {
        service
            .configure_target_output_streams(browser_context_runtime_id, output_transport.clone());
        Self {
            client_owner_id_allocator: service.client_owner_id_allocator(),
            state: Mutex::new(LazySharedWorkerRuntimeState::Live(service)),
            browser_context_runtime_id,
            output_transport,
        }
    }

    pub(super) fn get_or_init(&self) -> crate::shared_worker_runtime::SharedWorkerRuntimeService {
        let mut state = self.state.lock();
        if let LazySharedWorkerRuntimeState::Live(service) = &*state {
            return service.clone();
        }
        let LazySharedWorkerRuntimeState::Deferred {
            owner_wake_senders,
            owner_local_host_id,
        } = &mut *state
        else {
            unreachable!();
        };
        let owner_wake_senders = std::mem::take(owner_wake_senders);
        let owner_local_host_id = *owner_local_host_id;
        let service = crate::shared_worker_runtime::
            new_shared_worker_runtime_service_with_client_owner_id_allocator(
                self.client_owner_id_allocator.clone(),
            );
        service.configure_target_output_streams(
            self.browser_context_runtime_id,
            self.output_transport.clone(),
        );
        for sender in owner_wake_senders {
            service.add_owner_wake_sender(sender);
        }
        if let Some(owner_local_host_id) = owner_local_host_id {
            service.set_owner_local_host_id(owner_local_host_id);
        }
        *state = LazySharedWorkerRuntimeState::Live(service.clone());
        service
    }

    pub(super) fn get(&self) -> Option<crate::shared_worker_runtime::SharedWorkerRuntimeService> {
        let state = self.state.lock();
        let LazySharedWorkerRuntimeState::Live(service) = &*state else {
            return None;
        };
        Some(service.clone())
    }

    pub(super) fn is_initialized(&self) -> bool {
        matches!(*self.state.lock(), LazySharedWorkerRuntimeState::Live(_))
    }

    pub(super) fn allocate_client_owner_id(&self) -> SharedWorkerClientOwnerId {
        self.client_owner_id_allocator.allocate()
    }

    pub(super) fn add_owner_wake_sender(&self, sender: SharedWorkerRuntimeOwnerWakeSender) {
        let mut state = self.state.lock();
        match &mut *state {
            LazySharedWorkerRuntimeState::Deferred {
                owner_wake_senders, ..
            } => owner_wake_senders.push(sender),
            LazySharedWorkerRuntimeState::Live(service) => service.add_owner_wake_sender(sender),
        }
    }

    pub(super) fn set_owner_local_host_id(&self, owner_local_host_id: RendererOwnerLocalHostId) {
        let mut state = self.state.lock();
        match &mut *state {
            LazySharedWorkerRuntimeState::Deferred {
                owner_local_host_id: slot,
                ..
            } => *slot = Some(owner_local_host_id),
            LazySharedWorkerRuntimeState::Live(service) => {
                service.set_owner_local_host_id(owner_local_host_id)
            }
        }
    }
}

impl RendererBrowserContextRuntime {
    fn shared_worker_runtime_if_initialized(
        &self,
    ) -> Option<crate::shared_worker_runtime::SharedWorkerRuntimeService> {
        self.inner.shared_worker_runtime.get()
    }

    pub(crate) fn add_shared_worker_owner_wake_sender(
        &self,
        sender: SharedWorkerRuntimeOwnerWakeSender,
    ) {
        self.inner
            .shared_worker_runtime
            .add_owner_wake_sender(sender);
    }

    pub(crate) fn set_shared_worker_owner_local_host_id(
        &self,
        owner_local_host_id: RendererOwnerLocalHostId,
    ) {
        self.inner
            .shared_worker_runtime
            .set_owner_local_host_id(owner_local_host_id);
    }

    pub(crate) fn connect_shared_worker(
        &self,
        descriptor: SharedWorkerDescriptor,
        params: SharedWorkerLaunchParams,
    ) -> SharedWorkerClientId {
        self.inner
            .shared_worker_runtime
            .get_or_init()
            .connect(descriptor, params)
    }

    pub(crate) fn next_shared_worker_client_owner_id(&self) -> SharedWorkerClientOwnerId {
        self.inner.shared_worker_runtime.allocate_client_owner_id()
    }

    pub(crate) fn drain_shared_worker_service_lane(&self) -> usize {
        self.inner
            .shared_worker_runtime
            .get()
            .map_or(0, |runtime| runtime.drain_service_lane())
    }

    pub fn close_shared_worker_for_target_close(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> bool {
        self.shared_worker_runtime_if_initialized()
            .is_some_and(|runtime| runtime.close_instance_for_devtools_target_close(instance_id))
    }

    pub(crate) fn remove_shared_worker_client(&self, client_id: SharedWorkerClientId) {
        if let Some(runtime) = self.shared_worker_runtime_if_initialized() {
            runtime.remove_client(client_id);
        }
    }

    pub(crate) fn continue_shared_worker_fetch(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingFetchContinue,
    ) -> bool {
        self.shared_worker_runtime_if_initialized()
            .is_some_and(|runtime| runtime.continue_pending_fetch(instance_id, request))
    }

    pub(crate) fn continue_shared_worker_xhr(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingXhrContinue,
    ) -> bool {
        self.shared_worker_runtime_if_initialized()
            .is_some_and(|runtime| runtime.continue_pending_xhr(instance_id, request))
    }

    pub(crate) fn continue_shared_worker_csp_report(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingFetchContinue,
    ) -> bool {
        self.shared_worker_runtime_if_initialized()
            .is_some_and(|runtime| runtime.continue_pending_csp_report(instance_id, request))
    }

    pub(crate) fn continue_shared_worker_fetch_response(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingFetchContinue,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    ) -> bool {
        self.shared_worker_runtime_if_initialized()
            .is_some_and(|runtime| {
                runtime.continue_pending_fetch_response(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                )
            })
    }

    pub(crate) fn continue_shared_worker_xhr_response(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingXhrContinue,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    ) -> bool {
        self.shared_worker_runtime_if_initialized()
            .is_some_and(|runtime| {
                runtime.continue_pending_xhr_response(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                )
            })
    }

    pub(crate) fn fail_shared_worker_fetch(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        self.shared_worker_runtime_if_initialized()
            .is_some_and(|runtime| runtime.fail_pending_fetch(instance_id, request, error_text))
    }

    pub(crate) fn fail_shared_worker_xhr(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingXhrContinue,
        error_text: String,
    ) -> bool {
        self.shared_worker_runtime_if_initialized()
            .is_some_and(|runtime| runtime.fail_pending_xhr(instance_id, request, error_text))
    }

    pub(crate) fn fail_shared_worker_csp_report(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        self.shared_worker_runtime_if_initialized()
            .is_some_and(|runtime| {
                runtime.fail_pending_csp_report(instance_id, request, error_text)
            })
    }

    pub(crate) fn fail_shared_worker_fetch_auth(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        self.shared_worker_runtime_if_initialized()
            .is_some_and(|runtime| {
                runtime.fail_pending_fetch_auth(instance_id, request, error_text)
            })
    }

    pub(crate) fn fail_shared_worker_xhr_auth(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingXhrContinue,
        error_text: String,
    ) -> bool {
        self.shared_worker_runtime_if_initialized()
            .is_some_and(|runtime| runtime.fail_pending_xhr_auth(instance_id, request, error_text))
    }

    pub(crate) fn fail_shared_worker_fetch_response(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        self.shared_worker_runtime_if_initialized()
            .is_some_and(|runtime| {
                runtime.fail_pending_fetch_response(instance_id, request, error_text)
            })
    }

    pub(crate) fn fail_shared_worker_xhr_response(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingXhrContinue,
        error_text: String,
    ) -> bool {
        self.shared_worker_runtime_if_initialized()
            .is_some_and(|runtime| {
                runtime.fail_pending_xhr_response(instance_id, request, error_text)
            })
    }

    pub(crate) fn fulfill_shared_worker_fetch(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        self.shared_worker_runtime_if_initialized()
            .is_some_and(|runtime| {
                runtime.fulfill_pending_fetch(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                )
            })
    }

    pub(crate) fn fulfill_shared_worker_xhr(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingXhrContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        self.shared_worker_runtime_if_initialized()
            .is_some_and(|runtime| {
                runtime.fulfill_pending_xhr(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                )
            })
    }

    pub(crate) fn fulfill_shared_worker_csp_report(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        self.shared_worker_runtime_if_initialized()
            .is_some_and(|runtime| {
                runtime.fulfill_pending_csp_report(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                )
            })
    }

    pub(crate) fn fulfill_shared_worker_fetch_response(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        self.shared_worker_runtime_if_initialized()
            .is_some_and(|runtime| {
                runtime.fulfill_pending_fetch_response(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                )
            })
    }

    pub(crate) fn fulfill_shared_worker_xhr_response(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingXhrContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        self.shared_worker_runtime_if_initialized()
            .is_some_and(|runtime| {
                runtime.fulfill_pending_xhr_response(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                )
            })
    }

    pub async fn dispatch_shared_worker_runtime_protocol_message(
        &self,
        instance_id: SharedWorkerInstanceId,
        inspector_session_id: Option<String>,
        raw_json: String,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        let Some(runtime) = self.shared_worker_runtime_if_initialized() else {
            return Err("SharedWorkerRuntimeUnavailable".to_owned());
        };
        runtime
            .dispatch_runtime_protocol_message(instance_id, inspector_session_id, raw_json)
            .await
    }

    pub async fn dispatch_shared_worker_runtime_protocol_message_with_deferred_response(
        &self,
        instance_id: SharedWorkerInstanceId,
        inspector_session_id: Option<String>,
        raw_json: String,
        deferred_response: RendererRuntimeInspectorResponseSender,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        let Some(runtime) = self.shared_worker_runtime_if_initialized() else {
            return Err("SharedWorkerRuntimeUnavailable".to_owned());
        };
        runtime
            .dispatch_runtime_protocol_message_with_deferred_response(
                instance_id,
                inspector_session_id,
                raw_json,
                deferred_response,
            )
            .await
    }

    pub fn detach_shared_worker_runtime_inspector_session(
        &self,
        instance_id: SharedWorkerInstanceId,
        inspector_session_id: Option<String>,
    ) -> bool {
        self.shared_worker_runtime_if_initialized()
            .is_some_and(|runtime| {
                runtime.detach_runtime_inspector_session(instance_id, inspector_session_id)
            })
    }
}
