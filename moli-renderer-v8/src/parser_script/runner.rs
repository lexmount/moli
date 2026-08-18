use crate::document_runtime::DomHandle;
use crate::parser_script::action::{
    ParserClassicScriptNetworkRecordUrls, ParserClassicScriptRunnerStep,
    ParserPendingClassicScriptBeginExecutionAction, ParserPendingClassicScriptDisposedReadyAction,
    ParserPendingClassicScriptFinishedExecutionAction, ParserPendingClassicScriptReadyAction,
    ParserPendingClassicScriptSourceFailureAction, ParserPendingClassicScriptSourceLoadAction,
    ParserPendingClassicScriptSourceLoadCandidate,
    ParserPendingClassicScriptSourceLoadClientAction,
    ParserPendingClassicScriptSourceLoadCompletionAction,
    ParserPendingClassicScriptSourceLoadCompletionRecord,
    ParserPendingClassicScriptSourceLoadRequest, ParserPendingClassicScriptSourceLoadWaitAction,
    ParserPendingClassicScriptSourceResultAction,
};
use crate::parser_script::context::{
    ParserClassicScriptExecutionGateState, ParserClassicScriptSourceLoadCompletionState,
    ParserClassicScriptSourceLoadOutcomeState, ParserClassicScriptSourceLoadStartState,
    ParserClassicScriptSourceLoadState, ParserClassicScriptSourceLoadWaitState,
    ParserClassicScriptSourceResultState,
};
use crate::parser_script::item::ParserClassicScriptRunnerItem;
use crate::parser_script::owner::{
    ParserScriptBeginExecutionOwner, ParserScriptBeginSourceLoadOwner,
    ParserScriptDisposeReadyOwner, ParserScriptExecutionGate, ParserScriptFinishExecutionOwner,
    ParserScriptOwner, ParserScriptReadyOwner, ParserScriptSourceFailureOwner,
    ParserScriptSourceLoadClientOwner, ParserScriptSourceLoadCompletionOwner,
    ParserScriptSourceLoadWaitOwner, ParserScriptSourceResultOwner,
};
#[cfg(test)]
use crate::parser_script::payload::{
    ParserClassicScriptMetadata, ParserClassicScriptSourceFailure, ParserPreparedClassicScript,
};
use crate::parser_script::payload::{
    ParserClassicScriptSourceResult, ParserExecutableClassicScript,
};
#[cfg(test)]
use crate::parser_script::pending::ParserPendingClassicScriptEntry;
use crate::parser_script::projection::{
    ParserClassicScriptBlockedOnExecution, ParserClassicScriptBlockedOnSourceLoad,
    ParserClassicScriptExecutionGateProjection, ParserClassicScriptNextActionWithBlockedScript,
    ParserClassicScriptSourceResultApplication,
};
use crate::parser_script::queue::ParserClassicScriptRunnerQueue;
#[cfg(test)]
use crate::parser_script::slot::ParserClassicScriptRunnerSlot;
use crate::planning::PreparedScript;

#[derive(Debug, Clone)]
pub(crate) struct ParserClassicScriptRunner<C> {
    parser_blocking_scripts: ParserClassicScriptRunnerQueue<C>,
    deferred_scripts: ParserClassicScriptRunnerQueue<C>,
}

impl<C> ParserClassicScriptRunner<C>
where
    C: ParserClassicScriptExecutionGateState,
{
    pub(crate) fn empty() -> Self {
        Self {
            parser_blocking_scripts: ParserClassicScriptRunnerQueue::empty(),
            deferred_scripts: ParserClassicScriptRunnerQueue::empty(),
        }
    }

    pub(crate) fn from_parser_blocking_script(script: ParserClassicScriptRunnerItem<C>) -> Self {
        let mut runner = Self::empty();
        runner.parser_blocking_scripts.push(script);
        runner
    }

    #[cfg(test)]
    pub(crate) fn new_parser_blocking(
        scripts: impl IntoIterator<Item = ParserClassicScriptRunnerItem<C>>,
    ) -> Self {
        Self {
            parser_blocking_scripts: ParserClassicScriptRunnerQueue::new(scripts),
            deferred_scripts: ParserClassicScriptRunnerQueue::empty(),
        }
    }

    pub(crate) fn push_parser_blocking_script(&mut self, script: ParserClassicScriptRunnerItem<C>) {
        self.parser_blocking_scripts.push(script);
    }

    pub(crate) fn push_deferred_external_script_with_load_action<Owner>(
        &mut self,
        mut script: ParserClassicScriptRunnerItem<C>,
        load_id: u64,
        owner: &mut Owner,
    ) -> Option<Owner::SourceLoadAction>
    where
        C: ParserClassicScriptSourceLoadStartState,
        Owner: ParserScriptBeginSourceLoadOwner<C>,
    {
        let action =
            script.begin_runner_external_load_with_load_id_and_owner(Some(load_id), owner)?;
        self.deferred_scripts.push(script);
        Some(action)
    }

    pub(crate) fn has_parser_blocking_script(&self) -> bool {
        self.parser_blocking_scripts.has_current()
    }

    pub(crate) fn current_parser_blocking_context(&self) -> Option<&C> {
        Some(self.parser_blocking_scripts.current()?.context())
    }

    pub(crate) fn current_deferred_context(&self) -> Option<&C> {
        Some(self.deferred_scripts.current()?.context())
    }

    pub(crate) fn all_script_contexts_match(&self, predicate: impl FnMut(&C) -> bool) -> bool {
        let mut predicate = predicate;
        self.parser_blocking_scripts
            .all_contexts_match(&mut predicate)
            && self.deferred_scripts.all_contexts_match(predicate)
    }

    pub(crate) fn current_parser_blocking_script_handle(&self) -> Option<DomHandle> {
        self.current_parser_blocking_script()?
            .runner_metadata()
            .map(|metadata| metadata.script_handle())
    }

    pub(crate) fn current_deferred_script_handle(&self) -> Option<DomHandle> {
        self.deferred_scripts
            .current()?
            .runner_metadata()
            .map(|metadata| metadata.script_handle())
    }

    pub(crate) fn install_parser_blocking_script_blocked_on_execution(
        &mut self,
        blocked: ParserClassicScriptBlockedOnExecution<C>,
    ) {
        self.parser_blocking_scripts
            .install_current(blocked.into_script());
    }

    pub(crate) fn install_parser_blocking_script_blocked_on_source_load(
        &mut self,
        blocked: ParserClassicScriptBlockedOnSourceLoad<C>,
    ) {
        self.parser_blocking_scripts
            .install_current(blocked.into_script());
    }

    pub(crate) fn discard_current_parser_blocking_script_if_handle(
        &mut self,
        script_handle: DomHandle,
    ) -> bool {
        if self.current_parser_blocking_script_handle() != Some(script_handle) {
            return false;
        }
        self.parser_blocking_scripts.finish_current();
        true
    }

    pub(crate) fn discard_current_deferred_script_if_handle(
        &mut self,
        script_handle: DomHandle,
    ) -> bool {
        if self.current_deferred_script_handle() != Some(script_handle) {
            return false;
        }
        self.deferred_scripts.finish_current();
        true
    }

    #[cfg(test)]
    pub(crate) fn is_complete(&self) -> bool {
        self.parser_blocking_scripts.is_empty() && self.deferred_scripts.is_empty()
    }

    pub(crate) fn current_parser_blocking_script(
        &self,
    ) -> Option<&ParserClassicScriptRunnerItem<C>> {
        self.parser_blocking_scripts.current()
    }

    pub(crate) fn apply_current_parser_blocking_preloaded_script(
        &mut self,
        script_handle: DomHandle,
        prepared_script: PreparedScript,
    ) -> bool {
        self.update_current_parser_blocking_and_keep(|script| {
            if script
                .runner_metadata()
                .map(|metadata| metadata.script_handle())
                != Some(script_handle)
            {
                return Some(false);
            }
            let Some(current) = script.runner_script_mut() else {
                return Some(false);
            };
            *current = prepared_script;
            Some(script.promote_runner_external_pending_to_ready())
        })
        .unwrap_or(false)
    }

    pub(crate) fn current_parser_blocking_execution_gate_with_owner<Owner>(
        &mut self,
        owner: &mut Owner,
    ) -> ParserClassicScriptExecutionGateProjection<C>
    where
        C: Clone,
        Owner: ParserScriptOwner<C>,
    {
        self.update_current_parser_blocking_and_keep(|script| {
            if !owner.is_current_parser_script_owner(script.context()) {
                return Some(ParserClassicScriptExecutionGateProjection::NoCurrent);
            }
            Some(
                match owner.parser_script_execution_gate(
                    script.context().parser_classic_execution_gate_state(),
                ) {
                    ParserScriptExecutionGate::Ready => {
                        ParserClassicScriptExecutionGateProjection::Ready
                    }
                    ParserScriptExecutionGate::Blocked(blocker) => {
                        ParserClassicScriptExecutionGateProjection::Blocked(
                            ParserClassicScriptBlockedOnExecution::new(blocker, script.clone()),
                        )
                    }
                },
            )
        })
        .unwrap_or(ParserClassicScriptExecutionGateProjection::NoCurrent)
    }

    pub(crate) fn current_parser_blocking_source_load_client_action_with_owner<Owner>(
        &self,
        owner: &mut Owner,
    ) -> Option<Owner::SourceLoadClientAction>
    where
        Owner: ParserScriptSourceLoadClientOwner<C>,
    {
        let script = self.current_parser_blocking_script()?;
        if !owner.is_current_parser_script_owner(script.context()) {
            return None;
        }
        let script_url = script.runner_external_pending_script_url()?;
        let metadata = script.runner_metadata()?;
        let client = ParserPendingClassicScriptSourceLoadClientAction::new(metadata, script_url);
        owner.parser_script_source_load_client_action(client)
    }

    pub(crate) fn current_parser_blocking_source_load_completion_action_with_owner<Owner>(
        &self,
        owner: &mut Owner,
    ) -> Option<Owner::SourceLoadCompletionAction>
    where
        C: ParserClassicScriptSourceLoadCompletionState,
        Owner: ParserScriptSourceLoadCompletionOwner<C>,
    {
        let script = self.current_parser_blocking_script()?;
        if !owner.is_current_parser_script_owner(script.context()) {
            return None;
        }
        let (source_identity, _) = script.runner_external_load_identity()?;
        let record = ParserPendingClassicScriptSourceLoadCompletionRecord::new(source_identity);
        let completion = ParserPendingClassicScriptSourceLoadCompletionAction::new(
            script.context().parser_classic_source_load_owner(),
            record,
        );
        owner.parser_script_source_load_completion_action(completion)
    }

    pub(crate) fn deferred_source_load_completion_action_with_owner<Owner>(
        &self,
        owner: &mut Owner,
    ) -> Option<Owner::SourceLoadCompletionAction>
    where
        C: ParserClassicScriptSourceLoadCompletionState,
        Owner: ParserScriptSourceLoadCompletionOwner<C>,
    {
        self.deferred_scripts.find_map(|script| {
            if !owner.is_current_parser_script_owner(script.context()) {
                return None;
            }
            let (source_identity, _) = script.runner_external_load_identity()?;
            let record = ParserPendingClassicScriptSourceLoadCompletionRecord::new(source_identity);
            owner.parser_script_source_load_completion_action(
                ParserPendingClassicScriptSourceLoadCompletionAction::new(
                    script.context().parser_classic_source_load_owner(),
                    record,
                ),
            )
        })
    }

    pub(crate) fn current_parser_blocking_source_load_wait_action_with_owner<Owner>(
        &self,
        owner: &mut Owner,
    ) -> Option<Owner::SourceLoadWaitAction>
    where
        C: ParserClassicScriptSourceLoadWaitState,
        Owner: ParserScriptSourceLoadWaitOwner<C>,
    {
        let script = self.current_parser_blocking_script()?;
        if !owner.is_current_parser_script_owner(script.context()) {
            return None;
        }
        let action = ParserPendingClassicScriptSourceLoadWaitAction::new(
            script.context().parser_classic_source_load_wait(),
        );
        owner.parser_script_source_load_wait_action(action)
    }

    pub(crate) fn take_current_parser_blocking_next_action_with_owner<Owner, Action>(
        &mut self,
        owner: &mut Owner,
    ) -> Option<Action>
    where
        C: Clone,
        Owner: ParserScriptReadyOwner<C, ReadyAction = Action>
            + ParserScriptSourceFailureOwner<C, SourceFailureAction = Action>,
    {
        match self.take_current_parser_blocking_next_action_or_blocked_script_with_owner(owner) {
            ParserClassicScriptNextActionWithBlockedScript::Action(action) => Some(action),
            ParserClassicScriptNextActionWithBlockedScript::Blocked(_)
            | ParserClassicScriptNextActionWithBlockedScript::NotReady => None,
        }
    }

    pub(crate) fn take_current_deferred_next_action_with_owner<Owner, Action>(
        &mut self,
        owner: &mut Owner,
    ) -> Option<Action>
    where
        C: Clone,
        Owner: ParserScriptReadyOwner<C, ReadyAction = Action>
            + ParserScriptSourceFailureOwner<C, SourceFailureAction = Action>,
    {
        match self.take_current_deferred_ready_action_with_owner(owner) {
            ParserClassicScriptNextActionWithBlockedScript::Action(action) => Some(action),
            ParserClassicScriptNextActionWithBlockedScript::Blocked(_) => None,
            ParserClassicScriptNextActionWithBlockedScript::NotReady => {
                self.take_current_deferred_source_failure_action_with_owner(owner)
            }
        }
    }

    fn take_current_deferred_ready_action_with_owner<Owner>(
        &mut self,
        owner: &mut Owner,
    ) -> ParserClassicScriptNextActionWithBlockedScript<Owner::ReadyAction, C>
    where
        C: Clone,
        Owner: ParserScriptReadyOwner<C>,
    {
        self.deferred_scripts
            .update_current_and_keep(|script| {
                if !owner.is_current_parser_script_owner(script.context()) {
                    return Some(ParserClassicScriptNextActionWithBlockedScript::NotReady);
                }
                match owner.parser_script_execution_gate(
                    script.context().parser_classic_execution_gate_state(),
                ) {
                    ParserScriptExecutionGate::Ready => {}
                    ParserScriptExecutionGate::Blocked(blocker) => {
                        return Some(ParserClassicScriptNextActionWithBlockedScript::Blocked(
                            ParserClassicScriptBlockedOnExecution::new(blocker, script.clone()),
                        ));
                    }
                }
                let ParserClassicScriptRunnerStep::Ready(ready) = script.runner_step()? else {
                    return Some(ParserClassicScriptNextActionWithBlockedScript::NotReady);
                };
                let ready_action = ParserPendingClassicScriptReadyAction::new(&ready);
                Some(
                    owner
                        .parser_script_ready_action(script.context(), ready_action)
                        .map_or(
                            ParserClassicScriptNextActionWithBlockedScript::NotReady,
                            ParserClassicScriptNextActionWithBlockedScript::Action,
                        ),
                )
            })
            .unwrap_or(ParserClassicScriptNextActionWithBlockedScript::NotReady)
    }

    pub(crate) fn take_current_parser_blocking_next_action_or_blocked_script_with_owner<
        Owner,
        Action,
    >(
        &mut self,
        owner: &mut Owner,
    ) -> ParserClassicScriptNextActionWithBlockedScript<Action, C>
    where
        C: Clone,
        Owner: ParserScriptReadyOwner<C, ReadyAction = Action>
            + ParserScriptSourceFailureOwner<C, SourceFailureAction = Action>,
    {
        match self.take_current_parser_blocking_ready_action_or_blocked_script_with_owner(owner) {
            ParserClassicScriptNextActionWithBlockedScript::Action(action) => {
                ParserClassicScriptNextActionWithBlockedScript::Action(action)
            }
            ParserClassicScriptNextActionWithBlockedScript::Blocked(blocked) => {
                ParserClassicScriptNextActionWithBlockedScript::Blocked(blocked)
            }
            ParserClassicScriptNextActionWithBlockedScript::NotReady => self
                .take_current_parser_blocking_source_failure_action_with_owner(owner)
                .map_or(
                    ParserClassicScriptNextActionWithBlockedScript::NotReady,
                    ParserClassicScriptNextActionWithBlockedScript::Action,
                ),
        }
    }

    fn take_current_parser_blocking_ready_action_or_blocked_script_with_owner<Owner>(
        &mut self,
        owner: &mut Owner,
    ) -> ParserClassicScriptNextActionWithBlockedScript<Owner::ReadyAction, C>
    where
        C: Clone,
        Owner: ParserScriptReadyOwner<C>,
    {
        self.update_current_parser_blocking_and_keep(|script| {
            if !owner.is_current_parser_script_owner(script.context()) {
                return Some(ParserClassicScriptNextActionWithBlockedScript::NotReady);
            }
            match owner.parser_script_execution_gate(
                script.context().parser_classic_execution_gate_state(),
            ) {
                ParserScriptExecutionGate::Ready => {}
                ParserScriptExecutionGate::Blocked(blocker) => {
                    return Some(ParserClassicScriptNextActionWithBlockedScript::Blocked(
                        ParserClassicScriptBlockedOnExecution::new(blocker, script.clone()),
                    ));
                }
            }
            let ParserClassicScriptRunnerStep::Ready(ready) = script.runner_step()? else {
                return Some(ParserClassicScriptNextActionWithBlockedScript::NotReady);
            };
            let ready_action = ParserPendingClassicScriptReadyAction::new(&ready);
            Some(
                owner
                    .parser_script_ready_action(script.context(), ready_action)
                    .map_or(
                        ParserClassicScriptNextActionWithBlockedScript::NotReady,
                        ParserClassicScriptNextActionWithBlockedScript::Action,
                    ),
            )
        })
        .unwrap_or(ParserClassicScriptNextActionWithBlockedScript::NotReady)
    }

    pub(crate) fn take_current_parser_blocking_begin_execution_action_with_owner<Owner>(
        &mut self,
        script_handle: DomHandle,
        owner: &mut Owner,
    ) -> Option<Owner::BeginExecutionAction>
    where
        Owner: ParserScriptBeginExecutionOwner<C>,
    {
        self.update_current_parser_blocking_and_keep(|script| {
            if !owner.is_current_parser_script_owner(script.context()) {
                return None;
            }
            let ParserClassicScriptRunnerStep::Ready(_) = script.runner_step()? else {
                return None;
            };
            let executable_script = script
                .runner_script()
                .cloned()
                .and_then(ParserExecutableClassicScript::from_prepared_script)?;
            let execution = script.begin_runner_ready_execution(script_handle)?;
            let action =
                ParserPendingClassicScriptBeginExecutionAction::new(execution, executable_script);
            owner.parser_script_begin_execution_action(action)
        })
    }

    pub(crate) fn take_current_deferred_begin_execution_action_with_owner<Owner>(
        &mut self,
        script_handle: DomHandle,
        owner: &mut Owner,
    ) -> Option<Owner::BeginExecutionAction>
    where
        Owner: ParserScriptBeginExecutionOwner<C>,
    {
        self.deferred_scripts.update_current_and_keep(|script| {
            if !owner.is_current_parser_script_owner(script.context()) {
                return None;
            }
            let ParserClassicScriptRunnerStep::Ready(_) = script.runner_step()? else {
                return None;
            };
            let executable_script = script
                .runner_script()
                .cloned()
                .and_then(ParserExecutableClassicScript::from_prepared_script)?;
            let execution = script.begin_runner_ready_execution(script_handle)?;
            owner.parser_script_begin_execution_action(
                ParserPendingClassicScriptBeginExecutionAction::new(execution, executable_script),
            )
        })
    }

    pub(crate) fn take_current_parser_blocking_disposed_ready_action_with_owner<Owner>(
        &mut self,
        script_handle: DomHandle,
        owner: &mut Owner,
    ) -> Option<Owner::DisposedReadyAction>
    where
        Owner: ParserScriptDisposeReadyOwner<C>,
    {
        self.update_current_parser_blocking_and_advance(|script| {
            if !owner.is_current_parser_script_owner(script.context()) {
                return None;
            }
            let execution = script.dispose_runner_ready_without_execution(script_handle)?;
            let action = ParserPendingClassicScriptDisposedReadyAction::new(execution);
            owner.parser_script_disposed_ready_action(action)
        })
    }

    pub(crate) fn take_current_deferred_disposed_ready_action_with_owner<Owner>(
        &mut self,
        script_handle: DomHandle,
        owner: &mut Owner,
    ) -> Option<Owner::DisposedReadyAction>
    where
        Owner: ParserScriptDisposeReadyOwner<C>,
    {
        self.deferred_scripts.update_current_and_advance(|script| {
            if !owner.is_current_parser_script_owner(script.context()) {
                return None;
            }
            let execution = script.dispose_runner_ready_without_execution(script_handle)?;
            owner.parser_script_disposed_ready_action(
                ParserPendingClassicScriptDisposedReadyAction::new(execution),
            )
        })
    }

    pub(crate) fn take_current_parser_blocking_finished_execution_action_with_owner<Owner>(
        &mut self,
        script_handle: DomHandle,
        owner: &mut Owner,
    ) -> Option<Owner::FinishedExecutionAction>
    where
        Owner: ParserScriptFinishExecutionOwner<C>,
    {
        self.update_current_parser_blocking_and_advance(|script| {
            if !owner.is_current_parser_script_owner(script.context()) {
                return None;
            }
            let execution = script.finish_runner_execution(script_handle)?;
            let action = ParserPendingClassicScriptFinishedExecutionAction::new(execution);
            owner.parser_script_finished_execution_action(action)
        })
    }

    pub(crate) fn take_current_deferred_finished_execution_action_with_owner<Owner>(
        &mut self,
        script_handle: DomHandle,
        owner: &mut Owner,
    ) -> Option<Owner::FinishedExecutionAction>
    where
        Owner: ParserScriptFinishExecutionOwner<C>,
    {
        self.deferred_scripts.update_current_and_advance(|script| {
            if !owner.is_current_parser_script_owner(script.context()) {
                return None;
            }
            let execution = script.finish_runner_execution(script_handle)?;
            owner.parser_script_finished_execution_action(
                ParserPendingClassicScriptFinishedExecutionAction::new(execution),
            )
        })
    }

    pub(crate) fn take_current_parser_blocking_source_failure_action_with_owner<Owner>(
        &mut self,
        owner: &mut Owner,
    ) -> Option<Owner::SourceFailureAction>
    where
        Owner: ParserScriptSourceFailureOwner<C>,
    {
        self.update_current_parser_blocking_and_advance(|script| {
            if !owner.is_current_parser_script_owner(script.context()) {
                return None;
            }
            let failure = script.finish_runner_source_failure()?;
            let action = ParserPendingClassicScriptSourceFailureAction::new(failure);
            owner.parser_script_source_failure_action(script.context(), action)
        })
    }

    pub(crate) fn take_current_deferred_source_failure_action_with_owner<Owner>(
        &mut self,
        owner: &mut Owner,
    ) -> Option<Owner::SourceFailureAction>
    where
        Owner: ParserScriptSourceFailureOwner<C>,
    {
        self.deferred_scripts.update_current_and_advance(|script| {
            if !owner.is_current_parser_script_owner(script.context()) {
                return None;
            }
            let failure = script.finish_runner_source_failure()?;
            owner.parser_script_source_failure_action(
                script.context(),
                ParserPendingClassicScriptSourceFailureAction::new(failure),
            )
        })
    }

    pub(crate) fn take_current_parser_blocking_external_load_action_with_owner<Owner>(
        &mut self,
        load_id: u64,
        owner: &mut Owner,
    ) -> Option<Owner::SourceLoadAction>
    where
        C: ParserClassicScriptSourceLoadStartState,
        Owner: ParserScriptBeginSourceLoadOwner<C>,
    {
        self.update_current_parser_blocking_and_keep(|script| {
            if !owner.is_current_parser_script_owner(script.context()) {
                return None;
            }
            let script_url = script.runner_external_pending_script_url()?;
            let metadata = script.runner_metadata()?;
            let candidate =
                ParserPendingClassicScriptSourceLoadCandidate::new(metadata, script_url);
            if !owner.parser_script_source_load_candidate_matches(candidate) {
                return None;
            }
            let (source_identity, input) = script.begin_runner_external_load(load_id)?;
            let request = ParserPendingClassicScriptSourceLoadRequest::new(source_identity, input);
            let action = ParserPendingClassicScriptSourceLoadAction::new(request);
            let owner_action = owner.parser_script_source_load_action(action)?;
            if let Some(state) = owner.parser_script_source_load_state() {
                script
                    .context_mut()
                    .install_parser_classic_source_load_state(state);
            }
            Some(owner_action)
        })
    }

    pub(crate) fn fail_current_parser_blocking_external_pending_before_load(
        &mut self,
        metadata: crate::parser_script::payload::ParserClassicScriptMetadata,
        script_url: &url::Url,
        error: String,
    ) -> bool {
        self.update_current_parser_blocking_and_keep(|script| {
            if script.runner_metadata() != Some(metadata)
                || script.runner_external_pending_script_url() != Some(script_url)
            {
                return Some(false);
            }
            Some(script.fail_runner_external_pending_before_load(error))
        })
        .unwrap_or(false)
    }

    fn update_current_parser_blocking_and_keep<R>(
        &mut self,
        update: impl FnOnce(&mut ParserClassicScriptRunnerItem<C>) -> Option<R>,
    ) -> Option<R> {
        self.parser_blocking_scripts.update_current_and_keep(update)
    }

    pub(crate) fn take_current_parser_blocking_source_result_action_with_owner<Owner>(
        &mut self,
        source_result: ParserClassicScriptSourceResult,
        owner: &mut Owner,
    ) -> Option<Owner::SourceResultAction>
    where
        C: ParserClassicScriptSourceLoadState,
        Owner: ParserScriptSourceResultOwner<C>,
    {
        self.update_current_parser_blocking_and_keep(|script| {
            if !owner.is_current_parser_script_owner(script.context()) {
                return None;
            }
            let (notification, network_result) =
                script.notify_runner_source_result_with_network_result(source_result)?;
            script
                .context_mut()
                .clear_parser_classic_source_load_state();
            let network_record_urls = script
                .runner_script()
                .map(ParserClassicScriptNetworkRecordUrls::from_prepared_script);
            let action = ParserPendingClassicScriptSourceResultAction::new(
                notification,
                network_result.as_ref(),
                network_record_urls,
            );
            owner.parser_script_source_result_action(action)
        })
    }

    pub(crate) fn take_deferred_source_result_action_with_owner<Owner>(
        &mut self,
        source_result: ParserClassicScriptSourceResult,
        owner: &mut Owner,
    ) -> Option<Owner::SourceResultAction>
    where
        C: ParserClassicScriptSourceLoadState,
        Owner: ParserScriptSourceResultOwner<C>,
    {
        let source_result_to_match = source_result.clone();
        self.deferred_scripts.update_first_matching_and_keep(
            |script| script.runner_external_load_matches_source_result(&source_result_to_match),
            |script| {
                if !owner.is_current_parser_script_owner(script.context()) {
                    return None;
                }
                let (notification, network_result) =
                    script.notify_runner_source_result_with_network_result(source_result)?;
                script
                    .context_mut()
                    .clear_parser_classic_source_load_state();
                let network_record_urls = script
                    .runner_script()
                    .map(ParserClassicScriptNetworkRecordUrls::from_prepared_script);
                owner.parser_script_source_result_action(
                    ParserPendingClassicScriptSourceResultAction::new(
                        notification,
                        network_result.as_ref(),
                        network_record_urls,
                    ),
                )
            },
        )
    }

    pub(crate) fn apply_current_parser_blocking_source_result_if_ready_with_owner<Owner>(
        &mut self,
        owner: &mut Owner,
    ) -> ParserClassicScriptSourceResultApplication<Owner::SourceResultAction>
    where
        C: ParserClassicScriptSourceLoadState + ParserClassicScriptSourceResultState,
        Owner: ParserScriptSourceResultOwner<C>,
    {
        self.update_current_parser_blocking_and_keep(|script| {
            if !owner.is_current_parser_script_owner(script.context()) {
                return Some(ParserClassicScriptSourceResultApplication::NoSourceLoad);
            }
            let source_identity = script
                .runner_external_load_identity()
                .map(|(source_identity, _)| source_identity);
            let network_record_urls = script
                .runner_script()
                .map(ParserClassicScriptNetworkRecordUrls::from_prepared_script);
            let source_load_outcome =
                match script.context().parser_classic_source_load_outcome_state() {
                    ParserClassicScriptSourceLoadOutcomeState::NoSourceLoad => {
                        return Some(ParserClassicScriptSourceResultApplication::NoSourceLoad);
                    }
                    ParserClassicScriptSourceLoadOutcomeState::Waiting => {
                        return Some(ParserClassicScriptSourceResultApplication::Waiting);
                    }
                    ParserClassicScriptSourceLoadOutcomeState::Ready(outcome) => outcome,
                };
            let Some(source_identity) = source_identity else {
                return Some(ParserClassicScriptSourceResultApplication::Waiting);
            };
            let source_result = source_identity.into_source_result(source_load_outcome);
            let (notification, network_result) =
                script.notify_runner_source_result_with_network_result(source_result)?;
            script
                .context_mut()
                .clear_parser_classic_source_load_state();
            let action = ParserPendingClassicScriptSourceResultAction::new(
                notification,
                network_result.as_ref(),
                network_record_urls,
            );
            Some(ParserClassicScriptSourceResultApplication::Applied(
                owner.parser_script_source_result_action(action),
            ))
        })
        .unwrap_or(ParserClassicScriptSourceResultApplication::NoSourceLoad)
    }

    fn update_current_parser_blocking_and_advance<R>(
        &mut self,
        update: impl FnOnce(&mut ParserClassicScriptRunnerItem<C>) -> Option<R>,
    ) -> Option<R> {
        self.parser_blocking_scripts
            .update_current_and_advance(update)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ParserClassicScriptMetadata, ParserClassicScriptRunner, ParserClassicScriptRunnerItem,
        ParserClassicScriptRunnerSlot, ParserPendingClassicScriptEntry,
        ParserPreparedClassicScript,
    };
    use crate::{
        document_runtime::DomHandle,
        dom::NodeId,
        parser_script::action::{
            ParserPendingClassicScriptReady, ParserPendingClassicScriptReadyAction,
            ParserPendingClassicScriptSourceFailureAction,
        },
        parser_script::owner::{
            ParserScriptExecutionBlocker, ParserScriptExecutionGate, ParserScriptOwner,
            ParserScriptReadyOwner, ParserScriptSourceFailureOwner,
        },
        parser_script::projection::ParserClassicScriptNextActionWithBlockedScript,
        planning::{PreparedScript, ScriptFetchMetadata, ScriptSource},
        types::{ScriptKind, ScriptMode, ScriptSourceKind},
    };
    use url::Url;

    #[derive(Default)]
    struct CaptureLifecycleEventOwner {
        queued_events: usize,
    }

    impl ParserScriptOwner<()> for CaptureLifecycleEventOwner {}

    impl ParserScriptSourceFailureOwner<()> for CaptureLifecycleEventOwner {
        type SourceFailureAction = super::ParserClassicScriptSourceFailure;

        fn parser_script_source_failure_action(
            &mut self,
            _context: &(),
            action: ParserPendingClassicScriptSourceFailureAction,
        ) -> Option<Self::SourceFailureAction> {
            self.queued_events += 1;
            Some(action.into_failure())
        }
    }

    #[test]
    fn parser_script_owner_captures_script_element_event() {
        let script_handle = DomHandle::new(7);
        let script_url =
            Url::parse("https://parser-script-runner.test/fail.js").expect("script url");
        let mut runner = ParserClassicScriptRunner::new_parser_blocking(vec![
            ParserClassicScriptRunnerItem::from_slot_for_test(
                ParserClassicScriptRunnerSlot::from_pending_entry(
                    ParserPendingClassicScriptEntry::external_failed(
                        ParserClassicScriptMetadata::new(script_handle, 11),
                        script_url.clone(),
                        "network failure",
                    ),
                ),
                (),
            ),
        ]);
        let mut owner = CaptureLifecycleEventOwner::default();

        let failure = runner
            .take_current_parser_blocking_source_failure_action_with_owner(&mut owner)
            .expect("source failure should finish");

        assert_eq!(failure.metadata.script_handle(), script_handle);
        assert_eq!(failure.script_url, script_url);
        assert_eq!(owner.queued_events, 1);
        assert!(runner.is_complete());
    }

    struct BlockReadyProjectionOwner;

    impl ParserScriptOwner<()> for BlockReadyProjectionOwner {
        fn parser_script_execution_gate(&mut self, _state: ()) -> ParserScriptExecutionGate {
            ParserScriptExecutionGate::Blocked(ParserScriptExecutionBlocker::Stylesheet)
        }
    }

    impl ParserScriptReadyOwner<()> for BlockReadyProjectionOwner {
        type ReadyAction = ParserPendingClassicScriptReady;

        fn parser_script_ready_action(
            &mut self,
            _context: &(),
            ready: ParserPendingClassicScriptReadyAction<'_>,
        ) -> Option<Self::ReadyAction> {
            Some(ready.ready().clone())
        }
    }

    impl ParserScriptSourceFailureOwner<()> for BlockReadyProjectionOwner {
        type SourceFailureAction = ParserPendingClassicScriptReady;

        fn parser_script_source_failure_action(
            &mut self,
            _context: &(),
            _action: ParserPendingClassicScriptSourceFailureAction,
        ) -> Option<Self::SourceFailureAction> {
            None
        }
    }

    struct ReadyProjectionOwner;

    impl ParserScriptOwner<()> for ReadyProjectionOwner {}

    impl ParserScriptReadyOwner<()> for ReadyProjectionOwner {
        type ReadyAction = ParserPendingClassicScriptReady;

        fn parser_script_ready_action(
            &mut self,
            _context: &(),
            ready: ParserPendingClassicScriptReadyAction<'_>,
        ) -> Option<Self::ReadyAction> {
            Some(ready.ready().clone())
        }
    }

    impl ParserScriptSourceFailureOwner<()> for ReadyProjectionOwner {
        type SourceFailureAction = ParserPendingClassicScriptReady;

        fn parser_script_source_failure_action(
            &mut self,
            _context: &(),
            _action: ParserPendingClassicScriptSourceFailureAction,
        ) -> Option<Self::SourceFailureAction> {
            None
        }
    }

    fn loaded_prepared_script(
        script_handle: DomHandle,
        start_line: u64,
    ) -> ParserPreparedClassicScript {
        let script_url =
            Url::parse("https://parser-script-runner.test/ready.js").expect("script url");
        let script = PreparedScript {
            position: 0,
            node_id: NodeId::new(script_handle.index()),
            kind: ScriptKind::Classic,
            mode: ScriptMode::Normal,
            source_kind: ScriptSourceKind::External,
            fetch_metadata: ScriptFetchMetadata::default(),
            source: ScriptSource::Loaded("window.__ready = true".to_owned()),
            url: script_url.clone(),
            base_url: script_url.clone(),
            initiator_url: script_url,
            host_script_handle: None,
        };
        ParserPreparedClassicScript::new(
            ParserClassicScriptMetadata::new(script_handle, start_line),
            script,
        )
    }

    #[test]
    fn parser_script_owner_gate_blocks_ready_projection() {
        let script_handle = DomHandle::new(9);
        let mut runner = ParserClassicScriptRunner::new_parser_blocking(vec![
            ParserClassicScriptRunnerItem::from_slot_for_test(
                ParserClassicScriptRunnerSlot::from_pending_entry(
                    ParserPendingClassicScriptEntry::external_ready(loaded_prepared_script(
                        script_handle,
                        17,
                    )),
                ),
                (),
            ),
        ]);
        let mut blocked_owner = BlockReadyProjectionOwner;

        let blocked = runner.take_current_parser_blocking_next_action_or_blocked_script_with_owner(
            &mut blocked_owner,
        );
        match blocked {
            ParserClassicScriptNextActionWithBlockedScript::Blocked(blocked) => {
                assert_eq!(blocked.blocker(), ParserScriptExecutionBlocker::Stylesheet);
                assert_eq!(
                    blocked
                        .script()
                        .runner_metadata()
                        .map(|metadata| metadata.script_handle()),
                    Some(script_handle)
                );
            }
            projection => panic!("expected stylesheet-blocked projection, got {projection:?}"),
        }

        let mut current_owner = ReadyProjectionOwner;
        let ready = match runner
            .take_current_parser_blocking_next_action_or_blocked_script_with_owner(
                &mut current_owner,
            ) {
            ParserClassicScriptNextActionWithBlockedScript::Action(ready) => ready,
            projection => panic!("expected ready projection, got {projection:?}"),
        };
        assert_eq!(ready.script().script_handle(), script_handle);
        assert_eq!(ready.script().start_line(), 17);
    }

    #[test]
    fn parser_script_runner_discards_current_only_when_handle_still_matches() {
        let first_script_handle = DomHandle::new(11);
        let second_script_handle = DomHandle::new(12);
        let mut runner = ParserClassicScriptRunner::new_parser_blocking(vec![
            ParserClassicScriptRunnerItem::from_slot_for_test(
                ParserClassicScriptRunnerSlot::from_pending_entry(
                    ParserPendingClassicScriptEntry::external_ready(loaded_prepared_script(
                        first_script_handle,
                        17,
                    )),
                ),
                (),
            ),
            ParserClassicScriptRunnerItem::from_slot_for_test(
                ParserClassicScriptRunnerSlot::from_pending_entry(
                    ParserPendingClassicScriptEntry::external_ready(loaded_prepared_script(
                        second_script_handle,
                        23,
                    )),
                ),
                (),
            ),
        ]);

        assert_eq!(
            runner.current_parser_blocking_script_handle(),
            Some(first_script_handle)
        );
        assert!(
            runner.discard_current_parser_blocking_script_if_handle(first_script_handle),
            "first pending script should be discarded when it is still current"
        );
        assert_eq!(
            runner.current_parser_blocking_script_handle(),
            Some(second_script_handle)
        );
        assert!(
            !runner.discard_current_parser_blocking_script_if_handle(first_script_handle),
            "stale handle must not discard the next pending script"
        );
        assert_eq!(
            runner.current_parser_blocking_script_handle(),
            Some(second_script_handle)
        );
    }
}
