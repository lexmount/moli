use super::*;

pub(crate) async fn complete_pending_dom_command_output_plan(
    conn: &mut CdpConnection,
    completed: CompletedDomCommandDispatch,
) -> (DomCommandTaskStep, CommandOutputPlan) {
    let mut out = DomCommandOutput::default();
    if let Some(predecessor) = completed.renderer_output_predecessor() {
        // A DOM command may consist of several renderer commands (for
        // example, resolve a frontend node and then mutate it). Every segment
        // owns the concrete output it produced. Preserve its exact fence even
        // when this completion starts another pending segment; the outer
        // command context merges same-stream fences and releases the response
        // only after the final frontier has projected.
        out.set_renderer_output_predecessor(predecessor);
    }
    let step = complete_pending_dom_command(conn, completed, &mut out);
    (step, out.into_plan())
}

pub(super) fn complete_pending_dom_command_result(
    conn: &mut CdpConnection,
    completed: CompletedDomCommandDispatch,
) -> DevToolsDomCommandTaskStep {
    let owner_scope = completed.owner_scope.clone();
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(&owner_scope);
    let include_whitespace = dom_agent_includes_whitespace_for_owner(conn, &owner_scope);
    let completed_work = match completed.completed {
        Ok(completion) => completion,
        Err(error) => {
            return devtools_dom_command_task_complete(Err(DevToolsError::new(
                DevToolsErrorKind::Internal,
                error,
            )));
        }
    };
    let completion = *completed_work;

    let result = match completed.kind {
        PendingDomCommandKind::ResolveFrontendNodeForGetAttributes { frontend_node_id } => {
            return complete_frontend_node_binding_for_get_attributes_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                frontend_node_id,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForGetText { frontend_node_id } => {
            return complete_frontend_node_binding_for_get_text_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                frontend_node_id,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForGetProperty {
            frontend_node_id,
            name,
        } => {
            return complete_frontend_node_binding_for_get_property_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                name,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForDomGeometry {
            frontend_node_id,
            operation,
        } => {
            return complete_frontend_node_binding_for_dom_geometry_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                operation,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForDescribeNode {
            frontend_node_id,
            depth,
            pierce,
            top_frame_id,
        } => {
            return complete_frontend_node_binding_for_describe_node_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                depth,
                pierce,
                top_frame_id,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForRemoveNode { frontend_node_id } => {
            return complete_frontend_node_binding_for_remove_node_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                frontend_node_id,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForRequestChildNodes {
            depth,
            pierce,
            top_frame_id,
        } => {
            return complete_frontend_node_binding_for_request_child_nodes_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                depth,
                pierce,
                top_frame_id,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForQuerySelector {
            selector, multiple, ..
        } => {
            return complete_frontend_node_binding_for_query_selector_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                selector,
                multiple,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForResolveNode {
            frontend_node_id,
            requested_execution_context_id,
            object_group,
            top_frame_id,
        } => {
            return complete_frontend_node_binding_for_resolve_node_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                requested_execution_context_id,
                object_group,
                top_frame_id,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForGetOuterHtml {
            frontend_node_id,
            include_shadow_dom,
        } => {
            return complete_frontend_node_binding_for_get_outer_html_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                include_shadow_dom,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForScrollIntoViewIfNeeded {
            frontend_node_id,
            rect,
        } => {
            return complete_frontend_node_binding_for_scroll_into_view_if_needed_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                rect,
            );
        }
        PendingDomCommandKind::ResolveBidiNodeForSetFileInputFiles {
            object_id,
            files,
            append,
        } => {
            return set_file_input::complete_bidi_node_binding_for_set_file_input_files_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                object_id,
                files,
                append,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForSetFileInputFiles {
            frontend_node_id,
            file_paths,
            append,
        } => {
            return set_file_input::complete_frontend_node_binding_for_set_file_input_files_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                file_paths,
                append,
            );
        }
        PendingDomCommandKind::SetFileInputFiles => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            set_file_input::complete_set_file_input_files_result(page, completion)
                .map(|()| DevToolsCommandResult::Empty)
        }
        PendingDomCommandKind::SetFileInputFilesObjectReference => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            set_file_input::complete_set_file_input_files_object_reference_result(page, completion)
                .map(|()| DevToolsCommandResult::Empty)
        }
        PendingDomCommandKind::GetNodeForLocation { top_frame_id } => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            finish_document_hit_test(page, completion, top_frame_id)
                .map(DevToolsCommandResult::GetNodeForLocation)
        }
        PendingDomCommandKind::RendererBackendNodeClientRect { operation } => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_document_node_geometry_result(
                page.finish_document_geometry_for_backend_node_id(completion),
                operation,
                "Could not resolve node geometry",
            )
            .map(DevToolsCommandResult::DomGeometry)
        }
        PendingDomCommandKind::RendererBackendNodeScrollIntoViewIfNeeded => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_renderer_backend_node_scroll_into_view_if_needed_result(page, completion)
                .map(|()| DevToolsCommandResult::Empty)
        }
        PendingDomCommandKind::PushNodesByBackendIdsToFrontend {
            backend_node_ids,
            node_ids,
            renderer_backend_positions,
        } => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            match complete_push_nodes_by_backend_ids_to_frontend_result(
                page,
                completion,
                backend_node_ids,
                node_ids,
                renderer_backend_positions,
            ) {
                Ok(result) => Ok(DevToolsCommandResult::PushNodesByBackendIds(result)),
                Err(error) => Err(error),
            }
        }
        PendingDomCommandKind::GetFrameOwner { frame_id } => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_get_frame_owner_result(page, completion, &frame_id)
                .map(DevToolsCommandResult::GetFrameOwner)
        }
        PendingDomCommandKind::QuerySelectorLive { multiple } => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_query_selector_live_result(page, completion, multiple)
                .map(DevToolsCommandResult::QuerySelector)
        }
        PendingDomCommandKind::QuerySelectorSetChildNodesLive { multiple, .. } => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_query_selector_set_child_nodes_live_result(page, completion, multiple)
                .map(DevToolsCommandResult::QuerySelector)
        }
        PendingDomCommandKind::GetAttributesLive => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_get_attributes_live_result(page, completion)
                .map(DevToolsCommandResult::GetAttributes)
        }
        PendingDomCommandKind::GetTextLive => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_get_text_live_result(page, completion).map(DevToolsCommandResult::GetText)
        }
        PendingDomCommandKind::GetPropertyLive => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_get_property_live_result(page, completion)
                .map(DevToolsCommandResult::GetProperty)
        }
        PendingDomCommandKind::ResolveNode {
            object_group,
            cache_top_frame_id,
        } => {
            let remote_object = {
                let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                    return devtools_dom_command_task_complete(Err(DevToolsError::new(
                        DevToolsErrorKind::Internal,
                        "NoDocumentLoaded",
                    )));
                };
                match complete_resolve_node_page_result(page, completion) {
                    Ok(result) => result,
                    Err(error) => return devtools_dom_command_task_complete(Err(error)),
                }
            };
            if let Some(top_frame_id) = cache_top_frame_id
                && let Some(cache_object_id) = remote_object
                    .get("objectId")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            {
                let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                    return devtools_dom_command_task_complete(Err(DevToolsError::new(
                        DevToolsErrorKind::Internal,
                        "NoDocumentLoaded",
                    )));
                };
                return match page.start_document_node_snapshot_for_object_id_in_inspector_session(
                    renderer_inspector_session_id.clone(),
                    include_whitespace,
                    &cache_object_id,
                    0,
                    false,
                ) {
                    Ok(pending) => {
                        DevToolsDomCommandTaskStep::Pending(Box::new(PendingDomCommandDispatch {
                            command_id: None,
                            owner_scope: owner_scope.clone(),
                            kind: PendingDomCommandKind::ResolveNodeCacheSnapshot {
                                remote_object: Box::new(remote_object),
                                object_group,
                                cache_object_id,
                                top_frame_id,
                            },
                            pending,
                        }))
                    }
                    Err(error) => devtools_dom_command_task_complete(Err(DevToolsError::new(
                        DevToolsErrorKind::Internal,
                        format!("Could not snapshot resolved node: {error}"),
                    ))),
                };
            }
            Ok(DevToolsCommandResult::ResolveNode(
                register_resolve_node_result(conn, &owner_scope, remote_object, object_group),
            ))
        }
        PendingDomCommandKind::ResolveNodeCacheSnapshot {
            remote_object,
            object_group,
            cache_object_id,
            top_frame_id,
        } => {
            {
                let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                    return devtools_dom_command_task_complete(Err(DevToolsError::new(
                        DevToolsErrorKind::Internal,
                        "NoDocumentLoaded",
                    )));
                };
                match page.finish_document_node_snapshot_for_object_id(completion) {
                    Ok(Some(snapshot)) => cache_resolved_node_snapshot(
                        conn,
                        &owner_scope,
                        cache_object_id,
                        &snapshot.snapshot,
                        top_frame_id.as_deref(),
                    ),
                    Ok(None) => {}
                    Err(error) => {
                        return devtools_dom_command_task_complete(Err(DevToolsError::new(
                            DevToolsErrorKind::Internal,
                            format!("Could not snapshot resolved node: {error}"),
                        )));
                    }
                }
            }
            Ok(DevToolsCommandResult::ResolveNode(
                register_resolve_node_result(conn, &owner_scope, *remote_object, object_group),
            ))
        }
        PendingDomCommandKind::ResolveNodeExecutionContextFrame {
            reference,
            execution_context_id,
            object_group,
            top_frame_id,
        } => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            return complete_resolve_node_execution_context_frame_result(
                &owner_scope,
                renderer_inspector_session_id.clone(),
                page,
                completion,
                reference,
                execution_context_id,
                object_group,
                top_frame_id,
            );
        }
        PendingDomCommandKind::DocumentSnapshot { .. } => Err(DevToolsError::new(
            DevToolsErrorKind::Unsupported,
            "UnsupportedDevToolsCommand",
        )),
        PendingDomCommandKind::ObjectReferenceLiveClientRect { operation } => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_object_reference_live_client_rect_result(page, completion, operation)
                .map(DevToolsCommandResult::DomGeometry)
        }
        PendingDomCommandKind::GetOuterHtmlObjectReference => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_get_outer_html_object_reference_result(page, completion)
                .map(DevToolsCommandResult::GetOuterHtml)
        }
        PendingDomCommandKind::GetOuterHtmlDocument => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_get_outer_html_document_result(page, completion)
                .map(DevToolsCommandResult::GetOuterHtml)
        }
        PendingDomCommandKind::GetOuterHtmlBackendNodeReference => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_get_outer_html_backend_node_reference_result(page, completion)
                .map(DevToolsCommandResult::GetOuterHtml)
        }
        PendingDomCommandKind::ScrollIntoViewIfNeededObjectReference => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_scroll_into_view_if_needed_object_reference_result(page, completion)
                .map(|()| DevToolsCommandResult::Empty)
        }
        PendingDomCommandKind::DescribeNodeObjectReference {
            cached_object_node,
            top_frame_id,
        } => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_describe_node_object_reference_result(
                page,
                completion,
                cached_object_node,
                top_frame_id,
            )
            .map(DevToolsCommandResult::DescribeNode)
        }
        _ => Err(DevToolsError::new(
            DevToolsErrorKind::Unsupported,
            "UnsupportedDevToolsDomPendingCommand",
        )),
    };
    devtools_dom_command_task_complete(result)
}

pub(super) fn complete_scroll_into_view_result(
    result: Result<RendererScrollIntoViewResult, impl std::fmt::Display>,
) -> Result<(), DevToolsError> {
    match result {
        Ok(RendererScrollIntoViewResult::ScrolledOrAlreadyVisible) => Ok(()),
        Ok(RendererScrollIntoViewResult::NodeNotFound) => Err(devtools_dom_node_not_found_error()),
        Ok(RendererScrollIntoViewResult::NodeDetached) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "Node is detached from document",
        )),
        Ok(RendererScrollIntoViewResult::NodeDoesNotHaveLayoutObject) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "Node does not have a layout object",
        )),
        Err(error) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("Could not scroll node into view: {error}"),
        )),
    }
}

pub(super) fn complete_document_node_geometry_result(
    geometry: Result<Option<RendererDocumentNodeGeometry>, impl std::fmt::Display>,
    operation: DevToolsDomGeometryOperation,
    error_prefix: &str,
) -> Result<DevToolsDomGeometryResult, DevToolsError> {
    match geometry {
        Ok(Some(geometry)) => devtools_dom_geometry_result_from_renderer(operation, geometry),
        Ok(None) => Err(devtools_dom_node_not_found_error()),
        Err(error) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("{error_prefix}: {error}"),
        )),
    }
}

pub(super) fn finish_document_hit_test(
    page: &mut Page,
    completion: CompletedPageCommand,
    top_frame_id: String,
) -> Result<DevToolsGetNodeForLocationResult, DevToolsError> {
    match page.finish_document_hit_test(completion) {
        Ok(Some(hit)) => Ok(DevToolsGetNodeForLocationResult {
            backend_node_id: hit.node.backend_node_id,
            frame_id: DevToolsFrameId::from(hit.frame_id.unwrap_or(top_frame_id)),
            node_id: (hit.node.node_id != 0).then_some(hit.node.node_id),
        }),
        Ok(None) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "No node found at given location",
        )),
        Err(error) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("Could not hit test document: {error}"),
        )),
    }
}

pub(super) fn get_node_for_location_result_value(
    result: &DevToolsGetNodeForLocationResult,
) -> Value {
    let mut value = json!({
        "backendNodeId": result.backend_node_id,
        "frameId": result.frame_id.as_str(),
    });
    if let Some(node_id) = result.node_id {
        value["nodeId"] = json!(node_id);
    }
    value
}

pub(super) fn complete_get_frame_owner_result(
    page: &mut Page,
    completion: CompletedPageCommand,
    frame_id: &str,
) -> Result<DevToolsGetFrameOwnerResult, DevToolsError> {
    match page.finish_document_node_reference(completion) {
        Ok(Some(reference)) => Ok(get_frame_owner_result_from_node_reference(reference)),
        Ok(None) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "Frame with the given id does not belong to the target.",
        )),
        Err(error) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("Could not resolve frame owner for {frame_id}: {error}"),
        )),
    }
}

pub(super) fn get_frame_owner_result_from_node_reference(
    reference: RendererDocumentNodeReference,
) -> DevToolsGetFrameOwnerResult {
    DevToolsGetFrameOwnerResult {
        node_id: reference.node_id,
        backend_node_id: reference.backend_node_id,
    }
}

pub(super) fn get_frame_owner_result_value(result: &DevToolsGetFrameOwnerResult) -> Value {
    json!({
        "nodeId": result.node_id,
        "backendNodeId": result.backend_node_id,
    })
}

pub(super) fn live_node_not_element_error() -> PendingDomCommandStartError {
    PendingDomCommandStartError {
        code: -32000,
        message: "Node is not an Element".to_owned(),
    }
}

pub(in crate::domains::dom) fn attributes_result_from_renderer_resolution(
    resolution: RendererDocumentNodeAttributesResolution,
) -> Result<DevToolsGetAttributesResult, PendingDomCommandStartError> {
    match resolution {
        RendererDocumentNodeAttributesResolution::Found(attributes) => {
            Ok(DevToolsGetAttributesResult {
                attributes: attributes
                    .into_iter()
                    .map(|(name, value)| DevToolsDomAttribute { name, value })
                    .collect(),
            })
        }
        RendererDocumentNodeAttributesResolution::NotElement => Err(live_node_not_element_error()),
        RendererDocumentNodeAttributesResolution::MissingNode => {
            Err(PendingDomCommandStartError::node_not_found())
        }
    }
}

pub(in crate::domains::dom) fn text_result_from_renderer_resolution(
    resolution: RendererDocumentNodeTextResolution,
) -> Result<DevToolsGetTextResult, PendingDomCommandStartError> {
    match resolution {
        RendererDocumentNodeTextResolution::Found(text) => Ok(DevToolsGetTextResult { text }),
        RendererDocumentNodeTextResolution::MissingNode => {
            Err(PendingDomCommandStartError::node_not_found())
        }
    }
}

pub(in crate::domains::dom) fn property_result_from_renderer_resolution(
    resolution: RendererDocumentNodePropertyResolution,
) -> Result<DevToolsGetPropertyResult, PendingDomCommandStartError> {
    match resolution {
        RendererDocumentNodePropertyResolution::Found(value) => {
            Ok(DevToolsGetPropertyResult { value })
        }
        RendererDocumentNodePropertyResolution::NotElement => Err(live_node_not_element_error()),
        RendererDocumentNodePropertyResolution::MissingNode => {
            Err(PendingDomCommandStartError::node_not_found())
        }
    }
}

pub(in crate::domains::dom) fn query_selector_result_from_renderer_resolution(
    resolution: RendererDocumentQuerySelectorResolution,
    multiple: bool,
) -> Result<DevToolsQuerySelectorResult, PendingDomCommandStartError> {
    match resolution {
        RendererDocumentQuerySelectorResolution::Found(nodes) => Ok(DevToolsQuerySelectorResult {
            node_ids: nodes
                .into_iter()
                .map(|node| node.frontend_node_id)
                .collect(),
            multiple,
        }),
        RendererDocumentQuerySelectorResolution::MissingRoot => {
            Err(PendingDomCommandStartError::node_not_found())
        }
        RendererDocumentQuerySelectorResolution::InvalidSelector(message) => {
            Err(PendingDomCommandStartError::invalid_selector(message))
        }
    }
}

pub(super) fn complete_query_selector_live(
    page: &mut Page,
    completion: CompletedPageCommand,
    multiple: bool,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page
        .finish_document_query_selector(completion)
        .map_err(PendingDomCommandStartError::renderer_error)
        .and_then(|resolution| query_selector_result_from_renderer_resolution(resolution, multiple))
    {
        Ok(result) if result.multiple => {
            out.push_result(json!({ "nodeIds": result.node_ids }));
        }
        Ok(result) => {
            out.push_result(json!({ "nodeId": result.node_ids.first().copied().unwrap_or(0) }));
        }
        Err(error) => out.push_error(error.code, error.message),
    }
    DomCommandTaskStep::Complete
}

pub(super) fn complete_query_selector_live_result(
    page: &mut Page,
    completion: CompletedPageCommand,
    multiple: bool,
) -> Result<DevToolsQuerySelectorResult, DevToolsError> {
    page.finish_document_query_selector(completion)
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error.to_string()))
        .and_then(|resolution| {
            query_selector_result_from_renderer_resolution(resolution, multiple)
                .map_err(DevToolsError::from)
        })
}

pub(super) fn complete_query_selector_set_child_nodes_live_result(
    page: &mut Page,
    completion: CompletedPageCommand,
    multiple: bool,
) -> Result<DevToolsQuerySelectorResult, DevToolsError> {
    page.finish_document_query_selector_with_child_node_snapshot_events(completion)
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error.to_string()))
        .and_then(|result| {
            query_selector_set_child_nodes_result_from_renderer_resolution(
                result.query_selector_resolution,
                multiple,
            )
            .map_err(DevToolsError::from)
        })
}

pub(super) fn complete_frontend_node_binding_reference(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> Result<DevToolsDomNodeReference, DomCommandTaskStep> {
    let Some(page) = loaded_page_mut_for_owner(conn, owner) else {
        out.push_error(-32000, "NoDocumentLoaded");
        return Err(DomCommandTaskStep::Complete);
    };
    match super::frontend_binding::finish_reference(page, completion) {
        Ok(reference) => Ok(reference),
        Err(message) => {
            out.push_error(-32000, message);
            Err(DomCommandTaskStep::Complete)
        }
    }
}

pub(super) fn complete_frontend_node_binding_reference_result(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
) -> Result<DevToolsDomNodeReference, DevToolsDomCommandTaskStep> {
    let Some(page) = loaded_page_mut_for_owner(conn, owner) else {
        return Err(devtools_dom_command_task_complete(Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "NoDocumentLoaded",
        ))));
    };
    super::frontend_binding::finish_reference(page, completion).map_err(|message| {
        let kind = if message == "Could not find node with given id" {
            DevToolsErrorKind::NoSuchNode
        } else {
            DevToolsErrorKind::Internal
        };
        devtools_dom_command_task_complete(Err(DevToolsError::new(kind, message)))
    })
}

pub(super) fn complete_frontend_node_binding_followup<F>(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
    start: F,
) -> DomCommandTaskStep
where
    F: FnOnce(
        &Page,
        DevToolsDomNodeReference,
    )
        -> Result<(PendingPageCommand, PendingDomCommandKind), PendingDomCommandStartError>,
{
    let reference = match complete_frontend_node_binding_reference(conn, owner, completion, out) {
        Ok(reference) => reference,
        Err(step) => return step,
    };
    let Some(page) = loaded_page_mut_for_owner(conn, owner) else {
        out.push_error(-32000, "NoDocumentLoaded");
        return DomCommandTaskStep::Complete;
    };
    match start(page, reference) {
        Ok((pending, kind)) => DomCommandTaskStep::Pending(Box::new(PendingDomCommandDispatch {
            command_id,
            owner_scope: owner.clone(),
            kind,
            pending,
        })),
        Err(error) => {
            out.push_error(error.code, error.message);
            DomCommandTaskStep::Complete
        }
    }
}

pub(super) fn complete_frontend_node_binding_followup_result<F>(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    start: F,
) -> DevToolsDomCommandTaskStep
where
    F: FnOnce(
        &Page,
        DevToolsDomNodeReference,
    )
        -> Result<(PendingPageCommand, PendingDomCommandKind), PendingDomCommandStartError>,
{
    let reference = match complete_frontend_node_binding_reference_result(conn, owner, completion) {
        Ok(reference) => reference,
        Err(step) => return step,
    };
    let Some(page) = loaded_page_mut_for_owner(conn, owner) else {
        return devtools_dom_command_task_complete(Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "NoDocumentLoaded",
        )));
    };
    match start(page, reference) {
        Ok((pending, kind)) => {
            DevToolsDomCommandTaskStep::Pending(Box::new(PendingDomCommandDispatch {
                command_id,
                owner_scope: owner.clone(),
                kind,
                pending,
            }))
        }
        Err(error) => devtools_dom_command_task_complete(Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            error.message,
        ))),
    }
}

pub(super) fn complete_frontend_node_binding_for_get_attributes(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| {
            start_document_node_attributes_for_reference(page, reference)
                .map(|pending| (pending, PendingDomCommandKind::GetAttributesLive))
        },
    )
}

pub(super) fn complete_frontend_node_binding_for_get_attributes_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
) -> DevToolsDomCommandTaskStep {
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| {
            start_document_node_attributes_for_reference(page, reference)
                .map(|pending| (pending, PendingDomCommandKind::GetAttributesLive))
        },
    )
}

pub(super) fn complete_frontend_node_binding_for_remove_node(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| {
            start_remove_document_node_for_reference(page, reference)
                .map(|pending| (pending, PendingDomCommandKind::RemoveNode))
        },
    )
}

pub(super) fn complete_frontend_node_binding_for_remove_node_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
) -> DevToolsDomCommandTaskStep {
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| {
            start_remove_document_node_for_reference(page, reference)
                .map(|pending| (pending, PendingDomCommandKind::RemoveNode))
        },
    )
}

pub(super) fn complete_frontend_node_binding_for_focus(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| {
            start_focus_document_node_for_reference(page, reference).map(|pending| {
                (
                    pending,
                    PendingDomCommandKind::Focus {
                        missing_node_message: "Could not find node with given id",
                    },
                )
            })
        },
    )
}

pub(super) fn complete_frontend_node_binding_for_mutate_attribute(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    mutation: RendererDomAttributeMutation,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        move |page, reference| start_mutate_document_node_for_reference(page, reference, mutation),
    )
}

pub(super) fn complete_frontend_node_binding_for_get_text(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| {
            start_document_node_text_for_reference(page, reference)
                .map(|pending| (pending, PendingDomCommandKind::GetTextLive))
        },
    )
}

pub(super) fn complete_frontend_node_binding_for_get_text_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
) -> DevToolsDomCommandTaskStep {
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| {
            start_document_node_text_for_reference(page, reference)
                .map(|pending| (pending, PendingDomCommandKind::GetTextLive))
        },
    )
}

pub(super) fn complete_frontend_node_binding_for_get_property(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    name: String,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| {
            start_document_node_property_for_reference(page, reference, &name)
                .map(|pending| (pending, PendingDomCommandKind::GetPropertyLive))
        },
    )
}

pub(super) fn complete_frontend_node_binding_for_get_property_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    name: String,
) -> DevToolsDomCommandTaskStep {
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| {
            start_document_node_property_for_reference(page, reference, &name)
                .map(|pending| (pending, PendingDomCommandKind::GetPropertyLive))
        },
    )
}

pub(super) fn complete_frontend_node_binding_for_dom_geometry(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    operation: DevToolsDomGeometryOperation,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| start_client_rect_for_reference(page, reference, operation),
    )
}

pub(super) fn complete_frontend_node_binding_for_dom_geometry_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    operation: DevToolsDomGeometryOperation,
) -> DevToolsDomCommandTaskStep {
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| start_client_rect_for_reference(page, reference, operation),
    )
}

pub(super) fn complete_frontend_node_binding_for_describe_node(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    depth: i32,
    pierce: bool,
    top_frame_id: Option<String>,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let include_whitespace = dom_agent_includes_whitespace_for_owner(conn, owner);
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| {
            start_inspector_document_node_snapshot_for_reference(
                page,
                renderer_inspector_session_id,
                include_whitespace,
                reference,
                depth,
                pierce,
            )
            .map(|pending| {
                (
                    pending,
                    PendingDomCommandKind::DescribeNodeObjectReference {
                        cached_object_node: None,
                        top_frame_id,
                    },
                )
            })
        },
    )
}

pub(super) fn complete_frontend_node_binding_for_describe_node_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    depth: i32,
    pierce: bool,
    top_frame_id: Option<String>,
) -> DevToolsDomCommandTaskStep {
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let include_whitespace = dom_agent_includes_whitespace_for_owner(conn, owner);
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| {
            start_inspector_document_node_snapshot_for_reference(
                page,
                renderer_inspector_session_id,
                include_whitespace,
                reference,
                depth,
                pierce,
            )
            .map(|pending| {
                (
                    pending,
                    PendingDomCommandKind::DescribeNodeObjectReference {
                        cached_object_node: None,
                        top_frame_id,
                    },
                )
            })
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn complete_frontend_node_binding_for_request_child_nodes(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    depth: i32,
    pierce: bool,
    top_frame_id: Option<String>,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let include_whitespace = dom_agent_includes_whitespace_for_owner(conn, owner);
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| {
            start_request_child_nodes_for_reference(
                page,
                renderer_inspector_session_id,
                include_whitespace,
                reference,
                depth,
                pierce,
                top_frame_id,
            )
        },
    )
}

pub(super) fn complete_frontend_node_binding_for_request_child_nodes_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    depth: i32,
    pierce: bool,
    top_frame_id: Option<String>,
) -> DevToolsDomCommandTaskStep {
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let include_whitespace = dom_agent_includes_whitespace_for_owner(conn, owner);
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| {
            start_request_child_nodes_for_reference(
                page,
                renderer_inspector_session_id,
                include_whitespace,
                reference,
                depth,
                pierce,
                top_frame_id,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn complete_frontend_node_binding_for_query_selector(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    selector: String,
    multiple: bool,
    top_frame_id: Option<String>,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let include_whitespace = dom_agent_includes_whitespace_for_owner(conn, owner);
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| {
            start_query_selector_with_child_node_snapshot_events_for_reference(
                page,
                renderer_inspector_session_id,
                include_whitespace,
                reference,
                selector,
                multiple,
                top_frame_id,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn complete_frontend_node_binding_for_query_selector_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    selector: String,
    multiple: bool,
) -> DevToolsDomCommandTaskStep {
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    let include_whitespace = dom_agent_includes_whitespace_for_owner(conn, owner);
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| {
            start_query_selector_for_reference(
                page,
                renderer_inspector_session_id,
                include_whitespace,
                reference,
                selector,
                multiple,
            )
        },
    )
}

pub(super) fn start_resolve_node_for_bound_reference(
    page: &Page,
    renderer_inspector_session_id: Option<String>,
    reference: DevToolsDomNodeReference,
    requested_execution_context_id: Option<i64>,
    object_group: Option<String>,
    top_frame_id: Option<String>,
) -> Result<(PendingPageCommand, PendingDomCommandKind), PendingDomCommandStartError> {
    let resolution = start_resolve_runtime_object_for_reference(
        page,
        renderer_inspector_session_id,
        reference,
        requested_execution_context_id,
        object_group.as_deref(),
        top_frame_id,
    )?;
    Ok((
        resolution.pending,
        PendingDomCommandKind::ResolveNode {
            object_group,
            cache_top_frame_id: resolution.cache_top_frame_id,
        },
    ))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn complete_frontend_node_binding_for_resolve_node(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    requested_execution_context_id: Option<i64>,
    object_group: Option<String>,
    top_frame_id: Option<String>,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| {
            start_resolve_node_for_bound_reference(
                page,
                renderer_inspector_session_id,
                reference,
                requested_execution_context_id,
                object_group,
                top_frame_id,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn complete_frontend_node_binding_for_resolve_node_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    requested_execution_context_id: Option<i64>,
    object_group: Option<String>,
    top_frame_id: Option<String>,
) -> DevToolsDomCommandTaskStep {
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| {
            start_resolve_node_for_bound_reference(
                page,
                renderer_inspector_session_id,
                reference,
                requested_execution_context_id,
                object_group,
                top_frame_id,
            )
        },
    )
}

pub(super) fn complete_frontend_node_binding_for_get_outer_html(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    include_shadow_dom: bool,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| start_outer_html_for_reference(page, reference, include_shadow_dom),
    )
}

pub(super) fn complete_frontend_node_binding_for_get_outer_html_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    include_shadow_dom: bool,
) -> DevToolsDomCommandTaskStep {
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| start_outer_html_for_reference(page, reference, include_shadow_dom),
    )
}

pub(super) fn complete_frontend_node_binding_for_scroll_into_view_if_needed(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    rect: Option<DomScrollIntoViewRect>,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| start_scroll_into_view_for_reference(page, reference, rect),
    )
}

pub(super) fn complete_frontend_node_binding_for_scroll_into_view_if_needed_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    rect: Option<DomScrollIntoViewRect>,
) -> DevToolsDomCommandTaskStep {
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| start_scroll_into_view_for_reference(page, reference, rect),
    )
}

pub(super) fn complete_get_attributes_live(
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page
        .finish_document_node_attributes(completion)
        .map_err(PendingDomCommandStartError::renderer_error)
        .and_then(attributes_result_from_renderer_resolution)
    {
        Ok(result) => out.push_result(json!({
            "attributes": flatten_dom_attributes(result.attributes),
        })),
        Err(error) => out.push_error(error.code, error.message),
    }
    DomCommandTaskStep::Complete
}

pub(super) fn complete_get_text_live(
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page
        .finish_document_node_text(completion)
        .map_err(PendingDomCommandStartError::renderer_error)
        .and_then(text_result_from_renderer_resolution)
    {
        Ok(result) => out.push_result(json!({ "text": result.text })),
        Err(error) => out.push_error(error.code, error.message),
    }
    DomCommandTaskStep::Complete
}

pub(super) fn complete_get_property_live(
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page
        .finish_document_node_property(completion)
        .map_err(PendingDomCommandStartError::renderer_error)
        .and_then(property_result_from_renderer_resolution)
    {
        Ok(result) => out.push_result(json!({ "value": result.value })),
        Err(error) => out.push_error(error.code, error.message),
    }
    DomCommandTaskStep::Complete
}

pub(super) fn complete_get_attributes_live_result(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<DevToolsGetAttributesResult, DevToolsError> {
    page.finish_document_node_attributes(completion)
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error.to_string()))
        .and_then(|resolution| {
            attributes_result_from_renderer_resolution(resolution).map_err(DevToolsError::from)
        })
}

pub(super) fn complete_get_text_live_result(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<DevToolsGetTextResult, DevToolsError> {
    page.finish_document_node_text(completion)
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error.to_string()))
        .and_then(|resolution| {
            text_result_from_renderer_resolution(resolution).map_err(DevToolsError::from)
        })
}

pub(super) fn complete_get_property_live_result(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<DevToolsGetPropertyResult, DevToolsError> {
    page.finish_document_node_property(completion)
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error.to_string()))
        .and_then(|resolution| {
            property_result_from_renderer_resolution(resolution).map_err(DevToolsError::from)
        })
}

pub(super) fn complete_resolve_node_page_result(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<Value, DevToolsError> {
    let resolution = page.finish_resolve_runtime_object_for_backend_node_id(completion);
    let remote_object = match resolution {
        Ok(DocumentNodeRuntimeObjectResolution::Found(remote_object)) => remote_object,
        Ok(DocumentNodeRuntimeObjectResolution::MissingContext) => {
            return Err(DevToolsError::new(
                DevToolsErrorKind::Internal,
                "ContextNotFound",
            ));
        }
        Ok(DocumentNodeRuntimeObjectResolution::MissingNode) => {
            return Err(devtools_dom_node_not_found_error());
        }
        Err(error) => {
            return Err(DevToolsError::new(
                DevToolsErrorKind::Internal,
                format!("Could not resolve node runtime object: {error}"),
            ));
        }
    };
    Ok(remote_object.into_protocol_value())
}

pub(super) fn complete_resolve_node_execution_context_frame_result(
    owner: &CommandOwnerScope,
    renderer_inspector_session_id: Option<String>,
    page: &mut Page,
    completion: CompletedPageCommand,
    reference: DevToolsDomNodeReference,
    execution_context_id: i64,
    object_group: Option<String>,
    top_frame_id: Option<String>,
) -> DevToolsDomCommandTaskStep {
    let child_frame_id =
        match page.finish_child_frame_id_for_default_execution_context_id(completion) {
            Ok(frame_id) => frame_id,
            Err(error) => {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    format!("Could not resolve execution context frame: {error}"),
                )));
            }
        };
    if child_frame_id.is_some() {
        if let DevToolsDomNodeReference::BackendNodeId(backend_node_id) = reference
            && is_renderer_backend_node_id(backend_node_id)
        {
            return match page.start_resolve_runtime_object_for_backend_node_id_in_inspector_session(
                renderer_inspector_session_id,
                backend_node_id,
                Some(execution_context_id),
                object_group.as_deref(),
            ) {
                Ok(pending) => {
                    DevToolsDomCommandTaskStep::Pending(Box::new(PendingDomCommandDispatch {
                        command_id: None,
                        owner_scope: owner.clone(),
                        kind: PendingDomCommandKind::ResolveNode {
                            object_group,
                            cache_top_frame_id: None,
                        },
                        pending,
                    }))
                }
                Err(error) => devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    format!("Could not resolve node runtime object: {error}"),
                ))),
            };
        }
        return devtools_dom_command_task_complete(Err(devtools_dom_node_not_found_error()));
    }
    match start_resolve_runtime_object_for_reference(
        page,
        renderer_inspector_session_id,
        reference,
        Some(execution_context_id),
        object_group.as_deref(),
        top_frame_id,
    ) {
        Ok(resolution) => {
            DevToolsDomCommandTaskStep::Pending(Box::new(PendingDomCommandDispatch {
                command_id: None,
                owner_scope: owner.clone(),
                kind: PendingDomCommandKind::ResolveNode {
                    object_group,
                    cache_top_frame_id: resolution.cache_top_frame_id,
                },
                pending: resolution.pending,
            }))
        }
        Err(error) => devtools_dom_command_task_complete(Err(DevToolsError::from(error))),
    }
}

pub(super) fn complete_object_reference_live_client_rect_result(
    page: &mut Page,
    completion: CompletedPageCommand,
    operation: PendingDomObjectReferenceOperation,
) -> Result<DevToolsDomGeometryResult, DevToolsError> {
    let operation = match operation {
        PendingDomObjectReferenceOperation::GetBoxModel => {
            DevToolsDomGeometryOperation::GetBoxModel
        }
        PendingDomObjectReferenceOperation::GetContentQuads => {
            DevToolsDomGeometryOperation::GetContentQuads
        }
        _ => {
            return Err(DevToolsError::new(
                DevToolsErrorKind::Unsupported,
                "UnsupportedDevToolsDomObjectReferenceCommand",
            ));
        }
    };
    match page.finish_document_geometry_for_object_id(completion) {
        Ok(Some(geometry)) => devtools_dom_geometry_result_from_renderer(operation, geometry),
        Ok(None) => Err(devtools_dom_node_not_found_error()),
        Err(error) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("Could not resolve node geometry: {error}"),
        )),
    }
}

pub(super) fn cache_resolved_node_snapshot(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    cache_object_id: String,
    snapshot: &DocumentNodeSnapshot,
    top_frame_id: Option<&str>,
) {
    let top_snapshot_node_id = top_snapshot_node_id_for_live_object_snapshot(snapshot);
    if let Some(cached_node) = node_snapshot_to_cdp(snapshot, top_snapshot_node_id, top_frame_id) {
        cache_dom_remote_object_node_for_owner(conn, owner, cache_object_id, cached_node);
    }
}

pub(super) fn register_resolve_node_result(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    mut remote_object: Value,
    object_group: Option<String>,
) -> DevToolsResolveNodeResult {
    if let Some(remote_object) = remote_object.as_object_mut() {
        remote_object
            .entry("subtype".to_owned())
            .or_insert_with(|| json!("node"));
    }
    let result = json!({ "object": remote_object.clone() });
    if let Some(object_group) = object_group.as_deref() {
        conn.register_runtime_remote_object_ids_from_value_for_owner_with_group(
            owner,
            &result,
            object_group,
        );
    } else {
        conn.register_runtime_remote_object_ids_from_value_for_owner(owner, &result);
    }
    DevToolsResolveNodeResult {
        object: remote_object,
    }
}

pub(super) fn complete_set_child_nodes_snapshot_for_backend_node(
    session_id: Option<&str>,
    page: &mut Page,
    completion: CompletedPageCommand,
    after: PendingSetChildNodesAfter,
    top_frame_id: Option<String>,
    missing_node_message: &'static str,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let snapshot_events = match page.finish_document_child_node_snapshot_events(completion) {
        Ok(Some(events)) => events,
        Ok(None) => {
            out.push_error(-32000, missing_node_message);
            return DomCommandTaskStep::Complete;
        }
        Err(error) => {
            out.push_error(
                -32000,
                format!("Could not capture child node snapshots: {error}"),
            );
            return DomCommandTaskStep::Complete;
        }
    };
    let Some(event) = snapshot_events.events.into_iter().next() else {
        complete_set_child_nodes_after(after, out);
        return DomCommandTaskStep::Complete;
    };
    complete_set_child_nodes_from_snapshots(
        session_id,
        after,
        event.parent_frontend_node_id,
        event.snapshots,
        snapshot_events.top_snapshot_node_id,
        top_frame_id,
        out,
    )
}

pub(super) fn complete_query_selector_set_child_nodes_live(
    session_id: Option<&str>,
    page: &mut Page,
    completion: CompletedPageCommand,
    multiple: bool,
    top_frame_id: Option<String>,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let result =
        match page.finish_document_query_selector_with_child_node_snapshot_events(completion) {
            Ok(result) => result,
            Err(error) => {
                out.push_error(
                    -32000,
                    format!("Could not capture query selector child node snapshots: {error}"),
                );
                return DomCommandTaskStep::Complete;
            }
        };
    let after = PendingSetChildNodesAfter::QuerySelectorLive {
        resolution: result.query_selector_resolution,
        multiple,
    };
    let Some(snapshot_events) = result.child_node_snapshot_events else {
        complete_set_child_nodes_after(after, out);
        return DomCommandTaskStep::Complete;
    };
    for event in snapshot_events.events {
        push_set_child_nodes_event_from_snapshots(
            session_id,
            event.parent_frontend_node_id,
            &event.snapshots,
            snapshot_events.top_snapshot_node_id,
            top_frame_id.as_deref(),
            out,
        );
    }
    complete_set_child_nodes_after(after, out);
    DomCommandTaskStep::Complete
}

pub(super) fn complete_set_child_nodes_from_snapshots(
    session_id: Option<&str>,
    after: PendingSetChildNodesAfter,
    parent_frontend_node_id: u32,
    snapshots: Vec<DocumentNodeSnapshot>,
    top_snapshot_node_id: DocumentSnapshotNodeId,
    top_frame_id: Option<String>,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    push_set_child_nodes_event_from_snapshots(
        session_id,
        parent_frontend_node_id,
        &snapshots,
        top_snapshot_node_id,
        top_frame_id.as_deref(),
        out,
    );
    complete_set_child_nodes_after(after, out);
    DomCommandTaskStep::Complete
}

pub(super) fn complete_set_child_nodes_after(
    after: PendingSetChildNodesAfter,
    out: &mut DomCommandOutput,
) {
    match after {
        PendingSetChildNodesAfter::EmptyResult => {
            out.push_success();
        }
        PendingSetChildNodesAfter::QuerySelectorLive {
            resolution,
            multiple,
        } => match query_selector_set_child_nodes_result_from_renderer_resolution(
            resolution, multiple,
        ) {
            Ok(result) if result.multiple => {
                out.push_result(json!({ "nodeIds": result.node_ids }));
            }
            Ok(result) => {
                out.push_result(json!({ "nodeId": result.node_ids[0] }));
            }
            Err(error) => out.push_error(error.code, error.message),
        },
    }
}

pub(super) fn query_selector_set_child_nodes_result_from_renderer_resolution(
    resolution: RendererDocumentQuerySelectorResolution,
    multiple: bool,
) -> Result<DevToolsQuerySelectorResult, PendingDomCommandStartError> {
    match resolution {
        RendererDocumentQuerySelectorResolution::Found(nodes) => {
            if !multiple && nodes.is_empty() {
                return Ok(DevToolsQuerySelectorResult {
                    node_ids: vec![0],
                    multiple,
                });
            }
            Ok(DevToolsQuerySelectorResult {
                node_ids: nodes
                    .into_iter()
                    .map(|node| node.frontend_node_id)
                    .collect(),
                multiple,
            })
        }
        RendererDocumentQuerySelectorResolution::MissingRoot => {
            Err(PendingDomCommandStartError::node_not_found())
        }
        RendererDocumentQuerySelectorResolution::InvalidSelector(message) => {
            Err(PendingDomCommandStartError::invalid_selector(message))
        }
    }
}

pub(in crate::domains::dom) fn push_set_child_nodes_event_from_snapshots(
    session_id: Option<&str>,
    parent_frontend_node_id: u32,
    snapshots: &[DocumentNodeSnapshot],
    top_snapshot_node_id: DocumentSnapshotNodeId,
    top_frame_id: Option<&str>,
    out: &mut DomCommandOutput,
) {
    let nodes = snapshots
        .iter()
        .filter_map(|snapshot| {
            node_snapshot_to_cdp(snapshot, Some(top_snapshot_node_id), top_frame_id)
        })
        .collect::<Vec<_>>();
    push_set_child_nodes_event(out, session_id, parent_frontend_node_id, nodes);
}

pub(super) fn push_resolve_node_result(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    out: &mut DomCommandOutput,
    mut remote_object: Value,
    object_group: Option<String>,
) {
    if let Some(remote_object) = remote_object.as_object_mut() {
        remote_object
            .entry("subtype".to_owned())
            .or_insert_with(|| json!("node"));
    }
    let result = json!({ "object": remote_object });
    if let Some(object_group) = object_group.as_deref() {
        conn.register_runtime_remote_object_ids_from_value_for_owner_with_group(
            owner,
            &result,
            object_group,
        );
    } else {
        conn.register_runtime_remote_object_ids_from_value_for_owner(owner, &result);
    }
    out.push_result(result);
}

pub(super) fn top_snapshot_node_id_for_live_object_snapshot(
    snapshot: &DocumentNodeSnapshot,
) -> Option<DocumentSnapshotNodeId> {
    if snapshot.parent_id.is_none()
        && (snapshot.node_name == "#document"
            || (snapshot.is_element && snapshot.local_name == "html"))
    {
        Some(snapshot.node_id)
    } else {
        None
    }
}

pub(super) fn complete_request_node_object_reference(
    _session_id: Option<&str>,
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page.finish_document_node_snapshot_for_object_id(completion) {
        Ok(Some(object_snapshot)) => {
            let Some(frontend_node_id) =
                super::frontend_node_id_for_snapshot(&object_snapshot.snapshot)
            else {
                out.push_error(-32000, "Could not find node with given id");
                return DomCommandTaskStep::Complete;
            };
            out.push_result(json!({
                "nodeId": frontend_node_id
            }));
        }
        Ok(None) => out.push_error(-32000, "Could not find node with given id"),
        Err(error) => {
            out.push_error(-32000, format!("Could not request node object: {error}"));
        }
    }
    DomCommandTaskStep::Complete
}

pub(super) fn complete_get_outer_html_object_reference(
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page.finish_outer_html_for_object_id(completion) {
        Ok(Some(outer_html)) => out.push_result(json!({ "outerHTML": outer_html })),
        Ok(None) => out.push_error(-32000, "Could not find node with given id"),
        Err(error) => {
            out.push_error(
                -32000,
                format!("Could not get outerHTML for node object: {error}"),
            );
        }
    }
    DomCommandTaskStep::Complete
}

pub(super) fn complete_get_outer_html_object_reference_result(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<DevToolsGetOuterHtmlResult, DevToolsError> {
    match page.finish_outer_html_for_object_id(completion) {
        Ok(Some(outer_html)) => Ok(DevToolsGetOuterHtmlResult { outer_html }),
        Ok(None) => Err(devtools_dom_node_not_found_error()),
        Err(error) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("Could not get outerHTML for node object: {error}"),
        )),
    }
}

pub(super) fn complete_get_outer_html_document(
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page.finish_outer_html_for_document(completion) {
        Ok(outer_html) => out.push_result(json!({ "outerHTML": outer_html })),
        Err(error) => out.push_error(
            -32000,
            format!("Could not serialize document outerHTML: {error}"),
        ),
    }
    DomCommandTaskStep::Complete
}

pub(super) fn complete_get_outer_html_document_result(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<DevToolsGetOuterHtmlResult, DevToolsError> {
    match page.finish_outer_html_for_document(completion) {
        Ok(outer_html) => Ok(DevToolsGetOuterHtmlResult { outer_html }),
        Err(error) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("Could not serialize document outerHTML: {error}"),
        )),
    }
}

pub(super) fn complete_get_outer_html_backend_node_reference(
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page.finish_outer_html_for_backend_node_id(completion) {
        Ok(Some(outer_html)) => out.push_result(json!({ "outerHTML": outer_html })),
        Ok(None) => out.push_error(-32000, "Could not find node with given id"),
        Err(error) => out.push_error(-32000, format!("Could not get outerHTML for node: {error}")),
    }
    DomCommandTaskStep::Complete
}

pub(super) fn complete_get_outer_html_backend_node_reference_result(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<DevToolsGetOuterHtmlResult, DevToolsError> {
    match page.finish_outer_html_for_backend_node_id(completion) {
        Ok(Some(outer_html)) => Ok(DevToolsGetOuterHtmlResult { outer_html }),
        Ok(None) => Err(devtools_dom_node_not_found_error()),
        Err(error) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("Could not get outerHTML for node: {error}"),
        )),
    }
}

pub(super) fn complete_scroll_into_view_if_needed_object_reference(
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page.finish_scroll_node_into_view_if_needed(completion) {
        Ok(RendererScrollIntoViewResult::ScrolledOrAlreadyVisible) => out.push_success(),
        Ok(RendererScrollIntoViewResult::NodeNotFound) => {
            out.push_error(-32000, "Could not find node with given id")
        }
        Ok(RendererScrollIntoViewResult::NodeDetached) => {
            out.push_error(-32000, "Node is detached from document")
        }
        Ok(RendererScrollIntoViewResult::NodeDoesNotHaveLayoutObject) => {
            out.push_error(-32000, "Node does not have a layout object")
        }
        Err(error) => out.push_error(-32000, format!("Could not scroll node into view: {error}")),
    }
    DomCommandTaskStep::Complete
}

pub(super) fn complete_scroll_into_view_if_needed_object_reference_result(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<(), DevToolsError> {
    complete_scroll_into_view_result(page.finish_scroll_node_into_view_if_needed(completion))
}

pub(super) fn complete_renderer_backend_node_scroll_into_view_if_needed(
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page.finish_scroll_node_into_view_if_needed(completion) {
        Ok(RendererScrollIntoViewResult::ScrolledOrAlreadyVisible) => out.push_success(),
        Ok(RendererScrollIntoViewResult::NodeNotFound) => {
            out.push_error(-32000, "Could not find node with given id")
        }
        Ok(RendererScrollIntoViewResult::NodeDetached) => {
            out.push_error(-32000, "Node is detached from document")
        }
        Ok(RendererScrollIntoViewResult::NodeDoesNotHaveLayoutObject) => {
            out.push_error(-32000, "Node does not have a layout object")
        }
        Err(error) => out.push_error(-32000, format!("Could not scroll node into view: {error}")),
    }
    DomCommandTaskStep::Complete
}

pub(super) fn complete_renderer_backend_node_scroll_into_view_if_needed_result(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<(), DevToolsError> {
    complete_scroll_into_view_result(page.finish_scroll_node_into_view_if_needed(completion))
}

pub(super) fn fill_pushed_frontend_node_ids(
    node_ids: &mut [u32],
    renderer_backend_positions: Vec<usize>,
    renderer_frontend_node_ids: Vec<Option<u32>>,
) {
    for (position, frontend_node_id) in renderer_backend_positions
        .into_iter()
        .zip(renderer_frontend_node_ids)
    {
        if let Some(slot) = node_ids.get_mut(position) {
            *slot = frontend_node_id.unwrap_or(0);
        }
    }
}

pub(super) fn complete_push_nodes_by_backend_ids_to_frontend(
    page: &mut Page,
    completion: CompletedPageCommand,
    _session_id: Option<&str>,
    _backend_node_ids: Vec<u32>,
    mut node_ids: Vec<u32>,
    renderer_backend_positions: Vec<usize>,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page.finish_document_frontend_node_ids_for_backend_node_ids(completion) {
        Ok(RendererDocumentFrontendNodeIdsResolution::Found(renderer_frontend_node_ids)) => {
            fill_pushed_frontend_node_ids(
                &mut node_ids,
                renderer_backend_positions,
                renderer_frontend_node_ids,
            );
            out.push_result(json!({ "nodeIds": node_ids }));
        }
        Ok(RendererDocumentFrontendNodeIdsResolution::DocumentNotBound) => {
            out.push_error(-32000, "Document needs to be requested first")
        }
        Err(error) => out.push_error(
            -32000,
            format!("Could not resolve backend node ids: {error}"),
        ),
    }
    DomCommandTaskStep::Complete
}

pub(super) fn complete_push_nodes_by_backend_ids_to_frontend_result(
    page: &mut Page,
    completion: CompletedPageCommand,
    _backend_node_ids: Vec<u32>,
    mut node_ids: Vec<u32>,
    renderer_backend_positions: Vec<usize>,
) -> Result<DevToolsPushNodesByBackendIdsResult, DevToolsError> {
    match page.finish_document_frontend_node_ids_for_backend_node_ids(completion) {
        Ok(RendererDocumentFrontendNodeIdsResolution::Found(renderer_frontend_node_ids)) => {
            fill_pushed_frontend_node_ids(
                &mut node_ids,
                renderer_backend_positions,
                renderer_frontend_node_ids,
            );
            Ok(DevToolsPushNodesByBackendIdsResult { node_ids })
        }
        Ok(RendererDocumentFrontendNodeIdsResolution::DocumentNotBound) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "Document needs to be requested first",
        )),
        Err(error) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("Could not resolve backend node ids: {error}"),
        )),
    }
}

pub(super) fn node_for_describe_node_object_snapshot(
    snapshot: &DocumentNodeSnapshot,
    child_frame_id: Option<&str>,
    top_frame_id: Option<&str>,
) -> Option<Value> {
    let top_snapshot_node_id = if child_frame_id.is_some() {
        snapshot.parent_id.or(Some(snapshot.node_id))
    } else {
        top_snapshot_node_id_for_live_object_snapshot(snapshot)
    };
    let frame_id_for_html = if child_frame_id.is_some() {
        None
    } else {
        top_frame_id
    };
    let mut node = node_snapshot_to_cdp(snapshot, top_snapshot_node_id, frame_id_for_html)?;
    if let Some(child_frame_id) = child_frame_id
        && let Some(node) = node.as_object_mut()
    {
        node.insert("frameId".to_owned(), json!(child_frame_id));
    }
    Some(node)
}

pub(super) fn complete_describe_node_object_reference(
    page: &mut Page,
    completion: CompletedPageCommand,
    cached_object_node: Option<Value>,
    top_frame_id: Option<String>,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let object_snapshot = match page.finish_document_node_snapshot_for_object_id(completion) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            if let Some(node) = cached_object_node {
                out.push_result(json!({ "node": node }));
            } else {
                out.push_error(-32000, "Could not find node with given id");
            }
            return DomCommandTaskStep::Complete;
        }
        Err(error) => {
            out.push_error(-32000, format!("Could not describe node object: {error}"));
            return DomCommandTaskStep::Complete;
        }
    };

    let Some(node) = node_for_describe_node_object_snapshot(
        &object_snapshot.snapshot,
        object_snapshot.frame_id.as_deref(),
        top_frame_id.as_deref(),
    ) else {
        out.push_error(-32000, "Could not find node with given id");
        return DomCommandTaskStep::Complete;
    };
    out.push_result(json!({ "node": node }));
    DomCommandTaskStep::Complete
}

pub(super) fn complete_describe_node_object_reference_result(
    page: &mut Page,
    completion: CompletedPageCommand,
    cached_object_node: Option<Value>,
    top_frame_id: Option<String>,
) -> Result<DevToolsDescribeNodeResult, DevToolsError> {
    let object_snapshot = match page.finish_document_node_snapshot_for_object_id(completion) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            if let Some(node) = cached_object_node {
                return Ok(DevToolsDescribeNodeResult { node });
            }
            return Err(devtools_dom_node_not_found_error());
        }
        Err(error) => {
            return Err(DevToolsError::new(
                DevToolsErrorKind::Internal,
                format!("Could not describe node object: {error}"),
            ));
        }
    };
    let Some(node) = node_for_describe_node_object_snapshot(
        &object_snapshot.snapshot,
        object_snapshot.frame_id.as_deref(),
        top_frame_id.as_deref(),
    ) else {
        return Err(devtools_dom_node_not_found_error());
    };
    Ok(DevToolsDescribeNodeResult { node })
}

pub(super) fn complete_document_snapshot(
    operation: PendingDomDocumentSnapshotOperation,
    page: &mut Page,
    completion: CompletedPageCommand,
    top_frame_id: Option<String>,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let snapshot = match page.finish_document_node_snapshot_for_document(completion) {
        Ok(Some(object_snapshot)) => object_snapshot.snapshot,
        Ok(None) => {
            out.push_error(-32000, "NoDocumentLoaded");
            return DomCommandTaskStep::Complete;
        }
        Err(error) => {
            out.push_error(
                -32000,
                format!("Could not capture document snapshot: {error}"),
            );
            return DomCommandTaskStep::Complete;
        }
    };
    let top_snapshot_node_id = snapshot.node_id;
    match operation {
        PendingDomDocumentSnapshotOperation::GetDocument => {
            let Some(root) = node_snapshot_to_cdp(
                &snapshot,
                Some(top_snapshot_node_id),
                top_frame_id.as_deref(),
            ) else {
                out.push_error(-32000, "No node with given id found");
                return DomCommandTaskStep::Complete;
            };
            out.push_result(json!({ "root": root }));
        }
        PendingDomDocumentSnapshotOperation::GetFlattenedDocument => {
            let mut nodes = Vec::new();
            collect_flattened_node_snapshot(
                &snapshot,
                top_snapshot_node_id,
                top_frame_id.as_deref(),
                &mut nodes,
            );
            out.push_result(json!({ "nodes": nodes }));
        }
    }
    DomCommandTaskStep::Complete
}

pub(super) fn complete_object_reference_live_client_rect(
    operation: PendingDomObjectReferenceOperation,
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page.finish_document_geometry_for_object_id(completion) {
        Ok(Some(geometry)) => {
            let geometry_operation = match dom_geometry_operation_for_object_reference(&operation) {
                Ok(operation) => operation,
                Err(message) => {
                    out.push_error(-32000, message);
                    return DomCommandTaskStep::Complete;
                }
            };
            match devtools_dom_geometry_result_from_renderer(geometry_operation, geometry) {
                Ok(result) => push_devtools_dom_geometry_result(&result, out),
                Err(error) => out.push_error(-32000, error.message),
            }
        }
        Ok(None) => out.push_error(-32000, "Could not find node with given id"),
        Err(error) => out.push_error(-32000, format!("Could not resolve node geometry: {error}")),
    }
    DomCommandTaskStep::Complete
}

pub(super) fn complete_renderer_backend_node_client_rect(
    operation: DevToolsDomGeometryOperation,
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page.finish_document_geometry_for_backend_node_id(completion) {
        Ok(Some(geometry)) => {
            match devtools_dom_geometry_result_from_renderer(operation, geometry) {
                Ok(result) => push_devtools_dom_geometry_result(&result, out),
                Err(error) => out.push_error(-32000, error.message),
            }
        }
        Ok(None) => out.push_error(-32000, "Could not find node with given id"),
        Err(error) => out.push_error(-32000, format!("Could not resolve node geometry: {error}")),
    }
    DomCommandTaskStep::Complete
}

pub(super) fn dom_geometry_operation_for_object_reference(
    operation: &PendingDomObjectReferenceOperation,
) -> Result<DevToolsDomGeometryOperation, &'static str> {
    match operation {
        PendingDomObjectReferenceOperation::GetBoxModel => {
            Ok(DevToolsDomGeometryOperation::GetBoxModel)
        }
        PendingDomObjectReferenceOperation::GetContentQuads => {
            Ok(DevToolsDomGeometryOperation::GetContentQuads)
        }
        PendingDomObjectReferenceOperation::RequestNode
        | PendingDomObjectReferenceOperation::Focus
        | PendingDomObjectReferenceOperation::GetOuterHtml { .. }
        | PendingDomObjectReferenceOperation::ScrollIntoViewIfNeeded { .. }
        | PendingDomObjectReferenceOperation::DescribeNode { .. } => {
            Err("UnsupportedDevToolsDomObjectReferenceCommand")
        }
    }
}

pub(in crate::domains::dom) fn devtools_dom_geometry_result_from_renderer(
    operation: DevToolsDomGeometryOperation,
    geometry: RendererDocumentNodeGeometry,
) -> Result<DevToolsDomGeometryResult, DevToolsError> {
    match (operation, geometry) {
        (
            DevToolsDomGeometryOperation::GetBoxModel,
            RendererDocumentNodeGeometry::FoundElement { box_model, .. },
        ) => Ok(DevToolsDomGeometryResult {
            box_model: Some(DevToolsDomBoxModel {
                content: devtools_dom_quad(box_model.content.points),
                padding: devtools_dom_quad(box_model.padding.points),
                border: devtools_dom_quad(box_model.border.points),
                margin: devtools_dom_quad(box_model.margin.points),
                width: box_model.width,
                height: box_model.height,
            }),
            quads: Vec::new(),
            width: Some(box_model.width),
            height: Some(box_model.height),
        }),
        (
            DevToolsDomGeometryOperation::GetContentQuads,
            RendererDocumentNodeGeometry::FoundElement { content_quads, .. }
            | RendererDocumentNodeGeometry::FoundNonElement { content_quads },
        ) => Ok(DevToolsDomGeometryResult {
            box_model: None,
            quads: content_quads
                .into_iter()
                .map(|quad| devtools_dom_quad(quad.points))
                .collect(),
            width: None,
            height: None,
        }),
        (_, RendererDocumentNodeGeometry::NoLayoutObject) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "Could not compute node geometry.",
        )),
        (_, RendererDocumentNodeGeometry::FoundNonElement { .. })
        | (_, RendererDocumentNodeGeometry::NotElement) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "Node is not an element",
        )),
    }
}

pub(super) fn devtools_dom_quad(points: [f64; 8]) -> DevToolsDomQuad {
    DevToolsDomQuad {
        points: points.into(),
    }
}

pub(super) fn push_devtools_dom_geometry_result(
    result: &DevToolsDomGeometryResult,
    out: &mut DomCommandOutput,
) {
    out.push_result(devtools_dom_geometry_result_value(result));
}

pub(super) fn devtools_dom_geometry_result_value(result: &DevToolsDomGeometryResult) -> Value {
    if let Some(model) = result.box_model.as_ref() {
        json!({
            "model": {
                "content": model.content.points,
                "padding": model.padding.points,
                "border": model.border.points,
                "margin": model.margin.points,
                "width": model.width,
                "height": model.height,
            }
        })
    } else {
        json!({
            "quads": result.quads.iter().map(|quad| &quad.points).collect::<Vec<_>>()
        })
    }
}
