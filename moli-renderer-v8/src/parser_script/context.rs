//! Narrow context hooks used by shared parser script runners.
//!
//! Concrete main/child parser owners keep different per-script state, but the
//! runner only needs a small lifecycle operation after source load completion.

use crate::frame_owner_model::FrameDocumentTaskOwner;
use crate::planning::PreparedScriptSourceLoadOutcome;

/// Stable document identity captured when a parser-owned classic script is
/// accepted. Ready, failure, and completion work must retain this owner rather
/// than binding to whichever document is current when an async result arrives.
pub(crate) trait ParserClassicScriptDocumentOwnerState {
    fn parser_classic_document_task_owner(&self) -> FrameDocumentTaskOwner;
}

pub(crate) trait ParserClassicScriptExecutionGateState {
    type ExecutionGateState;

    fn parser_classic_execution_gate_state(&self) -> Self::ExecutionGateState;
}

impl ParserClassicScriptExecutionGateState for () {
    type ExecutionGateState = ();

    fn parser_classic_execution_gate_state(&self) -> Self::ExecutionGateState {}
}

pub(crate) enum ParserClassicScriptSourceLoadOutcomeState {
    NoSourceLoad,
    Waiting,
    Ready(PreparedScriptSourceLoadOutcome),
}

pub(crate) trait ParserClassicScriptSourceLoadState {
    fn clear_parser_classic_source_load_state(&mut self);
}

pub(crate) trait ParserClassicScriptSourceResultState {
    fn parser_classic_source_load_outcome_state(&self)
    -> ParserClassicScriptSourceLoadOutcomeState;
}

pub(crate) trait ParserClassicScriptSourceLoadStartState {
    type SourceLoadState;

    fn install_parser_classic_source_load_state(&mut self, state: Self::SourceLoadState);
}

pub(crate) trait ParserClassicScriptSourceLoadCompletionState {
    type SourceLoadOwner;

    fn parser_classic_source_load_owner(&self) -> Option<Self::SourceLoadOwner>;
}
