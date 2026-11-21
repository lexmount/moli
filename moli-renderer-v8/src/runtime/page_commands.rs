#[cfg(feature = "devtools")]
use super::page_devtools::devtools_page_command_action_barrier;
use super::*;

#[derive(Debug, thiserror::Error)]
pub(super) enum DocumentCommandError {
    #[error("Document command belongs to another renderer owner")]
    WrongOwner,
    #[error("Document command belongs to another renderer Page")]
    WrongPage,
    #[cfg(debug_assertions)]
    #[error("Renderer owner is unavailable")]
    OwnerUnavailable,
}

impl PageVm {
    pub(in crate::runtime) fn dispatch_renderer_page_command(
        &mut self,
        command: RendererPageCommand,
    ) -> Result<RendererPageReply> {
        let (source_document, command) = match command {
            RendererPageCommand::ForDocument {
                source_document,
                command,
            } => (Some(source_document), *command),
            command => (None, command),
        };
        if source_document.is_some_and(|source| self.document_lifecycle.identity() != source) {
            return Err(RendererPageAccessError::DocumentReplaced.into());
        }
        if let Some(barrier) = renderer_page_command_action_barrier(&command) {
            self.flush_page_action_window(barrier)?;
        }
        // Flushing queued input can run script, including document.open().
        // Recheck immediately before touching a Document's native request IDs.
        if source_document.is_some_and(|source| self.document_lifecycle.identity() != source) {
            return Err(RendererPageAccessError::DocumentReplaced.into());
        }
        match command {
            command @ RendererPageCommand::ForDocument { .. } => self.dispatch_renderer_page_command(command),
            #[cfg(feature = "devtools")]
            RendererPageCommand::DevTools(command) => self.dispatch_devtools_command(command),
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
            RendererPageCommand::EvaluateExpressionByValue { .. } => Err(anyhow!(
                "by-value runtime evaluation must be routed through the renderer owner continuation"
            )),
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
            RendererPageCommand::NavigateTopLevelSameDocument { url, replace_current } => self
                .vm_mut()
                .navigate_top_level_same_document_from_browser(&url, replace_current)
                .map(RendererPageReply::Bool),
            RendererPageCommand::ResetNavigationHistory => self
                .vm_mut()
                .reset_navigation_history()
                .map(RendererPageReply::Bool),
            RendererPageCommand::QueueTopLevelHistoryTraversalByDelta(delta) => self
                .vm_mut()
                .queue_top_level_history_traversal_by_delta(delta)
                .map(RendererPageReply::Bool),
            RendererPageCommand::RunPageSurfaceOverrideScript { source } => {
                self.run_page_surface_override_script(&source)?;
                Ok(RendererPageReply::Unit)
            }
            #[cfg(test)]
            RendererPageCommand::SetStoredDocumentStartScripts(scripts) => {
                self.set_stored_document_start_scripts(&scripts);
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::DefaultExecutionContextId => {
                self.vm_mut().ensure_main_world()?;
                Ok(RendererPageReply::OptionalExecutionContextId(self.default_execution_context_id()))
            }
            RendererPageCommand::DefaultOrInitialExecutionContextId => {
                self.vm_mut().ensure_main_world()?;
                Ok(RendererPageReply::OptionalExecutionContextId(self.default_or_initial_execution_context_id()))
            }
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
            RendererPageCommand::RuntimeRealmInventory => {
                self.vm_mut().ensure_main_world()?;
                Ok(RendererPageReply::RuntimeRealmInventory(self.runtime_realm_inventory()))
            }
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
            #[cfg(any(test, feature = "devtools"))]
            RendererPageCommand::RuntimeHeapUsage => {
                Ok(RendererPageReply::RuntimeHeapUsage(Box::new(
                    self.vm_mut()
                        .renderer_document_isolate_ops()
                        .renderer_document_isolate_heap_usage()?,
                )))
            }
            #[cfg(test)]
            RendererPageCommand::TakeDocumentLifecycleEvents => Ok(
                RendererPageReply::DocumentLifecycleEvents(
                    self.drain_document_lifecycle_events(),
                ),
            ),
            RendererPageCommand::StopDocumentLifecycle => {
                self.stop_document_lifecycle()?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::AccessibilityTreeNodes { frame_id } => Ok(
                RendererPageReply::OptionalAccessibilityNodes(
                    self.accessibility_tree_nodes(frame_id.as_deref()),
                ),
            ),
            RendererPageCommand::OuterHtmlForDocument { include_shadow_dom } => {
                Ok(RendererPageReply::OptionalString(
                    self.outer_html_for_document(include_shadow_dom),
                ))
            }
            RendererPageCommand::OuterHtmlForNode {
                reference,
                include_shadow_dom,
            } => Ok(RendererPageReply::OptionalString(
                self.outer_html_for_node(reference, include_shadow_dom)?,
            )),
            RendererPageCommand::RenderMarkdown { options } => Ok(RendererPageReply::OptionalString(
                Some(self.render_markdown(options)),
            )),
            RendererPageCommand::SerializeDocument { options } => {
                // Capture both from the live document in this handler turn.
                // Browser admission metadata may precede a queued history update.
                Ok(RendererPageReply::SerializedDocument {
                    url: self.vm().document_runtime.host_document().url().to_string(),
                    html: self.serialize_html(options)?,
                })
            }
            RendererPageCommand::LayoutMetrics => {
                Ok(RendererPageReply::LayoutMetrics(self.layout_metrics()?))
            }
            RendererPageCommand::PublishLayout => {
                self.vm_mut().publish_layout()?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::PrintToPdf(request) => self
                .print_to_pdf(request)
                .map(RendererPageReply::PrintToPdf),
            RendererPageCommand::CaptureScreenshot(request) => self
                .capture_screenshot(request)
                .map(RendererPageReply::CaptureScreenshot),
            RendererPageCommand::BlobBytesForUuid { uuid } => Ok(
                RendererPageReply::OptionalBlobBytes(self.vm().blob_bytes_for_uuid(&uuid)),
            ),
            RendererPageCommand::DocumentStorageKeySnapshot => Ok(RendererPageReply::DocumentStorageKey(
                self.vm_mut().top_document_storage_key_snapshot(),
            )),
            RendererPageCommand::ChildFrameTreeSnapshot => {
                Ok(RendererPageReply::ChildFrameTreeSnapshots(
                    self.vm_mut()
                        .child_browsing_context_frame_tree_snapshot_for_protocol(),
                ))
            }
            RendererPageCommand::PendingSubresourceRequestCount => {
                Ok(RendererPageReply::Usize(self.pending_subresource_request_count()))
            }
            RendererPageCommand::ReplaceBrowserResourceRuntime {
                resource_runtime,
                navigator_identity,
            } => {
                self.replace_browser_resource_runtime(&resource_runtime, &navigator_identity);
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
            RendererPageCommand::SetNetworkRequestPolicy {
                extra_http_headers,
                bypass_service_worker,
                cache_disabled,
                blocked_url_patterns,
            } => {
                self.set_extra_http_headers(&extra_http_headers);
                self.set_bypass_service_worker(bypass_service_worker);
                self.set_cache_disabled(cache_disabled);
                self.set_blocked_url_patterns(&blocked_url_patterns);
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
            RendererPageCommand::SetNavigatorOverrides(overrides) => {
                self.set_navigator_overrides(&overrides)?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetNavigatorAndDocumentActivity {
                navigator_overrides,
                document_activity,
            } => {
                // Preserve navigator-before-activity ordering for this policy;
                // the caller needs one completion for the entire update.
                self.set_navigator_overrides(&navigator_overrides)?;
                self.set_document_activity(document_activity)?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetDocumentActivity(activity) => {
                self.set_document_activity(activity)?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetScriptExecutionDisabled(disabled) => {
                self.set_script_execution_disabled(disabled)?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetScrollbarsHidden(hidden) => {
                self.set_scrollbars_hidden(hidden)?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetBypassContentSecurityPolicy(bypass) => {
                self.set_bypass_content_security_policy(bypass)?;
                Ok(RendererPageReply::Unit)
            }
            RendererPageCommand::SetEmulatedMedia(overrides) => {
                self.set_emulated_media(&overrides)?;
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
            #[cfg(feature = "ace-protocol")]
            RendererPageCommand::AcePageTask(task) => self.execute_ace_page_task(task),
            #[cfg(debug_assertions)]
            RendererPageCommand::PanicForTesting => {
                panic!("renderer page command panicked for testing")
            }
            #[cfg(test)]
            RendererPageCommand::ObserveRealtimeOpportunityForTesting { expression, completed } => {
                self.vm().context_host_weak_for_test().upgrade().expect("live Host").borrow().observe_next_realtime_opportunity_for_test(completed);
                self.vm_mut().eval(&expression)?;
                Ok(RendererPageReply::Unit)
            }
        }
    }
}

fn renderer_page_command_action_barrier(
    command: &RendererPageCommand,
) -> Option<moli_action_window::ActionBarrier> {
    match command {
        #[cfg(feature = "devtools")]
        RendererPageCommand::DevTools(command) => devtools_page_command_action_barrier(command),
        RendererPageCommand::CaptureScreenshot(_) => {
            Some(moli_action_window::ActionBarrier::Screenshot)
        }
        _ => Some(moli_action_window::ActionBarrier::Explicit),
    }
}
