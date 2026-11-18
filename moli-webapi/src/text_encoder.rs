use crate::interfaces as web_api_interfaces;
use moli_v8_util::{get_private_value, new_uint8_array_from_bytes, throw_type_error};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use moli_webidl as webidl;
const TEXT_ENCODER_ENCODING_SLOT: &str = "__lmTextEncoderEncoding";
#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::TextEncoder)]
struct TextEncoderObjectDeclaration {
    #[webapi(slot = TEXT_ENCODER_ENCODING_SLOT)]
    encoding: &'static str,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TextEncoder, enumerable)]
struct TextEncoderPrototypeDeclaration {
    #[webapi(method, length = 0, callback = text_encoder_encode_callback)]
    encode: (),
    #[webapi(method, length = 2, callback = text_encoder_encode_into_callback)]
    encode_into: (),
    #[webapi(
        accessor_property,
        getter = encoding_getter,
                enumerable
    )]
    encoding: (),
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct TextEncoderEncodeIntoResultDeclaration {
    read: usize,
    written: usize,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TextEncoder.encode")]
struct TextEncoderEncodeArgs {
    #[webidl(default = "")]
    input: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TextEncoder.encodeInto")]
struct TextEncoderEncodeIntoArgs<'s> {
    #[webidl(default = "")]
    input: String,
    #[webidl(
        required,
        allow_shared,
        missing_message = "Failed to execute 'encodeInto' on 'TextEncoder': parameter 2 is not of type 'Uint8Array'."
    )]
    dest: v8::Local<'s, v8::Uint8Array>,
}

pub fn text_encoder_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "TextEncoder constructor must be called with new");
        return;
    }
    TextEncoderObjectDeclaration::new("utf-8")
        .initialize(scope, args.this())
        .expect("TextEncoder declaration should initialize object");
    rv.set(args.this().into());
}
fn text_encoder_encode_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<TextEncoderEncodeArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    let encoded = parsed.input.into_bytes();
    if let Some(array) = new_uint8_array_from_bytes(scope, encoded) {
        rv.set(array.into());
    } else {
        rv.set_undefined();
    }
}
fn text_encoder_encode_into_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<TextEncoderEncodeIntoArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    let dest_len = parsed.dest.byte_length();
    let encoded = parsed.input.as_bytes();
    let mut read_chars: usize = 0;
    let mut written: usize = 0;
    for ch in parsed.input.chars() {
        let ch_len = ch.len_utf8();
        if written + ch_len > dest_len {
            break;
        }
        written += ch_len;
        read_chars += ch.len_utf16();
    }

    if written > 0 {
        let bytes = &encoded[..written];
        let destination = webidl::AllowSharedBufferSource::View(parsed.dest.into());
        assert!(
            destination.write_bytes(scope, bytes),
            "encoded bytes fit the converted view"
        );
    }

    let result = TextEncoderEncodeIntoResultDeclaration {
        read: read_chars,
        written,
    }
    .bind(scope)
    .expect("TextEncoder encodeInto result declaration should bind");
    rv.set(result.into());
}

fn encoding_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = get_private_value(scope, args.this(), TEXT_ENCODER_ENCODING_SLOT)
        .and_then(|value| value.to_string(scope));
    if let Some(value) = value {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}
pub fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    TextEncoderPrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}
