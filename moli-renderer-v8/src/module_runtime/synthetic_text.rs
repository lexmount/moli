use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use super::{ModuleIdentityHash, ModuleMapKey, module_identity_hash_from_v8_module};

/// Evaluation data belongs to the compiled module record, not the page's
/// default Document. The Context only holds weak references, so this lookup
/// does not keep a disposed realm or its compiled modules alive.
#[derive(Debug)]
pub(crate) struct SyntheticTextModuleSource {
    module: v8::Global<v8::Module>,
    key: ModuleMapKey,
    source: String,
}

#[derive(Default)]
struct SyntheticTextModuleSources {
    entries: RefCell<HashMap<ModuleIdentityHash, Vec<Weak<SyntheticTextModuleSource>>>>,
}

impl SyntheticTextModuleSource {
    pub(crate) fn register(
        scope: &mut v8::PinScope<'_, '_>,
        module: v8::Local<'_, v8::Module>,
        key: ModuleMapKey,
        source: &str,
    ) -> Rc<Self> {
        let context = scope.get_current_context();
        let sources = context
            .get_slot::<SyntheticTextModuleSources>()
            .unwrap_or_else(|| {
                let sources = Rc::new(SyntheticTextModuleSources::default());
                context.set_slot(sources.clone());
                sources
            });
        let record = Rc::new(Self {
            module: v8::Global::new(scope, module),
            key,
            source: source.to_owned(),
        });
        let mut entries = sources.entries.borrow_mut();
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
        let sources = context.get_slot::<SyntheticTextModuleSources>()?;
        let entries = sources.entries.borrow();
        entries
            .get(&module_identity_hash_from_v8_module(module))?
            .iter()
            .filter_map(Weak::upgrade)
            // V8 identity hashes are not unique, even within one Context.
            .find(|candidate| module == candidate.module)
    }

    pub(crate) fn key(&self) -> &ModuleMapKey {
        &self.key
    }

    pub(crate) fn source(&self) -> &str {
        &self.source
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module_runtime::{ModuleAttributesKey, ModuleRecordEntry};
    use url::Url;

    fn unused_evaluation_steps<'s>(
        context: v8::Local<'s, v8::Context>,
        _module: v8::Local<'s, v8::Module>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        v8::callback_scope!(unsafe scope, context);
        Some(v8::undefined(scope).into())
    }

    #[test]
    fn synthetic_text_module_lookup_checks_context_and_exact_identity() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let name = v8::String::new(scope, "https://example.test/shared.json").unwrap();
        let first = v8::Module::create_synthetic_module(scope, name, &[], unused_evaluation_steps);
        let second = v8::Module::create_synthetic_module(scope, name, &[], unused_evaluation_steps);
        let key = ModuleMapKey::json_with_attributes(
            Url::parse("https://example.test/shared.json").unwrap(),
            ModuleAttributesKey::empty(),
        );
        let first_source = SyntheticTextModuleSource::register(scope, first, key.clone(), "1");
        let second_source = SyntheticTextModuleSource::register(scope, second, key, "2");

        // Inject a conflicting candidate deterministically instead of waiting
        // for a random V8 identity-hash collision.
        let sources = context.get_slot::<SyntheticTextModuleSources>().unwrap();
        sources
            .entries
            .borrow_mut()
            .get_mut(&module_identity_hash_from_v8_module(second))
            .unwrap()
            .insert(0, Rc::downgrade(&first_source));
        assert!(Rc::ptr_eq(
            &SyntheticTextModuleSource::for_module(context, second).unwrap(),
            &second_source,
        ));
        let foreign = v8::Context::new(scope, Default::default());
        assert!(SyntheticTextModuleSource::for_module(foreign, first).is_none());
    }

    #[test]
    fn synthetic_text_module_lookup_does_not_keep_disposed_records_alive() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let name = v8::String::new(scope, "https://example.test/value.json").unwrap();
        let module = v8::Module::create_synthetic_module(scope, name, &[], unused_evaluation_steps);
        let key = ModuleMapKey::json_with_attributes(
            Url::parse("https://example.test/value.json").unwrap(),
            ModuleAttributesKey::empty(),
        );
        let source = SyntheticTextModuleSource::register(scope, module, key.clone(), "42");
        let weak = Rc::downgrade(&source);
        let record = ModuleRecordEntry::new(key, v8::Global::new(scope, module), Vec::new())
            .with_synthetic_text_module_source(source);
        let clone = record.clone();
        drop(record);
        assert_eq!(
            SyntheticTextModuleSource::for_module(context, module)
                .unwrap()
                .source(),
            "42"
        );
        drop(clone);
        assert!(weak.upgrade().is_none());
        assert!(SyntheticTextModuleSource::for_module(context, module).is_none());
    }
}
