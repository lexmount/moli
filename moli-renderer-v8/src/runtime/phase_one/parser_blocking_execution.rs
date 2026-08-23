use super::parser_blocking_document_script::MainParserBlockingDocumentScriptOwner;
use super::parser_blocking_owner::MainParserBlockingExecutionGateOwner;
use super::parser_blocking_pending::PendingParsingBlockingClassicScriptRunner;
use super::parser_blocking_task::PendingParsingBlockingClassicScriptBlockedOnExecution;
use super::*;
use crate::document_runtime::ParserConnectedScriptBridge;
use crate::document_script_scheduler::DocumentScriptExecutionOutcome;
use crate::live_document_parser::{DocumentParserSession, ParserStopReason};
use crate::parser_script::owner::ParserScriptExecutionBlocker;
use crate::parser_script::projection::{
    ParserClassicScriptExecutionGateProjection, ParserClassicScriptNextActionWithBlockedScript,
};

pub(super) enum MainParserBlockingExecutionOutcome {
    NoNavigation,
    StoppedCurrentDocument,
    BlockedOnStylesheet(Box<PendingParsingBlockingClassicScriptBlockedOnExecution>),
    BlockedOnDocumentWriteExternalLoad,
}

pub(super) async fn resolve_main_parser_blocking_classic_after_runtime_gate(
    parser_session: &mut DocumentParserSession,
    page_vm: &mut PageVm,
    pending_runner: &mut PendingParsingBlockingClassicScriptRunner,
    log_message: &'static str,
) -> Result<MainParserBlockingExecutionOutcome> {
    let execution_gate = {
        let mut owner = MainParserBlockingExecutionGateOwner { page_vm };
        pending_runner.current_parser_blocking_execution_gate_with_owner(&mut owner)
    };
    match execution_gate {
        ParserClassicScriptExecutionGateProjection::Ready => {}
        ParserClassicScriptExecutionGateProjection::Blocked(blocked) => {
            debug_assert_eq!(
                blocked.blocker(),
                ParserScriptExecutionBlocker::Stylesheet,
                "stylesheet is currently the only parser classic execution blocker"
            );
            return Ok(MainParserBlockingExecutionOutcome::BlockedOnStylesheet(
                Box::new(blocked),
            ));
        }
        ParserClassicScriptExecutionGateProjection::NoCurrent => {
            tracing::debug!(
                "canceling main parser-blocking classic work after document owner replacement"
            );
            return Ok(MainParserBlockingExecutionOutcome::StoppedCurrentDocument);
        }
    }
    if let Some(permit) = pending_runner
        .current_parser_blocking_context()
        .and_then(|context| context.resume_permit())
        && !parser_session.resume(permit)
    {
        tracing::debug!(
            ?permit,
            run_state = ?parser_session.run_state(),
            "canceling stale main parser-blocking continuation permit"
        );
        parser_session.stop(ParserStopReason::DocumentReplacement);
        return Ok(MainParserBlockingExecutionOutcome::StoppedCurrentDocument);
    }
    let parser_bridge = ParserConnectedScriptBridge::for_session(parser_session);
    page_vm.vm_mut().sync_live_document_style_sources();
    let next_projection = {
        let mut owner = MainParserBlockingExecutionGateOwner { page_vm };
        pending_runner
            .take_current_parser_blocking_next_action_or_blocked_script_with_owner(&mut owner)
    };
    let mut document_script_owner = MainParserBlockingDocumentScriptOwner::new(
        page_vm,
        pending_runner,
        parser_bridge,
        log_message,
    );
    let action = match next_projection {
        ParserClassicScriptNextActionWithBlockedScript::Action(action) => action,
        ParserClassicScriptNextActionWithBlockedScript::Blocked(blocked) => {
            debug_assert_eq!(
                blocked.blocker(),
                ParserScriptExecutionBlocker::Stylesheet,
                "stylesheet is currently the only parser classic execution blocker"
            );
            return Ok(MainParserBlockingExecutionOutcome::BlockedOnStylesheet(
                Box::new(blocked),
            ));
        }
        ParserClassicScriptNextActionWithBlockedScript::NotReady => {
            let execution_gate = {
                let mut owner = MainParserBlockingExecutionGateOwner { page_vm };
                pending_runner.current_parser_blocking_execution_gate_with_owner(&mut owner)
            };
            if matches!(
                execution_gate,
                ParserClassicScriptExecutionGateProjection::NoCurrent
            ) {
                return Ok(MainParserBlockingExecutionOutcome::StoppedCurrentDocument);
            }
            return Err(anyhow::anyhow!(
                "current parser-blocking classic PendingScript reached execution without a ready or failed source terminal"
            ));
        }
    };
    match document_script_owner
        .run_next_classic_script_action(action)
        .await?
    {
        DocumentScriptExecutionOutcome::NoProgress | DocumentScriptExecutionOutcome::Progressed => {
        }
        DocumentScriptExecutionOutcome::TriggeredNavigation => {
            return Ok(MainParserBlockingExecutionOutcome::StoppedCurrentDocument);
        }
        DocumentScriptExecutionOutcome::BlockedOnDocumentWriteExternalLoad => {
            return Ok(MainParserBlockingExecutionOutcome::BlockedOnDocumentWriteExternalLoad);
        }
    }
    Ok(MainParserBlockingExecutionOutcome::NoNavigation)
}
