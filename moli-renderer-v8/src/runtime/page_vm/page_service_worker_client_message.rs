//! ServiceWorker-to-Window client-message Page-task authorization.
//!
//! Root-Document identity and Window-client generation are separate
//! authorities. This coordinator validates both, executes one body, and maps
//! the resulting domain fact to the unique selected-task completion. It does
//! not inspect ServiceWorker internal callbacks or other task sources.

use anyhow::Result;

use crate::{
    page_task_queue::{
        PageServiceWorkerClientMessageTargetEffect, PageServiceWorkerClientMessageTurnAction,
        PageServiceWorkerClientMessageTurnOutcome, RendererPageServiceWorkerClientMessageTask,
        ServiceWorkerClientMessageCallbackEffect, ServiceWorkerClientMessageEventKind,
    },
    script_vm::{
        ServiceWorkerClientMessageBodyCallbackEffect, ServiceWorkerClientMessageBodyEffect,
        ServiceWorkerClientMessageBodyEventKind,
    },
};

use super::{IntoPageTaskCompletion, PageTaskCompletion, PageVm};

impl From<ServiceWorkerClientMessageBodyEffect> for PageServiceWorkerClientMessageTargetEffect {
    fn from(effect: ServiceWorkerClientMessageBodyEffect) -> Self {
        match effect {
            ServiceWorkerClientMessageBodyEffect::QueuedForClientMessageQueue => {
                Self::QueuedForClientMessageQueue
            }
            ServiceWorkerClientMessageBodyEffect::EventDispatched {
                event_kind,
                callback_effect,
            } => Self::EventDispatchedToCurrentTarget {
                event_kind: match event_kind {
                    ServiceWorkerClientMessageBodyEventKind::Message => {
                        ServiceWorkerClientMessageEventKind::Message
                    }
                    ServiceWorkerClientMessageBodyEventKind::MessageError => {
                        ServiceWorkerClientMessageEventKind::MessageError
                    }
                },
                callback_effect: match callback_effect {
                    ServiceWorkerClientMessageBodyCallbackEffect::CallbackDispatched => {
                        ServiceWorkerClientMessageCallbackEffect::CallbackDispatched
                    }
                    ServiceWorkerClientMessageBodyCallbackEffect::CurrentTargetHadNoCallback => {
                        ServiceWorkerClientMessageCallbackEffect::CurrentTargetHadNoCallback
                    }
                },
            },
            ServiceWorkerClientMessageBodyEffect::CurrentTargetProducedNoDispatchableEvent => {
                Self::CurrentTargetProducedNoDispatchableEvent
            }
        }
    }
}

impl IntoPageTaskCompletion for PageServiceWorkerClientMessageTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect {
            PageServiceWorkerClientMessageTargetEffect::EventDispatchedToCurrentTarget {
                callback_effect: ServiceWorkerClientMessageCallbackEffect::CallbackDispatched,
                ..
            } => PageTaskCompletion::CallbackCompletion,
            PageServiceWorkerClientMessageTargetEffect::EventDispatchedToCurrentTarget {
                callback_effect: ServiceWorkerClientMessageCallbackEffect::CurrentTargetHadNoCallback,
                ..
            }
            | PageServiceWorkerClientMessageTargetEffect::CurrentTargetProducedNoDispatchableEvent
            | PageServiceWorkerClientMessageTargetEffect::QueuedForClientMessageQueue => {
                PageTaskCompletion::CheckpointOnly
            }
            PageServiceWorkerClientMessageTargetEffect::DiscardedStaleTarget
            | PageServiceWorkerClientMessageTargetEffect::DiscardedStaleRoot { .. } => {
                PageTaskCompletion::NoCompletion
            }
        }
    }
}

/// Proof that the Page arbiter matched both the root Document and the exact
/// Window-client document/generation before the body entered V8.
pub(crate) struct AuthorizedCurrentPageServiceWorkerClientMessage {
    task: RendererPageServiceWorkerClientMessageTask,
    window_owner: crate::native_bridge::ServiceWorkerWindowOwner,
}

impl AuthorizedCurrentPageServiceWorkerClientMessage {
    fn new(
        task: RendererPageServiceWorkerClientMessageTask,
        window_owner: crate::native_bridge::ServiceWorkerWindowOwner,
    ) -> Self {
        Self { task, window_owner }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        RendererPageServiceWorkerClientMessageTask,
        crate::native_bridge::ServiceWorkerWindowOwner,
    ) {
        (self.task, self.window_owner)
    }
}

impl PageVm {
    /// Apply one ServiceWorker client message after both Page-level
    /// authorities have accepted its exact owner.
    pub(in crate::runtime) fn apply_selected_page_service_worker_client_message_turn(
        &mut self,
        task: RendererPageServiceWorkerClientMessageTask,
    ) -> Result<PageServiceWorkerClientMessageTurnOutcome> {
        let owner = task.owner();
        let current_root = self.document_lifecycle.identity().document;
        let target_effect = if owner.root_document() != current_root {
            PageServiceWorkerClientMessageTargetEffect::DiscardedStaleRoot { current_root }
        } else if let Some(window_owner) = self
            .vm()
            .current_service_worker_client_message_owner(owner.target())
        {
            self.vm_mut()
                .apply_current_service_worker_client_message_body(
                    AuthorizedCurrentPageServiceWorkerClientMessage::new(task, window_owner),
                )?
                .into()
        } else {
            PageServiceWorkerClientMessageTargetEffect::DiscardedStaleTarget
        };
        let action = PageServiceWorkerClientMessageTurnAction {
            owner,
            target_effect,
        };
        Ok(PageServiceWorkerClientMessageTurnOutcome::new(action))
    }
}
