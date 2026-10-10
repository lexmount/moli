//! Worker-owned Web Locks clients and their native completion route.

use super::WorkerMessage;
use crate::context_bootstrap::web_locks::WebLocksState;
use moli_storage_service::{SharedStorageService, StorageBucketLocator, WebLockEvent};
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};
use tokio::sync::mpsc;

struct Client {
    context: v8::Global<v8::Context>,
    state: Rc<RefCell<WebLocksState>>,
}

#[derive(Default)]
struct Clients {
    next_id: u64,
    exposed_id: Option<String>,
    clients: HashMap<u64, Client>,
}

#[derive(Clone)]
pub(crate) struct WorkerWebLocksTasks {
    clients: Rc<RefCell<Clients>>,
    wake_tx: mpsc::UnboundedSender<WorkerMessage>,
}

impl WorkerWebLocksTasks {
    pub(super) fn new(wake_tx: mpsc::UnboundedSender<WorkerMessage>) -> Self {
        Self {
            clients: Rc::new(RefCell::new(Clients::default())),
            wake_tx,
        }
    }

    pub(super) fn clear(&self) {
        let mut clients = self.clients.borrow_mut();
        for client in clients.clients.values() {
            client.state.borrow_mut().retire();
        }
        clients.clients.clear();
    }

    pub(super) fn has_pending(&self) -> bool {
        self.clients
            .borrow()
            .clients
            .values()
            .any(|client| client.state.borrow().has_pending())
    }

    pub(crate) fn state(&self, id: u64) -> Option<Rc<RefCell<WebLocksState>>> {
        self.clients
            .borrow()
            .clients
            .get(&id)
            .map(|client| client.state.clone())
    }

    pub(crate) fn register(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        storage: Option<(SharedStorageService, StorageBucketLocator)>,
    ) -> u64 {
        let exposed_id = super::global_scope::get_worker_state(scope)
            .and_then(|state| {
                let state = state.borrow();
                state.service_worker_client_id.map(|id| {
                    state
                        .service_worker_runtime
                        .as_ref()
                        .and_then(|runtime| runtime.client_exposed_id(id))
                        .unwrap_or_else(|| crate::runtime::service_worker_exposed_client_id(id))
                })
            })
            .unwrap_or_else(
                crate::service_worker_runtime::allocate_service_worker_exposed_client_id,
            );
        let mut clients = self.clients.borrow_mut();
        let exposed_id = clients.exposed_id.get_or_insert(exposed_id).clone();
        clients.next_id = clients
            .next_id
            .checked_add(1)
            .expect("worker Web Locks manager id overflow");
        let id = clients.next_id;
        let wake_tx = self.wake_tx.clone();
        let client = storage.map(|(service, bucket)| {
            service.web_locks().connect(
                bucket,
                exposed_id,
                Arc::new(move |event| {
                    let _ = wake_tx.send(WorkerMessage::RunWebLocksTask(id, event));
                }),
            )
        });
        clients.clients.insert(
            id,
            Client {
                context: v8::Global::new(scope, scope.get_current_context()),
                state: Rc::new(RefCell::new(WebLocksState::new(client))),
            },
        );
        id
    }

    pub(super) fn dispatch(&self, scope: &mut v8::PinScope<'_, '_>, id: u64, event: WebLockEvent) {
        let Some((context, state)) = self
            .clients
            .borrow()
            .clients
            .get(&id)
            .map(|client| (client.context.clone(), client.state.clone()))
        else {
            return;
        };
        let context = v8::Local::new(scope, context);
        let scope = &mut v8::ContextScope::new(scope, context);
        crate::context_bootstrap::web_locks::dispatch(scope, &state, event);
    }
}

pub(crate) fn queue(scope: &v8::PinScope<'_, '_>) -> Option<WorkerWebLocksTasks> {
    scope.get_slot::<WorkerWebLocksTasks>().cloned()
}
