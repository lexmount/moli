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
const CANVAS_OWNER_SLOT: &str = "__moliCanvasOwner";
const CANVAS_HAS_CONTEXT_SLOT: &str = "__moliCanvasHasContext";
const CANVAS_2D_CONTEXT_SLOT: &str = "__moliCanvas2DContext";

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct CanvasContextOwnerDeclaration<'scope> {
    #[webapi(data_property)]
    canvas: v8::Local<'scope, v8::Object>,
}

pub(crate) fn attach_canvas_like_context_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Object>,
) {
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

pub(super) fn canvas_like_has_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, canvas, CANVAS_HAS_CONTEXT_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

pub(crate) fn reset_canvas_like_backing_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) {
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
    let Some(bytes) = new_uint8_clamped_array_from_bytes(scope, vec![0; len]) else {
        remove_html_canvas_pixels(scope, canvas);
        return;
    };
    set_private_value(scope, canvas, CANVAS_BACKING_STORE_SLOT, bytes.into());
    replace_html_canvas_pixels(scope, canvas, width, height, vec![0; len]);
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
    let bytes = new_uint8_clamped_array_from_bytes(scope, vec![0; expected_len])?;
    set_private_value(scope, canvas, CANVAS_BACKING_STORE_SLOT, bytes.into());
    replace_html_canvas_pixels(scope, canvas, width, height, vec![0; expected_len]);
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

fn canvas_like_dimensions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<(u32, u32)> {
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
