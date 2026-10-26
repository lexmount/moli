use anyhow::Result;

use super::navigation::{ChildFrameNavigationInput, CompletedChildFrameNavigateCommand};
use crate::automation::DevToolsNavigationWait;
use crate::conn::{CommandOwnerScope, ProtocolStateAccess};
use crate::domains::command_output::{CommandOutputBuffer, CommandOutputPlan};
use moli_core::page::{CompletedPageCommand, RendererCommandTurnOutput};

pub(super) async fn finish_navigation(
    access: ProtocolStateAccess,
    owner: CommandOwnerScope,
    completed: Result<CompletedPageCommand>,
    input: ChildFrameNavigationInput,
) -> CompletedChildFrameNavigateCommand {
    let url = input.url.clone();
    let mut predecessor = completed
        .as_ref()
        .ok()
        .and_then(CompletedPageCommand::renderer_output_predecessor);
    let mut outputs = Vec::new();
    let plan = run(
        access,
        owner,
        completed,
        input,
        &mut predecessor,
        &mut outputs,
    )
    .await;
    CompletedChildFrameNavigateCommand::Owned {
        url,
        plan,
        predecessor,
        outputs,
    }
}

async fn run(
    access: ProtocolStateAccess,
    owner: CommandOwnerScope,
    completed: Result<CompletedPageCommand>,
    mut input: ChildFrameNavigationInput,
    predecessor: &mut Option<moli_core::RendererOutputFence>,
    outputs: &mut Vec<RendererCommandTurnOutput>,
) -> CommandOutputPlan {
    let binding = input.activity_binding.clone();
    let wait = input.wait != DevToolsNavigationWait::None;
    let started = match access
        .with_state(move |state| {
            state.start_child_navigation_completion(
                owner,
                binding,
                completed,
                wait,
                super::navigation::CHILD_FRAME_NAVIGATION_LOAD_GATE_TIMEOUT,
            )
        })
        .await
    {
        Ok(started) => started,
        Err(error) => return CommandOutputPlan::error(-32000, error.to_string()),
    };
    if let Some(output) = started.output {
        outputs.push(output);
    }
    let lifecycle = match started.lifecycle {
        Ok(lifecycle) => lifecycle,
        Err(plan) => return plan,
    };
    if let Some(lifecycle) = lifecycle {
        let completed = match lifecycle.wait().await {
            Ok(completed) => completed,
            Err(error) => return CommandOutputPlan::error(-32000, error),
        };
        let binding = input.activity_binding;
        let completed = access
            .with_state(move |state| state.finish_child_navigation_lifecycle(binding, completed))
            .await;
        let (loaded, output) = match completed {
            Ok(Ok(completed)) => completed,
            Ok(Err(plan)) => return plan,
            Err(error) => return CommandOutputPlan::error(-32000, error.to_string()),
        };
        if let Some(cursor) = output.renderer_output_predecessor() {
            *predecessor = Some(cursor);
        }
        outputs.push(output);
        if !loaded {
            return CommandOutputPlan::error(-32000, "ChildFrameNavigationTimeout");
        }
        if input.result_payload.get("url").is_some()
            && let Some(payload) = input.result_payload.as_object_mut()
        {
            payload.insert("url".to_owned(), serde_json::json!(input.url));
        }
    }
    let mut output = CommandOutputBuffer::default();
    output.extend_background_events_after_messages(input.prefix_events);
    output.push_result_after_messages(input.result_payload);
    output.into_plan()
}
