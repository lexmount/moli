use crate::{
    document_module_graph::{ModuleMapFetchDisposition, ModuleMapKey},
    document_runtime::DomHandle,
    frame_owner_model::{
        ChildDocumentModuleFetchTarget, FrameDocumentModuleTerminalBatch,
        FrameDocumentModuleTerminalQueueFollowup, FrameDocumentModulepreloadFetchTask,
        FrameDocumentOwner, FrameDocumentTaskOwner, FrameRealmId,
    },
    module_runtime::{
        ModuleGraphFetchedSource, ModuleLoadError, NativeModuleGraphFetchRequest,
        NativeModuleSingleFetchRequest, NativeModulepreloadLinkClient,
    },
};

use super::ChildDocumentModulatorStore;

pub(crate) enum FrameDocumentModulepreloadStartAction {
    ScheduleFetch {
        target: ChildDocumentModuleFetchTarget,
        link_handle: Option<DomHandle>,
        key: ModuleMapKey,
        load_id: u64,
        request: Box<NativeModuleGraphFetchRequest>,
    },
    JoinedFetching {
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        link_handle: Option<DomHandle>,
        key: ModuleMapKey,
    },
    JoinedTerminalSuccess {
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        link_handle: Option<DomHandle>,
        key: ModuleMapKey,
    },
    JoinedTerminalFailure {
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        link_handle: Option<DomHandle>,
        key: ModuleMapKey,
    },
}

pub(crate) trait FrameDocumentModulepreloadStartActionHooks {
    fn post_current_document_modulator_terminals(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
    ) -> FrameDocumentModuleTerminalQueueFollowup;

    fn schedule_modulepreload_fetch(
        &mut self,
        target: ChildDocumentModuleFetchTarget,
        link_handle: Option<DomHandle>,
        key: ModuleMapKey,
        load_id: u64,
        request: Box<NativeModuleGraphFetchRequest>,
    );

    fn record_joined_fetching(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        link_handle: Option<DomHandle>,
        key: &ModuleMapKey,
    );

    fn record_joined_terminal_success(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        link_handle: Option<DomHandle>,
        key: &ModuleMapKey,
    );

    fn record_joined_terminal_failure(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        link_handle: Option<DomHandle>,
        key: &ModuleMapKey,
    );
}

pub(crate) struct FrameDocumentModulepreloadStartActionRunner<Hooks> {
    hooks: Hooks,
}

pub(crate) struct FrameDocumentModulepreloadFetchCompletionAction {
    owner: FrameDocumentTaskOwner,
    realm_id: FrameRealmId,
    load_id: u64,
    request: NativeModuleSingleFetchRequest,
    source: Result<ModuleGraphFetchedSource, ModuleLoadError>,
}

pub(crate) enum FrameDocumentModulepreloadFetchFinishResult {
    Finished(FrameDocumentModuleTerminalBatch),
    MissingDocumentModulator,
}

pub(crate) trait FrameDocumentModulepreloadFetchCompletionHooks {
    fn finish_modulepreload_fetch(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        request: NativeModuleSingleFetchRequest,
        source: Result<ModuleGraphFetchedSource, ModuleLoadError>,
    ) -> FrameDocumentModulepreloadFetchFinishResult;

    fn queue_module_terminal_batch(
        &mut self,
        batch: FrameDocumentModuleTerminalBatch,
    ) -> FrameDocumentModuleTerminalQueueFollowup;

    fn record_missing_modulepreload_modulator(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        load_id: u64,
    );

    fn record_modulepreload_completion_finished(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        load_id: u64,
        key: &ModuleMapKey,
    );
}

pub(crate) struct FrameDocumentModulepreloadFetchCompletionRunner<Hooks> {
    hooks: Hooks,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameDocumentModulepreloadStartOutcome {
    start_action_consumed: bool,
    terminal_followup_queued: bool,
    fetch_scheduled: bool,
    joined_fetching: bool,
    joined_terminal_success: bool,
    joined_terminal_failure: bool,
}

impl FrameDocumentModulepreloadStartOutcome {
    pub(crate) fn start_action_consumed() -> Self {
        Self {
            start_action_consumed: true,
            ..Self::default()
        }
    }

    pub(crate) fn terminal_followup_queued() -> Self {
        Self {
            terminal_followup_queued: true,
            ..Self::default()
        }
    }

    pub(crate) fn fetch_scheduled() -> Self {
        Self {
            fetch_scheduled: true,
            ..Self::default()
        }
    }

    pub(crate) fn joined_fetching() -> Self {
        Self {
            joined_fetching: true,
            ..Self::default()
        }
    }

    pub(crate) fn joined_terminal_success() -> Self {
        Self {
            joined_terminal_success: true,
            ..Self::default()
        }
    }

    pub(crate) fn joined_terminal_failure() -> Self {
        Self {
            joined_terminal_failure: true,
            ..Self::default()
        }
    }

    pub(crate) fn merge(&mut self, other: Self) {
        self.start_action_consumed |= other.start_action_consumed;
        self.terminal_followup_queued |= other.terminal_followup_queued;
        self.fetch_scheduled |= other.fetch_scheduled;
        self.joined_fetching |= other.joined_fetching;
        self.joined_terminal_success |= other.joined_terminal_success;
        self.joined_terminal_failure |= other.joined_terminal_failure;
    }

    #[cfg(test)]
    pub(crate) fn terminal_followup_was_queued(self) -> bool {
        self.terminal_followup_queued
    }

    #[cfg(test)]
    pub(crate) fn fetch_was_scheduled(self) -> bool {
        self.fetch_scheduled
    }

    #[cfg(test)]
    pub(crate) fn joined_terminal_success_was_recorded(self) -> bool {
        self.joined_terminal_success
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameDocumentModulepreloadFetchCompletionOutcome {
    completion_consumed: bool,
    missing_document_modulator: bool,
    fetch_finished: bool,
    terminal_followup: FrameDocumentModuleTerminalQueueFollowup,
}

impl FrameDocumentModulepreloadFetchCompletionOutcome {
    pub(crate) fn completion_consumed() -> Self {
        Self {
            completion_consumed: true,
            ..Self::default()
        }
    }

    pub(crate) fn missing_document_modulator() -> Self {
        Self {
            missing_document_modulator: true,
            ..Self::default()
        }
    }

    pub(crate) fn fetch_finished() -> Self {
        Self {
            fetch_finished: true,
            ..Self::default()
        }
    }

    pub(crate) fn with_terminal_followup(
        followup: FrameDocumentModuleTerminalQueueFollowup,
    ) -> Self {
        Self {
            terminal_followup: followup,
            ..Self::default()
        }
    }

    pub(crate) fn merge(&mut self, other: Self) {
        self.completion_consumed |= other.completion_consumed;
        self.missing_document_modulator |= other.missing_document_modulator;
        self.fetch_finished |= other.fetch_finished;
        self.terminal_followup.merge(other.terminal_followup);
    }

    pub(crate) fn into_terminal_followup(self) -> FrameDocumentModuleTerminalQueueFollowup {
        self.terminal_followup
    }

    #[cfg(test)]
    pub(crate) fn fetch_was_finished(self) -> bool {
        self.fetch_finished
    }

    #[cfg(test)]
    pub(crate) fn terminal_followup_was_queued(self) -> bool {
        self.terminal_followup.made_progress()
    }

    #[cfg(test)]
    pub(crate) fn missing_document_modulator_was_recorded(self) -> bool {
        self.missing_document_modulator
    }
}

impl<Hooks> FrameDocumentModulepreloadStartActionRunner<Hooks>
where
    Hooks: FrameDocumentModulepreloadStartActionHooks,
{
    pub(crate) fn new(hooks: Hooks) -> Self {
        Self { hooks }
    }

    #[cfg(test)]
    pub(crate) fn into_hooks(self) -> Hooks {
        self.hooks
    }

    pub(crate) fn run_start_action(
        &mut self,
        action: FrameDocumentModulepreloadStartAction,
    ) -> FrameDocumentModulepreloadStartOutcome {
        let owner = action.owner();
        let realm_id = action.realm_id();
        let terminal_followup = self
            .hooks
            .post_current_document_modulator_terminals(owner, realm_id);
        let mut outcome = FrameDocumentModulepreloadStartOutcome::start_action_consumed();
        if terminal_followup.made_progress() {
            outcome.merge(FrameDocumentModulepreloadStartOutcome::terminal_followup_queued());
        }
        match action {
            FrameDocumentModulepreloadStartAction::ScheduleFetch {
                target,
                link_handle,
                key,
                load_id,
                request,
            } => {
                self.hooks
                    .schedule_modulepreload_fetch(target, link_handle, key, load_id, request);
                outcome.merge(FrameDocumentModulepreloadStartOutcome::fetch_scheduled());
            }
            FrameDocumentModulepreloadStartAction::JoinedFetching {
                owner,
                realm_id,
                link_handle,
                key,
            } => {
                self.hooks
                    .record_joined_fetching(owner, realm_id, link_handle, &key);
                outcome.merge(FrameDocumentModulepreloadStartOutcome::joined_fetching());
            }
            FrameDocumentModulepreloadStartAction::JoinedTerminalSuccess {
                owner,
                realm_id,
                link_handle,
                key,
            } => {
                self.hooks
                    .record_joined_terminal_success(owner, realm_id, link_handle, &key);
                outcome.merge(FrameDocumentModulepreloadStartOutcome::joined_terminal_success());
            }
            FrameDocumentModulepreloadStartAction::JoinedTerminalFailure {
                owner,
                realm_id,
                link_handle,
                key,
            } => {
                self.hooks
                    .record_joined_terminal_failure(owner, realm_id, link_handle, &key);
                outcome.merge(FrameDocumentModulepreloadStartOutcome::joined_terminal_failure());
            }
        }
        outcome
    }
}

impl FrameDocumentModulepreloadFetchCompletionAction {
    pub(crate) fn new(
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        load_id: u64,
        request: NativeModuleSingleFetchRequest,
        source: Result<ModuleGraphFetchedSource, ModuleLoadError>,
    ) -> Self {
        Self {
            owner,
            realm_id,
            load_id,
            request,
            source,
        }
    }
}

impl<Hooks> FrameDocumentModulepreloadFetchCompletionRunner<Hooks>
where
    Hooks: FrameDocumentModulepreloadFetchCompletionHooks,
{
    pub(crate) fn new(hooks: Hooks) -> Self {
        Self { hooks }
    }

    #[cfg(test)]
    pub(crate) fn into_hooks(self) -> Hooks {
        self.hooks
    }

    pub(crate) fn run_completion_action(
        &mut self,
        action: FrameDocumentModulepreloadFetchCompletionAction,
    ) -> FrameDocumentModulepreloadFetchCompletionOutcome {
        let FrameDocumentModulepreloadFetchCompletionAction {
            owner,
            realm_id,
            load_id,
            request,
            source,
        } = action;
        let mut outcome = FrameDocumentModulepreloadFetchCompletionOutcome::completion_consumed();
        let fetch_key = request.module_key().clone();
        match self
            .hooks
            .finish_modulepreload_fetch(owner, realm_id, request, source)
        {
            FrameDocumentModulepreloadFetchFinishResult::Finished(batch) => {
                outcome.merge(FrameDocumentModulepreloadFetchCompletionOutcome::fetch_finished());
                let followup = self.hooks.queue_module_terminal_batch(batch);
                outcome.merge(
                    FrameDocumentModulepreloadFetchCompletionOutcome::with_terminal_followup(
                        followup,
                    ),
                );
                self.hooks
                    .record_modulepreload_completion_finished(owner, realm_id, load_id, &fetch_key);
            }
            FrameDocumentModulepreloadFetchFinishResult::MissingDocumentModulator => {
                self.hooks.record_missing_modulepreload_modulator(
                    owner.document_owner(),
                    realm_id,
                    load_id,
                );
                outcome.merge(
                    FrameDocumentModulepreloadFetchCompletionOutcome::missing_document_modulator(),
                );
            }
        }
        outcome
    }
}

impl ChildDocumentModulatorStore {
    pub(crate) fn start_modulepreload_fetch_task(
        &mut self,
        task: FrameDocumentModulepreloadFetchTask,
    ) -> FrameDocumentModulepreloadStartAction {
        let task_owner = task.owner();
        let owner = task_owner.document_owner();
        let realm_id = task.realm_id();
        let target = task.target();
        let link_handle = task.link_handle();
        let key = task.request().module_key().clone();
        let link_client = task.client().map(|client| {
            NativeModulepreloadLinkClient::new_for_frame_document(
                client.link_handle(),
                key.clone(),
                client,
            )
        });
        let document_modulator_entry = self.document_modulator_entry_mut(owner, realm_id);
        let disposition = document_modulator_entry
            .document_modulator
            .start_or_join_fetch(key.clone());
        match disposition {
            ModuleMapFetchDisposition::StartedFetch(_entry_id) => {
                let (load_id, fetch_request) = document_modulator_entry
                    .document_modulator
                    .reserve_modulepreload_fetch(task.into_request());
                if let Some(link_client) = link_client {
                    document_modulator_entry
                        .document_modulator
                        .add_modulepreload_link_client(key.clone(), link_client);
                }
                FrameDocumentModulepreloadStartAction::ScheduleFetch {
                    target,
                    link_handle,
                    key,
                    load_id,
                    request: Box::new(fetch_request),
                }
            }
            ModuleMapFetchDisposition::JoinedFetching(_entry_id) => {
                if let Some(link_client) = link_client {
                    document_modulator_entry
                        .document_modulator
                        .add_modulepreload_link_client(key.clone(), link_client);
                }
                FrameDocumentModulepreloadStartAction::JoinedFetching {
                    owner,
                    realm_id,
                    link_handle,
                    key,
                }
            }
            ModuleMapFetchDisposition::AlreadyFetched(_)
            | ModuleMapFetchDisposition::AlreadyCompiled(_) => {
                if let Some(link_client) = link_client {
                    document_modulator_entry
                        .document_modulator
                        .add_terminal_modulepreload_link_client(key.clone(), link_client);
                }
                FrameDocumentModulepreloadStartAction::JoinedTerminalSuccess {
                    owner,
                    realm_id,
                    link_handle,
                    key,
                }
            }
            ModuleMapFetchDisposition::AlreadyFailed(_) => {
                if let Some(link_client) = link_client {
                    document_modulator_entry
                        .document_modulator
                        .add_terminal_modulepreload_link_client(key.clone(), link_client);
                }
                FrameDocumentModulepreloadStartAction::JoinedTerminalFailure {
                    owner,
                    realm_id,
                    link_handle,
                    key,
                }
            }
        }
    }

    pub(crate) fn take_modulepreload_graph_fetch(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        load_id: u64,
    ) -> Option<NativeModuleSingleFetchRequest> {
        let document_modulator_entry =
            self.current_document_modulator_entry_mut(owner, realm_id)?;
        document_modulator_entry
            .document_modulator
            .take_inflight_modulepreload_fetch(load_id)
    }

    pub(crate) fn finish_modulepreload_fetch(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        request: NativeModuleSingleFetchRequest,
        source: Result<ModuleGraphFetchedSource, ModuleLoadError>,
    ) -> Option<FrameDocumentModuleTerminalBatch> {
        let document_modulator_entry =
            self.current_document_modulator_entry_mut(owner.document_owner(), realm_id)?;
        let fetch_key = request.module_key().clone();
        match source {
            Ok(fetched_source) => {
                let effective_key = request.effective_key_for_fetched_source(&fetched_source);
                let effective_fetch_metadata =
                    request.effective_fetch_metadata_for_fetched_source(&fetched_source);
                document_modulator_entry
                    .document_modulator
                    .insert_fetched_source_for_request(
                        fetch_key,
                        effective_key,
                        fetched_source.into_source(),
                        effective_fetch_metadata,
                    );
            }
            Err(error) => {
                document_modulator_entry
                    .document_modulator
                    .mark_failed(fetch_key, error);
            }
        }
        Some(document_modulator_entry.take_ready_document_modulator_terminal_batches(owner))
    }
}

impl FrameDocumentModulepreloadStartAction {
    pub(crate) fn owner(&self) -> FrameDocumentOwner {
        match self {
            Self::ScheduleFetch { target, .. } => target.task_owner().document_owner(),
            Self::JoinedFetching { owner, .. }
            | Self::JoinedTerminalSuccess { owner, .. }
            | Self::JoinedTerminalFailure { owner, .. } => *owner,
        }
    }

    pub(crate) fn realm_id(&self) -> FrameRealmId {
        match self {
            Self::ScheduleFetch { target, .. } => target.realm_id(),
            Self::JoinedFetching { realm_id, .. }
            | Self::JoinedTerminalSuccess { realm_id, .. }
            | Self::JoinedTerminalFailure { realm_id, .. } => *realm_id,
        }
    }
}
