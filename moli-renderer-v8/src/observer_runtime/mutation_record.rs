//! Immutable MutationRecord payloads and WebIDL prototype attributes.

use super::{QueuedMutationRecord, QueuedMutationRecordKind, wrap_node_handle};
use crate::{
    native_bridge::{JsContextHost, build_static_handle_node_list_wrapper},
    util::{callback_data_index_value, callback_data_item, get_private_value, v8_string, v8str},
    web_api_interfaces,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, web_api_object_target};

const TYPE_SLOT: &str = "__moliMutationRecordType";
const TARGET_SLOT: &str = "__moliMutationRecordTarget";
const ADDED_NODES_SLOT: &str = "__moliMutationRecordAddedNodes";
const REMOVED_NODES_SLOT: &str = "__moliMutationRecordRemovedNodes";
const PREVIOUS_SIBLING_SLOT: &str = "__moliMutationRecordPreviousSibling";
const NEXT_SIBLING_SLOT: &str = "__moliMutationRecordNextSibling";
const ATTRIBUTE_NAME_SLOT: &str = "__moliMutationRecordAttributeName";
const ATTRIBUTE_NAMESPACE_SLOT: &str = "__moliMutationRecordAttributeNamespace";
const OLD_VALUE_SLOT: &str = "__moliMutationRecordOldValue";

const ATTRIBUTE_SLOTS: &[&str] = &[
    TYPE_SLOT,
    TARGET_SLOT,
    ADDED_NODES_SLOT,
    REMOVED_NODES_SLOT,
    PREVIOUS_SIBLING_SLOT,
    NEXT_SIBLING_SLOT,
    ATTRIBUTE_NAME_SLOT,
    ATTRIBUTE_NAMESPACE_SLOT,
    OLD_VALUE_SLOT,
];

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MutationRecord)]
struct MutationRecordDeclaration<'s> {
    #[webapi(slot = TYPE_SLOT)]
    record_type: v8::Local<'s, v8::Value>,
    #[webapi(slot = TARGET_SLOT)]
    target: v8::Local<'s, v8::Value>,
    #[webapi(slot = ADDED_NODES_SLOT)]
    added_nodes: v8::Local<'s, v8::Value>,
    #[webapi(slot = REMOVED_NODES_SLOT)]
    removed_nodes: v8::Local<'s, v8::Value>,
    #[webapi(slot = PREVIOUS_SIBLING_SLOT)]
    previous_sibling: v8::Local<'s, v8::Value>,
    #[webapi(slot = NEXT_SIBLING_SLOT)]
    next_sibling: v8::Local<'s, v8::Value>,
    #[webapi(slot = ATTRIBUTE_NAME_SLOT)]
    attribute_name: v8::Local<'s, v8::Value>,
    #[webapi(slot = ATTRIBUTE_NAMESPACE_SLOT)]
    attribute_namespace: v8::Local<'s, v8::Value>,
    #[webapi(slot = OLD_VALUE_SLOT)]
    old_value: v8::Local<'s, v8::Value>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MutationRecord, enumerable, receiver)]
struct MutationRecordPrototypeDeclaration {
    #[webapi(accessor_property = "type", getter = attribute_getter, data = callback_data_index_value(scope, 0))]
    record_type: (),
    #[webapi(accessor_property, getter = attribute_getter, data = callback_data_index_value(scope, 1))]
    target: (),
    #[webapi(accessor_property, getter = attribute_getter, data = callback_data_index_value(scope, 2))]
    added_nodes: (),
    #[webapi(accessor_property, getter = attribute_getter, data = callback_data_index_value(scope, 3))]
    removed_nodes: (),
    #[webapi(accessor_property, getter = attribute_getter, data = callback_data_index_value(scope, 4))]
    previous_sibling: (),
    #[webapi(accessor_property, getter = attribute_getter, data = callback_data_index_value(scope, 5))]
    next_sibling: (),
    #[webapi(accessor_property, getter = attribute_getter, data = callback_data_index_value(scope, 6))]
    attribute_name: (),
    #[webapi(accessor_property, getter = attribute_getter, data = callback_data_index_value(scope, 7))]
    attribute_namespace: (),
    #[webapi(accessor_property, getter = attribute_getter, data = callback_data_index_value(scope, 8))]
    old_value: (),
}

pub(crate) fn install_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    MutationRecordPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

fn attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(scope, &args, ATTRIBUTE_SLOTS, "MutationRecord attributes")
    else {
        return;
    };
    let target = web_api_object_target(scope, args.this())
        .expect("validated MutationRecord receiver has native backing");
    rv.set(
        get_private_value(scope, target, slot)
            .expect("initialized MutationRecord retains its private attributes"),
    );
}

pub(super) fn build_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    record: &QueuedMutationRecord,
) -> Option<v8::Local<'s, v8::Object>> {
    let target = wrap_node_handle(scope, host_ptr, record.target)?;
    let (
        record_type,
        attribute_name,
        attribute_namespace,
        old_value,
        added_nodes,
        removed_nodes,
        previous_sibling,
        next_sibling,
    ) = match &record.kind {
        QueuedMutationRecordKind::Attributes {
            attribute_name,
            attribute_namespace,
            old_value,
        } => {
            let attribute_namespace_value = attribute_namespace
                .as_ref()
                .and_then(|namespace| v8_string(scope, namespace))
                .map(v8::Local::<v8::Value>::from)
                .unwrap_or_else(|| v8::null(scope).into());
            let added_nodes = build_static_handle_node_list_wrapper(scope, host_ptr, &[]);
            let removed_nodes = build_static_handle_node_list_wrapper(scope, host_ptr, &[]);
            (
                v8str(scope, "attributes").into(),
                v8_string(scope, attribute_name)?.into(),
                attribute_namespace_value,
                old_value
                    .as_ref()
                    .and_then(|value| v8_string(scope, value))
                    .map(v8::Local::<v8::Value>::from)
                    .unwrap_or_else(|| v8::null(scope).into()),
                added_nodes.into(),
                removed_nodes.into(),
                v8::null(scope).into(),
                v8::null(scope).into(),
            )
        }
        QueuedMutationRecordKind::CharacterData { old_value } => {
            let added_nodes = build_static_handle_node_list_wrapper(scope, host_ptr, &[]);
            let removed_nodes = build_static_handle_node_list_wrapper(scope, host_ptr, &[]);
            (
                v8str(scope, "characterData").into(),
                v8::null(scope).into(),
                v8::null(scope).into(),
                old_value
                    .as_ref()
                    .and_then(|value| {
                        v8::String::new_from_two_byte(
                            scope,
                            &value.utf16_units(),
                            v8::NewStringType::Normal,
                        )
                    })
                    .map(v8::Local::<v8::Value>::from)
                    .unwrap_or_else(|| v8::null(scope).into()),
                added_nodes.into(),
                removed_nodes.into(),
                v8::null(scope).into(),
                v8::null(scope).into(),
            )
        }
        QueuedMutationRecordKind::ChildList {
            added_nodes,
            removed_nodes,
            previous_sibling,
            next_sibling,
        } => {
            let added_nodes = build_static_handle_node_list_wrapper(scope, host_ptr, added_nodes);
            let removed_nodes =
                build_static_handle_node_list_wrapper(scope, host_ptr, removed_nodes);
            let previous_sibling = previous_sibling
                .and_then(|handle| wrap_node_handle(scope, host_ptr, handle))
                .map(v8::Local::<v8::Value>::from)
                .unwrap_or_else(|| v8::null(scope).into());
            let next_sibling = next_sibling
                .and_then(|handle| wrap_node_handle(scope, host_ptr, handle))
                .map(v8::Local::<v8::Value>::from)
                .unwrap_or_else(|| v8::null(scope).into());
            (
                v8str(scope, "childList").into(),
                v8::null(scope).into(),
                v8::null(scope).into(),
                v8::null(scope).into(),
                added_nodes.into(),
                removed_nodes.into(),
                previous_sibling,
                next_sibling,
            )
        }
    };
    MutationRecordDeclaration {
        target: target.into(),
        record_type,
        attribute_name,
        attribute_namespace,
        old_value,
        added_nodes,
        removed_nodes,
        previous_sibling,
        next_sibling,
    }
    .bind(scope)
    .ok()
}
