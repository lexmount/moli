use super::backing_store::canvas_like_pixels_copy;
use super::offscreen::offscreen_canvas_receiver_branded;
use super::*;
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
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    premultiplied: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum BitmapRejection {
    InvalidState,
}

const IMAGE_BITMAP_WIDTH_SLOT: &str = "__moliImageBitmapWidth";
const IMAGE_BITMAP_HEIGHT_SLOT: &str = "__moliImageBitmapHeight";
const IMAGE_BITMAP_PIXELS_SLOT: &str = "__moliImageBitmapPixels";
const IMAGE_BITMAP_PREMULTIPLIED_SLOT: &str = "__moliImageBitmapPremultiplied";

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
#[webapi(interface = web_api_interfaces::ImageBitmap, enumerable)]
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
    let input = match source.snapshot(scope) {
        Some(input) => input,
        None => {
            reject_bitmap(scope, resolver);
            return;
        }
    };
    let producer = context_host_ptr_from_global_bridge(scope).and_then(|host_ptr| {
        // SAFETY: the bridge points at this callback's live Window host.
        unsafe { &mut *host_ptr }.register_pending_bitmap_task(scope, resolver)
    });
    let Some(producer) = producer else {
        reject_bitmap(scope, resolver);
        return;
    };
    match input {
        BitmapInput::Pixels(image) => {
            let _ = producer.send(parameters.apply(image));
        }
        BitmapInput::Blob(bytes) => {
            let decode = move || {
                let result = moli_image::decode_raster_image(&bytes)
                    .map_err(|_| BitmapRejection::InvalidState)
                    .and_then(|decoded| parameters.apply(decoded.image));
                let _ = producer.send(result);
            };
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn_blocking(decode);
            } else {
                std::thread::spawn(decode);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum BitmapSourceKind {
    Blob,
    Canvas,
    ImageData,
    ImageBitmap,
}

struct BitmapSource<'s> {
    object: v8::Local<'s, v8::Object>,
    kind: BitmapSourceKind,
}

impl<'s> webidl::WebIdlConverter<'s> for BitmapSource<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        let object = webidl::convert::<v8::Local<'s, v8::Object>>(scope, value, context)?;
        let kind = if crate::blob::blob_id_from_object(scope, object).is_some() {
            BitmapSourceKind::Blob
        } else if offscreen_canvas_receiver_branded(scope, object) || is_html_canvas(scope, object)
        {
            BitmapSourceKind::Canvas
        } else if crate::context_bootstrap::image_data::is_image_data_object(scope, object) {
            BitmapSourceKind::ImageData
        } else if image_bitmap_receiver_branded(scope, object) {
            BitmapSourceKind::ImageBitmap
        } else {
            return Err(webidl::WebIdlError::custom_message(
                "The value is not a supported ImageBitmapSource.",
            ));
        };
        Ok(Self { object, kind })
    }
}

fn is_html_canvas<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<'s, v8::Object>) -> bool {
    crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, object)
        .ok()
        .is_some_and(|(host, handle)| {
            unsafe { &*host }
                .dom_host()
                .is_html_element_named(handle, "canvas")
        })
}

enum BitmapInput {
    Blob(Vec<u8>),
    Pixels(moli_image::RgbaImage),
}

impl<'s> BitmapSource<'s> {
    fn snapshot(self, scope: &mut v8::PinScope<'s, '_>) -> Option<BitmapInput> {
        let (pixels, width, height) = match self.kind {
            BitmapSourceKind::Blob => {
                return crate::blob::blob_bytes_from_object(scope, self.object)
                    .map(BitmapInput::Blob);
            }
            BitmapSourceKind::Canvas => canvas_like_pixels_copy(scope, self.object)?,
            BitmapSourceKind::ImageData => {
                let data =
                    crate::context_bootstrap::image_data::image_data_clone_payload_from_object(
                        scope,
                        self.object,
                    )?;
                (data.bytes, data.width, data.height)
            }
            BitmapSourceKind::ImageBitmap => image_bitmap_pixels_copy(scope, self.object)?,
        };
        if width == 0 || height == 0 {
            return None;
        }
        moli_image::RgbaImage::try_new(width, height, pixels)
            .ok()
            .map(BitmapInput::Pixels)
    }
}

fn parse_bitmap_arguments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<(BitmapSource<'s>, BitmapParameters), webidl::WebIdlError> {
    let context = |index| webidl::Context::argument("createImageBitmap", index);
    if args.length() == 0 {
        return Err(webidl::WebIdlError::missing_required(context(1)));
    }
    if matches!(args.length(), 3 | 4) {
        return Err(webidl::WebIdlError::custom_message(
            "The crop overload requires five arguments.",
        ));
    }
    let source = webidl::argument::<BitmapSource>(scope, args, 0, context(1))?;
    let crop = if args.length() >= 5 {
        let mut crop = [0; 4];
        for (index, coordinate) in crop.iter_mut().enumerate() {
            *coordinate = webidl::argument::<webidl::Long>(
                scope,
                args,
                index as i32 + 1,
                context(index + 2),
            )?
            .0;
        }
        Some(crop)
    } else {
        None
    };
    let options_index = if crop.is_some() { 5 } else { 1 };
    let options = webidl::parse_dictionary::<BitmapOptions>(
        scope,
        args.get(options_index),
        context(options_index as usize + 1),
    )?
    .unwrap_or_default();
    Ok((source, BitmapParameters { crop, options }))
}

pub(crate) fn settle_bitmap_task_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    result: Result<BitmapTaskResult, BitmapRejection>,
) {
    let bitmap = result.ok().and_then(|result| {
        let bitmap = build_image_bitmap_object(scope, result.width, result.height)?;
        let pixels =
            super::super::image_data::new_uint8_clamped_array_from_bytes(scope, result.pixels)?;
        set_private_value(scope, bitmap, IMAGE_BITMAP_PIXELS_SLOT, pixels.into());
        set_private_value(
            scope,
            bitmap,
            IMAGE_BITMAP_PREMULTIPLIED_SLOT,
            v8::Boolean::new(scope, result.premultiplied).into(),
        );
        Some(bitmap)
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

pub(super) fn image_bitmap_pixels_copy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bitmap: v8::Local<'s, v8::Object>,
) -> Option<(Vec<u8>, u32, u32)> {
    if !image_bitmap_receiver_branded(scope, bitmap) {
        return None;
    }
    let width = get_private_value(scope, bitmap, IMAGE_BITMAP_WIDTH_SLOT)?.uint32_value(scope)?;
    let height = get_private_value(scope, bitmap, IMAGE_BITMAP_HEIGHT_SLOT)?.uint32_value(scope)?;
    let view = v8::Local::<v8::Uint8ClampedArray>::try_from(get_private_value(
        scope,
        bitmap,
        IMAGE_BITMAP_PIXELS_SLOT,
    )?)
    .ok()?;
    let mut pixels = vec![0; view.byte_length()];
    view.copy_contents(&mut pixels);
    if get_private_value(scope, bitmap, IMAGE_BITMAP_PREMULTIPLIED_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
    {
        for pixel in pixels.chunks_exact_mut(4) {
            let alpha = u32::from(pixel[3]);
            for channel in &mut pixel[..3] {
                *channel = (u32::from(*channel) * 255 + alpha / 2)
                    .checked_div(alpha)
                    .unwrap_or(0)
                    .min(255) as u8;
            }
        }
    }
    Some((pixels, width, height))
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
    if !image_bitmap_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(slot) = callback_data_item(
        scope,
        &args,
        IMAGE_BITMAP_DIMENSION_SLOTS,
        "ImageBitmap dimension slots",
    ) else {
        rv.set_uint32(0);
        return;
    };
    let value = get_private_value(scope, args.this(), slot)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(0.0);
    rv.set_uint32(value.max(0.0) as u32);
}

fn image_bitmap_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !image_bitmap_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    for slot in IMAGE_BITMAP_DIMENSION_SLOTS {
        set_private_value(scope, args.this(), slot, v8::Number::new(scope, 0.0).into());
    }
    set_private_value(
        scope,
        args.this(),
        IMAGE_BITMAP_PIXELS_SLOT,
        v8::undefined(scope).into(),
    );
    rv.set_undefined();
}

pub(super) fn image_bitmap_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::ImageBitmap::is_instance(scope, receiver)
}
