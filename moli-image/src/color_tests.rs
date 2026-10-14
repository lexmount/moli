use std::borrow::Cow;

use crate::*;

const P3: &[u8] = include_bytes!("../tests/data/color_profiles/Display-P3-FF0000FF.png");
const P3_ALPHA: &[u8] = include_bytes!("../tests/data/color_profiles/Display-P3-BB0000CC.png");
const P3_16: &[u8] = include_bytes!("../tests/data/color_profiles/Display-P3-BBBC00000000CCCC.png");

fn png_with_info(info: ::png::Info<'_>, pixels: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let cicp = info.coding_independent_code_points;
    let mut writer = ::png::Encoder::with_info(&mut bytes, info)
        .unwrap()
        .write_header()
        .unwrap();
    // png 0.18 reads cICP but its header encoder does not emit this field.
    if let Some(cicp) = cicp {
        writer
            .write_chunk(
                ::png::chunk::cICP,
                &[
                    cicp.color_primaries,
                    cicp.transfer_function,
                    cicp.matrix_coefficients,
                    u8::from(cicp.is_video_full_range_image),
                ],
            )
            .unwrap();
    }
    writer.write_image_data(pixels).unwrap();
    writer.finish().unwrap();
    bytes
}

#[test]
fn image_profiles_preserve_extended_gamut_alpha_and_unconverted_samples() {
    let srgb = decode_raster_image(include_bytes!(
        "../tests/data/color_profiles/sRGB-BB0000FF.png"
    ))
    .unwrap();
    assert_eq!(&srgb.image.rgba[..4], &[187, 0, 0, 255]);
    let adobe = decode_raster_image(include_bytes!(
        "../tests/data/color_profiles/Adobe-RGB-FF0000FF.png"
    ))
    .unwrap();
    assert!(adobe.color_managed.unwrap().rgba[0] > 1.0);
    let red = decode_raster_image(P3).unwrap();
    let native = red.color_managed.unwrap();
    assert!(native.rgba[0] > 1.0);
    assert!(native.rgba[1] < 0.0 && native.rgba[2] < 0.0);
    for (actual, expected) in native.rgba[..3].iter().zip([1.0931, -0.2267, -0.1501]) {
        assert!(
            (actual - expected).abs() < 0.002,
            "extended sRGB {:?}",
            &native.rgba[..3]
        );
    }
    assert_eq!(native.rgba[3], 1.0);
    assert_eq!(&red.image.rgba[..4], &[255, 0, 0, 255]);
    for bytes in [P3_ALPHA, P3_16] {
        let decoded = decode_raster_image(bytes).unwrap();
        assert_eq!(&decoded.image.rgba[..4], &[205, 0, 0, 204]);
        let native = decoded.color_managed.unwrap();
        assert!((native.rgba[3] - 0.8).abs() < 0.0001);
        assert!(native.unconverted.is_some());
        let raw = decode_raster_image_with_color_conversion(bytes, ColorProfileConversion::Ignore)
            .unwrap();
        assert_eq!(&raw.image.rgba[..4], &[187, 0, 0, 204]);
        assert!(
            probe_raster_image(bytes).unwrap().decoded_byte_len() >= 4 * (4 + 4 * size_of::<f32>())
        );
    }
}

#[test]
fn image_profile_lut_converts_cmyk_inks_without_treating_black_as_alpha() {
    let bytes = include_bytes!("../tests/data/color_profiles/Generic-CMYK-FF000000.jpg");
    let decoded = decode_raster_image(bytes).unwrap();
    for (actual, expected) in decoded.image.rgba[..4].iter().zip([0_u8, 163, 218, 255]) {
        assert!(
            actual.abs_diff(expected) <= 2,
            "actual {:?}",
            &decoded.image.rgba[..4]
        );
    }
    assert_eq!(decoded.color_managed.unwrap().rgba[3], 1.0);
    let raw =
        decode_raster_image_with_color_conversion(bytes, ColorProfileConversion::Ignore).unwrap();
    // JPEG quantization leaves 254 in the cyan sample of this fixture.
    let inks = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes))
        .decode()
        .unwrap();
    assert_eq!(&inks[..4], &[254, 0, 0, 0]);
    assert_eq!(&raw.image.rgba[..4], &[1, 255, 255, 255]);
}

#[test]
fn image_png_sixteen_bit_samples_keep_low_bits_and_straight_alpha() {
    let mut info = ::png::Info::with_size(1, 1);
    info.color_type = ::png::ColorType::Rgba;
    info.bit_depth = ::png::BitDepth::Sixteen;
    let values = [0x1234_u16, 0x5678, 0x9abc, 0xdef0];
    let bytes = png_with_info(
        info,
        &values
            .into_iter()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>(),
    );
    let decoded = decode_raster_image(&bytes).unwrap();
    let native = decoded.color_managed.unwrap();
    for (actual, value) in native.rgba.into_iter().zip(values) {
        assert!((actual - f32::from(value) / 65535.0).abs() < 0.000001);
    }
}

#[test]
fn image_png_gamma_applies_to_gray_colors_without_changing_alpha() {
    let mut info = ::png::Info::with_size(1, 1);
    info.color_type = ::png::ColorType::GrayscaleAlpha;
    info.bit_depth = ::png::BitDepth::Eight;
    info.source_gamma = Some(::png::ScaledFloat::new(1.0));
    let bytes = png_with_info(info, &[128, 64]);
    let reader = ::png::Decoder::new(std::io::Cursor::new(&bytes))
        .read_info()
        .unwrap();
    assert_eq!(
        reader.info().gama_chunk.map(::png::ScaledFloat::into_value),
        Some(1.0)
    );
    let decoded = decode_raster_image(&bytes).unwrap();
    assert_eq!(&decoded.image.rgba[..4], &[188, 188, 188, 64]);

    let raw =
        decode_raster_image_with_color_conversion(&bytes, ColorProfileConversion::Ignore).unwrap();
    assert_eq!(&raw.image.rgba[..4], &[128, 128, 128, 64]);
}

#[test]
fn image_invalid_icc_profiles_leave_valid_png_pixels_readable() {
    let mut info = ::png::Info::with_size(1, 1);
    info.color_type = ::png::ColorType::Rgba;
    info.bit_depth = ::png::BitDepth::Eight;
    info.icc_profile = Some(Cow::Borrowed(b"invalid ICC profile"));
    let bytes = png_with_info(info, &[23, 74, 145, 211]);
    let decoded = decode_raster_image(&bytes).unwrap();
    assert_eq!(&decoded.image.rgba[..4], &[23, 74, 145, 211]);
    assert!(decoded.color_managed.is_none());
}

#[test]
fn image_png_cicp_takes_precedence_over_icc_and_converts_neutral_gray() {
    let mut info = ::png::Info::with_size(1, 1);
    info.color_type = ::png::ColorType::Rgba;
    info.bit_depth = ::png::BitDepth::Eight;
    info.icc_profile = Some(Cow::Owned(
        moxcms::ColorProfile::new_srgb().encode().unwrap(),
    ));
    info.coding_independent_code_points = Some(::png::CodingIndependentCodePoints {
        color_primaries: 12,
        transfer_function: 13,
        matrix_coefficients: 0,
        is_video_full_range_image: true,
    });
    let red = decode_raster_image(&png_with_info(info, &[255, 0, 0, 255]))
        .unwrap()
        .color_managed
        .unwrap();
    assert!(red.rgba[0] > 1.0 && red.rgba[1] < 0.0);
    let mut info = ::png::Info::with_size(1, 1);
    info.color_type = ::png::ColorType::GrayscaleAlpha;
    info.bit_depth = ::png::BitDepth::Eight;
    info.coding_independent_code_points = Some(::png::CodingIndependentCodePoints {
        color_primaries: 1,
        transfer_function: 8,
        matrix_coefficients: 0,
        is_video_full_range_image: true,
    });
    let gray = decode_raster_image(&png_with_info(info, &[128, 64])).unwrap();
    assert_eq!(&gray.image.rgba[..4], &[188, 188, 188, 64]);
}

#[test]
fn image_jpeg_late_icc_cannot_bypass_color_plane_admission() {
    let image = RgbaImage::try_new(1, 1, vec![155, 27, 26, 255]).unwrap();
    let bytes = encode_jpeg(&image, 100).unwrap().bytes;
    let profile = moxcms::ColorProfile::new_display_p3().encode().unwrap();
    let mut payload = b"ICC_PROFILE\0\x01\x01".to_vec();
    payload.extend(profile);
    let mut marker = vec![0xff, 0xe2];
    marker.extend(u16::try_from(payload.len() + 2).unwrap().to_be_bytes());
    marker.extend(payload);
    assert_eq!(&bytes[bytes.len() - 2..], &[0xff, 0xd9]);
    let mut late = bytes.clone();
    late.splice(late.len() - 2..late.len() - 2, marker.iter().copied());
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(&late));
    decoder.decode().unwrap();
    assert!(decoder.icc_profile().is_some());
    let decoded = decode_raster_image(&late).unwrap();
    assert_eq!(
        decoded.metadata.color_precision,
        RasterColorPrecision::Unorm8
    );
    assert!(decoded.color_managed.is_none());
    assert_eq!(decoded.image, decode_raster_image(&bytes).unwrap().image);

    let mut header = bytes;
    header.splice(2..2, marker);
    let decoded = decode_raster_image(&header).unwrap();
    assert_eq!(
        decoded.metadata.color_precision,
        RasterColorPrecision::Float32
    );
    assert!(decoded.color_managed.is_some());
}
