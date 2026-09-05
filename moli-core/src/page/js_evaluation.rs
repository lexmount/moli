use anyhow::{Result, anyhow};

use super::Page;
use super::RuntimeConsoleMessageSnapshot;
use super::{
    CompletedPageCommand, PendingPageCommand, RendererCommandTurnOutput, RendererRuntimeRealmInfo,
};
use crate::renderer::{
    RendererDomDebuggerDomBreakpointResolution, RendererDomDebuggerEventListenersResolution,
    RendererPageCommand, RendererPageReply, RendererPerformanceMetricSnapshot,
    RendererRuntimeHeapUsage,
};

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

    pub fn cached_performance_metric_snapshot(&self) -> RendererPerformanceMetricSnapshot {
        self.page_state.performance_metric_snapshot().clone()
    }

    pub async fn add_runtime_binding_async(
        &mut self,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<()> {
        let command = RendererPageCommand::add_runtime_binding(
            None,
            name.to_owned(),
            execution_context_name.map(str::to_owned),
            execution_context_id,
        );
        self.dispatch_unit_page_command_async(command, "add runtime binding")
            .await
    }

    pub async fn remove_runtime_binding_async(&mut self, name: &str) -> Result<()> {
        let command = RendererPageCommand::RemoveRuntimeBinding(name.to_owned());
        self.dispatch_unit_page_command_async(command, "remove runtime binding")
            .await
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
}

// Decoding a frozen inspection reply requires no Browser Page residence.
impl CompletedPageCommand {
    pub fn finish_create_isolated_world_command_turn(
        self,
    ) -> Result<(i64, RendererCommandTurnOutput)> {
        let output = self.into_output();
        let RendererPageReply::ExecutionContextId(execution_context_id) =
            output.completion().reply()
        else {
            return Err(anyhow!(
                "create isolated world page command returned an unexpected renderer reply"
            ));
        };
        Ok((*execution_context_id, output))
    }

    pub fn finish_dom_debugger_get_event_listeners(
        self,
    ) -> Result<RendererDomDebuggerEventListenersResolution> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "DOMDebugger.getEventListeners page command",
            "a DOMDebugger event listeners resolution",
            RendererPageReply::DomDebuggerEventListeners(resolution) => Ok(resolution),
        )
    }

    pub fn finish_dom_debugger_configure_dom_breakpoint(
        self,
    ) -> Result<RendererDomDebuggerDomBreakpointResolution> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "DOMDebugger DOM breakpoint page command",
            "a DOMDebugger DOM breakpoint resolution",
            RendererPageReply::DomDebuggerDomBreakpoint(resolution) => Ok(resolution),
        )
    }

    pub fn finish_document_start_script_result_command_turn(
        self,
    ) -> Result<(Option<(i64, bool)>, RendererCommandTurnOutput)> {
        let output = self.into_output();
        let RendererPageReply::DocumentStartScriptResult(result) = output.completion().reply()
        else {
            return Err(anyhow!(
                "run document-start script page command returned an unexpected renderer reply"
            ));
        };
        Ok((*result, output))
    }

    pub fn finish_unit_runtime_page_command(self, operation: &str) -> Result<()> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            operation,
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub fn finish_runtime_optional_execution_context_id(self) -> Result<Option<i64>> {
        let reply = self.into_reply();
        expect_page_reply!(reply, "runtime execution context lookup", "an optional execution context reply",
            RendererPageReply::OptionalExecutionContextId(id) => Ok(id),
        )
    }

    pub fn finish_has_isolated_execution_context_id(self) -> Result<bool> {
        let reply = self.into_reply();
        expect_page_reply!(reply, "runtime isolated context lookup", "a bool reply",
            RendererPageReply::Bool(value) => Ok(value),
        )
    }

    pub fn finish_runtime_realm_inventory(self) -> Result<Vec<RendererRuntimeRealmInfo>> {
        let reply = self.into_reply();
        expect_page_reply!(reply, "runtime realm inventory", "runtime realm inventory",
            RendererPageReply::RuntimeRealmInventory(realms) => Ok(realms),
        )
    }

    pub fn finish_child_frame_id_for_default_execution_context_id(self) -> Result<Option<String>> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "child default context frame id page command",
            "an optional string reply",
            RendererPageReply::OptionalString(frame_id) => Ok(frame_id),
        )
    }
}
