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
    let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    decoder.set_max_decoding_buffer_size(MAX_DECODED_RGBA_BYTES);
    let decoded = decoder.decode()?;
    let info = decoder.info().ok_or(JpegDecodeError::MissingImageInfo)?;
    let rgba = match info.pixel_format {
        jpeg_decoder::PixelFormat::L8 => decoded
            .into_iter()
            .flat_map(|value| [value, value, value, 255])
            .collect(),
        jpeg_decoder::PixelFormat::RGB24 => decoded
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        jpeg_decoder::PixelFormat::L16 => {
            return Err(JpegDecodeError::UnsupportedPixelFormat { format: "L16" });
        }
        jpeg_decoder::PixelFormat::CMYK32 => {
            return Err(JpegDecodeError::UnsupportedPixelFormat { format: "CMYK32" });
        }
    };
    RgbaImage::try_new(u32::from(info.width), u32::from(info.height), rgba).map_err(Into::into)
}

pub(crate) fn jpeg_dimensions(bytes: &[u8]) -> Result<(u32, u32), JpegDecodeError> {
    let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    decoder.set_max_decoding_buffer_size(MAX_DECODED_RGBA_BYTES);
    decoder.read_info()?;
    let info = decoder.info().ok_or(JpegDecodeError::MissingImageInfo)?;
    let dimensions = (u32::from(info.width), u32::from(info.height));
    crate::rgba::checked_rgba_len(dimensions.0, dimensions.1)?;
    Ok(dimensions)
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
