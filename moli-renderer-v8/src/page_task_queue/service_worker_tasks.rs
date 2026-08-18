use crate::types::{
    ServiceWorkerClientFocusRequestCompletion, ServiceWorkerClientMessageCompletion,
    ServiceWorkerClientNavigateRequestCompletion, ServiceWorkerClientsOpenWindowRequestCompletion,
    ServiceWorkerControllerChangeCompletion, ServiceWorkerLifecycleNotification,
    ServiceWorkerNotificationActionNavigateRequestCompletion, ServiceWorkerReadyCompletion,
    ServiceWorkerRegisterCompletion, ServiceWorkerUnregisterCompletion,
};

use super::{
    RendererPageServiceWorkerClientMessageSender, RendererPageServiceWorkerInternalSender,
    RendererServiceWorkerInternalTask,
};

/// Complete PageVm-stamped ServiceWorker callback capability.
///
/// The facade keeps browser-context producers unaware of scheduler plumbing,
/// while its two non-optional routes preserve the distinct Chromium task
/// sources for internal callbacks and client `message` delivery.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageServiceWorkerTaskSender {
    internal: RendererPageServiceWorkerInternalSender,
    client_message: RendererPageServiceWorkerClientMessageSender,
}

impl RendererPageServiceWorkerTaskSender {
    pub(crate) fn new(
        internal: RendererPageServiceWorkerInternalSender,
        client_message: RendererPageServiceWorkerClientMessageSender,
    ) -> Self {
        Self {
            internal,
            client_message,
        }
    }

    pub(crate) fn send_service_worker_register(
        &self,
        completion: ServiceWorkerRegisterCompletion,
    ) -> Result<(), RendererPageServiceWorkerTaskRouteClosed> {
        self.send_internal(RendererServiceWorkerInternalTask::Register(completion))
    }

    pub(crate) fn send_service_worker_ready(
        &self,
        completion: ServiceWorkerReadyCompletion,
    ) -> Result<(), RendererPageServiceWorkerTaskRouteClosed> {
        self.send_internal(RendererServiceWorkerInternalTask::Ready(completion))
    }

    pub(crate) fn send_service_worker_unregister(
        &self,
        completion: ServiceWorkerUnregisterCompletion,
    ) -> Result<(), RendererPageServiceWorkerTaskRouteClosed> {
        self.send_internal(RendererServiceWorkerInternalTask::Unregister(completion))
    }

    pub(crate) fn send_service_worker_lifecycle(
        &self,
        completion: ServiceWorkerLifecycleNotification,
    ) -> Result<(), RendererPageServiceWorkerTaskRouteClosed> {
        self.send_internal(RendererServiceWorkerInternalTask::Lifecycle(completion))
    }

    pub(crate) fn send_service_worker_controller_change(
        &self,
        completion: ServiceWorkerControllerChangeCompletion,
    ) -> Result<(), RendererPageServiceWorkerTaskRouteClosed> {
        self.send_internal(RendererServiceWorkerInternalTask::ControllerChange(
            completion,
        ))
    }

    pub(crate) fn send_service_worker_client_message(
        &self,
        completion: ServiceWorkerClientMessageCompletion,
    ) -> Result<(), RendererPageServiceWorkerTaskRouteClosed> {
        self.client_message
            .send(completion)
            .map_err(|_| RendererPageServiceWorkerTaskRouteClosed)
    }

    pub(crate) fn send_service_worker_client_navigate_request(
        &self,
        completion: ServiceWorkerClientNavigateRequestCompletion,
    ) -> Result<(), RendererPageServiceWorkerTaskRouteClosed> {
        self.send_internal(RendererServiceWorkerInternalTask::ClientNavigateRequest(
            completion,
        ))
    }

    pub(crate) fn send_service_worker_client_focus_request(
        &self,
        completion: ServiceWorkerClientFocusRequestCompletion,
    ) -> Result<(), RendererPageServiceWorkerTaskRouteClosed> {
        self.send_internal(RendererServiceWorkerInternalTask::ClientFocusRequest(
            completion,
        ))
    }

    pub(crate) fn send_service_worker_clients_open_window_request(
        &self,
        completion: ServiceWorkerClientsOpenWindowRequestCompletion,
    ) -> Result<(), RendererPageServiceWorkerTaskRouteClosed> {
        self.send_internal(RendererServiceWorkerInternalTask::ClientsOpenWindowRequest(
            completion,
        ))
    }

    pub(crate) fn send_service_worker_notification_action_navigate_request(
        &self,
        completion: ServiceWorkerNotificationActionNavigateRequestCompletion,
    ) -> Result<(), RendererPageServiceWorkerTaskRouteClosed> {
        self.send_internal(
            RendererServiceWorkerInternalTask::NotificationActionNavigateRequest(completion),
        )
    }

    fn send_internal(
        &self,
        task: RendererServiceWorkerInternalTask,
    ) -> Result<(), RendererPageServiceWorkerTaskRouteClosed> {
        self.internal
            .send(task)
            .map_err(|_| RendererPageServiceWorkerTaskRouteClosed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageServiceWorkerTaskRouteClosed;

/// Retains the production ServiceWorker Page sources for low-level runtime
/// tests. Tests dequeue the same strongly typed scheduler tasks as PageVm;
/// they never reconstruct the removed legacy resource aggregate.
#[cfg(test)]
pub(crate) struct RendererPageServiceWorkerTestHarness {
    residence: crate::page_task_queue::RendererPageTaskTestResidence,
}

#[cfg(test)]
impl RendererPageServiceWorkerTestHarness {
    pub(crate) fn new() -> Self {
        Self {
            residence: crate::page_task_queue::RendererPageTaskTestResidence::new(None),
        }
    }

    pub(crate) fn sender(&self) -> RendererPageServiceWorkerTaskSender {
        self.residence
            .service_worker_task_sender_for_root(self.residence.root_document())
    }

    pub(crate) fn pop_internal(&mut self) -> Option<RendererServiceWorkerInternalTask> {
        let scheduler_task =
            self.residence
                .task_sources()
                .take_scheduler_task_for_executor_test(|descriptor| {
                    matches!(
                    descriptor,
                    crate::page_task_queue::RendererPageReadyDescriptor::ServiceWorkerInternal {
                        ..
                    }
                )
                })?;
        let crate::page_task_queue::RendererPageSchedulerTask::ServiceWorkerInternal(task) =
            scheduler_task
        else {
            panic!("ServiceWorker internal descriptor dequeued a different task variant")
        };
        Some(task.into_task())
    }

    pub(crate) fn pop_client_message(&mut self) -> Option<ServiceWorkerClientMessageCompletion> {
        let scheduler_task = self
            .residence
            .task_sources()
            .take_scheduler_task_for_executor_test(|descriptor| {
                matches!(
                    descriptor,
                    crate::page_task_queue::RendererPageReadyDescriptor::ServiceWorkerClientMessage {
                        ..
                    }
                )
            })?;
        let crate::page_task_queue::RendererPageSchedulerTask::ServiceWorkerClientMessage(task) =
            scheduler_task
        else {
            panic!("ServiceWorker message descriptor dequeued a different task variant")
        };
        Some(task.into_completion())
    }

    pub(crate) fn has_ready_task(&mut self) -> bool {
        self.residence
            .task_sources()
            .has_ready_service_worker_task()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        service_worker_runtime::{ServiceWorkerClientId, ServiceWorkerVersionId},
        structured_clone::V8StructuredClonePayload,
        types::{
            ServiceWorkerClientMessageCompletion, ServiceWorkerUnregisterCompletion,
            ServiceWorkerWindowClientTarget,
        },
    };

    #[test]
    fn internal_callbacks_and_client_messages_remain_in_distinct_sources() {
        let mut harness = RendererPageServiceWorkerTestHarness::new();
        let sender = harness.sender();
        let target = ServiceWorkerWindowClientTarget {
            client_id: ServiceWorkerClientId::from_u64_for_test(17),
            document_owner: crate::native_bridge::WindowDocumentOwner::for_test(23),
        };

        sender
            .send_service_worker_unregister(ServiceWorkerUnregisterCompletion {
                request_id: 31,
                document_owner: crate::native_bridge::WindowDocumentOwner::for_test(37),
                result: true,
            })
            .expect("internal callback should enter its typed source");
        sender
            .send_service_worker_client_message(ServiceWorkerClientMessageCompletion {
                target,
                source_version_id: ServiceWorkerVersionId::from_u64_for_test(41),
                source_script_url: url::Url::parse("https://service-worker-source.test/worker.js")
                    .expect("worker URL"),
                source_state: "activated",
                payload: V8StructuredClonePayload::default(),
            })
            .expect("client message should enter its typed source");

        let message = harness
            .pop_client_message()
            .expect("client-message source should retain its own task");
        assert_eq!(message.target, target);
        assert!(matches!(
            harness.pop_internal(),
            Some(RendererServiceWorkerInternalTask::Unregister(_))
        ));
        assert!(!harness.has_ready_task());
    }
}
