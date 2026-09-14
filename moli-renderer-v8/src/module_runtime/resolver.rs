use std::cell::Cell;
use std::ptr;

use super::{ModuleAttributesKey, ModuleEvaluationRecord, NativeDocumentModulator};
use crate::util::v8_string;

thread_local! {
    static ACTIVE_NATIVE_DOCUMENT_MODULATOR: Cell<*const NativeDocumentModulator> = const { Cell::new(ptr::null()) };
}

#[derive(Debug)]
pub(crate) struct ResolverScopeGuard {
    previous: *const NativeDocumentModulator,
}

impl ResolverScopeGuard {
    pub(crate) fn new(document_modulator: *const NativeDocumentModulator) -> Self {
        debug_assert!(!document_modulator.is_null());
        let previous = ACTIVE_NATIVE_DOCUMENT_MODULATOR.with(|slot| {
            let previous = slot.get();
            slot.set(document_modulator);
            previous
        });
        Self { previous }
    }
}

impl Drop for ResolverScopeGuard {
    fn drop(&mut self) {
        ACTIVE_NATIVE_DOCUMENT_MODULATOR.with(|slot| slot.set(self.previous));
    }
}

pub(crate) fn resolve_static_module_callback<'s>(
    context: v8::Local<'s, v8::Context>,
    specifier: v8::Local<'s, v8::String>,
    import_attributes: v8::Local<'s, v8::FixedArray>,
    referrer: v8::Local<'s, v8::Module>,
) -> Option<v8::Local<'s, v8::Module>> {
    v8::callback_scope!(unsafe scope, context);
    let specifier = specifier.to_rust_string_lossy(scope);
    let attributes = import_attributes_key(scope, import_attributes);
    ACTIVE_NATIVE_DOCUMENT_MODULATOR.with(|slot| {
        let document_modulator = slot.get();
        if document_modulator.is_null() {
            return None;
        }
        let document_modulator = unsafe { &*document_modulator };
        let record =
            document_modulator.resolve_static_dependency(referrer, &specifier, &attributes)?;
        Some(v8::Local::new(scope, record.compiled_module()))
    })
}

pub(crate) fn resolve_static_source_callback<'s>(
    context: v8::Local<'s, v8::Context>,
    specifier: v8::Local<'s, v8::String>,
    import_attributes: v8::Local<'s, v8::FixedArray>,
    referrer: v8::Local<'s, v8::Module>,
) -> Option<v8::Local<'s, v8::Object>> {
    v8::callback_scope!(unsafe scope, context);
    let specifier = specifier.to_rust_string_lossy(scope);
    let attributes = import_attributes_key(scope, import_attributes);
    ACTIVE_NATIVE_DOCUMENT_MODULATOR.with(|slot| {
        let document_modulator = slot.get();
        if document_modulator.is_null() {
            return throw_source_phase_syntax_error(
                scope,
                "module source resolver is not available",
            );
        }
        let Some(record) = (unsafe { &*document_modulator }).resolve_static_dependency(
            referrer,
            &specifier,
            &attributes,
        ) else {
            return throw_source_phase_syntax_error(
                scope,
                &format!("source-phase module `{specifier}` was not resolved"),
            );
        };
        let Some(wasm_record) = record.wasm_module() else {
            return throw_source_phase_syntax_error(
                scope,
                &format!("source-phase module `{specifier}` is not a WebAssembly module"),
            );
        };
        let Some(source) = wasm_record.source_module(scope) else {
            return throw_source_phase_syntax_error(
                scope,
                &format!("failed to materialize WebAssembly source for `{specifier}`"),
            );
        };
        Some(source.into())
    })
}

pub(crate) fn resolve_evaluation_module_callback<'s>(
    context: v8::Local<'s, v8::Context>,
    specifier: v8::Local<'s, v8::String>,
    import_attributes: v8::Local<'s, v8::FixedArray>,
    referrer: v8::Local<'s, v8::Module>,
) -> Option<v8::Local<'s, v8::Module>> {
    v8::callback_scope!(unsafe scope, context);
    let record = ModuleEvaluationRecord::for_module(context, referrer)?;
    let attributes = import_attributes_key(scope, import_attributes);
    let dependency = record.dependency(&specifier.to_rust_string_lossy(scope), &attributes)?;
    Some(v8::Local::new(scope, &dependency))
}

pub(crate) fn resolve_evaluation_source_callback<'s>(
    context: v8::Local<'s, v8::Context>,
    specifier: v8::Local<'s, v8::String>,
    import_attributes: v8::Local<'s, v8::FixedArray>,
    referrer: v8::Local<'s, v8::Module>,
) -> Option<v8::Local<'s, v8::Object>> {
    v8::callback_scope!(unsafe scope, context);
    let record = ModuleEvaluationRecord::for_module(context, referrer)?;
    let attributes = import_attributes_key(scope, import_attributes);
    let dependency = record.dependency(&specifier.to_rust_string_lossy(scope), &attributes)?;
    let dependency = v8::Local::new(scope, &dependency);
    let record = ModuleEvaluationRecord::for_module(context, dependency)?;
    let wasm = record.wasm()?;
    Some(wasm.source_module(scope)?.into())
}

fn throw_source_phase_syntax_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let message = v8_string(scope, message)?;
    let exception = v8::Exception::syntax_error(scope, message);
    scope.throw_exception(exception);
    None
}

fn import_attributes_key(
    scope: &mut v8::PinScope<'_, '_>,
    attributes: v8::Local<'_, v8::FixedArray>,
) -> ModuleAttributesKey {
    let mut pairs = Vec::with_capacity(attributes.length() / 2);
    let mut index = 0;
    while index + 1 < attributes.length() {
        let key = attributes
            .get(scope, index)
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .map(|value| value.to_rust_string_lossy(scope));
        let value = attributes
            .get(scope, index + 1)
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .map(|value| value.to_rust_string_lossy(scope));
        if let (Some(key), Some(value)) = (key, value) {
            pairs.push((key, value));
        }
        index += 2;
    }
    ModuleAttributesKey::from_pairs(pairs)
}
