use super::*;
use crate::shared_worker_runtime::{
    AppliedSharedWorkerClientErrorTarget, SharedWorkerClientEndpointDisposition,
    SharedWorkerClientEndpointReceiver,
};
use moli_shared_worker::SharedWorkerClientOwnerId;

impl JsContextHost {
    pub(crate) fn browser_context_runtime(&self) -> crate::runtime::RendererBrowserContextRuntime {
        self.browser_context_runtime.clone()
    }

    pub(crate) fn shared_worker_client_owner_id(&self) -> SharedWorkerClientOwnerId {
        self.shared_worker_client_owner_id
    }

    pub(crate) fn shared_worker_client_owner_id_for_child_context(
        &mut self,
        handle: DomHandle,
    ) -> SharedWorkerClientOwnerId {
        if let Some(owner_id) = self.child_shared_worker_client_owner_ids.get(&handle) {
            return *owner_id;
        }
        let owner_id = self
            .browser_context_runtime
            .next_shared_worker_client_owner_id();
        self.child_shared_worker_client_owner_ids
            .insert(handle, owner_id);
        owner_id
    }

    pub(crate) fn register_shared_worker_client(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        receiver: SharedWorkerClientEndpointReceiver,
        worker: v8::Local<'_, v8::Object>,
    ) {
        self.shared_worker_clients.insert(scope, receiver, worker);
    }

    pub(crate) fn current_shared_worker_client_event_identity(
        &self,
        client_id: moli_shared_worker::SharedWorkerClientId,
    ) -> Option<super::WindowExecutionContextIdentity> {
        let identity = self
            .shared_worker_clients
            .execution_context_identity(client_id)?;
        self.window_execution_context_identity_is_current(identity)
            .then_some(identity)
    }

    pub(crate) fn apply_authorized_shared_worker_client_close(
        &mut self,
        client_id: moli_shared_worker::SharedWorkerClientId,
        execution_context: super::WindowExecutionContextIdentity,
    ) {
        self.shared_worker_clients
            .apply_authorized_close(client_id, execution_context);
    }

    pub(crate) fn apply_authorized_shared_worker_client_error<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        client_id: moli_shared_worker::SharedWorkerClientId,
        execution_context: super::WindowExecutionContextIdentity,
        endpoint_disposition: SharedWorkerClientEndpointDisposition,
    ) -> AppliedSharedWorkerClientErrorTarget<'s> {
        self.shared_worker_clients.apply_authorized_error(
            scope,
            client_id,
            execution_context,
            endpoint_disposition,
        )
    }

    #[cfg(test)]
    pub(crate) fn shared_worker_client_count_for_test(&self) -> usize {
        self.shared_worker_clients.len()
    }

    #[cfg(test)]
    pub(crate) fn shared_worker_client_identities_for_test(
        &self,
    ) -> Vec<(
        moli_shared_worker::SharedWorkerClientId,
        super::WindowExecutionContextIdentity,
    )> {
        self.shared_worker_clients.identities_for_test()
    }

    pub(crate) fn close_shared_worker_clients(&mut self) {
        self.shared_worker_clients
            .disconnect_all_for_context_teardown();
        self.child_shared_worker_client_owner_ids.clear();
    }

    pub(crate) fn disconnect_shared_worker_clients_for_child_context(
        &mut self,
        handle: DomHandle,
    ) -> usize {
        self.child_shared_worker_client_owner_ids.remove(&handle);
        self.shared_worker_clients
            .disconnect_all_for_child_context(handle)
    }

    pub(crate) fn disconnect_shared_worker_clients_for_execution_context_owner(
        &mut self,
        owner: super::WindowExecutionContextOwner,
    ) -> usize {
        self.shared_worker_clients
            .disconnect_all_for_execution_context_owner(owner)
    }

    pub(crate) fn disconnect_shared_worker_clients_for_context_token(
        &mut self,
        context_token: super::RuntimeObservableContextToken,
    ) -> usize {
        self.shared_worker_clients
            .disconnect_all_for_context_token(context_token)
    }
}
