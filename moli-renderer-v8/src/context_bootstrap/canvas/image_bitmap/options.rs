use super::super::pixels::CanvasImage;
use super::{BitmapRejection, BitmapTaskResult};
use crate::webidl;
use moli_canvas::{DrawImageBlit, PixelBuffer, ScaleFilter};
use std::str::FromStr;

#[derive(Clone, Copy, Default, strum::EnumString, webidl::WebIdlEnum)]
#[webidl(name = "ImageOrientation", parse_with = Self::parse)]
enum ImageOrientation {
    #[default]
    #[strum(serialize = "from-image")]
    FromImage,
    #[strum(serialize = "flipY")]
    FlipY,
}

impl ImageOrientation {
    fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }
}

#[derive(Clone, Copy, Default, strum::EnumString, webidl::WebIdlEnum)]
#[webidl(name = "PremultiplyAlpha", parse_with = Self::parse)]
#[strum(serialize_all = "lowercase")]
enum PremultiplyAlpha {
    None,
    Premultiply,
    #[default]
    Default,
}

impl PremultiplyAlpha {
    fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }
}

#[derive(Clone, Copy, Default, strum::EnumString, webidl::WebIdlEnum)]
#[webidl(name = "ColorSpaceConversion", parse_with = Self::parse)]
#[strum(serialize_all = "lowercase")]
enum ColorSpaceConversion {
    None,
    #[default]
    Default,
}

impl ColorSpaceConversion {
    fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }
}

#[derive(Clone, Copy, Default, strum::EnumString, webidl::WebIdlEnum)]
#[webidl(name = "ResizeQuality", parse_with = Self::parse)]
#[strum(serialize_all = "lowercase")]
enum ResizeQuality {
    Pixelated,
    #[default]
    Low,
    Medium,
    High,
}

impl ResizeQuality {
    fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "ImageBitmapOptions")]
pub(super) struct BitmapOptions {
    // The raster backend leaves color profiles unconverted in both modes.
    // Members are declared in Web IDL's observable lexicographic read order.
    #[webidl(name = "colorSpaceConversion", converter = "enum", default = ColorSpaceConversion::Default)]
    _color_space_conversion: ColorSpaceConversion,
    #[webidl(converter = "enum", default = ImageOrientation::FromImage)]
    image_orientation: ImageOrientation,
    #[webidl(converter = "enum", default = PremultiplyAlpha::Default)]
    premultiply_alpha: PremultiplyAlpha,
    #[webidl(converter = "enforce_range_unsigned_long")]
    pub(super) resize_height: Option<u32>,
    #[webidl(converter = "enum", default = ResizeQuality::Low)]
    resize_quality: ResizeQuality,
    #[webidl(converter = "enforce_range_unsigned_long")]
    pub(super) resize_width: Option<u32>,
}

pub(super) struct BitmapParameters {
    pub(super) crop: Option<[i32; 4]>,
    pub(super) options: BitmapOptions,
}

impl BitmapParameters {
    pub(super) fn apply(self, image: CanvasImage) -> Result<BitmapTaskResult, BitmapRejection> {
        let image_width = image.pixels.width();
        let image_height = image.pixels.height();
        if image_width == 0 || image_height == 0 {
            return Err(BitmapRejection::InvalidState);
        }
        let [mut x, mut y, mut width, mut height] = self
            .crop
            .map(|crop| crop.map(f64::from))
            .unwrap_or([0.0, 0.0, f64::from(image_width), f64::from(image_height)]);
        // Negative extents select the rectangle on the other side of the
        // origin; they do not mirror its pixels.
        if width < 0.0 {
            x += width;
            width = -width;
        }
        if height < 0.0 {
            y += height;
            height = -height;
        }
        let (output_width, output_height) =
            match (self.options.resize_width, self.options.resize_height) {
                (Some(w), Some(h)) => (w, h),
                (Some(w), None) => (w, (height * f64::from(w) / width).ceil() as u32),
                (None, Some(h)) => ((width * f64::from(h) / height).ceil() as u32, h),
                (None, None) => (width as u32, height as u32),
            };
        if output_width == 0 || output_height == 0 {
            return Err(BitmapRejection::InvalidState);
        }
        let mut pixels = if x == 0.0
            && y == 0.0
            && width == f64::from(image_width)
            && height == f64::from(image_height)
            && output_width == image_width
            && output_height == image_height
        {
            image.pixels
        } else {
            let mut pixels = PixelBuffer::blank_with_alpha_mode(
                output_width,
                output_height,
                image.pixels.format(),
                image.pixels.alpha_mode(),
            )
            .ok_or(BitmapRejection::InvalidState)?;
            let blit = DrawImageBlit::new(
                x,
                y,
                width,
                height,
                0.0,
                0.0,
                f64::from(output_width),
                f64::from(output_height),
            )
            .ok_or(BitmapRejection::InvalidState)?;
            let filter = if (f64::from(output_width) == width && f64::from(output_height) == height)
                || matches!(self.options.resize_quality, ResizeQuality::Pixelated)
            {
                ScaleFilter::Nearest
            } else {
                ScaleFilter::Bilinear
            };
            pixels.blit(&image.pixels, blit, filter);
            pixels
        };
        if matches!(self.options.image_orientation, ImageOrientation::FlipY) {
            pixels.flip_y();
        }
        match self.options.premultiply_alpha {
            PremultiplyAlpha::None => pixels.unpremultiply(),
            PremultiplyAlpha::Premultiply | PremultiplyAlpha::Default => pixels.premultiply(),
        }
        Ok(BitmapTaskResult {
            image: CanvasImage {
                pixels,
                color_space: image.color_space,
                origin_clean: image.origin_clean,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_bootstrap::image_data::ImageDataColorSpace;
    use moli_canvas::{AlphaMode, PixelFormat};

    #[test]
    fn unscaled_crops_preserve_native_half_bits_and_unpremultiplied_transparent_rgb() {
        let bytes = [0x0672u16, 0xbc00, 0x7e21, 0x2e66, 0x4000, 0xbc00, 0x3800, 0]
            .into_iter()
            .flat_map(u16::to_ne_bytes)
            .collect::<Vec<_>>();
        for alpha_mode in [AlphaMode::Straight, AlphaMode::Premultiplied] {
            for row in 0..2 {
                let image = CanvasImage {
                    pixels: PixelBuffer::new_with_alpha_mode(
                        1,
                        2,
                        PixelFormat::Float16,
                        alpha_mode,
                        bytes.clone(),
                    )
                    .unwrap(),
                    color_space: ImageDataColorSpace::Srgb,
                    origin_clean: true,
                };
                let result = BitmapParameters {
                    crop: Some([0, row, 1, 1]),
                    options: BitmapOptions {
                        premultiply_alpha: if alpha_mode == AlphaMode::Straight {
                            PremultiplyAlpha::None
                        } else {
                            PremultiplyAlpha::Default
                        },
                        ..Default::default()
                    },
                }
                .apply(image)
                .unwrap();
                assert_eq!(result.image.pixels.alpha_mode(), alpha_mode);
                let start = row as usize * 8;
                assert_eq!(result.image.pixels.bytes(), &bytes[start..start + 8]);
            }
        }
    }
}
