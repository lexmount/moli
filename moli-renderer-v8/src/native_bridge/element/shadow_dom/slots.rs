use crate::document_runtime::DomHandle;
use crate::dom::native::Node;
use crate::style_engine::StyleMutationEffect;
use crate::util::serialize_v8_array;
use crate::util::throw_type_error;
use crate::util::v8str;

use super::super::super::{
    JsContextHost,
    bridge::wrapped_handle_value_for_receiver,
    node::{
        node_runtime_and_handle_from_args_or_detached,
        node_runtime_and_handle_from_object_or_detached,
    },
};
use super::super::{
    attribute_property_getter_from_object_or_detached, set_attribute_property_on_object_or_detached,
};
use crate::native_bridge::document;

pub(in crate::native_bridge) fn slot_name_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "name", rv);
}

pub(in crate::native_bridge) fn slot_name_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_attribute_property_on_object_or_detached(scope, args.this(), "name", args.get(0));
}

pub(in crate::native_bridge) fn node_slot_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "slot", rv);
}

pub(in crate::native_bridge) fn node_slot_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_attribute_property_on_object_or_detached(scope, args.this(), "slot", args.get(0));
}

pub(in crate::native_bridge) fn slot_assigned_slot_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    match assigned_slot_for_object(scope, args.this()) {
        Some(slot) => rv.set(slot),
        None => rv.set_null(),
    }
}

fn assigned_slot_for_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return None;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let slot_handle = runtime.dom_host().assigned_slot_for_node(handle)?;
    let shadow_root = runtime.dom_host().containing_shadow_root(slot_handle)?;
    if runtime.dom_host().shadow_root_mode(shadow_root).as_deref() != Some("open") {
        return None;
    }
    if document::detached_native_handle_for_runtime(scope, runtime_ptr, object).is_some() {
        document::detached_native_object_for_handle(scope, runtime_ptr, slot_handle).map(Into::into)
    } else {
        wrapped_handle_value_for_receiver(scope, runtime_ptr, object, slot_handle)
    }
}

pub(in crate::native_bridge) fn slot_assigned_nodes_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_null();
        return;
    };
    let flatten = slot_flatten_option(scope, &args);
    let handles = unsafe { &*runtime_ptr }
        .dom_host()
        .assigned_nodes_for_slot_with_options(handle, flatten);
    let receiver_is_detached =
        document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some();
    set_slot_handle_array(
        scope,
        &mut rv,
        runtime_ptr,
        args.this(),
        &handles,
        receiver_is_detached,
    );
}

pub(in crate::native_bridge) fn slot_assigned_elements_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let handles = runtime
        .dom_host()
        .assigned_nodes_for_slot_with_options(handle, slot_flatten_option(scope, &args))
        .into_iter()
        .filter(|candidate| {
            runtime
                .dom_host()
                .node(*candidate)
                .is_some_and(Node::is_element)
        })
        .collect::<Vec<_>>();
    let receiver_is_detached =
        document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some();
    set_slot_handle_array(
        scope,
        &mut rv,
        runtime_ptr,
        args.this(),
        &handles,
        receiver_is_detached,
    );
}

pub(in crate::native_bridge) fn slot_assign_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, slot_handle)) =
        node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_undefined();
        return;
    };
    let mut handles = Vec::new();
    for index in 0..args.length() {
        let value = args.get(index);
        let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
            throw_type_error(
                scope,
                "HTMLSlotElement.assign arguments must be Element or Text.",
            );
            return;
        };
        let Ok((argument_runtime_ptr, handle)) =
            node_runtime_and_handle_from_object_or_detached(scope, object)
        else {
            throw_type_error(
                scope,
                "HTMLSlotElement.assign arguments must be Element or Text.",
            );
            return;
        };
        if argument_runtime_ptr != runtime_ptr {
            throw_type_error(
                scope,
                "HTMLSlotElement.assign arguments must belong to this realm.",
            );
            return;
        }
        if !unsafe { &*runtime_ptr }
            .dom_host()
            .node(handle)
            .is_some_and(|node| node.is_element() || node.is_text())
        {
            throw_type_error(
                scope,
                "HTMLSlotElement.assign arguments must be Element or Text.",
            );
            return;
        }
        handles.push(handle);
    }
    let slot_assignment_changes = unsafe { &mut *runtime_ptr }
        .dom_host_mut()
        .assign_nodes_to_slot(slot_handle, handles);
    let runtime = unsafe { &mut *runtime_ptr };
    let changed_slots = slot_assignment_changes
        .iter()
        .map(|change| change.slot())
        .collect::<Vec<_>>();
    if !changed_slots.is_empty() {
        let style_effects = slot_assignment_changes
            .iter()
            .map(|change| StyleMutationEffect::SlotAssignment {
                slot: change.slot(),
                previous_assigned_nodes: Some(change.previous_assigned_nodes().to_vec()),
                assigned_nodes: Some(change.assigned_nodes().to_vec()),
            })
            .collect::<Vec<_>>();
        runtime.note_style_mutation_effects(&style_effects);
    }
    runtime.queue_slotchange_events(scope, &changed_slots);
    rv.set_undefined();
}

fn slot_flatten_option(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> bool {
    if args.length() == 0 {
        return false;
    }
    let options = args.get(0);
    if options.is_null_or_undefined() || !options.is_object() {
        return false;
    }
    options.to_object(scope).is_some_and(|options| {
        for name in ["flatten", "flattened"] {
            if options
                .get(scope, v8str(scope, name).into())
                .is_some_and(|value| value.boolean_value(scope))
            {
                return true;
            }
        }
        false
    })
}

fn set_slot_handle_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    runtime_ptr: *mut JsContextHost,
    receiver: v8::Local<'s, v8::Object>,
    handles: &[DomHandle],
    detached_objects: bool,
) {
    let Some(context) = receiver.get_creation_context(scope) else {
        rv.set_null();
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let mut values = Vec::with_capacity(handles.len());
    for handle in handles.iter().copied() {
        let node = if detached_objects {
            document::detached_native_object_for_handle(scope, runtime_ptr, handle)
        } else {
            let runtime = unsafe { &mut *runtime_ptr };
            runtime
                .native_bridge_mut()
                .wrap_handle(scope, runtime_ptr, handle)
        };
        let Some(node) = node else {
            rv.set_null();
            return;
        };
        values.push(v8::Local::<v8::Value>::from(node));
    }
    let array =
        serialize_v8_array(scope, values.as_slice()).unwrap_or_else(|| v8::Array::new(scope, 0));
    rv.set(array.into());
}
