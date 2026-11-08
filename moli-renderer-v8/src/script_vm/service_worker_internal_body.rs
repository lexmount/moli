//! Body-only coordination for browser-context ServiceWorker internal tasks.
//!
//! The Page source has already made each browser callback durable and the
//! Page coordinator has authorized its exact root Document. This module only
//! selects the domain-specific body executor and reports what that body
//! actually did. Promise settlement, event dispatch, and Window-client
//! requests live in separate modules because they have different target and
//! completion semantics.

use anyhow::Result;

use super::ScriptVm;
use crate::page_task_queue::RendererServiceWorkerInternalTask;
use crate::runtime::AuthorizedCurrentPageServiceWorkerInternalTask;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerInternalBodyEffect {
    /// An exact current request resolver was settled. Promise reactions remain
    /// pending for the selected task's ordinary checkpoint.
    PromiseSettled,
    /// The current task completed an event-dispatch pass. The nested effect
    /// records whether that pass actually entered callback code.
    EventDispatchPassCompleted {
        callback_effect: ServiceWorkerInternalBodyCallbackEffect,
    },
    /// A current client request updated browser/DOM-side state or published
    /// its typed completion without dispatching a Page callback.
    InternalActionApplied,
    /// The root Page remained current, but the exact pending request, Window
    /// client, or request context had already disappeared. No replacement
    /// realm may be entered merely to manufacture a checkpoint.
    ExactTargetUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerInternalBodyCallbackEffect {
    CallbackBodyDispatched,
    NoCallbackBodyDispatched,
}

impl From<crate::context_bootstrap::ServiceWorkerInternalEventCallbackDispatchEffect>
    for ServiceWorkerInternalBodyCallbackEffect
{
    fn from(
        effect: crate::context_bootstrap::ServiceWorkerInternalEventCallbackDispatchEffect,
    ) -> Self {
        match effect {
            crate::context_bootstrap::ServiceWorkerInternalEventCallbackDispatchEffect::CallbackBodyDispatched => {
                Self::CallbackBodyDispatched
            }
            crate::context_bootstrap::ServiceWorkerInternalEventCallbackDispatchEffect::NoCallbackBodyDispatched => {
                Self::NoCallbackBodyDispatched
            }
        }
    }
}

impl ScriptVm {
    pub(crate) fn apply_current_service_worker_internal_body(
        &mut self,
        authorization: AuthorizedCurrentPageServiceWorkerInternalTask,
    ) -> Result<ServiceWorkerInternalBodyEffect> {
        match authorization.into_task().into_task() {
            RendererServiceWorkerInternalTask::Register(completion) => {
                self.apply_service_worker_register_body(completion)
            }
            RendererServiceWorkerInternalTask::Ready(completion) => {
                self.apply_service_worker_ready_body(completion)
            }
            RendererServiceWorkerInternalTask::Unregister(completion) => {
                self.apply_service_worker_unregister_body(completion)
            }
            RendererServiceWorkerInternalTask::Lifecycle(completion) => {
                self.apply_service_worker_lifecycle_body(completion)
            }
            RendererServiceWorkerInternalTask::ControllerChange(completion) => {
                self.apply_service_worker_controller_change_body(completion)
            }
            RendererServiceWorkerInternalTask::ClientNavigateRequest(completion) => {
                self.apply_service_worker_client_navigate_request_body(completion)
            }
            RendererServiceWorkerInternalTask::ClientFocusRequest(completion) => {
                self.apply_service_worker_client_focus_request_body(completion)
            }
            RendererServiceWorkerInternalTask::WindowClientStateRequest(request) => {
                let queued = self
                    ._context_host
                    .borrow_mut()
                    .queue_service_worker_window_client_state_request(request);
                Ok(if queued {
                    ServiceWorkerInternalBodyEffect::InternalActionApplied
                } else {
                    ServiceWorkerInternalBodyEffect::ExactTargetUnavailable
                })
            }
            RendererServiceWorkerInternalTask::WindowClientFocusRequest(request) => {
                let queued = self
                    ._context_host
                    .borrow_mut()
                    .queue_service_worker_window_client_focus_request(request);
                Ok(if queued {
                    ServiceWorkerInternalBodyEffect::InternalActionApplied
                } else {
                    ServiceWorkerInternalBodyEffect::ExactTargetUnavailable
                })
            }
            RendererServiceWorkerInternalTask::ClientsOpenWindowRequest(completion) => {
                self.apply_service_worker_clients_open_window_request_body(completion)
            }
            RendererServiceWorkerInternalTask::NotificationActionNavigateRequest(completion) => {
                self.apply_service_worker_notification_action_navigate_request_body(completion)
            }
        }
    }
}
