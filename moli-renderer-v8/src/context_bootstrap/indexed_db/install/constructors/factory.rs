use super::*;
use crate::context_bootstrap::indexed_db::{
    IndexedDbRequestReadyState, IndexedDbRequestValue, dom_exception_value,
    indexed_db_request_ready_state, request_value,
};

use crate::util::v8str;
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

pub(super) fn install_factory_template_bindings<'s>(
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
        }
        _ => {}
    }
}

fn idb_request_result_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    request_attribute_getter(scope, args.this(), IndexedDbRequestValue::Result, rv);
}

fn idb_request_error_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    request_attribute_getter(scope, args.this(), IndexedDbRequestValue::Error, rv);
}

fn idb_request_source_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    request_attribute_getter(scope, args.this(), IndexedDbRequestValue::Source, rv);
}

fn idb_request_transaction_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    request_attribute_getter(scope, args.this(), IndexedDbRequestValue::Transaction, rv);
}

fn idb_request_ready_state_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let request = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("IDBRequest receiver was validated");
    if let Some(state) = indexed_db_request_ready_state(scope, request) {
        rv.set(v8str(scope, state.as_str()).into());
    }
}

fn request_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    field: IndexedDbRequestValue,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let request = moli_webapi_declare::web_api_object_target(scope, request)
        .expect("IDBRequest receiver was validated");
    // All five attributes read the native request state, even if script shadows
    // a public property. Only result and error are unavailable while pending.
    if matches!(
        field,
        IndexedDbRequestValue::Result | IndexedDbRequestValue::Error
    ) && indexed_db_request_ready_state(scope, request)
        == Some(IndexedDbRequestReadyState::Pending)
    {
        let error =
            dom_exception_value(scope, "The request has not finished.", "InvalidStateError");
        scope.throw_exception(error);
        return;
    }
    if let Some(value) = request_value(scope, request, field) {
        rv.set(value);
    }
}
