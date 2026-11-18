use crate::interfaces as web_api_interfaces;
use crate::text_codec::TextCodecStore;
use moli_v8_util::{
    callback_data_index_value, callback_data_item, get_private_value, throw_range_error,
    throw_type_error, v8_string,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use moli_webidl as webidl;
use webidl::WebIdlDictionarySerialize;

const TEXT_DECODER_ENCODING_SLOT: &str = "__lmTextDecoderEncoding";
const TEXT_DECODER_FATAL_SLOT: &str = "__lmTextDecoderFatal";
const TEXT_DECODER_IGNORE_BOM_SLOT: &str = "__lmTextDecoderIgnoreBOM";
const TEXT_DECODER_ENCODING_SLOT_INDEX: usize = 0;
const TEXT_DECODER_FATAL_SLOT_INDEX: usize = 1;
const TEXT_DECODER_IGNORE_BOM_SLOT_INDEX: usize = 2;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::TextDecoder)]
struct TextDecoderObjectDeclaration {
    #[webapi(slot = TEXT_DECODER_ENCODING_SLOT)]
    encoding: String,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TextDecoder, enumerable)]
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

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TextDecoder")]
struct TextDecoderConstructorArgs {
    #[webidl(default = "utf-8")]
    label: String,
    #[webidl(index = 1, dictionary)]
    options: TextDecoderOptions,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "TextDecoderOptions", snapshot)]
struct TextDecoderOptions {
    #[webidl(default = false)]
    #[webidl(snapshot_slot = TEXT_DECODER_FATAL_SLOT)]
    fatal: bool,
    #[webidl(name = "ignoreBOM", default = false)]
    #[webidl(snapshot_slot = TEXT_DECODER_IGNORE_BOM_SLOT)]
    ignore_bom: bool,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TextDecoder.decode")]
struct TextDecoderDecodeArgs<'s> {
    #[webidl(converter = "raw")]
    input: Option<webidl::AllowSharedBufferSource<'s>>,
    #[webidl(index = 1, dictionary)]
    options: TextDecodeOptions,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "TextDecodeOptions")]
struct TextDecodeOptions {
    #[webidl(default = false)]
    stream: bool,
}

pub fn install_text_codec_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "TextEncoder" => {
            crate::text_encoder::install(scope, template);
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
    rv.set_bool(
        get_private_value(scope, args.this(), slot)
            .is_some_and(|value| value.is_boolean() && value.boolean_value(scope)),
    );
}

const TEXT_CODEC_SLOTS: &[&str] = &[
    TEXT_DECODER_ENCODING_SLOT,
    TEXT_DECODER_FATAL_SLOT,
    TEXT_DECODER_IGNORE_BOM_SLOT,
];

pub fn text_decoder_constructor_callback<'s>(
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
    let label = parsed.label;
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
    parsed
        .options
        .initialize_snapshot(scope, args.this())
        .expect("TextDecoder options");
    TextDecoderObjectDeclaration::new(encoding_name)
        .initialize(scope, args.this())
        .expect("TextDecoder declaration should initialize object");
    rv.set(args.this().into());
}

fn text_decoder_decode_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<TextDecoderDecodeArgs<'s>>(scope, &args) else {
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
        .map(|input| input.to_vec(scope))
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

use crate::text_codec::TextDecodeError;

pub fn text_decoder_encoding_for_label(label: &str) -> Option<&'static encoding_rs::Encoding> {
    encoding_rs::Encoding::for_label_no_replacement(label.as_bytes())
}

pub fn canonical_text_decoder_encoding_name(encoding: &'static encoding_rs::Encoding) -> String {
    encoding.name().to_ascii_lowercase()
}

pub fn init_text_decoder_state(
    scope: &mut v8::PinScope<'_, '_>,
    decoder: v8::Local<'_, v8::Object>,
    encoding: &'static encoding_rs::Encoding,
    fatal: bool,
    ignore_bom: bool,
) {
    if let Some(codecs) = TextCodecStore::from_scope(scope) {
        let _ = codecs.init_decoder(scope, decoder, encoding, fatal, ignore_bom);
    }
}

pub fn decode_text_decoder_state(
    scope: &mut v8::PinScope<'_, '_>,
    decoder_id: u32,
    bytes: &[u8],
    stream: bool,
) -> Option<Result<String, TextDecodeError>> {
    let codecs = TextCodecStore::from_scope(scope)?;
    Some(codecs.decode(decoder_id, bytes, stream))
}
