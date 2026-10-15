//! Shared CanvasImageSource identity and native pixel snapshots.
use super::svg_source::SvgSource;
use super::{
    backing_store::canvas_image_snapshot, image_bitmap::image_bitmap_snapshot, pixels::CanvasImage,
};
use crate::native_bridge::DecodedImageContent;
use crate::{context_bootstrap::throw_dom_exception_value, web_api_interfaces};

pub(super) enum SourceSnapshot {
    Pixels(CanvasImage),
    Svg(SvgSource),
}

impl SourceSnapshot {
    pub(super) fn origin_clean(&self) -> bool {
        match self {
            Self::Pixels(image) => image.origin_clean,
            Self::Svg(image) => image.origin_clean,
        }
    }
}

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
    let image = image_source_image(scope, source)?;
    if !image.origin_clean {
        throw_dom_exception_value(
            scope,
            "The image source is not origin-clean.",
            "SecurityError",
        );
        return None;
    }
    image.into_rgba8()
}

pub(super) fn image_source_image<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> Option<CanvasImage> {
    image_source_image_with_conversion(scope, source, moli_image::ColorProfileConversion::Apply)
}

pub(super) fn image_source_image_with_conversion<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    conversion: moli_image::ColorProfileConversion,
) -> Option<CanvasImage> {
    match image_source_snapshot(scope, source, conversion)? {
        SourceSnapshot::Pixels(image) => Some(image),
        SourceSnapshot::Svg(image) => match image.rasterize(300.0, 150.0) {
            Ok(image) => Some(image),
            Err(error) => {
                crate::util::throw_range_error(scope, &error);
                None
            }
        },
    }
}

pub(super) fn image_source_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    conversion: moli_image::ColorProfileConversion,
) -> Option<SourceSnapshot> {
    if web_api_interfaces::VideoFrame::is_instance(scope, source) {
        let (bytes, width, height) = super::super::video_frame::rendered_pixels(scope, source)?;
        return CanvasImage::rgba8(width, height, bytes).map(SourceSnapshot::Pixels);
    }
    if web_api_interfaces::ImageBitmap::is_instance(scope, source) {
        let pixels = image_bitmap_snapshot(scope, source);
        if pixels.is_none() {
            throw_dom_exception_value(scope, "The ImageBitmap is detached.", "InvalidStateError");
        }
        return pixels.map(SourceSnapshot::Pixels);
    }
    if web_api_interfaces::HTMLVideoElement::is_instance(scope, source) {
        let (host, handle) =
            crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, source)
                .ok()?;
        let (frame, origin_clean) = unsafe { &*host }.media_video_frame_for_canvas(handle)?;
        let (bytes, width, height) =
            super::super::video_frame::render_decoded_media_frame(scope, &frame, conversion)?;
        return CanvasImage::rgba8(width, height, bytes)
            .map(|image| SourceSnapshot::Pixels(image.with_origin_clean(origin_clean)));
    }
    if web_api_interfaces::HTMLImageElement::is_instance(scope, source)
        || web_api_interfaces::SVGImageElement::is_instance(scope, source)
    {
        let (host, handle) =
            crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, source)
                .ok()?;
        let host = unsafe { &*host };
        match host.image_content_for_canvas(handle) {
            Ok(Some(DecodedImageContent::Raster(pixels))) => {
                let image = if let Some(managed) = pixels.color_managed {
                    CanvasImage::from_color_managed(&managed, conversion)
                } else {
                    CanvasImage::rgba8(
                        pixels.srgb.width,
                        pixels.srgb.height,
                        pixels.srgb.rgba.clone(),
                    )
                }?;
                Some(SourceSnapshot::Pixels(image.with_origin_clean(
                    host.image_resource_origin_clean(handle)?,
                )))
            }
            Ok(Some(DecodedImageContent::Svg(image))) => Some(SourceSnapshot::Svg(SvgSource {
                image,
                origin_clean: host.image_resource_origin_clean(handle)?,
            })),
            Ok(None) => None,
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
        if super::offscreen::offscreen_canvas_is_detached(scope, source) {
            throw_dom_exception_value(
                scope,
                "The OffscreenCanvas is detached.",
                "InvalidStateError",
            );
            return None;
        }
        canvas_image_snapshot(scope, source).map(SourceSnapshot::Pixels)
    } else {
        None
    }
}
