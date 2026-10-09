//! Web MIDI bindings without a platform device backend. Requests and port I/O
//! fail explicitly; native readonly maps retain live WebIDL collection semantics.

use super::super::{context_host_ptr_from_global_bridge, media_queries};
use crate::{
    util::{get_private_object, get_private_value, set_private_value, v8str},
    web_api_interfaces, webidl,
    webidl_iterator::{
        MaplikeWebIdlIteratorMethod, call_live_maplike_webidl_for_each,
        new_live_maplike_webidl_iterator,
    },
};
use moli_webapi_declare::WebApiFunctionTemplate;

const BACKING: &str = "__moliMidiMapBacking";
const LISTENERS: &str = "__moliMidiListeners";
const STATE_HANDLER: &str = "__moliMidiStateChangeHandler";
const MESSAGE_HANDLER: &str = "__moliMidiMessageHandler";

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MIDIOptions")]
struct MidiOptions {
    software: Option<bool>,
    sysex: Option<bool>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Navigator.requestMIDIAccess")]
struct RequestArgs {
    #[webidl(dictionary)]
    options: MidiOptions,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MIDIOutput.send")]
struct SendArgs {
    #[webidl(required, sequence)]
    data: Vec<u8>,
    #[webidl(converter = "double", default = 0.0)]
    timestamp: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MIDI map lookup")]
struct KeyArgs {
    // Generated maplike lookups convert missing keys as undefined, rather
    // than imposing the argument-count check of an ordinary IDL operation.
    #[webidl(converter = "raw", default = webidl::DomString16("undefined".encode_utf16().collect()))]
    key: webidl::DomString16,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MIDI map forEach")]
struct ForEachArgs<'s> {
    #[webidl(required)]
    callback: v8::Local<'s, v8::Value>,
    #[webidl(default = v8::undefined(scope).into())]
    this_arg: v8::Local<'s, v8::Value>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Navigator, enumerable, receiver)]
struct NavigatorMidiPrototype {
    #[webapi(method = "requestMIDIAccess", returns_promise, length = 0, callback = request)]
    request_midi_access: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MIDIAccess, enumerable, receiver)]
struct AccessPrototype {
    #[webapi(accessor_property, getter = backend_unavailable)]
    inputs: (),
    #[webapi(accessor_property, getter = backend_unavailable)]
    outputs: (),
    #[webapi(accessor_property, getter = handler_getter, setter = handler_setter, data = v8str(scope, STATE_HANDLER))]
    onstatechange: (),
    #[webapi(accessor_property, getter = backend_unavailable)]
    sysex_enabled: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MIDIPort, enumerable, receiver)]
struct PortPrototype {
    #[webapi(accessor_property, getter = backend_unavailable)]
    id: (),
    #[webapi(accessor_property, getter = backend_unavailable)]
    manufacturer: (),
    #[webapi(accessor_property, getter = backend_unavailable)]
    name: (),
    #[webapi(accessor_property, getter = backend_unavailable)]
    version: (),
    #[webapi(accessor_property = "type", getter = backend_unavailable)]
    port_type: (),
    #[webapi(accessor_property, getter = backend_unavailable)]
    state: (),
    #[webapi(accessor_property, getter = backend_unavailable)]
    connection: (),
    #[webapi(accessor_property, getter = handler_getter, setter = handler_setter, data = v8str(scope, STATE_HANDLER))]
    onstatechange: (),
    #[webapi(method, returns_promise, length = 0, callback = backend_unavailable)]
    open: (),
    #[webapi(method, returns_promise, length = 0, callback = backend_unavailable)]
    close: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MIDIInput, enumerable, receiver)]
struct InputPrototype {
    #[webapi(accessor_property, getter = handler_getter, setter = handler_setter, data = v8str(scope, MESSAGE_HANDLER))]
    onmidimessage: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MIDIOutput, enumerable, receiver)]
struct OutputPrototype {
    #[webapi(method, length = 1, callback = send)]
    send: (),
    #[webapi(method, length = 0, callback = backend_unavailable)]
    clear: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MIDIInputMap, enumerable, receiver)]
struct InputMapPrototype {
    #[webapi(accessor_property, getter = map_size)]
    size: (),
    #[webapi(method, length = 1, callback = map_get)]
    get: (),
    #[webapi(method, length = 1, callback = map_has)]
    has: (),
    #[webapi(method, length = 0, callback = map_entries)]
    entries: (),
    #[webapi(method, length = 0, callback = map_keys)]
    keys: (),
    #[webapi(method, length = 0, callback = map_values)]
    values: (),
    #[webapi(method, length = 1, callback = map_for_each)]
    for_each: (),
    #[webapi(alias = "entries", symbol = "iterator")]
    iterator: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MIDIOutputMap, enumerable, receiver)]
struct OutputMapPrototype {
    #[webapi(accessor_property, getter = map_size)]
    size: (),
    #[webapi(method, length = 1, callback = map_get)]
    get: (),
    #[webapi(method, length = 1, callback = map_has)]
    has: (),
    #[webapi(method, length = 0, callback = map_entries)]
    entries: (),
    #[webapi(method, length = 0, callback = map_keys)]
    keys: (),
    #[webapi(method, length = 0, callback = map_values)]
    values: (),
    #[webapi(method, length = 1, callback = map_for_each)]
    for_each: (),
    #[webapi(alias = "entries", symbol = "iterator")]
    iterator: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface {
        "Navigator" => NavigatorMidiPrototype::initialize_prototype_template(scope, prototype),
        "MIDIAccess" => AccessPrototype::initialize_prototype_template(scope, prototype),
        "MIDIPort" => PortPrototype::initialize_prototype_template(scope, prototype),
        "MIDIInput" => InputPrototype::initialize_prototype_template(scope, prototype),
        "MIDIOutput" => OutputPrototype::initialize_prototype_template(scope, prototype),
        "MIDIInputMap" => InputMapPrototype::initialize_prototype_template(scope, prototype),
        "MIDIOutputMap" => OutputMapPrototype::initialize_prototype_template(scope, prototype),
        _ => (),
    }
}

fn request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<RequestArgs>(scope, &args) else {
        return;
    };
    let _ = (parsed.options.software, parsed.options.sysex);
    // The algorithm checks the calling context's Document, not the borrowed
    // Navigator receiver or mutable public document/permissions properties.
    let context = scope.get_current_context();
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        // SAFETY: the bridge owns the host for the duration of this callback.
        let host = unsafe { &*host_ptr };
        if let Some(identity) =
            host.window_execution_context_identity_for_v8_context(scope, context)
            && host.window_execution_context_identity_is_current(identity)
            && host
                .document_permissions_policy_for_owner(identity.dispatch_scope())
                .is_some_and(|policy| !policy.midi_enabled())
        {
            webidl::throw_dom_exception(
                scope,
                "NotAllowedError",
                "MIDI access is disallowed by permissions policy.",
            );
            return;
        }
    }
    backend_unavailable(scope, args, rv);
}

fn backend_unavailable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    webidl::throw_dom_exception(
        scope,
        "NotSupportedError",
        "The platform MIDI device backend is not implemented.",
    );
}

fn send<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<SendArgs>(scope, &args) else {
        return;
    };
    let _ = (parsed.data, parsed.timestamp);
    // Byte-sequence conversion is implemented; message syntax, sysex permission,
    // connection state and scheduling belong to the future platform backend.
    backend_unavailable(scope, args, rv);
}

fn target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    moli_webapi_declare::web_api_object_target(scope, receiver).expect("validated MIDI receiver")
}

fn handler_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> (&'static str, &'static str) {
    match data.to_rust_string_lossy(scope).as_str() {
        STATE_HANDLER => (STATE_HANDLER, "statechange"),
        MESSAGE_HANDLER => (MESSAGE_HANDLER, "midimessage"),
        _ => unreachable!("native MIDI handler metadata"),
    }
}

fn handler_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let object = target(scope, args.this());
    let (slot, _) = handler_slot(scope, args.data());
    let value = get_private_value(scope, object, slot)
        .filter(|value| !value.is_undefined())
        .unwrap_or_else(|| v8::null(scope).into());
    rv.set(value);
}

fn handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let object = target(scope, args.this());
    let (slot, event) = handler_slot(scope, args.data());
    media_queries::mark_simple_event_target_slot(scope, object, LISTENERS);
    media_queries::install_simple_event_target_ordered_handlers(scope, object);
    let active = args.get(0).is_object();
    let value = if active {
        args.get(0)
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, object, slot, value);
    media_queries::simple_object_event_set_ordered_handler(
        scope, object, LISTENERS, event, slot, active,
    );
}

fn backing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Map>> {
    let object = target(scope, receiver);
    if let Some(map) = get_private_object(scope, object, BACKING)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
    {
        Some(map)
    } else {
        webidl::throw_dom_exception(
            scope,
            "NotSupportedError",
            "The native MIDI map has no device collection backing.",
        );
        None
    }
}

fn map_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(map) = backing(scope, args.this()) {
        rv.set(v8::Number::new(scope, map.size() as f64).into());
    }
}

fn map_get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<KeyArgs>(scope, &args) else {
        return;
    };
    let Some(map) = backing(scope, args.this()) else {
        return;
    };
    let key = v8::String::new_from_two_byte(scope, &parsed.key.0, v8::NewStringType::Normal)
        .expect("MIDI map key");
    if let Some(value) = map.get(scope, key.into()) {
        rv.set(value);
    }
}

fn map_has<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<KeyArgs>(scope, &args) else {
        return;
    };
    let Some(map) = backing(scope, args.this()) else {
        return;
    };
    let key = v8::String::new_from_two_byte(scope, &parsed.key.0, v8::NewStringType::Normal)
        .expect("MIDI map key");
    if let Some(value) = map.has(scope, key.into()) {
        rv.set_bool(value);
    }
}

fn map_for_each<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<ForEachArgs>(scope, &args) else {
        return;
    };
    let Some(map) = backing(scope, args.this()) else {
        return;
    };
    if let Some(result) = call_live_maplike_webidl_for_each(
        scope,
        map,
        args.this(),
        parsed.callback,
        parsed.this_arg,
        "MIDI map forEach",
    ) {
        rv.set(result);
    }
}

fn map_iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
    method: MaplikeWebIdlIteratorMethod,
) {
    let Some(map) = backing(scope, args.this()) else {
        return;
    };
    if let Some(iterator) = new_live_maplike_webidl_iterator(scope, map, method) {
        rv.set(iterator.into());
    }
}

fn map_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s>,
) {
    map_iterator(scope, args, rv, MaplikeWebIdlIteratorMethod::Entries);
}

fn map_keys<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s>,
) {
    map_iterator(scope, args, rv, MaplikeWebIdlIteratorMethod::Keys);
}

fn map_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s>,
) {
    map_iterator(scope, args, rv, MaplikeWebIdlIteratorMethod::Values);
}

/// Test-owned native collection; never exposed through requestMIDIAccess and
/// never claims permission to access or enumerate actual platform MIDI devices.
#[cfg(test)]
pub(crate) fn map_for_test<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    interface: &'static str,
    map: v8::Local<'s, v8::Map>,
) -> v8::Local<'s, v8::Object> {
    let prototype =
        crate::context_bootstrap::ensure_intrinsic_interface_prototype(scope, interface).unwrap();
    let object = v8::Object::new(scope);
    web_api_interfaces::initialize(scope, object, interface).unwrap();
    assert_eq!(object.set_prototype(scope, prototype.into()), Some(true));
    set_private_value(scope, object, BACKING, map.into());
    object
}
