use super::*;
use webidl::WebIdlDictionarySerialize;

const ERROR_DETAIL: &str = "__moliRtcErrorDetail";
const SDP_LINE_NUMBER: &str = "__moliRtcErrorSdpLineNumber";
const SCTP_CAUSE_CODE: &str = "__moliRtcErrorSctpCauseCode";
const RECEIVED_ALERT: &str = "__moliRtcErrorReceivedAlert";
const SENT_ALERT: &str = "__moliRtcErrorSentAlert";

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "RTCErrorDetailType", rename_all = "kebab-case")]
enum ErrorDetail {
    DataChannelFailure,
    DtlsFailure,
    FingerprintFailure,
    SctpFailure,
    SdpSyntaxError,
    HardwareEncoderNotAvailable,
    HardwareEncoderError,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCErrorInit", snapshot)]
struct ErrorInit {
    // Dictionary members are visited lexically, before the message argument.
    #[webidl(required, converter = "enum")]
    #[webidl(snapshot_slot = ERROR_DETAIL)]
    error_detail: ErrorDetail,
    #[webidl(snapshot_slot = RECEIVED_ALERT, snapshot_default = v8::null(scope).into())]
    received_alert: Option<u32>,
    #[webidl(snapshot_slot = SCTP_CAUSE_CODE, snapshot_default = v8::null(scope).into())]
    sctp_cause_code: Option<i32>,
    #[webidl(snapshot_slot = SDP_LINE_NUMBER, snapshot_default = v8::null(scope).into())]
    sdp_line_number: Option<i32>,
    #[webidl(snapshot_slot = SENT_ALERT, snapshot_default = v8::null(scope).into())]
    sent_alert: Option<u32>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCError")]
struct ConstructorArgs {
    #[webidl(required, dictionary)]
    init: ErrorInit,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    message: webidl::DomString16,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCError)]
struct ObjectDeclaration<'s> {
    #[webapi(slot = DOM_EXCEPTION_MESSAGE_SLOT)]
    message: v8::Local<'s, v8::String>,
    #[webapi(slot = DOM_EXCEPTION_NAME_SLOT, init = string("OperationError"))]
    name: (),
    #[webapi(slot = DOM_EXCEPTION_CODE_SLOT, init = 0)]
    code: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCError, receiver, enumerable)]
struct PrototypeDeclaration {
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, ERROR_DETAIL))]
    error_detail: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, SDP_LINE_NUMBER))]
    sdp_line_number: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, SCTP_CAUSE_CODE))]
    sctp_cause_code: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, RECEIVED_ALERT))]
    received_alert: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, SENT_ALERT))]
    sent_alert: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    PrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

pub(crate) fn rtc_error_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "RTCError constructor requires 'new'.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<ConstructorArgs>(scope, &args) else {
        return;
    };
    let Some(message) =
        v8::String::new_from_two_byte(scope, &parsed.message.0, v8::NewStringType::Normal)
    else {
        return;
    };
    let declaration = ObjectDeclaration::new(message);
    if parsed
        .init
        .initialize_snapshot(scope, args.this())
        .is_some()
        && declaration.initialize(scope, args.this()).is_ok()
    {
        rv.set(args.this().into());
    }
}

fn slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let slot = v8::Local::<v8::String>::try_from(args.data())
        .expect("RTCError attribute slot")
        .to_rust_string_lossy(scope);
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("RTCError receiver was validated");
    if let Some(value) = get_private_value(scope, receiver, &slot) {
        rv.set(value);
    }
}

pub(crate) fn build_rtc_sdp_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
    line_number: i32,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = build_dom_exception_instance(scope, "RTCError")?;
    ErrorInit {
        error_detail: ErrorDetail::SdpSyntaxError,
        received_alert: None,
        sctp_cause_code: None,
        sdp_line_number: Some(line_number),
        sent_alert: None,
    }
    .initialize_snapshot(scope, object)?;
    ObjectDeclaration::new(v8_string(scope, message)?)
        .initialize(scope, object)
        .ok()?;
    capture_dom_exception_stack(scope, object);
    Some(object)
}
