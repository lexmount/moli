use super::*;

pub(super) fn register_shared_worker_target(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    owner_target_id: Option<String>,
    info: RendererSharedWorkerTargetInfo,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    if conn
        .browser_context_by_id(browser_context_id)
        .and_then(|context| context.shared_worker_target_id_for_renderer_instance(info.instance_id))
        .is_some()
    {
        return outputs;
    }
    let target_id = conn.gen_target_id();
    let should_emit_created = conn.has_any_target_discovery();
    let auto_attach_owners = shared_worker_auto_attach_owner_sessions(conn);
    let attached_sessions = auto_attach_owners
        .iter()
        .map(|owner| {
            (
                owner.clone(),
                conn.gen_session_id(),
                conn.auto_attach_owner_waits_for_debugger_on_start(owner.as_deref()),
            )
        })
        .collect::<Vec<_>>();
    let created_snapshot = {
        let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
            return outputs;
        };
        context.insert_shared_worker_target(SharedWorkerTargetState::new(
            info.owner_local_host_id,
            info.instance_id,
            target_id.clone(),
            owner_target_id,
            info.url,
            info.name,
        ));
        if should_emit_created {
            let snapshot = context.devtools_target_info(&target_id);
            debug_assert!(snapshot.is_some());
            snapshot
        } else {
            None
        }
    };
    let mut attached_outputs = Vec::new();
    for (owner_session_id, session_id, waiting_for_debugger) in attached_sessions {
        if let Some(target_info) = conn
            .prepare_auto_attached_shared_worker_session_binding_info_in_browser_context(
                browser_context_id,
                &target_id,
                session_id.clone(),
            )
        {
            let attachment = conn
                .browser_context_by_id(browser_context_id)
                .and_then(|context| context.shared_worker_target(&target_id))
                .and_then(|target| {
                    target.protocol_attachment_identity(browser_context_id, &session_id)
                })
                .expect("new shared-worker session must expose its exact attachment identity");
            let prepared_session = TargetAttachSessionCommit::auto_attached(
                session_id,
                owner_session_id,
                CdpSessionRoute::SharedWorkerTarget {
                    browser_context_id: browser_context_id.to_owned(),
                    target_id: target_id.clone(),
                },
                waiting_for_debugger,
            );
            assert!(
                matches!(
                    prepared_session.route(),
                    CdpSessionRoute::SharedWorkerTarget {
                        browser_context_id: route_browser_context_id,
                        target_id: route_target_id,
                    } if route_browser_context_id == browser_context_id
                        && route_target_id == &target_id
                ),
                "shared-worker auto-attach must freeze its exact target route at capture"
            );
            attached_outputs.push((
                attachment,
                PreparedTargetAttach::new(target_id.clone(), target_info, [prepared_session]),
            ));
        }
    }
    if let Some(target_info) = created_snapshot {
        outputs.push(WorkerTargetLifecycleOutput::SharedWorkerCreated {
            target_delta: PreparedTargetHostDelta::created(target_id.clone(), Some(target_info)),
        });
    }
    for (attachment, prepared_attach) in attached_outputs {
        outputs.push(WorkerTargetLifecycleOutput::SharedWorkerAttached {
            attachment,
            prepared_attach,
        });
    }
    outputs
}

pub(super) fn remove_shared_worker_target(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_instance_id: SharedWorkerInstanceId,
) -> TargetPreparedOutputs {
    remove_shared_worker_target_with_reason(
        conn,
        browser_context_id,
        renderer_instance_id,
        "Target closed",
    )
}

pub(super) fn remove_shared_worker_target_with_reason(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_instance_id: SharedWorkerInstanceId,
    reason: &'static str,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let should_emit_destroyed = conn.has_any_target_discovery();
    let target_id = {
        let Some(context) = conn.browser_context_by_id(browser_context_id) else {
            return outputs;
        };
        let Some(target_id) = context
            .shared_worker_target_id_for_renderer_instance(renderer_instance_id)
            .map(str::to_owned)
        else {
            return outputs;
        };
        target_id
    };
    let destroyed_delta = should_emit_destroyed
        .then(|| conn.prepare_destroyed_target_host_delta(&target_id))
        .flatten();
    let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
        return outputs;
    };
    let Some(mut target) =
        context.remove_shared_worker_target_by_renderer_instance(renderer_instance_id)
    else {
        return outputs;
    };
    let session_ids = target.session_ids();
    let mut pending_await_direct_outputs = Vec::new();
    let mut pending_await_claimed_outputs = Vec::new();
    for session_id in &session_ids {
        let attachment = target
            .protocol_attachment_identity(browser_context_id, session_id)
            .expect("removed shared-worker session must retain its exact attachment identity");
        let mut pending_await_direct_events = Vec::new();
        let mut pending_await_claimed_events = Vec::new();
        conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
            &mut pending_await_direct_events,
            &mut pending_await_claimed_events,
            Some(session_id),
            reason,
        );
        CdpConnection::fail_pending_inspector_awaits_from_shared_worker_target_session_background_events_into(
            &mut pending_await_direct_events,
            &mut target,
            session_id,
            reason,
        );
        if !pending_await_direct_events.is_empty() {
            pending_await_direct_outputs.push((attachment.clone(), pending_await_direct_events));
        }
        if !pending_await_claimed_events.is_empty() {
            pending_await_claimed_outputs.push((attachment, pending_await_claimed_events));
        }
    }
    for (attachment, events) in pending_await_direct_outputs
        .into_iter()
        .chain(pending_await_claimed_outputs)
    {
        outputs
            .push(WorkerTargetLifecycleOutput::SharedWorkerAttachmentEvents { attachment, events });
    }
    for session_id in session_ids {
        let retirement = target
            .take_protocol_attachment_retirement(browser_context_id, &session_id)
            .expect("removed shared-worker session must transfer its attachment scope");
        outputs.push(WorkerTargetLifecycleOutput::SharedWorkerDetached {
            cleanup_plan: TargetSessionDetachCleanupPlan::new(
                target_id.clone(),
                session_id,
                None,
                None,
            ),
            retirement,
        });
    }
    if let Some(target_delta) = destroyed_delta {
        outputs.push(WorkerTargetLifecycleOutput::SharedWorkerDestroyed { target_delta });
    }
    outputs
}

pub(in crate::domains::target) async fn close_shared_worker_target_for_target_close_async(
    conn: &mut CdpConnection,
    target_id: &str,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> bool {
    let Some(prepared) = PreparedSharedWorkerRetirement::prepare_close(conn, target_id) else {
        return false;
    };
    prepared
        .run_inline(conn, command_context)
        .await
        .expect("inline SharedWorker retirement retains its original protocol owner");
    true
}

pub(in crate::domains::target) async fn close_dedicated_worker_target_for_target_close_async(
    conn: &mut CdpConnection,
    target_id: &str,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> bool {
    let Some((browser_context_id, renderer_runtime, instance_id)) =
        conn.browser_context.as_ref().and_then(|context| {
            let target = context.dedicated_worker_target(target_id)?;
            Some((
                context.id.clone(),
                context.renderer_runtime(),
                target.renderer_instance_id,
            ))
        })
    else {
        return false;
    };

    renderer_runtime.close_dedicated_worker_for_devtools(instance_id);
    let outputs = prepare_dedicated_worker_target_retirement(
        conn,
        &browser_context_id,
        instance_id,
        DedicatedWorkerRetirementCause::OwnerRetired,
    );
    let mut prepared_outputs =
        ProtocolOutputPayloads::from_slot(TargetPreparedOutputSlot::from_outputs(outputs));
    let owner = CommandOwnerScope::capture(conn, None);
    emit_target_lifecycle_events(
        conn,
        &mut ProtocolOutputProjectionContext::new(&owner, command_context),
        &mut prepared_outputs,
    )
    .await;
    true
}

pub(super) fn record_shared_worker_target_console_message(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_instance_id: SharedWorkerInstanceId,
    message: RendererSharedWorkerConsoleMessage,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
        return outputs;
    };
    let Some(target_id) = context
        .shared_worker_target_id_for_renderer_instance(renderer_instance_id)
        .map(str::to_owned)
    else {
        // Worker inspector messages can race with target destruction. Once the
        // renderer instance has no CDP target, late messages are stale and must
        // not recreate target state or replay into a later fresh worker.
        return outputs;
    };
    let Some(target) = context.shared_worker_target_mut(&target_id) else {
        return outputs;
    };
    target.record_console_message(message);
    for session_id in target.session_ids() {
        let attachment = target
            .protocol_attachment_identity(browser_context_id, &session_id)
            .expect("shared-worker output session must retain its exact attachment identity");
        let console_messages = target.pending_console_domain_messages(&session_id).to_vec();
        if !console_messages.is_empty() {
            outputs.push(WorkerTargetLifecycleOutput::SharedWorkerConsoleMessages {
                attachment: attachment.clone(),
                messages: console_messages,
                console_end: target.console_message_count(),
            });
        }
        let runtime_messages = target
            .pending_runtime_console_messages(&session_id)
            .to_vec();
        if !runtime_messages.is_empty() {
            outputs.push(
                WorkerTargetLifecycleOutput::SharedWorkerRuntimeConsoleMessages {
                    attachment,
                    messages: runtime_messages,
                    console_end: target.console_message_count(),
                },
            );
        }
    }
    outputs
}

pub(super) fn record_shared_worker_target_runtime_inspector_messages(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_instance_id: SharedWorkerInstanceId,
    inspector_session_id: Option<String>,
    messages: Vec<RendererRuntimeInspectorMessage>,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    if messages.is_empty() {
        return outputs;
    }
    let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
        return outputs;
    };
    let Some(target_id) = context
        .shared_worker_target_id_for_renderer_instance(renderer_instance_id)
        .map(str::to_owned)
    else {
        return outputs;
    };
    let Some(target) = context.shared_worker_target_mut(&target_id) else {
        return outputs;
    };
    let session_ids = if let Some(session_id) = inspector_session_id {
        if !target.is_session(&session_id) {
            return outputs;
        }
        vec![session_id]
    } else {
        target.session_ids()
    };
    for session_id in session_ids {
        let attachment = target
            .protocol_attachment_identity(browser_context_id, &session_id)
            .expect("shared-worker inspector route must retain its exact attachment identity");
        outputs.push(
            WorkerTargetLifecycleOutput::SharedWorkerRuntimeInspectorMessages {
                attachment,
                messages: messages.clone(),
            },
        );
    }
    outputs
}

pub(super) fn exact_shared_worker_target<'a>(
    conn: &'a CdpConnection,
    attachment: &TargetSharedWorkerProtocolAttachmentIdentity,
) -> Option<&'a SharedWorkerTargetState> {
    if !attachment.is_current() {
        return None;
    }
    let target = conn
        .browser_context_by_id(attachment.browser_context_id())?
        .shared_worker_target(attachment.target_id())?;
    (target.renderer_owner_local_host_id == attachment.renderer_owner_local_host_id()
        && target.renderer_instance_id == attachment.renderer_instance_id()
        && target.owner_target_id() == attachment.owner_target_id()
        && target.is_session(attachment.session_id()))
    .then_some(target)
}

pub(super) fn exact_shared_worker_target_mut<'a>(
    conn: &'a mut CdpConnection,
    attachment: &TargetSharedWorkerProtocolAttachmentIdentity,
) -> Option<&'a mut SharedWorkerTargetState> {
    if !attachment.is_current() {
        return None;
    }
    let target = conn
        .browser_context_by_id_mut(attachment.browser_context_id())?
        .shared_worker_target_mut(attachment.target_id())?;
    (target.renderer_owner_local_host_id == attachment.renderer_owner_local_host_id()
        && target.renderer_instance_id == attachment.renderer_instance_id()
        && target.owner_target_id() == attachment.owner_target_id()
        && target.is_session(attachment.session_id()))
    .then_some(target)
}

pub(super) fn exact_shared_worker_pending_runtime_console(
    conn: &CdpConnection,
    attachment: &TargetSharedWorkerProtocolAttachmentIdentity,
) -> Option<(Vec<RuntimeConsoleMessageSnapshot>, usize)> {
    let target = exact_shared_worker_target(conn, attachment)?;
    let messages = target
        .pending_runtime_console_messages(attachment.session_id())
        .to_vec();
    (!messages.is_empty()).then(|| (messages, target.console_message_count()))
}

pub(super) fn mark_exact_shared_worker_console_domain_emitted(
    conn: &mut CdpConnection,
    attachment: &TargetSharedWorkerProtocolAttachmentIdentity,
    console_end: usize,
) {
    if let Some(target) = exact_shared_worker_target_mut(conn, attachment) {
        target.mark_console_domain_emitted(attachment.session_id(), console_end);
    }
}

pub(super) fn mark_exact_shared_worker_runtime_console_emitted(
    conn: &mut CdpConnection,
    attachment: &TargetSharedWorkerProtocolAttachmentIdentity,
    console_end: usize,
) {
    if let Some(target) = exact_shared_worker_target_mut(conn, attachment) {
        target.mark_runtime_console_emitted(attachment.session_id(), console_end);
    }
}
