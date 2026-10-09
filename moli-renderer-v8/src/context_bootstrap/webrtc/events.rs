use super::{ice_candidate::ice_candidate_receiver_branded, rtc_data_channel_receiver_branded};
use crate::web_api_interfaces;
use crate::{
    context_bootstrap::events::{
        event_private_value, initialize_event_object_with_type, initialize_event_wrapper,
        new_event_state, parse_event_init,
    },
    util::{callback_data_index_value, callback_data_item, throw_type_error, v8str},
    webidl::{self, WebIdlConverter},
};
use moli_webapi_declare::{
    WebApiFunctionTemplate, WebApiObject, initialize_web_api_constructor_receiver,
};

const ICE_EVENT_CANDIDATE_SLOT: &str = "__moliRtcIceEventCandidate";
const ICE_EVENT_URL_SLOT: &str = "__moliRtcIceEventUrl";
const DATA_CHANNEL_EVENT_CHANNEL_SLOT: &str = "__moliRtcDataChannelEventChannel";
const ERROR_EVENT_ERROR_SLOT: &str = "__moliRtcErrorEventError";
const EVENT_MEMBER_SLOTS: &[&str] = &[
    ICE_EVENT_CANDIDATE_SLOT,
    ICE_EVENT_URL_SLOT,
    DATA_CHANNEL_EVENT_CHANNEL_SLOT,
    ERROR_EVENT_ERROR_SLOT,
];

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCPeerConnectionIceEvent)]
struct IceEventObjectDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    composed: bool,
    #[webapi(slot = ICE_EVENT_CANDIDATE_SLOT)]
    candidate: v8::Local<'scope, v8::Value>,
    #[webapi(slot = ICE_EVENT_URL_SLOT)]
    url: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCDataChannelEvent)]
struct DataChannelEventObjectDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    composed: bool,
    #[webapi(slot = DATA_CHANNEL_EVENT_CHANNEL_SLOT)]
    channel: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCErrorEvent)]
struct ErrorEventObjectDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    composed: bool,
    #[webapi(slot = ERROR_EVENT_ERROR_SLOT)]
    error: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCPeerConnectionIceEvent, receiver, enumerable)]
struct IceEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = event_member_getter, data = callback_data_index_value(scope, 0))]
    candidate: (),
    #[webapi(accessor_property, getter = event_member_getter, data = callback_data_index_value(scope, 1))]
    url: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCDataChannelEvent, receiver, enumerable)]
struct DataChannelEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = event_member_getter, data = callback_data_index_value(scope, 2))]
    channel: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCErrorEvent, receiver, enumerable)]
struct ErrorEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = event_member_getter, data = callback_data_index_value(scope, 3))]
    error: (),
}

pub(super) fn install_event_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "RTCPeerConnectionIceEvent" => {
            IceEventPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "RTCDataChannelEvent" => {
            DataChannelEventPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "RTCErrorEvent" => {
            ErrorEventPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        _ => unreachable!("unsupported WebRTC event interface"),
    }
}

pub(in crate::context_bootstrap) fn rtc_peer_connection_ice_event_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    construct_event(scope, args, rv, EventKind::IceCandidate);
}

pub(in crate::context_bootstrap) fn rtc_data_channel_event_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    construct_event(scope, args, rv, EventKind::DataChannel);
}

pub(in crate::context_bootstrap) fn rtc_error_event_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    construct_event(scope, args, rv, EventKind::Error);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EventKind {
    IceCandidate,
    DataChannel,
    Error,
}

fn construct_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    kind: EventKind,
) {
    let (name, required_arguments, member, member_interface) = match kind {
        EventKind::IceCandidate => (
            "RTCPeerConnectionIceEvent",
            1,
            "candidate",
            "RTCIceCandidate",
        ),
        EventKind::DataChannel => ("RTCDataChannelEvent", 2, "channel", "RTCDataChannel"),
        EventKind::Error => ("RTCErrorEvent", 2, "error", "RTCError"),
    };
    if !args.is_construct_call() || args.length() < required_arguments {
        throw_type_error(
            scope,
            &format!("{name} requires 'new' and {required_arguments} argument(s)."),
        );
        return;
    }
    if args.get(0).is_symbol() {
        throw_type_error(scope, "Cannot convert a Symbol to a DOMString.");
        return;
    }
    // Retain the DOMString in V8 instead of replacing lone UTF-16 surrogates.
    let Some(event_type) = args.get(0).to_string(scope) else {
        return;
    };
    let init = match webidl::dictionary_arg(&args, 1, webidl::Context::argument(name, 2)) {
        Ok(init) => init,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    // Convert inherited EventInit members before the subclass's own members.
    let (bubbles, cancelable, composed) = match parse_event_init(scope, init) {
        Ok(flags) => flags,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let value = if let Some(init) = init {
        let Some(value) = init.get(scope, v8str(scope, member).into()) else {
            return;
        };
        value
    } else {
        v8::undefined(scope).into()
    };
    let value = if kind == EventKind::IceCandidate && value.is_null_or_undefined() {
        v8::null(scope).into()
    } else {
        let branded =
            v8::Local::<v8::Object>::try_from(value)
                .ok()
                .is_some_and(|object| match kind {
                    EventKind::IceCandidate => ice_candidate_receiver_branded(scope, object),
                    EventKind::DataChannel => rtc_data_channel_receiver_branded(scope, object),
                    EventKind::Error => web_api_interfaces::RTCError::is_instance(scope, object),
                });
        if !branded {
            throw_type_error(
                scope,
                &format!("{name}.{member} must be a genuine {member_interface} object."),
            );
            return;
        }
        value
    };
    let mut url = v8::null(scope).into();
    if kind == EventKind::IceCandidate
        && let Some(init) = init
    {
        let Some(raw) = init.get(scope, v8str(scope, "url").into()) else {
            return;
        };
        if !raw.is_null_or_undefined() {
            let converted = match webidl::UsvString::convert(
                scope,
                raw,
                webidl::Context::member("RTCPeerConnectionIceEventInit", "url"),
                &Default::default(),
            ) {
                Ok(value) => value.0,
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return;
                }
            };
            let Some(converted) = v8::String::new(scope, &converted) else {
                return;
            };
            url = converted.into();
        }
    }
    let wrapper = args.this();
    if !initialize_web_api_constructor_receiver(scope, wrapper, name) {
        return;
    }
    let event = if kind == EventKind::Error {
        new_event_state(scope)
    } else {
        wrapper
    };
    initialize_event_object_with_type(scope, event, event_type, bubbles, cancelable);
    let initialized = match kind {
        EventKind::IceCandidate => {
            IceEventObjectDeclaration::new(composed, value, url).initialize(scope, event)
        }
        EventKind::DataChannel => {
            DataChannelEventObjectDeclaration::new(composed, value).initialize(scope, event)
        }
        EventKind::Error => {
            ErrorEventObjectDeclaration::new(composed, value).initialize(scope, event)
        }
    };
    if initialized.is_err() {
        return;
    }
    if kind == EventKind::Error && initialize_event_wrapper(scope, wrapper, event).is_none() {
        return;
    }
    rv.set(wrapper.into());
}

fn event_member_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(slot) = callback_data_item(scope, &args, EVENT_MEMBER_SLOTS, "WebRTC event attribute")
    else {
        return;
    };
    if let Some(value) = event_private_value(scope, args.this(), slot) {
        rv.set(value);
    } else {
        throw_type_error(scope, "Illegal invocation");
    }
}
