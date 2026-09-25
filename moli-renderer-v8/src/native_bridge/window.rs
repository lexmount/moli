use super::runtime_ptr_from_object;
use crate::util::v8str;

// ── Layer 1: isolate-level template build (C = ()) ────────────────────────────

pub(super) fn build_window_wrapper_template<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i, ()>,
) -> v8::Local<'s, v8::ObjectTemplate> {
    let template = v8::ObjectTemplate::new(scope);
    // Unlike the high-cardinality Node wrappers, Window wrappers retain an
    // object-local host pointer for cross-realm WebIDL brand checks. Field 1
    // distinguishes a reflected Window (positive id) from a synthetic shell
    // (zero).
    let _ = template.set_internal_field_count(2);
    // Synthetic same-origin Window wrappers (for example the lightweight
    // popup shell) are concrete Window objects too. They cannot inherit the
    // global interface surface from Window.prototype because [Global]
    // interface members are own properties in Blink and Web IDL.
    crate::context_bootstrap::install_window_own_template_bindings(scope, template);
    super::context_host::install_synthetic_window_access_check_handlers(template);
    template
}

pub(super) fn sync_window_wrapper_function_identity(
    scope: &mut v8::PinScope<'_, '_>,
    wrapper: v8::Local<'_, v8::Object>,
) {
    let property_names = v8::GetPropertyNamesArgsBuilder::new()
        .mode(v8::KeyCollectionMode::OwnOnly)
        .property_filter(v8::PropertyFilter::ALL_PROPERTIES)
        .build();
    let Some(names) = wrapper.get_own_property_names(scope, property_names) else {
        return;
    };
    let global = scope.get_current_context().global(scope);
    let value_key = v8str(scope, "value");
    for index in 0..names.length() {
        let Some(key_value) = names.get_index(scope, index) else {
            continue;
        };
        let Ok(key) = v8::Local::<v8::Name>::try_from(key_value) else {
            continue;
        };
        let Some(wrapper_value) = wrapper
            .get_own_property_descriptor(scope, key)
            .and_then(|descriptor| v8::Local::<v8::Object>::try_from(descriptor).ok())
            .and_then(|descriptor| descriptor.get(scope, value_key.into()))
        else {
            continue;
        };
        if !wrapper_value.is_function() {
            continue;
        }
        let Some(global_value) = global
            .get(scope, key.into())
            .filter(|value| value.is_function())
        else {
            continue;
        };
        let _ = wrapper.set(scope, key.into(), global_value);
    }
}

// ── Layer 3: runtime callbacks ─────────────────────────────────────────────────

/// Bridge object accessor: `__moliNativeBridge.window` → wrapped Window object.
pub(super) fn bridge_window_getter(
    scope: &mut v8::PinScope<'_, '_>,
    _key: v8::Local<'_, v8::Name>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let bridge = args.holder();
    let Ok(runtime_ptr) = runtime_ptr_from_object(scope, bridge) else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    match runtime.native_bridge_mut().wrap_window(scope, runtime_ptr) {
        Some(window) => rv.set(window.into()),
        None => rv.set_null(),
    }
}
