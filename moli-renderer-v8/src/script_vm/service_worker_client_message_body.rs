//! Exact-target ServiceWorker client-message body execution.
//!
//! The browser-context runtime has already transformed delivery into a stable
//! Page task. This component only enters the authorized Window client and
//! dispatches `message` or `messageerror`; it does not perform the selected
//! task's microtask checkpoint, child-record synchronization, or runtime
//! follow-up.

use anyhow::Result;

use super::ScriptVm;
use crate::{
    context_bootstrap::{
        ServiceWorkerClientMessageCallbackDispatchEffect, ServiceWorkerClientMessageDispatchEffect,
    },
    runtime::AuthorizedCurrentPageServiceWorkerClientMessage,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerClientMessageBodyEffect {
    QueuedForClientMessageQueue,
    EventDispatched {
        event_kind: ServiceWorkerClientMessageBodyEventKind,
        callback_effect: ServiceWorkerClientMessageBodyCallbackEffect,
    },
    /// The exact Window client remained current, but its body produced no
    /// dispatchable `message`/`messageerror` event.
    CurrentTargetProducedNoDispatchableEvent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerClientMessageBodyEventKind {
    Message,
    MessageError,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerClientMessageBodyCallbackEffect {
    /// Dispatch entered at least one registered callback body.
    CallbackDispatched,
    /// The event was dispatched, but the exact current target had no matching
    /// callback when dispatch began.
    CurrentTargetHadNoCallback,
}

impl From<ServiceWorkerClientMessageCallbackDispatchEffect>
    for ServiceWorkerClientMessageBodyCallbackEffect
{
    fn from(effect: ServiceWorkerClientMessageCallbackDispatchEffect) -> Self {
        match effect {
            ServiceWorkerClientMessageCallbackDispatchEffect::CallbackDispatched => {
                Self::CallbackDispatched
            }
            ServiceWorkerClientMessageCallbackDispatchEffect::CurrentTargetHadNoCallback => {
                Self::CurrentTargetHadNoCallback
            }
        }
    }
}

impl From<ServiceWorkerClientMessageDispatchEffect> for ServiceWorkerClientMessageBodyEffect {
    fn from(effect: ServiceWorkerClientMessageDispatchEffect) -> Self {
        match effect {
            ServiceWorkerClientMessageDispatchEffect::MessageDispatched { callback_effect } => {
                Self::EventDispatched {
                    event_kind: ServiceWorkerClientMessageBodyEventKind::Message,
                    callback_effect: callback_effect.into(),
                }
            }
            ServiceWorkerClientMessageDispatchEffect::MessageErrorDispatched {
                callback_effect,
            } => Self::EventDispatched {
                event_kind: ServiceWorkerClientMessageBodyEventKind::MessageError,
                callback_effect: callback_effect.into(),
            },
            ServiceWorkerClientMessageDispatchEffect::CurrentTargetProducedNoDispatchableEvent => {
                Self::CurrentTargetProducedNoDispatchableEvent
            }
        }
    }
}

impl ScriptVm {
    pub(crate) fn current_service_worker_client_message_owner(
        &self,
        target: crate::types::ServiceWorkerWindowClientTarget,
    ) -> Option<crate::native_bridge::ServiceWorkerWindowOwner> {
        self._context_host
            .borrow()
            .service_worker_window_client_completion_owner(target)
    }

    pub(crate) fn apply_current_service_worker_client_message_body(
        &mut self,
        authorized: AuthorizedCurrentPageServiceWorkerClientMessage,
    ) -> Result<ServiceWorkerClientMessageBodyEffect> {
        let (task, window_owner) = authorized.into_parts();
        assert!(
            self._context_host
                .borrow()
                .service_worker_window_owner_is_current(window_owner),
            "authorized ServiceWorker client-message owner changed inside one Page turn"
        );
        let client_queue_delivery = task.is_client_queue_delivery();
        let Some(completion) = self
            ._context_host
            .borrow_mut()
            .admit_service_worker_client_message(
                window_owner,
                task.into_completion(),
                client_queue_delivery,
            )
        else {
            return Ok(ServiceWorkerClientMessageBodyEffect::QueuedForClientMessageQueue);
        };
        self.with_default_context_scope(move |scope, _host_ptr| {
            Ok(
                crate::context_bootstrap::dispatch_service_worker_client_message_body(
                    scope,
                    window_owner.dispatch_scope(),
                    completion,
                )
                .into(),
            )
        })
    }
}
