use super::shared::{STYLE_SHEETS_SLOT, object_bool_property};
use crate::{
    context_bootstrap::set_style_sheet_list_contents,
    document_runtime::DomHandle,
    dom::native::DomHost,
    native_bridge::document::detached_native_object_for_handle,
    util::{
        context_host_ptr_from_global_bridge, node_wrapper_from_handle, object_property_as_object,
    },
};

pub(super) fn sync_document_style_sheets<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<'s, v8::Object>,
    dom_host: &DomHost,
    handle: DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    let list = object_property_as_object(scope, holder, STYLE_SHEETS_SLOT)?;
    let mut sheets = Vec::new();
    let document_is_connected = dom_host.is_connected(handle);

    let candidates = dom_host.stylesheet_candidate_handles_for_tree_scope(handle);
    for sheet_handle in candidates.iter().copied() {
        let Some(node) = dom_host.node(sheet_handle) else {
            continue;
        };
        let Some(element) = node.as_element() else {
            continue;
        };
        if !crate::style_engine::stylesheet_owner_can_have_sheet(element) {
            continue;
        }
        let wrapper = if document_is_connected {
            node_wrapper_from_handle(scope, sheet_handle)
        } else {
            context_host_ptr_from_global_bridge(scope).and_then(|runtime_ptr| {
                detached_native_object_for_handle(scope, runtime_ptr, sheet_handle)
            })
        };
        let Some(wrapper) = wrapper else {
            continue;
        };
        if element.local_name() == "link"
            && object_bool_property(scope, wrapper, "disabled").unwrap_or(false)
        {
            continue;
        }
        let Some(sheet) = crate::native_bridge::element::style_sheet_for_element(scope, wrapper)
        else {
            continue;
        };
        if sheet.is_null_or_undefined() {
            continue;
        }
        sheets.push(sheet.into());
    }

    set_style_sheet_list_contents(scope, list, &sheets);
    Some(list)
}
