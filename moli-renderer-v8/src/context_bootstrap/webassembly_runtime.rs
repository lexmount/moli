use super::*;
use crate::{
    definitions::define_non_enumerable_bool_property,
    util::{
        get_private_object, get_private_value, new_null_prototype_object, private_key,
        set_private_value, throw_range_error, throw_type_error, v8_string, v8str,
    },
};
use moli_webapi_declare::WebApiObject;

mod limits;

const WASM_RUNTIME_INSTALLED_SLOT: &str = "__moliWebAssemblyRuntimeExtensions";

const ORIGINAL_MODULE_CTOR_SLOT: &str = "__moliOriginalWebAssemblyModule";
const ORIGINAL_INSTANCE_CTOR_SLOT: &str = "__moliOriginalWebAssemblyInstance";
const ORIGINAL_VALIDATE_FN_SLOT: &str = "__moliOriginalWebAssemblyValidate";
const ORIGINAL_COMPILE_FN_SLOT: &str = "__moliOriginalWebAssemblyCompile";
const ORIGINAL_INSTANTIATE_FN_SLOT: &str = "__moliOriginalWebAssemblyInstantiate";
const ORIGINAL_MEMORY_CTOR_SLOT: &str = "__moliOriginalWebAssemblyMemory";
const ORIGINAL_TABLE_CTOR_SLOT: &str = "__moliOriginalWebAssemblyTable";
const ORIGINAL_GLOBAL_CTOR_SLOT: &str = "__moliOriginalWebAssemblyGlobal";
const ORIGINAL_TAG_CTOR_SLOT: &str = "__moliOriginalWebAssemblyTag";
const ORIGINAL_EXCEPTION_CTOR_SLOT: &str = "__moliOriginalWebAssemblyException";

const MEMORY_TYPE_SLOT: &str = "__moliWebAssemblyMemoryType";
const TABLE_TYPE_SLOT: &str = "__moliWebAssemblyTableType";
const GLOBAL_TYPE_SLOT: &str = "__moliWebAssemblyGlobalType";
const TAG_TYPE_SLOT: &str = "__moliWebAssemblyTagType";

const MEMORY_BUFFER_GETTER_SLOT: &str = "__moliWebAssemblyMemoryBufferGetter";
const TABLE_LENGTH_GETTER_SLOT: &str = "__moliWebAssemblyTableLengthGetter";
const GLOBAL_VALUE_SETTER_SLOT: &str = "__moliWebAssemblyGlobalValueSetter";
const GLOBAL_VALUE_SETTER_WRAPPED_SLOT: &str = "__moliAcceptsMissingArgument";
const EXCEPTION_GET_ARG_SLOT: &str = "__moliWebAssemblyExceptionGetArg";

const STREAMING_OPTIONS_SLOT: &str = "__moliWebAssemblyStreamingOptions";
const STREAMING_IMPORT_OBJECT_SLOT: &str = "__moliWebAssemblyStreamingImportObject";
const MODULE_INSTANTIATION_EXCEEDS_V8_LIMIT_SLOT: &str =
    "__moliWebAssemblyInstantiationExceedsV8Limit";
const WEBASSEMBLY_DEFAULT_PROTOTYPES_SLOT: &str = "__moliWebAssemblyDefaultPrototypes";

const WEBASSEMBLY_DEFAULT_PROTOTYPE_NAMES: &[&str] = &[
    "Module",
    "Instance",
    "Memory",
    "Table",
    "Global",
    "Tag",
    "Exception",
    "Function",
    "CompileError",
    "LinkError",
    "RuntimeError",
];

#[derive(WebApiObject)]
#[webapi(record)]
struct WebAssemblyStreamingDataDeclaration<'scope> {
    #[webapi(slot = STREAMING_OPTIONS_SLOT)]
    options: v8::Local<'scope, v8::Value>,
    #[webapi(slot = STREAMING_IMPORT_OBJECT_SLOT)]
    import_object: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(record)]
struct WebAssemblyInstantiateStreamingResultDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    module: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, enumerable)]
    instance: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(record)]
struct WebAssemblyMemoryNativeDescriptorDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    initial: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    maximum: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    shared: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(record)]
struct WebAssemblyMemoryTypeDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    minimum: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    maximum: Option<v8::Local<'scope, v8::Value>>,
    #[webapi(data_property, enumerable)]
    shared: Option<v8::Local<'scope, v8::Value>>,
}

#[derive(WebApiObject)]
#[webapi(record)]
struct WebAssemblyMemoryTypeCloneDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    minimum: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    maximum: Option<v8::Local<'scope, v8::Value>>,
    #[webapi(data_property, enumerable)]
    shared: Option<v8::Local<'scope, v8::Value>>,
}

#[derive(WebApiObject)]
#[webapi(record)]
struct WebAssemblyTableNativeDescriptorDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    element: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    initial: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    maximum: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(record)]
struct WebAssemblyTableTypeDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    minimum: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    maximum: Option<v8::Local<'scope, v8::Value>>,
    #[webapi(data_property, enumerable)]
    element: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(record)]
struct WebAssemblyTableTypeCloneDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    minimum: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    maximum: Option<v8::Local<'scope, v8::Value>>,
    #[webapi(data_property, enumerable)]
    element: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(record)]
struct WebAssemblyGlobalNativeDescriptorDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    mutable: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    value: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(record)]
struct WebAssemblyGlobalTypeDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    mutable: bool,
    #[webapi(data_property, enumerable)]
    value: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(record)]
struct WebAssemblyGlobalTypeCloneDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    r#mutable: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    value: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(record)]
struct WebAssemblyTagTypeDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    parameters: v8::Local<'scope, v8::Array>,
}

#[derive(WebApiObject)]
#[webapi(record)]
struct WebAssemblyMemoryFallbackTypeDeclaration {
    #[webapi(data_property, enumerable)]
    minimum: f64,
}

#[derive(WebApiObject)]
#[webapi(record)]
struct WebAssemblyTableFallbackTypeDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    minimum: f64,
    #[webapi(data_property, enumerable)]
    element: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(record)]
struct WebAssemblyTagTypeCloneDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    parameters: Option<v8::Local<'scope, v8::Array>>,
}

pub(in crate::context_bootstrap) fn install_webassembly_runtime_extensions(
    scope: &mut v8::PinScope<'_, '_>,
) -> Result<()> {
    let Some(webassembly) = webassembly_namespace(scope) else {
        return Ok(());
    };
    if runtime_already_installed(scope, webassembly) {
        return Ok(());
    }

    install_streaming_functions(scope, webassembly)?;
    install_v8_implementation_limit_compatibility(scope, webassembly)?;
    install_memory_constructor(scope, webassembly)?;
    install_table_constructor(scope, webassembly)?;
    install_global_constructor(scope, webassembly)?;
    install_tag_constructor(scope, webassembly)?;
    install_exception_shape(scope, webassembly)?;
    define_non_enumerable_bool_property(scope, webassembly, WASM_RUNTIME_INSTALLED_SLOT, true);
    Ok(())
}

pub(in crate::context_bootstrap) fn capture_webassembly_default_prototypes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    webassembly: v8::Local<'s, v8::Object>,
) {
    let prototypes = new_null_prototype_object(scope);
    for name in WEBASSEMBLY_DEFAULT_PROTOTYPE_NAMES {
        let Some(prototype) = webassembly_constructor(scope, webassembly, name)
            .and_then(|constructor| constructor.get(scope, v8str(scope, "prototype").into()))
            .and_then(|prototype| v8::Local::<v8::Object>::try_from(prototype).ok())
        else {
            continue;
        };
        let _ = prototypes.create_data_property(scope, v8str(scope, name).into(), prototype.into());
    }
    set_private_value(
        scope,
        global,
        WEBASSEMBLY_DEFAULT_PROTOTYPES_SLOT,
        prototypes.into(),
    );
}

pub(crate) fn set_current_context_webassembly_default_prototype(
    scope: &mut v8::PinScope<'_, '_>,
    name: &str,
    prototype: v8::Local<'_, v8::Object>,
) {
    let Some(name) = v8_string(scope, name) else {
        return;
    };
    let global = scope.get_current_context().global(scope);
    let prototypes = get_private_object(scope, global, WEBASSEMBLY_DEFAULT_PROTOTYPES_SLOT)
        .unwrap_or_else(|| {
            let prototypes = new_null_prototype_object(scope);
            set_private_value(
                scope,
                global,
                WEBASSEMBLY_DEFAULT_PROTOTYPES_SLOT,
                prototypes.into(),
            );
            prototypes
        });
    let _ = prototypes.create_data_property(scope, name.into(), prototype.into());
}

pub(crate) fn webassembly_default_prototype_for_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Context>,
    name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = {
        let context_scope = &mut v8::ContextScope::new(scope, context);
        let name = v8_string(context_scope, name)?;
        let global = context.global(context_scope);
        let prototypes =
            get_private_object(context_scope, global, WEBASSEMBLY_DEFAULT_PROTOTYPES_SLOT)?;
        let prototype = prototypes
            .get(context_scope, name.into())
            .and_then(|prototype| v8::Local::<v8::Object>::try_from(prototype).ok())?;
        v8::Global::new(context_scope, prototype)
    };
    Some(v8::Local::new(scope, &prototype))
}

fn webassembly_namespace<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    scope
        .get_current_context()
        .global(scope)
        .get(scope, v8str(scope, "WebAssembly").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn runtime_already_installed(
    scope: &mut v8::PinScope<'_, '_>,
    webassembly: v8::Local<'_, v8::Object>,
) -> bool {
    webassembly
        .get(scope, v8str(scope, WASM_RUNTIME_INSTALLED_SLOT).into())
        .is_some_and(|value| value.boolean_value(scope))
}

fn webassembly_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    webassembly: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Option<v8::Local<'s, v8::Function>> {
    webassembly
        .get(scope, v8str(scope, name).into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
}

fn original_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    slot: &'static str,
    fallback_name: &'static str,
) -> Option<v8::Local<'s, v8::Function>> {
    let global = scope.get_current_context().global(scope);
    get_private_value(scope, global, slot)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .or_else(|| {
            webassembly_namespace(scope)
                .and_then(|wa| webassembly_constructor(scope, wa, fallback_name))
        })
}

fn store_original_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    webassembly: v8::Local<'s, v8::Object>,
    name: &'static str,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Function>> {
    let constructor = webassembly_constructor(scope, webassembly, name)?;
    let global = scope.get_current_context().global(scope);
    set_private_value(scope, global, slot, constructor.into());
    Some(constructor)
}

fn original_namespace_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Function>> {
    let global = scope.get_current_context().global(scope);
    get_private_value(scope, global, slot)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
}

fn store_original_namespace_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    webassembly: v8::Local<'s, v8::Object>,
    name: &'static str,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Function>> {
    let function = webassembly
        .get(scope, v8str(scope, name).into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let global = scope.get_current_context().global(scope);
    set_private_value(scope, global, slot, function.into());
    Some(function)
}

fn define_value_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &'static str,
    value: v8::Local<'_, v8::Value>,
    writable: bool,
    enumerable: bool,
    configurable: bool,
) -> Result<()> {
    let mut descriptor = v8::PropertyDescriptor::new_from_value_writable(value, writable);
    descriptor.set_enumerable(enumerable);
    descriptor.set_configurable(configurable);
    object
        .define_property(scope, v8str(scope, name).into(), &descriptor)
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| anyhow!("failed to define WebAssembly property `{name}`"))
}

fn define_accessor_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &'static str,
    getter: v8::Local<'_, v8::Value>,
    setter: v8::Local<'_, v8::Value>,
    enumerable: bool,
    configurable: bool,
) -> Result<()> {
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter, setter);
    descriptor.set_enumerable(enumerable);
    descriptor.set_configurable(configurable);
    object
        .define_property(scope, v8str(scope, name).into(), &descriptor)
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| anyhow!("failed to define WebAssembly accessor `{name}`"))
}

fn install_namespace_function(
    scope: &mut v8::PinScope<'_, '_>,
    webassembly: v8::Local<'_, v8::Object>,
    name: &'static str,
    length: i32,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
) -> Result<()> {
    let function = v8::Function::builder(callback)
        .length(length)
        .build(scope)
        .ok_or_else(|| anyhow!("failed to build WebAssembly.{name}"))?;
    function.set_name(v8str(scope, name));
    define_value_property(scope, webassembly, name, function.into(), true, true, true)
}

fn install_namespace_function_wrapper(
    scope: &mut v8::PinScope<'_, '_>,
    webassembly: v8::Local<'_, v8::Object>,
    name: &'static str,
    length: i32,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
) -> Result<()> {
    let function = v8::Function::builder(callback)
        .length(length)
        .build(scope)
        .ok_or_else(|| anyhow!("failed to build WebAssembly.{name} wrapper"))?;
    function.set_name(v8str(scope, name));
    define_value_property(scope, webassembly, name, function.into(), true, true, true)
}

fn copy_constructor_static_function(
    scope: &mut v8::PinScope<'_, '_>,
    source: v8::Local<'_, v8::Function>,
    target: v8::Local<'_, v8::Function>,
    name: &'static str,
) -> Result<()> {
    let Some(value) = source
        .get(scope, v8str(scope, name).into())
        .filter(|value| value.is_function())
    else {
        return Ok(());
    };
    define_value_property(scope, target.into(), name, value, true, true, true)
}

fn install_prototype_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
    name: &'static str,
    length: i32,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
) -> Result<v8::Local<'s, v8::Function>> {
    let function = v8::Function::builder(callback)
        .length(length)
        .build(scope)
        .ok_or_else(|| anyhow!("failed to build WebAssembly prototype method `{name}`"))?;
    function.set_name(v8str(scope, name));
    define_value_property(scope, prototype, name, function.into(), true, false, true)?;
    Ok(function)
}

fn install_constructor_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    webassembly: v8::Local<'s, v8::Object>,
    name: &'static str,
    length: i32,
    native_constructor: v8::Local<'s, v8::Function>,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
) -> Result<v8::Local<'s, v8::Function>> {
    let wrapper = v8::Function::builder(callback)
        .length(length)
        .build(scope)
        .ok_or_else(|| anyhow!("failed to build WebAssembly.{name} wrapper"))?;
    wrapper.set_name(v8str(scope, name));

    if let Some(function_prototype) = native_constructor.get_prototype(scope) {
        let _ = wrapper.set_prototype(scope, function_prototype);
    }

    let Some(native_prototype) = native_constructor
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        define_value_property(scope, webassembly, name, wrapper.into(), true, false, true)?;
        return Ok(wrapper);
    };

    define_value_property(
        scope,
        wrapper.into(),
        "prototype",
        native_prototype.into(),
        false,
        false,
        false,
    )?;
    define_value_property(
        scope,
        native_prototype,
        "constructor",
        wrapper.into(),
        true,
        false,
        true,
    )?;
    define_value_property(scope, webassembly, name, wrapper.into(), true, false, true)?;
    Ok(wrapper)
}

fn constructor_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor: v8::Local<'s, v8::Function>,
) -> Option<v8::Local<'s, v8::Object>> {
    constructor
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn is_dictionary_like(value: v8::Local<'_, v8::Value>) -> bool {
    !value.is_null_or_undefined() && (value.is_object() || value.is_function())
}

fn object_from_dictionary_like<'s>(
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    if !is_dictionary_like(value) {
        return None;
    }
    v8::Local::<v8::Object>::try_from(value).ok()
}

fn object_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> v8::Local<'s, v8::Value> {
    object
        .get(scope, v8str(scope, name).into())
        .unwrap_or_else(|| v8::undefined(scope).into())
}

fn has_defined_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &'static str,
) -> bool {
    let key = v8str(scope, name);
    object.has(scope, key.into()).unwrap_or(false)
        && object
            .get(scope, key.into())
            .is_some_and(|value| !value.is_undefined())
}

fn type_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    let message = v8_string(scope, message)?;
    Some(v8::Exception::type_error(scope, message))
}

fn promise_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Function>> {
    scope
        .get_current_context()
        .global(scope)
        .get(scope, v8str(scope, "Promise").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
}

fn promise_resolve<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let promise = promise_constructor(scope)?;
    let resolve = promise
        .get(scope, v8str(scope, "resolve").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    resolve
        .call(scope, promise.into(), &[value])
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn promise_reject<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reason: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let promise = promise_constructor(scope)?;
    let reject = promise
        .get(scope, v8str(scope, "reject").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    reject.call(scope, promise.into(), &[reason])
}

fn promise_then<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    promise: v8::Local<'s, v8::Object>,
    on_fulfilled: v8::Local<'s, v8::Function>,
) -> Option<v8::Local<'s, v8::Value>> {
    let then = promise
        .get(scope, v8str(scope, "then").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    then.call(scope, promise.into(), &[on_fulfilled.into()])
}

fn promise_resolve_then<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    on_fulfilled: v8::Local<'s, v8::Function>,
) -> Option<v8::Local<'s, v8::Value>> {
    let promise = promise_resolve(scope, value)?;
    promise_then(scope, promise, on_fulfilled)
}

struct NormalizedWasmArgument<'s> {
    value: v8::Local<'s, v8::Value>,
    instantiation_exceeds_v8_limit: bool,
}

fn normalized_wasm_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<NormalizedWasmArgument<'s>> {
    if value.is_shared_array_buffer()
        || v8::Local::<v8::ArrayBufferView>::try_from(value)
            .ok()
            .and_then(|view| view.get_backing_store())
            .is_some_and(|store| store.is_shared())
    {
        return None;
    }
    let bytes = crate::blob::buffer_source_bytes_from_value(scope, value)?;
    let normalized = limits::normalize_v8_implementation_limits(&bytes)?;
    let byte_length = normalized.bytes.len();
    let buffer = crate::blob::array_buffer_from_bytes(scope, normalized.bytes)?;
    let view = v8::Uint8Array::new(scope, buffer, 0, byte_length)?;
    Some(NormalizedWasmArgument {
        value: view.into(),
        instantiation_exceeds_v8_limit: normalized.instantiation_exceeds_v8_limit,
    })
}

fn forwarded_namespace_arguments<'s>(
    args: &v8::FunctionCallbackArguments<'s>,
    first: v8::Local<'s, v8::Value>,
) -> Vec<v8::Local<'s, v8::Value>> {
    let mut forwarded = Vec::with_capacity(args.length().max(1) as usize);
    forwarded.push(first);
    for index in 1..args.length() {
        forwarded.push(args.get(index));
    }
    forwarded
}

fn call_original_namespace_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    slot: &'static str,
    forwarded: &[v8::Local<'s, v8::Value>],
) -> Option<v8::Local<'s, v8::Value>> {
    let function = original_namespace_function(scope, slot)?;
    let receiver = webassembly_namespace(scope)?;
    function.call(scope, receiver.into(), forwarded)
}

pub(crate) fn module_instantiation_exceeds_v8_limit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'_, v8::Value>,
) -> bool {
    let Ok(module) = v8::Local::<v8::Object>::try_from(value) else {
        return false;
    };
    let Some(key) = private_key(scope, MODULE_INSTANTIATION_EXCEEDS_V8_LIMIT_SLOT) else {
        return false;
    };
    module
        .get_private(scope, key)
        .is_some_and(|value| !value.is_undefined() && value.boolean_value(scope))
}

pub(crate) fn mark_module_instantiation_exceeds_v8_limit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: v8::Local<'s, v8::Object>,
) {
    let marker = v8::Boolean::new(scope, true);
    set_private_value(
        scope,
        module,
        MODULE_INSTANTIATION_EXCEEDS_V8_LIMIT_SLOT,
        marker.into(),
    );
}

fn reject_instantiation_exceeding_v8_limit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(message) = v8_string(
        scope,
        "WebAssembly resource declaration exceeds the implementation limit",
    ) else {
        return;
    };
    let error = v8::Exception::range_error(scope, message);
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let _ = resolver.reject(scope, error);
    rv.set(resolver.get_promise(scope).into());
}

fn install_v8_implementation_limit_compatibility<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    webassembly: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let validate = store_original_namespace_function(
        scope,
        webassembly,
        "validate",
        ORIGINAL_VALIDATE_FN_SLOT,
    );
    let compile =
        store_original_namespace_function(scope, webassembly, "compile", ORIGINAL_COMPILE_FN_SLOT);
    let instantiate = store_original_namespace_function(
        scope,
        webassembly,
        "instantiate",
        ORIGINAL_INSTANTIATE_FN_SLOT,
    );

    if validate.is_some() {
        install_namespace_function_wrapper(
            scope,
            webassembly,
            "validate",
            1,
            validate_with_v8_limit_compatibility_callback,
        )?;
    }
    if compile.is_some() {
        install_namespace_function_wrapper(
            scope,
            webassembly,
            "compile",
            1,
            compile_with_v8_limit_compatibility_callback,
        )?;
    }
    if instantiate.is_some() {
        install_namespace_function_wrapper(
            scope,
            webassembly,
            "instantiate",
            1,
            instantiate_with_v8_limit_compatibility_callback,
        )?;
    }

    if let Some(native_constructor) =
        original_constructor(scope, ORIGINAL_MODULE_CTOR_SLOT, "Module")
    {
        let wrapper = install_constructor_wrapper(
            scope,
            webassembly,
            "Module",
            1,
            native_constructor,
            module_with_v8_limit_compatibility_callback,
        )?;
        for name in ["exports", "imports", "customSections"] {
            copy_constructor_static_function(scope, native_constructor, wrapper, name)?;
        }
    }
    if let Some(native_constructor) =
        original_constructor(scope, ORIGINAL_INSTANCE_CTOR_SLOT, "Instance")
    {
        install_constructor_wrapper(
            scope,
            webassembly,
            "Instance",
            1,
            native_constructor,
            instance_with_v8_limit_guard_callback,
        )?;
    }
    Ok(())
}

fn module_with_v8_limit_compatibility_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "WebAssembly.Module must be invoked with 'new'");
        return;
    }
    let normalized = normalized_wasm_argument(scope, args.get(0));
    let forwarded = if let Some(normalized) = &normalized {
        forwarded_namespace_arguments(&args, normalized.value)
    } else {
        (0..args.length())
            .map(|index| args.get(index))
            .collect::<Vec<_>>()
    };
    let Some(module) = construct_native(
        scope,
        ORIGINAL_MODULE_CTOR_SLOT,
        "Module",
        &forwarded,
        args.new_target(),
    ) else {
        return;
    };
    if normalized.is_some_and(|normalized| normalized.instantiation_exceeds_v8_limit) {
        mark_module_instantiation_exceeds_v8_limit(scope, module);
    }
    rv.set(module.into());
}

fn validate_with_v8_limit_compatibility_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let first = normalized_wasm_argument(scope, args.get(0))
        .map_or_else(|| args.get(0), |normalized| normalized.value);
    let forwarded = forwarded_namespace_arguments(&args, first);
    if let Some(value) =
        call_original_namespace_function(scope, ORIGINAL_VALIDATE_FN_SLOT, &forwarded)
    {
        rv.set(value);
    }
}

fn compile_with_v8_limit_compatibility_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let normalized = normalized_wasm_argument(scope, args.get(0));
    let first = normalized
        .as_ref()
        .map_or_else(|| args.get(0), |normalized| normalized.value);
    let forwarded = forwarded_namespace_arguments(&args, first);
    let Some(value) = call_original_namespace_function(scope, ORIGINAL_COMPILE_FN_SLOT, &forwarded)
    else {
        return;
    };
    if !normalized.is_some_and(|normalized| normalized.instantiation_exceeds_v8_limit) {
        rv.set(value);
        return;
    }
    let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) else {
        rv.set(value);
        return;
    };
    let Some(on_fulfilled) =
        v8::Function::builder(mark_compiled_module_v8_limit_callback).build(scope)
    else {
        return;
    };
    if let Some(marked) = promise.then(scope, on_fulfilled) {
        rv.set(marked.into());
    }
}

fn mark_compiled_module_v8_limit_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let module = args.get(0);
    if let Ok(module_object) = v8::Local::<v8::Object>::try_from(module) {
        mark_module_instantiation_exceeds_v8_limit(scope, module_object);
    }
    rv.set(module);
}

fn instantiate_with_v8_limit_compatibility_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let source = args.get(0);
    if module_instantiation_exceeds_v8_limit(scope, source) {
        reject_instantiation_exceeding_v8_limit(scope, rv);
        return;
    }
    let normalized = normalized_wasm_argument(scope, source);
    if normalized
        .as_ref()
        .is_some_and(|normalized| normalized.instantiation_exceeds_v8_limit)
    {
        reject_instantiation_exceeding_v8_limit(scope, rv);
        return;
    }
    let first = normalized
        .as_ref()
        .map_or(source, |normalized| normalized.value);
    let forwarded = forwarded_namespace_arguments(&args, first);
    if let Some(value) =
        call_original_namespace_function(scope, ORIGINAL_INSTANTIATE_FN_SLOT, &forwarded)
    {
        let mut rv = rv;
        rv.set(value);
    }
}

fn instance_with_v8_limit_guard_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "WebAssembly.Instance must be invoked with 'new'");
        return;
    }
    if module_instantiation_exceeds_v8_limit(scope, args.get(0)) {
        throw_range_error(
            scope,
            "WebAssembly resource declaration exceeds the implementation limit",
        );
        return;
    }
    let forwarded = (0..args.length())
        .map(|index| args.get(index))
        .collect::<Vec<_>>();
    if let Some(instance) = construct_native(
        scope,
        ORIGINAL_INSTANCE_CTOR_SLOT,
        "Instance",
        &forwarded,
        args.new_target(),
    ) {
        rv.set(instance.into());
    }
}

fn install_streaming_functions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    webassembly: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let module_exists =
        store_original_constructor(scope, webassembly, "Module", ORIGINAL_MODULE_CTOR_SLOT)
            .is_some();
    if module_exists
        && webassembly
            .get(scope, v8str(scope, "compileStreaming").into())
            .is_none_or(|value| !value.is_function())
    {
        install_namespace_function(
            scope,
            webassembly,
            "compileStreaming",
            1,
            compile_streaming_callback,
        )?;
    }

    let instance_exists =
        store_original_constructor(scope, webassembly, "Instance", ORIGINAL_INSTANCE_CTOR_SLOT)
            .is_some();
    if module_exists
        && instance_exists
        && webassembly
            .get(scope, v8str(scope, "instantiateStreaming").into())
            .is_none_or(|value| !value.is_function())
    {
        install_namespace_function(
            scope,
            webassembly,
            "instantiateStreaming",
            1,
            instantiate_streaming_callback,
        )?;
    }
    Ok(())
}

fn streaming_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Value>,
    import_object: Option<v8::Local<'s, v8::Value>>,
) -> v8::Local<'s, v8::Object> {
    WebAssemblyStreamingDataDeclaration::new(
        options,
        import_object.unwrap_or_else(|| v8::undefined(scope).into()),
    )
    .bind(scope)
    .expect("WebAssembly streaming data declaration should bind")
}

fn compile_streaming_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let source = args.get(0);
    let options = args.get(1);
    let data = streaming_data(scope, options, None);
    let Some(on_response) = v8::Function::builder(compile_streaming_response_callback)
        .data(data.into())
        .build(scope)
    else {
        return;
    };
    if let Some(promise) = promise_resolve_then(scope, source, on_response) {
        rv.set(promise);
    }
}

fn instantiate_streaming_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let import_object = args.get(1);
    if !import_object.is_undefined() && !is_dictionary_like(import_object) {
        if let Some(error) = type_error_value(scope, "Argument 1 must be an object")
            && let Some(promise) = promise_reject(scope, error)
        {
            rv.set(promise);
        }
        return;
    }
    let data = streaming_data(scope, args.get(2), Some(import_object));
    let Some(on_response) = v8::Function::builder(instantiate_streaming_response_callback)
        .data(data.into())
        .build(scope)
    else {
        return;
    };
    if let Some(promise) = promise_resolve_then(scope, args.get(0), on_response) {
        rv.set(promise);
    }
}

fn compile_streaming_response_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(bytes_or_promise) =
        crate::network_host::consume_webassembly_streaming_response_value(scope, args.get(0))
    else {
        return;
    };
    let Some(on_bytes) = v8::Function::builder(compile_streaming_bytes_callback)
        .data(args.data())
        .build(scope)
    else {
        return;
    };
    if let Some(promise) = promise_resolve_then(scope, bytes_or_promise, on_bytes) {
        rv.set(promise);
    }
}

fn instantiate_streaming_response_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(bytes_or_promise) =
        crate::network_host::consume_webassembly_streaming_response_value(scope, args.get(0))
    else {
        return;
    };
    let Some(on_bytes) = v8::Function::builder(instantiate_streaming_bytes_callback)
        .data(args.data())
        .build(scope)
    else {
        return;
    };
    if let Some(promise) = promise_resolve_then(scope, bytes_or_promise, on_bytes) {
        rv.set(promise);
    }
}

fn streaming_callback_data<'s>(
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    v8::Local::<v8::Object>::try_from(value).ok()
}

fn streaming_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    get_private_value(scope, data, STREAMING_OPTIONS_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into())
}

fn streaming_import_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    get_private_value(scope, data, STREAMING_IMPORT_OBJECT_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into())
}

fn compile_streaming_bytes_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(data) = streaming_callback_data(args.data()) else {
        return;
    };
    let Some(module_ctor) =
        webassembly_namespace(scope).and_then(|wa| webassembly_constructor(scope, wa, "Module"))
    else {
        return;
    };
    let options = streaming_options(scope, data);
    let Some(module) = module_ctor.new_instance(scope, &[args.get(0), options]) else {
        return;
    };
    rv.set(module.into());
}

fn instantiate_streaming_bytes_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(data) = streaming_callback_data(args.data()) else {
        return;
    };
    let Some(module_ctor) =
        webassembly_namespace(scope).and_then(|wa| webassembly_constructor(scope, wa, "Module"))
    else {
        return;
    };
    let Some(instance_ctor) =
        webassembly_namespace(scope).and_then(|wa| webassembly_constructor(scope, wa, "Instance"))
    else {
        return;
    };
    let options = streaming_options(scope, data);
    let Some(module) = module_ctor.new_instance(scope, &[args.get(0), options]) else {
        return;
    };
    let import_object = streaming_import_object(scope, data);
    let Some(instance) = instance_ctor.new_instance(scope, &[module.into(), import_object]) else {
        return;
    };
    let result = WebAssemblyInstantiateStreamingResultDeclaration::new(module, instance)
        .bind(scope)
        .expect("WebAssembly instantiateStreaming result declaration should bind");
    rv.set(result.into());
}

fn install_memory_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    webassembly: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let Some(native_constructor) =
        store_original_constructor(scope, webassembly, "Memory", ORIGINAL_MEMORY_CTOR_SLOT)
    else {
        return Ok(());
    };
    let prototype = constructor_prototype(scope, native_constructor);
    if let Some(prototype) = prototype {
        store_accessor_function(scope, prototype, "buffer", "get", MEMORY_BUFFER_GETTER_SLOT);
    }
    install_constructor_wrapper(
        scope,
        webassembly,
        "Memory",
        1,
        native_constructor,
        memory_constructor_callback,
    )?;
    if let Some(prototype) = prototype
        && prototype
            .get(scope, v8str(scope, "type").into())
            .is_none_or(|value| !value.is_function())
    {
        install_prototype_method(scope, prototype, "type", 0, memory_type_callback)?;
    }
    Ok(())
}

fn install_table_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    webassembly: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let Some(native_constructor) =
        store_original_constructor(scope, webassembly, "Table", ORIGINAL_TABLE_CTOR_SLOT)
    else {
        return Ok(());
    };
    let prototype = constructor_prototype(scope, native_constructor);
    if let Some(prototype) = prototype {
        store_accessor_function(scope, prototype, "length", "get", TABLE_LENGTH_GETTER_SLOT);
    }
    install_constructor_wrapper(
        scope,
        webassembly,
        "Table",
        1,
        native_constructor,
        table_constructor_callback,
    )?;
    if let Some(prototype) = prototype
        && prototype
            .get(scope, v8str(scope, "type").into())
            .is_none_or(|value| !value.is_function())
    {
        install_prototype_method(scope, prototype, "type", 0, table_type_callback)?;
    }
    Ok(())
}

fn install_global_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    webassembly: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let Some(native_constructor) =
        store_original_constructor(scope, webassembly, "Global", ORIGINAL_GLOBAL_CTOR_SLOT)
    else {
        return Ok(());
    };
    let prototype = constructor_prototype(scope, native_constructor);
    install_constructor_wrapper(
        scope,
        webassembly,
        "Global",
        1,
        native_constructor,
        global_constructor_callback,
    )?;
    if let Some(prototype) = prototype {
        if prototype
            .get(scope, v8str(scope, "type").into())
            .is_none_or(|value| !value.is_function())
        {
            install_prototype_method(scope, prototype, "type", 0, global_type_callback)?;
        }
        install_global_value_setter_compatibility(scope, prototype)?;
    }
    Ok(())
}

fn install_tag_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    webassembly: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let Some(native_constructor) =
        store_original_constructor(scope, webassembly, "Tag", ORIGINAL_TAG_CTOR_SLOT)
    else {
        return Ok(());
    };
    let prototype = constructor_prototype(scope, native_constructor);
    install_constructor_wrapper(
        scope,
        webassembly,
        "Tag",
        1,
        native_constructor,
        tag_constructor_callback,
    )?;
    if let Some(prototype) = prototype
        && prototype
            .get(scope, v8str(scope, "type").into())
            .is_none_or(|value| !value.is_function())
    {
        install_prototype_method(scope, prototype, "type", 0, tag_type_callback)?;
    }
    Ok(())
}

fn store_accessor_function(
    scope: &mut v8::PinScope<'_, '_>,
    prototype: v8::Local<'_, v8::Object>,
    property_name: &'static str,
    accessor_name: &'static str,
    slot: &'static str,
) {
    let Some(descriptor) =
        prototype.get_own_property_descriptor(scope, v8str(scope, property_name).into())
    else {
        return;
    };
    let Some(accessor) = v8::Local::<v8::Object>::try_from(descriptor)
        .ok()
        .and_then(|descriptor| descriptor.get(scope, v8str(scope, accessor_name).into()))
        .filter(|value| value.is_function())
    else {
        return;
    };
    let global = scope.get_current_context().global(scope);
    set_private_value(scope, global, slot, accessor);
}

fn apply_new_target_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    new_target: v8::Local<'s, v8::Value>,
) {
    let Some(prototype) = v8::Local::<v8::Object>::try_from(new_target)
        .ok()
        .and_then(|new_target| new_target.get(scope, v8str(scope, "prototype").into()))
        .and_then(|prototype| v8::Local::<v8::Object>::try_from(prototype).ok())
    else {
        return;
    };
    let _ = object.set_prototype(scope, prototype.into());
}

fn construct_native<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor_slot: &'static str,
    fallback_name: &'static str,
    args: &[v8::Local<'s, v8::Value>],
    new_target: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let constructor = original_constructor(scope, constructor_slot, fallback_name)?;
    let object = constructor.new_instance(scope, args)?;
    apply_new_target_prototype(scope, object, new_target);
    Some(object)
}

fn memory_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "WebAssembly.Memory must be invoked with 'new'");
        return;
    }
    let descriptor = args.get(0);
    let Some(descriptor_object) = object_from_dictionary_like(descriptor) else {
        if let Some(memory) = construct_native(
            scope,
            ORIGINAL_MEMORY_CTOR_SLOT,
            "Memory",
            &[descriptor],
            args.new_target(),
        ) {
            rv.set(memory.into());
        }
        return;
    };
    let minimum = object_property(scope, descriptor_object, "minimum");
    if minimum.is_undefined() {
        if let Some(memory) = construct_native(
            scope,
            ORIGINAL_MEMORY_CTOR_SLOT,
            "Memory",
            &[descriptor],
            args.new_target(),
        ) {
            rv.set(memory.into());
        }
        return;
    }
    if has_defined_property(scope, descriptor_object, "initial") {
        throw_type_error(
            scope,
            "WebAssembly.Memory descriptor cannot specify both initial and minimum",
        );
        return;
    }
    let maximum = object_property(scope, descriptor_object, "maximum");
    let shared = object_property(scope, descriptor_object, "shared");
    let normalized = WebAssemblyMemoryNativeDescriptorDeclaration {
        initial: minimum,
        maximum,
        shared,
    }
    .bind(scope)
    .expect("WebAssembly Memory native descriptor declaration should bind");

    let Some(memory) = construct_native(
        scope,
        ORIGINAL_MEMORY_CTOR_SLOT,
        "Memory",
        &[normalized.into()],
        args.new_target(),
    ) else {
        return;
    };
    let type_object = WebAssemblyMemoryTypeDeclaration {
        minimum,
        maximum: (!maximum.is_undefined()).then_some(maximum),
        shared: (!shared.is_undefined()).then_some(shared),
    }
    .bind(scope)
    .expect("WebAssembly Memory type declaration should bind");
    set_private_value(scope, memory, MEMORY_TYPE_SLOT, type_object.into());
    rv.set(memory.into());
}

fn table_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "WebAssembly.Table must be invoked with 'new'");
        return;
    }
    let descriptor = args.get(0);
    let Some(descriptor_object) = object_from_dictionary_like(descriptor) else {
        if let Some(table) = construct_native(
            scope,
            ORIGINAL_TABLE_CTOR_SLOT,
            "Table",
            &[descriptor, args.get(1)],
            args.new_target(),
        ) {
            rv.set(table.into());
        }
        return;
    };
    let minimum = object_property(scope, descriptor_object, "minimum");
    if minimum.is_undefined() {
        if let Some(table) = construct_native(
            scope,
            ORIGINAL_TABLE_CTOR_SLOT,
            "Table",
            &[descriptor, args.get(1)],
            args.new_target(),
        ) {
            rv.set(table.into());
        }
        return;
    }
    if has_defined_property(scope, descriptor_object, "initial") {
        throw_type_error(
            scope,
            "WebAssembly.Table descriptor cannot specify both initial and minimum",
        );
        return;
    }
    let public_element = object_property(scope, descriptor_object, "element");
    let Some(native_element) = value_type_for_native_constructor(scope, public_element) else {
        return;
    };
    let maximum = object_property(scope, descriptor_object, "maximum");
    let native_descriptor = WebAssemblyTableNativeDescriptorDeclaration {
        element: native_element,
        initial: minimum,
        maximum,
    }
    .bind(scope)
    .expect("WebAssembly Table native descriptor declaration should bind");

    let Some(table) = construct_native(
        scope,
        ORIGINAL_TABLE_CTOR_SLOT,
        "Table",
        &[native_descriptor.into(), args.get(1)],
        args.new_target(),
    ) else {
        return;
    };
    let type_object = WebAssemblyTableTypeDeclaration {
        minimum,
        maximum: (!maximum.is_undefined()).then_some(maximum),
        element: public_element,
    }
    .bind(scope)
    .expect("WebAssembly Table type declaration should bind");
    set_private_value(scope, table, TABLE_TYPE_SLOT, type_object.into());
    rv.set(table.into());
}

fn global_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "WebAssembly.Global must be invoked with 'new'");
        return;
    }
    let descriptor = args.get(0);
    let mut native_descriptor = descriptor;
    let mut public_type = None;
    if let Some(descriptor_object) = object_from_dictionary_like(descriptor) {
        let mutable = object_property(scope, descriptor_object, "mutable");
        let value_type = object_property(scope, descriptor_object, "value");
        let Some(normalized_value) = value_type_for_native_constructor(scope, value_type) else {
            return;
        };
        let object = WebAssemblyGlobalNativeDescriptorDeclaration {
            mutable,
            value: normalized_value,
        }
        .bind(scope)
        .expect("WebAssembly Global native descriptor declaration should bind");
        native_descriptor = object.into();

        let type_object = WebAssemblyGlobalTypeDeclaration {
            mutable: mutable.boolean_value(scope),
            value: value_type,
        }
        .bind(scope)
        .expect("WebAssembly Global type declaration should bind");
        public_type = Some(type_object);
    }
    let native_value = if args.length() >= 2 {
        args.get(1)
    } else if descriptor_value_is_anyfunc(scope, native_descriptor) {
        v8::null(scope).into()
    } else {
        v8::undefined(scope).into()
    };
    let Some(global) = construct_native(
        scope,
        ORIGINAL_GLOBAL_CTOR_SLOT,
        "Global",
        &[native_descriptor, native_value],
        args.new_target(),
    ) else {
        return;
    };
    if let Some(public_type) = public_type {
        set_private_value(scope, global, GLOBAL_TYPE_SLOT, public_type.into());
    }
    rv.set(global.into());
}

fn tag_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "WebAssembly.Tag must be invoked with 'new'");
        return;
    }
    let descriptor = args.get(0);
    let Some(tag) = construct_native(
        scope,
        ORIGINAL_TAG_CTOR_SLOT,
        "Tag",
        &[descriptor],
        args.new_target(),
    ) else {
        return;
    };
    if let Some(descriptor_object) = object_from_dictionary_like(descriptor) {
        let parameters = object_property(scope, descriptor_object, "parameters");
        if let Some(parameters) = array_from_value_or_empty(scope, parameters) {
            let type_object = WebAssemblyTagTypeDeclaration { parameters }
                .bind(scope)
                .expect("WebAssembly Tag type declaration should bind");
            set_private_value(scope, tag, TAG_TYPE_SLOT, type_object.into());
        }
    }
    rv.set(tag.into());
}

fn string_value<'s>(scope: &mut v8::PinScope<'s, '_>, value: &str) -> v8::Local<'s, v8::Value> {
    v8_string(scope, value)
        .map(Into::into)
        .unwrap_or_else(|| v8::undefined(scope).into())
}

fn value_type_for_native_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    public_value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let value_string = public_value.to_string(scope)?;
    if value_string.to_rust_string_lossy(scope) == "funcref" {
        Some(string_value(scope, "anyfunc"))
    } else {
        Some(value_string.into())
    }
}

fn string_value_equals(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    expected: &str,
) -> bool {
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope) == expected)
        .unwrap_or(false)
}

fn descriptor_value_is_anyfunc<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    descriptor: v8::Local<'s, v8::Value>,
) -> bool {
    v8::Local::<v8::Object>::try_from(descriptor)
        .ok()
        .map(|descriptor| object_property(scope, descriptor, "value"))
        .is_some_and(|value| string_value_equals(scope, value, "anyfunc"))
}

fn array_from_value_or_empty<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Array>> {
    if value.is_null_or_undefined() {
        return Some(v8::Array::new(scope, 0));
    }
    // V8's WebAssembly.Tag constructor consumes `parameters` as an
    // array-like object (length plus indexed values), not a WebIDL sequence.
    // Mirror that native interpretation without consulting public Array.from.
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    let length = object
        .get(scope, v8str(scope, "length").into())?
        .uint32_value(scope)?;
    let mut values = Vec::with_capacity(length as usize);
    for index in 0..length {
        values.push(object.get_index(scope, index)?);
    }
    Some(v8::Array::new_with_elements(scope, &values))
}

fn clone_memory_type_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stored: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let minimum = stored.get(scope, v8str(scope, "minimum").into())?;
    if minimum.is_undefined() {
        return None;
    }
    let maximum = stored
        .get(scope, v8str(scope, "maximum").into())
        .filter(|value| !value.is_undefined());
    let shared = stored
        .get(scope, v8str(scope, "shared").into())
        .filter(|value| !value.is_undefined());
    WebAssemblyMemoryTypeCloneDeclaration::new(minimum, maximum, shared)
        .bind(scope)
        .ok()
}

fn clone_table_type_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stored: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let minimum = stored.get(scope, v8str(scope, "minimum").into())?;
    if minimum.is_undefined() {
        return None;
    }
    let element = stored.get(scope, v8str(scope, "element").into())?;
    if element.is_undefined() {
        return None;
    }
    let maximum = stored
        .get(scope, v8str(scope, "maximum").into())
        .filter(|value| !value.is_undefined());
    WebAssemblyTableTypeCloneDeclaration::new(minimum, maximum, element)
        .bind(scope)
        .ok()
}

fn clone_global_type_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stored: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let r#mutable = stored.get(scope, v8str(scope, "mutable").into())?;
    if r#mutable.is_undefined() {
        return None;
    }
    let value = stored.get(scope, v8str(scope, "value").into())?;
    if value.is_undefined() {
        return None;
    }
    WebAssemblyGlobalTypeCloneDeclaration::new(r#mutable, value)
        .bind(scope)
        .ok()
}

fn memory_type_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let this = args.this();
    if let Some(stored) = get_private_value(scope, this, MEMORY_TYPE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        if let Some(clone) = clone_memory_type_object(scope, stored) {
            rv.set(clone.into());
            return;
        }
        throw_type_error(
            scope,
            "WebAssembly.Memory.prototype.type called on incompatible receiver",
        );
        return;
    }
    if let Some(getter) = get_private_value(
        scope,
        scope.get_current_context().global(scope),
        MEMORY_BUFFER_GETTER_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        && let Some(buffer) = getter.call(scope, this.into(), &[])
        && let Ok(buffer) = v8::Local::<v8::Object>::try_from(buffer)
        && let Some(byte_length) = buffer
            .get(scope, v8str(scope, "byteLength").into())
            .and_then(|value| value.number_value(scope))
        && byte_length.is_finite()
    {
        let object = WebAssemblyMemoryFallbackTypeDeclaration {
            minimum: byte_length / 65536.0,
        }
        .bind(scope)
        .expect("WebAssembly Memory fallback type declaration should bind");
        rv.set(object.into());
        return;
    }
    throw_type_error(
        scope,
        "WebAssembly.Memory.prototype.type called on incompatible receiver",
    );
}

fn table_type_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let this = args.this();
    if let Some(stored) = get_private_value(scope, this, TABLE_TYPE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        if let Some(clone) = clone_table_type_object(scope, stored) {
            rv.set(clone.into());
            return;
        }
        throw_type_error(
            scope,
            "WebAssembly.Table.prototype.type called on incompatible receiver",
        );
        return;
    }
    if let Some(getter) = get_private_value(
        scope,
        scope.get_current_context().global(scope),
        TABLE_LENGTH_GETTER_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        && let Some(length) = getter.call(scope, this.into(), &[])
        && let Some(length) = length.number_value(scope)
    {
        let element = string_value(scope, "anyfunc");
        let object = WebAssemblyTableFallbackTypeDeclaration {
            minimum: length,
            element,
        }
        .bind(scope)
        .expect("WebAssembly Table fallback type declaration should bind");
        rv.set(object.into());
        return;
    }
    throw_type_error(
        scope,
        "WebAssembly.Table.prototype.type called on incompatible receiver",
    );
}

fn global_type_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let this = args.this();
    if let Some(stored) = get_private_value(scope, this, GLOBAL_TYPE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        if let Some(clone) = clone_global_type_object(scope, stored) {
            rv.set(clone.into());
            return;
        }
        throw_type_error(
            scope,
            "WebAssembly.Global.prototype.type called on incompatible receiver",
        );
        return;
    }
    throw_type_error(
        scope,
        "WebAssembly.Global.prototype.type called on incompatible receiver",
    );
}

fn tag_type_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let this = args.this();
    if let Some(stored) = get_private_value(scope, this, TAG_TYPE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        let parameters = stored
            .get(scope, v8str(scope, "parameters").into())
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
            .map(|parameters| clone_array(scope, parameters));
        let object = WebAssemblyTagTypeCloneDeclaration { parameters }
            .bind(scope)
            .expect("WebAssembly Tag type clone declaration should bind");
        rv.set(object.into());
        return;
    }
    throw_type_error(
        scope,
        "WebAssembly.Tag.prototype.type called on incompatible receiver",
    );
}

fn clone_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Array>,
) -> v8::Local<'s, v8::Array> {
    let len = source.length();
    let clone = v8::Array::new(scope, len as i32);
    for index in 0..len {
        if let Some(value) = source.get_index(scope, index) {
            let _ = clone.set_index(scope, index, value);
        }
    }
    clone
}

fn install_global_value_setter_compatibility(
    scope: &mut v8::PinScope<'_, '_>,
    prototype: v8::Local<'_, v8::Object>,
) -> Result<()> {
    let Some(descriptor) =
        prototype.get_own_property_descriptor(scope, v8str(scope, "value").into())
    else {
        return Ok(());
    };
    let Some(descriptor) = v8::Local::<v8::Object>::try_from(descriptor).ok() else {
        return Ok(());
    };
    let getter = descriptor
        .get(scope, v8str(scope, "get").into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    let setter = descriptor
        .get(scope, v8str(scope, "set").into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    if !setter.is_function() {
        return Ok(());
    }
    if v8::Local::<v8::Object>::try_from(setter)
        .ok()
        .and_then(|setter| get_private_value(scope, setter, GLOBAL_VALUE_SETTER_WRAPPED_SLOT))
        .is_some_and(|value| value.boolean_value(scope))
    {
        return Ok(());
    }

    let global = scope.get_current_context().global(scope);
    set_private_value(scope, global, GLOBAL_VALUE_SETTER_SLOT, setter);
    let value_setter = v8::Function::builder(global_value_setter_callback)
        .length(1)
        .build(scope)
        .ok_or_else(|| anyhow!("failed to build WebAssembly.Global value setter"))?;
    value_setter.set_name(v8str(scope, "set value"));
    set_private_value(
        scope,
        value_setter.into(),
        GLOBAL_VALUE_SETTER_WRAPPED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    let enumerable = descriptor
        .get(scope, v8str(scope, "enumerable").into())
        .is_some_and(|value| value.boolean_value(scope));
    let configurable = descriptor
        .get(scope, v8str(scope, "configurable").into())
        .is_some_and(|value| value.boolean_value(scope));
    define_accessor_property(
        scope,
        prototype,
        "value",
        getter,
        value_setter.into(),
        enumerable,
        configurable,
    )
}

fn global_value_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(setter) = get_private_value(
        scope,
        scope.get_current_context().global(scope),
        GLOBAL_VALUE_SETTER_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok()) else {
        return;
    };
    let value = if args.length() == 0 {
        v8::undefined(scope).into()
    } else {
        args.get(0)
    };
    let _ = setter.call(scope, args.this().into(), &[value]);
}

fn install_exception_shape<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    webassembly: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let Some(exception_ctor) = store_original_constructor(
        scope,
        webassembly,
        "Exception",
        ORIGINAL_EXCEPTION_CTOR_SLOT,
    ) else {
        return Ok(());
    };
    define_value_property(
        scope,
        exception_ctor.into(),
        "length",
        v8::Integer::new(scope, 2).into(),
        false,
        false,
        true,
    )?;
    let Some(prototype) = constructor_prototype(scope, exception_ctor) else {
        return Ok(());
    };
    if !prototype
        .has_own_property(scope, v8str(scope, "stack").into())
        .unwrap_or(false)
    {
        let getter = v8::Function::builder(exception_stack_getter_callback)
            .build(scope)
            .ok_or_else(|| anyhow!("failed to build WebAssembly.Exception stack getter"))?;
        define_accessor_property(
            scope,
            prototype,
            "stack",
            getter.into(),
            v8::undefined(scope).into(),
            true,
            true,
        )?;
    }
    if let Some(native_get_arg) = prototype
        .get(scope, v8str(scope, "getArg").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        set_private_value(
            scope,
            prototype,
            EXCEPTION_GET_ARG_SLOT,
            native_get_arg.into(),
        );
        install_prototype_method(scope, prototype, "getArg", 2, exception_get_arg_callback)?;
    }
    Ok(())
}

fn exception_stack_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(exception_ctor) =
        original_constructor(scope, ORIGINAL_EXCEPTION_CTOR_SLOT, "Exception")
    else {
        return;
    };
    if !args
        .this()
        .instance_of(scope, exception_ctor.into())
        .unwrap_or(false)
    {
        throw_type_error(
            scope,
            "WebAssembly.Exception.prototype.stack called on incompatible receiver",
        );
        return;
    }
    rv.set(v8str(scope, "").into());
}

fn exception_get_arg_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let native_get_arg = args
        .this()
        .get_prototype(scope)
        .and_then(|prototype| v8::Local::<v8::Object>::try_from(prototype).ok())
        .and_then(|prototype| get_private_value(scope, prototype, EXCEPTION_GET_ARG_SLOT))
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok());
    let Some(native_get_arg) = native_get_arg else {
        return;
    };
    if args.length() < 2 {
        let forwarded = (0..args.length())
            .map(|index| args.get(index))
            .collect::<Vec<_>>();
        if let Some(value) = native_get_arg.call(scope, args.this().into(), &forwarded) {
            rv.set(value);
        }
        return;
    }
    let tag = args.get(0);
    let Some(tag_type) = v8::Local::<v8::Object>::try_from(tag)
        .ok()
        .and_then(|tag| get_private_value(scope, tag, TAG_TYPE_SLOT))
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        if let Some(value) = native_get_arg.call(scope, args.this().into(), &[tag, args.get(1)]) {
            rv.set(value);
        }
        return;
    };
    let Some(parameters) = tag_type
        .get(scope, v8str(scope, "parameters").into())
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        if let Some(value) = native_get_arg.call(scope, args.this().into(), &[tag, args.get(1)]) {
            rv.set(value);
        }
        return;
    };
    let Some(index) = args.get(1).number_value(scope) else {
        return;
    };
    let out_of_range = !index.is_finite()
        || index.trunc() != index
        || index < 0.0
        || index >= parameters.length() as f64;
    if out_of_range {
        throw_range_error(scope, "WebAssembly.Exception index is out of range");
        return;
    }
    let index_value = v8::Number::new(scope, index);
    if let Some(value) = native_get_arg.call(scope, args.this().into(), &[tag, index_value.into()])
    {
        rv.set(value);
    }
}
