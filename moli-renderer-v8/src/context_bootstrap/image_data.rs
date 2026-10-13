use super::canvas;
pub(crate) use moli_webapi::image_data::*;

pub(super) fn build_image_data_object_with_pixels<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mut image: canvas::pixels::CanvasImage,
) -> Option<v8::Local<'s, v8::Object>> {
    image.pixels.unpremultiply();
    let width = image.pixels.width();
    let height = image.pixels.height();
    let color_space = image.color_space;
    let pixel_format = ImageDataPixelFormat::from_native(image.pixels.format());
    let data = image.into_view(scope)?;
    build_image_data_object_from_clone_payload(
        scope,
        ImageDataClonePayload {
            width,
            height,
            color_space: color_space.label().to_owned(),
            pixel_format,
            data,
        },
    )
}

pub(super) fn image_data_pixel_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<canvas::pixels::CanvasImage> {
    let data = image_data_clone_payload_from_object(scope, object)?;
    canvas::pixels::CanvasImage::from_view(
        data.data,
        data.width,
        data.height,
        data.pixel_format,
        ImageDataColorSpace::parse(&data.color_space)?,
        moli_canvas::AlphaMode::Straight,
    )
}
