use super::*;

pub(super) fn command_response_succeeded(
    messages: &[RendererRuntimeInspectorMessage],
    command_id: Option<u64>,
) -> bool {
    let Some(command_id) = command_id else {
        return false;
    };
    messages.iter().any(|message| {
        let RendererRuntimeInspectorMessage::Protocol(message) = message else {
            return false;
        };
        message.get("id").and_then(Value::as_u64) == Some(command_id)
            && message.get("result").is_some()
    })
}

pub(super) fn command_response_succeeded_for_events(
    events: &[BackgroundProtocolEvent],
    command_id: Option<u64>,
) -> bool {
    let Some(command_id) = command_id else {
        return false;
    };
    events.iter().any(|event| {
        if let Some((event_command_id, _, payload)) = event.command_response_payload_ref()
            && event_command_id == Some(command_id)
        {
            return matches!(payload, BackgroundCommandResponsePayloadRef::Success { .. });
        }
        event.protocol_message().is_some_and(|message| {
            message.get("id").and_then(Value::as_u64) == Some(command_id)
                && message.get("result").is_some()
        })
    })
}

pub(super) fn runtime_object_group_from_params(
    params: Option<&Map<String, Value>>,
) -> Option<&str> {
    params?.get("objectGroup").and_then(Value::as_str)
}

pub(super) fn runtime_object_id_from_params(params: Option<&Map<String, Value>>) -> Option<&str> {
    params?.get("objectId").and_then(Value::as_str)
}

pub(super) fn runtime_promise_object_id_from_params(
    params: Option<&Map<String, Value>>,
) -> Option<&str> {
    params?.get("promiseObjectId").and_then(Value::as_str)
}

pub(super) fn runtime_prototype_object_id_from_params(
    params: Option<&Map<String, Value>>,
) -> Option<&str> {
    params?.get("prototypeObjectId").and_then(Value::as_str)
}

pub(super) fn runtime_object_group_for_command_result(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    action: RuntimeAction,
) -> Option<String> {
    match action {
        RuntimeAction::Evaluate => runtime_object_group_from_params(cmd.params).map(str::to_owned),
        RuntimeAction::CallFunctionOn => runtime_object_group_from_params(cmd.params)
            .map(str::to_owned)
            .or_else(|| {
                conn.runtime_remote_object_group_for_session_owner(
                    cmd.session_id,
                    runtime_object_id_from_params(cmd.params)?,
                )
            }),
        RuntimeAction::GetProperties => conn.runtime_remote_object_group_for_session_owner(
            cmd.session_id,
            runtime_object_id_from_params(cmd.params)?,
        ),
        RuntimeAction::AwaitPromise => conn.runtime_remote_object_group_for_session_owner(
            cmd.session_id,
            runtime_promise_object_id_from_params(cmd.params)?,
        ),
        RuntimeAction::RunScript => runtime_object_group_from_params(cmd.params).map(str::to_owned),
        RuntimeAction::QueryObjects => runtime_object_group_from_params(cmd.params)
            .map(str::to_owned)
            .or_else(|| {
                conn.runtime_remote_object_group_for_session_owner(
                    cmd.session_id,
                    runtime_prototype_object_id_from_params(cmd.params)?,
                )
            }),
        _ => None,
    }
}

pub(super) fn runtime_object_group_for_command_result_for_owner(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    owner: &CommandOwnerScope,
    action: RuntimeAction,
) -> Option<String> {
    match action {
        RuntimeAction::Evaluate => runtime_object_group_from_params(cmd.params).map(str::to_owned),
        RuntimeAction::CallFunctionOn => runtime_object_group_from_params(cmd.params)
            .map(str::to_owned)
            .or_else(|| {
                conn.runtime_remote_object_group_for_owner(
                    owner,
                    runtime_object_id_from_params(cmd.params)?,
                )
            }),
        RuntimeAction::GetProperties => conn.runtime_remote_object_group_for_owner(
            owner,
            runtime_object_id_from_params(cmd.params)?,
        ),
        RuntimeAction::AwaitPromise => conn.runtime_remote_object_group_for_owner(
            owner,
            runtime_promise_object_id_from_params(cmd.params)?,
        ),
        RuntimeAction::RunScript => runtime_object_group_from_params(cmd.params).map(str::to_owned),
        RuntimeAction::QueryObjects => runtime_object_group_from_params(cmd.params)
            .map(str::to_owned)
            .or_else(|| {
                conn.runtime_remote_object_group_for_owner(
                    owner,
                    runtime_prototype_object_id_from_params(cmd.params)?,
                )
            }),
        _ => None,
    }
}

pub(super) fn runtime_command_awaits_promise(cmd: &Cmd<'_>, action: RuntimeAction) -> bool {
    action == RuntimeAction::AwaitPromise
        || cmd
            .params
            .and_then(|params| params.get("awaitPromise"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

pub(super) fn start_main_runtime_inspector_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    command: MainRuntimeInspectorCommand,
) -> RuntimeCommandTaskStep {
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    start_main_runtime_inspector_command_for_owner(conn, cmd, command, owner_scope)
}

pub(super) fn start_main_runtime_inspector_command_for_owner(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    command: MainRuntimeInspectorCommand,
    owner_scope: CommandOwnerScope,
) -> RuntimeCommandTaskStep {
    let action = command.action();
    let action_label = action.label();
    let await_promise = runtime_command_awaits_promise(cmd, action);
    let inspector_json = match prepare_runtime_inspector_payload_for_owner(
        conn,
        cmd,
        &owner_scope,
        command.payload_preparation(),
    ) {
        Ok(json) => json,
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message));
        }
    };
    let object_group =
        runtime_object_group_for_command_result_for_owner(conn, cmd, &owner_scope, action);
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
    let pre_registered_await = match pre_register_runtime_await_if_needed(
        conn,
        await_promise,
        cmd.id,
        &owner_scope,
        object_group.as_deref(),
        action_label,
    ) {
        Ok(command_id) => command_id,
        Err(error) => {
            return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
                -32600,
                error.to_string(),
            ));
        }
    };
    let pending = match start_pending_runtime_routable_inspector_dispatch(
        conn,
        cmd,
        &owner_scope,
        inspector_json,
        cmd.terminal_response_delivery(),
    ) {
        Ok(pending) => pending,
        Err(message) => {
            forget_pre_registered_runtime_await(conn, pre_registered_await, &owner_scope);
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message));
        }
    };
    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: action_label,
        owner_scope,
        object_group,
        release_object_ids,
        release_object_group,
        await_promise,
        wait_for_deferred_reply: await_promise,
        pending: PendingRuntimeCommandKind::Inspector { pending },
    }))
}

pub(super) fn start_heap_profiler_inspector_shared_worker_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: HeapProfilerAction,
) -> RuntimeCommandTaskStep {
    let Some(session_id) = cmd.session_id else {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        ));
    };
    if conn
        .shared_worker_target_for_session(Some(session_id))
        .is_none()
    {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        ));
    }

    if !can_dispatch(cmd) {
        return RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(
            "UnknownMethod".to_owned(),
        ));
    }

    let pending = match start_shared_worker_frontend_inspector_dispatch(
        conn,
        cmd,
        cmd.json.to_owned(),
        cmd.terminal_response_delivery(),
    ) {
        Ok(pending) => pending,
        Err(message)
            if matches!(
                action,
                HeapProfilerAction::Enable | HeapProfilerAction::Disable
            ) && (message == "NoDocumentLoaded" || worker_runtime_is_unavailable(&message)) =>
        {
            return RuntimeCommandTaskStep::Complete(CommandOutputPlan::success());
        }
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(message));
        }
    };

    let object_group = heap_profiler_object_group_for_command_result(cmd, action);
    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: heap_profiler_action_protocol_method(action),
        owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
        object_group,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: true,
        pending: PendingRuntimeCommandKind::SharedWorkerInspector {
            pending,
            binding_effect: None,
        },
    }))
}

pub(super) fn heap_profiler_action_protocol_method(action: HeapProfilerAction) -> &'static str {
    match action {
        HeapProfilerAction::AddInspectedHeapObject => "HeapProfiler.addInspectedHeapObject",
        HeapProfilerAction::Enable => "HeapProfiler.enable",
        HeapProfilerAction::Disable => "HeapProfiler.disable",
        HeapProfilerAction::CollectGarbage => "HeapProfiler.collectGarbage",
        HeapProfilerAction::GetHeapObjectId => "HeapProfiler.getHeapObjectId",
        HeapProfilerAction::GetObjectByHeapObjectId => "HeapProfiler.getObjectByHeapObjectId",
        HeapProfilerAction::GetSamplingProfile => "HeapProfiler.getSamplingProfile",
        HeapProfilerAction::StartSampling => "HeapProfiler.startSampling",
        HeapProfilerAction::StartTrackingHeapObjects => "HeapProfiler.startTrackingHeapObjects",
        HeapProfilerAction::StopSampling => "HeapProfiler.stopSampling",
        HeapProfilerAction::StopTrackingHeapObjects => "HeapProfiler.stopTrackingHeapObjects",
        HeapProfilerAction::TakeHeapSnapshot => "HeapProfiler.takeHeapSnapshot",
        HeapProfilerAction::MoliDiagnostics | HeapProfilerAction::MoliResetIdleEngine => {
            "HeapProfiler.moliExtension"
        }
    }
}

pub(super) fn heap_profiler_object_group_for_command_result(
    cmd: &Cmd<'_>,
    action: HeapProfilerAction,
) -> Option<String> {
    match action {
        HeapProfilerAction::GetObjectByHeapObjectId => {
            runtime_object_group_from_params(cmd.params).map(str::to_owned)
        }
        _ => None,
    }
}

pub(super) fn start_profiler_inspector_shared_worker_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    dispatch: InspectorCommandDispatch,
) -> RuntimeCommandTaskStep {
    let Some(session_id) = cmd.session_id else {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        ));
    };
    if conn
        .shared_worker_target_for_session(Some(session_id))
        .is_none()
    {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        ));
    }

    if !can_dispatch(cmd) {
        return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
            cmd.id,
            "UnknownMethod".to_owned(),
        ));
    }

    let action = dispatch.protocol_method();
    let pending = match start_shared_worker_frontend_inspector_dispatch(
        conn,
        cmd,
        dispatch.into_inspector_json(),
        cmd.terminal_response_delivery(),
    ) {
        Ok(pending) => pending,
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(message));
        }
    };

    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action,
        owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
        object_group: None,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: false,
        pending: PendingRuntimeCommandKind::SharedWorkerInspector {
            pending,
            binding_effect: None,
        },
    }))
}

pub(super) fn console_action_protocol_method(action: ConsoleAction) -> &'static str {
    match action {
        ConsoleAction::Enable => "Console.enable",
        ConsoleAction::Disable => "Console.disable",
        ConsoleAction::ClearMessages => "Console.clearMessages",
    }
}

pub(super) fn console_action_from_protocol_method(method: &str) -> Option<ConsoleAction> {
    match method {
        "Console.enable" => Some(ConsoleAction::Enable),
        "Console.disable" => Some(ConsoleAction::Disable),
        "Console.clearMessages" => Some(ConsoleAction::ClearMessages),
        _ => None,
    }
}

pub(super) fn start_console_inspector_shared_worker_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: ConsoleAction,
) -> RuntimeCommandTaskStep {
    let Some(session_id) = cmd.session_id else {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        ));
    };
    if conn
        .shared_worker_target_for_session(Some(session_id))
        .is_none()
    {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        ));
    }

    if !can_dispatch(cmd) {
        return RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(
            "UnknownMethod".to_owned(),
        ));
    }

    let pending = match start_shared_worker_frontend_inspector_dispatch(
        conn,
        cmd,
        cmd.json.to_owned(),
        cmd.terminal_response_delivery(),
    ) {
        Ok(pending) => pending,
        Err(message) if worker_runtime_is_unavailable(&message) => {
            if !apply_console_output_state_for_session(conn, cmd.session_id, action) {
                return RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(
                    "UnknownSession".to_owned(),
                ));
            }
            return RuntimeCommandTaskStep::Complete(CommandOutputPlan::success());
        }
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(message));
        }
    };

    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: console_action_protocol_method(action),
        owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
        object_group: None,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: false,
        pending: PendingRuntimeCommandKind::SharedWorkerInspector {
            pending,
            binding_effect: None,
        },
    }))
}

pub(super) fn start_console_inspector_service_worker_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: ConsoleAction,
) -> RuntimeCommandTaskStep {
    let Some(session_id) = cmd.session_id else {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        ));
    };
    if conn
        .service_worker_target_for_session(Some(session_id))
        .is_none()
    {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        ));
    }

    if !can_dispatch(cmd) {
        return RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(
            "UnknownMethod".to_owned(),
        ));
    }

    let pending = match start_service_worker_frontend_inspector_dispatch(
        conn,
        cmd,
        cmd.json.to_owned(),
        cmd.terminal_response_delivery(),
    ) {
        Ok(pending) => pending,
        Err(message) if message == "ServiceWorkerRuntimeUnavailable" => {
            if !apply_console_output_state_for_session(conn, cmd.session_id, action) {
                return RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(
                    "UnknownSession".to_owned(),
                ));
            }
            return RuntimeCommandTaskStep::Complete(CommandOutputPlan::success());
        }
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(message));
        }
    };

    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: console_action_protocol_method(action),
        owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
        object_group: None,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: false,
        pending: PendingRuntimeCommandKind::ServiceWorkerInspector { pending },
    }))
}

pub(super) fn try_start_pending_runtime_enable_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<RuntimeCommandTaskStep> {
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    Some(start_runtime_enable_command_for_owner(
        conn,
        cmd.id,
        owner_scope,
    ))
}

pub(super) fn start_runtime_enable_command_for_owner(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
) -> RuntimeCommandTaskStep {
    let has_loaded_page = match conn.runtime_session_owner_slot_for_owner(&owner_scope) {
        Ok(slot) => slot.has_loaded_page(),
        Err(_) if owner_scope.session_id().is_some() => {
            return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
                -32001,
                "Unknown sessionId",
            ));
        }
        Err(_) => {
            match conn.set_runtime_frontend_enabled_for_owner(&owner_scope, true) {
                SessionOwnerRuntimeFrontendEnableResult::Handled => {}
                SessionOwnerRuntimeFrontendEnableResult::UnknownSession => {
                    return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32001,
                        "Unknown sessionId",
                    ));
                }
            }
            return RuntimeCommandTaskStep::Complete(CommandOutputPlan::success());
        }
    };
    if !has_loaded_page {
        if conn.can_defer_initial_document_page_build() {
            match conn.set_runtime_frontend_enabled_for_owner(&owner_scope, true) {
                SessionOwnerRuntimeFrontendEnableResult::Handled => {}
                SessionOwnerRuntimeFrontendEnableResult::UnknownSession => {
                    return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32001,
                        "Unknown sessionId",
                    ));
                }
            }
            return RuntimeCommandTaskStep::Complete(CommandOutputPlan::success());
        }
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "NoDocumentLoaded",
        ));
    }
    start_pending_runtime_enable_events_phase(conn, command_id, owner_scope)
}

pub(super) fn start_pending_runtime_enable_events_phase(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
) -> RuntimeCommandTaskStep {
    match conn.start_runtime_enable_events_for_owner(&owner_scope) {
        Ok(pending) => RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
            command_id,
            action: "enable",
            owner_scope,
            object_group: None,
            release_object_ids: Vec::new(),
            release_object_group: None,
            await_promise: false,
            wait_for_deferred_reply: false,
            pending: PendingRuntimeCommandKind::Enable(pending),
        })),
        Err(message) if message == "NoDocumentLoaded" => {
            RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
        }
        Err(message) => RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message)),
    }
}

pub(super) fn runtime_remove_binding_should_skip_live_page_update(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
) -> bool {
    conn.target_runtime_session_state_for_owner(owner)
        .is_some_and(|state| state.runtime_frontend_enabled)
}

pub(super) fn try_start_pending_runtime_binding_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: RuntimeBindingCommand,
) -> Option<RuntimeCommandTaskStep> {
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let (name, execution_context_name, execution_context_id) = match action {
        RuntimeBindingCommand::Add => {
            let params = match cmd.get_params::<AddBindingParams>() {
                Ok(Some(params)) => params,
                _ => {
                    return Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32602,
                        "InvalidParams",
                    )));
                }
            };
            (
                params.name,
                params.execution_context_name,
                params.execution_context_id,
            )
        }
        RuntimeBindingCommand::Remove => {
            let params = match cmd
                .get_params::<chromiumoxide_cdp::cdp::js_protocol::runtime::RemoveBindingParams>()
            {
                Ok(Some(params)) => params,
                _ => {
                    return Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32602,
                        "InvalidParams",
                    )));
                }
            };
            (params.name, None, None)
        }
    };
    if conn.browser_contexts().next().is_none() {
        return Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        )));
    }
    if matches!(action, RuntimeBindingCommand::Add)
        && execution_context_id.is_some()
        && execution_context_name.is_some()
    {
        return Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32602,
            "executionContextName is mutually exclusive with executionContextId",
        )));
    }
    let should_persist =
        matches!(action, RuntimeBindingCommand::Remove) || execution_context_id.is_none();
    let mut task = RuntimeBindingCommandTask {
        action,
        renderer_policy: cmd.renderer_policy(),
        phase: RuntimeBindingPhase::LivePageUpdate,
        name,
        execution_context_name,
        execution_context_id,
        inspector_json: Some(cmd.json.to_owned()),
        command_response: None,
        response_delivery: cmd.terminal_response_delivery(),
        session_response_predecessor: None,
        session_response_succeeded: None,
        should_persist,
        skip_live_page_update_after_inspector_success: false,
    };
    let live_page_update_unavailable = conn
        .runtime_session_owner_slot_for_owner(&owner_scope)
        .is_ok_and(|slot| !slot.has_loaded_page())
        || should_persist
            && conn
                .runtime_session_owner_slot_for_owner(&owner_scope)
                .is_ok_and(|slot| slot.renderer_document_navigation_is_suspended());
    if live_page_update_unavailable {
        task.command_response = Some(RuntimeBindingCommandResponse::empty_success());
        let meta = RuntimeCommandCompletionMeta {
            command_id: cmd.id,
            action: task.action.label(),
            owner_scope: owner_scope.clone(),
            object_group: None,
            release_object_ids: Vec::new(),
            release_object_group: None,
            await_promise: false,
            wait_for_deferred_reply: false,
        };
        return Some(complete_runtime_binding_after_live_update(conn, meta, task));
    }
    if let Some(execution_context_id) = execution_context_id {
        return start_pending_runtime_binding_context_lookup_phase(
            conn,
            cmd.id,
            task,
            execution_context_id,
            owner_scope,
        );
    }
    if matches!(task.action, RuntimeBindingCommand::Add)
        || matches!(task.action, RuntimeBindingCommand::Remove)
            && runtime_remove_binding_should_skip_live_page_update(conn, &owner_scope)
    {
        task.skip_live_page_update_after_inspector_success = true;
    }
    let action_label = action.label();
    let pending = match action {
        RuntimeBindingCommand::Add => {
            start_pending_runtime_context_resolved_inspector_dispatch_with_delivery(
                conn,
                cmd,
                &owner_scope,
                action_label,
                cmd.json.to_owned(),
                task.response_delivery,
            )
        }
        RuntimeBindingCommand::Remove => start_pending_runtime_inspector_dispatch_with_delivery(
            conn,
            cmd,
            &owner_scope,
            cmd.json.to_owned(),
            task.response_delivery,
        ),
    };
    let pending = match pending {
        Ok(pending) => pending,
        Err(message) => {
            return Some(RuntimeCommandTaskStep::Complete(
                runtime_inspector_error_plan(cmd.id, message),
            ));
        }
    };
    Some(RuntimeCommandTaskStep::Pending(Box::new(
        PendingRuntimeCommandDispatch {
            command_id: cmd.id,
            action: action_label,
            owner_scope,
            object_group: None,
            release_object_ids: Vec::new(),
            release_object_group: None,
            await_promise: false,
            wait_for_deferred_reply: false,
            pending: PendingRuntimeCommandKind::BindingInspector { task, pending },
        },
    )))
}

pub(super) fn start_pending_runtime_routable_inspector_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    owner: &CommandOwnerScope,
    inspector_json: String,
    response_delivery: RendererInspectorResponseDelivery,
) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
    match cmd.renderer_policy().access() {
        CdpRendererCommandAccess::MainThread => {
            start_pending_runtime_inspector_dispatch_with_delivery(
                conn,
                cmd,
                owner,
                inspector_json,
                response_delivery,
            )
        }
        CdpRendererCommandAccess::Io => start_pending_runtime_io_inspector_dispatch(
            conn,
            cmd,
            owner,
            inspector_json,
            response_delivery,
        ),
        CdpRendererCommandAccess::OwnerIndependent => Err(
            "an owner-independent command cannot enter the Runtime Inspector dispatcher".to_owned(),
        ),
    }
}

pub(super) fn start_pending_runtime_io_inspector_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    owner: &CommandOwnerScope,
    inspector_json: String,
    response_delivery: RendererInspectorResponseDelivery,
) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
    if let Some(command_id) = cmd.id {
        let descriptor = RendererCommandDescriptor::from_frontend_policy(
            inspector_json,
            cmd.renderer_policy(),
            response_delivery,
        );
        conn.start_runtime_io_protocol_message_for_owner_with_deferred_response(
            owner, descriptor, command_id,
        )
    } else {
        conn.start_runtime_io_protocol_message_for_owner(owner, inspector_json)
    }
}

pub(super) fn start_pending_runtime_context_resolved_inspector_dispatch_with_delivery(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    owner: &CommandOwnerScope,
    action: &'static str,
    inspector_json: String,
    response_delivery: RendererInspectorResponseDelivery,
) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
    if let Some(command_id) = cmd.id {
        let descriptor = RendererCommandDescriptor::from_frontend_policy(
            inspector_json,
            cmd.renderer_policy(),
            response_delivery,
        );
        conn.start_runtime_protocol_message_with_context_resolution_for_owner_with_deferred_response(
            owner,
            action,
            descriptor,
            command_id,
        )
    } else {
        conn.start_runtime_protocol_message_with_context_resolution_for_owner(
            owner,
            action,
            inspector_json,
        )
    }
}

pub(super) fn start_pending_runtime_inspector_dispatch_with_delivery(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    owner: &CommandOwnerScope,
    inspector_json: String,
    response_delivery: RendererInspectorResponseDelivery,
) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
    if let Some(command_id) = cmd.id {
        let descriptor = RendererCommandDescriptor::from_frontend_policy(
            inspector_json,
            cmd.renderer_policy(),
            response_delivery,
        );
        conn.start_runtime_protocol_message_for_owner_with_deferred_response(
            owner, descriptor, command_id,
        )
    } else {
        conn.start_runtime_protocol_message_for_owner(owner, inspector_json)
    }
}

pub(super) fn start_shared_worker_frontend_inspector_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    inspector_json: String,
    response_delivery: RendererInspectorResponseDelivery,
) -> Result<PendingSharedWorkerRuntimeProtocolMessageDispatch, String> {
    if let Some(command_id) = cmd.id {
        let descriptor = RendererCommandDescriptor::from_frontend_policy(
            inspector_json,
            cmd.renderer_policy(),
            response_delivery,
        );
        conn.start_shared_worker_runtime_protocol_message_for_session_with_deferred_response(
            cmd.session_id,
            descriptor,
            command_id,
        )
    } else {
        conn.start_shared_worker_runtime_protocol_message_for_session(
            cmd.session_id,
            inspector_json,
        )
    }
}

pub(super) fn start_service_worker_frontend_inspector_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    inspector_json: String,
    response_delivery: RendererInspectorResponseDelivery,
) -> Result<PendingServiceWorkerRuntimeProtocolMessageDispatch, String> {
    if let Some(command_id) = cmd.id {
        let descriptor = RendererCommandDescriptor::from_frontend_policy(
            inspector_json,
            cmd.renderer_policy(),
            response_delivery,
        );
        conn.start_service_worker_runtime_protocol_message_for_session_with_deferred_response(
            cmd.session_id,
            descriptor,
            command_id,
        )
    } else {
        conn.start_service_worker_runtime_protocol_message_for_session(
            cmd.session_id,
            inspector_json,
        )
    }
}

pub(super) fn start_cdp_devtools_script_runtime_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    command_kind: RuntimeDevToolsScriptCommand,
) -> RuntimeCommandTaskStep {
    let (browser_context_id, target_id) =
        devtools_runtime_owner_identity_for_session(conn, cmd.session_id);
    let action = command_kind.action();
    let await_promise = runtime_command_awaits_promise(cmd, action);
    let command = match command_kind {
        RuntimeDevToolsScriptCommand::Evaluate => {
            AutomationCommand::EvaluateScript(build_cdp_evaluate_script_command(
                cmd,
                target_id.as_deref(),
                browser_context_id.as_deref(),
                await_promise,
            ))
        }
        RuntimeDevToolsScriptCommand::CallFunctionOn => {
            AutomationCommand::CallFunction(build_cdp_call_function_command(
                cmd,
                target_id.as_deref(),
                browser_context_id.as_deref(),
            ))
        }
    };
    let inspector_json = match prepare_pending_devtools_runtime_inspector_json(conn, cmd, &command)
    {
        Ok(json) => json,
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message));
        }
    };
    start_devtools_runtime_command(
        conn,
        cmd,
        command,
        inspector_json,
        await_promise,
        cmd.terminal_response_delivery(),
    )
}

pub(super) fn prepare_pending_devtools_runtime_inspector_json(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    command: &AutomationCommand,
) -> Result<String, String> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    prepare_pending_devtools_runtime_inspector_json_for_owner(conn, cmd, &owner, command)
}

pub(super) fn prepare_pending_devtools_runtime_inspector_json_for_owner(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    owner: &CommandOwnerScope,
    command: &AutomationCommand,
) -> Result<String, String> {
    match command {
        AutomationCommand::EvaluateScript(_) => Ok(cmd.json.to_owned()),
        AutomationCommand::CallFunction(command) => {
            prepare_pending_devtools_call_function_json_for_owner(conn, cmd, owner, command)
        }
        _ => Err("UnsupportedDevToolsCommand".to_owned()),
    }
}

pub(super) fn start_devtools_runtime_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    command: AutomationCommand,
    inspector_json: String,
    wait_for_deferred_reply: bool,
    response_delivery: RendererInspectorResponseDelivery,
) -> RuntimeCommandTaskStep {
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    start_devtools_runtime_command_for_owner(
        conn,
        cmd,
        command,
        inspector_json,
        wait_for_deferred_reply,
        response_delivery,
        owner_scope,
    )
}

pub(super) fn start_devtools_runtime_command_for_owner(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    command: AutomationCommand,
    inspector_json: String,
    wait_for_deferred_reply: bool,
    response_delivery: RendererInspectorResponseDelivery,
    owner_scope: CommandOwnerScope,
) -> RuntimeCommandTaskStep {
    let (action, action_label, await_promise) = match &command {
        AutomationCommand::EvaluateScript(command) => {
            (RuntimeAction::Evaluate, "evaluate", command.await_promise)
        }
        AutomationCommand::CallFunction(command) => (
            RuntimeAction::CallFunctionOn,
            "callFunctionOn",
            command.await_promise,
        ),
        _ => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                cmd.id,
                "UnsupportedDevToolsCommand".to_owned(),
            ));
        }
    };
    let object_group =
        runtime_object_group_for_command_result_for_owner(conn, cmd, &owner_scope, action);
    let pre_registered_await = match pre_register_runtime_await_if_needed(
        conn,
        await_promise,
        cmd.id,
        &owner_scope,
        object_group.as_deref(),
        action_label,
    ) {
        Ok(command_id) => command_id,
        Err(error) => {
            return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
                -32600,
                error.to_string(),
            ));
        }
    };
    let pending = match start_pending_runtime_context_resolved_inspector_dispatch_with_delivery(
        conn,
        cmd,
        &owner_scope,
        action_label,
        inspector_json,
        response_delivery,
    ) {
        Ok(pending) => pending,
        Err(message) => {
            forget_pre_registered_runtime_await(conn, pre_registered_await, &owner_scope);
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message));
        }
    };
    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: action_label,
        owner_scope,
        object_group,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise,
        wait_for_deferred_reply,
        pending: PendingRuntimeCommandKind::Inspector { pending },
    }))
}

pub(super) fn pre_register_runtime_await_if_needed(
    conn: &mut CdpConnection,
    await_promise: bool,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    object_group: Option<&str>,
    action: &'static str,
) -> Result<Option<u64>, DuplicatePendingRendererCommand> {
    if !await_promise {
        return Ok(None);
    }
    let Some(command_id) = command_id else {
        return Ok(None);
    };
    conn.try_register_pending_inspector_await_with_object_group_for_owner(
        command_id,
        owner,
        object_group,
    )?;
    conn.register_runtime_await_job_for_owner(command_id, owner, object_group, action);
    conn.trace_runtime_await_pending_registered(command_id, owner.session_id());
    Ok(Some(command_id))
}

pub(super) fn forget_pre_registered_runtime_await(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
) {
    if let Some(command_id) = command_id {
        conn.forget_pending_inspector_await_for_owner(command_id, owner);
    }
}
