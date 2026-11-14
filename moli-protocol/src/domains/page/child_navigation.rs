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
    let navigation_owner = owner.clone();
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
        let frame_id = input.frame_id.clone();
        let loaded = context.with_state(driver, move |state, context| {
            let (loaded, frame, output) =
                state.finish_child_navigation_lifecycle(binding, &frame_id, completed)?;
            context
                .context_mut()
                .consume_renderer_command_turn_output(output);
            Ok((loaded, frame))
        })?;
        let mut frame = match loaded {
            Ok((true, frame)) => frame,
            Ok((false, _)) => {
                return Ok(CommandOutputPlan::error(
                    -32000,
                    "ChildFrameNavigationTimeout",
                ));
            }
            Err(plan) => return Ok(plan),
        };
        if frame.is_none() {
            // Native navigation remains valid before attachment observations
            // reach the protocol index. Query that same Page, without inventing
            // a loader or treating a missing projection as a missing frame.
            let frame_id = input.frame_id.clone();
            let query = driver.with_state(move |state| {
                state.prepare_child_navigation_document_query(&navigation_owner, frame_id)
            })??;
            let mut predecessor = None;
            frame = query.run(driver, &mut predecessor).await?;
            if let Some(predecessor) = predecessor {
                context
                    .context_mut()
                    .set_renderer_output_predecessor(predecessor);
            }
        }
        let Some(frame) = frame else {
            return Ok(CommandOutputPlan::error(-31998, "NoSuchTarget"));
        };
        if Some(frame.loader_id.as_str()) != input.previous_loader_id.as_deref() {
            super::navigation::update_navigation_result_payload_identity(
                &mut input.result_projection,
                &input.frame_id,
                &frame.loader_id,
            );
        }
        input.url = frame.url;
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
