use moli_shared_worker::SharedWorkerClientId;

use crate::worker::WorkerParentErrorEventKind;

use super::{client::RendererSharedWorkerClient, host::RendererSharedWorkerHost};

impl RendererSharedWorkerHost {
    pub(super) fn add_client(
        &self,
        client_id: SharedWorkerClientId,
        client: RendererSharedWorkerClient,
    ) {
        self.clients.lock().insert(client_id, client);
    }

    fn remove_client(&self, client_id: SharedWorkerClientId) -> Option<RendererSharedWorkerClient> {
        self.clients.lock().remove(&client_id)
    }

    pub(super) fn remove_client_endpoint(&self, client_id: SharedWorkerClientId) {
        self.remove_client(client_id);
    }

    pub(super) fn has_client_endpoint(&self, client_id: SharedWorkerClientId) -> bool {
        self.clients.lock().contains_key(&client_id)
    }

    #[cfg(test)]
    pub(super) fn client_endpoint_count(&self) -> usize {
        self.clients.lock().len()
    }

    pub(super) fn worker_host_bridge_sender(
        &self,
    ) -> Option<crate::page_task_queue::RendererWorkerHostBridgeEventSender> {
        // Keep the oldest live client's policy source stable. HashMap order
        // must not switch an intercepted request to another Document.
        self.clients
            .lock()
            .iter()
            .min_by_key(|(id, _)| *id)
            .map(|(_, client)| client.worker_host_bridge_sender())
    }

    fn fail_client(
        &self,
        client_id: SharedWorkerClientId,
        message: impl Into<String>,
        filename: impl Into<String>,
        event_kind: WorkerParentErrorEventKind,
    ) -> bool {
        self.fail_client_with_location(client_id, message, filename, 0, 0, event_kind)
    }

    fn fail_client_with_location(
        &self,
        client_id: SharedWorkerClientId,
        message: impl Into<String>,
        filename: impl Into<String>,
        lineno: u32,
        colno: u32,
        event_kind: WorkerParentErrorEventKind,
    ) -> bool {
        let Some(client) = self.remove_client(client_id) else {
            return false;
        };
        client.send_error_with_location(message, filename, lineno, colno, event_kind);
        client.close_ports();
        true
    }

    pub(super) fn fail_clients(
        &self,
        client_ids: Vec<SharedWorkerClientId>,
        message: impl Into<String>,
        filename: impl Into<String>,
        event_kind: WorkerParentErrorEventKind,
    ) {
        let message = message.into();
        let filename = filename.into();
        for client_id in client_ids {
            self.fail_client(client_id, message.clone(), filename.clone(), event_kind);
        }
    }

    pub(super) fn fail_clients_with_location(
        &self,
        client_ids: Vec<SharedWorkerClientId>,
        message: impl Into<String>,
        filename: impl Into<String>,
        lineno: u32,
        colno: u32,
        event_kind: WorkerParentErrorEventKind,
    ) {
        let message = message.into();
        let filename = filename.into();
        for client_id in client_ids {
            self.fail_client_with_location(
                client_id,
                message.clone(),
                filename.clone(),
                lineno,
                colno,
                event_kind,
            );
        }
    }

    pub(super) fn notify_all_clients_error_with_location(
        &self,
        message: impl Into<String>,
        filename: impl Into<String>,
        lineno: u32,
        colno: u32,
        event_kind: WorkerParentErrorEventKind,
    ) {
        let message = message.into();
        let filename = filename.into();
        let clients = self.clients.lock().values().cloned().collect::<Vec<_>>();
        for client in clients {
            client.send_nonterminal_error_with_location(
                message.clone(),
                filename.clone(),
                lineno,
                colno,
                event_kind,
            );
        }
    }

    pub(super) fn add_and_connect_client(
        &self,
        client_id: SharedWorkerClientId,
        client: RendererSharedWorkerClient,
        failure_filename: &str,
    ) -> bool {
        self.add_client(client_id, client);
        if self.connect_client(client_id) {
            return true;
        }
        self.fail_client(
            client_id,
            "Failed to connect SharedWorker: worker runtime is unavailable.",
            failure_filename,
            WorkerParentErrorEventKind::ErrorEvent,
        );
        false
    }

    pub(super) fn connect_pending_clients(
        &self,
        client_ids: Vec<SharedWorkerClientId>,
    ) -> Vec<SharedWorkerClientId> {
        let mut failed = Vec::new();
        for client_id in client_ids {
            if !self.connect_client(client_id) {
                self.fail_client(
                    client_id,
                    "Failed to connect SharedWorker: worker runtime is unavailable.",
                    "",
                    WorkerParentErrorEventKind::ErrorEvent,
                );
                failed.push(client_id);
            }
        }
        failed
    }

    fn close_worker_port_and_send_closed(&self, client_id: SharedWorkerClientId) -> bool {
        let Some(client) = self.remove_client(client_id) else {
            return false;
        };
        client.close_worker_port();
        client.send_closed();
        true
    }

    pub(super) fn close_worker_ports_and_send_closed(&self, client_ids: Vec<SharedWorkerClientId>) {
        for client_id in client_ids {
            self.close_worker_port_and_send_closed(client_id);
        }
    }

    pub(super) fn close_all_worker_ports_and_send_closed(&self) {
        let client_ids = self.clients.lock().keys().copied().collect();
        self.close_worker_ports_and_send_closed(client_ids);
    }

    fn connect_client(&self, client_id: SharedWorkerClientId) -> bool {
        let Some(worker_port_id) = self
            .clients
            .lock()
            .get(&client_id)
            .map(RendererSharedWorkerClient::worker_port_id)
        else {
            return false;
        };
        self.connect(worker_port_id)
    }
}
