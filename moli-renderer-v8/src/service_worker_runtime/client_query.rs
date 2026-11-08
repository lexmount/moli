use std::{collections::HashMap, sync::Arc};

use parking_lot::Mutex;

use super::{
    clients::{
        ServiceWorkerClientFrameType, ServiceWorkerClientQueryResult, ServiceWorkerClientType,
        ServiceWorkerClientVisibilityState,
    },
    ids::ServiceWorkerClientId,
    run_owner::ServiceWorkerRunOwner,
    state::WeakServiceWorkerRuntimeService,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct ServiceWorkerWindowClientState {
    pub(crate) frame_type: ServiceWorkerClientFrameType,
    pub(crate) visibility_state: ServiceWorkerClientVisibilityState,
    pub(crate) focused: bool,
    pub(crate) last_focus_order: Option<u64>,
}

#[derive(Debug)]
struct PendingWindowClientStates {
    result: ServiceWorkerClientQueryResult,
    remaining: usize,
    focus_orders: HashMap<ServiceWorkerClientId, u64>,
}

#[derive(Debug)]
pub(super) struct ServiceWorkerClientQueryAccumulator {
    service: WeakServiceWorkerRuntimeService,
    owner: ServiceWorkerRunOwner,
    pending: Mutex<PendingWindowClientStates>,
}

impl ServiceWorkerClientQueryAccumulator {
    pub(super) fn new(
        service: WeakServiceWorkerRuntimeService,
        owner: ServiceWorkerRunOwner,
        result: ServiceWorkerClientQueryResult,
        window_count: usize,
    ) -> Arc<Self> {
        debug_assert!(window_count > 0);
        Arc::new(Self {
            service,
            owner,
            pending: Mutex::new(PendingWindowClientStates {
                result,
                remaining: window_count,
                focus_orders: HashMap::new(),
            }),
        })
    }

    pub(super) fn reply_for(
        self: &Arc<Self>,
        client_id: ServiceWorkerClientId,
    ) -> ServiceWorkerWindowClientStateReply {
        let accumulator = Arc::clone(self);
        ServiceWorkerWindowClientStateReply {
            client_id,
            callback: Some(Box::new(move |state| {
                accumulator.complete(client_id, state)
            })),
        }
    }

    fn complete(
        &self,
        client_id: ServiceWorkerClientId,
        state: Option<ServiceWorkerWindowClientState>,
    ) {
        let result = {
            let mut pending = self.pending.lock();
            if let Some(state) = state {
                if let Some(client) = pending
                    .result
                    .clients
                    .iter_mut()
                    .find(|client| client.id == client_id)
                {
                    client.frame_type = state.frame_type;
                    client.visibility_state = state.visibility_state;
                    client.focused = state.focused;
                }
                if let Some(order) = state.last_focus_order {
                    pending.focus_orders.insert(client_id, order);
                }
            } else {
                pending
                    .result
                    .clients
                    .retain(|client| client.id != client_id);
            }
            pending.remaining -= 1;
            if pending.remaining != 0 {
                return;
            }
            let mut clients = std::mem::take(&mut pending.result.clients);
            clients.sort_by(|left, right| {
                let left_window = left.client_type == ServiceWorkerClientType::Window;
                let right_window = right.client_type == ServiceWorkerClientType::Window;
                right_window
                    .cmp(&left_window)
                    .then_with(|| {
                        pending
                            .focus_orders
                            .get(&right.id)
                            .cmp(&pending.focus_orders.get(&left.id))
                    })
                    .then_with(|| left.id.as_u64().cmp(&right.id.as_u64()))
            });
            ServiceWorkerClientQueryResult {
                request_id: pending.result.request_id,
                clients,
            }
        };
        if let Some(service) = self.service.upgrade() {
            service.enqueue_client_query_completed(self.owner.clone(), result);
        }
    }
}

/// One Window's contribution to a Clients query. Discarded Documents and
/// closed Page routes finish their contribution without entering a new realm.
pub(crate) struct ServiceWorkerWindowClientStateReply {
    client_id: ServiceWorkerClientId,
    callback: Option<Box<dyn FnOnce(Option<ServiceWorkerWindowClientState>) + Send>>,
}

impl ServiceWorkerWindowClientStateReply {
    pub(crate) fn complete(mut self, state: ServiceWorkerWindowClientState) {
        if let Some(callback) = self.callback.take() {
            callback(Some(state));
        }
    }
}

impl Drop for ServiceWorkerWindowClientStateReply {
    fn drop(&mut self) {
        if let Some(callback) = self.callback.take() {
            callback(None);
        }
    }
}

impl std::fmt::Debug for ServiceWorkerWindowClientStateReply {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServiceWorkerWindowClientStateReply")
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}
