use super::page_surface::RendererInspectorPageCommand;
use super::*;

impl PageVm {
    pub(in crate::runtime) async fn dispatch_renderer_page_command_async(
        &mut self,
        command: RendererPageCommand,
    ) -> Result<RendererPageReply> {
        let throttling_started =
            renderer_page_command_uses_cpu_throttling(&command).then(std::time::Instant::now);
        let result = self.dispatch_renderer_page_command(command);
        self.apply_cpu_throttling_delay_after_page_command(throttling_started)
            .await;
        result
    }

    pub(in crate::runtime) fn dispatch_renderer_page_command(
        &mut self,
        command: RendererPageCommand,
    ) -> Result<RendererPageReply> {
        if let Some(barrier) = renderer_page_command_action_barrier(&command) {
            self.flush_page_action_window(barrier)?;
        }
        match command {
            RendererPageCommand::Inspector(command) => {
                self.dispatch_renderer_inspector_command(command)
            }
            RendererPageCommand::EvaluateExpression {
                expression,
                await_promise,
            }
            | RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression,
                await_promise,
            } => self
                .evaluate_expression_with_await(&expression, await_promise)
                .map(RendererRuntimeEvaluationResult::from_protocol_payload)
                .map(RendererPageReply::RuntimeEvaluationResult),
            RendererPageCommand::EvaluateExpressionInExecutionContext {
                execution_context_id,
                expression,
                await_promise,
            }
            | RendererPageCommand::EvaluateExpressionInExecutionContextAndFollowPendingNavigation {
                execution_context_id,
                expression,
                await_promise,
            } => self
                .evaluate_expression_in_execution_context_with_await(
                    execution_context_id,
                    &expression,
                    await_promise,
                )
                .map(RendererRuntimeEvaluationResult::from_protocol_payload)
                .map(RendererPageReply::RuntimeEvaluationResult),
            RendererPageCommand::WaitForSelector {
                ..
            } => Err(anyhow!(
                "wait-for-selector must be routed through the renderer owner continuation"
            )),
            RendererPageCommand::WaitForScriptTruthy {
                ..
            } => Err(anyhow!(
                "wait-for-script-truthy must be routed through the renderer owner continuation"
            )),
            RendererPageCommand::WaitForSubresourceResponse {
                ..
            } => Err(anyhow!(
                "wait-for-subresource-response must be routed through the renderer owner continuation"
            )),
            RendererPageCommand::CompleteChildFrameLifecycleWorkBestEffort { .. } => Err(anyhow!(
                "child-frame lifecycle best-effort observation must be routed through the renderer owner continuation"
            )),
            RendererPageCommand::SetDocumentContent { frame_id, html } => self
                .vm_mut()
                .set_document_content_for_frame(&frame_id, &html)
                .map(RendererPageReply::SetDocumentContentResult),
            RendererPageCommand::NavigateChildFrame { frame_id, url } => self
                .vm_mut()
                .navigate_child_browsing_context_frame_to_url(&frame_id, &url)
                .map(RendererPageReply::Bool),
            RendererPageCommand::NavigateTopLevelSameDocument { url } => self
                .vm_mut()
                .navigate_top_level_same_document_from_browser(&url)
                .map(RendererPageReply::Bool),
            RendererPageCommand::DispatchMouseEventAtPoint {
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
                .map(RendererPageReply::InputDispatchOutcome),
            RendererPageCommand::DispatchMouseEventAtPoint {
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
            } => self
                .dispatch_mouse_event_at_point_with_pointer(
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
                )
                .map(RendererPageReply::InputDispatchOutcome),
            RendererPageCommand::DispatchTouchEvent {
                points,
                event_name,
                activate,
            } => self
                .dispatch_touch_event_at_points(&points, &event_name, activate)
                .map(RendererPageReply::InputDispatchOutcome),
            RendererPageCommand::DispatchDragEventAtPoint {
                x,
                y,
                event_name,
                data,
                modifiers,
            } => self
                .dispatch_drag_event_at_point(x, y, &event_name, data, modifiers)
                .map(RendererPageReply::InputDispatchOutcome),
            RendererPageCommand::ClearActiveDragDataTransfer => {
                self.clear_active_drag_data_transfer()?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::MutateDocumentBackendNodeAttribute {
                backend_node_id,
                mutation,
            } => self
                .mutate_document_backend_node_attribute(backend_node_id, mutation)
                .map(RendererPageReply::DomAttributeMutationOutcome),
            RendererPageCommand::EditDocumentNode {
                inspector_session_id,
                edit,
            } => self
                .edit_document_node(inspector_session_id.as_deref(), edit)
                .map(RendererPageReply::DomEditOutcome),
            RendererPageCommand::FocusDocumentBackendNode { backend_node_id } => self
                .focus_document_backend_node(backend_node_id)
                .map(RendererPageReply::DomFocusOutcome),
            RendererPageCommand::TriggerAutofill(request) => self
                .trigger_autofill(request)
                .map(RendererPageReply::AutofillTriggerOutcome),
            RendererPageCommand::ResetNavigationHistory => self
                .vm_mut()
                .reset_navigation_history()
                .map(RendererPageReply::Bool),
            RendererPageCommand::SetFileInputFilesForBackendNodeId {
                backend_node_id,
                files,
                append,
            } => Ok(RendererPageReply::OptionalBool(
                self.set_file_input_files_for_backend_node_id(backend_node_id, files, append)?,
            )),
            RendererPageCommand::InsertTextIntoActiveControl(text) => self
                .insert_text_into_active_control(&text)
                .map(RendererPageReply::Bool),
            RendererPageCommand::DispatchKeyEvent {
                event_name,
                key,
                code,
                text,
                modifiers,
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
                )
                .map(RendererPageReply::InputDispatchOutcome),
            RendererPageCommand::CreateIsolatedWorld {
                name,
                grant_universal_access,
                frame_id,
                ..
            } => match frame_id {
                Some(frame_id) => self
                    .create_isolated_world_for_frame(&frame_id, &name, grant_universal_access)
                    .map(RendererPageReply::ExecutionContextId),
                None => self
                    .create_isolated_world(&name, grant_universal_access)
                    .map(RendererPageReply::ExecutionContextId),
            },
            RendererPageCommand::CreateIsolatedWorldRuntimeActivity {
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
                .map(RendererPageReply::ExecutionContextId),
            RendererPageCommand::InstallRuntimeBinding {
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
            RendererPageCommand::RemoveRuntimeBinding(name) => {
                self.remove_runtime_binding(&name)?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::RemoveDefaultRuntimeBinding(name) => {
                self.remove_default_runtime_binding(&name)?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::QueueTopLevelHistoryTraversalByDelta(delta) => self
                .vm_mut()
                .queue_top_level_history_traversal_by_delta(delta)
                .map(RendererPageReply::Bool),
            RendererPageCommand::RunPageSurfaceOverrideScript { source } => {
                self.run_page_surface_override_script(&source)?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::AddDocumentStartScriptRuntimeActivity {
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
            RendererPageCommand::RemoveDocumentStartScriptByRegistryKey(registry_key) => {
                self.remove_document_start_script_by_registry_key(&registry_key);
                Ok(RendererPageReply::Unit)
            }
            #[cfg(test)]
            RendererPageCommand::SetStoredDocumentStartScripts(scripts) => {
                self.set_stored_document_start_scripts(&scripts);
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetRuntimeBindingState {
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
            RendererPageCommand::DefaultExecutionContextId => Ok(
                RendererPageReply::OptionalExecutionContextId(self.default_execution_context_id()),
            ),
            RendererPageCommand::DefaultOrInitialExecutionContextId => Ok(
                RendererPageReply::OptionalExecutionContextId(
                    self.default_or_initial_execution_context_id(),
                ),
            ),
            RendererPageCommand::HasIsolatedWorldNamed { name, frame_id } => {
                Ok(RendererPageReply::Bool(match frame_id {
                    Some(frame_id) => self.has_isolated_world_named_for_frame(&frame_id, &name),
                    None => self.has_isolated_world_named(&name),
                }))
            }
            RendererPageCommand::HasIsolatedExecutionContextId(execution_context_id) => {
                Ok(RendererPageReply::Bool(
                    self.has_isolated_execution_context_id(execution_context_id),
                ))
            }
            RendererPageCommand::EnsureIsolatedWorldsAttachedToInspector => {
                self.ensure_isolated_worlds_attached_to_inspector()?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::InspectorExecutionContextIdForIsolatedContext(
                execution_context_id,
            ) => Ok(RendererPageReply::OptionalExecutionContextId(
                self.inspector_execution_context_id_for_isolated_context(execution_context_id),
            )),
            RendererPageCommand::IsolatedExecutionContextIdForInspectorContext(
                execution_context_id,
            ) => Ok(RendererPageReply::OptionalExecutionContextId(
                self.isolated_execution_context_id_for_inspector_context(execution_context_id),
            )),
            RendererPageCommand::RuntimeRealmInventory => Ok(
                RendererPageReply::RuntimeRealmInventory(self.runtime_realm_inventory()),
            ),
            RendererPageCommand::LiveChildDefaultRuntimeRealmInventory => {
                let realms = self.live_child_default_runtime_realm_inventory();
                Ok(RendererPageReply::RuntimeRealmInventory(realms))
            }
            RendererPageCommand::ChildFrameIdForDefaultExecutionContextId(execution_context_id) => {
                Ok(RendererPageReply::OptionalString(
                    self.vm_mut()
                        .child_default_frame_id_for_execution_context_id(execution_context_id),
                ))
            }
            RendererPageCommand::ChildDefaultExecutionContextIdForFrameId(frame_id) => Ok(
                RendererPageReply::OptionalExecutionContextId(
                    self.vm_mut()
                        .child_default_execution_context_id_for_frame_id(&frame_id),
                ),
            ),
            RendererPageCommand::RuntimeConsoleMessagesWithContext => {
                Ok(RendererPageReply::RuntimeConsoleMessageSnapshots(
                    self.vm_mut().snapshot_console_messages_with_context()?,
                ))
            }
            RendererPageCommand::RuntimeHeapUsage => {
                Ok(RendererPageReply::RuntimeHeapUsage(Box::new(
                    self.vm_mut()
                        .renderer_document_isolate_ops()
                        .renderer_document_isolate_heap_usage()?,
                )))
            }
            RendererPageCommand::PerformanceMetricSnapshot => Ok(
                RendererPageReply::PerformanceMetricSnapshot(Box::new(
                    self.vm_mut().performance_metric_snapshot()?,
                )),
            ),
            RendererPageCommand::DomDebuggerConfigureEventListenerBreakpoint {
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
            RendererPageCommand::DomDebuggerConfigureXhrBreakpoint {
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
            RendererPageCommand::DomDebuggerConfigureDomBreakpoint {
                inspector_session_id,
                frontend_node_id,
                breakpoint_type,
                enabled,
            } => Ok(RendererPageReply::DomDebuggerDomBreakpoint(
                self.configure_dom_debugger_dom_breakpoint(
                    inspector_session_id.as_deref(),
                    frontend_node_id,
                    &breakpoint_type,
                    enabled,
                ),
            )),
            RendererPageCommand::RuntimeCollectGarbage => {
                self.vm_mut()
                    .renderer_document_isolate_ops()
                    .collect_renderer_document_isolate_garbage()?;
                Ok(RendererPageReply::Unit)
            }
            #[cfg(test)]
            RendererPageCommand::TakeDocumentLifecycleEvents => Ok(
                RendererPageReply::DocumentLifecycleEvents(
                    self.drain_document_lifecycle_events(),
                ),
            ),
            RendererPageCommand::StopDocumentLifecycle => {
                self.stop_document_lifecycle();
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SearchTextByLines {
                text,
                query,
                case_sensitive,
                is_regex,
            } => Ok(RendererPageReply::ResourceTextSearchOutcome(
                RendererResourceTextSearchOutcome::Matches(self.vm_mut().search_text_by_lines(
                    &text,
                    &query,
                    case_sensitive,
                    is_regex,
                )?),
            )),
            RendererPageCommand::SearchChildFrameResourceByLines {
                frame_id,
                url,
                query,
                case_sensitive,
                is_regex,
            } => Ok(RendererPageReply::ResourceTextSearchOutcome(
                self.vm_mut().search_child_frame_resource_by_lines(
                    &frame_id,
                    &url,
                    &query,
                    case_sensitive,
                    is_regex,
                )?,
            )),
            RendererPageCommand::ComputedStylePropertiesForBackendNodeId { backend_node_id } => {
                Ok(RendererPageReply::ComputedStyleProperties(
                    self.computed_style_properties_for_backend_node_id(backend_node_id)?,
                ))
            }
            RendererPageCommand::SetInlineStyleSheetTextForStyleSheetId {
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
            RendererPageCommand::ScrollBackendNodeIntoViewIfNeeded {
                backend_node_id,
                rect,
            } => self
                .scroll_backend_node_into_view_if_needed(backend_node_id, rect)
                .map(RendererPageReply::ScrollIntoViewResult),
            RendererPageCommand::ClientRectForBackendNodeId { backend_node_id } => Ok(
                RendererPageReply::OptionalDocumentNodeClientRect(
                    self.client_rect_for_backend_node_id(backend_node_id)?,
                ),
            ),
            RendererPageCommand::DocumentGeometryForBackendNodeId { backend_node_id } => Ok(
                RendererPageReply::OptionalDocumentNodeGeometry(
                    self.document_geometry_for_backend_node_id(backend_node_id)?,
                ),
            ),
            RendererPageCommand::DocumentHitTest {
                inspector_session_id,
                x,
                y,
                include_user_agent_shadow_dom,
                ignore_pointer_events_none,
            } => Ok(RendererPageReply::OptionalDocumentHitTest(
                self.document_hit_test(
                    inspector_session_id.as_deref(),
                    x,
                    y,
                    include_user_agent_shadow_dom,
                    ignore_pointer_events_none,
                )?,
            )),
            RendererPageCommand::NodeHasGeometryForBackendNodeId { backend_node_id } => {
                Ok(RendererPageReply::OptionalBool(
                    self.node_has_geometry_for_backend_node_id(backend_node_id)?,
                ))
            }
            RendererPageCommand::RemoveDocumentBackendNodeId { backend_node_id } => self
                .remove_document_backend_node_id(backend_node_id)
                .map(RendererPageReply::Bool),
            RendererPageCommand::DocumentNodeSnapshotForBackendNodeId {
                backend_node_id,
                depth,
                pierce,
            } => Ok(RendererPageReply::OptionalDocumentNodeObjectSnapshot(Box::new(
                self.document_node_snapshot_for_backend_node_id(backend_node_id, depth, pierce)?,
            ))),
            RendererPageCommand::DocumentNodeSnapshotForBackendNodeIdInInspectorSession {
                inspector_session_id,
                include_whitespace,
                backend_node_id,
                depth,
                pierce,
            } => {
                self.configure_document_dom_agent_session(
                    inspector_session_id.as_deref(),
                    include_whitespace,
                );
                Ok(RendererPageReply::OptionalDocumentNodeObjectSnapshot(
                    Box::new(
                        self.document_node_snapshot_for_backend_node_id_in_inspector_session(
                            inspector_session_id.as_deref(),
                            backend_node_id,
                            depth,
                            pierce,
                        )?,
                    ),
                ))
            }
            RendererPageCommand::DocumentNodeSnapshotForDocument {
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
            RendererPageCommand::DiscardDomAgentFrontendBindings {
                inspector_session_id,
            } => {
                self.vm_mut()
                    .clear_dom_debugger_dom_breakpoints_for_session(
                        inspector_session_id.as_deref(),
                    );
                self.discard_document_frontend_bindings(inspector_session_id.as_deref());
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::DomSnapshotCapture {
                top_frame_id,
                options,
            } => Ok(RendererPageReply::OptionalDomSnapshotCapturePayload(
                self.dom_snapshot_capture_payload(&top_frame_id, options),
            )),
            RendererPageCommand::DocumentChildNodeSnapshotEventsForBackendNodeId {
                inspector_session_id,
                include_whitespace,
                backend_node_id,
                depth,
                pierce,
            } => {
                self.configure_document_dom_agent_session(
                    inspector_session_id.as_deref(),
                    include_whitespace,
                );
                Ok(RendererPageReply::OptionalDocumentChildNodeSnapshotEvents(
                    self.document_child_node_snapshot_events_for_backend_node_id(
                        inspector_session_id.as_deref(),
                        backend_node_id,
                        depth,
                        pierce,
                    ),
                ))
            }
            RendererPageCommand::DocumentQuerySelectorForDocument {
                inspector_session_id,
                include_whitespace,
                selector,
                multiple,
            } => {
                self.configure_document_dom_agent_session(
                    inspector_session_id.as_deref(),
                    include_whitespace,
                );
                Ok(RendererPageReply::DocumentQuerySelectorResolution(
                    self.document_query_selector_for_document(
                        inspector_session_id.as_deref(),
                        &selector,
                        multiple,
                    ),
                ))
            }
            RendererPageCommand::DocumentQuerySelectorForChildFrameBackendNodeId {
                inspector_session_id,
                include_whitespace,
                frame_id,
                root_backend_node_id,
                selector,
                multiple,
            } => {
                self.configure_document_dom_agent_session(
                    inspector_session_id.as_deref(),
                    include_whitespace,
                );
                Ok(RendererPageReply::DocumentQuerySelectorResolution(
                    self.document_query_selector_for_child_frame_backend_node_id(
                        inspector_session_id.as_deref(),
                        &frame_id,
                        root_backend_node_id,
                        &selector,
                        multiple,
                    ),
                ))
            }
            RendererPageCommand::DocumentQuerySelectorForBackendNodeId {
                inspector_session_id,
                include_whitespace,
                root_backend_node_id,
                selector,
                multiple,
            } => {
                self.configure_document_dom_agent_session(
                    inspector_session_id.as_deref(),
                    include_whitespace,
                );
                Ok(RendererPageReply::DocumentQuerySelectorResolution(
                    self.document_query_selector_for_backend_node_id(
                        inspector_session_id.as_deref(),
                        root_backend_node_id,
                        &selector,
                        multiple,
                    ),
                ))
            }
            RendererPageCommand::DocumentQuerySelectorWithChildNodeSnapshotEventsForBackendNodeId {
                inspector_session_id,
                include_whitespace,
                root_backend_node_id,
                selector,
                multiple,
            } => {
                self.configure_document_dom_agent_session(
                    inspector_session_id.as_deref(),
                    include_whitespace,
                );
                Ok(RendererPageReply::DocumentQuerySelectorWithChildNodeSnapshotEvents(
                    self.document_query_selector_with_child_node_snapshot_events_for_backend_node_id(
                        inspector_session_id.as_deref(),
                        root_backend_node_id,
                        &selector,
                        multiple,
                    ),
                ))
            }
            RendererPageCommand::DocumentPerformSearch {
                inspector_session_id,
                query,
                include_user_agent_shadow_dom,
                include_whitespace,
            } => {
                self.configure_document_dom_agent_session(
                    inspector_session_id.as_deref(),
                    include_whitespace,
                );
                Ok(RendererPageReply::DocumentPerformSearch(
                    self.document_perform_search(
                        inspector_session_id.as_deref(),
                        &query,
                        include_user_agent_shadow_dom,
                    ),
                ))
            }
            RendererPageCommand::DocumentGetSearchResults {
                inspector_session_id,
                search_id,
                from_index,
                to_index,
            } => Ok(RendererPageReply::DocumentSearchResults(
                self.document_search_results(
                    inspector_session_id.as_deref(),
                    &search_id,
                    from_index,
                    to_index,
                ),
            )),
            RendererPageCommand::DocumentDiscardSearchResults {
                inspector_session_id,
                search_id,
            } => {
                self.discard_document_search_results(inspector_session_id.as_deref(), &search_id);
                Ok(RendererPageReply::DocumentSearchResultsDiscarded)
            }
            RendererPageCommand::DocumentSetNodeStackTracesEnabled {
                inspector_session_id,
                enabled,
            } => {
                self.set_document_node_stack_traces_enabled(
                    inspector_session_id.as_deref(),
                    enabled,
                );
                Ok(RendererPageReply::DocumentNodeStackTracesEnabled)
            }
            RendererPageCommand::DocumentNodeStackTrace {
                inspector_session_id,
                frontend_node_id,
            } => Ok(RendererPageReply::DocumentNodeStackTrace(
                self.document_node_stack_trace(
                    inspector_session_id.as_deref(),
                    frontend_node_id,
                ),
            )),
            RendererPageCommand::DocumentFrontendNodeBinding {
                inspector_session_id,
                frontend_node_id,
            } => Ok(RendererPageReply::DocumentFrontendNodeBinding(
                self.document_frontend_node_binding(
                    inspector_session_id.as_deref(),
                    frontend_node_id,
                ),
            )),
            RendererPageCommand::RegisterDocumentBidiNodeBinding {
                inspector_session_id,
                shared_id,
                backend_node_id,
            } => {
                self.register_document_bidi_node_binding(
                    inspector_session_id.as_deref(),
                    shared_id,
                    backend_node_id,
                );
                Ok(RendererPageReply::DocumentBidiNodeBindingRegistered)
            }
            RendererPageCommand::DocumentBidiNodeBinding {
                inspector_session_id,
                shared_id,
            } => Ok(RendererPageReply::DocumentBidiNodeBinding(
                self.document_bidi_node_binding(inspector_session_id.as_deref(), &shared_id),
            )),
            RendererPageCommand::DocumentBidiNodeSharedIdForBackendNodeId {
                inspector_session_id,
                backend_node_id,
            } => Ok(RendererPageReply::DocumentBidiNodeSharedId(
                self.document_bidi_node_shared_id_for_backend_node_id(
                    inspector_session_id.as_deref(),
                    backend_node_id,
                ),
            )),
            RendererPageCommand::DocumentNodeAttributesForBackendNodeId { backend_node_id } => {
                Ok(RendererPageReply::DocumentNodeAttributesResolution(
                    self.document_node_attributes_for_backend_node_id(backend_node_id),
                ))
            }
            RendererPageCommand::DocumentNodeTextForBackendNodeId { backend_node_id } => {
                Ok(RendererPageReply::DocumentNodeTextResolution(
                    self.document_node_text_for_backend_node_id(backend_node_id),
                ))
            }
            RendererPageCommand::DocumentNodePropertyForBackendNodeId {
                backend_node_id,
                name,
            } => Ok(RendererPageReply::DocumentNodePropertyResolution(
                self.document_node_property_for_backend_node_id(backend_node_id, &name),
            )),
            RendererPageCommand::AccessibilityTreePayloadsForDocument { max_depth } => Ok(
                RendererPageReply::OptionalAccessibilityPayloads(
                    self.accessibility_tree_payloads_for_document(max_depth),
                ),
            ),
            RendererPageCommand::AccessibilityNodePayloadForDocument => Ok(
                RendererPageReply::OptionalAccessibilityPayload(
                    self.accessibility_node_payload_for_document(),
                ),
            ),
            RendererPageCommand::AccessibilityTreePayloadsForBackendNodeId {
                backend_node_id,
                max_depth,
            } => Ok(RendererPageReply::OptionalAccessibilityPayloadsForObjectId(
                self.accessibility_tree_payloads_for_backend_node_id(backend_node_id, max_depth),
            )),
            RendererPageCommand::AccessibilityNodePayloadForBackendNodeId { backend_node_id } => {
                Ok(RendererPageReply::OptionalAccessibilityPayloadsForObjectId(
                    self.accessibility_node_payload_for_backend_node_id(backend_node_id),
                ))
            }
            RendererPageCommand::AccessibilityNodeAndAncestorPayloadsForBackendNodeId {
                backend_node_id,
            } => Ok(RendererPageReply::OptionalAccessibilityPayloadsForObjectId(
                self.accessibility_node_and_ancestor_payloads_for_backend_node_id(backend_node_id),
            )),
            RendererPageCommand::AccessibilityChildNodePayloadsForBackendNodeId {
                backend_node_id,
            } => Ok(RendererPageReply::OptionalAccessibilityPayloadsForObjectId(
                self.accessibility_child_node_payloads_for_backend_node_id(backend_node_id),
            )),
            RendererPageCommand::AccessibilityPartialTreePayloadsForBackendNodeId {
                backend_node_id,
                fetch_relatives,
            } => Ok(RendererPageReply::OptionalAccessibilityPayloadsForObjectId(
                self.accessibility_partial_tree_payloads_for_backend_node_id(
                    backend_node_id,
                    fetch_relatives,
                ),
            )),
            RendererPageCommand::AccessibilityTreePayloadsForChildFrame {
                frame_id,
                max_depth,
            } => Ok(RendererPageReply::OptionalAccessibilityPayloadsForObjectId(
                self.accessibility_tree_payloads_for_child_frame(&frame_id, max_depth),
            )),
            RendererPageCommand::AccessibilityNodePayloadForChildFrame { frame_id } => {
                Ok(RendererPageReply::OptionalAccessibilityPayloadsForObjectId(
                    self.accessibility_node_payload_for_child_frame(&frame_id),
                ))
            }
            RendererPageCommand::StyleSheetPayloadForStyleSheetId {
                inspector_session_id,
                style_sheet_id,
            } => Ok(RendererPageReply::OptionalStyleSheetPayload(
                self.style_sheet_payload_for_style_sheet_id(
                    inspector_session_id.as_deref(),
                    &style_sheet_id,
                ),
            )),
            RendererPageCommand::StyleSheetInventoryForDocument {
                inspector_session_id,
            } => Ok(RendererPageReply::StyleSheetInventory(
                self.style_sheet_inventory_for_document(inspector_session_id.as_deref()),
            )),
            RendererPageCommand::ResetCssAgentSession {
                inspector_session_id,
            } => {
                self.reset_css_agent_session(inspector_session_id.as_deref());
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::OuterHtmlForDocument { include_shadow_dom } => {
                Ok(RendererPageReply::OptionalString(
                    self.outer_html_for_document(include_shadow_dom),
                ))
            }
            RendererPageCommand::OuterHtmlForBackendNodeId {
                backend_node_id,
                include_shadow_dom,
            } => Ok(RendererPageReply::OptionalString(
                self.outer_html_for_backend_node_id(backend_node_id, include_shadow_dom)?,
            )),
            RendererPageCommand::RenderPageDump { options } => Ok(RendererPageReply::OptionalString(
                Some(self.render_page_dump(options)),
            )),
            RendererPageCommand::SerializeHtml => {
                Ok(RendererPageReply::OptionalString(Some(self.serialize_html())))
            }
            RendererPageCommand::LayoutMetrics => self
                .layout_metrics()
                .map(RendererPageReply::LayoutMetrics),
            RendererPageCommand::CaptureScreenshot(request) => self
                .capture_screenshot(request)
                .map(RendererPageReply::CaptureScreenshot),
            RendererPageCommand::BlobBytesForUuid { uuid } => Ok(
                RendererPageReply::OptionalBlobBytes(self.vm().blob_bytes_for_uuid(&uuid)),
            ),
            RendererPageCommand::DocumentFrontendNodeIdsForBackendNodeIds {
                inspector_session_id,
                backend_node_ids,
            } => Ok(RendererPageReply::DocumentFrontendNodeIds(
                self.document_frontend_node_ids_for_backend_node_ids(
                    inspector_session_id.as_deref(),
                    &backend_node_ids,
                ),
            )),
            RendererPageCommand::DocumentStorageKeySnapshot => Ok(RendererPageReply::DocumentStorageKey(
                self.vm_mut().top_document_storage_key_snapshot(),
            )),
            RendererPageCommand::ChildFrameTreeSnapshot => {
                Ok(RendererPageReply::ChildFrameTreeSnapshots(
                    self.vm_mut()
                        .child_browsing_context_frame_tree_snapshot_for_protocol(),
                ))
            }
            RendererPageCommand::ChildFrameOwnerNodeReference {
                inspector_session_id,
                frame_id,
            } => Ok(RendererPageReply::OptionalDocumentNodeReference(
                self.child_frame_owner_node_reference_by_frame_id(
                    inspector_session_id.as_deref(),
                    &frame_id,
                ),
            )),
            RendererPageCommand::ChildFrameDocumentRootNodeReference {
                inspector_session_id,
                frame_id,
            } => Ok(RendererPageReply::OptionalDocumentNodeReference(
                self.child_frame_document_root_node_reference_by_frame_id(
                    inspector_session_id.as_deref(),
                    &frame_id,
                ),
            )),
            RendererPageCommand::ContinuePendingSubresourceFetch {
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
            RendererPageCommand::ContinuePendingSubresourceAuth { internal_id, auth } => self
                .continue_pending_subresource_auth(internal_id, auth)
                .map(RendererPageReply::PendingSubresourceContinueOutcome),
            RendererPageCommand::CancelPendingSubresourceAuth { internal_id } => {
                self.cancel_pending_subresource_auth(internal_id)?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::FailPendingSubresourceAuth {
                internal_id,
                error_text,
            } => {
                self.fail_pending_subresource_auth(internal_id, error_text)?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::FailPendingSubresourceFetch {
                internal_id,
                error_text,
            } => {
                self.fail_pending_subresource_fetch(internal_id, error_text)?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::FulfillPendingSubresourceFetch {
                internal_id,
                response_code,
                response_headers,
                response_body,
            } => {
                self.fulfill_pending_subresource_fetch(
                    internal_id,
                    response_code,
                    response_headers,
                    response_body,
                )?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::ContinuePendingSubresourceResponse {
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
            RendererPageCommand::FailPendingSubresourceResponse {
                internal_id,
                error_text,
            } => {
                self.fail_pending_subresource_response(internal_id, error_text)?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::FulfillPendingSubresourceResponse {
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
            RendererPageCommand::ReceiveSyntheticWebSocketText { socket_id, data } => {
                if self.receive_synthetic_websocket_text(socket_id, data) {
                    Ok(RendererPageReply::Unit)
                } else {
                    anyhow::bail!("unknown synthetic WebSocket `{socket_id}`")
                }
            }
            RendererPageCommand::ReceiveSyntheticWebSocketBinary { socket_id, data } => {
                if self.receive_synthetic_websocket_binary(socket_id, data) {
                    Ok(RendererPageReply::Unit)
                } else {
                    anyhow::bail!("unknown synthetic WebSocket `{socket_id}`")
                }
            }
            RendererPageCommand::CloseSyntheticWebSocketFromServer {
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
            RendererPageCommand::PendingSubresourceRequestCount => {
                Ok(RendererPageReply::Usize(self.pending_subresource_request_count()))
            }
            RendererPageCommand::SetFetchSubresourceInterception {
                enabled,
                resource_type,
            } => {
                self.set_fetch_subresource_interception(enabled, resource_type);
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetJavaScriptDialogHandlerEnabled(enabled) => {
                self.vm_mut()
                    .set_javascript_dialog_handler_enabled(enabled);
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::ReplaceBrowserResourceRuntime(resource_runtime) => {
                self.replace_browser_resource_runtime(&resource_runtime);
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::RetireDocumentResourceAuthorities => {
                self.retire_document_resource_authorities();
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::ApplyDocumentCookieFacadeOverrides(overrides) => {
                self.apply_document_cookie_facade_overrides(&overrides);
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::ClearDocumentCookieFacadeOverrides => {
                self.clear_document_cookie_facade_overrides();
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::DocumentCookieTelemetrySnapshot => {
                Ok(RendererPageReply::CookieFacadeSnapshot(Box::new(
                    RendererPageCookieFacadeSnapshotReply::Telemetry(
                        self.document_cookie_telemetry_snapshot(),
                    ),
                )))
            }
            RendererPageCommand::DocumentCookieOwnerSnapshot => {
                Ok(RendererPageReply::CookieFacadeSnapshot(Box::new(
                    RendererPageCookieFacadeSnapshotReply::Owner(Box::new(
                        self.document_cookie_owner_snapshot(),
                    )),
                )))
            }
            RendererPageCommand::PrepareNetworkResourceLoad {
                frame_id,
                url,
                disable_cache,
                include_credentials,
            } => Ok(RendererPageReply::NetworkResourceLoadPreparation(
                self.vm().prepare_devtools_network_resource_load(
                    &frame_id,
                    url,
                    disable_cache,
                    include_credentials,
                ),
            )),
            RendererPageCommand::PrepareAppManifestLoad => Ok(
                RendererPageReply::AppManifestLoadPreparation(
                    self.vm_mut().prepare_app_manifest_load(),
                ),
            ),
            RendererPageCommand::PublishAppManifestLoad(publication) => {
                self.vm_mut().publish_app_manifest_load(*publication);
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetExtraHttpHeaders(headers) => {
                self.set_extra_http_headers(&headers);
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetPermissionOverrides(overrides) => {
                self.set_permission_overrides(&overrides);
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetIdleOverride(idle_override) => {
                self.set_idle_override(idle_override)?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetLocaleOverride(locale) => {
                self.set_locale_override(locale.as_deref())?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetTimezoneOverride(timezone) => {
                self.set_timezone_override(timezone.as_deref())?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetScriptExecutionDisabled(disabled) => {
                self.set_script_execution_disabled(disabled);
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetBypassContentSecurityPolicy(bypass) => {
                self.set_bypass_content_security_policy(bypass);
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetCpuThrottlingRate(rate) => {
                self.set_cpu_throttling_rate(rate);
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetEmulatedMedia(overrides) => {
                self.set_emulated_media(&overrides);
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetViewportSurface(viewport_surface) => {
                self.set_viewport_surface(viewport_surface)?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetNetworkOffline(offline) => {
                self.set_network_offline(offline);
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetBypassServiceWorker(bypass) => {
                self.set_bypass_service_worker(bypass);
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetBlockedUrlPatterns(patterns) => {
                self.set_blocked_url_patterns(&patterns);
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::MsToNextTimeout => Ok(RendererPageReply::OptionalU64(
                self.vm().ms_to_next_timeout(),
            )),
            RendererPageCommand::RefreshFullPageState => Ok(RendererPageReply::Unit),
            RendererPageCommand::PageDiagnosticsSnapshot => {
                Ok(RendererPageReply::PageDiagnosticsSnapshot(
                    self.page_diagnostics_snapshot()?,
                ))
            }
            RendererPageCommand::HasPendingLocationNavigation => {
                Ok(RendererPageReply::Bool(self.vm().has_pending_location_navigation()))
            }
            #[cfg(debug_assertions)]
            RendererPageCommand::PanicForTesting => {
                panic!("renderer page command panicked for testing")
            }
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
                .dispatch_runtime_protocol_message_for_inspector_session_with_deferred_response(
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
                .dispatch_runtime_protocol_message_for_inspector_session_with_context_resolution_and_deferred_response(
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
            } => Ok(RendererPageReply::DomDebuggerEventListeners(
                self.vm_mut().dom_debugger_event_listeners(
                    inspector_session_id,
                    &object_id,
                    depth,
                    pierce,
                )?,
            )),
            RendererInspectorPageCommand::ComputedStylePropertiesForObjectId { object_id } => {
                Ok(RendererPageReply::ComputedStyleProperties(
                    self.computed_style_properties_for_object_id(
                        inspector_session_id,
                        &object_id,
                    )?,
                ))
            }
            RendererInspectorPageCommand::ScrollObjectNodeIntoViewIfNeeded { object_id, rect } => {
                self.scroll_node_into_view_if_needed_for_object_id(
                    inspector_session_id,
                    &object_id,
                    rect,
                )
                .map(RendererPageReply::ScrollIntoViewResult)
            }
            RendererInspectorPageCommand::ClientRectForObjectId { object_id } => {
                Ok(RendererPageReply::OptionalDocumentNodeClientRect(
                    self.client_rect_for_object_id(inspector_session_id, &object_id)?,
                ))
            }
            RendererInspectorPageCommand::DocumentGeometryForObjectId { object_id } => {
                Ok(RendererPageReply::OptionalDocumentNodeGeometry(
                    self.document_geometry_for_object_id(inspector_session_id, &object_id)?,
                ))
            }
            RendererInspectorPageCommand::NodeHasGeometryForObjectId { object_id } => {
                Ok(RendererPageReply::OptionalBool(
                    self.node_has_geometry_for_object_id(inspector_session_id, &object_id)?,
                ))
            }
            RendererInspectorPageCommand::FocusDocumentNodeForObjectId { object_id } => self
                .focus_document_node_for_object_id(inspector_session_id, &object_id)
                .map(RendererPageReply::DomFocusOutcome),
            RendererInspectorPageCommand::SetFileInputFilesForObjectId {
                object_id,
                files,
                append,
            } => Ok(RendererPageReply::OptionalBool(
                self.set_file_input_files_for_object_id(
                    inspector_session_id,
                    &object_id,
                    files,
                    append,
                )?,
            )),
            RendererInspectorPageCommand::DocumentNodeSnapshotForObjectId {
                include_whitespace,
                object_id,
                depth,
                pierce,
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
                    )?),
                ))
            }
            RendererInspectorPageCommand::AccessibilityTreePayloadsForObjectId { object_id } => {
                Ok(RendererPageReply::OptionalAccessibilityPayloadsForObjectId(
                    self.accessibility_tree_payloads_for_object_id(
                        inspector_session_id,
                        &object_id,
                    )?,
                ))
            }
            RendererInspectorPageCommand::AccessibilityNodeAndAncestorPayloadsForObjectId {
                object_id,
            } => Ok(RendererPageReply::OptionalAccessibilityPayloadsForObjectId(
                self.accessibility_node_and_ancestor_payloads_for_object_id(
                    inspector_session_id,
                    &object_id,
                )?,
            )),
            RendererInspectorPageCommand::AccessibilityPartialTreePayloadsForObjectId {
                object_id,
                fetch_relatives,
            } => Ok(RendererPageReply::OptionalAccessibilityPayloadsForObjectId(
                self.accessibility_partial_tree_payloads_for_object_id(
                    inspector_session_id,
                    &object_id,
                    fetch_relatives,
                )?,
            )),
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

    async fn apply_cpu_throttling_delay_after_page_command(
        &self,
        started: Option<std::time::Instant>,
    ) {
        let Some(started) = started else {
            return;
        };
        let rate = self.cpu_throttling_rate;
        if !rate.is_finite() || rate <= 1.0 {
            return;
        }
        let elapsed = started.elapsed();
        if elapsed.is_zero() {
            return;
        }
        let delay_secs = elapsed.as_secs_f64() * (rate - 1.0);
        if !delay_secs.is_finite() || delay_secs <= 0.0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs_f64(
            delay_secs.min(std::time::Duration::MAX.as_secs_f64()),
        ))
        .await;
    }
}

fn renderer_page_command_action_barrier(
    command: &RendererPageCommand,
) -> Option<moli_action_window::ActionBarrier> {
    match command {
        RendererPageCommand::DispatchMouseEventAtPoint { event_name, .. }
            if event_name == "wheel" =>
        {
            None
        }
        RendererPageCommand::CaptureScreenshot(request) => Some(match request.purpose {
            RendererScreenshotPurpose::Screenshot => moli_action_window::ActionBarrier::Screenshot,
            RendererScreenshotPurpose::Screencast => moli_action_window::ActionBarrier::Screencast,
            RendererScreenshotPurpose::Print { .. } => moli_action_window::ActionBarrier::Explicit,
        }),
        _ => Some(moli_action_window::ActionBarrier::Explicit),
    }
}

fn renderer_page_command_uses_cpu_throttling(command: &RendererPageCommand) -> bool {
    if let RendererPageCommand::Inspector(envelope) = command {
        return envelope.uses_cpu_throttling();
    }
    matches!(
        command,
        RendererPageCommand::EvaluateExpression { .. }
            | RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation { .. }
            | RendererPageCommand::EvaluateExpressionInExecutionContext { .. }
            | RendererPageCommand::EvaluateExpressionInExecutionContextAndFollowPendingNavigation { .. }
            | RendererPageCommand::DispatchMouseEventAtPoint { .. }
            | RendererPageCommand::DispatchTouchEvent { .. }
            | RendererPageCommand::DispatchDragEventAtPoint { .. }
            | RendererPageCommand::InsertTextIntoActiveControl(_)
            | RendererPageCommand::DispatchKeyEvent { .. }
            | RendererPageCommand::DomDebuggerConfigureEventListenerBreakpoint { .. }
            | RendererPageCommand::DomDebuggerConfigureXhrBreakpoint { .. }
            | RendererPageCommand::DomDebuggerConfigureDomBreakpoint { .. }
            | RendererPageCommand::PerformanceMetricSnapshot
            | RendererPageCommand::CreateIsolatedWorldRuntimeActivity { .. }
            | RendererPageCommand::AddDocumentStartScriptRuntimeActivity { .. }
            | RendererPageCommand::RunPageSurfaceOverrideScript { .. }
            | RendererPageCommand::MutateDocumentBackendNodeAttribute { .. }
            | RendererPageCommand::EditDocumentNode { .. }
            | RendererPageCommand::FocusDocumentBackendNode { .. }
            | RendererPageCommand::SetDocumentContent { .. }
    )
}
