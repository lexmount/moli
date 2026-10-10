//! Immutable CPU video frames. Codecs and media-element producers are separate.

mod layout;
mod pixels;
use super::{
    dom_rect::{DomRectInit, build_dom_rect_clone_object},
    video_color_space::VideoColorSpaceInit,
};
use crate::{
    blob::array_buffer_from_bytes,
    util::{get_private_value, set_private_value, throw_type_error, v8str},
    web_api_interfaces, webidl,
};
use layout::{Layout, PixelFormat, PlaneLayout, Rect, orientation};
use moli_webapi_declare::{
    WebApiFunctionTemplate, WebApiObject, initialize_web_api_constructor_receiver,
    web_api_object_target,
};

const INFO: &str = "__moliVideoFrameInfo";
const DATA: &str = "__moliVideoFrameData";

#[derive(Clone, Copy, Default, webidl::WebIdlEnum)]
enum AlphaOption {
    #[default]
    Keep,
    Discard,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "VideoFrameMetadata")]
struct MetadataInit {}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "VideoFrameInit")]
struct FrameInit {
    #[webidl(converter = "enum", default = AlphaOption::Keep)]
    alpha: AlphaOption,
    #[webidl(converter = "enforce_range_unsigned_long")]
    display_height: Option<u32>,
    #[webidl(converter = "enforce_range_unsigned_long")]
    display_width: Option<u32>,
    #[webidl(converter = "unsigned_long_long")]
    duration: Option<u64>,
    #[webidl(converter = "boolean", default = false)]
    flip: bool,
    #[webidl(name = "metadata", dictionary)]
    _metadata: Option<MetadataInit>,
    #[webidl(converter = "double", default = 0.0)]
    rotation: f64,
    #[webidl(converter = "long_long")]
    timestamp: Option<i64>,
    #[webidl(dictionary)]
    visible_rect: Option<DomRectInit>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "VideoFrameBufferInit")]
struct BufferInit<'s> {
    #[webidl(required, converter = "enforce_range_unsigned_long")]
    coded_height: u32,
    #[webidl(required, converter = "enforce_range_unsigned_long")]
    coded_width: u32,
    #[webidl(dictionary)]
    color_space: Option<VideoColorSpaceInit>,
    #[webidl(converter = "enforce_range_unsigned_long")]
    display_height: Option<u32>,
    #[webidl(converter = "enforce_range_unsigned_long")]
    display_width: Option<u32>,
    #[webidl(converter = "enforce_range_unsigned_long_long")]
    duration: Option<u64>,
    #[webidl(converter = "boolean", default = false)]
    flip: bool,
    #[webidl(required, converter = "enum")]
    format: PixelFormat,
    #[webidl(sequence, converter = "dictionary")]
    layout: Option<Vec<PlaneLayout>>,
    #[webidl(name = "metadata", dictionary)]
    _metadata: Option<MetadataInit>,
    #[webidl(converter = "double", default = 0.0)]
    rotation: f64,
    #[webidl(required, converter = "enforce_range_long_long")]
    timestamp: i64,
    #[webidl(converter = "raw", default = webidl::Sequence(Vec::new()))]
    transfer: webidl::Sequence<v8::Local<'s, v8::ArrayBuffer>>,
    #[webidl(dictionary)]
    visible_rect: Option<DomRectInit>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "VideoFrameCopyToOptions")]
struct CopyOptions {
    #[webidl(converter = "enum")]
    color_space: Option<pixels::RgbColorSpace>,
    #[webidl(converter = "enum")]
    format: Option<PixelFormat>,
    #[webidl(sequence, converter = "dictionary")]
    layout: Option<Vec<PlaneLayout>>,
    #[webidl(dictionary)]
    rect: Option<DomRectInit>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "VideoFrame.allocationSize")]
struct AllocationArgs {
    #[webidl(dictionary)]
    options: CopyOptions,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "VideoFrame.copyTo")]
struct CopyArgs<'s> {
    #[webidl(required, converter = "raw")]
    destination: webidl::AllowSharedBufferSource<'s>,
    #[webidl(dictionary)]
    options: CopyOptions,
}

// Metadata is private native state, not public properties. The resource's
// ArrayBuffer is immutable and shared by same-isolate clones until GC.
#[derive(WebApiObject)]
#[webapi(plain)]
struct InfoDeclaration<'s> {
    #[webapi(prototype, value = v8::null(scope))]
    prototype: (),
    #[webapi(slot = "format")]
    format: &'static str,
    #[webapi(slot = "codedWidth")]
    coded_width: u32,
    #[webapi(slot = "codedHeight")]
    coded_height: u32,
    #[webapi(slot = "x")]
    x: u32,
    #[webapi(slot = "y")]
    y: u32,
    #[webapi(slot = "width")]
    width: u32,
    #[webapi(slot = "height")]
    height: u32,
    #[webapi(slot = "displayWidth")]
    display_width: u32,
    #[webapi(slot = "displayHeight")]
    display_height: u32,
    #[webapi(slot = "rotation")]
    rotation: u32,
    #[webapi(slot = "flip")]
    flip: bool,
    #[webapi(slot = "timestamp")]
    timestamp: f64,
    #[webapi(slot = "duration")]
    duration: v8::Local<'s, v8::Value>,
    #[webapi(slot = "colorSpace")]
    color_space: v8::Local<'s, v8::Object>,
    #[webapi(slot = "layout")]
    layout: v8::Local<'s, v8::Array>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::VideoFrame, require_prototype)]
struct FrameDeclaration<'s> {
    #[webapi(slot = INFO)]
    info: v8::Local<'s, v8::Object>,
    #[webapi(slot = DATA)]
    data: v8::Local<'s, v8::ArrayBuffer>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct VideoFrameInfo {
    format: PixelFormat,
    coded_width: u32,
    coded_height: u32,
    visible: Rect,
    display_width: u32,
    display_height: u32,
    rotation: u32,
    flip: bool,
    timestamp: i64,
    duration: Option<u64>,
    color_space: VideoColorSpaceInit,
    layout: Vec<PlaneLayout>,
}

#[derive(Clone, Debug)]
pub(crate) struct VideoFrameClonePayload {
    pub info: VideoFrameInfo,
    pub bytes: Vec<u8>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::VideoFrame, enumerable, receiver)]
struct FramePrototype {
    #[webapi(accessor_property, getter = getter, data = v8str(scope, "format"))]
    format: (),
    #[webapi(accessor_property, getter = getter, data = v8str(scope, "codedWidth"))]
    coded_width: (),
    #[webapi(accessor_property, getter = getter, data = v8str(scope, "codedHeight"))]
    coded_height: (),
    #[webapi(accessor_property, getter = getter, data = v8str(scope, "codedRect"))]
    coded_rect: (),
    #[webapi(accessor_property, getter = getter, data = v8str(scope, "visibleRect"))]
    visible_rect: (),
    #[webapi(accessor_property, getter = getter, data = v8str(scope, "rotation"))]
    rotation: (),
    #[webapi(accessor_property, getter = getter, data = v8str(scope, "flip"))]
    flip: (),
    #[webapi(accessor_property, getter = getter, data = v8str(scope, "displayWidth"))]
    display_width: (),
    #[webapi(accessor_property, getter = getter, data = v8str(scope, "displayHeight"))]
    display_height: (),
    #[webapi(accessor_property, getter = getter, data = v8str(scope, "timestamp"))]
    timestamp: (),
    #[webapi(accessor_property, getter = getter, data = v8str(scope, "duration"))]
    duration: (),
    #[webapi(accessor_property, getter = getter, data = v8str(scope, "colorSpace"))]
    color_space: (),
    #[webapi(method, length = 0, callback = metadata)]
    metadata: (),
    #[webapi(method, length = 0, callback = allocation_size)]
    allocation_size: (),
    #[webapi(method, length = 1, callback = copy_to, returns_promise)]
    copy_to: (),
    #[webapi(method, length = 0, callback = clone_frame)]
    clone: (),
    #[webapi(method, length = 0, callback = close)]
    close: (),
}

pub(in crate::context_bootstrap) fn install_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    FramePrototype::initialize_prototype_template(scope, template.prototype_template(scope));
}

pub(in crate::context_bootstrap) fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "VideoFrame requires the 'new' operator.");
        return;
    }
    let Some(parsed) = parse_constructor_args(scope, &args) else {
        return;
    };
    if !initialize_web_api_constructor_receiver(scope, args.this(), "VideoFrame") {
        return;
    }
    let Some((info, data)) = construct(scope, parsed) else {
        return;
    };
    declaration(scope, &info, data)
        .initialize(scope, args.this())
        .expect("VideoFrame native slots");
    rv.set(args.this().into());
}

enum ConstructorArgs<'s> {
    Buffer(webidl::AllowSharedBufferSource<'s>, BufferInit<'s>),
    Image(v8::Local<'s, v8::Object>, FrameInit),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "VideoFrame")]
struct BufferConstructorArgs<'s> {
    #[webidl(required, converter = "raw")]
    source: webidl::AllowSharedBufferSource<'s>,
    #[webidl(required, dictionary)]
    init: BufferInit<'s>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "VideoFrame")]
struct ImageConstructorArgs<'s> {
    #[webidl(required, interface = super::canvas::CanvasImageSource)]
    image: v8::Local<'s, v8::Object>,
    #[webidl(dictionary)]
    init: FrameInit,
}

fn parse_constructor_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<ConstructorArgs<'s>> {
    // Web IDL overload discrimination is based only on native source identity.
    // Both branches then use the same derived conversion/order/error machinery.
    let source = args.get(0);
    if source.is_array_buffer() || source.is_shared_array_buffer() || source.is_array_buffer_view()
    {
        let parsed = webidl::parse_args::<BufferConstructorArgs>(scope, args)?;
        Some(ConstructorArgs::Buffer(parsed.source, parsed.init))
    } else {
        let parsed = webidl::parse_args::<ImageConstructorArgs>(scope, args)?;
        Some(ConstructorArgs::Image(parsed.image, parsed.init))
    }
}

fn construct<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: ConstructorArgs<'s>,
) -> Option<(VideoFrameInfo, v8::Local<'s, v8::ArrayBuffer>)> {
    if let ConstructorArgs::Buffer(source, init) = parsed {
        let coded = Rect::coded(init.coded_width, init.coded_height);
        if coded.width == 0 || coded.height == 0 {
            throw_type_error(scope, "The coded size must be nonzero.");
            return None;
        }
        let visible = type_result(
            scope,
            Rect::parse(coded, init.visible_rect, coded, init.format),
        )?;
        let layout = type_result(
            scope,
            Layout::compute(coded, init.format, init.layout.as_deref()),
        )?;
        if source.byte_length() < layout.size as usize {
            throw_type_error(scope, "The source buffer is too small.");
            return None;
        }
        let (rotation, flip) = orientation(0, false, init.rotation, init.flip);
        let (display_width, display_height) = display_size(
            scope,
            init.display_width,
            init.display_height,
            visible.width,
            visible.height,
            rotation,
        )?;
        for (index, buffer) in init.transfer.0.iter().enumerate() {
            if buffer.was_detached()
                || !buffer.is_detachable()
                || init.transfer.0[..index]
                    .iter()
                    .any(|other| other.strict_equals((*buffer).into()))
            {
                webidl::throw_dom_exception(
                    scope,
                    "DataCloneError",
                    "The transfer list contains a duplicate or detached ArrayBuffer.",
                );
                return None;
            }
        }
        let bytes = source.to_vec(scope);
        let data = array_buffer_from_bytes(scope, bytes[..layout.size as usize].to_vec())?;
        let info = VideoFrameInfo {
            format: init.format,
            coded_width: coded.width,
            coded_height: coded.height,
            visible,
            display_width,
            display_height,
            rotation,
            flip,
            timestamp: init.timestamp,
            duration: init.duration,
            color_space: init.color_space.unwrap_or_else(|| {
                if init.format.rgb() {
                    VideoColorSpaceInit::default_rgb()
                } else {
                    VideoColorSpaceInit::default_yuv()
                }
            }),
            layout: layout.planes.iter().map(|p| p.layout).collect(),
        };
        for buffer in init.transfer.0 {
            if buffer.detach(None) != Some(true) {
                webidl::throw_dom_exception(
                    scope,
                    "DataCloneError",
                    "The ArrayBuffer cannot be transferred.",
                );
                return None;
            }
        }
        return Some((info, data));
    }
    let ConstructorArgs::Image(object, init) = parsed else {
        unreachable!("buffer construction returned")
    };
    let (mut info, data) = if web_api_interfaces::VideoFrame::is_instance(scope, object) {
        require_open(scope, object)?;
        (read_info(scope, object), data_buffer(scope, object)?)
    } else {
        let (bytes, width, height) = snapshot_image(scope, object)?;
        if width == 0 || height == 0 {
            webidl::throw_dom_exception(scope, "InvalidStateError", "The source image is empty.");
            return None;
        }
        let Some(timestamp) = init.timestamp else {
            throw_type_error(scope, "A timestamp is required for an image source.");
            return None;
        };
        let format = PixelFormat::Rgba;
        let layout = type_result(
            scope,
            Layout::compute(Rect::coded(width, height), format, None),
        )?;
        (
            VideoFrameInfo {
                format,
                coded_width: width,
                coded_height: height,
                visible: Rect::coded(width, height),
                display_width: width,
                display_height: height,
                rotation: 0,
                flip: false,
                timestamp,
                duration: None,
                color_space: VideoColorSpaceInit::default_rgb(),
                layout: layout.planes.iter().map(|p| p.layout).collect(),
            },
            array_buffer_from_bytes(scope, bytes)?,
        )
    };
    let old_visible = info.visible;
    let visible = type_result(
        scope,
        Rect::parse(
            info.visible,
            init.visible_rect,
            Rect::coded(info.coded_width, info.coded_height),
            info.format,
        ),
    )?;
    let (old_w, old_h) = if info.rotation.is_multiple_of(180) {
        (info.display_width, info.display_height)
    } else {
        (info.display_height, info.display_width)
    };
    let width = ((visible.width as f64 / old_visible.width as f64) * old_w as f64).round() as u32;
    let height =
        ((visible.height as f64 / old_visible.height as f64) * old_h as f64).round() as u32;
    if width == 0 || height == 0 {
        throw_type_error(scope, "The computed display size must be nonzero.");
        return None;
    }
    let (rotation, flip) = orientation(info.rotation, info.flip, init.rotation, init.flip);
    let (display_width, display_height) = display_size(
        scope,
        init.display_width,
        init.display_height,
        width,
        height,
        rotation,
    )?;
    info.visible = visible;
    info.display_width = display_width;
    info.display_height = display_height;
    info.rotation = rotation;
    info.flip = flip;
    info.timestamp = init.timestamp.unwrap_or(info.timestamp);
    info.duration = init.duration.or(info.duration);
    if matches!(init.alpha, AlphaOption::Discard) {
        info.format = info.format.opaque();
        info.layout.truncate(info.format.planes().len());
    }
    Some((info, data))
}

fn type_result<T>(scope: &mut v8::PinScope<'_, '_>, result: Result<T, &'static str>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(message) => {
            throw_type_error(scope, message);
            None
        }
    }
}

fn display_size(
    scope: &mut v8::PinScope<'_, '_>,
    width: Option<u32>,
    height: Option<u32>,
    default_width: u32,
    default_height: u32,
    rotation: u32,
) -> Option<(u32, u32)> {
    match (width, height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
        (None, None) => Some(if rotation.is_multiple_of(180) {
            (default_width, default_height)
        } else {
            (default_height, default_width)
        }),
        _ => {
            throw_type_error(
                scope,
                "Both display dimensions must be nonzero and provided together.",
            );
            None
        }
    }
}

fn declaration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    info: &VideoFrameInfo,
    data: v8::Local<'s, v8::ArrayBuffer>,
) -> FrameDeclaration<'s> {
    let layouts: Vec<_> = info
        .layout
        .iter()
        .flat_map(|p| {
            [
                v8::Number::new(scope, p.offset as f64).into(),
                v8::Number::new(scope, p.stride as f64).into(),
            ]
        })
        .collect();
    let layouts = v8::Array::new_with_elements(scope, &layouts);
    let duration = info
        .duration
        .map(|v| v8::Number::new(scope, v as f64).into())
        .unwrap_or_else(|| v8::null(scope).into());
    let color_space = info.color_space.build(scope);
    let rect = info.visible;
    let info = InfoDeclaration::new(
        info.format.label(),
        info.coded_width,
        info.coded_height,
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        info.display_width,
        info.display_height,
        info.rotation,
        info.flip,
        info.timestamp as f64,
        duration,
        color_space,
        layouts,
    )
    .bind(scope)
    .expect("VideoFrame native metadata");
    FrameDeclaration::new(info, data)
}

fn info_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let target = web_api_object_target(scope, object).expect("VideoFrame native receiver");
    v8::Local::try_from(get_private_value(scope, target, INFO).expect("VideoFrame metadata"))
        .expect("VideoFrame metadata object")
}

fn data_buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::ArrayBuffer>> {
    let target = web_api_object_target(scope, object)?;
    v8::Local::try_from(get_private_value(scope, target, DATA)?).ok()
}

pub(crate) fn is_closed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    data_buffer(scope, object).is_none()
}
fn require_open<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<()> {
    if is_closed(scope, object) {
        webidl::throw_dom_exception(scope, "InvalidStateError", "The VideoFrame is closed.");
        None
    } else {
        Some(())
    }
}

fn read_info<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> VideoFrameInfo {
    let info = info_object(scope, object);
    let value = |scope: &mut v8::PinScope<'s, '_>, key| {
        get_private_value(scope, info, key).expect("VideoFrame metadata slot")
    };
    let number = |scope: &mut v8::PinScope<'s, '_>, key| {
        value(scope, key)
            .number_value(scope)
            .expect("native number")
    };
    let layouts =
        v8::Local::<v8::Array>::try_from(value(scope, "layout")).expect("native plane layouts");
    let layout = (0..layouts.length())
        .step_by(2)
        .map(|i| PlaneLayout {
            offset: layouts
                .get_index(scope, i)
                .unwrap()
                .uint32_value(scope)
                .unwrap(),
            stride: layouts
                .get_index(scope, i + 1)
                .unwrap()
                .uint32_value(scope)
                .unwrap(),
        })
        .collect();
    let color_space =
        v8::Local::<v8::Object>::try_from(value(scope, "colorSpace")).expect("native color space");
    VideoFrameInfo {
        format: <PixelFormat as webidl::WebIdlEnum>::parse_token(
            &value(scope, "format").to_rust_string_lossy(scope),
        )
        .expect("native pixel format"),
        coded_width: number(scope, "codedWidth") as u32,
        coded_height: number(scope, "codedHeight") as u32,
        visible: Rect {
            x: number(scope, "x") as u32,
            y: number(scope, "y") as u32,
            width: number(scope, "width") as u32,
            height: number(scope, "height") as u32,
        },
        display_width: number(scope, "displayWidth") as u32,
        display_height: number(scope, "displayHeight") as u32,
        rotation: number(scope, "rotation") as u32,
        flip: value(scope, "flip").boolean_value(scope),
        timestamp: number(scope, "timestamp") as i64,
        duration: (!value(scope, "duration").is_null()).then(|| number(scope, "duration") as u64),
        color_space: VideoColorSpaceInit::from_native_object(scope, color_space),
        layout,
    }
}

fn getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let key = args.data().to_rust_string_lossy(scope);
    let info = info_object(scope, args.this());
    if is_closed(scope, args.this()) && key != "timestamp" && key != "duration" {
        match key.as_str() {
            "format" | "codedRect" | "visibleRect" => rv.set_null(),
            "flip" => rv.set_bool(false),
            "colorSpace" => {
                rv.set(get_private_value(scope, info, "colorSpace").expect("closed color space"))
            }
            _ => rv.set_uint32(0),
        }
    } else if key == "codedRect" || key == "visibleRect" {
        let info = read_info(scope, args.this());
        let rect = if key == "codedRect" {
            Rect::coded(info.coded_width, info.coded_height)
        } else {
            info.visible
        };
        rv.set(build_dom_rect_clone_object(scope, false, rect.values()).into());
    } else {
        rv.set(get_private_value(scope, info, &key).expect("native VideoFrame slot"));
    }
}

fn metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if require_open(scope, args.this()).is_some() {
        rv.set(v8::Object::new(scope).into());
    }
}
fn clone_frame<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if require_open(scope, args.this()).is_none() {
        return;
    }
    let info = read_info(scope, args.this());
    let data = data_buffer(scope, args.this()).unwrap();
    rv.set(
        declaration(scope, &info, data)
            .bind(scope)
            .expect("VideoFrame clone")
            .into(),
    );
}
fn close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'_, v8::Value>,
) {
    close_object(scope, args.this());
}
pub(crate) fn close_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    if is_closed(scope, object) {
        return;
    }
    let target = web_api_object_target(scope, object).expect("native VideoFrame");
    let info = info_object(scope, object);
    let color = VideoColorSpaceInit::default().build(scope);
    set_private_value(scope, info, "colorSpace", color.into());
    set_private_value(scope, target, DATA, v8::null(scope).into());
}

fn copy_layout(
    scope: &mut v8::PinScope<'_, '_>,
    info: &VideoFrameInfo,
    options: &CopyOptions,
) -> Option<(Rect, PixelFormat, Layout)> {
    let rect = type_result(
        scope,
        Rect::parse(
            info.visible,
            options.rect,
            Rect::coded(info.coded_width, info.coded_height),
            info.format,
        ),
    )?;
    let format = options.format.unwrap_or(info.format);
    if options.format.is_some() && !format.rgb() {
        webidl::throw_dom_exception(
            scope,
            "NotSupportedError",
            "Explicit copy formats must be RGB.",
        );
        return None;
    }
    let destination_rect = if options.format.is_some() {
        Rect::coded(rect.width, rect.height)
    } else {
        rect
    };
    let layout = type_result(
        scope,
        Layout::compute(destination_rect, format, options.layout.as_deref()),
    )?;
    Some((rect, format, layout))
}
fn allocation_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<AllocationArgs>(scope, &args) else {
        return;
    };
    if require_open(scope, args.this()).is_none() {
        return;
    }
    let info = read_info(scope, args.this());
    if let Some((_, _, layout)) = copy_layout(scope, &info, &parsed.options) {
        rv.set_uint32(layout.size);
    }
}
fn copy_to<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CopyArgs>(scope, &args) else {
        return;
    };
    if require_open(scope, args.this()).is_none() {
        return;
    }
    let info = read_info(scope, args.this());
    let Some((rect, format, layout)) = copy_layout(scope, &info, &parsed.options) else {
        return;
    };
    if parsed.destination.byte_length() < layout.size as usize {
        throw_type_error(scope, "The destination buffer is too small.");
        return;
    }
    let buffer = data_buffer(scope, args.this()).unwrap();
    let backing = buffer.get_backing_store();
    let bytes: Vec<_> = backing.iter().map(|v| v.get()).collect();
    let (bytes, source_layout) = if parsed.options.format.is_some() {
        let Some(bytes) = pixels::convert(
            scope,
            &info,
            &bytes,
            rect,
            format,
            parsed.options.color_space.unwrap_or_default(),
        ) else {
            return;
        };
        (
            bytes,
            vec![PlaneLayout {
                offset: 0,
                stride: rect.width * 4,
            }],
        )
    } else {
        (bytes, info.layout)
    };
    let mut results = Vec::new();
    for (index, plane) in layout.planes.iter().enumerate() {
        let source = source_layout[index];
        let start = if parsed.options.format.is_some() {
            0
        } else {
            source.offset as usize
                + plane.top as usize * source.stride as usize
                + plane.left as usize
        };
        for row in 0..plane.rows as usize {
            let start = start + row * source.stride as usize;
            let dest = plane.layout.offset as usize + row * plane.layout.stride as usize;
            if !parsed.destination.write_bytes_at(
                scope,
                dest,
                &bytes[start..start + plane.row_bytes as usize],
            ) {
                throw_type_error(scope, "The destination buffer was detached.");
                return;
            }
        }
        results.push(
            PlaneLayoutResult::new(plane.layout.offset, plane.layout.stride)
                .bind(scope)
                .expect("plane layout")
                .into(),
        );
    }
    let resolver = v8::PromiseResolver::new(scope).expect("VideoFrame copy promise");
    let result = v8::Array::new_with_elements(scope, &results);
    resolver.resolve(scope, result.into());
    rv.set(resolver.get_promise(scope).into());
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct PlaneLayoutResult {
    #[webapi(data_property, enumerable)]
    offset: u32,
    #[webapi(data_property, enumerable)]
    stride: u32,
}

pub(crate) fn video_frame_clone_payload_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<VideoFrameClonePayload> {
    let buffer = data_buffer(scope, object)?;
    let backing = buffer.get_backing_store();
    Some(VideoFrameClonePayload {
        info: read_info(scope, object),
        bytes: backing.iter().map(|v| v.get()).collect(),
    })
}
pub(crate) fn build_video_frame_from_clone_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: VideoFrameClonePayload,
) -> Option<v8::Local<'s, v8::Object>> {
    if !super::exposed_interfaces::is_realm_interface_exposed(scope, "VideoFrame") {
        return None;
    }
    let info = &payload.info;
    if info.coded_width == 0
        || info.coded_height == 0
        || info.display_width == 0
        || info.display_height == 0
        || ![0, 90, 180, 270].contains(&info.rotation)
    {
        return None;
    }
    let coded = Rect::coded(info.coded_width, info.coded_height);
    let rect = DomRectInit {
        x: info.visible.x as f64,
        y: info.visible.y as f64,
        width: info.visible.width as f64,
        height: info.visible.height as f64,
    };
    Rect::parse(coded, Some(rect), coded, info.format).ok()?;
    let layout = Layout::compute(coded, info.format, Some(&info.layout)).ok()?;
    if payload.bytes.len() < layout.size as usize {
        return None;
    }
    let data = array_buffer_from_bytes(scope, payload.bytes)?;
    declaration(scope, &payload.info, data).bind(scope).ok()
}

pub(in crate::context_bootstrap) fn rendered_pixels<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<(Vec<u8>, u32, u32)> {
    require_open(scope, object)?;
    let payload = video_frame_clone_payload_from_object(scope, object)?;
    let info = payload.info;
    let pixels = pixels::convert(
        scope,
        &info,
        &payload.bytes,
        info.visible,
        PixelFormat::Rgba,
        pixels::RgbColorSpace::Srgb,
    )?;
    pixels::render(scope, &info, pixels)
}

fn snapshot_image<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<(Vec<u8>, u32, u32)> {
    v8::tc_scope!(let tc, scope);
    let pixels = super::canvas::image_source_pixels(tc, object);
    if pixels.is_none() {
        if !tc.has_caught() {
            webidl::throw_dom_exception(tc, "InvalidStateError", "The source has no image data.");
        }
        tc.rethrow();
    }
    pixels
}
