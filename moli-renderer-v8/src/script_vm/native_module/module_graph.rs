use std::sync::Arc;

use super::*;

impl ScriptVm {
    pub(crate) fn compile_native_module_record(
        &mut self,
        key: ModuleMapKey,
        source: &ModuleSource,
        source_url: &Url,
        fetch_metadata: &crate::module_runtime::ModuleFetchMetadata,
    ) -> std::result::Result<(ModuleRecordEntry, ModuleIdentityHash), ModuleLoadError> {
        let context_ptr = self.native_module_default_context_ptr();
        self.compile_native_module_record_in_context(
            context_ptr,
            key,
            source,
            source_url,
            fetch_metadata,
        )
    }
    pub(crate) fn compile_native_module_record_for_frame_realm(
        &mut self,
        realm_id: FrameRealmId,
        key: ModuleMapKey,
        source: &ModuleSource,
        source_url: &Url,
        fetch_metadata: &crate::module_runtime::ModuleFetchMetadata,
    ) -> std::result::Result<(ModuleRecordEntry, ModuleIdentityHash), ModuleLoadError> {
        let context_ptr = self.frame_realm_context_ptr(realm_id).map_err(|error| {
            ModuleLoadError::new(
                ModuleLoadStage::Compile,
                format!("failed to find FrameRealm {realm_id:?} for module compile: {error}"),
            )
        })?;
        self.compile_native_module_record_in_context(
            context_ptr,
            key,
            source,
            source_url,
            fetch_metadata,
        )
    }
    pub(super) fn native_module_default_context_ptr(&self) -> *const v8::Global<v8::Context> {
        &self.page_default_runtime.context as *const _
    }
    pub(super) fn compile_native_module_record_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        key: ModuleMapKey,
        source: &ModuleSource,
        source_url: &Url,
        fetch_metadata: &crate::module_runtime::ModuleFetchMetadata,
    ) -> std::result::Result<(ModuleRecordEntry, ModuleIdentityHash), ModuleLoadError> {
        match key.kind() {
            ModuleKind::JavaScript => {
                let origin = source.origin();
                let Some(source) = source.text_source() else {
                    return Err(ModuleLoadError::new(
                        ModuleLoadStage::Compile,
                        format!("javascript module `{source_url}` did not retain text source"),
                    ));
                };
                self.compile_javascript_module_record_in_context(
                    context_ptr,
                    key,
                    source,
                    source_url,
                    fetch_metadata,
                    origin,
                )
            }
            ModuleKind::Json | ModuleKind::Css => {
                let ModuleSource::Text(source) = source else {
                    return Err(ModuleLoadError::new(
                        ModuleLoadStage::Compile,
                        format!("synthetic text module `{source_url}` did not retain text source"),
                    ));
                };
                self.compile_synthetic_module_record_in_context(
                    context_ptr,
                    key,
                    Arc::clone(source),
                    source_url,
                )
            }
            ModuleKind::WebAssembly => {
                let Some(bytes) = source.binary_source() else {
                    return Err(ModuleLoadError::new(
                        ModuleLoadStage::Compile,
                        format!("WebAssembly module `{source_url}` did not retain binary source"),
                    ));
                };
                self.compile_wasm_module_record_in_context(context_ptr, key, bytes, source_url)
            }
            ModuleKind::ModulePreloadText => Err(ModuleLoadError::new(
                ModuleLoadStage::Compile,
                format!("modulepreload text `{source_url}` is not a module graph record"),
            )),
        }
    }
    pub(super) fn compile_javascript_module_record_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        key: ModuleMapKey,
        source: &str,
        source_url: &Url,
        fetch_metadata: &crate::module_runtime::ModuleFetchMetadata,
        source_origin: Option<&crate::document_module_graph::ModuleSourceOrigin>,
    ) -> std::result::Result<(ModuleRecordEntry, ModuleIdentityHash), ModuleLoadError> {
        let mut exception_id = None;
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let try_catch = pin!(v8::TryCatch::new(scope));
                let mut scope = try_catch.init();

                let source_string = v8_string(&scope, source)
                    .ok_or_else(|| anyhow::anyhow!("failed to allocate v8 module source string"))?;
                let origin = create_module_script_origin(
                    &mut scope,
                    source_url.as_str(),
                    fetch_metadata,
                    source_origin,
                );
                let mut compiler_source =
                    v8::script_compiler::Source::new(source_string, Some(&origin));
                let module = v8::script_compiler::compile_module(&scope, &mut compiler_source)
                    .ok_or_else(|| {
                        if let Some(exception) = scope.exception() {
                            match retain_module_exception(&mut scope, exception) {
                                Ok(id) => exception_id = Some(id),
                                Err(error) => return error,
                            }
                        }
                        let exception = scope
                            .exception()
                            .and_then(|exception| exception.to_detail_string(&scope))
                            .map(|message| message.to_rust_string_lossy(&scope))
                            .or_else(|| {
                                scope.message().map(|message| {
                                    message.get(&scope).to_rust_string_lossy(&scope)
                                })
                            })
                            .unwrap_or_else(|| {
                                format!(
                                    "unknown compile exception (caught={}, can_continue={}, terminated={})",
                                    scope.has_caught(),
                                    scope.can_continue(),
                                    scope.has_terminated()
                                )
                            });
                        anyhow::anyhow!(
                            "v8 failed to compile native module `{source_url}`: {}",
                            exception
                        )
                    })?;
                let requests = collect_module_requests(&mut scope, module)?;
                let identity = module_identity_hash_from_v8_module(module);
                let compiled_module = v8::Global::new(scope.as_ref(), module);
                Ok((
                    ModuleRecordEntry::new(key, compiled_module, requests),
                    identity,
                ))
            })
            .map_err(|error| {
                let message = error.to_string();
                let mut load_error = ModuleLoadError::new(ModuleLoadStage::Compile, message.clone());
                if let Some(exception_id) = exception_id {
                    load_error = load_error.with_exception_id(exception_id);
                }
                if message.starts_with("v8 failed to compile WebAssembly module `") {
                    load_error
                        .with_error_constructor(ScriptErrorConstructorKind::WebAssemblyCompileError)
                } else {
                    load_error.with_error_constructor(ScriptErrorConstructorKind::SyntaxError)
                }
            })
    }
    pub(super) fn compile_wasm_module_record_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        key: ModuleMapKey,
        bytes: &[u8],
        source_url: &Url,
    ) -> std::result::Result<(ModuleRecordEntry, ModuleIdentityHash), ModuleLoadError> {
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let try_catch = pin!(v8::TryCatch::new(scope));
                let scope = try_catch.init();
                let prepared = prepare_wasm_module_record(&scope, bytes)?.ok_or_else(|| {
                    let exception = v8_exception_message_or(
                        &scope,
                        scope.exception(),
                        "unknown wasm compile exception",
                    );
                    anyhow::anyhow!(
                        "v8 failed to compile WebAssembly module `{source_url}`: {exception}"
                    )
                })?;
                let requests = if prepared.has_reserved_name_link_error {
                    Vec::new()
                } else {
                    wasm_module_requests_for_imports(prepared.record.imports())
                };
                let export_name_refs = prepared
                    .record
                    .exports()
                    .iter()
                    .map(|export| export.name())
                    .collect::<Vec<_>>();
                let module_name = v8_string(&scope, source_url.as_str()).ok_or_else(|| {
                    anyhow::anyhow!("failed to allocate WebAssembly synthetic module name")
                })?;
                let export_names = export_name_refs
                    .iter()
                    .map(|name| {
                        v8_string(&scope, name)
                            .ok_or_else(|| anyhow::anyhow!("failed to allocate wasm export name"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let module = v8::Module::create_synthetic_module(
                    &scope,
                    module_name,
                    &export_names,
                    wasm_synthetic_module_evaluation_steps,
                );
                let identity = module_identity_hash_from_v8_module(module);
                let compiled_module = v8::Global::new(scope.as_ref(), module);
                Ok((
                    ModuleRecordEntry::new_with_wasm_module(
                        key,
                        compiled_module,
                        requests,
                        prepared.record,
                    ),
                    identity,
                ))
            })
            .map_err(|error| {
                ModuleLoadError::new(ModuleLoadStage::Compile, error.to_string())
                    .with_error_constructor(ScriptErrorConstructorKind::WebAssemblyCompileError)
            })
    }
    pub(super) fn compile_synthetic_module_record_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        key: ModuleMapKey,
        source: Arc<str>,
        source_url: &Url,
    ) -> std::result::Result<(ModuleRecordEntry, ModuleIdentityHash), ModuleLoadError> {
        let mut exception_id = None;
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let try_catch = pin!(v8::TryCatch::new(scope));
                let mut scope = try_catch.init();

                let value = if key.kind() == ModuleKind::Json {
                    let value = crate::module_runtime::parse_json_module(
                        &mut scope,
                        &source,
                        source_url.as_str(),
                    )
                    .ok_or_else(|| {
                        if let Some(exception) = scope.exception() {
                            match retain_module_exception(&mut scope, exception) {
                                Ok(id) => exception_id = Some(id),
                                Err(error) => return error,
                            }
                        }
                        let message = scope
                            .message()
                            .map(|message| message.get(&scope).to_rust_string_lossy(&scope))
                            .unwrap_or_else(|| "failed to parse JSON module".to_owned());
                        anyhow::anyhow!("{message}")
                    })?;
                    crate::module_runtime::SyntheticTextModuleValue::Json(v8::Global::new(
                        &scope, value,
                    ))
                } else {
                    crate::module_runtime::SyntheticTextModuleValue::Css(source)
                };
                let module_name = v8_string(&scope, source_url.as_str()).ok_or_else(|| {
                    anyhow::anyhow!("failed to allocate v8 synthetic module name")
                })?;
                let default_export = v8_string(&scope, "default")
                    .ok_or_else(|| anyhow::anyhow!("failed to allocate synthetic export name"))?;
                let module = v8::Module::create_synthetic_module(
                    &scope,
                    module_name,
                    &[default_export],
                    synthetic_text_module_evaluation_steps,
                );
                let identity = module_identity_hash_from_v8_module(module);
                let evaluation_source = crate::module_runtime::SyntheticTextModuleSource::register(
                    &mut scope,
                    module,
                    key.clone(),
                    value,
                );
                let compiled_module = v8::Global::new(scope.as_ref(), module);
                let entry = ModuleRecordEntry::new(key, compiled_module, Vec::new())
                    .with_synthetic_text_module_source(evaluation_source);
                Ok((entry, identity))
            })
            .map_err(|error| {
                let error = ModuleLoadError::new(ModuleLoadStage::Compile, error.to_string());
                if let Some(id) = exception_id {
                    error
                        .with_exception_id(id)
                        .with_error_constructor(ScriptErrorConstructorKind::SyntaxError)
                } else {
                    error
                }
            })
    }
    pub(crate) fn instantiate_native_module_graph(
        &mut self,
        graph: &crate::module_runtime::ModuleGraphHandle,
    ) -> std::result::Result<(), ModuleLoadError> {
        let context_ptr = self.native_module_default_context_ptr();
        self.instantiate_native_module_graph_in_context(context_ptr, graph)
    }
    pub(super) fn instantiate_native_module_graph_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        graph: &crate::module_runtime::ModuleGraphHandle,
    ) -> std::result::Result<(), ModuleLoadError> {
        let root_entry = graph.root_entry;
        let graph_urls = graph
            .entries
            .iter()
            .map(|entry_id| self.document_runtime.native_module_entry_url(*entry_id))
            .collect::<Vec<_>>();
        let has_wasm_entry = graph.entries.iter().any(|entry_id| {
            self.document_runtime
                .native_module_wasm_record(*entry_id)
                .is_some()
        });
        let document_modulator = self.document_runtime.native_document_modulator_ptr();
        let root_module = self
            .document_runtime
            .native_compiled_module(root_entry)
            .ok_or_else(|| {
                ModuleLoadError::new(
                    ModuleLoadStage::Instantiate,
                    format!("native root module entry {root_entry:?} is not compiled"),
                )
            })?;
        let mut caught_error_constructor = None;
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let try_catch = pin!(v8::TryCatch::new(scope));
                let mut scope = try_catch.init();

                let root_module = v8::Local::new(&scope, &root_module);
                if has_wasm_entry {
                    unsafe { &*document_modulator }
                        .register_wasm_evaluation_graph(&mut scope, graph);
                }
                let _resolver_scope = ResolverScopeGuard::new(document_modulator);
                match root_module.instantiate_module2(
                    &scope,
                    resolve_static_module_callback,
                    resolve_static_source_callback,
                ) {
                    Some(true) => Ok(()),
                    Some(false) => Err(anyhow::anyhow!("v8 reported module instantiate failure")),
                    None => {
                        let exception = match scope.exception() {
                            Some(exception) => {
                                caught_error_constructor =
                                    script_error_constructor_kind_from_value(&mut scope, exception);
                                exception
                                    .to_string(&scope)
                                    .map(|message| message.to_rust_string_lossy(&scope))
                                    .unwrap_or_else(|| "unknown instantiate exception".to_owned())
                            }
                            None => "unknown instantiate exception".to_owned(),
                        };
                        Err(anyhow::anyhow!(
                            "{}",
                            canonical_native_module_instantiate_error(&exception, &graph_urls)
                        ))
                    }
                }
            })
            .map_err(|error| {
                native_module_instantiate_load_error(
                    error.to_string(),
                    caught_error_constructor,
                    has_wasm_entry,
                )
            })?;
        self.document_runtime
            .mark_native_module_instantiated(root_entry);
        Ok(())
    }
    pub(crate) fn evaluate_native_module_graph(
        &mut self,
        root_entry: crate::module_runtime::ModuleEntryId,
    ) -> std::result::Result<Option<v8::Global<v8::Promise>>, ModuleLoadError> {
        let context_ptr = self.native_module_default_context_ptr();
        self.evaluate_native_module_graph_with_owner_in_context(
            context_ptr,
            root_entry,
            NativeModuleEvaluationOwner::Script,
        )
        .map(|result| result.promise)
    }
    pub(crate) fn evaluate_native_dynamic_module_graph(
        &mut self,
        root_entry: crate::module_runtime::ModuleEntryId,
    ) -> std::result::Result<NativeDynamicModuleEvaluation, ModuleLoadError> {
        let context_ptr = self.native_module_default_context_ptr();
        self.evaluate_native_module_graph_with_owner_in_context(
            context_ptr,
            root_entry,
            NativeModuleEvaluationOwner::DynamicImport,
        )
        .map(|result| NativeDynamicModuleEvaluation {
            target: DynamicModuleEvaluationTarget::new(root_entry, result.module),
            promise: result.promise,
        })
    }
    pub(super) fn start_native_dynamic_module_import_evaluation(
        &mut self,
        graph: ModuleGraphHandle,
    ) -> std::result::Result<DynamicModuleImportEvaluationStart, ModuleLoadError> {
        match self
            .document_runtime
            .native_module_entry_state(graph.root_entry)
        {
            ModuleMapEntryState::Compiled => {
                self.instantiate_native_module_graph(&graph)?;
                self.start_native_module_graph_evaluation(graph.root_entry)
            }
            ModuleMapEntryState::Instantiated | ModuleMapEntryState::Evaluating => {
                self.start_native_module_graph_evaluation(graph.root_entry)
            }
            ModuleMapEntryState::Evaluated => {
                let module = self
                    .document_runtime
                    .native_compiled_module(graph.root_entry)
                    .ok_or_else(|| {
                        ModuleLoadError::new(
                            ModuleLoadStage::Evaluate,
                            "native dynamic import root was evaluated without a compiled module",
                        )
                    })?;
                Ok(DynamicModuleImportEvaluationStart::Completed(
                    DynamicModuleEvaluationTarget::new(graph.root_entry, module),
                ))
            }
            ModuleMapEntryState::Fetching
            | ModuleMapEntryState::Fetched
            | ModuleMapEntryState::Failed => Err(ModuleLoadError::new(
                ModuleLoadStage::Evaluate,
                "native dynamic import root was not ready to evaluate",
            )),
        }
    }
    pub(super) fn start_native_module_graph_evaluation(
        &mut self,
        root_entry: crate::module_runtime::ModuleEntryId,
    ) -> std::result::Result<DynamicModuleImportEvaluationStart, ModuleLoadError> {
        let evaluation = self.evaluate_native_dynamic_module_graph(root_entry)?;
        let (target, promise) = evaluation.into_parts();
        let Some(promise) = promise else {
            return Ok(DynamicModuleImportEvaluationStart::Completed(target));
        };
        Ok(DynamicModuleImportEvaluationStart::Pending { target, promise })
    }
    pub(super) fn evaluate_native_module_graph_with_owner_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        root_entry: crate::module_runtime::ModuleEntryId,
        owner: NativeModuleEvaluationOwner,
    ) -> std::result::Result<NativeModuleEvaluationResult, ModuleLoadError> {
        let root_module = self
            .document_runtime
            .native_compiled_module(root_entry)
            .ok_or_else(|| {
                ModuleLoadError::new(
                    ModuleLoadStage::Evaluate,
                    format!("native root module entry {root_entry:?} is not compiled"),
                )
            })?;
        let document_owner_before_evaluation =
            self.current_main_document_task_owner().ok_or_else(|| {
                ModuleLoadError::new(
                    ModuleLoadStage::Evaluate,
                    "native module evaluation has no current main Document owner",
                )
            })?;
        self.document_runtime
            .mark_native_module_evaluating(root_entry);
        // currentScript is null for modules. The execute-script-element guard
        // belongs to its Document and lasts through the cleanup checkpoint,
        // not the lifetime of the module's evaluation promise.
        let _ignore_destructive_writes =
            (owner == NativeModuleEvaluationOwner::Script).then(|| {
                self.document_runtime
                    .enter_ignore_destructive_writes(self.document_runtime.document_handle())
            });
        let promise = self
            .renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let try_catch = pin!(v8::TryCatch::new(scope));
                let mut scope = try_catch.init();

                let root_module = v8::Local::new(&scope, &root_module);
                let Some(value) = crate::script_execution::evaluate_module(&mut scope, root_module)
                else {
                    let error = scope
                        .exception()
                        .map(|exception| {
                            native_module_evaluation_exception_error(
                                &mut scope,
                                exception,
                                "v8 failed to evaluate native module graph",
                            )
                        })
                        .unwrap_or_else(|| {
                            ModuleLoadError::new(
                                ModuleLoadStage::Evaluate,
                                "v8 failed to evaluate native module graph: unknown exception",
                            )
                        });
                    return Ok(Err(error));
                };
                let promise = v8::Local::<v8::Promise>::try_from(value).ok();
                if owner == NativeModuleEvaluationOwner::DynamicImport
                    && let Some(promise) = promise
                {
                    // Dynamic import owns the module-evaluation promise itself.
                    // Return it before a checkpoint so the import rejection
                    // handler is attached before V8 can report an unhandled
                    // rejection for synchronously rejected module evaluation.
                    match promise.state() {
                        v8::PromiseState::Fulfilled => return Ok(Ok(None)),
                        v8::PromiseState::Rejected | v8::PromiseState::Pending => {
                            let promise = v8::Global::new(scope.as_ref(), promise);
                            return Ok(Ok(Some(promise)));
                        }
                    }
                }
                // Script-element evaluation consumes rejection itself, either
                // below or through its retained TLA continuation. Claim that
                // responsibility before cleanup can notify rejected promises.
                // Dynamic import returned above and owns its own reaction.
                if let Some(promise) = promise {
                    promise.mark_as_handled();
                }
                if let Err(error) = Self::perform_microtask_checkpoints(&mut scope, None) {
                    return Ok(Err(ModuleLoadError::new(
                        ModuleLoadStage::Evaluate,
                        error.to_string(),
                    )));
                }
                if root_module.get_status() == v8::ModuleStatus::Errored {
                    let exception = root_module.get_exception();
                    return Ok(Err(native_module_evaluation_exception_error(
                        &mut scope,
                        exception,
                        "native module graph evaluation rejected",
                    )));
                }
                if let Some(promise) = promise {
                    match promise.state() {
                        v8::PromiseState::Fulfilled => return Ok(Ok(None)),
                        v8::PromiseState::Rejected => {
                            let result = promise.result(&scope);
                            return Ok(Err(native_module_evaluation_exception_error(
                                &mut scope,
                                result,
                                "native module graph evaluation rejected",
                            )));
                        }
                        v8::PromiseState::Pending => {
                            let promise = v8::Global::new(scope.as_ref(), promise);
                            return Ok(Ok(Some(promise)));
                        }
                    }
                }
                Ok(Ok(None))
            })
            .map_err(|error| {
                ModuleLoadError::new(ModuleLoadStage::Evaluate, error.to_string())
            })??;
        if promise.is_none()
            && self.current_main_document_task_owner() == Some(document_owner_before_evaluation)
        {
            self.document_runtime
                .mark_native_module_evaluated(root_entry);
        }
        Ok(NativeModuleEvaluationResult {
            module: root_module,
            promise,
        })
    }
}
