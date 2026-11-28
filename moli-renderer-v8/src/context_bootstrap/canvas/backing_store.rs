use std::{cell::Cell, ptr::NonNull};

use super::super::image_data::new_uint8_clamped_array_from_bytes;
use super::{OFFSCREEN_CANVAS_HEIGHT_SLOT, OFFSCREEN_CANVAS_WIDTH_SLOT};
use crate::util::{get_private_object, get_private_value, set_private_value};
use crate::webidl;
use crate::{
    document_runtime::DomHandle,
    native_bridge::{PageScriptHost, node_runtime_and_handle_from_object_or_detached},
};
use moli_canvas::{byte_len as canvas_byte_len, encode_data_url};
use moli_webapi_declare::WebApiObject;

use moli_webapi::canvas_context_values::CANVAS_BACKING_STORE_SLOT;
const CANVAS_ORIGIN_CLEAN_SLOT: &str = "__moliCanvasOriginClean";
const CANVAS_OWNER_SLOT: &str = "__moliCanvasOwner";
const CANVAS_HAS_CONTEXT_SLOT: &str = "__moliCanvasHasContext";
const CANVAS_2D_CONTEXT_SLOT: &str = "__moliCanvas2DContext";

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
    if canvas_owner_from_context(scope, context).is_none()
        && (moli_webapi::interfaces::WebGLRenderingContext::is_instance(scope, context)
            || moli_webapi::interfaces::WebGL2RenderingContext::is_instance(scope, context))
    {
        let dimensions = canvas_like_dimensions(scope, canvas);
        super::webgl::initialize_viewport(scope, dimensions, context);
    }
    let _ = CanvasContextOwnerDeclaration::new(canvas).initialize(scope, context);
    set_private_value(scope, context, CANVAS_OWNER_SLOT, canvas.into());
    if super::context2d::is_canvas_context_receiver(scope, context) {
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

pub(super) fn canvas_like_origin_clean<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, canvas, CANVAS_ORIGIN_CLEAN_SLOT)
        .is_none_or(|value| value.boolean_value(scope))
}

pub(super) fn set_canvas_like_origin_clean<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
    clean: bool,
) {
    let value = v8::Boolean::new(scope, clean);
    set_private_value(scope, canvas, CANVAS_ORIGIN_CLEAN_SLOT, value.into());
}

pub(crate) fn reset_canvas_like_backing_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) {
    set_canvas_like_origin_clean(scope, canvas, true);
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
    if let Some(storage) = CanvasPixels::from_view(scope, bytes, width, height) {
        replace_html_canvas_pixels(scope, canvas, storage);
    }
}

pub(super) fn canvas_2d_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, canvas, CANVAS_2D_CONTEXT_SLOT)
}

pub(crate) fn reset_html_canvas_backing_store_for_dimension_assignment<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
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
    if !canvas_like_origin_clean(scope, canvas) {
        webidl::throw_dom_exception(scope, "SecurityError", "Canvas is not origin-clean.");
        return None;
    }
    let (bytes, width, height) = canvas_like_pixels_copy(scope, canvas)?;
    encode_data_url(&bytes, width, height)
}

pub(crate) fn canvas_like_to_blob<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    if !canvas_like_origin_clean(scope, canvas) {
        webidl::throw_dom_exception(scope, "SecurityError", "Canvas is not origin-clean.");
        return None;
    }
    let blob = canvas_like_pixels_copy(scope, canvas)
        .filter(|(_, width, height)| *width != 0 && *height != 0)
        .and_then(|(bytes, width, height)| moli_image::encode_png_rgba8(width, height, &bytes).ok())
        .and_then(|png| super::build_blob_object(scope, png.bytes, "image/png".to_owned()));
    Some(
        blob.map(Into::into)
            .unwrap_or_else(|| v8::null(scope).into()),
    )
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
    let Some(storage) = canvas_like_pixels(scope, canvas) else {
        return false;
    };
    // The backing store is private to the canvas and belongs to this isolate.
    // The pixel operation runs on the owner thread without calling JavaScript.
    let pixels = unsafe { std::slice::from_raw_parts_mut(storage.data().as_ptr(), storage.len()) };
    mutate(pixels, storage.width, storage.height);
    replace_html_canvas_pixels(scope, canvas, storage);
    true
}

/// Private, fixed-size canvas storage. Only immutable snapshots leave the
/// renderer thread. Multiple views in this thread may own the same backing.
#[derive(Clone)]
pub(crate) struct CanvasPixels {
    backing: v8::SharedRef<v8::BackingStore>,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl CanvasPixels {
    fn from_view<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        view: v8::Local<'s, v8::Uint8ClampedArray>,
        width: u32,
        height: u32,
    ) -> Option<Self> {
        let backing = view.buffer(scope)?.get_backing_store();
        if view.byte_offset() != 0
            || view.byte_length() != canvas_byte_len(width, height)?
            || view.byte_length() != backing.byte_length()
            || backing.is_shared()
            || backing.is_resizable_by_user_javascript()
        {
            return None;
        }
        Some(Self {
            backing,
            width,
            height,
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.backing.byte_length()
    }

    fn data(&self) -> NonNull<u8> {
        self.backing
            .data()
            .map(NonNull::cast)
            .unwrap_or_else(NonNull::dangling)
    }

    pub(crate) fn same_storage(&self, other: &Self) -> bool {
        std::ptr::eq(&*self.backing, &*other.backing)
            && self.width == other.width
            && self.height == other.height
    }

    pub(crate) fn pixels_copy(&self) -> Vec<u8> {
        self.backing.iter().map(Cell::get).collect()
    }

    /// The caller must keep the isolate and the source allocation quiescent
    /// until the returned slice is dropped.
    pub(super) unsafe fn draw_source<'a>(
        &'a self,
        destination: &[u8],
    ) -> std::borrow::Cow<'a, [u8]> {
        if self.data().as_ptr().cast_const() == destination.as_ptr() {
            // A self draw must finish sampling the old pixels before writing.
            std::borrow::Cow::Owned(destination.to_vec())
        } else {
            // Canvas backings are disjoint allocations. The Canvas drawing
            // callback performs no script calls while it holds this borrow.
            let bytes = unsafe { std::slice::from_raw_parts(self.data().as_ptr(), self.len()) };
            std::borrow::Cow::Borrowed(bytes)
        }
    }

    #[cfg(test)]
    pub(crate) fn from_owned(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        assert_eq!(Some(rgba.len()), canvas_byte_len(width, height));
        Self {
            backing: v8::ArrayBuffer::new_backing_store_from_vec(rgba).make_shared(),
            width,
            height,
        }
    }
}

pub(super) fn canvas_like_pixels_copy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<(Vec<u8>, u32, u32)> {
    let storage = canvas_like_pixels(scope, canvas)?;
    Some((storage.pixels_copy(), storage.width, storage.height))
}

pub(super) fn canvas_like_pixels<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<CanvasPixels> {
    let (width, height) = canvas_like_dimensions(scope, canvas)?;
    let view = ensure_canvas_like_backing_store(scope, canvas)?;
    CanvasPixels::from_view(scope, view, width, height)
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
    if let Some(storage) = CanvasPixels::from_view(scope, bytes, width, height) {
        replace_html_canvas_pixels(scope, canvas, storage);
    }
    Some(bytes)
}

fn html_canvas_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<(*mut PageScriptHost, DomHandle)> {
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
    storage: CanvasPixels,
) {
    let Some((runtime_ptr, handle)) = html_canvas_identity(scope, canvas) else {
        return;
    };
    let _ = unsafe { &mut *runtime_ptr }.replace_canvas_pixels(handle, storage);
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
