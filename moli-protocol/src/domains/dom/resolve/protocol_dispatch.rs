use super::*;

pub(super) fn build_cdp_get_attributes_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsGetAttributesCommand, PendingDomCommandStartError> {
    let params: GetAttributesParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    let Some(cdp_node_id) = cdp_id_from_i64(*params.node_id.inner()) else {
        return Err(PendingDomCommandStartError::invalid_params());
    };
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsGetAttributesCommand {
        context: cmd.automation_context(target_id.as_deref(), browser_context_id.as_deref()),
        reference: DevToolsDomNodeReference::FrontendNodeId(cdp_node_id),
    })
}

pub(super) fn start_devtools_get_attributes_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsGetAttributesCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    if let DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) = command.reference {
        return start_document_frontend_node_binding_command(
            conn,
            command_id,
            owner,
            frontend_node_id,
            PendingDomCommandKind::ResolveFrontendNodeForGetAttributes { frontend_node_id },
        );
    }
    let reference = command.reference;
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = start_document_node_attributes_for_reference(page, reference)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::GetAttributesLive,
        pending,
    }))
}

pub(super) fn flatten_dom_attributes(attributes: Vec<DevToolsDomAttribute>) -> Vec<String> {
    let mut out = Vec::with_capacity(attributes.len() * 2);
    for attribute in attributes {
        out.push(attribute.name);
        out.push(attribute.value);
    }
    out
}

pub(super) fn start_document_frontend_node_binding_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    frontend_node_id: u32,
    kind: PendingDomCommandKind,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = page
        .start_document_frontend_node_binding(renderer_inspector_session_id, frontend_node_id)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind,
        pending,
    }))
}

pub(super) fn start_devtools_get_text_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsGetTextCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    if let DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) = command.reference {
        return start_document_frontend_node_binding_command(
            conn,
            command_id,
            owner,
            frontend_node_id,
            PendingDomCommandKind::ResolveFrontendNodeForGetText { frontend_node_id },
        );
    }
    let reference = command.reference;
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = start_document_node_text_for_reference(page, reference)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::GetTextLive,
        pending,
    }))
}

pub(super) fn start_devtools_get_property_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsGetPropertyCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    if let DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) = command.reference {
        return start_document_frontend_node_binding_command(
            conn,
            command_id,
            owner,
            frontend_node_id,
            PendingDomCommandKind::ResolveFrontendNodeForGetProperty {
                frontend_node_id,
                name: command.name,
            },
        );
    }
    let reference = command.reference;
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = start_document_node_property_for_reference(page, reference, &command.name)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::GetPropertyLive,
        pending,
    }))
}

pub(in crate::domains::dom) fn push_nodes_by_backend_ids_to_frontend(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<CommandOutputPlan> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    if !target_owner_exists_for_owner(conn, &owner) {
        return Some(CommandOutputPlan::error(-31998, "BrowserContextNotLoaded"));
    }
    match build_cdp_push_nodes_by_backend_ids_command(conn, cmd) {
        Ok(_) => None,
        Err(error) => Some(CommandOutputPlan::error(error.code, error.message)),
    }
}

pub(super) fn build_cdp_push_nodes_by_backend_ids_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsPushNodesByBackendIdsCommand, PendingDomCommandStartError> {
    let params: PushNodesByBackendIdsToFrontendParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    let backend_node_ids = params
        .backend_node_ids
        .iter()
        .map(|backend_node_id| cdp_id_from_i64(*backend_node_id.inner()))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(PendingDomCommandStartError::invalid_params)?;
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsPushNodesByBackendIdsCommand {
        context: cmd.automation_context(target_id.as_deref(), browser_context_id.as_deref()),
        backend_node_ids,
    })
}

pub(in crate::domains::dom) fn get_outer_html(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<CommandOutputPlan> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    if !target_owner_exists_for_owner(conn, &owner) {
        return Some(CommandOutputPlan::error(-31998, "BrowserContextNotLoaded"));
    }
    None
}

pub(super) fn build_cdp_get_outer_html_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<DevToolsGetOuterHtmlCommand>, PendingDomCommandStartError> {
    let params: GetOuterHtmlParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    if params.reference.object_id.is_some() {
        return Ok(None);
    }
    let reference = devtools_node_reference_from_ids(
        params.reference.node_id,
        params.reference.backend_node_id,
    )
    .ok_or_else(PendingDomCommandStartError::invalid_params)?;
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(Some(DevToolsGetOuterHtmlCommand {
        context: cmd.automation_context(target_id.as_deref(), browser_context_id.as_deref()),
        reference: Some(reference),
        include_shadow_dom: params.include_shadow_dom,
    }))
}

pub(crate) async fn execute_devtools_dom_command_async(
    conn: &mut CdpConnection,
    command: AutomationCommand,
) -> Result<AutomationResult, DevToolsError> {
    let context = devtools_dom_command_context(&command)?;
    let target_id = context
        .target_id
        .as_ref()
        .map(|target_id| target_id.to_string());
    let owner = conn
        .command_owner_scope_for_devtools_context(context)
        .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))?;
    if let Some(target_id) = target_id.as_deref()
        && conn.target_session_route_for_target_id(target_id).is_none()
    {
        let result =
            super::child_frame::execute_devtools_dom_command(conn, target_id, &owner, command)
                .await
                .map_err(DevToolsError::from)?;
        return Ok(result);
    }

    execute_devtools_dom_command_for_owner(conn, &owner, command).await
}

pub(super) fn devtools_dom_command_context(
    command: &AutomationCommand,
) -> Result<&crate::automation::AutomationContext, DevToolsError> {
    let context = match command {
        AutomationCommand::QuerySelector(command) => &command.context,
        AutomationCommand::GetAttributes(command) => &command.context,
        AutomationCommand::GetText(command) => &command.context,
        AutomationCommand::GetProperty(command) => &command.context,
        AutomationCommand::PushNodesByBackendIds(command) => &command.context,
        AutomationCommand::GetOuterHtml(command) => &command.context,
        AutomationCommand::DescribeNode(command) => &command.context,
        AutomationCommand::GetFrameOwner(command) => &command.context,
        AutomationCommand::GetNodeForLocation(command) => &command.context,
        AutomationCommand::ResolveNode(command) => &command.context,
        AutomationCommand::ScrollIntoViewIfNeeded(command) => &command.context,
        AutomationCommand::DomObjectReference(command)
            if matches!(
                command.operation,
                DevToolsDomObjectReferenceOperation::GetBoxModel
                    | DevToolsDomObjectReferenceOperation::GetContentQuads
            ) =>
        {
            &command.context
        }
        AutomationCommand::SetFileInputFiles(command) => &command.context,
        AutomationCommand::DomGeometry(command) => &command.context,
        _ => {
            return Err(DevToolsError::new(
                DevToolsErrorKind::Unsupported,
                "UnsupportedDevToolsCommand",
            ));
        }
    };
    Ok(context)
}

pub(super) async fn execute_devtools_dom_command_for_owner(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    command: AutomationCommand,
) -> Result<AutomationResult, DevToolsError> {
    match command {
        AutomationCommand::QuerySelector(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                AutomationCommand::QuerySelector(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        AutomationCommand::GetAttributes(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                AutomationCommand::GetAttributes(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        AutomationCommand::GetText(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                AutomationCommand::GetText(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        AutomationCommand::GetProperty(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                AutomationCommand::GetProperty(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        AutomationCommand::PushNodesByBackendIds(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                AutomationCommand::PushNodesByBackendIds(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        AutomationCommand::GetOuterHtml(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                AutomationCommand::GetOuterHtml(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        AutomationCommand::DescribeNode(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                AutomationCommand::DescribeNode(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        AutomationCommand::GetFrameOwner(command) => {
            let immediate_command = command.clone();
            let Some(pending) = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                AutomationCommand::GetFrameOwner(command),
            )
            .map_err(DevToolsError::from)?
            else {
                let result = complete_devtools_dom_command(
                    conn,
                    AutomationCommand::GetFrameOwner(immediate_command),
                )
                .map_err(DevToolsError::from)?;
                return devtools_get_frame_owner_result_from_value(&result)
                    .map(AutomationResult::GetFrameOwner);
            };

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        AutomationCommand::ResolveNode(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                AutomationCommand::ResolveNode(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        AutomationCommand::ScrollIntoViewIfNeeded(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                AutomationCommand::ScrollIntoViewIfNeeded(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        command
        @ (AutomationCommand::DomGeometry(_) | AutomationCommand::GetNodeForLocation(_)) => {
            let pending = start_devtools_dom_command_for_owner(conn, None, owner, command)
                .map_err(DevToolsError::from)?
                .ok_or_else(|| {
                    DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand")
                })?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        AutomationCommand::DomObjectReference(command)
            if matches!(
                command.operation,
                DevToolsDomObjectReferenceOperation::GetBoxModel
                    | DevToolsDomObjectReferenceOperation::GetContentQuads
            ) =>
        {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                AutomationCommand::DomObjectReference(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        AutomationCommand::SetFileInputFiles(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                AutomationCommand::SetFileInputFiles(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        _ => Err(DevToolsError::new(
            DevToolsErrorKind::Unsupported,
            "UnsupportedDevToolsCommand",
        )),
    }
}

pub(super) async fn await_pending_devtools_dom_command_result(
    conn: &mut CdpConnection,
    mut pending: PendingDomCommandDispatch,
) -> Result<AutomationResult, DevToolsError> {
    loop {
        let completed = Box::pin(pending.wait()).await;
        match complete_pending_dom_command_result(conn, completed) {
            DevToolsDomCommandTaskStep::Pending(next) => {
                pending = *next;
            }
            DevToolsDomCommandTaskStep::Complete(result) => return *result,
        }
    }
}

pub(super) fn devtools_get_frame_owner_result_from_value(
    result: &Value,
) -> Result<DevToolsGetFrameOwnerResult, DevToolsError> {
    let node_id = result
        .get("nodeId")
        .and_then(Value::as_u64)
        .and_then(|node_id| u32::try_from(node_id).ok())
        .ok_or_else(|| {
            DevToolsError::new(DevToolsErrorKind::Internal, "MissingFrameOwnerNodeId")
        })?;
    let backend_node_id = result
        .get("backendNodeId")
        .and_then(Value::as_u64)
        .and_then(|backend_node_id| u32::try_from(backend_node_id).ok())
        .ok_or_else(|| {
            DevToolsError::new(
                DevToolsErrorKind::Internal,
                "MissingFrameOwnerBackendNodeId",
            )
        })?;
    Ok(DevToolsGetFrameOwnerResult {
        node_id,
        backend_node_id,
    })
}

pub(in crate::domains::dom) fn renderer_backend_node_id_for_reference(
    reference: &DevToolsDomNodeReference,
) -> Option<u32> {
    match reference {
        DevToolsDomNodeReference::BackendNodeId(backend_node_id)
            if is_renderer_backend_node_id(*backend_node_id) =>
        {
            Some(*backend_node_id)
        }
        DevToolsDomNodeReference::FrontendNodeId(_)
        | DevToolsDomNodeReference::BackendNodeId(_) => None,
    }
}

pub(in crate::domains::dom) fn scroll_into_view_if_needed(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<CommandOutputPlan> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    if !target_owner_exists_for_owner(conn, &owner) {
        return Some(CommandOutputPlan::error(-31998, "BrowserContextNotLoaded"));
    }
    None
}

pub(super) fn build_cdp_scroll_into_view_if_needed_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<DevToolsScrollIntoViewIfNeededCommand>, PendingDomCommandStartError> {
    let params: ScrollIntoViewIfNeededParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    if params.reference.object_id.is_some() {
        return Ok(None);
    }
    let rect = validated_scroll_into_view_rect(params.rect)?;
    let reference = devtools_node_reference_from_ids(
        params.reference.node_id,
        params.reference.backend_node_id,
    );
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(Some(DevToolsScrollIntoViewIfNeededCommand {
        context: cmd.automation_context(target_id.as_deref(), browser_context_id.as_deref()),
        reference,
        rect,
    }))
}

pub(super) fn start_devtools_scroll_into_view_if_needed_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsScrollIntoViewIfNeededCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let Some(reference) = command.reference else {
        return Err(PendingDomCommandStartError::node_not_found());
    };
    if let DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) = reference {
        return start_document_frontend_node_binding_command(
            conn,
            command_id,
            owner,
            frontend_node_id,
            PendingDomCommandKind::ResolveFrontendNodeForScrollIntoViewIfNeeded {
                frontend_node_id,
                rect: command.rect,
            },
        );
    }
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let (pending, kind) = start_scroll_into_view_for_reference(page, reference, command.rect)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind,
        pending,
    }))
}
