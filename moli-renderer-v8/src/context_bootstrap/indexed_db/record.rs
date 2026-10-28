use super::{Key, key_to_js_value, require_idb_key};
use crate::{
    util::{get_private_value, private_key},
    web_api_interfaces,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const KEY: &str = "__moli_idb_record_key";
const PRIMARY_KEY: &str = "__moli_idb_record_primary_key";
const VALUE: &str = "__moli_idb_record_value";
const KEY_CACHE: &str = "__moli_idb_record_key_cache";
const PRIMARY_KEY_CACHE: &str = "__moli_idb_record_primary_key_cache";

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
    let _ = convert_key_attribute(scope, args.this(), KEY, KEY_CACHE, rv);
}

fn primary_key_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    let _ = convert_key_attribute(scope, args.this(), PRIMARY_KEY, PRIMARY_KEY_CACHE, rv);
}

fn convert_key_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    record: v8::Local<'s, v8::Object>,
    slot: &str,
    cache_slot: &str,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) -> Option<()> {
    let cache = private_key(scope, cache_slot)?;
    if record.has_private(scope, cache)? {
        rv.set(record.get_private(scope, cache)?);
        return Some(());
    }
    // The immutable snapshot is never exposed. Cache a separate first-getter
    // value in a traced slot so mutations are stable without changing storage.
    let snapshot = get_private_value(scope, record, slot)?;
    let key = require_idb_key(scope, snapshot)?;
    let record_context = record.get_creation_context(scope)?;
    let value = materialize_record_key(scope, &key, record_context)?;
    record.set_private(scope, cache, value)?;
    rv.set(value);
    Some(())
}

fn materialize_record_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: &Key,
    record_context: v8::Local<'s, v8::Context>,
) -> Option<v8::Local<'s, v8::Value>> {
    if let Key::Array(keys) = key {
        let values = keys
            .iter()
            .map(|key| materialize_record_key(scope, key, record_context))
            .collect::<Option<Vec<_>>>()?;
        return crate::util::serialize_v8_array(scope, values.as_slice()).map(Into::into);
    }
    // Match Chromium's key-to-value conversion: Arrays use the current getter
    // context, while Date and binary wrappers use the record's relevant context.
    if matches!(key, Key::Date(_) | Key::Binary(_)) && record_context != scope.get_current_context()
    {
        let value = {
            let target_scope = &mut v8::ContextScope::new(scope, record_context);
            let value = key_to_js_value(target_scope, key);
            v8::Global::new(target_scope, value)
        };
        return Some(v8::Local::new(scope, value));
    }
    Some(key_to_js_value(scope, key))
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
