use super::callbacks::{
    storage_clear_callback, storage_get_item_callback, storage_key_callback,
    storage_length_getter_callback, storage_remove_item_callback, storage_set_item_callback,
};
use super::helpers::{
    STORAGE_KIND_INTERNAL_FIELD_INDEX, STORAGE_OWNER_INTERNAL_FIELD_INDEX, WebStorageOwner,
    set_storage_owner_child_handle, web_storage_owner_for_window,
};
use super::interceptors::{
    storage_indexed_definer, storage_indexed_deleter, storage_indexed_descriptor,
    storage_indexed_getter, storage_indexed_query, storage_indexed_setter, storage_named_definer,
    storage_named_deleter, storage_named_descriptor, storage_named_enumerator,
    storage_named_getter, storage_named_query, storage_named_setter,
    storage_prototype_indexed_definer, storage_prototype_indexed_deleter,
    storage_prototype_indexed_descriptor, storage_prototype_indexed_getter,
    storage_prototype_indexed_query, storage_prototype_indexed_setter,
};
use super::*;
use crate::util::{get_private_value, set_private_value};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object", enumerable)]
struct StoragePrototypeMethodsDeclaration {
    #[webapi(method, length = 1, callback = storage_get_item_callback)]
    get_item: (),
    #[webapi(method, length = 2, callback = storage_set_item_callback)]
    set_item: (),
    #[webapi(method, length = 1, callback = storage_remove_item_callback)]
    remove_item: (),
    #[webapi(method, length = 0, callback = storage_clear_callback)]
    clear: (),
    #[webapi(method, length = 1, callback = storage_key_callback)]
    key: (),
    #[webapi(accessor_property, getter = storage_length_getter_callback, enumerable)]
    length: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Storage")]
struct StoragePrototypeMetadataDeclaration {
    #[webapi(to_string_tag, init = string("Storage"))]
    to_string_tag: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct StorageConstructorGlobalDeclaration<'scope> {
    #[webapi(data_property = "Storage")]
    constructor: v8::Local<'scope, v8::Function>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Storage", constructor = "illegal", constructor_length = 0)]
struct StorageConstructorTemplateDeclaration {}

pub(in crate::context_bootstrap) fn install_storage_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    install_storage_constructor_runtime_state(scope, global).map(|_| ())
}

pub(in crate::context_bootstrap) fn ensure_storage_runtime_state_for_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    slot_name: &'static str,
    storage_kind: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(value) = get_private_value(scope, window, slot_name)
        && let Ok(storage) = v8::Local::<v8::Object>::try_from(value)
    {
        return Some(storage);
    }

    let prototype = global_constructor_prototype(scope, "Storage");
    install_named_storage_runtime_state(scope, window, slot_name, storage_kind, prototype).ok()
}

fn install_named_storage_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    slot_name: &'static str,
    storage_kind: &str,
    prototype: Option<v8::Local<'s, v8::Object>>,
) -> Result<v8::Local<'s, v8::Object>> {
    let storage = build_storage_object_template(scope)
        .new_instance(scope)
        .ok_or_else(|| anyhow!("failed to instantiate storage object"))?;
    let storage_kind_value = v8_string(scope, storage_kind)
        .ok_or_else(|| anyhow!("failed to allocate storage kind `{storage_kind}`"))?;
    let _ =
        storage.set_internal_field(STORAGE_KIND_INTERNAL_FIELD_INDEX, storage_kind_value.into());
    let owner = match web_storage_owner_for_window(scope, global) {
        WebStorageOwner::ActiveDocument => 0,
        WebStorageOwner::Child(handle) => {
            set_storage_owner_child_handle(scope, storage, handle);
            0
        }
    };
    let owner_value = v8::BigInt::new_from_u64(scope, owner);
    let _ = storage.set_internal_field(STORAGE_OWNER_INTERNAL_FIELD_INDEX, owner_value.into());
    if let Some(prototype) = prototype {
        let _ = storage.set_prototype(scope, prototype.into());
    }
    set_private_value(scope, global, slot_name, storage.into());
    Ok(storage)
}

fn build_storage_object_template<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::ObjectTemplate> {
    let template = v8::ObjectTemplate::new(scope);
    let _ = template.set_internal_field_count(2);
    template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(storage_indexed_getter)
            .setter(storage_indexed_setter)
            .query(storage_indexed_query)
            .deleter(storage_indexed_deleter)
            .definer(storage_indexed_definer)
            .descriptor(storage_indexed_descriptor),
    );
    template.set_named_property_handler(
        v8::NamedPropertyHandlerConfiguration::new()
            .getter(storage_named_getter)
            .setter(storage_named_setter)
            .query(storage_named_query)
            .deleter(storage_named_deleter)
            .enumerator(storage_named_enumerator)
            .definer(storage_named_definer)
            .descriptor(storage_named_descriptor),
    );
    template
}

fn install_storage_constructor_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Object>> {
    let template = StorageConstructorTemplateDeclaration::build(scope);
    let prototype_template = template.prototype_template(scope);
    // V8 indexed interceptors do not mask numeric properties that already
    // exist on the prototype chain. Keep Storage.prototype numeric descriptors
    // virtual so storage[9] assignment still reaches the Storage setter while
    // reads and getOwnPropertyDescriptor preserve Web Storage's
    // no-LegacyOverrideBuiltIns visibility rules.
    prototype_template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(storage_prototype_indexed_getter)
            .setter(storage_prototype_indexed_setter)
            .query(storage_prototype_indexed_query)
            .deleter(storage_prototype_indexed_deleter)
            .definer(storage_prototype_indexed_definer)
            .descriptor(storage_prototype_indexed_descriptor),
    );
    let constructor = template
        .get_function(scope)
        .ok_or_else(|| anyhow!("failed to build Storage constructor"))?;
    StorageConstructorGlobalDeclaration::new(constructor)
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize Storage constructor global: {error}"))?;
    let prototype = constructor
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow!("failed to extract Storage.prototype"))?;
    StoragePrototypeMethodsDeclaration::default()
        .initialize(scope, prototype)
        .map_err(|error| anyhow!("failed to initialize Storage prototype methods: {error}"))?;
    StoragePrototypeMetadataDeclaration::default()
        .initialize(scope, prototype)
        .map_err(|error| anyhow!("failed to initialize Storage prototype metadata: {error}"))?;
    Ok(prototype)
}
