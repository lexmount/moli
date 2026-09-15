use crate::{
    document_module_graph::ModuleMapKey,
    document_runtime::DomHandle,
    frame_owner_model::{
        ChildDocumentModuleFetchTarget, FrameDocumentModuleDependencyFetchStartOutcome,
        FrameDocumentModuleFetchDisposition, FrameDocumentModuleFetchTerminalResult,
        FrameDocumentModuleTerminalBatch, FrameDocumentModuleTerminalQueueFollowup,
        FrameDocumentModulepreloadFetchCompletionAction,
        FrameDocumentModulepreloadFetchCompletionHooks,
        FrameDocumentModulepreloadFetchCompletionRunner,
        FrameDocumentModulepreloadFetchFinishResult, FrameDocumentModulepreloadStartActionHooks,
        FrameDocumentModulepreloadStartActionRunner, FrameDocumentOwner,
        FrameDocumentParserModuleRootStartKind, FrameDocumentTaskOwner, FrameRealmId,
    },
    module_runtime::NativeModuleGraphFetchRequest,
    module_runtime::{ModuleLoadError, ModuleLoadStage, NativeModuleSingleFetchRequest},
    page_task_queue::{
        RendererPageChildFrameTaskTarget, RendererPageChildParserModuleRootStartTarget,
    },
    types::{
        ChildModuleDependencyFetchCompletion, ChildModulepreloadFetchCompletion,
        ChildParserModuleRootFetchCompletion,
    },
};

use super::{ScriptVm, child_module_script_terminal::ChildModuleScriptTerminalOwner};

pub(super) struct ChildModuleFetchOwner<'vm> {
    vm: &'vm mut ScriptVm,
}

impl ScriptVm {
    pub(crate) fn current_child_parser_module_root_start_target(
        &self,
        expected: RendererPageChildParserModuleRootStartTarget,
    ) -> Option<RendererPageChildParserModuleRootStartTarget> {
        self._context_host
            .borrow()
            .current_child_parser_module_root_start_target(expected)
    }

    pub(crate) fn settle_stale_child_parser_module_root_start(
        &mut self,
        task: &crate::frame_owner_model::FrameDocumentParserModuleRootStartTask,
    ) {
        self._context_host
            .borrow_mut()
            .settle_stale_child_parser_module_root_start(task);
    }

    pub(crate) fn apply_current_child_parser_module_root_start(
        &mut self,
        authorization: crate::runtime::AuthorizedCurrentPageChildParserModuleRootStart,
    ) {
        ChildModuleFetchOwner::new(self).apply_current_parser_root_start(authorization);
    }

    /// Consume one parser-root task from the production child-frame family in
    /// low-level semantic fixtures. This does not restore the deleted pump
    /// queue or its runnable selector.
    #[cfg(test)]
    pub(crate) fn run_child_parser_module_root_start_body_for_test(
        &mut self,
    ) -> anyhow::Result<Option<bool>> {
        let source = self
            ._page_task_residence_for_executor_test
            .as_ref()
            .expect("parser-module executor fixture must retain its production Page source")
            .task_sources();
        let Some(task) = source.take_scheduler_task_for_executor_test(|descriptor| {
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
        let owner = task.owner();
        let RendererPageChildFrameTaskTarget::ParserModuleRootStart(target) = owner.target() else {
            unreachable!("parser-module selector must only dequeue parser-root tasks")
        };
        if self.current_child_parser_module_root_start_target(target) == Some(target) {
            self.apply_current_child_parser_module_root_start(
                crate::runtime::AuthorizedCurrentPageChildParserModuleRootStart::new_for_executor_test(
                    task,
                ),
            );
            return Ok(Some(true));
        }
        let root_start = task.into_parser_module_root_start_task();
        self.settle_stale_child_parser_module_root_start(&root_start);
        Ok(Some(false))
    }
}

impl<'vm> ChildModuleFetchOwner<'vm> {
    pub(super) fn new(vm: &'vm mut ScriptVm) -> Self {
        Self { vm }
    }

    pub(super) fn apply_current_parser_root_start(
        &mut self,
        authorization: crate::runtime::AuthorizedCurrentPageChildParserModuleRootStart,
    ) {
        let task = authorization.into_task();
        let RendererPageChildFrameTaskTarget::ParserModuleRootStart(target) = task.owner().target()
        else {
            unreachable!("parser-module-root executor received another child-frame task kind")
        };
        let root = task.into_parser_module_root_start_task();
        let realm_id = target.realm_id();
        let context_host = self.vm._context_host.clone();
        tracing::debug!(
            owner = ?root.owner(),
            realm_id = ?realm_id,
            script_node_id = ?root.script().node_id,
            script_url = %root.script().url,
            kind = ?root.kind(),
            "child parser module graph-start turn accepted for registered PendingScript"
        );
        if let FrameDocumentParserModuleRootStartKind::LoadedSource(source) = root.kind().clone() {
            let (_child_handle, owner, client, _kind) = root.into_parts();
            let _followup = ChildModuleScriptTerminalOwner::new(self.vm)
                .handle_loaded_parser_root_start(owner, realm_id, client, source);
            return;
        }
        let FrameDocumentParserModuleRootStartKind::ExternalFetch { key } = root.kind().clone()
        else {
            unreachable!("loaded child parser module roots return before external fetch setup");
        };
        let Some(target) = context_host
            .borrow()
            .capture_child_module_fetch_producer_for_child(
                root.child_handle(),
                root.owner(),
                realm_id,
            )
        else {
            tracing::debug!(
                child_handle = ?root.child_handle(),
                owner = ?root.owner(),
                realm_id = ?realm_id,
                script_url = %root.script().url,
                "failing child parser module root before fetch because its exact target is unavailable"
            );
            let (_child_handle, owner, client, _kind) = root.into_parts();
            let _followup = ChildModuleScriptTerminalOwner::new(self.vm)
                .handle_parser_root_start_failure(
                    owner,
                    realm_id,
                    key,
                    client,
                    ModuleLoadError::new(
                        ModuleLoadStage::Fetch,
                        "child parser module root lost its exact target before native fetch",
                    ),
                );
            return;
        };
        let Some(loader) = context_host
            .borrow()
            .document_resource_loader_for_owner(root.owner())
        else {
            let (_child_handle, owner, client, _kind) = root.into_parts();
            let _followup = ChildModuleScriptTerminalOwner::new(self.vm)
                .handle_parser_root_start_failure(
                    owner,
                    realm_id,
                    key,
                    client,
                    ModuleLoadError::new(
                        ModuleLoadStage::Fetch,
                        "child parser module graph accepted without an installed loader",
                    ),
                );
            return;
        };
        let reservation = self.vm.reserve_child_parser_root_module_client(
            root.owner(),
            realm_id,
            key.clone(),
            root.client().clone(),
        );
        match reservation.fetch_disposition() {
            FrameDocumentModuleFetchDisposition::StartedFetch(_) => {}
            FrameDocumentModuleFetchDisposition::JoinedFetching(_) => {
                let _followup = self
                    .vm
                    .post_current_child_document_modulator_terminals_to_frame_lane(
                        root.owner().document_owner(),
                        realm_id,
                    );
                return;
            }
            FrameDocumentModuleFetchDisposition::AlreadyFetched(_)
            | FrameDocumentModuleFetchDisposition::AlreadyLinked(_)
            | FrameDocumentModuleFetchDisposition::AlreadyFailed(_) => {
                let _followup = self
                    .vm
                    .post_current_child_document_modulator_terminals_to_frame_lane(
                        root.owner().document_owner(),
                        realm_id,
                    );
                return;
            }
        }
        let Some(start) = context_host
            .borrow_mut()
            .begin_current_child_module_root_fetch_client(reservation)
        else {
            let (_child_handle, owner, client, _kind) = root.into_parts();
            let _followup = ChildModuleScriptTerminalOwner::new(self.vm)
                .handle_parser_root_start_failure(
                    owner,
                    realm_id,
                    key,
                    client,
                    ModuleLoadError::new(
                        ModuleLoadStage::Fetch,
                        "child parser module graph could not admit its Document request before fetch start",
                    ),
                );
            return;
        };
        let fetch_start = root.into_external_fetch_start(realm_id, start);
        context_host
            .borrow_mut()
            .start_child_parser_module_root_fetch(&loader, fetch_start, target);
    }

    pub(super) fn apply_current_dependency_fetch_start(
        &mut self,
        authorization: crate::runtime::AuthorizedCurrentChildModuleDependencyFetchStart,
    ) -> FrameDocumentModuleDependencyFetchStartOutcome {
        let (target, task) = authorization.into_parts();
        let loader = self
            .vm
            ._context_host
            .borrow()
            .document_resource_loader_for_owner(task.owner())
            .expect("authorized child module dependency requires its committed Document authority");
        let Some(start) = self
            .vm
            ._context_host
            .borrow_mut()
            .begin_current_child_module_dependency_fetch_client(task.reservation().clone())
        else {
            let realm_id = task.realm_id();
            let terminal_followup = self.vm.finish_child_parser_module_dependency_fetch(
                realm_id,
                task,
                FrameDocumentModuleFetchTerminalResult::Failed(
                    "child module dependency could not admit its Document request before fetch start"
                        .to_owned(),
                ),
            );
            return FrameDocumentModuleDependencyFetchStartOutcome::RequestAdmissionUnavailable {
                terminal_followup,
            };
        };
        let disposition = start.fetch_disposition();
        let fetch_started = matches!(
            disposition,
            FrameDocumentModuleFetchDisposition::StartedFetch(_)
        );
        if fetch_started {
            tracing::debug!(
                owner = ?task.owner(),
                realm_id = ?task.realm_id(),
                parent_entry_id = task.client().parent_entry_id().raw(),
                parent_url = %task.client().parent_key().url(),
                specifier = %task.client().specifier(),
                dependency_url = %task.dependency_key().url(),
                tree_id = task.client().tree_client().tree_id.0,
                tree_client_sequence = task.client().tree_client().sequence,
                request_id = ?start.request_id(),
                entry_id = start.entry_id().raw(),
                disposition = ?start.fetch_disposition(),
                "child module dependency fetch handed to owner network bridge"
            );
        }
        self.vm
            ._context_host
            .borrow_mut()
            .start_child_module_dependency_fetch(&loader, start, task, target.child_handle());
        FrameDocumentModuleDependencyFetchStartOutcome::ClientAccepted { disposition }
    }

    pub(crate) fn apply_current_modulepreload_start_task(
        &mut self,
        authorization: crate::runtime::AuthorizedCurrentChildModulepreloadStartTask,
    ) -> crate::frame_owner_model::FrameDocumentModulepreloadStartOutcome {
        self.execute_current_modulepreload_start_task(authorization.into_task())
    }

    fn execute_current_modulepreload_start_task(
        &mut self,
        task: crate::frame_owner_model::FrameDocumentModulepreloadFetchTask,
    ) -> crate::frame_owner_model::FrameDocumentModulepreloadStartOutcome {
        let action = self
            .vm
            .child_document_modulator_store
            .start_modulepreload_fetch_task(task);
        FrameDocumentModulepreloadStartActionRunner::new(ScriptVmChildModulepreloadStartHooks {
            vm: self.vm,
        })
        .run_start_action(action)
    }

    pub(super) fn apply_parser_root_fetch_completion(
        &mut self,
        completion: ChildParserModuleRootFetchCompletion,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        let (target, request_key, result) = completion.into_module_terminal_parts();
        self.vm.finish_child_parser_root_module_fetch(
            target.task_owner(),
            target.realm_id(),
            request_key,
            result,
        )
    }

    pub(super) fn apply_dependency_fetch_completion(
        &mut self,
        completion: ChildModuleDependencyFetchCompletion,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        let (task, result) = completion.into_module_terminal_parts();
        let result = result
            .map(FrameDocumentModuleFetchTerminalResult::Fetched)
            .unwrap_or_else(FrameDocumentModuleFetchTerminalResult::Failed);
        let realm_id = task.realm_id();
        self.vm
            .finish_child_parser_module_dependency_fetch(realm_id, task, result)
    }

    pub(super) fn apply_current_modulepreload_fetch_completion(
        &mut self,
        completion: ChildModulepreloadFetchCompletion,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        let (target, load_id, result) = completion.into_module_terminal_parts();
        self.finish_modulepreload_fetch_completion(
            target.task_owner(),
            target.realm_id(),
            load_id,
            result,
        )
    }

    fn finish_modulepreload_fetch_completion(
        &mut self,
        task_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        load_id: u64,
        result: std::result::Result<crate::module_runtime::ModuleGraphFetchedSource, String>,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        let owner = task_owner.document_owner();
        let Some(request) = self
            .vm
            .child_document_modulator_store
            .take_modulepreload_graph_fetch(owner, realm_id, load_id)
        else {
            self.vm.record_runtime_warning(format_args!(
                "child modulepreload fetch completion {} for {:?}/{:?} arrived without an owner-local in-flight fetch record",
                load_id, owner, realm_id
            ));
            return FrameDocumentModuleTerminalQueueFollowup::terminal_warning_recorded();
        };
        let source = match result {
            Ok(fetched_source) => self.vm.module_graph_fetched_source_or_csp_error(
                load_id,
                fetched_source,
                request.fetch_metadata(),
            ),
            Err(error) => Err(ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                format!("child modulepreload fetch completion {load_id} failed: {error}"),
            )),
        };
        let action = FrameDocumentModulepreloadFetchCompletionAction::new(
            task_owner, realm_id, load_id, request, source,
        );
        let outcome = FrameDocumentModulepreloadFetchCompletionRunner::new(
            ScriptVmChildModulepreloadCompletionHooks { vm: self.vm },
        )
        .run_completion_action(action);
        outcome.into_terminal_followup()
    }
}

impl ScriptVm {
    pub(crate) fn apply_current_child_module_dependency_fetch_start(
        &mut self,
        authorization: crate::runtime::AuthorizedCurrentChildModuleDependencyFetchStart,
    ) -> FrameDocumentModuleDependencyFetchStartOutcome {
        ChildModuleFetchOwner::new(self).apply_current_dependency_fetch_start(authorization)
    }

    /// Applies a modulepreload start only after the Page owner has proved the
    /// complete root-Document and child/document/realm target current.
    pub(crate) fn apply_current_child_modulepreload_start_task(
        &mut self,
        authorization: crate::runtime::AuthorizedCurrentChildModulepreloadStartTask,
    ) -> crate::frame_owner_model::FrameDocumentModulepreloadStartOutcome {
        ChildModuleFetchOwner::new(self).apply_current_modulepreload_start_task(authorization)
    }
}

struct ScriptVmChildModulepreloadStartHooks<'vm> {
    vm: &'vm mut ScriptVm,
}

impl FrameDocumentModulepreloadStartActionHooks for ScriptVmChildModulepreloadStartHooks<'_> {
    fn post_current_document_modulator_terminals(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        self.vm
            .post_current_child_document_modulator_terminals_to_frame_lane(owner, realm_id)
    }

    fn schedule_modulepreload_fetch(
        &mut self,
        target: ChildDocumentModuleFetchTarget,
        link_handle: DomHandle,
        key: ModuleMapKey,
        load_id: u64,
        request: Box<NativeModuleGraphFetchRequest>,
    ) {
        let loader = self
            .vm
            ._context_host
            .borrow()
            .document_resource_loader_for_owner(target.task_owner())
            .expect("authorized child modulepreload requires its committed Document authority");
        if self
            .vm
            .current_child_document_module_fetch_target(target.child_handle())
            != Some(target)
        {
            tracing::debug!(
                ?target,
                link_handle = ?link_handle,
                load_id,
                url = %key.url(),
                "failing child modulepreload before fetch because its exact target is unavailable"
            );
            let completion = ChildModulepreloadFetchCompletion::new(
                target,
                load_id,
                Err("child modulepreload lost its exact target before native fetch".to_owned()),
            );
            let _ = self
                .vm
                ._context_host
                .borrow()
                .resource_completion_sender()
                .send_child_modulepreload_fetch(completion);
            return;
        };
        self.vm
            .resource_scheduler()
            .schedule_child_modulepreload_graph_fetch(loader, target, load_id, *request);
        tracing::debug!(
            ?target,
            link_handle = ?link_handle,
            load_id,
            url = %key.url(),
            "child modulepreload fetch scheduled through request-carrying owner route"
        );
    }

    fn record_joined_fetching(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        link_handle: DomHandle,
        key: &ModuleMapKey,
    ) {
        tracing::debug!(
            owner = ?owner,
            realm_id = ?realm_id,
            link_handle = ?link_handle,
            url = %key.url(),
            "child modulepreload link joined existing module map fetch"
        );
    }

    fn record_joined_terminal_success(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        link_handle: DomHandle,
        key: &ModuleMapKey,
    ) {
        tracing::debug!(
            owner = ?owner,
            realm_id = ?realm_id,
            link_handle = ?link_handle,
            url = %key.url(),
            "child modulepreload link joined terminal successful module map entry"
        );
    }

    fn record_joined_terminal_failure(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        link_handle: DomHandle,
        key: &ModuleMapKey,
    ) {
        tracing::debug!(
            owner = ?owner,
            realm_id = ?realm_id,
            link_handle = ?link_handle,
            url = %key.url(),
            "child modulepreload link joined terminal failed module map entry"
        );
    }
}

struct ScriptVmChildModulepreloadCompletionHooks<'vm> {
    vm: &'vm mut ScriptVm,
}

impl FrameDocumentModulepreloadFetchCompletionHooks
    for ScriptVmChildModulepreloadCompletionHooks<'_>
{
    fn finish_modulepreload_fetch(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        request: NativeModuleSingleFetchRequest,
        source: std::result::Result<
            crate::module_runtime::ModuleGraphFetchedSource,
            ModuleLoadError,
        >,
    ) -> FrameDocumentModulepreloadFetchFinishResult {
        self.vm
            .child_document_modulator_store
            .finish_modulepreload_fetch(owner, realm_id, request, source)
            .map(FrameDocumentModulepreloadFetchFinishResult::Finished)
            .unwrap_or(FrameDocumentModulepreloadFetchFinishResult::MissingDocumentModulator)
    }

    fn queue_module_terminal_batch(
        &mut self,
        batch: FrameDocumentModuleTerminalBatch,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        self.vm
            .push_child_module_terminal_batch_to_frame_lane(batch)
    }

    fn record_missing_modulepreload_modulator(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        load_id: u64,
    ) {
        self.vm.record_runtime_warning(format_args!(
            "child modulepreload fetch completion {load_id} for {:?}/{:?} had no current document modulator",
            owner, realm_id
        ));
    }

    fn record_modulepreload_completion_finished(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        load_id: u64,
        key: &ModuleMapKey,
    ) {
        tracing::debug!(
            owner = ?owner,
            realm_id = ?realm_id,
            load_id,
            url = %key.url(),
            "child modulepreload fetch completion settled child document modulator"
        );
    }
}
