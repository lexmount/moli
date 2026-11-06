use super::page_vm::{ParseTimeLiveExecution, ParseTimeMainParserBoundaryOutcome};
use super::*;
use crate::document_script_scheduler::{
    DocumentScriptExecutionLane, DocumentScriptSourceFailureLane, PageOwnedDocumentScriptWork,
};
use crate::frame_owner_model::MainDocumentScriptLoadDelayLease;
use crate::page_task_queue::PostParsePageOwnedWork;
use crate::planning::PreparedScript;
use crate::types::SharedNavigationResponseResult;

fn page_task_turn_result(
    page_vm: &PageVm,
    navigation_triggered: bool,
    parser_boundary: ParseTimeMainParserBoundaryOutcome,
) -> PageTaskTurnResult {
    if navigation_triggered
        || parser_boundary == ParseTimeMainParserBoundaryOutcome::DocumentReplaced
        || page_vm.vm().has_pending_location_navigation()
    {
        return PageTaskTurnResult::StoppedCurrentDocument;
    }
    PageTaskTurnResult::ExecutedTask
}

pub(super) async fn execute_page_owned_work_turn_on_local_task(
    page_vm: &mut PageVm,
    work: PostParsePageOwnedWork,
) -> Result<PageTaskTurnResult> {
    // Execute the page-owned task on a fresh local-task stack via spawn_local,
    // so that V8 calls (Script::compile, script.run, etc.) run with adequate
    // OS stack headroom.
    //
    // Background: the phase-one pump nests ~15 async layers from the outer
    // spawn_local boundary (run_named_owner_local_task) down to this point:
    //
    //   run_named_owner_local_task          (spawn_local — fresh stack)
    //     create_page_vm_from_html_with_stage
    //       finish_html_bootstrap_phase_one_on_named_owner_lane
    //         finish_html_phase_one
    //           finish_creation_from_page_vm
    //             finish_phase_one_creation_on_execution_context
    //               run_phase_one_execution_context_session
    //                 pump.run_to_completion            (main loop)
    //                   run_current_owner
    //                     drive_owner_step (dispatch)
    //                       document owner turn dispatch
    //                         drain_parse_time_turns_until_idle
    //                           run_parse_time_turn    <- WE ARE HERE
    //
    // Each async fn's state machine is embedded in its parent's enum. When
    // tokio polls the outermost future, every nested poll() adds a stack
    // frame. By level 15 the RSP is ~712 KB below isolate creation, well
    // past V8's default 984 KB jslimit (v8/src/common/globals.h:197).
    //
    // V8's Script::compile hits StackLimitCheck::HasOverflowed() immediately,
    // calls Isolate::StackOverflow() -> Isolate::Throw(), which asserts
    // CHECK(IsOnCentralStack()). That CHECK fails because the RSP has fallen
    // below the central stack's mapped range. The resulting crash message is
    // a stack overflow, not a thread-affinity issue.
    //
    // The fix: run_phase_one_local_task -> run_named_owner_local_task ->
    // tokio::task::spawn_local, which adds the future to the LocalSet's task
    // queue. The LocalSet polls it from its own shallow frame, giving V8
    // nearly the full thread stack.
    //
    // Box::pin is essential: without it, Rust monomorphizes
    // run_phase_one_local_task<F> with F = the full async closure type. That
    // inflates run_parse_time_turn's state machine enum, causing other tests
    // to also overflow. With Box::pin, F = Pin<Box<dyn Future>>, keeping the
    // enum size neutral.
    //
    // This matches the other V8-entry paths in phase one: live document
    // refresh, connected-style load dispatch, and parser-connected script
    // execution.
    let outcome = page_vm
        .execute_parse_time_on_existing_live_document_on_named_owner_local_task(
            ParseTimeLiveExecution::PageOwnedWork {
                work: Box::new(work),
            },
        )
        .await?;
    let navigation_triggered = outcome.navigation_triggered();
    let parser_boundary = if let Some(completion) = outcome.into_main_parser_completion() {
        page_vm
            .finish_parse_time_main_parser_boundary(completion)
            .await?
    } else {
        ParseTimeMainParserBoundaryOutcome::CurrentDocumentRetained
    };
    Ok(page_task_turn_result(
        page_vm,
        navigation_triggered,
        parser_boundary,
    ))
}

pub(super) async fn execute_page_owned_document_script_turn_on_local_task(
    page_vm: &mut PageVm,
    lane: DocumentScriptExecutionLane,
    script: PreparedScript,
    source_network_result: Option<SharedNavigationResponseResult>,
    load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
) -> Result<PageTaskTurnResult> {
    let outcome = page_vm
        .execute_parse_time_on_existing_live_document_on_named_owner_local_task(
            ParseTimeLiveExecution::PageOwnedDocumentScript {
                lane,
                script: Box::new(script),
                source_network_result,
                load_delay_binding,
            },
        )
        .await?;
    Ok(page_task_turn_result(
        page_vm,
        outcome.navigation_triggered(),
        ParseTimeMainParserBoundaryOutcome::CurrentDocumentRetained,
    ))
}

pub(super) async fn execute_page_owned_document_script_failure_turn_on_local_task(
    page_vm: &mut PageVm,
    lane: DocumentScriptSourceFailureLane,
    script: PreparedScript,
    error: String,
    source_network_result: Option<SharedNavigationResponseResult>,
    load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
) -> Result<PageTaskTurnResult> {
    let outcome = page_vm
        .execute_parse_time_on_existing_live_document_on_named_owner_local_task(
            ParseTimeLiveExecution::PageOwnedWork {
                work: Box::new(PostParsePageOwnedWork::document_script_work(
                    PageOwnedDocumentScriptWork::parser_async_source_failure(
                        lane,
                        script,
                        error,
                        source_network_result,
                        load_delay_binding,
                    ),
                )),
            },
        )
        .await?;
    Ok(page_task_turn_result(
        page_vm,
        outcome.navigation_triggered(),
        ParseTimeMainParserBoundaryOutcome::CurrentDocumentRetained,
    ))
}
