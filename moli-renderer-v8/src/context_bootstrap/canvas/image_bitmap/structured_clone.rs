use super::{BitmapData, build_image_bitmap_from_data, image_bitmap_data, image_bitmap_snapshot};
use crate::context_bootstrap::canvas::pixels::CanvasImage;
use crate::context_bootstrap::image_data::{ImageDataColorSpace, ImageDataPixelFormat};
use moli_canvas::{AlphaMode, PixelBuffer, PixelFormat};

/// An owned bitmap snapshot; no V8 handles or sender realm survive serialization.
#[derive(Clone, Debug)]
pub(crate) struct ImageBitmapClonePayload(CanvasImage);

impl ImageBitmapClonePayload {
    pub(crate) fn write(&self, serializer: &dyn v8::ValueSerializerHelper) {
        let image = &self.0;
        serializer.write_uint32(image.pixels.width());
        serializer.write_uint32(image.pixels.height());
        serializer.write_uint32(match image.pixels.alpha_mode() {
            AlphaMode::Straight => 0,
            AlphaMode::Premultiplied => 1,
        });
        serializer.write_uint32(match image.color_space {
            ImageDataColorSpace::Srgb => 0,
            ImageDataColorSpace::DisplayP3 => 1,
        });
        serializer.write_uint32(match image.pixels.format() {
            PixelFormat::Unorm8 => 0,
            PixelFormat::Float16 => 1,
        });
        let bytes = image.pixels.bytes();
        serializer.write_uint32(bytes.len() as u32);
        serializer.write_raw_bytes(bytes);
    }

    pub(crate) fn read(deserializer: &dyn v8::ValueDeserializerHelper) -> Option<Self> {
        let read = || {
            let mut value = 0;
            deserializer.read_uint32(&mut value).then_some(value)
        };
        let width = read()?;
        let height = read()?;
        if width == 0 || height == 0 {
            return None;
        }
        let alpha_mode = match read()? {
            0 => AlphaMode::Straight,
            1 => AlphaMode::Premultiplied,
            _ => return None,
        };
        let color_space = match read()? {
            0 => ImageDataColorSpace::Srgb,
            1 => ImageDataColorSpace::DisplayP3,
            _ => return None,
        };
        let format = match read()? {
            0 => PixelFormat::Unorm8,
            1 => PixelFormat::Float16,
            _ => return None,
        };
        let length = read()? as usize;
        if format.byte_len(width, height)? != length {
            return None;
        }
        let bytes = deserializer.read_raw_bytes(length)?.to_vec();
        Some(Self(CanvasImage {
            pixels: PixelBuffer::new_with_alpha_mode(width, height, format, alpha_mode, bytes)?,
            color_space,
            origin_clean: true,
        }))
    }
}

pub(crate) fn image_bitmap_clone_payload_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bitmap: v8::Local<'s, v8::Object>,
) -> Option<ImageBitmapClonePayload> {
    if !image_bitmap_data(scope, bitmap)?.origin_clean {
        return None;
    }
    let image = image_bitmap_snapshot(scope, bitmap)?;
    Some(ImageBitmapClonePayload(image))
}

pub(crate) fn build_image_bitmap_from_clone_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: ImageBitmapClonePayload,
) -> Option<v8::Local<'s, v8::Object>> {
    let image = payload.0;
    let width = image.pixels.width();
    let height = image.pixels.height();
    let premultiplied = image.pixels.alpha_mode() == AlphaMode::Premultiplied;
    let color_space = image.color_space;
    let pixel_format = ImageDataPixelFormat::from_native(image.pixels.format());
    let pixels = image.into_view(scope)?;
    build_image_bitmap_from_data(
        scope,
        BitmapData {
            pixels,
            width,
            height,
            premultiplied,
            color_space,
            pixel_format,
            origin_clean: true,
        },
    )
}
