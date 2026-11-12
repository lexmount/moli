use crate::document_runtime::DomHandle;
use crate::native_bridge::collections;
use crate::native_bridge::element::element_attribute_for_object;
use crate::native_bridge::identity::{CollectionKind, LiveCollectionQueryKind};
use crate::native_bridge::named_access::{
    LegacyNamedAccessKind, dom_element_name_attribute_is_exposed,
};
use crate::native_bridge::node::node_runtime_and_handle_from_object;
use crate::util::v8_string;
use moli_webapi_declare::ObjectLiteralDeclaration;

use super::super::super::super::PageScriptHost;

pub(super) fn document_all_items_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    document_handle: DomHandle,
) -> Option<v8::Local<'s, v8::Array>> {
    let runtime = unsafe { &mut *runtime_ptr };
    let handles =
        runtime
            .dom_host()
            .resolve_live_collection(document_handle, "tagName", Some("*"), true)?;
    let array = v8::Array::new(scope, handles.len() as i32);
    let mut visible_index = 0u32;
    for handle in handles {
        let Some(value) = runtime
            .native_bridge_mut()
            .wrap_handle(scope, runtime_ptr, handle)
            .map(Into::<v8::Local<'_, v8::Value>>::into)
        else {
            continue;
        };
        let _ = array.set_index(scope, visible_index, value);
        visible_index += 1;
    }
    Some(array)
}

pub(super) fn document_all_named_lookup<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    document_handle: DomHandle,
    items: v8::Local<'s, v8::Array>,
) -> v8::Local<'s, v8::Object> {
    let lookup = ObjectLiteralDeclaration::bind(scope);
    let length = items.length();
    for index in 0..length {
        let Some(item) = items.get_index(scope, index) else {
            continue;
        };
        let Ok(item) = v8::Local::<v8::Object>::try_from(item) else {
            continue;
        };
        for attribute_name in ["id", "name"] {
            if attribute_name == "name"
                && !node_runtime_and_handle_from_object(scope, item)
                    .ok()
                    .is_some_and(|(runtime_ptr, handle)| {
                        let dom = unsafe { &*runtime_ptr }.dom_host();
                        dom_element_name_attribute_is_exposed(
                            dom,
                            handle,
                            LegacyNamedAccessKind::DocumentAll,
                        )
                    })
            {
                continue;
            }
            let Some(key_text) = element_attribute_for_object(scope, item, attribute_name) else {
                continue;
            };
            if key_text.is_empty() {
                continue;
            }
            let Some(key) = v8_string(scope, &key_text) else {
                continue;
            };
            let lookup_object = lookup.as_object();
            let existing = lookup_object
                .get(scope, key.into())
                .filter(|existing| !existing.is_null_or_undefined());
            if !lookup_object
                .has_own_property(scope, key.into())
                .unwrap_or(false)
                && existing.is_some()
            {
                // Prototype members are not entries in the backing named
                // table; only own properties represent earlier matches.
                continue;
            }
            let Some(existing) = existing else {
                lookup.set_value_property(scope, key.into(), item.into());
                continue;
            };
            if existing.strict_equals(item.into()) {
                continue;
            }
            let collection = collections::build_live_collection_for_node(
                scope,
                runtime_ptr,
                document_handle,
                CollectionKind::HtmlCollection,
                LiveCollectionQueryKind::DocumentAllNamedItems,
                Some(key_text),
                false,
            );
            lookup.set_value_property(scope, key.into(), collection.into());
        }
    }
    lookup.into_object()
}
