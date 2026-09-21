use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use super::Page;
use super::RuntimeConsoleMessageSnapshot;
use super::protocol_support::{
    DocumentStartScript, RuntimeBindingRegistration, RuntimeContextRestoreEvent,
    RuntimeIsolatedWorldDefinition,
};
use super::{
    CompletedPageCommand, PendingDevToolsIoCommandDispatch, PendingPageCommand,
    PendingRuntimeInspectorCommandDispatch, RendererCommandTurnOutput,
    RendererInspectorSessionRestoreSnapshot, RendererRuntimeCommandOutput,
    RendererRuntimeInspectorMessage, RendererRuntimeRealmInfo,
};
use crate::RendererOutputFence;
use crate::renderer::{
    RendererDomDebuggerDomBreakpointResolution, RendererDomDebuggerEventListenerBreakpoint,
    RendererDomDebuggerEventListenersResolution, RendererDomDebuggerXhrBreakpoint,
    RendererInspectorCommandEnvelope, RendererInspectorCommandRoute,
    RendererInspectorIngressTicket, RendererPageCommand, RendererPageReply,
    RendererPerformanceMetricSnapshot, RendererRuntimeHeapUsage,
    RendererRuntimeInspectorResponseSender,
};

fn dedupe_runtime_context_created_events(events: &mut Vec<RuntimeContextRestoreEvent>) {
    let mut seen = Vec::new();
    events.retain(|event| {
        let RuntimeContextRestoreEvent::Created(event) = event else {
            return true;
        };
        let key = (event.context_id, event.realm_id.clone());
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
}

impl Page {
    pub async fn evaluate_runtime_expression_async(
        &mut self,
        expression: &str,
    ) -> Result<serde_json::Value> {
        self.evaluate_runtime_expression_with_await_async(expression, false)
            .await
    }

    pub async fn evaluate_runtime_expression_with_await_async(
        &mut self,
        expression: &str,
        await_promise: bool,
    ) -> Result<serde_json::Value> {
        let command = RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
            expression: expression.to_owned(),
            await_promise,
        };
        let reply = self.dispatch_page_command_async(command).await?;
        expect_page_reply!(
            reply,
            "evaluate expression page command",
            "a runtime evaluation result reply",
            RendererPageReply::RuntimeEvaluationResult(result) => Ok(result.into_protocol_payload()),
        )
    }

    pub async fn evaluate_runtime_expression_without_navigation_follow_with_await_async(
        &mut self,
        expression: &str,
        await_promise: bool,
    ) -> Result<serde_json::Value> {
        let command = RendererPageCommand::EvaluateExpression {
            expression: expression.to_owned(),
            await_promise,
        };
        let reply = self.dispatch_page_command_async(command).await?;
        expect_page_reply!(
            reply,
            "evaluate expression page command",
            "a runtime evaluation result reply",
            RendererPageReply::RuntimeEvaluationResult(result) => Ok(result.into_protocol_payload()),
        )
    }

    pub async fn evaluate_runtime_expression_by_value_async(
        &mut self,
        expression: &str,
    ) -> Result<serde_json::Value> {
        let command = RendererPageCommand::EvaluateExpressionByValue {
            expression: expression.to_owned(),
        };
        let reply = self.dispatch_page_command_async(command).await?;
        expect_page_reply!(
            reply,
            "evaluate expression by value page command",
            "a runtime evaluation result reply",
            RendererPageReply::RuntimeEvaluationResult(result) => Ok(result.into_protocol_payload()),
        )
    }

    pub async fn evaluate_runtime_expression_in_execution_context_with_await_async(
        &mut self,
        execution_context_id: i64,
        expression: &str,
        await_promise: bool,
    ) -> Result<serde_json::Value> {
        let command =
            RendererPageCommand::EvaluateExpressionInExecutionContextAndFollowPendingNavigation {
                execution_context_id,
                expression: expression.to_owned(),
                await_promise,
            };
        let reply = self.dispatch_page_command_async(command).await?;
        expect_page_reply!(
            reply,
            "evaluate-in-context page command",
            "a runtime evaluation result reply",
            RendererPageReply::RuntimeEvaluationResult(result) => Ok(result.into_protocol_payload()),
        )
    }

    pub async fn evaluate_runtime_expression_in_execution_context_without_navigation_follow_with_await_async(
        &mut self,
        execution_context_id: i64,
        expression: &str,
        await_promise: bool,
    ) -> Result<serde_json::Value> {
        let command = RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id,
            expression: expression.to_owned(),
            await_promise,
        };
        let reply = self.dispatch_page_command_async(command).await?;
        expect_page_reply!(
            reply,
            "evaluate-in-context page command",
            "a runtime evaluation result reply",
            RendererPageReply::RuntimeEvaluationResult(result) => Ok(result.into_protocol_payload()),
        )
    }

    pub async fn default_execution_context_id_async(&mut self) -> Result<Option<i64>> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::DefaultExecutionContextId)
            .await?;
        expect_page_reply!(
            reply,
            "default execution context page command",
            "an optional execution context reply",
            RendererPageReply::OptionalExecutionContextId(id) => Ok(id),
        )
    }

    pub async fn default_or_initial_execution_context_id_async(&mut self) -> Result<Option<i64>> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::DefaultOrInitialExecutionContextId)
            .await?;
        expect_page_reply!(
            reply,
            "default or initial execution context page command",
            "an optional execution context reply",
            RendererPageReply::OptionalExecutionContextId(id) => Ok(id),
        )
    }

    pub async fn has_isolated_world_named_async(&mut self, name: &str) -> Result<bool> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::HasIsolatedWorldNamed {
                name: name.to_owned(),
                frame_id: None,
            })
            .await?;
        expect_page_reply!(
            reply,
            "has isolated world name page command",
            "a bool reply",
            RendererPageReply::Bool(value) => Ok(value),
        )
    }

    pub async fn has_isolated_world_named_for_frame_async(
        &mut self,
        frame_id: &str,
        name: &str,
    ) -> Result<bool> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::HasIsolatedWorldNamed {
                name: name.to_owned(),
                frame_id: Some(frame_id.to_owned()),
            })
            .await?;
        expect_page_reply!(
            reply,
            "has isolated world name page command",
            "a bool reply",
            RendererPageReply::Bool(value) => Ok(value),
        )
    }

    pub fn start_has_isolated_world_named(
        &self,
        name: &str,
        frame_id: Option<&str>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::HasIsolatedWorldNamed {
            name: name.to_owned(),
            frame_id: frame_id.map(str::to_owned),
        })
    }

    pub fn finish_has_isolated_world_named(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<bool> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "has isolated world name page command",
            "a bool reply",
            RendererPageReply::Bool(value) => Ok(value),
        )
    }

    pub async fn has_isolated_execution_context_id_async(
        &mut self,
        execution_context_id: i64,
    ) -> Result<bool> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::HasIsolatedExecutionContextId(
                execution_context_id,
            ))
            .await?;
        expect_page_reply!(
            reply,
            "has isolated context page command",
            "a bool reply",
            RendererPageReply::Bool(value) => Ok(value),
        )
    }

    pub async fn ensure_isolated_worlds_attached_to_inspector_async(&mut self) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::EnsureIsolatedWorldsAttachedToInspector,
            "ensure isolated worlds attached to inspector",
        )
        .await
    }

    pub async fn inspector_execution_context_id_for_isolated_context_async(
        &mut self,
        execution_context_id: i64,
    ) -> Result<Option<i64>> {
        let reply = self
            .dispatch_page_command_async(
                RendererPageCommand::InspectorExecutionContextIdForIsolatedContext(
                    execution_context_id,
                ),
            )
            .await?;
        expect_page_reply!(
            reply,
            "inspector isolated context id page command",
            "an optional execution context reply",
            RendererPageReply::OptionalExecutionContextId(id) => Ok(id),
        )
    }

    pub async fn isolated_execution_context_id_for_inspector_context_async(
        &mut self,
        execution_context_id: i64,
    ) -> Result<Option<i64>> {
        let reply = self
            .dispatch_page_command_async(
                RendererPageCommand::IsolatedExecutionContextIdForInspectorContext(
                    execution_context_id,
                ),
            )
            .await?;
        expect_page_reply!(
            reply,
            "synthetic isolated context id page command",
            "an optional execution context reply",
            RendererPageReply::OptionalExecutionContextId(id) => Ok(id),
        )
    }

    pub async fn runtime_realm_inventory_async(&mut self) -> Result<Vec<RendererRuntimeRealmInfo>> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::RuntimeRealmInventory)
            .await?;
        expect_page_reply!(
            reply,
            "runtime realm inventory page command",
            "runtime realm inventory",
            RendererPageReply::RuntimeRealmInventory(realms) => Ok(realms),
        )
    }

    pub async fn live_child_default_runtime_realm_inventory_async(
        &mut self,
    ) -> Result<Vec<RendererRuntimeRealmInfo>> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::LiveChildDefaultRuntimeRealmInventory)
            .await?;
        expect_page_reply!(
            reply,
            "child default runtime realm inventory page command",
            "runtime realm inventory",
            RendererPageReply::RuntimeRealmInventory(realms) => Ok(realms),
        )
    }

    pub async fn child_frame_id_for_default_execution_context_id_async(
        &mut self,
        execution_context_id: i64,
    ) -> Result<Option<String>> {
        let reply = self
            .dispatch_page_command_async(
                RendererPageCommand::ChildFrameIdForDefaultExecutionContextId(execution_context_id),
            )
            .await?;
        expect_page_reply!(
            reply,
            "child default context frame id page command",
            "an optional string reply",
            RendererPageReply::OptionalString(frame_id) => Ok(frame_id),
        )
    }

    pub async fn child_default_execution_context_id_for_frame_id_async(
        &mut self,
        frame_id: &str,
    ) -> Result<Option<i64>> {
        let reply = self
            .dispatch_page_command_async(
                RendererPageCommand::ChildDefaultExecutionContextIdForFrameId(frame_id.to_owned()),
            )
            .await?;
        expect_page_reply!(
            reply,
            "child default execution context id page command",
            "an optional execution context id reply",
            RendererPageReply::OptionalExecutionContextId(execution_context_id) => {
                Ok(execution_context_id)
            },
        )
    }

    pub fn start_child_frame_id_for_default_execution_context_id(
        &self,
        execution_context_id: i64,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::ChildFrameIdForDefaultExecutionContextId(execution_context_id),
        )
    }

    pub fn finish_child_frame_id_for_default_execution_context_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<String>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "child default context frame id page command",
            "an optional string reply",
            RendererPageReply::OptionalString(frame_id) => Ok(frame_id),
        )
    }

    pub async fn create_isolated_world_async(
        &mut self,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<i64> {
        let command = RendererPageCommand::CreateIsolatedWorld {
            name: name.to_owned(),
            grant_universal_access,
            frame_id: None,
        };
        let reply = self.dispatch_page_command_async(command).await?;
        expect_page_reply!(
            reply,
            "create isolated world page command",
            "an execution context reply",
            RendererPageReply::ExecutionContextId(id) => Ok(id),
        )
    }

    pub async fn create_isolated_world_for_frame_async(
        &mut self,
        frame_id: &str,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<i64> {
        let command = RendererPageCommand::CreateIsolatedWorld {
            name: name.to_owned(),
            grant_universal_access,
            frame_id: Some(frame_id.to_owned()),
        };
        let reply = self.dispatch_page_command_async(command).await?;
        expect_page_reply!(
            reply,
            "create isolated world page command",
            "an execution context reply",
            RendererPageReply::ExecutionContextId(id) => Ok(id),
        )
    }

    pub fn start_create_isolated_world(
        &self,
        name: &str,
        grant_universal_access: bool,
        frame_id: Option<&str>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::CreateIsolatedWorld {
            name: name.to_owned(),
            grant_universal_access,
            frame_id: frame_id.map(str::to_owned),
        })
    }

    pub fn start_create_isolated_world_runtime_activity_capturing_runtime_inspector_messages(
        &self,
        inspector_session_id: Option<&str>,
        frame_id: Option<&str>,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::CreateIsolatedWorldRuntimeActivity {
            inspector_session_id: inspector_session_id.map(str::to_owned),
            frame_id: frame_id.map(str::to_owned),
            name: name.to_owned(),
            grant_universal_access,
        })
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
        let RendererPageReply::ExecutionContextId(execution_context_id) =
            output.completion().reply()
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

    pub fn runtime_inspector_pause_active(&self) -> bool {
        self.handle.runtime_inspector_pause_active()
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

    pub fn finish_runtime_protocol_message(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        let output = self.finish_runtime_protocol_message_command_turn(completion)?;
        let (completion, _renderer_output_predecessor) = output.into_completion_and_predecessor();
        let (reply, _, _) = completion.into_parts();
        Self::decode_runtime_inspector_protocol_messages_page_reply(
            reply,
            "runtime protocol page command",
        )
    }

    pub fn finish_runtime_protocol_message_command_turn(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererCommandTurnOutput> {
        self.replace_page_state(completion.output().completion().page_state().clone());
        completion.into_runtime_protocol_message_command_turn()
    }

    pub async fn prepare_runtime_protocol_message_async(
        &mut self,
        action: &str,
        raw_json: &str,
    ) -> Result<String> {
        let context_param_name = match action {
            "evaluate" => Some("contextId"),
            "callFunctionOn" => Some("executionContextId"),
            _ => None,
        };
        let Some(context_param_name) = context_param_name else {
            return Ok(raw_json.to_owned());
        };

        let mut message: Value = serde_json::from_str(raw_json)?;
        let params = message
            .as_object_mut()
            .and_then(|message| {
                message
                    .entry("params")
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
            })
            .ok_or_else(|| anyhow!("runtime protocol params must be an object"))?;

        if !params.contains_key(context_param_name)
            && !params.contains_key("objectId")
            && let Some(execution_context_id) = self.default_execution_context_id_async().await?
        {
            params.insert(context_param_name.to_owned(), json!(execution_context_id));
        }

        Ok(serde_json::to_string(&message)?)
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

    pub fn finish_runtime_enable_events(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        Ok(self
            .finish_runtime_enable_output(completion)?
            .into_messages())
    }

    #[doc(hidden)]
    pub fn finish_runtime_enable_output(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererRuntimeCommandOutput> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "runtime enable page command",
            "runtime inspector protocol messages reply",
            RendererPageReply::RuntimeInspectorProtocolMessages(output) => Ok(output),
        )
    }

    pub fn finish_runtime_enable_context_restore_events(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Vec<RuntimeContextRestoreEvent>> {
        let mut events = self
            .finish_runtime_enable_events(completion)?
            .into_iter()
            .filter_map(|message| match message {
                RendererRuntimeInspectorMessage::RuntimeContext(event) => Some(event),
                RendererRuntimeInspectorMessage::Protocol(_) => None,
            })
            .collect();
        dedupe_runtime_context_created_events(&mut events);
        Ok(events)
    }

    pub async fn detach_runtime_inspector_session_async(
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

    pub async fn runtime_console_messages_with_context_async(
        &mut self,
    ) -> Result<Vec<RuntimeConsoleMessageSnapshot>> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::RuntimeConsoleMessagesWithContext)
            .await?;
        expect_page_reply!(
            reply,
            "runtime console messages with context page command",
            "runtime console snapshots",
            RendererPageReply::RuntimeConsoleMessageSnapshots(messages) => Ok(messages),
        )
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
        self.start_page_command(RendererPageCommand::PerformanceMetricSnapshot)
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

    pub fn cached_performance_metric_snapshot(&self) -> RendererPerformanceMetricSnapshot {
        self.page_state.performance_metric_snapshot().clone()
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
            RendererPageReply::PerformanceMetricSnapshot(snapshot) => Ok(*snapshot),
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
            RendererPageReply::DomDebuggerEventListeners(resolution) => Ok(resolution),
        )
    }

    pub fn start_dom_debugger_configure_event_listener_breakpoint(
        &self,
        inspector_session_id: Option<String>,
        breakpoint: RendererDomDebuggerEventListenerBreakpoint,
        enabled: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::DomDebuggerConfigureEventListenerBreakpoint {
                inspector_session_id,
                breakpoint,
                enabled,
            },
        )
    }

    pub fn start_dom_debugger_configure_xhr_breakpoint(
        &self,
        inspector_session_id: Option<String>,
        breakpoint: RendererDomDebuggerXhrBreakpoint,
        enabled: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DomDebuggerConfigureXhrBreakpoint {
            inspector_session_id,
            breakpoint,
            enabled,
        })
    }

    pub fn start_dom_debugger_configure_dom_breakpoint(
        &self,
        inspector_session_id: Option<String>,
        frontend_node_id: u32,
        breakpoint_type: String,
        enabled: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DomDebuggerConfigureDomBreakpoint {
            inspector_session_id,
            frontend_node_id,
            breakpoint_type,
            enabled,
        })
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
            RendererPageReply::DomDebuggerDomBreakpoint(resolution) => Ok(resolution),
        )
    }

    pub fn start_runtime_collect_garbage(&self) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::RuntimeCollectGarbage)
    }

    pub fn finish_runtime_collect_garbage(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "runtime collect garbage page command",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
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
        let command = RendererPageCommand::InstallRuntimeBinding {
            name: name.to_owned(),
            execution_context_name: execution_context_name.map(str::to_owned),
            execution_context_id,
        };
        self.dispatch_unit_page_command_async(command, "install runtime binding")
            .await
    }

    pub fn start_install_runtime_binding(
        &self,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::InstallRuntimeBinding {
            name: name.to_owned(),
            execution_context_name: execution_context_name.map(str::to_owned),
            execution_context_id,
        })
    }

    pub async fn remove_runtime_binding_async(&mut self, name: &str) -> Result<()> {
        let command = RendererPageCommand::RemoveRuntimeBinding(name.to_owned());
        self.dispatch_unit_page_command_async(command, "remove runtime binding")
            .await
    }

    pub fn start_remove_runtime_binding(&self, name: &str) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::RemoveRuntimeBinding(name.to_owned()))
    }

    pub async fn remove_default_runtime_binding_async(&mut self, name: &str) -> Result<()> {
        let command = RendererPageCommand::RemoveDefaultRuntimeBinding(name.to_owned());
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

    pub async fn run_page_surface_override_script_async(&mut self, source: &str) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::RunPageSurfaceOverrideScript {
                source: source.to_owned(),
            },
            "run page surface override script",
        )
        .await
    }

    pub fn finish_document_start_script_result(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<(i64, bool)>> {
        let (result, _) = self.finish_document_start_script_result_command_turn(completion)?;
        Ok(result)
    }

    pub fn finish_document_start_script_result_command_turn(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<(Option<(i64, bool)>, RendererCommandTurnOutput)> {
        let output = self.finish_page_command_turn(completion);
        let RendererPageReply::DocumentStartScriptResult(result) = output.completion().reply()
        else {
            return Err(anyhow!(
                "run document-start script page command returned an unexpected renderer reply"
            ));
        };
        Ok((*result, output))
    }

    pub async fn add_document_start_script_runtime_activity_async(
        &mut self,
        inspector_session_id: Option<&str>,
        script: &DocumentStartScript,
        run_immediately: bool,
    ) -> Result<Option<(i64, bool)>> {
        let pending =
            self.start_page_command(RendererPageCommand::AddDocumentStartScriptRuntimeActivity {
                inspector_session_id: inspector_session_id.map(str::to_owned),
                script: script.clone(),
                run_immediately,
            })?;
        self.finish_document_start_script_result(pending.wait().await?)
    }

    pub fn start_add_document_start_script_runtime_activity(
        &self,
        inspector_session_id: Option<&str>,
        script: &DocumentStartScript,
        run_immediately: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::AddDocumentStartScriptRuntimeActivity {
            inspector_session_id: inspector_session_id.map(str::to_owned),
            script: script.clone(),
            run_immediately,
        })
    }

    pub async fn remove_document_start_script_by_registry_key_async(
        &mut self,
        registry_key: &str,
    ) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::RemoveDocumentStartScriptByRegistryKey(registry_key.to_owned()),
            "remove document-start script by registry key",
        )
        .await
    }

    pub fn start_remove_document_start_script_by_registry_key(
        &self,
        registry_key: &str,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::RemoveDocumentStartScriptByRegistryKey(
            registry_key.to_owned(),
        ))
    }

    pub async fn set_runtime_binding_state_async(
        &mut self,
        inspector_session_id: Option<String>,
        stored_runtime_bindings: &[RuntimeBindingRegistration],
        session_runtime_bindings: &[RuntimeBindingRegistration],
    ) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::SetRuntimeBindingState {
                inspector_session_id,
                stored_runtime_bindings: stored_runtime_bindings.to_vec(),
                session_runtime_bindings: session_runtime_bindings.to_vec(),
            },
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
        self.start_page_command(RendererPageCommand::SetRuntimeBindingState {
            inspector_session_id,
            stored_runtime_bindings: stored_runtime_bindings.to_vec(),
            session_runtime_bindings: session_runtime_bindings.to_vec(),
        })
    }

    pub fn finish_unit_runtime_page_command(
        &mut self,
        completion: CompletedPageCommand,
        operation: &str,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            operation,
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }
}
