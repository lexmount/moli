use crate::{
    frame_owner_model::{
        FrameDocumentModuleDependencyTerminalWork, FrameDocumentModuleFetchTerminalResult,
        FrameDocumentModuleScriptGraphNotification, FrameDocumentModuleScriptTerminalFollowup,
        FrameDocumentModuleScriptTerminalWork, FrameDocumentOwner,
        FrameDocumentParserModuleTreeAdvanceDependencyFetchResult,
        FrameDocumentParserModuleTreeAdvanceFailureTrace,
        FrameDocumentParserModuleTreeAdvanceHooks, FrameDocumentParserModuleTreeAdvanceRunner,
        FrameDocumentParserRootModuleClient, FrameDocumentParserRootTerminalResult,
        FrameDocumentParserRootTerminalWork, FrameDocumentStaticDependencyModuleClient,
        FrameDocumentTaskOwner, FrameRealmId, frame_document_parser_module_tree_advance_action,
        module_script_graph_failed_work_from_root_client,
        module_script_graph_failed_work_from_tree_job, trace_child_module_dependency_failure,
        trace_child_parser_module_root_failure,
    },
    module_runtime::{
        ModuleEntryId, ModuleFetchMetadata, ModuleGraphFetchedSource, ModuleLoadError,
        ModuleLoadStage, ModuleMapEntryState, ModuleMapKey, ModuleRequestRecord, ModuleSource,
        NativeModuleGraphFetchRequest, NativeModuleGraphJobAdvance,
        NativeModuleTreeDocumentOwnerAdapter, NativeParserModuleTreeJobResume,
    },
};

use super::ScriptVm;
use moli_module_script_tree::ModuleTreeId;
use url::Url;

pub(super) struct ChildModuleScriptTerminalOwner<'vm> {
    vm: &'vm mut ScriptVm,
}

struct ScriptVmParserModuleTreeAdvanceHooks<'vm> {
    vm: &'vm mut ScriptVm,
}

struct ChildModuleRootRecord {
    entry_id: ModuleEntryId,
    key: ModuleMapKey,
    base_url: Url,
    requests: Vec<ModuleRequestRecord>,
    fetch_metadata: ModuleFetchMetadata,
}

// Terminal tasks may have been queued before another consumer compiled the
// module. Recheck the map at delivery, retaining its record and fetch options.
fn cached_module_root_record(
    owner: &impl NativeModuleTreeDocumentOwnerAdapter,
    key: &ModuleMapKey,
) -> Result<Option<ChildModuleRootRecord>, ModuleLoadError> {
    let Some(entry_id) = owner.module_entry_id(key) else {
        return Ok(None);
    };
    match owner.module_entry_state(entry_id) {
        ModuleMapEntryState::Compiled
        | ModuleMapEntryState::Instantiated
        | ModuleMapEntryState::Evaluating
        | ModuleMapEntryState::Evaluated => Ok(Some(ChildModuleRootRecord {
            entry_id,
            key: owner.module_entry_key(entry_id),
            base_url: owner.module_entry_url(entry_id),
            requests: owner.module_requests(entry_id),
            fetch_metadata: owner.module_effective_fetch_metadata(entry_id),
        })),
        ModuleMapEntryState::Failed => Err(owner.module_failure(entry_id).unwrap_or_else(|| {
            ModuleLoadError::new(ModuleLoadStage::Fetch, "module previously failed to load")
        })),
        ModuleMapEntryState::Fetching | ModuleMapEntryState::Fetched => Ok(None),
    }
}

impl<'vm> ChildModuleScriptTerminalOwner<'vm> {
    pub(super) fn new(vm: &'vm mut ScriptVm) -> Self {
        Self { vm }
    }

    pub(super) fn handle_parser_root_terminal_work(
        &mut self,
        work: FrameDocumentParserRootTerminalWork,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        let (task_owner, realm_id, key, client, result) = work.into_terminal_parts();
        self.handle_parser_root_terminal_result(task_owner, realm_id, key, client, result)
    }

    pub(super) fn handle_loaded_parser_root_start(
        &mut self,
        task_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        client: FrameDocumentParserRootModuleClient,
        source: ModuleSource,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        self.vm
            .ensure_child_document_modulator_for_graph_start(task_owner.document_owner(), realm_id);
        let source = if !client.source_is_external()
            && let Some(text) = source.text_source()
        {
            self.vm
                .inline_module_script_source_with_origin(client.script(), text.to_owned())
        } else {
            source
        };
        let source_url = if client.source_is_external() {
            client.script().url.clone()
        } else {
            crate::module_runtime::next_inline_module_url(self.vm, client.base_url())
        };
        let request_key = if client.source_is_external() {
            ModuleMapKey::from_script_url(source_url.clone())
        } else {
            ModuleMapKey::java_script(source_url.clone())
        };
        self.handle_parser_root_terminal_result(
            task_owner,
            realm_id,
            request_key,
            client,
            FrameDocumentParserRootTerminalResult::Fetched(ModuleGraphFetchedSource::new(
                source_url, false, source,
            )),
        )
    }

    pub(super) fn handle_parser_root_start_failure(
        &mut self,
        task_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        request_key: ModuleMapKey,
        client: FrameDocumentParserRootModuleClient,
        error: ModuleLoadError,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        trace_child_parser_module_root_failure(
            task_owner,
            realm_id,
            client.script_handle(),
            &request_key,
            &error,
        );
        let work = module_script_graph_failed_work_from_root_client(
            task_owner,
            realm_id,
            client.pending_script_id(task_owner.document_owner()),
            client.script().clone(),
            client.script_handle(),
            request_key,
            client.load_delay_token(),
            error,
        );
        self.notify_graph_terminal_work(FrameDocumentModuleScriptGraphNotification::failed(work))
    }

    pub(super) fn handle_dependency_terminal_work(
        &mut self,
        work: FrameDocumentModuleDependencyTerminalWork,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        let (task_owner, realm_id, request_key, client, fetch_request, result) =
            work.into_terminal_parts();
        let result = match result {
            FrameDocumentModuleFetchTerminalResult::Fetched(source) => Ok(source),
            FrameDocumentModuleFetchTerminalResult::Failed(error) => {
                Err(ModuleLoadError::new(ModuleLoadStage::Fetch, error))
            }
        };
        self.finish_dependency_fetch(
            task_owner,
            realm_id,
            request_key,
            client,
            fetch_request,
            result,
        )
    }

    pub(super) fn handle_single_module_script_terminal_work(
        &mut self,
        work: FrameDocumentModuleScriptTerminalWork,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        let (task_owner, realm_id, key, client) = work.into_terminal_parts();
        let document_owner = task_owner.document_owner();
        let tree_id = client.token().tree_id;
        let Some(mut resume) =
            self.vm
                .take_child_parser_module_tree_job(document_owner, realm_id, tree_id)
        else {
            tracing::debug!(
                owner = ?task_owner,
                realm_id = ?realm_id,
                tree_id = tree_id.0,
                tree_client_sequence = client.token().sequence,
                phase = ?client.import_phase(),
                url = %key.url(),
                "module-script terminal task had no retained child parser tree job"
            );
            return FrameDocumentModuleScriptTerminalFollowup::none();
        };
        let advance_result = self
            .vm
            .with_current_child_module_tree_owner_or_module_load_error(
                document_owner,
                realm_id,
                key.url(),
                |module_owner| {
                    resume
                        .job_mut()
                        .finish_joined_module_map_fetch_for_local_key_with_owner(
                            module_owner,
                            &key,
                            client.token(),
                        )
                },
            );
        self.apply_tree_advance(document_owner, realm_id, tree_id, resume, advance_result)
    }

    fn compile_or_reuse_module_record(
        &mut self,
        document_owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        request_key: ModuleMapKey,
        compile_key: ModuleMapKey,
        source: &ModuleSource,
        source_url: &Url,
        fetch_metadata: &ModuleFetchMetadata,
    ) -> Result<ChildModuleRootRecord, ModuleLoadError> {
        self.vm
            .with_current_child_module_tree_owner_or_module_load_error(
                document_owner,
                realm_id,
                source_url,
                |module_owner| {
                    if let Some(record) = cached_module_root_record(module_owner, &request_key)? {
                        return Ok(record);
                    }
                    module_owner
                        .compile_module_record(compile_key, source, source_url, fetch_metadata)
                        .map(|(record, identity)| {
                            let parent_key = record.key().clone();
                            let requests = record.requests().to_vec();
                            let entry_id = module_owner.insert_compiled_module_record(
                                request_key,
                                record,
                                identity,
                                fetch_metadata.clone(),
                            );
                            ChildModuleRootRecord {
                                entry_id,
                                key: parent_key,
                                base_url: source_url.clone(),
                                requests,
                                fetch_metadata: fetch_metadata.clone(),
                            }
                        })
                },
            )
    }

    fn advance_tree_job(
        &mut self,
        document_owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        tree_id: ModuleTreeId,
        fallback_url: &Url,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        let Some(mut resume) =
            self.vm
                .take_child_parser_module_tree_job(document_owner, realm_id, tree_id)
        else {
            return FrameDocumentModuleScriptTerminalFollowup::none();
        };
        let advance_result = self
            .vm
            .with_current_child_module_tree_owner_or_module_load_error(
                document_owner,
                realm_id,
                fallback_url,
                |module_owner| {
                    resume
                        .job_mut()
                        .advance_chromium_tree_owner_lane_with_owner(module_owner)
                },
            );
        self.apply_tree_advance(document_owner, realm_id, tree_id, resume, advance_result)
    }

    fn finish_dependency_fetch(
        &mut self,
        task_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        request_key: ModuleMapKey,
        client: FrameDocumentStaticDependencyModuleClient,
        fetch_request: NativeModuleGraphFetchRequest,
        result: Result<ModuleGraphFetchedSource, ModuleLoadError>,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        let document_owner = task_owner.document_owner();
        let tree_id = client.tree_client().tree_id;
        let Some(mut resume) =
            self.vm
                .take_child_parser_module_tree_job(document_owner, realm_id, tree_id)
        else {
            let error = ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                "module dependency terminal task had no retained child parser tree job",
            );
            trace_child_module_dependency_failure(
                task_owner,
                realm_id,
                Some(client.parent_entry_id()),
                &request_key,
                &error,
            );
            return FrameDocumentModuleScriptTerminalFollowup::none();
        };
        let realm_id = resume.root().realm_id();
        let advance_result = self
            .vm
            .with_current_child_module_tree_owner_or_module_load_error(
                document_owner,
                realm_id,
                fetch_request.source_url(),
                |module_owner| {
                    resume
                        .job_mut()
                        .finish_module_tree_fetch_for_request_with_owner(
                            module_owner,
                            &fetch_request,
                            result,
                        )
                },
            );
        self.apply_tree_advance(document_owner, realm_id, tree_id, resume, advance_result)
    }

    fn apply_tree_advance(
        &mut self,
        document_owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        tree_id: ModuleTreeId,
        resume: NativeParserModuleTreeJobResume,
        advance_result: Result<NativeModuleGraphJobAdvance, ModuleLoadError>,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        let action = frame_document_parser_module_tree_advance_action(
            document_owner,
            realm_id,
            tree_id,
            resume,
            advance_result,
        );
        FrameDocumentParserModuleTreeAdvanceRunner::new(ScriptVmParserModuleTreeAdvanceHooks {
            vm: self.vm,
        })
        .run_tree_advance_action(action)
    }

    fn notify_graph_terminal_work(
        &mut self,
        notification: FrameDocumentModuleScriptGraphNotification,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        super::child_document_script_scheduler::ChildDocumentScriptSchedulerOwner::new(self.vm)
            .notify_module_script_graph_terminal_work(notification)
    }

    fn handle_parser_root_terminal_result(
        &mut self,
        task_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        request_key: ModuleMapKey,
        client: FrameDocumentParserRootModuleClient,
        result: FrameDocumentParserRootTerminalResult,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        let record = match result {
            FrameDocumentParserRootTerminalResult::Fetched(fetched_source) => {
                let compile_url = if client.source_is_external() {
                    fetched_source.final_url().clone()
                } else {
                    client.base_url().clone()
                };
                let compile_key = if client.source_is_external() {
                    fetched_source.effective_key_for_request(&request_key)
                } else {
                    request_key.clone()
                };
                let fetch_metadata = if client.source_is_external() {
                    ModuleFetchMetadata::from_top_level_script_fetch_metadata(
                        client.fetch_metadata(),
                    )
                } else {
                    ModuleFetchMetadata::from_loaded_module_script_fetch_metadata(
                        client.fetch_metadata(),
                    )
                }
                .with_response_referrer_policy(fetched_source.response_referrer_policy());
                self.compile_or_reuse_module_record(
                    task_owner.document_owner(),
                    realm_id,
                    request_key.clone(),
                    compile_key,
                    fetched_source.source(),
                    &compile_url,
                    &fetch_metadata,
                )
            }
            FrameDocumentParserRootTerminalResult::Compiled => self
                .vm
                .with_current_child_module_tree_owner_or_module_load_error(
                    task_owner.document_owner(),
                    realm_id,
                    request_key.url(),
                    |module_owner| {
                        cached_module_root_record(module_owner, &request_key)?.ok_or_else(|| {
                            ModuleLoadError::new(
                                ModuleLoadStage::Compile,
                                "compiled child module terminal lost its module record",
                            )
                        })
                    },
                ),
            FrameDocumentParserRootTerminalResult::Failed(error) => {
                Err(ModuleLoadError::new(ModuleLoadStage::Fetch, error))
            }
        };
        let record = match record {
            Ok(record) => record,
            Err(error) => {
                return self.handle_parser_root_start_failure(
                    task_owner,
                    realm_id,
                    request_key,
                    client,
                    error,
                );
            }
        };
        let tree_id = self.vm.record_compiled_child_parser_root(
            task_owner,
            realm_id,
            client.pending_script_id(task_owner.document_owner()),
            client.script().clone(),
            client.script_handle(),
            request_key,
            record.base_url.clone(),
            record.entry_id,
            record.key,
            record.requests,
            record.fetch_metadata,
            client.load_delay_token(),
        );
        self.advance_tree_job(
            task_owner.document_owner(),
            realm_id,
            tree_id,
            &record.base_url,
        )
    }
}

impl FrameDocumentParserModuleTreeAdvanceHooks for ScriptVmParserModuleTreeAdvanceHooks<'_> {
    fn queue_dependency_fetches(
        &mut self,
        document_owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        tree_id: ModuleTreeId,
        resume: Box<NativeParserModuleTreeJobResume>,
        fetches: Vec<NativeModuleGraphFetchRequest>,
    ) -> FrameDocumentParserModuleTreeAdvanceDependencyFetchResult {
        let mut resume = *resume;
        let fetch_tasks = self
            .vm
            .record_child_parser_module_tree_fetches(&mut resume, fetches);
        match fetch_tasks {
            Ok(fetch_tasks) => {
                let fetch_count = fetch_tasks.len();
                let mut queued_fetch = false;
                for task in fetch_tasks {
                    match self
                        .vm
                        ._context_host
                        .borrow()
                        .route_child_module_dependency_fetch_start(task)
                    {
                        Ok(outcome) => queued_fetch |= outcome.was_queued(),
                        Err(error) => {
                            return FrameDocumentParserModuleTreeAdvanceDependencyFetchResult::DependencyFetchStartFailed {
                                trace: FrameDocumentParserModuleTreeAdvanceFailureTrace::DependencyFetchStartRoute,
                                work: Box::new(module_script_graph_failed_work_from_tree_job(
                                    resume, error,
                                )),
                            };
                        }
                    }
                }
                tracing::debug!(
                    owner = ?document_owner,
                    realm_id = ?realm_id,
                    tree_id = tree_id.0,
                    fetch_count,
                    "child parser module tree job emitted shared dependency fetches"
                );
                self.vm.restore_child_parser_module_tree_job(resume);
                let followup = if queued_fetch {
                    FrameDocumentModuleScriptTerminalFollowup::module_dependency_fetch_queued()
                } else {
                    FrameDocumentModuleScriptTerminalFollowup::module_script_wait_retained()
                };
                FrameDocumentParserModuleTreeAdvanceDependencyFetchResult::Followup(followup)
            }
            Err(error) => {
                FrameDocumentParserModuleTreeAdvanceDependencyFetchResult::DependencyFetchStartFailed {
                    trace: FrameDocumentParserModuleTreeAdvanceFailureTrace::DependencyFetchTaskConversion,
                    work: Box::new(module_script_graph_failed_work_from_tree_job(resume, error)),
                }
            }
        }
    }

    fn restore_waiting(&mut self, resume: Box<NativeParserModuleTreeJobResume>) {
        self.vm.restore_child_parser_module_tree_job(*resume);
    }

    fn notify_graph_ready(
        &mut self,
        work: Box<crate::document_script_scheduler::DocumentModuleGraphReadyWork>,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        super::child_document_script_scheduler::ChildDocumentScriptSchedulerOwner::new(self.vm)
            .notify_module_script_graph_terminal_work(
                FrameDocumentModuleScriptGraphNotification::ready(*work),
            )
    }

    fn notify_graph_failed(
        &mut self,
        trace: FrameDocumentParserModuleTreeAdvanceFailureTrace,
        work: Box<crate::document_script_scheduler::DocumentModuleGraphFailedWork>,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        match trace {
            FrameDocumentParserModuleTreeAdvanceFailureTrace::DependencyFetchTaskConversion => {
                let owner = work.owner();
                let realm_id = work.realm_id();
                let tree_id = work.tree_id();
                let message = work.error().message();
                tracing::debug!(
                    owner = ?owner.document_owner(),
                    realm_id = ?realm_id,
                    tree_id = ?tree_id.map(|tree_id| tree_id.0),
                    message = %message,
                    "child parser module tree job dropped after dependency fetch task conversion failure"
                );
            }
            FrameDocumentParserModuleTreeAdvanceFailureTrace::DependencyFetchStartRoute => {
                let owner = work.owner();
                let realm_id = work.realm_id();
                let tree_id = work.tree_id();
                let message = work.error().message();
                tracing::debug!(
                    owner = ?owner.document_owner(),
                    realm_id = ?realm_id,
                    tree_id = ?tree_id.map(|tree_id| tree_id.0),
                    message = %message,
                    "child parser module tree job failed before a dependency fetch could enter its stable Page route"
                );
            }
            FrameDocumentParserModuleTreeAdvanceFailureTrace::OwnerLaneAdvance => {
                let owner = work.owner();
                let realm_id = work.realm_id();
                let tree_id = work.tree_id();
                let message = work.error().message();
                tracing::debug!(
                    owner = ?owner.document_owner(),
                    realm_id = ?realm_id,
                    tree_id = ?tree_id.map(|tree_id| tree_id.0),
                    message = %message,
                    "child parser module tree job failed during owner-lane advance"
                );
            }
        }
        super::child_document_script_scheduler::ChildDocumentScriptSchedulerOwner::new(self.vm)
            .notify_module_script_graph_terminal_work(
                FrameDocumentModuleScriptGraphNotification::Failed(work),
            )
    }
}
