use super::super::*;
use super::RendererInspectorPageCommand;

impl PageVm {
    pub(in crate::runtime) fn dispatch_devtools_command(
        &mut self,
        command: RendererDevToolsPageCommand,
    ) -> Result<RendererPageReply> {
        match command {
            RendererDevToolsPageCommand::Native(command) => {
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::NativeCommandReady(Box::new(command.dispatch(self)))))
            }
            RendererDevToolsPageCommand::Inspector(command) => {
                self.dispatch_renderer_inspector_command(command)
            }
            RendererDevToolsPageCommand::DispatchPreparedElementClick(click) => {
                let mut outcome = self.vm_mut().dispatch_prepared_element_click(click);
                if let Ok(outcome) = &mut outcome {
                    self.bind_input_dispatch_file_chooser_backend_node_id(outcome);
                }
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::ElementClickDispatch(outcome)))
            }
            RendererDevToolsPageCommand::DispatchMouseEventAtPoint {
                x,
                y,
                event_name,
                button,
                buttons,
                click_count,
                delta_x,
                delta_y,
                pointer,
                modifiers,
                intercept_drags: _,
            } if event_name == "wheel" => self
                .queue_wheel_event(
                    x,
                    y,
                    button,
                    buttons,
                    click_count,
                    delta_x,
                    delta_y,
                    pointer,
                    modifiers,
                )
                .map(RendererDevToolsPageReply::InputDispatchOutcome)
                .map(RendererPageReply::DevTools),
            RendererDevToolsPageCommand::DispatchMouseEventAtPoint {
                x,
                y,
                event_name,
                button,
                buttons,
                click_count,
                delta_x,
                delta_y,
                pointer,
                modifiers,
                intercept_drags,
            } => {
                self.dispatch_mouse_event_at_point_with_pointer(
                    x,
                    y,
                    &event_name,
                    button,
                    buttons,
                    click_count,
                    delta_x,
                    delta_y,
                    pointer,
                    modifiers,
                    intercept_drags,
                )
                .map(RendererDevToolsPageReply::InputDispatchOutcome)
                .map(RendererPageReply::DevTools)
            }
            RendererDevToolsPageCommand::DispatchTouchEvent {
                points,
                event_name,
                activate,
            } => self
                .dispatch_touch_event_at_points(&points, &event_name, activate)
                .map(RendererDevToolsPageReply::InputDispatchOutcome)
                .map(RendererPageReply::DevTools),
            RendererDevToolsPageCommand::DispatchDragEventAtPoint {
                x,
                y,
                event_name,
                data,
                modifiers,
            } => self
                .dispatch_drag_event_at_point(x, y, &event_name, data, modifiers)
                .map(RendererDevToolsPageReply::InputDispatchOutcome)
                .map(RendererPageReply::DevTools),
            RendererDevToolsPageCommand::ClearActiveDragDataTransfer => {
                self.clear_active_drag_data_transfer()?;
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::CancelDragging => self
                .cancel_dragging()
                .map(RendererDevToolsPageReply::InputDispatchOutcome)
                .map(RendererPageReply::DevTools),
            RendererDevToolsPageCommand::MutateDocumentNodeAttribute {
                reference,
                mutation,
            } => self
                .mutate_document_node_attribute(reference, mutation)
                .map(RendererDevToolsPageReply::DomAttributeMutationOutcome)
                .map(RendererPageReply::DevTools),
            RendererDevToolsPageCommand::EditDocumentNode {
                inspector_session_id,
                edit,
            } => self
                .edit_document_node(inspector_session_id.as_deref(), edit)
                .map(RendererDevToolsPageReply::DomEditOutcome)
                .map(RendererPageReply::DevTools),
            RendererDevToolsPageCommand::FocusDocumentNode { reference } => self
                .focus_document_node(reference)
                .map(RendererDevToolsPageReply::DomFocusOutcome)
                .map(RendererPageReply::DevTools),
            RendererDevToolsPageCommand::TriggerAutofill(request) => self
                .trigger_autofill(request)
                .map(RendererDevToolsPageReply::AutofillTriggerOutcome)
                .map(RendererPageReply::DevTools),
            RendererDevToolsPageCommand::WebMcp { session, command } => {
                let enabled = match command {
                    moli_page_types::RendererWebMcpCommand::Enable => Some(true),
                    moli_page_types::RendererWebMcpCommand::Disable => Some(false),
                    _ => None,
                };
                let result = self.vm_mut().dispatch_web_mcp_command(session.clone(), command)?;
                if result.is_ok() && let Some(enabled) = enabled {
                    self.devtools.inspector_protocol_configurations.entry(session).or_default().web_mcp_enabled = enabled;
                }
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::WebMcp(result)))
            }
            RendererDevToolsPageCommand::SetFileInputFilesForBackendNodeId {
                backend_node_id,
                files,
                append,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalBool(
                self.set_file_input_files_for_backend_node_id(backend_node_id, files, append)?,
            ))),
            RendererDevToolsPageCommand::InsertTextIntoActiveControl(text) => self
                .insert_text_into_active_control(&text)
                .map(RendererPageReply::Bool),
            RendererDevToolsPageCommand::DispatchKeyEvent {
                event_name,
                key,
                code,
                text,
                modifiers,
                location,
                auto_repeat,
                should_insert_text,
            } => self
                .dispatch_key_event(
                    &event_name,
                    &key,
                    &code,
                    &text,
                    modifiers,
                    auto_repeat,
                    should_insert_text,
                    location,
                )
                .map(RendererDevToolsPageReply::InputDispatchOutcome)
                .map(RendererPageReply::DevTools),
            RendererDevToolsPageCommand::CreateIsolatedWorld {
                name,
                grant_universal_access,
                frame_id,
                ..
            } => match frame_id {
                Some(frame_id) => self
                    .create_isolated_world_for_frame(&frame_id, &name, grant_universal_access)
                    .map(RendererDevToolsPageReply::ExecutionContextId)
                .map(RendererPageReply::DevTools),
                None => self
                    .create_isolated_world(&name, grant_universal_access)
                    .map(RendererDevToolsPageReply::ExecutionContextId)
                .map(RendererPageReply::DevTools),
            },
            RendererDevToolsPageCommand::CreateIsolatedWorldRuntimeActivity {
                inspector_session_id,
                frame_id,
                name,
                grant_universal_access,
            } => self
                .create_isolated_world_runtime_activity(
                    inspector_session_id.as_deref(),
                    frame_id.as_deref(),
                    &name,
                    grant_universal_access,
                )
                .map(RendererDevToolsPageReply::ExecutionContextId)
                .map(RendererPageReply::DevTools),
            RendererDevToolsPageCommand::InstallRuntimeBinding {
                name,
                execution_context_name,
                execution_context_id,
            } => {
                self.install_runtime_binding(
                    &name,
                    execution_context_name.as_deref(),
                    execution_context_id,
                )?;
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::RemoveRuntimeBinding(name) => {
                self.remove_runtime_binding(&name)?;
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::RemoveDefaultRuntimeBinding(name) => {
                self.remove_default_runtime_binding(&name)?;
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::AddDocumentStartScriptRuntimeActivity {
                inspector_session_id,
                script,
                run_immediately,
            } => Ok(RendererPageReply::DocumentStartScriptResult(
                self.add_document_start_script_runtime_activity(
                    inspector_session_id.as_deref(),
                    &script,
                    run_immediately,
                )?,
            )),
            RendererDevToolsPageCommand::RemoveDocumentStartScriptByRegistryKey(registry_key) => {
                self.remove_document_start_script_by_registry_key(&registry_key);
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::ConnectRuntimeInspectorSession { inspector_session_id, connection } => {
                anyhow::ensure!(connection.is_current(), "Inspector session connection was retired");
                self.vm_mut().ensure_runtime_inspector_session(inspector_session_id.as_deref());
                connection.connected();
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::SetSessionRuntimeBindingState { agent, inspector_session_id, session_runtime_bindings } => {
                anyhow::ensure!(self.devtools_agent_token() == agent, "Inspector agent was replaced before binding replay");
                // Replace this session's contribution, never an old snapshot
                // of bindings contributed by other Browser sessions.
                let session = DevToolsSessionKey::from_wire_session_id(inspector_session_id.as_deref());
                self.runtime_bindings.retain(|binding| binding.devtools_session.as_ref() != Some(&session));
                self.runtime_bindings.extend(session_runtime_bindings.iter().map(|binding| binding.with_devtools_session(session.clone())));
                let bindings = self.runtime_bindings.clone();
                self.vm_mut().set_stored_runtime_bindings(&bindings);
                self.set_inspector_session_runtime_bindings(inspector_session_id.as_deref(), &session_runtime_bindings);
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::SetRuntimeBindingState {
                inspector_session_id,
                stored_runtime_bindings,
                session_runtime_bindings,
            } => {
                self.set_runtime_binding_state(
                    inspector_session_id.as_deref(),
                    &stored_runtime_bindings,
                    &session_runtime_bindings,
                );
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::EnsureIsolatedWorldsAttachedToInspector => {
                self.ensure_isolated_worlds_attached_to_inspector()?;
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::InspectorExecutionContextIdForIsolatedContext(
                execution_context_id,
            ) => Ok(RendererPageReply::OptionalExecutionContextId(
                self.inspector_execution_context_id_for_isolated_context(execution_context_id),
            )),
            RendererDevToolsPageCommand::IsolatedExecutionContextIdForInspectorContext(
                execution_context_id,
            ) => Ok(RendererPageReply::OptionalExecutionContextId(
                self.isolated_execution_context_id_for_inspector_context(execution_context_id),
            )),
            RendererDevToolsPageCommand::PerformanceMetricSnapshot => Ok(
                RendererPageReply::DevTools(RendererDevToolsPageReply::PerformanceMetricSnapshot(Box::new(
                    self.vm_mut().performance_metric_snapshot()?,
                ))),
            ),
            RendererDevToolsPageCommand::DomDebuggerConfigureEventListenerBreakpoint {
                inspector_session_id,
                breakpoint,
                enabled,
            } => {
                self.configure_dom_debugger_event_listener_breakpoint(
                    inspector_session_id.as_deref(),
                    breakpoint,
                    enabled,
                );
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::DomDebuggerConfigureXhrBreakpoint {
                inspector_session_id,
                breakpoint,
                enabled,
            } => {
                self.configure_dom_debugger_xhr_breakpoint(
                    inspector_session_id.as_deref(),
                    breakpoint,
                    enabled,
                );
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::DomDebuggerConfigureDomBreakpoint {
                inspector_session_id,
                frontend_node_id,
                breakpoint_type,
                enabled,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::DomDebuggerDomBreakpoint(
                self.configure_dom_debugger_dom_breakpoint(
                    inspector_session_id.as_deref(),
                    frontend_node_id,
                    &breakpoint_type,
                    enabled,
                ),
            ))),
            RendererDevToolsPageCommand::RuntimeCollectGarbage => {
                self.vm_mut()
                    .renderer_document_isolate_ops()
                    .collect_renderer_document_isolate_garbage()?;
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::SearchResourceByLines(request) => Ok(
                RendererPageReply::DevTools(RendererDevToolsPageReply::ResourceTextSearchOutcome(self.search_resource_by_lines(*request)?)),
            ),
            RendererDevToolsPageCommand::ComputedStyleProperties { reference } => {
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::ComputedStyleProperties(
                    self.computed_style_properties_for_node(reference)?,
                )))
            }
            RendererDevToolsPageCommand::SetInlineStyleSheetTextForStyleSheetId {
                inspector_session_id,
                style_sheet_id,
                text,
            } => self
                .set_inline_style_sheet_text_for_style_sheet_id(
                    inspector_session_id.as_deref(),
                    &style_sheet_id,
                    &text,
                )
                .map(RendererPageReply::Bool),
            RendererDevToolsPageCommand::ScrollNodeIntoViewIfNeeded {
                reference,
                rect,
            } => self
                .scroll_node_into_view_if_needed(reference, rect)
                .map(RendererDevToolsPageReply::ScrollIntoViewResult)
                .map(RendererPageReply::DevTools),
            RendererDevToolsPageCommand::ClientRectForBackendNodeId { backend_node_id } => Ok(
                RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalDocumentNodeClientRect(
                    self.client_rect_for_backend_node_id(backend_node_id)?,
                )),
            ),
            RendererDevToolsPageCommand::DocumentGeometryForNode { reference } => Ok(
                RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalDocumentNodeGeometry(
                    self.document_geometry_for_node(reference)?,
                )),
            ),
            RendererDevToolsPageCommand::DocumentHitTest {
                inspector_session_id,
                x,
                y,
                include_user_agent_shadow_dom,
                ignore_pointer_events_none,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalDocumentHitTest(
                self.document_hit_test(
                    inspector_session_id.as_deref(),
                    x,
                    y,
                    include_user_agent_shadow_dom,
                    ignore_pointer_events_none,
                )?,
            ))),
            RendererDevToolsPageCommand::NodeHasGeometryForBackendNodeId { backend_node_id } => {
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalBool(
                    self.node_has_geometry_for_backend_node_id(backend_node_id)?,
                )))
            }
            RendererDevToolsPageCommand::RemoveDocumentNode { reference } => self
                .remove_document_node(reference)
                .map(RendererDevToolsPageReply::OptionalBool)
                .map(RendererPageReply::DevTools),
            RendererDevToolsPageCommand::DocumentNodeExistsForBackendNodeId { backend_node_id } => self.document_node_exists_for_backend_node_id(backend_node_id).map(RendererPageReply::Bool),
            RendererDevToolsPageCommand::DocumentNodeSnapshotForBackendNodeId {
                backend_node_id,
                depth,
                pierce,
            } => Ok(RendererPageReply::OptionalDocumentNodeObjectSnapshot(Box::new(
                self.document_node_snapshot_for_backend_node_id(backend_node_id, depth, pierce)?,
            ))),
            RendererDevToolsPageCommand::DocumentNodeSnapshotForNodeInInspectorSession {
                inspector_session_id,
                include_whitespace,
                reference,
                depth,
                pierce,
            } => Ok(RendererPageReply::OptionalDocumentNodeObjectSnapshot(
                Box::new(self.document_node_snapshot_for_node_in_inspector_session(
                    inspector_session_id.as_deref(), include_whitespace, reference, depth, pierce,
                )?),
            )),
            RendererDevToolsPageCommand::DocumentNodeSnapshotForDocument {
                inspector_session_id,
                include_whitespace,
                depth,
                pierce,
            } => {
                self.configure_document_dom_agent_session(
                    inspector_session_id.as_deref(),
                    include_whitespace,
                );
                self.discard_document_frontend_bindings(inspector_session_id.as_deref());
                Ok(RendererPageReply::OptionalDocumentNodeObjectSnapshot(Box::new(
                    self.document_node_snapshot_for_document(
                        inspector_session_id.as_deref(),
                        depth,
                        pierce,
                    ),
                )))
            }
            RendererDevToolsPageCommand::DiscardDomAgentFrontendBindings {
                inspector_session_id,
            } => {
                self.vm_mut()
                    .clear_dom_debugger_dom_breakpoints_for_session(
                        inspector_session_id.as_deref(),
                    );
                self.discard_document_frontend_bindings(inspector_session_id.as_deref());
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::DomSnapshotCapture {
                top_frame_id,
                options,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalDomSnapshotCapturePayload(
                self.dom_snapshot_capture_payload(&top_frame_id, options)?,
            ))),
            RendererDevToolsPageCommand::DocumentChildNodeSnapshotEventsForNode {
                inspector_session_id,
                include_whitespace,
                reference,
                depth,
                pierce,
            } => {
                Ok(RendererPageReply::OptionalDocumentChildNodeSnapshotEvents(
                    self.document_child_node_snapshot_events_for_node(
                        inspector_session_id.as_deref(),
                        include_whitespace,
                        reference,
                        depth,
                        pierce,
                    ),
                ))
            }
            RendererDevToolsPageCommand::DocumentQuerySelectorForDocument {
                inspector_session_id,
                include_whitespace,
                selector,
                multiple,
            } => {
                self.configure_document_dom_agent_session(
                    inspector_session_id.as_deref(),
                    include_whitespace,
                );
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentQuerySelectorResolution(
                    self.document_query_selector_for_document(
                        inspector_session_id.as_deref(),
                        &selector,
                        multiple,
                    ),
                )))
            }
            RendererDevToolsPageCommand::DocumentQuerySelectorForChildFrameNode {
                inspector_session_id,
                include_whitespace,
                frame_id,
                root,
                selector,
                multiple,
            } => {
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentQuerySelectorResolution(
                    self.document_query_selector_for_child_frame_node(
                        inspector_session_id.as_deref(),
                        include_whitespace,
                        &frame_id,
                        root,
                        &selector,
                        multiple,
                    ),
                )))
            }
            RendererDevToolsPageCommand::DocumentQuerySelectorForNode {
                inspector_session_id,
                include_whitespace,
                root,
                selector,
                multiple,
            } => {
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentQuerySelectorResolution(
                    self.document_query_selector_for_node(
                        inspector_session_id.as_deref(),
                        include_whitespace,
                        root,
                        &selector,
                        multiple,
                    ),
                )))
            }
            RendererDevToolsPageCommand::DocumentQuerySelectorWithChildNodeSnapshotEventsForNode {
                inspector_session_id,
                include_whitespace,
                root,
                selector,
                multiple,
            } => {
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentQuerySelectorWithChildNodeSnapshotEvents(
                    self.document_query_selector_with_child_node_snapshot_events_for_node(
                        inspector_session_id.as_deref(),
                        include_whitespace,
                        root,
                        &selector,
                        multiple,
                    ),
                )))
            }
            RendererDevToolsPageCommand::DocumentPerformSearch {
                inspector_session_id,
                query,
                include_user_agent_shadow_dom,
                include_whitespace,
            } => {
                self.configure_document_dom_agent_session(
                    inspector_session_id.as_deref(),
                    include_whitespace,
                );
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentPerformSearch(
                    self.document_perform_search(
                        inspector_session_id.as_deref(),
                        &query,
                        include_user_agent_shadow_dom,
                    ),
                )))
            }
            RendererDevToolsPageCommand::DocumentGetSearchResults {
                inspector_session_id,
                search_id,
                from_index,
                to_index,
            } => {
                let (resolution, child_node_snapshot_events) = self
                    .document_search_results_with_child_node_snapshot_events(
                    inspector_session_id.as_deref(),
                    &search_id,
                    from_index,
                    to_index,
                );
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentSearchResults {
                    resolution,
                    child_node_snapshot_events,
                }))
            }
            RendererDevToolsPageCommand::DocumentDiscardSearchResults {
                inspector_session_id,
                search_id,
            } => {
                self.discard_document_search_results(inspector_session_id.as_deref(), &search_id);
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentSearchResultsDiscarded))
            }
            RendererDevToolsPageCommand::DocumentSetNodeStackTracesEnabled {
                inspector_session_id,
                enabled,
            } => {
                self.set_document_node_stack_traces_enabled(
                    inspector_session_id.as_deref(),
                    enabled,
                );
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentNodeStackTracesEnabled))
            }
            RendererDevToolsPageCommand::DocumentNodeStackTrace {
                inspector_session_id,
                frontend_node_id,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentNodeStackTrace(
                self.document_node_stack_trace(
                    inspector_session_id.as_deref(),
                    frontend_node_id,
                ),
            ))),
            RendererDevToolsPageCommand::DocumentFrontendNodeBinding { reference } => {
                let binding = self.backend_node_id_for_document_node_reference(reference);
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentFrontendNodeBinding(
                    binding.map_or(
                        crate::RendererDomFrontendNodeBindingResolution::NotFound,
                        crate::RendererDomFrontendNodeBindingResolution::BackendNodeId,
                    ),
                )))
            }
            RendererDevToolsPageCommand::RegisterDocumentBidiNodeBinding {
                inspector_session_id,
                shared_id,
                backend_node_id,
            } => {
                self.register_document_bidi_node_binding(
                    inspector_session_id.as_deref(),
                    shared_id,
                    backend_node_id,
                );
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentBidiNodeBindingRegistered))
            }
            RendererDevToolsPageCommand::DocumentBidiNodeBinding {
                inspector_session_id,
                shared_id,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentBidiNodeBinding(
                self.document_bidi_node_binding(inspector_session_id.as_deref(), &shared_id),
            ))),
            RendererDevToolsPageCommand::DocumentBidiNodeSharedIdForBackendNodeId {
                inspector_session_id,
                backend_node_id,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentBidiNodeSharedId(
                self.document_bidi_node_shared_id_for_backend_node_id(
                    inspector_session_id.as_deref(),
                    backend_node_id,
                ),
            ))),
            RendererDevToolsPageCommand::DocumentNodeAttributes { reference } => {
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentNodeAttributesResolution(
                    self.document_node_attributes(reference),
                )))
            }
            RendererDevToolsPageCommand::DocumentNodeText { reference } => {
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentNodeTextResolution(
                    self.document_node_text(reference),
                )))
            }
            RendererDevToolsPageCommand::DocumentNodeProperty {
                reference,
                name,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentNodePropertyResolution(
                self.document_node_property(reference, &name),
            ))),
            RendererDevToolsPageCommand::AccessibilityTreePayloadsForDocument { max_depth } => Ok(
                RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalAccessibilityPayloads(
                    self.accessibility_tree_payloads_for_document(max_depth),
                )),
            ),
            RendererDevToolsPageCommand::AccessibilityNodePayloadForDocument => Ok(
                RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalAccessibilityPayload(
                    self.accessibility_node_payload_for_document(),
                )),
            ),
            RendererDevToolsPageCommand::AccessibilityTreePayloadsForNode {
                reference,
                max_depth,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalAccessibilityPayloadsForObjectId(
                self.accessibility_tree_payloads_for_node(reference, max_depth),
            ))),
            RendererDevToolsPageCommand::AccessibilityNodePayloadForNode { reference } => {
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalAccessibilityPayloadsForObjectId(
                    self.accessibility_node_payload_for_node(reference),
                )))
            }
            RendererDevToolsPageCommand::AccessibilityNodeAndAncestorPayloadsForNode {
                reference,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalAccessibilityPayloadsForObjectId(
                self.accessibility_node_and_ancestor_payloads_for_node(reference),
            ))),
            RendererDevToolsPageCommand::AccessibilityChildNodePayloadsForNode {
                reference,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalAccessibilityPayloadsForObjectId(
                self.accessibility_child_node_payloads_for_node(reference),
            ))),
            RendererDevToolsPageCommand::AccessibilityPartialTreePayloadsForNode {
                reference,
                fetch_relatives,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalAccessibilityPayloadsForObjectId(
                self.accessibility_partial_tree_payloads_for_node(
                    reference,
                    fetch_relatives,
                ),
            ))),
            RendererDevToolsPageCommand::AccessibilityTreePayloadsForChildFrame {
                frame_id,
                max_depth,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalAccessibilityPayloadsForObjectId(
                self.accessibility_tree_payloads_for_child_frame(&frame_id, max_depth),
            ))),
            RendererDevToolsPageCommand::AccessibilityNodePayloadForChildFrame { frame_id } => {
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalAccessibilityPayloadsForObjectId(
                    self.accessibility_node_payload_for_child_frame(&frame_id),
                )))
            }
            RendererDevToolsPageCommand::StyleSheetPayloadForStyleSheetId {
                inspector_session_id,
                style_sheet_id,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalStyleSheetPayload(
                self.style_sheet_payload_for_style_sheet_id(
                    inspector_session_id.as_deref(),
                    &style_sheet_id,
                ),
            ))),
            RendererDevToolsPageCommand::StyleSheetInventoryForDocument {
                inspector_session_id,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::StyleSheetInventory(
                self.style_sheet_inventory_for_document(inspector_session_id.as_deref()),
            ))),
            RendererDevToolsPageCommand::ResetCssAgentSession {
                inspector_session_id,
            } => {
                self.reset_css_agent_session(inspector_session_id.as_deref());
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::CaptureScreencastFrame(request) => self
                .capture_screencast_frame(request)
                .map(RendererDevToolsPageReply::CaptureScreencastFrame)
                .map(RendererPageReply::DevTools),
            RendererDevToolsPageCommand::DocumentFrontendNodeIdsForBackendNodeIds {
                inspector_session_id,
                backend_node_ids,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentFrontendNodeIds(
                self.document_frontend_node_ids_for_backend_node_ids(
                    inspector_session_id.as_deref(),
                    &backend_node_ids,
                ),
            ))),
            RendererDevToolsPageCommand::ChildFrameOwnerNodeReference {
                inspector_session_id,
                frame_id,
            } => Ok(RendererPageReply::OptionalDocumentNodeReference(
                self.child_frame_owner_node_reference_by_frame_id(
                    inspector_session_id.as_deref(),
                    &frame_id,
                ),
            )),
            RendererDevToolsPageCommand::ChildFrameDocumentRootNodeReference {
                inspector_session_id,
                frame_id,
            } => Ok(RendererPageReply::OptionalDocumentNodeReference(
                self.child_frame_document_root_node_reference_by_frame_id(
                    inspector_session_id.as_deref(),
                    &frame_id,
                ),
            )),
            RendererDevToolsPageCommand::ContinuePendingSubresourceFetch {
                internal_id,
                url,
                method,
                body,
                headers,
                intercept_response,
                handle_auth_requests,
            } => self
                .continue_pending_subresource_fetch(
                    internal_id,
                    url,
                    method,
                    body,
                    headers,
                    intercept_response,
                    handle_auth_requests,
                )
                .map(RendererPageReply::PendingSubresourceContinueOutcome),
            RendererDevToolsPageCommand::ContinuePendingSubresourceAuth { internal_id, auth } => self
                .continue_pending_subresource_auth(internal_id, auth)
                .map(RendererPageReply::PendingSubresourceContinueOutcome),
            RendererDevToolsPageCommand::CancelPendingSubresourceAuth { internal_id } => {
                self.cancel_pending_subresource_auth(internal_id)?;
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::FailPendingSubresourceAuth {
                internal_id,
                error_text,
            } => {
                self.fail_pending_subresource_auth(internal_id, error_text)?;
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::FailPendingSubresourceFetch {
                internal_id,
                error_text,
            } => {
                self.fail_pending_subresource_fetch(internal_id, error_text)?;
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::FulfillPendingSubresourceFetch {
                internal_id,
                request_overrides,
                response_code,
                response_headers,
                response_body,
            } => {
                self.fulfill_pending_subresource_fetch(
                    internal_id,
                    response_code,
                    response_headers,
                    response_body,
                    request_overrides.map(|overrides|*overrides),
                )?;
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::ContinuePendingSubresourceResponse {
                internal_id,
                response_code,
                response_headers,
            } => {
                self.continue_pending_subresource_response(
                    internal_id,
                    response_code,
                    response_headers,
                )?;
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::FailPendingSubresourceResponse {
                internal_id,
                error_text,
            } => {
                self.fail_pending_subresource_response(internal_id, error_text)?;
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::FulfillPendingSubresourceResponse {
                internal_id,
                response_code,
                response_headers,
                response_body,
            } => {
                self.fulfill_pending_subresource_response(
                    internal_id,
                    response_code,
                    response_headers,
                    response_body,
                )?;
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::ReceiveSyntheticWebSocketText { socket_id, data } => {
                if self.receive_synthetic_websocket_text(socket_id, data) {
                    Ok(RendererPageReply::Unit)
                } else {
                    anyhow::bail!("unknown synthetic WebSocket `{socket_id}`")
                }
            }
            RendererDevToolsPageCommand::ReceiveSyntheticWebSocketBinary { socket_id, data } => {
                if self.receive_synthetic_websocket_binary(socket_id, data) {
                    Ok(RendererPageReply::Unit)
                } else {
                    anyhow::bail!("unknown synthetic WebSocket `{socket_id}`")
                }
            }
            RendererDevToolsPageCommand::CloseSyntheticWebSocketFromServer {
                socket_id,
                code,
                reason,
            } => {
                if self.close_synthetic_websocket_from_server(socket_id, code, reason) {
                    Ok(RendererPageReply::Unit)
                } else {
                    anyhow::bail!("unknown synthetic WebSocket `{socket_id}`")
                }
            }
            RendererDevToolsPageCommand::SetFetchSubresourceInterception { classes } => {
                self.set_fetch_subresource_interception(classes);
                Ok(RendererPageReply::Unit)
            }
            RendererDevToolsPageCommand::PrepareNetworkResourceLoad {
                frame_id,
                url,
                disable_cache,
                include_credentials,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::NetworkResourceLoadPreparation(
                self.vm().prepare_devtools_network_resource_load(
                    &frame_id,
                    url,
                    disable_cache,
                    include_credentials,
                ),
            ))),
        }
    }

    fn dispatch_renderer_inspector_command(
        &mut self,
        envelope: RendererInspectorCommandEnvelope,
    ) -> Result<RendererPageReply> {
        let (ticket, command) = envelope.into_main_thread_parts();
        let inspector_session_id = ticket.session().wire_session_id();
        match command {
            RendererInspectorPageCommand::DispatchRuntimeProtocolMessage { raw_json } => self
                .dispatch_runtime_protocol_message_for_inspector_session(
                    inspector_session_id,
                    &raw_json,
                )
                .map(RendererRuntimeCommandOutput::from_messages)
                .map(RendererPageReply::RuntimeInspectorProtocolMessages),
            RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithDeferredResponse {
                raw_json,
                deferred_response,
            } => self
                .dispatch_frontend_runtime_protocol_message_for_inspector_session_with_deferred_response(
                    inspector_session_id,
                    &raw_json,
                    deferred_response,
                )
                .map(RendererRuntimeCommandOutput::from_messages)
                .map(RendererPageReply::RuntimeInspectorProtocolMessages),
            RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithContextResolution {
                action,
                raw_json,
            } => self
                .dispatch_runtime_protocol_message_for_inspector_session_with_context_resolution(
                    inspector_session_id,
                    &action,
                    &raw_json,
                )
                .map(RendererRuntimeCommandOutput::from_messages)
                .map(RendererPageReply::RuntimeInspectorProtocolMessages),
            RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithContextResolutionAndDeferredResponse {
                action,
                raw_json,
                deferred_response,
            } => self
                .dispatch_frontend_runtime_protocol_message_for_inspector_session_with_context_resolution_and_deferred_response(
                    inspector_session_id,
                    &action,
                    &raw_json,
                    deferred_response,
                )
                .map(RendererRuntimeCommandOutput::from_messages)
                .map(RendererPageReply::RuntimeInspectorProtocolMessages),
            RendererInspectorPageCommand::RuntimeEnableEvents => self
                .runtime_enable_events(inspector_session_id)
                .map(RendererPageReply::RuntimeInspectorProtocolMessages),
            RendererInspectorPageCommand::RuntimeDisableEvents => self
                .runtime_disable_events(inspector_session_id)
                .map(RendererPageReply::RuntimeInspectorProtocolMessages),
            RendererInspectorPageCommand::RuntimeBindingEvents { binding, remove } => self
                .runtime_binding_events(inspector_session_id, binding, remove)
                .map(RendererPageReply::RuntimeInspectorProtocolMessages),
            RendererInspectorPageCommand::CommitRuntimeBindingRemoval { name, output } => self
                .commit_runtime_binding_removal(inspector_session_id, &name, output)
                .map(RendererPageReply::RuntimeInspectorProtocolMessages),
            RendererInspectorPageCommand::ApplyRuntimeProtocolState {
                session_restore_snapshots,
                isolated_worlds,
                stored_runtime_bindings,
                session_runtime_bindings,
            } => {
                self.apply_runtime_protocol_state(
                    inspector_session_id,
                    &session_restore_snapshots,
                    &isolated_worlds,
                    &stored_runtime_bindings,
                    &session_runtime_bindings,
                )?;
                Ok(RendererPageReply::Unit)
            }
            RendererInspectorPageCommand::DetachRuntimeInspectorSession {
                pause_guard: _pause_guard,
            } => Ok(RendererPageReply::Bool(
                self.detach_runtime_inspector_session(inspector_session_id),
            )),
            RendererInspectorPageCommand::AddRuntimeBinding {
                name,
                execution_context_name,
                execution_context_id,
            } => {
                self.add_runtime_binding(
                    inspector_session_id,
                    &name,
                    execution_context_name.as_deref(),
                    execution_context_id,
                )?;
                Ok(RendererPageReply::Unit)
            }
            RendererInspectorPageCommand::DomDebuggerGetEventListeners {
                object_id,
                depth,
                pierce,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::DomDebuggerEventListeners(
                self.vm_mut().dom_debugger_event_listeners(
                    inspector_session_id,
                    &object_id,
                    depth,
                    pierce,
                )?,
            ))),
            RendererInspectorPageCommand::ComputedStylePropertiesForObjectId { object_id } => {
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::ComputedStyleProperties(
                    self.computed_style_properties_for_object_id(
                        inspector_session_id,
                        &object_id,
                    )?,
                )))
            }
            RendererInspectorPageCommand::PrepareElementClick { object_id } => {
                let handle = self.vm_mut().live_node_handle_for_runtime_object_id(inspector_session_id, &object_id)?;
                let preparation = match handle {
                    Some(handle) => self.vm_mut().prepare_element_click(handle),
                    None => Err(RendererElementClickError::StaleNode),
                };
                Ok(RendererPageReply::ElementClickPreparation(preparation))
            }
            RendererInspectorPageCommand::ScrollObjectNodeIntoViewIfNeeded { object_id, rect } => {
                self.scroll_node_into_view_if_needed_for_object_id(
                    inspector_session_id,
                    &object_id,
                    rect,
                )
                .map(RendererDevToolsPageReply::ScrollIntoViewResult)
                .map(RendererPageReply::DevTools)
            }
            RendererInspectorPageCommand::ClientRectForObjectId { object_id } => {
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalDocumentNodeClientRect(
                    self.client_rect_for_object_id(inspector_session_id, &object_id)?,
                )))
            }
            RendererInspectorPageCommand::DocumentGeometryForObjectId { object_id } => {
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalDocumentNodeGeometry(
                    self.document_geometry_for_object_id(inspector_session_id, &object_id)?,
                )))
            }
            RendererInspectorPageCommand::NodeHasGeometryForObjectId { object_id } => {
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalBool(
                    self.node_has_geometry_for_object_id(inspector_session_id, &object_id)?,
                )))
            }
            RendererInspectorPageCommand::FocusDocumentNodeForObjectId { object_id } => self
                .focus_document_node_for_object_id(inspector_session_id, &object_id)
                .map(RendererDevToolsPageReply::DomFocusOutcome)
                .map(RendererPageReply::DevTools),
            RendererInspectorPageCommand::SetFileInputFilesForObjectId {
                object_id,
                files,
                append,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalBool(
                self.set_file_input_files_for_object_id(
                    inspector_session_id,
                    &object_id,
                    files,
                    append,
                )?,
            ))),
            RendererInspectorPageCommand::DocumentNodeSnapshotForObjectId {
                include_whitespace,
                object_id,
                depth,
                pierce,
                bind_frontend_nodes,
            } => {
                self.configure_document_dom_agent_session(
                    inspector_session_id,
                    include_whitespace,
                );
                Ok(RendererPageReply::OptionalDocumentNodeObjectSnapshot(
                    Box::new(self.document_node_snapshot_for_object_id(
                        inspector_session_id,
                        &object_id,
                        depth,
                        pierce,
                        bind_frontend_nodes,
                    )?),
                ))
            }
            RendererInspectorPageCommand::RequestDocumentNodeForObjectId {
                include_whitespace,
                object_id,
            } => {
                self.configure_document_dom_agent_session(inspector_session_id, include_whitespace);
                let (node_id, child_node_snapshot_events) =
                    self.request_document_node_for_object_id(inspector_session_id, &object_id)?;
                Ok(RendererPageReply::DocumentNodeRequested {
                    node_id,
                    child_node_snapshot_events,
                })
            }
            RendererInspectorPageCommand::AccessibilityTreePayloadsForObjectId { object_id } => {
                Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalAccessibilityPayloadsForObjectId(
                    self.accessibility_tree_payloads_for_object_id(
                        inspector_session_id,
                        &object_id,
                    )?,
                )))
            }
            RendererInspectorPageCommand::AccessibilityNodeAndAncestorPayloadsForObjectId {
                object_id,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalAccessibilityPayloadsForObjectId(
                self.accessibility_node_and_ancestor_payloads_for_object_id(
                    inspector_session_id,
                    &object_id,
                )?,
            ))),
            RendererInspectorPageCommand::AccessibilityPartialTreePayloadsForObjectId {
                object_id,
                fetch_relatives,
            } => Ok(RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalAccessibilityPayloadsForObjectId(
                self.accessibility_partial_tree_payloads_for_object_id(
                    inspector_session_id,
                    &object_id,
                    fetch_relatives,
                )?,
            ))),
            RendererInspectorPageCommand::OuterHtmlForObjectId {
                object_id,
                include_shadow_dom,
            } => Ok(RendererPageReply::OptionalString(
                self.outer_html_for_object_id(
                    inspector_session_id,
                    &object_id,
                    include_shadow_dom,
                )?,
            )),
            RendererInspectorPageCommand::ResolveRuntimeObjectForBackendNodeId {
                backend_node_id,
                execution_context_id,
                object_group,
            } => Ok(RendererPageReply::RuntimeRemoteObjectResolution(
                self.resolve_runtime_object_for_backend_node_id(
                    inspector_session_id,
                    backend_node_id,
                    execution_context_id,
                    object_group.as_deref(),
                )?,
            )),
            RendererInspectorPageCommand::ResolveBlobObject { object_id } => {
                Ok(RendererPageReply::BlobUuid(
                    self.vm_mut()
                        .blob_uuid_for_runtime_object_id(inspector_session_id, &object_id)?,
                ))
            }
        }
    }
}

pub(in crate::runtime) fn devtools_page_command_action_barrier(
    command: &RendererDevToolsPageCommand,
) -> Option<moli_action_window::ActionBarrier> {
    match command {
        // The underlying operation flushes its own barrier. Wrapping it must
        // not turn a wheel event into an explicit action-window flush.
        RendererDevToolsPageCommand::Native(_) => None,
        // V8 already removed this binding at the Inspector-session boundary.
        // Finishing its metadata must not flush script-bearing Page actions
        // through the suspended owner stack.
        RendererDevToolsPageCommand::Inspector(envelope)
            if envelope.is_runtime_binding_removal_commit() =>
        {
            None
        }
        RendererDevToolsPageCommand::DispatchMouseEventAtPoint { event_name, .. }
            if event_name == "wheel" =>
        {
            None
        }
        RendererDevToolsPageCommand::CaptureScreencastFrame(_) => {
            Some(moli_action_window::ActionBarrier::Screencast)
        }
        _ => Some(moli_action_window::ActionBarrier::Explicit),
    }
}
