use super::*;
use crate::util::{get_private_value, set_private_value};
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const FILE_READER_READY_STATE_SLOT: &str = "__lmFileReaderReadyState";
const FILE_READER_RESULT_SLOT: &str = "__lmFileReaderResult";
const FILE_READER_ERROR_SLOT: &str = "__lmFileReaderError";

#[derive(Default, WebApiObject)]
#[webapi(interface = web_api_interfaces::FileReader)]
struct FileReaderObjectDeclaration {
    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = FILE_READER_LISTENERS_SLOT)]
    event_target_slot: (),

    #[webapi(slot = FILE_READER_READY_STATE_SLOT, init = 0)]
    ready_state: (),

    #[webapi(slot = FILE_READER_RESULT_SLOT, init = "null")]
    result: (),

    #[webapi(slot = FILE_READER_ERROR_SLOT, init = "null")]
    error: (),

    #[webapi(data_property = "onloadstart", init = "null")]
    onloadstart: (),

    #[webapi(data_property = "onprogress", init = "null")]
    onprogress: (),

    #[webapi(data_property = "onload", init = "null")]
    onload: (),

    #[webapi(data_property = "onloadend", init = "null")]
    onloadend: (),

    #[webapi(data_property = "onerror", init = "null")]
    onerror: (),

    #[webapi(data_property = "onabort", init = "null")]
    onabort: (),

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
#[webapi(interface = web_api_interfaces::FileReader)]
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
}

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
    let ready_state = file_reader_ready_state(scope, args.this());
    rv.set(v8::Number::new(scope, ready_state).into());
}

fn file_reader_result_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
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
    let error = args.this();
    let error = file_reader_slot_value(scope, error, FILE_READER_ERROR_SLOT)
        .unwrap_or_else(|| v8::null(scope).into());
    rv.set(error);
}

pub(in crate::context_bootstrap::file_api::file_reader) fn initialize_file_reader_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
) {
    FileReaderObjectDeclaration::default()
        .initialize(scope, reader)
        .expect("FileReader declaration should initialize");
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
