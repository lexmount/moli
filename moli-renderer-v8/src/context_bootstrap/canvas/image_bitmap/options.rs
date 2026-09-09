use super::{BitmapRejection, BitmapTaskResult};
use crate::webidl;
use moli_canvas::{DrawImageBlit, ScaleFilter, blit_draw_image_filtered, byte_len};
use moli_image::RgbaImage;
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
    pub(super) fn apply(self, image: RgbaImage) -> Result<BitmapTaskResult, BitmapRejection> {
        if image.width == 0 || image.height == 0 {
            return Err(BitmapRejection::InvalidState);
        }
        let [mut x, mut y, mut width, mut height] = self
            .crop
            .map(|crop| crop.map(f64::from))
            .unwrap_or([0.0, 0.0, f64::from(image.width), f64::from(image.height)]);
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
        let len = byte_len(output_width, output_height).ok_or(BitmapRejection::InvalidState)?;
        let mut pixels = if x == 0.0
            && y == 0.0
            && width == f64::from(image.width)
            && height == f64::from(image.height)
            && output_width == image.width
            && output_height == image.height
        {
            image.rgba
        } else {
            let mut pixels = vec![0; len];
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
            let filter = if matches!(self.options.resize_quality, ResizeQuality::Pixelated) {
                ScaleFilter::Nearest
            } else {
                ScaleFilter::Bilinear
            };
            blit_draw_image_filtered(
                &mut pixels,
                output_width,
                output_height,
                &image.rgba,
                image.width,
                image.height,
                blit,
                filter,
            );
            pixels
        };
        if matches!(self.options.image_orientation, ImageOrientation::FlipY) {
            let row_len = output_width as usize * 4;
            for y in 0..output_height as usize / 2 {
                let bottom = (output_height as usize - y - 1) * row_len;
                let (top_rows, bottom_rows) = pixels.split_at_mut(bottom);
                top_rows[y * row_len..(y + 1) * row_len]
                    .swap_with_slice(&mut bottom_rows[..row_len]);
            }
        }
        let premultiplied = matches!(
            self.options.premultiply_alpha,
            PremultiplyAlpha::Premultiply
        );
        if premultiplied {
            for pixel in pixels.chunks_exact_mut(4) {
                let alpha = u32::from(pixel[3]);
                for channel in &mut pixel[..3] {
                    *channel = ((u32::from(*channel) * alpha + 127) / 255) as u8;
                }
            }
        }
        Ok(BitmapTaskResult {
            width: output_width,
            height: output_height,
            pixels,
            premultiplied,
        })
    }
}
