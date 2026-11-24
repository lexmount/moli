use libjpeg_turbo_rs::{DecodeLimits, Decoder, FrameHeader, PixelFormat, Subsampling};

use crate::{MAX_DECODED_RGBA_BYTES, MAX_ENCODED_IMAGE_BYTES, RgbaImage, RgbaImageError};

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
    Encoding(#[from] libjpeg_turbo_rs::JpegError),
}

#[derive(Debug, thiserror::Error)]
pub enum JpegDecodeError {
    #[error("JPEG decoding failed: {0}")]
    Decoding(#[from] libjpeg_turbo_rs::JpegError),
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
    let sampling = if quality < 90 {
        Subsampling::S420
    } else {
        Subsampling::S444
    };
    let bytes = libjpeg_turbo_rs::compress(
        &image.rgba,
        usize::from(width),
        usize::from(height),
        PixelFormat::Rgba,
        quality,
        sampling,
    )?;
    if bytes.len() > MAX_ENCODED_IMAGE_BYTES {
        return Err(JpegEncodeError::EncodedBufferBudgetExceeded {
            actual: bytes.len(),
            max: MAX_ENCODED_IMAGE_BYTES,
        });
    }
    Ok(EncodedJpeg {
        width: image.width,
        height: image.height,
        bytes,
    })
}

pub fn decode_jpeg(bytes: &[u8]) -> Result<RgbaImage, JpegDecodeError> {
    let mut decoder = Decoder::new_with_limits(bytes, jpeg_decode_limits())?;
    let header = decoder.header();
    let (width, height) = checked_jpeg_dimensions(header)?;
    if header.precision > 8 {
        return Err(JpegDecodeError::UnsupportedPixelFormat { format: "L16" });
    }
    if header.components.len() == 4 {
        return Err(JpegDecodeError::UnsupportedPixelFormat { format: "CMYK32" });
    }
    decoder.set_output_format(PixelFormat::Rgba);
    let decoded = decoder.decode_image()?;
    RgbaImage::try_new(width, height, decoded.into_vec()).map_err(Into::into)
}

pub(crate) fn jpeg_dimensions(bytes: &[u8]) -> Result<(u32, u32), JpegDecodeError> {
    // Stop at the first scan header, including for progressive JPEGs.
    let decoder = Decoder::new_header_only(bytes, jpeg_decode_limits())?;
    checked_jpeg_dimensions(decoder.header())
}

fn jpeg_decode_limits() -> DecodeLimits {
    DecodeLimits {
        max_width: usize::from(u16::MAX),
        max_height: usize::from(u16::MAX),
        max_pixels: (MAX_DECODED_RGBA_BYTES / 4) as u64,
        ..DecodeLimits::default()
    }
}

fn checked_jpeg_dimensions(header: &FrameHeader) -> Result<(u32, u32), JpegDecodeError> {
    let dimensions = (u32::from(header.width), u32::from(header.height));
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
    fn jpeg_preserves_chroma_sampling_and_discards_alpha() {
        let opaque = sample();
        let mut transparent = opaque.clone();
        for pixel in transparent.rgba.as_chunks_mut::<4>().0 {
            pixel[3] = 0;
        }
        for (quality, sampling) in [(89, Subsampling::S420), (90, Subsampling::S444)] {
            let encoded = encode_jpeg(&opaque, quality).unwrap();
            assert_eq!(
                libjpeg_turbo_rs::probe(&encoded.bytes).unwrap().subsampling,
                sampling
            );
            assert_eq!(encode_jpeg(&transparent, quality).unwrap(), encoded);
        }
    }

    #[test]
    fn probe_progressive_jpeg_needs_no_scan_data() {
        let bytes = libjpeg_turbo_rs::compress_progressive(
            &[128; 8 * 8],
            8,
            8,
            PixelFormat::Grayscale,
            75,
            Subsampling::S444,
        )
        .unwrap();
        let scan = bytes
            .windows(2)
            .position(|marker| marker == [0xff, 0xda])
            .unwrap();
        let scan_len = u16::from_be_bytes([bytes[scan + 2], bytes[scan + 3]]);
        let headers = &bytes[..scan + 2 + usize::from(scan_len)];
        assert_eq!(jpeg_dimensions(headers).unwrap(), (8, 8));
        let image = decode_jpeg(&bytes).unwrap();
        assert_eq!(image.rgba, [128, 128, 128, 255].repeat(8 * 8));
    }

    #[test]
    fn decode_lossless_grayscale_preserves_samples() {
        let samples = [5, 32, 97, 129, 202, 255];
        let bytes =
            libjpeg_turbo_rs::compress_lossless(&samples, 3, 2, PixelFormat::Grayscale).unwrap();
        let image = decode_jpeg(&bytes).unwrap();
        assert_eq!((image.width, image.height), (3, 2));
        for (pixel, value) in image.rgba.as_chunks::<4>().0.iter().zip(samples) {
            assert_eq!(*pixel, [value, value, value, 255]);
        }
    }

    #[test]
    fn jpeg_dimensions_reject_an_rgba_buffer_above_the_budget() {
        let mut bytes = encode_jpeg(&sample(), 75).unwrap().bytes;
        let frame = bytes
            .windows(2)
            .position(|marker| marker == [0xff, 0xc0])
            .unwrap();
        let dimension = 8192_u16.to_be_bytes();
        bytes[frame + 5..frame + 7].copy_from_slice(&dimension);
        bytes[frame + 7..frame + 9].copy_from_slice(&dimension);
        for result in [
            jpeg_dimensions(&bytes).map(|_| ()),
            decode_jpeg(&bytes).map(|_| ()),
        ] {
            assert!(matches!(
                result,
                Err(JpegDecodeError::InvalidOutput(
                    RgbaImageError::BufferBudgetExceeded { .. }
                ))
            ));
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
            encode_jpeg(&image, 80),
            Err(JpegEncodeError::InvalidInput(
                RgbaImageError::InvalidBufferLength { .. }
            ))
        ));
    }
}
