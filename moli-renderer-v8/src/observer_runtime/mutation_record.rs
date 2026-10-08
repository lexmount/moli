//! Immutable MutationRecord payloads and WebIDL prototype attributes.

use super::{QueuedMutationRecord, QueuedMutationRecordKind, wrap_node_handle};
use crate::native_bridge::{PageScriptHost, build_static_handle_node_list_wrapper};
use moli_v8_util::{v8_string, v8str};
use moli_webapi::mutation_record::MutationRecordDeclaration;
pub(crate) use moli_webapi::mutation_record::install_template_bindings;
pub(super) fn build_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut PageScriptHost,
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
                    .and_then(|value| crate::util::v8_string_from_dom_string_value(scope, value))
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
