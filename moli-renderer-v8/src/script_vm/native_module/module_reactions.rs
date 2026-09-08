use super::*;

impl ScriptVm {
    pub(crate) fn attach_native_dynamic_module_import_reactions(
        &mut self,
        request: PendingDynamicModuleImport,
        target: DynamicModuleEvaluationTarget,
        promise: v8::Global<v8::Promise>,
    ) -> std::result::Result<(), ModuleLoadError> {
        let context = request.context().clone();
        let document_owner = request.owner();
        let reaction_id = self
            .document_runtime
            .reserve_native_dynamic_module_evaluation_reaction(request, target);
        let attach_result = self
            .renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, context);
                let reaction_id_value = v8::BigInt::new_from_u64(scope, reaction_id);
                let data = NativeDynamicModuleReactionDataDeclaration {
                    reaction_id: reaction_id_value,
                }
                .bind(scope)
                .map_err(|error| {
                    anyhow::anyhow!("failed to create native dynamic import reaction data: {error}")
                })?;
                let on_fulfilled =
                    v8::Function::builder(native_dynamic_module_reaction_fulfilled_callback)
                        .data(data.into())
                        .build(scope)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "failed to create native dynamic import success reaction"
                            )
                        })?;
                let on_rejected =
                    v8::Function::builder(native_dynamic_module_reaction_rejected_callback)
                        .data(data.into())
                        .build(scope)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "failed to create native dynamic import failure reaction"
                            )
                        })?;
                let promise = v8::Local::new(scope, &promise);
                promise
                    .then2(scope, on_fulfilled, on_rejected)
                    .map(|_| ())
                    .ok_or_else(|| {
                        anyhow::anyhow!("failed to attach native dynamic import reactions")
                    })?;
                // `then` does not invoke the dynamic-import reaction inline. It
                // queues a V8 promise-reaction microtask, even when the module
                // evaluation promise is already settled. Run the browser-style
                // checkpoint for this owner-lane task so the fulfilled/rejected
                // callback can transfer the dynamic import result into
                // DocumentRuntime before the driver decides whether more module
                // work is ready.
                Self::perform_microtask_checkpoints(scope, None)
            });
        if let Err(error) = attach_result {
            if let Some(reaction) = self
                .document_runtime
                .take_native_dynamic_module_evaluation_reaction(reaction_id, document_owner)
            {
                let (request, _) = reaction.into_parts();
                let _ = self.reject_native_dynamic_module_import(request, &error.to_string());
            }
            return Err(ModuleLoadError::new(
                ModuleLoadStage::Evaluate,
                error.to_string(),
            ));
        }
        Ok(())
    }
    pub(crate) fn attach_native_module_script_evaluation_reactions(
        &mut self,
        reaction_id: u64,
        promise: v8::Global<v8::Promise>,
    ) -> std::result::Result<(), ModuleLoadError> {
        let document_owner = self.current_main_document_task_owner().ok_or_else(|| {
            ModuleLoadError::new(
                ModuleLoadStage::Evaluate,
                "main module evaluation reaction has no current Document owner",
            )
        })?;
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, &self.page_default_runtime.context);
                let scope = &mut v8::ContextScope::new(scope, context);
                let reaction_id_value = v8::BigInt::new_from_u64(scope, reaction_id);
                let data = NativeModuleScriptReactionDataDeclaration {
                    module_script_reaction_id: reaction_id_value,
                    scheduler_lane_id: v8::BigInt::new_from_u64(
                        scope,
                        document_owner.scheduler_lane_id.0,
                    ),
                    local_window_id: v8::BigInt::new_from_u64(
                        scope,
                        document_owner.local_window_id.0,
                    ),
                    document_id: v8::BigInt::new_from_u64(scope, document_owner.document_id.0),
                }
                .bind(scope)
                .map_err(|error| {
                    anyhow::anyhow!("failed to create native module script reaction data: {error}")
                })?;
                let on_fulfilled =
                    v8::Function::builder(native_module_script_reaction_fulfilled_callback)
                        .data(data.into())
                        .build(scope)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "failed to create native module script success reaction"
                            )
                        })?;
                let on_rejected =
                    v8::Function::builder(native_module_script_reaction_rejected_callback)
                        .data(data.into())
                        .build(scope)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "failed to create native module script failure reaction"
                            )
                        })?;
                let promise = v8::Local::new(scope, &promise);
                promise
                    .then2(scope, on_fulfilled, on_rejected)
                    .map(|_| ())
                    .ok_or_else(|| {
                        anyhow::anyhow!("failed to attach native module script reactions")
                    })?;
                // `then` schedules the module-script evaluation reaction as a
                // V8 microtask, even if the evaluation promise is already
                // settled. Chromium runs module evaluation's error handling
                // before the script runner advances to later pending scripts;
                // drain this owner-lane checkpoint now so TLA rejection cannot
                // be observed after a later dynamic module.
                Self::perform_microtask_checkpoints(scope, None)
            })
            .map_err(|error| ModuleLoadError::new(ModuleLoadStage::Evaluate, error.to_string()))
    }
    pub(crate) fn attach_child_parser_module_script_evaluation_reactions(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        document_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        reaction_id: u64,
        promise: v8::Global<v8::Promise>,
    ) -> std::result::Result<(), ModuleLoadError> {
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let reaction_id_value = v8::BigInt::new_from_u64(scope, reaction_id);
                let data = NativeChildModuleScriptReactionDataDeclaration {
                    module_script_reaction_id: reaction_id_value,
                    scheduler_lane_id: v8::BigInt::new_from_u64(
                        scope,
                        document_owner.scheduler_lane_id.0,
                    ),
                    local_window_id: v8::BigInt::new_from_u64(
                        scope,
                        document_owner.local_window_id.0,
                    ),
                    document_id: v8::BigInt::new_from_u64(scope, document_owner.document_id.0),
                    realm_id: v8::BigInt::new_from_i64(scope, realm_id.0),
                }
                .bind(scope)
                .map_err(|error| {
                    anyhow::anyhow!("failed to create child parser module reaction data: {error}")
                })?;
                let on_fulfilled =
                    v8::Function::builder(child_parser_module_reaction_fulfilled_callback)
                        .data(data.into())
                        .build(scope)
                        .ok_or_else(|| {
                            anyhow::anyhow!("failed to create child parser module success reaction")
                        })?;
                let on_rejected =
                    v8::Function::builder(child_parser_module_reaction_rejected_callback)
                        .data(data.into())
                        .build(scope)
                        .ok_or_else(|| {
                            anyhow::anyhow!("failed to create child parser module failure reaction")
                        })?;
                let promise = v8::Local::new(scope, &promise);
                promise
                    .then2(scope, on_fulfilled, on_rejected)
                    .map(|_| ())
                    .ok_or_else(|| {
                        anyhow::anyhow!("failed to attach child parser module script reactions")
                    })?;
                // Evaluation has just returned a genuinely pending promise
                // after its algorithm-required checkpoint. Attaching the TLA
                // observers is setup, not a second checkpoint boundary. The
                // selected DocumentScriptReady task performs its ordinary
                // task-end checkpoint after the script element load event.
                Ok(())
            })
            .map_err(|error| ModuleLoadError::new(ModuleLoadStage::Evaluate, error.to_string()))
    }
    #[cfg(test)]
    pub(crate) fn resolve_native_dynamic_module_import(
        &mut self,
        request: PendingDynamicModuleImport,
        target: &DynamicModuleEvaluationTarget,
    ) -> std::result::Result<(), ModuleLoadError> {
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, request.context());
                let scope = &mut v8::ContextScope::new(scope, context);
                let resolver = v8::Local::new(scope, request.resolver());
                let root_module = v8::Local::new(scope, target.module());
                let namespace = root_module.get_module_namespace();
                let _ = resolver.resolve(scope, namespace);
                Self::perform_microtask_checkpoints(scope, None)?;
                Ok(())
            })
            .map_err(|error: anyhow::Error| {
                ModuleLoadError::new(ModuleLoadStage::Evaluate, error.to_string())
            })?;
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn resolve_native_dynamic_module_source_import(
        &mut self,
        request: PendingDynamicModuleImport,
        root_entry: crate::module_runtime::ModuleEntryId,
    ) -> std::result::Result<NativeDynamicModuleSourceImportResolution, ModuleLoadError> {
        let Some(wasm_record) = self.document_runtime.native_module_wasm_record(root_entry) else {
            let error = ModuleLoadError::new(
                ModuleLoadStage::Resolve,
                format!(
                    "source-phase dynamic import `{}` is not a WebAssembly module",
                    request.specifier()
                ),
            )
            .with_error_constructor(ScriptErrorConstructorKind::SyntaxError);
            self.reject_native_dynamic_module_import_with_error(request, &error)?;
            return Ok(NativeDynamicModuleSourceImportResolution::Rejected);
        };
        self.resolve_native_dynamic_module_source_import_with_wasm_record(request, wasm_record)
    }
    #[cfg(test)]
    pub(super) fn resolve_native_dynamic_module_source_import_with_wasm_record(
        &mut self,
        request: PendingDynamicModuleImport,
        wasm_record: WasmModuleRecord,
    ) -> std::result::Result<NativeDynamicModuleSourceImportResolution, ModuleLoadError> {
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, request.context());
                let scope = &mut v8::ContextScope::new(scope, context);
                let resolver = v8::Local::new(scope, request.resolver());
                let Some(source) = wasm_record.source_module(scope) else {
                    let exception = v8_string(scope, "failed to materialize WebAssembly source")
                        .map(|message| v8::Exception::type_error(scope, message))
                        .unwrap_or_else(|| v8::undefined(scope).into());
                    let _ = resolver.reject(scope, exception);
                    Self::perform_microtask_checkpoints(scope, None)?;
                    return Ok(NativeDynamicModuleSourceImportResolution::Rejected);
                };
                let _ = resolver.resolve(scope, source.into());
                Self::perform_microtask_checkpoints(scope, None)?;
                Ok(NativeDynamicModuleSourceImportResolution::Resolved)
            })
            .map_err(|error: anyhow::Error| {
                ModuleLoadError::new(ModuleLoadStage::Evaluate, error.to_string())
            })
    }
    pub(crate) fn reject_native_dynamic_module_import(
        &mut self,
        request: PendingDynamicModuleImport,
        message: &str,
    ) -> std::result::Result<(), ModuleLoadError> {
        self.reject_native_dynamic_module_import_and_checkpoint(
            request,
            &ModuleLoadError::new(ModuleLoadStage::Fetch, message),
        )
    }
    #[cfg(test)]
    pub(crate) fn reject_native_dynamic_module_import_with_error(
        &mut self,
        request: PendingDynamicModuleImport,
        error: &ModuleLoadError,
    ) -> std::result::Result<(), ModuleLoadError> {
        self.reject_native_dynamic_module_import_and_checkpoint(request, error)
    }
    pub(super) fn reject_native_dynamic_module_import_and_checkpoint(
        &mut self,
        request: PendingDynamicModuleImport,
        error: &ModuleLoadError,
    ) -> std::result::Result<(), ModuleLoadError> {
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, request.context());
                let scope = &mut v8::ContextScope::new(scope, context);
                let resolver = v8::Local::new(scope, request.resolver());
                let exception = module_load_error_value(scope, error)?;
                let _ = resolver.reject(scope, exception);
                Self::perform_microtask_checkpoints(scope, None)?;
                Ok(())
            })
            .map_err(|error: anyhow::Error| {
                ModuleLoadError::new(ModuleLoadStage::Evaluate, error.to_string())
            })
    }
    pub(crate) fn module_reaction_target_is_current(
        &self,
        target: crate::page_task_queue::RendererPageModuleReactionTarget,
    ) -> bool {
        match target {
            crate::page_task_queue::RendererPageModuleReactionTarget::DocumentModuleScript {
                document_owner,
            } => self.current_main_document_task_owner() == Some(document_owner),
            crate::page_task_queue::RendererPageModuleReactionTarget::ChildParserModule {
                document_owner,
                realm_id,
            } => self.child_parser_module_route_task_is_current(document_owner, realm_id),
            crate::page_task_queue::RendererPageModuleReactionTarget::DynamicModuleImport {
                import_owner,
            } => self.dynamic_module_import_owner_is_current(import_owner),
        }
    }
    /// Apply one module reaction whose exact target has already been accepted
    /// by the Page owner arbiter.
    pub(crate) fn apply_current_page_module_reaction(
        &mut self,
        authorization: crate::runtime::AuthorizedCurrentPageModuleReaction,
    ) -> Result<PageModuleReactionApplication> {
        let reaction = authorization.into_task().into_event();
        let application = match reaction {
            RendererPageModuleReactionEvent::DocumentModuleScriptEvaluationFulfilled {
                reaction_id,
                ..
            } => self
                .apply_native_module_script_evaluation_fulfilled(reaction_id)
                .map(PageModuleReactionApplication::module_state_updated),
            RendererPageModuleReactionEvent::DocumentModuleScriptEvaluationRejected {
                reaction_id,
                reason,
                error_value,
                ..
            } => self
                .apply_native_module_script_evaluation_rejected(reaction_id, reason, error_value)
                .map(PageModuleReactionApplication::module_state_updated),
            RendererPageModuleReactionEvent::ChildParserModuleEvaluationFulfilled {
                reaction_id,
                ..
            } => (self.apply_child_parser_module_evaluation_fulfilled(reaction_id) > 0).then_some(
                PageModuleReactionApplication::module_state_updated(
                    PageModuleReactionFollowup::None,
                ),
            ),
            RendererPageModuleReactionEvent::ChildParserModuleEvaluationRejected {
                reaction_id,
                reason,
                error_value,
                ..
            } => (self.apply_child_parser_module_evaluation_rejected(
                reaction_id,
                reason,
                error_value,
            ) > 0)
                .then_some(PageModuleReactionApplication::module_state_updated(
                    PageModuleReactionFollowup::None,
                )),
            RendererPageModuleReactionEvent::DynamicModuleEvaluationFulfilled {
                import_owner,
                reaction_id,
            } => self
                .apply_native_dynamic_module_evaluation_fulfilled(import_owner, reaction_id)
                .map_err(|error| anyhow::anyhow!(error.message().to_owned()))?
                .then_some(PageModuleReactionApplication::dynamic_import_promise_settled()),
            RendererPageModuleReactionEvent::DynamicModuleEvaluationRejected {
                import_owner,
                reaction_id,
                reason,
            } => self
                .apply_native_dynamic_module_evaluation_rejected(import_owner, reaction_id, reason)
                .map_err(|error| anyhow::anyhow!(error.message().to_owned()))?
                .then_some(PageModuleReactionApplication::dynamic_import_promise_settled()),
        };
        Ok(application.unwrap_or(PageModuleReactionApplication::NoPendingReaction))
    }
    pub(crate) fn discard_stale_page_module_reaction(
        &mut self,
        reaction: &RendererPageModuleReactionEvent,
    ) {
        let (import_owner, reaction_id) = match reaction {
            RendererPageModuleReactionEvent::DynamicModuleEvaluationFulfilled {
                import_owner,
                reaction_id,
            }
            | RendererPageModuleReactionEvent::DynamicModuleEvaluationRejected {
                import_owner,
                reaction_id,
                ..
            } => (*import_owner, *reaction_id),
            RendererPageModuleReactionEvent::DocumentModuleScriptEvaluationFulfilled { .. }
            | RendererPageModuleReactionEvent::DocumentModuleScriptEvaluationRejected { .. }
            | RendererPageModuleReactionEvent::ChildParserModuleEvaluationFulfilled { .. }
            | RendererPageModuleReactionEvent::ChildParserModuleEvaluationRejected { .. } => {
                return;
            }
        };
        let reaction_claimed = self
            .document_runtime
            .take_native_dynamic_module_evaluation_reaction(reaction_id, import_owner)
            .is_some();
        self.record_runtime_warning(format_args!(
            "ignored stale module reaction: owner={import_owner:?} reaction_id={reaction_id} reaction_claimed={reaction_claimed}"
        ));
    }
    #[cfg(test)]
    pub(crate) fn has_page_module_reaction_for_executor_test(&self) -> bool {
        self._page_task_residence_for_executor_test
            .as_ref()
            .expect("module-reaction executor fixture must retain its production Page source")
            .task_sources()
            .has_module_reaction_for_executor_test()
    }
    /// Publish a document-module reaction without reserving a local reaction
    /// record. This is only for Page authorization tests that need a concrete
    /// spent or stale source ticket.
    #[cfg(test)]
    pub(crate) fn queue_missing_document_module_reaction_for_test(&mut self, reaction_id: u64) {
        let document_owner = self
            .current_main_document_task_owner()
            .expect("module reaction fixture requires a current main Document owner");
        self._context_host
            .borrow_mut()
            .queue_document_module_script_evaluation_fulfilled(document_owner, reaction_id);
    }
    /// Apply only the body of one production module-reaction task in a
    /// low-level ScriptVm semantic fixture.
    ///
    /// This helper deliberately does not model selected-task completion.
    /// Page-root admission, task-end checkpoint ownership, and scheduler
    /// liveness are covered through the PageVm selected-dispatcher test driver.
    #[cfg(test)]
    pub(crate) fn run_page_module_reaction_body_for_test(
        &mut self,
    ) -> Result<Option<crate::page_task_queue::PageModuleReactionTargetEffect>> {
        let source = self
            ._page_task_residence_for_executor_test
            .as_ref()
            .expect("module-reaction executor fixture must retain its production Page source")
            .task_sources();
        let Some(task) = source.take_module_reaction_for_executor_test() else {
            return Ok(None);
        };
        if self.module_reaction_target_is_current(task.owner().target()) {
            let application = self.apply_current_page_module_reaction(
                crate::runtime::AuthorizedCurrentPageModuleReaction::new_for_executor_test(task),
            )?;
            return Ok(Some(match application {
                PageModuleReactionApplication::Applied { current_effect, .. } => {
                    crate::page_task_queue::PageModuleReactionTargetEffect::AppliedToCurrentOwner(
                        current_effect,
                    )
                }
                PageModuleReactionApplication::NoPendingReaction => {
                    crate::page_task_queue::PageModuleReactionTargetEffect::DiscardedMissingReaction
                }
            }));
        }
        self.discard_stale_page_module_reaction(&task.into_event());
        Ok(Some(
            crate::page_task_queue::PageModuleReactionTargetEffect::IgnoredStaleOwner,
        ))
    }
    pub(super) fn apply_native_module_script_evaluation_fulfilled(
        &mut self,
        reaction_id: u64,
    ) -> Option<PageModuleReactionFollowup> {
        let update = self.mark_module_evaluation_reaction_fulfilled_for_owner(reaction_id)?;
        let (root_entry, followup) = match update {
            DocumentModuleReactionUpdate::ParserOwned(update) => (
                update.root_entry(),
                PageModuleReactionFollowup::main_parser_owned_evaluations(
                    update.queued_ready_action_count(),
                ),
            ),
            DocumentModuleReactionUpdate::RuntimeOwned(update) => (
                update.root_entry,
                PageModuleReactionFollowup::RuntimeOwnedModuleContinuation,
            ),
        };
        self.document_runtime
            .mark_native_module_evaluated(root_entry);
        Some(followup)
    }
    pub(super) fn apply_native_module_script_evaluation_rejected(
        &mut self,
        reaction_id: u64,
        reason: String,
        error_value: Option<ScriptErrorValue>,
    ) -> Option<PageModuleReactionFollowup> {
        let update = self.mark_module_evaluation_reaction_rejected_for_owner(
            reaction_id,
            reason,
            error_value,
        )?;
        Some(match update {
            DocumentModuleReactionUpdate::ParserOwned(update) => {
                PageModuleReactionFollowup::main_parser_owned_evaluations(
                    update.queued_ready_action_count(),
                )
            }
            DocumentModuleReactionUpdate::RuntimeOwned(_) => {
                PageModuleReactionFollowup::RuntimeOwnedModuleContinuation
            }
        })
    }
    pub(crate) fn apply_native_dynamic_module_evaluation_fulfilled(
        &mut self,
        import_owner: crate::module_runtime::DynamicModuleImportOwner,
        reaction_id: u64,
    ) -> std::result::Result<bool, ModuleLoadError> {
        let Some(reaction) = self
            .document_runtime
            .take_native_dynamic_module_evaluation_reaction(reaction_id, import_owner)
        else {
            return Ok(false);
        };
        let (request, target) = reaction.into_parts();
        let child_owner_parts = request
            .owner()
            .child_parts()
            .map(|(_child_handle, task_owner, realm_id)| (task_owner.document_owner(), realm_id));
        if let Some((owner, realm_id)) = child_owner_parts {
            self.mark_child_native_dynamic_module_evaluated(owner, realm_id, target.root_entry());
        } else {
            self.document_runtime
                .mark_native_module_evaluated(target.root_entry());
        }
        // Commit the exact owner's module-map state before resolving the user
        // Promise. Its reactions may replace the Document when the selected
        // task dispatcher performs the task-end checkpoint.
        self.resolve_native_dynamic_module_import_selected_task_body(request, &target)?;
        Ok(true)
    }
    pub(crate) fn apply_native_dynamic_module_evaluation_rejected(
        &mut self,
        import_owner: crate::module_runtime::DynamicModuleImportOwner,
        reaction_id: u64,
        reason: v8::Global<v8::Value>,
    ) -> std::result::Result<bool, ModuleLoadError> {
        let Some(reaction) = self
            .document_runtime
            .take_native_dynamic_module_evaluation_reaction(reaction_id, import_owner)
        else {
            return Ok(false);
        };
        let (request, _) = reaction.into_parts();
        self.reject_native_dynamic_module_import_reaction_body(request, reason)?;
        Ok(true)
    }
}
