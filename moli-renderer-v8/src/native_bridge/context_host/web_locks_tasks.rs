use super::*;
use crate::context_bootstrap::web_locks::WebLocksState;
use moli_storage_service::{SharedStorageService, StorageBucketLocator};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct WindowWebLocksClient {
    pub(crate) execution_context: WindowExecutionContextIdentity,
    pub(crate) relevant_context: WindowExecutionContextBinding,
    pub(crate) state: Rc<RefCell<WebLocksState>>,
}

impl JsContextHost {
    pub(crate) fn register_web_locks_client(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        service: Option<SharedStorageService>,
    ) -> Option<u64> {
        let execution_context = self.current_runtime_window_execution_context_identity(scope)?;
        let storage = self
            .storage_context_for_window_execution_context_identity(execution_context)?
            .storage_key()
            .serialized_storage_key();
        let storage = moli_storage_service::storage_bucket_origin_allows_storage(&storage)
            .then_some(storage)
            .and_then(|storage_key| {
                service.map(|service| (service, StorageBucketLocator::Default { storage_key }))
            });
        let task_id = self.next_web_locks_task_id;
        self.next_web_locks_task_id = task_id
            .checked_next()
            .expect("Web Locks manager id overflow");
        let producer = self
            .page_web_locks_task_sender()
            .bind_task(execution_context, task_id);
        let client_id =
            self.service_worker_client_id_for_subresource_owner(execution_context.dispatch_scope());
        let runtime = self.browser_context_runtime();
        let exposed_id = runtime
            .service_worker_runtime_if_initialized()
            .and_then(|runtime| runtime.client_exposed_id(client_id))
            .unwrap_or_else(|| crate::runtime::service_worker_exposed_client_id(client_id));
        let client = storage.map(|(service, bucket)| {
            service.web_locks().connect(
                bucket,
                exposed_id,
                Arc::new(move |event| {
                    let _ = producer.send(event);
                }),
            )
        });
        let state = Rc::new(RefCell::new(WebLocksState::new(client)));
        let relevant_context = WindowExecutionContextBinding::new(
            execution_context.owner(),
            execution_context.dispatch_scope(),
            execution_context.realm_token(),
            v8::Global::new(scope, scope.get_current_context()),
        );
        let replaced = self.web_locks_clients.insert(
            task_id,
            WindowWebLocksClient {
                execution_context,
                relevant_context,
                state,
            },
        );
        assert!(
            replaced.is_none(),
            "Web Locks manager IDs must never be reused"
        );
        Some(task_id.task_id())
    }

    pub(crate) fn current_pending_web_locks_task_execution_context(
        &self,
        task: crate::page_task_queue::RendererPageWebLocksTaskId,
    ) -> Option<WindowExecutionContextIdentity> {
        let client = self.web_locks_clients.get(&task)?;
        self.window_execution_context_identity_is_current(client.execution_context)
            .then_some(client.execution_context)
    }

    pub(crate) fn web_locks_client_for_exact_owner(
        &self,
        execution_context: WindowExecutionContextIdentity,
        task: crate::page_task_queue::RendererPageWebLocksTaskId,
    ) -> Option<WindowWebLocksClient> {
        let client = self.web_locks_clients.get(&task)?;
        (client.execution_context == execution_context).then(|| client.clone())
    }

    pub(crate) fn web_locks_client_state(&self, id: u64) -> Option<Rc<RefCell<WebLocksState>>> {
        let id = crate::page_task_queue::RendererPageWebLocksTaskId::new(id);
        let owner = self.current_pending_web_locks_task_execution_context(id)?;
        self.web_locks_client_for_exact_owner(owner, id)
            .map(|client| client.state)
    }

    pub(crate) fn retire_web_locks_execution_context_owner(
        &mut self,
        owner: WindowExecutionContextOwner,
    ) {
        self.web_locks_clients.retain(|_, client| {
            if client.relevant_context.owner() != owner {
                return true;
            }
            client.state.borrow_mut().retire();
            false
        });
    }

    pub(crate) fn retire_web_locks_context_token(&mut self, token: RuntimeObservableContextToken) {
        self.web_locks_clients.retain(|_, client| {
            if client.relevant_context.realm_token() != token {
                return true;
            }
            client.state.borrow_mut().retire();
            false
        });
    }

    pub(crate) fn has_pending_web_locks_tasks(&self) -> bool {
        self.web_locks_clients
            .values()
            .any(|client| client.state.borrow().has_pending())
    }
}
