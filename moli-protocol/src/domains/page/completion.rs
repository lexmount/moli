use super::*;

#[derive(Debug, thiserror::Error)]
pub(crate) enum PageCommandCompletionError {
    #[error("run document-start script page command returned an unexpected renderer reply")]
    UnexpectedPreloadReply,
    #[error("stop document lifecycle page command returned an unexpected renderer reply")]
    UnexpectedStopLifecycleReply,
}

pub(crate) fn complete_finite(
    conn: &mut CdpConnection,
    completed: CompletedPageCommandDispatch,
    command_context: &mut CommandDispatchContext,
) -> PageCommandTaskStep {
    let command_id = completed.command_id;
    let owner_scope = completed.owner_scope;
    if let Some(predecessor) = completed.kind.renderer_output_predecessor() {
        command_context.set_renderer_output_predecessor(predecessor);
    }
    let plan = match *completed.kind {
        CompletedPageCommandKind::AppendDefaultDocumentStartScript {
            identifier,
            completed,
        } => {
            let completion = match *completed {
                Ok(completion) => completion,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000, message,
                    ));
                }
            };
            let completed_script = {
                let Some(page) = conn.loaded_page_mut_for_owner(&owner_scope) else {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        crate::conn::NoDocumentLoaded,
                    ));
                };
                page.finish_document_start_script_result_command_turn(completion)
            };
            let (_, output) = match completed_script {
                Ok(completed_script) => completed_script,
                Err(error) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        error.to_string(),
                    ));
                }
            };
            command_context.consume_renderer_command_turn_output(output);
            preload::add_preload_script_result_plan(identifier)
        }
        CompletedPageCommandKind::RemoveDocumentStartScript { completed } => {
            let completion = match *completed {
                Ok(completion) => completion,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000, message,
                    ));
                }
            };
            let Some(page) = conn.loaded_page_mut_for_owner(&owner_scope) else {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    crate::conn::NoDocumentLoaded,
                ));
            };
            if let Err(error) =
                page.finish_unit_runtime_page_command(completion, "remove document-start script")
            {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    error.to_string(),
                ));
            }
            CommandOutputPlan::success()
        }
        CompletedPageCommandKind::SearchInResource(completed) => {
            return resource_search::complete_search_in_resource_command(
                conn,
                command_id,
                &owner_scope,
                *completed,
            );
        }
        CompletedPageCommandKind::GetAppManifest(completed) => {
            return app_manifest::complete_get_app_manifest_command(
                conn,
                command_id,
                &owner_scope,
                *completed,
                command_context,
            );
        }
        CompletedPageCommandKind::ResetNavigationHistory { completed } => {
            return navigation::complete_reset_navigation_history_command(
                conn,
                &owner_scope,
                *completed,
            );
        }
        CompletedPageCommandKind::SetBypassContentSecurityPolicy { completed } => {
            let completion = match *completed {
                Ok(completion) => completion,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000, message,
                    ));
                }
            };
            let Some(page) = conn.loaded_page_mut_for_owner(&owner_scope) else {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    crate::conn::NoDocumentLoaded,
                ));
            };
            if let Err(error) = page.finish_set_bypass_content_security_policy(completion) {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    error.to_string(),
                ));
            }
            CommandOutputPlan::success()
        }
        CompletedPageCommandKind::SetDocumentContent { completed } => {
            let completion = match *completed {
                Ok(completion) => completion,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000, message,
                    ));
                }
            };
            let (result, output) = {
                let Some(page) = conn.loaded_page_mut_for_owner(&owner_scope) else {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        "No Document instance to set HTML for",
                    ));
                };
                match page.finish_set_document_content_command_turn(completion) {
                    Ok(completed) => completed,
                    Err(error) => {
                        return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                            -32000,
                            error.to_string(),
                        ));
                    }
                }
            };
            command_context.consume_renderer_command_turn_output(output);
            let mut plan = CommandOutputPlan::default();
            match result {
                RendererSetDocumentContentResult::Updated => plan.push_success(),
                RendererSetDocumentContentResult::FrameNotFound => {
                    plan.push_error(-32000, "No frame for given id found");
                }
                RendererSetDocumentContentResult::DocumentNotFound => {
                    plan.push_error(-32000, "No Document instance to set HTML for");
                }
            }
            plan
        }
        CompletedPageCommandKind::SameDocumentNavigate(completed) => {
            return navigation::complete_pending_same_document_navigate_command(
                conn,
                &owner_scope,
                *completed,
                command_context,
            );
        }
        CompletedPageCommandKind::GetFrameTree {
            output_kind,
            target_id,
            target_loader_id,
            target_url,
            target_unreachable_url,
            target_security_origin,
            target_secure_context_type,
            target_mime_type,
            completed,
        } => {
            if conn
                .ensure_document_accessible_for_owner(&owner_scope)
                .is_err()
            {
                return PageCommandTaskStep::Complete(get_frame_tree_command_output_plan(
                    output_kind,
                    target_id,
                    target_loader_id,
                    target_url,
                    target_unreachable_url,
                    target_security_origin,
                    target_secure_context_type,
                    target_mime_type,
                    Vec::new(),
                    &[],
                ));
            }
            let Some(page) = conn.loaded_page_mut_for_owner(&owner_scope) else {
                return PageCommandTaskStep::Complete(get_frame_tree_command_output_plan(
                    output_kind,
                    target_id,
                    target_loader_id,
                    target_url,
                    target_unreachable_url,
                    target_security_origin,
                    target_secure_context_type,
                    target_mime_type,
                    Vec::new(),
                    &[],
                ));
            };
            let child_frames = match *completed {
                Ok(completion) => match page.finish_child_frame_tree_snapshot(completion) {
                    Ok(frames) => frames,
                    Err(error) => {
                        return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                            -32000,
                            format!("Failed to snapshot child frame tree: {error}"),
                        ));
                    }
                },
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        format!("Failed to snapshot child frame tree: {message}"),
                    ));
                }
            };
            get_frame_tree_command_output_plan(
                output_kind,
                target_id,
                target_loader_id,
                target_url,
                target_unreachable_url,
                target_security_origin,
                target_secure_context_type,
                target_mime_type,
                child_frames,
                page.subresource_network_records(),
            )
        }
        CompletedPageCommandKind::CaptureSnapshot { completed } => {
            let completion = match *completed {
                Ok(completion) => completion,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        format!("Failed to serialize page snapshot: {message}"),
                    ));
                }
            };
            let Some(page) = conn.loaded_page_mut_for_owner(&owner_scope) else {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    crate::conn::NoDocumentLoaded,
                ));
            };
            let html = match page.finish_serialize_html(completion) {
                Ok(html) => html,
                Err(error) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        format!("Failed to serialize page snapshot: {error}"),
                    ));
                }
            };
            let url = page.final_url().as_str().to_owned();
            CommandOutputPlan::result(json!({ "data": build_mhtml_snapshot(&url, &html) }))
        }
        CompletedPageCommandKind::GetLayoutMetrics { completed } => {
            let completion = match *completed {
                Ok(completion) => completion,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        format!("Failed to produce layout metrics: {message}"),
                    ));
                }
            };
            let page = match conn.loaded_page_mut_for_protocol_access_for_owner(&owner_scope) {
                Ok(page) => page,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000, message,
                    ));
                }
            };
            match page.finish_layout_metrics(completion) {
                Ok(metrics) => CommandOutputPlan::from_devtools_result(
                    AutomationResult::LayoutMetrics(layout_metrics_result_from_renderer(metrics)),
                ),
                Err(error) => CommandOutputPlan::error(
                    -32000,
                    format!("Failed to finish layout metrics: {error}"),
                ),
            }
        }
        CompletedPageCommandKind::CaptureScreenshot { completed } => {
            let completion = match *completed {
                Ok(completion) => completion,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        format!("Failed to capture page screenshot: {message}"),
                    ));
                }
            };
            let page = match conn.loaded_page_mut_for_protocol_access_for_owner(&owner_scope) {
                Ok(page) => page,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000, message,
                    ));
                }
            };
            match page.finish_capture_screenshot(completion) {
                Ok(RendererCaptureScreenshotReply::Captured(image)) => {
                    CommandOutputPlan::from_devtools_result(AutomationResult::CaptureScreenshot(
                        DevToolsCaptureScreenshotResult {
                            mime_type: image.mime_type,
                            width: image.width,
                            height: image.height,
                            bytes: image.bytes,
                        },
                    ))
                }
                Ok(RendererCaptureScreenshotReply::LayoutDisabled) => {
                    CommandOutputPlan::error(-32000, CAPTURE_SCREENSHOT_LAYOUT_DISABLED_MESSAGE)
                }
                Ok(RendererCaptureScreenshotReply::NoDocument) => {
                    CommandOutputPlan::error(-32000, crate::conn::NoDocumentLoaded)
                }
                Err(error) => CommandOutputPlan::error(
                    -32000,
                    format!("Failed to capture page screenshot: {error}"),
                ),
            }
        }
        CompletedPageCommandKind::TraverseSameDocumentHistory(completed) => {
            return navigation::complete_pending_same_document_history_traversal_command(
                conn, command_id, *completed,
            );
        }
        _ => unreachable!("composite Page completion is prepared separately"),
    };
    PageCommandTaskStep::Complete(plan)
}

pub(super) async fn complete_using(
    driver: &mut crate::conn::ProtocolWorkflowDriver,
    context: &mut crate::conn::CommandWorkflowContext,
    completed: CompletedPageCommandDispatch,
) -> Result<PageCommandTaskStep> {
    if let Some(predecessor) = completed.kind.renderer_output_predecessor() {
        context
            .context_mut()
            .set_renderer_output_predecessor(predecessor);
    }
    if !is_composite(completed.kind.as_ref()) {
        return context.with_state(driver, move |state, context| {
            state.complete_finite_page_command(completed, context.context_mut())
        });
    }
    let command_id = completed.command_id;
    let owner = completed.owner_scope;
    let plan = match *completed.kind {
        CompletedPageCommandKind::BringToFront { route } => {
            bring_to_front_using(driver, route).await?
        }
        CompletedPageCommandKind::AddScriptToEvaluateOnNewDocument(prepared) => {
            prepared.run(driver, context).await?
        }
        CompletedPageCommandKind::CreateIsolatedWorld(completed) => {
            preload::complete_create_isolated_world_using(
                driver, context, command_id, owner, *completed,
            )
            .await?
        }
        CompletedPageCommandKind::ChildFrameNavigate(completed) => {
            super::child_navigation::finish_navigation(driver, context, owner, *completed).await?
        }
        CompletedPageCommandKind::BeforeUnloadNavigation(completed) => {
            let prepared = context.with_state(driver, move |state, context| {
                state.prepare_before_unload_navigation(*completed, context)
            })?;
            return Box::pin(prepared.run(driver, context, NavigationCommandOutput::Command)).await;
        }
        CompletedPageCommandKind::Navigate(completed) => {
            let prepared =
                driver.with_state(move |state| state.prepare_page_navigation(*completed))?;
            Box::pin(prepared.run(driver, context, NavigationCommandOutput::Command)).await?;
            CommandOutputPlan::default()
        }
        CompletedPageCommandKind::ContinueNavigationWithoutRequestPause(completed) => {
            let prepared = driver
                .with_state(move |state| state.prepare_page_navigation_request(*completed))?;
            Box::pin(prepared.run(driver, context, NavigationCommandOutput::Command)).await?;
            CommandOutputPlan::default()
        }
        CompletedPageCommandKind::PrintToPdf {
            completed,
            transfer_mode,
        } => {
            let prepared = context.with_state(driver, move |state, _| {
                state.prepare_pdf_output(owner, *completed, transfer_mode)
            })?;
            match prepared {
                Ok(prepared) => prepared.run(driver).await?,
                Err(plan) => plan,
            }
        }
        CompletedPageCommandKind::StopLoading(prepared) => prepared.run(driver, context).await?,
        CompletedPageCommandKind::Crash(prepared) => (*prepared).run(driver, context).await?,
        CompletedPageCommandKind::Close(prepared) => {
            prepared.run(driver, context).await?;
            CommandOutputPlan::success()
        }
        _ => unreachable!("finite Page completion is consumed before waiting"),
    };
    Ok(PageCommandTaskStep::Complete(plan))
}

fn is_composite(kind: &CompletedPageCommandKind) -> bool {
    matches!(
        kind,
        CompletedPageCommandKind::PrintToPdf { .. }
            | CompletedPageCommandKind::BringToFront { .. }
            | CompletedPageCommandKind::AddScriptToEvaluateOnNewDocument(_)
            | CompletedPageCommandKind::CreateIsolatedWorld(_)
            | CompletedPageCommandKind::ChildFrameNavigate(_)
            | CompletedPageCommandKind::Navigate(_)
            | CompletedPageCommandKind::BeforeUnloadNavigation(_)
            | CompletedPageCommandKind::ContinueNavigationWithoutRequestPause(_)
            | CompletedPageCommandKind::StopLoading(_)
            | CompletedPageCommandKind::Crash(_)
            | CompletedPageCommandKind::Close(_)
    )
}

fn workflow(
    access: crate::conn::ProtocolStateHandle,
    first: impl std::future::Future<Output = CompletedPageCommandDispatch> + 'static,
) -> crate::conn::PreparedCommandWorkflow {
    crate::conn::PreparedCommandWorkflow::new(move |mut context| async move {
        let mut driver = crate::conn::ProtocolWorkflowDriver::new(access);
        match Box::pin(complete_to_output_using(
            &mut driver,
            &mut context,
            first.await,
        ))
        .await
        {
            Ok(plan) => context.complete(plan),
            Err(error) => context.fail(CommandOutputPlan::error(-32000, error)),
        }
    })
}

pub(super) async fn complete_to_output_using(
    driver: &mut crate::conn::ProtocolWorkflowDriver,
    context: &mut crate::conn::CommandWorkflowContext,
    mut completed: CompletedPageCommandDispatch,
) -> Result<CommandOutputPlan> {
    loop {
        match complete_using(driver, context, completed).await? {
            PageCommandTaskStep::Complete(plan) => return Ok(plan),
            PageCommandTaskStep::Pending(pending) => {
                completed = Box::pin(pending.wait()).await;
            }
        }
    }
}

impl PendingPageCommandDispatch {
    pub(crate) fn try_into_workflow(
        self,
        access: crate::conn::ProtocolStateHandle,
    ) -> Result<crate::conn::PreparedCommandWorkflow, Self> {
        if !matches!(
            self.kind.as_ref(),
            PendingPageCommandKind::PrintToPdf { .. }
                | PendingPageCommandKind::BringToFront { .. }
                | PendingPageCommandKind::AddScriptToEvaluateOnNewDocument(_)
                | PendingPageCommandKind::CreateIsolatedWorld(_)
                | PendingPageCommandKind::ChildFrameNavigate(_)
                | PendingPageCommandKind::Navigate(_)
                | PendingPageCommandKind::BeforeUnloadNavigation(_)
                | PendingPageCommandKind::ContinueNavigationWithoutRequestPause(_)
                | PendingPageCommandKind::StopLoading(_)
                | PendingPageCommandKind::Crash(_)
                | PendingPageCommandKind::Close(_)
        ) {
            return Err(self);
        }
        Ok(workflow(access, self.wait()))
    }
}

impl CompletedPageCommandDispatch {
    pub(crate) fn try_into_workflow(
        self,
        access: crate::conn::ProtocolStateHandle,
    ) -> Result<crate::conn::PreparedCommandWorkflow, Self> {
        if !is_composite(self.kind.as_ref()) {
            return Err(self);
        }
        Ok(workflow(access, std::future::ready(self)))
    }
}

async fn bring_to_front_using(
    driver: &mut crate::conn::ProtocolWorkflowDriver,
    route: CdpSessionRoute,
) -> Result<CommandOutputPlan> {
    let prepared = driver.with_state(move |state| state.prepare_page_bring_to_front(route))?;
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(error) => return Ok(CommandOutputPlan::error(-31998, error.message)),
    };
    let mut plan = CommandOutputPlan::default();
    if let Some(prepared) = prepared {
        let output = match prepared.run(driver).await {
            Ok(output) => output,
            Err(error) => return Ok(CommandOutputPlan::error(-31998, error)),
        };
        let events = driver.with_state(move |state| state.finish_page_activation(output))?;
        plan.extend_background_events(events);
    }
    plan.push_success();
    Ok(plan)
}
