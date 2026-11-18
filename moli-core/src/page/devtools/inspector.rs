use crate::RendererOutputFence;
use crate::page::CompletedPageCommand;
use crate::page::Page;
use crate::page::PendingDevToolsIoCommandDispatch;
use crate::page::PendingPageCommand;
use crate::page::PendingRuntimeInspectorCommandDispatch;
use crate::page::RendererCommandTurnOutput;
use crate::page::RendererInspectorSessionRestoreSnapshot;
use crate::page::RendererRuntimeInspectorMessage;
use crate::page::protocol_support::DocumentStartScript;
use crate::page::protocol_support::RuntimeBindingRegistration;
use crate::page::protocol_support::RuntimeContextRestoreEvent;
use crate::page::protocol_support::RuntimeIsolatedWorldDefinition;
use crate::renderer::RendererDomDebuggerDomBreakpointResolution;
use crate::renderer::RendererDomDebuggerEventListenerBreakpoint;
use crate::renderer::RendererDomDebuggerEventListenersResolution;
use crate::renderer::RendererDomDebuggerXhrBreakpoint;
use crate::renderer::RendererInspectorCommandEnvelope;
use crate::renderer::RendererInspectorCommandRoute;
use crate::renderer::RendererInspectorIngressTicket;
use crate::renderer::RendererPageCommand;
use crate::renderer::RendererPageReply;
use crate::renderer::RendererPerformanceMetricSnapshot;
use crate::renderer::RendererRuntimeHeapUsage;
use crate::renderer::RendererRuntimeInspectorResponseSender;
use crate::{RendererDevToolsPageCommand, RendererDevToolsPageReply};
use anyhow::Result;
use anyhow::anyhow;
use serde_json::Value;

impl Page {
    pub fn start_ensure_isolated_worlds_attached_to_inspector(&self) -> Result<PendingPageCommand> {
        self.start_full_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::EnsureIsolatedWorldsAttachedToInspector,
        ))
    }

    pub async fn ensure_isolated_worlds_attached_to_inspector_async(&mut self) -> Result<()> {
        let pending = self.start_ensure_isolated_worlds_attached_to_inspector()?;
        self.finish_unit_runtime_page_command(
            pending.wait().await?,
            "ensure isolated worlds attached to inspector",
        )
    }

    pub fn start_inspector_execution_context_id_for_isolated_context(
        &self,
        execution_context_id: i64,
    ) -> Result<PendingPageCommand> {
        self.start_full_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::InspectorExecutionContextIdForIsolatedContext(
                execution_context_id,
            ),
        ))
    }

    pub async fn inspector_execution_context_id_for_isolated_context_async(
        &mut self,
        execution_context_id: i64,
    ) -> Result<Option<i64>> {
        let pending =
            self.start_inspector_execution_context_id_for_isolated_context(execution_context_id)?;
        self.finish_inspector_execution_context_id_for_isolated_context(pending.wait().await?)
    }

    pub async fn isolated_execution_context_id_for_inspector_context_async(
        &mut self,
        execution_context_id: i64,
    ) -> Result<Option<i64>> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::DevTools(
                RendererDevToolsPageCommand::IsolatedExecutionContextIdForInspectorContext(
                    execution_context_id,
                ),
            ))
            .await?;
        expect_page_reply!(
            reply,
            "synthetic isolated context id page command",
            "an optional execution context reply",
            RendererPageReply::OptionalExecutionContextId(id) => Ok(id),
        )
    }

    pub async fn create_isolated_world_async(
        &mut self,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<i64> {
        let command =
            RendererPageCommand::DevTools(RendererDevToolsPageCommand::CreateIsolatedWorld {
                name: name.to_owned(),
                grant_universal_access,
                frame_id: None,
            });
        let reply = self.dispatch_page_command_async(command).await?;
        expect_page_reply!(
            reply,
            "create isolated world page command",
            "an execution context reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::ExecutionContextId(id)) => Ok(id),
        )
    }

    pub async fn create_isolated_world_for_frame_async(
        &mut self,
        frame_id: &str,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<i64> {
        let command =
            RendererPageCommand::DevTools(RendererDevToolsPageCommand::CreateIsolatedWorld {
                name: name.to_owned(),
                grant_universal_access,
                frame_id: Some(frame_id.to_owned()),
            });
        let reply = self.dispatch_page_command_async(command).await?;
        expect_page_reply!(
            reply,
            "create isolated world page command",
            "an execution context reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::ExecutionContextId(id)) => Ok(id),
        )
    }

    pub fn start_create_isolated_world(
        &self,
        name: &str,
        grant_universal_access: bool,
        frame_id: Option<&str>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::CreateIsolatedWorld {
                name: name.to_owned(),
                grant_universal_access,
                frame_id: frame_id.map(str::to_owned),
            },
        ))
    }

    pub fn start_create_isolated_world_runtime_activity_capturing_runtime_inspector_messages(
        &self,
        inspector_session_id: Option<&str>,
        frame_id: Option<&str>,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::CreateIsolatedWorldRuntimeActivity {
                inspector_session_id: inspector_session_id.map(str::to_owned),
                frame_id: frame_id.map(str::to_owned),
                name: name.to_owned(),
                grant_universal_access,
            },
        ))
    }

    pub fn finish_create_isolated_world(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<i64> {
        let (execution_context_id, _) =
            self.finish_create_isolated_world_command_turn(completion)?;
        Ok(execution_context_id)
    }

    pub fn finish_create_isolated_world_command_turn(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<(i64, RendererCommandTurnOutput)> {
        let output = self.finish_page_command_turn(completion);
        let RendererPageReply::DevTools(RendererDevToolsPageReply::ExecutionContextId(
            execution_context_id,
        )) = output.completion().reply()
        else {
            return Err(anyhow!(
                "create isolated world page command returned an unexpected renderer reply"
            ));
        };
        Ok((*execution_context_id, output))
    }

    pub async fn dispatch_runtime_protocol_message_async(
        &mut self,
        raw_json: &str,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        let pending = self.start_runtime_protocol_message(raw_json.to_owned())?;
        let completion = pending.wait().await?;
        self.finish_runtime_protocol_message(completion)
    }

    pub async fn dispatch_runtime_protocol_message_for_inspector_session_async(
        &mut self,
        inspector_session_id: Option<String>,
        raw_json: &str,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        let pending = self.start_runtime_protocol_message_for_inspector_session(
            inspector_session_id,
            raw_json.to_owned(),
        )?;
        let completion = pending.wait().await?;
        self.finish_runtime_protocol_message(completion)
    }

    pub fn start_runtime_protocol_message(&self, raw_json: String) -> Result<PendingPageCommand> {
        self.start_runtime_protocol_message_for_inspector_session(None, raw_json)
    }

    pub fn start_runtime_protocol_message_for_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        raw_json: String,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::dispatch_runtime_protocol_message(
            inspector_session_id,
            raw_json,
        ))
    }

    pub fn start_runtime_protocol_message_with_deferred_response(
        &self,
        raw_json: String,
        deferred_response: RendererRuntimeInspectorResponseSender,
    ) -> Result<PendingPageCommand> {
        self.start_runtime_protocol_message_for_inspector_session_with_deferred_response(
            None,
            raw_json,
            deferred_response,
        )
    }

    pub fn start_runtime_protocol_message_for_inspector_session_with_deferred_response(
        &self,
        inspector_session_id: Option<String>,
        raw_json: String,
        deferred_response: RendererRuntimeInspectorResponseSender,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::dispatch_runtime_protocol_message_with_deferred_response(
                inspector_session_id,
                raw_json,
                deferred_response,
            ),
        )
    }

    pub fn start_routable_runtime_protocol_message_for_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        inspector_route: RendererInspectorCommandRoute,
        owner_context_resolution_action: Option<String>,
        raw_json: String,
        deferred_response: RendererRuntimeInspectorResponseSender,
    ) -> Result<PendingRuntimeInspectorCommandDispatch> {
        match inspector_route {
            RendererInspectorCommandRoute::MainThread => {
                let route = self.handle.enqueue_runtime_inspector_main_command(
                    RendererInspectorCommandEnvelope::new_main_protocol(
                        RendererInspectorIngressTicket::new(
                            self.renderer_agent_attachment_id,
                            inspector_session_id,
                            RendererInspectorCommandRoute::MainThread,
                        ),
                        owner_context_resolution_action,
                        raw_json,
                        deferred_response,
                    ),
                );
                Ok(Self::pending_main_ingress_runtime_inspector_command_dispatch(route))
            }
            RendererInspectorCommandRoute::Io => {
                if owner_context_resolution_action.is_some() {
                    return Err(anyhow!(
                        "an IO Inspector command cannot require Page owner context resolution"
                    ));
                }
                let route = self.handle.enqueue_runtime_inspector_io_command(
                    RendererInspectorCommandEnvelope::new_io(
                        RendererInspectorIngressTicket::new(
                            self.renderer_agent_attachment_id,
                            inspector_session_id,
                            RendererInspectorCommandRoute::Io,
                        ),
                        raw_json,
                        Some(deferred_response),
                    ),
                );
                Ok(Self::pending_io_runtime_inspector_command_dispatch(route))
            }
        }
    }

    pub fn start_runtime_inspector_io_message_without_response(
        &self,
        inspector_session_id: Option<String>,
        raw_json: String,
    ) -> Result<PendingRuntimeInspectorCommandDispatch> {
        let route = self.handle.enqueue_runtime_inspector_io_command(
            RendererInspectorCommandEnvelope::new_io(
                RendererInspectorIngressTicket::new(
                    self.renderer_agent_attachment_id,
                    inspector_session_id,
                    RendererInspectorCommandRoute::Io,
                ),
                raw_json,
                None,
            ),
        );
        Ok(Self::pending_io_runtime_inspector_command_dispatch(route))
    }

    pub fn start_runtime_protocol_message_with_context_resolution(
        &self,
        action: String,
        raw_json: String,
    ) -> Result<PendingPageCommand> {
        self.start_runtime_protocol_message_for_inspector_session_with_context_resolution(
            None, action, raw_json,
        )
    }

    pub fn start_runtime_protocol_message_for_inspector_session_with_context_resolution(
        &self,
        inspector_session_id: Option<String>,
        action: String,
        raw_json: String,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::dispatch_runtime_protocol_message_with_context_resolution(
                inspector_session_id,
                action,
                raw_json,
            ),
        )
    }

    pub fn start_runtime_protocol_message_with_context_resolution_and_deferred_response(
        &self,
        action: String,
        raw_json: String,
        deferred_response: RendererRuntimeInspectorResponseSender,
    ) -> Result<PendingPageCommand> {
        self.start_runtime_protocol_message_for_inspector_session_with_context_resolution_and_deferred_response(
            None,
            action,
            raw_json,
            deferred_response,
        )
    }

    pub fn start_runtime_protocol_message_for_inspector_session_with_context_resolution_and_deferred_response(
        &self,
        inspector_session_id: Option<String>,
        action: String,
        raw_json: String,
        deferred_response: RendererRuntimeInspectorResponseSender,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::dispatch_runtime_protocol_message_with_context_resolution_and_deferred_response(
                inspector_session_id,
                action,
                raw_json,
                deferred_response,
            ),
        )
    }

    pub async fn runtime_enable_events_async(
        &mut self,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        self.runtime_enable_events_for_inspector_session_async(None)
            .await
    }

    pub async fn runtime_enable_events_for_inspector_session_async(
        &mut self,
        inspector_session_id: Option<&str>,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        let pending =
            self.start_runtime_enable_events_for_inspector_session(inspector_session_id)?;
        let completion = pending.wait().await?;
        self.finish_runtime_enable_events(completion)
    }

    pub async fn runtime_enable_context_restore_events_for_inspector_session_async(
        &mut self,
        inspector_session_id: Option<&str>,
    ) -> Result<Vec<RuntimeContextRestoreEvent>> {
        let pending =
            self.start_runtime_enable_events_for_inspector_session(inspector_session_id)?;
        let completion = pending.wait().await?;
        self.finish_runtime_enable_context_restore_events(completion)
    }

    pub fn start_runtime_enable_events(&self) -> Result<PendingPageCommand> {
        self.start_runtime_enable_events_for_inspector_session(None)
    }

    pub fn start_runtime_enable_events_for_inspector_session(
        &self,
        inspector_session_id: Option<&str>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::runtime_enable_events(
            inspector_session_id.map(str::to_owned),
        ))
    }

    pub async fn detach_runtime_inspector_session_async(
        &mut self,
        inspector_session_id: Option<&str>,
    ) -> Result<bool> {
        self.detach_runtime_inspector_session(inspector_session_id)
    }

    pub fn detach_runtime_inspector_session(
        &mut self,
        inspector_session_id: Option<&str>,
    ) -> Result<bool> {
        self.handle
            .detach_runtime_inspector_session(inspector_session_id.map(str::to_owned))?;
        if self.renderer_devtools_command_session_id.as_deref() == inspector_session_id {
            // `runtime_session_owner_slot_mut` stamps the current frontend
            // session onto the Page before command construction. Do not leave
            // a detached child session as the fallback provenance for a
            // later owner-side observation that is not itself entered through
            // a CDP session lookup.
            self.renderer_devtools_command_session_id = None;
        }
        Ok(true)
    }

    pub async fn runtime_heap_usage_async(&mut self) -> Result<RendererRuntimeHeapUsage> {
        let pending = self.start_runtime_heap_usage()?;
        let completion = pending.wait().await?;
        self.finish_runtime_heap_usage(completion)
    }

    pub fn start_runtime_heap_usage(&self) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::RuntimeHeapUsage)
    }

    pub fn finish_runtime_heap_usage(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererRuntimeHeapUsage> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "runtime heap usage page command",
            "runtime heap usage",
            RendererPageReply::RuntimeHeapUsage(usage) => Ok(*usage),
        )
    }

    pub fn start_performance_metric_snapshot(&self) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::PerformanceMetricSnapshot,
        ))
    }

    /// Admits `Performance.getMetrics` through the same target IO task FIFO used
    /// by interruptible V8 Inspector commands. The snapshot remains the latest
    /// owner-published value because collecting a fresh JS-derived snapshot is
    /// not re-entrant while the isolate is executing JavaScript.
    pub fn start_performance_metric_snapshot_from_io(
        &self,
    ) -> (
        PendingDevToolsIoCommandDispatch,
        RendererPerformanceMetricSnapshot,
    ) {
        let route = self.handle.enqueue_performance_get_metrics_io_command(
            self.renderer_agent_attachment_id,
            self.renderer_devtools_command_session_id.clone(),
        );
        (
            Self::pending_devtools_io_command_dispatch(route),
            self.cached_performance_metric_snapshot(),
        )
    }

    /// Publishes the terminal `Performance.getMetrics` response through the
    /// concrete renderer DevTools session that owns this Page attachment.
    pub fn start_performance_get_metrics_from_io_with_response(
        &self,
        inspector_session_id: Option<String>,
        result: Value,
        response: RendererRuntimeInspectorResponseSender,
    ) -> Result<PendingDevToolsIoCommandDispatch> {
        let attachment = self
            .renderer_agent_attachment_id
            .ok_or_else(|| anyhow!("Performance IO response requires a renderer attachment"))?;
        let route = self
            .handle
            .enqueue_performance_get_metrics_io_command_with_response(
                attachment,
                inspector_session_id,
                result,
                response,
            );
        Ok(Self::pending_devtools_io_command_dispatch(route))
    }

    pub fn finish_performance_metric_snapshot(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererPerformanceMetricSnapshot> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "performance metric snapshot page command",
            "performance metric snapshot",
            RendererPageReply::DevTools(RendererDevToolsPageReply::PerformanceMetricSnapshot(snapshot)) => Ok(*snapshot),
        )
    }

    pub fn start_dom_debugger_get_event_listeners(
        &self,
        inspector_session_id: Option<String>,
        object_id: String,
        depth: i32,
        pierce: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::dom_debugger_get_event_listeners(
            inspector_session_id,
            object_id,
            depth,
            pierce,
        ))
    }

    pub fn finish_dom_debugger_get_event_listeners(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererDomDebuggerEventListenersResolution> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "DOMDebugger.getEventListeners page command",
            "a DOMDebugger event listeners resolution",
            RendererPageReply::DevTools(RendererDevToolsPageReply::DomDebuggerEventListeners(resolution)) => Ok(resolution),
        )
    }

    pub fn start_dom_debugger_configure_event_listener_breakpoint(
        &self,
        inspector_session_id: Option<String>,
        breakpoint: RendererDomDebuggerEventListenerBreakpoint,
        enabled: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DomDebuggerConfigureEventListenerBreakpoint {
                inspector_session_id,
                breakpoint,
                enabled,
            },
        ))
    }

    pub fn start_dom_debugger_configure_xhr_breakpoint(
        &self,
        inspector_session_id: Option<String>,
        breakpoint: RendererDomDebuggerXhrBreakpoint,
        enabled: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DomDebuggerConfigureXhrBreakpoint {
                inspector_session_id,
                breakpoint,
                enabled,
            },
        ))
    }

    pub fn start_dom_debugger_configure_dom_breakpoint(
        &self,
        inspector_session_id: Option<String>,
        frontend_node_id: u32,
        breakpoint_type: String,
        enabled: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DomDebuggerConfigureDomBreakpoint {
                inspector_session_id,
                frontend_node_id,
                breakpoint_type,
                enabled,
            },
        ))
    }

    pub fn finish_dom_debugger_configure_dom_breakpoint(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererDomDebuggerDomBreakpointResolution> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "DOMDebugger DOM breakpoint page command",
            "a DOMDebugger DOM breakpoint resolution",
            RendererPageReply::DevTools(RendererDevToolsPageReply::DomDebuggerDomBreakpoint(resolution)) => Ok(resolution),
        )
    }

    pub fn start_runtime_collect_garbage(&self) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::RuntimeCollectGarbage,
        ))
    }

    pub async fn add_runtime_binding_async(
        &mut self,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<()> {
        self.add_runtime_binding_for_inspector_session_async(
            None,
            name,
            execution_context_name,
            execution_context_id,
        )
        .await
    }

    pub async fn add_runtime_binding_for_inspector_session_async(
        &mut self,
        inspector_session_id: Option<String>,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<()> {
        let command = RendererPageCommand::add_runtime_binding(
            inspector_session_id,
            name.to_owned(),
            execution_context_name.map(str::to_owned),
            execution_context_id,
        );
        self.dispatch_unit_page_command_async(command, "add runtime binding")
            .await
    }

    pub async fn install_runtime_binding_async(
        &mut self,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<()> {
        let command =
            RendererPageCommand::DevTools(RendererDevToolsPageCommand::InstallRuntimeBinding {
                name: name.to_owned(),
                execution_context_name: execution_context_name.map(str::to_owned),
                execution_context_id,
            });
        self.dispatch_unit_page_command_async(command, "install runtime binding")
            .await
    }

    pub fn start_install_runtime_binding(
        &self,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::InstallRuntimeBinding {
                name: name.to_owned(),
                execution_context_name: execution_context_name.map(str::to_owned),
                execution_context_id,
            },
        ))
    }

    pub async fn remove_runtime_binding_async(&mut self, name: &str) -> Result<()> {
        let command = RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::RemoveRuntimeBinding(name.to_owned()),
        );
        self.dispatch_unit_page_command_async(command, "remove runtime binding")
            .await
    }

    pub fn start_remove_runtime_binding(&self, name: &str) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::RemoveRuntimeBinding(name.to_owned()),
        ))
    }

    pub async fn remove_default_runtime_binding_async(&mut self, name: &str) -> Result<()> {
        let command = RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::RemoveDefaultRuntimeBinding(name.to_owned()),
        );
        self.dispatch_unit_page_command_async(command, "remove default runtime binding")
            .await
    }

    pub async fn restore_runtime_protocol_state_async(
        &mut self,
        inspector_session_id: Option<String>,
        session_restore_snapshots: &[RendererInspectorSessionRestoreSnapshot],
        isolated_worlds: &[RuntimeIsolatedWorldDefinition],
        stored_runtime_bindings: &[RuntimeBindingRegistration],
        session_runtime_bindings: &[RuntimeBindingRegistration],
        runtime_enabled: bool,
    ) -> Result<Option<RendererOutputFence>> {
        let mut predecessor = self
            .apply_runtime_protocol_state_for_inspector_session_async(
                inspector_session_id.clone(),
                session_restore_snapshots,
                isolated_worlds,
                stored_runtime_bindings,
                session_runtime_bindings,
            )
            .await?;
        if runtime_enabled
            && let Ok(Some(runtime_predecessor)) = self
                .runtime_enable_concrete_output_for_inspector_session_async(
                    inspector_session_id.as_deref(),
                )
                .await
        {
            predecessor = Some(match predecessor {
                Some(predecessor) => predecessor.latest_in_same_stream(runtime_predecessor),
                None => runtime_predecessor,
            });
        }
        Ok(predecessor)
    }

    pub async fn apply_runtime_protocol_state_async(
        &mut self,
        isolated_worlds: &[RuntimeIsolatedWorldDefinition],
        runtime_bindings: &[RuntimeBindingRegistration],
    ) -> Result<Option<RendererOutputFence>> {
        self.apply_runtime_protocol_state_for_inspector_session_async(
            None,
            &[],
            isolated_worlds,
            runtime_bindings,
            runtime_bindings,
        )
        .await
    }

    pub async fn apply_runtime_protocol_state_for_inspector_session_async(
        &mut self,
        inspector_session_id: Option<String>,
        session_restore_snapshots: &[RendererInspectorSessionRestoreSnapshot],
        isolated_worlds: &[RuntimeIsolatedWorldDefinition],
        stored_runtime_bindings: &[RuntimeBindingRegistration],
        session_runtime_bindings: &[RuntimeBindingRegistration],
    ) -> Result<Option<RendererOutputFence>> {
        let pending =
            self.start_page_command(RendererPageCommand::apply_runtime_protocol_state(
                inspector_session_id.clone(),
                session_restore_snapshots.to_vec(),
                isolated_worlds.to_vec(),
                stored_runtime_bindings.to_vec(),
                session_runtime_bindings.to_vec(),
            ))?;
        let completion = pending.wait().await?;
        let output = self.finish_page_command_turn(completion);
        let (completion, predecessor) = output.into_completion_and_predecessor();
        let (reply, _, _) = completion.into_parts();
        expect_page_reply!(
            reply,
            "apply runtime protocol state page command",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )?;
        Ok(predecessor)
    }

    async fn runtime_enable_concrete_output_for_inspector_session_async(
        &mut self,
        inspector_session_id: Option<&str>,
    ) -> Result<Option<RendererOutputFence>> {
        let pending =
            self.start_runtime_enable_events_for_inspector_session(inspector_session_id)?;
        let completion = pending.wait().await?;
        let output = self.finish_page_command_turn(completion);
        let (completion, predecessor) = output.into_completion_and_predecessor();
        let (reply, _, _) = completion.into_parts();
        expect_page_reply!(
            reply,
            "runtime enable page command",
            "runtime inspector protocol messages reply",
            RendererPageReply::RuntimeInspectorProtocolMessages(_) => Ok(()),
        )?;
        Ok(predecessor)
    }

    pub async fn add_document_start_script_runtime_activity_async(
        &mut self,
        inspector_session_id: Option<&str>,
        script: &DocumentStartScript,
        run_immediately: bool,
    ) -> Result<Option<(i64, bool)>> {
        let pending = self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::AddDocumentStartScriptRuntimeActivity {
                inspector_session_id: inspector_session_id.map(str::to_owned),
                script: script.clone(),
                run_immediately,
            },
        ))?;
        self.finish_document_start_script_result(pending.wait().await?)
    }

    pub fn start_add_document_start_script_runtime_activity(
        &self,
        inspector_session_id: Option<&str>,
        script: &DocumentStartScript,
        run_immediately: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::AddDocumentStartScriptRuntimeActivity {
                inspector_session_id: inspector_session_id.map(str::to_owned),
                script: script.clone(),
                run_immediately,
            },
        ))
    }

    pub async fn remove_document_start_script_by_registry_key_async(
        &mut self,
        registry_key: &str,
    ) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::DevTools(
                RendererDevToolsPageCommand::RemoveDocumentStartScriptByRegistryKey(
                    registry_key.to_owned(),
                ),
            ),
            "remove document-start script by registry key",
        )
        .await
    }

    pub fn start_remove_document_start_script_by_registry_key(
        &self,
        registry_key: &str,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::RemoveDocumentStartScriptByRegistryKey(
                registry_key.to_owned(),
            ),
        ))
    }

    pub async fn set_runtime_binding_state_async(
        &mut self,
        inspector_session_id: Option<String>,
        stored_runtime_bindings: &[RuntimeBindingRegistration],
        session_runtime_bindings: &[RuntimeBindingRegistration],
    ) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::DevTools(RendererDevToolsPageCommand::SetRuntimeBindingState {
                inspector_session_id,
                stored_runtime_bindings: stored_runtime_bindings.to_vec(),
                session_runtime_bindings: session_runtime_bindings.to_vec(),
            }),
            "set runtime binding state",
        )
        .await
    }

    pub fn start_set_runtime_binding_state(
        &self,
        inspector_session_id: Option<String>,
        stored_runtime_bindings: &[RuntimeBindingRegistration],
        session_runtime_bindings: &[RuntimeBindingRegistration],
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::SetRuntimeBindingState {
                inspector_session_id,
                stored_runtime_bindings: stored_runtime_bindings.to_vec(),
                session_runtime_bindings: session_runtime_bindings.to_vec(),
            },
        ))
    }
}

impl Page {
    /// Seals this target's Main/IO DevTools ingress and interrupts active V8.
    ///
    /// `Page.crash` is a terminal renderer IO control in Chromium, not an
    /// ordinary V8 Inspector command. Protocol integrations use this boundary
    /// to retire the Page without waiting behind a session lane.
    #[doc(hidden)]
    pub fn crash_devtools_target_from_io(&self) {
        self.handle.crash_devtools_target_from_io();
    }

    #[cfg(all(test, debug_assertions))]
    pub(crate) async fn panic_wait_for_selector_for_testing(
        &mut self,
        loader: &crate::network::ResourceRequestClient,
    ) -> Result<()> {
        self.wait_for_selector(
            loader,
            "__moli_panic_wait_for_selector_for_testing__",
            std::time::Duration::from_secs(1),
        )
        .await
        .map(|_| ())
    }
}
