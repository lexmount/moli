use super::ImageDataColorSpace;
use crate::webidl;
use half::f16;
use std::str::FromStr;
use style::color::{AbsoluteColor, ColorSpace};

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    strum::EnumString,
    strum::IntoStaticStr,
    webidl::WebIdlEnum,
)]
#[webidl(name = "ImageDataPixelFormat", parse_with = Self::parse)]
pub(crate) enum ImageDataPixelFormat {
    #[default]
    #[strum(serialize = "rgba-unorm8")]
    RgbaUnorm8,
    #[strum(serialize = "rgba-float16")]
    RgbaFloat16,
}

impl ImageDataPixelFormat {
    fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }

    pub(crate) fn label(self) -> &'static str {
        self.into()
    }

    pub(super) fn bytes_per_pixel(self) -> usize {
        match self {
            Self::RgbaUnorm8 => 4,
            Self::RgbaFloat16 => 8,
        }
    }

    pub(in crate::context_bootstrap) fn byte_len(self, width: u32, height: u32) -> Option<usize> {
        (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(self.bytes_per_pixel())
            .filter(|length| *length <= moli_canvas::MAX_RGBA8_BYTE_LENGTH)
    }
}

/// The ImageDataArray union retains the view through later argument conversion.
pub(super) struct ImageDataArray<'s> {
    pub(super) view: v8::Local<'s, v8::ArrayBufferView>,
    pub(super) format: ImageDataPixelFormat,
}

impl ImageDataArray<'_> {
    pub(super) fn matches(value: v8::Local<'_, v8::Value>) -> bool {
        Self::format(value).is_some()
    }

    pub(super) fn format(value: v8::Local<'_, v8::Value>) -> Option<ImageDataPixelFormat> {
        if value.is_uint8_clamped_array() {
            Some(ImageDataPixelFormat::RgbaUnorm8)
        } else if value.is_float16_array() {
            Some(ImageDataPixelFormat::RgbaFloat16)
        } else {
            None
        }
    }
}

impl<'s> webidl::WebIdlConverter<'s> for ImageDataArray<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        let invalid = || webidl::WebIdlError::cannot_convert(context, "ImageDataArray");
        let format = Self::format(value).ok_or_else(invalid)?;
        let view = v8::Local::<v8::ArrayBufferView>::try_from(value).map_err(|_| invalid())?;
        let buffer = view.buffer(scope).ok_or_else(invalid)?;
        // Use native buffer flags, which survive detachment. No author getters
        // or Proxy traps participate in typed-array union conversion.
        if buffer.is_shared_array_buffer() || buffer.is_resizable_by_user_javascript() {
            return Err(webidl::WebIdlError::cannot_convert(
                context,
                "non-shared, non-resizable ImageDataArray",
            ));
        }
        Ok(Self { view, format })
    }
}

pub(super) fn new_pixel_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
    format: ImageDataPixelFormat,
) -> Option<v8::Local<'s, v8::ArrayBufferView>> {
    match format {
        ImageDataPixelFormat::RgbaUnorm8 => {
            super::new_uint8_clamped_array_from_bytes(scope, bytes).map(Into::into)
        }
        ImageDataPixelFormat::RgbaFloat16 => {
            let length = bytes.len() / 2;
            let backing = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
            let buffer = v8::ArrayBuffer::with_backing_store(scope, &backing);
            v8::Float16Array::new(scope, buffer, 0, length).map(|view| {
                // SAFETY: Float16Array inherits ArrayBufferView in V8. rusty_v8
                // exposes construction/downcasts but omits this mechanical upcast.
                unsafe { v8::Local::<v8::ArrayBufferView>::cast_unchecked(view) }
            })
        }
    }
}

pub(super) fn rgba8_bytes(
    view: v8::Local<'_, v8::ArrayBufferView>,
    color_space: ImageDataColorSpace,
) -> Option<Vec<u8>> {
    let format = ImageDataArray::format(view.into())?;
    let mut bytes = vec![0; view.byte_length()];
    let written = view.copy_contents(&mut bytes);
    bytes.truncate(written);
    Some(convert_pixel_bytes(
        bytes,
        format,
        color_space,
        ImageDataPixelFormat::RgbaUnorm8,
        ImageDataColorSpace::Srgb,
    ))
}

fn to_unorm8(value: f32) -> u8 {
    // Casting also maps NaN to zero.
    (value * 255.0).round_ties_even().clamp(0.0, 255.0) as u8
}

fn color_space(space: ImageDataColorSpace) -> ColorSpace {
    match space {
        ImageDataColorSpace::Srgb => ColorSpace::Srgb,
        ImageDataColorSpace::DisplayP3 => ColorSpace::DisplayP3,
    }
}

/// Relative-colorimetric conversion preserves out-of-gamut channels until the
/// destination representation requires clipping. Alpha is not color-managed.
pub(super) fn convert_pixel_bytes(
    bytes: Vec<u8>,
    source_format: ImageDataPixelFormat,
    source_space: ImageDataColorSpace,
    destination_format: ImageDataPixelFormat,
    destination_space: ImageDataColorSpace,
) -> Vec<u8> {
    if source_format == destination_format && source_space == destination_space {
        return bytes;
    }
    let pixels = bytes.len() / source_format.bytes_per_pixel();
    let mut output = Vec::with_capacity(pixels * destination_format.bytes_per_pixel());
    for pixel in bytes.chunks_exact(source_format.bytes_per_pixel()) {
        let mut channels: [f32; 4] = std::array::from_fn(|index| match source_format {
            ImageDataPixelFormat::RgbaUnorm8 => f32::from(pixel[index]) / 255.0,
            ImageDataPixelFormat::RgbaFloat16 => {
                f16::from_ne_bytes([pixel[index * 2], pixel[index * 2 + 1]]).to_f32()
            }
        });
        if source_space != destination_space {
            let converted = AbsoluteColor::new(
                color_space(source_space),
                channels[0],
                channels[1],
                channels[2],
                1.0,
            )
            .to_color_space(color_space(destination_space));
            channels[..3].copy_from_slice(&[
                converted.components.0,
                converted.components.1,
                converted.components.2,
            ]);
        }
        match destination_format {
            ImageDataPixelFormat::RgbaUnorm8 => output.extend(channels.map(to_unorm8)),
            ImageDataPixelFormat::RgbaFloat16 => output.extend(
                channels
                    .into_iter()
                    .flat_map(|value| f16::from_f32(value).to_ne_bytes()),
            ),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{f16, to_unorm8};

    #[test]
    fn half_pixels_quantize_only_at_the_rgba8_backend_boundary() {
        for (bits, expected) in [
            (0x0000, 0),
            (0x8000, 0),
            (0x0001, 0),
            (0x03ff, 0),
            (0xbc00, 0),
            (0x3400, 64),
            (0x3800, 128),
            (0x3a00, 191),
            (0x3c00, 255),
            (0x4000, 255),
            (0x7bff, 255),
            (0x7c00, 255),
            (0xfc00, 0),
            (0x7e00, 0),
            (0xfe00, 0),
        ] {
            assert_eq!(
                to_unorm8(f16::from_bits(bits).to_f32()),
                expected,
                "half bits {bits:04x}"
            );
        }
    }
}
