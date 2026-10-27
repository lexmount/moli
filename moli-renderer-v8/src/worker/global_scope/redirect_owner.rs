use super::*;
use crate::network_host::RedirectAdmission;

#[derive(Clone)]
enum Endpoint {
    Fetch(mpsc::UnboundedSender<WorkerFetchEvent>, u32),
    Xhr(mpsc::UnboundedSender<WorkerXhrEvent>, u32),
}

/// Opaque outside the Worker: the network coordinator need not know its event
/// queues or V8 state. The lease retains keepalive's existing retirement rules.
/// Unlike Window keepalive, a live Worker's keepalive still waits for its owner.
#[derive(Clone)]
pub(crate) struct WorkerRedirectOwner {
    endpoint: Endpoint,
    load: ResourceLoadLease,
}

impl WorkerRedirectOwner {
    pub(in crate::worker) fn fetch(
        sender: mpsc::UnboundedSender<WorkerFetchEvent>,
        id: u32,
        load: ResourceLoadLease,
    ) -> Self {
        Self {
            endpoint: Endpoint::Fetch(sender, id),
            load,
        }
    }

    pub(in crate::worker) fn xhr(
        sender: mpsc::UnboundedSender<WorkerXhrEvent>,
        id: u32,
        load: ResourceLoadLease,
    ) -> Self {
        Self {
            endpoint: Endpoint::Xhr(sender, id),
            load,
        }
    }

    pub(crate) fn post(&self, admission: RedirectAdmission) -> bool {
        match &self.endpoint {
            Endpoint::Fetch(sender, fetch_id) => sender
                .send(WorkerFetchEvent::RedirectAdmission {
                    fetch_id: *fetch_id,
                    admission,
                })
                .is_ok(),
            Endpoint::Xhr(sender, xhr_id) => sender
                .send(WorkerXhrEvent::RedirectAdmission {
                    xhr_id: *xhr_id,
                    admission,
                })
                .is_ok(),
        }
    }

    pub(crate) fn detached_keepalive(&self) -> bool {
        self.load.is_detached_keepalive()
    }
}
