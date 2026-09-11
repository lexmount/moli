use super::RtcSessionDescriptionInitDeclaration;
use crate::web_api_interfaces;
use crate::{
    util::{
        apply_webidl_constructor_prototype_fallback, callback_data_index_value, callback_data_item,
        get_private_value, throw_type_error, v8str,
    },
    webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const DESCRIPTION_TYPE_SLOT: &str = "__moliRtcSessionDescriptionType";
const DESCRIPTION_SDP_SLOT: &str = "__moliRtcSessionDescriptionSdp";
const DESCRIPTION_SLOTS: &[&str] = &[DESCRIPTION_TYPE_SLOT, DESCRIPTION_SDP_SLOT];

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCSessionDescription)]
struct SessionDescriptionObjectDeclaration<'scope> {
    #[webapi(slot = DESCRIPTION_TYPE_SLOT)]
    r#type: v8::Local<'scope, v8::String>,
    #[webapi(slot = DESCRIPTION_SDP_SLOT)]
    sdp: v8::Local<'scope, v8::String>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCSessionDescription, enumerable)]
struct SessionDescriptionPrototypeDeclaration {
    #[webapi(accessor_property, getter = description_attribute_getter, data = callback_data_index_value(scope, 0))]
    r#type: (),
    #[webapi(accessor_property, getter = description_attribute_getter, data = callback_data_index_value(scope, 1))]
    sdp: (),
    #[webapi(method = "toJSON", length = 0, callback = description_to_json_callback)]
    to_json: (),
}

pub(super) fn install_session_description_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    SessionDescriptionPrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

pub(in crate::context_bootstrap) fn rtc_session_description_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "RTCSessionDescription constructor requires 'new'.");
        return;
    }
    let dictionary = match webidl::dictionary_value(
        args.get(0),
        webidl::Context::argument("RTCSessionDescription", 1),
    ) {
        Ok(Some(dictionary)) => dictionary,
        Ok(None) => {
            throw_type_error(scope, "RTCSessionDescriptionInit requires type.");
            return;
        }
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    // WebIDL converts dictionary members in lexical order, including defaults.
    // SDP is a DOMString: retain the V8 string, including lone UTF-16 surrogates.
    // Its contents are not validated by this legacy data-model constructor.
    let Some(raw_sdp) = dictionary.get(scope, v8str(scope, "sdp").into()) else {
        return;
    };
    let sdp = if raw_sdp.is_undefined() {
        v8str(scope, "")
    } else {
        if raw_sdp.is_symbol() {
            throw_type_error(scope, "Cannot convert a Symbol to a DOMString.");
            return;
        }
        let Some(sdp) = raw_sdp.to_string(scope) else {
            return;
        };
        sdp
    };
    let Some(raw_type) = dictionary.get(scope, v8str(scope, "type").into()) else {
        return;
    };
    if raw_type.is_undefined() {
        throw_type_error(scope, "RTCSessionDescriptionInit requires type.");
        return;
    }
    if raw_type.is_symbol() {
        throw_type_error(scope, "Cannot convert a Symbol to an RTCSdpType.");
        return;
    }
    let Some(kind) = raw_type.to_string(scope) else {
        return;
    };
    if !matches!(
        kind.to_rust_string_lossy(scope).as_str(),
        "offer" | "pranswer" | "answer" | "rollback"
    ) {
        throw_type_error(scope, "Invalid RTCSdpType.");
        return;
    }
    if SessionDescriptionObjectDeclaration::new(kind, sdp)
        .initialize(scope, args.this())
        .is_err()
    {
        return;
    }
    apply_webidl_constructor_prototype_fallback(
        scope,
        args.this(),
        args.new_target(),
        "RTCSessionDescription",
    );
    rv.set(args.this().into());
}

fn description_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        DESCRIPTION_SLOTS,
        "RTCSessionDescription attribute",
    ) else {
        return;
    };
    if let Some(value) = get_private_value(scope, args.this(), slot) {
        rv.set(value);
    } else {
        throw_type_error(scope, "Illegal invocation");
    }
}

fn description_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(kind) = get_private_value(scope, args.this(), DESCRIPTION_TYPE_SLOT)
        .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
    else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    let Some(sdp) = get_private_value(scope, args.this(), DESCRIPTION_SDP_SLOT)
        .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
    else {
        return;
    };
    if let Ok(json) = RtcSessionDescriptionInitDeclaration::new(kind, sdp).bind(scope) {
        rv.set(json.into());
    }
}
