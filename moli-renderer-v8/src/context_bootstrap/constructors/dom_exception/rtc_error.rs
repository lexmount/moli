use super::*;
use moli_webapi_declare::initialize_web_api_constructor_receiver;
use webidl::WebIdlDictionary;

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

impl ErrorDetail {
    fn token(self) -> &'static str {
        match self {
            Self::DataChannelFailure => "data-channel-failure",
            Self::DtlsFailure => "dtls-failure",
            Self::FingerprintFailure => "fingerprint-failure",
            Self::SctpFailure => "sctp-failure",
            Self::SdpSyntaxError => "sdp-syntax-error",
            Self::HardwareEncoderNotAvailable => "hardware-encoder-not-available",
            Self::HardwareEncoderError => "hardware-encoder-error",
        }
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCErrorInit")]
struct ErrorInit {
    // Dictionary members are visited lexically, before the message argument.
    #[webidl(required, converter = "enum")]
    error_detail: ErrorDetail,
    received_alert: Option<u32>,
    sctp_cause_code: Option<i32>,
    sdp_line_number: Option<i32>,
    sent_alert: Option<u32>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCError")]
struct ConstructorArgs {
    #[webidl(required, with = parse_init_arg)]
    init: ErrorInit,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    message: webidl::DomString16,
}

fn parse_init_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<ErrorInit, webidl::WebIdlError> {
    let object = webidl::dictionary_value(
        args.get(index),
        webidl::Context::argument("RTCError", (index + 1) as usize),
    )?
    .ok_or_else(|| {
        webidl::WebIdlError::missing_required(webidl::Context::member(
            "RTCErrorInit",
            "errorDetail",
        ))
    })?;
    ErrorInit::parse_dictionary(scope, object)
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
    #[webapi(slot = ERROR_DETAIL)]
    error_detail: v8::Local<'s, v8::String>,
    #[webapi(slot = RECEIVED_ALERT)]
    received_alert: v8::Local<'s, v8::Value>,
    #[webapi(slot = SCTP_CAUSE_CODE)]
    sctp_cause_code: v8::Local<'s, v8::Value>,
    #[webapi(slot = SDP_LINE_NUMBER)]
    sdp_line_number: v8::Local<'s, v8::Value>,
    #[webapi(slot = SENT_ALERT)]
    sent_alert: v8::Local<'s, v8::Value>,
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
    if !initialize_web_api_constructor_receiver(scope, args.this(), "RTCError") {
        return;
    }
    let Some(message) =
        v8::String::new_from_two_byte(scope, &parsed.message.0, v8::NewStringType::Normal)
    else {
        return;
    };
    let declaration = ObjectDeclaration::new(
        message,
        v8str(scope, parsed.init.error_detail.token()),
        nullable_double_slot_value(scope, parsed.init.received_alert.map(f64::from)),
        nullable_double_slot_value(scope, parsed.init.sctp_cause_code.map(f64::from)),
        nullable_double_slot_value(scope, parsed.init.sdp_line_number.map(f64::from)),
        nullable_double_slot_value(scope, parsed.init.sent_alert.map(f64::from)),
    );
    if declaration.initialize(scope, args.this()).is_ok() {
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
    if let Some(value) = get_private_value(scope, args.this(), &slot) {
        rv.set(value);
    }
}
