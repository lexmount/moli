//! Dynamic-import Promise settlement performed inside a selected Page task.
//!
//! These helpers enter the exact request realm and settle only the Promise
//! body. They deliberately do not run a microtask checkpoint. The stable
//! `ModuleReaction` and child `DynamicImportOwnerAction` dispatchers own their
//! task-end checkpoint, so user reactions cannot run midway through either
//! domain transition.

use std::pin::pin;

use super::*;

impl ScriptVm {
    pub(super) fn resolve_native_dynamic_module_source_import_selected_task_body(
        &mut self,
        request: PendingDynamicModuleImport,
        root_entry: ModuleEntryId,
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
            self.reject_native_dynamic_module_import_with_error_selected_task_body(
                request, &error,
            )?;
            return Ok(NativeDynamicModuleSourceImportResolution::Rejected);
        };
        self.resolve_native_dynamic_module_source_import_with_wasm_record_selected_task_body(
            request,
            wasm_record,
        )
    }

    pub(super) fn resolve_native_dynamic_module_import_selected_task_body(
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
                Ok(())
            })
            .map_err(|error: anyhow::Error| {
                ModuleLoadError::new(ModuleLoadStage::Evaluate, error.to_string())
            })
    }

    pub(super) fn resolve_native_dynamic_module_source_import_with_wasm_record_selected_task_body(
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
                    return Ok(NativeDynamicModuleSourceImportResolution::Rejected);
                };
                let _ = resolver.resolve(scope, source.into());
                Ok(NativeDynamicModuleSourceImportResolution::Resolved)
            })
            .map_err(|error: anyhow::Error| {
                ModuleLoadError::new(ModuleLoadStage::Evaluate, error.to_string())
            })
    }

    pub(super) fn reject_native_dynamic_module_import_with_error_selected_task_body(
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
                Ok(())
            })
            .map_err(|error: anyhow::Error| {
                ModuleLoadError::new(ModuleLoadStage::Evaluate, error.to_string())
            })
    }

    pub(super) fn reject_native_dynamic_module_import_reaction_body(
        &mut self,
        request: PendingDynamicModuleImport,
        reason: v8::Global<v8::Value>,
    ) -> std::result::Result<(), ModuleLoadError> {
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, request.context());
                let scope = &mut v8::ContextScope::new(scope, context);
                let resolver = v8::Local::new(scope, request.resolver());
                let reason = v8::Local::new(scope, reason);
                let _ = resolver.reject(scope, reason);
                Ok(())
            })
            .map_err(|error: anyhow::Error| {
                ModuleLoadError::new(ModuleLoadStage::Evaluate, error.to_string())
            })
    }
}
