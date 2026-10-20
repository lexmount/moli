//! Immutable color metadata, independent of video decoding and rendering.

use crate::{
    util::{get_private_value, throw_type_error, v8str},
    web_api_interfaces,
    webidl::{self, WebIdlDictionarySerialize},
};
use moli_webapi_declare::{
    WebApiFunctionTemplate, WebApiObject, initialize_web_api_constructor_receiver,
};

const PRIMARIES_SLOT: &str = "__moliVideoColorSpacePrimaries";
const TRANSFER_SLOT: &str = "__moliVideoColorSpaceTransfer";
const MATRIX_SLOT: &str = "__moliVideoColorSpaceMatrix";
const FULL_RANGE_SLOT: &str = "__moliVideoColorSpaceFullRange";

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, webidl::WebIdlEnum)]
pub(super) enum VideoColorPrimaries {
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

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, webidl::WebIdlEnum)]
pub(super) enum VideoTransferCharacteristics {
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

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, webidl::WebIdlEnum)]
pub(super) enum VideoMatrixCoefficients {
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

// WebIDL reads dictionary members in lexicographic order.
#[derive(
    Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize, webidl::WebIdlDictionary,
)]
#[webidl(prefix = "VideoColorSpaceInit", snapshot)]
pub(super) struct VideoColorSpaceInit {
    #[webidl(nullable, converter = "boolean")]
    #[webidl(snapshot_order = 3, snapshot_slot = FULL_RANGE_SLOT)]
    pub(super) full_range: Option<bool>,
    #[webidl(nullable, converter = "enum")]
    #[webidl(snapshot_order = 2, snapshot_slot = MATRIX_SLOT)]
    pub(super) matrix: Option<VideoMatrixCoefficients>,
    #[webidl(nullable, converter = "enum")]
    #[webidl(snapshot_order = 0, snapshot_slot = PRIMARIES_SLOT)]
    pub(super) primaries: Option<VideoColorPrimaries>,
    #[webidl(nullable, converter = "enum")]
    #[webidl(snapshot_order = 1, snapshot_slot = TRANSFER_SLOT)]
    pub(super) transfer: Option<VideoTransferCharacteristics>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "VideoColorSpace")]
struct VideoColorSpaceConstructorArgs {
    #[webidl(dictionary)]
    init: VideoColorSpaceInit,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::VideoColorSpace, require_prototype)]
struct VideoColorSpaceObjectDeclaration {}

// Keep toJSON's property order independent of codec configuration snapshots.
#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct VideoColorSpaceJsonDeclaration<'s> {
    full_range: v8::Local<'s, v8::Value>,
    matrix: v8::Local<'s, v8::Value>,
    primaries: v8::Local<'s, v8::Value>,
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
    parsed
        .init
        .initialize_snapshot(scope, args.this())
        .expect("VideoColorSpace slots initialize");
    rv.set(args.this().into());
}

impl VideoColorSpaceInit {
    pub(super) fn from_native_object<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Self {
        fn token<'s, T: webidl::WebIdlEnum>(
            scope: &mut v8::PinScope<'s, '_>,
            object: v8::Local<'s, v8::Object>,
            slot: &str,
        ) -> Option<T> {
            let value = get_private_value(scope, object, slot).expect("native color space slot");
            (!value.is_null())
                .then(|| T::parse_token(&value.to_rust_string_lossy(scope)))
                .flatten()
        }
        let full_range =
            get_private_value(scope, object, FULL_RANGE_SLOT).expect("native color range");
        Self {
            full_range: (!full_range.is_null()).then(|| full_range.boolean_value(scope)),
            matrix: token(scope, object, MATRIX_SLOT),
            primaries: token(scope, object, PRIMARIES_SLOT),
            transfer: token(scope, object, TRANSFER_SLOT),
        }
    }
    pub(super) fn default_rgb() -> Self {
        Self {
            full_range: Some(true),
            matrix: Some(VideoMatrixCoefficients::Rgb),
            primaries: Some(VideoColorPrimaries::Bt709),
            transfer: Some(VideoTransferCharacteristics::Iec61966_2_1),
        }
    }

    pub(super) fn default_yuv() -> Self {
        Self {
            full_range: Some(false),
            matrix: Some(VideoMatrixCoefficients::Bt709),
            primaries: Some(VideoColorPrimaries::Bt709),
            transfer: Some(VideoTransferCharacteristics::Bt709),
        }
    }

    pub(super) fn build<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
        let object = VideoColorSpaceObjectDeclaration::new()
            .bind(scope)
            .expect("native VideoColorSpace");
        self.initialize_snapshot(scope, object)
            .expect("VideoColorSpace slots initialize");
        object
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

// Configuration snapshots share the same dictionary conversion as the public
// VideoColorSpace constructor, including nullable defaults and enum validation.
