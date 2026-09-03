use super::*;
use crate::util::{callback_data_item, get_private_value, set_private_value};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const FILE_READER_READY_STATE_SLOT: &str = "__lmFileReaderReadyState";
const FILE_READER_RESULT_SLOT: &str = "__lmFileReaderResult";
const FILE_READER_ERROR_SLOT: &str = "__lmFileReaderError";
const FILE_READER_BRAND_SLOT: &str = "__lmFileReaderBrand";
const FILE_READER_ONLOADSTART_SLOT: &str = "__lmFileReaderOnloadstart";
const FILE_READER_ONPROGRESS_SLOT: &str = "__lmFileReaderOnprogress";
const FILE_READER_ONLOAD_SLOT: &str = "__lmFileReaderOnload";
const FILE_READER_ONABORT_SLOT: &str = "__lmFileReaderOnabort";
const FILE_READER_ONERROR_SLOT: &str = "__lmFileReaderOnerror";
const FILE_READER_ONLOADEND_SLOT: &str = "__lmFileReaderOnloadend";

#[derive(Default, WebApiObject)]
#[webapi(interface = "FileReader")]
struct FileReaderObjectDeclaration {
    #[webapi(slot = FILE_READER_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = FILE_READER_LISTENERS_SLOT)]
    event_target_slot: (),

    #[webapi(slot = SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),

    #[webapi(slot = FILE_READER_READY_STATE_SLOT, init = 0)]
    ready_state: (),

    #[webapi(slot = FILE_READER_RESULT_SLOT, init = "null")]
    result: (),

    #[webapi(slot = FILE_READER_ERROR_SLOT, init = "null")]
    error: (),

    #[webapi(slot = FILE_READER_ONLOADSTART_SLOT, init = "null")]
    onloadstart: (),

    #[webapi(slot = FILE_READER_ONPROGRESS_SLOT, init = "null")]
    onprogress: (),

    #[webapi(slot = FILE_READER_ONLOAD_SLOT, init = "null")]
    onload: (),

    #[webapi(slot = FILE_READER_ONABORT_SLOT, init = "null")]
    onabort: (),

    #[webapi(slot = FILE_READER_ONERROR_SLOT, init = "null")]
    onerror: (),

    #[webapi(slot = FILE_READER_ONLOADEND_SLOT, init = "null")]
    onloadend: (),

    #[webapi(slot = FILE_READER_LISTENERS_SLOT, init = "null_object")]
    listeners: (),

    #[webapi(slot = FILE_READER_SCHEDULED_SLOT, init = false)]
    scheduled: (),

    #[webapi(slot = FILE_READER_PENDING_RESULT_SLOT, init = "null")]
    pending_result: (),

    #[webapi(slot = FILE_READER_PENDING_TOTAL_SLOT, init = 0)]
    pending_total: (),

    #[webapi(slot = FILE_READER_READ_ID_SLOT, init = 0)]
    read_id: (),

    #[webapi(slot = FILE_READER_TASK_PHASE_SLOT, init = 0)]
    task_phase: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FileReader")]
struct FileReaderPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = file_reader_ready_state_getter_callback,
        enumerable
    )]
    ready_state: (),

    #[webapi(accessor_property, getter = file_reader_result_getter_callback, enumerable)]
    result: (),

    #[webapi(accessor_property, getter = file_reader_error_getter_callback, enumerable)]
    error: (),

    #[webapi(
        accessor_property,
        getter = file_reader_event_handler_getter_callback,
        setter = file_reader_event_handler_setter_callback,
        data = crate::util::callback_data_index_value(scope, 0),
        enumerable
    )]
    onloadstart: (),

    #[webapi(
        accessor_property,
        getter = file_reader_event_handler_getter_callback,
        setter = file_reader_event_handler_setter_callback,
        data = crate::util::callback_data_index_value(scope, 1),
        enumerable
    )]
    onprogress: (),

    #[webapi(
        accessor_property,
        getter = file_reader_event_handler_getter_callback,
        setter = file_reader_event_handler_setter_callback,
        data = crate::util::callback_data_index_value(scope, 2),
        enumerable
    )]
    onload: (),

    #[webapi(
        accessor_property,
        getter = file_reader_event_handler_getter_callback,
        setter = file_reader_event_handler_setter_callback,
        data = crate::util::callback_data_index_value(scope, 3),
        enumerable
    )]
    onabort: (),

    #[webapi(
        accessor_property,
        getter = file_reader_event_handler_getter_callback,
        setter = file_reader_event_handler_setter_callback,
        data = crate::util::callback_data_index_value(scope, 4),
        enumerable
    )]
    onerror: (),

    #[webapi(
        accessor_property,
        getter = file_reader_event_handler_getter_callback,
        setter = file_reader_event_handler_setter_callback,
        data = crate::util::callback_data_index_value(scope, 5),
        enumerable
    )]
    onloadend: (),
}

#[derive(Clone, Copy)]
struct FileReaderEventHandler {
    event_type: &'static str,
    slot_name: &'static str,
}

const FILE_READER_EVENT_HANDLERS: &[FileReaderEventHandler] = &[
    FileReaderEventHandler {
        event_type: "loadstart",
        slot_name: FILE_READER_ONLOADSTART_SLOT,
    },
    FileReaderEventHandler {
        event_type: "progress",
        slot_name: FILE_READER_ONPROGRESS_SLOT,
    },
    FileReaderEventHandler {
        event_type: "load",
        slot_name: FILE_READER_ONLOAD_SLOT,
    },
    FileReaderEventHandler {
        event_type: "abort",
        slot_name: FILE_READER_ONABORT_SLOT,
    },
    FileReaderEventHandler {
        event_type: "error",
        slot_name: FILE_READER_ONERROR_SLOT,
    },
    FileReaderEventHandler {
        event_type: "loadend",
        slot_name: FILE_READER_ONLOADEND_SLOT,
    },
];

pub(in crate::context_bootstrap::file_api) fn install_file_reader_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name != "FileReader" {
        return;
    }
    FileReaderPrototypeAccessorsDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

fn file_reader_ready_state_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !file_reader_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let ready_state = file_reader_ready_state(scope, args.this());
    rv.set(v8::Number::new(scope, ready_state).into());
}

fn file_reader_result_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !file_reader_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let result = args.this();
    let result = file_reader_slot_value(scope, result, FILE_READER_RESULT_SLOT)
        .unwrap_or_else(|| v8::null(scope).into());
    rv.set(result);
}

fn file_reader_error_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !file_reader_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let error = args.this();
    let error = file_reader_slot_value(scope, error, FILE_READER_ERROR_SLOT)
        .unwrap_or_else(|| v8::null(scope).into());
    rv.set(error);
}

fn file_reader_event_handler_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !file_reader_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(handler) = callback_data_item(
        scope,
        &args,
        FILE_READER_EVENT_HANDLERS,
        "FileReader event handlers",
    ) else {
        rv.set_null();
        return;
    };
    let value = get_private_value(scope, args.this(), handler.slot_name)
        .unwrap_or_else(|| v8::null(scope).into());
    if value.is_null_or_undefined() {
        rv.set_null();
    } else {
        rv.set(value);
    }
}

fn file_reader_event_handler_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !file_reader_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(handler) = callback_data_item(
        scope,
        &args,
        FILE_READER_EVENT_HANDLERS,
        "FileReader event handlers",
    ) else {
        return;
    };
    let value = args.get(0);
    let stored = if value.is_function() {
        value
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, args.this(), handler.slot_name, stored);
    simple_object_event_set_ordered_handler(
        scope,
        args.this(),
        FILE_READER_LISTENERS_SLOT,
        handler.event_type,
        handler.slot_name,
        stored.is_function(),
    );
}

pub(in crate::context_bootstrap::file_api::file_reader) fn initialize_file_reader_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
) {
    FileReaderObjectDeclaration::default()
        .initialize(scope, reader)
        .expect("FileReader declaration should initialize");
}

pub(in crate::context_bootstrap::file_api::file_reader) fn file_reader_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, receiver, FILE_READER_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

pub(in crate::context_bootstrap::file_api::file_reader) fn file_reader_ready_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
) -> f64 {
    file_reader_slot_number(scope, reader, FILE_READER_READY_STATE_SLOT).unwrap_or(0.0)
}

pub(in crate::context_bootstrap::file_api::file_reader) fn set_file_reader_ready_state(
    scope: &mut v8::PinScope<'_, '_>,
    reader: v8::Local<'_, v8::Object>,
    ready_state: f64,
) {
    set_file_reader_number_slot(scope, reader, FILE_READER_READY_STATE_SLOT, ready_state);
}

pub(in crate::context_bootstrap::file_api::file_reader) fn set_file_reader_result(
    scope: &mut v8::PinScope<'_, '_>,
    reader: v8::Local<'_, v8::Object>,
    result: v8::Local<'_, v8::Value>,
) {
    set_file_reader_slot_value(scope, reader, FILE_READER_RESULT_SLOT, result);
}

pub(in crate::context_bootstrap::file_api::file_reader) fn set_file_reader_error(
    scope: &mut v8::PinScope<'_, '_>,
    reader: v8::Local<'_, v8::Object>,
    error: v8::Local<'_, v8::Value>,
) {
    set_file_reader_slot_value(scope, reader, FILE_READER_ERROR_SLOT, error);
}

pub(in crate::context_bootstrap::file_api::file_reader) fn file_reader_read_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
) -> f64 {
    file_reader_slot_number(scope, reader, FILE_READER_READ_ID_SLOT).unwrap_or(0.0)
}

pub(in crate::context_bootstrap::file_api::file_reader) fn set_file_reader_read_id(
    scope: &mut v8::PinScope<'_, '_>,
    reader: v8::Local<'_, v8::Object>,
    read_id: f64,
) {
    set_file_reader_number_slot(scope, reader, FILE_READER_READ_ID_SLOT, read_id);
}

pub(in crate::context_bootstrap::file_api::file_reader) fn file_reader_pending_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    file_reader_slot_value(scope, reader, FILE_READER_PENDING_RESULT_SLOT)
}

pub(in crate::context_bootstrap::file_api::file_reader) fn set_file_reader_pending_result(
    scope: &mut v8::PinScope<'_, '_>,
    reader: v8::Local<'_, v8::Object>,
    result: v8::Local<'_, v8::Value>,
) {
    set_file_reader_slot_value(scope, reader, FILE_READER_PENDING_RESULT_SLOT, result);
}

pub(in crate::context_bootstrap::file_api::file_reader) fn file_reader_pending_total<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
) -> f64 {
    file_reader_slot_number(scope, reader, FILE_READER_PENDING_TOTAL_SLOT).unwrap_or(0.0)
}

pub(in crate::context_bootstrap::file_api::file_reader) fn set_file_reader_pending_total(
    scope: &mut v8::PinScope<'_, '_>,
    reader: v8::Local<'_, v8::Object>,
    total: f64,
) {
    set_file_reader_number_slot(scope, reader, FILE_READER_PENDING_TOTAL_SLOT, total);
}

pub(in crate::context_bootstrap::file_api::file_reader) fn file_reader_task_phase<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
) -> f64 {
    file_reader_slot_number(scope, reader, FILE_READER_TASK_PHASE_SLOT).unwrap_or(0.0)
}

pub(in crate::context_bootstrap::file_api::file_reader) fn set_file_reader_task_phase(
    scope: &mut v8::PinScope<'_, '_>,
    reader: v8::Local<'_, v8::Object>,
    phase: f64,
) {
    set_file_reader_number_slot(scope, reader, FILE_READER_TASK_PHASE_SLOT, phase);
}

pub(in crate::context_bootstrap::file_api::file_reader) fn file_reader_scheduled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
) -> bool {
    file_reader_slot_bool(scope, reader, FILE_READER_SCHEDULED_SLOT).unwrap_or(false)
}

pub(in crate::context_bootstrap::file_api::file_reader) fn set_file_reader_scheduled(
    scope: &mut v8::PinScope<'_, '_>,
    reader: v8::Local<'_, v8::Object>,
    scheduled: bool,
) {
    set_file_reader_bool_slot(scope, reader, FILE_READER_SCHEDULED_SLOT, scheduled);
}

fn file_reader_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, reader, slot)
}

fn file_reader_slot_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<f64> {
    file_reader_slot_value(scope, reader, slot).and_then(|value| value.number_value(scope))
}

fn file_reader_slot_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<bool> {
    file_reader_slot_value(scope, reader, slot).map(|value| value.boolean_value(scope))
}

fn set_file_reader_slot_value(
    scope: &mut v8::PinScope<'_, '_>,
    reader: v8::Local<'_, v8::Object>,
    slot: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    set_private_value(scope, reader, slot, value);
}

fn set_file_reader_number_slot(
    scope: &mut v8::PinScope<'_, '_>,
    reader: v8::Local<'_, v8::Object>,
    slot: &'static str,
    value: f64,
) {
    let value = v8::Number::new(scope, value);
    set_file_reader_slot_value(scope, reader, slot, value.into());
}

fn set_file_reader_bool_slot(
    scope: &mut v8::PinScope<'_, '_>,
    reader: v8::Local<'_, v8::Object>,
    slot: &'static str,
    value: bool,
) {
    let value = v8::Boolean::new(scope, value);
    set_file_reader_slot_value(scope, reader, slot, value.into());
}
