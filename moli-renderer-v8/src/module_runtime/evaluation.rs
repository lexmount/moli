use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use super::{
    ModuleAttributesKey, ModuleIdentityHash, ModuleImportPhase, ModuleRequestRecord,
    WasmModuleRecord, module_identity_hash_from_v8_module,
};

/// Resolved evaluation inputs belong to a compiled module, including when
/// evaluation resumes after an asynchronous dependency in a child realm.
/// Context lookup is weak so it cannot keep a retired module map alive.
#[derive(Debug)]
pub(crate) struct ModuleEvaluationRecord {
    module: v8::Global<v8::Module>,
    wasm: Option<WasmModuleRecord>,
    dependencies: Vec<(ModuleRequestRecord, v8::Global<v8::Module>)>,
}

#[derive(Default)]
struct ModuleEvaluationRecords {
    entries: RefCell<HashMap<ModuleIdentityHash, Vec<Weak<ModuleEvaluationRecord>>>>,
}

impl ModuleEvaluationRecord {
    pub(super) fn register(
        scope: &mut v8::PinScope<'_, '_>,
        module: v8::Local<'_, v8::Module>,
        wasm: Option<WasmModuleRecord>,
        dependencies: Vec<(ModuleRequestRecord, v8::Global<v8::Module>)>,
    ) -> Rc<Self> {
        let context = scope.get_current_context();
        let records = context
            .get_slot::<ModuleEvaluationRecords>()
            .unwrap_or_else(|| {
                let records = Rc::new(ModuleEvaluationRecords::default());
                context.set_slot(records.clone());
                records
            });
        let record = Rc::new(Self {
            module: v8::Global::new(scope, module),
            wasm,
            dependencies,
        });
        let mut entries = records.entries.borrow_mut();
        let candidates = entries
            .entry(module_identity_hash_from_v8_module(module))
            .or_default();
        candidates.retain(|candidate| candidate.strong_count() != 0);
        candidates.push(Rc::downgrade(&record));
        record
    }

    pub(crate) fn for_module(
        context: v8::Local<'_, v8::Context>,
        module: v8::Local<'_, v8::Module>,
    ) -> Option<Rc<Self>> {
        let records = context.get_slot::<ModuleEvaluationRecords>()?;
        let entries = records.entries.borrow();
        entries
            .get(&module_identity_hash_from_v8_module(module))?
            .iter()
            .filter_map(Weak::upgrade)
            // Identity hashes can collide; always check the actual V8 module.
            .find(|candidate| module == candidate.module)
    }

    pub(crate) fn wasm(&self) -> Option<&WasmModuleRecord> {
        self.wasm.as_ref()
    }

    pub(crate) fn dependency(
        &self,
        specifier: &str,
        attributes: &ModuleAttributesKey,
    ) -> Option<v8::Global<v8::Module>> {
        self.dependencies.iter().find_map(|(request, module)| {
            (request.specifier() == specifier && request.attributes() == attributes)
                .then(|| module.clone())
        })
    }

    pub(crate) fn evaluation_dependencies(&self) -> Vec<v8::Global<v8::Module>> {
        self.dependencies
            .iter()
            .filter(|(request, _)| request.phase() == ModuleImportPhase::Evaluation)
            .map(|(_, module)| module.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module_runtime::{ModuleMapKey, ModuleRecordEntry};
    use url::Url;

    fn unused_evaluation_steps<'s>(
        context: v8::Local<'s, v8::Context>,
        _module: v8::Local<'s, v8::Module>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        v8::callback_scope!(unsafe scope, context);
        Some(v8::undefined(scope).into())
    }

    #[test]
    fn evaluation_records_check_context_identity_and_compiled_record_lifetime() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let name = v8::String::new(scope, "https://example.test/module.wasm").unwrap();
        let first = v8::Module::create_synthetic_module(scope, name, &[], unused_evaluation_steps);
        let second = v8::Module::create_synthetic_module(scope, name, &[], unused_evaluation_steps);
        let key =
            ModuleMapKey::webassembly(Url::parse("https://example.test/module.wasm").unwrap());
        let first_record =
            ModuleRecordEntry::new(key.clone(), v8::Global::new(scope, first), Vec::new());
        let second_record = ModuleRecordEntry::new(key, v8::Global::new(scope, second), Vec::new());
        first_record.register_evaluation(scope, Vec::new());
        let dependency = v8::Global::new(scope, first);
        second_record.register_evaluation(
            scope,
            vec![(
                ModuleRequestRecord::new(
                    "./dependency.wasm",
                    ModuleAttributesKey::empty(),
                    ModuleImportPhase::Evaluation,
                ),
                dependency.clone(),
            )],
        );
        let first_evaluation = ModuleEvaluationRecord::for_module(context, first).unwrap();
        let second_evaluation = ModuleEvaluationRecord::for_module(context, second).unwrap();
        // Force an identity-hash collision candidate; lookup must still find
        // the exact compiled module in the correct Context.
        context
            .get_slot::<ModuleEvaluationRecords>()
            .unwrap()
            .entries
            .borrow_mut()
            .get_mut(&module_identity_hash_from_v8_module(second))
            .unwrap()
            .insert(0, Rc::downgrade(&first_evaluation));
        assert!(Rc::ptr_eq(
            &ModuleEvaluationRecord::for_module(context, second).unwrap(),
            &second_evaluation
        ));
        assert_eq!(
            second_evaluation.dependency("./dependency.wasm", &ModuleAttributesKey::empty()),
            Some(dependency)
        );
        let foreign = v8::Context::new(scope, Default::default());
        assert!(ModuleEvaluationRecord::for_module(foreign, second).is_none());
        let clone = second_record.clone();
        drop(second_record);
        drop(second_evaluation);
        assert!(ModuleEvaluationRecord::for_module(context, second).is_some());
        drop(clone);
        assert!(ModuleEvaluationRecord::for_module(context, second).is_none());
    }
}
