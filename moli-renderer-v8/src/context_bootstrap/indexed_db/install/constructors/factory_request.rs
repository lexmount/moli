use super::*;
use crate::context_bootstrap::indexed_db::{
    INDEXED_DB_REQUEST_ERROR_SLOT, INDEXED_DB_REQUEST_READY_STATE_SLOT,
    INDEXED_DB_REQUEST_RESULT_SLOT, INDEXED_DB_REQUEST_SOURCE_SLOT,
    INDEXED_DB_REQUEST_TRANSACTION_SLOT, dom_exception_value, object_hidden_value,
    object_string_property,
};
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBFactory, enumerable, receiver)]
struct IdbFactoryPrototypeDeclaration {
    #[webapi(method, length = 1, callback = idb_factory_open_callback)]
    open: (),
    #[webapi(method, length = 1, callback = idb_factory_delete_database_callback)]
    delete_database: (),
    #[webapi(method, length = 0, callback = idb_factory_databases_callback, returns_promise)]
    databases: (),
    #[webapi(method, length = 2, callback = idb_factory_cmp_callback)]
    cmp: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBRequest, enumerable, receiver)]
struct IdbRequestPrototypeDeclaration {
    #[webapi(accessor_property, getter = idb_request_result_getter)]
    result: (),
    #[webapi(accessor_property, getter = idb_request_error_getter)]
    error: (),
    #[webapi(accessor_property, getter = idb_request_source_getter)]
    source: (),
    #[webapi(accessor_property, getter = idb_request_transaction_getter)]
    transaction: (),
    #[webapi(accessor_property, getter = idb_request_ready_state_getter)]
    ready_state: (),
}

pub(super) fn install_factory_and_request_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface_name: &str,
) {
    match interface_name {
        "IDBFactory" => {
            IdbFactoryPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "IDBRequest" => {
            IdbRequestPrototypeDeclaration::initialize_prototype_template(scope, prototype);
            install_idb_event_target_methods(scope, prototype);
        }
        _ => {}
    }
}

fn idb_request_result_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    request_attribute_getter(scope, args.this(), INDEXED_DB_REQUEST_RESULT_SLOT, rv);
}

fn idb_request_error_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    request_attribute_getter(scope, args.this(), INDEXED_DB_REQUEST_ERROR_SLOT, rv);
}

fn idb_request_source_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    request_attribute_getter(scope, args.this(), INDEXED_DB_REQUEST_SOURCE_SLOT, rv);
}

fn idb_request_transaction_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    request_attribute_getter(scope, args.this(), INDEXED_DB_REQUEST_TRANSACTION_SLOT, rv);
}

fn idb_request_ready_state_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    request_attribute_getter(scope, args.this(), INDEXED_DB_REQUEST_READY_STATE_SLOT, rv);
}

fn request_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    slot: &'static str,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    // All five attributes read the native request state, even if script shadows
    // a public property. Only result and error are unavailable while pending.
    if matches!(
        slot,
        INDEXED_DB_REQUEST_RESULT_SLOT | INDEXED_DB_REQUEST_ERROR_SLOT
    ) && object_string_property(scope, request, INDEXED_DB_REQUEST_READY_STATE_SLOT).as_deref()
        == Some("pending")
    {
        let error =
            dom_exception_value(scope, "The request has not finished.", "InvalidStateError");
        scope.throw_exception(error);
        return;
    }
    if let Some(value) = object_hidden_value(scope, request, slot) {
        rv.set(value);
    }
}
