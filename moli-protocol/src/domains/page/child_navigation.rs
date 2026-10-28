use anyhow::Result;

use super::navigation::CompletedChildFrameNavigateCommand;
use crate::automation::DevToolsNavigationWait;
use crate::conn::{CommandOwnerScope, CommandWorkflowContext, ProtocolWorkflowDriver};
use crate::domains::command_output::{CommandOutputBuffer, CommandOutputPlan};

pub(super) async fn finish_navigation(
    driver: &mut ProtocolWorkflowDriver,
    context: &mut CommandWorkflowContext,
    owner: CommandOwnerScope,
    completed: CompletedChildFrameNavigateCommand,
) -> Result<CommandOutputPlan> {
    if let Some(result) = completed.input.result_projection.automation_result() {
        result.observe_child(&completed);
    }
    let CompletedChildFrameNavigateCommand {
        mut input,
        completed,
    } = completed;
    let binding = input.activity_binding.clone();
    let wait = input.wait != DevToolsNavigationWait::None;
    let lifecycle = context.with_state(driver, move |state, context| {
        let started = state.start_child_navigation_completion(
            owner,
            binding,
            completed,
            wait,
            super::navigation::CHILD_FRAME_NAVIGATION_LOAD_GATE_TIMEOUT,
        );
        if let Some(output) = started.output {
            context
                .context_mut()
                .consume_renderer_command_turn_output(output);
        }
        started.lifecycle
    })?;
    let lifecycle = match lifecycle {
        Ok(lifecycle) => lifecycle,
        Err(plan) => return Ok(plan),
    };
    if let Some(lifecycle) = lifecycle {
        let completed = match lifecycle.wait().await {
            Ok(completed) => completed,
            Err(error) => return Ok(CommandOutputPlan::error(-32000, error)),
        };
        let binding = input.activity_binding;
        let loaded = context.with_state(driver, move |state, context| {
            let (loaded, output) = state.finish_child_navigation_lifecycle(binding, completed)?;
            context
                .context_mut()
                .consume_renderer_command_turn_output(output);
            Ok(loaded)
        })?;
        match loaded {
            Ok(true) => {}
            Ok(false) => {
                return Ok(CommandOutputPlan::error(
                    -32000,
                    "ChildFrameNavigationTimeout",
                ));
            }
            Err(plan) => return Ok(plan),
        }
        if input.result_projection.payload().get("url").is_some()
            && let Some(payload) = input.result_projection.payload_mut().as_object_mut()
        {
            payload.insert("url".to_owned(), serde_json::json!(input.url));
        }
    }
    let mut output = CommandOutputBuffer::default();
    output.extend_background_events_after_messages(input.prefix_events);
    output.push_result_after_messages(input.result_projection.into_payload());
    Ok(output.into_plan())
}
