use super::pixels::CanvasImage;
use super::*;
use crate::context_bootstrap::image_data::{ImageDataColorSpace, ImageDataPixelFormat};
use crate::context_bootstrap::new_dom_exception_value;
use crate::util::{
    callback_data_index_value, callback_data_item, context_host_ptr_from_global_bridge,
    get_private_value, set_private_value,
};
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

mod options;
use options::{BitmapOptions, BitmapParameters};

#[derive(Debug)]
pub(crate) struct BitmapTaskResult {
    image: CanvasImage,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum BitmapRejection {
    InvalidState,
}

const IMAGE_BITMAP_WIDTH_SLOT: &str = "__moliImageBitmapWidth";
const IMAGE_BITMAP_HEIGHT_SLOT: &str = "__moliImageBitmapHeight";
const IMAGE_BITMAP_PIXELS_SLOT: &str = "__moliImageBitmapPixels";
const IMAGE_BITMAP_PREMULTIPLIED_SLOT: &str = "__moliImageBitmapPremultiplied";
const IMAGE_BITMAP_COLOR_SPACE_SLOT: &str = "__moliImageBitmapColorSpace";
const IMAGE_BITMAP_PIXEL_FORMAT_SLOT: &str = "__moliImageBitmapPixelFormat";

const IMAGE_BITMAP_DIMENSION_SLOTS: &[&str] = &[IMAGE_BITMAP_WIDTH_SLOT, IMAGE_BITMAP_HEIGHT_SLOT];

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ImageBitmap)]
struct ImageBitmapObjectDeclaration {
    #[webapi(slot = IMAGE_BITMAP_WIDTH_SLOT)]
    width: f64,
    #[webapi(slot = IMAGE_BITMAP_HEIGHT_SLOT)]
    height: f64,
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ImageBitmap, enumerable, receiver)]
struct ImageBitmapPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = image_bitmap_dimension_getter,
        data = callback_data_index_value(scope, 0)
    )]
    width: (),

    #[webapi(
        accessor_property,
        getter = image_bitmap_dimension_getter,
        data = callback_data_index_value(scope, 1)
    )]
    height: (),

    #[webapi(method, length = 0, callback = image_bitmap_close_callback)]
    close: (),
}

pub(super) fn install_image_bitmap_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    ImageBitmapPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

pub(crate) fn window_create_image_bitmap_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !crate::context_bootstrap::require_same_origin_window_receiver(scope, args.this(), false) {
        return;
    }
    create_image_bitmap_callback(scope, args, rv);
}

pub(crate) fn create_image_bitmap_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.is_construct_call() {
        throw_type_error(scope, "createImageBitmap is not a constructor");
        return;
    }
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    // Promise-returning Web IDL operations turn conversion failures (including
    // arbitrary getter exceptions) into rejections of the returned Promise.
    let parsed = {
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let mut conversion_scope = try_catch.init();
        match parse_bitmap_arguments(&mut conversion_scope, &args) {
            Ok(parsed) => Ok(parsed),
            Err(error) => {
                webidl::throw_error(&mut conversion_scope, &error);
                let exception = conversion_scope
                    .exception()
                    .unwrap_or_else(|| v8::undefined(&conversion_scope).into());
                conversion_scope.reset();
                Err(exception)
            }
        }
    };
    let (source, parameters) = match parsed {
        Ok(parsed) => parsed,
        Err(exception) => {
            let _ = resolver.reject(scope, exception);
            return;
        }
    };
    if parameters
        .crop
        .is_some_and(|[_, _, width, height]| width == 0 || height == 0)
    {
        let message = crate::util::v8str(scope, "The crop rectangle has a zero dimension.");
        let error = v8::Exception::range_error(scope, message);
        let _ = resolver.reject(scope, error);
        return;
    }
    if parameters.options.resize_width == Some(0) || parameters.options.resize_height == Some(0) {
        reject_bitmap(scope, resolver);
        return;
    }
    // Snapshot only after all observable conversions, which may mutate or
    // detach the source. No V8 handles or live DOM state cross the task boundary.
    let (input, exception) = {
        v8::tc_scope!(let tc, scope);
        let input = snapshot_bitmap_source(tc, source);
        let exception = tc.exception();
        if exception.is_some() {
            tc.reset();
        }
        (input, exception)
    };
    if let Some(exception) = exception {
        let _ = resolver.reject(scope, exception);
        return;
    }
    let input = match input {
        Some(input) => input,
        None => {
            reject_bitmap(scope, resolver);
            return;
        }
    };
    let producer = BitmapTaskProducer::register(scope, resolver);
    let Some(producer) = producer else {
        reject_bitmap(scope, resolver);
        return;
    };
    match input {
        BitmapInput::Pixels(image) => {
            producer.send(parameters.apply(image));
        }
        BitmapInput::Blob(bytes) => {
            let decode = move || {
                let result = moli_image::decode_raster_image(&bytes)
                    .map_err(|_| BitmapRejection::InvalidState)
                    .and_then(|decoded| {
                        CanvasImage::rgba8(
                            decoded.image.width,
                            decoded.image.height,
                            decoded.image.rgba,
                        )
                        .ok_or(BitmapRejection::InvalidState)
                    })
                    .and_then(|image| parameters.apply(image));
                producer.send(result);
            };
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn_blocking(decode);
            } else {
                std::thread::spawn(decode);
            }
        }
    }
}

enum BitmapTaskProducer {
    Window(crate::page_task_queue::RendererPageBitmapTaskProducer),
    Worker(crate::worker::WorkerBitmapTaskProducer),
}

impl BitmapTaskProducer {
    fn register(
        scope: &mut v8::PinScope<'_, '_>,
        resolver: v8::Local<'_, v8::PromiseResolver>,
    ) -> Option<Self> {
        if let Some(producer) = crate::worker::register_worker_bitmap_task(scope, resolver) {
            Some(Self::Worker(producer))
        } else if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
            // SAFETY: the bridge points at this callback's live Window host.
            unsafe { &mut *host_ptr }
                .register_pending_bitmap_task(scope, resolver)
                .map(Self::Window)
        } else {
            None
        }
    }

    fn send(self, result: Result<BitmapTaskResult, BitmapRejection>) {
        match self {
            Self::Window(producer) => {
                let _ = producer.send(result);
            }
            Self::Worker(producer) => producer.send(result),
        }
    }
}

struct ImageBitmapSource;

impl ImageBitmapSource {
    const NAME: &'static str = "ImageBitmapSource";

    fn is_instance<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        source: v8::Local<'s, v8::Object>,
    ) -> bool {
        web_api_interfaces::Blob::is_instance(scope, source)
            || web_api_interfaces::ImageData::is_instance(scope, source)
            || CanvasImageSource::is_instance(scope, source)
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "createImageBitmap")]
struct BitmapArgs<'s> {
    #[webidl(required, interface = ImageBitmapSource)]
    source: v8::Local<'s, v8::Object>,
    #[webidl(dictionary)]
    options: BitmapOptions,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "createImageBitmap")]
struct BitmapCropArgs<'s> {
    #[webidl(required, interface = ImageBitmapSource)]
    source: v8::Local<'s, v8::Object>,
    #[webidl(required, converter = "long")]
    sx: i32,
    #[webidl(required, converter = "long")]
    sy: i32,
    #[webidl(required, converter = "long")]
    sw: i32,
    #[webidl(required, converter = "long")]
    sh: i32,
    #[webidl(dictionary)]
    options: BitmapOptions,
}

enum BitmapInput {
    Blob(Vec<u8>),
    Pixels(CanvasImage),
}

fn snapshot_bitmap_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> Option<BitmapInput> {
    let source = moli_webapi_declare::web_api_object_target(scope, source)?;
    if web_api_interfaces::Blob::is_instance(scope, source) {
        return crate::blob::blob_bytes_from_object(scope, source).map(BitmapInput::Blob);
    }
    let image = if web_api_interfaces::ImageData::is_instance(scope, source) {
        crate::context_bootstrap::image_data::image_data_pixel_snapshot(scope, source)?
    } else {
        super::image_source::image_source_image(scope, source)?
    };
    if image.pixels.width() == 0 || image.pixels.height() == 0 {
        return None;
    }
    Some(BitmapInput::Pixels(image))
}

fn parse_bitmap_arguments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<(v8::Local<'s, v8::Object>, BitmapParameters), webidl::WebIdlError> {
    if matches!(args.length(), 3 | 4) {
        return Err(webidl::WebIdlError::custom_message(
            "The crop overload requires five arguments.",
        ));
    }
    if args.length() >= 5 {
        let parsed = webidl::try_parse_args::<BitmapCropArgs>(scope, args)?;
        Ok((
            parsed.source,
            BitmapParameters {
                crop: Some([parsed.sx, parsed.sy, parsed.sw, parsed.sh]),
                options: parsed.options,
            },
        ))
    } else {
        let parsed = webidl::try_parse_args::<BitmapArgs>(scope, args)?;
        Ok((
            parsed.source,
            BitmapParameters {
                crop: None,
                options: parsed.options,
            },
        ))
    }
}

pub(crate) fn settle_bitmap_task_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    result: Result<BitmapTaskResult, BitmapRejection>,
) {
    let bitmap = result.ok().and_then(|result| {
        let width = result.image.pixels.width();
        let height = result.image.pixels.height();
        let color_space = result.image.color_space;
        let pixel_format = ImageDataPixelFormat::from_native(result.image.pixels.format());
        let premultiplied =
            result.image.pixels.alpha_mode() == moli_canvas::AlphaMode::Premultiplied;
        let pixels = result.image.into_view(scope)?;
        build_image_bitmap_from_data(
            scope,
            BitmapData {
                pixels,
                width,
                height,
                color_space,
                pixel_format,
                premultiplied,
            },
        )
    });
    match bitmap {
        Some(bitmap) => {
            let _ = resolver.resolve(scope, bitmap.into());
        }
        None => reject_bitmap(scope, resolver),
    }
}

fn reject_bitmap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) {
    let error = new_dom_exception_value(
        scope,
        "The image source could not be decoded or allocated.",
        "InvalidStateError",
    );
    let _ = resolver.reject(scope, error);
}

pub(super) fn image_bitmap_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bitmap: v8::Local<'s, v8::Object>,
) -> Option<CanvasImage> {
    let data = image_bitmap_data(scope, bitmap)?;
    CanvasImage::from_view(
        data.pixels,
        data.width,
        data.height,
        data.pixel_format,
        data.color_space,
        if data.premultiplied {
            moli_canvas::AlphaMode::Premultiplied
        } else {
            moli_canvas::AlphaMode::Straight
        },
    )
}

pub(super) struct BitmapData<'s> {
    pub(super) pixels: v8::Local<'s, v8::ArrayBufferView>,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) premultiplied: bool,
    pub(super) color_space: ImageDataColorSpace,
    pub(super) pixel_format: ImageDataPixelFormat,
}

pub(super) fn build_image_bitmap_from_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: BitmapData<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let bitmap = build_image_bitmap_object(scope, data.width, data.height)?;
    set_private_value(scope, bitmap, IMAGE_BITMAP_PIXELS_SLOT, data.pixels.into());
    let color_space = crate::util::v8str(scope, data.color_space.label());
    set_private_value(
        scope,
        bitmap,
        IMAGE_BITMAP_COLOR_SPACE_SLOT,
        color_space.into(),
    );
    let pixel_format = crate::util::v8str(scope, data.pixel_format.label());
    set_private_value(
        scope,
        bitmap,
        IMAGE_BITMAP_PIXEL_FORMAT_SLOT,
        pixel_format.into(),
    );
    set_private_value(
        scope,
        bitmap,
        IMAGE_BITMAP_PREMULTIPLIED_SLOT,
        v8::Boolean::new(scope, data.premultiplied).into(),
    );
    Some(bitmap)
}

pub(super) fn image_bitmap_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bitmap: v8::Local<'s, v8::Object>,
) -> Option<BitmapData<'s>> {
    let bitmap = moli_webapi_declare::web_api_object_target(scope, bitmap)?;
    let width = get_private_value(scope, bitmap, IMAGE_BITMAP_WIDTH_SLOT)?.uint32_value(scope)?;
    let height = get_private_value(scope, bitmap, IMAGE_BITMAP_HEIGHT_SLOT)?.uint32_value(scope)?;
    let view = v8::Local::<v8::ArrayBufferView>::try_from(get_private_value(
        scope,
        bitmap,
        IMAGE_BITMAP_PIXELS_SLOT,
    )?)
    .ok()?;
    let premultiplied = get_private_value(scope, bitmap, IMAGE_BITMAP_PREMULTIPLIED_SLOT)
        .is_some_and(|value| value.boolean_value(scope));
    let color_space = get_private_value(scope, bitmap, IMAGE_BITMAP_COLOR_SPACE_SLOT)?
        .to_string(scope)?
        .to_rust_string_lossy(scope)
        .parse()
        .ok()?;
    let pixel_format = get_private_value(scope, bitmap, IMAGE_BITMAP_PIXEL_FORMAT_SLOT)?
        .to_string(scope)?
        .to_rust_string_lossy(scope)
        .parse()
        .ok()?;
    Some(BitmapData {
        pixels: view,
        width,
        height,
        premultiplied,
        color_space,
        pixel_format,
    })
}

fn build_image_bitmap_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    width: u32,
    height: u32,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = global_constructor_prototype(scope, "ImageBitmap")?;
    let bitmap = v8::Object::new(scope);
    if bitmap.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    ImageBitmapObjectDeclaration::new(width as f64, height as f64)
        .initialize(scope, bitmap)
        .ok()?;
    Some(bitmap)
}

fn image_bitmap_dimension_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(bitmap) = moli_webapi_declare::web_api_object_target(scope, args.this()) else {
        return;
    };
    let Some(slot) = callback_data_item(
        scope,
        &args,
        IMAGE_BITMAP_DIMENSION_SLOTS,
        "ImageBitmap dimension slots",
    ) else {
        rv.set_uint32(0);
        return;
    };
    let value = get_private_value(scope, bitmap, slot)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(0.0);
    rv.set_uint32(value.max(0.0) as u32);
}

fn image_bitmap_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    detach_image_bitmap(scope, args.this());
    rv.set_undefined();
}

pub(super) fn detach_image_bitmap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bitmap: v8::Local<'s, v8::Object>,
) {
    let Some(bitmap) = moli_webapi_declare::web_api_object_target(scope, bitmap) else {
        return;
    };
    for slot in IMAGE_BITMAP_DIMENSION_SLOTS {
        set_private_value(scope, bitmap, slot, v8::Number::new(scope, 0.0).into());
    }
    set_private_value(
        scope,
        bitmap,
        IMAGE_BITMAP_PIXELS_SLOT,
        v8::undefined(scope).into(),
    );
}
