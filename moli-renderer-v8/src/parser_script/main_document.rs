use std::collections::HashSet;

use crate::DocumentBlockingStylesheetSignature;
use crate::frame_owner_model::FrameDocumentTaskOwner;
use crate::live_document_parser::ParserResumePermit;
use crate::parser_script::context::{
    ParserClassicScriptDocumentOwnerState, ParserClassicScriptExecutionGateState,
    ParserClassicScriptSourceLoadOutcomeState, ParserClassicScriptSourceLoadStartState,
    ParserClassicScriptSourceLoadState, ParserClassicScriptSourceResultState,
};
use crate::parser_script::item::ParserClassicScriptRunnerItem;
use crate::planning::SharedScriptSourceLoad;

pub(crate) type PendingMainParserScript =
    ParserClassicScriptRunnerItem<PendingParsingBlockingClassicScriptContext>;

#[derive(Debug, Clone)]
pub(crate) struct PendingParsingBlockingClassicScriptContext {
    owner: FrameDocumentTaskOwner,
    pub(crate) blocking_signatures_before: HashSet<DocumentBlockingStylesheetSignature>,
    pub(crate) source_load: Option<PendingParserBlockingSourceLoad>,
    resume_permit: Option<ParserResumePermit>,
}

#[derive(Debug, Clone)]
pub(crate) enum PendingParserBlockingSourceLoad {
    ReusablePreload(SharedScriptSourceLoad),
    ParserDiscovered(SharedScriptSourceLoad),
}

impl PendingParserBlockingSourceLoad {
    pub(crate) fn shared_load(&self) -> SharedScriptSourceLoad {
        match self {
            Self::ReusablePreload(load) | Self::ParserDiscovered(load) => load.clone(),
        }
    }
}

impl PendingParsingBlockingClassicScriptContext {
    pub(crate) fn new(
        owner: FrameDocumentTaskOwner,
        blocking_signatures_before: HashSet<DocumentBlockingStylesheetSignature>,
        source_load: Option<PendingParserBlockingSourceLoad>,
    ) -> Self {
        Self {
            owner,
            blocking_signatures_before,
            source_load,
            resume_permit: None,
        }
    }

    pub(crate) fn install_resume_permit(&mut self, permit: ParserResumePermit) {
        assert!(
            self.resume_permit.replace(permit).is_none(),
            "one parser-blocking script context can own only one parser resume permit"
        );
    }

    pub(crate) fn resume_permit(&self) -> Option<ParserResumePermit> {
        self.resume_permit
    }
}

impl ParserClassicScriptDocumentOwnerState for PendingParsingBlockingClassicScriptContext {
    fn parser_classic_document_task_owner(&self) -> FrameDocumentTaskOwner {
        self.owner
    }
}

impl ParserClassicScriptExecutionGateState for PendingParsingBlockingClassicScriptContext {
    type ExecutionGateState = HashSet<DocumentBlockingStylesheetSignature>;

    fn parser_classic_execution_gate_state(&self) -> Self::ExecutionGateState {
        self.blocking_signatures_before.clone()
    }
}

impl ParserClassicScriptSourceLoadState for PendingParsingBlockingClassicScriptContext {
    fn clear_parser_classic_source_load_state(&mut self) {
        self.source_load = None;
    }
}

impl ParserClassicScriptSourceResultState for PendingParsingBlockingClassicScriptContext {
    fn parser_classic_source_load_outcome_state(
        &self,
    ) -> ParserClassicScriptSourceLoadOutcomeState {
        let Some(source_load) = self.source_load.as_ref() else {
            return ParserClassicScriptSourceLoadOutcomeState::NoSourceLoad;
        };
        let Some(outcome) = source_load.shared_load().try_outcome() else {
            return ParserClassicScriptSourceLoadOutcomeState::Waiting;
        };
        ParserClassicScriptSourceLoadOutcomeState::Ready(outcome)
    }
}

impl ParserClassicScriptSourceLoadStartState for PendingParsingBlockingClassicScriptContext {
    type SourceLoadState = PendingParserBlockingSourceLoad;

    fn install_parser_classic_source_load_state(&mut self, state: Self::SourceLoadState) {
        self.source_load = Some(state);
    }
}
