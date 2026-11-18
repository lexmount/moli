use super::CompletedPageCommand;
use super::Page;
use super::PendingPageCommand;
use super::RendererCommandTurnOutput;
use super::RendererRuntimeCommandOutput;
use super::RendererRuntimeInspectorMessage;
use super::RendererRuntimeRealmInfo;
use super::RuntimeConsoleMessageSnapshot;
use super::protocol_support::RuntimeContextRestoreEvent;
use crate::renderer::RendererPageCommand;
use crate::renderer::RendererPageReply;
use crate::renderer::RendererPerformanceMetricSnapshot;
use anyhow::Result;
use anyhow::anyhow;
use serde_json::Value;
use serde_json::json;

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
    pub fn finish_evaluate_runtime_expression(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<serde_json::Value> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(reply, "evaluate expression page command", "a runtime evaluation result reply",
            RendererPageReply::RuntimeEvaluationResult(result) => Ok(result.into_protocol_payload()),)
    }

    pub async fn evaluate_runtime_expression_async(
        &mut self,
        expression: &str,
    ) -> Result<serde_json::Value> {
        self.evaluate_runtime_expression_with_await_async(expression, false)
            .await
    }

    pub fn start_evaluate_runtime_expression(
        &self,
        expression: &str,
        await_promise: bool,
    ) -> Result<PendingPageCommand> {
        self.start_full_page_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: expression.to_owned(),
                await_promise,
            },
        )
    }

    pub async fn evaluate_runtime_expression_with_await_async(
        &mut self,
        expression: &str,
        await_promise: bool,
    ) -> Result<serde_json::Value> {
        let pending = self.start_evaluate_runtime_expression(expression, await_promise)?;
        self.finish_evaluate_runtime_expression(pending.wait().await?)
    }

    pub fn start_evaluate_runtime_expression_without_navigation_follow(
        &self,
        expression: &str,
        await_promise: bool,
    ) -> Result<PendingPageCommand> {
        self.start_full_page_command(RendererPageCommand::EvaluateExpression {
            expression: expression.to_owned(),
            await_promise,
        })
    }

    pub async fn evaluate_runtime_expression_without_navigation_follow_with_await_async(
        &mut self,
        expression: &str,
        await_promise: bool,
    ) -> Result<serde_json::Value> {
        let pending = self.start_evaluate_runtime_expression_without_navigation_follow(
            expression,
            await_promise,
        )?;
        self.finish_evaluate_runtime_expression(pending.wait().await?)
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

    pub fn start_default_execution_context_id(&self) -> Result<PendingPageCommand> {
        self.start_full_page_command(RendererPageCommand::DefaultExecutionContextId)
    }

    pub fn finish_default_execution_context_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<i64>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(reply, "default execution context page command", "an optional execution context reply",
            RendererPageReply::OptionalExecutionContextId(id) => Ok(id),)
    }

    pub async fn default_execution_context_id_async(&mut self) -> Result<Option<i64>> {
        let pending = self.start_default_execution_context_id()?;
        self.finish_default_execution_context_id(pending.wait().await?)
    }

    pub fn start_default_or_initial_execution_context_id(&self) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DefaultOrInitialExecutionContextId)
    }

    pub fn finish_default_or_initial_execution_context_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<i64>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "default or initial execution context page command",
            "an optional execution context reply",
            RendererPageReply::OptionalExecutionContextId(id) => Ok(id),
        )
    }

    pub async fn default_or_initial_execution_context_id_async(&mut self) -> Result<Option<i64>> {
        let pending = self.start_default_or_initial_execution_context_id()?;
        self.finish_default_or_initial_execution_context_id(pending.wait().await?)
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

    pub fn finish_inspector_execution_context_id_for_isolated_context(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<i64>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(reply, "inspector isolated context id page command", "an optional execution context reply",
            RendererPageReply::OptionalExecutionContextId(id) => Ok(id),)
    }

    pub fn finish_runtime_realm_inventory(
        &mut self,
        completed: CompletedPageCommand,
    ) -> Result<Vec<RendererRuntimeRealmInfo>> {
        let reply = self.finish_page_command(completed);
        expect_page_reply!(
            reply,
            "runtime realm inventory page command",
            "runtime realm inventory",
            RendererPageReply::RuntimeRealmInventory(realms) => Ok(realms),
        )
    }

    pub async fn runtime_realm_inventory_async(&mut self) -> Result<Vec<RendererRuntimeRealmInfo>> {
        let pending = self.start_runtime_realm_inventory()?;
        let completed = pending.wait().await?;
        self.finish_runtime_realm_inventory(completed)
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

    pub fn start_child_default_execution_context_id_for_frame_id(
        &self,
        frame_id: &str,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::ChildDefaultExecutionContextIdForFrameId(frame_id.to_owned()),
        )
    }

    pub fn finish_child_default_execution_context_id_for_frame_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<i64>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "child default execution context id page command",
            "an optional execution context id reply",
            RendererPageReply::OptionalExecutionContextId(execution_context_id) => Ok(execution_context_id),
        )
    }

    pub async fn child_default_execution_context_id_for_frame_id_async(
        &mut self,
        frame_id: &str,
    ) -> Result<Option<i64>> {
        let pending = self.start_child_default_execution_context_id_for_frame_id(frame_id)?;
        self.finish_child_default_execution_context_id_for_frame_id(pending.wait().await?)
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
}

impl Page {
    pub fn runtime_inspector_pause_active(&self) -> bool {
        self.handle.runtime_inspector_pause_active()
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
}

impl Page {
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

    pub async fn runtime_console_messages_with_context_async(
        &mut self,
    ) -> Result<Vec<RuntimeConsoleMessageSnapshot>> {
        let pending = self.start_runtime_console_messages_with_context()?;
        let completion = pending.wait().await?;
        self.finish_runtime_console_messages_with_context(completion)
    }

    pub fn start_runtime_console_messages_with_context(&self) -> Result<PendingPageCommand> {
        self.start_full_page_command(RendererPageCommand::RuntimeConsoleMessagesWithContext)
    }

    pub fn finish_runtime_console_messages_with_context(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Vec<RuntimeConsoleMessageSnapshot>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "runtime console messages with context page command",
            "runtime console snapshots",
            RendererPageReply::RuntimeConsoleMessageSnapshots(messages) => Ok(messages),
        )
    }
}

impl Page {
    pub fn cached_performance_metric_snapshot(&self) -> RendererPerformanceMetricSnapshot {
        self.page_state.performance_metric_snapshot().clone()
    }
}

impl Page {
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
}

impl Page {
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
}

impl Page {
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
