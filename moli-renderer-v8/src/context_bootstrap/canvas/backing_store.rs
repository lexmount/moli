use super::super::image_data::new_uint8_clamped_array_from_bytes;
use super::{OFFSCREEN_CANVAS_HEIGHT_SLOT, OFFSCREEN_CANVAS_WIDTH_SLOT};
use crate::util::{get_private_object, get_private_value, set_private_value};
use crate::webidl;
use crate::{
    document_runtime::DomHandle,
    native_bridge::{JsContextHost, node_runtime_and_handle_from_object_or_detached},
};
use moli_canvas::{byte_len as canvas_byte_len, encode_data_url};
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
    if get_private_value(scope, context, super::CANVAS_CONTEXT_FILL_STYLE_SLOT).is_some() {
        set_private_value(scope, canvas, CANVAS_2D_CONTEXT_SLOT, context.into());
    }
    set_private_value(
        scope,
        canvas,
        CANVAS_HAS_CONTEXT_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    let _ = ensure_canvas_like_backing_store(scope, canvas);
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
    set_private_value(
        scope,
        canvas,
        CANVAS_BITMAP_PREMULTIPLIED_SLOT,
        v8::Boolean::new(scope, false).into(),
    );
    if let Some(context) = canvas_2d_context(scope, canvas) {
        super::context2d::reset_canvas_context_state(scope, context);
    }
    let Some((width, height)) = canvas_like_dimensions(scope, canvas) else {
        remove_html_canvas_pixels(scope, canvas);
        return;
    };
    let Some(len) = canvas_byte_len(width, height) else {
        remove_html_canvas_pixels(scope, canvas);
        return;
    };
    let pixels = blank_canvas_pixels(scope, canvas, len);
    let Some(bytes) = new_uint8_clamped_array_from_bytes(scope, pixels.clone()) else {
        remove_html_canvas_pixels(scope, canvas);
        return;
    };
    set_private_value(scope, canvas, CANVAS_BACKING_STORE_SLOT, bytes.into());
    replace_html_canvas_pixels(scope, canvas, width, height, pixels);
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

fn has_valid_bitmap_output<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, canvas, CANVAS_BITMAP_VALID_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

fn blank_canvas_pixels<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
    len: usize,
) -> Vec<u8> {
    let mut pixels = vec![0; len];
    if get_private_value(scope, canvas, CANVAS_BITMAP_OPAQUE_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
    {
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
    }
    pixels
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

pub(super) fn with_canvas_like_pixels_mut<'s, F>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
    mutate: F,
) -> bool
where
    F: FnOnce(&mut [u8], u32, u32),
{
    let Some((view, width, height)) = canvas_like_pixel_view(scope, canvas) else {
        return false;
    };
    let mut bytes = vec![0; view.byte_length()];
    let written = view.copy_contents(&mut bytes);
    bytes.truncate(written);
    mutate(&mut bytes, width, height);
    if write_bytes_to_view(scope, view, &bytes).is_none() {
        return false;
    }
    replace_html_canvas_pixels(scope, canvas, width, height, bytes);
    true
}

pub(super) fn canvas_like_pixels_copy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<(Vec<u8>, u32, u32)> {
    let (view, width, height) = canvas_like_pixel_view(scope, canvas)?;
    let mut bytes = vec![0; view.byte_length()];
    let written = view.copy_contents(&mut bytes);
    bytes.truncate(written);
    let premultiplied = get_private_value(scope, canvas, CANVAS_BITMAP_PREMULTIPLIED_SLOT)
        .is_some_and(|value| value.boolean_value(scope));
    let opaque = get_private_value(scope, canvas, CANVAS_BITMAP_OPAQUE_SLOT)
        .is_some_and(|value| value.boolean_value(scope));
    if opaque {
        if !premultiplied {
            moli_canvas::premultiply_rgba8_in_place(&mut bytes)?;
        }
        for pixel in bytes.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
    } else {
        super::image_bitmap::unpremultiply_bitmap_pixels(&mut bytes, premultiplied);
    }
    Some((bytes, width, height))
}

fn canvas_like_pixel_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Uint8ClampedArray>, u32, u32)> {
    let (width, height) = canvas_like_dimensions(scope, canvas)?;
    let view = ensure_canvas_like_backing_store(scope, canvas)?;
    Some((view, width, height))
}

fn ensure_canvas_like_backing_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Uint8ClampedArray>> {
    let (width, height) = canvas_like_dimensions(scope, canvas)?;
    let expected_len = canvas_byte_len(width, height)?;
    if let Some(existing) = get_private_value(scope, canvas, CANVAS_BACKING_STORE_SLOT)
        .and_then(|value| v8::Local::<v8::Uint8ClampedArray>::try_from(value).ok())
        && existing.byte_length() == expected_len
    {
        return Some(existing);
    }
    let pixels = blank_canvas_pixels(scope, canvas, expected_len);
    let bytes = new_uint8_clamped_array_from_bytes(scope, pixels.clone())?;
    set_private_value(scope, canvas, CANVAS_BACKING_STORE_SLOT, bytes.into());
    replace_html_canvas_pixels(scope, canvas, width, height, pixels);
    Some(bytes)
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
    view: v8::Local<'s, v8::Uint8ClampedArray>,
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
