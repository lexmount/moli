use std::cell::OnceCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::document_module_graph::{ModuleMapKey, ModuleRequestRecord};
use crate::wasm_module_support::WasmExportRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModuleRecordState {
    Compiled,
    Instantiated,
    Evaluating,
    Evaluated,
}

#[derive(Debug, Clone)]
pub(crate) struct ModuleRecordEntry {
    key: ModuleMapKey,
    compiled_module: v8::Global<v8::Module>,
    requests: Vec<ModuleRequestRecord>,
    wasm_module: Option<WasmModuleRecord>,
    // Own the evaluation data; the Context's lookup only keeps a weak reference.
    synthetic_text_module_source: Option<Rc<super::SyntheticTextModuleSource>>,
    state: ModuleRecordState,
}

impl ModuleRecordEntry {
    pub(crate) fn new(
        key: ModuleMapKey,
        compiled_module: v8::Global<v8::Module>,
        requests: Vec<ModuleRequestRecord>,
    ) -> Self {
        Self {
            key,
            compiled_module,
            requests,
            wasm_module: None,
            synthetic_text_module_source: None,
            state: ModuleRecordState::Compiled,
        }
    }

    pub(crate) fn new_with_wasm_module(
        key: ModuleMapKey,
        compiled_module: v8::Global<v8::Module>,
        requests: Vec<ModuleRequestRecord>,
        wasm_module: WasmModuleRecord,
    ) -> Self {
        Self {
            key,
            compiled_module,
            requests,
            wasm_module: Some(wasm_module),
            synthetic_text_module_source: None,
            state: ModuleRecordState::Compiled,
        }
    }

    pub(crate) fn key(&self) -> &ModuleMapKey {
        &self.key
    }

    pub(crate) fn with_synthetic_text_module_source(
        mut self,
        source: Rc<super::SyntheticTextModuleSource>,
    ) -> Self {
        self.synthetic_text_module_source = Some(source);
        self
    }

    pub(crate) fn compiled_module(&self) -> &v8::Global<v8::Module> {
        &self.compiled_module
    }

    pub(crate) fn requests(&self) -> &[ModuleRequestRecord] {
        &self.requests
    }

    pub(crate) fn wasm_module(&self) -> Option<&WasmModuleRecord> {
        self.wasm_module.as_ref()
    }

    pub(crate) fn set_state(&mut self, state: ModuleRecordState) {
        self.state = state;
    }
}

#[derive(Clone)]
pub(crate) struct WasmModuleRecord {
    compiled_module: Arc<v8::CompiledWasmModule>,
    source_module: Rc<OnceCell<v8::Global<v8::WasmModuleObject>>>,
    instance: Rc<OnceCell<v8::Global<v8::Object>>>,
    exports: Vec<WasmExportRecord>,
    imports: Vec<WasmImportRecord>,
}

impl std::fmt::Debug for WasmModuleRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmModuleRecord")
            .field("exports", &self.exports)
            .field("imports", &self.imports)
            .finish_non_exhaustive()
    }
}

impl WasmModuleRecord {
    pub(crate) fn new(
        compiled_module: Arc<v8::CompiledWasmModule>,
        exports: Vec<WasmExportRecord>,
        imports: Vec<WasmImportRecord>,
    ) -> Self {
        Self {
            compiled_module,
            source_module: Rc::new(OnceCell::new()),
            instance: Rc::new(OnceCell::new()),
            exports,
            imports,
        }
    }

    pub(crate) fn source_module<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::WasmModuleObject>> {
        if let Some(source_module) = self.source_module.get() {
            return Some(v8::Local::new(scope, source_module));
        }
        let source_module =
            v8::WasmModuleObject::from_compiled_module(scope, self.compiled_module.as_ref())?;
        if self
            .source_module
            .set(v8::Global::new(scope, source_module))
            .is_err()
        {
            return self
                .source_module
                .get()
                .map(|source_module| v8::Local::new(scope, source_module));
        }
        Some(source_module)
    }

    pub(crate) fn set_instance<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        instance: v8::Local<'s, v8::Object>,
    ) {
        let _ = self.instance.set(v8::Global::new(scope, instance));
    }

    pub(crate) fn instance<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.instance
            .get()
            .map(|instance| v8::Local::new(scope, instance))
    }

    pub(crate) fn exports(&self) -> &[WasmExportRecord] {
        &self.exports
    }

    pub(crate) fn imports(&self) -> &[WasmImportRecord] {
        &self.imports
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WasmImportRecord {
    module: String,
    name: String,
}

impl WasmImportRecord {
    pub(crate) fn new(module: String, name: String) -> Self {
        Self { module, name }
    }

    pub(crate) fn module(&self) -> &str {
        &self.module
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }
}
