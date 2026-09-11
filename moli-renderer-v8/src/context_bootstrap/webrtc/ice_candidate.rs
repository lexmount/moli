use super::ice_candidate_parser::parse_ice_candidate;
use crate::web_api_interfaces;
use crate::{
    util::{
        apply_webidl_constructor_prototype_fallback, callback_data_index_value, get_private_value,
        throw_type_error, v8str,
    },
    webidl::{self, WebIdlConverter},
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const CANDIDATE_VALUES_SLOT: &str = "__moliRtcIceCandidateValues";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCIceCandidate)]
struct IceCandidateObjectDeclaration<'scope> {
    #[webapi(slot = CANDIDATE_VALUES_SLOT)]
    values: v8::Local<'scope, v8::Array>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCIceCandidate, enumerable)]
struct IceCandidatePrototypeDeclaration {
    #[webapi(accessor_property, getter = candidate_attribute_getter, data = callback_data_index_value(scope, 0))]
    candidate: (),
    #[webapi(accessor_property, getter = candidate_attribute_getter, data = callback_data_index_value(scope, 1))]
    sdp_mid: (),
    #[webapi(accessor_property, getter = candidate_attribute_getter, data = callback_data_index_value(scope, 2))]
    sdp_m_line_index: (),
    #[webapi(accessor_property, getter = candidate_attribute_getter, data = callback_data_index_value(scope, 3))]
    foundation: (),
    #[webapi(accessor_property, getter = candidate_attribute_getter, data = callback_data_index_value(scope, 4))]
    component: (),
    #[webapi(accessor_property, getter = candidate_attribute_getter, data = callback_data_index_value(scope, 5))]
    priority: (),
    #[webapi(accessor_property, getter = candidate_attribute_getter, data = callback_data_index_value(scope, 6))]
    address: (),
    #[webapi(accessor_property, getter = candidate_attribute_getter, data = callback_data_index_value(scope, 7))]
    protocol: (),
    #[webapi(accessor_property, getter = candidate_attribute_getter, data = callback_data_index_value(scope, 8))]
    port: (),
    #[webapi(accessor_property, getter = candidate_attribute_getter, data = callback_data_index_value(scope, 9))]
    r#type: (),
    #[webapi(accessor_property, getter = candidate_attribute_getter, data = callback_data_index_value(scope, 10))]
    tcp_type: (),
    #[webapi(accessor_property, getter = candidate_attribute_getter, data = callback_data_index_value(scope, 11))]
    related_address: (),
    #[webapi(accessor_property, getter = candidate_attribute_getter, data = callback_data_index_value(scope, 12))]
    related_port: (),
    #[webapi(accessor_property, getter = candidate_attribute_getter, data = callback_data_index_value(scope, 13))]
    username_fragment: (),
    #[webapi(accessor_property, getter = candidate_attribute_getter, data = callback_data_index_value(scope, 14))]
    relay_protocol: (),
    #[webapi(accessor_property, getter = candidate_attribute_getter, data = callback_data_index_value(scope, 15))]
    url: (),
    #[webapi(method = "toJSON", length = 0, callback = candidate_to_json_callback)]
    to_json: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct IceCandidateJsonDeclaration<'scope> {
    candidate: v8::Local<'scope, v8::Value>,
    sdp_mid: v8::Local<'scope, v8::Value>,
    sdp_m_line_index: v8::Local<'scope, v8::Value>,
    username_fragment: v8::Local<'scope, v8::Value>,
}

pub(super) fn install_ice_candidate_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    IceCandidatePrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

pub(in crate::context_bootstrap) fn rtc_ice_candidate_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "RTCIceCandidate constructor requires 'new'.");
        return;
    }
    let dictionary = match webidl::dictionary_value(
        args.get(0),
        webidl::Context::argument("RTCIceCandidate", 1),
    ) {
        Ok(dictionary) => dictionary,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let Some(mut values) = parse_candidate_init(scope, dictionary) else {
        return;
    };
    if values[1].is_null() && values[2].is_null() {
        throw_type_error(scope, "RTCIceCandidate requires sdpMid or sdpMLineIndex.");
        return;
    }
    let candidate =
        v8::Local::<v8::String>::try_from(values[0]).expect("converted candidate string");
    let candidate = candidate.to_rust_string_lossy(scope);
    if let Some(parsed) = parse_ice_candidate(&candidate) {
        values[3] = v8::String::new(scope, parsed.foundation).unwrap().into();
        values[4] = v8str(scope, parsed.component).into();
        values[5] = v8::Integer::new_from_unsigned(scope, parsed.priority).into();
        values[6] = v8::String::new(scope, parsed.address).unwrap().into();
        values[7] = v8str(scope, parsed.protocol).into();
        values[8] = v8::Integer::new_from_unsigned(scope, parsed.port.into()).into();
        values[9] = v8str(scope, parsed.kind).into();
        if let Some(tcp_type) = parsed.tcp_type {
            values[10] = v8str(scope, tcp_type).into();
        }
        if let Some(address) = parsed.related_address {
            values[11] = v8::String::new(scope, address).unwrap().into();
        }
        if let Some(port) = parsed.related_port {
            values[12] = v8::Integer::new_from_unsigned(scope, port.into()).into();
        }
    }
    let values = v8::Array::new_with_elements(scope, &values);
    if IceCandidateObjectDeclaration::new(values)
        .initialize(scope, args.this())
        .is_err()
    {
        return;
    }
    apply_webidl_constructor_prototype_fallback(
        scope,
        args.this(),
        args.new_target(),
        "RTCIceCandidate",
    );
    rv.set(args.this().into());
}

fn parse_candidate_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    dictionary: Option<v8::Local<'s, v8::Object>>,
) -> Option<[v8::Local<'s, v8::Value>; 16]> {
    let mut values = [v8::null(scope).into(); 16];
    values[0] = v8str(scope, "").into();
    let Some(dictionary) = dictionary else {
        return Some(values);
    };
    // Convert base dictionary members in lexical order before derived members.
    // Keep DOMStrings in V8, preserving lone UTF-16 surrogates; only `url` is a USVString.
    for (name, index) in [
        ("candidate", 0),
        ("sdpMLineIndex", 2),
        ("sdpMid", 1),
        ("usernameFragment", 13),
        ("relayProtocol", 14),
        ("url", 15),
    ] {
        let raw = dictionary.get(scope, v8str(scope, name).into())?;
        if raw.is_undefined() || (index != 0 && raw.is_null()) {
            continue;
        }
        let context = webidl::Context::member("RTCLocalIceCandidateInit", name);
        if index == 2 {
            let value = match webidl::UnsignedShort::convert(scope, raw, context, &()) {
                Ok(value) => value.0,
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return None;
                }
            };
            values[index] = v8::Integer::new_from_unsigned(scope, value.into()).into();
        } else if index == 15 {
            let value = match webidl::UsvString::convert(scope, raw, context, &Default::default()) {
                Ok(value) => value.0,
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return None;
                }
            };
            values[index] = v8::String::new(scope, &value)?.into();
        } else {
            if raw.is_symbol() {
                throw_type_error(scope, "Cannot convert a Symbol to a DOMString.");
                return None;
            }
            let value = raw.to_string(scope)?;
            if index == 14
                && !matches!(
                    value.to_rust_string_lossy(scope).as_str(),
                    "udp" | "tcp" | "tls"
                )
            {
                throw_type_error(scope, "Invalid RTCIceServerTransportProtocol.");
                return None;
            }
            values[index] = value.into();
        }
    }
    Some(values)
}

fn candidate_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    let values = get_private_value(scope, receiver, CANDIDATE_VALUES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok());
    if values.is_none() {
        throw_type_error(scope, "Illegal invocation");
    }
    values
}

fn candidate_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(values) = candidate_values(scope, args.this()) else {
        return;
    };
    let Some(index) = args.data().uint32_value(scope) else {
        return;
    };
    if let Some(value) = values.get_index(scope, index) {
        rv.set(value);
    }
}

fn candidate_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(values) = candidate_values(scope, args.this()) else {
        return;
    };
    let Some(candidate) = values.get_index(scope, 0) else {
        return;
    };
    let Some(mid) = values.get_index(scope, 1) else {
        return;
    };
    let Some(line) = values.get_index(scope, 2) else {
        return;
    };
    let Some(fragment) = values.get_index(scope, 13) else {
        return;
    };
    if let Ok(result) = IceCandidateJsonDeclaration::new(candidate, mid, line, fragment).bind(scope)
    {
        rv.set(result.into());
    }
}
