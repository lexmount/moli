use super::parser_blocking_pending::{
    PendingParsingBlockingClassicScript, PendingParsingBlockingClassicScriptContext,
};
use crate::document_runtime::ParserConnectedScriptBridge;
use crate::document_script_scheduler::{
    MainDocumentClassicReadyWork, MainDocumentClassicScriptTarget,
    MainDocumentClassicSourceFailureWork,
};
use crate::frame_owner_model::FrameDocumentTaskOwner;
use crate::parser_script::action::{
    ParserClassicScriptBeginExecutionAction, ParserClassicScriptNextOwnerAction,
    ParserClassicScriptReadyAction, ParserClassicScriptSourceFailureAction,
    ParserPendingClassicScriptBeginExecutionAction,
    ParserPendingClassicScriptFinishedExecutionAction, ParserPendingClassicScriptReadyAction,
    ParserPendingClassicScriptSourceFailureAction,
};
use crate::parser_script::owner::ParserScriptExecutionBlocker;
use crate::parser_script::projection::{
    ParserClassicScriptBlockedOnExecution, ParserClassicScriptBlockedOnSourceLoad,
};
use crate::script_vm::ParserOwnedClassicScriptCompletion;
pub(super) type PendingParsingBlockingClassicScriptBlockedOnExecution =
    ParserClassicScriptBlockedOnExecution<PendingParsingBlockingClassicScriptContext>;
pub(super) type PendingParsingBlockingClassicScriptBlockedOnSourceLoad =
    ParserClassicScriptBlockedOnSourceLoad<PendingParsingBlockingClassicScriptContext>;

pub(super) type MainParserBlockingClassicScriptExecutionEntry =
    ParserClassicScriptBeginExecutionAction<MainParserBlockingClassicScriptExecutionTarget>;

pub(super) type MainParserBlockingNextAction = ParserClassicScriptNextOwnerAction<
    MainParserBlockingClassicScriptReadyAction,
    MainParserBlockingClassicScriptSourceFailureAction,
>;

pub(super) type MainParserBlockingClassicScriptReadyAction = MainDocumentClassicReadyWork;

pub(super) type MainParserBlockingClassicScriptSourceFailureAction =
    MainDocumentClassicSourceFailureWork;

pub(super) struct MainParserBlockingClassicScriptCompletionAction {
    target: MainDocumentClassicScriptTarget,
    completion: ParserOwnedClassicScriptCompletion,
}

impl MainParserBlockingClassicScriptCompletionAction {
    pub(super) fn new(
        target: MainDocumentClassicScriptTarget,
        completion: ParserOwnedClassicScriptCompletion,
    ) -> Self {
        Self { target, completion }
    }

    pub(super) fn target(&self) -> MainDocumentClassicScriptTarget {
        self.target
    }

    pub(super) fn into_completion(self) -> ParserOwnedClassicScriptCompletion {
        self.completion
    }
}

#[derive(Clone)]
pub(super) struct MainParserBlockingClassicScriptExecutionTarget {
    parser_bridge: Option<ParserConnectedScriptBridge>,
    completion_target: MainDocumentClassicScriptTarget,
}

impl MainParserBlockingClassicScriptExecutionTarget {
    pub(super) fn new(
        parser_bridge: Option<ParserConnectedScriptBridge>,
        completion_target: MainDocumentClassicScriptTarget,
    ) -> Self {
        Self {
            parser_bridge,
            completion_target,
        }
    }

    pub(super) fn parser_bridge(&self) -> Option<ParserConnectedScriptBridge> {
        self.parser_bridge.clone()
    }

    pub(super) fn completion_target(&self) -> MainDocumentClassicScriptTarget {
        self.completion_target
    }
}

pub(super) fn main_parser_blocking_ready_action(
    owner: FrameDocumentTaskOwner,
    ready: ParserPendingClassicScriptReadyAction<'_>,
) -> MainParserBlockingClassicScriptReadyAction {
    let target = MainDocumentClassicScriptTarget::new(owner, ready.ready_script().script_handle());
    ParserClassicScriptReadyAction::from_pending_ready_action(target, ready)
}

pub(super) fn main_parser_blocking_begin_execution_action(
    parser_bridge: Option<ParserConnectedScriptBridge>,
    completion_target: MainDocumentClassicScriptTarget,
    action: ParserPendingClassicScriptBeginExecutionAction,
) -> MainParserBlockingClassicScriptExecutionEntry {
    ParserClassicScriptBeginExecutionAction::from_pending_begin_execution_action(
        MainParserBlockingClassicScriptExecutionTarget::new(parser_bridge, completion_target),
        action,
    )
}

pub(super) fn main_parser_blocking_finished_execution_action(
    target: MainDocumentClassicScriptTarget,
    _action: ParserPendingClassicScriptFinishedExecutionAction,
    completion: ParserOwnedClassicScriptCompletion,
) -> MainParserBlockingClassicScriptCompletionAction {
    MainParserBlockingClassicScriptCompletionAction::new(target, completion)
}

pub(super) fn main_parser_blocking_source_failure_action(
    owner: FrameDocumentTaskOwner,
    failure: ParserPendingClassicScriptSourceFailureAction,
) -> MainParserBlockingClassicScriptSourceFailureAction {
    let target = MainDocumentClassicScriptTarget::new(owner, failure.script_handle());
    ParserClassicScriptSourceFailureAction::from_pending_source_failure_action(
        target, failure, None,
    )
}

pub(super) fn stylesheet_blocked_main_parser_blocking_classic_script(
    script: PendingParsingBlockingClassicScript,
) -> PendingParsingBlockingClassicScriptBlockedOnExecution {
    ParserClassicScriptBlockedOnExecution::new(ParserScriptExecutionBlocker::Stylesheet, script)
}

pub(super) fn source_load_blocked_main_parser_blocking_classic_script(
    script: PendingParsingBlockingClassicScript,
) -> PendingParsingBlockingClassicScriptBlockedOnSourceLoad {
    ParserClassicScriptBlockedOnSourceLoad::new(script)
}
