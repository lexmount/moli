use crate::{
    frame_owner_model::{
        FrameDocumentModuleDependencyTerminalWork, FrameDocumentModuleFetchTerminalResult,
        FrameDocumentModuleScriptGraphNotification, FrameDocumentModuleScriptTerminalFollowup,
        FrameDocumentModuleScriptTerminalWork, FrameDocumentOwner,
        FrameDocumentParserModuleTreeAdvanceDependencyFetchResult,
        FrameDocumentParserModuleTreeAdvanceFailureTrace,
        FrameDocumentParserModuleTreeAdvanceHooks, FrameDocumentParserModuleTreeAdvanceRunner,
        FrameDocumentParserRootModuleClient, FrameDocumentParserRootTerminalWork,
        FrameDocumentStaticDependencyModuleClient, FrameDocumentTaskOwner, FrameRealmId,
        frame_document_parser_module_tree_advance_action,
        module_script_graph_failed_work_from_root_client,
        module_script_graph_failed_work_from_tree_job, trace_child_module_dependency_failure,
        trace_child_parser_module_root_failure,
    },
    module_runtime::{
        ModuleEntryId, ModuleFetchMetadata, ModuleGraphFetchedSource, ModuleLoadError,
        ModuleLoadStage, ModuleMapKey, ModuleRequestRecord, ModuleSource,
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
        let request_key = ModuleMapKey::java_script(source_url.clone());
        self.handle_parser_root_terminal_result(
            task_owner,
            realm_id,
            request_key,
            client,
            FrameDocumentModuleFetchTerminalResult::Fetched(ModuleGraphFetchedSource::new(
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

    fn compile_module_record_into_owner(
        &mut self,
        document_owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        request_key: ModuleMapKey,
        compile_key: ModuleMapKey,
        source: &ModuleSource,
        source_url: &Url,
        fetch_metadata: &ModuleFetchMetadata,
    ) -> Result<(ModuleEntryId, ModuleMapKey, Vec<ModuleRequestRecord>), ModuleLoadError> {
        self.vm
            .with_current_child_module_tree_owner_or_module_load_error(
                document_owner,
                realm_id,
                source_url,
                |module_owner| {
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
                            (entry_id, parent_key, requests)
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
        result: FrameDocumentModuleFetchTerminalResult,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        match result {
            FrameDocumentModuleFetchTerminalResult::Fetched(fetched_source) => {
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
                let root_source = fetched_source.source().clone();
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
                let compile_result = self.compile_module_record_into_owner(
                    task_owner.document_owner(),
                    realm_id,
                    request_key.clone(),
                    compile_key,
                    &root_source,
                    &compile_url,
                    &fetch_metadata,
                );
                match compile_result {
                    Ok((entry_id, parent_key, requests)) => {
                        let tree_id = self.vm.record_compiled_child_parser_root(
                            task_owner,
                            realm_id,
                            client.pending_script_id(task_owner.document_owner()),
                            client.script().clone(),
                            client.script_handle(),
                            request_key,
                            compile_url.clone(),
                            entry_id,
                            parent_key,
                            requests,
                            fetch_metadata,
                            client.load_delay_token(),
                        );
                        self.advance_tree_job(
                            task_owner.document_owner(),
                            realm_id,
                            tree_id,
                            &compile_url,
                        )
                    }
                    Err(error) => {
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
                        self.notify_graph_terminal_work(
                            FrameDocumentModuleScriptGraphNotification::failed(work),
                        )
                    }
                }
            }
            FrameDocumentModuleFetchTerminalResult::Failed(error) => {
                let error = ModuleLoadError::new(ModuleLoadStage::Fetch, error);
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
                self.notify_graph_terminal_work(FrameDocumentModuleScriptGraphNotification::failed(
                    work,
                ))
            }
        }
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
