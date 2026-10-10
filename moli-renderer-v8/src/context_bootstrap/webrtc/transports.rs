//! Native local transports. Applying an offer exposes the planned ICE/DTLS
//! objects; only a transport backend may supply candidates or advance them.

use super::*;
use crate::page_task_queue::RendererPageWebRtcTaskKind;
use crate::util::get_private_object;
use base64::Engine as _;

pub(super) const RTP_TRANSPORT: &str = "__moliRtpTransport";
const OWNED: &str = "__moliRtcOwnedTransports";
const BUNDLED: &str = "__moliRtcBundledTransport";
const OWNER: &str = "__moliRtcTransportOwner";
const ICE: &str = "__moliRtcIceTransport";
const STATE: &str = "__moliRtcTransportState";
const APPLIED: &str = "__moliRtcIceLocalDescriptionApplied";
const UFRAG: &str = "__moliRtcIceUsernameFragment";
const PASSWORD: &str = "__moliRtcIcePassword";
const LISTENERS: &str = "__moliRtcTransportListeners";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCIceTransport, require_prototype)]
struct IceTransport<'s> {
    #[webapi(slot = UFRAG)]
    username_fragment: v8::Local<'s, v8::String>,
    #[webapi(slot = PASSWORD)]
    password: v8::Local<'s, v8::String>,
    #[webapi(slot = STATE, init = string("new"))]
    state: (),
    #[webapi(slot = APPLIED, init = false)]
    applied: (),
    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = LISTENERS)]
    event_target_slot: (),
    #[webapi(slot = SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCDtlsTransport, require_prototype)]
struct DtlsTransport<'s> {
    #[webapi(slot = OWNER)]
    owner: v8::Local<'s, v8::Object>,
    #[webapi(slot = ICE)]
    ice_transport: v8::Local<'s, v8::Object>,
    #[webapi(slot = STATE, init = string("new"))]
    state: (),
    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = LISTENERS)]
    event_target_slot: (),
    #[webapi(slot = SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct IceParameters<'s> {
    #[webapi(data_property, enumerable)]
    username_fragment: v8::Local<'s, v8::String>,
    #[webapi(data_property, enumerable)]
    password: v8::Local<'s, v8::String>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCIceTransport, enumerable, receiver)]
struct IcePrototype {
    #[webapi(accessor_property, getter = constant, data = v8str(scope, "rtp"))]
    component: (),
    #[webapi(accessor_property, getter = constant, data = v8str(scope, "unknown"))]
    role: (),
    #[webapi(accessor_property, getter = value, data = v8str(scope, STATE))]
    state: (),
    #[webapi(accessor_property, getter = constant, data = v8str(scope, "new"))]
    gathering_state: (),
    #[webapi(accessor_property, getter = handler, setter = set_handler, data = v8str(scope, "gatheringstatechange"))]
    ongatheringstatechange: (),
    #[webapi(accessor_property, getter = handler, setter = set_handler, data = v8str(scope, "statechange"))]
    onstatechange: (),
    #[webapi(accessor_property, getter = handler, setter = set_handler, data = v8str(scope, "selectedcandidatepairchange"))]
    onselectedcandidatepairchange: (),
    #[webapi(method, length = 0, callback = empty_sequence)]
    get_local_candidates: (),
    #[webapi(method, length = 0, callback = empty_sequence)]
    get_remote_candidates: (),
    #[webapi(method, length = 0, callback = absent)]
    get_selected_candidate_pair: (),
    #[webapi(method, length = 0, callback = local_parameters)]
    get_local_parameters: (),
    #[webapi(method, length = 0, callback = absent)]
    get_remote_parameters: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCDtlsTransport, enumerable, receiver)]
struct DtlsPrototype {
    #[webapi(accessor_property, getter = value, data = v8str(scope, ICE))]
    ice_transport: (),
    #[webapi(accessor_property, getter = value, data = v8str(scope, STATE))]
    state: (),
    #[webapi(method, length = 0, callback = empty_sequence)]
    get_remote_certificates: (),
    #[webapi(accessor_property, getter = handler, setter = set_handler, data = v8str(scope, "statechange"))]
    onstatechange: (),
    #[webapi(accessor_property, getter = handler, setter = set_handler, data = v8str(scope, "error"))]
    onerror: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    name: &str,
) {
    match name {
        "RTCIceTransport" => IcePrototype::initialize_prototype_template(scope, prototype),
        "RTCDtlsTransport" => DtlsPrototype::initialize_prototype_template(scope, prototype),
        _ => unreachable!("local transport interface"),
    }
}

pub(super) fn initialize<'s>(scope: &mut v8::PinScope<'s, '_>, pc: v8::Local<'s, v8::Object>) {
    set_private_value(scope, pc, OWNED, v8::Set::new(scope).into());
    clear_bundle(scope, pc);
}

fn owned<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Set> {
    v8::Local::try_from(get_private_value(scope, pc, OWNED).expect("owned transports"))
        .expect("native transport Set")
}

fn ice<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    dtls: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    get_private_object(scope, dtls, ICE).expect("DTLS ICE transport")
}

fn build<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
    username_fragment: v8::Local<'s, v8::String>,
    password: v8::Local<'s, v8::String>,
) -> Option<v8::Local<'s, v8::Object>> {
    // Transport objects belong to the connection, even when its methods were
    // borrowed from another realm. Never consult replaceable global constructors.
    let context = pc.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let ice = IceTransport::new(username_fragment, password)
        .bind(scope)
        .ok()?;
    let dtls = DtlsTransport::new(pc, ice).bind(scope).ok()?;
    owned(scope, pc).add(scope, dtls.into())?;
    Some(dtls)
}

pub(super) fn plan<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
    existing: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let bundled = configuration::max_bundle(scope, pc);
    if let Some(transport) = existing.or_else(|| {
        bundled
            .then(|| get_private_object(scope, pc, BUNDLED))
            .flatten()
    }) {
        return Some(transport);
    }
    // Independent ICE credentials for unbundled transports; each password has
    // 128 bits of entropy. Unpadded base64 uses RFC 8445's ice-char alphabet.
    let mut bytes = [0_u8; 32];
    if moli_crypto::fill_secure_random(&mut bytes).is_err() {
        crate::native_bridge::throw_dom_exception(
            scope,
            "UnknownError",
            0,
            "The ICE credentials could not be generated.",
        );
        return None;
    }
    let username_fragment = v8_string(
        scope,
        &base64::engine::general_purpose::STANDARD_NO_PAD.encode(&bytes[..16]),
    )?;
    let password = v8_string(
        scope,
        &base64::engine::general_purpose::STANDARD_NO_PAD.encode(&bytes[16..]),
    )?;
    let transport = build(scope, pc, username_fragment, password)?;
    if bundled {
        set_private_value(scope, pc, BUNDLED, transport.into());
    }
    Some(transport)
}

/// An older locally generated offer remains applicable after rollback. Its
/// credentials may be reused, but its closed observable transport may not.
pub(super) fn from_offer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
    planned: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if !get_private_value(scope, planned, STATE)?.strict_equals(v8str(scope, "closed").into()) {
        return Some(planned);
    }
    let ice = ice(scope, planned);
    let username_fragment = v8::Local::try_from(get_private_value(scope, ice, UFRAG)?).ok()?;
    let password = v8::Local::try_from(get_private_value(scope, ice, PASSWORD)?).ok()?;
    build(scope, pc, username_fragment, password)
}

pub(super) fn sdp_attributes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transport: v8::Local<'s, v8::Object>,
) -> String {
    let ice = ice(scope, transport);
    let ufrag = get_private_value(scope, ice, UFRAG)
        .expect("ICE ufrag")
        .to_rust_string_lossy(scope);
    let password = get_private_value(scope, ice, PASSWORD)
        .expect("ICE password")
        .to_rust_string_lossy(scope);
    format!("a=ice-ufrag:{ufrag}\r\na=ice-pwd:{password}\r\na=setup:actpass\r\n")
}

pub(super) fn applied<'s>(scope: &mut v8::PinScope<'s, '_>, transport: v8::Local<'s, v8::Object>) {
    let ice = ice(scope, transport);
    set_private_value(scope, ice, APPLIED, v8::Boolean::new(scope, true).into());
}

pub(super) fn clear_bundle<'s>(scope: &mut v8::PinScope<'s, '_>, pc: v8::Local<'s, v8::Object>) {
    set_private_value(scope, pc, BUNDLED, v8::null(scope).into());
}

pub(super) fn remember_bundle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
    transport: v8::Local<'s, v8::Object>,
) {
    if configuration::max_bundle(scope, pc) {
        set_private_value(scope, pc, BUNDLED, transport.into());
    }
}

pub(super) fn rollback<'s>(scope: &mut v8::PinScope<'s, '_>, transport: v8::Local<'s, v8::Object>) {
    let ice = ice(scope, transport);
    set_private_value(scope, ice, APPLIED, v8::Boolean::new(scope, false).into());
    if !get_private_value(scope, transport, STATE)
        .expect("DTLS state")
        .strict_equals(v8str(scope, "closed").into())
    {
        set_string_slot(scope, transport, STATE, "closed");
        operations::queue(
            scope,
            transport,
            transport,
            RendererPageWebRtcTaskKind::DtlsStateChange,
        );
    }
}

pub(super) fn close<'s>(scope: &mut v8::PinScope<'s, '_>, pc: v8::Local<'s, v8::Object>) {
    let transports = owned(scope, pc).as_array(scope);
    for index in 0..transports.length() {
        let transport =
            v8::Local::try_from(transports.get_index(scope, index).expect("owned transport"))
                .expect("native DTLS transport");
        set_string_slot(scope, transport, STATE, "closed");
        let ice = ice(scope, transport);
        set_string_slot(scope, ice, STATE, "closed");
    }
    // PC.close() changes transport states synchronously without firing events.
    owned(scope, pc).clear();
    clear_bundle(scope, pc);
}

pub(super) fn state_change_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transport: v8::Local<'s, v8::Object>,
) -> bool {
    let pc = get_private_object(scope, transport, OWNER).expect("DTLS owner");
    if rtp_transceivers::closed(scope, pc) {
        return false;
    }
    signaling::dispatch_event(scope, transport, LISTENERS, "statechange")
}

fn constant<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(args.data());
}

fn value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = rtp_transceivers::target(scope, args.this());
    let slot = args.data().to_rust_string_lossy(scope);
    rv.set(get_private_value(scope, target, &slot).expect("transport slot"));
}

fn empty_sequence<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::Array::new(scope, 0).into());
}

fn absent<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_null();
}

fn local_parameters<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let ice = rtp_transceivers::target(scope, args.this());
    if !get_private_value(scope, ice, APPLIED)
        .expect("ICE local description flag")
        .boolean_value(scope)
    {
        rv.set_null();
        return;
    }
    let username_fragment =
        v8::Local::try_from(get_private_value(scope, ice, UFRAG).expect("ICE ufrag"))
            .expect("native ICE string");
    let password =
        v8::Local::try_from(get_private_value(scope, ice, PASSWORD).expect("ICE password"))
            .expect("native ICE string");
    if let Ok(parameters) = IceParameters::new(username_fragment, password).bind(scope) {
        rv.set(parameters.into());
    }
}

fn handler_slot(event: &str) -> &'static str {
    match event {
        "statechange" => "__moliRtcTransportOnStateChange",
        "error" => "__moliRtcTransportOnError",
        "gatheringstatechange" => "__moliRtcTransportOnGatheringStateChange",
        "selectedcandidatepairchange" => "__moliRtcTransportOnSelectedCandidatePairChange",
        _ => unreachable!("transport event callback data"),
    }
}

fn handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = rtp_transceivers::target(scope, args.this());
    let event = args.data().to_rust_string_lossy(scope);
    rv.set(
        get_private_value(scope, target, handler_slot(&event))
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

fn set_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = rtp_transceivers::target(scope, args.this());
    let event = args.data().to_rust_string_lossy(scope);
    let slot = handler_slot(&event);
    let active = args.get(0).is_object();
    let value = if active {
        args.get(0)
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, target, slot, value);
    media_queries::simple_object_event_set_ordered_handler(
        scope, target, LISTENERS, &event, slot, active,
    );
}
