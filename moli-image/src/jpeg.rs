use std::io::Cursor;

use crate::{
    MAX_DECODED_RGBA_BYTES, MAX_ENCODED_IMAGE_BYTES, RgbaImage, RgbaImageError,
    bounded::BoundedBytes,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedJpeg {
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum JpegEncodeError {
    #[error(transparent)]
    InvalidInput(#[from] RgbaImageError),
    #[error("JPEG dimensions {width}x{height} exceed encoder limit {max_dimension}")]
    DimensionLimit {
        width: u32,
        height: u32,
        max_dimension: u32,
    },
    #[error("JPEG output contains {actual} bytes, exceeding the {max}-byte encoded image budget")]
    EncodedBufferBudgetExceeded { actual: usize, max: usize },
    #[error("JPEG encoding failed: {0}")]
    Encoding(#[from] jpeg_encoder::EncodingError),
}

#[derive(Debug, thiserror::Error)]
pub enum JpegDecodeError {
    #[error("JPEG decoding failed: {0}")]
    Decoding(#[from] jpeg_decoder::Error),
    #[error("JPEG decoder returned no image metadata")]
    MissingImageInfo,
    #[error("JPEG decoder produced unsupported pixel format {format}")]
    UnsupportedPixelFormat { format: &'static str },
    #[error(transparent)]
    InvalidOutput(#[from] RgbaImageError),
    #[error(transparent)]
    Color(#[from] crate::ColorDecodeError),
}

pub fn encode_jpeg(image: &RgbaImage, quality: u8) -> Result<EncodedJpeg, JpegEncodeError> {
    image.validate()?;
    let width = u16::try_from(image.width).map_err(|_| JpegEncodeError::DimensionLimit {
        width: image.width,
        height: image.height,
        max_dimension: u32::from(u16::MAX),
    })?;
    let height = u16::try_from(image.height).map_err(|_| JpegEncodeError::DimensionLimit {
        width: image.width,
        height: image.height,
        max_dimension: u32::from(u16::MAX),
    })?;
    let mut bytes = BoundedBytes::new(MAX_ENCODED_IMAGE_BYTES);
    let result = jpeg_encoder::Encoder::new(&mut bytes, quality).encode(
        &image.rgba,
        width,
        height,
        jpeg_encoder::ColorType::Rgba,
    );
    if bytes.limit_exceeded() {
        return Err(JpegEncodeError::EncodedBufferBudgetExceeded {
            actual: bytes.rejected_len().unwrap_or(usize::MAX),
            max: MAX_ENCODED_IMAGE_BYTES,
        });
    }
    result?;
    let bytes = bytes.into_inner();
    Ok(EncodedJpeg {
        width: image.width,
        height: image.height,
        bytes,
    })
}

pub fn decode_jpeg(bytes: &[u8]) -> Result<RgbaImage, JpegDecodeError> {
    decode_jpeg_colors(bytes, true).map(|(image, _)| image)
}

pub(crate) fn decode_jpeg_colors(
    bytes: &[u8],
    convert_profile: bool,
) -> Result<crate::color::DecodedColorPixels, JpegDecodeError> {
    use crate::color::{ColorSamples, RawColorImage};
    let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    decoder.set_max_decoding_buffer_size(MAX_DECODED_RGBA_BYTES);
    let decoded = decoder.decode()?;
    let info = decoder.info().ok_or(JpegDecodeError::MissingImageInfo)?;
    // Use the same admission condition as the metadata probe. The decoder
    // collects APP2 markers even after scan data; a late-only profile must not
    // materialize color planes that the resource owner could not reserve.
    let profile = if has_icc_header(bytes) {
        decoder
            .icc_profile()
            .and_then(|bytes| moxcms::ColorProfile::new_from_slice(&bytes).ok())
    } else {
        None
    };
    let (layout, samples, cmyk) = match info.pixel_format {
        jpeg_decoder::PixelFormat::L8 => (moxcms::Layout::Gray, ColorSamples::U8(decoded), false),
        jpeg_decoder::PixelFormat::RGB24 => (moxcms::Layout::Rgb, ColorSamples::U8(decoded), false),
        jpeg_decoder::PixelFormat::L16 => {
            return Err(JpegDecodeError::UnsupportedPixelFormat { format: "L16" });
        }
        jpeg_decoder::PixelFormat::CMYK32 => {
            (moxcms::Layout::Rgba, ColorSamples::U8(decoded), true)
        }
    };
    Ok(RawColorImage {
        width: u32::from(info.width),
        height: u32::from(info.height),
        samples,
        layout,
        profile,
        cmyk,
    }
    .finish(convert_profile)?)
}

pub(crate) fn jpeg_metadata(
    bytes: &[u8],
) -> Result<(u32, u32, crate::RasterColorPrecision), JpegDecodeError> {
    let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    decoder.set_max_decoding_buffer_size(MAX_DECODED_RGBA_BYTES);
    decoder.read_info()?;
    let info = decoder.info().ok_or(JpegDecodeError::MissingImageInfo)?;
    let dimensions = (u32::from(info.width), u32::from(info.height));
    crate::rgba::checked_rgba_len(dimensions.0, dimensions.1)?;
    let high_precision = matches!(
        info.pixel_format,
        jpeg_decoder::PixelFormat::L16 | jpeg_decoder::PixelFormat::CMYK32
    ) || has_icc_header(bytes);
    Ok((
        dimensions.0,
        dimensions.1,
        if high_precision {
            crate::RasterColorPrecision::Float32
        } else {
            crate::RasterColorPrecision::Unorm8
        },
    ))
}

fn has_icc_header(bytes: &[u8]) -> bool {
    let mut remaining = bytes.get(2..).unwrap_or_default();
    while let Some(marker) = remaining.strip_prefix(&[0xff]) {
        let Some((&kind, tail)) = marker.split_first() else {
            return false;
        };
        if kind == 0xff {
            remaining = marker;
            continue;
        }
        if kind == 0xda || kind == 0xd9 {
            break;
        }
        if kind == 0x01 || (0xd0..=0xd8).contains(&kind) {
            remaining = tail;
            continue;
        }
        let Some(length) = tail
            .get(..2)
            .map(|value| usize::from(u16::from_be_bytes([value[0], value[1]])))
        else {
            return false;
        };
        if length < 2 {
            return false;
        }
        let Some(segment) = tail.get(2..length) else {
            return false;
        };
        if kind == 0xe2 && segment.starts_with(b"ICC_PROFILE\0") {
            return true;
        }
        let Some(next) = tail.get(length..) else {
            return false;
        };
        remaining = next;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> RgbaImage {
        RgbaImage::try_new(
            3,
            2,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255,
                255, 255, 255, 255,
            ],
        )
        .unwrap()
    }

    #[test]
    fn jpeg_encodes_and_decodes_with_expected_dimensions() {
        let encoded = encode_jpeg(&sample(), 80).unwrap();
        assert_eq!(&encoded.bytes[..2], &[0xff, 0xd8]);
        assert_eq!(&encoded.bytes[encoded.bytes.len() - 2..], &[0xff, 0xd9]);
        let decoded = decode_jpeg(&encoded.bytes).unwrap();
        assert_eq!((decoded.width, decoded.height), (3, 2));
        assert_eq!(decoded.rgba.len(), 3 * 2 * 4);
    }

    #[test]
    fn encode_rejects_inconsistent_rgba_metadata() {
        let image = RgbaImage {
            width: 2,
            height: 2,
            rgba: vec![0; 15],
        };
        assert!(matches!(
            encode_jpeg(&image, 80),
            Err(JpegEncodeError::InvalidInput(
                RgbaImageError::InvalidBufferLength { .. }
            ))
        ));
    }
}
