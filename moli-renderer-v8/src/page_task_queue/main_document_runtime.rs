use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource, OwnerTaskReadySignal};

use crate::{
    document_script_scheduler::MainParserAsyncModuleAdmission,
    frame_owner_model::FrameDocumentTaskOwner,
    host::RuntimeScriptAdmission,
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
    script_vm::RuntimeScriptContinuationBodyEffect,
};

use super::{
    MainDocumentPostParseExecution, PageDynamicModuleJobTurnAction, PageMainNativeModuleSettlement,
    PageMainNativeModuleTargetEffect, PageNativeModuleOwnerEventTurnAction,
    PageParserAsyncModuleAdmissionTargetEffect, PageParserAsyncModuleAdmissionTurnAction,
    PageParserOwnedModuleContinuationTargetEffect, PageParserOwnedModuleContinuationTurnAction,
    PostParsePageOwnedWork, RendererOwnerWakeSender,
};

pub(crate) type RendererPageMainDocumentRuntimeOwner = super::RendererPageMainDocumentTaskOwner;

/// Concrete actions from the HTML internal-continue-script-loading family.
///
/// These are executable opportunities, not generic requests to rescan the
/// PageVm. An admission transfers one exact parser or runtime script into its
/// shared owner. A continuation advances one runtime-script state-machine action;
/// a module continuation consumes one already-ready graph/evaluation action;
/// a parser-owned continuation consumes one already-ready parser module
/// action; a native module owner event dispatches one already-posted module-map
/// notification or modulepreload link event;
/// and post-parse work carries the concrete payload to execute.
#[derive(Debug)]
pub(crate) enum RendererPageMainDocumentRuntimeAction {
    AdmitRuntimeScript(RuntimeScriptAdmission),
    AdmitParserAsyncModule(MainParserAsyncModuleAdmission),
    ContinueRuntimeScriptWork,
    ContinueDynamicModuleJob,
    ContinueRuntimeOwnedModule,
    ContinueParserOwnedModule,
    ContinueNativeModuleOwnerEvent,
    ExecuteReadyPostParseWork(RendererPageReadyMainDocumentRuntimeWork),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageMainDocumentRuntimeActionKind {
    RuntimeScriptAdmission,
    ParserAsyncModuleAdmission,
    RuntimeScriptContinuation,
    DynamicModuleJob,
    RuntimeOwnedModuleContinuation,
    ParserOwnedModuleContinuation,
    NativeModuleOwnerEvent,
    PostParseWork,
}

impl RendererPageMainDocumentRuntimeAction {
    pub(crate) const fn kind(&self) -> PageMainDocumentRuntimeActionKind {
        match self {
            Self::AdmitRuntimeScript(_) => {
                PageMainDocumentRuntimeActionKind::RuntimeScriptAdmission
            }
            Self::AdmitParserAsyncModule(_) => {
                PageMainDocumentRuntimeActionKind::ParserAsyncModuleAdmission
            }
            Self::ContinueRuntimeScriptWork => {
                PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation
            }
            Self::ContinueDynamicModuleJob => PageMainDocumentRuntimeActionKind::DynamicModuleJob,
            Self::ContinueRuntimeOwnedModule => {
                PageMainDocumentRuntimeActionKind::RuntimeOwnedModuleContinuation
            }
            Self::ContinueParserOwnedModule => {
                PageMainDocumentRuntimeActionKind::ParserOwnedModuleContinuation
            }
            Self::ContinueNativeModuleOwnerEvent => {
                PageMainDocumentRuntimeActionKind::NativeModuleOwnerEvent
            }
            Self::ExecuteReadyPostParseWork(_) => PageMainDocumentRuntimeActionKind::PostParseWork,
        }
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageMainDocumentRuntimeTask {
    owner: RendererPageMainDocumentRuntimeOwner,
    action: RendererPageMainDocumentRuntimeAction,
}

impl RendererPageMainDocumentRuntimeTask {
    fn new(
        root_document: RendererDocumentToken,
        document_owner: FrameDocumentTaskOwner,
        action: RendererPageMainDocumentRuntimeAction,
    ) -> Self {
        Self {
            owner: RendererPageMainDocumentRuntimeOwner::new(root_document, document_owner),
            action,
        }
    }

    pub(crate) const fn owner(&self) -> RendererPageMainDocumentRuntimeOwner {
        self.owner
    }

    pub(crate) const fn action_kind(&self) -> PageMainDocumentRuntimeActionKind {
        self.action.kind()
    }

    pub(crate) fn into_action(self) -> RendererPageMainDocumentRuntimeAction {
        self.action
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageMainDocumentRuntimeRouteClosed;

#[derive(Clone, Debug)]
pub(crate) struct RendererPageMainDocumentRuntimeRoute {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageMainDocumentRuntimeTask>,
        RendererPageMainDocumentRuntimeReadySignal,
    >,
}

impl RendererPageMainDocumentRuntimeRoute {
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageMainDocumentRuntimeSender {
        RendererPageMainDocumentRuntimeSender {
            task_route: self.task_route.clone(),
            root_document,
        }
    }

    fn same_route_as(&self, source: &RendererPageMainDocumentRuntimeSource) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

/// PageVm-stamped producer. Callers must additionally provide the exact
/// main-Document owner captured at task production.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageMainDocumentRuntimeSender {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageMainDocumentRuntimeTask>,
        RendererPageMainDocumentRuntimeReadySignal,
    >,
    root_document: RendererDocumentToken,
}

impl RendererPageMainDocumentRuntimeSender {
    fn send(
        &self,
        document_owner: FrameDocumentTaskOwner,
        action: RendererPageMainDocumentRuntimeAction,
    ) -> Result<(), RendererPageMainDocumentRuntimeRouteClosed> {
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(
                RendererPageMainDocumentRuntimeTask::new(
                    self.root_document,
                    document_owner,
                    action,
                ),
            ))
            .map_err(|_| RendererPageMainDocumentRuntimeRouteClosed)
    }

    pub(crate) fn bind_producer(
        &self,
        document_owner: FrameDocumentTaskOwner,
        runtime_generation: u64,
    ) -> RendererPageMainDocumentRuntimeProducer {
        RendererPageMainDocumentRuntimeProducer {
            sender: self.clone(),
            document_owner,
            runtime_generation,
        }
    }

    fn send_runtime_script_admission(
        &self,
        document_owner: FrameDocumentTaskOwner,
        admission: RuntimeScriptAdmission,
    ) -> Result<(), RuntimeScriptAdmission> {
        debug_assert_eq!(
            admission.owner(),
            document_owner,
            "runtime script admission lease must match its stable-source owner"
        );
        let task = ReadyPageTask::new(RendererPageMainDocumentRuntimeTask::new(
            self.root_document,
            document_owner,
            RendererPageMainDocumentRuntimeAction::AdmitRuntimeScript(admission),
        ));
        self.task_route
            .send_and_signal_if_newly_ready(task)
            .map_err(|error| {
                let (_, task) = error.0.into_parts();
                match task.into_action() {
                    RendererPageMainDocumentRuntimeAction::AdmitRuntimeScript(admission) => {
                        admission
                    }
                    _ => unreachable!("runtime-script admission send must recover its payload"),
                }
            })
    }

    #[cfg(test)]
    pub(crate) fn send_for_source_contract_test(
        &self,
        document_owner: FrameDocumentTaskOwner,
        action: RendererPageMainDocumentRuntimeAction,
    ) -> Result<(), RendererPageMainDocumentRuntimeRouteClosed> {
        self.send(document_owner, action)
    }
}

/// Producer bound to one exact main Document.
///
/// Asynchronous runtime state may retain this value, and the Document-owned
/// host scheduler replaces its current producer only at the synchronous
/// `document.open()` owner transition. Already-created producers therefore
/// cannot rebind work to a later Document. `runtime_generation` remains only
/// as transitional metadata for binding pending `PreparedScript` payloads; it
/// is not part of the Page task's authorization identity.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageMainDocumentRuntimeProducer {
    sender: RendererPageMainDocumentRuntimeSender,
    document_owner: FrameDocumentTaskOwner,
    runtime_generation: u64,
}

impl RendererPageMainDocumentRuntimeProducer {
    fn send_action(
        &self,
        action: RendererPageMainDocumentRuntimeAction,
    ) -> Result<(), RendererPageMainDocumentRuntimeRouteClosed> {
        self.sender.send(self.document_owner, action)
    }

    pub(crate) fn send_runtime_script_continuation(
        &self,
    ) -> Result<(), RendererPageMainDocumentRuntimeRouteClosed> {
        self.send_action(RendererPageMainDocumentRuntimeAction::ContinueRuntimeScriptWork)
    }

    pub(crate) fn send_runtime_script_admission(
        &self,
        admission: RuntimeScriptAdmission,
    ) -> Result<(), RuntimeScriptAdmission> {
        self.sender
            .send_runtime_script_admission(self.document_owner, admission)
    }

    pub(crate) fn send_parser_async_module_admission(
        &self,
        admission: MainParserAsyncModuleAdmission,
    ) -> Result<(), RendererPageMainDocumentRuntimeRouteClosed> {
        debug_assert_eq!(
            admission.owner(),
            self.document_owner,
            "parser async-module admission lease must match its stable-source owner"
        );
        self.send_action(RendererPageMainDocumentRuntimeAction::AdmitParserAsyncModule(admission))
    }

    pub(crate) fn send_dynamic_module_job(
        &self,
    ) -> Result<(), RendererPageMainDocumentRuntimeRouteClosed> {
        self.send_action(RendererPageMainDocumentRuntimeAction::ContinueDynamicModuleJob)
    }

    pub(crate) fn send_runtime_module_continuation(
        &self,
    ) -> Result<(), RendererPageMainDocumentRuntimeRouteClosed> {
        self.send_action(RendererPageMainDocumentRuntimeAction::ContinueRuntimeOwnedModule)
    }

    pub(crate) fn send_parser_owned_module_continuation(
        &self,
    ) -> Result<(), RendererPageMainDocumentRuntimeRouteClosed> {
        self.send_action(RendererPageMainDocumentRuntimeAction::ContinueParserOwnedModule)
    }

    pub(crate) fn send_native_module_owner_event(
        &self,
    ) -> Result<(), RendererPageMainDocumentRuntimeRouteClosed> {
        self.send_action(RendererPageMainDocumentRuntimeAction::ContinueNativeModuleOwnerEvent)
    }

    fn send_ready_post_parse_work(
        &self,
        work: RendererPageReadyMainDocumentRuntimeWork,
    ) -> Result<(), RendererPageMainDocumentRuntimeRouteClosed> {
        self.send_action(RendererPageMainDocumentRuntimeAction::ExecuteReadyPostParseWork(work))
    }

    /// Admit post-parse work only after its source-load prerequisite is ready.
    ///
    /// Pending script work is retained by the source-load completion callback,
    /// together with this exact-Document producer. The stable Page source sees
    /// no descriptor until the payload is executable, and a late completion
    /// can only enqueue under the producer's original Document epoch.
    pub(crate) fn send_post_parse_work_when_ready(
        &self,
        mut work: PostParsePageOwnedWork,
    ) -> Result<(), RendererPageMainDocumentRuntimeAdmissionError> {
        if !work.bind_main_document_runtime_target(self.document_owner, self.runtime_generation) {
            return Err(RendererPageMainDocumentRuntimeAdmissionError::TargetMismatch);
        }
        work.complete_source_load_if_ready();
        let mut work = match RendererPageReadyMainDocumentRuntimeWork::try_new(work) {
            Ok(ready) => {
                return self
                    .send_ready_post_parse_work(ready)
                    .map_err(|_| RendererPageMainDocumentRuntimeAdmissionError::RouteClosed);
            }
            Err(pending) => pending,
        };
        let source_load = work
            .claim_source_load_completion_wake()
            .ok_or(RendererPageMainDocumentRuntimeAdmissionError::SourceLoadWakeAlreadyClaimed)?;
        let producer = self.clone();
        source_load.register_completion_wake(move || {
            let completed = work.complete_source_load_if_ready();
            assert!(
                completed,
                "source-load wake must publish a terminal outcome"
            );
            let ready = RendererPageReadyMainDocumentRuntimeWork::try_new(work)
                .unwrap_or_else(|_| panic!("completed source load must produce runnable work"));
            let _ = producer.send_ready_post_parse_work(ready);
        });
        Ok(())
    }

    pub(crate) fn send_lifecycle_work(
        &self,
        work: super::PostParseLifecycleWork,
    ) -> Result<(), RendererPageMainDocumentRuntimeAdmissionError> {
        self.send_post_parse_work_when_ready(PostParsePageOwnedWork::lifecycle_work(work))
    }
}

/// Post-parse work proven runnable before it enters the ready Page source.
///
/// The broad lifecycle adapter payload can also represent a script whose
/// source is still loading. Keeping the constructor private to this module
/// prevents queue producers from turning that pending state into a runnable
/// scheduler descriptor.
#[derive(Debug)]
pub(crate) struct RendererPageReadyMainDocumentRuntimeWork(PostParsePageOwnedWork);

impl RendererPageReadyMainDocumentRuntimeWork {
    fn try_new(work: PostParsePageOwnedWork) -> Result<Self, PostParsePageOwnedWork> {
        if work.is_waiting_for_source_load() {
            return Err(work);
        }
        Ok(Self(work))
    }

    pub(crate) fn into_post_parse_work(self) -> PostParsePageOwnedWork {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageMainDocumentRuntimeAdmissionError {
    RouteClosed,
    SourceLoadWakeAlreadyClaimed,
    TargetMismatch,
}

#[derive(Clone, Debug)]
struct RendererPageMainDocumentRuntimeReadySignal {
    owner_wake: RendererOwnerWakeSender,
}

impl OwnerTaskReadySignal for RendererPageMainDocumentRuntimeReadySignal {
    fn signal_ready(&self) {
        self.owner_wake.signal_main_document_runtime_task();
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageMainDocumentRuntimeSource {
    source: OwnerReadyTaskSource<
        ReadyPageTask<RendererPageMainDocumentRuntimeTask>,
        RendererPageMainDocumentRuntimeReadySignal,
    >,
}

impl RendererPageMainDocumentRuntimeSource {
    pub(crate) fn new(owner_wake: RendererOwnerWakeSender) -> Self {
        Self {
            source: OwnerReadyTaskSource::new(RendererPageMainDocumentRuntimeReadySignal {
                owner_wake,
            }),
        }
    }

    pub(crate) fn route(&self) -> RendererPageMainDocumentRuntimeRoute {
        RendererPageMainDocumentRuntimeRoute {
            task_route: self.source.route(),
        }
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.front().map(ReadyPageTask::metadata)
    }

    pub(crate) fn next_ready_owner(&mut self) -> Option<RendererPageMainDocumentRuntimeOwner> {
        self.source.front().map(|ready| ready.value().owner())
    }

    #[cfg(test)]
    pub(crate) fn next_ready_action_kind(&mut self) -> Option<PageMainDocumentRuntimeActionKind> {
        self.source.front().map(|ready| ready.value().action_kind())
    }

    #[cfg(test)]
    pub(crate) fn has_ready_action_kind(
        &mut self,
        expected_kind: PageMainDocumentRuntimeActionKind,
    ) -> bool {
        self.source
            .has_matching_task(|ready| ready.value().action_kind() == expected_kind)
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(
        RendererPageTaskReadyMetadata,
        RendererPageMainDocumentRuntimeTask,
    )> {
        self.source.pop_front().map(ReadyPageTask::into_parts)
    }

    pub(crate) fn has_ready_task(&mut self) -> bool {
        !self.source.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.source.clear_local();
    }

    pub(crate) fn route_matches(&self, route: &RendererPageMainDocumentRuntimeRoute) -> bool {
        route.same_route_as(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageMainDocumentRuntimeTargetEffect {
    AppliedToCurrentOwner,
    CurrentOwnerHadNoMatchingWork,
    IgnoredStaleOwner,
}

/// Common exact-owner result for one selected main-Document runtime action.
///
/// The scheduler source intentionally keeps several ordered script-loading
/// actions in one lane. Once an action has executed, however, its concrete
/// kind must remain visible so P5 can move task completion one variant at a
/// time without claiming that every action in this heterogeneous family
/// already uses the central completion boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageMainDocumentRuntimeActionResult {
    owner: RendererPageMainDocumentRuntimeOwner,
    target_effect: PageMainDocumentRuntimeTargetEffect,
}

impl PageMainDocumentRuntimeActionResult {
    const fn new(
        owner: RendererPageMainDocumentRuntimeOwner,
        target_effect: PageMainDocumentRuntimeTargetEffect,
    ) -> Self {
        Self {
            owner,
            target_effect,
        }
    }

    #[cfg(test)]
    pub(crate) const fn owner(self) -> RendererPageMainDocumentRuntimeOwner {
        self.owner
    }

    pub(crate) const fn target_effect(self) -> PageMainDocumentRuntimeTargetEffect {
        self.target_effect
    }
}

/// Exact-Document effect of one selected runtime-script admission.
///
/// Admission has no "matching work disappeared" state: the selected task
/// carries the concrete `RuntimeScriptAdmission` payload. Once its owner is
/// current, consuming that payload is the body of this internal HTML task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageRuntimeScriptAdmissionTargetEffect {
    AdmittedToCurrentOwner,
    DiscardedStaleOwner,
}

/// Post-execution result reserved for `AdmitRuntimeScript`.
///
/// This type is produced only after exact-owner authorization. Keeping it
/// separate from the other actions in the shared FIFO prevents their
/// checkpoint rules from being inferred from `kind + bool` at the selected
/// dispatcher.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageRuntimeScriptAdmissionTurnAction {
    owner: RendererPageMainDocumentRuntimeOwner,
    target_effect: PageRuntimeScriptAdmissionTargetEffect,
}

impl PageRuntimeScriptAdmissionTurnAction {
    const fn new(
        owner: RendererPageMainDocumentRuntimeOwner,
        target_effect: PageRuntimeScriptAdmissionTargetEffect,
    ) -> Self {
        Self {
            owner,
            target_effect,
        }
    }

    pub(crate) const fn target_effect(self) -> PageRuntimeScriptAdmissionTargetEffect {
        self.target_effect
    }
}

/// Exact-Document effect of one selected runtime-script continuation.
///
/// The body effect remains visible instead of being flattened to "made
/// progress": publishing a concrete successor, waiting for a producer, and
/// consuming a spent reservation are different runtime-owner transitions.
/// They share only the ordinary task-end rule. Stale owner rejection remains
/// separate because it must not enter the replacement realm or checkpoint it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageRuntimeScriptContinuationTargetEffect {
    AppliedToCurrentOwner(RuntimeScriptContinuationBodyEffect),
    DiscardedStaleOwner,
}

/// Post-execution result reserved for `ContinueRuntimeScriptWork`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageRuntimeScriptContinuationTurnAction {
    owner: RendererPageMainDocumentRuntimeOwner,
    target_effect: PageRuntimeScriptContinuationTargetEffect,
}

impl PageRuntimeScriptContinuationTurnAction {
    const fn new(
        owner: RendererPageMainDocumentRuntimeOwner,
        target_effect: PageRuntimeScriptContinuationTargetEffect,
    ) -> Self {
        Self {
            owner,
            target_effect,
        }
    }

    pub(crate) const fn target_effect(self) -> PageRuntimeScriptContinuationTargetEffect {
        self.target_effect
    }
}

/// Execution result for the exact runtime-owned module continuation variant.
///
/// This newtype is produced only after `ContinueRuntimeOwnedModule` has been
/// authorized and attempted. It cannot be constructed from the other actions
/// sharing the main-Document runtime source, so the selected-task dispatcher
/// can own this variant's checkpoint without a completion-policy flag or a
/// kind/boolean join.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageRuntimeOwnedModuleContinuationTurnAction(PageMainDocumentRuntimeActionResult);

impl PageRuntimeOwnedModuleContinuationTurnAction {
    const fn new(result: PageMainDocumentRuntimeActionResult) -> Self {
        Self(result)
    }

    pub(crate) const fn target_effect(self) -> PageMainDocumentRuntimeTargetEffect {
        self.0.target_effect()
    }
}

/// Strongly typed post-execution action from the heterogeneous
/// main-Document runtime source.
///
/// The source remains shared because these actions participate in one ordered
/// internal script-loading lane. The variants remain distinct after execution
/// because their task-end contracts are migrated independently. Runtime-script
/// admission, runtime-script continuation, dynamic-module job,
/// runtime-owned/parser-owned module continuation, and native-module owner
/// event currently delegate their task-end to the unique selected-task
/// dispatcher. Only generic post-parse work retains its family-local behavior
/// until the C-batch migration.
#[derive(Debug)]
pub(crate) enum PageMainDocumentRuntimeTurnAction {
    RuntimeScriptAdmission(PageRuntimeScriptAdmissionTurnAction),
    ParserAsyncModuleAdmission(PageParserAsyncModuleAdmissionTurnAction),
    RuntimeScriptContinuation(PageRuntimeScriptContinuationTurnAction),
    DynamicModuleJob(PageDynamicModuleJobTurnAction),
    RuntimeOwnedModuleContinuation(PageRuntimeOwnedModuleContinuationTurnAction),
    ParserOwnedModuleContinuation(PageParserOwnedModuleContinuationTurnAction),
    NativeModuleOwnerEvent(PageNativeModuleOwnerEventTurnAction),
    PostParseWork(PageMainDocumentPostParseTurnAction),
}

/// Post-execution result for the `ExecuteReadyPostParseWork` carrier.
///
/// The C-batch variants retain their exact execution fact until the selected
/// dispatcher submits the shared post-parse completion. Other post-parse
/// families already completed through their dedicated coordinator and remain
/// explicit as `CompletedByFamily`; this avoids pretending the heterogeneous
/// carrier has one source-wide completion policy.
#[derive(Debug)]
pub(crate) enum PageMainDocumentPostParseTurnAction {
    Executed {
        owner: RendererPageMainDocumentRuntimeOwner,
        execution: MainDocumentPostParseExecution,
    },
    CompletedByFamily(PageMainDocumentRuntimeActionResult),
}

impl PageMainDocumentPostParseTurnAction {
    pub(crate) const fn executed(
        owner: RendererPageMainDocumentRuntimeOwner,
        execution: MainDocumentPostParseExecution,
    ) -> Self {
        Self::Executed { owner, execution }
    }

    const fn completed_by_family(result: PageMainDocumentRuntimeActionResult) -> Self {
        Self::CompletedByFamily(result)
    }

    pub(crate) fn into_execution(
        self,
    ) -> Option<(
        RendererPageMainDocumentRuntimeOwner,
        MainDocumentPostParseExecution,
    )> {
        match self {
            Self::Executed { owner, execution } => Some((owner, execution)),
            Self::CompletedByFamily(_result) => None,
        }
    }

    #[cfg(test)]
    const fn owner(&self) -> RendererPageMainDocumentRuntimeOwner {
        match self {
            Self::Executed { owner, .. } => *owner,
            Self::CompletedByFamily(result) => result.owner(),
        }
    }

    #[cfg(test)]
    const fn target_effect(&self) -> PageMainDocumentRuntimeTargetEffect {
        match self {
            Self::Executed { execution, .. } => {
                if execution.target().applied_to_selected_owner() {
                    PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner
                } else {
                    PageMainDocumentRuntimeTargetEffect::IgnoredStaleOwner
                }
            }
            Self::CompletedByFamily(result) => result.target_effect(),
        }
    }
}

impl PageMainDocumentRuntimeTurnAction {
    pub(crate) const fn runtime_script_admission(
        owner: RendererPageMainDocumentRuntimeOwner,
        target_effect: PageRuntimeScriptAdmissionTargetEffect,
    ) -> Self {
        Self::RuntimeScriptAdmission(PageRuntimeScriptAdmissionTurnAction::new(
            owner,
            target_effect,
        ))
    }

    pub(crate) const fn parser_async_module_admission(
        owner: RendererPageMainDocumentRuntimeOwner,
        target_effect: PageParserAsyncModuleAdmissionTargetEffect,
    ) -> Self {
        Self::ParserAsyncModuleAdmission(PageParserAsyncModuleAdmissionTurnAction::new(
            owner,
            target_effect,
        ))
    }

    pub(crate) const fn runtime_script_continuation(
        owner: RendererPageMainDocumentRuntimeOwner,
        target_effect: PageRuntimeScriptContinuationTargetEffect,
    ) -> Self {
        Self::RuntimeScriptContinuation(PageRuntimeScriptContinuationTurnAction::new(
            owner,
            target_effect,
        ))
    }

    pub(crate) const fn parser_owned_module_continuation(
        owner: RendererPageMainDocumentRuntimeOwner,
        target_effect: PageParserOwnedModuleContinuationTargetEffect,
    ) -> Self {
        Self::ParserOwnedModuleContinuation(PageParserOwnedModuleContinuationTurnAction::new(
            owner,
            target_effect,
        ))
    }

    pub(crate) const fn dynamic_module_job(
        owner: RendererPageMainDocumentRuntimeOwner,
        target_effect: PageMainNativeModuleTargetEffect,
        settlement: PageMainNativeModuleSettlement,
    ) -> Self {
        Self::DynamicModuleJob(PageDynamicModuleJobTurnAction::new(
            owner,
            target_effect,
            settlement,
        ))
    }

    pub(crate) const fn native_module_owner_event(
        owner: RendererPageMainDocumentRuntimeOwner,
        target_effect: PageMainNativeModuleTargetEffect,
        settlement: PageMainNativeModuleSettlement,
    ) -> Self {
        Self::NativeModuleOwnerEvent(PageNativeModuleOwnerEventTurnAction::new(
            owner,
            target_effect,
            settlement,
        ))
    }

    pub(crate) const fn post_parse_execution(
        owner: RendererPageMainDocumentRuntimeOwner,
        execution: MainDocumentPostParseExecution,
    ) -> Self {
        Self::PostParseWork(PageMainDocumentPostParseTurnAction::executed(
            owner, execution,
        ))
    }

    /// Construct one action whose P5 completion has not yet been migrated, or
    /// the already-specialized runtime-owned module action.
    ///
    /// Runtime-script admission and continuation deliberately have no route
    /// through this generic constructor. Their stronger effect types are the
    /// proof consumed by the selected-task dispatcher.
    pub(crate) const fn remaining_or_runtime_owned(
        owner: RendererPageMainDocumentRuntimeOwner,
        kind: PageMainDocumentRuntimeActionKind,
        target_effect: PageMainDocumentRuntimeTargetEffect,
    ) -> Self {
        let result = PageMainDocumentRuntimeActionResult::new(owner, target_effect);
        match kind {
            PageMainDocumentRuntimeActionKind::RuntimeScriptAdmission
            | PageMainDocumentRuntimeActionKind::ParserAsyncModuleAdmission
            | PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation
            | PageMainDocumentRuntimeActionKind::ParserOwnedModuleContinuation
            | PageMainDocumentRuntimeActionKind::DynamicModuleJob
            | PageMainDocumentRuntimeActionKind::NativeModuleOwnerEvent => {
                panic!("migrated runtime actions require their exact effect type")
            }
            PageMainDocumentRuntimeActionKind::RuntimeOwnedModuleContinuation => {
                Self::RuntimeOwnedModuleContinuation(
                    PageRuntimeOwnedModuleContinuationTurnAction::new(result),
                )
            }
            PageMainDocumentRuntimeActionKind::PostParseWork => Self::PostParseWork(
                PageMainDocumentPostParseTurnAction::completed_by_family(result),
            ),
        }
    }

    #[cfg(test)]
    pub(crate) const fn kind(&self) -> PageMainDocumentRuntimeActionKind {
        match self {
            Self::RuntimeScriptAdmission(_) => {
                PageMainDocumentRuntimeActionKind::RuntimeScriptAdmission
            }
            Self::ParserAsyncModuleAdmission(_) => {
                PageMainDocumentRuntimeActionKind::ParserAsyncModuleAdmission
            }
            Self::RuntimeScriptContinuation(_) => {
                PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation
            }
            Self::DynamicModuleJob(_) => PageMainDocumentRuntimeActionKind::DynamicModuleJob,
            Self::RuntimeOwnedModuleContinuation(_) => {
                PageMainDocumentRuntimeActionKind::RuntimeOwnedModuleContinuation
            }
            Self::ParserOwnedModuleContinuation(_) => {
                PageMainDocumentRuntimeActionKind::ParserOwnedModuleContinuation
            }
            Self::NativeModuleOwnerEvent(_) => {
                PageMainDocumentRuntimeActionKind::NativeModuleOwnerEvent
            }
            Self::PostParseWork(_) => PageMainDocumentRuntimeActionKind::PostParseWork,
        }
    }

    #[cfg(test)]
    pub(crate) const fn owner(&self) -> RendererPageMainDocumentRuntimeOwner {
        match self {
            Self::RuntimeScriptAdmission(action) => action.owner,
            Self::ParserAsyncModuleAdmission(action) => action.owner(),
            Self::RuntimeScriptContinuation(action) => action.owner,
            Self::DynamicModuleJob(action) => action.owner(),
            Self::NativeModuleOwnerEvent(action) => action.owner(),
            Self::PostParseWork(action) => action.owner(),
            Self::RuntimeOwnedModuleContinuation(action) => action.0.owner(),
            Self::ParserOwnedModuleContinuation(action) => action.owner(),
        }
    }

    #[cfg(test)]
    pub(crate) const fn target_effect(&self) -> PageMainDocumentRuntimeTargetEffect {
        match self {
            Self::RuntimeScriptAdmission(action) => match action.target_effect {
                PageRuntimeScriptAdmissionTargetEffect::AdmittedToCurrentOwner => {
                    PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner
                }
                PageRuntimeScriptAdmissionTargetEffect::DiscardedStaleOwner => {
                    PageMainDocumentRuntimeTargetEffect::IgnoredStaleOwner
                }
            },
            Self::ParserAsyncModuleAdmission(action) => match action.target_effect() {
                PageParserAsyncModuleAdmissionTargetEffect::AdmittedToCurrentOwner => {
                    PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner
                }
                PageParserAsyncModuleAdmissionTargetEffect::RejectedByCurrentOwner => {
                    PageMainDocumentRuntimeTargetEffect::CurrentOwnerHadNoMatchingWork
                }
                PageParserAsyncModuleAdmissionTargetEffect::DiscardedStaleOwner => {
                    PageMainDocumentRuntimeTargetEffect::IgnoredStaleOwner
                }
            },
            Self::RuntimeScriptContinuation(action) => match action.target_effect {
                PageRuntimeScriptContinuationTargetEffect::AppliedToCurrentOwner(_) => {
                    PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner
                }
                PageRuntimeScriptContinuationTargetEffect::DiscardedStaleOwner => {
                    PageMainDocumentRuntimeTargetEffect::IgnoredStaleOwner
                }
            },
            Self::ParserOwnedModuleContinuation(action) => match action.target_effect() {
                PageParserOwnedModuleContinuationTargetEffect::AppliedToSelectedOwner(_) => {
                    PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner
                }
                PageParserOwnedModuleContinuationTargetEffect::CurrentOwnerReservationSpent => {
                    PageMainDocumentRuntimeTargetEffect::CurrentOwnerHadNoMatchingWork
                }
                PageParserOwnedModuleContinuationTargetEffect::DiscardedStaleOwner => {
                    PageMainDocumentRuntimeTargetEffect::IgnoredStaleOwner
                }
            },
            Self::DynamicModuleJob(action) => {
                Self::compatibility_native_module_target_effect(action.target_effect())
            }
            Self::NativeModuleOwnerEvent(action) => {
                Self::compatibility_native_module_target_effect(action.target_effect())
            }
            Self::PostParseWork(action) => action.target_effect(),
            Self::RuntimeOwnedModuleContinuation(action) => action.0.target_effect(),
        }
    }

    #[cfg(test)]
    const fn compatibility_native_module_target_effect(
        effect: PageMainNativeModuleTargetEffect,
    ) -> PageMainDocumentRuntimeTargetEffect {
        match effect {
            PageMainNativeModuleTargetEffect::AppliedToSelectedOwner(_) => {
                PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner
            }
            PageMainNativeModuleTargetEffect::CurrentOwnerReservationSpent => {
                PageMainDocumentRuntimeTargetEffect::CurrentOwnerHadNoMatchingWork
            }
            PageMainNativeModuleTargetEffect::DiscardedStaleOwner => {
                PageMainDocumentRuntimeTargetEffect::IgnoredStaleOwner
            }
        }
    }

    #[cfg(test)]
    pub(crate) const fn runtime_script_continuation_body_effect(
        &self,
    ) -> Option<RuntimeScriptContinuationBodyEffect> {
        match self {
            Self::RuntimeScriptContinuation(action) => match action.target_effect {
                PageRuntimeScriptContinuationTargetEffect::AppliedToCurrentOwner(effect) => {
                    Some(effect)
                }
                PageRuntimeScriptContinuationTargetEffect::DiscardedStaleOwner => None,
            },
            _ => None,
        }
    }
}

pub(crate) type PageMainDocumentRuntimeTurnOutcome =
    PageOwnerTurnOutcome<PageMainDocumentRuntimeTurnAction>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PageId,
        document_script_scheduler::{DocumentScriptExecutionLane, PageOwnedDocumentScriptWork},
        frame_owner_model::{DocumentId, FrameSchedulerLaneId, LocalWindowId},
        planning::{
            PreparedScript, PreparedScriptRuntimeGeneration, ScriptFetchMetadata, ScriptSource,
            SharedScriptSourceLoad,
        },
        runtime::RendererPageToken,
        types::{ScriptKind, ScriptMode, ScriptSourceKind},
    };
    use std::time::Duration;
    use url::Url;

    fn task_owner(document: u64) -> FrameDocumentTaskOwner {
        FrameDocumentTaskOwner::new(
            FrameSchedulerLaneId(1),
            LocalWindowId(2),
            DocumentId(document),
        )
    }

    fn document_token(generation: u64) -> RendererDocumentToken {
        RendererDocumentToken {
            page_id: PageId::new_for_testing(7),
            generation,
        }
    }

    fn prepared_script(runtime_generation: PreparedScriptRuntimeGeneration) -> PreparedScript {
        let url = Url::parse("https://main-runtime.test/async.js").expect("script URL");
        PreparedScript {
            position: 1,
            node_id: crate::dom::NodeId::new(1),
            kind: ScriptKind::Classic,
            mode: ScriptMode::Async,
            source_kind: ScriptSourceKind::External,
            fetch_metadata: ScriptFetchMetadata::default(),
            source: ScriptSource::External,
            url: url.clone(),
            base_url: url.clone(),
            initiator_url: url,
            host_script_handle: None,
            runtime_generation,
        }
    }

    fn source_and_producer(
        owner: FrameDocumentTaskOwner,
        runtime_generation: u64,
    ) -> (
        RendererPageMainDocumentRuntimeSource,
        RendererPageMainDocumentRuntimeProducer,
        tokio::sync::mpsc::UnboundedReceiver<super::super::RendererOwnerWake>,
    ) {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageMainDocumentRuntimeSource::new(RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(PageId::new_for_testing(7)),
        ));
        let producer = source
            .route()
            .sender(document_token(1))
            .bind_producer(owner, runtime_generation);
        (source, producer, wake_rx)
    }

    #[test]
    fn lifecycle_owner_mismatch_is_rejected_before_readiness() {
        let owner = task_owner(3);
        let (mut source, producer, mut wake_rx) = source_and_producer(owner, 5);

        assert_eq!(
            producer.send_lifecycle_work(
                super::super::PostParseLifecycleWork::CheckMainDocumentCompletion {
                    owner: task_owner(4),
                },
            ),
            Err(RendererPageMainDocumentRuntimeAdmissionError::TargetMismatch)
        );
        assert!(!source.has_ready_task());
        assert!(wake_rx.try_recv().is_err());
    }

    #[test]
    fn prepared_script_generation_mismatch_is_rejected_before_readiness() {
        let owner = task_owner(3);
        let (mut source, producer, mut wake_rx) = source_and_producer(owner, 5);
        let work =
            PostParsePageOwnedWork::document_script_work(PageOwnedDocumentScriptWork::script(
                DocumentScriptExecutionLane::AsyncPhase,
                prepared_script(PreparedScriptRuntimeGeneration::Live(6)),
            ));

        assert_eq!(
            producer.send_post_parse_work_when_ready(work),
            Err(RendererPageMainDocumentRuntimeAdmissionError::TargetMismatch)
        );
        assert!(!source.has_ready_task());
        assert!(wake_rx.try_recv().is_err());
    }

    #[test]
    fn parser_owned_module_continuation_is_one_exact_ready_action() {
        let owner = task_owner(3);
        let runtime_generation = 5;
        let (mut source, producer, mut wake_rx) = source_and_producer(owner, runtime_generation);

        producer
            .send_parser_owned_module_continuation()
            .expect("parser-owned continuation should enter the bound source");
        let wake = wake_rx
            .try_recv()
            .expect("empty-to-nonempty transition should wake the Page owner");
        assert_eq!(wake.page_id().as_u64(), 7);

        let (_, task) = source
            .pop_front()
            .expect("one ready parser continuation should retain one task");
        assert_eq!(task.owner().document_owner(), owner);
        assert_eq!(
            task.action_kind(),
            PageMainDocumentRuntimeActionKind::ParserOwnedModuleContinuation
        );
        assert!(!source.has_ready_task());
        assert!(wake_rx.try_recv().is_err());
    }

    #[test]
    fn native_module_owner_event_is_one_exact_ready_action() {
        let owner = task_owner(3);
        let runtime_generation = 5;
        let (mut source, producer, mut wake_rx) = source_and_producer(owner, runtime_generation);

        producer
            .send_native_module_owner_event()
            .expect("native module owner event should enter the bound source");
        let wake = wake_rx
            .try_recv()
            .expect("empty-to-nonempty transition should wake the Page owner");
        assert_eq!(wake.page_id().as_u64(), 7);

        let (_, task) = source
            .pop_front()
            .expect("one native module owner event should retain one task");
        assert_eq!(task.owner().document_owner(), owner);
        assert_eq!(
            task.action_kind(),
            PageMainDocumentRuntimeActionKind::NativeModuleOwnerEvent
        );
        assert!(!source.has_ready_task());
        assert!(wake_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn pending_source_load_becomes_ready_once_under_its_bound_document() {
        let owner = task_owner(3);
        let runtime_generation = 5;
        let (mut source, producer, mut wake_rx) = source_and_producer(owner, runtime_generation);
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let source_load = SharedScriptSourceLoad::spawn_for_test(async move {
            finish_rx.await.expect("source release");
            Ok("globalThis.__mainRuntimeReady = true;".to_owned())
        });
        let work = PostParsePageOwnedWork::document_script_work(
            PageOwnedDocumentScriptWork::script_waiting_for_source(
                DocumentScriptExecutionLane::AsyncPhase,
                prepared_script(PreparedScriptRuntimeGeneration::PendingBinding),
                source_load,
            ),
        );

        producer
            .send_post_parse_work_when_ready(work)
            .expect("pending work should retain its exact producer");
        assert!(!source.has_ready_task());
        assert!(wake_rx.try_recv().is_err());

        finish_tx.send(()).expect("release source load");
        tokio::time::timeout(Duration::from_secs(1), wake_rx.recv())
            .await
            .expect("source completion should wake the owner")
            .expect("owner wake route should remain open");

        let (_, task) = source
            .pop_front()
            .expect("completed source work should enter the ready source once");
        assert_eq!(task.owner().document_owner(), owner);
        let RendererPageMainDocumentRuntimeAction::ExecuteReadyPostParseWork(work) =
            task.into_action()
        else {
            panic!("source completion must preserve the concrete post-parse action");
        };
        assert_eq!(
            work.into_post_parse_work()
                .as_script()
                .expect("document-script work")
                .runtime_generation,
            PreparedScriptRuntimeGeneration::Live(runtime_generation)
        );
        assert!(!source.has_ready_task());
        assert!(wake_rx.try_recv().is_err());
    }
}
