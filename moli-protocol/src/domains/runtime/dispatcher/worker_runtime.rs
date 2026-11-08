use super::*;
use crate::conn::RuntimeInspectorError;

pub(super) fn try_start_shared_worker_runtime_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: RuntimeAction,
) -> Option<RuntimeCommandTaskStep> {
    if conn
        .shared_worker_target_for_session(cmd.session_id)
        .is_none()
    {
        return Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        )));
    }
    let command = WorkerRuntimeCommand::classify(action);

    if command.kind() == WorkerRuntimeCommandKind::RunIfWaitingForDebugger
        && matches!(
            conn.session_route(cmd.session_id),
            Some(CdpSessionRoute::DedicatedWorkerTarget { .. })
        )
    {
        let plan = match conn
            .run_dedicated_worker_if_waiting_for_debugger_for_session(cmd.session_id)
        {
            Ok(true) => CommandOutputPlan::success(),
            Ok(false) | Err(_) => {
                if let Some(events) =
                    crate::domains::target::release_failed_dedicated_worker_target_after_debugger_resume(
                        conn,
                        cmd.session_id,
                    )
                {
                    let mut plan = CommandOutputPlan::success();
                    plan.extend_background_events(events);
                    plan
                } else {
                    shared_worker_runtime_error_plan(
                        "DedicatedWorkerRuntimeUnavailable".to_owned(),
                    )
                }
            }
        };
        return Some(RuntimeCommandTaskStep::Complete(plan));
    }

    match command.kind() {
        WorkerRuntimeCommandKind::Enable => Some(
            match start_pending_shared_worker_runtime_inspector_command(conn, cmd, command) {
                Ok(step) => step,
                Err(message)
                    if message.is::<crate::conn::NoDocumentLoaded>()
                        || worker_runtime_is_unavailable(&message) =>
                {
                    RuntimeCommandTaskStep::Complete(
                        shared_worker_runtime_enable_command_output_plan_for_session(
                            conn,
                            cmd.session_id,
                        ),
                    )
                }
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(message))
                }
            },
        ),
        WorkerRuntimeCommandKind::Disable if is_bidi_runtime_listener_session(cmd.session_id) => {
            apply_shared_worker_runtime_disable_projection(conn, cmd.session_id);
            Some(RuntimeCommandTaskStep::Complete(
                CommandOutputPlan::success(),
            ))
        }
        WorkerRuntimeCommandKind::Disable => Some(
            match start_pending_shared_worker_runtime_inspector_command(conn, cmd, command) {
                Ok(step) => step,
                Err(message)
                    if message.is::<crate::conn::NoDocumentLoaded>()
                        || worker_runtime_is_unavailable(&message) =>
                {
                    apply_shared_worker_runtime_disable_projection(conn, cmd.session_id);
                    RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
                }
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(message))
                }
            },
        ),
        WorkerRuntimeCommandKind::DiscardConsoleEntries => Some(
            match start_pending_shared_worker_runtime_inspector_command(conn, cmd, command) {
                Ok(step) => step,
                Err(message)
                    if message.is::<crate::conn::NoDocumentLoaded>()
                        || worker_runtime_is_unavailable(&message) =>
                {
                    if let Some(session_id) = cmd.session_id
                        && let Some(target) =
                            conn.shared_worker_target_for_session_mut(Some(session_id))
                    {
                        target.discard_runtime_console_entries(session_id);
                    }
                    RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
                }
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(message))
                }
            },
        ),
        WorkerRuntimeCommandKind::RunIfWaitingForDebugger => Some(
            match start_pending_shared_worker_runtime_inspector_command(conn, cmd, command) {
                Ok(step) => step,
                Err(message)
                    if message.is::<crate::conn::NoDocumentLoaded>()
                        || worker_runtime_is_unavailable(&message) =>
                {
                    RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
                }
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(message))
                }
            },
        ),
        WorkerRuntimeCommandKind::Inspector => Some(
            match start_pending_shared_worker_runtime_inspector_command(conn, cmd, command) {
                Ok(step) => step,
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(message))
                }
            },
        ),
    }
}

pub(super) fn release_service_worker_if_waiting_for_debugger(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> bool {
    let Some(session_id) = session_id else {
        return false;
    };
    let Some(CdpSessionRoute::ServiceWorkerTarget {
        browser_context_id,
        target_id,
    }) = conn.session_route(Some(session_id))
    else {
        return false;
    };
    let Some(browser_context) = conn.browser_context_by_id(&browser_context_id) else {
        return false;
    };
    let Some(version_id) = browser_context
        .service_worker_target(&target_id)
        .map(|target| target.renderer_version_id)
    else {
        return false;
    };
    browser_context
        .renderer_runtime()
        .run_service_worker_if_waiting_for_debugger_for_devtools(version_id)
}

pub(super) fn is_bidi_runtime_listener_session(session_id: Option<&str>) -> bool {
    session_id.is_some_and(|session_id| session_id.starts_with("SID-bidi-runtime-listener-"))
}

pub(super) fn try_start_service_worker_runtime_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: RuntimeAction,
) -> Option<RuntimeCommandTaskStep> {
    if conn
        .service_worker_target_for_session(cmd.session_id)
        .is_none()
    {
        return Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        )));
    }
    let command = WorkerRuntimeCommand::classify(action);

    match command.kind() {
        WorkerRuntimeCommandKind::Enable => Some(
            match start_pending_service_worker_runtime_inspector_command(conn, cmd, command) {
                Ok(step) => step,
                Err(message)
                    if message.is::<crate::conn::NoDocumentLoaded>()
                        || worker_runtime_is_unavailable(&message) =>
                {
                    RuntimeCommandTaskStep::Complete(
                        service_worker_runtime_enable_command_output_plan_for_session(
                            conn,
                            cmd.session_id,
                        ),
                    )
                }
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(message))
                }
            },
        ),
        WorkerRuntimeCommandKind::Disable => Some(
            match start_pending_service_worker_runtime_inspector_command(conn, cmd, command) {
                Ok(step) => step,
                Err(message)
                    if message.is::<crate::conn::NoDocumentLoaded>()
                        || worker_runtime_is_unavailable(&message) =>
                {
                    apply_service_worker_runtime_disable_projection(conn, cmd.session_id);
                    RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
                }
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(message))
                }
            },
        ),
        WorkerRuntimeCommandKind::DiscardConsoleEntries => Some(
            match start_pending_service_worker_runtime_inspector_command(conn, cmd, command) {
                Ok(step) => step,
                Err(message)
                    if message.is::<crate::conn::NoDocumentLoaded>()
                        || worker_runtime_is_unavailable(&message) =>
                {
                    if let Some(session_id) = cmd.session_id
                        && let Some(target) =
                            conn.service_worker_target_for_session_mut(Some(session_id))
                    {
                        target.discard_runtime_console_entries(session_id);
                    }
                    RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
                }
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(message))
                }
            },
        ),
        WorkerRuntimeCommandKind::RunIfWaitingForDebugger => {
            release_service_worker_if_waiting_for_debugger(conn, cmd.session_id);
            let dispatch =
                start_pending_service_worker_runtime_inspector_command(conn, cmd, command);
            Some(match dispatch {
                Ok(step) => step,
                Err(message)
                    if message.is::<crate::conn::NoDocumentLoaded>()
                        || worker_runtime_is_unavailable(&message) =>
                {
                    RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
                }
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(message))
                }
            })
        }
        WorkerRuntimeCommandKind::Inspector => Some(
            match start_pending_service_worker_runtime_inspector_command(conn, cmd, command) {
                Ok(step) => step,
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(message))
                }
            },
        ),
    }
}

pub(super) fn start_pending_shared_worker_runtime_inspector_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    command: WorkerRuntimeCommand,
) -> Result<RuntimeCommandTaskStep> {
    if !can_dispatch(cmd) {
        return Err(RuntimeInspectorError::UnknownMethod.into());
    }

    let action = command.action();
    let await_promise = runtime_command_awaits_promise(cmd, action);
    let inspector_json =
        prepare_runtime_inspector_payload(conn, cmd, command.payload_preparation())?;
    let object_group = runtime_object_group_for_command_result(conn, cmd, action);
    let release_object_ids = if action == RuntimeAction::ReleaseObject {
        cmd.params
            .map(runtime_remote_object_ids_in_map)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let release_object_group = if action == RuntimeAction::ReleaseObjectGroup {
        runtime_object_group_from_params(cmd.params).map(str::to_owned)
    } else {
        None
    };
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let pre_registered_await = pre_register_runtime_await_if_needed(
        conn,
        await_promise,
        cmd.id,
        &owner_scope,
        object_group.as_deref(),
        action.label(),
    )?;
    let response_delivery = cmd.terminal_response_delivery();
    let pending = match start_shared_worker_frontend_inspector_dispatch(
        conn,
        cmd,
        inspector_json,
        response_delivery,
    ) {
        Ok(pending) => pending,
        Err(message) => {
            forget_pre_registered_runtime_await(conn, pre_registered_await, &owner_scope);
            return Err(message);
        }
    };
    let binding_effect = shared_worker_runtime_binding_effect_from_command(cmd, command.binding())?;
    Ok(RuntimeCommandTaskStep::Pending(Box::new(
        PendingRuntimeCommandDispatch {
            command_id: cmd.id,
            action: action.label(),
            owner_scope,
            object_group,
            release_object_ids,
            release_object_group,
            await_promise,
            wait_for_deferred_reply: await_promise,
            pending: PendingRuntimeCommandKind::SharedWorkerInspector {
                pending,
                binding_effect,
            },
        },
    )))
}

pub(super) fn start_pending_service_worker_runtime_inspector_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    command: WorkerRuntimeCommand,
) -> Result<RuntimeCommandTaskStep> {
    if !can_dispatch(cmd) {
        return Err(RuntimeInspectorError::UnknownMethod.into());
    }

    let action = command.action();
    let await_promise = runtime_command_awaits_promise(cmd, action);
    let inspector_json =
        prepare_runtime_inspector_payload(conn, cmd, command.payload_preparation())?;
    let object_group = runtime_object_group_for_command_result(conn, cmd, action);
    let release_object_ids = if action == RuntimeAction::ReleaseObject {
        cmd.params
            .map(runtime_remote_object_ids_in_map)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let release_object_group = if action == RuntimeAction::ReleaseObjectGroup {
        runtime_object_group_from_params(cmd.params).map(str::to_owned)
    } else {
        None
    };
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let pre_registered_await = pre_register_runtime_await_if_needed(
        conn,
        await_promise,
        cmd.id,
        &owner_scope,
        object_group.as_deref(),
        action.label(),
    )?;
    let response_delivery = cmd.terminal_response_delivery();
    let pending = match start_service_worker_frontend_inspector_dispatch(
        conn,
        cmd,
        inspector_json,
        response_delivery,
    ) {
        Ok(pending) => pending,
        Err(message) => {
            forget_pre_registered_runtime_await(conn, pre_registered_await, &owner_scope);
            return Err(message);
        }
    };
    Ok(RuntimeCommandTaskStep::Pending(Box::new(
        PendingRuntimeCommandDispatch {
            command_id: cmd.id,
            action: action.label(),
            owner_scope,
            object_group,
            release_object_ids,
            release_object_group,
            await_promise,
            wait_for_deferred_reply: await_promise,
            pending: PendingRuntimeCommandKind::ServiceWorkerInspector { pending },
        },
    )))
}

pub(super) fn shared_worker_runtime_binding_replay_json(
    command_id: u64,
    binding: &RuntimeBindingDefinition,
) -> String {
    let mut params = serde_json::Map::new();
    params.insert("name".to_owned(), json!(&binding.name));
    if let Some(execution_context_name) = &binding.execution_context_name {
        params.insert(
            "executionContextName".to_owned(),
            json!(execution_context_name),
        );
    }
    json!({
        "id": command_id,
        "method": "Runtime.addBinding",
        "params": params,
    })
    .to_string()
}

pub(super) fn shared_worker_runtime_binding_effect_from_command(
    cmd: &Cmd<'_>,
    binding: Option<RuntimeBindingCommand>,
) -> Result<Option<SharedWorkerRuntimeBindingEffect>> {
    match binding {
        Some(RuntimeBindingCommand::Add) => {
            let params = cmd
                .get_params::<AddBindingParams>()
                .map_err(|_| RuntimeInspectorError::InvalidParams)?
                .ok_or(RuntimeInspectorError::InvalidParams)?;
            if params.execution_context_id.is_some() {
                return Ok(None);
            }
            Ok(Some(SharedWorkerRuntimeBindingEffect::Add {
                name: params.name,
                execution_context_name: params.execution_context_name,
            }))
        }
        Some(RuntimeBindingCommand::Remove) => {
            let params = cmd
                .get_params::<chromiumoxide_cdp::cdp::js_protocol::runtime::RemoveBindingParams>()
                .map_err(|_| RuntimeInspectorError::InvalidParams)?
                .ok_or(RuntimeInspectorError::InvalidParams)?;
            Ok(Some(SharedWorkerRuntimeBindingEffect::Remove {
                name: params.name,
            }))
        }
        None => Ok(None),
    }
}

pub(super) fn apply_shared_worker_runtime_binding_effect_after_success(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    effect: SharedWorkerRuntimeBindingEffect,
) -> Result<()> {
    let session_id = session_id.ok_or(RuntimeInspectorError::UnknownSession)?;
    match effect {
        SharedWorkerRuntimeBindingEffect::Add {
            name,
            execution_context_name,
        } => {
            let target = conn
                .shared_worker_target_for_session_mut(Some(session_id))
                .ok_or(RuntimeInspectorError::UnknownSession)?;
            target.upsert_live_runtime_binding_definition(session_id, name, execution_context_name);
            Ok(())
        }
        SharedWorkerRuntimeBindingEffect::Remove { name } => {
            let target = conn
                .shared_worker_target_for_session_mut(Some(session_id))
                .ok_or(RuntimeInspectorError::UnknownSession)?;
            target.remove_live_runtime_binding_definitions(session_id, &name);
            Ok(())
        }
    }
}

pub(super) fn apply_shared_worker_runtime_completion_projection(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    action: &str,
    succeeded: bool,
) {
    let Some(owner_session_id) = session_id else {
        return;
    };
    let Some(target) = conn.shared_worker_target_for_session_mut(Some(owner_session_id)) else {
        return;
    };

    match action {
        "enable" if succeeded => target.set_runtime_frontend_enabled(owner_session_id, true),
        "disable" if succeeded => {
            let was_enabled = target.runtime_frontend_enabled(owner_session_id);
            target.set_runtime_frontend_enabled(owner_session_id, false);
            if was_enabled {
                target.clear_runtime_binding_definitions(owner_session_id);
            }
        }
        "discardConsoleEntries" if succeeded => {
            target.discard_runtime_console_entries(owner_session_id)
        }
        _ => {}
    }
}

pub(super) fn apply_shared_worker_runtime_disable_projection(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) {
    let Some(owner_session_id) = session_id else {
        return;
    };
    let Some(target) = conn.shared_worker_target_for_session_mut(Some(owner_session_id)) else {
        return;
    };
    let was_enabled = target.runtime_frontend_enabled(owner_session_id);
    target.set_runtime_frontend_enabled(owner_session_id, false);
    if was_enabled {
        target.clear_runtime_binding_definitions(owner_session_id);
    }
}

pub(super) fn complete_shared_worker_runtime_inspector_error(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    message: anyhow::Error,
) -> RuntimeCommandTaskStep {
    if let Some(command_id) = completed.command_id {
        let renderer_call = conn
            .take_renderer_call_for_frontend_for_session_owner(completed.session_id(), command_id);
        conn.forget_pending_inspector_await(command_id, completed.session_id());
        if renderer_call.is_none() {
            tracing::debug!(
                command_id,
                session_id = completed.session_id(),
                error = %message,
                "ignored shared-worker completion after another route settled the frontend call"
            );
            return RuntimeCommandTaskStep::Complete(CommandOutputPlan::default());
        }
    }
    match completed.action {
        "enable"
            if message.is::<crate::conn::NoDocumentLoaded>()
                || worker_runtime_is_unavailable(&message) =>
        {
            RuntimeCommandTaskStep::Complete(
                shared_worker_runtime_enable_command_output_plan_for_session(
                    conn,
                    completed.session_id(),
                ),
            )
        }
        "disable"
            if message.is::<crate::conn::NoDocumentLoaded>()
                || worker_runtime_is_unavailable(&message) =>
        {
            apply_shared_worker_runtime_disable_projection(conn, completed.session_id());
            RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
        }
        "discardConsoleEntries"
            if message.is::<crate::conn::NoDocumentLoaded>()
                || worker_runtime_is_unavailable(&message) =>
        {
            if let Some(owner_session_id) = completed.session_id()
                && let Some(target) =
                    conn.shared_worker_target_for_session_mut(Some(owner_session_id))
            {
                target.discard_runtime_console_entries(owner_session_id);
            }
            RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
        }
        "Console.enable" | "Console.disable" | "Console.clearMessages"
            if worker_runtime_is_unavailable(&message) =>
        {
            if let Some(console_action) = console_action_from_protocol_method(completed.action)
                && !apply_console_output_state_for_session(
                    conn,
                    completed.session_id(),
                    console_action,
                )
            {
                return RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(
                    "UnknownSession".to_owned(),
                ));
            }
            RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
        }
        _ => RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(message)),
    }
}

pub(super) fn apply_service_worker_runtime_completion_projection(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    action: &str,
    succeeded: bool,
) {
    let Some(owner_session_id) = session_id else {
        return;
    };
    let Some(target) = conn.service_worker_target_for_session_mut(Some(owner_session_id)) else {
        return;
    };

    match action {
        "enable" if succeeded => target.set_runtime_frontend_enabled(owner_session_id, true),
        "disable" if succeeded => {
            target.set_runtime_frontend_enabled(owner_session_id, false);
        }
        "discardConsoleEntries" if succeeded => {
            target.discard_runtime_console_entries(owner_session_id)
        }
        _ => {}
    }
}

pub(super) fn apply_service_worker_runtime_disable_projection(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) {
    let Some(owner_session_id) = session_id else {
        return;
    };
    let Some(target) = conn.service_worker_target_for_session_mut(Some(owner_session_id)) else {
        return;
    };
    target.set_runtime_frontend_enabled(owner_session_id, false);
}

pub(super) fn complete_service_worker_runtime_inspector_error(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    message: anyhow::Error,
) -> RuntimeCommandTaskStep {
    if let Some(command_id) = completed.command_id {
        let renderer_call = conn
            .take_renderer_call_for_frontend_for_session_owner(completed.session_id(), command_id);
        conn.forget_pending_inspector_await(command_id, completed.session_id());
        if renderer_call.is_none() {
            tracing::debug!(
                command_id,
                session_id = completed.session_id(),
                error = %message,
                "ignored service-worker completion after another route settled the frontend call"
            );
            return RuntimeCommandTaskStep::Complete(CommandOutputPlan::default());
        }
    }
    match completed.action {
        "enable"
            if message.is::<crate::conn::NoDocumentLoaded>()
                || worker_runtime_is_unavailable(&message) =>
        {
            RuntimeCommandTaskStep::Complete(
                service_worker_runtime_enable_command_output_plan_for_session(
                    conn,
                    completed.session_id(),
                ),
            )
        }
        "disable"
            if message.is::<crate::conn::NoDocumentLoaded>()
                || worker_runtime_is_unavailable(&message) =>
        {
            apply_service_worker_runtime_disable_projection(conn, completed.session_id());
            RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
        }
        "discardConsoleEntries"
            if message.is::<crate::conn::NoDocumentLoaded>()
                || worker_runtime_is_unavailable(&message) =>
        {
            if let Some(owner_session_id) = completed.session_id()
                && let Some(target) =
                    conn.service_worker_target_for_session_mut(Some(owner_session_id))
            {
                target.discard_runtime_console_entries(owner_session_id);
            }
            RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
        }
        "Console.enable" | "Console.disable" | "Console.clearMessages"
            if worker_runtime_is_unavailable(&message) =>
        {
            if let Some(console_action) = console_action_from_protocol_method(completed.action)
                && !apply_console_output_state_for_session(
                    conn,
                    completed.session_id(),
                    console_action,
                )
            {
                return RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(
                    "UnknownSession".to_owned(),
                ));
            }
            RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
        }
        _ => RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(message)),
    }
}

pub(super) fn shared_worker_runtime_error_plan(
    message: impl std::fmt::Display,
) -> CommandOutputPlan {
    let message = message.to_string();
    match message.as_str() {
        "Duplicate `id` in protocol request" => CommandOutputPlan::error(-32600, message),
        "UnknownSession" => CommandOutputPlan::error(-32001, "Unknown sessionId"),
        "InvalidParams" => CommandOutputPlan::error(-32602, "InvalidParams"),
        "UnknownMethod" => CommandOutputPlan::error(-32601, "UnknownMethod"),
        "NoDocumentLoaded" => CommandOutputPlan::error(-32000, "NoDocumentLoaded"),
        _ => CommandOutputPlan::error(-32000, message),
    }
}

pub(super) fn service_worker_runtime_error_plan(
    message: impl std::fmt::Display,
) -> CommandOutputPlan {
    let message = message.to_string();
    match message.as_str() {
        "Duplicate `id` in protocol request" => CommandOutputPlan::error(-32600, message),
        "UnknownSession" => CommandOutputPlan::error(-32001, "Unknown sessionId"),
        "InvalidParams" => CommandOutputPlan::error(-32602, "InvalidParams"),
        "UnknownMethod" => CommandOutputPlan::error(-32601, "UnknownMethod"),
        "NoDocumentLoaded" => CommandOutputPlan::error(-32000, "NoDocumentLoaded"),
        _ => CommandOutputPlan::error(-32000, message),
    }
}

pub(super) fn append_shared_worker_runtime_console_messages(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    plan: &mut CommandOutputPlan,
) {
    let Some(target) = conn.shared_worker_target_for_session_mut(session_id) else {
        return;
    };
    let Some(session_id) = session_id else {
        return;
    };
    let has_runtime_context = target.real_runtime_execution_context_id().is_some();
    let runtime_messages = target.pending_runtime_console_messages(session_id).to_vec();
    let console_end = target.console_message_count();
    if has_runtime_context {
        target.mark_runtime_console_emitted(session_id, console_end);
    }

    let base_timestamp = monotonic_timestamp_seconds();
    for (index, message) in runtime_messages.iter().enumerate() {
        let (console_type, text) = runtime_console_message_type_and_text(&message.message);
        push_runtime_console_api_called_background_event(
            plan,
            Some(session_id),
            console_type,
            text,
            &message.args,
            message.stack.as_deref(),
            message.execution_context_id,
            base_timestamp + ((index + 1) as f64 * 0.000_001),
        );
    }
}

pub(super) fn append_service_worker_runtime_console_messages(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    plan: &mut CommandOutputPlan,
) {
    let Some(target) = conn.service_worker_target_for_session_mut(session_id) else {
        return;
    };
    let Some(session_id) = session_id else {
        return;
    };
    let has_runtime_context = target.real_runtime_execution_context_id().is_some();
    let runtime_messages = target.pending_runtime_console_messages(session_id).to_vec();
    let console_end = target.console_message_count();
    if has_runtime_context {
        target.mark_runtime_console_emitted(session_id, console_end);
    }

    let base_timestamp = monotonic_timestamp_seconds();
    for (index, message) in runtime_messages.iter().enumerate() {
        let (console_type, text) = runtime_console_message_type_and_text(&message.message);
        push_runtime_console_api_called_background_event(
            plan,
            Some(session_id),
            console_type,
            text,
            &message.args,
            message.stack.as_deref(),
            message.execution_context_id,
            base_timestamp + ((index + 1) as f64 * 0.000_001),
        );
    }
}

pub(super) fn append_service_worker_runtime_exception_messages(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    plan: &mut CommandOutputPlan,
) {
    let Some(target) = conn.service_worker_target_for_session_mut(session_id) else {
        return;
    };
    let Some(session_id) = session_id else {
        return;
    };
    let has_runtime_context = target.real_runtime_execution_context_id().is_some();
    let exception_messages = target
        .pending_runtime_exception_messages(session_id)
        .to_vec();
    let exception_start = target
        .exception_message_count()
        .saturating_sub(exception_messages.len());
    let exception_end = target.exception_message_count();
    if has_runtime_context {
        target.mark_runtime_exception_emitted(session_id, exception_end);
    }

    push_runtime_exception_thrown_protocol_messages(
        plan,
        session_id,
        &exception_messages,
        exception_start,
    );
}

pub(super) fn shared_worker_runtime_enable_command_output_plan_for_session(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) -> CommandOutputPlan {
    let Some(target) = conn.shared_worker_target_for_session_mut(session_id) else {
        return CommandOutputPlan::error(-32001, "Unknown sessionId");
    };
    let Some(session_id) = session_id else {
        return CommandOutputPlan::error(-32001, "Unknown sessionId");
    };
    target.set_runtime_frontend_enabled(session_id, true);
    let execution_context_created = build_shared_worker_execution_context_created_event(target);
    let has_runtime_context = target.real_runtime_execution_context_id().is_some();
    let runtime_messages = target.pending_runtime_console_messages(session_id).to_vec();
    let console_end = target.console_message_count();
    if has_runtime_context {
        target.mark_runtime_console_emitted(session_id, console_end);
    }

    // Runtime.enable reports its existing inventory before acknowledging the
    // enable. Keep replay and reply in one ordered plan, including worker
    // adapters that synthesize the replay outside the V8 session sink.
    let mut plan = CommandOutputPlan::default();
    if let Some(execution_context_created) = execution_context_created {
        target.record_runtime_contexts_reported_to_frontend(session_id);
        push_execution_context_created_background_event(
            &mut plan,
            execution_context_created,
            session_id,
        );
    }
    let base_timestamp = monotonic_timestamp_seconds();
    for (index, message) in runtime_messages.iter().enumerate() {
        let (console_type, text) = runtime_console_message_type_and_text(&message.message);
        push_runtime_console_api_called_background_event(
            &mut plan,
            Some(session_id),
            console_type,
            text,
            &message.args,
            message.stack.as_deref(),
            message.execution_context_id,
            base_timestamp + ((index + 1) as f64 * 0.000_001),
        );
    }
    plan.push_success();
    plan
}

pub(super) fn service_worker_runtime_enable_command_output_plan_for_session(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) -> CommandOutputPlan {
    let Some(target) = conn.service_worker_target_for_session_mut(session_id) else {
        return CommandOutputPlan::error(-32001, "Unknown sessionId");
    };
    let Some(session_id) = session_id else {
        return CommandOutputPlan::error(-32001, "Unknown sessionId");
    };
    target.set_runtime_frontend_enabled(session_id, true);
    let execution_context_created = build_service_worker_execution_context_created_event(target);
    let has_runtime_context = target.real_runtime_execution_context_id().is_some();
    let runtime_messages = target.pending_runtime_console_messages(session_id).to_vec();
    let console_end = target.console_message_count();
    if has_runtime_context {
        target.mark_runtime_console_emitted(session_id, console_end);
    }
    let exception_messages = target
        .pending_runtime_exception_messages(session_id)
        .to_vec();
    let exception_start = target
        .exception_message_count()
        .saturating_sub(exception_messages.len());
    let exception_end = target.exception_message_count();
    if has_runtime_context {
        target.mark_runtime_exception_emitted(session_id, exception_end);
    }

    // Runtime.enable reports its existing inventory before acknowledging the
    // enable. Keep replay and reply in one ordered plan, including worker
    // adapters that synthesize the replay outside the V8 session sink.
    let mut plan = CommandOutputPlan::default();
    if let Some(execution_context_created) = execution_context_created {
        target.record_runtime_contexts_reported_to_frontend(session_id);
        push_execution_context_created_background_event(
            &mut plan,
            execution_context_created,
            session_id,
        );
    }
    let base_timestamp = monotonic_timestamp_seconds();
    for (index, message) in runtime_messages.iter().enumerate() {
        let (console_type, text) = runtime_console_message_type_and_text(&message.message);
        push_runtime_console_api_called_background_event(
            &mut plan,
            Some(session_id),
            console_type,
            text,
            &message.args,
            message.stack.as_deref(),
            message.execution_context_id,
            base_timestamp + ((index + 1) as f64 * 0.000_001),
        );
    }
    push_runtime_exception_thrown_protocol_messages(
        &mut plan,
        session_id,
        &exception_messages,
        exception_start,
    );
    plan.push_success();
    plan
}

pub(super) fn push_execution_context_created_background_event(
    plan: &mut CommandOutputPlan,
    event: RuntimeContextProtocolEvent,
    session_id: &str,
) {
    let mut events = Vec::new();
    emit_runtime_context_protocol_background_event_typed(&mut events, event, Some(session_id));
    plan.extend_background_events(events);
}

pub(super) fn push_runtime_console_api_called_background_event(
    plan: &mut CommandOutputPlan,
    session_id: Option<&str>,
    console_type: &str,
    text: &str,
    args: &[Value],
    stack: Option<&str>,
    execution_context_id: i64,
    timestamp: f64,
) {
    plan.push_background_event(runtime_console_api_called_background_event(
        session_id,
        None,
        console_type,
        text,
        args,
        stack,
        execution_context_id,
        timestamp,
    ));
}

pub(super) fn push_runtime_exception_thrown_protocol_messages(
    plan: &mut CommandOutputPlan,
    session_id: &str,
    messages: &[ServiceWorkerRuntimeExceptionSnapshot],
    exception_start: usize,
) {
    let base_timestamp = monotonic_timestamp_seconds();
    for (offset, message) in messages.iter().enumerate() {
        let exception_index = exception_start + offset;
        plan.push_background_event(runtime_exception_thrown_background_event(
            Some(session_id),
            None,
            &message.message.message,
            &message.message.filename,
            message.execution_context_id,
            exception_index,
            base_timestamp + ((offset + 1) as f64 * 0.000_001),
            Some(u64::from(message.message.lineno.saturating_sub(1))),
            Some(u64::from(message.message.colno.saturating_sub(1))),
        ));
    }
}

pub(super) fn build_shared_worker_execution_context_created_event(
    target: &crate::conn::SharedWorkerTargetState,
) -> Option<RuntimeContextProtocolEvent> {
    let context_id = target.real_runtime_execution_context_id()?;
    let realm_id = format!("shared-worker-{}", target.target_id);
    let origin = url::Url::parse(&target.url)
        .ok()
        .map(|url| moli_url::origin_ascii_serialization(&url))
        .unwrap_or_else(|| "null".to_owned());
    Some(RuntimeContextProtocolEvent::Created(
        RuntimeExecutionContextEvent {
            target_id: None,
            context_id: Some(context_id),
            realm_id: Some(DevToolsRealmId::from(realm_id)),
            frame_id: None,
            origin: Some(origin),
            name: Some(target.name.clone()),
            is_default: Some(true),
            context_type: Some("worker".to_owned()),
            grant_universal_access: None,
        },
    ))
}

pub(super) fn build_service_worker_execution_context_created_event(
    target: &crate::conn::ServiceWorkerTargetState,
) -> Option<RuntimeContextProtocolEvent> {
    let context_id = target.real_runtime_execution_context_id()?;
    let realm_id = format!("service-worker-{}", target.target_id);
    let origin = url::Url::parse(&target.script_url)
        .ok()
        .map(|url| moli_url::origin_ascii_serialization(&url))
        .unwrap_or_else(|| "null".to_owned());
    Some(RuntimeContextProtocolEvent::Created(
        RuntimeExecutionContextEvent {
            target_id: None,
            context_id: Some(context_id),
            realm_id: Some(DevToolsRealmId::from(realm_id)),
            frame_id: None,
            origin: Some(origin),
            name: Some(String::new()),
            is_default: Some(true),
            context_type: Some("service-worker".to_owned()),
            grant_universal_access: None,
        },
    ))
}

pub(super) fn disable_command_output_plan_sync_for_owner(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
) -> CommandOutputPlan {
    match apply_runtime_disable_projection_after_success_for_owner(conn, owner) {
        Ok(()) => CommandOutputPlan::success(),
        Err(_) => CommandOutputPlan::error(-32001, "Unknown sessionId"),
    }
}
