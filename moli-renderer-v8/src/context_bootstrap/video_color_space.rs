//! Immutable color metadata, independent of video decoding and rendering.

use crate::{
    util::{get_private_value, throw_type_error, v8str},
    web_api_interfaces, webidl,
};
use moli_webapi_declare::{
    WebApiFunctionTemplate, WebApiObject, initialize_web_api_constructor_receiver,
};

const PRIMARIES_SLOT: &str = "__moliVideoColorSpacePrimaries";
const TRANSFER_SLOT: &str = "__moliVideoColorSpaceTransfer";
const MATRIX_SLOT: &str = "__moliVideoColorSpaceMatrix";
const FULL_RANGE_SLOT: &str = "__moliVideoColorSpaceFullRange";

#[derive(Clone, Copy, webidl::WebIdlEnum)]
enum VideoColorPrimaries {
    #[webidl(token = "bt709")]
    Bt709,
    #[webidl(token = "bt470bg")]
    Bt470Bg,
    #[webidl(token = "smpte170m")]
    Smpte170M,
    #[webidl(token = "bt2020")]
    Bt2020,
    #[webidl(token = "smpte432")]
    Smpte432,
}

impl VideoColorPrimaries {
    fn label(self) -> &'static str {
        match self {
            Self::Bt709 => "bt709",
            Self::Bt470Bg => "bt470bg",
            Self::Smpte170M => "smpte170m",
            Self::Bt2020 => "bt2020",
            Self::Smpte432 => "smpte432",
        }
    }
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
enum VideoTransferCharacteristics {
    #[webidl(token = "bt709")]
    Bt709,
    #[webidl(token = "smpte170m")]
    Smpte170M,
    #[webidl(token = "iec61966-2-1")]
    Iec61966_2_1,
    Linear,
    Pq,
    Hlg,
}

impl VideoTransferCharacteristics {
    fn label(self) -> &'static str {
        match self {
            Self::Bt709 => "bt709",
            Self::Smpte170M => "smpte170m",
            Self::Iec61966_2_1 => "iec61966-2-1",
            Self::Linear => "linear",
            Self::Pq => "pq",
            Self::Hlg => "hlg",
        }
    }
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
enum VideoMatrixCoefficients {
    Rgb,
    #[webidl(token = "bt709")]
    Bt709,
    #[webidl(token = "bt470bg")]
    Bt470Bg,
    #[webidl(token = "smpte170m")]
    Smpte170M,
    #[webidl(token = "bt2020-ncl")]
    Bt2020Ncl,
}

impl VideoMatrixCoefficients {
    fn label(self) -> &'static str {
        match self {
            Self::Rgb => "rgb",
            Self::Bt709 => "bt709",
            Self::Bt470Bg => "bt470bg",
            Self::Smpte170M => "smpte170m",
            Self::Bt2020Ncl => "bt2020-ncl",
        }
    }
}

// WebIDL reads dictionary members in lexicographic order.
#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "VideoColorSpaceInit")]
struct VideoColorSpaceInit {
    #[webidl(nullable, converter = "boolean")]
    full_range: Option<bool>,
    #[webidl(nullable, converter = "enum")]
    matrix: Option<VideoMatrixCoefficients>,
    #[webidl(nullable, converter = "enum")]
    primaries: Option<VideoColorPrimaries>,
    #[webidl(nullable, converter = "enum")]
    transfer: Option<VideoTransferCharacteristics>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "VideoColorSpace")]
struct VideoColorSpaceConstructorArgs {
    #[webidl(with = parse_init)]
    init: VideoColorSpaceInit,
}

fn parse_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<VideoColorSpaceInit, webidl::WebIdlError> {
    let context = webidl::Context::argument("VideoColorSpace", (index + 1) as usize);
    let Some(object) = webidl::dictionary_arg(args, index, context)? else {
        // A nullish dictionary is empty; do not consult Object.prototype.
        return Ok(VideoColorSpaceInit::default());
    };
    webidl::parse_dictionary_object(scope, object)
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::VideoColorSpace, require_prototype)]
struct VideoColorSpaceObjectDeclaration<'s> {
    #[webapi(slot = PRIMARIES_SLOT)]
    primaries: v8::Local<'s, v8::Value>,
    #[webapi(slot = TRANSFER_SLOT)]
    transfer: v8::Local<'s, v8::Value>,
    #[webapi(slot = MATRIX_SLOT)]
    matrix: v8::Local<'s, v8::Value>,
    #[webapi(slot = FULL_RANGE_SLOT)]
    full_range: v8::Local<'s, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct VideoColorSpaceJsonDeclaration<'s> {
    #[webapi(data_property, enumerable)]
    full_range: v8::Local<'s, v8::Value>,
    #[webapi(data_property, enumerable)]
    matrix: v8::Local<'s, v8::Value>,
    #[webapi(data_property, enumerable)]
    primaries: v8::Local<'s, v8::Value>,
    #[webapi(data_property, enumerable)]
    transfer: v8::Local<'s, v8::Value>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::VideoColorSpace, enumerable, receiver)]
struct VideoColorSpacePrototypeDeclaration {
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, PRIMARIES_SLOT))]
    primaries: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, TRANSFER_SLOT))]
    transfer: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, MATRIX_SLOT))]
    matrix: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, FULL_RANGE_SLOT))]
    full_range: (),
    #[webapi(method = "toJSON", length = 0, callback = to_json_callback)]
    to_json: (),
}

pub(in crate::context_bootstrap) fn install_video_color_space_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    VideoColorSpacePrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

pub(in crate::context_bootstrap) fn video_color_space_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "VideoColorSpace requires the 'new' operator.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<VideoColorSpaceConstructorArgs>(scope, &args) else {
        return;
    };
    if !initialize_web_api_constructor_receiver(scope, args.this(), "VideoColorSpace") {
        return;
    }
    let init = parsed.init;
    let primaries = nullable_string(scope, init.primaries.map(VideoColorPrimaries::label));
    let transfer = nullable_string(
        scope,
        init.transfer.map(VideoTransferCharacteristics::label),
    );
    let matrix = nullable_string(scope, init.matrix.map(VideoMatrixCoefficients::label));
    let full_range = match init.full_range {
        Some(value) => v8::Boolean::new(scope, value).into(),
        None => v8::null(scope).into(),
    };
    VideoColorSpaceObjectDeclaration::new(primaries, transfer, matrix, full_range)
        .initialize(scope, args.this())
        .expect("VideoColorSpace slots initialize");
    rv.set(args.this().into());
}

fn nullable_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<&'static str>,
) -> v8::Local<'s, v8::Value> {
    match value {
        Some(value) => v8str(scope, value).into(),
        None => v8::null(scope).into(),
    }
}

fn slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let slot = args.data().to_rust_string_lossy(scope);
    let target = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("validated VideoColorSpace receiver");
    rv.set(get_private_value(scope, target, &slot).expect("VideoColorSpace slot"));
}

fn to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("validated VideoColorSpace receiver");
    // Return a fresh dictionary in the method's realm using native getter
    // values, ignoring shadowing author properties and prototype setters.
    let json = VideoColorSpaceJsonDeclaration::new(
        get_private_value(scope, target, FULL_RANGE_SLOT).expect("VideoColorSpace fullRange"),
        get_private_value(scope, target, MATRIX_SLOT).expect("VideoColorSpace matrix"),
        get_private_value(scope, target, PRIMARIES_SLOT).expect("VideoColorSpace primaries"),
        get_private_value(scope, target, TRANSFER_SLOT).expect("VideoColorSpace transfer"),
    );
    let Ok(json) = json.bind(scope) else {
        return;
    };
    rv.set(json.into());
}
