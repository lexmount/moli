mod blit;
mod encode;
mod pixel;
mod rect;
mod text;
mod types;

pub use blit::{blit_draw_image, blit_draw_image_filtered, blit_image_data, extract_image_data};
pub use encode::{
    data_image_intrinsic_dimensions, data_image_rgba8_pixels, encode_data_url,
    image_dimensions_from_bytes, image_intrinsic_dimensions_from_bytes,
};
pub use pixel::{
    copy_rgba8_rect, flip_y_rgba8_in_place, multiply_u8_color, premultiply_rgba8_in_place,
    scale_rgba8, scale_rgba8_bilinear, scale_rgba8_nearest,
};
pub use rect::{canonicalize_fill_style, fill_style_rgba, normalize_rect, paint_rect};
pub use text::{draw_text, measure_text_width};
pub use types::{
    CanvasRect, DEFAULT_FILL_STYLE, DEFAULT_FONT, DrawImageBlit, MAX_RGBA8_BYTE_LENGTH, Rgba8Rect,
    ScaleFilter, byte_len,
};

#[cfg(test)]
mod tests {
    use super::*;

    const ONE_BY_ONE_GIF: &str = "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

    fn decode_png_dimensions_from_data_url(data_url: &str) -> (u32, u32) {
        let encoded = data_url
            .strip_prefix("data:image/png;base64,")
            .expect("png data url prefix");
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
            .expect("valid base64 png");
        moli_image::raster_image_dimensions(&bytes).expect("png metadata should parse")
    }

    #[test]
    fn fill_style_canonicalization_matches_canvas_surface() {
        assert_eq!(canonicalize_fill_style("black").as_deref(), Some("#000000"));
        assert_eq!(canonicalize_fill_style("#abc").as_deref(), Some("#aabbcc"));
        assert_eq!(
            canonicalize_fill_style("#ff000080").as_deref(),
            Some("rgba(255, 0, 0, 0.50)")
        );
        assert_eq!(
            canonicalize_fill_style("#112233ff").as_deref(),
            Some("#112233")
        );
        assert_eq!(canonicalize_fill_style("#ggg"), None);
        assert_eq!(canonicalize_fill_style("#zzzzzz"), None);
        assert_eq!(canonicalize_fill_style("#112233gg"), None);
        assert_eq!(canonicalize_fill_style("bad"), None);
    }

    #[test]
    fn hex_alpha_canonicalization_round_trips_every_byte() {
        for alpha in 0..=255_u8 {
            let raw = format!("#12ab34{alpha:02x}");
            let canonical = canonicalize_fill_style(&raw).expect("valid eight-digit hex");
            assert_eq!(
                fill_style_rgba(&canonical),
                [0x12, 0xab, 0x34, alpha],
                "{raw}: {canonical}"
            );
        }
        for alpha in 0..16_u8 {
            let raw = format!("#0f0{alpha:x}");
            let canonical = canonicalize_fill_style(&raw).expect("valid four-digit hex");
            assert_eq!(fill_style_rgba(&canonical), [0, 255, 0, alpha * 17]);
        }
    }

    #[test]
    fn malformed_hex_colors_cannot_panic_or_install_a_style() {
        for raw in ["#", "#12", "#12345", "#gggg", "#💚", "#éé", "#0000000x"] {
            assert_eq!(canonicalize_fill_style(raw), None, "{raw}");
            assert_eq!(fill_style_rgba(raw), [0, 0, 0, 255], "{raw}");
        }
    }

    #[test]
    fn data_url_zero_and_png_dimensions_are_stable() {
        assert_eq!(encode_data_url(&[], 0, 0).as_deref(), Some("data:,"));

        let bytes = vec![0; byte_len(17, 9).expect("buffer len")];
        let url = encode_data_url(&bytes, 17, 9).expect("png data url");
        assert_eq!(decode_png_dimensions_from_data_url(&url), (17, 9));
        assert_eq!(encode_data_url(&bytes[..bytes.len() - 1], 17, 9), None);
    }

    #[test]
    fn data_image_rgba8_pixels_decodes_canvas_png_data_urls() {
        let bytes = vec![0, 0, 0, 255, 255, 0, 0, 128];
        let url = encode_data_url(&bytes, 2, 1).expect("png data url");

        assert_eq!(data_image_rgba8_pixels(&url), Some((bytes, 2, 1)));
        assert_eq!(
            data_image_rgba8_pixels("data:text/plain,not an image"),
            None
        );
        assert_eq!(
            data_image_rgba8_pixels("data:image/png-bad;base64,iVBORw0KGgo="),
            None
        );
    }

    #[test]
    fn data_image_intrinsic_dimensions_accepts_mime_parameters() {
        assert_eq!(
            data_image_intrinsic_dimensions(&format!(
                "data:IMAGE/GIF ; charset=utf-8; base64,{ONE_BY_ONE_GIF}"
            )),
            Some((1, 1))
        );
    }

    #[test]
    fn data_image_intrinsic_dimensions_uses_data_url_processor_for_plain_bodies() {
        assert_eq!(
            data_image_intrinsic_dimensions(
                "data:image/gif,GIF89a%01%00%01%00%80%00%00%00%00%00%ff%ff%ff%21%f9%04%01%00%00%00%00%2c%00%00%00%00%01%00%01%00%00%02%01D%00%3b#ignored"
            ),
            Some((1, 1))
        );
    }

    #[test]
    fn data_image_intrinsic_dimensions_sniffs_mislabelled_image_bytes() {
        assert_eq!(
            data_image_intrinsic_dimensions(
                "data:text/plain,GIF89a%01%00%01%00%80%00%00%00%00%00%ff%ff%ff%21%f9%04%01%00%00%00%00%2c%00%00%00%00%01%00%01%00%00%02%01D%00%3b"
            ),
            Some((1, 1))
        );
    }

    #[test]
    fn data_image_intrinsic_dimensions_reads_svg_size_attributes() {
        assert_eq!(
            data_image_intrinsic_dimensions(
                r#"data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg"/>"#
            ),
            Some((300, 150))
        );
        assert_eq!(
            data_image_intrinsic_dimensions(
                r#"data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg" width="500" height="400"/>"#
            ),
            Some((500, 400))
        );
        assert_eq!(
            data_image_intrinsic_dimensions(
                r#"data:image/svg+xml,<svg data-note="1 > 0" width="12" height="8"/>"#
            ),
            Some((12, 8))
        );
        assert_eq!(
            data_image_intrinsic_dimensions(
                r#"data:image/svg+xml,<svg data-note='1 > 0' width="13" height="9"/>"#
            ),
            Some((13, 9))
        );
        assert_eq!(
            data_image_intrinsic_dimensions(
                r#"data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg" width="500"/>"#
            ),
            Some((500, 150))
        );
        assert_eq!(
            data_image_intrinsic_dimensions(
                r#"data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg" width="500" height="100%"/>"#
            ),
            Some((500, 150))
        );
        assert_eq!(
            data_image_intrinsic_dimensions(
                r#"data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1000"/>"#
            ),
            Some((150, 150))
        );
        assert_eq!(
            data_image_intrinsic_dimensions(
                r#"data:image/svg+xml-bad,<svg xmlns="http://www.w3.org/2000/svg" width="3" height="4"/>"#
            ),
            None
        );
        assert_eq!(
            data_image_intrinsic_dimensions(
                r#"data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg" width="400" viewBox="0 0 800 600"/>"#
            ),
            Some((400, 300))
        );
        assert_eq!(
            data_image_intrinsic_dimensions(
                r#"data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg" height="300" viewBox="0 0 800 600"/>"#
            ),
            Some((400, 300))
        );
    }

    #[test]
    fn data_image_intrinsic_dimensions_rejects_non_image_mime() {
        assert_eq!(
            data_image_intrinsic_dimensions("data:application/octet-stream;base64,aGVsbG8="),
            None
        );
    }

    #[test]
    fn rgba8_byte_len_matches_servo_allocation_guard() {
        assert_eq!(byte_len(1, 1), Some(4));
        assert_eq!(byte_len(23_170, 23_170), Some(2_147_395_600));
        assert_eq!(byte_len(23_171, 23_171), None);
        assert_eq!(byte_len(u32::MAX, u32::MAX), None);
    }

    #[test]
    fn servo_pixel_helpers_premultiply_and_flip_rows() {
        let mut pixels = vec![
            100, 50, 25, 128, 10, 20, 30, 255, //
            1, 2, 3, 0, 40, 80, 120, 255,
        ];

        assert_eq!(premultiply_rgba8_in_place(&mut pixels), Some(false));
        assert_eq!(
            pixels,
            vec![
                50, 25, 13, 128, 10, 20, 30, 255, //
                0, 0, 0, 0, 40, 80, 120, 255,
            ]
        );

        flip_y_rgba8_in_place(&mut pixels, 2, 2).expect("valid surface");
        assert_eq!(
            pixels,
            vec![
                0, 0, 0, 0, 40, 80, 120, 255, //
                50, 25, 13, 128, 10, 20, 30, 255,
            ]
        );
    }

    #[test]
    fn copy_rgba8_rect_copies_rows_and_rejects_invalid_surfaces() {
        let source = vec![
            1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, //
            4, 0, 0, 255, 5, 0, 0, 255, 6, 0, 0, 255,
        ];
        let mut dest = vec![0; byte_len(4, 3).expect("buffer len")];

        copy_rgba8_rect(
            &source,
            3,
            2,
            Rgba8Rect::new(1, 0, 2, 2).expect("valid rect"),
            &mut dest,
            4,
            3,
            1,
            1,
        )
        .expect("copy should fit");

        assert_eq!(
            extract_image_data(&dest, 4, 3, 1, 1, 2, 2),
            vec![
                2, 0, 0, 255, 3, 0, 0, 255, //
                5, 0, 0, 255, 6, 0, 0, 255,
            ]
        );
        assert_eq!(
            copy_rgba8_rect(
                &source[..source.len() - 1],
                3,
                2,
                Rgba8Rect::new(0, 0, 1, 1).expect("valid rect"),
                &mut dest,
                4,
                3,
                0,
                0,
            ),
            None
        );
    }

    #[test]
    fn scale_rgba8_nearest_rasterizes_visible_draw_image_region() {
        let source = vec![
            255, 0, 0, 255, 0, 255, 0, 255, //
            0, 0, 255, 255, 255, 255, 0, 255,
        ];
        let scaled = scale_rgba8_nearest(
            &source,
            2,
            2,
            DrawImageBlit::new(0.0, 0.0, 2.0, 2.0, -1.0, 0.0, 2.0, 2.0).expect("valid blit"),
            0,
            0,
            1,
            2,
        )
        .expect("scale should fit");

        assert_eq!(
            scaled,
            vec![
                0, 255, 0, 255, //
                255, 255, 0, 255,
            ]
        );
    }

    #[test]
    fn scale_rgba8_bilinear_interpolates_visible_region() {
        let source = vec![255, 0, 0, 255, 0, 255, 0, 255];
        let scaled = scale_rgba8(
            &source,
            2,
            1,
            DrawImageBlit::new(0.0, 0.0, 2.0, 1.0, 0.0, 0.0, 3.0, 1.0).expect("valid blit"),
            0,
            0,
            3,
            1,
            ScaleFilter::Bilinear,
        )
        .expect("scale should fit");

        assert_eq!(
            scaled,
            vec![
                255, 0, 0, 255, //
                85, 170, 0, 255, //
                0, 255, 0, 255,
            ]
        );
    }

    #[test]
    fn paint_rect_clips_to_canvas_bounds() {
        let mut pixels = vec![0; byte_len(4, 4).expect("buffer len")];
        paint_rect(&mut pixels, 4, 4, (2, 1, 6, 3), [255, 0, 0, 255]);

        let hot = pixels
            .chunks_exact(4)
            .filter(|px| px[0] == 255 && px[3] == 255)
            .count();
        assert_eq!(hot, 4);
    }

    #[test]
    fn measure_text_scales_with_font_size_and_draw_text_changes_pixels() {
        let small = measure_text_width("Hi", "10px sans-serif");
        let large = measure_text_width("Hi", "24px sans-serif");
        assert!(large > small);

        let mut pixels = vec![0; byte_len(96, 48).expect("buffer len")];
        draw_text(
            &mut pixels,
            96,
            48,
            "Moli",
            4.0,
            24.0,
            "16px sans-serif",
            fill_style_rgba("#ff0000"),
        );
        assert!(pixels.iter().any(|&value| value != 0));
    }

    #[test]
    fn draw_image_blit_validation_and_normalized_rect_cover_invalid_inputs() {
        assert!(
            DrawImageBlit::new(0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0).is_some(),
            "positive finite blit should be accepted"
        );
        assert!(
            DrawImageBlit::new(f64::NAN, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0).is_none(),
            "non-finite source coordinate should be rejected"
        );
        assert!(
            DrawImageBlit::new(0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0).is_none(),
            "non-positive source width should be rejected"
        );
        assert!(
            DrawImageBlit::new(0.0, 0.0, 1.0, 1.0, 0.0, 0.0, -1.0, 1.0).is_none(),
            "non-positive destination width should be rejected"
        );

        assert_eq!(normalize_rect(5.0, 7.0, -2.0, -3.0), Some((3, 4, 5, 7)));
        assert_eq!(normalize_rect(1.1, 2.2, 3.3, 4.4), Some((1, 2, 5, 7)));
        assert_eq!(normalize_rect(f64::INFINITY, 0.0, 1.0, 1.0), None);
    }

    #[test]
    fn extract_and_put_image_data_round_trip_pixels() {
        let mut dst = vec![0; byte_len(2, 2).expect("buffer len")];
        let src = vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ];

        blit_image_data(&mut dst, 2, 2, &src, 2, 2, 0, 0, 0, 0, 2, 2);
        assert_eq!(extract_image_data(&dst, 2, 2, 0, 0, 2, 2), src);
    }

    #[test]
    fn extract_image_data_out_of_bounds_fills_transparent_black() {
        let pixels = vec![255, 0, 0, 255];
        assert_eq!(
            extract_image_data(&pixels, 1, 1, -1, -1, 2, 2),
            vec![
                0, 0, 0, 0, 0, 0, 0, 0, //
                0, 0, 0, 0, 255, 0, 0, 255,
            ]
        );
    }

    #[test]
    fn blit_image_data_honors_dirty_rect_and_destination_clipping() {
        let source = vec![
            255, 0, 0, 255, 0, 255, 0, 255, //
            0, 0, 255, 255, 255, 255, 0, 255,
        ];

        let mut dst = vec![0; byte_len(2, 2).expect("buffer len")];
        blit_image_data(&mut dst, 2, 2, &source, 2, 2, 0, 0, 1, 0, 1, 2);
        assert_eq!(
            dst,
            vec![
                0, 255, 0, 255, 0, 0, 0, 0, //
                255, 255, 0, 255, 0, 0, 0, 0,
            ]
        );

        let mut clipped = vec![0; byte_len(1, 2).expect("buffer len")];
        blit_image_data(&mut clipped, 1, 2, &source, 2, 2, -1, 0, 0, 0, 2, 2);
        assert_eq!(
            clipped,
            vec![
                0, 255, 0, 255, //
                255, 255, 0, 255,
            ]
        );

        let mut negative_destination = vec![0; byte_len(1, 2).expect("buffer len")];
        blit_image_data(
            &mut negative_destination,
            1,
            2,
            &source,
            2,
            2,
            -1,
            0,
            0,
            0,
            2,
            2,
        );
        assert_eq!(
            negative_destination,
            vec![
                0, 255, 0, 255, //
                255, 255, 0, 255,
            ]
        );
    }

    #[test]
    fn draw_image_supports_scaling_and_cropping() {
        let source = vec![
            255, 0, 0, 255, 0, 255, 0, 255, //
            0, 0, 255, 255, 255, 255, 0, 255,
        ];

        let mut scaled = vec![0; byte_len(4, 4).expect("buffer len")];
        blit_draw_image(
            &mut scaled,
            4,
            4,
            &source,
            2,
            2,
            DrawImageBlit::new(0.0, 0.0, 2.0, 2.0, 1.0, 1.0, 2.0, 2.0).expect("valid blit"),
        );
        let scaled_crop = extract_image_data(&scaled, 4, 4, 1, 1, 2, 2);
        assert_eq!(
            scaled_crop,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, //
                0, 0, 255, 255, 255, 255, 0, 255,
            ]
        );

        let mut cropped = vec![0; byte_len(1, 2).expect("buffer len")];
        blit_draw_image(
            &mut cropped,
            1,
            2,
            &source,
            2,
            2,
            DrawImageBlit::new(1.0, 0.0, 1.0, 2.0, 0.0, 0.0, 1.0, 2.0).expect("valid crop"),
        );
        assert_eq!(cropped, vec![0, 255, 0, 255, 255, 255, 0, 255]);
    }
}
