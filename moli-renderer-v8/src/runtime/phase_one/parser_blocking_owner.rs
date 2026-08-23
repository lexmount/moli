use super::parser_blocking_pending::{
    PendingParserBlockingSourceLoad, PendingParsingBlockingClassicScriptContext,
};
use super::parser_blocking_task::{
    MainParserBlockingClassicScriptCompletionAction, MainParserBlockingClassicScriptExecutionEntry,
    MainParserBlockingNextAction, main_parser_blocking_begin_execution_action,
    main_parser_blocking_finished_execution_action, main_parser_blocking_ready_action,
    main_parser_blocking_source_failure_action,
};
use super::*;
use crate::DocumentBlockingStylesheetSignature;
use crate::document_runtime::ParserConnectedScriptBridge;
use crate::document_script_scheduler::MainDocumentClassicScriptTarget;
use crate::frame_owner_model::FrameDocumentTaskOwner;
use crate::parser_script::action::{
    ParserPendingClassicScriptBeginExecutionAction,
    ParserPendingClassicScriptFinishedExecutionAction, ParserPendingClassicScriptNotification,
    ParserPendingClassicScriptReadyAction, ParserPendingClassicScriptSourceFailureAction,
    ParserPendingClassicScriptSourceLoadAction, ParserPendingClassicScriptSourceLoadRequest,
    ParserPendingClassicScriptSourceLoadWaitAction, ParserPendingClassicScriptSourceResultAction,
};
use crate::parser_script::context::ParserClassicScriptDocumentOwnerState;
use crate::parser_script::owner::{
    ParserScriptBeginExecutionOwner, ParserScriptBeginSourceLoadOwner,
    ParserScriptExecutionBlocker, ParserScriptExecutionGate, ParserScriptFinishExecutionOwner,
    ParserScriptOwner, ParserScriptReadyOwner, ParserScriptSourceFailureOwner,
    ParserScriptSourceLoadWaitOwner, ParserScriptSourceResultOwner,
};
use crate::planning::SharedScriptSourceLoad;
use crate::script_vm::ParserOwnedClassicScriptCompletion;
use std::collections::HashSet;

pub(super) struct MainParserBlockingSourceResultOwner<'a> {
    pub(super) page_vm: &'a mut PageVm,
}

pub(super) struct MainParserBlockingExternalLoadOwner {
    owner: FrameDocumentTaskOwner,
    source_load: Option<PendingParserBlockingSourceLoad>,
}

pub(super) struct MainParserBlockingExecutionGateOwner<'a> {
    pub(super) page_vm: &'a mut PageVm,
}

pub(super) struct MainParserBlockingBeginExecutionOwner {
    pub(super) parser_bridge: Option<ParserConnectedScriptBridge>,
    pub(super) completion_target: MainDocumentClassicScriptTarget,
}

pub(super) struct MainParserBlockingSourceLoadWaitOwner;

pub(super) struct MainParserBlockingLifecycleOwner {
    target: MainDocumentClassicScriptTarget,
    current_owner: Option<FrameDocumentTaskOwner>,
    completion: Option<ParserOwnedClassicScriptCompletion>,
}

impl MainParserBlockingLifecycleOwner {
    pub(super) fn new(
        target: MainDocumentClassicScriptTarget,
        current_owner: Option<FrameDocumentTaskOwner>,
        completion: ParserOwnedClassicScriptCompletion,
    ) -> Self {
        Self {
            target,
            current_owner,
            completion: Some(completion),
        }
    }
}

impl MainParserBlockingExternalLoadOwner {
    pub(super) fn new(
        owner: FrameDocumentTaskOwner,
        source_load: PendingParserBlockingSourceLoad,
    ) -> Self {
        Self {
            owner,
            source_load: Some(source_load),
        }
    }

    pub(super) fn source_load_transferred(&self) -> bool {
        self.source_load.is_none()
    }
}

impl ParserScriptOwner<PendingParsingBlockingClassicScriptContext>
    for MainParserBlockingExternalLoadOwner
{
    fn is_current_parser_script_owner(
        &mut self,
        context: &PendingParsingBlockingClassicScriptContext,
    ) -> bool {
        context.parser_classic_document_task_owner() == self.owner
    }
}

impl ParserScriptBeginSourceLoadOwner<PendingParsingBlockingClassicScriptContext>
    for MainParserBlockingExternalLoadOwner
{
    type SourceLoadAction = ParserPendingClassicScriptSourceLoadRequest;

    fn parser_script_source_load_state(&mut self) -> Option<PendingParserBlockingSourceLoad> {
        self.source_load.take()
    }

    fn parser_script_source_load_action(
        &mut self,
        action: ParserPendingClassicScriptSourceLoadAction,
    ) -> Option<Self::SourceLoadAction> {
        Some(action.into_request())
    }
}

impl ParserScriptOwner<PendingParsingBlockingClassicScriptContext>
    for MainParserBlockingSourceResultOwner<'_>
{
    fn is_current_parser_script_owner(
        &mut self,
        context: &PendingParsingBlockingClassicScriptContext,
    ) -> bool {
        main_parser_classic_owner_is_current(self.page_vm, context, "apply_source_result")
    }
}

impl ParserScriptSourceResultOwner<PendingParsingBlockingClassicScriptContext>
    for MainParserBlockingSourceResultOwner<'_>
{
    type SourceResultAction = ParserPendingClassicScriptNotification;

    fn parser_script_source_result_action(
        &mut self,
        action: ParserPendingClassicScriptSourceResultAction<'_>,
    ) -> Option<Self::SourceResultAction> {
        let notification = action.notification();
        if matches!(
            notification,
            ParserPendingClassicScriptNotification::SourceReady
        ) && let (Some(urls), Some(network_result)) =
            (action.network_record_urls(), action.network_result())
        {
            self.page_vm
                .vm_mut()
                .record_script_subresource_network_result(
                    urls.initiator_url().clone(),
                    urls.script_url().clone(),
                    network_result,
                );
        }
        Some(notification)
    }
}

impl ParserScriptReadyOwner<PendingParsingBlockingClassicScriptContext>
    for MainParserBlockingExecutionGateOwner<'_>
{
    type ReadyAction = MainParserBlockingNextAction;

    fn parser_script_ready_action(
        &mut self,
        context: &PendingParsingBlockingClassicScriptContext,
        ready: ParserPendingClassicScriptReadyAction<'_>,
    ) -> Option<Self::ReadyAction> {
        let owner = context.parser_classic_document_task_owner();
        Some(MainParserBlockingNextAction::Ready(
            main_parser_blocking_ready_action(owner, ready),
        ))
    }
}

impl ParserScriptSourceFailureOwner<PendingParsingBlockingClassicScriptContext>
    for MainParserBlockingExecutionGateOwner<'_>
{
    type SourceFailureAction = MainParserBlockingNextAction;

    fn parser_script_source_failure_action(
        &mut self,
        context: &PendingParsingBlockingClassicScriptContext,
        action: ParserPendingClassicScriptSourceFailureAction,
    ) -> Option<Self::SourceFailureAction> {
        let owner = context.parser_classic_document_task_owner();
        Some(MainParserBlockingNextAction::SourceFailed(
            main_parser_blocking_source_failure_action(owner, action),
        ))
    }
}

impl ParserScriptOwner<PendingParsingBlockingClassicScriptContext>
    for MainParserBlockingExecutionGateOwner<'_>
{
    fn is_current_parser_script_owner(
        &mut self,
        context: &PendingParsingBlockingClassicScriptContext,
    ) -> bool {
        main_parser_classic_owner_is_current(self.page_vm, context, "resolve_execution_gate")
    }

    fn parser_script_execution_gate(
        &mut self,
        blocking_signatures_before: HashSet<DocumentBlockingStylesheetSignature>,
    ) -> ParserScriptExecutionGate {
        let vm = self.page_vm.vm_mut();
        let still_blocked = vm
            .document_runtime
            .has_pending_parser_script_blocking_stylesheet_signatures(
                blocking_signatures_before.iter(),
            );
        vm.record_ready_stylesheet_network_results();
        if still_blocked {
            ParserScriptExecutionGate::Blocked(ParserScriptExecutionBlocker::Stylesheet)
        } else {
            ParserScriptExecutionGate::Ready
        }
    }
}

impl ParserScriptOwner<PendingParsingBlockingClassicScriptContext>
    for MainParserBlockingSourceLoadWaitOwner
{
}

impl ParserScriptSourceLoadWaitOwner<PendingParsingBlockingClassicScriptContext>
    for MainParserBlockingSourceLoadWaitOwner
{
    type SourceLoadWaitAction = SharedScriptSourceLoad;

    fn parser_script_source_load_wait_action(
        &mut self,
        action: ParserPendingClassicScriptSourceLoadWaitAction<SharedScriptSourceLoad>,
    ) -> Option<Self::SourceLoadWaitAction> {
        action.into_source_load_wait()
    }
}

impl ParserScriptOwner<PendingParsingBlockingClassicScriptContext>
    for MainParserBlockingLifecycleOwner
{
    fn is_current_parser_script_owner(
        &mut self,
        context: &PendingParsingBlockingClassicScriptContext,
    ) -> bool {
        let captured_owner = context.parser_classic_document_task_owner();
        let target_owner = self.target.owner();
        let is_current =
            target_owner == captured_owner && self.current_owner == Some(captured_owner);
        if !is_current {
            tracing::debug!(
                ?captured_owner,
                ?target_owner,
                current_owner = ?self.current_owner,
                "dropping stale main parser-blocking classic completion"
            );
        }
        is_current
    }
}

impl ParserScriptFinishExecutionOwner<PendingParsingBlockingClassicScriptContext>
    for MainParserBlockingLifecycleOwner
{
    type FinishedExecutionAction = MainParserBlockingClassicScriptCompletionAction;

    fn parser_script_finished_execution_action(
        &mut self,
        action: ParserPendingClassicScriptFinishedExecutionAction,
    ) -> Option<Self::FinishedExecutionAction> {
        let completion = self.completion.take().unwrap_or_else(|| {
            panic!("main parser-blocking lifecycle owner must retain its completion effects")
        });
        Some(main_parser_blocking_finished_execution_action(
            self.target,
            action,
            completion,
        ))
    }
}

impl ParserScriptOwner<PendingParsingBlockingClassicScriptContext>
    for MainParserBlockingBeginExecutionOwner
{
    fn is_current_parser_script_owner(
        &mut self,
        context: &PendingParsingBlockingClassicScriptContext,
    ) -> bool {
        let captured_owner = context.parser_classic_document_task_owner();
        let target_owner = self.completion_target.owner();
        let is_current = target_owner == captured_owner;
        if !is_current {
            tracing::debug!(
                ?captured_owner,
                ?target_owner,
                "dropping stale main parser-blocking classic execution start"
            );
        }
        is_current
    }
}

impl ParserScriptBeginExecutionOwner<PendingParsingBlockingClassicScriptContext>
    for MainParserBlockingBeginExecutionOwner
{
    type BeginExecutionAction = MainParserBlockingClassicScriptExecutionEntry;

    fn parser_script_begin_execution_action(
        &mut self,
        action: ParserPendingClassicScriptBeginExecutionAction,
    ) -> Option<Self::BeginExecutionAction> {
        Some(main_parser_blocking_begin_execution_action(
            self.parser_bridge.clone(),
            self.completion_target,
            action,
        ))
    }
}

fn main_parser_classic_owner_is_current(
    page_vm: &PageVm,
    context: &PendingParsingBlockingClassicScriptContext,
    operation: &'static str,
) -> bool {
    let captured_owner = context.parser_classic_document_task_owner();
    let current_owner = page_vm.vm().current_main_document_task_owner();
    let is_current = current_owner == Some(captured_owner);
    if !is_current {
        tracing::debug!(
            ?captured_owner,
            ?current_owner,
            operation,
            "dropping stale main parser-blocking classic PendingScript action"
        );
    }
    is_current
}
