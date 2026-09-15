use crate::page_task_queue::{
    PageChildClassicScriptSourceLoadTargetEffect, PageChildClassicScriptSourceLoadTurnAction,
    PageChildClassicScriptSourceLoadTurnOutcome, PageChildDocumentLifecycleTargetEffect,
    PageChildDocumentLifecycleTurnAction, PageChildDocumentLifecycleTurnOutcome,
    PageChildDocumentScriptReadyTargetEffect, PageChildDocumentScriptReadyTurnAction,
    PageChildDocumentScriptReadyTurnOutcome, PageChildHostLoadTargetEffect,
    PageChildHostLoadTurnAction, PageChildHostLoadTurnOutcome,
    PageChildParserModuleRootStartTargetEffect, PageChildParserModuleRootStartTurnAction,
    PageChildParserModuleRootStartTurnOutcome, PageChildRealmMaterializationTargetEffect,
    PageChildRealmMaterializationTurnAction, PageChildRealmMaterializationTurnOutcome,
    RendererPageChildFrameTask, RendererPageChildFrameTaskOwner, RendererPageChildFrameTaskTarget,
};
use crate::script_vm::{
    ChildDocumentLifecycleRunOutcome, ChildDocumentScriptActivity,
    ChildDocumentScriptReadyRunOutcome, ChildHostLoadRunOutcome,
    ChildRealmMaterializationApplication, ChildRealmMaterializationBodyActivity,
};

use super::page_child_document_script_ready_task_completion::PageChildDocumentScriptReadyCompletionBoundary;
use super::{IntoPageTaskCompletion, PageVm};

/// Proof that the Page arbiter matched a selected materialization request
/// against both its root PageVm namespace and exact child Document.
pub(crate) struct AuthorizedCurrentPageChildRealmMaterialization(RendererPageChildFrameTask);

impl AuthorizedCurrentPageChildRealmMaterialization {
    fn new(task: RendererPageChildFrameTask) -> Self {
        Self(task)
    }

    pub(crate) fn into_task(self) -> RendererPageChildFrameTask {
        self.0
    }

    #[cfg(test)]
    pub(crate) fn new_for_executor_test(task: RendererPageChildFrameTask) -> Self {
        Self(task)
    }
}

/// Proof that the Page arbiter matched the root PageVm namespace, exact child
/// Document/realm, and PageVm-local payload ledger slot.
pub(crate) struct AuthorizedCurrentPageChildDocumentScriptReady(RendererPageChildFrameTask);

impl AuthorizedCurrentPageChildDocumentScriptReady {
    fn new(task: RendererPageChildFrameTask) -> Self {
        Self(task)
    }

    pub(crate) fn into_task(self) -> RendererPageChildFrameTask {
        self.0
    }

    #[cfg(test)]
    pub(crate) fn new_for_executor_test(task: RendererPageChildFrameTask) -> Self {
        Self(task)
    }
}

/// Proof that the Page arbiter matched an exact pending child Document
/// lifecycle transition and its materialized realm.
pub(crate) struct AuthorizedCurrentPageChildDocumentLifecycle(RendererPageChildFrameTask);

impl AuthorizedCurrentPageChildDocumentLifecycle {
    fn new(task: RendererPageChildFrameTask) -> Self {
        Self(task)
    }

    pub(crate) fn into_task(self) -> RendererPageChildFrameTask {
        self.0
    }

    #[cfg(test)]
    pub(crate) fn new_for_executor_test(task: RendererPageChildFrameTask) -> Self {
        Self(task)
    }
}

/// Proof that one stable HostLoad task still owns the exact current child
/// Document load-delivery reservation.
pub(crate) struct AuthorizedCurrentPageChildHostLoad(RendererPageChildFrameTask);

impl AuthorizedCurrentPageChildHostLoad {
    fn new(task: RendererPageChildFrameTask) -> Self {
        Self(task)
    }

    pub(crate) fn into_task(self) -> RendererPageChildFrameTask {
        self.0
    }

    #[cfg(test)]
    pub(crate) fn new_for_executor_test(task: RendererPageChildFrameTask) -> Self {
        Self(task)
    }
}

/// Proof that the Page arbiter matched a concrete parser classic source-load
/// client against its root PageVm, exact child Document and reserved realm.
pub(crate) struct AuthorizedCurrentPageChildClassicScriptSourceLoad(RendererPageChildFrameTask);

impl AuthorizedCurrentPageChildClassicScriptSourceLoad {
    fn new(task: RendererPageChildFrameTask) -> Self {
        Self(task)
    }

    pub(crate) fn into_task(self) -> RendererPageChildFrameTask {
        self.0
    }

    #[cfg(test)]
    pub(crate) fn new_for_executor_test(task: RendererPageChildFrameTask) -> Self {
        Self(task)
    }
}

/// Proof that the Page arbiter matched the root PageVm namespace and exact
/// child Document/realm before admitting a parser module graph start.
pub(crate) struct AuthorizedCurrentPageChildParserModuleRootStart(RendererPageChildFrameTask);

impl AuthorizedCurrentPageChildParserModuleRootStart {
    fn new(task: RendererPageChildFrameTask) -> Self {
        Self(task)
    }

    pub(crate) fn into_task(self) -> RendererPageChildFrameTask {
        self.0
    }

    #[cfg(test)]
    pub(crate) fn new_for_executor_test(task: RendererPageChildFrameTask) -> Self {
        Self(task)
    }
}

impl PageVm {
    fn current_page_child_document_lifecycle_owner(
        &self,
        expected: RendererPageChildFrameTaskOwner,
    ) -> Option<RendererPageChildFrameTaskOwner> {
        let RendererPageChildFrameTaskTarget::DocumentLifecycle(expected_target) =
            expected.target()
        else {
            return None;
        };
        if expected.root_document() != self.document_lifecycle.identity().document {
            return None;
        }
        let target = self
            .vm()
            .current_child_document_lifecycle_target(expected_target)?;
        Some(RendererPageChildFrameTaskOwner::new(
            expected.root_document(),
            RendererPageChildFrameTaskTarget::DocumentLifecycle(target),
        ))
    }

    pub(super) fn apply_selected_page_child_document_lifecycle_turn(
        &mut self,
        task: RendererPageChildFrameTask,
    ) -> PageChildDocumentLifecycleTurnOutcome {
        let owner = task.owner();
        let current_owner = self.current_page_child_document_lifecycle_owner(owner);
        let target_effect = if current_owner == Some(owner) {
            match self.vm_mut().apply_current_child_document_lifecycle(
                AuthorizedCurrentPageChildDocumentLifecycle::new(task),
            ) {
                Ok(ChildDocumentLifecycleRunOutcome::EventDispatched) => {
                    PageChildDocumentLifecycleTargetEffect::EventDispatchedToCurrentOwner
                }
                Ok(ChildDocumentLifecycleRunOutcome::ConsumedWithoutEvent) => {
                    PageChildDocumentLifecycleTargetEffect::ConsumedCurrentOwnerWithoutEvent
                }
                Err(error) => {
                    self.vm_mut().record_runtime_warning(format_args!(
                        "child DocumentLifecycle application failed: {error}"
                    ));
                    PageChildDocumentLifecycleTargetEffect::FailedForCurrentOwner
                }
            }
        } else {
            tracing::debug!(
                ?owner,
                ?current_owner,
                "discarded stale exact-Document child lifecycle action"
            );
            PageChildDocumentLifecycleTargetEffect::DiscardedStaleOwner { current_owner }
        };
        let action = PageChildDocumentLifecycleTurnAction {
            owner,
            target_effect,
        };
        PageChildDocumentLifecycleTurnOutcome::new(action)
    }

    fn current_page_child_host_load_owner(
        &self,
        expected: RendererPageChildFrameTaskOwner,
    ) -> Option<RendererPageChildFrameTaskOwner> {
        let RendererPageChildFrameTaskTarget::HostLoad(expected_target) = expected.target() else {
            return None;
        };
        if expected.root_document() != self.document_lifecycle.identity().document {
            return None;
        }
        let target = self.vm().current_child_host_load_target(expected_target)?;
        Some(RendererPageChildFrameTaskOwner::new(
            expected.root_document(),
            RendererPageChildFrameTaskTarget::HostLoad(target),
        ))
    }

    pub(super) fn apply_selected_page_child_host_load_turn(
        &mut self,
        task: RendererPageChildFrameTask,
    ) -> PageChildHostLoadTurnOutcome {
        let owner = task.owner();
        let RendererPageChildFrameTaskTarget::HostLoad(target) = owner.target() else {
            unreachable!("HostLoad executor received another child-frame task kind")
        };
        let current_owner = self.current_page_child_host_load_owner(owner);
        let target_effect = if current_owner == Some(owner) {
            match self
                .vm_mut()
                .apply_current_child_host_load(AuthorizedCurrentPageChildHostLoad::new(task))
            {
                Ok(ChildHostLoadRunOutcome::CallbackDispatched) => {
                    PageChildHostLoadTargetEffect::CallbackDispatchedToCurrentOwner
                }
                Ok(ChildHostLoadRunOutcome::ConsumedWithoutCallback) => {
                    PageChildHostLoadTargetEffect::ConsumedCurrentOwnerWithoutCallback
                }
                Err(error) => {
                    self.vm_mut().discard_stale_child_host_load(target);
                    self.vm_mut().record_runtime_warning(format_args!(
                        "child HostLoad application failed: {error}"
                    ));
                    PageChildHostLoadTargetEffect::FailedForCurrentOwner
                }
            }
        } else {
            if owner.root_document() == self.document_lifecycle.identity().document {
                self.vm_mut().discard_stale_child_host_load(target);
            }
            tracing::debug!(
                ?owner,
                ?current_owner,
                "discarded stale exact-Document child HostLoad"
            );
            PageChildHostLoadTargetEffect::DiscardedStaleOwner { current_owner }
        };
        let action = PageChildHostLoadTurnAction {
            owner,
            target_effect,
        };
        PageChildHostLoadTurnOutcome::new(action)
    }

    fn current_page_child_realm_materialization_owner(
        &self,
        expected: RendererPageChildFrameTaskOwner,
    ) -> Option<RendererPageChildFrameTaskOwner> {
        let RendererPageChildFrameTaskTarget::RealmMaterialization(expected_target) =
            expected.target()
        else {
            return None;
        };
        let target = self
            .vm()
            .current_child_realm_materialization_target(expected_target)?;
        Some(RendererPageChildFrameTaskOwner::new(
            self.document_lifecycle.identity().document,
            RendererPageChildFrameTaskTarget::RealmMaterialization(target),
        ))
    }

    fn apply_selected_page_child_realm_materialization_turn(
        &mut self,
        task: RendererPageChildFrameTask,
    ) -> PageChildRealmMaterializationTurnOutcome {
        let owner = task.owner();
        let current_owner = self.current_page_child_realm_materialization_owner(owner);
        let target_effect = if current_owner == Some(owner) {
            let runtime_isolated_worlds = self.runtime_isolated_worlds.clone();
            match self.vm_mut().apply_current_child_realm_materialization(
                AuthorizedCurrentPageChildRealmMaterialization::new(task),
                &runtime_isolated_worlds,
            ) {
                Ok(ChildRealmMaterializationApplication::Materialized(
                    ChildRealmMaterializationBodyActivity::StateOnly,
                )) => PageChildRealmMaterializationTargetEffect::MaterializedCurrentOwnerWithoutDocumentStartScript,
                Ok(ChildRealmMaterializationApplication::Materialized(
                    ChildRealmMaterializationBodyActivity::DocumentStartScript,
                )) => PageChildRealmMaterializationTargetEffect::MaterializedCurrentOwnerAfterDocumentStartScript,
                Ok(ChildRealmMaterializationApplication::NoPendingRequest) => {
                    PageChildRealmMaterializationTargetEffect::CurrentOwnerHadNoPendingRequest
                }
                Err(error) => {
                    self.vm_mut().record_runtime_warning(format_args!(
                        "child FrameRealm materialization failed: {error}"
                    ));
                    PageChildRealmMaterializationTargetEffect::FailedCurrentOwner
                }
            }
        } else {
            // A different root token names another PageVm namespace. Never
            // use its child-local ids to touch this PageVm. Same-root stale
            // state can be removed by exact child Document identity.
            if owner.root_document() == self.document_lifecycle.identity().document {
                self.vm_mut()
                    .discard_stale_child_realm_materialization(owner);
            }
            tracing::debug!(
                ?owner,
                ?current_owner,
                "discarded stale exact-Document child realm materialization"
            );
            PageChildRealmMaterializationTargetEffect::IgnoredStaleOwner { current_owner }
        };
        let action = PageChildRealmMaterializationTurnAction {
            owner,
            target_effect,
        };
        PageChildRealmMaterializationTurnOutcome::new(action)
    }

    fn current_page_child_document_script_ready_owner(
        &self,
        expected: RendererPageChildFrameTaskOwner,
    ) -> Option<RendererPageChildFrameTaskOwner> {
        let RendererPageChildFrameTaskTarget::DocumentScriptReady(expected_target) =
            expected.target()
        else {
            return None;
        };
        if expected.root_document() != self.document_lifecycle.identity().document {
            return None;
        }
        let target = self
            .vm()
            .current_pending_child_document_script_ready_target(expected_target)?;
        Some(RendererPageChildFrameTaskOwner::new(
            expected.root_document(),
            RendererPageChildFrameTaskTarget::DocumentScriptReady(target),
        ))
    }

    pub(super) async fn apply_selected_page_child_document_script_ready_turn(
        &mut self,
        task: RendererPageChildFrameTask,
    ) -> anyhow::Result<PageChildDocumentScriptReadyTurnOutcome> {
        let owner = task.owner();
        let RendererPageChildFrameTaskTarget::DocumentScriptReady(target) = owner.target() else {
            unreachable!("document-script executor received another child-frame task kind")
        };
        let task_id = target.task_id();
        let current_owner = self.current_page_child_document_script_ready_owner(owner);
        let target_effect = if current_owner == Some(owner) {
            let run = self
                .vm_mut()
                .apply_current_child_document_script_ready(
                    AuthorizedCurrentPageChildDocumentScriptReady::new(task),
                )
                .await?;
            let (made_progress, script_or_event) = match run {
                ChildDocumentScriptReadyRunOutcome::Applied(outcome) => (
                    outcome.made_progress(),
                    outcome.activity() == ChildDocumentScriptActivity::ScriptOrEvent,
                ),
                #[cfg(test)]
                ChildDocumentScriptReadyRunOutcome::DiscardedStale => {
                    unreachable!(
                        "Page authorization must reject stale child DocumentScriptReady work before ScriptVm execution"
                    )
                }
            };
            if script_or_event {
                PageChildDocumentScriptReadyTargetEffect::AppliedScriptOrEventToCurrentOwner {
                    made_progress,
                }
            } else {
                PageChildDocumentScriptReadyTargetEffect::AppliedWithoutScriptOrEventToCurrentOwner {
                    made_progress,
                }
            }
        } else {
            if owner.root_document() == self.document_lifecycle.identity().document {
                self.vm_mut()
                    .discard_stale_child_document_script_ready_task(task_id);
            }
            tracing::debug!(
                ?owner,
                ?current_owner,
                ?task_id,
                "discarded stale exact-Document child script task"
            );
            PageChildDocumentScriptReadyTargetEffect::DiscardedStaleOwner { current_owner }
        };
        let action = PageChildDocumentScriptReadyTurnAction {
            owner,
            task_id,
            target_effect,
        };
        Ok(PageChildDocumentScriptReadyTurnOutcome::new(action))
    }

    fn current_page_child_classic_source_load_owner(
        &self,
        expected: RendererPageChildFrameTaskOwner,
    ) -> Option<RendererPageChildFrameTaskOwner> {
        let RendererPageChildFrameTaskTarget::ClassicScriptSourceLoad(expected_target) =
            expected.target()
        else {
            return None;
        };
        if expected.root_document() != self.document_lifecycle.identity().document {
            return None;
        }
        let target = self
            .vm()
            .current_child_classic_source_load_target(expected_target)?;
        Some(RendererPageChildFrameTaskOwner::new(
            expected.root_document(),
            RendererPageChildFrameTaskTarget::ClassicScriptSourceLoad(target),
        ))
    }

    fn apply_selected_page_child_classic_source_load_turn(
        &mut self,
        task: RendererPageChildFrameTask,
    ) -> PageChildClassicScriptSourceLoadTurnOutcome {
        let owner = task.owner();
        let current_owner = self.current_page_child_classic_source_load_owner(owner);
        let target_effect = if current_owner == Some(owner) {
            match self.vm_mut().apply_current_child_classic_source_load(
                AuthorizedCurrentPageChildClassicScriptSourceLoad::new(task),
            ) {
                crate::frame_owner_model::FrameDocumentClassicScriptSourceLoadStartOutcome::NetworkRequestStarted => {
                    PageChildClassicScriptSourceLoadTargetEffect::NetworkRequestStartedForCurrentOwner
                }
                crate::frame_owner_model::FrameDocumentClassicScriptSourceLoadStartOutcome::RejectedBeforeNetworkStart => {
                    PageChildClassicScriptSourceLoadTargetEffect::RejectedBeforeNetworkStartForCurrentOwner
                }
            }
        } else {
            if owner.root_document() == self.document_lifecycle.identity().document {
                let source_load = task.into_classic_script_source_load_task();
                self.vm_mut()
                    .settle_stale_child_classic_source_load(&source_load);
            }
            tracing::debug!(
                ?owner,
                ?current_owner,
                "discarded stale exact-Document child classic source load"
            );
            PageChildClassicScriptSourceLoadTargetEffect::DiscardedStaleOwner { current_owner }
        };
        let action = PageChildClassicScriptSourceLoadTurnAction {
            owner,
            target_effect,
        };
        PageChildClassicScriptSourceLoadTurnOutcome::new(action)
    }

    fn current_page_child_parser_module_root_start_owner(
        &self,
        expected: RendererPageChildFrameTaskOwner,
    ) -> Option<RendererPageChildFrameTaskOwner> {
        let RendererPageChildFrameTaskTarget::ParserModuleRootStart(expected_target) =
            expected.target()
        else {
            return None;
        };
        if expected.root_document() != self.document_lifecycle.identity().document {
            return None;
        }
        let target = self
            .vm()
            .current_child_parser_module_root_start_target(expected_target)?;
        Some(RendererPageChildFrameTaskOwner::new(
            expected.root_document(),
            RendererPageChildFrameTaskTarget::ParserModuleRootStart(target),
        ))
    }

    fn apply_selected_page_child_parser_module_root_start_turn(
        &mut self,
        task: RendererPageChildFrameTask,
    ) -> PageChildParserModuleRootStartTurnOutcome {
        let owner = task.owner();
        let current_owner = self.current_page_child_parser_module_root_start_owner(owner);
        let target_effect = if current_owner == Some(owner) {
            self.vm_mut().apply_current_child_parser_module_root_start(
                AuthorizedCurrentPageChildParserModuleRootStart::new(task),
            );
            PageChildParserModuleRootStartTargetEffect::ConsumedByCurrentOwner
        } else {
            if owner.root_document() == self.document_lifecycle.identity().document {
                let root_start = task.into_parser_module_root_start_task();
                self.vm_mut()
                    .settle_stale_child_parser_module_root_start(&root_start);
            }
            tracing::debug!(
                ?owner,
                ?current_owner,
                "discarded stale exact-Document child parser module root start"
            );
            PageChildParserModuleRootStartTargetEffect::DiscardedStaleOwner { current_owner }
        };
        let action = PageChildParserModuleRootStartTurnAction {
            owner,
            target_effect,
        };
        PageChildParserModuleRootStartTurnOutcome::new(action)
    }

    pub(in crate::runtime) async fn apply_selected_page_child_frame_task_turn(
        &mut self,
        task: RendererPageChildFrameTask,
    ) -> anyhow::Result<()> {
        match task.owner().target() {
            RendererPageChildFrameTaskTarget::RealmMaterialization(_) => {
                let outcome = self.apply_selected_page_child_realm_materialization_turn(task);
                self.finish_selected_page_child_realm_materialization(outcome.action)?;
                Ok(())
            }
            RendererPageChildFrameTaskTarget::DocumentLifecycle(_) => {
                let outcome = self.apply_selected_page_child_document_lifecycle_turn(task);
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                )
                .await?;
                Ok(())
            }
            RendererPageChildFrameTaskTarget::DocumentScriptReady(_) => {
                let outcome = self
                    .apply_selected_page_child_document_script_ready_turn(task)
                    .await?;
                match outcome.action.into_completion_boundary() {
                    PageChildDocumentScriptReadyCompletionBoundary::Complete(completion) => {
                        self.finish_selected_page_task_completion(completion)
                            .await?;
                    }
                    PageChildDocumentScriptReadyCompletionBoundary::DiscardedStale => {}
                }
                Ok(())
            }
            RendererPageChildFrameTaskTarget::HostLoad(_) => {
                let outcome = self.apply_selected_page_child_host_load_turn(task);
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                )
                .await?;
                Ok(())
            }
            RendererPageChildFrameTaskTarget::ParserModuleRootStart(_) => {
                let outcome = self.apply_selected_page_child_parser_module_root_start_turn(task);
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                )
                .await?;
                Ok(())
            }
            RendererPageChildFrameTaskTarget::ClassicScriptSourceLoad(_) => {
                let outcome = self.apply_selected_page_child_classic_source_load_turn(task);
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                )
                .await?;
                Ok(())
            }
        }
    }

    #[cfg(test)]
    /// Run only exact-owner materialization application for domain fixtures.
    /// Complete task tests must use `PageSelectedTaskTestSelector` so the
    /// production dispatcher owns the sole task-end checkpoint.
    pub(in crate::runtime) fn run_child_realm_materialization_body_for_test(
        &mut self,
    ) -> anyhow::Result<Option<PageChildRealmMaterializationTurnOutcome>> {
        let task_sources = self.page_task_executor_sources_for_test();
        let Some(task) = task_sources.take_scheduler_task_for_executor_test(|descriptor| {
            matches!(
                descriptor,
                crate::page_task_queue::RendererPageReadyDescriptor::ChildFrameTask {
                    owner,
                    ..
                } if matches!(
                    owner.target(),
                    RendererPageChildFrameTaskTarget::RealmMaterialization(_)
                )
            )
        }) else {
            return Ok(None);
        };
        let crate::page_task_queue::RendererPageSchedulerTask::ChildFrameTask(task) = task else {
            unreachable!("child-frame descriptor must dequeue its own family source")
        };
        Ok(Some(
            self.apply_selected_page_child_realm_materialization_turn(task),
        ))
    }

    #[cfg(test)]
    /// Apply one exact parser-module root-start body without submitting the
    /// selected Page task's completion. Complete task tests must use the shared
    /// exact selector and production dispatcher.
    pub(in crate::runtime) fn run_child_parser_module_root_start_body_for_test(
        &mut self,
    ) -> anyhow::Result<Option<PageChildParserModuleRootStartTurnOutcome>> {
        let task_sources = self.page_task_executor_sources_for_test();
        let Some(task) = task_sources.take_scheduler_task_for_executor_test(|descriptor| {
            matches!(
                descriptor,
                crate::page_task_queue::RendererPageReadyDescriptor::ChildFrameTask {
                    owner,
                    ..
                } if matches!(
                    owner.target(),
                    RendererPageChildFrameTaskTarget::ParserModuleRootStart(_)
                )
            )
        }) else {
            return Ok(None);
        };
        let crate::page_task_queue::RendererPageSchedulerTask::ChildFrameTask(task) = task else {
            unreachable!("child-frame descriptor must dequeue its own family source")
        };
        Ok(Some(
            self.apply_selected_page_child_parser_module_root_start_turn(task),
        ))
    }

    #[cfg(test)]
    /// Apply one exact classic source-start body without submitting the
    /// selected Page task's completion. Complete task tests must use the shared
    /// exact selector and production dispatcher.
    pub(in crate::runtime) fn run_child_classic_source_load_body_for_test(
        &mut self,
    ) -> anyhow::Result<Option<PageChildClassicScriptSourceLoadTurnOutcome>> {
        let task_sources = self.page_task_executor_sources_for_test();
        let Some(task) = task_sources.take_scheduler_task_for_executor_test(|descriptor| {
            matches!(
                descriptor,
                crate::page_task_queue::RendererPageReadyDescriptor::ChildFrameTask {
                    owner,
                    ..
                } if matches!(
                    owner.target(),
                    RendererPageChildFrameTaskTarget::ClassicScriptSourceLoad(_)
                )
            )
        }) else {
            return Ok(None);
        };
        let crate::page_task_queue::RendererPageSchedulerTask::ChildFrameTask(task) = task else {
            unreachable!("child-frame descriptor must dequeue its own family source")
        };
        Ok(Some(
            self.apply_selected_page_child_classic_source_load_turn(task),
        ))
    }
}
