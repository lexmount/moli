use crate::util::{context_host_ptr_from_global_bridge, v8_string, v8str};

use super::super::super::{
    build_detached_shadow_root_object_for_native_handle, detached_native_handle,
    detached_state_object, object_property_as_object, object_string_property,
    paired_detached_native_object_for_handle,
};

pub(in crate::native_bridge) fn detached_shadow_root_for_host<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let state = detached_state_object(scope, host)?;
    if object_string_property(scope, state, "shadowRootMode").as_deref() == Some("open") {
        return object_property_as_object(scope, state, "shadowRoot");
    }

    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let host_handle = detached_native_handle(scope, host)?;
    let root_handle = unsafe { &*runtime_ptr }
        .dom_host()
        .shadow_root_handle(host_handle)?;
    let mode = unsafe { &*runtime_ptr }
        .dom_host()
        .shadow_root_mode(root_handle)
        .filter(|mode| mode == "open")?;
    let root = paired_detached_native_object_for_handle(scope, runtime_ptr, root_handle).or_else(
        || {
            build_detached_shadow_root_object_for_native_handle(
                scope,
                host,
                &mode,
                Some(root_handle),
            )
        },
    )?;
    let _ = state.set(scope, v8str(scope, "shadowRoot").into(), root.into());
    let _ = state.set(
        scope,
        v8str(scope, "shadowRootMode").into(),
        v8_string(scope, &mode)
            .map(Into::<v8::Local<'_, v8::Value>>::into)
            .unwrap_or_else(|| v8::String::empty(scope).into()),
    );
    Some(root)
}
