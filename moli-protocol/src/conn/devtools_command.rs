use crate::devtools_runtime::{
    DevToolsCommand, DevToolsCommandContext, DevToolsCommandResult, DevToolsError,
    DevToolsErrorKind, DevToolsTargetId,
};

use super::*;

/// Complete result of a protocol-neutral DevTools command before the outer
/// protocol scheduler establishes its response fence.
///
/// The three fields deliberately travel together. A renderer command may have
/// already published concrete observations even when its DevTools result is an
/// error, so neither the events nor the exact renderer cursor may be discarded
/// while a domain-specific adapter translates the result.
#[must_use = "DevTools execution output carries protocol events and an exact renderer cursor"]
pub(crate) struct DevToolsCommandExecutionOutput {
    result: Result<DevToolsCommandResult, DevToolsError>,
    protocol_events: Vec<BackgroundProtocolEvent>,
    renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
}

impl DevToolsCommandExecutionOutput {
    pub(crate) fn new(result: Result<DevToolsCommandResult, DevToolsError>) -> Self {
        Self {
            result,
            protocol_events: Vec::new(),
            renderer_output_predecessor: None,
        }
    }

    pub(crate) fn from_parts(
        result: Result<DevToolsCommandResult, DevToolsError>,
        protocol_events: Vec<BackgroundProtocolEvent>,
        renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
    ) -> Self {
        Self {
            result,
            protocol_events,
            renderer_output_predecessor,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Result<DevToolsCommandResult, DevToolsError>,
        Vec<BackgroundProtocolEvent>,
        Option<moli_core::RendererOutputFence>,
    ) {
        (
            self.result,
            self.protocol_events,
            self.renderer_output_predecessor,
        )
    }
}

pub struct DevToolsCommandDispatchOutcome {
    result: Result<DevToolsCommandResult, DevToolsError>,
    scheduler_events: Vec<CdpSchedulerEvent>,
    protocol_events: Vec<BackgroundProtocolEvent>,
    renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
}

impl DevToolsCommandDispatchOutcome {
    pub(crate) fn new_with_protocol_events(
        result: Result<DevToolsCommandResult, DevToolsError>,
        scheduler_events: Vec<CdpSchedulerEvent>,
        protocol_events: Vec<BackgroundProtocolEvent>,
    ) -> Self {
        Self {
            result,
            scheduler_events,
            protocol_events,
            renderer_output_predecessor: None,
        }
    }

    pub(crate) fn with_renderer_output_predecessor(
        mut self,
        predecessor: Option<moli_core::RendererOutputFence>,
    ) -> Self {
        self.renderer_output_predecessor = predecessor;
        self
    }

    pub fn renderer_output_predecessor(&self) -> Option<moli_core::RendererOutputFence> {
        self.renderer_output_predecessor.clone()
    }

    /// Consumes a command known not to have produced protocol observations or
    /// an exact renderer response fence.
    ///
    /// This deliberately remains a checked, narrow operation. If the command
    /// later starts producing auxiliary output, its owning call site fails
    /// immediately instead of silently losing that output.
    #[track_caller]
    pub fn into_parts(
        self,
    ) -> (
        Result<DevToolsCommandResult, DevToolsError>,
        Vec<CdpSchedulerEvent>,
    ) {
        assert!(
            self.protocol_events.is_empty(),
            "command output with protocol events requires complete consumption"
        );
        assert!(
            self.renderer_output_predecessor.is_none(),
            "command output with a renderer cursor requires complete consumption"
        );
        (self.result, self.scheduler_events)
    }

    /// Consumes a command which may have protocol events but is known not to
    /// have an exact renderer response fence.
    #[track_caller]
    pub fn into_parts_with_protocol_events(
        self,
    ) -> (
        Result<DevToolsCommandResult, DevToolsError>,
        Vec<CdpSchedulerEvent>,
        Vec<BackgroundProtocolEvent>,
    ) {
        assert!(
            self.renderer_output_predecessor.is_none(),
            "command output with a renderer cursor requires complete consumption"
        );
        (self.result, self.scheduler_events, self.protocol_events)
    }

    /// Consumes the complete command transaction, including the exact
    /// renderer stream position that must be projected before its response.
    pub fn into_complete_parts(
        self,
    ) -> (
        Result<DevToolsCommandResult, DevToolsError>,
        Vec<CdpSchedulerEvent>,
        Vec<BackgroundProtocolEvent>,
        Option<moli_core::RendererOutputFence>,
    ) {
        (
            self.result,
            self.scheduler_events,
            self.protocol_events,
            self.renderer_output_predecessor,
        )
    }
}

impl CdpConnection {
    pub async fn execute_devtools_command(
        &mut self,
        command: DevToolsCommand,
    ) -> DevToolsCommandDispatchOutcome {
        Box::pin(self.execute_devtools_command_with_protocol_events(command)).await
    }

    pub async fn execute_devtools_command_with_protocol_events(
        &mut self,
        command: DevToolsCommand,
    ) -> DevToolsCommandDispatchOutcome {
        self.execute_devtools_command_with_protocol_events_with_background_command_id(command, None)
            .await
    }

    pub async fn execute_devtools_command_with_protocol_events_with_background_command_id(
        &mut self,
        command: DevToolsCommand,
        background_command_id: Option<u64>,
    ) -> DevToolsCommandDispatchOutcome {
        let command_context = command.context().clone();
        let mut renderer_output_predecessor = None;
        let (result, protocol_events) = match command {
            DevToolsCommand::CreateTarget(command) => {
                let (result, protocol_events, predecessor) =
                    crate::domains::target::execute_devtools_create_target_command_async_with_protocol_events(
                        self,
                        command,
                    )
                    .await;
                renderer_output_predecessor = predecessor;
                (result, protocol_events)
            }
            command @ (DevToolsCommand::GetTargets(_)
            | DevToolsCommand::GetServiceWorkerLogs(_)
            | DevToolsCommand::GetClientWindows(_)
            | DevToolsCommand::CreateBrowserContext(_)
            | DevToolsCommand::GetBrowserContexts(_)
            | DevToolsCommand::GetTargetInfo(_)) => {
                crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
                    self, command,
                )
            }
            command @ DevToolsCommand::SetDownloadBehavior(_) => (
                crate::domains::browser::execute_devtools_browser_command(self, command),
                Vec::new(),
            ),
            command @ DevToolsCommand::SetPermission(_) => (
                Box::pin(crate::domains::browser::execute_devtools_browser_command_async(
                    self, command,
                ))
                .await,
                Vec::new(),
            ),
            command @ (DevToolsCommand::CloseTarget(_)
            | DevToolsCommand::ActivateTarget(_)
            | DevToolsCommand::RemoveBrowserContext(_)) => {
                Box::pin(
                    crate::domains::target::execute_devtools_target_command_async_with_protocol_events(
                        self, command,
                    ),
                )
                .await
            }
            command @ (DevToolsCommand::Navigate(_)
            | DevToolsCommand::Reload(_)
            | DevToolsCommand::CaptureScreenshot(_)
            | DevToolsCommand::PrintToPdf(_)
            | DevToolsCommand::GetJavaScriptDialog(_)
            | DevToolsCommand::SetJavaScriptDialogPromptText(_)
            | DevToolsCommand::HandleJavaScriptDialog(_)
            | DevToolsCommand::GetFrameTree(_)
            | DevToolsCommand::GetFrameTrees(_)
            | DevToolsCommand::GetLayoutMetrics(_)
            | DevToolsCommand::GetNavigationHistory(_)
            | DevToolsCommand::TraverseHistory(_)
            | DevToolsCommand::AddPreloadScript(_)
            | DevToolsCommand::RemovePreloadScript(_)) => {
                let (result, protocol_events, predecessor) = Box::pin(
                    crate::domains::page::execute_devtools_page_command_async_with_protocol_events(
                        self,
                        command,
                        background_command_id,
                    ),
                )
                .await;
                renderer_output_predecessor = predecessor;
                (result, protocol_events)
            }
            command @ (DevToolsCommand::SetViewport(_)
            | DevToolsCommand::SetWindowState(_)
            | DevToolsCommand::SetClientWindowState(_)
            | DevToolsCommand::SetUserAgentOverride(_)
            | DevToolsCommand::SetLocaleOverride(_)
            | DevToolsCommand::SetTimezoneOverride(_)
            | DevToolsCommand::SetGeolocationOverride(_)
            | DevToolsCommand::SetNetworkConditions(_)
            | DevToolsCommand::SetExtraHeaders(_)) => (
                Box::pin(crate::domains::emulation::execute_devtools_emulation_command_async(
                    self, command,
                ))
                .await,
                Vec::new(),
            ),
            command @ (DevToolsCommand::GetRealms(_)
            | DevToolsCommand::EvaluateScript(_)
            | DevToolsCommand::CallFunction(_)
            | DevToolsCommand::TerminateExecution(_)
            | DevToolsCommand::LocateNodes(_)
            | DevToolsCommand::ReleaseObjects(_)) => {
                let output = Box::pin(
                    crate::domains::runtime::execute_devtools_runtime_command_async_with_protocol_events(
                        self, command,
                    ),
                )
                .await
                ;
                let (result, protocol_events, predecessor) = output.into_parts();
                renderer_output_predecessor = predecessor;
                (result, protocol_events)
            }
            command @ (DevToolsCommand::GetCookies(_)
            | DevToolsCommand::DeleteCookies(_)
            | DevToolsCommand::SetCookies(_)) => (
                Box::pin(crate::domains::storage::execute_devtools_storage_command_async(
                    self, command,
                ))
                .await,
                Vec::new(),
            ),
            command @ (DevToolsCommand::AddNetworkIntercept(_)
            | DevToolsCommand::RemoveNetworkIntercept(_)
            | DevToolsCommand::ContinueInterceptedRequest(_)
            | DevToolsCommand::ContinueInterceptedResponse(_)
            | DevToolsCommand::ContinueWithAuth(_)
            | DevToolsCommand::FailInterceptedRequest(_)
            | DevToolsCommand::FulfillInterceptedRequest(_)) => {
                let output = Box::pin(
                    crate::domains::fetch::execute_devtools_fetch_command_async_with_protocol_events(
                        self, command,
                    ),
                )
                .await
                ;
                let (result, protocol_events, predecessor) = output.into_parts();
                renderer_output_predecessor = predecessor;
                (result, protocol_events)
            }
            command @ (DevToolsCommand::AddNetworkDataCollector(_)
            | DevToolsCommand::RemoveNetworkDataCollector(_)
            | DevToolsCommand::DisownNetworkData(_)
            | DevToolsCommand::GetNetworkData(_)
            | DevToolsCommand::SetCacheBehavior(_)) => (
                crate::domains::network::execute_devtools_network_command(self, command),
                Vec::new(),
            ),
            command @ (DevToolsCommand::DispatchMouseEvent(_)
            | DevToolsCommand::DispatchKeyEvent(_)
            | DevToolsCommand::DispatchTouchEvent(_)
            | DevToolsCommand::DispatchDragEvent(_)
            | DevToolsCommand::SynthesizeTapGesture(_)) => {
                Box::pin(
                    crate::domains::input::execute_devtools_input_command_async_with_protocol_events(
                        self, command,
                    ),
                )
                .await
            }
            command @ (DevToolsCommand::QuerySelector(_)
            | DevToolsCommand::GetAttributes(_)
            | DevToolsCommand::GetText(_)
            | DevToolsCommand::GetProperty(_)
            | DevToolsCommand::GetOuterHtml(_)
            | DevToolsCommand::DescribeNode(_)
            | DevToolsCommand::GetFrameOwner(_)
            | DevToolsCommand::ResolveNode(_)
            | DevToolsCommand::ScrollIntoViewIfNeeded(_)
            | DevToolsCommand::DomObjectReference(_)
            | DevToolsCommand::SetFileInputFiles(_)
            | DevToolsCommand::DomGeometry(_)) => (
                Box::pin(crate::domains::dom::execute_devtools_dom_command_async(
                    self, command,
                ))
                .await,
                Vec::new(),
            ),
            _ => (
                Err(DevToolsError::new(
                    DevToolsErrorKind::Unsupported,
                    "UnsupportedDevToolsCommand",
                )),
                Vec::new(),
            ),
        };
        self.finish_devtools_command_dispatch(
            command_context,
            result,
            protocol_events,
            renderer_output_predecessor,
        )
        .await
    }

    pub(crate) async fn finish_devtools_command_dispatch(
        &mut self,
        command_context: DevToolsCommandContext,
        result: Result<DevToolsCommandResult, DevToolsError>,
        mut protocol_events: Vec<BackgroundProtocolEvent>,
        renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
    ) -> DevToolsCommandDispatchOutcome {
        let mut dispatch_context = CommandDispatchContext::default();
        Box::pin(self.project_protocol_local_outputs_for_direct_command(
            &command_context,
            &mut dispatch_context,
            &mut protocol_events,
        ))
        .await;
        protocol_events.extend(dispatch_context.take_post_response_events());
        let scheduler_events = self.take_scheduler_events();
        DevToolsCommandDispatchOutcome::new_with_protocol_events(
            result,
            scheduler_events,
            protocol_events,
        )
        .with_renderer_output_predecessor(renderer_output_predecessor)
    }

    pub(crate) async fn ensure_created_target_initial_document_page(
        &mut self,
        target_id: &DevToolsTargetId,
    ) -> (
        Vec<BackgroundProtocolEvent>,
        Option<moli_core::RendererOutputFence>,
    ) {
        let Some(route) = self.target_session_route_for_target_id(target_id.as_str()) else {
            return (Vec::new(), None);
        };
        let has_default_bidi_channel_preload_script = route
            .browser_context_id()
            .and_then(|browser_context_id| self.browser_context_by_id(browser_context_id))
            .is_some_and(|browser_context| {
                browser_context.has_default_bidi_channel_preload_script()
            });
        let mut route_scope = self.scoped_none_session_owner_route_override(route);
        let mut initial_runtime_execution_context_ids = Vec::new();
        let mut renderer_output_predecessor = None;
        let pending = match route_scope
            .conn_mut()
            .start_initial_document_page_ensure_for_session_owner(None)
        {
            Ok(pending) => pending,
            Err(error) => {
                tracing::debug!(
                    ?error,
                    target_id = %target_id.as_str(),
                    "failed to start create-target initial document page ensure"
                );
                return (Vec::new(), None);
            }
        };
        if let Some(pending) = pending {
            match pending.wait().await {
                Ok(completed) => {
                    match route_scope
                        .conn_mut()
                        .complete_initial_document_page_build_for_owner_with_creation_diagnostics(
                            completed,
                        )
                        .await
                    {
                        Ok(diagnostics) => {
                            renderer_output_predecessor = diagnostics.renderer_output_predecessor;
                            initial_runtime_execution_context_ids = diagnostics
                                .initial_runtime_realms
                                .into_iter()
                                .filter(|realm| {
                                    realm
                                        .realm_id
                                        .as_deref()
                                        .is_some_and(|realm_id| !realm_id.is_empty())
                                })
                                .map(|realm| realm.context_id)
                                .collect();
                        }
                        Err(error) => {
                            tracing::debug!(
                                ?error,
                                target_id = %target_id.as_str(),
                                "failed to complete create-target initial document page ensure"
                            );
                            return (Vec::new(), None);
                        }
                    }
                }
                Err(error) => {
                    let error = route_scope
                        .conn_mut()
                        .reset_failed_initial_document_page_build_for_owner(error);
                    tracing::debug!(
                        ?error,
                        target_id = %target_id.as_str(),
                        "failed to await create-target initial document page ensure"
                    );
                    return (Vec::new(), None);
                }
            }
        }
        let mut listener_events = Vec::new();
        if has_default_bidi_channel_preload_script
            || route_scope
                .conn_mut()
                .target_owner_has_bidi_channel_preload_script_for_session(None)
        {
            let mut execution_context_ids = initial_runtime_execution_context_ids;
            // Target lifecycle creation can materialize initial about:blank without Runtime.enable;
            // in that path the renderer has an initial default context id but no Runtime
            // frontend-created inspector batch.
            if execution_context_ids.is_empty()
                && let Ok(Some(default_context_id)) = route_scope
                    .conn_mut()
                    .runtime_default_or_initial_execution_context_id_for_session_owner_async(None)
                    .await
            {
                execution_context_ids.push(default_context_id);
            }
            for execution_context_id in execution_context_ids {
                Box::pin(
                    crate::domains::runtime::start_bidi_preload_channel_listeners_for_execution_context_background_events_async(
                        route_scope.conn_mut(),
                        None,
                        execution_context_id,
                        &mut listener_events,
                    ),
                )
                .await;
            }
        }
        (listener_events, renderer_output_predecessor)
    }

    async fn project_protocol_local_outputs_for_direct_command(
        &mut self,
        context: &DevToolsCommandContext,
        dispatch_context: &mut CommandDispatchContext,
        protocol_events: &mut Vec<BackgroundProtocolEvent>,
    ) {
        if self.has_pending_javascript_dialog() {
            return;
        }
        let mut session_id = context
            .session_id
            .as_ref()
            .map(|session_id| session_id.as_str());
        let none_session_owner_route = match context.target_id.as_ref() {
            Some(target_id) => {
                let Some(route) = self.target_session_route_for_target_id(target_id.as_str())
                else {
                    return;
                };
                session_id = None;
                Some(route)
            }
            None => None,
        };
        if let Some(route) = none_session_owner_route {
            let mut route_scope = self.scoped_none_session_owner_route_override(route);
            crate::domains::activity::project_protocol_local_command_outputs(
                route_scope.conn_mut(),
                session_id,
                dispatch_context,
            )
            .await;
            let turn_events = dispatch_context.take_protocol_events();
            protocol_events.extend(turn_events);
        } else {
            crate::domains::activity::project_protocol_local_command_outputs(
                self,
                session_id,
                dispatch_context,
            )
            .await;
            protocol_events.extend(dispatch_context.take_protocol_events());
        }
    }
}
