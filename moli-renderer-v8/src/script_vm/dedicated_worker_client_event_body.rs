//! Exact-target DedicatedWorker client-event body execution.
//!
//! This component owns Worker state transitions, realm entry, and event
//! dispatch only. The Page arbiter authorizes the exact target before entry;
//! the selected-task dispatcher owns the ordinary task-end checkpoint and
//! callback reconciliation after this component returns a typed effect.

use anyhow::Result;

use super::ScriptVm;
use crate::{
    page_task_queue::{RendererDedicatedWorkerClientEvent, RendererDedicatedWorkerMessageEvent},
    runtime::AuthorizedCurrentPageDedicatedWorkerClientEvent,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DedicatedWorkerClientEventBodyEffect {
    /// A non-callback Worker state transition, such as ScriptLoaded or relay
    /// drainage, changed the exact current Worker record.
    StateTransitionApplied,
    /// The exact Worker or owning Window dispatched callback-visible work.
    CallbackDispatched,
    /// The exact Worker remained current, but this event had no matching
    /// listener and therefore produced no callback follow-up.
    CurrentTargetHadNoCallback,
    /// Page arbitration authorized the target, but it disappeared before the
    /// body could apply its state transition or enter its realm.
    CurrentTargetDisappeared,
}

#[derive(Clone, Copy)]
struct AuthorizedWorkerWrapperTarget<'s> {
    owner: crate::native_bridge::OwnerDispatchScope,
    wrapper: v8::Local<'s, v8::Object>,
}

impl ScriptVm {
    pub(crate) fn current_dedicated_worker_client_event_identity(
        &self,
        worker_id: crate::types::DedicatedWorkerId,
    ) -> Option<crate::native_bridge::WindowExecutionContextIdentity> {
        self._context_host
            .borrow()
            .current_dedicated_worker_client_event_identity(worker_id)
    }

    /// Apply one exact DedicatedWorker client-event body.
    ///
    /// The selected Page-task dispatcher owns the ordinary task-end
    /// checkpoint and callback follow-up. Worker error dispatch retains the
    /// established inner checkpoint used to determine cancellation before
    /// propagating an uncanceled error; that does not replace the selected
    /// task's completion boundary.
    pub(crate) fn apply_current_dedicated_worker_client_event_body(
        &mut self,
        authorized: AuthorizedCurrentPageDedicatedWorkerClientEvent,
    ) -> Result<DedicatedWorkerClientEventBodyEffect> {
        let task = authorized.into_task();
        let owner = task.owner();
        let worker_id = owner.worker_id();
        match task.into_event() {
            RendererDedicatedWorkerClientEvent::ScriptLoaded {
                script_url,
                script_source,
                network_response,
                script_kind,
                secure_context,
                response_referrer_policy,
                network_partition_key,
                policy_context,
                content_security_policies,
                content_security_report_only_policies,
                content_security_reporting_endpoints,
            } => {
                let recorded = self
                    ._context_host
                    .borrow_mut()
                    .record_dedicated_worker_target_script_loaded(
                        worker_id,
                        script_url.clone(),
                        network_response,
                    );
                if !recorded {
                    return Ok(DedicatedWorkerClientEventBodyEffect::CurrentTargetDisappeared);
                }
                let handled = self._context_host.borrow_mut().finish_loading_worker(
                    worker_id,
                    script_url,
                    script_source,
                    script_kind,
                    secure_context,
                    response_referrer_policy,
                    network_partition_key,
                    policy_context,
                    content_security_policies,
                    content_security_report_only_policies,
                    content_security_reporting_endpoints,
                );
                Ok(if handled {
                    DedicatedWorkerClientEventBodyEffect::StateTransitionApplied
                } else {
                    DedicatedWorkerClientEventBodyEffect::CurrentTargetDisappeared
                })
            }
            RendererDedicatedWorkerClientEvent::ClientSourceDrained => {
                let handled = self
                    ._context_host
                    .borrow_mut()
                    .mark_dedicated_worker_client_source_drained(worker_id);
                Ok(if handled {
                    DedicatedWorkerClientEventBodyEffect::StateTransitionApplied
                } else {
                    DedicatedWorkerClientEventBodyEffect::CurrentTargetDisappeared
                })
            }
            event @ (RendererDedicatedWorkerClientEvent::ScriptLoadFailed { .. }
            | RendererDedicatedWorkerClientEvent::Message(_)) => {
                if let RendererDedicatedWorkerClientEvent::ScriptLoadFailed {
                    script_url,
                    error_message,
                    network_response,
                    ..
                } = &event
                {
                    let recorded = self
                        ._context_host
                        .borrow_mut()
                        .record_dedicated_worker_target_script_load_failed(
                            worker_id,
                            script_url.clone(),
                            error_message.clone(),
                            network_response.clone(),
                        );
                    if !recorded {
                        return Ok(DedicatedWorkerClientEventBodyEffect::CurrentTargetDisappeared);
                    }
                }
                let context_host = self._context_host.clone();
                self.with_default_context_scope(|scope, host_ptr| {
                    let target = context_host
                        .borrow()
                        .authorized_dedicated_worker_dispatch_target(
                            scope,
                            worker_id,
                        owner.execution_context(),
                    );
                    let Some((dispatch_scope, context, worker)) = target else {
                        return Ok(
                            DedicatedWorkerClientEventBodyEffect::CurrentTargetDisappeared,
                        );
                    };
                    let scope = &mut v8::ContextScope::new(scope, context);
                    let target = AuthorizedWorkerWrapperTarget {
                        owner: dispatch_scope,
                        wrapper: worker,
                    };
                    let previous_owner_context = target.owner.enter(scope);
                    let dispatched = match event {
                        RendererDedicatedWorkerClientEvent::ScriptLoadFailed {
                            script_url,
                            error_message,
                            ..
                        } => {
                            super::dedicated_worker_error_dispatch::dispatch_script_load_failure(
                                scope,
                                target.wrapper,
                                &error_message,
                                &script_url,
                            );
                            context_host
                                .borrow_mut()
                                .release_loading_worker_service_worker_client(worker_id);
                            context_host.borrow_mut().forget_worker(worker_id);
                            true
                        }
                        RendererDedicatedWorkerClientEvent::Message(message) => {
                            let parent_message = match message {
                                RendererDedicatedWorkerMessageEvent::Message(payload) => {
                                    crate::worker::WorkerToParentMessage::Post(payload)
                                }
                                RendererDedicatedWorkerMessageEvent::Error {
                                    message,
                                    filename,
                                    lineno,
                                    colno,
                                    event_kind,
                                    phase,
                                    source,
                                } => crate::worker::WorkerToParentMessage::Error {
                                    message,
                                    filename,
                                    lineno,
                                    colno,
                                    event_kind,
                                    phase,
                                    source,
                                },
                            };
                            let has_message_listener = matches!(
                                parent_message,
                                crate::worker::WorkerToParentMessage::Post(_)
                            ) && crate::context_bootstrap::worker_has_message_delivery_listener(
                                scope,
                                target.wrapper,
                            );
                            match &parent_message {
                                crate::worker::WorkerToParentMessage::Post(_) => {
                                    let _ = crate::context_bootstrap::dispatch_worker_event(
                                        scope,
                                        target.wrapper,
                                        &parent_message,
                                    );
                                    has_message_listener
                                }
                                crate::worker::WorkerToParentMessage::Error {
                                    message,
                                    filename,
                                    lineno,
                                    colno,
                                    event_kind,
                                    phase,
                                    ..
                                } => {
                                    if let Err(error) = super::dedicated_worker_error_dispatch::dispatch_error_and_maybe_propagate(
                                        scope,
                                        host_ptr,
                                        target.wrapper,
                                        super::dedicated_worker_error_dispatch::DedicatedWorkerError {
                                            message,
                                            filename,
                                            lineno: *lineno,
                                            colno: *colno,
                                            event_kind: *event_kind,
                                            phase: *phase,
                                        },
                                    )
                                    {
                                        target.owner.restore(scope, previous_owner_context);
                                        return Err(error);
                                    }
                                    true
                                }
                                _ => unreachable!(
                                    "DedicatedWorker client-event payload admitted a bridge message"
                                ),
                            }
                        }
                        RendererDedicatedWorkerClientEvent::ScriptLoaded { .. }
                        | RendererDedicatedWorkerClientEvent::ClientSourceDrained => {
                            unreachable!(
                                "non-dispatch DedicatedWorker event entered V8 dispatch path"
                            )
                        }
                    };
                    if dispatched {
                        target.owner.defer_restore(scope, previous_owner_context);
                    } else {
                        target.owner.restore(scope, previous_owner_context);
                    }
                    Ok(if dispatched {
                        DedicatedWorkerClientEventBodyEffect::CallbackDispatched
                    } else {
                        DedicatedWorkerClientEventBodyEffect::CurrentTargetHadNoCallback
                    })
                })
            }
        }
    }
}
