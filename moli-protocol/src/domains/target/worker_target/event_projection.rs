use anyhow::Result;

use super::super::session_disposal::{PreparedSessionDisposal, SessionDisposalIdentity};
use super::*;
use crate::conn::{CommandWorkflowContext, ProtocolWorkflowDriver, WorkerRuntimeScope};
use crate::domains::command_output::CommandOutputPlan;
use crate::domains::runtime::PreparedWorkerBindingReplay;

pub(crate) struct WorkerTargetOutputProjection {
    events: Vec<WorkerTargetLifecycleOutput>,
}

impl WorkerTargetOutputProjection {
    pub(crate) fn new(events: Vec<WorkerTargetLifecycleOutput>) -> Self {
        Self { events }
    }

    pub(in crate::domains) fn take(payloads: &mut ProtocolOutputPayloads) -> Option<Self> {
        payloads
            .target_mut()
            .and_then(TargetPreparedOutputSlot::take_worker_target_lifecycle_outputs)
            .map(Self::new)
    }

    pub(crate) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        let mut first_error = None;
        for event in self.events {
            let result: Result<()> = async {
                match event {
                WorkerTargetLifecycleOutput::DedicatedWorkerDetached {
                    target_delta,
                    cleanup_plan,
                } => {
                    let pending = context
                        .with_state(driver, move |state, context| {
                            state.prepare_dedicated_worker_projection_disposal(
                                target_delta,
                                cleanup_plan,
                                context,
                            )
                        })
                        ??;
                    let mut background = Vec::new();
                    let mut protocol = Vec::new();
                    let result = pending
                        .disposal
                        .run(driver, &mut background, &mut protocol)
                        .await;
                    background.extend(protocol);
                    retain_events(context, background);
                    let failed = result.is_err();
                    context
                        .with_state(driver, move |state, context| {
                            state.finish_dedicated_worker_projection_disposal(
                                pending.identity,
                                pending.detachment,
                                failed,
                                context,
                            )
                        })
                        ??;
                    result?;
                }
                WorkerTargetLifecycleOutput::SharedWorkerDetached {
                    retirement,
                    cleanup_plan,
                } => {
                    assert_eq!(cleanup_plan.target_id(), retirement.identity().target_id());
                    assert_eq!(
                        cleanup_plan.session_id(),
                        retirement.identity().session_id()
                    );
                    let Some((pending, retirement)) = context
                        .with_state(driver, move |state, _| {
                            if !retirement.is_current() { return Ok(None); }
                            state.prepare_removed_worker_projection(cleanup_plan)
                                .map(|pending| Some((pending, retirement)))
                        })
                        ?? else { return Ok(()); };
                    pending.wait().await;
                    context
                        .with_state(driver, move |state, context| {
                            state.finish_removed_worker_projection(pending, context);
                            retirement.retire();
                        })
                        ?;
                }
                WorkerTargetLifecycleOutput::ServiceWorkerDetached {
                    retirement,
                    cleanup_plan,
                } => {
                    assert_eq!(cleanup_plan.target_id(), retirement.identity().target_id());
                    assert_eq!(
                        cleanup_plan.session_id(),
                        retirement.identity().session_id()
                    );
                    let Some((pending, retirement)) = context
                        .with_state(driver, move |state, _| {
                            if !retirement.is_current() { return Ok(None); }
                            state.prepare_removed_worker_projection(cleanup_plan)
                                .map(|pending| Some((pending, retirement)))
                        })
                        ?? else { return Ok(()); };
                    pending.wait().await;
                    context
                        .with_state(driver, move |state, context| {
                            state.finish_removed_worker_projection(pending, context);
                            retirement.retire();
                        })
                        ?;
                }
                event @ (WorkerTargetLifecycleOutput::ServiceWorkerRuntimeInspectorMessages {
                    ..
                }
                | WorkerTargetLifecycleOutput::SharedWorkerRuntimeInspectorMessages {
                    ..
                }
                | WorkerTargetLifecycleOutput::DedicatedWorkerRuntimeInspectorMessages {
                    ..
                }) => {
                    if let Some(replay) = context
                        .with_state(driver, move |state, context| {
                            state.prepare_worker_inspector_projection(event, context)
                        })
                        ?
                    {
                        replay.run(driver).await;
                    }
                }
                event => {
                    context
                        .with_state(driver, move |state, context| {
                            state.project_worker_target_event(event, context)
                        })
                        ?;
                }
            }
                Ok(())
            }
            .await;
            if let Err(error) = result {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

pub(crate) struct PreparedDedicatedWorkerProjectionDisposal {
    disposal: PreparedSessionDisposal,
    identity: SessionDisposalIdentity,
    detachment: TargetSessionDetachCleanupPlan,
}

impl PreparedDedicatedWorkerProjectionDisposal {
    pub(crate) fn prepare(
        conn: &mut CdpConnection,
        delta: Option<PreparedTargetHostDelta>,
        cleanup: TargetSessionDetachCleanupPlan,
        context: &mut CommandWorkflowContext,
    ) -> Result<Self> {
        if let Some(delta) = delta {
            retain_events(
                context,
                conn.prepared_target_info_changed_event_plan_for_discovery_owners(delta),
            );
        }
        let (plan, detachment) =
            super::super::session_disposal::prepare_session_disposal(conn, cleanup)?;
        let disposal = PreparedSessionDisposal::prepare(conn, plan);
        let identity = disposal.identity();
        Ok(Self {
            disposal,
            identity,
            detachment,
        })
    }

    pub(crate) fn finish(
        conn: &mut CdpConnection,
        identity: SessionDisposalIdentity,
        detachment: TargetSessionDetachCleanupPlan,
        failed: bool,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        if !identity.is_current(conn) {
            return Ok(());
        }
        if failed {
            crate::domains::session::dispose_closed_handlers_sync(
                conn,
                identity.plan().session_id(),
            );
        }
        conn.commit_session_disposal(identity.plan())?;
        retain_events(
            context,
            conn.commit_target_session_detachment_after_prepared_state_delta_event_plan(detachment),
        );
        Ok(())
    }
}

pub(crate) fn retain_events(
    context: &mut CommandWorkflowContext,
    events: impl IntoIterator<Item = BackgroundProtocolEvent>,
) {
    let mut output = CommandOutputPlan::default();
    output.extend_background_events(events);
    context.retain_output(output);
}

pub(crate) fn prepare_worker_inspector_projection(
    conn: &mut CdpConnection,
    event: WorkerTargetLifecycleOutput,
    context: &mut CommandWorkflowContext,
) -> Option<PreparedWorkerBindingReplay> {
    let mut output = Vec::new();
    let session = match event {
        WorkerTargetLifecycleOutput::ServiceWorkerRuntimeInspectorMessages {
            runtime,
            background_events,
            response_events,
            pending_runtime_exceptions,
        } => {
            if !runtime.is_current() {
                return None;
            }
            output.extend(background_events);
            output.extend(response_events);
            if let Some((messages, start, _)) = pending_runtime_exceptions {
                output.extend(runtime_exception_thrown_events(
                    runtime.session_id(),
                    &messages,
                    start,
                ));
            }
            runtime.session_id().to_owned()
        }
        WorkerTargetLifecycleOutput::SharedWorkerRuntimeInspectorMessages {
            attachment,
            messages,
        } => {
            if !attachment.is_current() {
                return None;
            }
            let session = attachment.session_id();
            let mut responses = Vec::new();
            let current_response_seen = route_worker_runtime_inspector_messages_into(
                conn,
                messages,
                session,
                &mut responses,
                &mut output,
            );
            debug_assert!(!current_response_seen);
            output.extend(responses);
            session.to_owned()
        }
        WorkerTargetLifecycleOutput::DedicatedWorkerRuntimeInspectorMessages {
            browser_context_id,
            renderer_instance_id,
            target_id,
            session_id,
            messages,
        } => {
            if !dedicated_worker_target_is_current(
                conn,
                &browser_context_id,
                renderer_instance_id,
                &target_id,
            ) {
                return None;
            }
            let mut responses = Vec::new();
            let current_response_seen = route_worker_runtime_inspector_messages_into(
                conn,
                messages,
                &session_id,
                &mut responses,
                &mut output,
            );
            debug_assert!(!current_response_seen);
            output.extend(responses);
            session_id
        }
        _ => unreachable!("only Worker Inspector messages use this projection"),
    };
    retain_events(context, output);
    WorkerRuntimeScope::capture(conn, Some(&session))
        .ok()
        .and_then(|scope| PreparedWorkerBindingReplay::prepare(conn, scope))
}

pub(crate) fn project_worker_target_event(
    conn: &mut CdpConnection,
    event: WorkerTargetLifecycleOutput,
    context: &mut CommandWorkflowContext,
) {
    let mut side_effects = events::TargetProtocolSideEffects::default();
    for event in std::iter::once(event) {
        match event {
            WorkerTargetLifecycleOutput::DedicatedWorkerEvents {
                browser_context_id,
                renderer_instance_id,
                target_id,
                events,
            } => {
                if dedicated_worker_target_is_current(
                    conn,
                    &browser_context_id,
                    renderer_instance_id,
                    &target_id,
                ) {
                    side_effects.extend_background_events(events);
                }
            }
            WorkerTargetLifecycleOutput::DedicatedWorkerConsoleMessages {
                browser_context_id,
                renderer_instance_id,
                target_id,
                session_id,
                console_messages,
                console_end,
            } => {
                if !dedicated_worker_target_is_current(
                    conn,
                    &browser_context_id,
                    renderer_instance_id,
                    &target_id,
                ) || !conn
                    .browser_context_by_id(&browser_context_id)
                    .and_then(|context| context.dedicated_worker_targets.get(&renderer_instance_id))
                    .is_some_and(|target| target.is_session(&session_id))
                {
                    continue;
                }
                side_effects.extend_background_events(console_message_added_events(
                    &session_id,
                    &console_messages,
                ));
                if let Some(target) = conn
                    .browser_context_by_id_mut(&browser_context_id)
                    .and_then(|context| {
                        context
                            .dedicated_worker_targets
                            .get_mut(&renderer_instance_id)
                    })
                    .filter(|target| target.target_id == target_id)
                    && !console_messages.is_empty()
                {
                    target.mark_console_domain_emitted(&session_id, console_end);
                }
            }
            WorkerTargetLifecycleOutput::DedicatedWorkerCreated {
                browser_context_id,
                renderer_instance_id,
                target_delta,
            } => {
                if dedicated_worker_target_is_current(
                    conn,
                    &browser_context_id,
                    renderer_instance_id,
                    target_delta.target_id(),
                ) {
                    side_effects.extend_background_events(
                        conn.prepared_target_host_delta_event_plan(target_delta),
                    );
                }
            }
            WorkerTargetLifecycleOutput::DedicatedWorkerInfoChanged {
                browser_context_id,
                renderer_instance_id,
                target_id,
                target_delta,
            } => {
                if dedicated_worker_target_is_current(
                    conn,
                    &browser_context_id,
                    renderer_instance_id,
                    &target_id,
                ) {
                    side_effects.extend_background_events(
                        conn.prepared_target_host_delta_event_plan(target_delta),
                    );
                }
            }
            WorkerTargetLifecycleOutput::DedicatedWorkerAttached {
                browser_context_id,
                renderer_instance_id,
                target_id,
                session_id,
                prepared_attach,
            } => {
                let current = conn
                    .browser_context_by_id(&browser_context_id)
                    .and_then(|context| context.dedicated_worker_targets.get(&renderer_instance_id))
                    .is_some_and(|target| {
                        target.target_id == target_id && target.is_session(&session_id)
                    });
                if current {
                    side_effects.extend_background_events(
                        conn.commit_prepared_dedicated_worker_attach_event_plan(prepared_attach),
                    );
                }
            }
            WorkerTargetLifecycleOutput::DedicatedWorkerDestroyed {
                browser_context_id,
                renderer_instance_id,
                target_id,
                target_delta,
            } => {
                let removed = conn
                    .browser_context_by_id_mut(&browser_context_id)
                    .and_then(|context| {
                        let target = context
                            .dedicated_worker_targets
                            .get(&renderer_instance_id)?;
                        (target.target_id == target_id).then(|| {
                            context.remove_dedicated_worker_target_by_renderer_instance(
                                renderer_instance_id,
                            )
                        })
                    })
                    .flatten();
                if removed.is_some()
                    && let Some(delta) = target_delta
                {
                    side_effects.extend_background_events(
                        conn.prepared_target_host_delta_event_plan(delta),
                    );
                }
            }
            WorkerTargetLifecycleOutput::SharedWorkerAttachmentEvents { attachment, events } => {
                if attachment.is_current() {
                    side_effects.extend_background_events(events);
                }
            }
            WorkerTargetLifecycleOutput::SharedWorkerCreated { target_delta } => {
                side_effects.extend_background_events(
                    conn.prepared_target_host_delta_event_plan(target_delta),
                );
            }
            WorkerTargetLifecycleOutput::SharedWorkerAttached {
                attachment,
                prepared_attach,
            } => {
                if !attachment.is_current() {
                    continue;
                }
                side_effects.extend_background_events(
                    conn.commit_prepared_attach_event_plan(prepared_attach),
                );
            }
            WorkerTargetLifecycleOutput::ServiceWorkerVersionEvents { version, events } => {
                if version.is_current() {
                    side_effects.extend_background_events(events);
                }
            }
            WorkerTargetLifecycleOutput::ServiceWorkerAttachmentEvents { attachment, events } => {
                if attachment.is_current() {
                    side_effects.extend_background_events(events);
                }
            }
            WorkerTargetLifecycleOutput::ServiceWorkerRunEvents { run, events } => {
                if run.is_current() {
                    side_effects.extend_background_events(events);
                }
            }
            WorkerTargetLifecycleOutput::ServiceWorkerRuntimeEvents { runtime, events } => {
                if runtime.is_current() {
                    side_effects.extend_background_events(events);
                }
            }
            WorkerTargetLifecycleOutput::ServiceWorkerCreated {
                version,
                target_delta,
            } => {
                if version.is_current() {
                    side_effects.extend_background_events(
                        conn.prepared_target_host_delta_event_plan(target_delta),
                    );
                }
            }
            WorkerTargetLifecycleOutput::ServiceWorkerAttached {
                attachment,
                prepared_attach,
            } => {
                if attachment.is_current() {
                    side_effects.extend_background_events(
                        conn.commit_prepared_attach_event_plan(prepared_attach),
                    );
                }
            }
            WorkerTargetLifecycleOutput::SharedWorkerDestroyed { target_delta } => {
                side_effects.extend_background_events(
                    conn.prepared_target_host_delta_event_plan(target_delta),
                );
            }
            WorkerTargetLifecycleOutput::ServiceWorkerRunRetired { retirement } => {
                assert!(
                    retirement.is_current(),
                    "service-worker run retirement must be consumed exactly once in source order"
                );
                retirement.retire();
            }
            WorkerTargetLifecycleOutput::ServiceWorkerDestroyed {
                retirement,
                target_delta,
            } => {
                assert!(
                    retirement.is_current(),
                    "service-worker version retirement must be consumed exactly once"
                );
                if let Some(target_delta) = target_delta {
                    assert_eq!(
                        target_delta.target_id(),
                        retirement.identity().target_id(),
                        "service-worker destruction must retain its exact version target"
                    );
                    side_effects.extend_background_events(
                        conn.prepared_target_host_delta_event_plan(target_delta),
                    );
                }
                retirement.retire();
            }
            WorkerTargetLifecycleOutput::ServiceWorkerConsoleMessages {
                runtime,
                messages,
                console_end: _,
            } => {
                if runtime.is_current() {
                    side_effects.extend_background_events(console_message_added_events(
                        runtime.session_id(),
                        &messages,
                    ));
                }
            }
            WorkerTargetLifecycleOutput::SharedWorkerConsoleMessages {
                attachment,
                messages,
                console_end,
            } => {
                if !attachment.is_current() {
                    continue;
                }
                side_effects.extend_background_events(console_message_added_events(
                    attachment.session_id(),
                    &messages,
                ));
                mark_exact_shared_worker_console_domain_emitted(conn, &attachment, console_end);
            }
            WorkerTargetLifecycleOutput::ServiceWorkerRuntimeExceptionMessages {
                runtime,
                messages,
                exception_start,
                exception_end: _,
            } => {
                if runtime.is_current() {
                    side_effects.extend_background_events(runtime_exception_thrown_events(
                        runtime.session_id(),
                        &messages,
                        exception_start,
                    ));
                }
            }
            WorkerTargetLifecycleOutput::ServiceWorkerFetchDiagnostics {
                runtime,
                diagnostics,
                diagnostic_start,
                diagnostic_end: _,
            } => {
                if runtime.is_current() {
                    side_effects.extend_background_events(service_worker_fetch_diagnostic_events(
                        runtime.session_id(),
                        runtime.target_id(),
                        &diagnostics,
                        diagnostic_start,
                    ));
                }
            }
            WorkerTargetLifecycleOutput::DedicatedWorkerDetached { .. }
            | WorkerTargetLifecycleOutput::SharedWorkerDetached { .. }
            | WorkerTargetLifecycleOutput::ServiceWorkerDetached { .. }
            | WorkerTargetLifecycleOutput::ServiceWorkerRuntimeInspectorMessages { .. }
            | WorkerTargetLifecycleOutput::SharedWorkerRuntimeInspectorMessages { .. }
            | WorkerTargetLifecycleOutput::DedicatedWorkerRuntimeInspectorMessages { .. } => {
                unreachable!("waiting Worker projections use the ordinary async flow")
            }
        }
    }
    retain_events(context, side_effects.into_background_events());
}

pub(super) fn route_worker_runtime_inspector_messages_into(
    conn: &mut CdpConnection,
    messages: Vec<RendererRuntimeInspectorMessage>,
    session_id: &str,
    response_events: &mut Vec<BackgroundProtocolEvent>,
    background_events: &mut Vec<BackgroundProtocolEvent>,
) -> bool {
    let owner = crate::conn::CommandOwnerScope::for_session(session_id);
    conn.route_renderer_runtime_inspector_messages_for_owner_with_background_events_into(
        messages,
        None,
        &owner,
        response_events,
        background_events,
    )
}

pub(super) fn console_message_added_events(
    session_id: &str,
    messages: &[RuntimeConsoleMessageSnapshot],
) -> Vec<BackgroundProtocolEvent> {
    messages
        .iter()
        .map(|message| {
            let (level, text) = console_message_level_and_text(&message.message);
            console_message_added_background_event(Some(session_id), "console-api", level, text, "")
        })
        .collect()
}

pub(super) fn runtime_exception_thrown_events(
    session_id: &str,
    messages: &[ServiceWorkerRuntimeExceptionSnapshot],
    exception_start: usize,
) -> Vec<BackgroundProtocolEvent> {
    let base_timestamp = monotonic_timestamp_seconds();
    messages
        .iter()
        .enumerate()
        .map(|(offset, message)| {
            let exception_index = exception_start + offset;
            runtime_exception_thrown_background_event(
                Some(session_id),
                None,
                &message.message.message,
                &message.message.filename,
                message.execution_context_id,
                exception_index,
                base_timestamp + ((offset + 1) as f64 * 0.000_001),
                Some(u64::from(message.message.lineno.saturating_sub(1))),
                Some(u64::from(message.message.colno.saturating_sub(1))),
            )
        })
        .collect()
}
