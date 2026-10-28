use super::*;
use crate::dom::native::ShadowRootInit;
use crate::{
    context_bootstrap::selection_value_for_window,
    custom_elements,
    native_bridge::document::{
        detached_install::install_detached_parent_node_instance_properties,
        validate_registry_association_for_document,
    },
    native_bridge::element::shadow_root_init_from_attach_shadow_value,
    util::{context_host_ptr_from_global_bridge, throw_type_error},
};

const DETACHED_ACTIVE_ELEMENT_SLOT: &str = "__moliDetachedActiveElement";

fn build_detached_shadow_root_object_with_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: v8::Local<'s, v8::Object>,
    init: ShadowRootInit,
    available_to_element_internals: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let mode = init.mode().to_owned();
    let root_handle = if let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(host_handle) = detached_native_handle(scope, host)
    {
        let runtime = unsafe { &mut *runtime_ptr };
        let root_handle = runtime
            .dom_host_mut()
            .attach_shadow_root_with_init(host_handle, init);
        if let Some(root_handle) = root_handle {
            runtime
                .dom_host_mut()
                .set_shadow_root_available_to_element_internals(
                    root_handle,
                    available_to_element_internals,
                );
        }
        root_handle
    } else {
        None
    };
    build_detached_shadow_root_object_for_native_handle(scope, host, &mode, root_handle)
}

pub(in crate::native_bridge::document) fn build_detached_shadow_root_object_for_native_handle<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    host: v8::Local<'s, v8::Object>,
    mode: &str,
    root_handle: Option<DomHandle>,
) -> Option<v8::Local<'s, v8::Object>> {
    let owner_document = detached_owner_document_object(scope, host)?;
    let object = new_detached_object_with_prototype(
        scope,
        "__detachedShadowRootPrototype",
        Some("ShadowRoot"),
    )?;
    let state = new_detached_state_object(scope, "shadowRoot", 11, "#document-fragment")?;
    let _ = state.set(
        scope,
        v8str(scope, "ownerDocument").into(),
        owner_document.into(),
    );
    let _ = state.set(scope, v8str(scope, "host").into(), host.into());
    let _ = state.set(
        scope,
        v8str(scope, "mode").into(),
        v8_string(scope, mode)?.into(),
    );
    define_detached_state(scope, object, state);
    if let Some(root_handle) = root_handle {
        define_detached_native_handle(scope, object, root_handle);
    }
    install_detached_node_core_instance_properties(scope, object);
    install_detached_parent_node_instance_properties(scope, object);
    Some(object)
}

pub(in crate::native_bridge) fn detached_shadow_root_active_element_value<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    root: v8::Local<'a, v8::Object>,
) -> Option<v8::Local<'a, v8::Value>> {
    let Some(document) = detached_owner_document_object(scope, root) else {
        return Some(v8::null(scope).into());
    };
    let active = detached_state_object(scope, document)
        .and_then(|state| object_property_as_object(scope, state, DETACHED_ACTIVE_ELEMENT_SLOT));
    Some(match active {
        Some(active) if detached_shadow_root_contains_object(scope, root, active) => active.into(),
        _ => v8::null(scope).into(),
    })
}

pub(in crate::native_bridge) fn detached_shadow_root_selection_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    root: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let Some(document) = detached_owner_document_object(scope, root) else {
        return Some(v8::null(scope).into());
    };
    let Some(default_view) = document.get(scope, v8str(scope, "defaultView").into()) else {
        return Some(v8::null(scope).into());
    };
    let Ok(window) = v8::Local::<v8::Object>::try_from(default_view) else {
        return Some(v8::null(scope).into());
    };
    Some(
        selection_value_for_window(scope, window)
            .map(Into::into)
            .unwrap_or_else(|| v8::null(scope).into()),
    )
}

fn detached_object_contains<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    root: v8::Local<'s, v8::Object>,
    target: v8::Local<'s, v8::Object>,
) -> bool {
    if root.strict_equals(target.into()) {
        return true;
    }
    detached_child_node_objects(scope, root)
        .into_iter()
        .any(|child| detached_object_contains(scope, child, target))
}

fn detached_shadow_root_contains_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    root: v8::Local<'s, v8::Object>,
    target: v8::Local<'s, v8::Object>,
) -> bool {
    if detached_object_contains(scope, root, target) {
        return true;
    }
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let Some(root_handle) = detached_native_handle_for_runtime(scope, runtime_ptr, root) else {
        return false;
    };
    let Some(target_handle) = detached_native_handle_for_runtime(scope, runtime_ptr, target) else {
        return false;
    };
    let dom_host = unsafe { &*runtime_ptr }.dom_host();
    let dom = dom_host.dom();
    let mut current = Some(target_handle);
    while let Some(handle) = current {
        if handle == root_handle {
            return true;
        }
        current = dom
            .parent_node(handle)
            .or_else(|| dom_host.shadow_root_host(handle));
    }
    false
}

pub(in crate::native_bridge) fn detached_attach_shadow_method_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let host = args.this();
    if detached_state_kind(scope, host).as_deref() != Some("element") {
        throw_type_error(scope, "Element.attachShadow receiver must be an element.");
        return;
    }
    let Some((registry_association, shadow_init)) =
        shadow_root_init_from_attach_shadow_value(scope, args.get(0))
    else {
        return;
    };
    let Some(host_state) = detached_state_object(scope, host) else {
        rv.set_null();
        return;
    };
    if object_property_as_object(scope, host_state, "shadowRoot").is_some() {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "Shadow root cannot be created on this host.",
        );
        return;
    }
    if let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(owner_document) = detached_owner_document_object(scope, host)
        && let Some(document_handle) = detached_native_handle(scope, owner_document)
        && let Some(registry_association) = registry_association
        && !validate_registry_association_for_document(
            scope,
            runtime_ptr,
            document_handle,
            Some(registry_association),
        )
    {
        return;
    }
    let available_to_element_internals = context_host_ptr_from_global_bridge(scope)
        .zip(detached_native_handle(scope, host))
        .is_some_and(|(runtime_ptr, host_handle)| {
            custom_elements::preserves_custom_element_identity(runtime_ptr, host_handle)
        });
    let existing_root = context_host_ptr_from_global_bridge(scope)
        .zip(detached_native_handle(scope, host))
        .and_then(|(runtime_ptr, handle)| {
            unsafe { &*runtime_ptr }
                .dom_host()
                .shadow_root_handle(handle)
        });
    let shadow_root_mode = shadow_init.mode().to_owned();
    let Some(root) = build_detached_shadow_root_object_with_init(
        scope,
        host,
        shadow_init,
        available_to_element_internals,
    ) else {
        rv.set_null();
        return;
    };
    if let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(root_handle) = detached_native_handle(scope, root)
        && existing_root != Some(root_handle)
    {
        let root_registry_association = registry_association.or_else(|| {
            let owner_document = detached_owner_document_object(scope, host)?;
            let document_handle = detached_native_handle(scope, owner_document)?;
            Some(
                unsafe { &*runtime_ptr }
                    .effective_custom_element_registry_association(document_handle),
            )
        });
        if let Some(root_registry_association) = root_registry_association {
            unsafe { &mut *runtime_ptr }
                .set_custom_element_registry_association(root_handle, root_registry_association);
        }
    }
    let _ = host_state.set(scope, v8str(scope, "shadowRoot").into(), root.into());
    let _ = host_state.set(
        scope,
        v8str(scope, "shadowRootMode").into(),
        v8_string(scope, &shadow_root_mode)
            .map(Into::<v8::Local<'_, v8::Value>>::into)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
    rv.set(root.into());
}
