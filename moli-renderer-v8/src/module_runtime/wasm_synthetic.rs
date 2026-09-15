use std::{cell::RefCell, collections::HashSet};

use crate::context_bootstrap::{
    ORIGINAL_WEBASSEMBLY_GLOBAL_VALUE_GETTER_SLOT, ORIGINAL_WEBASSEMBLY_INSTANCE_CONSTRUCTOR_SLOT,
    ORIGINAL_WEBASSEMBLY_LINK_ERROR_CONSTRUCTOR_SLOT,
};
use crate::util::{get_private_value, throw_type_error, v8_string, v8str};
use crate::wasm_module_support::{WasmNamespaceExportMode, reserved_wasm_name_link_error};
use moli_webapi_declare::ObjectLiteralDeclaration;

use super::{
    ModuleIdentityHash, WasmImportRecord, WasmModuleRecord, module_identity_hash_from_v8_module,
};

thread_local! {
    static EVALUATING_WASM_SYNTHETIC_MODULES: RefCell<Vec<ModuleIdentityHash>> =
        const { RefCell::new(Vec::new()) };
}

pub(crate) struct WasmSyntheticModuleEvaluationGuard {
    identity: ModuleIdentityHash,
}

impl WasmSyntheticModuleEvaluationGuard {
    fn enter(module: v8::Local<'_, v8::Module>) -> Option<Self> {
        let identity = module_identity_hash_from_v8_module(module);
        EVALUATING_WASM_SYNTHETIC_MODULES.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.contains(&identity) {
                return None;
            }
            stack.push(identity);
            Some(Self { identity })
        })
    }
}

impl Drop for WasmSyntheticModuleEvaluationGuard {
    fn drop(&mut self) {
        EVALUATING_WASM_SYNTHETIC_MODULES.with(|stack| {
            let mut stack = stack.borrow_mut();
            if let Some(index) = stack
                .iter()
                .rposition(|identity| identity == &self.identity)
            {
                stack.remove(index);
            }
        });
    }
}

pub(crate) fn evaluate_wasm_synthetic_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: v8::Local<'s, v8::Module>,
    wasm_record: &WasmModuleRecord,
    import_value: impl FnMut(
        &mut v8::PinScope<'s, '_>,
        &WasmImportRecord,
    ) -> Option<v8::Local<'s, v8::Value>>,
) -> Option<v8::Local<'s, v8::Value>> {
    let _guard = begin_wasm_synthetic_module_evaluation(scope, module, wasm_record)?;
    let imports = build_wasm_synthetic_import_object(scope, wasm_record, import_value)?;
    finish_wasm_synthetic_module_evaluation(scope, module, wasm_record, imports)
}

fn begin_wasm_synthetic_module_evaluation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: v8::Local<'s, v8::Module>,
    wasm_record: &WasmModuleRecord,
) -> Option<WasmSyntheticModuleEvaluationGuard> {
    let guard = match WasmSyntheticModuleEvaluationGuard::enter(module) {
        Some(guard) => guard,
        None => {
            throw_wasm_synthetic_module_error(
                scope,
                "cyclic WebAssembly module evaluation is not supported yet",
            );
            return None;
        }
    };
    if let Some(link_error) =
        reserved_wasm_name_link_error(wasm_record.imports(), wasm_record.exports())
    {
        throw_wasm_link_error(scope, link_error.message());
        return None;
    }
    Some(guard)
}

fn is_wasm_synthetic_module_evaluating(module: v8::Local<'_, v8::Module>) -> bool {
    let identity = module_identity_hash_from_v8_module(module);
    EVALUATING_WASM_SYNTHETIC_MODULES.with(|stack| stack.borrow().contains(&identity))
}

fn wasm_dependency_graph_reaches_evaluating_synthetic_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: v8::Local<'s, v8::Module>,
    seen: &mut HashSet<ModuleIdentityHash>,
    dependencies_for: &mut impl FnMut(
        &mut v8::PinScope<'s, '_>,
        v8::Local<'s, v8::Module>,
    ) -> Option<Vec<v8::Global<v8::Module>>>,
) -> Option<bool> {
    if is_wasm_synthetic_module_evaluating(module) {
        return Some(true);
    }
    if !seen.insert(module_identity_hash_from_v8_module(module)) {
        return Some(false);
    }
    for dependency in dependencies_for(scope, module)? {
        let dependency = v8::Local::new(scope, &dependency);
        if wasm_dependency_graph_reaches_evaluating_synthetic_module(
            scope,
            dependency,
            seen,
            dependencies_for,
        )? {
            return Some(true);
        }
    }
    Some(false)
}

fn finish_wasm_synthetic_module_evaluation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: v8::Local<'s, v8::Module>,
    wasm_record: &WasmModuleRecord,
    imports: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let wasm_module = wasm_record.source_module(scope)?;
    let instance = construct_webassembly_instance(scope, wasm_module, imports)?;
    wasm_record.set_instance(scope, instance);
    let exports = wasm_instance_exports(scope, instance)?;
    for export in wasm_record.exports() {
        let Some(export_name) = v8_string(scope, export.name()) else {
            return throw_wasm_synthetic_module_error(scope, "failed to allocate wasm export name");
        };
        let mode = export.namespace_export_mode();
        let value = match mode {
            WasmNamespaceExportMode::UninitializedGlobal => {
                if module
                    .set_synthetic_module_export_uninitialized(scope, export_name)
                    .is_none_or(|ok| !ok)
                {
                    return throw_wasm_synthetic_module_error(
                        scope,
                        "failed to set wasm v128 synthetic export",
                    );
                }
                continue;
            }
            WasmNamespaceExportMode::RawValue | WasmNamespaceExportMode::GlobalSnapshotValue => {
                let Some(value) = exports.get(scope, export_name.into()) else {
                    return throw_wasm_synthetic_module_error(scope, "missing wasm export value");
                };
                wasm_export_value_for_module_namespace(scope, mode, value)?
            }
        };
        if module
            .set_synthetic_module_export(scope, export_name, value)
            .is_none_or(|ok| !ok)
        {
            return throw_wasm_synthetic_module_error(scope, "failed to set wasm synthetic export");
        }
    }
    Some(v8::undefined(scope).into())
}

fn build_wasm_synthetic_import_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wasm_record: &WasmModuleRecord,
    mut import_value: impl FnMut(
        &mut v8::PinScope<'s, '_>,
        &WasmImportRecord,
    ) -> Option<v8::Local<'s, v8::Value>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let imports = ObjectLiteralDeclaration::bind(scope).into_object();
    for import in wasm_record.imports() {
        let module_imports = wasm_module_import_object(scope, imports, import.module())?;
        let value = import_value(scope, import)?;
        let Some(name) = v8_string(scope, import.name()) else {
            throw_wasm_synthetic_module_error(scope, "failed to allocate wasm import name");
            return None;
        };
        if module_imports
            .set(scope, name.into(), value)
            .is_none_or(|ok| !ok)
        {
            throw_wasm_synthetic_module_error(scope, "failed to set wasm import value");
            return None;
        }
    }
    Some(imports)
}

fn wasm_module_import_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    imports: v8::Local<'s, v8::Object>,
    module_specifier: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let Some(module_key) = v8_string(scope, module_specifier) else {
        throw_wasm_synthetic_module_error(scope, "failed to allocate wasm import module name");
        return None;
    };
    if let Some(value) = imports.get(scope, module_key.into())
        && let Ok(module_imports) = v8::Local::<v8::Object>::try_from(value)
    {
        return Some(module_imports);
    }
    let module_imports = ObjectLiteralDeclaration::bind(scope).into_object();
    if imports
        .set(scope, module_key.into(), module_imports.into())
        .is_none_or(|ok| !ok)
    {
        throw_wasm_synthetic_module_error(scope, "failed to set wasm import module object");
        return None;
    }
    Some(module_imports)
}

fn wasm_record_raw_export_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wasm_record: &WasmModuleRecord,
    export_name: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let instance = wasm_record.instance(scope)?;
    let exports = wasm_instance_exports(scope, instance)?;
    let name = v8_string(scope, export_name)?;
    exports.get(scope, name.into())
}

pub(crate) fn wasm_dependency_export_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    dependency: v8::Local<'s, v8::Module>,
    wasm_record: Option<&WasmModuleRecord>,
    export_name: &str,
    allocation_error: &str,
    missing_error: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    if let Some(wasm_record) = wasm_record
        && let Some(value) = wasm_record_raw_export_value(scope, wasm_record, export_name)
    {
        return Some(value);
    }
    let namespace = dependency.get_module_namespace();
    let Some(namespace) = v8::Local::<v8::Object>::try_from(namespace).ok() else {
        return throw_wasm_link_error(scope, missing_error);
    };
    let Some(name) = v8_string(scope, export_name) else {
        return throw_wasm_link_error(scope, allocation_error);
    };
    if let Some(value) = namespace.get(scope, name.into()) {
        return Some(value);
    }
    throw_wasm_link_error(scope, missing_error)
}

enum WasmDependencyModuleReadiness {
    NeedsEvaluation,
    Ready,
}

#[derive(Clone, Copy)]
pub(crate) struct WasmDependencyModuleMessages {
    pub(crate) instantiating: &'static str,
    pub(crate) already_failed: &'static str,
    pub(crate) evaluation_failed: &'static str,
    pub(crate) not_instantiated: &'static str,
    pub(crate) cyclic: &'static str,
    pub(crate) graph_unavailable: &'static str,
    pub(crate) pending: &'static str,
}

fn wasm_dependency_module_readiness<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: v8::Local<'s, v8::Module>,
    instantiate: impl FnOnce(&mut v8::PinScope<'s, '_>, v8::Local<'s, v8::Module>) -> Option<()>,
    messages: WasmDependencyModuleMessages,
) -> Option<WasmDependencyModuleReadiness> {
    match module.get_status() {
        v8::ModuleStatus::Uninstantiated => instantiate(scope, module)?,
        v8::ModuleStatus::Instantiating => {
            throw_wasm_synthetic_module_error(scope, messages.instantiating);
            return None;
        }
        v8::ModuleStatus::Instantiated
        | v8::ModuleStatus::Evaluating
        | v8::ModuleStatus::Evaluated => {}
        v8::ModuleStatus::Errored => {
            throw_wasm_module_exception_or_synthetic_fallback(
                scope,
                module,
                messages.already_failed,
            );
            return None;
        }
    }
    match module.get_status() {
        v8::ModuleStatus::Instantiated => Some(WasmDependencyModuleReadiness::NeedsEvaluation),
        v8::ModuleStatus::Evaluating | v8::ModuleStatus::Evaluated => {
            Some(WasmDependencyModuleReadiness::Ready)
        }
        v8::ModuleStatus::Errored => {
            throw_wasm_module_exception_or_synthetic_fallback(
                scope,
                module,
                messages.evaluation_failed,
            );
            None
        }
        v8::ModuleStatus::Uninstantiated | v8::ModuleStatus::Instantiating => {
            throw_wasm_synthetic_module_error(scope, messages.not_instantiated);
            None
        }
    }
}

pub(crate) fn ensure_wasm_dependency_module_namespace_ready<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: v8::Local<'s, v8::Module>,
    instantiate: impl FnOnce(&mut v8::PinScope<'s, '_>, v8::Local<'s, v8::Module>) -> Option<()>,
    dependency_modules_for: &mut impl FnMut(
        &mut v8::PinScope<'s, '_>,
        v8::Local<'s, v8::Module>,
    ) -> Option<Vec<v8::Global<v8::Module>>>,
    perform_microtasks: impl FnOnce(&mut v8::PinScope<'s, '_>) -> Option<()>,
    messages: WasmDependencyModuleMessages,
) -> Option<()> {
    let readiness = wasm_dependency_module_readiness(scope, module, instantiate, messages)?;
    match readiness {
        WasmDependencyModuleReadiness::NeedsEvaluation => {
            match wasm_dependency_graph_reaches_evaluating_synthetic_module(
                scope,
                module,
                &mut HashSet::new(),
                dependency_modules_for,
            ) {
                Some(true) => {
                    // Do not simply call Module::Evaluate on this dependency.
                    // V8's public module API is not re-entrant-safe for this
                    // synthetic wasm -> JS -> same wasm evaluation shape; the
                    // correct fix needs a real wasm module binding/evaluation
                    // model instead of recursive evaluation from import object
                    // construction.
                    throw_wasm_synthetic_module_error(scope, messages.cyclic);
                    return None;
                }
                Some(false) => {}
                None => {
                    throw_wasm_synthetic_module_error(scope, messages.graph_unavailable);
                    return None;
                }
            }
            evaluate_wasm_dependency_module(scope, module, perform_microtasks, messages)
        }
        WasmDependencyModuleReadiness::Ready => Some(()),
    }
}

fn evaluate_wasm_dependency_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: v8::Local<'s, v8::Module>,
    perform_microtasks: impl FnOnce(&mut v8::PinScope<'s, '_>) -> Option<()>,
    messages: WasmDependencyModuleMessages,
) -> Option<()> {
    let value = crate::script_execution::evaluate_module(scope, module)
        .or_else(|| preserve_current_v8_module_exception(scope))?;
    perform_microtasks(scope)?;
    finish_wasm_dependency_module_evaluation(
        scope,
        module,
        value,
        messages.evaluation_failed,
        messages.pending,
    )
}

fn finish_wasm_dependency_module_evaluation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: v8::Local<'s, v8::Module>,
    value: v8::Local<'s, v8::Value>,
    evaluation_failed_error: &str,
    pending_error: &str,
) -> Option<()> {
    throw_wasm_module_exception_or_synthetic_fallback(scope, module, evaluation_failed_error)?;
    if let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) {
        match promise.state() {
            v8::PromiseState::Fulfilled => {}
            v8::PromiseState::Rejected => {
                scope.throw_exception(promise.result(scope));
                return None;
            }
            v8::PromiseState::Pending => {
                throw_wasm_synthetic_module_error(scope, pending_error);
                return None;
            }
        }
    }
    Some(())
}

pub(crate) fn throw_wasm_module_exception_or_synthetic_fallback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: v8::Local<'s, v8::Module>,
    fallback: &str,
) -> Option<()> {
    if module.get_status() != v8::ModuleStatus::Errored {
        return Some(());
    }
    let exception = module.get_exception();
    if !exception.is_undefined() {
        scope.throw_exception(exception);
        return None;
    }
    let _ = throw_wasm_synthetic_module_error(scope, fallback);
    None
}

pub(crate) fn preserve_current_v8_module_exception<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Value>> {
    // V8 module APIs return an empty MaybeLocal when V8 has already scheduled
    // an exception. Returning None preserves that exception for the module
    // callback caller instead of replacing it with a synthetic fallback.
    None
}

pub(crate) fn throw_wasm_synthetic_module_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let message = v8_string(scope, message)?;
    let exception = v8::Exception::type_error(scope, message);
    scope.throw_exception(exception);
    None
}

pub(crate) fn throw_wasm_link_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let message = v8_string(scope, message)?;
    let exception = webassembly_original_or_static_value(
        scope,
        "LinkError",
        ORIGINAL_WEBASSEMBLY_LINK_ERROR_CONSTRUCTOR_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    .and_then(|ctor| ctor.new_instance(scope, &[message.into()]))
    .map(v8::Local::<v8::Value>::from)
    .unwrap_or_else(|| v8::Exception::type_error(scope, message));
    scope.throw_exception(exception);
    None
}

pub(crate) fn webassembly_original_or_static_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
    original_slot: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    get_private_value(scope, global, original_slot)
        .or_else(|| webassembly_static_value(scope, name))
}

fn wasm_instance_exports<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    instance: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let exports_key = v8str(scope, "exports");
    let exports = instance.get(scope, exports_key.into())?;
    v8::Local::<v8::Object>::try_from(exports).ok()
}

fn wasm_export_value_for_module_namespace<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mode: WasmNamespaceExportMode,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    match mode {
        WasmNamespaceExportMode::RawValue => Some(value),
        WasmNamespaceExportMode::GlobalSnapshotValue => wasm_global_value(scope, value),
        WasmNamespaceExportMode::UninitializedGlobal => {
            unreachable!("uninitialized wasm global exports do not have namespace values")
        }
    }
}

fn wasm_global_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global_value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let Some(getter) =
        get_private_value(scope, global, ORIGINAL_WEBASSEMBLY_GLOBAL_VALUE_GETTER_SLOT)
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        throw_type_error(scope, "WebAssembly.Global value getter is not available.");
        return None;
    };
    getter.call(scope, global_value, &[])
}

fn construct_webassembly_instance<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wasm_module: v8::Local<'s, v8::WasmModuleObject>,
    imports: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let ctor = webassembly_original_or_static_value(
        scope,
        "Instance",
        ORIGINAL_WEBASSEMBLY_INSTANCE_CONSTRUCTOR_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    ctor.new_instance(scope, &[wasm_module.into(), imports.into()])
}

fn webassembly_static_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let webassembly = global.get(scope, v8str(scope, "WebAssembly").into())?;
    let webassembly = v8::Local::<v8::Object>::try_from(webassembly).ok()?;
    let key = v8_string(scope, name)?;
    webassembly.get(scope, key.into())
}
