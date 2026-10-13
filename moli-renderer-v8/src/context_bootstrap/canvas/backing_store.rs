use super::super::image_data::{ImageDataColorSpace, ImageDataPixelFormat};
use super::{OFFSCREEN_CANVAS_HEIGHT_SLOT, OFFSCREEN_CANVAS_WIDTH_SLOT};
use super::{pixels::CanvasImage, settings::Canvas2dSettings};
use crate::util::{get_private_object, get_private_value, set_private_value};
use crate::webidl;
use crate::{
    document_runtime::DomHandle,
    native_bridge::{JsContextHost, node_runtime_and_handle_from_object_or_detached},
};
use moli_canvas::{AlphaMode, PixelBuffer, encode_data_url};
use moli_webapi_declare::WebApiObject;

const CANVAS_BACKING_STORE_SLOT: &str = "__moliCanvasBackingStore";
pub(super) const CANVAS_OWNER_SLOT: &str = "__moliCanvasOwner";
const CANVAS_HAS_CONTEXT_SLOT: &str = "__moliCanvasHasContext";
const CANVAS_2D_CONTEXT_SLOT: &str = "__moliCanvas2DContext";
const CANVAS_BITMAP_VALID_SLOT: &str = "__moliCanvasBitmapValid";
const CANVAS_BITMAP_WIDTH_SLOT: &str = "__moliCanvasBitmapWidth";
const CANVAS_BITMAP_HEIGHT_SLOT: &str = "__moliCanvasBitmapHeight";
const CANVAS_BITMAP_PREMULTIPLIED_SLOT: &str = "__moliCanvasBitmapPremultiplied";
const CANVAS_BITMAP_OPAQUE_SLOT: &str = "__moliCanvasBitmapOpaque";
const CANVAS_COLOR_SPACE_SLOT: &str = "__moliCanvasColorSpace";
const CANVAS_PIXEL_FORMAT_SLOT: &str = "__moliCanvasPixelFormat";

#[derive(WebApiObject)]
#[webapi(plain)]
struct CanvasContextOwnerDeclaration<'scope> {
    #[webapi(data_property)]
    canvas: v8::Local<'scope, v8::Object>,
}

pub(crate) fn attach_canvas_like_context_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Object>,
) {
    if canvas_owner_from_context(scope, context).is_none() {
        super::webgl::initialize_attached_webgl_viewport(scope, canvas, context);
    }
    let _ = CanvasContextOwnerDeclaration::new(canvas).initialize(scope, context);
    set_private_value(scope, context, CANVAS_OWNER_SLOT, canvas.into());
    let is_2d = get_private_value(scope, context, super::CANVAS_CONTEXT_FILL_STYLE_SLOT).is_some();
    if is_2d {
        set_private_value(scope, canvas, CANVAS_2D_CONTEXT_SLOT, context.into());
        set_private_value(
            scope,
            canvas,
            CANVAS_BITMAP_PREMULTIPLIED_SLOT,
            v8::Boolean::new(scope, true).into(),
        );
        let settings = Canvas2dSettings::for_context(scope, context);
        set_canvas_pixel_settings(scope, canvas, settings.color_space, settings.pixel_format());
        set_private_value(
            scope,
            canvas,
            CANVAS_BITMAP_OPAQUE_SLOT,
            v8::Boolean::new(scope, !settings.alpha).into(),
        );
    }
    set_private_value(
        scope,
        canvas,
        CANVAS_HAS_CONTEXT_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    if is_2d {
        // Acquiring the first 2D context initializes its configured bitmap,
        // including opaque alpha, even if dimensions allocated a blank surface.
        reset_canvas_like_backing_store(scope, canvas);
    } else {
        let _ = ensure_canvas_like_backing_store(scope, canvas);
    }
}

pub(crate) fn reset_canvas_like_backing_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) {
    // HTML width/height attributes resize only a blank bitmaprenderer output.
    // A transferred bitmap retains its own natural dimensions and pixels.
    if has_valid_bitmap_output(scope, canvas) && html_canvas_identity(scope, canvas).is_some() {
        return;
    }
    set_private_value(
        scope,
        canvas,
        CANVAS_BITMAP_VALID_SLOT,
        v8::Boolean::new(scope, false).into(),
    );
    let context = canvas_2d_context(scope, canvas);
    set_private_value(
        scope,
        canvas,
        CANVAS_BITMAP_PREMULTIPLIED_SLOT,
        v8::Boolean::new(scope, context.is_some()).into(),
    );
    if let Some(context) = context {
        super::context2d::reset_canvas_context_state(scope, context);
    }
    let Some((width, height)) = canvas_like_dimensions(scope, canvas) else {
        remove_html_canvas_pixels(scope, canvas);
        return;
    };
    let Some(image) = blank_canvas_image(scope, canvas, width, height) else {
        remove_html_canvas_pixels(scope, canvas);
        return;
    };
    let pixels = image.clone().into_rgba8();
    let Some(bytes) = image.into_view(scope) else {
        remove_html_canvas_pixels(scope, canvas);
        return;
    };
    set_private_value(scope, canvas, CANVAS_BACKING_STORE_SLOT, bytes.into());
    if let Some((pixels, _, _)) = pixels {
        replace_html_canvas_pixels(scope, canvas, width, height, pixels);
    }
}

pub(super) fn initialize_canvas_bitmap_renderer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
    alpha: bool,
) {
    set_private_value(
        scope,
        canvas,
        CANVAS_BITMAP_OPAQUE_SLOT,
        v8::Boolean::new(scope, !alpha).into(),
    );
    set_private_value(
        scope,
        canvas,
        CANVAS_HAS_CONTEXT_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    clear_canvas_bitmap_output(scope, canvas);
}

pub(super) fn clear_canvas_bitmap_output<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) {
    set_canvas_pixel_settings(
        scope,
        canvas,
        ImageDataColorSpace::Srgb,
        ImageDataPixelFormat::RgbaUnorm8,
    );
    set_private_value(
        scope,
        canvas,
        CANVAS_BITMAP_VALID_SLOT,
        v8::Boolean::new(scope, false).into(),
    );
    reset_canvas_like_backing_store(scope, canvas);
}

pub(super) fn set_canvas_bitmap_output<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
    data: super::image_bitmap::BitmapData<'s>,
) {
    set_canvas_pixel_settings(scope, canvas, data.color_space, data.pixel_format);
    set_private_value(scope, canvas, CANVAS_BACKING_STORE_SLOT, data.pixels.into());
    set_private_value(
        scope,
        canvas,
        CANVAS_BITMAP_WIDTH_SLOT,
        v8::Integer::new_from_unsigned(scope, data.width).into(),
    );
    set_private_value(
        scope,
        canvas,
        CANVAS_BITMAP_HEIGHT_SLOT,
        v8::Integer::new_from_unsigned(scope, data.height).into(),
    );
    set_private_value(
        scope,
        canvas,
        CANVAS_BITMAP_PREMULTIPLIED_SLOT,
        v8::Boolean::new(scope, data.premultiplied).into(),
    );
    set_private_value(
        scope,
        canvas,
        CANVAS_BITMAP_VALID_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    if html_canvas_identity(scope, canvas).is_some()
        && let Some((pixels, width, height)) = canvas_like_pixels_copy(scope, canvas)
    {
        replace_html_canvas_pixels(scope, canvas, width, height, pixels);
    }
}

pub(super) fn transfer_canvas_bitmap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let (pixels, width, height) = canvas_like_pixel_view(scope, canvas)?;
    let replacement = blank_canvas_image(scope, canvas, width, height)?.into_view(scope)?;
    let (color_space, pixel_format) = canvas_pixel_settings(scope, canvas);
    let premultiplied = get_private_value(scope, canvas, CANVAS_BITMAP_PREMULTIPLIED_SLOT)
        .is_some_and(|value| value.boolean_value(scope));
    let bitmap = super::image_bitmap::build_image_bitmap_from_data(
        scope,
        super::image_bitmap::BitmapData {
            pixels,
            width,
            height,
            premultiplied,
            color_space,
            pixel_format,
        },
    )?;
    // Both allocations succeeded: move the old view without resetting drawing
    // state or the natural dimensions of a bitmaprenderer output.
    set_private_value(scope, canvas, CANVAS_BACKING_STORE_SLOT, replacement.into());
    set_private_value(
        scope,
        canvas,
        CANVAS_BITMAP_PREMULTIPLIED_SLOT,
        v8::Boolean::new(scope, premultiplied).into(),
    );
    Some(bitmap)
}

fn has_valid_bitmap_output<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, canvas, CANVAS_BITMAP_VALID_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

fn blank_canvas_image<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
    width: u32,
    height: u32,
) -> Option<CanvasImage> {
    let (color_space, pixel_format) = canvas_pixel_settings(scope, canvas);
    let mut pixels = PixelBuffer::blank_with_alpha_mode(
        width,
        height,
        pixel_format.native(),
        canvas_alpha_mode(scope, canvas),
    )?;
    if get_private_value(scope, canvas, CANVAS_BITMAP_OPAQUE_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
    {
        pixels.force_opaque();
    }
    Some(CanvasImage {
        pixels,
        color_space,
    })
}

pub(super) fn canvas_2d_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, canvas, CANVAS_2D_CONTEXT_SLOT)
}

pub(crate) fn reset_html_canvas_backing_store_for_dimension_assignment<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    namespace: Option<&str>,
    local_name: &str,
) {
    if namespace.is_some()
        || !unsafe { &*runtime_ptr }
            .dom_host()
            .is_html_element_named(handle, "canvas")
        || (!local_name.eq_ignore_ascii_case("width") && !local_name.eq_ignore_ascii_case("height"))
    {
        return;
    }
    let Some(canvas) = crate::util::node_wrapper_from_handle(scope, handle) else {
        let _ = unsafe { &mut *runtime_ptr }.remove_canvas_pixels(handle);
        return;
    };
    if get_private_value(scope, canvas, CANVAS_BACKING_STORE_SLOT).is_none() {
        let _ = unsafe { &mut *runtime_ptr }.remove_canvas_pixels(handle);
        return;
    }
    reset_canvas_like_backing_store(scope, canvas);
}

pub(crate) fn canvas_like_to_data_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let (bytes, width, height) = canvas_like_pixels_copy(scope, canvas)?;
    encode_data_url(&bytes, width, height)
}

pub(super) fn canvas_owner_from_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, context, CANVAS_OWNER_SLOT)
}

pub(super) fn with_canvas_image_mut<'s, F>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
    mutate: F,
) -> bool
where
    F: FnOnce(&mut CanvasImage),
{
    let Some((view, width, height)) = canvas_like_pixel_view(scope, canvas) else {
        return false;
    };
    let (space, format) = canvas_pixel_settings(scope, canvas);
    let alpha_mode = canvas_alpha_mode(scope, canvas);
    let Some(mut image) = CanvasImage::from_view(view, width, height, format, space, alpha_mode)
    else {
        return false;
    };
    mutate(&mut image);
    if get_private_value(scope, canvas, CANVAS_BITMAP_OPAQUE_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
    {
        image.pixels.force_opaque();
    }
    if write_bytes_to_view(scope, view, image.pixels.bytes()).is_none() {
        return false;
    }
    if html_canvas_identity(scope, canvas).is_some()
        && let Some((bytes, _, _)) = image.into_rgba8()
    {
        replace_html_canvas_pixels(scope, canvas, width, height, bytes);
    }
    true
}

pub(super) fn canvas_like_pixels_copy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<(Vec<u8>, u32, u32)> {
    canvas_image_snapshot(scope, canvas)?.into_rgba8()
}

pub(super) fn canvas_image_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<CanvasImage> {
    let (view, width, height) = canvas_like_pixel_view(scope, canvas)?;
    let (color_space, format) = canvas_pixel_settings(scope, canvas);
    let alpha_mode = canvas_alpha_mode(scope, canvas);
    let mut image = CanvasImage::from_view(view, width, height, format, color_space, alpha_mode)?;
    let opaque = get_private_value(scope, canvas, CANVAS_BITMAP_OPAQUE_SLOT)
        .is_some_and(|value| value.boolean_value(scope));
    if opaque && canvas_2d_context(scope, canvas).is_none() {
        image.pixels.premultiply();
        image.pixels.force_opaque();
    }
    Some(image)
}

fn canvas_alpha_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> AlphaMode {
    if get_private_value(scope, canvas, CANVAS_BITMAP_PREMULTIPLIED_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
    {
        AlphaMode::Premultiplied
    } else {
        AlphaMode::Straight
    }
}

fn canvas_like_pixel_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::ArrayBufferView>, u32, u32)> {
    let (width, height) = canvas_like_dimensions(scope, canvas)?;
    let view = ensure_canvas_like_backing_store(scope, canvas)?;
    Some((view, width, height))
}

fn ensure_canvas_like_backing_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::ArrayBufferView>> {
    let (width, height) = canvas_like_dimensions(scope, canvas)?;
    let (_, format) = canvas_pixel_settings(scope, canvas);
    let expected_len = format.byte_len(width, height)?;
    if let Some(existing) = get_private_value(scope, canvas, CANVAS_BACKING_STORE_SLOT)
        .and_then(|value| v8::Local::<v8::ArrayBufferView>::try_from(value).ok())
        && existing.byte_length() == expected_len
        && existing.is_float16_array() == (format == ImageDataPixelFormat::RgbaFloat16)
    {
        return Some(existing);
    }
    let image = blank_canvas_image(scope, canvas, width, height)?;
    let mirror = html_canvas_identity(scope, canvas)
        .is_some()
        .then(|| image.clone().into_rgba8())
        .flatten();
    let bytes = image.into_view(scope)?;
    set_private_value(scope, canvas, CANVAS_BACKING_STORE_SLOT, bytes.into());
    if let Some((pixels, _, _)) = mirror {
        replace_html_canvas_pixels(scope, canvas, width, height, pixels);
    }
    Some(bytes)
}

fn set_canvas_pixel_settings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
    color_space: ImageDataColorSpace,
    format: ImageDataPixelFormat,
) {
    let space = crate::util::v8str(scope, color_space.label());
    set_private_value(scope, canvas, CANVAS_COLOR_SPACE_SLOT, space.into());
    let format = crate::util::v8str(scope, format.label());
    set_private_value(scope, canvas, CANVAS_PIXEL_FORMAT_SLOT, format.into());
}

fn canvas_pixel_settings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> (ImageDataColorSpace, ImageDataPixelFormat) {
    let color_space = get_private_value(scope, canvas, CANVAS_COLOR_SPACE_SLOT)
        .and_then(|value| value.to_string(scope))
        .and_then(|value| value.to_rust_string_lossy(scope).parse().ok())
        .unwrap_or(ImageDataColorSpace::Srgb);
    let format = get_private_value(scope, canvas, CANVAS_PIXEL_FORMAT_SLOT)
        .and_then(|value| value.to_string(scope))
        .and_then(|value| value.to_rust_string_lossy(scope).parse().ok())
        .unwrap_or_default();
    (color_space, format)
}

fn html_canvas_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let (runtime_ptr, handle) =
        node_runtime_and_handle_from_object_or_detached(scope, canvas).ok()?;
    unsafe { &*runtime_ptr }
        .dom_host()
        .is_html_element_named(handle, "canvas")
        .then_some((runtime_ptr, handle))
}

fn replace_html_canvas_pixels<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
) {
    let Some((runtime_ptr, handle)) = html_canvas_identity(scope, canvas) else {
        return;
    };
    let _ = unsafe { &mut *runtime_ptr }.replace_canvas_pixels(handle, width, height, rgba);
}

fn remove_html_canvas_pixels<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) {
    let Some((runtime_ptr, handle)) = html_canvas_identity(scope, canvas) else {
        return;
    };
    let _ = unsafe { &mut *runtime_ptr }.remove_canvas_pixels(handle);
}

pub(super) fn canvas_like_dimensions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<(u32, u32)> {
    if has_valid_bitmap_output(scope, canvas) {
        let width =
            get_private_value(scope, canvas, CANVAS_BITMAP_WIDTH_SLOT)?.uint32_value(scope)?;
        let height =
            get_private_value(scope, canvas, CANVAS_BITMAP_HEIGHT_SLOT)?.uint32_value(scope)?;
        return Some((width, height));
    }
    if html_canvas_identity(scope, canvas).is_some() {
        let width =
            crate::native_bridge::element::canvas_dimension_value(scope, canvas, "width", 300);
        let height =
            crate::native_bridge::element::canvas_dimension_value(scope, canvas, "height", 150);
        return Some((width, height));
    }
    let width = canvas_like_dimension(scope, canvas, OFFSCREEN_CANVAS_WIDTH_SLOT, "width")?;
    let height = canvas_like_dimension(scope, canvas, OFFSCREEN_CANVAS_HEIGHT_SLOT, "height")?;
    Some((width, height))
}

fn canvas_like_dimension<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
    slot: &str,
    public_name: &'static str,
) -> Option<u32> {
    let value = get_private_value(scope, canvas, slot)
        .and_then(|value| value.number_value(scope))
        .or_else(|| webidl::optional_number_property(scope, canvas, public_name))
        .unwrap_or(0.0);
    Some(value.max(0.0).trunc() as u32)
}

fn write_bytes_to_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    view: v8::Local<'s, v8::ArrayBufferView>,
    bytes: &[u8],
) -> Option<()> {
    if view.byte_length() != bytes.len() {
        return None;
    }
    let backing_store = view.buffer(scope)?;
    let data = backing_store.data()?;
    let ptr = data.as_ptr() as *mut u8;
    let byte_offset = view.byte_offset();
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.add(byte_offset), bytes.len());
    }
    Some(())
}
