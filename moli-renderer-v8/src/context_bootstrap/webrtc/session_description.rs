use super::{RtcSessionDescriptionInitDeclaration, dictionary_string::DictionaryDomString};
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

#[derive(Clone, Copy, PartialEq, Eq, webidl::WebIdlEnum)]
#[webidl(name = "RTCSdpType", rename_all = "kebab-case")]
pub(super) enum SdpType {
    Offer,
    Pranswer,
    Answer,
    Rollback,
}

// The deprecated data-model constructor allows an omitted type. PeerConnection
// description algorithms retain their own state and SDP constraints.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCSessionDescriptionInit")]
pub(super) struct SessionDescriptionInit<'s> {
    #[webidl(converter = "raw", default = DictionaryDomString::default())]
    pub(super) sdp: DictionaryDomString<'s>,
    #[webidl(converter = "enum")]
    pub(super) r#type: Option<SdpType>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCSessionDescription")]
struct SessionDescriptionArgs<'s> {
    #[webidl(dictionary)]
    init: SessionDescriptionInit<'s>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCSessionDescription)]
struct SessionDescriptionObjectDeclaration<'scope> {
    #[webapi(slot = DESCRIPTION_TYPE_SLOT)]
    r#type: v8::Local<'scope, v8::Value>,
    #[webapi(slot = DESCRIPTION_SDP_SLOT)]
    sdp: v8::Local<'scope, v8::String>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCSessionDescription, enumerable, receiver)]
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

pub(super) fn from_parts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: v8::Local<'s, v8::String>,
    sdp: v8::Local<'s, v8::String>,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = super::super::exposed_interfaces::build_intrinsic_interface_instance(
        scope,
        "RTCSessionDescription",
    )
    .ok()?;
    let prototype = super::super::exposed_interfaces::ensure_intrinsic_interface_prototype(
        scope,
        "RTCSessionDescription",
    )
    .ok()?;
    if object.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    SessionDescriptionObjectDeclaration::new(kind, sdp)
        .initialize(scope, object)
        .ok()?;
    Some(object)
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
    let Some(args_init) = webidl::parse_args::<SessionDescriptionArgs>(scope, &args) else {
        return;
    };
    let sdp = args_init.init.sdp.into_value(scope);
    let kind: v8::Local<'s, v8::Value> = args_init
        .init
        .r#type
        .map(|value| v8str(scope, webidl::WebIdlEnumSnapshot::snapshot_token(&value)).into())
        .unwrap_or_else(|| v8::null(scope).into());
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

pub(super) fn native_description<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: SdpType,
    sdp: v8::Local<'s, v8::String>,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = super::super::exposed_interfaces::build_intrinsic_interface_instance(
        scope,
        "RTCSessionDescription",
    )
    .ok()?;
    let prototype = super::super::exposed_interfaces::ensure_intrinsic_interface_prototype(
        scope,
        "RTCSessionDescription",
    )
    .ok()?;
    object.set_prototype(scope, prototype.into())?;
    let kind = v8str(scope, webidl::WebIdlEnumSnapshot::snapshot_token(&kind));
    SessionDescriptionObjectDeclaration::new(kind.into(), sdp)
        .initialize(scope, object)
        .ok()?;
    Some(object)
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
    let Some(kind) = get_private_value(scope, args.this(), DESCRIPTION_TYPE_SLOT) else {
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
