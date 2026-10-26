use super::*;

pub(in crate::domains) async fn project_worker_target_output_async(
    output: ProtocolOutputSlot,
    conn: &mut CdpConnection,
    context: &mut ProtocolOutputProjectionContext<'_>,
    prepared_outputs: &mut ProtocolOutputPayloads,
) {
    match output {
        ProtocolOutputSlot::SharedWorkerTargetLifecycle
        | ProtocolOutputSlot::ServiceWorkerTargetLifecycle
        | ProtocolOutputSlot::DedicatedWorkerTargetLifecycle => {}
        _ => panic!("non-Target output routed through the Target projector: {output:?}"),
    }
    emit_target_lifecycle_events(conn, context, prepared_outputs).await;
}

pub(super) async fn emit_target_lifecycle_events(
    conn: &mut CdpConnection,
    context: &mut ProtocolOutputProjectionContext<'_>,
    prepared_outputs: &mut ProtocolOutputPayloads,
) {
    let Some(events) = prepared_outputs
        .target_mut()
        .and_then(TargetPreparedOutputSlot::take_worker_target_lifecycle_outputs)
    else {
        return;
    };
    let mut side_effects = events::TargetProtocolSideEffects::default();
    for event in events {
        let event = match commit_dedicated_worker_retirement_output_async(conn, event).await {
            Ok(events) => {
                side_effects.extend_background_events(events);
                continue;
            }
            Err(event) => event,
        };
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
            WorkerTargetLifecycleOutput::DedicatedWorkerDetached { .. }
            | WorkerTargetLifecycleOutput::DedicatedWorkerDestroyed { .. } => {
                unreachable!("DedicatedWorker retirement outputs are committed before projection")
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
            WorkerTargetLifecycleOutput::SharedWorkerDetached {
                retirement,
                cleanup_plan,
            } => {
                if !retirement.is_current() {
                    continue;
                }
                assert_eq!(
                    cleanup_plan.target_id(),
                    retirement.identity().target_id(),
                    "shared-worker detach plan must retain its exact target"
                );
                assert_eq!(
                    cleanup_plan.session_id(),
                    retirement.identity().session_id(),
                    "shared-worker detach plan must retain its exact attachment"
                );
                let event_plan =
                    super::super::session_disposal::dispose_removed_worker_session_async(
                        conn,
                        cleanup_plan,
                    )
                    .await
                    .expect("retired shared-worker session cleanup should succeed");
                side_effects.extend_background_events(event_plan);
                retirement.retire();
            }
            WorkerTargetLifecycleOutput::ServiceWorkerDetached {
                retirement,
                cleanup_plan,
            } => {
                if !retirement.is_current() {
                    continue;
                }
                assert_eq!(
                    cleanup_plan.target_id(),
                    retirement.identity().target_id(),
                    "service-worker detach plan must retain its exact version target"
                );
                assert_eq!(
                    cleanup_plan.session_id(),
                    retirement.identity().session_id(),
                    "service-worker detach plan must retain its exact attachment"
                );
                let event_plan =
                    super::super::session_disposal::dispose_removed_worker_session_async(
                        conn,
                        cleanup_plan,
                    )
                    .await
                    .expect("retired service-worker session cleanup should succeed");
                side_effects.extend_background_events(event_plan);
                retirement.retire();
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
            WorkerTargetLifecycleOutput::ServiceWorkerRuntimeInspectorMessages {
                runtime,
                background_events,
                response_events,
                pending_runtime_exceptions,
            } => {
                if !runtime.is_current() {
                    continue;
                }
                let session_id = runtime.session_id();
                side_effects.extend_background_events(background_events);
                if service_worker_runtime_is_registry_current(conn, &runtime) {
                    replay_shared_worker_runtime_bindings_for_session_async(conn, Some(session_id))
                        .await;
                }
                side_effects.extend_background_events(response_events);
                if let Some((messages, exception_start, _exception_end)) =
                    pending_runtime_exceptions
                {
                    side_effects.extend_background_events(runtime_exception_thrown_events(
                        session_id,
                        &messages,
                        exception_start,
                    ));
                }
            }
            WorkerTargetLifecycleOutput::SharedWorkerRuntimeInspectorMessages {
                attachment,
                messages,
            } => {
                if !attachment.is_current() {
                    continue;
                }
                let session_id = attachment.session_id();
                let mut response_events = Vec::new();
                let mut background_events = Vec::new();
                let current_response_seen = route_worker_runtime_inspector_messages_into(
                    conn,
                    messages,
                    session_id,
                    &mut response_events,
                    &mut background_events,
                );
                debug_assert!(!current_response_seen);
                side_effects.extend_background_events(background_events);
                replay_shared_worker_runtime_bindings_for_session_async(conn, Some(session_id))
                    .await;
                side_effects.extend_background_events(response_events);
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
                    continue;
                }
                let mut response_events = Vec::new();
                let mut background_events = Vec::new();
                let current_response_seen = route_worker_runtime_inspector_messages_into(
                    conn,
                    messages,
                    &session_id,
                    &mut response_events,
                    &mut background_events,
                );
                debug_assert!(!current_response_seen);
                side_effects.extend_background_events(background_events);
                replay_shared_worker_runtime_bindings_for_session_async(conn, Some(&session_id))
                    .await;
                side_effects.extend_background_events(response_events);
            }
        }
    }
    for event in side_effects.into_background_events() {
        context.command.push_protocol_event(event);
    }
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
