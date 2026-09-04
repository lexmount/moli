use super::document_runtime::DomHandle;
use super::native_bridge::{DateLocaleRuntimeState, JsContextHost};
pub use moli_v8_util::{
    array_contains_strict, array_push_value, call_object_method, callback_data_index_value,
    callback_data_item, constructor_object, constructor_prototype, constructor_prototype_object,
    define_non_enumerable_static_bool_property, define_non_enumerable_static_number_property,
    define_non_enumerable_static_property, define_non_enumerable_static_string_property,
    get_own_static_property, get_private_object, get_private_value, get_property,
    global_constructor_object, global_constructor_prototype,
    initialize_intrinsic_interface_registry, new_null_prototype_object, object_bool_property,
    object_chain_contains, object_defined_string_property, object_non_empty_string_property,
    object_number_property, object_own_static_bool_property, object_own_static_property_as_array,
    object_own_static_string_property, object_property_as_array, object_property_as_object,
    object_string_property, private_key, register_intrinsic_interface,
    register_public_interface_object, registered_intrinsic_constructor,
    registered_intrinsic_prototype, registered_public_interface_object, set_null_prototype,
    set_private_value, set_symbol_to_string_tag, throw_range_error, throw_type_error,
    v8_json_parse, v8_string, v8str, walk_object_chain,
};
use moli_webapi_declare::WebApiValue;
pub(crate) use moli_webapi_declare::define_array_data_property as define_v8_array_data_property;
use std::{ptr::NonNull, rc::Rc};
use url::Url;
use widestring::U16String;

const SCRIPT_BASE_URL_HOST_DEFINED_OPTIONS_MARKER: &str = "moli-script-base-url-v1";

// Access-check callbacks cannot inspect the global object to rediscover the host:
// that property lookup would recursively invoke the same V8 access check. This
// non-owning slot is valid while the context can execute because the matching
// ScriptVm context state retains its `JsContextHostBridgeRef`.
#[derive(Debug)]
struct ContextHostPointerSlot(NonNull<JsContextHost>);

/// Materializes the unexposed prototype described by a WebIDL iterator
/// FunctionTemplate. The temporary constructor is an implementation detail, so
/// its back-reference must not leak onto the iterator prototype.
pub(crate) fn materialize_hidden_function_template_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) -> Option<v8::Local<'s, v8::Object>> {
    let constructor = template.get_function(scope)?;
    let prototype = constructor
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let constructor_key = v8str(scope, "constructor");
    let _ = prototype.delete(scope, constructor_key.into());
    Some(prototype)
}

// # Why V8 callbacks use `RefCell::as_ptr()` instead of `borrow_mut()`
//
// `JsContextHost` is stored as `Rc<RefCell<JsContextHost>>`. Every V8 callback
// needs a `*mut` pointer to call methods on it.
//
// The obvious approach — `hrc.borrow_mut()` — panics at runtime because **V8
// callbacks are inherently re-entrant**: any V8 API call (property access, object
// creation, function invocation, scope operations, etc.) can synchronously execute
// JavaScript, which can trigger *other* V8 callbacks that also need the host.
// If the outer callback holds a `RefMut` guard (or even a `Ref` guard), the inner
// callback's attempt to `borrow_mut()` (or `borrow()`) causes a `RefCell` panic.
//
// Example call chain that panics with `borrow_mut()`:
//   1. `event_target_add_event_listener_callback` — holds `borrow_mut()` guard
//   2. calls `host.add_event_listener(scope, ...)` which touches V8 scope
//   3. V8 evaluates a getter → `window_document_getter` fires
//   4. `window_document_getter` calls `hrc.borrow()` → panics (already mutably borrowed)
//
// `RefCell::as_ptr()` returns a `*mut T` without acquiring any borrow guard, so
// nested callbacks can freely access the host. This is safe because:
// - The `Rc` guarantees the underlying allocation is alive for the callback's duration.
// - All V8 callbacks run on a single thread (V8 isolates are single-threaded).
// - No `&mut` reference alias is observable across a V8 re-entry boundary, since the
//   outer callback does not hold a Rust reference while V8 is executing JavaScript.
//
// `borrow_mut()` is still used in `script_vm.rs` top-level entry points (e.g.
// `replace_document_resource_runtime`, `take_network_output`) which are called from Rust code
// outside V8 callback context, where re-entrancy cannot occur.

pub(crate) fn serialize_v8_array<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    value: T,
) -> Option<v8::Local<'s, v8::Array>>
where
    T: WebApiValue<'s>,
{
    let value = value.to_v8_value(scope)?;
    v8::Local::<v8::Array>::try_from(value).ok()
}

pub(crate) fn serialize_v8_iter_array<'s, I, T>(
    scope: &mut v8::PinScope<'s, '_>,
    values: I,
) -> Option<v8::Local<'s, v8::Array>>
where
    I: IntoIterator<Item = T>,
    T: WebApiValue<'s>,
{
    let values = values.into_iter().collect::<Vec<_>>();
    serialize_v8_array(scope, values.as_slice())
}

pub(crate) fn callable_relevant_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mut callable: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Context>> {
    // V8's GetFunctionRealm unwraps Proxy chains. Bound functions are created
    // in their target function's context, so their creation context already
    // identifies the target realm through the public embedding API.
    while callable.is_proxy() {
        let proxy = v8::Local::<v8::Proxy>::try_from(callable).ok()?;
        callable = proxy.get_target(scope);
    }
    v8::Local::<v8::Object>::try_from(callable)
        .ok()?
        .get_creation_context(scope)
}

pub(crate) fn define_v8_array_data_properties<'s, I, T>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Array>,
    values: I,
) -> Option<()>
where
    I: IntoIterator<Item = T>,
    T: WebApiValue<'s>,
{
    for (index, value) in values.into_iter().enumerate() {
        let value = value.to_v8_value(scope)?;
        define_v8_array_data_property(scope, array, index as u32, value)?;
    }
    Some(())
}

pub(super) fn create_script_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resource_name: &str,
    line_offset: i32,
) -> v8::ScriptOrigin<'s> {
    create_script_origin_with_base_url(scope, resource_name, line_offset, None)
}

pub(super) fn create_script_origin_with_base_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resource_name: &str,
    line_offset: i32,
    base_url: Option<&Url>,
) -> v8::ScriptOrigin<'s> {
    create_script_origin_with_base_url_and_nonce(scope, resource_name, line_offset, base_url, None)
}

pub(super) fn create_script_origin_with_base_url_and_nonce<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resource_name: &str,
    line_offset: i32,
    base_url: Option<&Url>,
    nonce: Option<&str>,
) -> v8::ScriptOrigin<'s> {
    let name = v8::String::new(scope, resource_name).expect("v8 string allocation");
    let host_defined_options = base_url.and_then(|base_url| {
        script_host_defined_options_with_base_url_and_nonce(scope, base_url, nonce)
    });
    v8::ScriptOrigin::new(
        scope,
        name.into(),
        line_offset,
        0,
        false,
        -1,
        None,
        false,
        false,
        false,
        host_defined_options,
    )
}

pub(crate) fn script_base_url_from_host_defined_options(
    scope: &mut v8::PinScope<'_, '_>,
    host_defined_options: v8::Local<'_, v8::Data>,
) -> Option<Url> {
    let options = script_host_defined_options_as_fixed_array(host_defined_options)?;
    if options.length() < 2 {
        return None;
    }
    let marker = options.get(scope, 0)?;
    let marker = v8::Local::<v8::String>::try_from(marker).ok()?;
    if marker.to_rust_string_lossy(scope) != SCRIPT_BASE_URL_HOST_DEFINED_OPTIONS_MARKER {
        return None;
    }
    let base_url = options.get(scope, 1)?;
    let base_url = v8::Local::<v8::String>::try_from(base_url).ok()?;
    Url::parse(&base_url.to_rust_string_lossy(scope)).ok()
}

pub(crate) fn script_nonce_from_host_defined_options(
    scope: &mut v8::PinScope<'_, '_>,
    host_defined_options: v8::Local<'_, v8::Data>,
) -> Option<String> {
    let options = script_host_defined_options_as_fixed_array(host_defined_options)?;
    if options.length() < 3 {
        return None;
    }
    let marker = options.get(scope, 0)?;
    let marker = v8::Local::<v8::String>::try_from(marker).ok()?;
    if marker.to_rust_string_lossy(scope) != SCRIPT_BASE_URL_HOST_DEFINED_OPTIONS_MARKER {
        return None;
    }
    let nonce = options.get(scope, 2)?;
    let nonce = v8::Local::<v8::String>::try_from(nonce).ok()?;
    let nonce = nonce.to_rust_string_lossy(scope);
    (!nonce.is_empty()).then_some(nonce)
}

pub(crate) fn script_parser_inserted_from_host_defined_options(
    scope: &mut v8::PinScope<'_, '_>,
    host_defined_options: v8::Local<'_, v8::Data>,
) -> Option<bool> {
    let options = script_host_defined_options_as_fixed_array(host_defined_options)?;
    if options.length() < 4 {
        return None;
    }
    let marker = options.get(scope, 0)?;
    let marker = v8::Local::<v8::String>::try_from(marker).ok()?;
    if marker.to_rust_string_lossy(scope) != SCRIPT_BASE_URL_HOST_DEFINED_OPTIONS_MARKER {
        return None;
    }
    let parser_metadata = options.get(scope, 3)?;
    let parser_metadata = v8::Local::<v8::String>::try_from(parser_metadata).ok()?;
    match parser_metadata.to_rust_string_lossy(scope).as_str() {
        "parser-inserted" => Some(true),
        "not-parser-inserted" => Some(false),
        _ => None,
    }
}

fn script_host_defined_options_as_fixed_array<'s>(
    host_defined_options: v8::Local<'s, v8::Data>,
) -> Option<v8::Local<'s, v8::FixedArray>> {
    // V8 passes ScriptOrigin host-defined options back to dynamic import
    // callbacks. The public Rust binding does not expose a PrimitiveArray
    // predicate or TryFrom<Data>, but PrimitiveArray is readable through the
    // FixedArray view and the binding has a safe Data -> FixedArray predicate.
    // Use that read-only view for both our PrimitiveArray payload and V8's
    // default empty FixedArray; the marker below decides whether it is ours.
    v8::Local::<v8::FixedArray>::try_from(host_defined_options).ok()
}

pub(crate) fn script_base_url_from_continuation_data(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<Url> {
    let value = scope.get_continuation_preserved_embedder_data();
    let value = v8::Local::<v8::String>::try_from(value).ok()?;
    Url::parse(&value.to_rust_string_lossy(scope)).ok()
}

pub(crate) fn script_base_url_continuation_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    base_url: &Url,
) -> Option<v8::Local<'s, v8::Value>> {
    v8_string(scope, base_url.as_str()).map(Into::into)
}

pub(crate) fn script_host_defined_options_with_base_url_and_nonce<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    base_url: &Url,
    nonce: Option<&str>,
) -> Option<v8::Local<'s, v8::Data>> {
    script_host_defined_options_with_fetch_metadata(scope, base_url, nonce, false)
}

pub(crate) fn script_host_defined_options_with_fetch_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    base_url: &Url,
    nonce: Option<&str>,
    parser_inserted: bool,
) -> Option<v8::Local<'s, v8::Data>> {
    let marker = v8_string(scope, SCRIPT_BASE_URL_HOST_DEFINED_OPTIONS_MARKER)?;
    let value = v8_string(scope, base_url.as_str())?;
    let nonce = v8_string(scope, nonce.unwrap_or_default())?;
    let parser_metadata = v8_string(
        scope,
        if parser_inserted {
            "parser-inserted"
        } else {
            "not-parser-inserted"
        },
    )?;
    let options = v8::PrimitiveArray::new(scope, 4);
    options.set(scope, 0, marker.into());
    options.set(scope, 1, value.into());
    options.set(scope, 2, nonce.into());
    options.set(scope, 3, parser_metadata.into());
    Some(options.into())
}

pub(super) fn enqueue_host_microtask(
    scope: &mut v8::PinScope<'_, '_>,
    callback: v8::Local<'_, v8::Function>,
) {
    scope.enqueue_microtask(callback);
}

pub(super) fn global_bridge_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    get_own_static_property(scope, global, "__moliNativeBridge")
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(super) fn call_global_bridge_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
    args: &[v8::Local<'s, v8::Value>],
) -> Option<v8::Local<'s, v8::Value>> {
    let bridge = global_bridge_object(scope)?;
    let method = bridge.get(scope, v8_string(scope, name)?.into())?;
    let method = v8::Local::<v8::Function>::try_from(method).ok()?;
    method.call(scope, bridge.into(), args)
}

/// Obtain a raw `*mut JsContextHost` pointer from the V8 global bridge.
///
/// This is the standard way to access the host inside **V8 callbacks**. It reads
/// the pointer stored in bridge internal field 0 (set during `install_into_bridge`).
///
/// We return a raw pointer instead of a `&mut` reference because V8 callbacks are
/// inherently re-entrant: any V8 API call (property access, object creation, scope
/// operations…) can synchronously execute JavaScript, which fires *other* V8
/// callbacks that also need the host. If we used `RefCell::borrow_mut()`, the
/// inner callback's `borrow_mut()` would panic ("already mutably borrowed").
///
/// See the module-level comment at the top of this file for a detailed example.
///
/// # Safety contract for callers
///
/// The returned pointer is valid for the duration of the V8 callback because the
/// owning ScriptVm page state keeps a Rust-side `Rc`, and each live V8 context
/// owns an additional bridge-ref token for the ref-count stored in the native
/// bridge object. All document V8 callbacks run on the render
/// owner thread, so there is no data race. Callers must not hold a Rust `&mut`
/// reference across a V8 API call that could trigger re-entrant JavaScript
/// execution.
pub(super) fn context_host_ptr_from_global_bridge(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<*mut JsContextHost> {
    let context = scope.get_current_context();
    if let Some(host_ptr) = context_host_ptr_from_context_slot(context) {
        return Some(host_ptr);
    }
    // Read directly from bridge internal field 0 — no Rc ref-count overhead.
    let global = context.global(scope);
    context_host_ptr_from_window_object(scope, global)
}

pub(super) fn install_context_host_pointer_slot(
    context: v8::Local<'_, v8::Context>,
    host_ptr: *mut JsContextHost,
) {
    let host_ptr =
        NonNull::new(host_ptr).expect("V8 context JsContextHost pointer should not be null");
    // SAFETY: the caller installs context slots while the owning ScriptVm's
    // `Rc<RefCell<JsContextHost>>` is alive. Clone the independently owned
    // Date/Intl state before exposing the context to script.
    let date_locale_runtime_state = unsafe { host_ptr.as_ref() }.date_locale_runtime_state();
    let previous = context.set_slot(Rc::new(ContextHostPointerSlot(host_ptr)));
    assert!(
        previous
            .as_deref()
            .is_none_or(|previous| previous.0 == host_ptr),
        "V8 context JsContextHost pointer must not be rebound"
    );
    let previous = context.set_slot(Rc::clone(&date_locale_runtime_state));
    assert!(
        previous
            .as_ref()
            .is_none_or(|previous| Rc::ptr_eq(previous, &date_locale_runtime_state)),
        "V8 context Date/Intl runtime state must not be rebound"
    );
}

pub(super) fn date_locale_runtime_state_from_context(
    context: v8::Local<'_, v8::Context>,
) -> Option<Rc<DateLocaleRuntimeState>> {
    context.get_slot::<DateLocaleRuntimeState>()
}

pub(super) fn context_host_ptr_from_context_slot(
    context: v8::Local<'_, v8::Context>,
) -> Option<*mut JsContextHost> {
    context
        .get_slot::<ContextHostPointerSlot>()
        .map(|slot| slot.0.as_ptr())
}

/// Convenience wrapper for callbacks that immediately need the host value.
///
/// This has the same re-entrancy caveat as `context_host_ptr_from_global_bridge`:
/// do not keep the returned `&mut JsContextHost` across V8 calls that can
/// synchronously enter JavaScript and re-enter native callbacks.
pub(super) fn context_host_from_global_bridge<'host>(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<&'host mut JsContextHost> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    Some(unsafe { &mut *host_ptr })
}

pub(crate) fn debug_assert_script_visible_callback_outside_structural_mutation(
    scope: &mut v8::PinScope<'_, '_>,
    action: &str,
) {
    #[cfg(debug_assertions)]
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &*host_ptr }.debug_assert_not_in_structural_mutation(action);
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = (scope, action);
    }
}

pub(crate) fn call_script_visible_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    function: v8::Local<'s, v8::Function>,
    receiver: v8::Local<'s, v8::Value>,
    args: &[v8::Local<'s, v8::Value>],
    action: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    debug_assert_script_visible_callback_outside_structural_mutation(scope, action);
    function.call(scope, receiver, args)
}

pub(super) fn context_host_ptr_from_window_object(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> Option<*mut JsContextHost> {
    if let Some(value) = object.get_internal_field(scope, 0)
        && let Ok(external) = v8::Local::<v8::External>::try_from(value)
    {
        let ptr = external.value() as *mut JsContextHost;
        if !ptr.is_null() {
            return Some(ptr);
        }
    }

    let bridge = get_own_static_property(scope, object, "__moliNativeBridge")
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let value = bridge.get_internal_field(scope, 0)?;
    let external = v8::Local::<v8::External>::try_from(value).ok()?;
    let ptr = external.value() as *mut JsContextHost;
    if ptr.is_null() { None } else { Some(ptr) }
}

pub(super) fn node_wrapper_from_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    // V8 callback helper — wrap_handle triggers V8 operations (property
    // interceptors, instantiate_wrapper) that re-enter other callbacks.
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, host_ptr, handle)
}

pub(super) fn global_bridge_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Function>)> {
    let bridge = global_bridge_object(scope)?;
    let method = bridge.get(scope, v8_string(scope, name)?.into())?;
    let method = v8::Local::<v8::Function>::try_from(method).ok()?;
    Some((bridge, method))
}

pub(super) fn callback_arg_string(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
    index: i32,
) -> Option<String> {
    args.get(index)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(crate) fn v8_string_to_u16_string(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::String>,
) -> U16String {
    let mut units = vec![0; value.length()];
    // The vendored V8 binding for String::write_v2 returns `()`. The buffer is
    // sized from the same V8 string length and the binding writes at most that
    // many UTF-16 code units.
    value.write_v2(scope, 0, &mut units, v8::WriteFlags::empty());
    U16String::from_vec(units)
}

pub(crate) fn v8_value_to_dom_string_u16<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    treat_null_as_empty_string: bool,
) -> Option<U16String> {
    if value.is_null() && treat_null_as_empty_string {
        return Some(U16String::new());
    }
    if value.is_symbol() {
        throw_type_error(scope, "Failed to convert value to DOMString.");
        return None;
    }

    let converted = {
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let mut scope = try_catch.init();
        match value.to_string(&scope) {
            Some(value) => Some(v8_string_to_u16_string(&mut scope, value)),
            None if scope.has_caught() => {
                let _ = scope.rethrow();
                return None;
            }
            None => None,
        }
    };
    if converted.is_none() {
        throw_type_error(scope, "Failed to convert value to DOMString.");
    }
    converted
}

pub(crate) fn v8_string_from_utf16_units<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    units: &[u16],
) -> Option<v8::Local<'s, v8::String>> {
    v8::String::new_from_two_byte(scope, units, v8::NewStringType::Normal)
}

pub(crate) fn utf16_units(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}

pub(crate) fn utf16_len(value: &str) -> usize {
    value.encode_utf16().count()
}

pub(crate) fn string_from_utf16_units_lossy(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

pub(crate) fn utf16_slice_units_lossy(units: &[u16], start: usize, end: usize) -> String {
    let start = start.min(units.len());
    let end = end.min(units.len()).max(start);
    string_from_utf16_units_lossy(&units[start..end])
}

pub(crate) fn utf16_slice_lossy(value: &str, start: usize, end: usize) -> String {
    let units = utf16_units(value);
    utf16_slice_units_lossy(&units, start, end)
}

pub(crate) fn utf16_split_units_lossy(units: &[u16], offset: usize) -> (String, String) {
    let offset = offset.min(units.len());
    (
        string_from_utf16_units_lossy(&units[..offset]),
        string_from_utf16_units_lossy(&units[offset..]),
    )
}

pub(crate) fn utf16_replace_units_range_lossy(
    units: &[u16],
    start: usize,
    count: usize,
    replacement: &[u16],
) -> String {
    let start = start.min(units.len());
    let end = start.saturating_add(count).min(units.len());
    let mut next = Vec::with_capacity(
        units
            .len()
            .saturating_sub(end.saturating_sub(start))
            .saturating_add(replacement.len()),
    );
    next.extend_from_slice(&units[..start]);
    next.extend_from_slice(replacement);
    next.extend_from_slice(&units[end..]);
    string_from_utf16_units_lossy(&next)
}

pub(crate) fn utf16_units_contain_unpaired_surrogate(units: &[u16]) -> bool {
    let mut index = 0;
    while index < units.len() {
        let unit = units[index];
        if (0xD800..=0xDBFF).contains(&unit) {
            if units
                .get(index + 1)
                .is_some_and(|next| (0xDC00..=0xDFFF).contains(next))
            {
                index += 2;
                continue;
            }
            return true;
        }
        if (0xDC00..=0xDFFF).contains(&unit) {
            return true;
        }
        index += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use std::pin::pin;

    use super::{
        SCRIPT_BASE_URL_HOST_DEFINED_OPTIONS_MARKER, object_chain_contains,
        script_base_url_from_host_defined_options,
        script_host_defined_options_with_base_url_and_nonce,
        script_host_defined_options_with_fetch_metadata, script_nonce_from_host_defined_options,
        script_parser_inserted_from_host_defined_options, utf16_replace_units_range_lossy,
        utf16_slice_lossy, utf16_split_units_lossy, utf16_units, walk_object_chain,
    };
    use crate::ensure_v8_for_test as ensure_v8;
    use moli_v8_util::walk_object_chain_last;
    use moli_webapi_declare::WebApiObject;
    use url::Url;

    #[derive(WebApiObject)]
    #[webapi(interface = "Object", data_properties)]
    struct TestParentObjectDeclaration<'scope> {
        parent_node: v8::Local<'scope, v8::Object>,
    }

    #[derive(WebApiObject)]
    #[webapi(interface = "Object", data_properties)]
    struct TestParentValueDeclaration<'scope> {
        parent_node: v8::Local<'scope, v8::Value>,
    }

    #[derive(WebApiObject)]
    #[webapi(interface = "Object", allow_empty)]
    struct TestIdentityObjectDeclaration {}

    fn primitive_host_defined_options<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        fields: &[&str],
    ) -> v8::Local<'s, v8::Data> {
        let options = v8::PrimitiveArray::new(scope, fields.len());
        for (index, field) in fields.iter().enumerate() {
            let value = v8::String::new(scope, field).expect("test option field");
            options.set(scope, index, value.into());
        }
        options.into()
    }

    #[test]
    fn utf16_string_helpers_use_code_unit_offsets() {
        let value = "a\u{1f306}b";
        assert_eq!(utf16_slice_lossy(value, 1, 3), "\u{1f306}");
        assert_eq!(utf16_slice_lossy(value, 2, 3), "\u{fffd}");
        let units = utf16_units(value);
        assert_eq!(
            utf16_split_units_lossy(&units, 3),
            ("\u{61}\u{1f306}".to_owned(), "b".to_owned())
        );
        assert_eq!(utf16_replace_units_range_lossy(&units, 1, 2, &[]), "ab");
        assert_eq!(
            utf16_replace_units_range_lossy(&units, 1, 2, &[b'x' as u16]),
            "axb"
        );
    }

    #[test]
    fn script_host_defined_options_read_moli_primitive_array_payload() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        let base_url = Url::parse("https://example.test/scripts/entry.js").unwrap();
        let options =
            script_host_defined_options_with_base_url_and_nonce(scope, &base_url, Some("abc123"))
                .expect("script host-defined options should allocate");

        assert_eq!(
            script_base_url_from_host_defined_options(scope, options),
            Some(base_url.clone())
        );
        assert_eq!(
            script_nonce_from_host_defined_options(scope, options),
            Some("abc123".to_owned())
        );
        assert_eq!(
            script_parser_inserted_from_host_defined_options(scope, options),
            Some(false)
        );

        let parser_options =
            script_host_defined_options_with_fetch_metadata(scope, &base_url, Some("abc123"), true)
                .expect("parser-inserted host-defined options should allocate");
        assert_eq!(
            script_parser_inserted_from_host_defined_options(scope, parser_options),
            Some(true)
        );
    }

    #[test]
    fn script_host_defined_options_ignore_non_fixed_array_data() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        let value: v8::Local<'_, v8::Value> = v8::String::new(scope, "not-moli-options")
            .expect("test string")
            .into();
        let data: v8::Local<'_, v8::Data> = value.into();

        assert_eq!(script_base_url_from_host_defined_options(scope, data), None);
        assert_eq!(script_nonce_from_host_defined_options(scope, data), None);
        assert_eq!(
            script_parser_inserted_from_host_defined_options(scope, data),
            None
        );
    }

    #[test]
    fn script_host_defined_options_ignore_malformed_payloads() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        let empty = primitive_host_defined_options(scope, &[]);
        assert_eq!(
            script_base_url_from_host_defined_options(scope, empty),
            None
        );
        assert_eq!(script_nonce_from_host_defined_options(scope, empty), None);
        assert_eq!(
            script_parser_inserted_from_host_defined_options(scope, empty),
            None
        );

        let marker_only =
            primitive_host_defined_options(scope, &[SCRIPT_BASE_URL_HOST_DEFINED_OPTIONS_MARKER]);
        assert_eq!(
            script_base_url_from_host_defined_options(scope, marker_only),
            None
        );
        assert_eq!(
            script_nonce_from_host_defined_options(scope, marker_only),
            None
        );

        let wrong_marker = primitive_host_defined_options(
            scope,
            &["not-moli-options", "https://example.test/entry.js"],
        );
        assert_eq!(
            script_base_url_from_host_defined_options(scope, wrong_marker),
            None
        );
        assert_eq!(
            script_nonce_from_host_defined_options(scope, wrong_marker),
            None
        );

        let invalid_base = primitive_host_defined_options(
            scope,
            &[SCRIPT_BASE_URL_HOST_DEFINED_OPTIONS_MARKER, "http://[::1"],
        );
        assert_eq!(
            script_base_url_from_host_defined_options(scope, invalid_base),
            None
        );
        assert_eq!(
            script_nonce_from_host_defined_options(scope, invalid_base),
            None
        );
    }

    #[test]
    fn script_host_defined_options_treat_missing_or_empty_nonce_as_absent() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        let base_url = Url::parse("https://example.test/no-nonce.js").unwrap();
        let missing_nonce = primitive_host_defined_options(
            scope,
            &[
                SCRIPT_BASE_URL_HOST_DEFINED_OPTIONS_MARKER,
                "https://example.test/no-nonce.js",
            ],
        );
        assert_eq!(
            script_base_url_from_host_defined_options(scope, missing_nonce),
            Some(base_url)
        );
        assert_eq!(
            script_nonce_from_host_defined_options(scope, missing_nonce),
            None
        );

        let empty_nonce = primitive_host_defined_options(
            scope,
            &[
                SCRIPT_BASE_URL_HOST_DEFINED_OPTIONS_MARKER,
                "https://example.test/empty-nonce.js",
                "",
            ],
        );
        assert_eq!(
            script_base_url_from_host_defined_options(scope, empty_nonce),
            Some(Url::parse("https://example.test/empty-nonce.js").unwrap())
        );
        assert_eq!(
            script_nonce_from_host_defined_options(scope, empty_nonce),
            None
        );
    }

    #[test]
    fn script_host_defined_options_ignore_v8_native_fixed_array_payload() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        let source = v8::String::new(scope, "export default 1;").expect("module source");
        let name = v8::String::new(scope, "https://example.test/native-fixed-array.js")
            .expect("module source name");
        let origin = v8::ScriptOrigin::new(
            scope,
            name.into(),
            0,
            0,
            false,
            -1,
            None,
            false,
            false,
            true,
            None,
        );
        let mut source = v8::script_compiler::Source::new(source, Some(&origin));
        let module =
            v8::script_compiler::compile_module(scope, &mut source).expect("module should compile");
        let requests = module.get_module_requests();
        assert_eq!(requests.length(), 0);

        let data: v8::Local<'_, v8::Data> = requests.into();
        assert_eq!(script_base_url_from_host_defined_options(scope, data), None);
        assert_eq!(script_nonce_from_host_defined_options(scope, data), None);
    }

    fn object_with_parent<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        parent_node: v8::Local<'s, v8::Object>,
    ) -> v8::Local<'s, v8::Object> {
        TestParentObjectDeclaration::new(parent_node)
            .bind(scope)
            .expect("test parent object declaration should bind")
    }

    fn object_with_parent_value<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        parent_node: v8::Local<'s, v8::Value>,
    ) -> v8::Local<'s, v8::Object> {
        TestParentValueDeclaration::new(parent_node)
            .bind(scope)
            .expect("test parent value declaration should bind")
    }

    fn test_identity_object<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
        TestIdentityObjectDeclaration::new()
            .bind(scope)
            .expect("test identity object declaration should bind")
    }

    #[test]
    fn walk_object_chain_follows_property_until_missing() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        let root = test_identity_object(scope);
        let middle = object_with_parent(scope, root);
        let leaf = object_with_parent(scope, middle);

        let chain = walk_object_chain(scope, leaf, "parentNode");
        assert_eq!(chain.len(), 3, "expected leaf->middle->root chain");
        assert!(chain[0].strict_equals(leaf.into()));
        assert!(chain[1].strict_equals(middle.into()));
        assert!(chain[2].strict_equals(root.into()));

        // A node with no parent property is its own one-element chain.
        let solo = test_identity_object(scope);
        let solo_chain = walk_object_chain(scope, solo, "parentNode");
        assert_eq!(solo_chain.len(), 1);
        assert!(solo_chain[0].strict_equals(solo.into()));
    }

    #[test]
    fn walk_object_chain_stops_on_non_object_property() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        // parentNode set to v8::null/undefined/Number — walk must NOT consume
        // these as objects, even though they are present on the object.
        let null = v8::null(scope);
        let leaf = object_with_parent_value(scope, null.into());
        assert_eq!(walk_object_chain(scope, leaf, "parentNode").len(), 1);

        let number = v8::Number::new(scope, 42.0);
        let leaf2 = object_with_parent_value(scope, number.into());
        assert_eq!(walk_object_chain(scope, leaf2, "parentNode").len(), 1);
    }

    #[test]
    fn walk_object_chain_last_returns_root_of_chain() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        let root = test_identity_object(scope);
        let mid = object_with_parent(scope, root);
        let leaf = object_with_parent(scope, mid);

        let last = walk_object_chain_last(scope, leaf, "parentNode");
        assert!(
            last.strict_equals(root.into()),
            "expected last() to land on root"
        );

        // Single-node chain — its last is itself.
        let solo = test_identity_object(scope);
        let solo_last = walk_object_chain_last(scope, solo, "parentNode");
        assert!(solo_last.strict_equals(solo.into()));
    }

    #[test]
    fn object_chain_contains_uses_strict_equality_not_property_equality() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        let a = test_identity_object(scope);
        let b = test_identity_object(scope);
        let c = test_identity_object(scope);
        let chain = [a, b];

        assert!(object_chain_contains(&chain, a));
        assert!(object_chain_contains(&chain, b));
        assert!(!object_chain_contains(&chain, c));

        // Two fresh-but-equivalent empty objects must not be considered the
        // same node — strict equality on object handles is identity.
        let look_alike = test_identity_object(scope);
        assert!(!object_chain_contains(&chain, look_alike));

        // An empty chain trivially contains nothing.
        let empty: [v8::Local<v8::Object>; 0] = [];
        assert!(!object_chain_contains(&empty, a));
    }
}
