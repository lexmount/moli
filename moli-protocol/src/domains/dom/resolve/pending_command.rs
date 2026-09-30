use super::*;

pub(super) fn complete_pending_dom_command(
    conn: &mut CdpConnection,
    completed: CompletedDomCommandDispatch,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let command_id = completed.command_id;
    let owner_scope = completed.owner_scope.clone();
    let session_id = owner_scope.session_id();
    let completed_work = match completed.completed {
        Ok(completion) => completion,
        Err(error) => {
            out.push_error(-32000, error);
            return DomCommandTaskStep::Complete;
        }
    };
    let completion = *completed_work;
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(&owner_scope);
    let include_whitespace = dom_agent_includes_whitespace_for_owner(conn, &owner_scope);

    let kind = match completed.kind {
        PendingDomCommandKind::PerformSearchLive => {
            return search::complete_perform_search_live(conn, &owner_scope, completion, out);
        }
        PendingDomCommandKind::GetSearchResultsLive => {
            return search::complete_get_search_results_live(conn, &owner_scope, completion, out);
        }
        PendingDomCommandKind::DiscardSearchResultsLive => {
            return search::complete_discard_search_results_live(
                conn,
                &owner_scope,
                completion,
                out,
            );
        }
        PendingDomCommandKind::SetNodeStackTracesEnabled => {
            return stack_traces::complete_set_node_stack_traces_enabled_command(
                conn,
                &owner_scope,
                completion,
                out,
            );
        }
        PendingDomCommandKind::GetNodeStackTraces => {
            return stack_traces::complete_get_node_stack_traces_command(
                conn,
                &owner_scope,
                completion,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForGetAttributes { frontend_node_id } => {
            return complete_frontend_node_binding_for_get_attributes(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForGetText { frontend_node_id } => {
            return complete_frontend_node_binding_for_get_text(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForGetProperty {
            frontend_node_id,
            name,
        } => {
            return complete_frontend_node_binding_for_get_property(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                name,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForDomGeometry {
            frontend_node_id,
            operation,
        } => {
            return complete_frontend_node_binding_for_dom_geometry(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                operation,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForDescribeNode {
            frontend_node_id,
            depth,
            pierce,
            top_frame_id,
        } => {
            return complete_frontend_node_binding_for_describe_node(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                depth,
                pierce,
                top_frame_id,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForRequestChildNodes {
            depth,
            pierce,
            top_frame_id,
        } => {
            return complete_frontend_node_binding_for_request_child_nodes(
                conn,
                command_id,
                &owner_scope,
                completion,
                depth,
                pierce,
                top_frame_id,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForRemoveNode { frontend_node_id } => {
            return complete_frontend_node_binding_for_remove_node(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForFocus { frontend_node_id } => {
            return complete_frontend_node_binding_for_focus(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForMutateAttribute { mutation } => {
            return complete_frontend_node_binding_for_mutate_attribute(
                conn,
                command_id,
                &owner_scope,
                completion,
                mutation,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForQuerySelector {
            selector,
            multiple,
            top_frame_id,
        } => {
            return complete_frontend_node_binding_for_query_selector(
                conn,
                command_id,
                &owner_scope,
                completion,
                selector,
                multiple,
                top_frame_id,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForResolveNode {
            frontend_node_id,
            requested_execution_context_id,
            object_group,
            top_frame_id,
        } => {
            return complete_frontend_node_binding_for_resolve_node(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                requested_execution_context_id,
                object_group,
                top_frame_id,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForGetOuterHtml {
            frontend_node_id,
            include_shadow_dom,
        } => {
            return complete_frontend_node_binding_for_get_outer_html(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                include_shadow_dom,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForScrollIntoViewIfNeeded {
            frontend_node_id,
            rect,
        } => {
            return complete_frontend_node_binding_for_scroll_into_view_if_needed(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                rect,
                out,
            );
        }
        PendingDomCommandKind::ResolveBidiNodeForSetFileInputFiles {
            object_id,
            files,
            append,
        } => {
            return set_file_input::complete_bidi_node_binding_for_set_file_input_files(
                conn,
                command_id,
                &owner_scope,
                completion,
                object_id,
                files,
                append,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForSetFileInputFiles {
            frontend_node_id,
            file_paths,
            append,
        } => {
            return set_file_input::complete_frontend_node_binding_for_set_file_input_files(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                file_paths,
                append,
                out,
            );
        }
        kind => kind,
    };
    let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
        out.push_error(-32000, "NoDocumentLoaded");
        return DomCommandTaskStep::Complete;
    };

    match kind {
        PendingDomCommandKind::DiscardDomAgentFrontendBindings => {
            match page.finish_discard_dom_agent_frontend_bindings(completion) {
                Ok(()) => out.push_success(),
                Err(error) => {
                    out.push_error(-32000, format!("Could not disable DOM agent: {error}"));
                }
            }
        }
        PendingDomCommandKind::RemoveNode => match page.finish_remove_document_node(completion) {
            Ok(true) => out.push_success(),
            Ok(false) => out.push_error(-32000, "Could not remove node"),
            Err(error) => out.push_error(-32000, format!("Could not remove node: {error}")),
        },
        PendingDomCommandKind::SetFileInputFilesPreflight {
            reference,
            file_paths,
            append,
        } => {
            return set_file_input::complete_preflight(
                page,
                command_id,
                &owner_scope,
                completion,
                reference,
                file_paths,
                append,
                out,
            );
        }
        PendingDomCommandKind::Focus {
            missing_node_message,
        } => match page.finish_focus_document_node_id(completion) {
            Ok(RendererDomFocusOutcome::Focused) => out.push_success(),
            Ok(RendererDomFocusOutcome::NodeNotFound) => {
                out.push_error(-32000, missing_node_message);
            }
            Ok(RendererDomFocusOutcome::NodeNotElement) => {
                out.push_error(-32000, "Node is not an Element");
            }
            Ok(RendererDomFocusOutcome::ElementNotFocusable) => {
                out.push_error(-32000, "Element is not focusable");
            }
            Err(error) => out.push_error(-32000, format!("Could not focus node: {error}")),
        },
        PendingDomCommandKind::MutateAttribute => {
            match page.finish_mutate_document_node_attribute(completion) {
                Ok(RendererDomAttributeMutationOutcome::Applied { .. }) => out.push_success(),
                Ok(RendererDomAttributeMutationOutcome::NodeNotFound) => {
                    out.push_error(-32000, "Could not find node with given id");
                }
                Ok(RendererDomAttributeMutationOutcome::NodeNotElement) => {
                    out.push_error(-32000, "Node is not an Element");
                }
                Ok(RendererDomAttributeMutationOutcome::InvalidName { name }) => {
                    out.push_error(
                        -32000,
                        format!("InvalidCharacterError '{name}' is not a valid attribute name."),
                    );
                }
                Err(error) => {
                    out.push_error(-32000, format!("Could not mutate node attribute: {error}"));
                }
            }
        }
        PendingDomCommandKind::EditDocumentNode => {
            match page.finish_edit_document_node(completion) {
                Ok(RendererDomEditOutcome::Applied {
                    result_frontend_node_id: None,
                }) => out.push_success(),
                Ok(RendererDomEditOutcome::Applied {
                    result_frontend_node_id: Some(node_id),
                }) => out.push_result(json!({ "nodeId": node_id })),
                Ok(RendererDomEditOutcome::NodeNotFound) => {
                    out.push_error(-32000, "Could not find node with given id");
                }
                Ok(RendererDomEditOutcome::NodeNotElement) => {
                    out.push_error(-32000, "Node is not an Element");
                }
                Ok(RendererDomEditOutcome::NodeValueUnsupported) => out.push_error(
                    -32000,
                    "Can only set value of text nodes or processing instructions",
                ),
                Ok(RendererDomEditOutcome::MoveIntoSelfOrDescendant) => {
                    out.push_error(-32000, "Unable to move node into self or descendant");
                }
                Ok(RendererDomEditOutcome::AnchorNotChildOfTarget) => {
                    out.push_error(-32000, "Anchor node must be child of the target element")
                }
                Ok(RendererDomEditOutcome::DetachedNode) => {
                    out.push_error(-32000, "Cannot edit detached node");
                }
                Ok(RendererDomEditOutcome::InvalidName { name }) => out.push_error(
                    -32000,
                    format!("InvalidCharacterError '{name}' is not a valid node name."),
                ),
                Ok(RendererDomEditOutcome::CouldNotParseAttributes) => {
                    out.push_error(-32000, "Could not parse value as attributes");
                }
                Ok(RendererDomEditOutcome::MutationFailed) => {
                    out.push_error(-32000, "Could not edit node");
                }
                Err(error) => {
                    out.push_error(-32000, format!("Could not edit node: {error}"));
                }
            }
        }
        PendingDomCommandKind::SetFileInputFiles => {
            return set_file_input::complete_set_file_input_files(page, completion, out);
        }
        PendingDomCommandKind::SetFileInputFilesObjectReference => {
            return set_file_input::complete_set_file_input_files_object_reference(
                page, completion, out,
            );
        }
        PendingDomCommandKind::RendererBackendNodeClientRect { operation } => {
            return complete_renderer_backend_node_client_rect(operation, page, completion, out);
        }
        PendingDomCommandKind::GetNodeForLocation { top_frame_id } => {
            match finish_document_hit_test(page, completion, top_frame_id) {
                Ok(result) => out.push_result(get_node_for_location_result_value(&result)),
                Err(error) => out.push_error(-32000, error.message),
            }
        }
        PendingDomCommandKind::RendererBackendNodeScrollIntoViewIfNeeded => {
            return complete_renderer_backend_node_scroll_into_view_if_needed(
                page, completion, out,
            );
        }
        PendingDomCommandKind::PushNodesByBackendIdsToFrontend {
            backend_node_ids,
            node_ids,
            renderer_backend_positions,
        } => {
            return complete_push_nodes_by_backend_ids_to_frontend(
                page,
                completion,
                session_id,
                backend_node_ids,
                node_ids,
                renderer_backend_positions,
                out,
            );
        }
        PendingDomCommandKind::GetFrameOwner { frame_id } => {
            match page.finish_document_node_reference(completion) {
                Ok(Some(reference)) => {
                    let result = get_frame_owner_result_from_node_reference(reference);
                    out.push_result(get_frame_owner_result_value(&result));
                }
                Ok(None) => out.push_error(
                    -32000,
                    "Frame with the given id does not belong to the target.",
                ),
                Err(error) => out.push_error(
                    -32000,
                    format!("Could not resolve frame owner for {frame_id}: {error}"),
                ),
            }
        }
        PendingDomCommandKind::QuerySelectorLive { multiple } => {
            return complete_query_selector_live(page, completion, multiple, out);
        }
        PendingDomCommandKind::GetAttributesLive => {
            return complete_get_attributes_live(page, completion, out);
        }
        PendingDomCommandKind::GetTextLive => {
            return complete_get_text_live(page, completion, out);
        }
        PendingDomCommandKind::GetPropertyLive => {
            return complete_get_property_live(page, completion, out);
        }
        PendingDomCommandKind::RequestNodeObjectReference => {
            return complete_request_node_object_reference(session_id, page, completion, out);
        }
        PendingDomCommandKind::GetOuterHtmlDocument => {
            return complete_get_outer_html_document(page, completion, out);
        }
        PendingDomCommandKind::GetOuterHtmlObjectReference => {
            return complete_get_outer_html_object_reference(page, completion, out);
        }
        PendingDomCommandKind::GetOuterHtmlBackendNodeReference => {
            return complete_get_outer_html_backend_node_reference(page, completion, out);
        }
        PendingDomCommandKind::ScrollIntoViewIfNeededObjectReference => {
            return complete_scroll_into_view_if_needed_object_reference(page, completion, out);
        }
        PendingDomCommandKind::DescribeNodeObjectReference {
            cached_object_node,
            top_frame_id,
        } => {
            return complete_describe_node_object_reference(
                page,
                completion,
                cached_object_node,
                top_frame_id,
                out,
            );
        }
        PendingDomCommandKind::ObjectReferenceLiveClientRect { operation } => {
            return complete_object_reference_live_client_rect(operation, page, completion, out);
        }
        PendingDomCommandKind::DocumentSnapshot {
            operation,
            top_frame_id,
        } => {
            return complete_document_snapshot(operation, page, completion, top_frame_id, out);
        }
        PendingDomCommandKind::SetChildNodesSnapshotForBackendNode {
            after,
            top_frame_id,
            missing_node_message,
        } => {
            return complete_set_child_nodes_snapshot_for_backend_node(
                session_id,
                page,
                completion,
                after,
                top_frame_id,
                missing_node_message,
                out,
            );
        }
        PendingDomCommandKind::QuerySelectorSetChildNodesLive {
            multiple,
            top_frame_id,
        } => {
            return complete_query_selector_set_child_nodes_live(
                session_id,
                page,
                completion,
                multiple,
                top_frame_id,
                out,
            );
        }
        PendingDomCommandKind::ResolveNode {
            object_group,
            cache_top_frame_id,
        } => {
            let resolution = page.finish_resolve_runtime_object_for_backend_node_id(completion);
            let remote_object = match resolution {
                Ok(DocumentNodeRuntimeObjectResolution::Found(remote_object)) => remote_object,
                Ok(DocumentNodeRuntimeObjectResolution::MissingContext) => {
                    out.push_error(-32000, "ContextNotFound");
                    return DomCommandTaskStep::Complete;
                }
                Ok(DocumentNodeRuntimeObjectResolution::MissingNode) => {
                    out.push_error(-32000, "Could not find node with given id");
                    return DomCommandTaskStep::Complete;
                }
                Err(error) => {
                    out.push_error(
                        -32000,
                        format!("Could not resolve node runtime object: {error}"),
                    );
                    return DomCommandTaskStep::Complete;
                }
            };
            let remote_object = remote_object.into_protocol_value();
            if let Some(top_frame_id) = cache_top_frame_id
                && let Some(cache_object_id) = remote_object
                    .get("objectId")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            {
                return match page.start_document_node_snapshot_for_object_id_in_inspector_session(
                    renderer_inspector_session_id.clone(),
                    include_whitespace,
                    &cache_object_id,
                    0,
                    false,
                ) {
                    Ok(pending) => {
                        DomCommandTaskStep::Pending(Box::new(PendingDomCommandDispatch {
                            command_id,
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
                    Err(error) => {
                        out.push_error(
                            -32000,
                            format!("Could not snapshot resolved node: {error}"),
                        );
                        DomCommandTaskStep::Complete
                    }
                };
            }
            push_resolve_node_result(conn, &owner_scope, out, remote_object, object_group);
        }
        PendingDomCommandKind::ResolveNodeCacheSnapshot {
            remote_object,
            object_group,
            cache_object_id,
            top_frame_id,
        } => {
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
                    out.push_error(-32000, format!("Could not snapshot resolved node: {error}"));
                    return DomCommandTaskStep::Complete;
                }
            }
            push_resolve_node_result(conn, &owner_scope, out, *remote_object, object_group);
        }
        PendingDomCommandKind::ResolveNodeExecutionContextFrame {
            reference,
            execution_context_id,
            object_group,
            top_frame_id,
        } => {
            let child_frame_id =
                match page.finish_child_frame_id_for_default_execution_context_id(completion) {
                    Ok(frame_id) => frame_id,
                    Err(error) => {
                        out.push_error(
                            -32000,
                            format!("Could not resolve execution context frame: {error}"),
                        );
                        return DomCommandTaskStep::Complete;
                    }
                };
            if child_frame_id.is_some() {
                if let DevToolsDomNodeReference::BackendNodeId(backend_node_id) = reference
                    && is_renderer_backend_node_id(backend_node_id)
                {
                    return match page
                        .start_resolve_runtime_object_for_backend_node_id_in_inspector_session(
                            renderer_inspector_session_id.clone(),
                            backend_node_id,
                            Some(execution_context_id),
                            object_group.as_deref(),
                        ) {
                        Ok(pending) => {
                            DomCommandTaskStep::Pending(Box::new(PendingDomCommandDispatch {
                                command_id,
                                owner_scope: owner_scope.clone(),
                                kind: PendingDomCommandKind::ResolveNode {
                                    object_group,
                                    cache_top_frame_id: None,
                                },
                                pending,
                            }))
                        }
                        Err(error) => {
                            out.push_error(
                                -32000,
                                format!("Could not resolve node runtime object: {error}"),
                            );
                            DomCommandTaskStep::Complete
                        }
                    };
                }
                out.push_error(-32000, "Could not find node with given id");
                return DomCommandTaskStep::Complete;
            }
            return match start_resolve_runtime_object_for_reference(
                page,
                renderer_inspector_session_id.clone(),
                reference,
                Some(execution_context_id),
                object_group.as_deref(),
                top_frame_id,
            ) {
                Ok(resolution) => {
                    DomCommandTaskStep::Pending(Box::new(PendingDomCommandDispatch {
                        command_id,
                        owner_scope: owner_scope.clone(),
                        kind: PendingDomCommandKind::ResolveNode {
                            object_group,
                            cache_top_frame_id: resolution.cache_top_frame_id,
                        },
                        pending: resolution.pending,
                    }))
                }
                Err(error) => {
                    out.push_error(error.code, error.message);
                    DomCommandTaskStep::Complete
                }
            };
        }
        PendingDomCommandKind::PerformSearchLive
        | PendingDomCommandKind::GetSearchResultsLive
        | PendingDomCommandKind::DiscardSearchResultsLive
        | PendingDomCommandKind::SetNodeStackTracesEnabled
        | PendingDomCommandKind::GetNodeStackTraces
        | PendingDomCommandKind::ResolveFrontendNodeForGetAttributes { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForGetText { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForGetProperty { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForDomGeometry { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForDescribeNode { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForRequestChildNodes { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForRemoveNode { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForFocus { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForMutateAttribute { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForQuerySelector { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForResolveNode { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForGetOuterHtml { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForScrollIntoViewIfNeeded { .. }
        | PendingDomCommandKind::ResolveBidiNodeForSetFileInputFiles { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForSetFileInputFiles { .. } => {
            unreachable!(
                "DOM search/frontend/shared-node lookup pending commands are completed before borrowing page"
            )
        }
    }
    DomCommandTaskStep::Complete
}
