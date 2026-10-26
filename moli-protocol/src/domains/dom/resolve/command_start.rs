use super::*;

pub(super) fn start_pending_dom_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let Some(action) = cmd.parse_action::<DomAction>() else {
        return Ok(None);
    };
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    if action == DomAction::DiscardSearchResults && !target_owner_exists_for_owner(conn, &owner) {
        return Ok(None);
    }
    if !target_owner_exists_for_owner(conn, &owner) {
        return Err(PendingDomCommandStartError {
            code: -31998,
            message: "BrowserContextNotLoaded".to_owned(),
        });
    }
    if action.requires_document_access()
        && let Err(message) = conn.ensure_document_accessible_for_owner(&owner)
    {
        return Err(PendingDomCommandStartError {
            code: -32000,
            message: format!("{message:#}"),
        });
    }
    if action == DomAction::GetDocument {
        let command = build_cdp_get_document_command(
            conn,
            cmd,
            PendingDomDocumentSnapshotOperation::GetDocument,
        )?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            AutomationCommand::GetDocument(command),
        );
    }
    if action == DomAction::GetFlattenedDocument {
        let command = build_cdp_get_document_command(
            conn,
            cmd,
            PendingDomDocumentSnapshotOperation::GetFlattenedDocument,
        )?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            AutomationCommand::GetDocument(command),
        );
    }
    if action == DomAction::RequestChildNodes {
        let command = build_cdp_request_child_nodes_command(conn, cmd)?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            AutomationCommand::RequestChildNodes(command),
        );
    }
    if action == DomAction::QuerySelector {
        let command = build_cdp_query_selector_command(conn, cmd, false)?;
        return start_query_selector_command(conn, cmd.id, &owner, command, true);
    }
    if action == DomAction::QuerySelectorAll {
        let command = build_cdp_query_selector_command(conn, cmd, true)?;
        return start_query_selector_command(conn, cmd.id, &owner, command, true);
    }
    if action == DomAction::PerformSearch {
        let command = search::build_cdp_perform_search_command(conn, cmd)?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            AutomationCommand::PerformSearch(command),
        );
    }
    if action == DomAction::GetSearchResults {
        let command = search::build_cdp_get_search_results_command(conn, cmd)?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            AutomationCommand::GetSearchResults(command),
        );
    }
    if action == DomAction::DiscardSearchResults {
        let Some(command) = search::build_cdp_discard_search_results_command(conn, cmd) else {
            return Ok(None);
        };
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            AutomationCommand::DiscardSearchResults(command),
        );
    }
    if action == DomAction::SetNodeStackTracesEnabled {
        return stack_traces::start_set_node_stack_traces_enabled_command(conn, cmd);
    }
    if action == DomAction::GetNodeStackTraces {
        return stack_traces::start_get_node_stack_traces_command(conn, cmd);
    }
    if action == DomAction::ResolveNode {
        let command = build_cdp_resolve_node_command(conn, cmd)?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            AutomationCommand::ResolveNode(command),
        );
    }
    if action == DomAction::GetFrameOwner {
        let command = build_cdp_get_frame_owner_command(conn, cmd)?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            AutomationCommand::GetFrameOwner(command),
        );
    }
    if action == DomAction::GetAttributes {
        let command = build_cdp_get_attributes_command(conn, cmd)?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            AutomationCommand::GetAttributes(command),
        );
    }
    if action == DomAction::GetNodeForLocation {
        let command = build_cdp_get_node_for_location_command(conn, cmd)?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            AutomationCommand::GetNodeForLocation(command),
        );
    }
    if action == DomAction::RequestNode {
        let command = build_cdp_dom_object_reference_command(
            conn,
            cmd,
            DevToolsDomObjectReferenceOperation::RequestNode,
        )?
        .ok_or_else(PendingDomCommandStartError::invalid_params)?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            AutomationCommand::DomObjectReference(command),
        );
    }
    if action == DomAction::DescribeNode {
        let Some(command) = build_cdp_describe_node_command(conn, cmd)? else {
            let params: DescribeNodeParams = match cmd.get_params() {
                Ok(Some(params)) => params,
                _ => return Err(PendingDomCommandStartError::invalid_params()),
            };
            let command = build_cdp_dom_object_reference_command(
                conn,
                cmd,
                DevToolsDomObjectReferenceOperation::DescribeNode {
                    depth: params.depth,
                    pierce: params.pierce,
                },
            )?
            .ok_or_else(PendingDomCommandStartError::invalid_params)?;
            return start_devtools_dom_command_for_owner(
                conn,
                cmd.id,
                &owner,
                AutomationCommand::DomObjectReference(command),
            );
        };
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            AutomationCommand::DescribeNode(command),
        );
    }
    if action == DomAction::GetOuterHtml {
        let params: GetOuterHtmlParams = match cmd.get_params() {
            Ok(Some(params)) => params,
            _ => return Err(PendingDomCommandStartError::invalid_params()),
        };
        if params.reference.object_id.is_some() {
            let Some(command) = build_cdp_dom_object_reference_command(
                conn,
                cmd,
                DevToolsDomObjectReferenceOperation::GetOuterHtml {
                    include_shadow_dom: params.include_shadow_dom,
                },
            )?
            else {
                return Ok(None);
            };
            return start_devtools_dom_command_for_owner(
                conn,
                cmd.id,
                &owner,
                AutomationCommand::DomObjectReference(command),
            );
        }
        let Some(command) = build_cdp_get_outer_html_command(conn, cmd)? else {
            return Ok(None);
        };
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            AutomationCommand::GetOuterHtml(command),
        );
    }
    if action == DomAction::ScrollIntoViewIfNeeded {
        let params: ScrollIntoViewIfNeededParams = match cmd.get_params() {
            Ok(Some(params)) => params,
            _ => return Err(PendingDomCommandStartError::invalid_params()),
        };
        if params.reference.object_id.is_some() {
            let rect = validated_scroll_into_view_rect(params.rect)?;
            let Some(command) = build_cdp_dom_object_reference_command(
                conn,
                cmd,
                DevToolsDomObjectReferenceOperation::ScrollIntoViewIfNeeded { rect },
            )?
            else {
                return Ok(None);
            };
            return start_devtools_dom_command_for_owner(
                conn,
                cmd.id,
                &owner,
                AutomationCommand::DomObjectReference(command),
            );
        }
        let Some(command) = build_cdp_scroll_into_view_if_needed_command(conn, cmd)? else {
            return Ok(None);
        };
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            AutomationCommand::ScrollIntoViewIfNeeded(command),
        );
    }
    if action == DomAction::Focus {
        return start_cdp_dom_focus_command(conn, cmd).map(Some);
    }
    if action == DomAction::SetFileInputFiles {
        let Some(command) = set_file_input::build_cdp_set_file_input_files_command(conn, cmd)?
        else {
            return set_file_input::start_cdp_set_file_input_files_by_node_reference(conn, cmd);
        };
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            AutomationCommand::SetFileInputFiles(command),
        );
    }
    if matches!(
        action,
        DomAction::SetAttributeValue | DomAction::RemoveAttribute
    ) {
        return start_cdp_dom_attribute_mutation_command(conn, cmd, action).map(Some);
    }
    if matches!(
        action,
        DomAction::MoveTo
            | DomAction::SetAttributesAsText
            | DomAction::SetNodeName
            | DomAction::SetNodeValue
            | DomAction::SetOuterHtml
    ) {
        return start_cdp_dom_edit_command(conn, cmd, action).map(Some);
    }
    if action == DomAction::PushNodesByBackendIdsToFrontend {
        let command = build_cdp_push_nodes_by_backend_ids_command(conn, cmd)?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            AutomationCommand::PushNodesByBackendIds(command),
        );
    }
    if matches!(action, DomAction::GetBoxModel | DomAction::GetContentQuads) {
        let operation = match action {
            DomAction::GetBoxModel => DevToolsDomGeometryOperation::GetBoxModel,
            DomAction::GetContentQuads => DevToolsDomGeometryOperation::GetContentQuads,
            _ => unreachable!("guarded by matches! above"),
        };
        let Some(command) = build_cdp_dom_geometry_command(conn, cmd, operation)? else {
            let operation = match action {
                DomAction::GetBoxModel => DevToolsDomObjectReferenceOperation::GetBoxModel,
                DomAction::GetContentQuads => DevToolsDomObjectReferenceOperation::GetContentQuads,
                _ => unreachable!("guarded by matches! above"),
            };
            let command = build_cdp_dom_object_reference_command(conn, cmd, operation)?
                .ok_or_else(PendingDomCommandStartError::invalid_params)?;
            return start_devtools_dom_command_for_owner(
                conn,
                cmd.id,
                &owner,
                AutomationCommand::DomObjectReference(command),
            );
        };
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            AutomationCommand::DomGeometry(command),
        );
    }
    if action == DomAction::RemoveNode {
        let Some(command) = build_cdp_remove_node_command(conn, cmd)? else {
            return Ok(None);
        };
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            AutomationCommand::RemoveNode(command),
        );
    }

    Ok(None)
}

pub(super) fn start_cdp_dom_focus_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<PendingDomCommandDispatch, PendingDomCommandStartError> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let params: NodeReferenceParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        Ok(None) => NodeReferenceParams::default(),
        Err(_) => return Err(PendingDomCommandStartError::invalid_params()),
    };
    if params.node_id.is_none() && params.backend_node_id.is_none() {
        let object_id = params
            .object_id
            .ok_or_else(|| PendingDomCommandStartError {
                code: -32000,
                message: "Either nodeId, backendNodeId or objectId must be specified".to_owned(),
            })?;
        return start_dom_object_reference_operation(
            conn,
            cmd.id,
            &owner,
            DevToolsRemoteHandleId::from(object_id),
            PendingDomObjectReferenceOperation::Focus,
        )?
        .ok_or_else(PendingDomCommandStartError::node_not_found);
    }

    let reference = devtools_node_reference_from_ids(params.node_id, params.backend_node_id)
        .ok_or_else(PendingDomCommandStartError::invalid_params)?;
    let missing_node_message = if matches!(reference, DevToolsDomNodeReference::FrontendNodeId(_)) {
        "Could not find node with given id"
    } else {
        "No node found for given backend id"
    };
    let session = conn.target_renderer_runtime_inspector_session_id_for_owner(&owner);
    let page = loaded_page_mut_for_owner(conn, &owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = page
        .start_focus_document_node(reference.into_renderer_reference(session))
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(PendingDomCommandDispatch {
        command_id: cmd.id,
        owner_scope: owner,
        kind: PendingDomCommandKind::Focus {
            missing_node_message,
        },
        pending,
    })
}

pub(super) fn start_cdp_dom_attribute_mutation_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: DomAction,
) -> Result<PendingDomCommandDispatch, PendingDomCommandStartError> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let (frontend_node_id, mutation) = match action {
        DomAction::SetAttributeValue => {
            let params: SetAttributeValueParams = match cmd.get_params() {
                Ok(Some(params)) => params,
                _ => return Err(PendingDomCommandStartError::invalid_params()),
            };
            let Some(frontend_node_id) = cdp_id_from_i64(*params.node_id.inner()) else {
                return Err(PendingDomCommandStartError::invalid_params());
            };
            (
                frontend_node_id,
                RendererDomAttributeMutation::Set {
                    name: params.name,
                    value: params.value,
                },
            )
        }
        DomAction::RemoveAttribute => {
            let params: RemoveAttributeParams = match cmd.get_params() {
                Ok(Some(params)) => params,
                _ => return Err(PendingDomCommandStartError::invalid_params()),
            };
            let Some(frontend_node_id) = cdp_id_from_i64(*params.node_id.inner()) else {
                return Err(PendingDomCommandStartError::invalid_params());
            };
            (
                frontend_node_id,
                RendererDomAttributeMutation::Remove { name: params.name },
            )
        }
        _ => unreachable!("attribute mutation command requires an attribute mutation action"),
    };

    let session = conn.target_renderer_runtime_inspector_session_id_for_owner(&owner);
    let page = loaded_page_mut_for_owner(conn, &owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let reference =
        DevToolsDomNodeReference::FrontendNodeId(frontend_node_id).into_renderer_reference(session);
    let pending = page
        .start_mutate_document_node_attribute(reference, mutation)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(PendingDomCommandDispatch {
        command_id: cmd.id,
        owner_scope: owner,
        kind: PendingDomCommandKind::MutateAttribute,
        pending,
    })
}

pub(super) fn start_cdp_dom_edit_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: DomAction,
) -> Result<PendingDomCommandDispatch, PendingDomCommandStartError> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let edit = super::edit::renderer_dom_edit_from_cdp(cmd, action)?;
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(&owner);
    let page = loaded_page_mut_for_owner(conn, &owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = page
        .start_edit_document_node(renderer_inspector_session_id, edit)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(PendingDomCommandDispatch {
        command_id: cmd.id,
        owner_scope: owner,
        kind: PendingDomCommandKind::EditDocumentNode,
        pending,
    })
}

pub(super) fn build_cdp_remove_node_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<DevToolsRemoveNodeCommand>, PendingDomCommandStartError> {
    let params: NodeReferenceParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    if params.object_id.is_some() {
        return Ok(None);
    }
    let reference = devtools_node_reference_from_ids(params.node_id, params.backend_node_id)
        .ok_or_else(PendingDomCommandStartError::invalid_params)?;
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(Some(DevToolsRemoveNodeCommand {
        context: cmd.automation_context(target_id.as_deref(), browser_context_id.as_deref()),
        reference,
    }))
}

pub(super) fn start_devtools_remove_node_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsRemoveNodeCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let session = conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = page
        .start_remove_document_node(
            command
                .reference
                .into_renderer_reference_for_protocol(session, command.context.protocol),
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::RemoveNode,
        pending,
    }))
}

pub(super) fn build_cdp_dom_geometry_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    operation: DevToolsDomGeometryOperation,
) -> Result<Option<DevToolsDomGeometryCommand>, PendingDomCommandStartError> {
    let params: NodeReferenceParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    if params.object_id.is_some() {
        return Ok(None);
    }
    let reference = devtools_node_reference_from_ids(params.node_id, params.backend_node_id)
        .ok_or_else(PendingDomCommandStartError::invalid_params)?;
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(Some(DevToolsDomGeometryCommand {
        context: cmd.automation_context(target_id.as_deref(), browser_context_id.as_deref()),
        reference,
        operation,
    }))
}

pub(super) fn start_devtools_dom_geometry_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsDomGeometryCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let session = conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = page
        .start_document_geometry_for_node(
            command
                .reference
                .into_renderer_reference_for_protocol(session, command.context.protocol),
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
    let kind = PendingDomCommandKind::RendererNodeGeometry {
        operation: command.operation,
    };
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind,
        pending,
    }))
}

pub(super) fn build_cdp_describe_node_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<DevToolsDescribeNodeCommand>, PendingDomCommandStartError> {
    let params: DescribeNodeParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    if params.reference.object_id.is_some() {
        return Ok(None);
    }
    let reference = devtools_node_reference_from_ids(
        params.reference.node_id,
        params.reference.backend_node_id,
    );
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(Some(DevToolsDescribeNodeCommand {
        context: cmd.automation_context(target_id.as_deref(), browser_context_id.as_deref()),
        reference,
        depth: params.depth,
        pierce: params.pierce,
    }))
}

pub(super) fn start_devtools_describe_node_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsDescribeNodeCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let Some(reference) = command.reference else {
        return Err(PendingDomCommandStartError::node_not_found());
    };
    let top_frame_id = top_frame_id_for_owner(conn, owner);
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let include_whitespace = dom_agent_includes_whitespace_for_owner(conn, owner);
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let reference = reference.into_renderer_reference_for_protocol(
        renderer_inspector_session_id.clone(),
        command.context.protocol,
    );
    let pending = page
        .start_document_node_snapshot_for_node_in_inspector_session(
            renderer_inspector_session_id,
            include_whitespace,
            reference,
            command.depth,
            command.pierce,
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::DescribeNodeObjectReference {
            cached_object_node: None,
            top_frame_id,
        },
        pending,
    }))
}

pub(super) fn build_cdp_request_child_nodes_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsRequestChildNodesCommand, PendingDomCommandStartError> {
    let params: RequestChildNodesParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    let Some(cdp_node_id_value) = cdp_id_from_i64(*params.node_id.inner()) else {
        return Err(PendingDomCommandStartError::invalid_params());
    };
    let Some(depth) = optional_i64_to_i32(params.depth) else {
        return Err(PendingDomCommandStartError::invalid_params());
    };
    let depth = depth.unwrap_or_else(default_describe_depth);
    if depth == 0 {
        return Err(PendingDomCommandStartError {
            code: -32000,
            message: INVALID_REQUEST_CHILD_NODES_DEPTH_MESSAGE.to_owned(),
        });
    }
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsRequestChildNodesCommand {
        context: cmd.automation_context(target_id.as_deref(), browser_context_id.as_deref()),
        reference: DevToolsDomNodeReference::FrontendNodeId(cdp_node_id_value),
        depth,
        pierce: params.pierce.unwrap_or(false),
    })
}

pub(super) fn start_devtools_request_child_nodes_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsRequestChildNodesCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let top_frame_id = top_frame_id_for_owner(conn, owner);
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let include_whitespace = dom_agent_includes_whitespace_for_owner(conn, owner);
    let next_depth = if command.depth > 0 {
        command.depth - 1
    } else {
        command.depth
    };
    let missing_node_message = match (&command.reference, command.context.protocol) {
        (DevToolsDomNodeReference::FrontendNodeId(_), crate::automation::FrontendProtocol::Cdp) => {
            None
        }
        (DevToolsDomNodeReference::FrontendNodeId(_), _) => {
            Some("Could not find node with given id")
        }
        (DevToolsDomNodeReference::BackendNodeId(_), _) => Some("InvalidNode"),
    };
    let reference = command.reference.into_renderer_reference_for_protocol(
        renderer_inspector_session_id.clone(),
        command.context.protocol,
    );
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = page
        .start_document_child_node_snapshot_events_for_node(
            renderer_inspector_session_id,
            include_whitespace,
            reference,
            next_depth,
            command.pierce,
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        pending,
        kind: PendingDomCommandKind::SetChildNodesSnapshotForBackendNode {
            after: PendingSetChildNodesAfter::EmptyResult,
            top_frame_id,
            missing_node_message,
        },
    }))
}

pub(super) fn complete_devtools_request_child_nodes_command(
    _conn: &mut CdpConnection,
    _command: DevToolsRequestChildNodesCommand,
    _out: &mut DomCommandOutput,
) -> Result<(), PendingDomCommandStartError> {
    Err(PendingDomCommandStartError {
        code: -32000,
        message: "RequestChildNodesRequiresPendingRendererCapture".to_owned(),
    })
}

pub(super) fn build_cdp_query_selector_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    multiple: bool,
) -> Result<DevToolsQuerySelectorCommand, PendingDomCommandStartError> {
    let (cdp_node_id_value, selector) = if multiple {
        let params: QuerySelectorAllParams = match cmd.get_params() {
            Ok(Some(params)) => params,
            _ => return Err(PendingDomCommandStartError::invalid_params()),
        };
        let Some(cdp_node_id_value) = cdp_id_from_i64(*params.node_id.inner()) else {
            return Err(PendingDomCommandStartError::invalid_params());
        };
        (cdp_node_id_value, params.selector)
    } else {
        let params: QuerySelectorParams = match cmd.get_params() {
            Ok(Some(params)) => params,
            _ => return Err(PendingDomCommandStartError::invalid_params()),
        };
        let Some(cdp_node_id_value) = cdp_id_from_i64(*params.node_id.inner()) else {
            return Err(PendingDomCommandStartError::invalid_params());
        };
        (cdp_node_id_value, params.selector)
    };
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsQuerySelectorCommand {
        context: cmd.automation_context(target_id.as_deref(), browser_context_id.as_deref()),
        root: Some(DevToolsDomNodeReference::FrontendNodeId(cdp_node_id_value)),
        selector,
        multiple,
    })
}

pub(super) fn start_devtools_query_selector_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsQuerySelectorCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    start_query_selector_command(conn, command_id, owner, command, false)
}

fn start_query_selector_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsQuerySelectorCommand,
    publish_node_paths: bool,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let top_frame_id = top_frame_id_for_owner(conn, owner);
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let include_whitespace = dom_agent_includes_whitespace_for_owner(conn, owner);
    let Some(reference) = command.root else {
        let page =
            loaded_page_mut_for_owner(conn, owner).ok_or_else(|| PendingDomCommandStartError {
                code: -32000,
                message: "Could not find node with given id".to_owned(),
            })?;
        let pending = page
            .start_document_query_selector_for_document_in_inspector_session(
                renderer_inspector_session_id,
                include_whitespace,
                command.selector,
                command.multiple,
            )
            .map_err(PendingDomCommandStartError::renderer_error)?;
        return Ok(Some(PendingDomCommandDispatch {
            command_id,
            owner_scope: owner.clone(),
            kind: PendingDomCommandKind::QuerySelectorLive {
                multiple: command.multiple,
            },
            pending,
        }));
    };

    let publish_node_paths =
        publish_node_paths && matches!(reference, DevToolsDomNodeReference::FrontendNodeId(_));
    let reference = reference.into_renderer_reference_for_protocol(
        renderer_inspector_session_id.clone(),
        command.context.protocol,
    );
    let page =
        loaded_page_mut_for_owner(conn, owner).ok_or_else(|| PendingDomCommandStartError {
            code: -32000,
            message: "Could not find node with given id".to_owned(),
        })?;
    let (pending, kind) = if publish_node_paths {
        (
            page.start_document_query_selector_with_child_node_snapshot_events_for_node(
                renderer_inspector_session_id,
                include_whitespace,
                reference,
                command.selector,
                command.multiple,
            ),
            PendingDomCommandKind::QuerySelectorSetChildNodesLive {
                multiple: command.multiple,
                top_frame_id,
            },
        )
    } else {
        (
            page.start_document_query_selector_for_node_in_inspector_session(
                renderer_inspector_session_id,
                include_whitespace,
                reference,
                command.selector,
                command.multiple,
            ),
            PendingDomCommandKind::QuerySelectorLive {
                multiple: command.multiple,
            },
        )
    };
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind,
        pending: pending.map_err(PendingDomCommandStartError::renderer_error)?,
    }))
}

pub(super) fn build_cdp_get_document_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    operation: PendingDomDocumentSnapshotOperation,
) -> Result<DevToolsGetDocumentCommand, PendingDomCommandStartError> {
    let params: GetDocumentParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        Ok(None) => match operation {
            PendingDomDocumentSnapshotOperation::GetDocument => GetDocumentParams::default(),
            PendingDomDocumentSnapshotOperation::GetFlattenedDocument => GetDocumentParams {
                depth: Some(-1),
                pierce: None,
            },
        },
        Err(_) => return Err(PendingDomCommandStartError::invalid_params()),
    };
    let Some(depth) = optional_i64_to_i32(params.depth) else {
        return Err(PendingDomCommandStartError::invalid_params());
    };
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsGetDocumentCommand {
        context: cmd.automation_context(target_id.as_deref(), browser_context_id.as_deref()),
        depth,
        pierce: params.pierce.unwrap_or(false),
        flattened: matches!(
            operation,
            PendingDomDocumentSnapshotOperation::GetFlattenedDocument
        ),
    })
}

pub(super) fn start_devtools_dom_command_for_owner(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: AutomationCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    match command {
        AutomationCommand::GetFrameOwner(command) => {
            start_devtools_get_frame_owner_command(conn, command_id, owner, command)
        }
        AutomationCommand::GetAttributes(command) => {
            start_devtools_get_attributes_command(conn, command_id, owner, command)
        }
        AutomationCommand::GetText(command) => {
            start_devtools_get_text_command(conn, command_id, owner, command)
        }
        AutomationCommand::GetProperty(command) => {
            start_devtools_get_property_command(conn, command_id, owner, command)
        }
        AutomationCommand::GetDocument(command) => {
            start_devtools_get_document_command(conn, command_id, owner, command)
        }
        AutomationCommand::RequestChildNodes(command) => {
            start_devtools_request_child_nodes_command(conn, command_id, owner, command)
        }
        AutomationCommand::QuerySelector(command) => {
            start_devtools_query_selector_command(conn, command_id, owner, command)
        }
        AutomationCommand::PerformSearch(command) => {
            search::start_devtools_perform_search_command(conn, command_id, owner, command)
        }
        AutomationCommand::GetSearchResults(command) => {
            search::start_devtools_get_search_results_command(conn, command_id, owner, command)
        }
        AutomationCommand::DiscardSearchResults(command) => {
            search::start_devtools_discard_search_results_command(conn, command_id, owner, command)
        }
        AutomationCommand::GetNodeForLocation(command) => {
            start_devtools_get_node_for_location_command(conn, command_id, owner, command)
        }
        AutomationCommand::ResolveNode(command) => {
            start_devtools_resolve_node_command(conn, command_id, owner, command)
        }
        AutomationCommand::DescribeNode(command) => {
            start_devtools_describe_node_command(conn, command_id, owner, command)
        }
        AutomationCommand::DomObjectReference(command) => {
            start_devtools_dom_object_reference_command(conn, command_id, owner, command)
        }
        AutomationCommand::SetFileInputFiles(command) => {
            set_file_input::start_devtools_set_file_input_files_command(
                conn, command_id, owner, command,
            )
        }
        AutomationCommand::PushNodesByBackendIds(command) => {
            start_devtools_push_nodes_by_backend_ids_command(conn, command_id, owner, command)
        }
        AutomationCommand::GetOuterHtml(command) => {
            start_devtools_get_outer_html_command(conn, command_id, owner, command)
        }
        AutomationCommand::DomGeometry(command) => {
            start_devtools_dom_geometry_command(conn, command_id, owner, command)
        }
        AutomationCommand::ScrollIntoViewIfNeeded(command) => {
            start_devtools_scroll_into_view_if_needed_command(conn, command_id, owner, command)
        }
        AutomationCommand::RemoveNode(command) => {
            start_devtools_remove_node_command(conn, command_id, owner, command)
        }
        _ => Err(PendingDomCommandStartError {
            code: -32000,
            message: "UnsupportedDevToolsCommand".to_owned(),
        }),
    }
}

pub(super) fn complete_devtools_dom_command(
    conn: &mut CdpConnection,
    command: AutomationCommand,
) -> Result<Value, PendingDomCommandStartError> {
    match command {
        AutomationCommand::GetFrameOwner(command) => {
            complete_devtools_get_frame_owner_command(conn, command)
        }
        AutomationCommand::GetAttributes(_)
        | AutomationCommand::GetText(_)
        | AutomationCommand::GetProperty(_) => Err(PendingDomCommandStartError {
            code: -32000,
            message: "MissingDomCommand".to_owned(),
        }),
        AutomationCommand::PushNodesByBackendIds(_) => Err(PendingDomCommandStartError {
            code: -32000,
            message: "MissingDomCommand".to_owned(),
        }),
        AutomationCommand::DescribeNode(_)
        | AutomationCommand::GetOuterHtml(_)
        | AutomationCommand::ScrollIntoViewIfNeeded(_) => Err(PendingDomCommandStartError {
            code: -32000,
            message: "MissingDomCommand".to_owned(),
        }),
        _ => Err(PendingDomCommandStartError {
            code: -32000,
            message: "UnsupportedDevToolsCommand".to_owned(),
        }),
    }
}

pub(super) fn start_devtools_get_document_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsGetDocumentCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let operation = if command.flattened {
        PendingDomDocumentSnapshotOperation::GetFlattenedDocument
    } else {
        PendingDomDocumentSnapshotOperation::GetDocument
    };
    let top_frame_id = top_frame_id_for_owner(conn, owner);
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let include_whitespace = dom_agent_includes_whitespace_for_owner(conn, owner);
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let depth = match operation {
        PendingDomDocumentSnapshotOperation::GetDocument => command.depth.unwrap_or(2),
        PendingDomDocumentSnapshotOperation::GetFlattenedDocument => command.depth.unwrap_or(-1),
    };
    let pending = page
        .start_document_node_snapshot_for_document(
            renderer_inspector_session_id,
            include_whitespace,
            depth,
            command.pierce,
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::DocumentSnapshot {
            operation,
            top_frame_id,
        },
        pending,
    }))
}

pub(super) fn build_cdp_get_frame_owner_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsGetFrameOwnerCommand, PendingDomCommandStartError> {
    let params: GetFrameOwnerParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsGetFrameOwnerCommand {
        context: cmd.automation_context(target_id.as_deref(), browser_context_id.as_deref()),
        frame_id: DevToolsFrameId::new(params.frame_id.as_ref()),
    })
}

pub(super) fn start_devtools_get_frame_owner_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsGetFrameOwnerCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let page =
        loaded_page_mut_for_owner(conn, owner).ok_or_else(|| PendingDomCommandStartError {
            code: -32000,
            message: "Frame with the given id does not belong to the target.".to_owned(),
        })?;
    let frame_id = command.frame_id.into_string();
    let pending = page
        .start_child_frame_owner_node_reference(&frame_id, renderer_inspector_session_id)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::GetFrameOwner { frame_id },
        pending,
    }))
}

pub(super) fn complete_devtools_get_frame_owner_command(
    _conn: &mut CdpConnection,
    _command: DevToolsGetFrameOwnerCommand,
) -> Result<Value, PendingDomCommandStartError> {
    Err(PendingDomCommandStartError {
        code: -32000,
        message: "GetFrameOwnerRequiresPendingRendererResolution".to_owned(),
    })
}

pub(super) fn build_cdp_get_node_for_location_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsGetNodeForLocationCommand, PendingDomCommandStartError> {
    let params: GetNodeForLocationParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsGetNodeForLocationCommand {
        context: cmd.automation_context(target_id.as_deref(), browser_context_id.as_deref()),
        x: params.x as f64,
        y: params.y as f64,
        include_user_agent_shadow_dom: params.include_user_agent_shadow_dom.unwrap_or(false),
        ignore_pointer_events_none: params.ignore_pointer_events_none.unwrap_or(false),
    })
}

pub(super) fn start_devtools_get_node_for_location_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsGetNodeForLocationCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let top_frame_id = conn
        .target_session_owner_frame_tree_identity_for_owner(owner)
        .map(|identity| identity.0)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let inspector_session_id = conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = page
        .start_document_hit_test(
            inspector_session_id,
            command.x,
            command.y,
            command.include_user_agent_shadow_dom,
            command.ignore_pointer_events_none,
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::GetNodeForLocation { top_frame_id },
        pending,
    }))
}

pub(super) fn complete_devtools_get_node_for_location_command(
    conn: &mut CdpConnection,
    command: DevToolsGetNodeForLocationCommand,
) -> Result<Value, PendingDomCommandStartError> {
    let _ = (conn, command);
    Err(PendingDomCommandStartError {
        code: -32000,
        message: "GetNodeForLocationRequiresPendingRendererHitTest".to_owned(),
    })
}

pub(super) fn build_cdp_resolve_node_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsResolveNodeCommand, PendingDomCommandStartError> {
    let params: ResolveNodeParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    let reference = devtools_node_reference_from_ids(params.node_id, params.backend_node_id)
        .ok_or_else(|| PendingDomCommandStartError {
            code: -32602,
            message: "InvalidParam".to_owned(),
        })?;
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsResolveNodeCommand {
        context: cmd.automation_context(target_id.as_deref(), browser_context_id.as_deref()),
        reference,
        execution_context_id: params.execution_context_id,
        object_group: params.object_group,
    })
}

pub(super) fn start_resolve_runtime_object_for_reference(
    page: &Page,
    renderer_inspector_session_id: Option<String>,
    reference: DevToolsDomNodeReference,
    execution_context_id: Option<i64>,
    object_group: Option<&str>,
    top_frame_id: Option<String>,
) -> Result<PendingResolveRuntimeObjectForReference, PendingDomCommandStartError> {
    let backend_node_id = required_backend_node_id_for_reference(&reference)?;
    let pending = page
        .start_resolve_runtime_object_for_backend_node_id_in_inspector_session(
            renderer_inspector_session_id,
            backend_node_id,
            execution_context_id,
            object_group,
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(PendingResolveRuntimeObjectForReference {
        pending,
        cache_top_frame_id: Some(top_frame_id),
    })
}

pub(super) fn start_devtools_resolve_node_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsResolveNodeCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let reference = command.reference;
    let top_frame_id = top_frame_id_for_owner(conn, owner);
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    if let DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) = reference {
        return start_document_frontend_node_binding_command(
            conn,
            command_id,
            owner,
            frontend_node_id,
            command.context.protocol == crate::automation::FrontendProtocol::Cdp,
            PendingDomCommandKind::ResolveFrontendNodeForResolveNode {
                frontend_node_id,
                requested_execution_context_id: command.execution_context_id,
                object_group: command.object_group,
                top_frame_id,
            },
        );
    }
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    if let Some(execution_context_id) = command.execution_context_id {
        let pending = page
            .start_child_frame_id_for_default_execution_context_id(execution_context_id)
            .map_err(PendingDomCommandStartError::renderer_error)?;
        return Ok(Some(PendingDomCommandDispatch {
            command_id,
            owner_scope: owner.clone(),
            kind: PendingDomCommandKind::ResolveNodeExecutionContextFrame {
                reference,
                execution_context_id,
                object_group: command.object_group,
                top_frame_id,
            },
            pending,
        }));
    }
    let object_group = command.object_group;
    let resolution = start_resolve_runtime_object_for_reference(
        page,
        renderer_inspector_session_id,
        reference,
        None,
        object_group.as_deref(),
        top_frame_id,
    )?;

    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::ResolveNode {
            object_group,
            cache_top_frame_id: resolution.cache_top_frame_id,
        },
        pending: resolution.pending,
    }))
}

pub(super) fn build_cdp_dom_object_reference_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    operation: DevToolsDomObjectReferenceOperation,
) -> Result<Option<DevToolsDomObjectReferenceCommand>, PendingDomCommandStartError> {
    let object_id = match operation.clone() {
        DevToolsDomObjectReferenceOperation::RequestNode => {
            let params: RequestNodeParams = match cmd.get_params() {
                Ok(Some(params)) => params,
                _ => return Err(PendingDomCommandStartError::invalid_params()),
            };
            Some(params.object_id.inner().to_owned())
        }
        DevToolsDomObjectReferenceOperation::GetOuterHtml { .. } => {
            let params: GetOuterHtmlParams = match cmd.get_params() {
                Ok(Some(params)) => params,
                _ => return Err(PendingDomCommandStartError::invalid_params()),
            };
            params.reference.object_id
        }
        DevToolsDomObjectReferenceOperation::GetBoxModel
        | DevToolsDomObjectReferenceOperation::GetContentQuads => {
            let params: NodeReferenceParams = match cmd.get_params() {
                Ok(Some(params)) => params,
                _ => return Err(PendingDomCommandStartError::invalid_params()),
            };
            params.object_id
        }
        DevToolsDomObjectReferenceOperation::ScrollIntoViewIfNeeded { .. } => {
            let params: ScrollIntoViewIfNeededParams = match cmd.get_params() {
                Ok(Some(params)) => params,
                _ => return Err(PendingDomCommandStartError::invalid_params()),
            };
            params.reference.object_id
        }
        DevToolsDomObjectReferenceOperation::DescribeNode { .. } => {
            let params: DescribeNodeParams = match cmd.get_params() {
                Ok(Some(params)) => params,
                _ => return Err(PendingDomCommandStartError::invalid_params()),
            };
            params.reference.object_id
        }
    };
    let Some(object_id) = object_id else {
        return Ok(None);
    };
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(Some(DevToolsDomObjectReferenceCommand {
        context: cmd.automation_context(target_id.as_deref(), browser_context_id.as_deref()),
        object_id: DevToolsRemoteHandleId::from(object_id),
        operation,
    }))
}

pub(super) fn pending_dom_object_reference_operation_from_devtools(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
    object_id: &str,
    operation: DevToolsDomObjectReferenceOperation,
) -> PendingDomObjectReferenceOperation {
    match operation {
        DevToolsDomObjectReferenceOperation::RequestNode => {
            PendingDomObjectReferenceOperation::RequestNode
        }
        DevToolsDomObjectReferenceOperation::GetOuterHtml { include_shadow_dom } => {
            PendingDomObjectReferenceOperation::GetOuterHtml { include_shadow_dom }
        }
        DevToolsDomObjectReferenceOperation::GetBoxModel => {
            PendingDomObjectReferenceOperation::GetBoxModel
        }
        DevToolsDomObjectReferenceOperation::GetContentQuads => {
            PendingDomObjectReferenceOperation::GetContentQuads
        }
        DevToolsDomObjectReferenceOperation::ScrollIntoViewIfNeeded { rect } => {
            PendingDomObjectReferenceOperation::ScrollIntoViewIfNeeded { rect }
        }
        DevToolsDomObjectReferenceOperation::DescribeNode { depth, pierce } => {
            PendingDomObjectReferenceOperation::DescribeNode {
                depth,
                pierce,
                cached_object_node: cached_dom_remote_object_node_for_owner(conn, owner, object_id),
                top_frame_id: top_frame_id_for_owner(conn, owner),
            }
        }
    }
}

pub(super) fn start_devtools_dom_object_reference_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsDomObjectReferenceCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let operation = pending_dom_object_reference_operation_from_devtools(
        conn,
        owner,
        command.object_id.as_str(),
        command.operation,
    );
    start_dom_object_reference_operation(conn, command_id, owner, command.object_id, operation)
}

pub(super) fn start_dom_object_reference_operation(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    object_id: DevToolsRemoteHandleId,
    operation: PendingDomObjectReferenceOperation,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let reference = dom_object_reference_id_for_owner(conn, owner, &object_id);
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let include_whitespace = dom_agent_includes_whitespace_for_owner(conn, owner);
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let object_id = reference;
    match operation {
        PendingDomObjectReferenceOperation::RequestNode => {
            let pending = page
                .start_request_document_node_for_object_id(
                    renderer_inspector_session_id,
                    include_whitespace,
                    object_id,
                )
                .map_err(PendingDomCommandStartError::renderer_error)?;
            Ok(Some(PendingDomCommandDispatch {
                command_id,
                owner_scope: owner.clone(),
                kind: PendingDomCommandKind::RequestNodeObjectReference {
                    top_frame_id: top_frame_id_for_owner(conn, owner),
                },
                pending,
            }))
        }
        PendingDomObjectReferenceOperation::Focus => {
            let pending = page
                .start_focus_document_node_for_object_id(renderer_inspector_session_id, object_id)
                .map_err(PendingDomCommandStartError::renderer_error)?;
            Ok(Some(PendingDomCommandDispatch {
                command_id,
                owner_scope: owner.clone(),
                kind: PendingDomCommandKind::Focus {
                    missing_node_message: "Could not find node with given id",
                },
                pending,
            }))
        }
        PendingDomObjectReferenceOperation::GetOuterHtml { include_shadow_dom } => {
            let pending = page
                .start_outer_html_for_object_id_in_inspector_session(
                    renderer_inspector_session_id,
                    &object_id,
                    include_shadow_dom,
                )
                .map_err(PendingDomCommandStartError::renderer_error)?;
            Ok(Some(PendingDomCommandDispatch {
                command_id,
                owner_scope: owner.clone(),
                kind: PendingDomCommandKind::GetOuterHtmlObjectReference,
                pending,
            }))
        }
        PendingDomObjectReferenceOperation::DescribeNode {
            depth,
            pierce,
            cached_object_node,
            top_frame_id,
        } => {
            let pending = page
                .start_document_node_snapshot_for_object_id_in_inspector_session(
                    renderer_inspector_session_id,
                    include_whitespace,
                    &object_id,
                    depth,
                    pierce,
                    false,
                )
                .map_err(PendingDomCommandStartError::renderer_error)?;
            Ok(Some(PendingDomCommandDispatch {
                command_id,
                owner_scope: owner.clone(),
                kind: PendingDomCommandKind::DescribeNodeObjectReference {
                    cached_object_node,
                    top_frame_id,
                },
                pending,
            }))
        }
        PendingDomObjectReferenceOperation::GetBoxModel
        | PendingDomObjectReferenceOperation::GetContentQuads => {
            let pending = page
                .start_document_geometry_for_object_id_in_inspector_session(
                    renderer_inspector_session_id,
                    &object_id,
                )
                .map_err(PendingDomCommandStartError::renderer_error)?;
            Ok(Some(PendingDomCommandDispatch {
                command_id,
                owner_scope: owner.clone(),
                kind: PendingDomCommandKind::ObjectReferenceLiveClientRect { operation },
                pending,
            }))
        }
        PendingDomObjectReferenceOperation::ScrollIntoViewIfNeeded { rect } => {
            let pending = page
                .start_scroll_node_into_view_if_needed_for_object_id_in_inspector_session(
                    renderer_inspector_session_id,
                    &object_id,
                    rect,
                )
                .map_err(PendingDomCommandStartError::renderer_error)?;
            Ok(Some(PendingDomCommandDispatch {
                command_id,
                owner_scope: owner.clone(),
                kind: PendingDomCommandKind::ScrollIntoViewIfNeededObjectReference,
                pending,
            }))
        }
    }
}

pub(super) fn start_devtools_push_nodes_by_backend_ids_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsPushNodesByBackendIdsCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let renderer_runtime_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let renderer_backend_positions = (0..command.backend_node_ids.len()).collect::<Vec<_>>();
    let node_ids = vec![0; command.backend_node_ids.len()];
    let backend_node_ids = command.backend_node_ids.clone();

    let pending = page
        .start_document_frontend_node_ids_for_backend_node_ids(
            renderer_runtime_inspector_session_id,
            command.backend_node_ids,
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::PushNodesByBackendIdsToFrontend {
            backend_node_ids,
            node_ids,
            renderer_backend_positions,
        },
        pending,
    }))
}

pub(super) fn start_devtools_get_outer_html_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsGetOuterHtmlCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let include_shadow_dom = command.include_shadow_dom;
    let Some(reference) = command.reference else {
        let page = loaded_page_mut_for_owner(conn, owner)
            .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
        let pending = page
            .start_outer_html_for_document(include_shadow_dom)
            .map_err(PendingDomCommandStartError::renderer_error)?;
        return Ok(Some(PendingDomCommandDispatch {
            command_id,
            owner_scope: owner.clone(),
            kind: PendingDomCommandKind::GetOuterHtmlDocument,
            pending,
        }));
    };
    let session = conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = page
        .start_outer_html_for_node(
            reference.into_renderer_reference_for_protocol(session, command.context.protocol),
            include_shadow_dom,
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
    let kind = PendingDomCommandKind::GetOuterHtmlNodeReference;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind,
        pending,
    }))
}
