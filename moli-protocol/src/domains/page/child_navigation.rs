use anyhow::Result;

use super::navigation::CompletedChildFrameNavigateCommand;
use crate::automation::{DevToolsNavigationWait, FrontendProtocol};
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
    let wait_for_commit = input.result_projection.protocol() == FrontendProtocol::Cdp;
    let (lifecycle, navigation) = context.with_state(driver, move |state, context| {
        let started = state.start_child_navigation_completion(
            owner,
            binding,
            completed,
            wait,
            wait_for_commit,
            super::navigation::CHILD_FRAME_NAVIGATION_LOAD_GATE_TIMEOUT,
        );
        if let Some(output) = started.output {
            context
                .context_mut()
                .consume_renderer_command_turn_output(output);
        }
        (started.lifecycle, started.navigation)
    })?;
    let (navigation_id, lifecycle) = match lifecycle {
        Ok(lifecycle) => lifecycle,
        Err(plan) => return Ok(plan),
    };
    if let Some(navigation) = navigation.as_ref().filter(|_| wait_for_commit) {
        super::navigation::update_navigation_result_payload_identity(
            &mut input.result_projection,
            &input.frame_id,
            navigation.loader_id(),
        );
    }
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
        if let Some(navigation) = navigation.filter(|_| wait_for_commit) {
            match navigation.outcome() {
                Some(moli_core::page::RendererChildFrameNavigationOutcome::Committed) => {}
                Some(outcome) => {
                    use moli_core::page::RendererChildFrameNavigationOutcome;
                    let (error_text, is_download) = match outcome {
                        RendererChildFrameNavigationOutcome::Aborted => {
                            (moli_fetch::NET_ERR_ABORTED_ERROR_TEXT.to_owned(), false)
                        }
                        RendererChildFrameNavigationOutcome::Download => {
                            (moli_fetch::NET_ERR_ABORTED_ERROR_TEXT.to_owned(), true)
                        }
                        RendererChildFrameNavigationOutcome::Failed(error) => (error, false),
                        RendererChildFrameNavigationOutcome::Committed => {
                            unreachable!("committed navigation handled above")
                        }
                    };
                    if let Some(payload) = input.result_projection.payload_mut().as_object_mut() {
                        payload.insert("errorText".to_owned(), serde_json::json!(error_text));
                        payload.insert("isDownload".to_owned(), serde_json::json!(is_download));
                    }
                }
                None => {
                    return Ok(CommandOutputPlan::error(
                        -32000,
                        "ChildFrameNavigationTimeout",
                    ));
                }
            }
        } else {
            // Read the native generation and committed Document together. The
            // current frame's loader alone cannot identify which command committed.
            let frame_id = input.frame_id.clone();
            let query = driver.with_state(move |state| {
                state.prepare_child_navigation_document_query(&navigation_owner, frame_id)
            })??;
            let mut predecessor = None;
            let frame = query.run(driver, &mut predecessor).await?;
            if let Some(predecessor) = predecessor {
                context
                    .context_mut()
                    .set_renderer_output_predecessor(predecessor);
            }
            let Some(frame) = frame else {
                return Ok(CommandOutputPlan::error(-31998, "NoSuchTarget"));
            };
            if frame.navigation_id != Some(navigation_id) {
                let Some(result) = super::navigation::aborted_cdp_navigation_result_payload(
                    &input.result_projection,
                    &input.frame_id,
                ) else {
                    return Ok(CommandOutputPlan::error(-32000, "Navigation aborted"));
                };
                let mut output = CommandOutputBuffer::default();
                output.extend_background_events_after_messages(input.prefix_events);
                output.push_result_after_messages(result);
                return Ok(output.into_plan());
            }
            super::navigation::update_navigation_result_payload_identity(
                &mut input.result_projection,
                &input.frame_id,
                &frame.loader_id,
            );
            input.url = frame.url;
            if input.result_projection.payload().get("url").is_some()
                && let Some(payload) = input.result_projection.payload_mut().as_object_mut()
            {
                payload.insert("url".to_owned(), serde_json::json!(input.url));
            }
        }
    }
    let mut output = CommandOutputBuffer::default();
    output.extend_background_events_after_messages(input.prefix_events);
    output.push_result_after_messages(input.result_projection.into_payload());
    Ok(output.into_plan())
}
