//! Owner-side contract for parser script runners.
//!
//! This module is intentionally separate from `parser_script::runner`: the
//! runner owns pending-script state transitions, while this contract describes
//! the document/runtime actions that remain main/child specific.

use crate::parser_script::action::{
    ParserPendingClassicScriptBeginExecutionAction, ParserPendingClassicScriptDisposedReadyAction,
    ParserPendingClassicScriptFinishedExecutionAction, ParserPendingClassicScriptReadyAction,
    ParserPendingClassicScriptSourceFailureAction, ParserPendingClassicScriptSourceLoadAction,
    ParserPendingClassicScriptSourceLoadCandidate,
    ParserPendingClassicScriptSourceLoadClientAction,
    ParserPendingClassicScriptSourceLoadCompletionAction,
    ParserPendingClassicScriptSourceResultAction,
};
use crate::parser_script::context::{
    ParserClassicScriptExecutionGateState, ParserClassicScriptSourceLoadCompletionState,
    ParserClassicScriptSourceLoadStartState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParserScriptExecutionGate {
    Ready,
    Blocked(ParserScriptExecutionBlocker),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParserScriptExecutionBlocker {
    Stylesheet,
}

pub(crate) trait ParserScriptOwner<Context>
where
    Context: ParserClassicScriptExecutionGateState,
{
    fn is_current_parser_script_owner(&mut self, _context: &Context) -> bool {
        true
    }

    fn parser_script_execution_gate(
        &mut self,
        _state: Context::ExecutionGateState,
    ) -> ParserScriptExecutionGate {
        ParserScriptExecutionGate::Ready
    }
}

pub(crate) trait ParserScriptReadyOwner<Context>: ParserScriptOwner<Context>
where
    Context: ParserClassicScriptExecutionGateState,
{
    type ReadyAction;

    fn parser_script_ready_action(
        &mut self,
        context: &Context,
        ready: ParserPendingClassicScriptReadyAction<'_>,
    ) -> Option<Self::ReadyAction>;
}

pub(crate) trait ParserScriptSourceFailureOwner<Context>:
    ParserScriptOwner<Context>
where
    Context: ParserClassicScriptExecutionGateState,
{
    type SourceFailureAction;

    fn parser_script_source_failure_action(
        &mut self,
        context: &Context,
        action: ParserPendingClassicScriptSourceFailureAction,
    ) -> Option<Self::SourceFailureAction>;
}

pub(crate) trait ParserScriptFinishExecutionOwner<Context>:
    ParserScriptOwner<Context>
where
    Context: ParserClassicScriptExecutionGateState,
{
    type FinishedExecutionAction;

    fn parser_script_finished_execution_action(
        &mut self,
        action: ParserPendingClassicScriptFinishedExecutionAction,
    ) -> Option<Self::FinishedExecutionAction>;
}

pub(crate) trait ParserScriptBeginExecutionOwner<Context>:
    ParserScriptOwner<Context>
where
    Context: ParserClassicScriptExecutionGateState,
{
    type BeginExecutionAction;

    fn parser_script_begin_execution_action(
        &mut self,
        action: ParserPendingClassicScriptBeginExecutionAction,
    ) -> Option<Self::BeginExecutionAction>;
}

pub(crate) trait ParserScriptDisposeReadyOwner<Context>: ParserScriptOwner<Context>
where
    Context: ParserClassicScriptExecutionGateState,
{
    type DisposedReadyAction;

    fn parser_script_disposed_ready_action(
        &mut self,
        action: ParserPendingClassicScriptDisposedReadyAction,
    ) -> Option<Self::DisposedReadyAction>;
}

pub(crate) trait ParserScriptBeginSourceLoadOwner<Context>:
    ParserScriptOwner<Context>
where
    Context: ParserClassicScriptExecutionGateState + ParserClassicScriptSourceLoadStartState,
{
    type SourceLoadAction;

    fn parser_script_source_load_candidate_matches(
        &mut self,
        _candidate: ParserPendingClassicScriptSourceLoadCandidate<'_>,
    ) -> bool {
        true
    }

    fn parser_script_source_load_state(&mut self) -> Option<Context::SourceLoadState>;

    fn parser_script_source_load_action(
        &mut self,
        action: ParserPendingClassicScriptSourceLoadAction,
    ) -> Option<Self::SourceLoadAction>;
}

pub(crate) trait ParserScriptSourceLoadClientOwner<Context>:
    ParserScriptOwner<Context>
where
    Context: ParserClassicScriptExecutionGateState,
{
    type SourceLoadClientAction;

    fn parser_script_source_load_client_action(
        &mut self,
        client: ParserPendingClassicScriptSourceLoadClientAction<'_>,
    ) -> Option<Self::SourceLoadClientAction>;
}

pub(crate) trait ParserScriptSourceLoadCompletionOwner<Context>:
    ParserScriptOwner<Context>
where
    Context: ParserClassicScriptExecutionGateState + ParserClassicScriptSourceLoadCompletionState,
{
    type SourceLoadCompletionAction;

    fn parser_script_source_load_completion_action(
        &mut self,
        completion: ParserPendingClassicScriptSourceLoadCompletionAction<Context::SourceLoadOwner>,
    ) -> Option<Self::SourceLoadCompletionAction>;
}

pub(crate) trait ParserScriptSourceResultOwner<Context>: ParserScriptOwner<Context>
where
    Context: ParserClassicScriptExecutionGateState,
{
    type SourceResultAction;

    fn parser_script_source_result_action(
        &mut self,
        action: ParserPendingClassicScriptSourceResultAction<'_>,
    ) -> Option<Self::SourceResultAction>;
}
