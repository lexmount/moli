mod dispatch;
mod workflow;
pub(crate) use dispatch::AutomationCommandResult;
pub use dispatch::{
    AutomationCommandTaskStep, CompletedAutomationCommandDispatch, PendingAutomationCommandDispatch,
};

use crate::automation::{
    AutomationCommand, AutomationContext, AutomationResult, DevToolsError, DevToolsErrorKind,
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
pub(crate) struct AutomationExecutionOutput {
    result: Result<AutomationResult, DevToolsError>,
    protocol_events: Vec<BackgroundProtocolEvent>,
    renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
}

impl AutomationExecutionOutput {
    pub(crate) fn new(result: Result<AutomationResult, DevToolsError>) -> Self {
        Self {
            result,
            protocol_events: Vec::new(),
            renderer_output_predecessor: None,
        }
    }

    pub(crate) fn from_parts(
        result: Result<AutomationResult, DevToolsError>,
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
        Result<AutomationResult, DevToolsError>,
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

pub struct AutomationCommandDispatchOutcome {
    result: Result<AutomationResult, DevToolsError>,
    scheduler_events: Vec<CdpSchedulerEvent>,
    protocol_events: Vec<BackgroundProtocolEvent>,
    renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
}

impl AutomationCommandDispatchOutcome {
    pub(crate) fn new_with_protocol_events(
        result: Result<AutomationResult, DevToolsError>,
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
    /// later starts producing additional output, its owning call site fails
    /// immediately instead of silently losing that output.
    #[track_caller]
    pub fn into_parts(
        self,
    ) -> (
        Result<AutomationResult, DevToolsError>,
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
        Result<AutomationResult, DevToolsError>,
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
        Result<AutomationResult, DevToolsError>,
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
    pub async fn execute_automation_command(
        &mut self,
        command: AutomationCommand,
    ) -> AutomationCommandDispatchOutcome {
        Box::pin(self.execute_automation_command_with_protocol_events(command)).await
    }

    pub async fn execute_automation_command_with_protocol_events(
        &mut self,
        command: AutomationCommand,
    ) -> AutomationCommandDispatchOutcome {
        self.execute_automation_command_with_protocol_events_with_background_command_id(
            command, None,
        )
        .await
    }

    pub async fn execute_automation_command_with_protocol_events_with_background_command_id(
        &mut self,
        command: AutomationCommand,
        background_command_id: Option<u64>,
    ) -> AutomationCommandDispatchOutcome {
        let command_context = command.context().clone();
        let mut renderer_output_predecessor = None;
        let (result, protocol_events) = match command {
            command @ (AutomationCommand::GetTargets(_)
            | AutomationCommand::GetServiceWorkerLogs(_)
            | AutomationCommand::GetClientWindows(_)
            | AutomationCommand::CreateBrowserContext(_)
            | AutomationCommand::GetBrowserContexts(_)
            | AutomationCommand::GetTargetInfo(_)) => {
                crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
                    self, command,
                )
            }
            command @ AutomationCommand::SetDownloadBehavior(_) => (
                crate::domains::browser::execute_devtools_browser_command(self, command),
                Vec::new(),
            ),
            command @ (AutomationCommand::PrintToPdf(_)
            | AutomationCommand::GetJavaScriptDialog(_)
            | AutomationCommand::SetJavaScriptDialogPromptText(_)
            | AutomationCommand::HandleJavaScriptDialog(_)
            | AutomationCommand::GetNavigationHistory(_)
            | AutomationCommand::AddPreloadScript(_)
            | AutomationCommand::RemovePreloadScript(_)) => {
                let (result, protocol_events, predecessor) = Box::pin(
                    crate::domains::page::execute_devtools_page_command_async_with_protocol_events(
                        self,
                        command,
                    ),
                )
                .await;
                renderer_output_predecessor = predecessor;
                (result, protocol_events)
            }
            command @ (AutomationCommand::GetRealms(_)
            | AutomationCommand::EvaluateScript(_)
            | AutomationCommand::CallFunction(_)
            | AutomationCommand::TerminateExecution(_)
            | AutomationCommand::LocateNodes(_)
            | AutomationCommand::ReleaseObjects(_)) => {
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
            command @ (AutomationCommand::GetFrameTree(_)
            | AutomationCommand::GetFrameTrees(_)
            | AutomationCommand::GetLayoutMetrics(_)
            | AutomationCommand::CaptureScreenshot(_)
            | AutomationCommand::QuerySelector(_)
            | AutomationCommand::GetAttributes(_)
            | AutomationCommand::GetText(_)
            | AutomationCommand::GetProperty(_)
            | AutomationCommand::GetOuterHtml(_)
            | AutomationCommand::DescribeNode(_)
            | AutomationCommand::GetFrameOwner(_)
            | AutomationCommand::GetNodeForLocation(_)
            | AutomationCommand::ResolveNode(_)
            | AutomationCommand::ScrollIntoViewIfNeeded(_)
            | AutomationCommand::DomObjectReference(_)
            | AutomationCommand::SetFileInputFiles(_)
            | AutomationCommand::DomGeometry(_)
            | AutomationCommand::CreateTarget(_)
            | AutomationCommand::ActivateTarget(_)
            | AutomationCommand::ElementClick(_)
            | AutomationCommand::DispatchMouseEvent(_)
            | AutomationCommand::DispatchKeyEvent(_)
            | AutomationCommand::DispatchTouchEvent(_)
            | AutomationCommand::DispatchDragEvent(_)
            | AutomationCommand::SynthesizeTapGesture(_)
            | AutomationCommand::SetPermission(_)
            | AutomationCommand::GetCookies(_)
            | AutomationCommand::SetCookies(_)
            | AutomationCommand::DeleteCookies(_)
            | AutomationCommand::SetViewport(_)
            | AutomationCommand::SetWindowState(_)
            | AutomationCommand::SetClientWindowState(_)
            | AutomationCommand::SetUserAgentOverride(_)
            | AutomationCommand::SetLocaleOverride(_)
            | AutomationCommand::SetTimezoneOverride(_)
            | AutomationCommand::SetGeolocationOverride(_)
            | AutomationCommand::SetNetworkConditions(_)
            | AutomationCommand::SetExtraHeaders(_)) => {
                let (result, events, predecessor) =
                    workflow::execute_inline(self, command).await.into_parts();
                renderer_output_predecessor = predecessor;
                (result, events)
            }
            command @ (AutomationCommand::Navigate(_)
            | AutomationCommand::Reload(_)
            | AutomationCommand::TraverseHistory(_)) => {
                let (result, events, predecessor) = Box::pin(
                    crate::domains::page::execute_automation_navigation_inline(
                        self, command, background_command_id,
                    ),
                ).await;
                renderer_output_predecessor = predecessor;
                (result, events)
            }
            command @ (AutomationCommand::CloseTarget(_)
            | AutomationCommand::RemoveBrowserContext(_)
            | AutomationCommand::AddNetworkIntercept(_)
            | AutomationCommand::RemoveNetworkIntercept(_)
            | AutomationCommand::ContinueInterceptedRequest(_)
            | AutomationCommand::ContinueInterceptedResponse(_)
            | AutomationCommand::ContinueWithAuth(_)
            | AutomationCommand::FailInterceptedRequest(_)
            | AutomationCommand::FulfillInterceptedRequest(_)) => {
                let output = Box::pin(
                    workflow::execute_protocol_inline(self, command),
                )
                .await
                ;
                let (result, protocol_events, predecessor) = output.into_parts();
                renderer_output_predecessor = predecessor;
                (result, protocol_events)
            }
            command @ (AutomationCommand::AddNetworkDataCollector(_)
            | AutomationCommand::RemoveNetworkDataCollector(_)
            | AutomationCommand::DisownNetworkData(_)
            | AutomationCommand::GetNetworkData(_)
            | AutomationCommand::SetCacheBehavior(_)) => (
                crate::domains::network::execute_devtools_network_command(self, command),
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
        self.finish_automation_command_dispatch(
            command_context,
            result,
            protocol_events,
            renderer_output_predecessor,
        )
    }

    pub(crate) fn finish_automation_command_dispatch(
        &mut self,
        command_context: AutomationContext,
        result: Result<AutomationResult, DevToolsError>,
        mut protocol_events: Vec<BackgroundProtocolEvent>,
        renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
    ) -> AutomationCommandDispatchOutcome {
        let mut dispatch_context = CommandDispatchContext::default();
        self.project_protocol_local_outputs_for_direct_command(
            &command_context,
            &mut dispatch_context,
            &mut protocol_events,
        );
        protocol_events.extend(dispatch_context.take_post_response_events());
        let scheduler_events = self.take_scheduler_events();
        AutomationCommandDispatchOutcome::new_with_protocol_events(
            result,
            scheduler_events,
            protocol_events,
        )
        .with_renderer_output_predecessor(renderer_output_predecessor)
    }

    pub(super) fn project_protocol_local_outputs_for_direct_command(
        &mut self,
        context: &AutomationContext,
        dispatch_context: &mut CommandDispatchContext,
        protocol_events: &mut Vec<BackgroundProtocolEvent>,
    ) {
        if self.has_pending_javascript_dialog() {
            return;
        }
        let session_id = context
            .session_id
            .as_ref()
            .map(|session_id| session_id.as_str());
        let owner = match context.target_id.as_ref() {
            Some(target_id) => {
                let Some(route) = self.target_session_route_for_target_id(target_id.as_str())
                else {
                    return;
                };
                CommandOwnerScope::for_route(route)
            }
            None => CommandOwnerScope::capture(self, session_id),
        };
        crate::domains::activity::project_protocol_local_command_outputs(
            self,
            &owner,
            dispatch_context,
        );
        protocol_events.extend(dispatch_context.take_protocol_events());
    }
}
