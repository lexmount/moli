use std::borrow::Cow;

use image::ImageEncoder;

use crate::*;

// The source is three columns by two rows. Expected pixel orders are stated
// independently of the implementation's coordinate transformation.
const ORDERS: [[usize; 6]; 8] = [
    [0, 1, 2, 3, 4, 5],
    [2, 1, 0, 5, 4, 3],
    [5, 4, 3, 2, 1, 0],
    [3, 4, 5, 0, 1, 2],
    [0, 3, 1, 4, 2, 5],
    [3, 0, 4, 1, 5, 2],
    [5, 2, 4, 1, 3, 0],
    [2, 5, 1, 4, 0, 3],
];

fn exif(orientation: u16, big_endian: bool) -> Vec<u8> {
    if big_endian {
        let mut bytes = b"MM\0*\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01".to_vec();
        bytes.extend_from_slice(&orientation.to_be_bytes());
        bytes.extend_from_slice(&[0; 6]);
        bytes
    } else {
        let mut bytes = b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
        bytes.extend_from_slice(&orientation.to_le_bytes());
        bytes.extend_from_slice(&[0; 6]);
        bytes
    }
}

fn pixels() -> RgbaImage {
    RgbaImage::try_new(
        3,
        2,
        vec![
            10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255, 130, 140, 150,
            255, 160, 170, 180, 255,
        ],
    )
    .unwrap()
}

fn png(exif: &[u8], sixteen_bit: bool) -> Vec<u8> {
    let mut info = ::png::Info::with_size(3, 2);
    info.color_type = ::png::ColorType::Rgba;
    info.exif_metadata = Some(Cow::Borrowed(exif));
    info.bit_depth = if sixteen_bit {
        ::png::BitDepth::Sixteen
    } else {
        ::png::BitDepth::Eight
    };
    if sixteen_bit {
        info.source_gamma = Some(::png::ScaledFloat::new(1.0));
    }
    let samples = if sixteen_bit {
        (0..24_u16)
            .flat_map(|value| (value * 2000 + 123).to_be_bytes())
            .collect()
    } else {
        pixels().rgba
    };
    let mut bytes = Vec::new();
    let mut writer = ::png::Encoder::with_info(&mut bytes, info)
        .unwrap()
        .write_header()
        .unwrap();
    writer.write_image_data(&samples).unwrap();
    writer.finish().unwrap();
    bytes
}

fn jpeg_with_exif(encoded: &[u8], exif: &[u8], position: usize) -> Vec<u8> {
    let mut bytes = encoded[..position].to_vec();
    bytes.extend_from_slice(&[0xff, 0xe1]);
    bytes.extend_from_slice(&u16::try_from(exif.len() + 8).unwrap().to_be_bytes());
    bytes.extend_from_slice(b"Exif\0\0");
    bytes.extend_from_slice(exif);
    bytes.extend_from_slice(&encoded[position..]);
    bytes
}

fn assert_oriented(bytes: &[u8], original: &RgbaImage, orientation: usize) {
    let expected_size = if orientation >= 5 { (2, 3) } else { (3, 2) };
    let metadata = probe_raster_image(bytes).unwrap();
    assert_eq!((metadata.width, metadata.height), expected_size);
    assert_eq!(raster_image_dimensions(bytes).unwrap(), expected_size);
    let decoded = decode_raster_image_with_metadata(bytes, metadata).unwrap();
    assert_eq!(decoded.metadata, metadata);
    assert_eq!((decoded.image.width, decoded.image.height), expected_size);
    let expected: Vec<_> = ORDERS[orientation - 1]
        .iter()
        .flat_map(|pixel| original.rgba[pixel * 4..pixel * 4 + 4].iter().copied())
        .collect();
    assert_eq!(decoded.image.rgba, expected);
    assert_eq!(metadata.decoded_byte_len(), original.byte_len());
}

#[test]
fn raster_codecs_apply_all_eight_exif_orientations_with_matching_metadata() {
    let source = pixels();
    let jpeg = encode_jpeg(&source, 95).unwrap();
    let jpeg_pixels = decode_jpeg(&jpeg.bytes).unwrap();
    for big_endian in [false, true] {
        for orientation in 1..=8 {
            let exif = exif(orientation as u16, big_endian);
            let png = png(&exif, false);
            assert_oriented(&png, &source, orientation);
            assert_eq!(
                decode_png(&png).unwrap(),
                decode_raster_image(&png).unwrap().image
            );
            let jpeg = jpeg_with_exif(&jpeg.bytes, &exif, 2);
            assert_oriented(&jpeg, &jpeg_pixels, orientation);
            assert_eq!(
                decode_jpeg(&jpeg).unwrap(),
                decode_raster_image(&jpeg).unwrap().image
            );
            let mut webp = Vec::new();
            let mut encoder = image::codecs::webp::WebPEncoder::new_lossless(&mut webp);
            encoder.set_exif_metadata(exif).unwrap();
            encoder
                .write_image(&source.rgba, 3, 2, image::ExtendedColorType::Rgba8)
                .unwrap();
            assert_oriented(&webp, &source, orientation);
        }
    }
}

#[test]
fn oriented_sixteen_bit_samples_and_float_colors_remain_in_sync() {
    let original = decode_raster_image(&png(&exif(1, false), true)).unwrap();
    let ignored_original = decode_raster_image_with_color_conversion(
        &png(&exif(1, false), true),
        ColorProfileConversion::Ignore,
    )
    .unwrap();
    let original_colors = original.color_managed.unwrap();
    let Some(UnconvertedRasterPixels::Unorm16(original_samples)) = original_colors.unconverted
    else {
        panic!("16-bit samples must be retained");
    };
    for orientation in 1..=8 {
        let bytes = png(&exif(orientation as u16, false), true);
        let decoded = decode_raster_image(&bytes).unwrap();
        let colors = decoded.color_managed.unwrap();
        let expected_size = if orientation >= 5 { (2, 3) } else { (3, 2) };
        assert_eq!((colors.width, colors.height), expected_size);
        let Some(UnconvertedRasterPixels::Unorm16(samples)) = colors.unconverted else {
            panic!("16-bit samples must be retained after orientation");
        };
        for (output, input) in ORDERS[orientation - 1].iter().enumerate() {
            assert_eq!(
                &samples[output * 4..output * 4 + 4],
                &original_samples[input * 4..input * 4 + 4]
            );
            assert_eq!(
                &colors.rgba[output * 4..output * 4 + 4],
                &original_colors.rgba[input * 4..input * 4 + 4]
            );
            assert_eq!(
                &decoded.image.rgba[output * 4..output * 4 + 4],
                &original.image.rgba[input * 4..input * 4 + 4]
            );
        }
        let ignored =
            decode_raster_image_with_color_conversion(&bytes, ColorProfileConversion::Ignore)
                .unwrap();
        for (output, input) in ORDERS[orientation - 1].iter().enumerate() {
            assert_eq!(
                &ignored.image.rgba[output * 4..output * 4 + 4],
                &ignored_original.image.rgba[input * 4..input * 4 + 4]
            );
        }
        assert_eq!(
            decoded.metadata.decoded_byte_len(),
            original.metadata.decoded_byte_len()
        );
    }
}

#[test]
fn invalid_exif_is_ignored_without_rejecting_valid_raster_pixels() {
    for exif in [
        vec![],
        b"invalid TIFF".to_vec(),
        exif(0, false),
        exif(9, true),
        exif(6, false)[..19].to_vec(),
    ] {
        let bytes = png(&exif, false);
        assert_eq!(decode_raster_image(&bytes).unwrap().image, pixels());
        let jpeg = encode_jpeg(&pixels(), 95).unwrap();
        assert_eq!(
            decode_jpeg(&jpeg_with_exif(&jpeg.bytes, &exif, 2)).unwrap(),
            decode_jpeg(&jpeg.bytes).unwrap()
        );
    }
}

#[test]
fn jpeg_orientation_after_frame_header_is_probed_without_decoding_pixels() {
    let encoded = encode_jpeg(&pixels(), 95).unwrap().bytes;
    let sof = encoded
        .windows(2)
        .position(|bytes| bytes == [0xff, 0xc0])
        .unwrap();
    let length = usize::from(u16::from_be_bytes([encoded[sof + 2], encoded[sof + 3]]));
    let bytes = jpeg_with_exif(&encoded, &exif(6, false), sof + length + 2);
    assert_oriented(&bytes, &decode_jpeg(&encoded).unwrap(), 6);
    let scan = bytes
        .windows(2)
        .position(|bytes| bytes == [0xff, 0xda])
        .unwrap();
    assert_eq!(raster_image_dimensions(&bytes[..scan]).unwrap(), (2, 3));
    assert!(decode_raster_image(&bytes[..scan]).is_err());
}
