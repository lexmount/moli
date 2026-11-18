//! Native media error values and their legacy constants.
use crate::interfaces as web_api_interfaces;
use moli_v8_util::{get_private_value, set_private_value, throw_type_error, v8_string};
use moli_webapi_declare::WebApiFunctionTemplate;

const MEDIA_ERROR_CODE_SLOT: &str = "__moliMediaErrorCode";
const MEDIA_ERROR_MESSAGE_SLOT: &str = "__moliMediaErrorMessage";

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaError, enumerable)]
struct MediaErrorConstantsDeclaration {
    #[webapi(constant = "MEDIA_ERR_ABORTED", value = 1u32)]
    media_err_aborted: (),
    #[webapi(constant = "MEDIA_ERR_NETWORK", value = 2u32)]
    media_err_network: (),
    #[webapi(constant = "MEDIA_ERR_DECODE", value = 3u32)]
    media_err_decode: (),
    #[webapi(constant = "MEDIA_ERR_SRC_NOT_SUPPORTED", value = 4u32)]
    media_err_src_not_supported: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaError, enumerable)]
struct MediaErrorPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = media_error_code_getter_callback,
        enumerable
    )]
    code: (),
    #[webapi(
        accessor_property,
        getter = media_error_message_getter_callback,
        enumerable
    )]
    message: (),
}

fn media_error_code_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(code) = get_private_value(scope, args.this(), MEDIA_ERROR_CODE_SLOT) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(code);
}

fn media_error_message_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(message) = get_private_value(scope, args.this(), MEDIA_ERROR_MESSAGE_SLOT) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(message);
}

pub fn new_media_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    code: u32,
    message: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = moli_v8_util::global_constructor_prototype(scope, "MediaError")?;
    let object = v8::Object::new(scope);
    if object.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    web_api_interfaces::MediaError::DESCRIPTOR
        .initialize(scope, object)
        .ok()?;
    set_private_value(
        scope,
        object,
        MEDIA_ERROR_CODE_SLOT,
        v8::Integer::new_from_unsigned(scope, code).into(),
    );
    set_private_value(
        scope,
        object,
        MEDIA_ERROR_MESSAGE_SLOT,
        v8_string(scope, message)?.into(),
    );
    Some(object)
}

pub fn media_error_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    throw_type_error(
        scope,
        "Failed to construct 'MediaError': Illegal constructor.",
    );
}

pub fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    MediaErrorConstantsDeclaration::initialize_template(scope, template);
    let prototype = template.prototype_template(scope);
    MediaErrorConstantsDeclaration::initialize_prototype_template(scope, prototype);
    MediaErrorPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}
