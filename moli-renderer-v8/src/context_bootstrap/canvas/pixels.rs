//! Native image snapshots shared by Canvas, ImageData and ImageBitmap tasks.

use crate::context_bootstrap::image_data::{
    ImageDataColorSpace, ImageDataPixelFormat, convert_pixel_bytes, new_pixel_array,
};
use moli_canvas::{AlphaMode, PixelBuffer, PixelFormat};

#[derive(Clone, Debug)]
pub(in crate::context_bootstrap) struct CanvasImage {
    pub(in crate::context_bootstrap) pixels: PixelBuffer,
    pub(in crate::context_bootstrap) color_space: ImageDataColorSpace,
    pub(in crate::context_bootstrap) origin_clean: bool,
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
            origin_clean: true,
        })
    }

    pub(super) fn from_decoded_raster(decoded: moli_image::DecodedRasterImage) -> Option<Self> {
        if let Some(managed) = decoded.color_managed {
            Self::from_color_managed(&managed, moli_image::ColorProfileConversion::Apply)
        } else {
            Self::rgba8(
                decoded.image.width,
                decoded.image.height,
                decoded.image.rgba,
            )
        }
    }

    pub(super) fn from_color_managed(
        image: &moli_image::ColorManagedRasterImage,
        conversion: moli_image::ColorProfileConversion,
    ) -> Option<Self> {
        if matches!(conversion, moli_image::ColorProfileConversion::Ignore)
            && let Some(moli_image::UnconvertedRasterPixels::Unorm8(rgba)) = &image.unconverted
        {
            return Self::rgba8(image.width, image.height, rgba.clone());
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(moli_canvas::byte_len(image.width, image.height)?.checked_mul(2)?)
            .ok()?;
        if matches!(conversion, moli_image::ColorProfileConversion::Ignore)
            && let Some(moli_image::UnconvertedRasterPixels::Unorm16(rgba)) = &image.unconverted
        {
            for value in rgba {
                bytes.extend_from_slice(
                    &half::f16::from_f32(f32::from(*value) / 65535.0).to_ne_bytes(),
                );
            }
        } else {
            for value in &image.rgba {
                bytes.extend_from_slice(&half::f16::from_f32(*value).to_ne_bytes());
            }
        }
        Some(Self {
            pixels: PixelBuffer::new(image.width, image.height, PixelFormat::Float16, bytes)?,
            color_space: ImageDataColorSpace::Srgb,
            origin_clean: true,
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
            origin_clean: true,
        })
    }

    pub(in crate::context_bootstrap) fn with_origin_clean(mut self, origin_clean: bool) -> Self {
        self.origin_clean = origin_clean;
        self
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
            origin_clean: self.origin_clean,
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
