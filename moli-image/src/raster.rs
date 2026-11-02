use std::io::{BufReader, Cursor};

use image::{ImageFormat, ImageReader, Limits};

use crate::{
    JpegDecodeError, MAX_DECODED_RGBA_BYTES, MAX_ENCODED_IMAGE_BYTES, PngDecodeError, RgbaImage,
    RgbaImageError, decode_jpeg, decode_png, jpeg::jpeg_dimensions, png::png_dimensions,
};

/// Static raster formats admitted by the HTML image pipeline.
///
/// GIF and WebP are decoded to their first composited frame. Moli has no
/// animation timeline/compositor, so later frames deliberately remain outside
/// this codec contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RasterImageFormat {
    Png,
    Jpeg,
    Gif,
    WebP,
    Bmp,
}

impl RasterImageFormat {
    fn from_image_format(format: ImageFormat) -> Option<Self> {
        match format {
            ImageFormat::Png => Some(Self::Png),
            ImageFormat::Jpeg => Some(Self::Jpeg),
            ImageFormat::Gif => Some(Self::Gif),
            ImageFormat::WebP => Some(Self::WebP),
            ImageFormat::Bmp => Some(Self::Bmp),
            _ => None,
        }
    }
}

/// Whether the image pipeline can consume a parsed, lowercase MIME essence.
/// Keep preload admission tied to the codecs admitted by the image decoder.
pub fn supports_image_mime_essence(essence: &str) -> bool {
    if essence == "image/svg+xml" {
        return true;
    }
    let essence = match essence {
        "image/x-png" => "image/png",
        "image/pjpeg" => "image/jpeg",
        _ => essence,
    };
    ImageFormat::from_mime_type(essence)
        .and_then(RasterImageFormat::from_image_format)
        .is_some()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RasterImageMetadata {
    pub format: RasterImageFormat,
    pub width: u32,
    pub height: u32,
}

impl RasterImageMetadata {
    /// Returns the validated RGBA8 allocation required by this image.
    pub fn decoded_byte_len(self) -> usize {
        crate::rgba::checked_rgba_len(self.width, self.height)
            .expect("raster metadata is validated before construction")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedRasterImage {
    pub metadata: RasterImageMetadata,
    pub image: RgbaImage,
}

#[derive(Debug, thiserror::Error)]
pub enum RasterDecodeError {
    #[error("encoded image contains {actual} bytes, exceeding the {max}-byte input budget")]
    EncodedBufferBudgetExceeded { actual: usize, max: usize },
    #[error("image format could not be identified")]
    UnknownFormat,
    #[error("image format {0:?} is not supported")]
    UnsupportedFormat(ImageFormat),
    #[error("image metadata could not be decoded: {0}")]
    Metadata(#[source] image::ImageError),
    #[error("PNG image could not be decoded: {0}")]
    Png(#[from] PngDecodeError),
    #[error("JPEG image could not be decoded: {0}")]
    Jpeg(#[from] JpegDecodeError),
    #[error("{format:?} image could not be decoded: {source}")]
    Image {
        format: RasterImageFormat,
        #[source]
        source: image::ImageError,
    },
    #[error(
        "decoded image dimensions {actual_width}x{actual_height} do not match probed metadata {expected_width}x{expected_height}"
    )]
    MetadataMismatch {
        expected_width: u32,
        expected_height: u32,
        actual_width: u32,
        actual_height: u32,
    },
    #[error(transparent)]
    InvalidOutput(#[from] RgbaImageError),
}

pub fn probe_raster_image(bytes: &[u8]) -> Result<RasterImageMetadata, RasterDecodeError> {
    check_encoded_budget(bytes)?;
    let format = image::guess_format(bytes).map_err(|_| RasterDecodeError::UnknownFormat)?;
    let admitted = RasterImageFormat::from_image_format(format)
        .ok_or(RasterDecodeError::UnsupportedFormat(format))?;
    let (width, height) = match admitted {
        RasterImageFormat::Png => png_dimensions(bytes)?,
        RasterImageFormat::Jpeg => jpeg_dimensions(bytes)?,
        RasterImageFormat::Gif | RasterImageFormat::WebP | RasterImageFormat::Bmp => {
            let mut reader = ImageReader::with_format(BufReader::new(Cursor::new(bytes)), format);
            reader.limits(decode_limits());
            reader
                .into_dimensions()
                .map_err(RasterDecodeError::Metadata)?
        }
    };
    crate::rgba::checked_rgba_len(width, height)?;
    Ok(RasterImageMetadata {
        format: admitted,
        width,
        height,
    })
}

/// Reads encoded raster dimensions without allocating or decoding an RGBA surface.
///
/// The decoder only parses enough format metadata to determine width and
/// height. Call [`decode_raster_image`] only when a consumer explicitly needs
/// pixels.
pub fn raster_image_dimensions(bytes: &[u8]) -> Result<(u32, u32), RasterDecodeError> {
    let metadata = probe_raster_image(bytes)?;
    Ok((metadata.width, metadata.height))
}

pub fn decode_raster_image(bytes: &[u8]) -> Result<DecodedRasterImage, RasterDecodeError> {
    let metadata = probe_raster_image(bytes)?;
    decode_raster_image_with_metadata(bytes, metadata)
}

/// Decodes pixels after a caller has already probed the same immutable byte
/// buffer. This lets resource owners reserve aggregate decoded-byte budget
/// before allocating RGBA without parsing metadata a second time.
pub fn decode_raster_image_with_metadata(
    bytes: &[u8],
    metadata: RasterImageMetadata,
) -> Result<DecodedRasterImage, RasterDecodeError> {
    check_encoded_budget(bytes)?;
    let image = match metadata.format {
        RasterImageFormat::Png => decode_png(bytes)?,
        RasterImageFormat::Jpeg => decode_jpeg(bytes)?,
        format @ (RasterImageFormat::Gif | RasterImageFormat::WebP | RasterImageFormat::Bmp) => {
            let image_format = match format {
                RasterImageFormat::Gif => ImageFormat::Gif,
                RasterImageFormat::WebP => ImageFormat::WebP,
                RasterImageFormat::Bmp => ImageFormat::Bmp,
                RasterImageFormat::Png | RasterImageFormat::Jpeg => unreachable!(),
            };
            let mut reader =
                ImageReader::with_format(BufReader::new(Cursor::new(bytes)), image_format);
            reader.limits(decode_limits());
            let decoded = reader
                .decode()
                .map_err(|source| RasterDecodeError::Image { format, source })?;
            let rgba = decoded.into_rgba8();
            RgbaImage::try_new(rgba.width(), rgba.height(), rgba.into_raw())?
        }
    };
    if (image.width, image.height) != (metadata.width, metadata.height) {
        return Err(RasterDecodeError::MetadataMismatch {
            expected_width: metadata.width,
            expected_height: metadata.height,
            actual_width: image.width,
            actual_height: image.height,
        });
    }
    Ok(DecodedRasterImage { metadata, image })
}

fn check_encoded_budget(bytes: &[u8]) -> Result<(), RasterDecodeError> {
    if bytes.len() > MAX_ENCODED_IMAGE_BYTES {
        return Err(RasterDecodeError::EncodedBufferBudgetExceeded {
            actual: bytes.len(),
            max: MAX_ENCODED_IMAGE_BYTES,
        });
    }
    Ok(())
}

fn decode_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODED_RGBA_BYTES as u64);
    limits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PngEncodeOptions, encode_jpeg, encode_png_with_options};

    fn sample() -> RgbaImage {
        RgbaImage::try_new(
            2,
            2,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128,
            ],
        )
        .unwrap()
    }

    #[test]
    fn probes_and_decodes_png_through_the_common_resource_contract() {
        let encoded = encode_png_with_options(&sample(), PngEncodeOptions::default()).unwrap();
        assert_eq!(
            probe_raster_image(&encoded.bytes).unwrap(),
            RasterImageMetadata {
                format: RasterImageFormat::Png,
                width: 2,
                height: 2,
            }
        );
        assert_eq!(raster_image_dimensions(&encoded.bytes).unwrap(), (2, 2));
        assert_eq!(decode_raster_image(&encoded.bytes).unwrap().image, sample());
    }

    #[test]
    fn dimension_probe_does_not_require_or_decode_the_png_pixel_payload() {
        let encoded = encode_png_with_options(&sample(), PngEncodeOptions::default()).unwrap();
        // Keep valid structural metadata but corrupt the compressed IDAT
        // payload. A metadata probe must not inflate those pixels.
        let mut header_only = encoded.bytes.clone();
        let idat = header_only
            .windows(4)
            .position(|window| window == b"IDAT")
            .expect("encoded PNG has an IDAT chunk");
        header_only[idat + 4] ^= 0xff;

        assert_eq!(raster_image_dimensions(&header_only).unwrap(), (2, 2));
        assert!(decode_raster_image(&header_only).is_err());
    }

    #[test]
    fn rejects_non_image_and_unadmitted_raster_formats() {
        assert!(matches!(
            probe_raster_image(b"not an image"),
            Err(RasterDecodeError::UnknownFormat)
        ));
        // Minimal ICO signature is enough for format identification; the
        // browser resource contract intentionally admits only its named set.
        assert!(matches!(
            probe_raster_image(b"\0\0\x01\0\0\0\0\0\0\0\0\0\0\0\0\0"),
            Err(RasterDecodeError::UnsupportedFormat(ImageFormat::Ico))
        ));
    }

    fn bmp_fixture(
        width: i32,
        height: i32,
        bits: u16,
        palette: &[[u8; 4]],
        pixels: &[u8],
    ) -> Vec<u8> {
        let offset = 54 + palette.len() * 4;
        let mut bytes = Vec::with_capacity(offset + pixels.len());
        bytes.extend_from_slice(b"BM");
        bytes.extend_from_slice(&u32::try_from(offset + pixels.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&u32::try_from(offset).unwrap().to_le_bytes());
        bytes.extend_from_slice(&40_u32.to_le_bytes());
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&bits.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(pixels.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&[0; 8]);
        bytes.extend_from_slice(&u32::try_from(palette.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        assert_eq!(bytes.len(), 54);
        for entry in palette {
            bytes.extend_from_slice(entry);
        }
        bytes.extend_from_slice(pixels);
        bytes
    }

    fn bmp_rgb_pixels() -> Vec<u8> {
        vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ]
    }

    #[test]
    fn bmp_metadata_and_pixels_respect_bottom_up_rows_and_padding() {
        let bytes = bmp_fixture(
            2,
            2,
            24,
            &[],
            &[
                255, 0, 0, 255, 255, 255, 0xa1, 0xa2, 0, 0, 255, 0, 255, 0, 0xb1, 0xb2,
            ],
        );
        let metadata = probe_raster_image(&bytes).unwrap();
        assert_eq!(
            metadata,
            RasterImageMetadata {
                format: RasterImageFormat::Bmp,
                width: 2,
                height: 2,
            }
        );
        assert_eq!(metadata.decoded_byte_len(), 16);
        let decoded = decode_raster_image_with_metadata(&bytes, metadata).unwrap();
        assert_eq!(
            decoded.image,
            RgbaImage::try_new(2, 2, bmp_rgb_pixels()).unwrap()
        );
    }

    #[test]
    fn bmp_negative_height_preserves_top_down_pixel_order() {
        let bytes = bmp_fixture(
            2,
            -2,
            24,
            &[],
            &[
                0, 0, 255, 0, 255, 0, 0xb1, 0xb2, 255, 0, 0, 255, 255, 255, 0xa1, 0xa2,
            ],
        );
        assert_eq!(raster_image_dimensions(&bytes).unwrap(), (2, 2));
        assert_eq!(
            decode_raster_image(&bytes).unwrap().image,
            RgbaImage::try_new(2, 2, bmp_rgb_pixels()).unwrap()
        );
    }

    #[test]
    fn bmp_indexed_pixels_use_the_palette_and_ignore_row_padding() {
        let bytes = bmp_fixture(3, 1, 8, &[[0, 0, 255, 0], [0, 255, 0, 0]], &[0, 1, 0, 0xff]);
        assert_eq!(
            decode_raster_image(&bytes).unwrap().image,
            RgbaImage::try_new(3, 1, vec![255, 0, 0, 255, 0, 255, 0, 255, 255, 0, 0, 255]).unwrap()
        );
    }

    #[test]
    fn bmp_metadata_probe_does_not_decode_a_truncated_pixel_payload() {
        let mut bytes = bmp_fixture(2, 2, 24, &[], &[0; 16]);
        bytes.truncate(54);
        assert_eq!(raster_image_dimensions(&bytes).unwrap(), (2, 2));
        assert!(matches!(
            decode_raster_image(&bytes),
            Err(RasterDecodeError::Image {
                format: RasterImageFormat::Bmp,
                ..
            })
        ));
    }

    #[test]
    fn bmp_metadata_rejects_invalid_headers_and_oversized_dimensions() {
        assert!(matches!(
            probe_raster_image(b"BM\0\0\0\0\0\0\0\0\0\0\0\0\0\0"),
            Err(RasterDecodeError::Metadata(_))
        ));
        for (width, height) in [(0, 2), (2, 0), (-2, 2), (i32::MAX, i32::MAX)] {
            assert!(
                probe_raster_image(&bmp_fixture(width, height, 24, &[], &[])).is_err(),
                "{width}x{height}"
            );
        }
    }

    #[test]
    fn common_contract_admits_jpeg_gif_and_webp_pixels() {
        let jpeg = encode_jpeg(&sample(), 90).unwrap();
        assert_eq!(
            probe_raster_image(&jpeg.bytes).unwrap(),
            RasterImageMetadata {
                format: RasterImageFormat::Jpeg,
                width: 2,
                height: 2,
            }
        );
        let decoded_jpeg = decode_raster_image(&jpeg.bytes).unwrap();
        assert_eq!(
            (decoded_jpeg.image.width, decoded_jpeg.image.height),
            (2, 2)
        );

        let gif = b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff!\xf9\x04\x01\0\0\0\0,\0\0\0\0\x01\0\x01\0\0\x02\x02D\x01\0;";
        assert_eq!(
            probe_raster_image(gif).unwrap(),
            RasterImageMetadata {
                format: RasterImageFormat::Gif,
                width: 1,
                height: 1,
            }
        );
        let decoded_gif = decode_raster_image(gif).unwrap();
        assert_eq!((decoded_gif.image.width, decoded_gif.image.height), (1, 1));

        let mut webp = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut webp)
            .encode(sample().as_ref(), 2, 2, image::ExtendedColorType::Rgba8)
            .unwrap();
        assert_eq!(
            probe_raster_image(&webp).unwrap(),
            RasterImageMetadata {
                format: RasterImageFormat::WebP,
                width: 2,
                height: 2,
            }
        );
        assert_eq!(decode_raster_image(&webp).unwrap().image, sample());
    }
}
