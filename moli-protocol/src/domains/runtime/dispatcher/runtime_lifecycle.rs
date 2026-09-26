use super::*;

pub(super) fn start_pending_runtime_disable_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> RuntimeCommandTaskStep {
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let response_delivery = cmd.terminal_response_delivery();
    start_runtime_disable_command_for_owner(conn, cmd, owner_scope, response_delivery)
}

pub(super) fn start_runtime_disable_command_for_owner(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    owner_scope: CommandOwnerScope,
    response_delivery: RendererInspectorResponseDelivery,
) -> RuntimeCommandTaskStep {
    if !conn
        .runtime_session_owner_slot_for_owner(&owner_scope)
        .is_ok_and(|slot| slot.has_loaded_page())
    {
        return RuntimeCommandTaskStep::Complete(disable_command_output_plan_sync_for_owner(
            conn,
            &owner_scope,
        ));
    }

    let pending = match start_pending_runtime_inspector_dispatch_with_delivery(
        conn,
        cmd,
        &owner_scope,
        cmd.json.to_owned(),
        response_delivery,
    ) {
        Ok(pending) => pending,
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message));
        }
    };

    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: "disable",
        owner_scope,
        object_group: None,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: false,
        pending: PendingRuntimeCommandKind::Inspector { pending },
    }))
}

pub(super) fn start_runtime_run_if_waiting_for_debugger_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> RuntimeCommandTaskStep {
    if !conn
        .runtime_session_owner_slot(cmd.session_id)
        .is_ok_and(|slot| slot.has_loaded_page())
    {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::success());
    }

    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let pending = match start_pending_runtime_inspector_dispatch_with_delivery(
        conn,
        cmd,
        &owner_scope,
        cmd.json.to_owned(),
        cmd.terminal_response_delivery(),
    ) {
        Ok(pending) => pending,
        Err(message) if message == "NoDocumentLoaded" => {
            return RuntimeCommandTaskStep::Complete(CommandOutputPlan::success());
        }
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message));
        }
    };

    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: "runIfWaitingForDebugger",
        owner_scope,
        object_group: None,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: false,
        pending: PendingRuntimeCommandKind::Inspector { pending },
    }))
}

pub(super) fn apply_runtime_disable_projection_after_success_for_owner(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
) -> Result<(), String> {
    let was_enabled = conn
        .target_runtime_session_state_for_owner(owner)
        .is_some_and(|state| state.runtime_frontend_enabled);
    match conn.set_runtime_frontend_enabled_for_owner(owner, false) {
        SessionOwnerRuntimeFrontendEnableResult::Handled => {
            advance_runtime_observable_cursors_to_current_for_owner(conn, owner);
            if was_enabled {
                clear_runtime_binding_definitions_for_owner(conn, owner)?;
            }
            Ok(())
        }
        SessionOwnerRuntimeFrontendEnableResult::UnknownSession => {
            Err("Runtime.disable succeeded after session owner disappeared".to_owned())
        }
    }
}

pub(super) fn start_runtime_discard_console_entries_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> RuntimeCommandTaskStep {
    if !conn
        .runtime_session_owner_slot(cmd.session_id)
        .is_ok_and(|slot| slot.has_loaded_page())
    {
        advance_runtime_observable_cursors_to_current_for_session_owner(conn, cmd.session_id);
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::success());
    }
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let pending = match start_pending_runtime_inspector_dispatch_with_delivery(
        conn,
        cmd,
        &owner_scope,
        cmd.json.to_owned(),
        cmd.terminal_response_delivery(),
    ) {
        Ok(pending) => pending,
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message));
        }
    };
    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: "discardConsoleEntries",
        owner_scope,
        object_group: None,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: false,
        pending: PendingRuntimeCommandKind::Inspector { pending },
    }))
}
