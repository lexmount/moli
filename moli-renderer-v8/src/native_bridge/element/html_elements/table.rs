use crate::custom_elements;
use crate::dom::native::Node;
use crate::util::throw_type_error;
use crate::webidl;

use super::super::super::document::{
    define_detached_native_handle, detached_native_handle_for_runtime,
};
use super::super::super::node::{
    append_child_to_current_reaction_queue, insert_before_to_current_reaction_queue,
    node_or_existing_detached_arg_handle, node_runtime_and_handle_from_object_or_detached,
    remove_child_in_reaction_scope, remove_child_to_current_reaction_queue,
    set_wrapped_node_or_null, throw_incompatible_getter_receiver,
    throw_incompatible_method_receiver, throw_incompatible_setter_receiver,
};
use super::super::super::{
    CollectionKind, LiveCollectionQueryKind, collections::build_live_collection_for_node,
    set_wrapped_handle_or_null, throw_dom_exception,
};
use super::super::set_reflected_attribute;
use super::super::{element_attribute, global_attributes::parse_non_negative_integer};
use super::{DomHandle, JsContextHost};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLTableElement.insertRow")]
struct TableInsertRowArgs {
    #[webidl(with = optional_index_arg)]
    index: Option<i32>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLTableElement.deleteRow")]
struct TableDeleteRowArgs {
    #[webidl(required)]
    index: i32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLTableSectionElement.insertRow")]
struct TableSectionInsertRowArgs {
    #[webidl(with = optional_index_arg)]
    index: Option<i32>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLTableSectionElement.deleteRow")]
struct TableSectionDeleteRowArgs {
    #[webidl(required)]
    index: i32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLTableRowElement.insertCell")]
struct TableRowInsertCellArgs {
    #[webidl(with = optional_index_arg)]
    index: Option<i32>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLTableRowElement.deleteCell")]
struct TableRowDeleteCellArgs {
    #[webidl(required)]
    index: i32,
}

fn optional_index_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<Option<i32>, webidl::WebIdlError> {
    if args.length() <= index {
        return Ok(None);
    }
    webidl::argument::<webidl::Long>(
        scope,
        args,
        index,
        webidl::Context::argument("table mutation", (index + 1) as usize),
    )
    .map(|value| Some(value.0))
}

#[derive(Clone, Copy)]
enum TableReceiverKind {
    Table,
    Section,
    Row,
    Col,
    Cell,
}

fn table_receiver_matches(
    runtime: &JsContextHost,
    handle: DomHandle,
    kind: TableReceiverKind,
) -> bool {
    match kind {
        TableReceiverKind::Table => runtime.dom_host().is_html_element_named(handle, "table"),
        TableReceiverKind::Section => is_html_table_section(runtime, handle),
        TableReceiverKind::Row => runtime.dom_host().is_html_element_named(handle, "tr"),
        TableReceiverKind::Col => ["col", "colgroup"]
            .into_iter()
            .any(|name| runtime.dom_host().is_html_element_named(handle, name)),
        TableReceiverKind::Cell => is_html_table_cell(runtime, handle),
    }
}

fn table_getter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    interface: &'static str,
    member: &'static str,
    kind: TableReceiverKind,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        throw_incompatible_getter_receiver(scope, interface, member);
        return None;
    };
    if !table_receiver_matches(unsafe { &*runtime_ptr }, handle, kind) {
        throw_incompatible_getter_receiver(scope, interface, member);
        return None;
    }
    Some((runtime_ptr, handle))
}

fn table_setter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    interface: &'static str,
    member: &'static str,
    kind: TableReceiverKind,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        throw_incompatible_setter_receiver(scope, interface, member);
        return None;
    };
    if !table_receiver_matches(unsafe { &*runtime_ptr }, handle, kind) {
        throw_incompatible_setter_receiver(scope, interface, member);
        return None;
    }
    Some((runtime_ptr, handle))
}

fn table_method_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    interface: &'static str,
    method: &'static str,
    kind: TableReceiverKind,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        throw_incompatible_method_receiver(scope, interface, method);
        return None;
    };
    if !table_receiver_matches(unsafe { &*runtime_ptr }, handle, kind) {
        throw_incompatible_method_receiver(scope, interface, method);
        return None;
    }
    Some((runtime_ptr, handle))
}

fn html_table_element_getter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    table_getter_receiver(
        scope,
        receiver,
        "HTMLTableElement",
        member,
        TableReceiverKind::Table,
    )
}

fn html_table_element_setter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    table_setter_receiver(
        scope,
        receiver,
        "HTMLTableElement",
        member,
        TableReceiverKind::Table,
    )
}

fn html_table_element_method_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    method: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    table_method_receiver(
        scope,
        receiver,
        "HTMLTableElement",
        method,
        TableReceiverKind::Table,
    )
}

fn html_table_section_method_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    method: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    table_method_receiver(
        scope,
        receiver,
        "HTMLTableSectionElement",
        method,
        TableReceiverKind::Section,
    )
}

fn html_table_row_getter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    table_getter_receiver(
        scope,
        receiver,
        "HTMLTableRowElement",
        member,
        TableReceiverKind::Row,
    )
}

fn html_table_row_method_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    method: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    table_method_receiver(
        scope,
        receiver,
        "HTMLTableRowElement",
        method,
        TableReceiverKind::Row,
    )
}

fn html_table_cell_getter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    table_getter_receiver(
        scope,
        receiver,
        "HTMLTableCellElement",
        member,
        TableReceiverKind::Cell,
    )
}

fn html_table_cell_setter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    table_setter_receiver(
        scope,
        receiver,
        "HTMLTableCellElement",
        member,
        TableReceiverKind::Cell,
    )
}

fn html_table_col_getter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    table_getter_receiver(
        scope,
        receiver,
        "HTMLTableColElement",
        member,
        TableReceiverKind::Col,
    )
}

fn html_table_col_setter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    table_setter_receiver(
        scope,
        receiver,
        "HTMLTableColElement",
        member,
        TableReceiverKind::Col,
    )
}

fn parse_clamped_table_span(
    runtime: &JsContextHost,
    handle: DomHandle,
    attribute: &str,
    default: u32,
    min: u32,
    max: u32,
) -> u32 {
    let Some(raw) = element_attribute(runtime, handle, attribute) else {
        return default;
    };
    parse_non_negative_integer(&raw)
        .map(|value| value.clamp(min, max))
        .unwrap_or(default)
}

fn set_table_unsigned_long_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    attribute: &str,
    value: v8::Local<'s, v8::Value>,
    default: u32,
    interface: &'static str,
    member: &'static str,
) {
    let value = match webidl::convert::<webidl::UnsignedLong>(
        scope,
        value,
        webidl::Context::member(interface, member),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let reflected = if value <= i32::MAX as u32 {
        value
    } else {
        default
    };
    set_reflected_attribute(
        scope,
        runtime_ptr,
        handle,
        attribute,
        &reflected.to_string(),
    );
}

pub(in crate::native_bridge::element) fn table_cell_col_span_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_table_cell_getter_receiver(scope, args.this(), "colSpan")
    else {
        rv.set_uint32(1);
        return;
    };
    rv.set_uint32(parse_clamped_table_span(
        unsafe { &*runtime_ptr },
        handle,
        "colspan",
        1,
        1,
        1000,
    ));
}

pub(in crate::native_bridge::element) fn table_cell_col_span_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_table_cell_setter_receiver(scope, args.this(), "colSpan")
    else {
        return;
    };
    set_table_unsigned_long_attribute(
        scope,
        runtime_ptr,
        handle,
        "colspan",
        args.get(0),
        1,
        "HTMLTableCellElement",
        "colSpan",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge::element) fn table_cell_row_span_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_table_cell_getter_receiver(scope, args.this(), "rowSpan")
    else {
        rv.set_uint32(1);
        return;
    };
    rv.set_uint32(parse_clamped_table_span(
        unsafe { &*runtime_ptr },
        handle,
        "rowspan",
        1,
        0,
        65534,
    ));
}

pub(in crate::native_bridge::element) fn table_cell_row_span_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_table_cell_setter_receiver(scope, args.this(), "rowSpan")
    else {
        return;
    };
    set_table_unsigned_long_attribute(
        scope,
        runtime_ptr,
        handle,
        "rowspan",
        args.get(0),
        1,
        "HTMLTableCellElement",
        "rowSpan",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge::element) fn table_col_span_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = html_table_col_getter_receiver(scope, args.this(), "span")
    else {
        rv.set_uint32(1);
        return;
    };
    rv.set_uint32(parse_clamped_table_span(
        unsafe { &*runtime_ptr },
        handle,
        "span",
        1,
        1,
        1000,
    ));
}

pub(in crate::native_bridge::element) fn table_col_span_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = html_table_col_setter_receiver(scope, args.this(), "span")
    else {
        return;
    };
    set_table_unsigned_long_attribute(
        scope,
        runtime_ptr,
        handle,
        "span",
        args.get(0),
        1,
        "HTMLTableColElement",
        "span",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge::element) fn table_caption_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_table_slot_for_object(scope, args.this(), &mut rv, "caption", "caption");
}

pub(in crate::native_bridge::element) fn table_caption_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_table_slot_on_object(
        scope,
        args.this(),
        args.get(0),
        "caption",
        "caption",
        TableSlotPlacement::FirstChild,
    );
    rv.set_undefined();
}

pub(in crate::native_bridge::element) fn table_t_head_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_table_slot_for_object(scope, args.this(), &mut rv, "thead", "tHead");
}

pub(in crate::native_bridge::element) fn table_t_head_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_table_slot_on_object(
        scope,
        args.this(),
        args.get(0),
        "tHead",
        "thead",
        TableSlotPlacement::Head,
    );
    rv.set_undefined();
}

pub(in crate::native_bridge::element) fn table_t_foot_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_table_slot_for_object(scope, args.this(), &mut rv, "tfoot", "tFoot");
}

pub(in crate::native_bridge::element) fn table_t_foot_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_table_slot_on_object(
        scope,
        args.this(),
        args.get(0),
        "tFoot",
        "tfoot",
        TableSlotPlacement::LastChild,
    );
    rv.set_undefined();
}

pub(in crate::native_bridge::element) fn table_rows_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_table_collection_for_object(
        scope,
        args.this(),
        &mut rv,
        "HTMLTableElement",
        "rows",
        TableReceiverKind::Table,
        LiveCollectionQueryKind::TableRows,
    );
}

pub(in crate::native_bridge::element) fn table_t_bodies_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_table_collection_for_object(
        scope,
        args.this(),
        &mut rv,
        "HTMLTableElement",
        "tBodies",
        TableReceiverKind::Table,
        LiveCollectionQueryKind::TableBodies,
    );
}

pub(in crate::native_bridge::element) fn table_section_rows_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_table_collection_for_object(
        scope,
        args.this(),
        &mut rv,
        "HTMLTableSectionElement",
        "rows",
        TableReceiverKind::Section,
        LiveCollectionQueryKind::TableSectionRows,
    );
}

pub(in crate::native_bridge::element) fn table_row_cells_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_table_collection_for_object(
        scope,
        args.this(),
        &mut rv,
        "HTMLTableRowElement",
        "cells",
        TableReceiverKind::Row,
        LiveCollectionQueryKind::TableRowCells,
    );
}

pub(in crate::native_bridge::element) fn table_row_index_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_table_row_getter_receiver(scope, args.this(), "rowIndex")
    else {
        rv.set_int32(-1);
        return;
    };
    rv.set_int32(table_row_index(unsafe { &*runtime_ptr }, handle));
}

pub(in crate::native_bridge::element) fn table_section_row_index_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_table_row_getter_receiver(scope, args.this(), "sectionRowIndex")
    else {
        rv.set_int32(-1);
        return;
    };
    rv.set_int32(table_section_row_index(unsafe { &*runtime_ptr }, handle));
}

pub(in crate::native_bridge::element) fn table_cell_index_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_table_cell_getter_receiver(scope, args.this(), "cellIndex")
    else {
        rv.set_int32(-1);
        return;
    };
    rv.set_int32(table_cell_index(unsafe { &*runtime_ptr }, handle));
}

pub(in crate::native_bridge::element) fn table_create_caption_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    create_or_return_table_slot(
        scope,
        args,
        &mut rv,
        "createCaption",
        "caption",
        TableSlotPlacement::FirstChild,
    );
}

pub(in crate::native_bridge::element) fn table_delete_caption_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    delete_table_slot(scope, args, "deleteCaption", "caption");
    rv.set_undefined();
}

pub(in crate::native_bridge::element) fn table_create_t_head_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    create_or_return_table_slot(
        scope,
        args,
        &mut rv,
        "createTHead",
        "thead",
        TableSlotPlacement::Head,
    );
}

pub(in crate::native_bridge::element) fn table_delete_t_head_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    delete_table_slot(scope, args, "deleteTHead", "thead");
    rv.set_undefined();
}

pub(in crate::native_bridge::element) fn table_create_t_foot_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    create_or_return_table_slot(
        scope,
        args,
        &mut rv,
        "createTFoot",
        "tfoot",
        TableSlotPlacement::LastChild,
    );
}

pub(in crate::native_bridge::element) fn table_delete_t_foot_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    delete_table_slot(scope, args, "deleteTFoot", "tfoot");
    rv.set_undefined();
}

pub(in crate::native_bridge::element) fn table_create_t_body_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, table)) =
        html_table_element_method_receiver(scope, args.this(), "createTBody")
    else {
        rv.set_null();
        return;
    };
    let tbody = unsafe { &mut *runtime_ptr }.create_element("tbody");
    let reference = create_tbody_reference(unsafe { &*runtime_ptr }, table);
    insert_table_child(scope, runtime_ptr, table, tbody, reference, &mut rv);
}

pub(in crate::native_bridge::element) fn table_insert_row_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, table)) =
        html_table_element_method_receiver(scope, args.this(), "insertRow")
    else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<TableInsertRowArgs>(scope, &args) else {
        return;
    };
    let index = parsed.index.unwrap_or(-1);
    let rows = unsafe { &*runtime_ptr }
        .dom_host()
        .table_row_elements(table);
    let len = rows.len() as i32;
    if index < -1 || index > len {
        throw_index_size_error(scope);
        return;
    }
    let row = unsafe { &mut *runtime_ptr }.create_element("tr");
    if rows.is_empty() {
        let tbody = unsafe { &mut *runtime_ptr }.create_element("tbody");
        let inserted =
            custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
                append_child_to_current_reaction_queue(scope, runtime_ptr, table, tbody)
                    && insert_table_child_to_current_reaction_queue(
                        scope,
                        runtime_ptr,
                        tbody,
                        row,
                        None,
                    )
            });
        if !inserted {
            throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
            return;
        }
        set_wrapped_node_or_null(scope, &mut rv, runtime_ptr, Some(row));
        return;
    }
    let reference = (index != -1 && index < len).then(|| rows[index as usize]);
    let parent = reference
        .or_else(|| rows.last().copied())
        .and_then(|row| unsafe { &*runtime_ptr }.dom_host().node(row))
        .and_then(Node::parent_node)
        .unwrap_or(table);
    insert_table_child(scope, runtime_ptr, parent, row, reference, &mut rv);
}

pub(in crate::native_bridge::element) fn table_delete_row_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, table)) =
        html_table_element_method_receiver(scope, args.this(), "deleteRow")
    else {
        rv.set_undefined();
        return;
    };
    let Some(parsed) = webidl::parse_args::<TableDeleteRowArgs>(scope, &args) else {
        return;
    };
    delete_row_from_handles(
        scope,
        runtime_ptr,
        unsafe { &*runtime_ptr }
            .dom_host()
            .table_row_elements(table),
        parsed.index,
    );
    rv.set_undefined();
}

pub(in crate::native_bridge::element) fn table_section_insert_row_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, section)) =
        html_table_section_method_receiver(scope, args.this(), "insertRow")
    else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<TableSectionInsertRowArgs>(scope, &args) else {
        return;
    };
    let index = parsed.index.unwrap_or(-1);
    let rows = unsafe { &*runtime_ptr }
        .dom_host()
        .table_section_row_elements(section);
    let len = rows.len() as i32;
    if index < -1 || index > len {
        throw_index_size_error(scope);
        return;
    }
    let row = unsafe { &mut *runtime_ptr }.create_element("tr");
    let reference = (index != -1 && index < len).then(|| rows[index as usize]);
    insert_table_child(scope, runtime_ptr, section, row, reference, &mut rv);
}

pub(in crate::native_bridge::element) fn table_section_delete_row_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, section)) =
        html_table_section_method_receiver(scope, args.this(), "deleteRow")
    else {
        rv.set_undefined();
        return;
    };
    let Some(parsed) = webidl::parse_args::<TableSectionDeleteRowArgs>(scope, &args) else {
        return;
    };
    delete_row_from_handles(
        scope,
        runtime_ptr,
        unsafe { &*runtime_ptr }
            .dom_host()
            .table_section_row_elements(section),
        parsed.index,
    );
    rv.set_undefined();
}

pub(in crate::native_bridge::element) fn table_row_insert_cell_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, row)) = html_table_row_method_receiver(scope, args.this(), "insertCell")
    else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<TableRowInsertCellArgs>(scope, &args) else {
        return;
    };
    let index = parsed.index.unwrap_or(-1);
    let cells = unsafe { &*runtime_ptr }
        .dom_host()
        .table_row_cell_elements(row);
    let len = cells.len() as i32;
    if index < -1 || index > len {
        throw_index_size_error(scope);
        return;
    }
    let cell = unsafe { &mut *runtime_ptr }.create_element("td");
    let reference = (index != -1 && index < len).then(|| cells[index as usize]);
    insert_table_child(scope, runtime_ptr, row, cell, reference, &mut rv);
}

pub(in crate::native_bridge::element) fn table_row_delete_cell_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, row)) = html_table_row_method_receiver(scope, args.this(), "deleteCell")
    else {
        rv.set_undefined();
        return;
    };
    let Some(parsed) = webidl::parse_args::<TableRowDeleteCellArgs>(scope, &args) else {
        return;
    };
    let cells = unsafe { &*runtime_ptr }
        .dom_host()
        .table_row_cell_elements(row);
    let len = cells.len() as i32;
    if parsed.index == -1 && len == 0 {
        rv.set_undefined();
        return;
    }
    let index = if parsed.index == -1 {
        len - 1
    } else {
        parsed.index
    };
    if index < 0 || index >= len {
        throw_index_size_error(scope);
        return;
    }
    let cell = cells[index as usize];
    if let Some(parent) = unsafe { &*runtime_ptr }
        .dom_host()
        .node(cell)
        .and_then(Node::parent_node)
    {
        let _ = remove_child_in_reaction_scope(scope, runtime_ptr, parent, cell);
    }
    rv.set_undefined();
}

fn set_table_collection_for_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    rv: &mut v8::ReturnValue<'s, v8::Value>,
    interface: &'static str,
    member: &'static str,
    receiver_kind: TableReceiverKind,
    query_kind: LiveCollectionQueryKind,
) {
    let Some((runtime_ptr, handle)) =
        table_getter_receiver(scope, object, interface, member, receiver_kind)
    else {
        rv.set_null();
        return;
    };
    let collection = build_live_collection_for_node(
        scope,
        runtime_ptr,
        handle,
        CollectionKind::HtmlCollection,
        query_kind,
        None,
        false,
    );
    rv.set(collection.into());
}

#[derive(Clone, Copy)]
enum TableSlotPlacement {
    FirstChild,
    Head,
    LastChild,
}

fn set_table_slot_for_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    rv: &mut v8::ReturnValue<'s, v8::Value>,
    local_name: &str,
    member: &'static str,
) {
    let Some((runtime_ptr, handle)) = html_table_element_getter_receiver(scope, object, member)
    else {
        rv.set_null();
        return;
    };
    let child = first_direct_html_child(unsafe { &*runtime_ptr }, handle, local_name);
    set_wrapped_handle_or_null(scope, rv, runtime_ptr, child);
}

fn set_table_slot_on_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    member: &'static str,
    local_name: &str,
    placement: TableSlotPlacement,
) {
    let Some((runtime_ptr, table)) = html_table_element_setter_receiver(scope, object, member)
    else {
        return;
    };
    set_table_slot_for_handles_preserving_identity(
        scope,
        runtime_ptr,
        table,
        value,
        local_name,
        placement,
    );
}

fn set_table_slot_for_handles_preserving_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    table: DomHandle,
    value: v8::Local<'s, v8::Value>,
    local_name: &str,
    placement: TableSlotPlacement,
) {
    if value.is_null() {
        delete_direct_html_child(scope, runtime_ptr, table, local_name);
        return;
    }
    let Some(child) = table_child_arg_handle(scope, runtime_ptr, value) else {
        throw_type_error(scope, "Assigned table child must be an HTML element.");
        return;
    };
    set_table_slot_child(scope, runtime_ptr, table, child, local_name, placement);
}

fn set_table_slot_child(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    table: DomHandle,
    child: DomHandle,
    local_name: &str,
    placement: TableSlotPlacement,
) {
    if !unsafe { &*runtime_ptr }
        .dom_host()
        .is_html_element_named(child, local_name)
    {
        if matches!(local_name, "thead" | "tfoot")
            && is_html_table_section(unsafe { &*runtime_ptr }, child)
        {
            throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
            return;
        }
        throw_type_error(scope, "Assigned table child has the wrong element type.");
        return;
    }
    let existing = first_direct_html_child(unsafe { &*runtime_ptr }, table, local_name)
        .filter(|existing| *existing != child);
    let inserted =
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            if let Some(existing) = existing {
                let _ = remove_child_to_current_reaction_queue(scope, runtime_ptr, table, existing);
            }
            let reference =
                table_slot_reference(unsafe { &*runtime_ptr }, table, placement, Some(child));
            if let Some(reference) = reference {
                insert_before_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    table,
                    child,
                    Some(reference),
                )
            } else {
                append_child_to_current_reaction_queue(scope, runtime_ptr, table, child)
            }
        });
    if !inserted {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
    }
}

fn table_child_arg_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    value: v8::Local<'s, v8::Value>,
) -> Option<DomHandle> {
    let handle = node_or_existing_detached_arg_handle(scope, runtime_ptr, value)?;
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && detached_native_handle_for_runtime(scope, runtime_ptr, object) == Some(handle)
    {
        define_detached_native_handle(scope, object, handle);
    }
    Some(handle)
}

fn create_or_return_table_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: &mut v8::ReturnValue<'s, v8::Value>,
    method: &'static str,
    local_name: &str,
    placement: TableSlotPlacement,
) {
    let Some((runtime_ptr, table)) = html_table_element_method_receiver(scope, args.this(), method)
    else {
        rv.set_null();
        return;
    };
    if let Some(existing) = first_direct_html_child(unsafe { &*runtime_ptr }, table, local_name) {
        set_wrapped_node_or_null(scope, rv, runtime_ptr, Some(existing));
        return;
    }
    let child = unsafe { &mut *runtime_ptr }.create_element(local_name);
    let reference = table_slot_reference(unsafe { &*runtime_ptr }, table, placement, None);
    insert_table_child(scope, runtime_ptr, table, child, reference, rv);
}

fn delete_table_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    method: &'static str,
    local_name: &str,
) {
    let Some((runtime_ptr, table)) = html_table_element_method_receiver(scope, args.this(), method)
    else {
        return;
    };
    delete_direct_html_child(scope, runtime_ptr, table, local_name);
}

fn delete_direct_html_child(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    table: DomHandle,
    local_name: &str,
) {
    let Some(child) = first_direct_html_child(unsafe { &*runtime_ptr }, table, local_name) else {
        return;
    };
    let _ = remove_child_in_reaction_scope(scope, runtime_ptr, table, child);
}

fn insert_table_child<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    parent: DomHandle,
    child: DomHandle,
    reference: Option<DomHandle>,
    rv: &mut v8::ReturnValue<'s, v8::Value>,
) {
    let inserted =
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            insert_table_child_to_current_reaction_queue(
                scope,
                runtime_ptr,
                parent,
                child,
                reference,
            )
        });
    if !inserted {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    }
    set_wrapped_node_or_null(scope, rv, runtime_ptr, Some(child));
}

fn insert_table_child_to_current_reaction_queue(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    parent: DomHandle,
    child: DomHandle,
    reference: Option<DomHandle>,
) -> bool {
    if let Some(reference) = reference {
        insert_before_to_current_reaction_queue(scope, runtime_ptr, parent, child, Some(reference))
    } else {
        append_child_to_current_reaction_queue(scope, runtime_ptr, parent, child)
    }
}

fn first_direct_html_child(
    runtime: &JsContextHost,
    parent: DomHandle,
    local_name: &str,
) -> Option<DomHandle> {
    runtime.dom_host().find_child(parent, |handle| {
        runtime.dom_host().is_html_element_named(handle, local_name)
    })
}

fn table_slot_reference(
    runtime: &JsContextHost,
    table: DomHandle,
    placement: TableSlotPlacement,
    moving: Option<DomHandle>,
) -> Option<DomHandle> {
    match placement {
        TableSlotPlacement::FirstChild => runtime
            .dom_host()
            .find_child(table, |handle| Some(handle) != moving),
        TableSlotPlacement::LastChild => None,
        TableSlotPlacement::Head => runtime.dom_host().find_child(table, |handle| {
            Some(handle) != moving
                && runtime
                    .dom_host()
                    .node(handle)
                    .is_some_and(Node::is_element)
                && !runtime.dom_host().is_html_element_named(handle, "caption")
                && !runtime.dom_host().is_html_element_named(handle, "colgroup")
        }),
    }
}

fn create_tbody_reference(runtime: &JsContextHost, table: DomHandle) -> Option<DomHandle> {
    let mut reference = None;
    for handle in runtime.dom_host().child_handles(table) {
        let next = runtime.dom_host().next_sibling(handle);
        if runtime.dom_host().is_html_element_named(handle, "tbody") {
            reference = next;
        }
    }
    reference
}

fn delete_row_from_handles(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    rows: Vec<DomHandle>,
    raw_index: i32,
) {
    let len = rows.len() as i32;
    if raw_index == -1 && len == 0 {
        return;
    }
    let index = if raw_index == -1 { len - 1 } else { raw_index };
    if index < 0 || index >= len {
        throw_index_size_error(scope);
        return;
    }
    let row = rows[index as usize];
    if let Some(parent) = unsafe { &*runtime_ptr }
        .dom_host()
        .node(row)
        .and_then(Node::parent_node)
    {
        let _ = remove_child_in_reaction_scope(scope, runtime_ptr, parent, row);
    }
}

fn table_row_index(runtime: &JsContextHost, row: DomHandle) -> i32 {
    let Some(table) = row_table(runtime, row) else {
        return -1;
    };
    runtime
        .dom_host()
        .table_row_elements(table)
        .iter()
        .position(|candidate| *candidate == row)
        .map(|index| index as i32)
        .unwrap_or(-1)
}

fn table_section_row_index(runtime: &JsContextHost, row: DomHandle) -> i32 {
    let Some(parent) = runtime.dom_host().node(row).and_then(Node::parent_node) else {
        return -1;
    };
    let rows = if runtime.dom_host().is_html_element_named(parent, "table") {
        runtime.dom_host().table_row_elements(parent)
    } else if is_html_table_section(runtime, parent)
        && runtime
            .dom_host()
            .node(parent)
            .and_then(Node::parent_node)
            .is_some_and(|table| runtime.dom_host().is_html_element_named(table, "table"))
    {
        runtime.dom_host().table_section_row_elements(parent)
    } else {
        Vec::new()
    };
    rows.iter()
        .position(|candidate| *candidate == row)
        .map(|index| index as i32)
        .unwrap_or(-1)
}

fn table_cell_index(runtime: &JsContextHost, cell: DomHandle) -> i32 {
    runtime
        .dom_host()
        .node(cell)
        .and_then(Node::parent_node)
        .filter(|parent| runtime.dom_host().is_html_element_named(*parent, "tr"))
        .and_then(|row| {
            runtime
                .dom_host()
                .table_row_cell_elements(row)
                .iter()
                .position(|candidate| *candidate == cell)
        })
        .map(|index| index as i32)
        .unwrap_or(-1)
}

fn row_table(runtime: &JsContextHost, row: DomHandle) -> Option<DomHandle> {
    let parent = runtime.dom_host().node(row).and_then(Node::parent_node)?;
    if runtime.dom_host().is_html_element_named(parent, "table") {
        return Some(parent);
    }
    if is_html_table_section(runtime, parent) {
        let table = runtime
            .dom_host()
            .node(parent)
            .and_then(Node::parent_node)?;
        if runtime.dom_host().is_html_element_named(table, "table") {
            return Some(table);
        }
    }
    None
}

fn is_html_table_section(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime.dom_host().is_html_element_named(handle, "thead")
        || runtime.dom_host().is_html_element_named(handle, "tbody")
        || runtime.dom_host().is_html_element_named(handle, "tfoot")
}

fn is_html_table_cell(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime.dom_host().is_html_element_named(handle, "td")
        || runtime.dom_host().is_html_element_named(handle, "th")
}

fn throw_index_size_error(scope: &mut v8::PinScope<'_, '_>) {
    throw_dom_exception(
        scope,
        "IndexSizeError",
        1,
        "The index is not in the allowed range.",
    );
}
