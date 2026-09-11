use super::range_algorithms::range_selection_string_contents;
use super::selection::{
    new_selection_runtime_object, selection_bind_owner_document, selection_owner_document,
    selection_range,
};
use super::selection_callbacks::{
    selection_add_range_callback, selection_attribute_getter_callback, selection_collapse_callback,
    selection_collapse_to_end_callback, selection_collapse_to_start_callback,
    selection_contains_node_callback, selection_delete_from_document_callback,
    selection_extend_callback, selection_get_composed_ranges_callback,
    selection_get_range_at_callback, selection_modify_callback,
    selection_remove_all_ranges_callback, selection_remove_range_callback,
    selection_select_all_children_callback, selection_set_base_and_extent_callback,
    selection_set_position_callback,
};
use super::*;
use crate::util::{callback_data_index_value, get_private_value, set_private_value};
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Selection, enumerable)]
struct SelectionPrototypeMethodsDeclaration {
    #[webapi(method, length = 1, callback = selection_get_range_at_callback)]
    get_range_at: (),
    #[webapi(method, length = 1, callback = selection_add_range_callback)]
    add_range: (),
    #[webapi(method, length = 1, callback = selection_remove_range_callback)]
    remove_range: (),
    #[webapi(method, length = 0, callback = selection_remove_all_ranges_callback)]
    remove_all_ranges: (),
    #[webapi(method, length = 0, callback = selection_remove_all_ranges_callback)]
    empty: (),
    #[webapi(method, length = 1, callback = selection_collapse_callback)]
    collapse: (),
    #[webapi(method, length = 1, callback = selection_set_position_callback)]
    set_position: (),
    #[webapi(method, length = 0, callback = selection_collapse_to_start_callback)]
    collapse_to_start: (),
    #[webapi(method, length = 0, callback = selection_collapse_to_end_callback)]
    collapse_to_end: (),
    #[webapi(method, length = 1, callback = selection_extend_callback)]
    extend: (),
    #[webapi(method, length = 1, callback = selection_select_all_children_callback)]
    select_all_children: (),
    #[webapi(method, length = 4, callback = selection_set_base_and_extent_callback)]
    set_base_and_extent: (),
    #[webapi(method, length = 1, callback = selection_contains_node_callback)]
    contains_node: (),
    #[webapi(method, length = 0, callback = selection_delete_from_document_callback)]
    delete_from_document: (),
    #[webapi(method, length = 0, callback = selection_modify_callback)]
    modify: (),
    #[webapi(method, length = 0, callback = selection_get_composed_ranges_callback)]
    get_composed_ranges: (),
    #[webapi(method, length = 0, callback = selection_to_string_callback)]
    to_string: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Selection)]
struct SelectionPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = selection_attribute_getter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    anchor_node: (),
    #[webapi(
        accessor_property,
        getter = selection_attribute_getter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    anchor_offset: (),
    #[webapi(
        accessor_property,
        getter = selection_attribute_getter_callback,
        data = callback_data_index_value(scope, 2),
        enumerable
    )]
    focus_node: (),
    #[webapi(
        accessor_property,
        getter = selection_attribute_getter_callback,
        data = callback_data_index_value(scope, 3),
        enumerable
    )]
    focus_offset: (),
    #[webapi(
        accessor_property,
        getter = selection_attribute_getter_callback,
        data = callback_data_index_value(scope, 4),
        enumerable
    )]
    is_collapsed: (),
    #[webapi(
        accessor_property,
        getter = selection_attribute_getter_callback,
        data = callback_data_index_value(scope, 5),
        enumerable
    )]
    range_count: (),
    #[webapi(
        accessor_property = "type",
        getter = selection_attribute_getter_callback,
        data = callback_data_index_value(scope, 6),
        enumerable
    )]
    type_: (),
    #[webapi(
        accessor_property,
        getter = selection_attribute_getter_callback,
        data = callback_data_index_value(scope, 7),
        enumerable
    )]
    direction: (),
}

pub(super) fn install_selection_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name != "Selection" {
        return;
    }
    let prototype = template.prototype_template(scope);
    SelectionPrototypeMethodsDeclaration::initialize_prototype_template(scope, prototype);
    SelectionPrototypeAccessorsDeclaration::initialize_prototype_template(scope, prototype);
}

fn selection_to_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(range) = selection_range(scope, args.this()) else {
        rv.set(v8str(scope, "").into());
        return;
    };
    let value = range_selection_string_contents(scope, range).unwrap_or_default();
    match v8_string(scope, &value) {
        Some(value) => rv.set(value.into()),
        None => rv.set(v8str(scope, "").into()),
    }
}

pub(super) fn window_get_selection_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(selection) = selection_value_for_window(scope, args.this()) {
        rv.set(selection.into());
    } else {
        rv.set(v8::undefined(scope).into());
    }
}

pub(super) fn document_get_selection_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(default_view) = args.this().get(scope, v8str(scope, "defaultView").into()) else {
        return;
    };
    if default_view.is_null_or_undefined() {
        rv.set(v8::null(scope).into());
        return;
    }
    let Ok(window) = v8::Local::<v8::Object>::try_from(default_view) else {
        rv.set(v8::null(scope).into());
        return;
    };
    if let Some(selection) = selection_value_for_window(scope, window) {
        rv.set(selection.into());
    } else {
        rv.set(v8::null(scope).into());
    }
}

fn new_selection_object<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    new_selection_runtime_object(scope)
}

pub(crate) fn selection_value_for_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(selection) = get_private_value(scope, window, WINDOW_SELECTION_SLOT)
        .and_then(|value| (!value.is_null_or_undefined()).then_some(value))
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        if selection_owner_document(scope, selection).is_none()
            && let Some(document) = object_property_as_object(scope, window, "document")
        {
            selection_bind_owner_document(scope, selection, document);
        }
        return Some(selection);
    }
    let selection = new_selection_object(scope);
    if let Some(document) = object_property_as_object(scope, window, "document") {
        selection_bind_owner_document(scope, selection, document);
    }
    set_private_value(scope, window, WINDOW_SELECTION_SLOT, selection.into());
    Some(selection)
}

pub(crate) fn sync_selection_owner_document_for_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    document: v8::Local<'s, v8::Object>,
) {
    let Some(selection) = get_private_value(scope, window, WINDOW_SELECTION_SLOT)
        .and_then(|value| (!value.is_null_or_undefined()).then_some(value))
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    selection_bind_owner_document(scope, selection, document);
}
