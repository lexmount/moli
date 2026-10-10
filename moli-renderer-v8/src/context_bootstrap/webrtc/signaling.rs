//! Authoritative description slots and signaling events. Events belong to the
//! connection's realm; operation promises keep their own calling realm.

use crate::{
    context_bootstrap::{events, exposed_interfaces, media_queries},
    util::{callback_data_index_value, callback_data_item, get_private_value, set_private_value},
    web_api_interfaces,
};
use moli_webapi_declare::WebApiFunctionTemplate;

pub(super) const CURRENT_LOCAL: &str = "__moliRtcPeerConnectionCurrentLocalDescription";
pub(super) const PENDING_LOCAL: &str = "__moliRtcPeerConnectionPendingLocalDescription";
pub(super) const CURRENT_REMOTE: &str = "__moliRtcPeerConnectionCurrentRemoteDescription";
pub(super) const PENDING_REMOTE: &str = "__moliRtcPeerConnectionPendingRemoteDescription";
pub(super) const HANDLER: &str = "__moliRtcOnSignalingStateChange";

const DESCRIPTIONS: &[(&str, Option<&str>)] = &[
    (PENDING_LOCAL, Some(CURRENT_LOCAL)),
    (CURRENT_LOCAL, None),
    (PENDING_LOCAL, None),
    (PENDING_REMOTE, Some(CURRENT_REMOTE)),
    (CURRENT_REMOTE, None),
    (PENDING_REMOTE, None),
];

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCPeerConnection, enumerable, receiver)]
struct Prototype {
    #[webapi(accessor_property, getter = description_getter, data = callback_data_index_value(scope, 0))]
    local_description: (),
    #[webapi(accessor_property, getter = description_getter, data = callback_data_index_value(scope, 1))]
    current_local_description: (),
    #[webapi(accessor_property, getter = description_getter, data = callback_data_index_value(scope, 2))]
    pending_local_description: (),
    #[webapi(accessor_property, getter = description_getter, data = callback_data_index_value(scope, 3))]
    remote_description: (),
    #[webapi(accessor_property, getter = description_getter, data = callback_data_index_value(scope, 4))]
    current_remote_description: (),
    #[webapi(accessor_property, getter = description_getter, data = callback_data_index_value(scope, 5))]
    pending_remote_description: (),
    #[webapi(accessor_property, getter = handler_getter, setter = handler_setter)]
    onsignalingstatechange: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    Prototype::initialize_prototype_template(scope, prototype);
}

fn description<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
    slot: &str,
    fallback: Option<&str>,
) -> v8::Local<'s, v8::Value> {
    let value = get_private_value(scope, pc, slot).expect("native description slot");
    if value.is_null()
        && let Some(fallback) = fallback
    {
        return get_private_value(scope, pc, fallback).expect("current description slot");
    }
    value
}

pub(super) fn local_description<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    description(scope, pc, PENDING_LOCAL, Some(CURRENT_LOCAL))
}

fn description_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let (slot, fallback) = callback_data_item(scope, &args, DESCRIPTIONS, "native descriptions")
        .expect("description selector");
    let pc = super::rtp_transceivers::target(scope, args.this());
    rv.set(description(scope, pc, slot, fallback));
}

fn handler_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let pc = super::rtp_transceivers::target(scope, args.this());
    rv.set(get_private_value(scope, pc, HANDLER).expect("native signaling handler"));
}

fn handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let pc = super::rtp_transceivers::target(scope, args.this());
    let active = args.get(0).is_object();
    let value = if active {
        args.get(0)
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, pc, HANDLER, value);
    media_queries::simple_object_event_set_ordered_handler(
        scope,
        pc,
        super::RTC_PEER_CONNECTION_LISTENERS_SLOT,
        "signalingstatechange",
        HANDLER,
        active,
    );
}

pub(super) fn dispatch_state_change<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
) -> bool {
    dispatch_event(
        scope,
        pc,
        super::RTC_PEER_CONNECTION_LISTENERS_SLOT,
        "signalingstatechange",
    )
}

pub(super) fn dispatch_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    listeners: &'static str,
    event_type: &'static str,
) -> bool {
    let context = target
        .get_creation_context(scope)
        .expect("event target realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let state = events::new_event_state(scope);
    events::initialize_event_object(scope, state, event_type, false, false);
    web_api_interfaces::initialize(scope, state, "Event").expect("native WebRTC event");
    events::mark_event_trusted(scope, state);
    let wrapper = exposed_interfaces::build_intrinsic_interface_instance(scope, "Event")
        .expect("Event instance");
    let prototype = exposed_interfaces::ensure_intrinsic_interface_prototype(scope, "Event")
        .expect("Event prototype");
    if wrapper.set_prototype(scope, prototype.into()) != Some(true)
        || events::initialize_event_wrapper(scope, wrapper, state).is_none()
    {
        return false;
    }
    media_queries::dispatch_simple_event_target_event(
        scope, target, listeners, event_type, wrapper,
    );
    true
}
