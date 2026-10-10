//! Shared CanvasImageSource identity and native pixel snapshots.
use super::{backing_store::canvas_like_pixels_copy, image_bitmap::image_bitmap_pixels_copy};
use crate::{context_bootstrap::throw_dom_exception_value, web_api_interfaces};

pub(in crate::context_bootstrap) struct CanvasImageSource;

impl CanvasImageSource {
    pub(in crate::context_bootstrap) const NAME: &'static str = "CanvasImageSource";

    pub(in crate::context_bootstrap) fn is_instance<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        source: v8::Local<'s, v8::Object>,
    ) -> bool {
        [
            web_api_interfaces::HTMLImageElement::DESCRIPTOR,
            web_api_interfaces::SVGImageElement::DESCRIPTOR,
            web_api_interfaces::HTMLVideoElement::DESCRIPTOR,
            web_api_interfaces::HTMLCanvasElement::DESCRIPTOR,
            web_api_interfaces::OffscreenCanvas::DESCRIPTOR,
            web_api_interfaces::ImageBitmap::DESCRIPTOR,
            web_api_interfaces::VideoFrame::DESCRIPTOR,
        ]
        .iter()
        .any(|interface| interface.is_instance(scope, source))
    }
}

pub(in crate::context_bootstrap) fn image_source_pixels<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> Option<(Vec<u8>, u32, u32)> {
    if web_api_interfaces::VideoFrame::is_instance(scope, source) {
        return super::super::video_frame::rendered_pixels(scope, source);
    }
    if web_api_interfaces::ImageBitmap::is_instance(scope, source) {
        let pixels = image_bitmap_pixels_copy(scope, source);
        if pixels.is_none() {
            throw_dom_exception_value(scope, "The ImageBitmap is detached.", "InvalidStateError");
        }
        return pixels;
    }
    if web_api_interfaces::HTMLImageElement::is_instance(scope, source) {
        let (host, handle) =
            crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, source)
                .ok()?;
        match unsafe { &*host }.raster_image_for_canvas(handle) {
            Ok(pixels) => pixels.map(|pixels| (pixels.rgba.clone(), pixels.width, pixels.height)),
            Err(_) => {
                throw_dom_exception_value(
                    scope,
                    "The source image could not be decoded.",
                    "InvalidStateError",
                );
                None
            }
        }
    } else if web_api_interfaces::HTMLCanvasElement::is_instance(scope, source)
        || web_api_interfaces::OffscreenCanvas::is_instance(scope, source)
    {
        canvas_like_pixels_copy(scope, source)
    } else {
        // Video elements and SVG image sources still need producers.
        None
    }
}
