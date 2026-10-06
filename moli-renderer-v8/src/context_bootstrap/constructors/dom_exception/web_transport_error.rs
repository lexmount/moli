use super::*;
use moli_webapi_declare::initialize_web_api_constructor_receiver;
use webidl::WebIdlEnum;

const SOURCE_SLOT: &str = "__moliWebTransportErrorSource";
const STREAM_ERROR_CODE_SLOT: &str = "__moliWebTransportErrorStreamErrorCode";

#[derive(Clone, Copy, Debug, webidl::WebIdlEnum)]
#[webidl(name = "WebTransportErrorSource")]
pub(crate) enum WebTransportErrorSource {
    Stream,
    Session,
}

impl WebTransportErrorSource {
    fn token(self) -> &'static str {
        match self {
            Self::Stream => "stream",
            Self::Session => "session",
        }
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "WebTransportErrorOptions")]
struct Options {
    #[webidl(converter = "enum", default = WebTransportErrorSource::Stream)]
    source: WebTransportErrorSource,
    #[webidl(nullable, converter = "clamped_unsigned_long")]
    stream_error_code: Option<u32>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebTransportError")]
struct ConstructorArgs {
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    message: webidl::DomString16,
    #[webidl(dictionary)]
    options: Options,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::WebTransportError)]
struct ObjectDeclaration<'s> {
    #[webapi(slot = DOM_EXCEPTION_MESSAGE_SLOT)]
    message: v8::Local<'s, v8::String>,
    #[webapi(slot = DOM_EXCEPTION_NAME_SLOT, init = string("WebTransportError"))]
    name: (),
    #[webapi(slot = DOM_EXCEPTION_CODE_SLOT, init = 0)]
    code: (),
    #[webapi(slot = SOURCE_SLOT)]
    source: v8::Local<'s, v8::String>,
    #[webapi(slot = STREAM_ERROR_CODE_SLOT)]
    stream_error_code: v8::Local<'s, v8::Value>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WebTransportError, receiver, enumerable)]
struct PrototypeDeclaration {
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, SOURCE_SLOT))]
    source: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, STREAM_ERROR_CODE_SLOT))]
    stream_error_code: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    PrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

pub(crate) fn web_transport_error_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "WebTransportError constructor requires 'new'.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<ConstructorArgs>(scope, &args) else {
        return;
    };
    if !initialize_web_api_constructor_receiver(scope, args.this(), "WebTransportError") {
        return;
    }
    let payload = WebTransportErrorClonePayload {
        message: parsed.message.0,
        source: parsed.options.source,
        stream_error_code: parsed.options.stream_error_code,
    };
    let Some(declaration) = declaration(scope, &payload) else {
        return;
    };
    if declaration.initialize(scope, args.this()).is_ok() {
        capture_dom_exception_stack(scope, args.this());
        rv.set(args.this().into());
    }
}

fn slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let slot = args.data().to_rust_string_lossy(scope);
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("WebTransportError receiver was validated");
    if let Some(value) = get_private_value(scope, receiver, &slot) {
        rv.set(value);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct WebTransportErrorClonePayload {
    pub(crate) message: Vec<u16>,
    pub(crate) source: WebTransportErrorSource,
    pub(crate) stream_error_code: Option<u32>,
}

pub(crate) fn web_transport_error_clone_payload_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<WebTransportErrorClonePayload> {
    web_api_interfaces::WebTransportError::is_instance(scope, object).then_some(())?;
    let object = moli_webapi_declare::web_api_object_target(scope, object)?;
    let message = get_private_value(scope, object, DOM_EXCEPTION_MESSAGE_SLOT)?;
    let message = webidl::convert::<webidl::DomString16>(
        scope,
        message,
        webidl::Context::member("WebTransportError", "message"),
    )
    .ok()?
    .0;
    let source = get_private_value(scope, object, SOURCE_SLOT)?.to_rust_string_lossy(scope);
    let source = WebTransportErrorSource::parse_token(&source)?;
    let code = get_private_value(scope, object, STREAM_ERROR_CODE_SLOT)?;
    let stream_error_code = if code.is_null() {
        None
    } else {
        Some(v8::Local::<v8::Number>::try_from(code).ok()?.value() as u32)
    };
    Some(WebTransportErrorClonePayload {
        message,
        source,
        stream_error_code,
    })
}

fn declaration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: &WebTransportErrorClonePayload,
) -> Option<ObjectDeclaration<'s>> {
    let message =
        v8::String::new_from_two_byte(scope, &payload.message, v8::NewStringType::Normal)?;
    let source = v8str(scope, payload.source.token());
    let code = nullable_double_slot_value(scope, payload.stream_error_code.map(f64::from));
    Some(ObjectDeclaration::new(message, source, code))
}

pub(crate) fn build_web_transport_error_from_clone_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: &WebTransportErrorClonePayload,
) -> Option<v8::Local<'s, v8::Object>> {
    if !crate::context_bootstrap::exposed_interfaces::is_realm_interface_exposed(
        scope,
        "WebTransportError",
    ) {
        throw_dom_exception_value(
            scope,
            "WebTransportError is not exposed in this realm.",
            "DataCloneError",
        );
        return None;
    }
    crate::context_bootstrap::exposed_interfaces::ensure_intrinsic_interface_prototype(
        scope,
        "WebTransportError",
    )
    .ok()?;
    let object = declaration(scope, payload)?.bind(scope).ok()?;
    capture_dom_exception_stack(scope, object);
    Some(object)
}
