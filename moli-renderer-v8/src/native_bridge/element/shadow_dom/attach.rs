use crate::custom_elements;
use crate::dom::native::ShadowRootInit;
use crate::util::{throw_type_error, v8str};

use super::super::super::{
    document::{self, validate_registry_association_for_document},
    node::node_runtime_and_handle_from_args,
    set_wrapped_handle_or_null_for_receiver, throw_dom_exception,
};
use super::super::property_string_value;

fn dictionary_boolean(
    scope: &mut v8::PinScope<'_, '_>,
    init: v8::Local<'_, v8::Object>,
    name: &'static str,
) -> bool {
    init.get(scope, crate::util::v8str(scope, name).into())
        .is_some_and(|value| value.boolean_value(scope))
}

fn dictionary_string(
    scope: &mut v8::PinScope<'_, '_>,
    init: v8::Local<'_, v8::Object>,
    name: &'static str,
) -> Option<Option<String>> {
    let value = init.get(scope, crate::util::v8str(scope, name).into())?;
    if value.is_undefined() {
        return Some(None);
    }
    property_string_value(scope, value).map(Some)
}

fn dictionary_nullable_string(
    scope: &mut v8::PinScope<'_, '_>,
    init: v8::Local<'_, v8::Object>,
    name: &'static str,
) -> Option<Option<String>> {
    let value = init.get(scope, crate::util::v8str(scope, name).into())?;
    if value.is_null_or_undefined() {
        return Some(None);
    }
    property_string_value(scope, value).map(Some)
}

pub(in crate::native_bridge) fn shadow_root_init_from_attach_shadow_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<(v8::Local<'s, v8::Object>, ShadowRootInit)> {
    let Some(init) = value.to_object(scope) else {
        throw_type_error(scope, "Element.attachShadow requires an init dictionary.");
        return None;
    };
    let Some(Some(mode)) = dictionary_string(scope, init, "mode") else {
        throw_type_error(scope, "Element.attachShadow init.mode is required.");
        return None;
    };
    if mode != "open" && mode != "closed" {
        throw_type_error(
            scope,
            "Element.attachShadow init.mode must be 'open' or 'closed'.",
        );
        return None;
    }
    let mut shadow_init = ShadowRootInit::new(&mode);
    shadow_init.set_delegates_focus(dictionary_boolean(scope, init, "delegatesFocus"));
    shadow_init.set_clonable(dictionary_boolean(scope, init, "clonable"));
    shadow_init.set_serializable(dictionary_boolean(scope, init, "serializable"));
    if let Some(Some(slot_assignment)) = dictionary_string(scope, init, "slotAssignment") {
        if slot_assignment != "named" && slot_assignment != "manual" {
            throw_type_error(
                scope,
                "Element.attachShadow init.slotAssignment must be 'named' or 'manual'.",
            );
            return None;
        }
        shadow_init.set_slot_assignment(&slot_assignment);
    }
    if let Some(reference_target) = dictionary_nullable_string(scope, init, "referenceTarget") {
        shadow_init.set_reference_target(reference_target);
    }
    Some((init, shadow_init))
}

pub(in crate::native_bridge) fn element_attach_shadow_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        document::detached_attach_shadow_method_callback(scope, args, rv);
        return;
    };
    if document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some() {
        document::detached_attach_shadow_method_callback(scope, args, rv);
        return;
    }
    let Some((init, shadow_init)) = shadow_root_init_from_attach_shadow_value(scope, args.get(0))
    else {
        return;
    };
    let available_to_element_internals =
        custom_elements::preserves_custom_element_identity(runtime_ptr, handle);
    let registry_association = init
        .get(scope, v8str(scope, "customElementRegistry").into())
        .and_then(|value| custom_elements::registry_association_from_value(scope, value));
    let runtime = unsafe { &mut *runtime_ptr };
    let Some(document_handle) = runtime.dom_host().owner_document_handle(handle) else {
        rv.set_null();
        return;
    };
    if let Some(registry_association) = registry_association
        && !validate_registry_association_for_document(
            scope,
            runtime_ptr,
            document_handle,
            Some(registry_association),
        )
    {
        return;
    }
    if runtime
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| store.definition_disables_shadow_for_handle(runtime_ptr, handle))
    {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "Shadow root cannot be created on this host.",
        );
        return;
    }
    let Some(root_handle) = runtime
        .dom_host_mut()
        .attach_shadow_root_with_init(handle, shadow_init)
    else {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "Shadow root cannot be created on this host.",
        );
        return;
    };
    runtime
        .dom_host_mut()
        .set_shadow_root_available_to_element_internals(
            root_handle,
            available_to_element_internals,
        );
    let root_registry_association = registry_association
        .unwrap_or_else(|| runtime.effective_custom_element_registry_association(document_handle));
    runtime.set_custom_element_registry_association(root_handle, root_registry_association);
    set_wrapped_handle_or_null_for_receiver(
        scope,
        &mut rv,
        runtime_ptr,
        args.this(),
        Some(root_handle),
    );
}
