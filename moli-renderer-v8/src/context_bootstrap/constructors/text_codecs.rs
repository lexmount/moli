use super::*;
use crate::{
    text_codec::TextDecodeError,
    util::{callback_data_index_value, callback_data_item, get_private_value},
    webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const TEXT_ENCODER_ENCODING_SLOT: &str = "__lmTextEncoderEncoding";
const TEXT_DECODER_ENCODING_SLOT: &str = "__lmTextDecoderEncoding";
const TEXT_DECODER_FATAL_SLOT: &str = "__lmTextDecoderFatal";
const TEXT_DECODER_IGNORE_BOM_SLOT: &str = "__lmTextDecoderIgnoreBOM";
const TEXT_ENCODER_ENCODING_SLOT_INDEX: usize = 0;
const TEXT_DECODER_ENCODING_SLOT_INDEX: usize = 1;
const TEXT_DECODER_FATAL_SLOT_INDEX: usize = 2;
const TEXT_DECODER_IGNORE_BOM_SLOT_INDEX: usize = 3;

#[derive(WebApiObject)]
#[webapi(interface = "TextEncoder")]
struct TextEncoderObjectDeclaration {
    #[webapi(slot = TEXT_ENCODER_ENCODING_SLOT)]
    encoding: &'static str,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "TextEncoder", enumerable)]
struct TextEncoderPrototypeDeclaration {
    #[webapi(method, length = 0, callback = text_encoder_encode_callback)]
    encode: (),
    #[webapi(method, length = 2, callback = text_encoder_encode_into_callback)]
    encode_into: (),
    #[webapi(
        accessor_property,
        getter = text_codec_string_attribute_getter_callback,
        data = callback_data_index_value(scope, TEXT_ENCODER_ENCODING_SLOT_INDEX),
        enumerable
    )]
    encoding: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "TextDecoder")]
struct TextDecoderObjectDeclaration {
    #[webapi(slot = TEXT_DECODER_ENCODING_SLOT)]
    encoding: String,
    #[webapi(slot = TEXT_DECODER_FATAL_SLOT)]
    fatal: bool,
    #[webapi(slot = TEXT_DECODER_IGNORE_BOM_SLOT)]
    ignore_bom: bool,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "TextDecoder", enumerable)]
struct TextDecoderPrototypeDeclaration {
    #[webapi(method, length = 0, callback = text_decoder_decode_callback)]
    decode: (),
    #[webapi(
        accessor_property,
        getter = text_codec_string_attribute_getter_callback,
        data = callback_data_index_value(scope, TEXT_DECODER_ENCODING_SLOT_INDEX),
        enumerable
    )]
    encoding: (),
    #[webapi(
        accessor_property,
        getter = text_codec_bool_attribute_getter_callback,
        data = callback_data_index_value(scope, TEXT_DECODER_FATAL_SLOT_INDEX),
        enumerable
    )]
    fatal: (),
    #[webapi(
        accessor_property = "ignoreBOM",
        getter = text_codec_bool_attribute_getter_callback,
        data = callback_data_index_value(scope, TEXT_DECODER_IGNORE_BOM_SLOT_INDEX),
        enumerable
    )]
    ignore_bom: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
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
        missing_message = "Failed to execute 'encodeInto' on 'TextEncoder': parameter 2 is not of type 'Uint8Array'."
    )]
    dest: v8::Local<'s, v8::Uint8Array>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TextDecoder")]
struct TextDecoderConstructorArgs {
    #[webidl(default = "utf-8")]
    label: String,
    #[webidl(index = 1, with = parse_text_decoder_options_arg)]
    options: TextDecoderOptions,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "TextDecoderOptions")]
struct TextDecoderOptions {
    #[webidl(default = false)]
    fatal: bool,
    #[webidl(name = "ignoreBOM", default = false)]
    ignore_bom: bool,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TextDecoder.decode")]
struct TextDecoderDecodeArgs {
    #[webidl(converter = "buffer_source")]
    input: Option<webidl::BufferSource>,
    #[webidl(index = 1, with = parse_text_decode_options_arg)]
    options: TextDecodeOptions,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "TextDecodeOptions")]
struct TextDecodeOptions {
    #[webidl(default = false)]
    stream: bool,
}

pub(in crate::context_bootstrap) fn install_text_codec_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "TextEncoder" => {
            TextEncoderPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "TextDecoder" => {
            TextDecoderPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => {}
    }
}

fn text_codec_string_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(scope, &args, TEXT_CODEC_SLOTS, "Text codec slots") else {
        rv.set_empty_string();
        return;
    };
    if !text_codec_receiver_branded_for_slot(scope, args.this(), slot) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let value = get_private_value(scope, args.this(), slot)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

fn text_codec_bool_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(scope, &args, TEXT_CODEC_SLOTS, "Text codec slots") else {
        rv.set_bool(false);
        return;
    };
    if !text_codec_receiver_branded_for_slot(scope, args.this(), slot) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set_bool(
        get_private_value(scope, args.this(), slot)
            .is_some_and(|value| value.is_boolean() && value.boolean_value(scope)),
    );
}

const TEXT_CODEC_SLOTS: &[&str] = &[
    TEXT_ENCODER_ENCODING_SLOT,
    TEXT_DECODER_ENCODING_SLOT,
    TEXT_DECODER_FATAL_SLOT,
    TEXT_DECODER_IGNORE_BOM_SLOT,
];

pub(in crate::context_bootstrap) fn text_encoder_constructor_callback<'s>(
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
    if !text_encoder_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
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
    if !text_encoder_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(parsed) = webidl::parse_args::<TextEncoderEncodeIntoArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    if parsed
        .dest
        .get_backing_store()
        .is_some_and(|backing_store| backing_store.is_resizable_by_user_javascript())
    {
        throw_type_error(
            scope,
            "TextEncoder.encodeInto does not accept a resizable or growable destination buffer.",
        );
        return;
    }

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
        read_chars += 1;
    }

    if written > 0 {
        let bytes = &encoded[..written];
        let mut buf = vec![0u8; parsed.dest.byte_length()];
        parsed.dest.copy_contents(&mut buf);
        buf[..written].copy_from_slice(bytes);
        if let Some(backing_store) = parsed.dest.buffer(scope) {
            let byte_offset = parsed.dest.byte_offset();
            let data = backing_store.data();
            if let Some(ptr) = data {
                let ptr = ptr.as_ptr() as *mut u8;
                unsafe {
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.add(byte_offset), written);
                }
            }
        }
    }

    let result = TextEncoderEncodeIntoResultDeclaration {
        read: read_chars,
        written,
    }
    .bind(scope)
    .expect("TextEncoder encodeInto result declaration should bind");
    rv.set(result.into());
}

pub(in crate::context_bootstrap) fn text_decoder_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "TextDecoder constructor must be called with new");
        return;
    }

    let Some(parsed) = webidl::parse_args::<TextDecoderConstructorArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    let label = parsed.label.trim().to_ascii_lowercase();
    let Some(encoding) = text_decoder_encoding_for_label(&label) else {
        throw_range_error(
            scope,
            &format!("The encoding label provided ('{label}') is invalid."),
        );
        return;
    };
    let encoding_name = canonical_text_decoder_encoding_name(encoding);

    let fatal = parsed.options.fatal;
    let ignore_bom = parsed.options.ignore_bom;
    init_text_decoder_state(scope, args.this(), encoding, fatal, ignore_bom);
    TextDecoderObjectDeclaration::new(encoding_name, fatal, ignore_bom)
        .initialize(scope, args.this())
        .expect("TextDecoder declaration should initialize object");
    rv.set(args.this().into());
}

fn parse_text_decoder_options_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<TextDecoderOptions, webidl::WebIdlError> {
    let context = webidl::Context::argument("TextDecoder", (index + 1) as usize);
    webidl::dictionary_arg(args, index, context)?
        .map(|object| webidl::parse_dictionary_object(scope, object))
        .transpose()
        .map(|options| options.unwrap_or_default())
}

fn parse_text_decode_options_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<TextDecodeOptions, webidl::WebIdlError> {
    let context = webidl::Context::argument("TextDecoder.decode", (index + 1) as usize);
    webidl::dictionary_arg(args, index, context)?
        .map(|object| webidl::parse_dictionary_object(scope, object))
        .transpose()
        .map(|options| options.unwrap_or_default())
}

fn text_decoder_decode_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !text_decoder_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(parsed) = webidl::parse_args::<TextDecoderDecodeArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };

    let Some(decoder_id) = TextCodecStore::decoder_id_from_object(scope, args.this()) else {
        rv.set(v8::String::empty(scope).into());
        return;
    };
    let stream = parsed.options.stream;
    let bytes = parsed
        .input
        .map(webidl::BufferSource::into_bytes)
        .unwrap_or_default();

    let Some(decode_result) = decode_text_decoder_state(scope, decoder_id, &bytes, stream) else {
        rv.set(v8::String::empty(scope).into());
        return;
    };
    match decode_result {
        Ok(text) => {
            if let Some(text) = v8_string(scope, &text) {
                rv.set(text.into());
            } else {
                rv.set(v8::String::empty(scope).into());
            }
        }
        Err(error) => {
            throw_type_error(scope, error.message());
        }
    }
}

fn text_codec_receiver_branded_for_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    slot: &str,
) -> bool {
    match slot {
        TEXT_ENCODER_ENCODING_SLOT => text_encoder_receiver_branded(scope, receiver),
        TEXT_DECODER_ENCODING_SLOT | TEXT_DECODER_FATAL_SLOT | TEXT_DECODER_IGNORE_BOM_SLOT => {
            text_decoder_receiver_branded(scope, receiver)
        }
        _ => false,
    }
}

fn text_encoder_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    moli_webapi_declare::implements_interface(scope, receiver, "TextEncoder")
}

fn text_decoder_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    moli_webapi_declare::implements_interface(scope, receiver, "TextDecoder")
}

fn init_text_decoder_state(
    scope: &mut v8::PinScope<'_, '_>,
    decoder: v8::Local<'_, v8::Object>,
    encoding: &'static encoding_rs::Encoding,
    fatal: bool,
    ignore_bom: bool,
) {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host_ptr }
            .text_codecs_mut()
            .init_decoder(scope, decoder, encoding, fatal, ignore_bom);
        return;
    }
    if let Some(worker_state) = crate::worker::get_worker_state(scope) {
        worker_state
            .borrow_mut()
            .text_codecs
            .init_decoder(scope, decoder, encoding, fatal, ignore_bom);
    }
}

fn decode_text_decoder_state(
    scope: &mut v8::PinScope<'_, '_>,
    decoder_id: u32,
    bytes: &[u8],
    stream: bool,
) -> Option<Result<String, TextDecodeError>> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        return Some(
            unsafe { &mut *host_ptr }
                .text_codecs_mut()
                .decode(decoder_id, bytes, stream),
        );
    }
    crate::worker::get_worker_state(scope).map(|worker_state| {
        worker_state
            .borrow_mut()
            .text_codecs
            .decode(decoder_id, bytes, stream)
    })
}
