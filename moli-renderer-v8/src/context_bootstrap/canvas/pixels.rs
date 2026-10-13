//! Native image snapshots shared by Canvas, ImageData and ImageBitmap tasks.

use crate::context_bootstrap::image_data::{
    ImageDataColorSpace, ImageDataPixelFormat, convert_pixel_bytes, new_pixel_array,
};
use moli_canvas::{AlphaMode, PixelBuffer, PixelFormat};

#[derive(Clone, Debug)]
pub(in crate::context_bootstrap) struct CanvasImage {
    pub(in crate::context_bootstrap) pixels: PixelBuffer,
    pub(in crate::context_bootstrap) color_space: ImageDataColorSpace,
}

impl CanvasImage {
    pub(in crate::context_bootstrap) fn rgba8(
        width: u32,
        height: u32,
        bytes: Vec<u8>,
    ) -> Option<Self> {
        Some(Self {
            pixels: PixelBuffer::new(width, height, PixelFormat::Unorm8, bytes)?,
            color_space: ImageDataColorSpace::Srgb,
        })
    }

    pub(in crate::context_bootstrap) fn from_view(
        view: v8::Local<'_, v8::ArrayBufferView>,
        width: u32,
        height: u32,
        format: ImageDataPixelFormat,
        color_space: ImageDataColorSpace,
        alpha_mode: AlphaMode,
    ) -> Option<Self> {
        let mut bytes = vec![0; view.byte_length()];
        let written = view.copy_contents(&mut bytes);
        bytes.truncate(written);
        Some(Self {
            pixels: PixelBuffer::new_with_alpha_mode(
                width,
                height,
                format.native(),
                alpha_mode,
                bytes,
            )?,
            color_space,
        })
    }

    pub(in crate::context_bootstrap) fn convert(
        self,
        color_space: ImageDataColorSpace,
        format: ImageDataPixelFormat,
        alpha_mode: AlphaMode,
    ) -> Option<Self> {
        let width = self.pixels.width();
        let height = self.pixels.height();
        let source_format = ImageDataPixelFormat::from_native(self.pixels.format());
        let source_alpha = self.pixels.alpha_mode();
        let bytes = convert_pixel_bytes(
            self.pixels.into_bytes(),
            source_format,
            self.color_space,
            format,
            color_space,
            source_alpha,
            alpha_mode,
        );
        Some(Self {
            pixels: PixelBuffer::new_with_alpha_mode(
                width,
                height,
                format.native(),
                alpha_mode,
                bytes,
            )?,
            color_space,
        })
    }

    pub(in crate::context_bootstrap) fn into_view<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::ArrayBufferView>> {
        let format = ImageDataPixelFormat::from_native(self.pixels.format());
        new_pixel_array(scope, self.pixels.into_bytes(), format)
    }

    pub(in crate::context_bootstrap) fn into_rgba8(self) -> Option<(Vec<u8>, u32, u32)> {
        let image = self.convert(
            ImageDataColorSpace::Srgb,
            ImageDataPixelFormat::RgbaUnorm8,
            AlphaMode::Straight,
        )?;
        let width = image.pixels.width();
        let height = image.pixels.height();
        Some((image.pixels.into_bytes(), width, height))
    }
}
