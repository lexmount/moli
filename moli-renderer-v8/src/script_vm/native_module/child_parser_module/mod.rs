mod execution;

use super::*;

pub(crate) use crate::document_script_scheduler::FrameModuleScriptEvaluationStart as ChildParserModuleEvaluationStart;
pub(super) use execution::ChildModuleScriptExecutionOwner;

enum ChildParserModuleEvaluationStartInternal {
    AlreadyEvaluated,
    EvaluatedSynchronously,
    Pending {
        root_entry: crate::module_runtime::ModuleEntryId,
        promise: v8::Global<v8::Promise>,
    },
}

impl ScriptVm {
    fn current_child_parser_module_route_task_owner(
        &self,
        owner: crate::frame_owner_model::FrameDocumentOwner,
        realm_id: FrameRealmId,
    ) -> Option<FrameDocumentTaskOwner> {
        self._context_host
            .borrow()
            .current_child_module_route_task_owner(owner, realm_id)
    }

    pub(crate) fn child_parser_module_route_task_is_current(
        &self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
    ) -> bool {
        self.current_child_parser_module_route_task_owner(owner.document_owner(), realm_id)
            == Some(owner)
    }

    pub(crate) fn start_child_parser_module_graph_evaluation(
        &mut self,
        work: &crate::document_script_scheduler::DocumentModuleGraphReadyWork,
    ) -> std::result::Result<ChildParserModuleEvaluationStart, ModuleLoadError> {
        let owner = work.owner();
        let realm_id = work.realm_id();
        let task_owner = self
            .current_child_parser_module_route_task_owner(owner.document_owner(), realm_id)
            .ok_or_else(|| {
                ModuleLoadError::new(
                    ModuleLoadStage::Evaluate,
                    "child parser module graph-ready work no longer has a current owner realm",
                )
            })?;
        if task_owner != owner {
            return Err(ModuleLoadError::new(
                ModuleLoadStage::Evaluate,
                "child parser module graph-ready work owner token is stale",
            ));
        }

        let evaluation = self
            .with_current_child_document_modulator(
                owner.document_owner(),
                realm_id,
                |vm, document_modulator| {
                    vm.start_child_parser_module_graph_evaluation_with_modulator(
                        owner.document_owner(),
                        realm_id,
                        document_modulator,
                        work.graph(),
                    )
                },
            )
            .ok_or_else(|| {
                ModuleLoadError::new(
                    ModuleLoadStage::Evaluate,
                    "child parser module graph-ready work has no current document modulator",
                )
            })??;

        match evaluation {
            ChildParserModuleEvaluationStartInternal::AlreadyEvaluated => {
                Ok(ChildParserModuleEvaluationStart::AlreadyEvaluated)
            }
            ChildParserModuleEvaluationStartInternal::EvaluatedSynchronously => {
                Ok(ChildParserModuleEvaluationStart::EvaluatedSynchronously)
            }
            ChildParserModuleEvaluationStartInternal::Pending {
                root_entry,
                promise,
            } => {
                let reaction_id =
                    super::super::child_document_script_scheduler::ChildDocumentScriptSchedulerOwner::new(
                        self,
                    )
                    .reserve_pending_parser_module_evaluation(work, root_entry);
                let context_ptr = self.frame_realm_context_ptr(realm_id).map_err(|error| {
                    ModuleLoadError::new(
                        ModuleLoadStage::Evaluate,
                        format!(
                            "failed to find FrameRealm {realm_id:?} for child parser module evaluation reaction: {error}"
                        ),
                    )
                })?;
                if let Err(error) = self.attach_child_parser_module_script_evaluation_reactions(
                    context_ptr,
                    owner,
                    realm_id,
                    reaction_id,
                    promise,
                ) {
                    super::super::child_document_script_scheduler::ChildDocumentScriptSchedulerOwner::new(
                        self,
                    )
                    .remove_pending_parser_module_evaluation(reaction_id);
                    return Err(error);
                }
                Ok(ChildParserModuleEvaluationStart::Pending { root_entry })
            }
        }
    }

    pub(in crate::script_vm) async fn run_child_module_script_ready_work(
        &mut self,
        work: crate::document_script_scheduler::FrameDocumentModuleScriptReadyWork,
    ) -> crate::document_script_scheduler::FrameModuleScriptRunOutcome<
        crate::document_script_scheduler::DocumentScriptExecutionOutcome,
    > {
        crate::document_script_scheduler::FrameModuleScriptDocumentScriptRunner::new(
            ChildModuleScriptExecutionOwner::new(self),
        )
        .run_document_script_work(work)
        .await
    }

    fn start_child_parser_module_graph_evaluation_with_modulator(
        &mut self,
        document_owner: crate::frame_owner_model::FrameDocumentOwner,
        realm_id: FrameRealmId,
        document_modulator: &mut NativeDocumentModulator,
        graph: &crate::module_runtime::ModuleGraphHandle,
    ) -> std::result::Result<ChildParserModuleEvaluationStartInternal, ModuleLoadError> {
        if document_modulator.entry(graph.root_entry).state() == ModuleMapEntryState::Evaluated {
            return Ok(ChildParserModuleEvaluationStartInternal::AlreadyEvaluated);
        }

        let context_ptr = self.frame_realm_context_ptr(realm_id).map_err(|error| {
            ModuleLoadError::new(
                ModuleLoadStage::Instantiate,
                format!("failed to find FrameRealm {realm_id:?} for module instantiate: {error}"),
            )
        })?;
        if document_modulator.entry(graph.root_entry).state() == ModuleMapEntryState::Compiled {
            self.instantiate_native_module_graph_with_modulator_in_context(
                context_ptr,
                document_modulator,
                graph,
            )?;
        }
        let evaluation = self.evaluate_native_module_graph_with_modulator_in_context(
            context_ptr,
            document_owner,
            realm_id,
            document_modulator,
            graph.root_entry,
            NativeModuleEvaluationOwner::Script,
        )?;
        if let Some(promise) = evaluation.promise {
            return Ok(ChildParserModuleEvaluationStartInternal::Pending {
                root_entry: graph.root_entry,
                promise,
            });
        }
        Ok(ChildParserModuleEvaluationStartInternal::EvaluatedSynchronously)
    }

    pub(crate) fn apply_child_parser_module_evaluation_fulfilled(
        &mut self,
        reaction_id: u64,
    ) -> usize {
        super::super::child_document_script_scheduler::ChildDocumentScriptSchedulerOwner::new(self)
            .mark_parser_module_evaluation_fulfilled(reaction_id)
    }

    pub(crate) fn apply_child_parser_module_evaluation_rejected(
        &mut self,
        reaction_id: u64,
        reason: String,
        error_constructor: Option<ScriptErrorConstructorKind>,
    ) -> usize {
        super::super::child_document_script_scheduler::ChildDocumentScriptSchedulerOwner::new(self)
            .mark_parser_module_evaluation_rejected(reaction_id, reason, error_constructor)
    }
}
