use super::*;

pub(super) fn prepare_runtime_inspector_payload(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    preparation: RuntimeInspectorPayloadPreparation,
) -> Result<String, String> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    prepare_runtime_inspector_payload_for_owner(conn, cmd, &owner, preparation)
}

pub(super) fn prepare_runtime_inspector_payload_for_owner(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    owner: &CommandOwnerScope,
    preparation: RuntimeInspectorPayloadPreparation,
) -> Result<String, String> {
    match preparation {
        RuntimeInspectorPayloadPreparation::Passthrough => Ok(cmd.json.to_owned()),
        RuntimeInspectorPayloadPreparation::ValidateObjectOwner => {
            let object_ids = cmd
                .params
                .map(runtime_remote_object_ids_in_map)
                .unwrap_or_default();
            conn.validate_runtime_remote_object_ids_for_owner(owner, &object_ids)?;
            Ok(cmd.json.to_owned())
        }
        RuntimeInspectorPayloadPreparation::ValidatePrototypeOwner => {
            let object_ids = runtime_prototype_object_id_from_params(cmd.params)
                .map(|object_id| vec![object_id.to_owned()])
                .unwrap_or_default();
            conn.validate_runtime_remote_object_ids_for_owner(owner, &object_ids)?;
            Ok(cmd.json.to_owned())
        }
        RuntimeInspectorPayloadPreparation::PrepareCallFunctionOn => {
            let (browser_context_id, target_id) = conn
                .target_owner_identity_for_owner(owner)
                .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
                .unwrap_or((None, None));
            let command = build_cdp_call_function_command(
                cmd,
                target_id.as_deref(),
                browser_context_id.as_deref(),
            );
            prepare_pending_devtools_call_function_json_for_owner(conn, cmd, owner, &command)
        }
    }
}

pub(super) fn build_cdp_evaluate_script_command(
    cmd: &Cmd<'_>,
    target_id: Option<&str>,
    browser_context_id: Option<&str>,
    await_promise: bool,
) -> DevToolsEvaluateScriptCommand {
    DevToolsEvaluateScriptCommand {
        context: cmd.automation_context(target_id, browser_context_id),
        realm_id: None,
        world_name: None,
        expression: cdp_evaluate_expression_from_params(cmd.params).unwrap_or_default(),
        await_promise,
        user_gesture: cdp_runtime_user_gesture_from_params(cmd.params),
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: cdp_evaluate_result_ownership_from_params(cmd.params),
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: None,
    }
}

pub(super) fn cdp_evaluate_expression_from_params(
    params: Option<&Map<String, Value>>,
) -> Option<String> {
    params?
        .get("expression")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

pub(super) fn cdp_evaluate_result_ownership_from_params(
    params: Option<&Map<String, Value>>,
) -> DevToolsResultOwnership {
    if params
        .and_then(|params| params.get("returnByValue"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        DevToolsResultOwnership::ByValue
    } else {
        DevToolsResultOwnership::Root
    }
}

pub(super) fn cdp_runtime_user_gesture_from_params(params: Option<&Map<String, Value>>) -> bool {
    params
        .and_then(|params| params.get("userGesture"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub(super) fn devtools_runtime_owner_identity_for_session(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> (Option<String>, Option<String>) {
    if let Some(
        CdpSessionRoute::SharedWorkerTarget {
            browser_context_id,
            target_id,
        }
        | CdpSessionRoute::DedicatedWorkerTarget {
            browser_context_id,
            target_id,
        }
        | CdpSessionRoute::ServiceWorkerTarget {
            browser_context_id,
            target_id,
        },
    ) = conn.session_route(session_id)
    {
        return (Some(browser_context_id), Some(target_id));
    }
    conn.target_owner_identity_for_session(session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None))
}

pub(super) fn build_cdp_call_function_command(
    cmd: &Cmd<'_>,
    target_id: Option<&str>,
    browser_context_id: Option<&str>,
) -> DevToolsCallFunctionCommand {
    DevToolsCallFunctionCommand {
        context: cmd.automation_context(target_id, browser_context_id),
        realm_id: cdp_call_function_realm_id_from_params(cmd.params),
        world_name: None,
        object_id: runtime_object_id_from_params(cmd.params).map(DevToolsRemoteHandleId::from),
        this_parameter: None,
        function_declaration: cdp_call_function_declaration_from_params(cmd.params)
            .unwrap_or_default(),
        arguments: cdp_call_function_arguments_from_params(cmd.params),
        await_promise: cmd
            .params
            .and_then(|params| params.get("awaitPromise"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        user_gesture: cdp_runtime_user_gesture_from_params(cmd.params),
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: cdp_evaluate_result_ownership_from_params(cmd.params),
        object_group: runtime_object_group_from_params(cmd.params).map(str::to_owned),
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: None,
    }
}

pub(super) fn cdp_call_function_realm_id_from_params(
    params: Option<&Map<String, Value>>,
) -> Option<DevToolsRealmId> {
    params?
        .get("executionContextId")
        .and_then(Value::as_i64)
        .map(|id| DevToolsRealmId::from(id.to_string()))
}

pub(super) fn cdp_call_function_declaration_from_params(
    params: Option<&Map<String, Value>>,
) -> Option<String> {
    params?
        .get("functionDeclaration")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

pub(super) fn cdp_call_function_arguments_from_params(
    params: Option<&Map<String, Value>>,
) -> Vec<Value> {
    params
        .and_then(|params| params.get("arguments"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

pub(super) fn prepare_pending_devtools_call_function_json_for_owner(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    owner: &CommandOwnerScope,
    command: &DevToolsCallFunctionCommand,
) -> Result<String, String> {
    let object_ids = devtools_call_function_remote_object_ids(command);
    conn.validate_runtime_remote_object_ids_for_owner(owner, &object_ids)?;
    Ok(cmd.json.to_owned())
}

pub(super) fn start_pending_runtime_binding_context_lookup_phase(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    task: RuntimeBindingCommandTask,
    execution_context_id: i64,
    owner_scope: CommandOwnerScope,
) -> Option<RuntimeCommandTaskStep> {
    let pending = conn
        .start_child_default_execution_context_lookup_for_owner(&owner_scope, execution_context_id)
        .ok()?;
    Some(RuntimeCommandTaskStep::Pending(Box::new(
        PendingRuntimeCommandDispatch {
            command_id,
            action: task.action.label(),
            owner_scope,
            object_group: None,
            release_object_ids: Vec::new(),
            release_object_group: None,
            await_promise: false,
            wait_for_deferred_reply: false,
            pending: PendingRuntimeCommandKind::BindingContextLookup { task, pending },
        },
    )))
}

pub(super) fn start_pending_runtime_binding_inspector_phase(
    conn: &mut CdpConnection,
    completed: &RuntimeCommandCompletionMeta,
    task: RuntimeBindingCommandTask,
) -> RuntimeCommandTaskStep {
    let Some(inspector_json) = task.inspector_json.clone() else {
        return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
            completed.command_id,
            "InvalidParams".to_owned(),
        ));
    };
    let result = match task.action {
        RuntimeBindingCommand::Add => {
            if let Some(command_id) = completed.command_id {
                let descriptor = RendererCommandDescriptor::from_frontend_policy(
                    inspector_json,
                    task.renderer_policy,
                    task.response_delivery,
                );
                conn.start_runtime_protocol_message_with_context_resolution_for_owner_with_deferred_response(
                    &completed.owner_scope,
                    "addBinding",
                    descriptor,
                    command_id,
                )
            } else {
                conn.start_runtime_protocol_message_with_context_resolution_for_owner(
                    &completed.owner_scope,
                    "addBinding",
                    inspector_json,
                )
            }
        }
        RuntimeBindingCommand::Remove => {
            if let Some(command_id) = completed.command_id {
                let descriptor = RendererCommandDescriptor::from_frontend_policy(
                    inspector_json,
                    task.renderer_policy,
                    task.response_delivery,
                );
                conn.start_runtime_protocol_message_for_owner_with_deferred_response(
                    &completed.owner_scope,
                    descriptor,
                    command_id,
                )
            } else {
                conn.start_runtime_protocol_message_for_owner(
                    &completed.owner_scope,
                    inspector_json,
                )
            }
        }
    };
    match result {
        Ok(pending) => RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
            command_id: completed.command_id,
            action: task.action.label(),
            owner_scope: completed.owner_scope.clone(),
            object_group: None,
            release_object_ids: Vec::new(),
            release_object_group: None,
            await_promise: false,
            wait_for_deferred_reply: false,
            pending: PendingRuntimeCommandKind::BindingInspector { task, pending },
        })),
        Err(message) if message == "NoDocumentLoaded" => {
            let mut task = task;
            task.command_response = Some(RuntimeBindingCommandResponse::empty_success());
            match start_pending_runtime_binding_page_phase(
                conn,
                completed.command_id,
                task.clone(),
                completed.owner_scope.clone(),
            ) {
                Some(pending) => RuntimeCommandTaskStep::Pending(Box::new(pending)),
                None => complete_runtime_binding_after_live_update(conn, completed.clone(), task),
            }
        }
        Err(message) => RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
            completed.command_id,
            message,
        )),
    }
}

pub(super) fn start_pending_runtime_binding_page_phase(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    task: RuntimeBindingCommandTask,
    owner_scope: CommandOwnerScope,
) -> Option<PendingRuntimeCommandDispatch> {
    let pending = match task.phase {
        RuntimeBindingPhase::LivePageUpdate => match task.action {
            RuntimeBindingCommand::Add => conn.start_install_runtime_binding_for_owner(
                &owner_scope,
                &task.name,
                task.execution_context_name.as_deref(),
                task.execution_context_id,
            ),
            RuntimeBindingCommand::Remove => {
                conn.start_remove_runtime_binding_for_owner(&owner_scope, &task.name)
            }
        },
        RuntimeBindingPhase::StoredBindingsApply => {
            conn.start_apply_stored_runtime_bindings_for_owner(&owner_scope)
        }
    }
    .ok()?;
    Some(PendingRuntimeCommandDispatch {
        command_id,
        action: task.action.label(),
        owner_scope,
        object_group: None,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: false,
        pending: PendingRuntimeCommandKind::BindingPage { task, pending },
    })
}

pub(super) async fn route_registered_runtime_response_receiver_into(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    response_rx: RuntimeInspectorAsyncCompletionReceiver,
    routed_output: &mut RuntimeInspectorRoutedOutput,
) -> bool {
    let Some(command_id) = command_id else {
        return false;
    };
    let response = RuntimeInspectorResponseReady::for_owner(
        command_id,
        owner,
        response_rx
            .await
            .map_err(|_| "RuntimeInspectorResponseCanceled".to_owned()),
    );
    let Some(response) = conn.resolve_runtime_inspector_response_ready(response) else {
        return false;
    };
    let (_, output, renderer_output_predecessor) = response.into_renderer_command_output();
    if let Some(predecessor) = renderer_output_predecessor {
        routed_output.set_renderer_output_predecessor(predecessor);
    }
    route_runtime_command_output_into_routed_output(
        conn,
        output,
        Some(command_id),
        owner,
        routed_output,
    )
    .await
}

pub(super) async fn route_runtime_command_output_into_routed_output(
    conn: &mut CdpConnection,
    output: RendererRuntimeCommandOutput,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    routed_output: &mut RuntimeInspectorRoutedOutput,
) -> bool {
    let mut ordered_events = Vec::new();
    let saw_current_response = conn.route_renderer_runtime_command_output_for_owner_into(
        output,
        command_id,
        owner,
        &mut ordered_events,
    );
    routed_output.append_ordered_events(ordered_events);
    saw_current_response
}

pub(super) async fn route_renderer_command_turn_output_into_routed_output(
    conn: &mut CdpConnection,
    output: RendererCommandTurnOutput,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    response_flush: &crate::conn::CommandResponseFlushContext,
    routed_output: &mut RuntimeInspectorRoutedOutput,
) -> bool {
    let mut ordered_events = Vec::new();
    let mut post_response_events = Vec::new();
    let (saw_current_response, renderer_output_predecessor) = conn
        .route_renderer_command_turn_output_for_owner_into(
            output,
            command_id,
            owner,
            response_flush,
            &mut ordered_events,
            &mut post_response_events,
        );
    if let Some(predecessor) = renderer_output_predecessor {
        routed_output.set_renderer_output_predecessor(predecessor);
    }
    routed_output.append_ordered_events(ordered_events);
    routed_output.append_post_response_events(post_response_events);
    saw_current_response
}

pub(super) fn route_inspector_messages_into_routed_output(
    conn: &mut CdpConnection,
    messages: Vec<RendererRuntimeInspectorMessage>,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    routed_output: &mut RuntimeInspectorRoutedOutput,
) -> bool {
    let mut saw_current_response = false;
    for message in messages {
        let mut response_events = Vec::new();
        let mut background_events = Vec::new();
        saw_current_response |= conn
            .route_renderer_runtime_inspector_messages_for_owner_with_background_events_into(
                vec![message],
                command_id,
                owner,
                &mut response_events,
                &mut background_events,
            );
        routed_output.append_ordered_events(response_events);
        routed_output.append_ordered_events(background_events);
    }
    saw_current_response
}

pub(super) async fn complete_pending_runtime_inspector_command(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    completed_inspector: Result<CompletedRuntimeProtocolMessageDispatch, String>,
    timing_started: Option<std::time::Instant>,
    response_flush: &crate::conn::CommandResponseFlushContext,
) -> RuntimeCommandTaskStep {
    let (
        messages,
        mut renderer_response_rx,
        response_delivery,
        session_response_predecessor,
        session_response_succeeded,
    ) = match completed_inspector {
        Ok(mut completed_protocol) => {
            let session_response_succeeded = completed_protocol.session_response_succeeded();
            let session_response_predecessor = completed_protocol.session_response_predecessor();
            let response_delivery = completed_protocol.response_delivery();
            let renderer_response_rx = completed_protocol.take_deferred_response_receiver();
            match conn
                .complete_runtime_protocol_message_async(completed_protocol)
                .await
            {
                Ok(messages) => (
                    messages,
                    renderer_response_rx,
                    response_delivery,
                    session_response_predecessor,
                    session_response_succeeded,
                ),
                Err(message) => {
                    if let Some(command_id) = completed.command_id {
                        conn.forget_pending_inspector_await_for_owner(
                            command_id,
                            &completed.owner_scope,
                        );
                    }
                    if session_response_succeeded.is_some() {
                        tracing::debug!(
                            command_id = completed.command_id,
                            session_id = completed.session_id(),
                            error = %message,
                            "ignored browser-side completion error after the renderer session settled the terminal response"
                        );
                        (
                            None,
                            renderer_response_rx,
                            response_delivery,
                            session_response_predecessor,
                            session_response_succeeded,
                        )
                    } else {
                        if let Some(command_id) = completed.command_id {
                            let correlation = conn.take_renderer_call_for_frontend_for_owner(
                                &completed.owner_scope,
                                command_id,
                            );
                            if correlation.is_none() {
                                tracing::debug!(
                                    command_id,
                                    session_id = completed.session_id(),
                                    error = %message,
                                    "ignored renderer completion error after another route settled the frontend call"
                                );
                                return RuntimeCommandTaskStep::Complete(
                                    CommandOutputPlan::default(),
                                );
                            }
                        }
                        return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                            completed.command_id,
                            message,
                        ));
                    }
                }
            }
        }
        Err(message) => {
            if let Some(command_id) = completed.command_id {
                conn.forget_pending_inspector_await_for_owner(command_id, &completed.owner_scope);
                let correlation = conn
                    .take_renderer_call_for_frontend_for_owner(&completed.owner_scope, command_id);
                if correlation.is_none() {
                    tracing::debug!(
                        command_id,
                        session_id = completed.session_id(),
                        error = %message,
                        "ignored canceled renderer route after another route settled the frontend call"
                    );
                    return RuntimeCommandTaskStep::Complete(CommandOutputPlan::default());
                }
            }
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                completed.command_id,
                message,
            ));
        }
    };
    let initial_message_count = messages.as_ref().map_or(0, |messages| {
        messages
            .runtime_inspector_output()
            .map_or(0, |messages| messages.len())
    });
    if let Some(started) = timing_started {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            action = completed.action,
            stage = "runtime_inspector_dispatch_returned",
            output_items = initial_message_count,
            elapsed_ms = started.elapsed().as_millis(),
        );
    }

    let mut plan = CommandOutputPlan::default();
    let mut routed_output = RuntimeInspectorRoutedOutput::default();
    if let Some(predecessor) = session_response_predecessor {
        routed_output.set_renderer_output_predecessor(predecessor);
    }
    let mut saw_current_response = if let Some(messages) = messages {
        route_renderer_command_turn_output_into_routed_output(
            conn,
            messages,
            completed.command_id,
            &completed.owner_scope,
            response_flush,
            &mut routed_output,
        )
        .await
    } else {
        false
    };
    if saw_current_response {
        renderer_response_rx.take();
    }
    // Target teardown can settle the frontend command from a different
    // command turn before this renderer completion reaches the scheduler. In
    // that case the renderer-call correlation has already been consumed and
    // the deferred response sender can remain alive in the retired Page
    // command. Waiting on its receiver here would stall the entire CDP owner
    // after, for example, Page.crash interrupted an active Runtime.evaluate.
    // Treat the missing correlation as the command's already-settled
    // tombstone; any non-response Inspector output in this late completion is
    // still projected below.
    if !saw_current_response
        && renderer_response_rx.is_some()
        && completed.command_id.is_some_and(|command_id| {
            conn.renderer_runtime_command_cause_for_owner(&completed.owner_scope, command_id)
                .is_none()
        })
    {
        renderer_response_rx.take();
        saw_current_response = true;
    }
    if response_delivery == RendererInspectorResponseDelivery::SessionSink {
        // The attachment-scoped renderer session stream owns the terminal
        // response. The typed Page completion remains internal state only and
        // must not join or await the legacy per-command receiver.
        renderer_response_rx.take();
    }
    if response_delivery == RendererInspectorResponseDelivery::AdapterReply
        && !completed.wait_for_deferred_reply
        && let Some(renderer_response_rx) = renderer_response_rx.take()
    {
        let saw_deferred_response = route_registered_runtime_response_receiver_into(
            conn,
            completed.command_id,
            &completed.owner_scope,
            renderer_response_rx,
            &mut routed_output,
        )
        .await;
        saw_current_response |= saw_deferred_response;
    }
    if completed.await_promise {
        conn.trace_runtime_await_initial_dispatch_done(
            completed.command_id,
            completed.session_id(),
            initial_message_count,
            saw_current_response,
        );
    }
    if response_delivery == RendererInspectorResponseDelivery::AdapterReply
        && completed.wait_for_deferred_reply
        && (renderer_response_rx.is_some() || !saw_current_response)
        && completed.command_id.is_some()
    {
        return pending_runtime_deferred_inspector_reply_command(
            conn,
            completed,
            routed_output,
            renderer_response_rx,
        );
    }
    let succeeded = session_response_succeeded
        .unwrap_or_else(|| routed_output.command_response_succeeded(completed.command_id));
    if succeeded {
        routed_output.register_object_group_for_success(
            conn,
            &completed.owner_scope,
            completed.object_group.as_deref(),
        );
    }
    if succeeded {
        if let Some(console_action) = console_action_from_protocol_method(completed.action)
            && !apply_console_output_state_for_owner(conn, &completed.owner_scope, console_action)
        {
            let message = format!("ConsoleCommandCompletionFailed: {}", completed.action);
            if session_response_succeeded.is_some() {
                tracing::warn!(
                    command_id = completed.command_id,
                    session_id = completed.session_id(),
                    error = %message,
                    "could not apply browser projection after the terminal renderer response was published"
                );
            } else {
                return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                    completed.command_id,
                    message,
                ));
            }
        }
        if completed.action == "discardConsoleEntries" {
            advance_runtime_observable_cursors_to_current_for_owner(conn, &completed.owner_scope);
        }
        if completed.action == "disable" {
            if let Err(message) = apply_runtime_disable_projection_after_success_for_owner(
                conn,
                &completed.owner_scope,
            ) {
                if session_response_succeeded.is_some() {
                    tracing::warn!(
                        command_id = completed.command_id,
                        session_id = completed.session_id(),
                        error = %message,
                        "could not apply browser projection after the terminal renderer response was published"
                    );
                } else {
                    return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                        completed.command_id,
                        message,
                    ));
                }
            }
            if let Err(message) = conn
                .apply_runtime_binding_state_for_owner_async(&completed.owner_scope)
                .await
            {
                if session_response_succeeded.is_some() {
                    tracing::warn!(
                        command_id = completed.command_id,
                        session_id = completed.session_id(),
                        error = %message,
                        "could not apply browser projection after the terminal renderer response was published"
                    );
                } else {
                    return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                        completed.command_id,
                        message,
                    ));
                }
            }
        }
        if !completed.release_object_ids.is_empty() {
            conn.unregister_runtime_remote_object_ids_for_owner(
                &completed.owner_scope,
                &completed.release_object_ids,
            );
        }
        if let Some(object_group) = completed.release_object_group.as_deref() {
            conn.unregister_runtime_remote_object_group_for_owner(
                &completed.owner_scope,
                object_group,
            );
        }
        if completed.action == "runIfWaitingForDebugger"
            && conn.release_waiting_for_debugger_session(completed.session_id())
        {
            crate::domains::target::schedule_initial_document_target_url_navigation_after_debugger_resume(
                conn,
                completed.session_id(),
            );
        }
    }
    routed_output.push_ordered_into_plan(&mut plan, completed.command_id);
    if let Some(started) = timing_started {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            action = completed.action,
            stage = "runtime_inspector_plan_ready",
            elapsed_ms = started.elapsed().as_millis(),
        );
    }
    RuntimeCommandTaskStep::Complete(plan)
}

pub(super) fn pending_runtime_deferred_inspector_reply_command(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    routed_output: RuntimeInspectorRoutedOutput,
    renderer_response_rx: Option<RuntimeInspectorAsyncCompletionReceiver>,
) -> RuntimeCommandTaskStep {
    let claimed_await = completed.command_id.and_then(|command_id| {
        conn.claim_pending_inspector_await_for_scheduler_deferred_reply(
            command_id,
            &completed.owner_scope,
        )
    });
    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: completed.command_id,
        action: completed.action,
        owner_scope: completed.owner_scope,
        object_group: completed.object_group,
        release_object_ids: completed.release_object_ids,
        release_object_group: completed.release_object_group,
        await_promise: completed.await_promise,
        wait_for_deferred_reply: completed.wait_for_deferred_reply,
        pending: PendingRuntimeCommandKind::InspectorDeferredReply {
            routed_output,
            renderer_response_rx,
            claimed_await,
        },
    }))
}

pub(super) fn complete_pending_runtime_deferred_inspector_reply_command(
    completed: RuntimeCommandCompletionMeta,
    routed_output: RuntimeInspectorRoutedOutput,
) -> CommandOutputPlan {
    let mut plan = CommandOutputPlan::default();
    routed_output.push_ordered_into_plan(&mut plan, completed.command_id);
    plan
}

pub(super) fn push_runtime_protocol_event_or_background_event(
    plan: &mut CommandOutputPlan,
    command_id: Option<u64>,
    event: BackgroundProtocolEvent,
) {
    let Some(command_id) = command_id else {
        plan.push_background_event(event);
        return;
    };
    let is_command_response = event.protocol_message_id() == Some(command_id);
    if !is_command_response {
        plan.push_background_event(event);
        return;
    }
    let event = match event.into_command_response_payload() {
        Ok((_, _, payload)) => {
            push_command_response_payload_into_plan(plan, payload);
            return;
        }
        Err(event) => event,
    };
    let Some(message) = event.protocol_message().cloned() else {
        plan.push_background_event(event);
        return;
    };
    if !plan.push_runtime_inspector_protocol_response(message, Some(command_id)) {
        plan.push_background_event(event);
    }
}

pub(super) fn push_command_response_payload_into_plan(
    plan: &mut CommandOutputPlan,
    payload: BackgroundCommandResponsePayload,
) {
    match payload {
        BackgroundCommandResponsePayload::Success { result } => plan.push_result(result),
        BackgroundCommandResponsePayload::Error {
            code,
            message,
            data,
        } => plan.push_error_with_data(code, message, data),
    }
}

pub(super) fn complete_pending_runtime_enable_command(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    completed_enable: Result<CompletedRuntimeEnableEventsDispatch, String>,
) -> CommandOutputPlan {
    let replay = match completed_enable {
        Ok(completed_enable) => match conn.complete_runtime_enable_events(completed_enable) {
            Ok(replay) => replay,
            Err(message) => return CommandOutputPlan::error(-32000, message),
        },
        Err(message) => return CommandOutputPlan::error(-32000, message),
    };
    if let Err(message) =
        apply_runtime_enable_projection_after_success(conn, &completed.owner_scope)
    {
        return CommandOutputPlan::error(-32000, message);
    }
    let frame_id = conn.runtime_session_owner_frame_id_for_owner(&completed.owner_scope);

    // Runtime.enable reports its existing inventory before acknowledging the
    // enable. Keep replay and reply in one ordered plan, including worker
    // adapters that synthesize the replay outside the V8 session sink.
    let mut plan = CommandOutputPlan::default();
    for event in replay.into_events() {
        match event {
            RuntimeEnableReplayEvent::Context(event) => {
                if !should_emit_child_default_context_inventory_replay_once_for_owner(
                    conn,
                    &completed.owner_scope,
                    frame_id.as_deref(),
                    &event,
                ) {
                    continue;
                }
                // Runtime.enable is a command-local replay, not a second live
                // context producer. Record delivery only after the replay
                // cursor accepts this exact event; marking it while merely
                // preparing the replay would suppress the first delivery.
                apply_runtime_context_protocol_event_side_effects_for_owner_typed(
                    conn,
                    &event,
                    &completed.owner_scope,
                );
                let mut background_events = Vec::new();
                emit_runtime_context_protocol_background_event_typed(
                    &mut background_events,
                    event,
                    completed.session_id(),
                );
                for event in background_events {
                    plan.push_background_event(event);
                }
            }
            RuntimeEnableReplayEvent::Background(event) => {
                plan.push_background_event(event);
            }
        }
    }
    plan.push_success();
    plan
}

pub(super) fn apply_runtime_enable_projection_after_success(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
) -> Result<(), String> {
    match conn.set_runtime_frontend_enabled_for_owner(owner, true) {
        SessionOwnerRuntimeFrontendEnableResult::Handled => Ok(()),
        SessionOwnerRuntimeFrontendEnableResult::UnknownSession => {
            Err("Runtime.enable succeeded after session owner disappeared".to_owned())
        }
    }
}

pub(super) fn complete_pending_runtime_binding_context_lookup_command(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    mut task: RuntimeBindingCommandTask,
    completed_lookup: Result<CompletedRuntimeChildDefaultContextLookupDispatch, String>,
) -> RuntimeCommandTaskStep {
    if let Err(message) = completed_lookup
        .and_then(|lookup| conn.complete_child_default_execution_context_lookup(lookup))
    {
        return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
            completed.command_id,
            message,
        ));
    }

    // Child default realms are registered V8 Inspector contexts too. Installing
    // a native callback here would bypass the registering Inspector session.
    if matches!(task.action, RuntimeBindingCommand::Add)
        || matches!(task.action, RuntimeBindingCommand::Remove)
            && runtime_remove_binding_should_skip_live_page_update(conn, &completed.owner_scope)
    {
        task.skip_live_page_update_after_inspector_success = true;
    }
    start_pending_runtime_binding_inspector_phase(conn, &completed, task)
}

pub(super) async fn complete_pending_runtime_binding_inspector_command(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    mut task: RuntimeBindingCommandTask,
    completed_inspector: Result<CompletedRuntimeProtocolMessageDispatch, String>,
    response_flush: &crate::conn::CommandResponseFlushContext,
) -> RuntimeCommandTaskStep {
    let (
        messages,
        mut renderer_response_rx,
        response_delivery,
        session_response_predecessor,
        session_response_succeeded,
    ) = match completed_inspector {
        Ok(mut completed_protocol) => {
            let response_delivery = completed_protocol.response_delivery();
            let session_response_predecessor = completed_protocol.session_response_predecessor();
            let session_response_succeeded = completed_protocol.session_response_succeeded();
            let renderer_response_rx = completed_protocol.take_deferred_response_receiver();
            match conn
                .complete_runtime_protocol_message_async(completed_protocol)
                .await
            {
                Ok(messages) => (
                    messages,
                    renderer_response_rx,
                    response_delivery,
                    session_response_predecessor,
                    session_response_succeeded,
                ),
                Err(message) => {
                    if session_response_succeeded.is_some() {
                        tracing::warn!(
                            command_id = completed.command_id,
                            session_id = completed.session_id(),
                            error = %message,
                            "could not apply Runtime binding browser projection after the renderer session settled the terminal response"
                        );
                        (
                            None,
                            renderer_response_rx,
                            response_delivery,
                            session_response_predecessor,
                            session_response_succeeded,
                        )
                    } else {
                        return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                            completed.command_id,
                            message,
                        ));
                    }
                }
            }
        }
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                completed.command_id,
                message,
            ));
        }
    };
    debug_assert_eq!(response_delivery, task.response_delivery);
    if let Some(predecessor) = session_response_predecessor {
        predecessor.merge_into_same_stream_tail(&mut task.session_response_predecessor);
    }
    task.session_response_succeeded = session_response_succeeded;
    let mut routed_output = RuntimeInspectorRoutedOutput::default();
    let saw_current_response = if let Some(messages) = messages {
        route_renderer_command_turn_output_into_routed_output(
            conn,
            messages,
            completed.command_id,
            &completed.owner_scope,
            response_flush,
            &mut routed_output,
        )
        .await
    } else {
        false
    };
    if saw_current_response {
        renderer_response_rx.take();
    }
    if response_delivery == RendererInspectorResponseDelivery::SessionSink {
        renderer_response_rx.take();
    }
    if response_delivery == RendererInspectorResponseDelivery::AdapterReply
        && let Some(renderer_response_rx) = renderer_response_rx
    {
        route_registered_runtime_response_receiver_into(
            conn,
            completed.command_id,
            &completed.owner_scope,
            renderer_response_rx,
            &mut routed_output,
        )
        .await;
    }
    if response_delivery == RendererInspectorResponseDelivery::AdapterReply {
        record_runtime_binding_command_response_from_routed_events(
            &mut task,
            routed_output.events(),
            completed.command_id,
        );
    }
    if routed_output.background_event_count() > 0 {
        tracing::debug!(
            events = routed_output.background_event_count(),
            "dropping unexpected Runtime binding background events during inspector phase"
        );
    }
    if !runtime_binding_command_response_succeeded(&task) {
        return RuntimeCommandTaskStep::Complete(runtime_binding_output_plan(task));
    }
    if task.skip_live_page_update_after_inspector_success {
        return complete_runtime_binding_after_live_update(conn, completed, task);
    }
    match start_pending_runtime_binding_page_phase(
        conn,
        completed.command_id,
        task.clone(),
        completed.owner_scope.clone(),
    ) {
        Some(pending) => RuntimeCommandTaskStep::Pending(Box::new(pending)),
        None => complete_runtime_binding_after_live_update(conn, completed, task),
    }
}

pub(super) fn complete_pending_runtime_binding_page_command(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    task: RuntimeBindingCommandTask,
    completed_page: Result<CompletedRuntimeBindingPageCommandDispatch, String>,
) -> RuntimeCommandTaskStep {
    match completed_page {
        Ok(completed_page) => {
            if let Err(message) = conn.complete_runtime_binding_page_command(completed_page)
                && message != "NoDocumentLoaded"
            {
                tracing::warn!(
                    binding = %task.name,
                    error = %message,
                    "Runtime binding page command succeeded before completion failed"
                );
            }
        }
        Err(message) => {
            tracing::warn!(
                binding = %task.name,
                error = %message,
                "Runtime binding page command failed"
            );
        }
    }
    match task.phase {
        RuntimeBindingPhase::LivePageUpdate => {
            complete_runtime_binding_after_live_update(conn, completed, task)
        }
        RuntimeBindingPhase::StoredBindingsApply => {
            RuntimeCommandTaskStep::Complete(runtime_binding_output_plan(task))
        }
    }
}

pub(super) fn complete_runtime_binding_after_live_update(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    mut task: RuntimeBindingCommandTask,
) -> RuntimeCommandTaskStep {
    if task.should_persist {
        let persistence = match task.action {
            RuntimeBindingCommand::Add => persist_runtime_binding_definition_for_owner(
                conn,
                &completed.owner_scope,
                task.name.clone(),
                task.execution_context_name.clone(),
            ),
            RuntimeBindingCommand::Remove => remove_runtime_binding_definitions_for_owner(
                conn,
                &completed.owner_scope,
                &task.name,
            ),
        };
        if let Err(message) = persistence {
            tracing::warn!(
                binding = %task.name,
                error = %message,
                "Runtime binding command succeeded before owner binding persistence update failed"
            );
            return RuntimeCommandTaskStep::Complete(runtime_binding_output_plan(task));
        }
        task.phase = RuntimeBindingPhase::StoredBindingsApply;
        if let Some(pending) = start_pending_runtime_binding_page_phase(
            conn,
            completed.command_id,
            task.clone(),
            completed.owner_scope.clone(),
        ) {
            return RuntimeCommandTaskStep::Pending(Box::new(pending));
        }
    }
    RuntimeCommandTaskStep::Complete(runtime_binding_output_plan(task))
}

pub(super) fn runtime_binding_output_plan(task: RuntimeBindingCommandTask) -> CommandOutputPlan {
    let mut plan = CommandOutputPlan::default();
    if let Some(predecessor) = task.session_response_predecessor {
        plan.set_renderer_output_predecessor(predecessor);
    }
    if task.session_response_succeeded.is_some() {
        return plan;
    }
    match task.command_response {
        Some(response) => response.push_into_plan(&mut plan),
        None => plan.push_error(-32000, "MissingRuntimeBindingCommandResponse"),
    }
    plan
}

pub(super) fn record_runtime_binding_command_response_from_routed_events(
    task: &mut RuntimeBindingCommandTask,
    events: &[BackgroundProtocolEvent],
    command_id: Option<u64>,
) {
    let Some(command_id) = command_id else {
        return;
    };
    for event in events {
        let Some((event_command_id, _, payload)) = event.command_response_payload_ref() else {
            tracing::debug!(
                ?event,
                "dropping non-command Runtime binding inspector event after routing"
            );
            continue;
        };
        if event_command_id != Some(command_id) {
            tracing::debug!(
                ?event_command_id,
                command_id,
                "dropping non-matching Runtime binding inspector command response after routing"
            );
            continue;
        }
        let response = match payload {
            BackgroundCommandResponsePayloadRef::Success { .. } => {
                RuntimeBindingCommandResponse::Success
            }
            BackgroundCommandResponsePayloadRef::Error { code, message, .. } => {
                RuntimeBindingCommandResponse::Error {
                    code,
                    message: message.to_owned(),
                }
            }
        };
        if task.command_response.is_some() {
            tracing::warn!("overwriting Runtime binding command response");
        }
        task.command_response = Some(response);
    }
}

pub(super) fn runtime_binding_command_response_succeeded(task: &RuntimeBindingCommandTask) -> bool {
    if let Some(succeeded) = task.session_response_succeeded {
        return succeeded;
    }
    task.command_response
        .as_ref()
        .is_some_and(RuntimeBindingCommandResponse::succeeded)
}

pub(super) fn runtime_inspector_error_plan(
    command_id: Option<u64>,
    message: String,
) -> CommandOutputPlan {
    if message == "Duplicate `id` in protocol request" {
        return CommandOutputPlan::error(-32600, message);
    }
    let error = match message.as_str() {
        "NoDocumentLoaded" => DevToolsError::new(DevToolsErrorKind::Internal, "NoDocumentLoaded"),
        "InvalidParams" => DevToolsError::new(DevToolsErrorKind::InvalidArgument, "InvalidParams"),
        _ => {
            if command_id.is_some() {
                DevToolsError::new(DevToolsErrorKind::Internal, message)
            } else {
                DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "Runtime inspector dispatch failed",
                )
            }
        }
    };
    CommandOutputPlan::from_devtools_error(error)
}

pub(super) fn parse_synthesized_runtime_command(
    raw_json: String,
) -> Result<ParsedCdpCommand, String> {
    ParsedCdpCommand::parse_str(raw_json)
        .map_err(|error| format!("invalid synthesized Runtime Inspector command: {error}"))
}

pub(super) fn worker_runtime_is_unavailable(message: &str) -> bool {
    matches!(
        message,
        "SharedWorkerRuntimeUnavailable" | "DedicatedWorkerRuntimeUnavailable"
    )
}
