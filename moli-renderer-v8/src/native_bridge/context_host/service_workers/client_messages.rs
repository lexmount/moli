use std::collections::VecDeque;

use super::{OwnerDispatchScope, PageScriptHost, ServiceWorkerWindowOwner, WindowDocumentOwner};
use crate::types::ServiceWorkerClientMessageCompletion;

#[derive(Default)]
pub(in crate::native_bridge::context_host) struct ServiceWorkerClientMessageQueue {
    enabled: bool,
    pending: VecDeque<ServiceWorkerClientMessageCompletion>,
    scheduled: usize,
}

impl PageScriptHost {
    pub(crate) fn service_worker_window_id_for_dispatch_scope(
        &self,
        scope: OwnerDispatchScope,
    ) -> Option<u64> {
        use super::super::WindowExecutionContextOwner;
        match self.current_window_execution_context_owner(scope)? {
            WindowExecutionContextOwner::Frame(id) => Some(id.0),
            WindowExecutionContextOwner::LightweightPopup {
                local_window_id, ..
            } => Some(local_window_id.as_u64()),
        }
    }

    fn service_worker_client_queue_initially_enabled(&self, scope: OwnerDispatchScope) -> bool {
        let document = match scope {
            OwnerDispatchScope::Top => Some(self.document_handle()),
            OwnerDispatchScope::Child(child) => self.child_browsing_context_document_handle(child),
            OwnerDispatchScope::LightweightPopup(popup) => {
                self.lightweight_popup_document_handle(popup)
            }
        };
        // Initial about:blank documents can already be complete without a
        // parser lifecycle task. Interactive Documents are enabled explicitly
        // by their real DOMContentLoaded transition.
        document
            .is_some_and(|document| self.document_ready_state_for_handle(document) == "complete")
    }

    pub(crate) fn enable_service_worker_client_message_queue(&mut self, scope: OwnerDispatchScope) {
        let Some(owner) = self.service_worker_window_owner_for_dispatch_scope(scope) else {
            return;
        };
        self.enable_service_worker_client_message_queue_for_document(
            scope,
            owner.window_document_owner(),
        );
    }

    pub(crate) fn enable_service_worker_client_message_queue_for_document(
        &mut self,
        scope: OwnerDispatchScope,
        document: WindowDocumentOwner,
    ) {
        if self
            .service_worker_window_owner_for_dispatch_scope(scope)
            .is_none_or(|owner| owner.window_document_owner() != document)
        {
            return;
        }
        let queue = self
            .service_worker_client_message_queues
            .entry(document)
            .or_default();
        if queue.enabled {
            return;
        }
        queue.enabled = true;
        let pending = std::mem::take(&mut queue.pending);
        for completion in pending {
            self.schedule_service_worker_client_queue_delivery(document, completion);
        }
    }

    fn schedule_service_worker_client_queue_delivery(
        &mut self,
        document: WindowDocumentOwner,
        completion: ServiceWorkerClientMessageCompletion,
    ) {
        let queue = self
            .service_worker_client_message_queues
            .get_mut(&document)
            .expect("a queued client message retains its current Document queue");
        queue.scheduled = queue
            .scheduled
            .checked_add(1)
            .expect("ServiceWorker client message queue size exhausted");
        if self
            .service_worker_task_tx
            .send_service_worker_client_queue_delivery(completion)
            .is_err()
        {
            self.service_worker_client_message_queues
                .get_mut(&document)
                .expect("sending a Page task cannot retire its Document queue")
                .scheduled -= 1;
        }
    }

    pub(crate) fn admit_service_worker_client_message(
        &mut self,
        owner: ServiceWorkerWindowOwner,
        completion: ServiceWorkerClientMessageCompletion,
        client_queue_delivery: bool,
    ) -> Option<ServiceWorkerClientMessageCompletion> {
        assert!(self.service_worker_window_owner_is_current(owner));
        let document = owner.window_document_owner();
        let enabled = self.service_worker_client_queue_initially_enabled(owner.dispatch_scope());
        let queue = self
            .service_worker_client_message_queues
            .entry(document)
            .or_insert_with(|| ServiceWorkerClientMessageQueue {
                enabled,
                ..Default::default()
            });
        if client_queue_delivery {
            queue.scheduled = queue
                .scheduled
                .checked_sub(1)
                .expect("a resumed client message must own a scheduled queue delivery");
            return Some(completion);
        }
        if !queue.enabled {
            queue.pending.push_back(completion);
            return None;
        }
        if queue.scheduled != 0 {
            // New incoming Page tasks can precede messages resumed by
            // startMessages(). Put them behind that older backlog, rather
            // than dispatching them ahead of it or dispatching synchronously.
            self.schedule_service_worker_client_queue_delivery(document, completion);
            return None;
        }
        Some(completion)
    }
}
