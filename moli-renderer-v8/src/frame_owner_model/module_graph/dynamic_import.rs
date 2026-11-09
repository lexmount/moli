use crate::{
    frame_owner_model::{FrameDocumentModuleFetchClientStart, FrameDocumentOwner, FrameRealmId},
    module_runtime::{
        DynamicModuleFetchContinuation, DynamicModuleFetchFailure, DynamicModuleFetchFinish,
        DynamicModuleFetchOwnerAdvance, DynamicModuleInflightFetch, DynamicModuleScheduledFetch,
        ModuleEntryId, ModuleGraphFetchedSource, ModuleLoadError, ModuleLoadStage,
        NativeDynamicModuleImportReady, NativeModuleGraphFetchRequest, NativeModuleGraphJob,
        NativeModuleGraphJobAdvance, WasmModuleRecord,
    },
    types::ScriptErrorConstructorKind,
};

use super::{
    ChildDocumentModulatorStore, ChildDynamicModuleCompletedFetchRestoreAction,
    ChildDynamicModuleFetchAction, ChildDynamicModuleInflightFetch, ChildDynamicModuleJoinedFetch,
    ChildDynamicModuleOwnerFetchCompletionSettlementAction,
    FrameDocumentDynamicImportMissingJoinedTerminalFetch, FrameDocumentDynamicImportOwnerAction,
    FrameDocumentDynamicImportOwnerActionQueueRequest,
    FrameDocumentDynamicImportTerminalClientFinishResult,
};
use moli_module_script_tree as module_tree;

pub(crate) enum FrameDocumentDynamicImportSourceWasmRecordLookup {
    Found(WasmModuleRecord),
    MissingDocumentModulator(ModuleLoadError),
    NotWasm(ModuleLoadError),
}

fn child_dynamic_import_owner_fetch_starts(
    _job: &NativeModuleGraphJob,
    requests: &[NativeModuleGraphFetchRequest],
) -> Vec<Option<FrameDocumentModuleFetchClientStart>> {
    vec![None; requests.len()]
}

fn child_dynamic_import_owner_fetch_starts_for_continuation(
    continuation: &DynamicModuleFetchContinuation,
) -> Vec<Option<FrameDocumentModuleFetchClientStart>> {
    let Some(requests) = continuation.pending_fetch_requests() else {
        return Vec::new();
    };
    child_dynamic_import_owner_fetch_starts(continuation.job(), requests)
}

pub(crate) enum FrameDocumentDynamicImportGraphAdvanceFollowup {
    QueueOwnerAction(Box<FrameDocumentDynamicImportOwnerActionQueueRequest>),
    ResumePendingJob(FrameDocumentDynamicImportPendingJobResume),
    RecordMissingJoinedTerminalFetch(FrameDocumentDynamicImportMissingJoinedTerminalFetch),
    RecordUnexpectedCompleteWarning(FrameDocumentDynamicImportUnexpectedCompleteWarning),
    WaitRetained,
}

pub(crate) struct FrameDocumentDynamicImportPendingJobResume {
    job: Box<NativeModuleGraphJob>,
}

pub(crate) struct FrameDocumentDynamicImportUnexpectedCompleteWarning {
    owner: FrameDocumentOwner,
    realm_id: FrameRealmId,
}

impl FrameDocumentDynamicImportPendingJobResume {
    pub(crate) fn new(job: NativeModuleGraphJob) -> Self {
        Self { job: Box::new(job) }
    }

    pub(crate) fn into_job(self) -> NativeModuleGraphJob {
        *self.job
    }

    #[cfg(test)]
    pub(crate) fn job(&self) -> &NativeModuleGraphJob {
        &self.job
    }
}

impl FrameDocumentDynamicImportUnexpectedCompleteWarning {
    pub(crate) fn new(owner: FrameDocumentOwner, realm_id: FrameRealmId) -> Self {
        Self { owner, realm_id }
    }

    pub(crate) fn into_parts(self) -> (FrameDocumentOwner, FrameRealmId) {
        (self.owner, self.realm_id)
    }

    #[cfg(test)]
    pub(crate) fn owner(&self) -> FrameDocumentOwner {
        self.owner
    }

    #[cfg(test)]
    pub(crate) fn realm_id(&self) -> FrameRealmId {
        self.realm_id
    }
}

impl ChildDocumentModulatorStore {
    fn reserve_dynamic_module_graph_fetches(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        requests: Vec<NativeModuleGraphFetchRequest>,
    ) -> Vec<(u64, NativeModuleGraphFetchRequest)> {
        let document_modulator_entry = self.document_modulator_entry_mut(owner, realm_id);
        document_modulator_entry
            .document_modulator
            .reserve_module_graph_fetches(requests)
    }

    pub(crate) fn suspend_dynamic_module_import_job_fetches(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        requests: Vec<NativeModuleGraphFetchRequest>,
        joined_clients: Vec<module_tree::SingleModuleClientToken>,
        job: NativeModuleGraphJob,
    ) -> Vec<DynamicModuleScheduledFetch> {
        let owner_module_fetch_starts = child_dynamic_import_owner_fetch_starts(&job, &requests);
        self.suspend_dynamic_module_import_fetches(
            owner,
            realm_id,
            requests,
            joined_clients,
            job,
            owner_module_fetch_starts,
        )
    }

    pub(crate) fn suspend_dynamic_module_import_fetches(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        requests: Vec<NativeModuleGraphFetchRequest>,
        joined_clients: Vec<module_tree::SingleModuleClientToken>,
        job: NativeModuleGraphJob,
        owner_module_fetch_starts: Vec<Option<FrameDocumentModuleFetchClientStart>>,
    ) -> Vec<DynamicModuleScheduledFetch> {
        let fetches = self.reserve_dynamic_module_graph_fetches(owner, realm_id, requests);
        let document_modulator_entry = self.document_modulator_entry_mut(owner, realm_id);
        document_modulator_entry
            .document_modulator
            .suspend_dynamic_module_import_fetches(
                fetches,
                joined_clients,
                job,
                owner_module_fetch_starts,
            )
    }

    pub(crate) fn take_inflight_dynamic_module_import_fetch(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        load_id: u64,
    ) -> Option<ChildDynamicModuleInflightFetch> {
        let document_modulator_entry =
            self.current_document_modulator_entry_mut(owner, realm_id)?;
        let inflight = document_modulator_entry
            .document_modulator
            .take_inflight_dynamic_module_import_fetch(load_id)?;
        Some(ChildDynamicModuleInflightFetch { inflight })
    }

    pub(crate) fn take_joined_dynamic_module_import_fetch(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        client: module_tree::SingleModuleClientToken,
    ) -> Option<ChildDynamicModuleJoinedFetch> {
        let document_modulator_entry =
            self.current_document_modulator_entry_mut(owner, realm_id)?;
        let joined = document_modulator_entry
            .document_modulator
            .take_joined_dynamic_module_import_fetch(client)?;
        Some(ChildDynamicModuleJoinedFetch {
            owner,
            realm_id: document_modulator_entry.realm_id,
            joined,
        })
    }

    pub(crate) fn restore_dynamic_module_import_fetch_as_joined_owner_client(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        inflight: DynamicModuleInflightFetch,
    ) -> Option<module_tree::SingleModuleClientToken> {
        self.current_document_modulator_entry_mut(owner, realm_id)?
            .document_modulator
            .restore_dynamic_module_import_fetch_as_joined_owner_client(inflight)
    }

    pub(crate) fn restore_inflight_dynamic_module_import_fetch_as_joined_owner_client(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        load_id: u64,
    ) -> Option<module_tree::SingleModuleClientToken> {
        let document_modulator_entry =
            self.current_document_modulator_entry_mut(owner, realm_id)?;
        let inflight = document_modulator_entry
            .document_modulator
            .take_inflight_dynamic_module_import_fetch(load_id)?;
        document_modulator_entry
            .document_modulator
            .restore_dynamic_module_import_fetch_as_joined_owner_client(inflight)
    }

    #[cfg(test)]
    pub(crate) fn dynamic_module_source_wasm_record(
        &self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        root_entry: ModuleEntryId,
        specifier: &str,
    ) -> Result<WasmModuleRecord, ModuleLoadError> {
        match self.dynamic_module_source_wasm_record_lookup(owner, realm_id, root_entry, specifier)
        {
            FrameDocumentDynamicImportSourceWasmRecordLookup::Found(wasm_record) => Ok(wasm_record),
            FrameDocumentDynamicImportSourceWasmRecordLookup::MissingDocumentModulator(error)
            | FrameDocumentDynamicImportSourceWasmRecordLookup::NotWasm(error) => Err(error),
        }
    }

    pub(crate) fn dynamic_module_source_wasm_record_lookup(
        &self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        root_entry: ModuleEntryId,
        specifier: &str,
    ) -> FrameDocumentDynamicImportSourceWasmRecordLookup {
        let Some(document_modulator_entry) = self.current_document_modulator_entry(owner, realm_id)
        else {
            return FrameDocumentDynamicImportSourceWasmRecordLookup::MissingDocumentModulator(
                ModuleLoadError::new(
                    ModuleLoadStage::Resolve,
                    "child dynamic import has no current document modulator",
                ),
            );
        };
        let Some(wasm_record) = document_modulator_entry
            .document_modulator
            .module_wasm_record(root_entry)
        else {
            return FrameDocumentDynamicImportSourceWasmRecordLookup::NotWasm(
                ModuleLoadError::new(
                    ModuleLoadStage::Resolve,
                    format!(
                        "source-phase dynamic import `{specifier}` is not a WebAssembly module"
                    ),
                )
                .with_error_constructor(ScriptErrorConstructorKind::SyntaxError),
            );
        };
        FrameDocumentDynamicImportSourceWasmRecordLookup::Found(wasm_record)
    }

    pub(crate) fn continue_dynamic_module_import_fetch(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        continuation: DynamicModuleFetchContinuation,
        owner_module_fetch_starts: Vec<Option<FrameDocumentModuleFetchClientStart>>,
    ) -> DynamicModuleFetchOwnerAdvance {
        let fetches = continuation
            .pending_fetch_requests()
            .map(|requests| {
                self.reserve_dynamic_module_graph_fetches(owner, realm_id, requests.to_vec())
            })
            .unwrap_or_default();
        let document_modulator_entry = self.document_modulator_entry_mut(owner, realm_id);
        document_modulator_entry
            .document_modulator
            .continue_dynamic_module_import_fetch(continuation, fetches, owner_module_fetch_starts)
    }

    pub(crate) fn dynamic_import_fetch_finish_to_terminal_client_finish_result(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        finish: DynamicModuleFetchFinish,
    ) -> FrameDocumentDynamicImportTerminalClientFinishResult {
        match finish {
            DynamicModuleFetchFinish::Advanced(continuation) => self
                .continue_dynamic_import_fetch_as_terminal_client_finish_result(
                    owner,
                    realm_id,
                    continuation,
                ),
            DynamicModuleFetchFinish::Failed(failure) => {
                let Some((request, error)) =
                    self.clear_failed_dynamic_module_import_fetch(owner, realm_id, failure)
                else {
                    return FrameDocumentDynamicImportTerminalClientFinishResult::MissingJoinedClient;
                };
                FrameDocumentDynamicImportTerminalClientFinishResult::followup_action(
                    FrameDocumentDynamicImportOwnerAction::fetch_failed(request, error),
                )
            }
        }
    }

    pub(crate) fn dynamic_import_fetch_finish_followup(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        load_id: u64,
        finish: DynamicModuleFetchFinish,
    ) -> FrameDocumentDynamicImportGraphAdvanceFollowup {
        match self
            .dynamic_import_fetch_finish_to_terminal_client_finish_result(owner, realm_id, finish)
        {
            FrameDocumentDynamicImportTerminalClientFinishResult::FollowupActions(actions) => {
                FrameDocumentDynamicImportGraphAdvanceFollowup::QueueOwnerAction(Box::new(
                    FrameDocumentDynamicImportOwnerActionQueueRequest::fetch_completion_actions(
                        owner, realm_id, load_id, actions,
                    ),
                ))
            }
            FrameDocumentDynamicImportTerminalClientFinishResult::RestoredAfterUnexpectedComplete => {
                FrameDocumentDynamicImportGraphAdvanceFollowup::RecordUnexpectedCompleteWarning(
                    FrameDocumentDynamicImportUnexpectedCompleteWarning::new(owner, realm_id),
                )
            }
            FrameDocumentDynamicImportTerminalClientFinishResult::WaitRetained => {
                FrameDocumentDynamicImportGraphAdvanceFollowup::WaitRetained
            }
            FrameDocumentDynamicImportTerminalClientFinishResult::MissingJoinedClient => {
                FrameDocumentDynamicImportGraphAdvanceFollowup::RecordMissingJoinedTerminalFetch(
                    FrameDocumentDynamicImportMissingJoinedTerminalFetch::new(
                        owner, realm_id, load_id,
                    ),
                )
            }
        }
    }

    pub(crate) fn dynamic_import_graph_advance_followup(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        mut job: NativeModuleGraphJob,
        advance: NativeModuleGraphJobAdvance,
    ) -> FrameDocumentDynamicImportGraphAdvanceFollowup {
        match advance {
            NativeModuleGraphJobAdvance::NeedFetches(requests) => {
                let joined_clients = job.take_pending_joined_clients();
                let scheduled = self.suspend_dynamic_module_import_job_fetches(
                    owner,
                    realm_id,
                    requests,
                    joined_clients,
                    job,
                );
                FrameDocumentDynamicImportGraphAdvanceFollowup::QueueOwnerAction(Box::new(
                    FrameDocumentDynamicImportOwnerActionQueueRequest::waiting(
                        owner,
                        realm_id,
                        scheduled
                            .into_iter()
                            .map(ChildDynamicModuleFetchAction::from)
                            .collect(),
                    ),
                ))
            }
            NativeModuleGraphJobAdvance::WaitingForFetches => {
                let joined_clients = job.take_pending_joined_clients();
                if joined_clients.is_empty() {
                    FrameDocumentDynamicImportGraphAdvanceFollowup::ResumePendingJob(
                        FrameDocumentDynamicImportPendingJobResume::new(job),
                    )
                } else {
                    let _scheduled = self.suspend_dynamic_module_import_job_fetches(
                        owner,
                        realm_id,
                        Vec::new(),
                        joined_clients,
                        job,
                    );
                    FrameDocumentDynamicImportGraphAdvanceFollowup::WaitRetained
                }
            }
            NativeModuleGraphJobAdvance::Complete(graph) => self.dynamic_import_ready_followup(
                owner,
                realm_id,
                NativeDynamicModuleImportReady { job, graph },
            ),
        }
    }

    pub(crate) fn dynamic_import_ready_followup(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        dynamic_import: NativeDynamicModuleImportReady,
    ) -> FrameDocumentDynamicImportGraphAdvanceFollowup {
        FrameDocumentDynamicImportGraphAdvanceFollowup::QueueOwnerAction(Box::new(
            FrameDocumentDynamicImportOwnerActionQueueRequest::continuation(
                owner,
                realm_id,
                FrameDocumentDynamicImportOwnerAction::ready(dynamic_import),
            ),
        ))
    }

    pub(crate) fn dynamic_import_owner_module_fetch_completion_followup(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        load_id: u64,
        owner_start: FrameDocumentModuleFetchClientStart,
        source: std::result::Result<ModuleGraphFetchedSource, ModuleLoadError>,
        inflight: DynamicModuleInflightFetch,
    ) -> FrameDocumentDynamicImportGraphAdvanceFollowup {
        FrameDocumentDynamicImportGraphAdvanceFollowup::QueueOwnerAction(Box::new(
            FrameDocumentDynamicImportOwnerActionQueueRequest::fetch_completion(
                owner,
                realm_id,
                load_id,
                FrameDocumentDynamicImportOwnerAction::OwnerModuleFetchCompleted {
                    load_id,
                    settle: ChildDynamicModuleOwnerFetchCompletionSettlementAction::new(
                        owner_start,
                        source,
                    ),
                    restore: ChildDynamicModuleCompletedFetchRestoreAction::new(
                        owner, realm_id, inflight,
                    ),
                },
            ),
        ))
    }

    pub(crate) fn dynamic_import_graph_advance_failure_followup(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        job: NativeModuleGraphJob,
        error: ModuleLoadError,
    ) -> FrameDocumentDynamicImportGraphAdvanceFollowup {
        FrameDocumentDynamicImportGraphAdvanceFollowup::QueueOwnerAction(Box::new(
            FrameDocumentDynamicImportOwnerActionQueueRequest::continuation(
                owner,
                realm_id,
                FrameDocumentDynamicImportOwnerAction::graph_advance_failed(job, error),
            ),
        ))
    }

    fn continue_dynamic_import_fetch_as_terminal_client_finish_result(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        continuation: DynamicModuleFetchContinuation,
    ) -> FrameDocumentDynamicImportTerminalClientFinishResult {
        let owner_module_fetch_starts =
            child_dynamic_import_owner_fetch_starts_for_continuation(&continuation);
        match self.continue_dynamic_module_import_fetch(
            owner,
            realm_id,
            continuation,
            owner_module_fetch_starts,
        ) {
            DynamicModuleFetchOwnerAdvance::Waiting { scheduled_fetches } => {
                FrameDocumentDynamicImportTerminalClientFinishResult::followup_waiting_fetches(
                    owner,
                    realm_id,
                    ChildDynamicModuleFetchAction::wrap_all(scheduled_fetches),
                )
            }
            DynamicModuleFetchOwnerAdvance::Ready(dynamic_import) => {
                FrameDocumentDynamicImportTerminalClientFinishResult::followup_action(
                    FrameDocumentDynamicImportOwnerAction::ready(*dynamic_import),
                )
            }
            DynamicModuleFetchOwnerAdvance::RestoredAfterUnexpectedComplete => {
                FrameDocumentDynamicImportTerminalClientFinishResult::RestoredAfterUnexpectedComplete
            }
        }
    }

    pub(crate) fn clear_failed_dynamic_module_import_fetch(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        failure: DynamicModuleFetchFailure,
    ) -> Option<(
        crate::module_runtime::PendingDynamicModuleImport,
        ModuleLoadError,
    )> {
        let document_modulator_entry =
            self.current_document_modulator_entry_mut(owner, realm_id)?;
        let (joined_clients, request, error) = document_modulator_entry
            .document_modulator
            .clear_failed_dynamic_module_import_fetch(failure);
        for client in joined_clients {
            document_modulator_entry
                .document_modulator
                .detach_single_module_fetch_client(client);
        }
        Some((request, error))
    }

    pub(crate) fn has_pending_dynamic_module_import(&self) -> bool {
        self.documents.values().any(|document_modulator_entry| {
            document_modulator_entry
                .document_modulator
                .has_pending_dynamic_module_import()
        })
    }

    #[cfg(test)]
    pub(crate) fn has_inflight_dynamic_module_import_fetch(&self) -> bool {
        self.documents.values().any(|document_modulator_entry| {
            document_modulator_entry
                .document_modulator
                .has_inflight_dynamic_module_import_fetch()
        })
    }
}
