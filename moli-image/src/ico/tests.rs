use super::*;
use crate::{
    RasterDecodeError, RasterImageFormat, decode_raster_image, probe_raster_image,
    raster_image_dimensions, supports_image_mime_essence,
};

fn icon(entries: &[(u8, u8, u16, &[u8])]) -> Vec<u8> {
    let mut bytes = vec![0, 0, 1, 0];
    bytes.extend_from_slice(&u16::try_from(entries.len()).unwrap().to_le_bytes());
    let mut offset = 6 + entries.len() * 16;
    for &(width, height, depth, payload) in entries {
        bytes.extend_from_slice(&[width, height, 0, 0]);
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&depth.to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(offset).unwrap().to_le_bytes());
        offset += payload.len();
    }
    for &(_, _, _, payload) in entries {
        bytes.extend_from_slice(payload);
    }
    bytes
}

fn png() -> Vec<u8> {
    crate::encode_png_rgba8(2, 1, &[255, 0, 0, 255, 0, 0, 255, 128])
        .unwrap()
        .bytes
}

#[test]
fn png_icons_share_bounded_metadata_and_transparent_pixels() {
    let png = png();
    let bytes = icon(&[(2, 1, 32, &png)]);
    let metadata = probe_raster_image(&bytes).unwrap();
    assert_eq!(metadata.format, RasterImageFormat::Ico);
    assert_eq!(metadata.decoded_byte_len(), 8);
    assert_eq!(raster_image_dimensions(&bytes).unwrap(), (2, 1));
    assert_eq!(
        decode_raster_image(&bytes).unwrap().image,
        crate::decode_png(&png).unwrap()
    );
    for essence in ["image/x-icon", "image/vnd.microsoft.icon"] {
        assert!(supports_image_mime_essence(essence), "{essence}");
    }
}

#[test]
fn png_icons_retain_full_precision_samples() {
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Sixteen);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xff, 0xff])
            .unwrap();
        writer.finish().unwrap();
    }
    let plain = decode_raster_image(&png).unwrap();
    let wrapped = decode_raster_image(&icon(&[(1, 1, 32, &png)])).unwrap();
    assert_eq!(
        wrapped.metadata.color_precision,
        RasterColorPrecision::Float32
    );
    assert_eq!(wrapped.image, plain.image);
    assert!(wrapped.color_managed.is_some());
    assert_eq!(wrapped.color_managed, plain.color_managed);
}

#[test]
fn png_icon_frames_ignore_embedded_exif_orientation() {
    let exif = b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut info = png::Info::with_size(2, 1);
    info.color_type = png::ColorType::Rgba;
    info.exif_metadata = Some(std::borrow::Cow::Borrowed(exif));
    let mut png = Vec::new();
    {
        let mut writer = png::Encoder::with_info(&mut png, info)
            .unwrap()
            .write_header()
            .unwrap();
        writer
            .write_image_data(&[255, 0, 0, 255, 0, 0, 255, 255])
            .unwrap();
        writer.finish().unwrap();
    }
    let bytes = icon(&[(2, 1, 32, &png)]);
    assert_eq!(raster_image_dimensions(&bytes).unwrap(), (2, 1));
    let decoded = decode_raster_image(&bytes).unwrap();
    assert_eq!((decoded.image.width, decoded.image.height), (2, 1));
    assert_eq!(decoded.image.rgba, [255, 0, 0, 255, 0, 0, 255, 255]);
}

#[test]
fn bmp_icons_apply_bottom_up_rows_and_the_and_mask() {
    let mut dib = Vec::new();
    dib.extend_from_slice(&40_u32.to_le_bytes());
    dib.extend_from_slice(&2_i32.to_le_bytes());
    dib.extend_from_slice(&4_i32.to_le_bytes()); // XOR pixels and AND mask share the height.
    dib.extend_from_slice(&1_u16.to_le_bytes());
    dib.extend_from_slice(&24_u16.to_le_bytes());
    dib.extend_from_slice(&0_u32.to_le_bytes());
    dib.extend_from_slice(&24_u32.to_le_bytes());
    dib.extend_from_slice(&[0; 16]);
    dib.extend_from_slice(&[
        255, 0, 0, 255, 255, 255, 0, 0, // blue, white, padding (bottom row)
        0, 0, 255, 0, 255, 0, 0, 0, // red, green, padding (top row)
        0x80, 0, 0, 0, // bottom-left pixel is transparent
        0, 0, 0, 0,
    ]);
    let bytes = icon(&[(2, 2, 24, &dib)]);
    assert_eq!(raster_image_dimensions(&bytes).unwrap(), (2, 2));
    assert_eq!(
        decode_raster_image(&bytes).unwrap().image.rgba,
        [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 0, 255, 255, 255, 255,
        ]
    );
}

#[test]
fn icon_metadata_and_decode_select_the_same_directory_entry() {
    let large = png();
    let small = crate::encode_png_rgba8(1, 1, &[0, 255, 0, 255])
        .unwrap()
        .bytes;
    for bytes in [
        icon(&[(1, 1, 32, &small), (2, 1, 32, &large)]),
        icon(&[(2, 1, 32, &large), (1, 1, 32, &small)]),
    ] {
        assert_eq!(raster_image_dimensions(&bytes).unwrap(), (2, 1));
        assert_eq!(
            decode_raster_image(&bytes).unwrap().image,
            crate::decode_png(&large).unwrap()
        );
    }
}

#[test]
fn png_icon_metadata_does_not_inflate_the_pixel_payload() {
    let mut png = png();
    let idat = png.windows(4).position(|chunk| chunk == b"IDAT").unwrap();
    png[idat + 4] ^= 0xff;
    let bytes = icon(&[(2, 1, 32, &png)]);
    assert_eq!(raster_image_dimensions(&bytes).unwrap(), (2, 1));
    assert!(decode_raster_image(&bytes).is_err());
}

#[test]
fn icon_probe_rejects_invalid_directories_offsets_and_dimensions() {
    assert!(matches!(
        probe_raster_image(b"\0\0\x01\0\0\0"),
        Err(RasterDecodeError::Ico(IcoDecodeError::InvalidDirectory))
    ));
    let png = png();
    let bytes = icon(&[(2, 1, 32, &png)]);
    for end in [5, 6, 21, bytes.len() - 1] {
        assert!(
            probe_raster_image(&bytes[..end]).is_err(),
            "truncated at {end}"
        );
    }
    for offset in [0_u32, u32::MAX] {
        let mut invalid = bytes.clone();
        invalid[18..22].copy_from_slice(&offset.to_le_bytes());
        assert!(matches!(
            probe_raster_image(&invalid),
            Err(RasterDecodeError::Ico(IcoDecodeError::InvalidEntry))
        ));
    }
    assert!(matches!(
        probe_raster_image(&icon(&[(1, 1, 32, &png)])),
        Err(RasterDecodeError::Ico(IcoDecodeError::DimensionMismatch))
    ));
}
