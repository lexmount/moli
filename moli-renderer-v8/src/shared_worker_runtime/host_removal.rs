use moli_shared_worker::{
    SharedWorkerClientId, SharedWorkerInstanceId, SharedWorkerInstanceRemoval,
};

use crate::worker::WorkerParentErrorEventKind;

use super::{
    host::SharedRendererSharedWorkerHost,
    service::{SharedWorkerRuntimeService, WeakSharedWorkerRuntimeService},
};

pub(super) enum SharedWorkerRemovedHost {
    Running {
        host: SharedRendererSharedWorkerHost,
        clients: Vec<SharedWorkerClientId>,
    },
    Loading {
        host: SharedRendererSharedWorkerHost,
        clients: Vec<SharedWorkerClientId>,
    },
    Missing,
}

impl SharedWorkerRemovedHost {
    pub(super) fn close_after_worker_closed(self) {
        match self {
            Self::Running { host, clients } => {
                host.publish_destroyed_target_event();
                host.close_worker_ports_and_send_closed(clients);
            }
            Self::Loading { host, clients } => {
                host.cancel_loading();
                host.close_worker_ports_and_send_closed(clients);
            }
            Self::Missing => {}
        }
    }

    pub(super) fn fail_after_worker_bootstrap_error(
        self,
        message: String,
        filename: String,
        lineno: u32,
        colno: u32,
        event_kind: WorkerParentErrorEventKind,
    ) {
        match self {
            Self::Running { host, clients } => {
                host.publish_destroyed_target_event();
                host.fail_clients_with_location(
                    clients, message, filename, lineno, colno, event_kind,
                );
            }
            Self::Loading { host, clients } => {
                host.cancel_loading();
                host.fail_clients_with_location(
                    clients, message, filename, lineno, colno, event_kind,
                );
            }
            Self::Missing => {}
        }
    }

    pub(super) fn terminate(self) -> bool {
        match self {
            Self::Running { host, clients } => {
                host.publish_destroyed_target_event();
                host.close_worker_ports_and_send_closed(clients);
                host.terminate();
                true
            }
            Self::Loading { host, clients } => {
                host.cancel_loading();
                host.close_worker_ports_and_send_closed(clients);
                true
            }
            Self::Missing => false,
        }
    }
}

impl WeakSharedWorkerRuntimeService {
    pub(super) fn remove_host_for_instance(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> SharedWorkerRemovedHost {
        let Some(service) = self.upgrade() else {
            return SharedWorkerRemovedHost::Missing;
        };
        service.remove_host_for_instance(instance_id)
    }
}

impl SharedWorkerRuntimeService {
    pub(super) fn remove_host_for_instance(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> SharedWorkerRemovedHost {
        match self.remove_matching_instance(instance_id) {
            SharedWorkerInstanceRemoval::Removed {
                clients,
                instance: Some(host),
                ..
            } => SharedWorkerRemovedHost::Running { host, clients },
            SharedWorkerInstanceRemoval::Removed {
                clients,
                instance: None,
                instance_id,
                ..
            } => {
                let Some(host) = self.remove_loading_host(instance_id) else {
                    return SharedWorkerRemovedHost::Missing;
                };
                SharedWorkerRemovedHost::Loading { host, clients }
            }
            SharedWorkerInstanceRemoval::Missing => SharedWorkerRemovedHost::Missing,
        }
    }
}
