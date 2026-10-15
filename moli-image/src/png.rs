use std::io::Cursor;

use crate::{
    MAX_DECODED_RGBA_BYTES, MAX_ENCODED_IMAGE_BYTES, RgbaImage, RgbaImageError,
    bounded::BoundedBytes, rgba::checked_rgba_len,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PngEncodeOptions {
    /// Prefer lower CPU cost over smaller encoded output.
    pub optimize_for_speed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedPng {
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum PngEncodeError {
    #[error(transparent)]
    InvalidInput(#[from] RgbaImageError),
    #[error("PNG output contains {actual} bytes, exceeding the {max}-byte encoded image budget")]
    EncodedBufferBudgetExceeded { actual: usize, max: usize },
    #[error("PNG encoding failed: {0}")]
    Encoding(#[from] png::EncodingError),
}

#[derive(Debug, thiserror::Error)]
pub enum PngDecodeError {
    #[error("PNG decoding failed: {0}")]
    Decoding(#[from] png::DecodingError),
    #[error(transparent)]
    InvalidOutput(#[from] RgbaImageError),
    #[error(transparent)]
    Color(#[from] crate::ColorDecodeError),
    #[error("PNG decoder produced unsupported color type {color_type:?}")]
    UnsupportedColorType { color_type: png::ColorType },
}

pub fn encode_png(image: &RgbaImage) -> Result<EncodedPng, PngEncodeError> {
    encode_png_rgba8_with_options(
        image.width,
        image.height,
        &image.rgba,
        PngEncodeOptions::default(),
    )
}

pub fn encode_png_with_options(
    image: &RgbaImage,
    options: PngEncodeOptions,
) -> Result<EncodedPng, PngEncodeError> {
    encode_png_rgba8_with_options(image.width, image.height, &image.rgba, options)
}

/// Encodes a borrowed RGBA8 surface without first copying it into an owned
/// [`RgbaImage`].
pub fn encode_png_rgba8(
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<EncodedPng, PngEncodeError> {
    encode_png_rgba8_with_options(width, height, rgba, PngEncodeOptions::default())
}

pub fn encode_png_rgba8_with_options(
    width: u32,
    height: u32,
    rgba: &[u8],
    options: PngEncodeOptions,
) -> Result<EncodedPng, PngEncodeError> {
    let expected = checked_rgba_len(width, height)?;
    if rgba.len() != expected {
        return Err(PngEncodeError::InvalidInput(
            RgbaImageError::InvalidBufferLength {
                expected,
                actual: rgba.len(),
            },
        ));
    }
    let mut bytes = BoundedBytes::new(MAX_ENCODED_IMAGE_BYTES);
    let result = (|| {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        if options.optimize_for_speed {
            encoder.set_compression(png::Compression::Fast);
            encoder.set_filter(png::Filter::NoFilter);
        }
        let mut writer = encoder.write_header()?;
        writer.write_image_data(rgba)?;
        writer.finish()?;
        Ok::<_, png::EncodingError>(())
    })();
    if bytes.limit_exceeded() {
        return Err(PngEncodeError::EncodedBufferBudgetExceeded {
            actual: bytes.rejected_len().unwrap_or(usize::MAX),
            max: MAX_ENCODED_IMAGE_BYTES,
        });
    }
    result?;
    let bytes = bytes.into_inner();
    Ok(EncodedPng {
        width,
        height,
        bytes,
    })
}

pub fn decode_png(bytes: &[u8]) -> Result<RgbaImage, PngDecodeError> {
    decode_png_colors(bytes, true).map(|(image, _)| image)
}

pub(crate) fn decode_png_colors(
    bytes: &[u8],
    convert_profile: bool,
) -> Result<crate::color::DecodedColorPixels, PngDecodeError> {
    use crate::color::{ColorSamples, RawColorImage};
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(bytes),
        png::Limits {
            bytes: MAX_DECODED_RGBA_BYTES,
        },
    );
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info()?;
    let (width, height) = (reader.info().width, reader.info().height);
    let orientation = crate::orientation::from_exif(reader.info().exif_metadata.as_deref());
    checked_rgba_len(width, height)?;
    let cicp = cicp_profile(reader.info());
    let expand_gray = cicp.is_some();
    let profile = cicp.or_else(|| color_profile(reader.info()));
    let size = reader
        .output_buffer_size()
        .ok_or(RgbaImageError::BufferLengthOverflow { width, height })?;
    let mut decoded = vec![0; size];
    let output = reader.next_frame(&mut decoded)?;
    decoded.truncate(output.buffer_size());
    let mut layout = match output.color_type {
        png::ColorType::Rgb => moxcms::Layout::Rgb,
        png::ColorType::Rgba => moxcms::Layout::Rgba,
        png::ColorType::Grayscale => moxcms::Layout::Gray,
        png::ColorType::GrayscaleAlpha => moxcms::Layout::GrayAlpha,
        color_type @ png::ColorType::Indexed => {
            return Err(PngDecodeError::UnsupportedColorType { color_type });
        }
    };
    let mut samples = if output.bit_depth == png::BitDepth::Sixteen {
        ColorSamples::U16(
            decoded
                .as_chunks::<2>()
                .0
                .iter()
                .map(|value| u16::from_be_bytes(*value))
                .collect(),
        )
    } else {
        ColorSamples::U8(decoded)
    };
    if expand_gray && matches!(layout, moxcms::Layout::Gray | moxcms::Layout::GrayAlpha) {
        // cICP describes RGB colorimetry even when PNG stores neutral gray.
        let channels = if layout == moxcms::Layout::GrayAlpha {
            2
        } else {
            1
        };
        samples = match samples {
            ColorSamples::U8(values) => ColorSamples::U8(
                values
                    .chunks_exact(channels)
                    .flat_map(|p| [p[0], p[0], p[0], if channels == 2 { p[1] } else { 255 }])
                    .collect(),
            ),
            ColorSamples::U16(values) => ColorSamples::U16(
                values
                    .chunks_exact(channels)
                    .flat_map(|p| [p[0], p[0], p[0], if channels == 2 { p[1] } else { 65535 }])
                    .collect(),
            ),
        };
        layout = moxcms::Layout::Rgba;
    }
    Ok(RawColorImage {
        width,
        height,
        samples,
        layout,
        profile,
        cmyk: false,
    }
    .finish(convert_profile, orientation)?)
}

pub(crate) fn png_metadata(
    bytes: &[u8],
) -> Result<(u32, u32, crate::RasterColorPrecision), PngDecodeError> {
    let decoder = png::Decoder::new_with_limits(
        Cursor::new(bytes),
        png::Limits {
            bytes: MAX_DECODED_RGBA_BYTES,
        },
    );
    let reader = decoder.read_info()?;
    let info = reader.info();
    let (width, height) = crate::orientation::dimensions(
        info.width,
        info.height,
        crate::orientation::from_exif(info.exif_metadata.as_deref()),
    );
    checked_rgba_len(info.width, info.height)?;
    let high_precision = info.bit_depth == png::BitDepth::Sixteen
        || info.icc_profile.is_some()
        || info.coding_independent_code_points.is_some()
        || (info.srgb.is_none() && (info.gama_chunk.is_some() || info.chrm_chunk.is_some()));
    Ok((
        width,
        height,
        if high_precision {
            crate::RasterColorPrecision::Float32
        } else {
            crate::RasterColorPrecision::Unorm8
        },
    ))
}

fn cicp_profile(info: &png::Info<'_>) -> Option<moxcms::ColorProfile> {
    if let Some(cicp) = info.coding_independent_code_points
        && cicp.matrix_coefficients == 0
        && cicp.is_video_full_range_image
        && let (Ok(color_primaries), Ok(transfer_characteristics), Ok(matrix_coefficients)) = (
            moxcms::CicpColorPrimaries::try_from(cicp.color_primaries),
            moxcms::TransferCharacteristics::try_from(cicp.transfer_function),
            moxcms::MatrixCoefficients::try_from(cicp.matrix_coefficients),
        )
        && moxcms::ColorPrimaries::try_from(color_primaries).is_ok()
        && moxcms::ToneReprCurve::try_from(transfer_characteristics).is_ok()
    {
        return Some(moxcms::ColorProfile::new_from_cicp(moxcms::CicpProfile {
            color_primaries,
            transfer_characteristics,
            matrix_coefficients,
            full_range: true,
        }));
    }
    None
}

fn color_profile(info: &png::Info<'_>) -> Option<moxcms::ColorProfile> {
    use moxcms::{Chromaticity, ColorPrimaries, ColorProfile, ToneReprCurve, XyY};
    if let Some(profile) = info
        .icc_profile
        .as_deref()
        .and_then(|bytes| ColorProfile::new_from_slice(bytes).ok())
    {
        return Some(profile);
    }
    if info.srgb.is_some() || (info.gama_chunk.is_none() && info.chrm_chunk.is_none()) {
        return None;
    }
    let gray = matches!(
        info.color_type,
        png::ColorType::Grayscale | png::ColorType::GrayscaleAlpha
    );
    let mut profile = if gray {
        let mut profile = ColorProfile::new_gray_with_gamma(1.0);
        profile.gray_trc = ColorProfile::new_srgb().red_trc;
        profile
    } else {
        ColorProfile::new_srgb()
    };
    profile.cicp = None;
    if let Some(gamma) = info
        .gama_chunk
        .map(png::ScaledFloat::into_value)
        .filter(|gamma| *gamma > 0.0)
    {
        let curve = ToneReprCurve::Parametric(vec![1.0 / gamma]);
        if gray {
            profile.gray_trc = Some(curve);
        } else {
            profile.red_trc = Some(curve.clone());
            profile.green_trc = Some(curve.clone());
            profile.blue_trc = Some(curve);
        }
    }
    if !gray && let Some(chroma) = info.chrm_chunk {
        let point = |(x, y): (png::ScaledFloat, png::ScaledFloat)| {
            Chromaticity::new(x.into_value(), y.into_value())
        };
        let white = point(chroma.white);
        if white.y > 0.0
            && [chroma.red, chroma.green, chroma.blue]
                .iter()
                .all(|(_, y)| y.into_value() > 0.0)
        {
            let white = XyY::new(f64::from(white.x), f64::from(white.y), 1.0);
            let primaries = ColorPrimaries {
                red: point(chroma.red),
                green: point(chroma.green),
                blue: point(chroma.blue),
            };
            if ColorProfile::colorants_matrix(white, primaries)
                .determinant()
                .is_some_and(|value| value.is_finite())
            {
                profile.update_rgb_colorimetry(white, primaries);
            }
        }
    }
    Some(profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> RgbaImage {
        RgbaImage::try_new(
            2,
            2,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 128, 255, 255, 255, 255,
            ],
        )
        .unwrap()
    }

    #[test]
    fn default_and_fast_png_round_trip_exact_rgba() {
        for options in [
            PngEncodeOptions::default(),
            PngEncodeOptions {
                optimize_for_speed: true,
            },
        ] {
            let encoded = encode_png_with_options(&sample(), options).unwrap();
            assert_eq!(&encoded.bytes[..8], b"\x89PNG\r\n\x1a\n");
            assert_eq!(decode_png(&encoded.bytes).unwrap(), sample());
        }
    }

    #[test]
    fn encode_rejects_inconsistent_rgba_metadata() {
        let image = RgbaImage {
            width: 2,
            height: 2,
            rgba: vec![0; 15],
        };
        assert!(matches!(
            encode_png(&image),
            Err(PngEncodeError::InvalidInput(
                RgbaImageError::InvalidBufferLength { .. }
            ))
        ));
    }
}
