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
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(bytes),
        png::Limits {
            bytes: MAX_DECODED_RGBA_BYTES,
        },
    );
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let (width, height) = (reader.info().width, reader.info().height);
    checked_rgba_len(width, height)?;
    let output_buffer_size = reader
        .output_buffer_size()
        .ok_or(RgbaImageError::BufferLengthOverflow { width, height })?;
    let mut decoded = vec![0; output_buffer_size];
    let output = reader.next_frame(&mut decoded)?;
    decoded.truncate(output.buffer_size());

    let rgba = match output.color_type {
        png::ColorType::Rgba => decoded,
        png::ColorType::Rgb => decoded
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        png::ColorType::Grayscale => decoded
            .into_iter()
            .flat_map(|value| [value, value, value, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => decoded
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
            .collect(),
        png::ColorType::Indexed => {
            return Err(PngDecodeError::UnsupportedColorType {
                color_type: output.color_type,
            });
        }
    };
    RgbaImage::try_new(output.width, output.height, rgba).map_err(Into::into)
}

pub(crate) fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32), PngDecodeError> {
    let decoder = png::Decoder::new_with_limits(
        Cursor::new(bytes),
        png::Limits {
            bytes: MAX_DECODED_RGBA_BYTES,
        },
    );
    let reader = decoder.read_info()?;
    let dimensions = (reader.info().width, reader.info().height);
    checked_rgba_len(dimensions.0, dimensions.1)?;
    Ok(dimensions)
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
