use super::{Key, key_to_js_value, require_idb_key};
use crate::{util::get_private_value, web_api_interfaces};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const KEY: &str = "__moli_idb_record_key";
const PRIMARY_KEY: &str = "__moli_idb_record_primary_key";
const VALUE: &str = "__moli_idb_record_value";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::IDBRecord, require_prototype)]
struct RecordDeclaration<'s> {
    #[webapi(slot = KEY)]
    key: v8::Local<'s, v8::Value>,
    #[webapi(slot = PRIMARY_KEY)]
    primary_key: v8::Local<'s, v8::Value>,
    #[webapi(slot = VALUE)]
    value: v8::Local<'s, v8::Value>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBRecord, enumerable, receiver)]
struct RecordPrototype {
    #[webapi(accessor_property, getter = key_getter)]
    key: (),
    #[webapi(accessor_property, getter = primary_key_getter)]
    primary_key: (),
    #[webapi(accessor_property, getter = value_getter)]
    value: (),
}

pub(super) fn install_record_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    name: &str,
) {
    if name == "IDBRecord" {
        RecordPrototype::initialize_prototype_template(scope, prototype);
    }
}

pub(super) fn create_record<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: &Key,
    primary_key: &Key,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    // Keep key snapshots in private, never-exposed V8 values. The object owns
    // all references, including cyclic structured-clone values, so ordinary
    // V8 tracing collects records without a realm-wide table of strong roots.
    let key = key_to_js_value(scope, key);
    let primary_key = key_to_js_value(scope, primary_key);
    RecordDeclaration::new(key, primary_key, value)
        .bind(scope)
        .ok()
}

fn key_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    convert_key_attribute(scope, args.this(), KEY, rv);
}

fn primary_key_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    convert_key_attribute(scope, args.this(), PRIMARY_KEY, rv);
}

fn convert_key_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    record: v8::Local<'s, v8::Object>,
    slot: &str,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    // These are native dense arrays / Date / fixed ArrayBuffer snapshots, never
    // author objects. Converting them cannot invoke author getters. Each key
    // getter performs key-to-value conversion in the called getter's realm.
    if let Some(value) = get_private_value(scope, record, slot)
        && let Some(key) = require_idb_key(scope, value)
    {
        rv.set(key_to_js_value(scope, &key));
    }
}

fn value_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(value) = get_private_value(scope, args.this(), VALUE) {
        rv.set(value);
    }
}
