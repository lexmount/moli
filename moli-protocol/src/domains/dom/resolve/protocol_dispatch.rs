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
    let session = conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = page
        .start_document_node_attributes(
            command
                .reference
                .into_renderer_reference_for_protocol(session, command.context.protocol),
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
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
    require_published: bool,
    kind: PendingDomCommandKind,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = if require_published {
        page.start_document_frontend_node_binding(renderer_inspector_session_id, frontend_node_id)
    } else {
        page.start_document_internal_frontend_node_binding(
            renderer_inspector_session_id,
            frontend_node_id,
        )
    }
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
    let session = conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = page
        .start_document_node_text(
            command
                .reference
                .into_renderer_reference_for_protocol(session, command.context.protocol),
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
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
    let session = conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = page
        .start_document_node_property(
            command
                .reference
                .into_renderer_reference_for_protocol(session, command.context.protocol),
            &command.name,
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
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
    let session = conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = page
        .start_scroll_node_into_view_if_needed(
            reference.into_renderer_reference_for_protocol(session, command.context.protocol),
            command.rect,
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
    let kind = PendingDomCommandKind::RendererNodeScrollIntoViewIfNeeded;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind,
        pending,
    }))
}
